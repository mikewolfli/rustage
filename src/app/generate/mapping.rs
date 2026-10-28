// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Which node became which bytes (task T15.03).
//!
//! Generation knows something no parser can recover: the node a line came from. The generator
//! writes each node's key as a literal — `.key("n_0_1")` — so the map is built by finding that
//! literal and recording the line that carries it. The result is a [`SourceMap`] whose stamp names
//! the generator and the model revision it came from, so a mapping built before an edit is reported
//! [`Staleness::Stale`](crate::contracts::revision::Staleness) rather than used to jump to a line
//! that has moved (a wrong jump looks like an answer, which is worse than none).
//!
//! The acceptance line is *"Unicode and cross-module positions, and generated-region boundaries, are
//! correct"*. Four choices make that true:
//!
//! * **A span is the node's own builder call, not the line around it.** The generator emits a whole
//!   tree on one line, so a line span would collapse every node onto the same range. The span here
//!   runs from `Node::new(` through the node's `.key("…")` and its own `.prop(…)` calls, stopping
//!   before `.child(`. Each node therefore has a distinct range even when the file is one line.
//! * **Offsets are bytes.** The literal is found by a byte search and the parentheses are balanced
//!   byte by byte, so a file whose earlier text holds multi-byte characters still slices on a
//!   character boundary.
//! * **A key that is not in the source is no entry.** A node the generator refused has no span, and
//!   the map answers `None` — it does not guess, and the caller keeps its second way to locate.
//! * **Relocation is a rebuild, not an offset guess.** [`relocate`] finds the keys again in the new
//!   text; a formatter that reflowed a line moves the span to where the construct actually is,
//!   which a running offset could not know.

#![allow(dead_code)]

use crate::contracts::identity::NodeId;
use crate::contracts::revision::{GeneratorVersion, Revision};
use crate::contracts::source_map::{SourceMap, SourceSpan};

/// Builds a source map by locating each node's key literal in `source`.
///
/// `keys` is [`super::document::node_keys`]: each node's id and the key the generator will write.
/// A key not found in `source` (a node the generator refused, or a scan of the wrong file) is left
/// out rather than mapped to a guessed span.
pub fn build(
    file: &str,
    source: &str,
    generator: GeneratorVersion,
    model: Revision,
    keys: &[(NodeId, String)],
) -> SourceMap {
    build_from_sources(&[(file, source)], generator, model, keys)
}

/// Builds a source map across several generated files, such as one module per page.
///
/// A key is looked for in each file in order; the first file that carries it owns the node's span.
/// Keys emitted into different modules therefore map into different files, and a key present in no
/// file stays unmapped rather than being attached to a guessed location.
pub fn build_from_sources(
    sources: &[(&str, &str)],
    generator: GeneratorVersion,
    model: Revision,
    keys: &[(NodeId, String)],
) -> SourceMap {
    let mut map = SourceMap::new(generator, model);
    for (node, key) in keys {
        for (file, source) in sources {
            if let Some(span) = span_of_key(file, source, key) {
                map.insert(*node, span);
                break;
            }
        }
    }
    map
}

/// Builds a map from the generated document itself.
///
/// Generation is the only party that knows the order it emitted keys in, so when `keys` is empty
/// the key literals are read out of `source` in that order. Without a page there is no stable
/// `NodeId` to recover, so the derived ids are the source order (`1`, `2`, …); a caller that holds
/// the page's ids should pass [`super::document::node_keys`] instead and keep identity stable across
/// an insertion.
pub fn mapping_from_source(
    file: &str,
    source: &str,
    generator: GeneratorVersion,
    model: Revision,
    keys: &[(NodeId, String)],
) -> SourceMap {
    if keys.is_empty() {
        let derived: Vec<(NodeId, String)> = keys_in_source(source)
            .into_iter()
            .enumerate()
            .map(|(index, key)| (NodeId::from_raw(index as u64 + 1), key))
            .collect();
        build(file, source, generator, model, &derived)
    } else {
        build(file, source, generator, model, keys)
    }
}

