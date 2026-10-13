// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Event probes: the designer's property panel, wiring surface and generator all read the same
//! signal contract, so its dispatch ordering and its `once` semantics are load-bearing
//! (principle #17/#55: a wired event is a typed, ordered fact, not a name).
//!
//! Dispatch order is asserted, not assumed: the library documents "High before Normal before Low,
//! and connection order within a priority", and the designer's handlers depend on that reaching the
//! user's code in the order the panel listed them.

use std::sync::{Arc, Mutex};

use rust_widgets::signal::{Priority, Signal};

use super::{ApiFamily, ProbeResult, require};

/// The family these probes belong to.
const FAMILY: ApiFamily = ApiFamily::Event;

/// Runs every event probe.
pub fn probes() -> Vec<ProbeResult> {
    vec![
        priority_dispatch(),
        connection_order_dispatch(),
        slot_count_and_once(),
    ]
}

/// `Signal::connect_with_priority`: High fires before Normal before Low.
fn priority_dispatch() -> ProbeResult {
    const API: &str = "signal::Signal::connect_with_priority";
    let signal: Signal<u32> = Signal::new();
    let log = Arc::new(Mutex::new(Vec::<&'static str>::new()));
    connect_logged(&signal, &log, "low", Priority::Low);
    connect_logged(&signal, &log, "normal", Priority::Normal);
    connect_logged(&signal, &log, "high", Priority::High);

    signal.emit(7);

    let observed = take(&log);
    let expected = vec!["high", "normal", "low"];
    require!(
        observed == expected,
        FAMILY,
        API,
        format!("dispatch order was {observed:?}, expected {expected:?}")
    );

    ProbeResult::verified(FAMILY, API, vec![format!("dispatch order {observed:?}")])
}

/// `Signal::emit`: slots of equal priority fire in the order they were connected.
fn connection_order_dispatch() -> ProbeResult {
    const API: &str = "signal::Signal::emit (equal priority)";
    let signal: Signal<u32> = Signal::new();
    let log = Arc::new(Mutex::new(Vec::<&'static str>::new()));
    connect_logged(&signal, &log, "first", Priority::Normal);
    connect_logged(&signal, &log, "second", Priority::Normal);
    connect_logged(&signal, &log, "third", Priority::Normal);

    signal.emit(1);

    let observed = take(&log);
    let expected = vec!["first", "second", "third"];
    require!(
        observed == expected,
        FAMILY,
        API,
        format!("emit order was {observed:?}, expected {expected:?}")
    );

    ProbeResult::verified(FAMILY, API, vec![format!("emit order {observed:?}")])
}

/// `Signal::slot_count` and `Signal::connect_once`: `once` fires once and frees its handle.
fn slot_count_and_once() -> ProbeResult {
    const API: &str = "signal::Signal::connect_once/slot_count";
    let signal: Signal<u32> = Signal::new();
    require!(
        signal.slot_count() == 0,
        FAMILY,
        API,
        format!("a fresh signal reported {} slots", signal.slot_count())
    );

    let hits = Arc::new(Mutex::new(0u32));
    let counter = hits.clone();
    signal.connect_once(move |_| *counter.lock().expect("log lock") += 1);
    let stable = Arc::new(Mutex::new(0u32));
    let stable_counter = stable.clone();
    signal.connect(move |_| *stable_counter.lock().expect("log lock") += 1);

    require!(
        signal.slot_count() == 2,
        FAMILY,
        API,
        format!("two connections reported {} slots", signal.slot_count())
    );

    signal.emit(1);
    signal.emit(2);

    let once_hits = *hits.lock().expect("log lock");
    let stable_hits = *stable.lock().expect("log lock");
    require!(
        once_hits == 1,
        FAMILY,
        API,
        format!("a `once` slot fired {once_hits} times across two emits, expected 1")
    );
    require!(
        stable_hits == 2,
        FAMILY,
        API,
        format!("a persistent slot fired {stable_hits} times across two emits, expected 2")
    );
    require!(
        signal.slot_count() == 1,
        FAMILY,
        API,
        format!(
            "after the `once` slot fired, {} slots remained, expected 1",
            signal.slot_count()
        )
    );

    ProbeResult::verified(
        FAMILY,
        API,
        vec![
            "once slot fired 1 time, persistent slot fired 2 times".to_string(),
            "slot_count returned to 1 after the once slot released".to_string(),
        ],
    )
}

/// Connects a slot that records its label.
fn connect_logged(
    signal: &Signal<u32>,
    log: &Arc<Mutex<Vec<&'static str>>>,
    label: &'static str,
    priority: Priority,
) {
    let log = log.clone();
    signal.connect_with_priority(move |_| log.lock().expect("log lock").push(label), priority);
}

/// Takes the recorded labels, leaving the log empty.
fn take(log: &Arc<Mutex<Vec<&'static str>>>) -> Vec<&'static str> {
    std::mem::take(&mut *log.lock().expect("log lock"))
}

#[cfg(test)]
mod tests {
    use super::probes;
    use crate::probe::ProbeOutcome;

    #[test]
    fn every_event_probe_is_verified() {
        for probe in probes() {
            assert!(
                matches!(probe.outcome, ProbeOutcome::Verified),
                "event probe {} was {:?}",
                probe.api,
                probe.outcome
            );
        }
    }
}
