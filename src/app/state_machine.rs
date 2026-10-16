// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! An explicit state machine: states, guarded transitions, the inspector, and the boundary that keeps
//! a synchronous event from becoming async IPC (task T13.06).
//!
//! The acceptance line is *"a synchronous event is not arbitrarily turned into async IPC"*. The
//! boundary is therefore a parameter, not an accident:
//!
//! * **Every guard answer says how it was obtained.** [`Answer`] carries the value *and* whether
//!   getting it needed the seam ([`Interaction`]), so a guard that reads only the design is not
//!   indistinguishable from one that made a request.
//! * **A [`Mode::Sync`] send refuses an async answer.** [`send`] answers `Err` when a synchronous
//!   path would have to consult an async seam, so a synchronous event cannot be quietly rerouted
//!   through IPC; only an explicitly [`Mode::Async`] send performs it, and it records that it did.
//! * **The machine is explicit.** States, transitions and final states are declared, and
//!   [`Machine::unreachable`] and [`Machine::dead_ends`] report states nothing reaches and states
//!   that can be entered but not left, rather than leaving them implicit.
//! * **A refusal names the event.** No transition, no passing guard and a missing guard answer are
//!   three different answers, so a non-step is diagnosable.

#![allow(dead_code)]

use std::collections::BTreeMap;

/// How a guard was evaluated.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Interaction {
    /// Read locally, in the same call.
    Sync,
    /// Needed the seam, which answered asynchronously.
    Async,
}

impl Interaction {
    /// The interaction's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Sync => "sync",
            Self::Async => "async",
        }
    }
}

/// The seam's answer to a guard: its value, and how it was obtained.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Answer {
    /// Whether the guard held.
    pub value: bool,
    /// How the answer was obtained.
    pub interaction: Interaction,
}

/// How a send is performed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// In the same call, consulting no async seam.
    Sync,
    /// May consult the async seam.
    Async,
}

impl Mode {
    /// The mode's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Sync => "sync",
            Self::Async => "async",
        }
    }
}

/// One guarded transition.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Transition {
    /// The state it leaves.
    pub from: String,
    /// The event that triggers it.
    pub on: String,
    /// The state it enters.
    pub to: String,
    /// The guard that must pass, if any.
    pub guard: Option<String>,
}

/// The declared states and transitions.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Machine {
    /// The declared states.
    pub states: Vec<String>,
    /// The state a session starts in.
    pub initial: Option<String>,
    /// The states a session may end in.
    pub finals: Vec<String>,
    /// The transitions, in declaration order.
    pub transitions: Vec<Transition>,
}

impl Machine {
    /// A machine with nothing declared.
    pub fn new() -> Self {
        Self::default()
    }

    /// The same machine with one more state.
    pub fn with_state(mut self, state: impl Into<String>) -> Self {
        self.states.push(state.into());
        self
    }

    /// The same machine with an initial state.
    pub fn starting_at(mut self, state: impl Into<String>) -> Self {
        self.initial = Some(state.into());
        self
    }

    /// The same machine with one more final state.
    pub fn final_state(mut self, state: impl Into<String>) -> Self {
        self.finals.push(state.into());
        self
    }

    /// The same machine with one more transition.
    pub fn with_transition(
        mut self,
        from: impl Into<String>,
        on: impl Into<String>,
        to: impl Into<String>,
        guard: Option<&str>,
    ) -> Self {
        self.transitions.push(Transition {
            from: from.into(),
            on: on.into(),
            to: to.into(),
            guard: guard.map(str::to_owned),
        });
        self
    }

    /// Whether a state is declared.
    pub fn has_state(&self, state: &str) -> bool {
        self.states.iter().any(|declared| declared == state)
    }

    /// Whether a state is final.
    pub fn is_final(&self, state: &str) -> bool {
        self.finals.iter().any(|final_state| final_state == state)
    }

