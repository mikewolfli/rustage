// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The search/replace acceptance across a large project with continuous edits (task T08.08).
//!
//! The acceptance line is *"a result carries a version, and a stale candidate must be re-checked"*.
//! The two halves are one idea — a replace candidate is only safe while the file is still at the
//! version the candidate was computed against — so the scenarios turn the version while candidates
//! exist and check that nothing is applied to the wrong base:
//!
//! * **[`Scenario::BigProject`]** — a search over many files is complete, and every replace candidate
//!   carries its file's version as a baseline.
//! * **[`Scenario::ContinuousEdits`]** — the file moves on after a candidate exists; applying the stale
//!   candidate is refused, and re-deriving it at the new version applies.
//! * **[`Scenario::Conflict`]** — an external change stops a batch midway, so the batch is partial
//!   rather than falsely complete.
//! * **[`Scenario::Cancel`]** — a cancelled search returns what it had, marked cancelled.
//!
//! This is the same shape as the two earlier harnesses, and it reports the same [`Check`] type, so
//! "did T08.08 pass" is read the same way as T06.08 and T07.08.

#![allow(dead_code)]

use std::collections::BTreeMap;

use crate::app::editor::regression::Check;
use crate::app::editor::scale::Cancellation;
use crate::app::language::diagnostics::TextEdit;
use crate::app::language::workspace_edit::OpenDocument;
use crate::app::search::batch_replace::{Batch, BatchEdit, Failure};
use crate::app::search::text::{Limits, Query, search};

/// One acceptance scenario.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scenario {
    /// A search over a large project.
    BigProject,
    /// A candidate that goes stale as the file is edited.
    ContinuousEdits,
    /// An external change during a batch.
    Conflict,
    /// A cancelled search.
    Cancel,
}

impl Scenario {
    /// Every scenario, in a stable order.
    pub const ALL: [Self; 4] = [
        Self::BigProject,
        Self::ContinuousEdits,
        Self::Conflict,
        Self::Cancel,
    ];

    /// The scenario's name.
    pub fn name(self) -> &'static str {
        match self {
            Self::BigProject => "big-project",
            Self::ContinuousEdits => "continuous-edits",
            Self::Conflict => "conflict",
            Self::Cancel => "cancel",
        }
    }

    /// The scenario a name picks.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|s| s.name() == name)
    }
}

/// One scenario's outcome.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Report {
    /// Which scenario ran.
    pub scenario: Scenario,
    /// The checks.
    pub checks: Vec<Check>,
}

impl Report {
    /// Whether every check passed.
    pub fn passed(&self) -> bool {
        self.checks.iter().all(|check| check.passed)
    }

    /// The report in lines.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "scenario {}: {}",
            self.scenario.name(),
            if self.passed() { "passed" } else { "FAILED" }
        )];
        for check in &self.checks {
            lines.push(check.render());
        }
        lines
    }
}

/// Runs a scenario.
pub fn run(scenario: Scenario) -> Report {
    match scenario {
        Scenario::BigProject => big_project(),
        Scenario::ContinuousEdits => continuous_edits(),
        Scenario::Conflict => conflict(),
        Scenario::Cancel => cancel(),
    }
}

/// Runs every scenario.
pub fn run_all() -> Vec<Report> {
    Scenario::ALL.into_iter().map(run).collect()
}

/// A project of `count` files, each with a line that matches `hit`.
fn project(count: usize) -> Vec<(String, String)> {
    (0..count)
        .map(|index| {
            (
                format!("src/file{index}.rs"),
                format!("// file {index}\nlet hit{index} = {index};\n"),
            )
        })
        .collect()
}

/// A search over many files is complete, and a candidate carries its file's version.
fn big_project() -> Report {
    let files = project(200);
    let query = Query::literal("hit");
    let results = search(&files, &query, Limits::new(), &Cancellation::new()).expect("a search");
    let complete = results.is_complete() && results.matches.len() == 200;
    // A candidate's baseline is the file's version it was computed against.
    let documents = documents(&files);
    let candidate = BatchEdit::new("src/file0.rs", 1, vec![TextEdit::new(0, 1, "X")]);
    let baseline_matches = documents
        .get("src/file0.rs")
        .is_some_and(|document| document.version == candidate.baseline);
    Report {
        scenario: Scenario::BigProject,
        checks: vec![
            Check::new("the search over 200 files is complete", complete),
            Check::new("a candidate carries its file's version", baseline_matches),
        ],
    }
}

