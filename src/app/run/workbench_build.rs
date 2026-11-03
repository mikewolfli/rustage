// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The build-output panel's state (roadmap F16, `panel.build`).
//!
//! A build is submitted to the host executor the tasks panel reads
//! ([`BuildTask::submit`]), so the queue/running/finished record is the executor's and the panel
//! only shows what the run measured: the cargo argv, the real verdict, the exit, the pid and the
//! bounded stderr. A cancel is a request to that executor ([`Executor::cancel`]) — not a flag this
//! panel invents.
//!
//! **Boundary.** The panel does not parse cargo's JSON diagnostics into a second format and does
//! not save a log file; the outcome rows are the measured result, and the stderr excerpt is what a
//! failure names. A project directory without `Cargo.toml` is refused before anything is submitted,
//! so a build can never run against the designer's own tree by accident.

use std::path::Path;

use crate::app::build::compile::{Action, BuildTask, Job, Submitted};
use crate::app::build::reproducible::Reproducibility;
use crate::app::build::toolchain::selection;
use crate::host::tasks::{CancelResult, Executor, OwnerId, TaskId};

/// One report from a submitted build.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct Update {
    /// `done` or `failed`.
    pub phase: &'static str,
    /// The one-line status.
    pub status: String,
    /// The rows the panel shows.
    pub rows: Vec<String>,
}

/// The panel's view of the last submitted build.
#[derive(Default)]
pub(super) struct BuildState {
    submitted: Option<Submitted>,
    task: Option<TaskId>,
    rows: Vec<String>,
    live: bool,
    reported: bool,
}

impl BuildState {
    /// No build yet.
    pub(super) fn new() -> Self {
        Self {
            rows: vec!["no build has run; right-click to build".to_owned()],
            ..Self::default()
        }
    }

    /// Submits a build of `root` to `executor`.
    ///
    /// Refused before submitting when the directory has no `Cargo.toml`: `--manifest-path` would
    /// otherwise point cargo at a file that does not exist, and omitting it would build whatever
    /// tree the designer happens to be running in.
    pub(super) fn start(
        &mut self,
        executor: &Executor,
        owner: OwnerId,
        root: &Path,
        action: Action,
    ) -> Result<(), String> {
        if self.live {
            return Err("a build is already running".to_owned());
        }
        let manifest = root.join("Cargo.toml");
        if !manifest.is_file() {
            return Err(format!(
                "{} has no Cargo.toml; nothing was run",
                root.display()
            ));
        }
        let selection = selection("desktop", &[]).map_err(|findings| {
            findings
                .iter()
                .map(|finding| finding.render())
                .collect::<Vec<_>>()
                .join("; ")
        })?;
        let job = Job {
            action,
            selection,
            target: None,
            package: None,
            manifest: Some(manifest.display().to_string()),
            reproducible: Reproducibility::default(),
        };
        let submitted = BuildTask::submit(executor, owner, job, "cargo")
            .map_err(|error| format!("the build could not be submitted: {error:?}"))?;
        self.task = Some(submitted.task());
        self.submitted = Some(submitted);
        self.live = true;
        self.reported = false;
        self.rows = vec![format!("{} {} …", action.name(), manifest.display())];
        Ok(())
    }

    /// The report for a finished build, once.
    pub(super) fn poll(&mut self) -> Option<Update> {
        if self.reported {
            return None;
        }
        let submitted = self.submitted.as_ref()?;
        let result = submitted.result()?;
        self.live = false;
        self.reported = true;
        let (phase, status, rows) = match result {
            Ok(outcome) => {
                let mut rows = vec![format!(
                    "command: cargo {}",
                    outcome.job.command().join(" ")
                )];
                rows.extend(outcome.render());
                rows.push(format!(
                    "pid: {}",
                    outcome
                        .pid
                        .map(|pid| pid.to_string())
                        .unwrap_or_else(|| "none".to_owned())
                ));
                let status = format!("build: {}", outcome.verdict.render());
                ("done", status, rows)
            }
            Err(reason) => (
                "failed",
                reason.clone(),
                vec![format!("build failed: {reason}")],
            ),
        };
        self.rows = rows.clone();
        Some(Update {
            phase,
            status,
            rows,
        })
    }

