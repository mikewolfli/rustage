// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Check, build and rebuild: what they run and what their exit means (task T16.02).
//!
//! The acceptance line is *"success rests on the real exit, and a cancel does not leave a misleading
//! artifact"*. Both are in the types:
//!
//! * **The command is data.** [`Job`] holds the action and the selection, and [`Job::command`] is the
//!   one place the argv is built, so the check the designer runs and the build a person runs by hand
//!   are the same line (principle 51 for the feature selection).
//! * **The verdict comes from the exit, not from output.** [`Outcome::from_exit`] maps a real exit
//!   code to a [`Verdict`]; a run that was cancelled has **no exit code** and is
//!   [`Verdict::Cancelled`], which is never success — a partially written target that a cancel left
//!   behind is reported stale rather than presented as the build.
//! * **A rebuild is a clean of one package, not of the tree.** [`Job::clean_command`] removes only the
//!   artifacts of the package being rebuilt, so a rebuild cannot invalidate an unrelated dependency's
//!   cache.

#![allow(dead_code)]

use super::toolchain::Selection;

/// What a build action asks cargo to do.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    /// Type-check without producing a binary.
    Check,
    /// Build the binary.
    Build,
    /// Remove this package's artifacts, then build.
    Rebuild,
}

impl Action {
    /// The action's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Check => "check",
            Self::Build => "build",
            Self::Rebuild => "rebuild",
        }
    }

    /// The cargo subcommand this action runs last.
    pub fn cargo_subcommand(&self) -> &'static str {
        match self {
            Self::Check => "check",
            Self::Build | Self::Rebuild => "build",
        }
    }
}

/// One build request: the action, the selection, and where it runs.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Job {
    /// What to do.
    pub action: Action,
    /// The profile and features.
    pub selection: Selection,
    /// The target triple, when it differs from the host.
    pub target: Option<String>,
    /// The package to build, for the `-p` flag and for a scoped clean.
    pub package: Option<String>,
    /// The manifest to build, when the working directory is not the one that holds it.
    pub manifest: Option<String>,
}

impl Job {
    /// The cargo argv for this job, without the program name.
    pub fn command(&self) -> Vec<String> {
        let mut argv = vec![self.action.cargo_subcommand().to_owned()];
        argv.extend(self.selection.feature_arguments());
        if let Some(target) = &self.target {
            argv.push("--target".to_owned());
            argv.push(target.clone());
        }
        if let Some(package) = &self.package {
            argv.push("-p".to_owned());
            argv.push(package.clone());
        }
        if let Some(manifest) = &self.manifest {
            argv.push("--manifest-path".to_owned());
            argv.push(manifest.clone());
        }
        argv
    }

    /// The scoped clean a rebuild runs first, or `None` for a plain check or build.
    ///
    /// A rebuild cleans **this package**, not the workspace: cleaning the tree would throw away every
    /// dependency's cache to repair one crate.
    pub fn clean_command(&self) -> Option<Vec<String>> {
        if self.action != Action::Rebuild {
            return None;
        }
        let mut argv = vec!["clean".to_owned(), "-p".to_owned()];
        argv.push(self.package.clone().unwrap_or_else(|| ".".to_owned()));
        if let Some(manifest) = &self.manifest {
            argv.push("--manifest-path".to_owned());
            argv.push(manifest.clone());
        }
        Some(argv)
    }

    /// The job in one line.
    pub fn render(&self) -> String {
        format!(
            "{} [{}]{}",
            self.action.name(),
            self.selection.render(),
            self.package
                .as_ref()
                .map(|package| format!(" package {package}"))
                .unwrap_or_default(),
        )
    }
}

/// What a build run ended as.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Verdict {
    /// Cargo exited zero.
    Succeeded,
    /// Cargo exited non-zero.
    Failed {
        /// The exit code it reported.
        code: i32,
    },
    /// The run was cancelled before it exited, so there is no exit code and nothing to trust.
    Cancelled,
}

impl Verdict {
    /// Whether this verdict is a real success.
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Succeeded)
    }

    /// The verdict in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Succeeded => "succeeded".to_owned(),
            Self::Failed { code } => format!("failed with exit code {code}"),
            Self::Cancelled => "cancelled (no exit code; any artifact is stale)".to_owned(),
        }
    }
}

/// One build run's result.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Outcome {
    /// The job that ran.
    pub job: Job,
    /// What it ended as.
    pub verdict: Verdict,
    /// Standard output, kept bounded by the caller.
    pub stdout: String,
    /// Standard error, kept bounded by the caller.
    pub stderr: String,
}

impl Outcome {
    /// Builds the outcome of a run that exited with `exit` (`None` when it was cancelled).
    pub fn from_exit(job: Job, exit: Option<i32>, stdout: String, stderr: String) -> Self {
        let verdict = match exit {
            Some(0) => Verdict::Succeeded,
            Some(code) => Verdict::Failed { code },
            None => Verdict::Cancelled,
        };
        Self {
            job,
            verdict,
            stdout,
            stderr,
        }
    }

    /// The outcome in one line per fact.
    pub fn render(&self) -> Vec<String> {
        let mut out = vec![format!("{}: {}", self.job.render(), self.verdict.render())];
        if !self.stderr.trim().is_empty() {
            out.push(format!("stderr: {} bytes", self.stderr.len()));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::{Action, Job, Outcome, Verdict};
    use crate::app::build::toolchain::Selection;

    fn job(action: Action) -> Job {
        Job {
            action,
            selection: Selection {
                profile: "desktop".to_owned(),
                extra: Vec::new(),
            },
            target: None,
            package: Some("demo".to_owned()),
            manifest: None,
        }
    }

    #[test]
    fn a_check_runs_cargo_check_with_the_profile_feature() {
        assert_eq!(
            job(Action::Check).command(),
            vec![
                "check",
                "--no-default-features",
                "--features",
                "desktop",
                "-p",
                "demo"
            ],
        );
    }

    #[test]
    fn a_build_runs_cargo_build_and_a_cross_target_adds_target() {
        let mut cross = job(Action::Build);
        cross.target = Some("x86_64-pc-windows-msvc".to_owned());
        let argv = cross.command();
        assert_eq!(argv[0], "build");
        assert!(
            argv.windows(2)
                .any(|pair| pair == ["--target", "x86_64-pc-windows-msvc"])
        );
    }

    #[test]
    fn only_a_rebuild_cleans_first_and_only_the_package() {
        assert!(job(Action::Check).clean_command().is_none());
        assert!(job(Action::Build).clean_command().is_none());
        assert_eq!(
            job(Action::Rebuild).clean_command(),
            Some(vec!["clean".to_owned(), "-p".to_owned(), "demo".to_owned()]),
        );
    }

    #[test]
    fn success_rests_on_a_real_zero_exit() {
        let outcome = Outcome::from_exit(job(Action::Build), Some(0), String::new(), String::new());
        assert_eq!(outcome.verdict, Verdict::Succeeded);
        assert!(outcome.verdict.is_success());

        let failed = Outcome::from_exit(job(Action::Build), Some(101), String::new(), "e".into());
        assert_eq!(failed.verdict, Verdict::Failed { code: 101 });
        assert!(!failed.verdict.is_success());
    }

    #[test]
    fn a_cancel_is_not_a_success_and_says_its_artifact_is_stale() {
        let cancelled = Outcome::from_exit(job(Action::Build), None, String::new(), String::new());
        assert_eq!(cancelled.verdict, Verdict::Cancelled);
        assert!(!cancelled.verdict.is_success());
        assert!(cancelled.verdict.render().contains("stale"));
    }
}
