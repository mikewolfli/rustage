// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! A design document's structural diff, and the generated/hand-written split of a code diff
//! (roadmap task T20.02).
//!
//! A text diff of a design document answers "which bytes moved", which is not the question a person
//! asks. This module answers the two that are:
//!
//! * **What changed in the design.** [`compare`] walks two parsed documents and reports the path of
//!   each added, removed or changed value (`$.pages[1].title`), so a moved property reads as itself
//!   rather than as forty changed lines. Paths are structural and deterministic.
//! * **Which half changed in the code.** [`split_code`] reads the library's own region markers in the
//!   committed and working text and reports, per region, whether the *generated* body or a named
//!   **hand-written** region differs. That is the difference between "the generator changed this" and
//!   "a person changed this", and it is the one a reviewer needs before approving a regeneration.
//!
//! Neither function writes anything, runs Git, or guesses: a document that is not JSON is refused by
//! the parser before it gets here, and a code file whose markers do not balance is refused by
//! [`regions`] rather than half-split.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use crate::app::generate::artifact::{RegionKind, regions};
use crate::project::record::json::Json;

/// What happened to one path in a design document.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Change {
    /// The path exists only in the new document.
    Added,
    /// The path exists only in the old document.
    Removed,
    /// The path exists in both with different values.
    Changed,
}

impl Change {
    /// The change in one word.
    pub fn name(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Removed => "removed",
            Self::Changed => "changed",
        }
    }
}

/// One structural difference.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Difference {
    /// The path, from `$` down.
    pub path: String,
    /// What happened.
    pub change: Change,
    /// The old value, for a removal or a change.
    pub before: Option<String>,
    /// The new value, for an addition or a change.
    pub after: Option<String>,
}

impl Difference {
    /// The difference in one line.
    pub fn render(&self) -> String {
        match (self.change, &self.before, &self.after) {
            (Change::Added, _, Some(after)) => format!("added {}: {after}", self.path),
            (Change::Removed, Some(before), _) => format!("removed {}: {before}", self.path),
            (Change::Changed, Some(before), Some(after)) => {
                format!("changed {}: {before} -> {after}", self.path)
            }
            // `compare` never builds these shapes; a line still has to exist if one appears.
            _ => format!("{} {}", self.change.name(), self.path),
        }
    }
}

/// The structural differences between two documents, in path order.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct DesignDiff {
    /// The differences, in the order the walk found them.
    pub differences: Vec<Difference>,
}

impl DesignDiff {
    /// Whether the two documents are structurally equal.
    pub fn is_empty(&self) -> bool {
        self.differences.is_empty()
    }

    /// How many of each change happened.
    pub fn counts(&self) -> (usize, usize, usize) {
        let count = |change: Change| {
            self.differences
                .iter()
                .filter(|difference| difference.change == change)
                .count()
        };
        (
            count(Change::Added),
            count(Change::Removed),
            count(Change::Changed),
        )
    }

    /// The diff in lines, ended by a summary line.
    pub fn render(&self) -> Vec<String> {
        let mut lines: Vec<String> = self.differences.iter().map(Difference::render).collect();
        let (added, removed, changed) = self.counts();
        lines.push(format!(
            "design diff: {added} added, {removed} removed, {changed} changed"
        ));
        lines
    }
}

/// Compares two parsed design documents.
///
/// Objects compare by member name and arrays by index, so an element inserted at the front of an
/// array shows up as the changed elements rather than one opaque array change. A value that changes
/// type is a change at its own path.
pub fn compare(before: &Json, after: &Json) -> DesignDiff {
    let mut differences = Vec::new();
    walk("$", before, after, &mut differences);
    DesignDiff { differences }
}

fn walk(path: &str, before: &Json, after: &Json, out: &mut Vec<Difference>) {
    if before == after {
        return;
    }
    match (before, after) {
        (Json::Object(before), Json::Object(after)) => {
            let names: BTreeSet<&str> = before.names().chain(after.names()).collect();
            for name in names {
                let child = format!("{path}.{name}");
                match (before.get(name), after.get(name)) {
                    (Some(old), None) => out.push(Difference {
                        path: child,
                        change: Change::Removed,
                        before: Some(old.render_compact()),
                        after: None,
                    }),
                    (None, Some(new)) => out.push(Difference {
                        path: child,
                        change: Change::Added,
                        before: None,
                        after: Some(new.render_compact()),
                    }),
                    (Some(old), Some(new)) => walk(&child, old, new, out),
                    (None, None) => {}
                }
            }
        }
        (Json::Array(before), Json::Array(after)) => {
            for index in 0..before.len().max(after.len()) {
                let child = format!("{path}[{index}]");
                match (before.get(index), after.get(index)) {
                    (Some(old), None) => out.push(Difference {
                        path: child,
                        change: Change::Removed,
                        before: Some(old.render_compact()),
                        after: None,
                    }),
                    (None, Some(new)) => out.push(Difference {
                        path: child,
                        change: Change::Added,
                        before: None,
                        after: Some(new.render_compact()),
                    }),
                    (Some(old), Some(new)) => walk(&child, old, new, out),
                    (None, None) => {}
                }
            }
        }
        _ => out.push(Difference {
            path: path.to_owned(),
            change: Change::Changed,
            before: Some(before.render_compact()),
            after: Some(after.render_compact()),
        }),
    }
}

