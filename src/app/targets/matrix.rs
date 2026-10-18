// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The desktop multi-OS build/behaviour matrix and its independent artifacts (task T18.03).
//!
//! The acceptance line is *"each committed combination has real build and run evidence"*. The design
//! makes a false matrix success impossible:
//!
//! * **A combination is committed or it is not in the matrix.** [`committed`] is the list of desktop
//!   triples this milestone promises; a triple that is not in it cannot be started, so a run for
//!   another machine cannot be counted towards this one.
//! * **Every combination has its own artifact.** [`Combination::artifact`] is a distinct name for
//!   each triple, so one build cannot stand in for another by writing the same path.
//! * **Success needs a run, not a build.** [`Matrix::is_success`] requires every combination to
//!   reach [`Proof::Ran`]; a build that produced a binary nobody executed is a [`Proof::Built`], and
//!   [`Matrix::unverified`] names it instead of counting it.
//! * **A failure is visible in the whole.** One failed or cancelled combination makes
//!   [`Matrix::is_success`] false even when every other combination passed — a single-target failure
//!   must not be absorbed into a green matrix.

#![allow(dead_code)]

use super::configuration::Os;

/// One committed desktop combination.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Combination {
    /// The system it runs on.
    pub os: Os,
    /// The target triple.
    pub triple: String,
    /// The artifact this combination alone writes.
    pub artifact: String,
}

impl Combination {
    /// A combination of one system, one triple and one artifact.
    pub fn new(os: Os, triple: impl Into<String>, artifact: impl Into<String>) -> Self {
        Self {
            os,
            triple: triple.into(),
            artifact: artifact.into(),
        }
    }
}

/// The desktop combinations this milestone promises.
///
/// Four rather than three: macOS is committed on both Apple architectures, because one build does
/// not prove the other and the matrix exists to keep that difference visible.
pub fn committed() -> Vec<Combination> {
    vec![
        Combination::new(
            Os::Macos,
            "aarch64-apple-darwin",
            "rustage-desktop-macos-aarch64",
        ),
        Combination::new(
            Os::Macos,
            "x86_64-apple-darwin",
            "rustage-desktop-macos-x86_64",
        ),
        Combination::new(
            Os::Linux,
            "x86_64-unknown-linux-gnu",
            "rustage-desktop-linux-x86_64",
        ),
        Combination::new(
            Os::Windows,
            "x86_64-pc-windows-msvc",
            "rustage-desktop-windows-x86_64.exe",
        ),
    ]
}

/// How far a combination has been verified.
///
/// Ordered from the weakest evidence to the strongest: a proof level is what a *stage* reached, and
/// [`Proof::at_least`] is how the matrix asks whether the level it needs was reached.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Proof {
    /// Source was generated for the combination.
    Generated,
    /// Cargo checked the source.
    Checked,
    /// Cargo built the artifact.
    Built,
    /// The artifact ran.
    Ran,
    /// The artifact ran on the device the combination names.
    Device,
}

impl Proof {
    /// The proof's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Generated => "generated",
            Self::Checked => "checked",
            Self::Built => "built",
            Self::Ran => "ran",
            Self::Device => "device",
        }
    }

    /// Reads a proof name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "generated" => Some(Self::Generated),
            "checked" => Some(Self::Checked),
            "built" => Some(Self::Built),
            "ran" => Some(Self::Ran),
            "device" => Some(Self::Device),
            _ => None,
        }
    }

    /// Whether this proof is at least `required`.
    pub fn at_least(&self, required: Proof) -> bool {
        *self >= required
    }
}

/// What one combination is doing.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum State {
    /// It has not started.
    Pending,
    /// It is building or running.
    Running,
    /// It reached a proof level.
    Passed(Proof),
    /// It failed, with the reason.
    Failed(String),
    /// It was cancelled.
    Cancelled,
}

impl State {
    /// The state in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Pending => "pending".to_owned(),
            Self::Running => "running".to_owned(),
            Self::Passed(proof) => format!("passed {}", proof.name()),
            Self::Failed(reason) => format!("failed: {reason}"),
            Self::Cancelled => "cancelled".to_owned(),
        }
    }

    /// Whether this state is a failure rather than unfinished work.
    pub fn is_failure(&self) -> bool {
        matches!(self, Self::Failed(_) | Self::Cancelled)
    }
}

/// One combination's entry in the matrix.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Job {
    /// The combination.
    pub combination: Combination,
    /// What it is doing.
    pub state: State,
    /// A note, such as which binary ran.
    pub note: String,
}

/// The matrix: one job per committed combination.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Matrix {
    jobs: Vec<Job>,
}

