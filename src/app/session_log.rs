// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The joint acceptance for data, events and navigation: event/state vectors, the two-way loop that
//! must settle, and the log that keeps a human edit undoable and a business side effect not (T13.08).
//!
//! The acceptance line is *"a human edit can be undone, and a business side effect is not claimed to
//! be undoable"*. The log is built around that one distinction:
//!
//! * **An entry says who made it.** [`Origin::Human`] or [`Origin::Business`], so the two are never
//!   confused once recorded.
//! * **Undo refuses a side effect.** [`Log::undo`] answers `Err` when the most recent entry is a
//!   business side effect, because undoing the designer's bookkeeping would not undo what happened
//!   out in the world — so it is refused rather than pretended (principle 22).
//! * **A vector is run, not asserted from memory.** [`run_vector`] sends each event through the
//!   machine and records what state it reached, so a mismatch names the step that diverged.
//! * **A two-way loop must settle.** [`Loop::settle`] runs the two sides until they agree or the round
//!   limit is hit, so an oscillating binding is a reported fact rather than a silent spin.

#![allow(dead_code)]

use std::collections::BTreeMap;

use crate::app::state_machine::{Answer, Machine, Mode, Session, send};

/// Who made a change.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Origin {
    /// A person, through the designer.
    Human,
    /// A business action, out in the world.
    Business,
}

impl Origin {
    /// The origin's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Business => "business",
        }
    }

    /// Whether a change of this origin can be undone here.
    pub fn is_undoable(&self) -> bool {
        matches!(self, Self::Human)
    }
}

/// One recorded change.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Entry {
    /// What changed, in a phrase.
    pub label: String,
    /// Who made it.
    pub origin: Origin,
    /// The value before.
    pub before: String,
    /// The value after.
    pub after: String,
}

impl Entry {
    /// The entry in one line.
    pub fn render(&self) -> String {
        format!(
            "[{}] {}: {} -> {}{}",
            self.origin.name(),
            self.label,
            self.before,
            self.after,
            if self.origin.is_undoable() {
                String::new()
            } else {
                " (not undoable)".to_owned()
            }
        )
    }
}

/// What undoing did.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Undone {
    /// The entry that was taken back.
    pub entry: Entry,
}

impl Undone {
    /// The undo in one line.
    pub fn render(&self) -> String {
        format!("undid {}: back to {}", self.entry.label, self.entry.before)
    }
}

/// The changes a session recorded, and which of them can be taken back.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Log {
    entries: Vec<Entry>,
    undone: Vec<Entry>,
}

impl Log {
    /// An empty log.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a change.
    pub fn record(
        &mut self,
        label: impl Into<String>,
        origin: Origin,
        before: impl Into<String>,
        after: impl Into<String>,
    ) {
        self.entries.push(Entry {
            label: label.into(),
            origin,
            before: before.into(),
            after: after.into(),
        });
    }

    /// The entries, oldest first.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// The entries already taken back, most recent last.
    pub fn undone(&self) -> &[Entry] {
        &self.undone
    }

    /// Takes back the most recent entry, if it is a human edit.
    ///
    /// A business side effect is refused: what happened out in the world is not in this log's gift to
    /// undo, and saying otherwise would be a claim the designer cannot keep.
    pub fn undo(&mut self) -> Result<Undone, String> {
        let Some(last) = self.entries.last() else {
            return Err("nothing has been recorded".to_owned());
        };
        if !last.origin.is_undoable() {
            return Err(format!(
                "{} is a business side effect, which cannot be undone here",
                last.label
            ));
        }
        let entry = self.entries.pop().expect("the entry is there");
        self.undone.push(entry.clone());
        Ok(Undone { entry })
    }

    /// The log in one line per entry, then what was taken back.
    pub fn render(&self) -> Vec<String> {
        let mut lines: Vec<String> = self.entries.iter().map(Entry::render).collect();
        for entry in &self.undone {
            lines.push(format!("undone: {}", entry.render()));
        }
        lines.push(format!(
            "recorded: {}, undone: {}",
            self.entries.len(),
            self.undone.len()
        ));
        lines
    }
}

