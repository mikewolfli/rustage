// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Running the built program, independent of the designer (task T16.04).
//!
//! The acceptance line is *"the program runs on its own, and a stop names its reason"*.
//!
//! * **The configuration is what a shell would use.** [`Config`] holds the program, its arguments, its
//!   environment and its directory, and [`Config::command`] is the one spelling of the argv, so the
//!   run the designer starts and the run a person types are the same line.
//! * **The program really runs.** [`Run::start`] starts it through
//!   [`OwnedProcess`](crate::host::subprocess::OwnedProcess), which leads a process group of its own,
//!   and keeps the process handle: [`Run::stop`] signals that group and [`Run::wait`] records how the
//!   process really ended. No state is set to "running" without a process behind it.
//! * **Independence is a flag, not a hope.** [`Config::standalone`] records that the program is the
//!   delivered artifact and carries no designer session; a config that asked for a debug port the
//!   designer owns is refused rather than started (principle 15). [`Artifact`] keeps a preview and a
//!   compiled artifact apart, and a preview is refused as a standalone artifact.
//! * **A stop says why.** [`Reason`] distinguishes a clean exit, a non-zero code, a signal and an
//!   explicit stop, so "it ended" is never mistaken for "it finished well". A requested stop that
//!   loses the race to a real exit keeps the real exit.

#![allow(dead_code)]

use std::time::Duration;

use crate::contracts::permission::{EffectClass, Principal};
use crate::host::permission::{PermissionService, Scope};
use crate::host::subprocess::{DEFAULT_GRACE, ExitInfo, OwnedProcess, ProcessSpec, Termination};

/// Whether the program is the designer's live preview or the compiled artifact.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Artifact {
    /// The designer's preview process; it belongs to a designer session.
    Preview,
    /// An artifact produced by a build; it can be delivered and run on its own.
    Compiled,
}

impl Artifact {
    /// The artifact's name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Preview => "preview",
            Self::Compiled => "compiled",
        }
    }
}

/// How a run was configured.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Config {
    /// The program to run.
    pub program: String,
    /// Its arguments, in order.
    pub args: Vec<String>,
    /// Environment variables to set, `(name, value)` in order.
    pub environment: Vec<(String, String)>,
    /// The working directory, when it is not the caller's.
    pub directory: Option<String>,
    /// Whether this is the delivered artifact with no designer session attached.
    pub standalone: bool,
    /// Whether the program is a preview or a compiled artifact.
    pub artifact: Artifact,
}

impl Config {
    /// The argv for this config, program first.
    pub fn command(&self) -> Vec<String> {
        let mut argv = vec![self.program.clone()];
        argv.extend(self.args.iter().cloned());
        argv
    }

    /// The config in one line.
    pub fn render(&self) -> String {
        format!(
            "{}{}",
            self.program,
            if self.args.is_empty() {
                String::new()
            } else {
                format!(" {}", self.args.join(" "))
            },
        )
    }
}

/// Why a run ended.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Reason {
    /// It exited zero.
    Clean,
    /// It exited non-zero.
    NonZero {
        /// The exit code.
        code: i32,
    },
    /// It was killed by a signal.
    Signalled {
        /// The signal number.
        signal: i32,
    },
    /// It ended without either a code or a signal, so the host cannot say why.
    NoStatus,
    /// Someone asked it to stop.
    Stopped,
}

impl Reason {
    /// Derives the reason from how the process really ended.
    pub fn from_exit(exit: &ExitInfo) -> Self {
        match (exit.code, exit.signal) {
            (Some(0), _) => Self::Clean,
            (Some(code), _) => Self::NonZero { code },
            (None, Some(signal)) => Self::Signalled { signal },
            (None, None) => Self::NoStatus,
        }
    }

    /// The reason in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Clean => "exited cleanly".to_owned(),
            Self::NonZero { code } => format!("exited with code {code}"),
            Self::Signalled { signal } => format!("killed by signal {signal}"),
            Self::NoStatus => "ended without an exit code or a signal".to_owned(),
            Self::Stopped => "stopped on request".to_owned(),
        }
    }
}

/// Where a run is in its lifecycle.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    /// Nothing is running.
    Idle,
    /// The program is running.
    Running,
    /// The program ended; [`Run::reason`] says why.
    Ended,
}

/// One program's run state, its owned process, and the reason it last ended.
#[derive(Debug)]
pub struct Run {
    /// The configuration.
    pub config: Config,
    /// The current state.
    pub state: State,
    /// The reason it last ended, for the current or previous run.
    pub reason: Option<Reason>,
    /// The process this run owns; `None` once it has ended or before it starts.
    process: Option<OwnedProcess>,
}