impl Matrix {
    /// The committed matrix, all pending.
    pub fn plan() -> Self {
        Self {
            jobs: committed()
                .into_iter()
                .map(|combination| Job {
                    combination,
                    state: State::Pending,
                    note: String::new(),
                })
                .collect(),
        }
    }

    /// Every job, in committed order.
    pub fn jobs(&self) -> &[Job] {
        &self.jobs
    }

    /// One job by triple.
    pub fn job(&self, triple: &str) -> Option<&Job> {
        self.jobs
            .iter()
            .find(|job| job.combination.triple == triple)
    }

    fn job_mut(&mut self, triple: &str) -> Result<&mut Job, String> {
        self.jobs
            .iter_mut()
            .find(|job| job.combination.triple == triple)
            .ok_or_else(|| format!("{triple} is not a committed desktop combination"))
    }

    /// Starts a combination.
    pub fn start(&mut self, triple: &str) -> Result<(), String> {
        let job = self.job_mut(triple)?;
        if matches!(job.state, State::Passed(_)) {
            return Err(format!("{triple} already passed; a pass is not restarted"));
        }
        job.state = State::Running;
        Ok(())
    }

    /// Records that a combination reached `proof`.
    pub fn pass(
        &mut self,
        triple: &str,
        proof: Proof,
        note: impl Into<String>,
    ) -> Result<(), String> {
        let job = self.job_mut(triple)?;
        job.state = State::Passed(proof);
        job.note = note.into();
        Ok(())
    }

    /// Records that a combination failed.
    pub fn fail(&mut self, triple: &str, reason: impl Into<String>) -> Result<(), String> {
        let job = self.job_mut(triple)?;
        job.state = State::Failed(reason.into());
        job.note.clear();
        Ok(())
    }

    /// Records that a combination was cancelled.
    pub fn cancel(&mut self, triple: &str) -> Result<(), String> {
        let job = self.job_mut(triple)?;
        job.state = State::Cancelled;
        job.note.clear();
        Ok(())
    }

    /// How many combinations reached at least a run.
    pub fn passed(&self) -> usize {
        self.jobs
            .iter()
            .filter(|job| matches!(job.state, State::Passed(proof) if proof.at_least(Proof::Ran)))
            .count()
    }

    /// Whether every combination has stopped in a terminal state.
    pub fn is_complete(&self) -> bool {
        self.jobs
            .iter()
            .all(|job| !matches!(job.state, State::Pending | State::Running))
    }

    /// Whether every committed combination has a real build **and run**.
    ///
    /// `false` is the honest answer for a matrix with one failed combination, one cancelled
    /// combination, or one that only reached a build.
    pub fn is_success(&self) -> bool {
        self.jobs
            .iter()
            .all(|job| matches!(job.state, State::Passed(proof) if proof.at_least(Proof::Ran)))
    }

    /// The combinations that are not a passing run, with their state.
    pub fn unverified(&self) -> Vec<String> {
        self.jobs
            .iter()
            .filter(|job| !matches!(job.state, State::Passed(proof) if proof.at_least(Proof::Ran)))
            .map(|job| format!("{}: {}", job.combination.triple, job.state.render()))
            .collect()
    }

    /// The artifacts a combination actually produced, with the proof they reached.
    ///
    /// Only a combination that reached a build has an artifact; a failed or pending one has none,
    /// which is what keeps a matrix report from naming a binary that was never written.
    pub fn artifacts(&self) -> Vec<(&Combination, Proof)> {
        self.jobs
            .iter()
            .filter_map(|job| match job.state {
                State::Passed(proof) if proof.at_least(Proof::Built) => {
                    Some((&job.combination, proof))
                }
                _ => None,
            })
            .collect()
    }

