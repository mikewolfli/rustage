// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Debouncing edits into one incremental build, and admitting only an honest candidate that does not
//! overwrite the running program (task T17.04).
//!
//! The acceptance line is *"the running binary is not overwritten, and a `check` does not count as a
//! candidate"*:
//!
//! * **A candidate comes from a build, never from a check.** [`Candidate::from_build`] refuses a
//!   [`BuildAction::Check`] with [`Refusal::CheckIsNotACandidate`], because a check proves the code
//!   compiles but produces no runnable artifact.
//! * **A candidate is stamped.** A build whose stamp is not the current revision is refused with
//!   [`Refusal::Stale`], so an artifact from behind the edits it is meant to replace cannot be started
//!   (the same rule the design scene and the preview service use for a stale result).
//! * **A candidate cannot replace the running program.** [`Candidate::may_replace`] refuses a candidate
//!   whose path is the running program's, so the file backing a live process is never written under it.
//! * **A burst is one build.** [`Debounce`] coalesces the sources that edit inside its window into one
//!   batch, so a fast series of edits does not start one build per keystroke.

#![allow(dead_code)]

use crate::contracts::revision::{Revision, VersionStamp};

/// What a build was asked to do.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BuildAction {
    /// Type-check only; produces no artifact.
    Check,
    /// Build the logic.
    Build,
    /// Rebuild the logic from scratch.
    Rebuild,
}

impl BuildAction {
    /// The action's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Check => "check",
            Self::Build => "build",
            Self::Rebuild => "rebuild",
        }
    }

    /// Reads an action's name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "check" => Some(Self::Check),
            "build" => Some(Self::Build),
            "rebuild" => Some(Self::Rebuild),
            _ => None,
        }
    }

    /// Whether the action produces a runnable artifact.
    pub fn produces_artifact(&self) -> bool {
        !matches!(self, Self::Check)
    }
}

/// How a build ended.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    /// It succeeded.
    Success,
    /// It failed.
    Failure,
}

impl Outcome {
    /// The outcome's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
        }
    }

    /// Reads an outcome's name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "success" | "ok" => Some(Self::Success),
            "failure" | "fail" => Some(Self::Failure),
            _ => None,
        }
    }
}

/// A produced artifact and the version it was built from.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Artifact {
    /// Where it is.
    pub path: String,
    /// What it was built from.
    pub stamp: VersionStamp,
}

/// One finished build.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Build {
    /// What was asked.
    pub action: BuildAction,
    /// How it ended.
    pub outcome: Outcome,
    /// What it produced, if anything.
    pub artifact: Option<Artifact>,
    /// What it was built from.
    pub stamp: VersionStamp,
}

/// An artifact that may be started to replace a running one.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Candidate {
    /// Where the artifact is.
    pub path: String,
    /// What it was built from.
    pub stamp: VersionStamp,
}

impl Candidate {
    /// Admits a build's artifact as a candidate for the current revision.
    pub fn from_build(build: &Build, current: Revision) -> Result<Self, Refusal> {
        if build.action == BuildAction::Check {
            return Err(Refusal::CheckIsNotACandidate);
        }
        if build.outcome == Outcome::Failure {
            return Err(Refusal::BuildFailed {
                action: build.action.name().to_owned(),
            });
        }
        if !build.stamp.is_current_for(current) {
            return Err(Refusal::Stale {
                built_at: build.stamp.model,
                current,
            });
        }
        let artifact = build.artifact.clone().ok_or_else(|| Refusal::NoArtifact {
            action: build.action.name().to_owned(),
        })?;
        Ok(Self {
            path: artifact.path,
            stamp: artifact.stamp,
        })
    }

    /// Checks the candidate may replace what is running.
    pub fn may_replace(&self, running: &str) -> Result<(), Refusal> {
        if self.path == running {
            return Err(Refusal::WouldOverwriteRunning {
                path: running.to_owned(),
            });
        }
        Ok(())
    }
}

/// Why a build's artifact was not admitted.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Refusal {
    /// A check produces no candidate.
    CheckIsNotACandidate,
    /// The build failed.
    BuildFailed {
        /// What was asked.
        action: String,
    },
    /// The build is behind the current revision.
    Stale {
        /// The revision it was built at.
        built_at: Revision,
        /// The current revision.
        current: Revision,
    },
    /// The build produced nothing that can be started.
    NoArtifact {
        /// What was asked.
        action: String,
    },
    /// The candidate would overwrite the running program.
    WouldOverwriteRunning {
        /// The running program's path.
        path: String,
    },
}

impl Refusal {
    /// The refusal in one line.
    pub fn render(&self) -> String {
        match self {
            Self::CheckIsNotACandidate => {
                "a check produces no candidate; build the logic instead".to_owned()
            }
            Self::BuildFailed { action } => format!("the {action} failed; there is no candidate"),
            Self::Stale { built_at, current } => format!(
                "the build is from {built_at} and the document is at {current}; it is not a candidate"
            ),
            Self::NoArtifact { action } => format!("the {action} produced no artifact"),
            Self::WouldOverwriteRunning { path } => format!(
                "starting {path} would overwrite the program that is running; build beside it first"
            ),
        }
    }
}

