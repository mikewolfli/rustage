// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The debug session: configuration, launch versus attach, phases and command legality
//! (tasks T19.02 and T19.04).
//!
//! The acceptance lines are *"attaching does not kill an external process"* and *"a command that
//! does not belong to the current state is refused, and the source version is visible"*. Both are
//! properties of this type:
//!
//! * [`Session::disconnect`] answers `terminate_debuggee = owns_process`. A launched process is this
//!   session's, so it is asked to end; an attached process belongs to somebody else, so the request
//!   says `false` and the adapter is told not to touch it. There is no path that sets the flag from a
//!   caller's wish.
//! * [`Session::permits`] is the state table. Continuing a running program, pausing a stopped one and
//!   inspecting before a launch all have their own refusal, so a UI can disable exactly the right
//!   command instead of showing a failure after the fact.
//! * [`Session::source_version`] is the model revision the generated source and the debug information
//!   were built from. It is printed with every stop; a version that moved is a stale session, not a
//!   silently current one.
//!
//! The session does not own the protocol client; the CLI handler does, and calls the transitions here
//! when the adapter reports what happened. That keeps the state rules testable without a process.

#![allow(dead_code)]

use std::path::PathBuf;

use crate::app::debug::adapter::Adapter;

/// Why the debugger stopped.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum StopReason {
    /// A breakpoint was hit.
    Breakpoint {
        /// The line the adapter reported.
        line: u32,
    },
    /// A step finished.
    Step,
    /// A pause was requested.
    Pause,
    /// The program stopped at its entry point.
    Entry,
    /// An exception, panic or signal stopped it.
    Exception {
        /// What the adapter said.
        description: String,
    },
    /// The adapter's own reason, which this build does not interpret further.
    Other {
        /// The adapter's word.
        reason: String,
    },
}

impl StopReason {
    /// Reads a DAP stop reason.
    pub fn parse(reason: &str, description: Option<&str>, line: u32) -> Self {
        match reason {
            "breakpoint" | "function breakpoint" | "data breakpoint" => Self::Breakpoint { line },
            "step" => Self::Step,
            "pause" => Self::Pause,
            "entry" => Self::Entry,
            "exception" | "signal" | "panic" => Self::Exception {
                description: description.unwrap_or(reason).to_owned(),
            },
            other => Self::Other {
                reason: other.to_owned(),
            },
        }
    }

    /// The reason in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Breakpoint { line } => format!("breakpoint at line {line}"),
            Self::Step => "step".to_owned(),
            Self::Pause => "pause".to_owned(),
            Self::Entry => "entry".to_owned(),
            Self::Exception { description } => format!("exception: {description}"),
            Self::Other { reason } => format!("stopped: {reason}"),
        }
    }
}

/// What a session is doing.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Phase {
    /// Nothing has started.
    Idle,
    /// A start was sent and the adapter has not answered yet.
    Starting,
    /// The program runs.
    Running,
    /// The program is stopped, with the thread it stopped on.
    Stopped {
        /// The reason.
        reason: StopReason,
        /// The thread, when the adapter names one.
        thread: Option<i64>,
    },
    /// The program ended by itself.
    Exited {
        /// Its exit code, when the adapter names one.
        code: Option<i64>,
    },
    /// The session was ended by a request or a failure.
    Ended,
}

impl Phase {
    /// The phase's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Stopped { .. } => "stopped",
            Self::Exited { .. } => "exited",
            Self::Ended => "ended",
        }
    }

    /// Whether a program is under the debugger and can be inspected.
    pub fn is_stopped(&self) -> bool {
        matches!(self, Self::Stopped { .. })
    }

    /// Whether a session exists at all.
    pub fn is_live(&self) -> bool {
        matches!(self, Self::Starting | Self::Running | Self::Stopped { .. })
    }
}

/// A command a person can give.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    /// Continue from a stop.
    Continue,
    /// Pause a running program.
    Pause,
    /// Step over.
    StepOver,
    /// Step into.
    StepIn,
    /// Step out.
    StepOut,
    /// Run to a source position.
    RunToCursor,
    /// Read threads, stack, scopes or variables.
    Inspect,
    /// Evaluate an expression.
    Evaluate,
    /// Add or change a breakpoint.
    Breakpoint,
    /// End the session.
    Disconnect,
}