impl Run {
    /// A run that has not started.
    pub fn new(config: Config) -> Self {
        Self {
            config,
            state: State::Idle,
            reason: None,
            process: None,
        }
    }

    /// Starts the program.
    pub fn start(&mut self) -> Result<(), String> {
        if self.state == State::Running {
            // A process that already ended is not still running: notice that before refusing.
            if self.wait(Duration::ZERO).is_none() {
                return Err("the program is already running".to_owned());
            }
        }
        if self.config.program.trim().is_empty() {
            return Err("no program to run".to_owned());
        }
        if self.config.artifact == Artifact::Preview && self.config.standalone {
            // A preview is the designer's session; calling it a delivered artifact would hide the
            // session it depends on.
            return Err(
                "a preview belongs to the designer session; it cannot be a standalone artifact"
                    .to_owned(),
            );
        }
        if self.config.standalone && !self.config.environment.is_empty() {
            // A standalone run may set its own environment, but a variable that names a designer
            // session would tie the delivered artifact to the host that built it.
            if self
                .config
                .environment
                .iter()
                .any(|(name, _)| name.starts_with("RUSTAGE_"))
            {
                return Err(
                    "a standalone run cannot depend on a designer session (a RUSTAGE_* variable \
                     is set)"
                        .to_owned(),
                );
            }
        }

        // The person asked this host to run a program; that is the session's own decision, not
        // project code fetched from somewhere else.
        let permit = PermissionService::new()
            .permit(EffectClass::Command, Scope::Session, Principal::Human)
            .map_err(|refusal| refusal.message())?;
        let mut spec = ProcessSpec::new(self.config.render(), self.config.program.as_str())
            .args(self.config.args.iter().cloned());
        if let Some(directory) = &self.config.directory {
            spec = spec.in_directory(directory);
        }
        for (name, value) in &self.config.environment {
            spec = spec.env(name, value);
        }
        let process = OwnedProcess::spawn(spec, &permit)
            .map_err(|error| format!("{} could not be started: {error}", self.config.render()))?;

        self.process = Some(process);
        self.state = State::Running;
        self.reason = None;
        Ok(())
    }

    /// Waits up to `limit` for the program to end, recording its real exit.
    ///
    /// Returns the reason when it has ended, and `None` when it is still running at the limit.
    pub fn wait(&mut self, limit: Duration) -> Option<Reason> {
        if self.state != State::Running {
            return self.reason.clone();
        }
        let exit = self.process.as_mut()?.wait_for(limit)?;
        // It has ended; dropping the handle now is safe because `wait_for` reaped it.
        self.process = None;
        let reason = Reason::from_exit(&exit);
        self.ended(reason.clone());
        Some(reason)
    }

    /// Whether the program has ended since the last look, without waiting.
    pub fn poll(&mut self) -> Option<Reason> {
        self.wait(Duration::ZERO)
    }

    /// Records that the program ended with `reason`.
    fn ended(&mut self, reason: Reason) {
        self.state = State::Ended;
        self.reason = Some(reason);
    }

    /// Stops the program on request.
    pub fn stop(&mut self) -> Result<(), String> {
        if self.state != State::Running {
            return Err("nothing is running".to_owned());
        }
        let Some(mut process) = self.process.take() else {
            // A run marked running without a process would be a defect; do not claim a stop.
            self.state = State::Ended;
            self.reason = Some(Reason::NoStatus);
            return Err("the run has no process to stop".to_owned());
        };
        match process.terminate(DEFAULT_GRACE) {
            Termination::SignalFailed { reason } => {
                // It may still be running, so keep the handle and the running state.
                self.process = Some(process);
                Err(format!("the program could not be signalled: {reason}"))
            }
            // The request lost the race to a real exit: keep the real reason.
            Termination::AlreadyExited { exit } => {
                self.ended(Reason::from_exit(&exit));
                Ok(())
            }
            // The signal is what ended it. "Stopped" names the request; no exit code is invented.
            Termination::Stopped { .. }
            | Termination::Killed { .. }
            | Termination::DirectOnly { .. } => {
                self.ended(Reason::Stopped);
                Ok(())
            }
        }
    }

    /// Stops if running, then starts again; the new run clears the old reason.
    pub fn restart(&mut self) -> Result<(), String> {
        if self.state == State::Running {
            self.stop()?;
        }
        self.start()
    }

    /// The operating system's pid while the program is owned by this run.
    pub fn pid(&self) -> Option<u32> {
        self.process.as_ref().map(OwnedProcess::pid)
    }

