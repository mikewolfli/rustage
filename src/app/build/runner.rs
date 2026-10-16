// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Running the built program, independent of the designer (task T16.04).
//!
//! The acceptance line is *"the program runs on its own, and a stop names its reason"*.
//!
//! * **The configuration is what a shell would use.** [`Config`] holds the program, its arguments, its
//!   environment and its directory, and [`Config::command`] is the one spelling of the argv, so the
//!   run the designer starts and the run a person types are the same line.
//! * **Independence is a flag, not a hope.** [`Config::standalone`] records that the program is the
//!   delivered artifact and carries no designer session; a config that asked for a debug port the
//!   designer owns is refused rather than started (principle 15).
//! * **A stop says why.** [`Reason`] distinguishes a clean exit, a non-zero code, a signal and an
//!   explicit stop, so "it ended" is never mistaken for "it finished well".

#![allow(dead_code)]

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
    /// Someone asked it to stop.
    Stopped,
}

impl Reason {
    /// The reason in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Clean => "exited cleanly".to_owned(),
            Self::NonZero { code } => format!("exited with code {code}"),
            Self::Signalled { signal } => format!("killed by signal {signal}"),
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

/// One program's run state and the reason it last ended.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Run {
    /// The configuration.
    pub config: Config,
    /// The current state.
    pub state: State,
    /// The reason it last ended, for the current or previous run.
    pub reason: Option<Reason>,
}

impl Run {
    /// A run that has not started.
    pub fn new(config: Config) -> Self {
        Self {
            config,
            state: State::Idle,
            reason: None,
        }
    }

    /// Starts the program.
    pub fn start(&mut self) -> Result<(), String> {
        if self.state == State::Running {
            return Err("the program is already running".to_owned());
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
        self.state = State::Running;
        self.reason = None;
        Ok(())
    }

    /// Records that the program ended with `reason`.
    pub fn ended(&mut self, reason: Reason) {
        self.state = State::Ended;
        self.reason = Some(reason);
    }

    /// Stops the program on request.
    pub fn stop(&mut self) -> Result<(), String> {
        if self.state != State::Running {
            return Err("nothing is running".to_owned());
        }
        self.ended(Reason::Stopped);
        Ok(())
    }

    /// Stops if running, then starts again; the new run clears the old reason.
    pub fn restart(&mut self) -> Result<(), String> {
        if self.state == State::Running {
            self.stop()?;
        }
        self.start()
    }
}

#[cfg(test)]
mod tests {
    use super::{Config, Reason, Run, State};

    fn config() -> Config {
        Config {
            program: "target/debug/demo".to_owned(),
            args: vec!["--headless".to_owned()],
            environment: Vec::new(),
            directory: None,
            standalone: true,
        }
    }

    #[test]
    fn the_command_is_the_program_then_its_arguments() {
        assert_eq!(config().command(), vec!["target/debug/demo", "--headless"],);
    }

    #[test]
    fn a_run_goes_idle_to_running_to_ended_with_a_reason() {
        let mut run = Run::new(config());
        assert_eq!(run.state, State::Idle);
        run.start().expect("a start");
        assert_eq!(run.state, State::Running);
        run.ended(Reason::NonZero { code: 3 });
        assert_eq!(run.state, State::Ended);
        assert_eq!(run.reason, Some(Reason::NonZero { code: 3 }));
        assert!(run.reason.unwrap().render().contains("3"));
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
    fn a_stop_names_its_reason_and_a_restart_clears_it() {
        let mut run = Run::new(config());
        run.start().expect("a start");
        run.stop().expect("a stop");
        assert_eq!(run.reason, Some(Reason::Stopped));
        run.restart().expect("a restart");
        assert_eq!(run.state, State::Running);
        assert_eq!(run.reason, None);
    }

    #[test]
    fn stopping_nothing_is_refused_rather_than_reported_as_success() {
        let mut run = Run::new(config());
        assert!(run.stop().is_err());
    }
}