impl Action {
    /// The DAP command a resume uses.
    pub fn dap_command(&self) -> Option<&'static str> {
        match self {
            Self::Continue | Self::RunToCursor => Some("continue"),
            Self::Pause => Some("pause"),
            Self::StepOver => Some("next"),
            Self::StepIn => Some("stepIn"),
            Self::StepOut => Some("stepOut"),
            Self::Disconnect => Some("disconnect"),
            Self::Inspect | Self::Evaluate | Self::Breakpoint => None,
        }
    }

    /// The action's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Continue => "continue",
            Self::Pause => "pause",
            Self::StepOver => "step over",
            Self::StepIn => "step in",
            Self::StepOut => "step out",
            Self::RunToCursor => "run to cursor",
            Self::Inspect => "inspect",
            Self::Evaluate => "evaluate",
            Self::Breakpoint => "breakpoint",
            Self::Disconnect => "disconnect",
        }
    }
}

/// Why a command is not legal now.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Refusal {
    /// There is no session.
    NoSession {
        /// The action.
        action: Action,
    },
    /// The program is already running.
    Running {
        /// The action.
        action: Action,
    },
    /// The program is not stopped.
    NotStopped {
        /// The action.
        action: Action,
        /// What the session is doing instead.
        phase: String,
    },
    /// The program is already stopped.
    AlreadyStopped {
        /// The action.
        action: Action,
    },
    /// The session ended.
    Ended,
    /// The session was never authorized.
    Unauthorized {
        /// The action.
        action: Action,
    },
    /// The adapter does not provide this capability.
    Unsupported {
        /// The action.
        action: Action,
        /// What is missing.
        reason: String,
    },
}

impl Refusal {
    /// The refusal in one line.
    pub fn render(&self) -> String {
        match self {
            Self::NoSession { action } => {
                format!("{} needs a debug session; none has started", action.name())
            }
            Self::Running { action } => {
                format!(
                    "{} is refused: the program is already running",
                    action.name()
                )
            }
            Self::NotStopped { action, phase } => {
                format!(
                    "{} needs a stopped program; the session is {phase}",
                    action.name()
                )
            }
            Self::AlreadyStopped { action } => {
                format!(
                    "{} is refused: the program is already stopped",
                    action.name()
                )
            }
            Self::Ended => "the debug session has ended".to_owned(),
            Self::Unauthorized { action } => format!(
                "{} was not authorized; authorize the session before it starts",
                action.name()
            ),
            Self::Unsupported { action, reason } => {
                format!(
                    "{} is not provided by this adapter ({reason})",
                    action.name()
                )
            }
        }
    }
}

/// How a session is configured.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Config {
    /// The program to launch, or the process id to attach to.
    pub target: String,
    /// The program's arguments.
    pub arguments: Vec<String>,
    /// The working directory.
    pub directory: Option<PathBuf>,
    /// Extra environment.
    pub environment: Vec<(String, String)>,
    /// Whether to stop at the entry point.
    pub stop_on_entry: bool,
    /// The model revision the generated source was built from.
    pub source_version: u64,
}

impl Config {
    /// A launch configuration for a program.
    pub fn launch(program: impl Into<String>) -> Self {
        Self {
            target: program.into(),
            arguments: Vec::new(),
            directory: None,
            environment: Vec::new(),
            stop_on_entry: false,
            source_version: 0,
        }
    }

    /// An attach configuration for a process id.
    pub fn attach(pid: u32) -> Self {
        Self {
            target: pid.to_string(),
            arguments: Vec::new(),
            directory: None,
            environment: Vec::new(),
            stop_on_entry: false,
            source_version: 0,
        }
    }
}

/// A debug session and its state.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Session {
    /// The adapter that drives it.
    pub adapter: String,
    /// Its configuration, once one was set.
    pub config: Option<Config>,
    /// What it is doing.
    pub phase: Phase,
    /// Whether a person allowed it to start.
    pub authorized: bool,
    /// Whether this session started the program.
    pub owns_process: bool,
    /// Whether it attached instead of launching.
    pub attached: bool,
}

impl Session {
    /// A session with an adapter and nothing started.
    pub fn new(adapter: &Adapter) -> Self {
        Self {
            adapter: adapter.name.clone(),
            config: None,
            phase: Phase::Idle,
            authorized: false,
            owns_process: false,
            attached: false,
        }
    }

    /// Allows the session to start a process.
    pub fn authorize(&mut self) {
        self.authorized = true;
    }

