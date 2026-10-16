// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Regenerating without overwriting what a person wrote (task T15.04).
//!
//! The acceptance line is *"a small change does not wholesale-overwrite hand-written logic"*. The
//! merge is built so that sentence is a property of the code rather than a hope:
//!
//! * **Hand-written bytes are copied, never re-emitted.** Every [`RegionKind::Handwritten`] region of
//!   the committed file is carried into the result verbatim. The emitter never produces those bytes,
//!   so a regeneration cannot change what is inside one.
//! * **A region goes back where its marker is, or to the end.** When the regenerated body carries a
//!   marker of the same name, the region is placed [`Placement::InPlace`] there; when it does not, it
//!   is appended and reported [`Placement::Orphaned`] — losing a person's code is worse than a file
//!   with a tail.
//! * **The generated diff is shown, not applied blindly.** [`merge`] returns a [`Review`] of the
//!   generated lines that changed, so a user approving a regeneration sees what it does to the
//!   generated half without reading the whole file (principle 16).

#![allow(dead_code)]

use std::collections::BTreeMap;

use super::artifact::{RegionKind, regions};

/// Where a carried region ended up in the merged file.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Placement {
    /// It went back under the same-named marker it came from.
    InPlace,
    /// The regenerated body had no marker for it, so it is appended after the body.
    Orphaned,
}

impl Placement {
    /// The placement in one line.
    pub fn render(&self) -> &'static str {
        match self {
            Self::InPlace => "in place",
            Self::Orphaned => "appended (no marker in the regenerated body)",
        }
    }
}

/// A hand-written region the merge carried, and where it went.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Carried {
    /// The region's name.
    pub name: String,
    /// Where it was placed.
    pub placement: Placement,
    /// The bytes carried, without the markers.
    pub body: String,
}

/// One line of the generated half that changed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Change {
    /// A line the regeneration added.
    Added(String),
    /// A line the regeneration removed.
    Removed(String),
}

impl Change {
    /// The change in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Added(line) => format!("+ {line}"),
            Self::Removed(line) => format!("- {line}"),
        }
    }
}

/// What a regeneration did, in a form a person can approve.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Review {
    /// The hand-written regions carried, in the order they were placed.
    pub carried: Vec<Carried>,
    /// The generated lines that changed.
    pub changes: Vec<Change>,
}

impl Review {
    /// The review in one line per fact.
    pub fn render(&self) -> Vec<String> {
        let mut out = Vec::new();
        for carried in &self.carried {
            out.push(format!(
                "kept {} ({})",
                carried.name,
                carried.placement.render()
            ));
        }
        for change in &self.changes {
            out.push(change.render());
        }
        if out.is_empty() {
            out.push("no change".to_owned());
        }
        out
    }
}

/// The merged file and what the merge did.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Merged {
    /// The file to write.
    pub text: String,
    /// What the merge carried and changed.
    pub review: Review,
}

/// The most lines the review will diff; beyond it a change is summarised rather than enumerated.
///
/// A diff is quadratic in the number of lines, and a generated file that large cannot be reviewed by
/// eye anyway. The bound keeps an accidental large file from turning a merge into a long pause, and
/// the review says so instead of silently hiding the change.
pub const REVIEW_LINE_LIMIT: usize = 4000;

/// Merges a freshly generated body into the file that is already committed.
///
/// `previous` is the committed file (marker, generated body and any hand-written regions);
/// `regenerated` is what the generator just produced — normally just the generated body, optionally
/// with empty placeholders for the hand-written regions that belong in it.
pub fn merge(previous: &str, regenerated: &str) -> Result<Merged, String> {
    let old = regions(previous)?;
    let new = regions(regenerated)?;

    // The hand-written bodies the committed file holds, by name.
    let mut previous_bodies: BTreeMap<String, String> = BTreeMap::new();
    for region in &old {
        if region.kind == RegionKind::Handwritten && !region.name.is_empty() {
            previous_bodies.insert(region.name.clone(), region.content(previous).to_owned());
        }
    }

    let mut text = String::new();
    let mut carried = Vec::new();
    let mut placed: Vec<String> = Vec::new();

    for region in &new {
        match region.kind {
            RegionKind::Generated => text.push_str(region.content(regenerated)),
            RegionKind::Handwritten => {
                let name = region.name.clone();
                let (body, placement) = match previous_bodies.get(&name) {
                    Some(body) => (body.clone(), Placement::InPlace),
                    // A placeholder with no committed body: keep whatever it carries (normally
                    // nothing) so the merged file still has the boundary a person can fill in.
                    None => (region.content(regenerated).to_owned(), Placement::InPlace),
                };
                push_region(&mut text, &name, &body);
                carried.push(Carried {
                    name: name.clone(),
                    placement,
                    body,
                });
                placed.push(name);
            }
        }
    }

    // Any committed region the regenerated body had no marker for: appended, never dropped.
    let mut tail = String::new();
    for region in &old {
        if region.kind != RegionKind::Handwritten || region.name.is_empty() {
            continue;
        }
        if placed.contains(&region.name) {
            continue;
        }
        let body = region.content(previous).to_owned();
        push_region(&mut tail, &region.name, &body);
        carried.push(Carried {
            name: region.name.clone(),
            placement: Placement::Orphaned,
            body,
        });
    }

    if !text.ends_with('\n') && !text.is_empty() {
        text.push('\n');
    }
    text.push_str(&tail);

    Ok(Merged {
        review: Review {
            carried,
            changes: diff(generated_lines(previous), generated_lines(&text)),
        },
        text,
    })
}

