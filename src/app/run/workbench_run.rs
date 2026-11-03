// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The application-output panel's state (roadmap F16, `panel.run`).
//!
//! A run is a real host-owned process: the worker starts `program` through
//! [`OwnedProcess`] in the project directory, polls its captured stdout/stderr while it lives, and
//! records how it ended. A stop is a real signal to the process group; the panel never says
//! "stopped" unless the request is what ended it.
//!
//! **Boundary.** The program path is the one the person typed into the panel; this build does not
//! guess an artifact path from a package name. The panel shows the session, pid, stdout/stderr and
//! the exit reason; "locate in source" and "export" need a source map or a chosen file, so they are
//! not fabricated here.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::time::Duration;

use crate::contracts::permission::{EffectClass, Principal};
use crate::host::permission::{PermissionService, Scope};
use crate::host::subprocess::{CapturedOutput, ExitInfo, OwnedProcess, ProcessSpec, Termination};

/// How much of each stream the host keeps.
const CAPTURE_LIMIT: usize = 64 * 1024;
/// The most output rows one report carries.
const ROW_LIMIT: usize = 200;
/// How often the worker looks for an end.
const POLL: Duration = Duration::from_millis(100);
/// How long a stopped program has to exit.
const GRACE: Duration = Duration::from_secs(3);

/// One report from the running program.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct Update {
    /// `running`, `done` or `failed`.
    pub phase: &'static str,
    /// The one-line status.
    pub status: String,
    /// The rows the panel shows.
    pub rows: Vec<String>,
}

/// The panel's view of the last started program.
pub(super) struct RunState {
    rows: Vec<String>,
    updates: Receiver<Update>,
    cancel: Arc<AtomicBool>,
    live: bool,
}

impl RunState {
    /// No run yet.
    pub(super) fn new() -> Self {
        let (_sender, updates) = channel();
        Self {
            rows: vec!["no program has run".to_owned()],
            updates,
            cancel: Arc::new(AtomicBool::new(false)),
            live: false,
        }
    }

