// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The shared business core: an event and a state become a state and actions, through one seam that
//! is a value rather than a live channel (task T17.01).
//!
//! The acceptance line is *"the core does not depend on desktop IPC, a watcher or a JSON service"*.
//! That is a property of what the core is allowed to touch, so the design removes the ability to
//! touch any of them:
//!
//! * **A reduction is a pure function.** [`Core::reduce`] takes a [`State`] and an [`Event`] and
//!   answers a [`Step`]; it reads and writes no file, opens no socket and holds no handle. The only
//!   imports in this module are `std::collections`.
//! * **The seam is a request, not a call.** When a transition needs a fact the core does not hold, it
//!   declares a [`Need`] and the reduction stops there. [`drive`] resolves the need from a [`Seam`] the
//!   caller supplied, so the core itself never reaches out — the same reduction runs whether the
//!   answer comes from the compiled-in static path or the replaceable process.
//! * **The payload is typed.** [`Value`] is a small closed enum, not free-form JSON, so nothing here
//!   depends on a document/JSON service to interpret an event.
//! * **The same core is both seams.** A compiled-in caller and the swapped-in logic process drive the
//!   same [`Core`] with the same events; there is no second, drifting engine (principle 35).

#![allow(dead_code)]

use std::collections::BTreeMap;

/// A typed value an event carries or a state field holds.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Value {
    /// A yes/no value.
    Flag(bool),
    /// A whole number.
    Number(i64),
    /// A text value.
    Text(String),
}

impl Value {
    /// The value's type name.
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Flag(_) => "flag",
            Self::Number(_) => "number",
            Self::Text(_) => "text",
        }
    }

    /// The value in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Flag(value) => value.to_string(),
            Self::Number(value) => value.to_string(),
            Self::Text(value) => value.clone(),
        }
    }

    /// Reads a value written as `flag:yes`, `number:3` or `text:hello`.
    pub fn parse(text: &str) -> Option<Self> {
        let (kind, rest) = text.split_once(':')?;
        match kind {
            "flag" => match rest {
                "yes" | "true" => Some(Self::Flag(true)),
                "no" | "false" => Some(Self::Flag(false)),
                _ => None,
            },
            "number" => rest.parse().ok().map(Self::Number),
            "text" => Some(Self::Text(rest.to_owned())),
            _ => None,
        }
    }
}

/// One input event.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Event {
    /// The event's name.
    pub name: String,
    /// What it carries.
    pub payload: Value,
}

impl Event {
    /// A bare event with a text-free payload.
    pub fn new(name: impl Into<String>, payload: Value) -> Self {
        Self {
            name: name.into(),
            payload,
        }
    }

    /// The event in one line.
    pub fn render(&self) -> String {
        format!("{} ({})", self.name, self.payload.render())
    }
}

/// The business state a reduction works from and produces.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct State {
    /// The state's name.
    pub name: String,
    /// The state's fields.
    pub fields: BTreeMap<String, Value>,
}

impl State {
    /// A named state with no fields.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            fields: BTreeMap::new(),
        }
    }
}

/// When a transition may fire.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Guard {
    /// Always fires.
    Always,
    /// Fires when a field holds exactly this value.
    Field {
        /// The field.
        field: String,
        /// The value it must hold.
        equals: Value,
    },
    /// Fires when a field is absent.
    Absent {
        /// The field.
        field: String,
    },
}

impl Guard {
    /// Whether the guard holds for a state.
    pub fn holds(&self, state: &State) -> bool {
        match self {
            Self::Always => true,
            Self::Field { field, equals } => state.fields.get(field) == Some(equals),
            Self::Absent { field } => !state.fields.contains_key(field),
        }
    }
}

/// What a transition does when it fires.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Action {
    /// Write a value to a state field.
    Set {
        /// The field.
        field: String,
        /// The value to write.
        value: Value,
    },
    /// Emit another event.
    Emit {
        /// The event's name.
        event: String,
        /// What it carries.
        payload: Value,
    },
    /// Call a business function through the seam.
    Call {
        /// The function.
        function: String,
        /// What it is called with.
        argument: Value,
    },
}

