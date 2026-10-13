// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! One error type for the host, and one mapping from it to a process exit code.
//!
//! The designer will have many failures that are not errors — a cancelled build, a refused
//! permission, an unsupported target — and those are values in the contracts, not exceptions. What
//! is left for this module is the small set of failures that stop the host itself from doing its
//! job: the platform runtime cannot start, the event loop cannot run, or a subsystem the process
//! depends on is missing.
//!
//! Two rules keep the failure paths honest:
//!
//! * **Every variant explains itself.** [`HostError::message`] is a sentence with the found state
//!   in it, so a terminal shows something more useful than "failed".
//! * **The exit code says which kind of failure it was**, because automation has to distinguish
//!   "the environment cannot run a GUI" from "the code is broken". [`ExitCode`] is the only place
//!   that mapping exists; the CLI does not invent numbers of its own.

use std::fmt;
use std::io;

/// What a host operation can fail with.
#[derive(Debug)]
pub enum HostError {
    /// The platform runtime refused to initialise.
    PlatformInit { reason: String },
    /// The runtime's event loop could not be entered or returned an error.
    EventLoop { reason: String },
    /// The runtime is not an interactive GUI backend, so a desktop session cannot be claimed.
    ///
    /// This is the case the dependency documents: a stub backend runs a loop and creates no window.
    /// It is a refusal with a reason, not a crash.
    NoGuiSession { backend: String, mode: String },
    /// A window could not be created.
    WindowCreation { reason: String },
    /// Writing a log record failed.
    LogSink { reason: String },
}

impl HostError {
    /// A sentence naming what was found, for a terminal or a problem panel.
    pub fn message(&self) -> String {
        match self {
            Self::PlatformInit { reason } => format!("the platform runtime did not initialise: {reason}"),
            Self::EventLoop { reason } => format!("the event loop did not run: {reason}"),
            Self::NoGuiSession { backend, mode } => format!(
                "backend {backend} reports gui mode {mode}, so no window would appear; \
                 this host does not claim a started session",
            ),
            Self::WindowCreation { reason } => format!("the window was not created: {reason}"),
            Self::LogSink { reason } => format!("a log record was not written: {reason}"),
        }
    }

    /// The exit code this failure maps to.
    pub fn exit_code(&self) -> ExitCode {
        match self {
            Self::NoGuiSession { .. } => ExitCode::EnvironmentUnavailable,
            Self::PlatformInit { .. } | Self::EventLoop { .. } | Self::WindowCreation { .. } => {
                ExitCode::Failed
            }
            Self::LogSink { .. } => ExitCode::Failed,
        }
    }
}

impl fmt::Display for HostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message())
    }
}

impl std::error::Error for HostError {}

impl From<io::Error> for HostError {
    fn from(error: io::Error) -> Self {
        Self::LogSink { reason: error.to_string() }
    }
}

/// What the process should exit with. The numbers are the contract automation reads, so they are
/// defined once here rather than at each `std::process::exit` call.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExitCode {
    /// The command did what it said it would.
    Ok,
    /// The command failed: something in the host or the input is wrong.
    Failed,
    /// Usage error: the arguments do not parse.
    Usage,
    /// The environment cannot do this (no GUI session, no toolchain, no device). Not a code defect.
    EnvironmentUnavailable,
    /// The command found a real problem and reported it: a probe gap, a failed check.
    Reported,
}

impl ExitCode {
    /// The number the process exits with.
    pub const fn code(self) -> i32 {
        match self {
            Self::Ok => 0,
            Self::Failed => 1,
            Self::Usage => 2,
            Self::EnvironmentUnavailable => 3,
            Self::Reported => 4,
        }
    }
}

/// The host's result type.
pub type HostResult<T> = std::result::Result<T, HostError>;

#[cfg(test)]
mod tests {
    use super::{ExitCode, HostError};
    use std::error::Error;

    fn every_error() -> Vec<HostError> {
        vec![
            HostError::PlatformInit { reason: "no display".into() },
            HostError::EventLoop { reason: "loop returned 1".into() },
            HostError::NoGuiSession { backend: "stub".into(), mode: "PreviewOrStub".into() },
            HostError::WindowCreation { reason: "geometry rejected".into() },
            HostError::LogSink { reason: "broken pipe".into() },
        ]
    }

    #[test]
    fn every_error_explains_itself_with_the_found_state() {
        for error in every_error() {
            let message = error.message();
            assert!(message.len() > 20, "{message}");
            assert_eq!(message, error.to_string(), "Display and message must agree");
            assert!(!format!("{error:?}").is_empty());
        }
    }

    #[test]
    fn a_missing_gui_session_is_distinct_from_a_defect() {
        let no_session = HostError::NoGuiSession { backend: "stub".into(), mode: "PreviewOrStub".into() };

        assert_eq!(no_session.exit_code(), ExitCode::EnvironmentUnavailable);
        assert_eq!(HostError::PlatformInit { reason: "x".into() }.exit_code(), ExitCode::Failed);
        assert!(no_session.message().contains("does not claim a started session"));
    }

    #[test]
    fn exit_codes_are_fixed_numbers() {
        assert_eq!(ExitCode::Ok.code(), 0);
        assert_eq!(ExitCode::Failed.code(), 1);
        assert_eq!(ExitCode::Usage.code(), 2);
        assert_eq!(ExitCode::EnvironmentUnavailable.code(), 3);
        assert_eq!(ExitCode::Reported.code(), 4);
    }

    #[test]
    fn an_io_error_becomes_a_log_sink_failure() {
        let error = HostError::from(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "closed"));

        assert_eq!(error.exit_code(), ExitCode::Failed);
        assert!(error.message().contains("closed"));
    }
}