/// Which half of a code file a region belongs to.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Side {
    /// The generated body (the unnamed region).
    Generated,
    /// A named hand-written region.
    Handwritten(String),
}

impl Side {
    /// The side as a report writes it.
    pub fn name(&self) -> String {
        match self {
            Self::Generated => "generated".to_owned(),
            Self::Handwritten(name) => format!("handwritten {name}"),
        }
    }
}

/// One region compared between the two versions.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PartChange {
    /// Which half it is.
    pub side: Side,
    /// Whether its content differs.
    pub changed: bool,
    /// Whether it is only in the new version.
    pub added: bool,
    /// Whether it is only in the old version.
    pub removed: bool,
}

impl PartChange {
    /// The part in one line.
    pub fn render(&self) -> String {
        let state = if self.added {
            "added"
        } else if self.removed {
            "removed"
        } else if self.changed {
            "changed"
        } else {
            "unchanged"
        };
        format!("{}: {state}", self.side.name())
    }
}

/// The generated/hand-written split of a code diff.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct CodeDiff {
    /// One entry per region, in marker order.
    pub parts: Vec<PartChange>,
}

impl CodeDiff {
    /// The hand-written regions that differ.
    pub fn handwritten_changed(&self) -> Vec<String> {
        self.parts
            .iter()
            .filter(|part| {
                matches!(part.side, Side::Handwritten(_))
                    && (part.changed || part.added || part.removed)
            })
            .map(|part| part.side.name())
            .collect()
    }

    /// Whether the generated body differs.
    pub fn generated_changed(&self) -> bool {
        self.parts.iter().any(|part| {
            part.side == Side::Generated && (part.changed || part.added || part.removed)
        })
    }

    /// The diff in lines, ended by a summary line.
    pub fn render(&self) -> Vec<String> {
        let mut lines: Vec<String> = self.parts.iter().map(PartChange::render).collect();
        let handwritten = self.handwritten_changed().len();
        lines.push(format!(
            "code diff: generated {}, {handwritten} hand-written region(s) changed",
            if self.generated_changed() {
                "changed"
            } else {
                "unchanged"
            }
        ));
        lines
    }
}

/// Splits a committed file and its working copy by their region markers and compares each half.
///
/// The marker structure comes from the library ([`regions`]); a file whose markers do not balance is
/// refused rather than partly split, because attributing half a file to the wrong owner is worse than
/// saying the file cannot be attributed.
pub fn split_code(committed: &str, working: &str) -> Result<CodeDiff, String> {
    let before = bodies(committed)?;
    let after = bodies(working)?;
    let sides: BTreeSet<Side> = before.keys().chain(after.keys()).cloned().collect();
    let mut parts = Vec::new();
    for side in sides {
        match (before.get(&side), after.get(&side)) {
            (Some(old), Some(new)) => parts.push(PartChange {
                side,
                changed: old != new,
                added: false,
                removed: false,
            }),
            (None, Some(_)) => parts.push(PartChange {
                side,
                changed: false,
                added: true,
                removed: false,
            }),
            (Some(_), None) => parts.push(PartChange {
                side,
                changed: false,
                added: false,
                removed: true,
            }),
            (None, None) => {}
        }
    }
    Ok(CodeDiff { parts })
}