/// One step of an event/state vector: an event, and the state it should reach.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Step {
    /// The event sent.
    pub event: String,
    /// The state the vector expects.
    pub expected: String,
    /// The state actually reached.
    pub actual: String,
}

/// Runs an event/state vector against a machine, answering what each step reached.
pub fn run_vector(
    machine: &Machine,
    session: &mut Session,
    steps: &[(String, String)],
    answers: &BTreeMap<String, Answer>,
    mode: Mode,
) -> Result<Vec<Step>, String> {
    let mut taken = Vec::new();
    for (event, expected) in steps {
        let step = send(machine, session, event, answers, mode)?;
        if &step.to != expected {
            return Err(format!(
                "after {event:?} the machine is at {:?}, but the vector expects {expected:?}",
                step.to
            ));
        }
        taken.push(Step {
            event: event.clone(),
            expected: expected.clone(),
            actual: step.to.clone(),
        });
    }
    Ok(taken)
}

/// How a two-way loop carries a value in each direction.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Transform {
    /// Both directions copy the value verbatim, so the sides agree at once.
    Identity,
    /// Each direction appends its own marker, so the sides never agree.
    Drifting,
}

/// A two-way binding's two sides.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Loop {
    /// One side.
    pub left: String,
    /// The other side.
    pub right: String,
    /// How each direction carries the value.
    pub transform: Transform,
}

/// How a loop ended.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Settled {
    /// The sides agreed, after this many rounds.
    Fixed {
        /// The rounds taken.
        rounds: u32,
    },
    /// The sides never agreed within the limit.
    Oscillating {
        /// The limit that was reached.
        limit: u32,
    },
}

impl Settled {
    /// Whether the loop settled.
    pub fn is_fixed(&self) -> bool {
        matches!(self, Self::Fixed { .. })
    }

    /// The result in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Fixed { rounds } => format!("settled after {rounds} round(s)"),
            Self::Oscillating { limit } => {
                format!("did not settle within {limit} round(s)")
            }
        }
    }
}

impl Loop {
    /// A loop with the value on the left and nothing on the right.
    pub fn new(value: impl Into<String>, transform: Transform) -> Self {
        Self {
            left: value.into(),
            right: String::new(),
            transform,
        }
    }

    /// One round: the left carries to the right, then the right carries back.
    fn round(&mut self) -> bool {
        let before = (self.left.clone(), self.right.clone());
        match self.transform {
            Transform::Identity => {
                self.right = self.left.clone();
                self.left = self.right.clone();
            }
            Transform::Drifting => {
                self.right = format!("{}-r", self.left);
                self.left = format!("{}-l", self.right);
            }
        }
        (self.left.clone(), self.right.clone()) != before
    }

    /// Runs the loop until the sides agree or the limit is reached.
    pub fn settle(&mut self, limit: u32) -> Settled {
        for round in 1..=limit {
            if !self.round() {
                return Settled::Fixed { rounds: round };
            }
        }
        Settled::Oscillating { limit }
    }
}

#[cfg(test)]
mod tests {
    use super::{Log, Loop, Origin, Settled, Transform, run_vector};
    use crate::app::state_machine::{Machine, Session};
    use std::collections::BTreeMap;

    fn machine() -> Machine {
        Machine::new()
            .with_state("idle")
            .with_state("loading")
            .with_state("done")
            .starting_at("idle")
            .final_state("done")
            .with_transition("idle", "start", "loading", None)
            .with_transition("loading", "finish", "done", None)
    }

    fn steps(entries: &[(&str, &str)]) -> Vec<(String, String)> {
        entries
            .iter()
            .map(|(event, state)| ((*event).to_owned(), (*state).to_owned()))
            .collect()
    }

    #[test]
    fn a_human_edit_can_be_undone_and_restores_the_before_value() {
        let mut log = Log::new();
        log.record("set width", Origin::Human, "8", "16");
        let undone = log.undo().expect("an undo");
        assert_eq!(undone.entry.before, "8");
        assert!(log.entries().is_empty());
        assert_eq!(log.undone().len(), 1);
        assert!(undone.render().contains("back to 8"));
    }

