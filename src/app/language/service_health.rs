// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! A regression harness for a language service that fails (task T07.08).
//!
//! The acceptance line is *"basic editing still works, and a restart can be diagnosed"*. The danger a
//! language service brings is that its failure becomes the editor's failure, so every scenario here
//! turns the service off in some way and then checks that **the editor's own work still happened** and
//! that the service's state is **named** rather than silently gone:
//!
//! * **[`Scenario::Crash`]** — a running service dies; the editor still edits, and a restart reads
//!   back a diagnosis rather than a bare "stopped".
//! * **[`Scenario::Slow`]** — a reply arrives after the document moved on; it is dropped as stale, and
//!   the edit that overtook it is intact.
//! * **[`Scenario::NoToolchain`]** — the service cannot start at all; editing is unaffected, and a
//!   restart once a toolchain exists recovers.
//! * **[`Scenario::MultiProject`]** — two projects have their own sessions; one crashing leaves the
//!   other running, and closing one leaves the other.
//!
//! The checks are [`regression::Check`](crate::app::editor::regression::Check) — the same type
//! [T06.08](crate::app::editor::regression) uses, so "did this pass" is one vocabulary across both
//! harnesses.

#![allow(dead_code)]

use crate::app::editor::regression::Check;
use crate::app::language::features::{Delivery, Pending, Response};
use crate::app::language::session::{
    Capabilities, Capability, Launcher, Services, StartupFailure, State,
};
use crate::project::text::piece_table::TextStore;

/// One failure scenario.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scenario {
    /// The service dies while running.
    Crash,
    /// A reply arrives after the document moved on.
    Slow,
    /// No toolchain is installed.
    NoToolchain,
    /// Two projects, one failing.
    MultiProject,
}

impl Scenario {
    /// Every scenario, in a stable order.
    pub const ALL: [Self; 4] = [
        Self::Crash,
        Self::Slow,
        Self::NoToolchain,
        Self::MultiProject,
    ];

    /// The scenario's name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Crash => "crash",
            Self::Slow => "slow",
            Self::NoToolchain => "no-toolchain",
            Self::MultiProject => "multi-project",
        }
    }

    /// The scenario a name picks.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|s| s.name() == name)
    }
}

/// One scenario's outcome.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Report {
    /// Which scenario ran.
    pub scenario: Scenario,
    /// The checks.
    pub checks: Vec<Check>,
}

impl Report {
    /// Whether every check passed.
    pub fn passed(&self) -> bool {
        self.checks.iter().all(|check| check.passed)
    }

    /// The report in lines.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "scenario {}: {}",
            self.scenario.name(),
            if self.passed() { "passed" } else { "FAILED" }
        )];
        for check in &self.checks {
            lines.push(check.render());
        }
        lines
    }
}

/// A launcher that answers from a fixed sequence of outcomes.
pub struct Scripted {
    outcomes: Vec<Result<Capabilities, StartupFailure>>,
    next: usize,
}

impl Scripted {
    /// A launcher over a sequence of launch outcomes.
    pub fn new(outcomes: Vec<Result<Capabilities, StartupFailure>>) -> Self {
        Self { outcomes, next: 0 }
    }
}

impl Launcher for Scripted {
    fn launch(&mut self, _project: &str) -> Result<Capabilities, StartupFailure> {
        let outcome = self
            .outcomes
            .get(self.next)
            .or_else(|| self.outcomes.last())
            .cloned()
            .unwrap_or_else(|| Ok(Capabilities::none()));
        self.next += 1;
        outcome
    }

    fn stop(&mut self, _project: &str) {}
}

/// Runs a scenario.
pub fn run(scenario: Scenario) -> Report {
    match scenario {
        Scenario::Crash => crash(),
        Scenario::Slow => slow(),
        Scenario::NoToolchain => no_toolchain(),
        Scenario::MultiProject => multi_project(),
    }
}

/// Runs every scenario.
pub fn run_all() -> Vec<Report> {
    Scenario::ALL.into_iter().map(run).collect()
}

/// Editing a small document, to prove the editor is independent of the service.
fn edit_still_works() -> bool {
    let mut store = TextStore::new("abc");
    store.splice(1, 0, "X").is_ok() && store.text() == "aXbc"
}