    /// Starts `program` in `directory` on a worker thread.
    pub(super) fn start(&mut self, program: String, directory: PathBuf) -> Result<(), String> {
        if self.live {
            return Err("a program is already running".to_owned());
        }
        if program.trim().is_empty() {
            return Err("no program to run".to_owned());
        }
        let (updates, updates_rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        self.updates = updates_rx;
        self.cancel = Arc::clone(&cancel);
        self.live = true;
        self.rows = vec![format!("starting {program} …")];
        std::thread::spawn(move || worker(program, directory, cancel, updates));
        Ok(())
    }

    /// Asks the running program to stop; `false` when nothing is running.
    pub(super) fn stop(&self) -> bool {
        if !self.live {
            return false;
        }
        self.cancel.store(true, Ordering::SeqCst);
        true
    }

    /// The latest report, if one arrived.
    pub(super) fn poll(&mut self) -> Option<Update> {
        let mut last = None;
        loop {
            match self.updates.try_recv() {
                Ok(update) => {
                    if update.phase != "running" {
                        self.live = false;
                    }
                    last = Some(update);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.live = false;
                    break;
                }
            }
        }
        if let Some(update) = &last {
            self.rows = update.rows.clone();
        }
        last
    }

    /// The rows the panel shows.
    pub(super) fn rows(&self) -> Vec<String> {
        self.rows.clone()
    }

    /// The rows as one text block, for an explicit export to the clipboard.
    pub(super) fn text(&self) -> String {
        self.rows.join("\n")
    }

    /// Clears the shown output; refused while the program still owns a process, because the next
    /// poll would immediately write the live output back and make the clear look ineffective.
    pub(super) fn clear(&mut self) -> bool {
        if self.live {
            return false;
        }
        self.rows = vec!["no program has run".to_owned()];
        true
    }

    /// Whether a process is still owned by this run.
    pub(super) fn is_live(&self) -> bool {
        self.live
    }
}

/// Starts the program, reports its output while it lives, and how it ended.
fn worker(program: String, directory: PathBuf, cancel: Arc<AtomicBool>, updates: Sender<Update>) {
    let permit = match PermissionService::new().permit(
        EffectClass::Command,
        Scope::Session,
        Principal::Human,
    ) {
        Ok(permit) => permit,
        Err(refusal) => {
            let reason = refusal.message();
            let _ = updates.send(Update {
                phase: "failed",
                status: reason.clone(),
                rows: vec![format!("run refused: {reason}")],
            });
            return;
        }
    };
    let spec = ProcessSpec::new(program.clone(), program.clone())
        .in_directory(directory)
        .capture_limit(CAPTURE_LIMIT);
    let mut process = match OwnedProcess::spawn(spec, &permit) {
        Ok(process) => process,
        Err(error) => {
            let reason = format!("{program} could not be started: {error}");
            let _ = updates.send(Update {
                phase: "failed",
                status: reason.clone(),
                rows: vec![reason],
            });
            return;
        }
    };
    let pid = process.pid();
    loop {
        let output = process.output_so_far();
        let _ = updates.send(Update {
            phase: "running",
            status: format!("{program} is running (pid {pid})"),
            rows: output_rows(&output, None),
        });
        if cancel.load(Ordering::SeqCst) {
            let stopped = process.terminate(GRACE);
            let output = process.output_so_far();
            let reason = match stopped {
                Termination::SignalFailed { reason } => {
                    format!("stop was requested but the program could not be signalled: {reason}")
                }
                Termination::AlreadyExited { exit } => exit_reason(&exit),
                Termination::Stopped { .. }
                | Termination::Killed { .. }
                | Termination::DirectOnly { .. } => "stopped on request".to_owned(),
            };
            let _ = updates.send(Update {
                phase: "done",
                status: format!("{program}: {reason}"),
                rows: output_rows(&output, Some(&reason)),
            });
            return;
        }
        if let Some(exit) = process.wait_for(POLL) {
            let output = process.output_so_far();
            let reason = exit_reason(&exit);
            let _ = updates.send(Update {
                phase: "done",
                status: format!("{program}: {reason}"),
                rows: output_rows(&output, Some(&reason)),
            });
            return;
        }
    }
}

/// The exit in one phrase.
fn exit_reason(exit: &ExitInfo) -> String {
    match (exit.code, exit.signal) {
        (Some(0), None) => "exited 0".to_owned(),
        (Some(code), None) => format!("exited {code}"),
        (None, Some(signal)) => format!("killed by signal {signal}"),
        (code, signal) => format!("ended with code {code:?} and signal {signal:?}"),
    }
}

/// The rows for one snapshot: the ending (when there is one), then stdout and stderr.
fn output_rows(output: &CapturedOutput, ending: Option<&str>) -> Vec<String> {
    let mut rows = Vec::new();
    if let Some(ending) = ending {
        rows.push(format!("session: {ending}"));
    }
    let mut detail = 0usize;
    for (stream, text) in [("stdout", &output.stdout), ("stderr", &output.stderr)] {
        for line in text.lines() {
            if detail >= ROW_LIMIT {
                rows.push(format!(
                    "omitted: output beyond the first {ROW_LIMIT} line(s)"
                ));
                detail += 1;
                break;
            }
            rows.push(format!("{stream}: {line}"));
            detail += 1;
        }
        if detail >= ROW_LIMIT {
            break;
        }
    }
    if output.is_truncated() {
        rows.push(format!("dropped: {} byte(s)", output.dropped_bytes));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::RunState;

    #[test]
    fn a_new_state_has_not_run() {
        let run = RunState::new();
        assert!(!run.is_live());
        assert_eq!(run.rows(), vec!["no program has run"]);
        assert!(!run.stop(), "there is nothing to stop");
    }

    #[test]
    fn an_empty_program_is_refused() {
        let mut run = RunState::new();
        let result = run.start("   ".to_owned(), std::env::temp_dir());
        assert!(result.is_err());
        assert!(!run.is_live());
    }

    #[test]
    fn a_finished_run_can_be_cleared_and_its_text_exported() {
        let mut run = RunState::new();
        assert_eq!(run.text(), "no program has run");
        assert!(run.clear());
        assert_eq!(run.rows(), vec!["no program has run"]);
    }
}
