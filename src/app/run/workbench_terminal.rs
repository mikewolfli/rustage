// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The terminal panel's state (roadmap F16, `panel.terminal`).
//!
//! The session is a real pseudo-terminal ([`crate::host::pty::PtySession`]): the child sees a
//! terminal, typed lines are written to its stdin, and the bounded output is mirrored into the
//! panel. The worker owns the session and folds both directions, so the window never blocks on a
//! read.
//!
//! **Boundary.** The program is this session's shell: `$SHELL` when the environment names one and
//! `/bin/sh` otherwise. The panel reports the session, pid, output and exit; environment editing is
//! left to the caller that starts the session, and this build does not offer it as a form. A
//! platform without a pseudo-terminal answers with the refusal, not an empty terminal.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::time::Duration;

/// The most output rows one report carries.
const ROW_LIMIT: usize = 200;
/// How often the worker looks for an end.
const POLL: Duration = Duration::from_millis(100);

/// One report from the terminal's worker.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct Update {
    /// `running`, `done` or `failed`.
    pub phase: &'static str,
    /// The one-line status.
    pub status: String,
    /// The rows the panel shows.
    pub rows: Vec<String>,
}

/// A command for the worker.
enum Command {
    /// Write these bytes to the terminal.
    Write(String),
    /// Ask the child to stop.
    Stop,
}

/// The panel's view of one terminal session.
pub(super) struct TerminalState {
    rows: Vec<String>,
    commands: Option<Sender<Command>>,
    updates: Receiver<Update>,
    live: bool,
}

impl TerminalState {
    /// No session yet.
    pub(super) fn new() -> Self {
        let (_sender, updates) = channel();
        Self {
            rows: vec!["no terminal session".to_owned()],
            commands: None,
            updates,
            live: false,
        }
    }

    /// Starts the session's shell in `directory`.
    pub(super) fn start(&mut self, directory: PathBuf) -> Result<(), String> {
        if self.live {
            return Err("a terminal is already running".to_owned());
        }
        #[cfg(not(unix))]
        {
            let _ = directory;
            Err("this build has no pseudo-terminal: the host's PTY is Unix-only".to_owned())
        }
        #[cfg(unix)]
        {
            let program = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_owned());
            let (commands, commands_rx) = channel();
            let (updates, updates_rx) = channel();
            self.commands = Some(commands);
            self.updates = updates_rx;
            self.live = true;
            self.rows = vec![format!("starting {program} …")];
            std::thread::spawn(move || worker(program, directory, commands_rx, updates));
            Ok(())
        }
    }

    /// Writes a line to the running session; `false` when there is none.
    pub(super) fn write(&self, text: &str) -> bool {
        self.commands
            .as_ref()
            .is_some_and(|sender| sender.send(Command::Write(text.to_owned())).is_ok())
    }

    /// Asks the session's child to stop; `false` when no session is connected.
    pub(super) fn stop(&self) -> bool {
        self.commands
            .as_ref()
            .is_some_and(|sender| sender.send(Command::Stop).is_ok())
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

    /// Whether a session is connected.
    pub(super) fn is_live(&self) -> bool {
        self.live
    }
}

/// Runs the shell on a real pseudo-terminal and folds input and output.
#[cfg(unix)]
fn worker(
    program: String,
    directory: PathBuf,
    commands: Receiver<Command>,
    updates: Sender<Update>,
) {
    let directory = directory.display().to_string();
    let mut session =
        match crate::host::pty::PtySession::spawn(&program, &[], Some(&directory), &[], 80, 24) {
            Ok(session) => session,
            Err(reason) => {
                let _ = updates.send(Update {
                    phase: "failed",
                    status: reason.clone(),
                    rows: vec![format!("terminal unavailable: {reason}")],
                });
                return;
            }
        };
    let pid = session.pid();
    loop {
        while let Ok(command) = commands.try_recv() {
            match command {
                Command::Write(text) => {
                    if let Err(reason) = session.write(&text) {
                        let _ = updates.send(Update {
                            phase: "failed",
                            status: reason.clone(),
                            rows: vec![format!("terminal write failed: {reason}")],
                        });
                        return;
                    }
                }
                Command::Stop => session.terminate(),
            }
        }
        let output = session.output();
        let _ = updates.send(Update {
            phase: "running",
            status: format!("{program} (pid {pid})"),
            rows: tail_rows(&output),
        });
        match session.wait(POLL) {
            Ok(Some(exit)) => {
                let reason = match (exit.code, exit.signal) {
                    (Some(code), _) => format!("exited {code}"),
                    (None, Some(signal)) => format!("killed by signal {signal}"),
                    (None, None) => "ended without a status".to_owned(),
                };
                let _ = updates.send(Update {
                    phase: "done",
                    status: format!("{program}: {reason}"),
                    rows: tail_rows(&session.output()),
                });
                return;
            }
            Ok(None) => {}
            Err(reason) => {
                let _ = updates.send(Update {
                    phase: "failed",
                    status: reason.clone(),
                    rows: vec![format!("terminal lost: {reason}")],
                });
                return;
            }
        }
    }
}

/// The last [`ROW_LIMIT`] lines of the terminal's output.
#[cfg(unix)]
fn tail_rows(output: &str) -> Vec<String> {
    let lines: Vec<&str> = output.lines().collect();
    let start = lines.len().saturating_sub(ROW_LIMIT);
    let mut rows: Vec<String> = lines[start..]
        .iter()
        .map(|line| (*line).to_owned())
        .collect();
    if start > 0 {
        rows.insert(0, format!("… {start} earlier line(s) …"));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::TerminalState;

    #[test]
    fn a_new_state_has_no_session() {
        let terminal = TerminalState::new();
        assert!(!terminal.is_live());
        assert_eq!(terminal.rows(), vec!["no terminal session"]);
        assert!(!terminal.write("echo hi\n"), "nothing to write to");
        assert!(!terminal.stop(), "nothing to stop");
    }
}