/// Emits one hand-written region: its markers and its body.
fn push_region(out: &mut String, name: &str, body: &str) {
    out.push_str(&format!("// region:handwritten {name}\n"));
    out.push_str(body);
    if !body.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&format!("// endregion:handwritten {name}\n"));
}

/// The generated (non-hand-written) lines of a file, in order.
fn generated_lines(text: &str) -> Vec<&str> {
    match regions(text) {
        Ok(parsed) => parsed
            .iter()
            .filter(|region| region.kind == RegionKind::Generated)
            .flat_map(|region| region.content(text).lines())
            .collect(),
        Err(_) => text.lines().collect(),
    }
}

/// A line diff, or a single summary change when the files are too large to diff by eye.
fn diff(old: Vec<&str>, new: Vec<&str>) -> Vec<Change> {
    if old.len() > REVIEW_LINE_LIMIT || new.len() > REVIEW_LINE_LIMIT {
        return vec![Change::Added(format!(
            "({} lines regenerated; too large to enumerate)",
            new.len()
        ))];
    }
    let n = old.len();
    let m = new.len();
    let mut lcs = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if old[i] == new[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let mut changes = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if old[i] == new[j] {
            i += 1;
            j += 1;
        } else if lcs[i + 1][j] >= lcs[i][j + 1] {
            changes.push(Change::Removed(old[i].to_owned()));
            i += 1;
        } else {
            changes.push(Change::Added(new[j].to_owned()));
            j += 1;
        }
    }
    for line in &old[i..] {
        changes.push(Change::Removed((*line).to_owned()));
    }
    for line in &new[j..] {
        changes.push(Change::Added((*line).to_owned()));
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::{Change, Placement, merge};

    const OLD: &str = "// Generated by the rust_widgets designer. Do not edit by hand.\n\
        pub fn build_ui() {\n\
        \x20   let a = 1;\n\
        }\n\
        // region:handwritten save\n\
        fn save() { /* mine */ }\n\
        // endregion:handwritten save\n";

    #[test]
    fn a_handwritten_region_survives_a_regeneration() {
        let regenerated = "// Generated by the rust_widgets designer. Do not edit by hand.\n\
            pub fn build_ui() {\n\
            \x20   let a = 2;\n\
            }\n";
        let merged = merge(OLD, regenerated).expect("a merge");

        assert!(merged.text.contains("fn save() { /* mine */ }"));
        assert!(
            merged.text.contains("let a = 2;"),
            "the generated half is the new one"
        );
        assert_eq!(merged.review.carried.len(), 1);
        assert_eq!(merged.review.carried[0].placement, Placement::Orphaned);
    }

    #[test]
    fn the_review_names_the_generated_line_that_changed() {
        let regenerated = "// Generated by the rust_widgets designer. Do not edit by hand.\n\
            pub fn build_ui() {\n\
            \x20   let a = 2;\n\
            }\n";
        let merged = merge(OLD, regenerated).expect("a merge");
        assert!(
            merged
                .review
                .changes
                .iter()
                .any(|change| matches!(change, Change::Added(line) if line.contains("let a = 2;"))),
            "{:?}",
            merged.review.render(),
        );
        assert!(
            merged.review.changes.iter().any(
                |change| matches!(change, Change::Removed(line) if line.contains("let a = 1;"))
            ),
        );
    }

    #[test]
    fn a_region_with_a_marker_in_the_new_body_goes_back_in_place() {
        let regenerated = "// Generated by the rust_widgets designer. Do not edit by hand.\n\
            pub fn build_ui() {}\n\
            // region:handwritten save\n\
            // endregion:handwritten save\n";
        let merged = merge(OLD, regenerated).expect("a merge");
        assert_eq!(merged.review.carried[0].placement, Placement::InPlace);
        assert!(merged.review.carried[0].body.contains("fn save()"));
        assert!(merged.text.contains("fn save() { /* mine */ }"));
        // The region sits between the two generated halves, not appended at the very end.
        let body = merged.text.find("fn build_ui").unwrap();
        let hand = merged.text.find("fn save()").unwrap();
        assert!(body < hand);
    }

    #[test]
    fn a_merge_with_nothing_to_carry_keeps_the_generated_half() {
        let regenerated = "// Generated by the rust_widgets designer. Do not edit by hand.\n\
            pub fn build_ui() {}\n";
        let merged = merge(regenerated, regenerated).expect("a merge");
        assert!(merged.review.carried.is_empty());
        assert_eq!(merged.text, regenerated);
    }
}
