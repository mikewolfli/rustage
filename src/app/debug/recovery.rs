// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! What a debug session leaves behind when it fails (task T19.08).
//!
//! The acceptance line is *"after a failure the project is editable again and the session's
//! resources are cleaned up"*. Failures here are not one thing:
//!
//! * the adapter exits while the session is live;
//! * the adapter answers with a protocol error;
//! * the source map goes stale under a live session;
//! * the target profile cannot be debugged at all;
//! * the debuggee raises an exception that the adapter reports as a stop.
//!
//! [`recover`] turns each into a [`Recovery`] that states four facts separately: whether the session
//! ended, whether the process **this session owned** was terminated, whether the session's resources
//! were released, and whether the project is editable again. An exception is not a failure — it is a
//! stop the person continues from — so it ends nothing.
//!
//! The distinction that matters for safety is the same as T19.02's: an attached process is never
//! terminated here, because it was never this session's to terminate.

#![allow(dead_code)]

/// Something that went wrong during a session.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Incident {
    /// The adapter process exited on its own.
    AdapterExit {
        /// Its exit code, when it has one.
        code: Option<i32>,
    },
    /// The adapter reported a protocol-level failure.
    AdapterError {
        /// What it said.
        message: String,
    },
    /// The source map no longer matches the model.
    MappingStale {
        /// What the map says.
        detail: String,
    },
    /// The target profile cannot be debugged by this build.
    UnsupportedTarget {
        /// The profile or triple.
        target: String,
    },
    /// The debuggee stopped on an exception, which is a stop rather than a failure.
    Exception {
        /// What the adapter said.
        description: String,
    },
}

impl Incident {
    /// The incident in one line.
    pub fn render(&self) -> String {
        match self {
            Self::AdapterExit { code } => match code {
                Some(code) => format!("the debug adapter exited with {code}"),
                None => "the debug adapter exited".to_owned(),
            },
            Self::AdapterError { message } => format!("the debug adapter reported: {message}"),
            Self::MappingStale { detail } => format!("the debug source map is stale: {detail}"),
            Self::UnsupportedTarget { target } => {
                format!("{target} is not a debug target this build supports")
            }
            Self::Exception { description } => format!("the program stopped on {description}"),
        }
    }

    /// Whether this incident ends the session.
    pub fn ends_session(&self) -> bool {
        !matches!(self, Self::Exception { .. })
    }
}

/// What the host did after an incident.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Recovery {
    /// What happened.
    pub incident: Incident,
    /// Whether the session ended.
    pub session_ended: bool,
    /// Whether a process this session launched was terminated.
    pub owned_process_terminated: bool,
    /// Whether adapter handles, pipes and the reader thread were released.
    pub resources_released: bool,
    /// Whether the project can be edited again.
    pub project_editable: bool,
    /// Whether mapping-based navigation still works.
    pub mapping_usable: bool,
    /// What the person is told.
    pub detail: String,
}

impl Recovery {
    /// Whether nothing is left running for this session.
    pub fn is_clean(&self) -> bool {
        self.resources_released && self.project_editable
    }

    /// The recovery in one line.
    pub fn render(&self) -> String {
        format!(
            "{}; session {}; owned process {}; resources {}; project {}; mapping {}",
            self.incident.render(),
            if self.session_ended { "ended" } else { "kept" },
            if self.owned_process_terminated {
                "terminated"
            } else {
                "left alone"
            },
            if self.resources_released {
                "released"
            } else {
                "still held"
            },
            if self.project_editable {
                "editable"
            } else {
                "locked"
            },
            if self.mapping_usable {
                "usable"
            } else {
                "unusable"
            }
        )
    }
}

/// What to do after an incident.
///
/// `owns_process` is the session's own flag, not a caller's guess: only a launched process is
/// terminated. An exception keeps the session and its resources because the person may continue.
pub fn recover(incident: Incident, owns_process: bool) -> Recovery {
    if let Incident::Exception { description } = &incident {
        let detail =
            format!("the program stopped on {description}; inspect it and continue, step or stop");
        return Recovery {
            incident,
            session_ended: false,
            owned_process_terminated: false,
            resources_released: false,
            project_editable: true,
            mapping_usable: true,
            detail,
        };
    }
    let mapping_usable = !matches!(incident, Incident::MappingStale { .. });
    let detail = match &incident {
        Incident::AdapterExit { .. } | Incident::AdapterError { .. } => {
            "the session ended and its resources were released; the project is editable again"
                .to_owned()
        }
        Incident::MappingStale { .. } => {
            "the session ended; regenerate the source before navigating from a debug position"
                .to_owned()
        }
        Incident::UnsupportedTarget { target } => format!(
            "{target} cannot be debugged by this build; the session did not start and nothing is left running"
        ),
        Incident::Exception { .. } => unreachable!(),
    };
    Recovery {
        incident,
        session_ended: true,
        owned_process_terminated: owns_process,
        resources_released: true,
        project_editable: true,
        mapping_usable,
        detail,
    }
}

#[cfg(test)]
mod tests {
    use super::{Incident, recover};

    #[test]
    fn an_adapter_exit_ends_the_session_and_releases_what_it_owned() {
        let recovery = recover(Incident::AdapterExit { code: Some(9) }, true);
        assert!(recovery.session_ended);
        assert!(recovery.owned_process_terminated);
        assert!(recovery.is_clean());
        assert!(recovery.render().contains("exited with 9"));
    }

    #[test]
    fn an_attached_process_is_left_alone_when_the_adapter_dies() {
        let recovery = recover(
            Incident::AdapterError {
                message: "the adapter closed the connection".to_owned(),
            },
            false,
        );
        assert!(recovery.session_ended);
        assert!(!recovery.owned_process_terminated);
        assert!(recovery.project_editable);
    }

    #[test]
    fn a_stale_map_ends_the_session_and_disables_navigation() {
        let recovery = recover(
            Incident::MappingStale {
                detail: "the document is at revision 8".to_owned(),
            },
            true,
        );
        assert!(recovery.session_ended);
        assert!(!recovery.mapping_usable);
        assert!(recovery.detail.contains("regenerate"));
    }

    #[test]
    fn an_unsupported_target_never_started_anything() {
        let recovery = recover(
            Incident::UnsupportedTarget {
                target: "embedded".to_owned(),
            },
            false,
        );
        assert!(recovery.session_ended);
        assert!(!recovery.owned_process_terminated);
        assert!(recovery.is_clean());
        assert!(
            recovery.render().contains("nothing is left running")
                || recovery.detail.contains("nothing is left running")
        );
    }

    #[test]
    fn an_exception_is_a_stop_and_not_a_cleanup() {
        let recovery = recover(
            Incident::Exception {
                description: "attempt to divide by zero".to_owned(),
            },
            true,
        );
        assert!(!recovery.session_ended);
        assert!(!recovery.owned_process_terminated);
        assert!(recovery.project_editable);
        assert!(recovery.mapping_usable);
        assert!(recovery.render().contains("stopped on"));
    }
}
