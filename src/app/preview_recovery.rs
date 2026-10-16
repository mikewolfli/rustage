// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! What happens when the preview app fails, crashes, or is reloaded, rebuilt or reset (task T14.07).
//!
//! The acceptance line is *"it shows that the edit and the running versions disagree, and does not
//! pretend to have succeeded"*. Two facts do the work:
//!
//! * **The edit version and the running version are two separate facts.** [`Preview`] keeps both, so
//!   "what the document is" and "what the instance is" can be compared instead of assumed equal — and
//!   an edit while the app runs leaves them visibly apart.
//! * **A recovery says whether it is in step.** [`Recovery::succeeded`] is only true when the instance
//!   is running *at the edit version*; a start that could not happen answers `life: down`,
//!   `running version: none` and a reason, so a failed recovery cannot be read as a good one.
//! * **The state's fate is named.** [`StateOutcome`] distinguishes kept, deliberately reset and lost,
//!   so a rebuild that dropped the live state says so rather than reporting a plain success.

#![allow(dead_code)]

/// How the preview app is doing.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub enum Life {
    /// It is up.
    Running,
    /// It ended on its own.
    Exited {
        /// The code it ended with.
        code: i32,
    },
    /// It died without ending cleanly.
    Crashed {
        /// The signal that killed it.
        signal: i32,
    },
    /// Nothing is up.
    #[default]
    Down,
}

impl Life {
    /// The life's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Exited { .. } => "exited",
            Self::Crashed { .. } => "crashed",
            Self::Down => "down",
        }
    }

    /// The life as one phrase.
    pub fn render(&self) -> String {
        match self {
            Self::Running => "running".to_owned(),
            Self::Exited { code } => format!("exited with code {code}"),
            Self::Crashed { signal } => format!("crashed on signal {signal}"),
            Self::Down => "down".to_owned(),
        }
    }
}

/// What was asked of the preview app.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    /// Start it for the first time.
    Launch,
    /// Restart it without recompiling.
    Reload,
    /// Recompile and restart it.
    Rebuild,
    /// Restart it with the live state dropped.
    Reset,
}

impl Action {
    /// The action's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Launch => "launch",
            Self::Reload => "reload",
            Self::Rebuild => "rebuild",
            Self::Reset => "reset",
        }
    }

    /// Reads an action's name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "launch" => Some(Self::Launch),
            "reload" => Some(Self::Reload),
            "rebuild" => Some(Self::Rebuild),
            "reset" => Some(Self::Reset),
            _ => None,
        }
    }
}

/// What became of the live state across the attempt.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum StateOutcome {
    /// It was carried over.
    Kept,
    /// It was deliberately dropped.
    Reset,
    /// It could not be preserved, and why.
    Lost {
        /// The reason.
        reason: String,
    },
}

impl StateOutcome {
    /// The outcome as one phrase.
    pub fn render(&self) -> String {
        match self {
            Self::Kept => "kept".to_owned(),
            Self::Reset => "reset".to_owned(),
            Self::Lost { reason } => format!("lost ({reason})"),
        }
    }
}

/// What one attempt achieved.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Recovery {
    /// What was asked.
    pub action: Action,
    /// The document's version at the time.
    pub edit_version: u32,
    /// The instance's version after the attempt, if it is up.
    pub running_version: Option<u32>,
    /// How the app is doing after the attempt.
    pub life: Life,
    /// What became of the live state.
    pub state: StateOutcome,
    /// Why the app could not start, if it could not.
    pub fault: Option<String>,
}

impl Recovery {
    /// Whether the instance is up and at the document's version.
    pub fn succeeded(&self) -> bool {
        matches!(self.life, Life::Running) && self.running_version == Some(self.edit_version)
    }

    /// Why the attempt did not succeed.
    pub fn reason(&self) -> String {
        if let Some(fault) = &self.fault {
            return format!("the preview app did not start: {fault}");
        }
        match self.running_version {
            None => "there is no running instance".to_owned(),
            Some(version) => format!(
                "the running instance is at v {version} and the document is at v {}",
                self.edit_version
            ),
        }
    }