    /// The states nothing reaches from the initial state, in declaration order.
    pub fn unreachable(&self) -> Vec<String> {
        let Some(initial) = &self.initial else {
            return self.states.clone();
        };
        let mut reached = vec![initial.clone()];
        let mut index = 0;
        while index < reached.len() {
            let state = reached[index].clone();
            index += 1;
            for transition in &self.transitions {
                if transition.from == state && !reached.contains(&transition.to) {
                    reached.push(transition.to.clone());
                }
            }
        }
        self.states
            .iter()
            .filter(|state| !reached.contains(state))
            .cloned()
            .collect()
    }

    /// The states that can be entered but not left, and are not final.
    pub fn dead_ends(&self) -> Vec<String> {
        self.states
            .iter()
            .filter(|state| !self.is_final(state))
            .filter(|state| {
                !self
                    .transitions
                    .iter()
                    .any(|transition| &transition.from == *state)
            })
            .cloned()
            .collect()
    }

    /// The machine in one line per fact.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "states: {}; initial: {}; finals: {}",
            self.states.len(),
            self.initial.as_deref().unwrap_or("(none)"),
            self.finals.len()
        )];
        for transition in &self.transitions {
            lines.push(format!(
                "  {} --{}--> {}{}",
                transition.from,
                transition.on,
                transition.to,
                match &transition.guard {
                    Some(guard) => format!(" [{guard}]"),
                    None => String::new(),
                }
            ));
        }
        for state in self.unreachable() {
            lines.push(format!("unreachable: {state}"));
        }
        for state in self.dead_ends() {
            lines.push(format!("dead end: {state}"));
        }
        lines
    }
}

/// One session's position in a machine.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Session {
    /// The state it is in.
    pub current: String,
}

impl Session {
    /// Starts a session in the machine's initial state.
    pub fn start(machine: &Machine) -> Result<Self, String> {
        match &machine.initial {
            Some(initial) if machine.has_state(initial) => Ok(Self {
                current: initial.clone(),
            }),
            Some(initial) => Err(format!("the initial state {initial:?} is not declared")),
            None => Err("the machine declares no initial state".to_owned()),
        }
    }
}

/// What a send did.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Step {
    /// The state left.
    pub from: String,
    /// The event.
    pub event: String,
    /// The state entered.
    pub to: String,
    /// The guard that passed, if any.
    pub guard: Option<String>,
    /// How the guard was evaluated.
    pub interaction: Interaction,
}

impl Step {
    /// Whether the step consulted the async seam.
    pub fn is_async(&self) -> bool {
        self.interaction == Interaction::Async
    }

    /// The step in one line.
    pub fn render(&self) -> String {
        format!(
            "{} --{}--> {} ({})",
            self.from,
            self.event,
            self.to,
            self.interaction.name()
        )
    }
}

/// Sends an event to a session, taking the first transition whose guard passes.
///
/// In [`Mode::Sync`] a guard answered [`Interaction::Async`] is refused, so a synchronous event is
/// never quietly routed through async IPC; in [`Mode::Async`] it is allowed and the step records it.
pub fn send(
    machine: &Machine,
    session: &mut Session,
    event: &str,
    answers: &BTreeMap<String, Answer>,
    mode: Mode,
) -> Result<Step, String> {
    let candidates: Vec<&Transition> = machine
        .transitions
        .iter()
        .filter(|transition| transition.from == session.current && transition.on == event)
        .collect();
    if candidates.is_empty() {
        return Err(format!(
            "no transition from {:?} on {event:?}",
            session.current
        ));
    }
    for transition in candidates {
        let interaction = match &transition.guard {
            None => Interaction::Sync,
            Some(guard) => {
                let answer = answers
                    .get(guard)
                    .ok_or_else(|| format!("no answer was supplied for the guard {guard:?}"))?;
                if answer.interaction == Interaction::Async && mode == Mode::Sync {
                    return Err(format!(
                        "a synchronous send cannot consult the async seam for the guard {guard:?}"
                    ));
                }
                if !answer.value {
                    continue;
                }
                answer.interaction
            }
        };
        let step = Step {
            from: session.current.clone(),
            event: event.to_owned(),
            to: transition.to.clone(),
            guard: transition.guard.clone(),
            interaction,
        };
        session.current = transition.to.clone();
        return Ok(step);
    }
    Err(format!(
        "no transition from {:?} on {event:?} passed its guard",
        session.current
    ))
}