    /// Whether this run currently owns a live process.
    pub fn is_running(&self) -> bool {
        self.state == State::Running && self.process.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::{Artifact, Config, Reason, Run, State};
    #[cfg(unix)]
    use crate::host::subprocess::{ProcessPresence, presence};
    #[cfg(unix)]
    use std::time::{Duration, Instant};

    fn config() -> Config {
        Config {
            program: "target/debug/demo".to_owned(),
            args: vec!["--headless".to_owned()],
            environment: Vec::new(),
            directory: None,
            standalone: true,
            artifact: Artifact::Compiled,
        }
    }

    /// A real, short program for the lifecycle tests.
    #[cfg(unix)]
    fn shell(script: &str) -> Config {
        Config {
            program: "sh".to_owned(),
            args: vec!["-c".to_owned(), script.to_owned()],
            environment: Vec::new(),
            directory: None,
            standalone: false,
            artifact: Artifact::Preview,
        }
    }

    #[cfg(unix)]
    fn wait_until_gone(pid: u32, limit: Duration) -> ProcessPresence {
        let deadline = Instant::now() + limit;
        loop {
            let seen = presence(pid);
            if seen != ProcessPresence::Running || Instant::now() >= deadline {
                return seen;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn the_command_is_the_program_then_its_arguments() {
        assert_eq!(config().command(), vec!["target/debug/demo", "--headless"],);
    }

    #[cfg(unix)]
    #[test]
    fn a_run_reports_the_real_clean_and_non_zero_exits() {
        let mut clean = Run::new(shell("exit 0"));
        clean.start().expect("a start");
        assert_eq!(clean.state, State::Running);
        assert_eq!(clean.wait(Duration::from_secs(10)), Some(Reason::Clean));
        assert_eq!(clean.state, State::Ended);
        assert_eq!(clean.reason, Some(Reason::Clean));

        let mut failed = Run::new(shell("exit 3"));
        failed.start().expect("a start");
        assert_eq!(
            failed.wait(Duration::from_secs(10)),
            Some(Reason::NonZero { code: 3 }),
        );
        assert_eq!(failed.reason, Some(Reason::NonZero { code: 3 }));
    }

    #[cfg(unix)]
    #[test]
    fn a_stop_signals_the_owned_process_and_leaves_nothing_behind() {
        let mut run = Run::new(shell("sleep 30"));
        run.start().expect("a start");
        let pid = run.pid().expect("a real pid");
        assert!(run.is_running());

        let began = Instant::now();
        run.stop().expect("a stop");
        assert_eq!(run.reason, Some(Reason::Stopped));
        assert_eq!(run.state, State::Ended);
        assert!(
            began.elapsed() < Duration::from_secs(10),
            "a stop must not wait for the program: {:?}",
            began.elapsed(),
        );
        assert!(!run.is_running());
        assert_eq!(
            wait_until_gone(pid, Duration::from_secs(5)),
            ProcessPresence::Gone,
            "the program outlived its stop",
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_program_that_kills_itself_reports_the_real_signal() {
        let mut run = Run::new(shell("kill -TERM $$"));
        run.start().expect("a start");
        let reason = run.wait(Duration::from_secs(10)).expect("an end");
        assert!(
            matches!(reason, Reason::Signalled { signal: 15 }),
            "{reason:?}",
        );
    }

    #[test]
    fn a_standalone_run_cannot_depend_on_the_designer() {
        let mut run = Run::new(config());
        run.config
            .environment
            .push(("RUSTAGE_PREVIEW".to_owned(), "1".to_owned()));
        let error = run.start().expect_err("a designer dependency");
        assert!(error.contains("standalone"), "{error}");
        assert_eq!(run.state, State::Idle);
    }

    #[test]
    fn a_preview_is_not_a_standalone_artifact() {
        let mut run = Run::new(config());
        run.config.artifact = Artifact::Preview;
        let error = run.start().expect_err("a preview cannot be standalone");
        assert!(error.contains("preview"), "{error}");
        assert_eq!(run.state, State::Idle);
    }

    #[test]
    fn a_program_that_does_not_exist_is_refused_rather_than_marked_running() {
        let mut run = Run::new(config());
        let error = run.start().expect_err("nothing to start");
        assert!(error.contains("could not be started"), "{error}");
        assert_eq!(run.state, State::Idle);
        assert_eq!(run.pid(), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_stop_names_its_reason_and_a_restart_clears_it() {
        let mut run = Run::new(shell("sleep 30"));
        run.start().expect("a start");
        let first = run.pid().expect("a pid");
        run.stop().expect("a stop");
        assert_eq!(run.reason, Some(Reason::Stopped));

        run.restart().expect("a restart");
        assert_eq!(run.state, State::Running);
        assert_eq!(run.reason, None);
        let second = run.pid().expect("a pid");
        assert_ne!(first, second);
        run.stop().expect("a final stop");
    }

    #[test]
    fn stopping_nothing_is_refused_rather_than_reported_as_success() {
        let mut run = Run::new(config());
        assert!(run.stop().is_err());
    }
}