impl Action {
    /// The action in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Set { field, value } => format!("set {field} = {}", value.render()),
            Self::Emit { event, payload } => format!("emit {event} ({})", payload.render()),
            Self::Call { function, argument } => {
                format!("call {function}({})", argument.render())
            }
        }
    }
}

/// A fact a transition needs from outside the core.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Need {
    /// The function to ask.
    pub function: String,
    /// What to ask it with.
    pub argument: Value,
    /// The state field the answer is written into.
    pub into: String,
}

impl Need {
    /// The need in one line.
    pub fn render(&self) -> String {
        format!(
            "need {} -> {} ({})",
            self.function,
            self.into,
            self.argument.render()
        )
    }
}

/// One transition of the core.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Transition {
    /// The state it fires from.
    pub state: String,
    /// The event it answers.
    pub event: String,
    /// When it may fire.
    pub guard: Guard,
    /// The state it moves to.
    pub next: String,
    /// What it does.
    pub actions: Vec<Action>,
    /// What it needs from outside, if anything.
    pub need: Option<Need>,
}

impl Transition {
    /// A transition from one state to another on an event, firing always and doing nothing.
    pub fn new(
        state: impl Into<String>,
        event: impl Into<String>,
        next: impl Into<String>,
    ) -> Self {
        Self {
            state: state.into(),
            event: event.into(),
            guard: Guard::Always,
            next: next.into(),
            actions: Vec::new(),
            need: None,
        }
    }
}

/// Why a reduction produced no step.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Refusal {
    /// No transition answers this event in this state.
    NoTransition {
        /// The state.
        state: String,
        /// The event.
        event: String,
    },
    /// A transition exists but its guard does not hold.
    GuardFailed {
        /// The state.
        state: String,
        /// The event.
        event: String,
    },
    /// Two transitions answer the same state and event.
    Duplicate {
        /// The state.
        state: String,
        /// The event.
        event: String,
    },
    /// A transition asked the seam something the caller did not answer.
    UnansweredSeam {
        /// The function that was asked.
        function: String,
    },
}

impl Refusal {
    /// The refusal in one line.
    pub fn render(&self) -> String {
        match self {
            Self::NoTransition { state, event } => {
                format!("{state} has no transition for {event}")
            }
            Self::GuardFailed { state, event } => {
                format!("the guard on {state}/{event} does not hold")
            }
            Self::Duplicate { state, event } => {
                format!("{state}/{event} already has a transition")
            }
            Self::UnansweredSeam { function } => {
                format!("the seam was asked for {function} and answered nothing")
            }
        }
    }
}

/// The reduction of one event.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Step {
    /// The state the reduction produced.
    pub state: State,
    /// What the transition did.
    pub actions: Vec<Action>,
    /// What it needs from outside, if anything.
    pub need: Option<Need>,
    /// The events it emitted.
    pub emitted: Vec<Event>,
}

impl Step {
    /// The step in one line per fact.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!("state {}", self.state.name)];
        for action in &self.actions {
            lines.push(action.render());
        }
        if let Some(need) = &self.need {
            lines.push(need.render());
        }
        for event in &self.emitted {
            lines.push(format!("emitted {}", event.render()));
        }
        lines
    }
}

/// The business core: a declared transition table reduced purely.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Core {
    /// The state the core starts in.
    pub initial: String,
    transitions: Vec<Transition>,
}

impl Core {
    /// A core starting in a state.
    pub fn new(initial: impl Into<String>) -> Self {
        Self {
            initial: initial.into(),
            transitions: Vec::new(),
        }
    }

    /// Adds a transition, refusing one that would double-answer a state and event.
    pub fn add(&mut self, transition: Transition) -> Result<(), Refusal> {
        if self
            .transitions
            .iter()
            .any(|known| known.state == transition.state && known.event == transition.event)
        {
            return Err(Refusal::Duplicate {
                state: transition.state,
                event: transition.event,
            });
        }
        self.transitions.push(transition);
        Ok(())
    }