    /// The attempt as lines: what happened, then both versions and the state's fate.
    pub fn render(&self) -> Vec<String> {
        let verdict = match (self.life.clone(), self.running_version) {
            (_, _) if self.succeeded() => "started".to_owned(),
            (Life::Exited { code }, _) => format!("the app exited with code {code}"),
            (Life::Crashed { signal }, _) => format!("the app crashed on signal {signal}"),
            _ => format!("not started — {}", self.reason()),
        };
        vec![
            format!("{}: {verdict}", self.action.name()),
            format!(
                "  life: {}; edit version: {}; running version: {}; in step: {}; state: {}",
                self.life.render(),
                self.edit_version,
                match self.running_version {
                    Some(version) => version.to_string(),
                    None => "none".to_owned(),
                },
                if self.succeeded() { "yes" } else { "no" },
                self.state.render()
            ),
        ]
    }
}

/// The preview app and where it stands next to the document.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Preview {
    edit_version: u32,
    running_version: Option<u32>,
    life: Life,
    fault: Option<String>,
    attempts: Vec<Recovery>,
}

impl Preview {
    /// A preview that has never started, at document version 0.
    pub fn new() -> Self {
        Self::default()
    }

    /// The document's version.
    pub fn edit_version(&self) -> u32 {
        self.edit_version
    }

    /// The instance's version, if it is up.
    pub fn running_version(&self) -> Option<u32> {
        self.running_version
    }

    /// How the app is doing.
    pub fn life(&self) -> &Life {
        &self.life
    }

    /// The declared reason the app cannot start, if one is declared.
    pub fn fault(&self) -> Option<&str> {
        self.fault.as_deref()
    }

    /// Every attempt, in order.
    pub fn attempts(&self) -> &[Recovery] {
        &self.attempts
    }

    /// Moves the document's version.
    ///
    /// The running instance is deliberately **not** moved: that is the whole point of keeping the two
    /// versions apart, and it is what an edit to a running preview actually does.
    pub fn edit(&mut self, version: u32) {
        self.edit_version = version;
    }

    /// Whether the running instance is at the document's version.
    pub fn in_step(&self) -> bool {
        matches!(self.life, Life::Running) && self.running_version == Some(self.edit_version)
    }

    /// Declares why the app cannot start (a build error, a missing capability).
    pub fn declare_fault(&mut self, reason: impl Into<String>) {
        self.fault = Some(reason.into());
    }

    /// Says the app can start again.
    pub fn clear_fault(&mut self) {
        self.fault = None;
    }

    /// The app ends on its own.
    pub fn exit(&mut self, code: i32) {
        self.life = Life::Exited { code };
        self.running_version = None;
    }

    /// The app dies without ending cleanly.
    pub fn crash(&mut self, signal: i32) {
        self.life = Life::Crashed { signal };
        self.running_version = None;
    }

    /// Starts, restarts or resets the app, and answers what happened.
    fn attempt(&mut self, action: Action, state: StateOutcome) -> Recovery {
        let recovery = match &self.fault {
            Some(reason) => {
                self.life = Life::Down;
                self.running_version = None;
                Recovery {
                    action,
                    edit_version: self.edit_version,
                    running_version: None,
                    life: Life::Down,
                    state: StateOutcome::Lost {
                        reason: reason.clone(),
                    },
                    fault: Some(reason.clone()),
                }
            }
            None => {
                self.life = Life::Running;
                self.running_version = Some(self.edit_version);
                let state = match action {
                    Action::Reset => StateOutcome::Reset,
                    _ => state,
                };
                Recovery {
                    action,
                    edit_version: self.edit_version,
                    running_version: self.running_version,
                    life: Life::Running,
                    state,
                    fault: None,
                }
            }
        };
        self.attempts.push(recovery.clone());
        recovery
    }

    /// Starts the app for the first time.
    pub fn launch(&mut self) -> Recovery {
        self.attempt(Action::Launch, StateOutcome::Kept)
    }