#[cfg(test)]
mod tests {
    use super::{Answer, Interaction, Machine, Mode, Session, send};
    use std::collections::BTreeMap;

    fn machine() -> Machine {
        Machine::new()
            .with_state("idle")
            .with_state("loading")
            .with_state("done")
            .with_state("failed")
            .with_state("orphan")
            .starting_at("idle")
            .final_state("done")
            .final_state("failed")
            .with_transition("idle", "start", "loading", None)
            .with_transition("loading", "finish", "done", None)
            .with_transition("loading", "fail", "failed", None)
    }

    fn answers(entries: &[(&str, bool, Interaction)]) -> BTreeMap<String, Answer> {
        entries
            .iter()
            .map(|(name, value, interaction)| {
                (
                    (*name).to_owned(),
                    Answer {
                        value: *value,
                        interaction: *interaction,
                    },
                )
            })
            .collect()
    }

    #[test]
    fn a_transition_without_a_guard_always_fires() {
        let machine = machine();
        let mut session = Session::start(&machine).expect("a session");
        let step =
            send(&machine, &mut session, "start", &answers(&[]), Mode::Sync).expect("a step");
        assert_eq!(step.from, "idle");
        assert_eq!(step.to, "loading");
        assert_eq!(step.interaction, Interaction::Sync);
        assert_eq!(session.current, "loading");
    }

    #[test]
    fn a_transition_with_a_passing_guard_fires() {
        let machine = Machine::new()
            .with_state("idle")
            .with_state("done")
            .starting_at("idle")
            .with_transition("idle", "go", "done", Some("ready"));
        let mut session = Session::start(&machine).expect("a session");
        let answers = answers(&[("ready", true, Interaction::Sync)]);
        let step = send(&machine, &mut session, "go", &answers, Mode::Sync).expect("a step");
        assert_eq!(step.guard.as_deref(), Some("ready"));
    }

    #[test]
    fn a_failing_guard_is_not_taken() {
        let machine = Machine::new()
            .with_state("idle")
            .with_state("done")
            .starting_at("idle")
            .with_transition("idle", "go", "done", Some("ready"));
        let mut session = Session::start(&machine).expect("a session");
        let answers = answers(&[("ready", false, Interaction::Sync)]);
        let outcome = send(&machine, &mut session, "go", &answers, Mode::Sync);
        assert!(outcome.unwrap_err().contains("passed its guard"));
        assert_eq!(session.current, "idle");
    }

    #[test]
    fn a_later_transition_is_taken_when_an_earlier_guard_fails() {
        let machine = Machine::new()
            .with_state("idle")
            .with_state("fast")
            .with_state("slow")
            .starting_at("idle")
            .with_transition("idle", "go", "fast", Some("cached"))
            .with_transition("idle", "go", "slow", None);
        let mut session = Session::start(&machine).expect("a session");
        let answers = answers(&[("cached", false, Interaction::Sync)]);
        let step = send(&machine, &mut session, "go", &answers, Mode::Sync).expect("a step");
        assert_eq!(step.to, "slow");
    }

    #[test]
    fn an_event_with_no_transition_is_refused_and_named() {
        let machine = machine();
        let mut session = Session::start(&machine).expect("a session");
        let outcome = send(&machine, &mut session, "finish", &answers(&[]), Mode::Sync);
        assert!(outcome.unwrap_err().contains("no transition from \"idle\""));
    }