    /// The matrix in one line per combination.
    pub fn render(&self) -> Vec<String> {
        self.jobs
            .iter()
            .map(|job| {
                let note = if job.note.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", job.note)
                };
                format!(
                    "{} {}: {}{note}",
                    job.combination.os.name(),
                    job.combination.triple,
                    job.state.render()
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{Matrix, Proof, State, committed};

    #[test]
    fn the_committed_matrix_has_four_independent_combinations() {
        let combinations = committed();
        assert_eq!(combinations.len(), 4);
        let mut triples: Vec<&str> = combinations
            .iter()
            .map(|combination| combination.triple.as_str())
            .collect();
        triples.sort_unstable();
        triples.dedup();
        assert_eq!(triples.len(), 4);
        let mut artifacts: Vec<&str> = combinations
            .iter()
            .map(|combination| combination.artifact.as_str())
            .collect();
        artifacts.sort_unstable();
        artifacts.dedup();
        assert_eq!(artifacts.len(), 4, "each combination owns its artifact");
    }

    #[test]
    fn a_fresh_matrix_is_not_a_success_and_names_every_gap() {
        let matrix = Matrix::plan();
        assert!(!matrix.is_success());
        assert!(!matrix.is_complete());
        assert_eq!(matrix.unverified().len(), 4);
        assert!(matrix.artifacts().is_empty());
    }

    #[test]
    fn a_build_without_a_run_stays_unverified() {
        let mut matrix = Matrix::plan();
        matrix
            .pass("aarch64-apple-darwin", Proof::Built, "target/debug/rustage")
            .expect("a committed triple");
        assert_eq!(matrix.artifacts().len(), 1);
        assert!(!matrix.is_success());
        assert!(
            matrix
                .unverified()
                .iter()
                .any(|line| line.contains("passed built"))
        );
    }

    #[test]
    fn a_single_failure_keeps_the_whole_matrix_from_succeeding() {
        let mut matrix = Matrix::plan();
        for triple in [
            "aarch64-apple-darwin",
            "x86_64-apple-darwin",
            "x86_64-unknown-linux-gnu",
        ] {
            matrix
                .pass(triple, Proof::Ran, "ran")
                .expect("a committed triple");
        }
        matrix
            .fail("x86_64-pc-windows-msvc", "the SDK is missing")
            .expect("a committed triple");
        assert!(matrix.is_complete());
        assert!(!matrix.is_success());
        assert_eq!(matrix.passed(), 3);
        assert!(
            matrix
                .unverified()
                .iter()
                .any(|line| line.contains("the SDK is missing"))
        );
    }

    #[test]
    fn a_cancelled_combination_is_not_absorbed_either() {
        let mut matrix = Matrix::plan();
        for triple in [
            "aarch64-apple-darwin",
            "x86_64-apple-darwin",
            "x86_64-pc-windows-msvc",
        ] {
            matrix
                .pass(triple, Proof::Ran, "ran")
                .expect("a committed triple");
        }
        matrix
            .cancel("x86_64-unknown-linux-gnu")
            .expect("a committed triple");
        assert!(!matrix.is_success());
        assert_eq!(
            matrix.job("x86_64-unknown-linux-gnu").expect("a job").state,
            State::Cancelled
        );
    }

    #[test]
    fn every_combination_at_a_run_or_better_is_a_success() {
        let mut matrix = Matrix::plan();
        for (index, job) in committed().into_iter().enumerate() {
            let proof = if index == 0 {
                Proof::Device
            } else {
                Proof::Ran
            };
            matrix
                .pass(&job.triple, proof, "ran")
                .expect("a committed triple");
        }
        assert!(matrix.is_success());
        assert!(matrix.is_complete());
        assert_eq!(matrix.passed(), 4);
        assert!(matrix.unverified().is_empty());
        assert!(matrix.artifacts().len() == 4);
    }

    #[test]
    fn an_uncommitted_triple_is_refused() {
        let mut matrix = Matrix::plan();
        assert!(matrix.start("riscv64gc-unknown-linux-gnu").is_err());
        assert!(
            matrix
                .pass("riscv64gc-unknown-linux-gnu", Proof::Ran, "")
                .is_err()
        );
        assert!(matrix.fail("riscv64gc-unknown-linux-gnu", "").is_err());
    }

    #[test]
    fn a_pass_is_not_restarted() {
        let mut matrix = Matrix::plan();
        matrix
            .pass("aarch64-apple-darwin", Proof::Ran, "")
            .expect("a committed triple");
        assert!(matrix.start("aarch64-apple-darwin").is_err());
    }

    #[test]
    fn a_proof_orders_from_generation_to_the_device() {
        assert!(Proof::Ran.at_least(Proof::Built));
        assert!(!Proof::Built.at_least(Proof::Ran));
        assert_eq!(Proof::parse("device"), Some(Proof::Device));
        assert_eq!(Proof::parse("somehow"), None);
    }

    #[test]
    fn the_matrix_renders_each_combination_and_its_note() {
        let mut matrix = Matrix::plan();
        matrix
            .pass(
                "aarch64-apple-darwin",
                Proof::Ran,
                "target/debug/rustage --version",
            )
            .expect("a committed triple");
        let lines = matrix.render();
        assert_eq!(lines.len(), 4);
        assert!(lines[0].contains("macos aarch64-apple-darwin: passed ran"));
        assert!(lines[0].contains("--version"));
    }
}