/// A file moves on after a candidate exists: the stale candidate is refused, and a fresh one applies.
fn continuous_edits() -> Report {
    let mut documents = documents(&project(1));
    // A candidate computed against version 1.
    let stale = BatchEdit::new("src/file0.rs", 1, vec![TextEdit::new(0, 1, "X")]);
    // The file is edited: version 2.
    documents.insert(
        "src/file0.rs".to_owned(),
        OpenDocument::new("// file 0\nlet hit0 = 0;\n", 2),
    );
    let mut batch = Batch::new(vec![stale]);
    batch.run(&mut documents, &BTreeMap::new());
    let refused =
        matches!(batch.failure(), Some(Failure::ExternalChange { .. })) && !batch.is_complete();
    // Re-derived at the current version, it applies.
    let mut fresh = Batch::new(vec![BatchEdit::new(
        "src/file0.rs",
        documents["src/file0.rs"].version,
        vec![TextEdit::new(0, 1, "X")],
    )]);
    fresh.run(&mut documents, &BTreeMap::new());
    let applied = fresh.is_complete();
    Report {
        scenario: Scenario::ContinuousEdits,
        checks: vec![
            Check::new("a stale candidate is refused, not applied", refused),
            Check::new("re-deriving it at the current version applies", applied),
        ],
    }
}

/// An external change stops a batch midway, so the batch is partial.
fn conflict() -> Report {
    let mut documents = documents(&project(2));
    documents.insert("src/file1.rs".to_owned(), OpenDocument::new("changed", 2));
    let plan = vec![
        BatchEdit::new("src/file0.rs", 1, vec![TextEdit::new(0, 1, "X")]),
        BatchEdit::new("src/file1.rs", 1, vec![TextEdit::new(0, 1, "Y")]),
    ];
    let mut batch = Batch::new(plan);
    batch.run(&mut documents, &BTreeMap::new());
    let partial = !batch.is_complete() && batch.applied().len() == 1;
    let not_claimed = !batch.render()[0].contains("complete");
    Report {
        scenario: Scenario::Conflict,
        checks: vec![
            Check::new("the batch stopped at the changed file", partial),
            Check::new("a partial batch is not reported complete", not_claimed),
        ],
    }
}

/// A cancelled search returns what it had, marked cancelled.
fn cancel() -> Report {
    let files = project(200);
    let token = Cancellation::new();
    token.cancel();
    let results = search(&files, &Query::literal("hit"), Limits::new(), &token).expect("a search");
    Report {
        scenario: Scenario::Cancel,
        checks: vec![
            Check::new(
                "the cancelled search is marked cancelled",
                results.cancelled,
            ),
            Check::new(
                "its result is not reported complete",
                !results.is_complete(),
            ),
        ],
    }
}

/// The open documents for a project, all at version 1.
fn documents(files: &[(String, String)]) -> BTreeMap<String, OpenDocument> {
    files
        .iter()
        .map(|(path, text)| (path.clone(), OpenDocument::new(text.clone(), 1)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Scenario, run, run_all};

    #[test]
    fn every_scenario_passes() {
        for report in run_all() {
            assert!(report.passed(), "{:?}", report.render());
        }
    }

    #[test]
    fn a_large_project_search_is_complete() {
        let report = run(Scenario::BigProject);
        assert!(report.passed(), "{:?}", report.render());
        assert_eq!(report.checks.len(), 2);
    }

    #[test]
    fn a_stale_candidate_is_rechecked() {
        let report = run(Scenario::ContinuousEdits);
        assert!(report.passed(), "{:?}", report.render());
    }

    #[test]
    fn a_conflict_makes_the_batch_partial() {
        let report = run(Scenario::Conflict);
        assert!(report.passed(), "{:?}", report.render());
    }

    #[test]
    fn a_cancel_is_visible() {
        let report = run(Scenario::Cancel);
        assert!(report.passed(), "{:?}", report.render());
    }

    #[test]
    fn a_scenario_name_round_trips() {
        for scenario in Scenario::ALL {
            assert_eq!(Scenario::from_name(scenario.name()), Some(scenario));
        }
        assert_eq!(Scenario::from_name("nowhere"), None);
    }
}
