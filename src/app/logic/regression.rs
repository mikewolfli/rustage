// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Running the event vectors against the shared core, over both call seams, and the recovery rules for
//! the faults a replacement can produce (task T17.08).
//!
//! The acceptance line is *"an unknown result is not replayed blindly, and a lossless rollback after the
//! switch is not promised"*:
//!
//! * **An unknown outcome is reconciled, not retried.** [`decide`] answers [`Recovery::Reconcile`] for a
//!   call whose result is unknown, so a side effect is not repeated by a retry that assumes it failed
//!   (the rule T13.07 fixed for adapters, applied to the swap).
//! * **A timeout is investigated, not retried.** [`Recovery::Investigate`] names the step, because a
//!   timed-out call may or may not have happened.
//! * **A crash after the switch is not a lossless rollback.** [`Recovery::NoLosslessRollback`] names the
//!   generation, so a crash after the old version is retired is reported as a loss rather than a silent
//!   rewind.
//! * **A rapid burst is ordered, not raced.** [`Recovery::Ordered`] applies the burst in order rather
//!   than replaying whichever finished last.
//! * **The two call seams agree.** [`run`] drives the same [`Core`] once directly and once with every
//!   event carried through the typed channel ([`super::ipc`]), so the vectors are the evidence that the
//!   static call and the IPC call do not diverge.

#![allow(dead_code)]

use super::combination::Generation;
use super::core::{Core, Event, Refusal, Seam, State, Value, drive};
use super::ipc::{self, Frame, Kind, Manifest, Protocol, Session};

/// One scenario: an ordered sequence of events.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Vector {
    /// The scenario's name.
    pub name: String,
    /// The events.
    pub events: Vec<Event>,
}

impl Vector {
    /// An empty vector with a name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            events: Vec::new(),
        }
    }

    /// The same vector with one more event.
    pub fn say(mut self, event: Event) -> Self {
        self.events.push(event);
        self
    }
}

/// What one vector produced over both seams.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Report {
    /// The vector's name.
    pub name: String,
    /// How many steps it took.
    pub steps: usize,
    /// Whether the channel-carried walk reached the same states as the direct one.
    pub identical: bool,
}

impl Report {
    /// The report in one line.
    pub fn render(&self) -> String {
        let agreement = if self.identical {
            "identical"
        } else {
            "diverged"
        };
        format!("{}: {} step(s), {agreement}", self.name, self.steps)
    }
}

/// Why a walk could not be run.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Error {
    /// The core refused an event.
    Core(Refusal),
    /// The channel refused a frame.
    Channel(ipc::Fault),
    /// An event the channel carried could not be read back.
    Undecodable {
        /// The payload that could not be read.
        payload: String,
    },
}

impl Error {
    /// The error in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Core(refusal) => refusal.render(),
            Self::Channel(fault) => fault.render(),
            Self::Undecodable { payload } => {
                format!("an event carried as {payload:?} could not be read back")
            }
        }
    }
}

/// Runs every vector directly and again with each event carried through the channel.
pub fn run(
    vectors: &[Vector],
    core: &Core,
    start: &State,
    seam: &Seam,
) -> Result<Vec<Report>, Error> {
    let session = Session::open(
        "regression",
        Protocol::VERSION,
        Manifest::new("core", "0.1.0", 1),
        1,
    )
    .map_err(Error::Channel)?;

    let mut reports = Vec::new();
    for vector in vectors {
        let direct = drive(core, start, &vector.events, seam).map_err(Error::Core)?;
        let mut relayed = Vec::with_capacity(vector.events.len());
        for event in &vector.events {
            let frame = Frame::new(Kind::Request, "regression", 1, event_to_payload(event));
            let text = session.send(&frame).map_err(Error::Channel)?;
            let received = session.receive(&text).map_err(Error::Channel)?;
            relayed.push(payload_to_event(&received.payload).ok_or_else(|| {
                Error::Undecodable {
                    payload: String::from_utf8_lossy(&received.payload).into_owned(),
                }
            })?);
        }
        let through_channel = drive(core, start, &relayed, seam).map_err(Error::Core)?;
        let identical = direct.len() == through_channel.len()
            && direct
                .iter()
                .zip(&through_channel)
                .all(|(left, right)| left.state == right.state);
        reports.push(Report {
            name: vector.name.clone(),
            steps: direct.len(),
            identical,
        });
    }
    Ok(reports)
}

/// Writes an event as the bytes the channel carries.
fn event_to_payload(event: &Event) -> Vec<u8> {
    format!(
        "{}={}:{}",
        event.name,
        event.payload.type_name(),
        event.payload.render()
    )
    .into_bytes()
}

/// Reads an event back from the bytes the channel carried.
fn payload_to_event(payload: &[u8]) -> Option<Event> {
    let text = std::str::from_utf8(payload).ok()?;
    let (name, value) = text.split_once('=')?;
    Some(Event::new(name, Value::parse(value)?))
}