/// Coalesces the sources that edit inside a window into one build batch.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Debounce {
    /// How long a burst stays open, in the caller's unit.
    pub window: u32,
    opened: Option<u32>,
    pending: Vec<String>,
}

impl Debounce {
    /// A debouncer with a window.
    pub fn new(window: u32) -> Self {
        Self {
            window,
            opened: None,
            pending: Vec::new(),
        }
    }

    /// Records an edit from a source at time `now`.
    pub fn push(&mut self, source: impl Into<String>, now: u32) {
        if self.opened.is_none() {
            self.opened = Some(now);
        }
        let source = source.into();
        if !self.pending.contains(&source) {
            self.pending.push(source);
        }
    }

    /// How many sources are waiting.
    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    /// Coalesces the pending sources once the window has passed, or answers `None`.
    pub fn flush(&mut self, now: u32) -> Option<Vec<String>> {
        if self.pending.is_empty() {
            return None;
        }
        if self
            .opened
            .is_some_and(|opened| now.saturating_sub(opened) < self.window)
        {
            return None;
        }
        self.opened = None;
        Some(std::mem::take(&mut self.pending))
    }
}

#[cfg(test)]
mod tests {
    use super::{Artifact, Build, BuildAction, Candidate, Debounce, Outcome, Refusal};
    use crate::contracts::revision::{Revision, VersionStamp};

    fn stamp(revision: u64) -> VersionStamp {
        VersionStamp::current(Revision::from_raw(revision))
    }

    fn successful(action: BuildAction, path: &str) -> Build {
        Build {
            action,
            outcome: Outcome::Success,
            artifact: Some(Artifact {
                path: path.to_owned(),
                stamp: stamp(7),
            }),
            stamp: stamp(7),
        }
    }

    #[test]
    fn a_check_is_not_a_candidate() {
        let build = successful(BuildAction::Check, "target/debug/logic");
        let refusal = Candidate::from_build(&build, Revision::from_raw(7)).expect_err("a refusal");
        assert_eq!(refusal, Refusal::CheckIsNotACandidate);
        assert!(refusal.render().contains("check produces no candidate"));
    }

    #[test]
    fn a_failed_build_is_not_a_candidate() {
        let mut build = successful(BuildAction::Build, "target/debug/logic");
        build.outcome = Outcome::Failure;
        assert!(matches!(
            Candidate::from_build(&build, Revision::from_raw(7)),
            Err(Refusal::BuildFailed { .. })
        ));
    }

    #[test]
    fn a_stale_build_is_not_a_candidate() {
        let build = successful(BuildAction::Build, "target/debug/logic");
        let refusal = Candidate::from_build(&build, Revision::from_raw(8)).expect_err("a refusal");
        assert_eq!(
            refusal,
            Refusal::Stale {
                built_at: Revision::from_raw(7),
                current: Revision::from_raw(8),
            }
        );
        assert!(refusal.render().contains("document is at r8"));
    }

    #[test]
    fn a_successful_current_build_is_a_candidate() {
        let build = successful(BuildAction::Build, "target/debug/logic-2");
        let candidate = Candidate::from_build(&build, Revision::from_raw(7)).expect("a candidate");
        assert_eq!(candidate.path, "target/debug/logic-2");
    }

    #[test]
    fn a_candidate_may_not_replace_the_running_program() {
        let build = successful(BuildAction::Build, "target/debug/logic");
        let candidate = Candidate::from_build(&build, Revision::from_raw(7)).expect("a candidate");
        let refusal = candidate
            .may_replace("target/debug/logic")
            .expect_err("a refusal");
        assert_eq!(
            refusal,
            Refusal::WouldOverwriteRunning {
                path: "target/debug/logic".to_owned(),
            }
        );
        assert!(candidate.may_replace("target/debug/logic-2").is_ok());
    }

    #[test]
    fn a_build_without_an_artifact_is_refused() {
        let mut build = successful(BuildAction::Build, "target/debug/logic");
        build.artifact = None;
        assert!(matches!(
            Candidate::from_build(&build, Revision::from_raw(7)),
            Err(Refusal::NoArtifact { .. })
        ));
    }

    #[test]
    fn a_burst_opens_one_window_and_flushes_once() {
        let mut debounce = Debounce::new(16);
        debounce.push("editor", 100);
        debounce.push("editor", 105);
        debounce.push("inspector", 110);
        assert_eq!(debounce.pending(), 2);
        assert_eq!(debounce.flush(114), None, "the window is still open");
        assert_eq!(
            debounce.flush(116),
            Some(vec!["editor".to_owned(), "inspector".to_owned()])
        );
        assert_eq!(debounce.flush(200), None);
        assert_eq!(debounce.pending(), 0);
    }

    #[test]
    fn actions_and_outcomes_parse_their_names() {
        assert_eq!(BuildAction::parse("rebuild"), Some(BuildAction::Rebuild));
        assert!(BuildAction::Rebuild.produces_artifact());
        assert!(!BuildAction::Check.produces_artifact());
        assert_eq!(Outcome::parse("fail"), Some(Outcome::Failure));
    }
}
