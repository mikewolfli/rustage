// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Terminals: sessions, their environment, and what may be typed into one (task T16.06).
//!
//! The acceptance line is *"a terminal is a real session on each of the three hosts, its working
//! directory and environment are explicit, and running one is authorized"*. The line between the two
//! layers is drawn here rather than blurred:
//!
//! * **This module is the session model, not the byte pump.** [`Session`] holds the working
//!   directory, the environment, the size and the buffer's bounds; the pseudo-terminal itself is a
//!   platform mechanism, so which one is used is asked of the host ([`mechanism`]) rather than
//!   assumed here — a `cfg(target_os)` in this file would be the platform branch principle 42 keeps
//!   out of the layers above the backend.
//! * **Starting a session is authorized.** [`Session::authorize`] must be answered before a session
//!   can run, so "the terminal opened by itself" is not a reachable state (principle 19's permission
//!   rule applied to a process a person drives).
//! * **Output has a bound.** The buffer keeps the most recent bytes and counts what it dropped, so a
//!   fast producer does not grow memory without limit and does not silently hide output either.

#![allow(dead_code)]

/// The host this build runs on, as the designer names it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Platform {
    /// macOS.
    Macos,
    /// Linux.
    Linux,
    /// Windows.
    Windows,
}

impl Platform {
    /// The platform's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Macos => "macos",
            Self::Linux => "linux",
            Self::Windows => "windows",
        }
    }
}

/// The pseudo-terminal mechanism a platform provides.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mechanism {
    /// A Unix pseudo-terminal (`openpty`/`posix_openpt`).
    PosixPty,
    /// Windows' ConPTY.
    ConPty,
}

impl Mechanism {
    /// The mechanism in one line.
    pub fn render(&self) -> &'static str {
        match self {
            Self::PosixPty => "a POSIX pseudo-terminal (openpty), provided by the host backend",
            Self::ConPty => "Windows ConPTY, provided by the host backend",
        }
    }
}

/// Which mechanism a platform's terminal uses.
pub fn mechanism(platform: Platform) -> Mechanism {
    match platform {
        Platform::Macos | Platform::Linux => Mechanism::PosixPty,
        Platform::Windows => Mechanism::ConPty,
    }
}

/// How many bytes of output a session keeps before it starts dropping the oldest.
pub const BUFFER_LIMIT: usize = 64 * 1024;

/// A terminal session's configuration and running state.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Session {
    /// The session's name.
    pub name: String,
    /// The working directory, or `None` for the project root.
    pub directory: Option<String>,
    /// Environment variables set for the session.
    pub environment: Vec<(String, String)>,
    /// The number of columns.
    pub columns: u16,
    /// The number of rows.
    pub rows: u16,
    /// Whether a person authorized running processes in it.
    pub authorized: bool,
    /// Whether it is running.
    pub running: bool,
    buffer: Vec<u8>,
    dropped: usize,
}

impl Session {
    /// A session with a name and a default size.
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            directory: None,
            environment: Vec::new(),
            columns: 80,
            rows: 24,
            authorized: false,
            running: false,
            buffer: Vec::new(),
            dropped: 0,
        }
    }

    /// Records the person's authorization to run processes here.
    pub fn authorize(&mut self) {
        self.authorized = true;
    }

    /// Starts the session, or explains why it cannot.
    pub fn start(&mut self) -> Result<(), String> {
        if !self.authorized {
            return Err(format!(
                "session {:?} has not been authorized to run processes",
                self.name
            ));
        }
        if self.columns == 0 || self.rows == 0 {
            return Err("a terminal needs at least one column and one row".to_owned());
        }
        self.running = true;
        Ok(())
    }

    /// Records output, keeping the most recent [`BUFFER_LIMIT`] bytes.
    pub fn write(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
        if self.buffer.len() > BUFFER_LIMIT {
            let excess = self.buffer.len() - BUFFER_LIMIT;
            self.buffer.drain(..excess);
            self.dropped += excess;
        }
    }

    /// The output kept, as text (lossy for anything that is not UTF-8).
    pub fn output(&self) -> String {
        String::from_utf8_lossy(&self.buffer).into_owned()
    }

    /// How many bytes were dropped because the buffer was full.
    pub fn dropped(&self) -> usize {
        self.dropped
    }

    /// Resizes the terminal.
    pub fn resize(&mut self, columns: u16, rows: u16) -> Result<(), String> {
        if columns == 0 || rows == 0 {
            return Err("a terminal needs at least one column and one row".to_owned());
        }
        self.columns = columns;
        self.rows = rows;
        Ok(())
    }

    /// Copies a selection of the kept output, by byte range.
    pub fn copy(&self, start: usize, end: usize) -> Result<String, String> {
        if start > end || end > self.buffer.len() {
            return Err(format!(
                "selection {start}..{end} is not inside the {} bytes kept",
                self.buffer.len()
            ));
        }
        Ok(String::from_utf8_lossy(&self.buffer[start..end]).into_owned())
    }

    /// The session in one line.
    pub fn render(&self) -> String {
        format!(
            "{}: {}x{}, {}, {}",
            self.name,
            self.columns,
            self.rows,
            if self.running { "running" } else { "stopped" },
            if self.authorized {
                "authorized"
            } else {
                "not authorized"
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{BUFFER_LIMIT, Mechanism, Platform, Session, mechanism};

    #[test]
    fn each_of_the_three_hosts_names_its_own_mechanism() {
        assert_eq!(mechanism(Platform::Macos), Mechanism::PosixPty);
        assert_eq!(mechanism(Platform::Linux), Mechanism::PosixPty);
        assert_eq!(mechanism(Platform::Windows), Mechanism::ConPty);
        assert!(mechanism(Platform::Windows).render().contains("ConPTY"));
    }

    #[test]
    fn a_session_cannot_start_before_it_is_authorized() {
        let mut session = Session::new("build");
        let error = session.start().expect_err("unauthorized");
        assert!(error.contains("not been authorized"), "{error}");
        session.authorize();
        session.start().expect("an authorized start");
        assert!(session.running);
    }

    #[test]
    fn the_buffer_keeps_the_newest_bytes_and_counts_what_it_dropped() {
        let mut session = Session::new("t");
        session.write(&vec![b'x'; BUFFER_LIMIT + 100]);
        assert_eq!(session.output().len(), BUFFER_LIMIT);
        assert_eq!(session.dropped(), 100);
    }

    #[test]
    fn a_selection_outside_the_buffer_is_refused() {
        let mut session = Session::new("t");
        session.write(b"hello");
        assert_eq!(session.copy(0, 5).unwrap(), "hello");
        assert!(session.copy(0, 6).is_err());
    }

    #[test]
    fn a_zero_sized_terminal_is_refused() {
        let mut session = Session::new("t");
        assert!(session.resize(0, 24).is_err());
    }
}
