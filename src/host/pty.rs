// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! A real pseudo-terminal on Unix (task T16.06).
//!
//! [`app::build::terminal::Session`](crate::app::build::terminal::Session) is the session *model* —
//! working directory, environment, size, authorization and the bounded buffer. This module is the
//! byte pump the model deliberately does not own: a master/slave pair from `posix_openpt`, a child
//! attached to the slave, and a reader that keeps the output in the same bounded shape.
//!
//! The split is the point. The model's tests run without a process; these tests run a real `/bin/sh`
//! and prove the bytes actually move. A single type doing both would make the model's failure paths
//! unreachable without a Unix host, and would put `cfg(unix)` branches into the layer above the
//! backend that principle 42 keeps clean.

#![cfg(unix)]

use std::ffi::CStr;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::process::ExitStatusExt;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// How many bytes of output are retained (the model's limit, applied here too).
pub const BUFFER_LIMIT: usize = 64 * 1024;

/// How a terminal's child ended.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Exit {
    /// The exit code, when the host reports one.
    pub code: Option<i32>,
    /// The signal, when the host ended the process with one (Unix).
    pub signal: Option<i32>,
}

impl Exit {
    /// Whether the child ended successfully.
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
}

/// A running child attached to a real pseudo-terminal.
#[derive(Debug)]
pub struct PtySession {
    master: File,
    child: Child,
    output: Arc<Mutex<Vec<u8>>>,
    dropped: Arc<Mutex<usize>>,
    reader: Option<JoinHandle<()>>,
    finished: Option<Exit>,
}

impl PtySession {
    /// Starts `program` with `args` on a new pseudo-terminal.
    ///
    /// The child's stdin/stdout/stderr are the slave side, so a program that checks `isatty(3)` sees
    /// a terminal rather than a pipe. The caller has already authorized the session in the model;
    /// this function does not decide permissions.
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
        // SAFETY: `posix_openpt` takes only flags and returns a fresh fd or -1; no pointer or
        // lifetime is involved. `O_NOCTTY` keeps the master from becoming this process's
        // controlling terminal.
        let master_fd = unsafe { libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY) };
        if master_fd < 0 {
            return Err(format!(
                "a pseudo-terminal could not be opened: {}",
                std::io::Error::last_os_error()
            ));
        }
        // SAFETY: `master_fd` is the valid descriptor returned above, and both calls take only it.
        if unsafe { libc::grantpt(master_fd) } != 0 || unsafe { libc::unlockpt(master_fd) } != 0 {
            let error = std::io::Error::last_os_error();
            // SAFETY: `master_fd` is owned by this function at this point and not used again.
            unsafe { libc::close(master_fd) };
            return Err(format!(
                "the pseudo-terminal could not be prepared: {error}"
            ));
        }
        // SAFETY: `ptsname` returns a NUL-terminated string owned by libc for this descriptor; it is
        // read immediately and copied before any other libc call could invalidate it.
        let slave_name = unsafe {
            let pointer = libc::ptsname(master_fd);
            if pointer.is_null() {
                libc::close(master_fd);
                return Err(format!(
                    "the pseudo-terminal has no slave name: {}",
                    std::io::Error::last_os_error()
                ));
            }
            CStr::from_ptr(pointer).to_string_lossy().into_owned()
        };

        let slave = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&slave_name)
            .map_err(|error| {
                // SAFETY: `master_fd` is still owned here and is closed exactly once.
                unsafe { libc::close(master_fd) };
                format!("the pseudo-terminal slave {slave_name:?} could not be opened: {error}")
            })?;

        let mut command = Command::new(program);
        command.args(args);
        command.stdin(Stdio::from(slave.try_clone().map_err(|error| {
            // SAFETY: as above; the master descriptor is closed exactly once.
            unsafe { libc::close(master_fd) };
            format!("the terminal's input could not be attached: {error}")
        })?));
        command.stdout(Stdio::from(slave.try_clone().map_err(|error| {
            // SAFETY: as above; the master descriptor is closed exactly once.
            unsafe { libc::close(master_fd) };
            format!("the terminal's output could not be attached: {error}")
        })?));
        command.stderr(Stdio::from(slave.try_clone().map_err(|error| {
            // SAFETY: as above; the master descriptor is closed exactly once.
            unsafe { libc::close(master_fd) };
            format!("the terminal's errors could not be attached: {error}")
        })?));
        if let Some(directory) = directory {
            command.current_dir(directory);
        }
        for (name, value) in environment {
            command.env(name, value);
        }
        let child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                // SAFETY: the master descriptor is owned by this function and is not used again.
                unsafe { libc::close(master_fd) };
                return Err(format!("{program:?} could not be started: {error}"));
            }
        };

        // SAFETY: `master_fd` is a valid descriptor returned by `posix_openpt`; `File` takes
        // ownership and will close it exactly once.
        let master = unsafe { File::from_raw_fd(master_fd) };
        let mut session = Self {
            master,
            child,
            output: Arc::new(Mutex::new(Vec::new())),
            dropped: Arc::new(Mutex::new(0)),
            reader: None,
            finished: None,
        };
        session.resize(columns, rows)?;
        session.start_reader();
        Ok(session)
    }

    /// The child's pid.
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Writes text exactly as given (the caller decides whether it ends with a newline).
    pub fn write(&mut self, text: &str) -> Result<(), String> {
        self.master
            .write_all(text.as_bytes())
            .map_err(|error| format!("the terminal refused input: {error}"))
    }

    /// Sets the terminal's window size.
    pub fn resize(&mut self, columns: u16, rows: u16) -> Result<(), String> {
        if columns == 0 || rows == 0 {
            return Err("a terminal needs at least one column and one row".to_owned());
        }
        // SAFETY: `winsize` is a plain-old-data struct; zeroed is a valid initial value and every
        // field used by `TIOCSWINSZ` is then assigned.
        let mut size: libc::winsize = unsafe { std::mem::zeroed() };
        size.ws_col = columns;
        size.ws_row = rows;
        // SAFETY: the descriptor is live for the lifetime of `self`, and `&size` is a valid pointer
        // to an initialized `winsize` for the duration of the call.
        let result = unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ, &size) };
        if result != 0 {
            return Err(format!(
                "the terminal size could not be set: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(())
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
        *self.dropped.lock().expect("the terminal drop lock")
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
                        code: status.code(),
                        // `ExitStatusExt` is the Unix-specific half; importing it here keeps the
                        // public `Exit` free of `std::os` and lets the Windows module answer with
                        // the same shape.
                        signal: status.signal(),
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
                    code: status.code(),
                    signal: status.signal(),
                });
            }
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }

    fn start_reader(&mut self) {
        let mut reader = match self.master.try_clone() {
            Ok(reader) => reader,
            Err(_) => return,
        };
        let output = Arc::clone(&self.output);
        let dropped = Arc::clone(&self.dropped);
        self.reader = Some(thread::spawn(move || {
            let mut chunk = [0u8; 4096];
            loop {
                match reader.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(count) => {
                        let mut buffer = output.lock().expect("the terminal output lock");
                        buffer.extend_from_slice(&chunk[..count]);
                        if buffer.len() > BUFFER_LIMIT {
                            let excess = buffer.len() - BUFFER_LIMIT;
                            buffer.drain(..excess);
                            *dropped.lock().expect("the terminal drop lock") += excess;
                        }
                    }
                    // EIO is how a Unix master reports "the slave side is gone"; anything else is
                    // also the end of this session's output.
                    Err(_) => break,
                }
            }
        }));
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        self.terminate();
    }
}