/// A fault a replacement can produce.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Fault {
    /// A call left the machine and its result is unknown.
    UnknownSideEffect {
        /// The call.
        call: String,
    },
    /// A step ran out of time.
    Timeout {
        /// The step.
        step: String,
    },
    /// The new version crashed after the switch.
    PostSwapCrash {
        /// The generation that crashed.
        generation: Generation,
    },
    /// Several changes arrived at once.
    RapidChange {
        /// How many.
        count: usize,
    },
}

/// What to do about a fault.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Recovery {
    /// Reconcile the effect; do not replay it.
    Reconcile {
        /// The call to reconcile.
        call: String,
    },
    /// Investigate the outcome; do not retry.
    Investigate {
        /// The step to investigate.
        step: String,
    },
    /// A crash after the switch cannot be rolled back without loss.
    NoLosslessRollback {
        /// The generation that crashed.
        generation: Generation,
    },
    /// Apply the burst in order.
    Ordered {
        /// How many changes.
        count: usize,
    },
}

impl Recovery {
    /// The recovery in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Reconcile { call } => {
                format!("reconcile {call}: its result is unknown, so it is not replayed")
            }
            Self::Investigate { step } => {
                format!("investigate {step}: a timed-out call may or may not have happened")
            }
            Self::NoLosslessRollback { generation } => format!(
                "generation {generation} crashed after the switch; there is no lossless rollback"
            ),
            Self::Ordered { count } => {
                format!("apply {count} change(s) in order rather than racing them")
            }
        }
    }
}

/// Decides what to do about a fault.
pub fn decide(fault: &Fault) -> Recovery {
    match fault {
        Fault::UnknownSideEffect { call } => Recovery::Reconcile { call: call.clone() },
        Fault::Timeout { step } => Recovery::Investigate { step: step.clone() },
        Fault::PostSwapCrash { generation } => Recovery::NoLosslessRollback {
            generation: *generation,
        },
        Fault::RapidChange { count } => Recovery::Ordered { count: *count },
    }
}

#[cfg(test)]
mod tests {
    use super::{Error, Fault, Recovery, Vector, decide, payload_to_event, run};
    use crate::app::logic::core::{Action, Core, Event, Refusal, Seam, Transition, Value};

    fn core() -> Core {
        let mut core = Core::new("idle");
        let mut go = Transition::new("idle", "go", "running");
        go.actions.push(Action::Set {
            field: "started".to_owned(),
            value: Value::Flag(true),
        });
        core.add(go).expect("a transition");
        core
    }

    fn vectors() -> Vec<Vector> {
        vec![Vector::new("start").say(Event::new("go", Value::Flag(true)))]
    }

    #[test]
    fn an_unknown_side_effect_is_reconciled_not_replayed() {
        let recovery = decide(&Fault::UnknownSideEffect {
            call: "place-order".to_owned(),
        });
        assert_eq!(
            recovery,
            Recovery::Reconcile {
                call: "place-order".to_owned(),
            }
        );
        assert!(recovery.render().contains("not replayed"));
    }

    #[test]
    fn a_timeout_is_investigated_not_retried() {
        let recovery = decide(&Fault::Timeout {
            step: "import".to_owned(),
        });
        assert!(recovery.render().contains("may or may not have happened"));
    }

    #[test]
    fn a_crash_after_the_swap_is_not_a_lossless_rollback() {
        let recovery = decide(&Fault::PostSwapCrash { generation: 5 });
        assert_eq!(recovery, Recovery::NoLosslessRollback { generation: 5 });
        assert!(recovery.render().contains("no lossless rollback"));
    }

    #[test]
    fn a_rapid_burst_is_ordered() {
        let recovery = decide(&Fault::RapidChange { count: 3 });
        assert_eq!(recovery, Recovery::Ordered { count: 3 });
        assert!(recovery.render().contains("in order"));
    }

    #[test]
    fn the_channel_carried_walk_agrees_with_the_direct_one() {
        let core = core();
        let reports = run(&vectors(), &core, &core.start(), &Seam::new()).expect("a run");
        assert_eq!(reports.len(), 1);
        assert!(reports[0].identical);
        assert_eq!(reports[0].steps, 1);
        assert!(reports[0].render().contains("identical"));
    }

    #[test]
    fn a_vector_that_needs_an_unanswered_seam_is_refused() {
        let mut core = Core::new("idle");
        let mut load = Transition::new("idle", "load", "loaded");
        load.need = Some(crate::app::logic::core::Need {
            function: "fetch".to_owned(),
            argument: Value::Flag(true),
            into: "order".to_owned(),
        });
        core.add(load).expect("a transition");

        let vectors = vec![Vector::new("load").say(Event::new("load", Value::Flag(true)))];
        let error = run(&vectors, &core, &core.start(), &Seam::new()).expect_err("a refusal");
        assert_eq!(
            error,
            Error::Core(Refusal::UnansweredSeam {
                function: "fetch".to_owned(),
            })
        );
        assert!(error.render().contains("fetch"));
    }

    #[test]
    fn an_event_round_trips_through_the_channel_payload() {
        let event = Event::new("order", Value::Text("a: b".to_owned()));
        let payload = super::event_to_payload(&event);
        assert_eq!(payload_to_event(&payload), Some(event));
        assert_eq!(payload_to_event(b"nonsense"), None);
    }
}