    /// The state the core starts in.
    pub fn start(&self) -> State {
        State::new(self.initial.clone())
    }

    /// Reduces one event against a state.
    pub fn reduce(&self, state: &State, event: &Event) -> Result<Step, Refusal> {
        let transition = self
            .transitions
            .iter()
            .find(|known| known.state == state.name && known.event == event.name)
            .ok_or_else(|| Refusal::NoTransition {
                state: state.name.clone(),
                event: event.name.clone(),
            })?;
        if !transition.guard.holds(state) {
            return Err(Refusal::GuardFailed {
                state: state.name.clone(),
                event: event.name.clone(),
            });
        }
        let mut fields = state.fields.clone();
        let mut emitted = Vec::new();
        for action in &transition.actions {
            match action {
                Action::Set { field, value } => {
                    fields.insert(field.clone(), value.clone());
                }
                Action::Emit { event, payload } => {
                    emitted.push(Event::new(event.clone(), payload.clone()))
                }
                // A call is declared, not performed: the core names it and the caller decides.
                Action::Call { .. } => {}
            }
        }
        Ok(Step {
            state: State {
                name: transition.next.clone(),
                fields,
            },
            actions: transition.actions.clone(),
            need: transition.need.clone(),
            emitted,
        })
    }
}

/// The answers a caller has for the seam's requests, keyed by function.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Seam {
    answers: BTreeMap<String, Value>,
}

impl Seam {
    /// A seam that answers nothing.
    pub fn new() -> Self {
        Self::default()
    }

    /// The same seam with one more answer.
    pub fn with(mut self, function: impl Into<String>, value: Value) -> Self {
        self.answers.insert(function.into(), value);
        self
    }

    /// The answer for a function, if the caller has one.
    pub fn answer(&self, function: &str) -> Option<&Value> {
        self.answers.get(function)
    }
}

/// Drives events through the core, answering each seam request from `seam`.
///
/// A transition that declares a [`Need`] is answered from the seam and the answer is written into the
/// field the need named; a need with no answer stops the walk with [`Refusal::UnansweredSeam`]
/// rather than a half-applied state.
pub fn drive(
    core: &Core,
    start: &State,
    events: &[Event],
    seam: &Seam,
) -> Result<Vec<Step>, Refusal> {
    let mut state = start.clone();
    let mut steps = Vec::new();
    for event in events {
        let mut step = core.reduce(&state, event)?;
        if let Some(need) = step.need.clone() {
            let answer = seam
                .answer(&need.function)
                .cloned()
                .ok_or(Refusal::UnansweredSeam {
                    function: need.function.clone(),
                })?;
            step.state.fields.insert(need.into, answer);
        }
        state = step.state.clone();
        steps.push(step);
    }
    Ok(steps)
}

#[cfg(test)]
mod tests {
    use super::{Action, Core, Event, Guard, Need, Refusal, Seam, State, Transition, Value, drive};

    fn core() -> Core {
        // The core tests here are built the way a caller builds one: a declared table, no channel.
        let mut core = Core::new("idle");
        let mut start = Transition::new("idle", "start", "running");
        start.actions.push(Action::Set {
            field: "seen".to_owned(),
            value: Value::Number(1),
        });
        start.actions.push(Action::Emit {
            event: "started".to_owned(),
            payload: Value::Flag(true),
        });
        core.add(start).expect("a first transition");
        core
    }

    #[test]
    fn reducing_an_event_yields_a_state_and_actions() {
        let step = core()
            .reduce(&State::new("idle"), &Event::new("start", Value::Flag(true)))
            .expect("a step");
        assert_eq!(step.state.name, "running");
        assert_eq!(step.state.fields.get("seen"), Some(&Value::Number(1)));
        assert_eq!(step.actions.len(), 2);
        assert_eq!(step.emitted.len(), 1);
        assert_eq!(step.emitted[0].name, "started");
    }

    #[test]
    fn an_unknown_event_is_refused_by_name() {
        let refusal = core()
            .reduce(&State::new("idle"), &Event::new("stop", Value::Flag(true)))
            .expect_err("a refusal");
        assert_eq!(
            refusal,
            Refusal::NoTransition {
                state: "idle".to_owned(),
                event: "stop".to_owned(),
            }
        );
        assert!(refusal.render().contains("no transition"));
    }

