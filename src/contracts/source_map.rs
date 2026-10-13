// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Source mapping: contract **C-SOURCEMAP**.
//!
//! Generation knows something no parser can recover: which control produced which lines. That
//! mapping is what turns "go to source" into "go to this button" (whitepaper §11.4 E1/E4), and it
//! is the reason a generated file can be edited through the design instead of by hand.
//!
//! Because the mapping is produced by the generator from a particular model, it carries both the
//! generator version and the model revision it was built from. A map built for an older revision
//! is reported as stale rather than used to jump to a line that has since moved — the failure mode
//! this avoids is worse than a missing mapping, because a *wrong* jump looks like an answer.
//!
//! A node with no entry is normal, not an error: only generated constructs have mappings, and
//! whitepaper §17.6 requires the diagnostic to keep its own source position in that case. The
//! lookup therefore returns `Option` at both ends — node to span, and span to node — and callers
//! are expected to have a second way to locate things.

use std::collections::BTreeMap;

use super::identity::NodeId;
use super::revision::{GeneratorVersion, Revision, Staleness, VersionStamp};

/// A half-open byte range inside one project-relative file.
///
/// Offsets are bytes, not characters, because that is what a generator and a compiler agree on;
/// turning them into line/column for display is the editor's job (`code_editor`'s coordinate
/// mapping, whitepaper §11.7.3), and doing it here would create a second definition of "column".
///
/// The file is carried as a project-relative path. Interning paths for large projects is a
/// measurable question and belongs with the source-mapping task (T15.03), not here.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct SourceSpan {
    file: String,
    start: u32,
    end: u32,
}

impl SourceSpan {
    /// Builds a span, or `None` when the path is blank or the range is empty or reversed.
    pub fn new(file: &str, start: u32, end: u32) -> Option<Self> {
        let file = file.trim();
        if file.is_empty() || start >= end {
            return None;
        }
        Some(Self {
            file: file.to_owned(),
            start,
            end,
        })
    }

    /// The project-relative file this span lives in.
    pub fn file(&self) -> &str {
        &self.file
    }

    /// The first byte of the span.
    pub const fn start(&self) -> u32 {
        self.start
    }

    /// One past the last byte of the span.
    pub const fn end(&self) -> u32 {
        self.end
    }

    /// Whether this span contains the given byte offset.
    pub const fn contains(&self, offset: u32) -> bool {
        self.start <= offset && offset < self.end
    }

    /// How many bytes the span covers.
    pub const fn len(&self) -> u32 {
        self.end - self.start
    }
}

/// Which control produced which source range, for one generator run.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SourceMap {
    stamp: VersionStamp,
    entries: BTreeMap<NodeId, SourceSpan>,
}

impl SourceMap {
    /// A map produced by the given generator version from the given model revision.
    pub fn new(generator: GeneratorVersion, model: Revision) -> Self {
        let mut stamp = VersionStamp::current(model);
        stamp.generator = generator;
        Self {
            stamp,
            entries: BTreeMap::new(),
        }
    }

    /// Records where a node's construct was written.
    ///
    /// Replacing an entry is allowed — regeneration after an edit rewrites the same node — and the
    /// map keeps one span per node on purpose: two spans for one node would leave "go to source"
    /// with an arbitrary choice to make.
    pub fn insert(&mut self, node: NodeId, span: SourceSpan) {
        self.entries.insert(node, span);
    }

    /// What this map was built from.
    pub fn stamp(&self) -> VersionStamp {
        self.stamp
    }

    /// The generator that produced it.
    pub fn generator(&self) -> GeneratorVersion {
        self.stamp.generator
    }

    /// Whether the model has moved on since this map was built.
    pub fn staleness(&self, current: Revision) -> Staleness {
        self.stamp.staleness(current)
    }

    /// How many nodes are mapped.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether nothing is mapped.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Where a node was written, if it was.
    pub fn span_of(&self, node: NodeId) -> Option<&SourceSpan> {
        self.entries.get(&node)
    }