    #[test]
    fn a_business_side_effect_cannot_be_undone() {
        let mut log = Log::new();
        log.record("sent the order", Origin::Business, "none", "order-7");
        let refusal = log.undo().expect_err("a refusal");
        assert!(refusal.contains("business side effect"));
        assert_eq!(log.entries().len(), 1);
        assert!(log.undone().is_empty());
    }

    #[test]
    fn an_edit_recorded_after_a_side_effect_is_still_undoable() {
        let mut log = Log::new();
        log.record("sent the order", Origin::Business, "none", "order-7");
        log.record("set width", Origin::Human, "8", "16");
        assert!(log.undo().is_ok());
        // Now the side effect is the most recent entry, and it is refused.
        assert!(log.undo().is_err());
    }

    #[test]
    fn undoing_nothing_is_refused() {
        let mut log = Log::new();
        assert!(
            log.undo()
                .unwrap_err()
                .contains("nothing has been recorded")
        );
    }

    #[test]
    fn the_log_marks_a_side_effect_as_not_undoable() {
        let mut log = Log::new();
        log.record("sent the order", Origin::Business, "none", "order-7");
        log.record("set width", Origin::Human, "8", "16");
        let lines = log.render();
        assert!(lines[0].contains("(not undoable)"));
        assert!(!lines[1].contains("(not undoable)"));
        assert_eq!(lines[2], "recorded: 2, undone: 0");
    }

    #[test]
    fn a_vector_passes_and_reports_each_step() {
        let machine = machine();
        let mut session = Session::start(&machine).expect("a session");
        let taken = run_vector(
            &machine,
            &mut session,
            &steps(&[("start", "loading"), ("finish", "done")]),
            &BTreeMap::new(),
            super::Mode::Sync,
        )
        .expect("a vector");
        assert_eq!(taken.len(), 2);
        assert_eq!(taken[1].actual, "done");
    }

    #[test]
    fn a_vector_that_expects_the_wrong_state_names_the_step() {
        let machine = machine();
        let mut session = Session::start(&machine).expect("a session");
        let outcome = run_vector(
            &machine,
            &mut session,
            &steps(&[("start", "done")]),
            &BTreeMap::new(),
            super::Mode::Sync,
        );
        let reason = outcome.expect_err("a mismatch");
        assert!(reason.contains("after \"start\""));
        assert!(reason.contains("expects \"done\""));
    }

    #[test]
    fn an_identity_loop_settles_quickly() {
        let mut loop_ = Loop::new("value", Transform::Identity);
        let settled = loop_.settle(10);
        assert!(settled.is_fixed());
        assert_eq!(loop_.left, loop_.right);
        // The first round carries the value across; the second finds nothing to change.
        assert_eq!(settled, Settled::Fixed { rounds: 2 });
    }

    #[test]
    fn a_drifting_loop_does_not_settle() {
        let mut loop_ = Loop::new("value", Transform::Drifting);
        let settled = loop_.settle(8);
        assert!(!settled.is_fixed());
        assert_eq!(settled, Settled::Oscillating { limit: 8 });
        assert!(settled.render().contains("did not settle"));
        assert_ne!(loop_.left, loop_.right);
    }

    #[test]
    fn the_origin_says_whether_it_is_undoable() {
        assert!(Origin::Human.is_undoable());
        assert!(!Origin::Business.is_undoable());
        assert_eq!(Origin::Business.name(), "business");
    }

    #[test]
    fn a_multi_step_vector_and_a_loop_together() {
        // The joint check: a vector drives the machine while a two-way loop settles independently.
        let machine = machine();
        let mut session = Session::start(&machine).expect("a session");
        let taken = run_vector(
            &machine,
            &mut session,
            &steps(&[("start", "loading"), ("finish", "done")]),
            &BTreeMap::new(),
            super::Mode::Sync,
        )
        .expect("a vector");
        let mut loop_ = Loop::new("x", Transform::Identity);
        let settled = loop_.settle(4);
        assert_eq!(taken.len(), 2);
        assert!(settled.is_fixed());
        assert_eq!(session.current, "done");
    }
}
