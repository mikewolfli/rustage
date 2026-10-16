// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The generate → check → build → run → feedback walk (task T16.08).
//!
//! This is the milestone's exit demonstration as a value: one [`Report`] that records what each stage
//! did, so "the generated project builds and runs on its own" is a sequence of observations rather
//! than a claim.
//!
//! * **Every stage reports, including the ones that were skipped.** A stage that did not run because
//!   an earlier one failed is [`Outcome::Skipped`], never a silent success — the difference between
//!   "the run worked" and "the run never happened" is exactly what a false completion hides.
//! * **The feedback stage closes the loop.** [`Stage::Feedback`] carries the problems a build
//!   reported back to the design, so the walk does not stop at "it built"; it says whether a person
//!   can get from a compiler error to the node that caused it.
//! * **The first blocking stage is named.** [`Report::blocked_at`] answers which stage stopped the
//!   walk, which is what a caller needs to decide whether to retry.

#![allow(dead_code)]

/// One stage of the walk.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stage {
    /// The design was turned into source.
    Generate,
    /// Cargo checked the source.
    Check,
    /// Cargo built the source.
    Build,
    /// The built program ran on its own.
    Run,
    /// Build problems were mapped back to the design.
    Feedback,
}

impl Stage {
    /// The stage's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Generate => "generate",
            Self::Check => "check",
            Self::Build => "build",
            Self::Run => "run",
            Self::Feedback => "feedback",
        }
    }

    /// Every stage, in walk order.
    pub fn all() -> [Stage; 5] {
        [
            Self::Generate,
            Self::Check,
            Self::Build,
            Self::Run,
            Self::Feedback,
        ]
    }
}

/// What one stage came to.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Outcome {
    /// The stage succeeded.
    Ok,
    /// The stage failed, with a reason.
    Failed(String),
    /// The stage did not run because an earlier one blocked it.
    Skipped,
}

impl Outcome {
    /// The outcome in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Ok => "ok".to_owned(),
            Self::Failed(reason) => format!("failed: {reason}"),
            Self::Skipped => "skipped (an earlier stage blocked it)".to_owned(),
        }
    }

    /// Whether this outcome is a success.
    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Ok)
    }
}

/// One stage's entry in the walk.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Step {
    /// Which stage.
    pub stage: Stage,
    /// What it came to.
    pub outcome: Outcome,
    /// A note, such as the binary that ran or the number of problems mapped.
    pub note: String,
}

/// The whole walk, in order.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Report {
    /// The steps taken.
    pub steps: Vec<Step>,
}

impl Report {
    /// An empty report.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a step that ran.
    pub fn record(&mut self, stage: Stage, outcome: Outcome, note: impl Into<String>) {
        self.steps.push(Step {
            stage,
            outcome,
            note: note.into(),
        });
    }

    /// Fills in every stage not yet recorded as [`Outcome::Skipped`], in walk order.
    pub fn skip_missing(&mut self) {
        for stage in Stage::all() {
            if !self.steps.iter().any(|step| step.stage == stage) {
                self.steps.push(Step {
                    stage,
                    outcome: Outcome::Skipped,
                    note: String::new(),
                });
            }
        }
        self.steps.sort_by_key(|step| {
            Stage::all()
                .iter()
                .position(|stage| *stage == step.stage)
                .unwrap_or(usize::MAX)
        });
    }

    /// Whether every stage succeeded.
    pub fn is_success(&self) -> bool {
        !self.steps.is_empty() && self.steps.iter().all(|step| step.outcome.is_ok())
    }

    /// The first stage that failed, if any.
    pub fn blocked_at(&self) -> Option<Stage> {
        self.steps
            .iter()
            .find(|step| matches!(step.outcome, Outcome::Failed(_)))
            .map(|step| step.stage)
    }

    /// The report in one line per step.
    pub fn render(&self) -> Vec<String> {
        self.steps
            .iter()
            .map(|step| {
                let note = if step.note.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", step.note)
                };
                format!("{}{note}: {}", step.stage.name(), step.outcome.render())
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{Outcome, Report, Stage};

    #[test]
    fn a_full_walk_is_a_success() {
        let mut report = Report::new();
        for stage in Stage::all() {
            report.record(stage, Outcome::Ok, "");
        }
        assert!(report.is_success());
        assert_eq!(report.blocked_at(), None);
        assert_eq!(report.render().len(), 5);
    }

    #[test]
    fn a_failed_stage_names_where_the_walk_stopped() {
        let mut report = Report::new();
        report.record(Stage::Generate, Outcome::Ok, "");
        report.record(Stage::Check, Outcome::Failed("E0308".to_owned()), "");
        report.skip_missing();
        assert!(!report.is_success());
        assert_eq!(report.blocked_at(), Some(Stage::Check));
        // The stages after the failure are recorded as skipped, never silently missing.
        assert!(
            report
                .steps
                .iter()
                .any(|step| step.stage == Stage::Build && step.outcome == Outcome::Skipped)
        );
    }

    #[test]
    fn a_stage_that_did_not_run_is_not_a_success() {
        let mut report = Report::new();
        report.record(Stage::Generate, Outcome::Ok, "2 files");
        report.skip_missing();
        assert!(!report.is_success());
    }

    #[test]
    fn the_steps_are_in_walk_order_even_when_recorded_out_of_order() {
        let mut report = Report::new();
        report.record(Stage::Feedback, Outcome::Ok, "1 problem mapped");
        report.record(Stage::Generate, Outcome::Ok, "");
        report.skip_missing();
        assert_eq!(report.steps[0].stage, Stage::Generate);
        assert_eq!(report.steps[4].stage, Stage::Feedback);
    }
}