    #[test]
    fn a_guard_that_does_not_hold_refuses_the_transition() {
        let mut core = Core::new("idle");
        let mut transition = Transition::new("idle", "go", "running");
        transition.guard = Guard::Field {
            field: "ready".to_owned(),
            equals: Value::Flag(true),
        };
        core.add(transition).expect("a transition");

        let mut state = State::new("idle");
        assert!(matches!(
            core.reduce(&state, &Event::new("go", Value::Flag(true))),
            Err(Refusal::GuardFailed { .. })
        ));
        state.fields.insert("ready".to_owned(), Value::Flag(true));
        assert!(
            core.reduce(&state, &Event::new("go", Value::Flag(true)))
                .is_ok()
        );
    }

    #[test]
    fn a_duplicate_transition_is_refused() {
        let mut core = core();
        let refusal = core
            .add(Transition::new("idle", "start", "elsewhere"))
            .expect_err("a duplicate");
        assert!(matches!(refusal, Refusal::Duplicate { .. }));
    }

    #[test]
    fn a_need_is_declared_and_answered_from_the_seam() {
        let mut core = Core::new("idle");
        let mut transition = Transition::new("idle", "load", "loaded");
        transition.need = Some(Need {
            function: "fetch".to_owned(),
            argument: Value::Text("order".to_owned()),
            into: "order".to_owned(),
        });
        core.add(transition).expect("a transition");

        let seam = Seam::new().with("fetch", Value::Text("A-1".to_owned()));
        let steps = drive(
            &core,
            &State::new("idle"),
            &[Event::new("load", Value::Flag(true))],
            &seam,
        )
        .expect("a walk");
        assert_eq!(steps[0].state.name, "loaded");
        assert_eq!(
            steps[0].state.fields.get("order"),
            Some(&Value::Text("A-1".to_owned()))
        );
    }

    #[test]
    fn an_unanswered_need_stops_the_walk() {
        let mut core = Core::new("idle");
        let mut transition = Transition::new("idle", "load", "loaded");
        transition.need = Some(Need {
            function: "fetch".to_owned(),
            argument: Value::Flag(true),
            into: "order".to_owned(),
        });
        core.add(transition).expect("a transition");

        let refusal = drive(
            &core,
            &State::new("idle"),
            &[Event::new("load", Value::Flag(true))],
            &Seam::new(),
        )
        .expect_err("a refusal");
        assert_eq!(
            refusal,
            Refusal::UnansweredSeam {
                function: "fetch".to_owned(),
            }
        );
    }

    #[test]
    fn the_core_reduces_with_no_channel_at_all() {
        // Nothing here constructs a service, a socket or a file: a core and two values are the whole
        // dependency, which is the acceptance line "the core does not depend on IPC, a watcher or a
        // JSON service" made visible.
        let core = core();
        let state = core.start();
        let step = core
            .reduce(&state, &Event::new("start", Value::Flag(true)))
            .expect("a step");
        assert_eq!(step.state.name, "running");
    }

    #[test]
    fn a_value_round_trips_through_its_written_form() {
        for value in [
            Value::Flag(false),
            Value::Number(-7),
            Value::Text("hello: world".to_owned()),
        ] {
            let text = format!("{}:{}", value.type_name(), value.render());
            assert_eq!(Value::parse(&text), Some(value));
        }
        assert_eq!(Value::parse("number:oops"), None);
        assert_eq!(Value::parse("nothing"), None);
    }

    #[test]
    fn a_step_renders_every_fact_it_carries() {
        let step = core()
            .reduce(&State::new("idle"), &Event::new("start", Value::Flag(true)))
            .expect("a step");
        let lines = step.render();
        assert!(lines.contains(&"state running".to_owned()));
        assert!(lines.iter().any(|line| line.starts_with("set seen")));
        assert!(lines.iter().any(|line| line.starts_with("emitted started")));
    }
}
