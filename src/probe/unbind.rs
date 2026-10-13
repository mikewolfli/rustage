// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Unbind probes: a designer opens and closes views constantly — the property panel is rebuilt on
//! every selection, the preview is torn down when a document closes — so "a view that goes away
//! stops receiving events" is a correctness property, not a nicety (principle #23: work must not
//! accumulate invisibly).
//!
//! Two teardown routes are exercised: an explicit handle returned by `connect`, and a
//! `ConnectionScope` that unbinds when it drops. The second is the one a view should use, because it
//! cannot be forgotten.

use std::sync::{Arc, Mutex};

use rust_widgets::signal::{ConnectionScope, Signal};

use super::{ApiFamily, ProbeResult, require};

/// The family these probes belong to.
const FAMILY: ApiFamily = ApiFamily::Unbind;

/// Runs every unbind probe.
pub fn probes() -> Vec<ProbeResult> {
    vec![
        disconnect_by_handle(),
        scope_unbinds_on_drop(),
        disconnect_all(),
    ]
}

/// `Signal::disconnect`: a handle removes exactly one slot, and is spent afterwards.
fn disconnect_by_handle() -> ProbeResult {
    const API: &str = "signal::Signal::disconnect";
    let signal: Signal<u32> = Signal::new();
    let hits = Arc::new(Mutex::new(0u32));
    let counter = hits.clone();
    let handle = signal.connect(move |_| *counter.lock().expect("counter lock") += 1);

    signal.emit(1);
    require!(
        read(&hits) == 1,
        FAMILY,
        API,
        format!(
            "the slot fired {} times before disconnect, expected 1",
            read(&hits)
        )
    );

    let removed = signal.disconnect(handle);
    require!(
        removed,
        FAMILY,
        API,
        "disconnect reported the handle was invalid"
    );

    signal.emit(2);
    require!(
        read(&hits) == 1,
        FAMILY,
        API,
        format!(
            "a disconnected slot still fired: {} times, expected 1",
            read(&hits)
        )
    );
    require!(
        signal.slot_count() == 0,
        FAMILY,
        API,
        format!(
            "{} slots remained after disconnect, expected 0",
            signal.slot_count()
        )
    );

    // A second disconnect of the same handle must report "nothing to remove" rather than lie.
    let again = signal.disconnect(handle);
    require!(
        !again,
        FAMILY,
        API,
        "a second disconnect of the same handle reported success"
    );

    ProbeResult::verified(
        FAMILY,
        API,
        vec!["disconnect removed the slot and reported the repeat as invalid".to_string()],
    )
}

/// `ConnectionScope`: dropping the scope unbinds everything it owns.
fn scope_unbinds_on_drop() -> ProbeResult {
    const API: &str = "signal::ConnectionScope";
    let signal: Signal<u32> = Signal::new();
    let hits = Arc::new(Mutex::new(0u32));

    {
        let scope = ConnectionScope::new();
        let counter = hits.clone();
        let _handle =
            signal.connect_scoped(&scope, move |_| *counter.lock().expect("counter lock") += 1);

        require!(
            scope.disconnect_count() == 1,
            FAMILY,
            API,
            format!(
                "the scope tracked {} connections, expected 1",
                scope.disconnect_count()
            )
        );
        signal.emit(1);
        require!(
            read(&hits) == 1,
            FAMILY,
            API,
            format!("the scoped slot fired {} times, expected 1", read(&hits))
        );
    }

    // The scope is gone; the slot must have gone with it.
    signal.emit(2);
    require!(
        read(&hits) == 1,
        FAMILY,
        API,
        format!(
            "a slot outlived its scope and fired {} times, expected 1",
            read(&hits)
        )
    );
    require!(
        signal.slot_count() == 0,
        FAMILY,
        API,
        format!(
            "{} slots survived the scope drop, expected 0",
            signal.slot_count()
        )
    );

    ProbeResult::verified(
        FAMILY,
        API,
        vec!["dropping the scope removed its connection automatically".to_string()],
    )
}

/// `Signal::disconnect_all`: a signal can be cleared wholesale.
fn disconnect_all() -> ProbeResult {
    const API: &str = "signal::Signal::disconnect_all";
    let signal: Signal<u32> = Signal::new();
    let hits = Arc::new(Mutex::new(0u32));
    for _ in 0..3 {
        let counter = hits.clone();
        signal.connect(move |_| *counter.lock().expect("counter lock") += 1);
    }

    signal.emit(1);
    require!(
        read(&hits) == 3,
        FAMILY,
        API,
        format!(
            "three slots fired a total of {} times, expected 3",
            read(&hits)
        )
    );

    signal.disconnect_all();
    require!(
        signal.slot_count() == 0,
        FAMILY,
        API,
        format!(
            "{} slots remained after disconnect_all, expected 0",
            signal.slot_count()
        )
    );

    signal.emit(2);
    require!(
        read(&hits) == 3,
        FAMILY,
        API,
        format!(
            "a slot survived disconnect_all and fired; total {} , expected 3",
            read(&hits)
        )
    );

    ProbeResult::verified(
        FAMILY,
        API,
        vec!["disconnect_all cleared three slots".to_string()],
    )
}

/// Reads a counter.
fn read(counter: &Arc<Mutex<u32>>) -> u32 {
    *counter.lock().expect("counter lock")
}

#[cfg(test)]
mod tests {
    use super::probes;
    use crate::probe::ProbeOutcome;

    #[test]
    fn every_unbind_probe_is_verified() {
        for probe in probes() {
            assert!(
                matches!(probe.outcome, ProbeOutcome::Verified),
                "unbind probe {} was {:?}",
                probe.api,
                probe.outcome
            );
        }
    }
}