/// Every key literal in a generated document, in the order the generator emitted them.
///
/// This is the generated document's own answer to "which nodes are here", independent of any
/// caller-supplied `key:` list. A key is only returned when its literal is well-formed;
/// `.key("unterminated` is skipped rather than guessed at.
pub fn keys_in_source(source: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let mut cursor = 0;
    while let Some(found) = key_literal(source, cursor) {
        keys.push(found.key);
        cursor = found.call_end;
    }
    keys
}

/// Rebuilds a map against a reformatted or edited source, keeping its stamp.
///
/// A formatter moves bytes; the node a key literal belongs to does not change. Finding the keys
/// again is therefore the correct correction, where adding a running offset would be a guess that
/// drifts with every reflow.
pub fn relocate(
    file: &str,
    source: &str,
    previous: &SourceMap,
    keys: &[(NodeId, String)],
) -> SourceMap {
    build(
        file,
        source,
        previous.generator(),
        previous.stamp().model,
        keys,
    )
}

/// One `.key("…")` literal and the byte range of its call.
struct KeyLiteral {
    /// The key text between the quotes.
    key: String,
    /// The first byte of the `.key(` call.
    call_start: usize,
    /// One past the `)` of the call.
    call_end: usize,
}

/// The first well-formed `.key("…")` call at or after `from`.
fn key_literal(source: &str, from: usize) -> Option<KeyLiteral> {
    let rest = &source[from..];
    let call_start = from + rest.find(".key(\"")?;
    let content_start = call_start + ".key(\"".len();
    let bytes = source.as_bytes();
    let mut cursor = content_start;
    let mut escaped = false;
    while cursor < bytes.len() {
        let byte = bytes[cursor];
        if escaped {
            escaped = false;
        } else if byte == b'\\' {
            escaped = true;
        } else if byte == b'"' {
            break;
        }
        cursor += 1;
    }
    if cursor >= bytes.len() || bytes.get(cursor + 1) != Some(&b')') {
        return None;
    }
    Some(KeyLiteral {
        key: source[content_start..cursor].to_owned(),
        call_start,
        call_end: cursor + 2,
    })
}

/// The span of the builder call that carries `.key("<key>")`, or `None` when it is not there.
///
/// The range starts at the node's `Node::new(` when that can be identified in the same statement
/// and ends after the node's own `.key(…)`/`.prop(…)` calls, before `.child(`. This is what keeps
/// two nodes on one generated line from collapsing onto the same span.
fn span_of_key(file: &str, source: &str, key: &str) -> Option<SourceSpan> {
    let mut cursor = 0;
    while let Some(found) = key_literal(source, cursor) {
        if found.key == key {
            let start = builder_start(source, found.call_start);
            let end = builder_end(source, found.call_end);
            return SourceSpan::new(file, start as u32, end as u32);
        }
        cursor = found.call_end;
    }
    None
}

/// The nearest `Node::new(` before `key_start`, provided no statement boundary intervenes.
///
/// When the call cannot be identified the literal itself is the span: a smaller true range is
/// better than a larger range that claims to include code it does not.
fn builder_start(source: &str, key_start: usize) -> usize {
    match source[..key_start].rfind("Node::new(") {
        Some(start) if !source[start..key_start].contains(';') => start,
        _ => key_start,
    }
}

/// One past the node's own builder chain: `.prop(…)` calls, but never a `.child(` subtree.
fn builder_end(source: &str, key_end: usize) -> usize {
    let mut cursor = key_end;
    loop {
        let rest = &source[cursor..];
        let at = cursor + (rest.len() - rest.trim_start().len());
        if !source[at..].starts_with(".prop(") {
            return cursor;
        }
        match call_end(source, at + ".prop".len()) {
            Some(end) => cursor = end,
            None => return cursor,
        }
    }
}

