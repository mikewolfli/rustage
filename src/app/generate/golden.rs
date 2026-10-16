// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Pinning generation down with golden comparisons (task T15.08).
//!
//! The acceptance line is *"a conflict loses nothing, and independent compilation and run semantics
//! are re-checked in T16.08"*. This module is the part of that which lives in the generator: a set of
//! recorded outputs compared against what generation produces now, so a change to the emitter is
//! caught as a diff rather than noticed later.
//!
//! Three golden families cover the failure modes the milestone cares about:
//!
//! * **Dual-mode snapshots.** A file per target — the default template and the stripped one — so the
//!   two modes are compared as text, and a change to one that should not touch the other shows up.
//! * **Semantics.** The nodes and properties a generated file *reads back to* are compared, so two
//!   sources that differ in whitespace but agree on meaning do not fail, and two that agree on text
//!   but disagree on meaning cannot hide.
//! * **Hand-written and external-edit goldens.** A file with a carried hand-written region, and one
//!   changed outside the designer, are both recorded, so the recovery those paths promise is pinned.

#![allow(dead_code)]

use super::reverse;

/// A recorded output: a name and the bytes it is expected to be.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Golden {
    /// The golden's name, such as `ui_default.rs`.
    pub name: String,
    /// The expected content.
    pub expected: String,
}

/// The verdict of one comparison.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Verdict {
    /// The produced content is exactly the golden.
    Match,
    /// The golden was not produced at all.
    Missing,
    /// The produced content differs, with the first differing lines named.
    Drift {
        /// The first golden line that differs, if any.
        expected: Option<String>,
        /// The first produced line that differs, if any.
        produced: Option<String>,
    },
}

/// One golden compared against what was produced.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Comparison {
    /// The golden's name.
    pub name: String,
    /// What the comparison found.
    pub verdict: Verdict,
}

impl Comparison {
    /// Whether the comparison passed.
    pub fn is_match(&self) -> bool {
        matches!(self.verdict, Verdict::Match)
    }

    /// The comparison in one line.
    pub fn render(&self) -> String {
        match &self.verdict {
            Verdict::Match => format!("match   {}", self.name),
            Verdict::Missing => format!("missing {}", self.name),
            Verdict::Drift { expected, produced } => format!(
                "drift   {}: expected {:?}, produced {:?}",
                self.name,
                expected.as_deref().unwrap_or("<nothing>"),
                produced.as_deref().unwrap_or("<nothing>"),
            ),
        }
    }
}

/// Compares one golden against the content produced for it.
pub fn compare(golden: &Golden, produced: &str) -> Comparison {
    if golden.expected == produced {
        return Comparison {
            name: golden.name.clone(),
            verdict: Verdict::Match,
        };
    }
    let expected_line = first_difference(golden.expected.lines(), produced.lines()).0;
    let produced_line = first_difference(produced.lines(), golden.expected.lines()).0;
    Comparison {
        name: golden.name.clone(),
        verdict: Verdict::Drift {
            expected: expected_line,
            produced: produced_line,
        },
    }
}

/// The first line of `left` not present at the same position in `right`, plus its index.
fn first_difference<'a>(
    left: impl Iterator<Item = &'a str>,
    right: impl Iterator<Item = &'a str>,
) -> (Option<String>, Option<usize>) {
    for (index, (a, b)) in left.zip(right).enumerate() {
        if a != b {
            return (Some(a.to_owned()), Some(index));
        }
    }
    (None, None)
}

/// Runs a set of goldens against the produced artifacts, in golden order.
pub fn check(goldens: &[Golden], produced: &[(String, String)]) -> Vec<Comparison> {
    goldens
        .iter()
        .map(
            |golden| match produced.iter().find(|(name, _)| name == &golden.name) {
                Some((_, content)) => compare(golden, content),
                None => Comparison {
                    name: golden.name.clone(),
                    verdict: Verdict::Missing,
                },
            },
        )
        .collect()
}

/// The semantics a generated source describes: the property values it reads back to.
///
/// Two sources with the same semantics describe the same design; this is what the dual-mode check
/// compares so whitespace does not fail it and a real difference cannot hide behind it.
pub fn semantics(source: &str) -> Vec<String> {
    reverse::read("", source)
        .recognized
        .iter()
        .map(reverse::Change::render)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Golden, Verdict, check, compare, semantics};

    fn golden(name: &str, expected: &str) -> Golden {
        Golden {
            name: name.to_owned(),
            expected: expected.to_owned(),
        }
    }

    #[test]
    fn an_identical_output_matches() {
        let g = golden("ui_default.rs", "fn a() {}\n");
        assert_eq!(compare(&g, "fn a() {}\n").verdict, Verdict::Match);
    }

    #[test]
    fn a_changed_line_is_a_drift_with_both_sides_named() {
        let g = golden("ui_default.rs", "let a = 1;\n");
        let comparison = compare(&g, "let a = 2;\n");
        assert!(!comparison.is_match());
        assert!(comparison.render().contains("let a = 1;"));
        assert!(comparison.render().contains("let a = 2;"));
    }

    #[test]
    fn a_golden_that_was_not_produced_is_missing() {
        let results = check(&[golden("ui_stripped.rs", "x")], &[]);
        assert_eq!(results[0].verdict, Verdict::Missing);
    }

    #[test]
    fn both_targets_are_compared_so_one_template_cannot_change_alone() {
        let goldens = vec![
            golden("ui_default.rs", "a\n"),
            golden("ui_stripped.rs", "b\n"),
        ];
        let produced = vec![
            ("ui_default.rs".to_owned(), "a\n".to_owned()),
            ("ui_stripped.rs".to_owned(), "c\n".to_owned()),
        ];
        let results = check(&goldens, &produced);
        assert!(results[0].is_match());
        assert!(!results[1].is_match());
    }

    #[test]
    fn semantics_ignore_whitespace_and_keep_meaning() {
        let a = "Node::new(\"button\").key(\"n_0\").prop(\"text\", \"Go\");\n";
        let b = "  Node::new(\"button\")  .key(\"n_0\")  .prop(\"text\", \"Go\");  \n";
        assert_eq!(semantics(a), semantics(b));
        let c = "Node::new(\"button\").key(\"n_0\").prop(\"text\", \"Stop\");\n";
        assert_ne!(semantics(a), semantics(c));
    }
}