    /// Restarts the app without recompiling, keeping the live state.
    pub fn reload(&mut self) -> Recovery {
        self.attempt(Action::Reload, StateOutcome::Kept)
    }

    /// Recompiles and restarts the app, keeping the live state.
    pub fn rebuild(&mut self) -> Recovery {
        self.attempt(Action::Rebuild, StateOutcome::Kept)
    }

    /// Restarts the app with the live state dropped.
    pub fn reset(&mut self) -> Recovery {
        self.attempt(Action::Reset, StateOutcome::Reset)
    }

    /// Where the preview stands, in one line per fact.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "life: {}; edit version: {}; running version: {}; in step: {}",
            self.life.render(),
            self.edit_version,
            match self.running_version {
                Some(version) => version.to_string(),
                None => "none".to_owned(),
            },
            if self.in_step() { "yes" } else { "no" }
        )];
        if !self.in_step() {
            lines.push(match self.running_version {
                Some(version) => format!(
                    "out of step: the running instance is at v {version} and the document is at v {}",
                    self.edit_version
                ),
                None => "out of step: nothing is running".to_owned(),
            });
        }
        if let Some(fault) = &self.fault {
            lines.push(format!("fault: {fault}"));
        }
        lines.push(format!("attempts: {}", self.attempts.len()));
        lines.extend(self.attempts.iter().flat_map(|recovery| {
            recovery
                .render()
                .into_iter()
                .map(|line| format!("  {line}"))
        }));
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{Action, Life, Preview, StateOutcome};

    #[test]
    fn a_launch_puts_the_instance_at_the_document_version() {
        let mut preview = Preview::new();
        preview.edit(7);
        let recovery = preview.launch();
        assert!(recovery.succeeded());
        assert_eq!(recovery.running_version, Some(7));
        assert_eq!(recovery.state, StateOutcome::Kept);
        assert!(preview.in_step());
    }

    #[test]
    fn an_edit_while_the_app_runs_leaves_the_two_versions_apart() {
        let mut preview = Preview::new();
        preview.edit(7);
        preview.launch();
        preview.edit(8);
        assert_eq!(preview.edit_version(), 8);
        assert_eq!(preview.running_version(), Some(7));
        assert!(!preview.in_step());
        let lines = preview.render();
        assert_eq!(
            lines[0],
            "life: running; edit version: 8; running version: 7; in step: no"
        );
        assert_eq!(
            lines[1],
            "out of step: the running instance is at v 7 and the document is at v 8"
        );
    }

    #[test]
    fn a_reload_moves_the_running_instance_to_the_document_version() {
        let mut preview = Preview::new();
        preview.edit(7);
        preview.launch();
        preview.edit(8);
        let recovery = preview.reload();
        assert!(recovery.succeeded());
        assert_eq!(recovery.action, Action::Reload);
        assert_eq!(recovery.running_version, Some(8));
        assert!(preview.in_step());
    }

    #[test]
    fn a_rebuild_moves_the_running_instance_to_the_document_version() {
        let mut preview = Preview::new();
        preview.edit(7);
        preview.launch();
        preview.edit(9);
        let recovery = preview.rebuild();
        assert_eq!(recovery.action, Action::Rebuild);
        assert_eq!(recovery.running_version, Some(9));
        assert_eq!(recovery.state, StateOutcome::Kept);
        assert!(recovery.succeeded());
    }

    #[test]
    fn a_reset_drops_the_state_and_still_lands_in_step() {
        let mut preview = Preview::new();
        preview.edit(7);
        preview.launch();
        preview.edit(9);
        let recovery = preview.reset();
        assert_eq!(recovery.action, Action::Reset);
        assert_eq!(recovery.state, StateOutcome::Reset);
        assert!(recovery.succeeded());
    }

    #[test]
    fn a_launch_with_a_fault_stays_down_and_does_not_succeed() {
        let mut preview = Preview::new();
        preview.edit(8);
        preview.declare_fault("cargo: no such target");
        let recovery = preview.launch();
        assert!(!recovery.succeeded());
        assert_eq!(recovery.life, Life::Down);
        assert_eq!(recovery.running_version, None);
        assert_eq!(
            recovery.state,
            StateOutcome::Lost {
                reason: "cargo: no such target".to_owned()
            }
        );
        assert!(!preview.in_step());
    }

    #[test]
    fn a_reload_with_a_fault_does_not_pretend_to_have_recovered() {
        let mut preview = Preview::new();
        preview.edit(8);
        preview.launch();
        preview.edit(9);
        preview.declare_fault("the preview binary is missing");
        let recovery = preview.reload();
        assert!(!recovery.succeeded());
        assert_eq!(recovery.running_version, None);
        assert_eq!(
            recovery.reason(),
            "the preview app did not start: the preview binary is missing"
        );
        assert_eq!(
            recovery.render()[0],
            "reload: not started — the preview app did not start: the preview binary is missing"
        );
        assert_eq!(preview.render()[1], "out of step: nothing is running");
    }

    #[test]
    fn clearing_the_fault_lets_the_next_reload_start() {
        let mut preview = Preview::new();
        preview.edit(8);
        preview.declare_fault("cargo: no such target");
        preview.launch();
        preview.clear_fault();
        let recovery = preview.reload();
        assert!(recovery.succeeded());
        assert_eq!(recovery.fault, None);
        assert!(preview.in_step());
    }

    #[test]
    fn a_crash_is_recorded_with_its_signal_and_a_recovery_after_it_succeeds() {
        let mut preview = Preview::new();
        preview.edit(4);
        preview.launch();
        preview.crash(11);
        assert_eq!(*preview.life(), Life::Crashed { signal: 11 });
        assert_eq!(preview.running_version(), None);
        let lines = preview.render();
        assert_eq!(
            lines[0],
            "life: crashed on signal 11; edit version: 4; running version: none; in step: no"
        );
        let recovery = preview.reload();
        assert!(recovery.succeeded());
        assert_eq!(recovery.running_version, Some(4));
    }

    #[test]
    fn an_exit_is_recorded_with_its_code() {
        let mut preview = Preview::new();
        preview.edit(2);
        preview.launch();
        preview.exit(101);
        assert_eq!(*preview.life(), Life::Exited { code: 101 });
        let recovery = preview.rebuild();
        assert_eq!(recovery.action, Action::Rebuild);
        assert!(recovery.succeeded());
    }

    #[test]
    fn the_recovery_line_names_both_versions_and_the_in_step_answer() {
        let mut preview = Preview::new();
        preview.edit(3);
        let recovery = preview.launch();
        assert_eq!(
            recovery.render()[1],
            "  life: running; edit version: 3; running version: 3; in step: yes; state: kept"
        );
    }

    #[test]
    fn the_history_records_every_attempt() {
        let mut preview = Preview::new();
        preview.edit(1);
        preview.launch();
        preview.edit(2);
        preview.reload();
        preview.reset();
        assert_eq!(preview.attempts().len(), 3);
        let lines = preview.render();
        assert!(lines.contains(&"attempts: 3".to_owned()));
    }

    #[test]
    fn a_fault_is_shown_in_the_summary() {
        let mut preview = Preview::new();
        preview.declare_fault("the preview binary is missing");
        assert_eq!(preview.fault(), Some("the preview binary is missing"));
        assert!(
            preview
                .render()
                .contains(&"fault: the preview binary is missing".to_owned())
        );
    }

    #[test]
    fn the_life_and_the_action_parse_and_render() {
        assert_eq!(Life::default(), Life::Down);
        assert_eq!(Action::parse("reload"), Some(Action::Reload));
        assert_eq!(Action::parse("restart"), None);
        assert_eq!(Action::Reset.name(), "reset");
        assert_eq!(Life::Crashed { signal: 6 }.render(), "crashed on signal 6");
        assert_eq!(Life::Exited { code: 0 }.render(), "exited with code 0");
    }
}