    /// Which node a position belongs to.
    ///
    /// When spans nest — a container's construct contains its children's — the most specific one
    /// wins, so clicking inside a child's generated code selects the child rather than the window.
    pub fn node_at(&self, file: &str, offset: u32) -> Option<NodeId> {
        self.entries
            .iter()
            .filter(|(_, span)| span.file == file && span.contains(offset))
            .min_by_key(|(_, span)| span.len())
            .map(|(node, _)| *node)
    }
}

#[cfg(test)]
mod tests {
    use super::{SourceMap, SourceSpan};
    use crate::contracts::identity::NodeId;
    use crate::contracts::revision::{GeneratorVersion, Revision, Staleness};

    #[test]
    fn a_span_needs_a_path_and_a_range_that_can_hold_something() {
        assert!(SourceSpan::new("", 0, 4).is_none());
        assert!(SourceSpan::new("  ", 0, 4).is_none());
        assert!(SourceSpan::new("src/ui/main.rs", 4, 4).is_none());
        assert!(SourceSpan::new("src/ui/main.rs", 9, 4).is_none());
    }

    #[test]
    fn a_span_is_half_open() {
        let span = SourceSpan::new("  src/ui/main.rs  ", 10, 14).unwrap();

        assert_eq!(span.file(), "src/ui/main.rs");
        assert_eq!((span.start(), span.end()), (10, 14));
        assert_eq!(span.len(), 4);
        assert!(!span.contains(9));
        assert!(span.contains(10));
        assert!(span.contains(13));
        assert!(!span.contains(14));
    }

    #[test]
    fn a_map_carries_the_generator_and_the_revision_it_came_from() {
        let map = SourceMap::new(GeneratorVersion::new(1, 0, 0), Revision::from_raw(5));

        assert_eq!(map.stamp().model, Revision::from_raw(5));
        assert_eq!(map.stamp().generator, GeneratorVersion::new(1, 0, 0));
        assert_eq!(map.generator(), GeneratorVersion::new(1, 0, 0));
        assert!(map.is_empty());
        assert_eq!(map.staleness(Revision::from_raw(5)), Staleness::Current);
        assert_eq!(
            map.staleness(Revision::from_raw(6)),
            Staleness::Stale {
                computed_at: Revision::from_raw(5),
                current: Revision::from_raw(6)
            },
        );
    }

    #[test]
    fn a_node_keeps_one_span_and_the_latest_wins() {
        let mut map = SourceMap::new(GeneratorVersion::CURRENT, Revision::INITIAL);
        let node = NodeId::from_raw(1);
        let first = SourceSpan::new("src/generated/main.rs", 0, 10).unwrap();
        let second = SourceSpan::new("src/generated/main.rs", 40, 60).unwrap();

        map.insert(node, first);
        map.insert(node, second.clone());

        assert_eq!(map.len(), 1);
        assert_eq!(map.span_of(node), Some(&second));
    }

    #[test]
    fn an_unmapped_node_has_no_span_instead_of_a_guess() {
        let map = SourceMap::new(GeneratorVersion::CURRENT, Revision::INITIAL);

        assert_eq!(map.span_of(NodeId::from_raw(7)), None);
        assert_eq!(map.node_at("src/generated/main.rs", 3), None);
    }

    #[test]
    fn the_most_specific_span_wins_when_spans_nest() {
        let mut map = SourceMap::new(GeneratorVersion::CURRENT, Revision::INITIAL);
        let window = NodeId::from_raw(1);
        let button = NodeId::from_raw(2);
        map.insert(
            window,
            SourceSpan::new("src/generated/main.rs", 0, 100).unwrap(),
        );
        map.insert(
            button,
            SourceSpan::new("src/generated/main.rs", 40, 60).unwrap(),
        );

        assert_eq!(map.node_at("src/generated/main.rs", 50), Some(button));
        assert_eq!(map.node_at("src/generated/main.rs", 10), Some(window));
        assert_eq!(map.node_at("src/generated/main.rs", 100), None);
        assert_eq!(map.node_at("src/generated/other.rs", 50), None);
    }
}