    #[test]
    fn a_synchronous_send_refuses_an_async_guard() {
        let machine = Machine::new()
            .with_state("idle")
            .with_state("done")
            .starting_at("idle")
            .with_transition("idle", "go", "done", Some("remote"));
        let mut session = Session::start(&machine).expect("a session");
        let answers = answers(&[("remote", true, Interaction::Async)]);
        let outcome = send(&machine, &mut session, "go", &answers, Mode::Sync);
        assert!(
            outcome
                .unwrap_err()
                .contains("cannot consult the async seam")
        );
        assert_eq!(session.current, "idle");
    }

    #[test]
    fn an_async_send_accepts_the_async_guard_and_records_it() {
        let machine = Machine::new()
            .with_state("idle")
            .with_state("done")
            .starting_at("idle")
            .with_transition("idle", "go", "done", Some("remote"));
        let mut session = Session::start(&machine).expect("a session");
        let answers = answers(&[("remote", true, Interaction::Async)]);
        let step = send(&machine, &mut session, "go", &answers, Mode::Async).expect("a step");
        assert!(step.is_async());
        assert!(step.render().contains("(async)"));
        assert_eq!(session.current, "done");
    }

    #[test]
    fn a_missing_guard_answer_is_refused() {
        let machine = Machine::new()
            .with_state("idle")
            .with_state("done")
            .starting_at("idle")
            .with_transition("idle", "go", "done", Some("ready"));
        let mut session = Session::start(&machine).expect("a session");
        let outcome = send(&machine, &mut session, "go", &answers(&[]), Mode::Sync);
        assert!(outcome.unwrap_err().contains("no answer was supplied"));
    }

    #[test]
    fn a_state_cycle_is_fine() {
        let machine = Machine::new()
            .with_state("a")
            .with_state("b")
            .starting_at("a")
            .final_state("a")
            .with_transition("a", "next", "b", None)
            .with_transition("b", "next", "a", None);
        let mut session = Session::start(&machine).expect("a session");
        send(&machine, &mut session, "next", &answers(&[]), Mode::Sync).expect("a step");
        send(&machine, &mut session, "next", &answers(&[]), Mode::Sync).expect("a step");
        assert_eq!(session.current, "a");
    }

    #[test]
    fn a_state_nothing_reaches_is_reported() {
        assert_eq!(machine().unreachable(), vec!["orphan"]);
    }

    #[test]
    fn a_non_final_dead_end_is_reported_and_a_final_state_is_not() {
        // "loading" is neither final nor left once it is entered by the transitions above? It is,
        // so add an explicit dead end.
        let machine = Machine::new()
            .with_state("idle")
            .with_state("stuck")
            .with_state("done")
            .starting_at("idle")
            .final_state("done")
            .with_transition("idle", "go", "stuck", None)
            .with_transition("idle", "finish", "done", None);
        assert_eq!(machine.dead_ends(), vec!["stuck"]);
        assert!(!machine.dead_ends().contains(&"done".to_owned()));
    }

    #[test]
    fn a_session_cannot_start_without_a_declared_initial_state() {
        let machine = Machine::new().with_state("idle");
        assert!(
            Session::start(&machine)
                .unwrap_err()
                .contains("no initial state")
        );
        let machine = Machine::new().with_state("idle").starting_at("gone");
        assert!(
            Session::start(&machine)
                .unwrap_err()
                .contains("not declared")
        );
    }

    #[test]
    fn the_machine_renders_states_transitions_and_diagnostics() {
        let lines = machine().render();
        assert_eq!(lines[0], "states: 5; initial: idle; finals: 2");
        assert!(lines.iter().any(|line| line == "  idle --start--> loading"));
        assert!(lines.iter().any(|line| line == "unreachable: orphan"));
    }

    #[test]
    fn a_guarded_transition_is_rendered_with_its_guard() {
        let machine = Machine::new()
            .with_state("idle")
            .with_state("done")
            .starting_at("idle")
            .with_transition("idle", "go", "done", Some("ready"));
        assert!(
            machine
                .render()
                .iter()
                .any(|line| line == "  idle --go--> done [ready]")
        );
    }
}