/// A service that dies: the editor edits, and a restart reads back a diagnosis.
fn crash() -> Report {
    // First launch runs; a later launch fails, as if the server died and cannot come back.
    let launcher = Scripted::new(vec![
        Ok(Capabilities::all()),
        Err(StartupFailure::HandshakeFailed(
            "the server exited".to_owned(),
        )),
    ]);
    let mut services = Services::new(launcher);
    let running = services.open_view("app").state().is_running();
    // The document is still editable while the service is down.
    let edited = edit_still_works();
    // A restart reports the handshake failure as a diagnosis.
    let restarted = services.restart("app");
    let diagnosed = restarted
        .diagnosis()
        .is_some_and(|failure| failure.message().contains("handshake"));
    Report {
        scenario: Scenario::Crash,
        checks: vec![
            Check::new("the service was running before the crash", running),
            Check::new("basic editing still works after the crash", edited),
            Check::new("the restart reads back a diagnosis", diagnosed),
        ],
    }
}

/// A reply that arrives after the document moved on: dropped, and the edit is intact.
fn slow() -> Report {
    let mut pending = Pending::new();
    let request = pending.request(1);
    let mut store = TextStore::new("abc");
    // The person types before the reply arrives.
    store.splice(0, 0, "Z").expect("an edit");
    let delivery = pending.deliver(Response::new(request, "stale completion"), 2);
    let dropped = matches!(
        delivery,
        Delivery::Stale {
            computed: 1,
            current: 2
        }
    );
    Report {
        scenario: Scenario::Slow,
        checks: vec![
            Check::new("a slow reply is dropped as stale", dropped),
            Check::new(
                "the edit that overtook it is intact",
                store.text() == "Zabc",
            ),
        ],
    }
}

/// No toolchain: the service cannot start, editing is unaffected, and a restart recovers.
fn no_toolchain() -> Report {
    let launcher = Scripted::new(vec![
        Err(StartupFailure::NoToolchain),
        Ok(Capabilities::of([Capability::Completion])),
    ]);
    let mut services = Services::new(launcher);
    let failed = services.state("app") == State::Stopped;
    let opened = matches!(
        services.open_view("app").state(),
        State::Failed(StartupFailure::NoToolchain)
    );
    let edited = edit_still_works();
    // A restart, once a toolchain appears, recovers.
    let recovered = services.restart("app").state().is_running();
    Report {
        scenario: Scenario::NoToolchain,
        checks: vec![
            Check::new("a project with no session is stopped", failed),
            Check::new("the start is diagnosed as no toolchain", opened),
            Check::new("basic editing still works", edited),
            Check::new("a restart after a toolchain appears recovers", recovered),
        ],
    }
}

/// Two projects: one failing leaves the other running, and closing one leaves the other.
fn multi_project() -> Report {
    // The first launch runs, the second fails, the third runs.
    let launcher = Scripted::new(vec![
        Ok(Capabilities::all()),
        Err(StartupFailure::NoToolchain),
        Ok(Capabilities::all()),
    ]);
    let mut services = Services::new(launcher);
    let first_running = services.open_view("first").state().is_running();
    let second_failed = matches!(
        services.open_view("second").state(),
        State::Failed(StartupFailure::NoToolchain)
    );
    // Closing the second leaves the first untouched.
    let closed = services.close("second");
    let first_survives = services
        .session("first")
        .is_some_and(|session| session.state().is_running());
    Report {
        scenario: Scenario::MultiProject,
        checks: vec![
            Check::new("the first project runs", first_running),
            Check::new(
                "the second project is diagnosed independently",
                second_failed,
            ),
            Check::new("closing one project releases only its session", closed),
            Check::new("the other project keeps running", first_survives),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::{Scenario, run, run_all};

    #[test]
    fn every_scenario_passes() {
        for report in run_all() {
            assert!(report.passed(), "{:?}", report.render());
        }
    }

    #[test]
    fn a_crash_leaves_editing_intact_and_the_restart_diagnosable() {
        let report = run(Scenario::Crash);
        assert!(report.passed(), "{:?}", report.render());
        assert_eq!(report.checks.len(), 3);
    }

    #[test]
    fn a_slow_reply_is_dropped() {
        let report = run(Scenario::Slow);
        assert!(report.passed(), "{:?}", report.render());
    }

    #[test]
    fn no_toolchain_is_diagnosed_and_recovered_from() {
        let report = run(Scenario::NoToolchain);
        assert!(report.passed(), "{:?}", report.render());
    }

    #[test]
    fn two_projects_are_independent() {
        let report = run(Scenario::MultiProject);
        assert!(report.passed(), "{:?}", report.render());
    }

    #[test]
    fn a_scenario_name_round_trips() {
        for scenario in Scenario::ALL {
            assert_eq!(Scenario::from_name(scenario.name()), Some(scenario));
        }
        assert_eq!(Scenario::from_name("nowhere"), None);
    }
}
