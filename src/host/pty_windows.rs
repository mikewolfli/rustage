// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! A real pseudo-terminal on Windows (ConPTY), behind the same surface as the Unix module.
//!
//! The ConPTY setup — a pseudo-console handle and `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE` in a
//! `STARTUPINFOEXW` — is protocol plumbing, not designer logic. `portable-pty` owns exactly that
//! seam; this file owns the host's policy (the bounded output buffer, the size rules and the
//! shutdown order) and keeps the public `PtySession`/`Exit` shape identical to `host::pty` on Unix,
//! so `--terminal exec` has one call site rather than a platform branch above the backend.
//!
//! **What is verified.** This module is compiled for `x86_64-pc-windows-msvc` by the cross-build
//! check. It has not been run on a Windows machine in this repository; a real ConPTY session is a
//! device-level fact the Windows host must observe (the Unix session is the one with runtime
//! evidence here).

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};

/// How many bytes of output are retained (the model's limit, applied here too).
pub const BUFFER_LIMIT: usize = 64 * 1024;

/// How a terminal's child ended.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Exit {
    /// The exit code, when the host reports one.
    pub code: Option<i32>,
    /// The signal. ConPTY reports a signal as text; the designer's `Exit` keeps the Unix shape and
    /// leaves this `None` on Windows.
    pub signal: Option<i32>,
}

impl Exit {
    /// Whether the child ended successfully.
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
}

/// A running child attached to a ConPTY.
pub struct PtySession {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    output: Arc<Mutex<Vec<u8>>>,
    dropped: Arc<Mutex<usize>>,
    reader: Option<JoinHandle<()>>,
    finished: Option<Exit>,
}

impl PtySession {
    /// Starts `program` with `args` on a new pseudo-console.
    pub fn spawn(
        program: &str,
        args: &[String],
        directory: Option<&str>,
        environment: &[(String, String)],
        columns: u16,
        rows: u16,
    ) -> Result<Self, String> {
        if columns == 0 || rows == 0 {
            return Err("a terminal needs at least one column and one row".to_owned());
        }
        let pair = native_pty_system()
            .openpty(PtySize {
                rows,
                cols: columns,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| format!("a pseudo-console could not be opened: {error}"))?;
        let mut command = CommandBuilder::new(program);
        command.args(args);
        if let Some(directory) = directory {
            command.cwd(directory);
        }
        for (name, value) in environment {
            command.env(name, value);
        }
        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| format!("{program:?} could not be started: {error}"))?;
        // The slave handle must be dropped before the master's reader is used; otherwise the
        // console stays open even after the child exits.
        drop(pair.slave);

        let master = pair.master;
        let writer = master
            .take_writer()
            .map_err(|error| format!("the terminal's input could not be written: {error}"))?;
        let mut reader = master
            .try_clone_reader()
            .map_err(|error| format!("the terminal's output could not be read: {error}"))?;
        let output = Arc::new(Mutex::new(Vec::new()));
        let dropped = Arc::new(Mutex::new(0));
        let reader_output = Arc::clone(&output);
        let reader_dropped = Arc::clone(&dropped);
        let handle = thread::spawn(move || {
            let mut chunk = [0u8; 4096];
            loop {
                match reader.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(count) => {
                        let mut buffer = reader_output
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        buffer.extend_from_slice(&chunk[..count]);
                        if buffer.len() > BUFFER_LIMIT {
                            let excess = buffer.len() - BUFFER_LIMIT;
                            buffer.drain(..excess);
                            *reader_dropped
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner()) += excess;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            master,
            writer,
            child,
            output,
            dropped,
            reader: Some(handle),
            finished: None,
        })
    }

    /// The child's pid.
    pub fn pid(&self) -> u32 {
        self.child.process_id().unwrap_or_default()
    }

    /// Writes text exactly as given (the caller decides whether it ends with a newline).
    pub fn write(&mut self, text: &str) -> Result<(), String> {
        self.writer
            .write_all(text.as_bytes())
            .and_then(|()| self.writer.flush())
            .map_err(|error| format!("the terminal refused input: {error}"))
    }

    /// Sets the terminal's window size.
    pub fn resize(&mut self, columns: u16, rows: u16) -> Result<(), String> {
        if columns == 0 || rows == 0 {
            return Err("a terminal needs at least one column and one row".to_owned());
        }
        self.master
            .resize(PtySize {
                rows,
                cols: columns,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| format!("the terminal size could not be set: {error}"))
    }

    /// The output seen so far, with the most recent [`BUFFER_LIMIT`] bytes retained.
    pub fn output(&self) -> String {
        let bytes = self
            .output
            .lock()
            .expect("the terminal output lock")
            .clone();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    /// How many bytes were dropped because the buffer was full.
    pub fn dropped(&self) -> usize {
        *self
            .dropped
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Waits up to `limit` for the child to exit, returning its status if it did.
    pub fn wait(&mut self, limit: Duration) -> Result<Option<Exit>, String> {
        if let Some(status) = self.finished {
            return Ok(Some(status));
        }
        let deadline = Instant::now() + limit;
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => {
                    let exit = Exit {
                        code: Some(status.exit_code() as i32),
                        signal: None,
                    };
                    self.finished = Some(exit);
                    return Ok(Some(exit));
                }
                Ok(None) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(10));
                }
                Ok(None) => return Ok(None),
                Err(error) => {
                    return Err(format!("the terminal's status could not be read: {error}"));
                }
            }
        }
    }

    /// Stops the child if it is still running and joins the reader.
    pub fn terminate(&mut self) {
        if self.finished.is_none() {
            let _ = self.child.kill();
            if let Ok(status) = self.child.wait() {
                self.finished = Some(Exit {
                    code: Some(status.exit_code() as i32),
                    signal: None,
                });
            }
        }
        if let Some(reader) = self.reader.take()
            && reader.join().is_err()
        {
            crate::host::log::emit(
                crate::host::log::Level::Warn,
                "host::pty",
                "the terminal output reader panicked while shutting down; output after this \
                 point is missing from the session",
            );
        }
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        self.terminate();
    }
}