    /// Asks the executor to stop the running task.
    pub(super) fn cancel(&self, executor: &Executor) -> CancelResult {
        match self.task {
            Some(task) => executor.cancel(task),
            None => CancelResult::Unknown,
        }
    }

    /// The rows the panel shows.
    pub(super) fn rows(&self) -> Vec<String> {
        self.rows.clone()
    }

    /// Whether the submitted task has not reported yet.
    pub(super) fn is_live(&self) -> bool {
        self.live
    }

    /// The first source location named by the recorded rows, when one is there.
    ///
    /// Cargo's human-readable diagnostics put the location after `-->`, or at the start of a
    /// `file:line:column` line. The panel keeps only an excerpt of stderr, so this answers from
    /// what was actually recorded rather than asking cargo again.
    pub(super) fn first_error_location(&self) -> Option<(String, usize, usize)> {
        self.rows.iter().find_map(|row| location_in(row))
    }

    /// The task this build was submitted as, when it was.
    pub(super) fn task(&self) -> Option<TaskId> {
        self.task
    }
}

/// The source location one build row names, when it names one.
///
/// The path may be absolute or relative; it must look like a path (a `.` or a separator), so an
/// ordinary sentence containing two colons is not mistaken for a diagnostic.
fn location_in(row: &str) -> Option<(String, usize, usize)> {
    row.lines().find_map(location_in_line)
}

/// The source location one line names, when it is a bare `path:line:column` or an arrow line.
fn location_in_line(line: &str) -> Option<(String, usize, usize)> {
    let text = line.trim();
    let text = text.strip_prefix("-->").map(str::trim).unwrap_or(text);
    let mut parts = text.rsplitn(3, ':');
    let column = parts.next()?.trim();
    let line = parts.next()?.trim();
    let path = parts.next()?.trim();
    if path.is_empty() || !(path.contains('.') || path.contains('/') || path.contains('\\')) {
        return None;
    }
    if column.is_empty() || !column.chars().all(|character| character.is_ascii_digit()) {
        return None;
    }
    if line.is_empty() || !line.chars().all(|character| character.is_ascii_digit()) {
        return None;
    }
    Some((path.to_owned(), line.parse().ok()?, column.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::{BuildState, location_in};
    use crate::app::build::compile::Action;
    use crate::host::tasks::{Executor, ExecutorConfig, OwnerId};

    fn executor() -> Executor {
        Executor::new(ExecutorConfig::default()).expect("the default configuration is valid")
    }

    #[test]
    fn a_new_state_has_not_run() {
        let build = BuildState::new();
        assert!(!build.is_live());
        assert_eq!(build.rows(), vec!["no build has run; right-click to build"]);
    }

    #[test]
    fn a_directory_without_a_manifest_is_refused_before_submitting() {
        let executor = executor();
        let mut build = BuildState::new();
        let directory = std::env::temp_dir().join(format!(
            "rustage-workbench-build-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&directory).expect("a temporary directory");
        let result = build.start(&executor, OwnerId::new(), &directory, Action::Check);
        let _ = std::fs::remove_dir_all(&directory);

        assert!(result.is_err(), "no manifest means no build");
        assert!(!build.is_live());
        assert!(executor.records().is_empty(), "nothing was submitted");
    }

    #[test]
    fn a_build_row_names_its_first_source_location() {
        assert_eq!(
            location_in("--> src/main.rs:12:5"),
            Some(("src/main.rs".to_owned(), 12, 5))
        );
        assert_eq!(
            location_in("stderr: /tmp/a/b.rs:3:1: error: nope"),
            None,
            "a sentence after the location is not a bare path:line:column row"
        );
        assert_eq!(
            location_in("C:\\work\\a.rs:4:2"),
            Some(("C:\\work\\a.rs".to_owned(), 4, 2))
        );
        assert_eq!(location_in("error: something happened"), None);
        assert_eq!(location_in("note: see 1:2"), None);
        assert_eq!(
            location_in("stderr: error[E0599]: no method\n  --> src/main.rs:12:5\n   |"),
            Some(("src/main.rs".to_owned(), 12, 5))
        );
    }
}