    /// Sets the configuration, before anything starts.
    pub fn configure(&mut self, config: Config) -> Result<(), Refusal> {
        if self.phase != Phase::Idle {
            return Err(Refusal::Running {
                action: Action::Disconnect,
            });
        }
        self.config = Some(config);
        Ok(())
    }

    /// Marks a launch under way.
    pub fn launching(&mut self) -> Result<(), Refusal> {
        if !self.authorized {
            return Err(Refusal::Unauthorized {
                action: Action::Continue,
            });
        }
        if self.phase != Phase::Idle {
            return Err(Refusal::Running {
                action: Action::Continue,
            });
        }
        self.attached = false;
        self.owns_process = true;
        self.phase = Phase::Starting;
        Ok(())
    }

    /// Marks an attach under way.
    pub fn attaching(&mut self) -> Result<(), Refusal> {
        if !self.authorized {
            return Err(Refusal::Unauthorized {
                action: Action::Continue,
            });
        }
        if self.phase != Phase::Idle {
            return Err(Refusal::Running {
                action: Action::Continue,
            });
        }
        self.attached = true;
        // The process is somebody else's; ending the session must never terminate it.
        self.owns_process = false;
        self.phase = Phase::Starting;
        Ok(())
    }

    /// The program is running.
    pub fn resumed(&mut self) {
        if self.phase.is_live() {
            self.phase = Phase::Running;
        }
    }

    /// The program stopped.
    pub fn stopped(&mut self, reason: StopReason, thread: Option<i64>) {
        if self.phase.is_live() {
            self.phase = Phase::Stopped { reason, thread };
        }
    }

    /// The program ended by itself.
    pub fn exited(&mut self, code: Option<i64>) {
        self.phase = Phase::Exited { code };
    }

    /// The session was ended.
    pub fn ended(&mut self) {
        self.phase = Phase::Ended;
    }

    /// The source version this session's debug information belongs to.
    pub fn source_version(&self) -> u64 {
        self.config
            .as_ref()
            .map(|config| config.source_version)
            .unwrap_or(0)
    }

    /// What a disconnect request must tell the adapter.
    ///
    /// The flag is decided here rather than by a caller: a launched program is this session's and is
    /// asked to end, while an attached one is not and must be left running.
    pub fn disconnect(&self) -> Disconnect {
        Disconnect {
            terminate_debuggee: self.owns_process && !self.attached,
            reason: if self.attached {
                "detached from a process this session did not start".to_owned()
            } else {
                "ending the launched process".to_owned()
            },
        }
    }

    /// Whether a command is legal now.
    pub fn permits(&self, action: Action) -> Result<(), Refusal> {
        if self.phase == Phase::Ended {
            return Err(Refusal::Ended);
        }
        let refused_while_running = matches!(
            action,
            Action::Continue
                | Action::StepOver
                | Action::StepIn
                | Action::StepOut
                | Action::RunToCursor
                | Action::Inspect
                | Action::Evaluate
        );
        match &self.phase {
            Phase::Idle => Err(Refusal::NoSession { action }),
            Phase::Starting => Err(Refusal::NotStopped {
                action,
                phase: "starting".to_owned(),
            }),
            Phase::Running => {
                if refused_while_running {
                    Err(Refusal::Running { action })
                } else {
                    Ok(())
                }
            }
            Phase::Stopped { .. } => match action {
                Action::Pause => Err(Refusal::AlreadyStopped { action }),
                _ => Ok(()),
            },
            Phase::Exited { .. } => match action {
                Action::Disconnect | Action::Inspect => Ok(()),
                _ => Err(Refusal::NoSession { action }),
            },
            Phase::Ended => Err(Refusal::Ended),
        }
    }

    /// The session in one line.
    pub fn render(&self) -> String {
        let phase = match &self.phase {
            Phase::Stopped { reason, thread } => match thread {
                Some(thread) => format!("stopped ({}), thread {thread}", reason.render()),
                None => format!("stopped ({})", reason.render()),
            },
            Phase::Exited { code } => match code {
                Some(code) => format!("exited {code}"),
                None => "exited".to_owned(),
            },
            other => other.name().to_owned(),
        };
        let kind = if self.attached {
            "attached"
        } else if self.owns_process {
            "launched"
        } else {
            "not started"
        };
        format!(
            "session {}: {phase}; {kind}; source version {}",
            self.adapter,
            self.source_version()
        )
    }
}