/// Each region's body, keyed by side.
fn bodies(text: &str) -> Result<BTreeMap<Side, String>, String> {
    let mut out: BTreeMap<Side, String> = BTreeMap::new();
    for region in regions(text)? {
        let start = region.start as usize;
        let end = (region.end as usize).min(text.len());
        if start > end {
            return Err(format!(
                "the region {:?} ends before it starts",
                region.name
            ));
        }
        let body = text[start..end].to_owned();
        let side = match region.kind {
            RegionKind::Generated => Side::Generated,
            RegionKind::Handwritten => Side::Handwritten(region.name),
        };
        // A file can hold more than one generated region (text after the last hand-written region
        // is generated too), so the bodies of one side are joined rather than overwriting each
        // other.
        match out.entry(side) {
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                entry.get_mut().push('\n');
                entry.get_mut().push_str(&body);
            }
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(body);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{Change, Side, compare, split_code};
    use crate::project::record::json::parse;

    fn document(text: &str) -> crate::project::record::json::Json {
        parse(text).expect("a document")
    }

    const OLD: &str = "// Generated by the rust_widgets designer. Do not edit by hand.\n\
// region:generated\n\
root\n\
// endregion:generated\n\
// region:handwritten after_build\n\
fn after_build() {}\n\
// endregion:handwritten after_build\n";

    const NEW: &str = "// Generated by the rust_widgets designer. Do not edit by hand.\n\
// region:generated\n\
root + button\n\
// endregion:generated\n\
// region:handwritten after_build\n\
fn after_build() { println!(\"hi\") }\n\
// endregion:handwritten after_build\n";

    #[test]
    fn an_added_removed_and_changed_value_are_named_by_their_path() {
        let before = document(r#"{"title":"a","pages":[{"name":"home"}],"tags":["x"]}"#);
        let after =
            document(r#"{"title":"b","pages":[{"name":"home"},{"name":"settings"}],"other":1}"#);
        let diff = compare(&before, &after);
        let rendered = diff.render().join("\n");
        assert!(
            rendered.contains("changed $.title: \"a\" -> \"b\""),
            "{rendered}"
        );
        assert!(
            rendered.contains("added $.pages[1]: {\"name\":\"settings\"}"),
            "{rendered}"
        );
        assert!(rendered.contains("removed $.tags"), "{rendered}");
        assert!(rendered.contains("added $.other"), "{rendered}");
        assert_eq!(diff.counts(), (2, 1, 1));
        assert!(
            diff.differences
                .iter()
                .any(|difference| difference.change == Change::Changed)
        );
    }

    #[test]
    fn identical_documents_have_no_differences() {
        let text = r#"{"a":[1,2,3]}"#;
        let diff = compare(&document(text), &document(text));
        assert!(diff.is_empty());
        assert_eq!(
            diff.render(),
            vec!["design diff: 0 added, 0 removed, 0 changed"]
        );
    }

    #[test]
    fn an_array_change_is_per_index_not_one_opaque_value() {
        let before = document(r#"{"items":["a","b"]}"#);
        let after = document(r#"{"items":["a","b","c"]}"#);
        let diff = compare(&before, &after);
        assert_eq!(diff.differences.len(), 1, "{diff:?}");
        assert_eq!(diff.differences[0].path, "$.items[2]");
        assert_eq!(diff.differences[0].change, Change::Added);
    }

    #[test]
    fn a_change_of_type_is_a_change_at_its_own_path() {
        let before = document(r#"{"size":3}"#);
        let after = document(r#"{"size":"three"}"#);
        let diff = compare(&before, &after);
        assert_eq!(diff.differences.len(), 1, "{diff:?}");
        assert_eq!(diff.differences[0].path, "$.size");
        assert_eq!(diff.differences[0].before.as_deref(), Some("3"));
        assert_eq!(diff.differences[0].after.as_deref(), Some("\"three\""));
    }

    #[test]
    fn the_code_diff_says_which_half_changed() {
        let diff = split_code(OLD, NEW).expect("a split");
        assert!(diff.generated_changed());
        assert_eq!(
            diff.handwritten_changed(),
            vec!["handwritten after_build".to_owned()]
        );
        let rendered = diff.render().join("\n");
        assert!(rendered.contains("generated: changed"), "{rendered}");
        assert!(
            rendered.contains("handwritten after_build: changed"),
            "{rendered}"
        );
        assert!(
            rendered.contains("1 hand-written region(s) changed"),
            "{rendered}"
        );
    }

    #[test]
    fn a_region_only_in_one_side_is_added_or_removed() {
        let shorter = OLD.replace(
            "// region:handwritten after_build\nfn after_build() {}\n\
             // endregion:handwritten after_build\n",
            "",
        );
        let diff = split_code(&shorter, OLD).expect("a split");
        let part = diff
            .parts
            .iter()
            .find(|part| part.side == Side::Handwritten("after_build".to_owned()))
            .expect("the region");
        assert!(part.added && !part.removed);
        assert!(diff.render().join("\n").contains("added"));
    }

    #[test]
    fn balanced_markers_are_required_rather_than_half_split() {
        let broken = "// Generated by the rust_widgets designer. Do not edit by hand.\n\
// region:generated\nroot\n";
        assert!(split_code(OLD, broken).is_err());
    }
}