#[cfg(test)]
mod tests {
    use super::PtySession;
    use std::time::Duration;

    fn sh(script: &str) -> PtySession {
        PtySession::spawn(
            "/bin/sh",
            &["-c".to_owned(), script.to_owned()],
            None,
            &[],
            80,
            24,
        )
        .expect("a pseudo-terminal session")
    }

    #[test]
    fn a_real_child_writes_through_the_terminal() {
        let mut session = sh("printf hello-from-pty");
        let status = session
            .wait(Duration::from_secs(5))
            .expect("the child's status")
            .expect("the child exits");
        assert!(status.success(), "{status:?}");
        // The reader may observe the output a moment after the child exits.
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !session.output().contains("hello-from-pty") && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            session.output().contains("hello-from-pty"),
            "output was {:?}",
            session.output()
        );
        assert_eq!(session.dropped(), 0);
    }

    #[test]
    fn input_reaches_the_program_and_output_comes_back() {
        let mut session = PtySession::spawn("/bin/cat", &[], None, &[], 40, 12)
            .expect("a pseudo-terminal session");
        session.write("ping-from-input\n").expect("input");
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while !session.output().contains("ping-from-input") && std::time::Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            session.output().contains("ping-from-input"),
            "output was {:?}",
            session.output()
        );
        session.terminate();
    }

    #[test]
    fn a_size_of_zero_is_refused_before_anything_is_started() {
        let error = PtySession::spawn("/bin/true", &[], None, &[], 0, 24).expect_err("a refusal");
        assert!(error.contains("at least one column"), "{error}");
    }
}