/// What a disconnect request says.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Disconnect {
    /// Whether the adapter may terminate the debuggee.
    pub terminate_debuggee: bool,
    /// Why this value.
    pub reason: String,
}

#[cfg(test)]
mod tests {
    use super::{Action, Config, Phase, Refusal, Session, StopReason};
    use crate::app::debug::adapter::{Adapter, Capabilities, State};

    fn adapter() -> Adapter {
        Adapter {
            name: "lldb-dap".to_owned(),
            program: None,
            arguments: Vec::new(),
            license: "test".to_owned(),
            state: State::Missing {
                remedy: "test".to_owned(),
            },
            capabilities: Capabilities {
                launch: true,
                attach: true,
                conditional_breakpoints: true,
                logpoints: true,
                hit_conditions: true,
                evaluate: true,
                restart: false,
            },
        }
    }

    fn running() -> Session {
        let mut session = Session::new(&adapter());
        session
            .configure(Config::launch("/tmp/example"))
            .expect("a configuration");
        session.authorize();
        session.launching().expect("a launch");
        session.resumed();
        session
    }

    fn stopped() -> Session {
        let mut session = running();
        session.stopped(StopReason::Breakpoint { line: 12 }, Some(1));
        session
    }

    #[test]
    fn an_unauthorized_session_does_not_start() {
        let mut session = Session::new(&adapter());
        session
            .configure(Config::launch("/tmp/example"))
            .expect("a configuration");
        let refusal = session.launching().expect_err("no authorization");
        assert!(matches!(refusal, Refusal::Unauthorized { .. }));
    }

    #[test]
    fn a_launched_process_is_terminated_and_an_attached_one_is_not() {
        let launched = stopped();
        assert!(launched.disconnect().terminate_debuggee);
        let mut attached = Session::new(&adapter());
        attached
            .configure(Config::attach(4242))
            .expect("a configuration");
        attached.authorize();
        attached.attaching().expect("an attach");
        attached.stopped(StopReason::Entry, Some(1));
        assert!(!attached.disconnect().terminate_debuggee);
        assert!(attached.disconnect().reason.contains("did not start"));
    }

    #[test]
    fn commands_that_do_not_belong_to_the_phase_are_refused() {
        let running = running();
        assert!(matches!(
            running.permits(Action::Continue),
            Err(Refusal::Running { .. })
        ));
        assert!(running.permits(Action::Pause).is_ok());
        assert!(matches!(
            running.permits(Action::Inspect),
            Err(Refusal::Running { .. })
        ));

        let stopped = stopped();
        assert!(stopped.permits(Action::Continue).is_ok());
        assert!(stopped.permits(Action::Inspect).is_ok());
        assert!(matches!(
            stopped.permits(Action::Pause),
            Err(Refusal::AlreadyStopped { .. })
        ));

        let idle = Session::new(&adapter());
        assert!(matches!(
            idle.permits(Action::Continue),
            Err(Refusal::NoSession { .. })
        ));
        assert!(idle.permits(Action::Disconnect).is_err());
    }

    #[test]
    fn an_exited_session_only_allows_cleanup_and_inspection_of_what_it_has() {
        let mut session = stopped();
        session.exited(Some(0));
        assert!(session.permits(Action::Disconnect).is_ok());
        assert!(session.permits(Action::Inspect).is_ok());
        assert!(session.permits(Action::Continue).is_err());
        session.ended();
        assert!(matches!(
            session.permits(Action::Disconnect),
            Err(Refusal::Ended)
        ));
    }

    #[test]
    fn the_source_version_is_visible_with_the_session() {
        let mut session = Session::new(&adapter());
        let mut config = Config::launch("/tmp/example");
        config.source_version = 17;
        session.configure(config).expect("a configuration");
        assert_eq!(session.source_version(), 17);
        assert!(session.render().contains("source version 17"));
    }

    #[test]
    fn a_stop_reason_names_an_exception_rather_than_a_breakpoint() {
        let reason = StopReason::parse("exception", Some("attempt to add with overflow"), 0);
        assert_eq!(reason.render(), "exception: attempt to add with overflow");
        assert_eq!(
            Phase::Stopped {
                reason,
                thread: Some(3)
            }
            .name(),
            "stopped"
        );
    }
}