/// One past the `)` that closes the call whose `(` is at `open`, honouring string literals.
fn call_end(source: &str, open: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    if bytes.get(open) != Some(&b'(') {
        return None;
    }
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    let mut cursor = open;
    while cursor < bytes.len() {
        let byte = bytes[cursor];
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
        } else {
            match byte {
                b'"' => in_string = true,
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(cursor + 1);
                    }
                }
                _ => {}
            }
        }
        cursor += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{build, build_from_sources, keys_in_source, mapping_from_source, relocate};
    use crate::contracts::identity::NodeId;
    use crate::contracts::revision::{GeneratorVersion, Revision, Staleness};

    const SOURCE: &str = "// Generated by the rust_widgets designer. Do not edit by hand.\n\
        pub fn build_ui() {\n\
        \x20   let tree = Node::new(\"window\").key(\"root\")\n\
        \x20       .child(Node::new(\"button\").key(\"n_0\"));\n\
        }\n";

    const ONE_LINE: &str = concat!(
        "pub fn build_ui() { let tree = Node::new(\"window\")",
        ".key(\"root\").prop(\"title\", CapabilityValue::String(String::from(\"Probe\")))",
        ".child(Node::new(\"button\").key(\"n_0\").prop(\"text\", CapabilityValue::String(String::from(\"Go\")))); }\n",
    );

    fn keys() -> Vec<(NodeId, String)> {
        vec![
            (NodeId::from_raw(1), "root".to_owned()),
            (NodeId::from_raw(2), "n_0".to_owned()),
        ]
    }

    #[test]
    fn a_node_maps_to_the_line_that_carries_its_key() {
        let map = build(
            "src/generated/ui.rs",
            SOURCE,
            GeneratorVersion::CURRENT,
            Revision::from_raw(3),
            &keys(),
        );
        assert_eq!(map.len(), 2);
        let root = map.span_of(NodeId::from_raw(1)).expect("a span");
        assert_eq!(root.file(), "src/generated/ui.rs");
        assert!(SOURCE[root.start() as usize..root.end() as usize].contains(".key(\"root\")"));
        let button = map.span_of(NodeId::from_raw(2)).expect("a span");
        assert!(SOURCE[button.start() as usize..button.end() as usize].contains(".key(\"n_0\")"));
        // The two lines are distinct.
        assert_ne!(root, button);
    }

    #[test]
    fn two_nodes_on_one_line_have_distinct_builder_spans() {
        let map = build(
            "src/generated/ui.rs",
            ONE_LINE,
            GeneratorVersion::CURRENT,
            Revision::INITIAL,
            &keys(),
        );
        let root = map.span_of(NodeId::from_raw(1)).expect("a root span");
        let button = map.span_of(NodeId::from_raw(2)).expect("a button span");

        let root_text = &ONE_LINE[root.start() as usize..root.end() as usize];
        let button_text = &ONE_LINE[button.start() as usize..button.end() as usize];
        assert_ne!(root, button, "one line must not collapse the two nodes");
        assert!(
            root_text.starts_with("Node::new(\"window\")"),
            "{root_text:?}"
        );
        assert!(root_text.contains(".key(\"root\")"), "{root_text:?}");
        assert!(
            !root_text.contains(".key(\"n_0\")"),
            "the parent's span must stop before its child: {root_text:?}"
        );
        assert!(
            button_text.starts_with("Node::new(\"button\")"),
            "{button_text:?}"
        );
        assert!(button_text.contains(".key(\"n_0\")"), "{button_text:?}");
        assert!(
            root.end() <= button.start(),
            "the parent's span ends before the child's"
        );
    }

    #[test]
    fn a_unicode_prefix_does_not_shift_a_node_span() {
        let source = concat!(
            "// 你好，世界\n",
            "pub fn build_ui() { let tree = Node::new(\"window\").key(\"root\")",
            ".prop(\"title\", CapabilityValue::String(String::from(\"保存 (ok)\")))",
            ".child(Node::new(\"button\").key(\"n_0\")); }\n",
        );
        let map = build(
            "src/generated/ui.rs",
            source,
            GeneratorVersion::CURRENT,
            Revision::INITIAL,
            &keys(),
        );
        let button = map.span_of(NodeId::from_raw(2)).expect("a span");
        assert!(source.is_char_boundary(button.start() as usize));
        assert!(source.is_char_boundary(button.end() as usize));
        assert_eq!(
            &source[button.start() as usize..button.end() as usize],
            "Node::new(\"button\").key(\"n_0\")"
        );
    }

    #[test]
    fn keys_are_read_from_the_generated_document() {
        assert_eq!(keys_in_source(ONE_LINE), vec!["root", "n_0"]);

        let map = mapping_from_source(
            "src/generated/ui.rs",
            ONE_LINE,
            GeneratorVersion::CURRENT,
            Revision::INITIAL,
            &[],
        );
        assert_eq!(map.len(), 2);
        let first = map.span_of(NodeId::from_raw(1)).expect("the root");
        let second = map.span_of(NodeId::from_raw(2)).expect("the child");
        assert_ne!(first, second);
    }

    #[test]
    fn two_modules_map_their_own_files() {
        let root_only = "pub fn build_ui() { let tree = Node::new(\"window\").key(\"root\"); }\n";
        let child_only = "pub fn build_more() { let tree = Node::new(\"button\").key(\"n_0\"); }\n";
        let map = build_from_sources(
            &[
                ("src/generated/ui.rs", root_only),
                ("src/generated/more.rs", child_only),
            ],
            GeneratorVersion::CURRENT,
            Revision::INITIAL,
            &keys(),
        );
        assert_eq!(
            map.span_of(NodeId::from_raw(1)).expect("the root").file(),
            "src/generated/ui.rs"
        );
        assert_eq!(
            map.span_of(NodeId::from_raw(2)).expect("the child").file(),
            "src/generated/more.rs"
        );
    }

    #[test]
    fn the_map_carries_the_generator_and_the_revision_it_came_from() {
        let map = build(
            "src/ui.rs",
            SOURCE,
            GeneratorVersion::new(1, 2, 3),
            Revision::from_raw(4),
            &keys(),
        );
        assert_eq!(map.generator(), GeneratorVersion::new(1, 2, 3));
        assert_eq!(map.staleness(Revision::from_raw(4)), Staleness::Current);
        assert!(!map.staleness(Revision::from_raw(9)).is_current());
    }

    #[test]
    fn a_node_the_generator_refused_has_no_span() {
        let mut with_extra = keys();
        with_extra.push((NodeId::from_raw(3), "n_1".to_owned()));
        let map = build(
            "src/ui.rs",
            SOURCE,
            GeneratorVersion::CURRENT,
            Revision::INITIAL,
            &with_extra,
        );
        assert_eq!(map.len(), 2);
        assert_eq!(map.span_of(NodeId::from_raw(3)), None);
    }

    #[test]
    fn a_reformatted_source_relocates_rather_than_drifting() {
        let map = build(
            "src/ui.rs",
            SOURCE,
            GeneratorVersion::CURRENT,
            Revision::INITIAL,
            &keys(),
        );
        let formatted = SOURCE.replace("let tree", "\n    // a comment was inserted\n    let tree");
        let moved = relocate("src/ui.rs", &formatted, &map, &keys());

        let old = map.span_of(NodeId::from_raw(2)).expect("a span");
        let new = moved.span_of(NodeId::from_raw(2)).expect("a span");
        assert!(new.start() > old.start(), "the button moved down a line");
        assert!(
            formatted[new.start() as usize..new.end() as usize].contains(".key(\"n_0\")"),
            "the relocated span still points at the button"
        );
    }

    #[test]
    fn a_node_at_a_byte_inside_a_span_resolves_to_the_deepest_node() {
        let map = build(
            "src/ui.rs",
            SOURCE,
            GeneratorVersion::CURRENT,
            Revision::INITIAL,
            &keys(),
        );
        let button = map.span_of(NodeId::from_raw(2)).expect("a span");
        assert_eq!(
            map.node_at("src/ui.rs", button.start()),
            Some(NodeId::from_raw(2))
        );
    }
}
