// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The controlled replacement, in order: pause, drain, snapshot, import, confirm, resume, retire the old
//! version (task T17.06).
//!
//! The acceptance line is *"a failure before the switch keeps the old version, and a pause has a bound
//! and a state"*:
//!
//! * **The old version is retired last.** [`Swap::retire_old`] answers true only from [`Phase::Resumed`],
//!   so a failure at any earlier step leaves the old version running ([`Swap::old_running`]).
//! * **A pause is bounded.** [`Swap::pause_ticked`] aborts with [`Outcome::PauseTimedOut`] once the
//!   elapsed time passes the limit, so a pause cannot hang the editor.
//! * **Every step has a state.** [`Phase`] names where the swap is, and a step taken out of order is
//!   refused with [`Refusal::OutOfOrder`], so the walk cannot be half-entered.
//! * **The snapshot the new version imports is explicit.** [`Swap::snapshot`] records the migrated
//!   snapshot, and a failed import ([`Swap::import`] with `false`) aborts before the confirm, so nothing
//!   is switched on a snapshot the new version could not read.

#![allow(dead_code)]

use super::migration::Snapshot;

/// Where a swap is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    /// Not started.
    Idle,
    /// Pausing the old version.
    Pausing,
    /// The old version is paused.
    Paused,
    /// Its queue is drained.
    Drained,
    /// A snapshot has been taken.
    Snapshotted,
    /// The new version has imported the snapshot.
    Imported,
    /// A person confirmed the switch.
    Confirmed,
    /// The new version is running.
    Resumed,
    /// The old version is gone.
    Retired,
    /// The walk stopped before it finished; the old version is still running.
    Aborted,
}

impl Phase {
    /// The phase's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Pausing => "pausing",
            Self::Paused => "paused",
            Self::Drained => "drained",
            Self::Snapshotted => "snapshotted",
            Self::Imported => "imported",
            Self::Confirmed => "confirmed",
            Self::Resumed => "resumed",
            Self::Retired => "retired",
            Self::Aborted => "aborted",
        }
    }
}

/// Why a step was refused.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Refusal {
    /// The step does not follow the phase the swap is in.
    OutOfOrder {
        /// Where the swap is.
        at: Phase,
        /// What the step wanted.
        wanted: &'static str,
    },
}

impl Refusal {
    /// The refusal in one line.
    pub fn render(&self) -> String {
        match self {
            Self::OutOfOrder { at, wanted } => {
                format!("{wanted} cannot be taken while the swap is {}", at.name())
            }
        }
    }
}

/// Why a swap aborted.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Outcome {
    /// The pause missed its deadline.
    PauseTimedOut {
        /// The elapsed time.
        elapsed: u32,
        /// The limit.
        limit: u32,
    },
    /// The new version could not import the snapshot.
    ImportFailed,
}

impl Outcome {
    /// The outcome in one line.
    pub fn render(&self) -> String {
        match self {
            Self::PauseTimedOut { elapsed, limit } => {
                format!("the pause reached {elapsed} and its limit is {limit}")
            }
            Self::ImportFailed => "the new version could not import the snapshot".to_owned(),
        }
    }
}

/// One controlled replacement.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Swap {
    phase: Phase,
    /// The version being replaced.
    pub old: String,
    /// The version taking over.
    pub new: String,
    /// How long the pause may take.
    pub limit: u32,
    elapsed: u32,
    snapshot: Option<Snapshot>,
    reason: Option<String>,
}

impl Swap {
    /// Begins a swap from one version to another with a pause limit.
    pub fn begin(old: impl Into<String>, new: impl Into<String>, limit: u32) -> Self {
        Self {
            phase: Phase::Pausing,
            old: old.into(),
            new: new.into(),
            limit,
            elapsed: 0,
            snapshot: None,
            reason: None,
        }
    }

    /// Where the swap is.
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// Why the swap aborted, if it did.
    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }

    /// Advances the pause by the time that has passed, aborting past the limit.
    pub fn pause_ticked(&mut self, elapsed: u32) -> Result<(), Outcome> {
        self.elapsed = elapsed;
        if elapsed >= self.limit {
            self.phase = Phase::Aborted;
            self.reason = Some(format!(
                "pause timed out at {elapsed} (limit {})",
                self.limit
            ));
            return Err(Outcome::PauseTimedOut {
                elapsed,
                limit: self.limit,
            });
        }
        Ok(())
    }

    /// Marks the old version paused.
    pub fn paused(&mut self) -> Result<Phase, Refusal> {
        self.step(Phase::Pausing, Phase::Paused, "paused")
    }

    /// Marks the old version's queue drained.
    pub fn drain(&mut self) -> Result<Phase, Refusal> {
        self.step(Phase::Paused, Phase::Drained, "drain")
    }

    /// Records the snapshot the new version will import.
    pub fn snapshot(&mut self, snapshot: Snapshot) -> Result<Phase, Refusal> {
        let phase = self.step(Phase::Drained, Phase::Snapshotted, "snapshot")?;
        self.snapshot = Some(snapshot);
        Ok(phase)
    }

    /// Marks whether the new version imported the snapshot.
    pub fn import(&mut self, imported: bool) -> Result<Phase, Outcome> {
        if self.phase != Phase::Snapshotted {
            return Err(Outcome::ImportFailed);
        }
        if !imported {
            self.phase = Phase::Aborted;
            self.reason = Some("the import failed".to_owned());
            return Err(Outcome::ImportFailed);
        }
        self.phase = Phase::Imported;
        Ok(Phase::Imported)
    }

    /// Records the person's confirmation.
    pub fn confirm(&mut self) -> Result<Phase, Refusal> {
        self.step(Phase::Imported, Phase::Confirmed, "confirm")
    }

    /// Marks the new version running.
    pub fn resume(&mut self) -> Result<Phase, Refusal> {
        self.step(Phase::Confirmed, Phase::Resumed, "resume")
    }

    /// Retires the old version, answering whether it happened.
    pub fn retire_old(&mut self) -> bool {
        if self.phase != Phase::Resumed {
            return false;
        }
        self.phase = Phase::Retired;
        true
    }

    /// Whether the old version is still running.
    pub fn old_running(&self) -> bool {
        self.phase != Phase::Retired
    }

    /// The snapshot the new version imported, if it got that far.
    pub fn imported_snapshot(&self) -> Option<&Snapshot> {
        self.snapshot.as_ref()
    }

    /// Stops the walk with a reason; the old version keeps running.
    pub fn fail(&mut self, reason: impl Into<String>) -> Phase {
        self.phase = Phase::Aborted;
        self.reason = Some(reason.into());
        Phase::Aborted
    }

    /// Takes one ordered step.
    fn step(&mut self, from: Phase, next: Phase, wanted: &'static str) -> Result<Phase, Refusal> {
        if self.phase != from {
            return Err(Refusal::OutOfOrder {
                at: self.phase,
                wanted,
            });
        }
        self.phase = next;
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::{Outcome, Phase, Refusal, Swap};
    use crate::app::logic::migration::Snapshot;

    #[test]
    fn the_walk_reaches_retired_only_at_the_end() {
        let mut swap = Swap::begin("logic-1", "logic-2", 300);
        assert_eq!(swap.phase(), Phase::Pausing);
        swap.pause_ticked(10).expect("within the limit");
        assert_eq!(swap.paused().expect("paused"), Phase::Paused);
        assert!(swap.old_running(), "the old version is still running");
        swap.drain().expect("drained");
        swap.snapshot(Snapshot::new(2)).expect("snapshotted");
        swap.import(true).expect("imported");
        swap.confirm().expect("confirmed");
        swap.resume().expect("resumed");
        assert!(
            swap.old_running(),
            "the old version runs until it is retired"
        );
        assert!(swap.retire_old());
        assert_eq!(swap.phase(), Phase::Retired);
        assert!(!swap.old_running());
        assert!(
            !swap.retire_old(),
            "retiring twice is not a second retirement"
        );
    }

    #[test]
    fn a_pause_past_its_limit_aborts_with_the_old_version_running() {
        let mut swap = Swap::begin("logic-1", "logic-2", 300);
        let outcome = swap.pause_ticked(301).expect_err("a timeout");
        assert_eq!(
            outcome,
            Outcome::PauseTimedOut {
                elapsed: 301,
                limit: 300,
            }
        );
        assert_eq!(swap.phase(), Phase::Aborted);
        assert!(swap.old_running());
        assert!(outcome.render().contains("limit is 300"));
    }

    #[test]
    fn a_failed_import_aborts_with_the_old_version_running() {
        let mut swap = Swap::begin("logic-1", "logic-2", 300);
        swap.pause_ticked(5).expect("within the limit");
        swap.paused().expect("paused");
        swap.drain().expect("drained");
        swap.snapshot(Snapshot::new(2)).expect("snapshotted");
        let outcome = swap.import(false).expect_err("a refusal");
        assert_eq!(outcome, Outcome::ImportFailed);
        assert_eq!(swap.phase(), Phase::Aborted);
        assert!(swap.old_running());
    }

    #[test]
    fn a_step_out_of_order_is_refused() {
        let mut swap = Swap::begin("logic-1", "logic-2", 300);
        let refusal = swap.drain().expect_err("a refusal");
        assert_eq!(
            refusal,
            Refusal::OutOfOrder {
                at: Phase::Pausing,
                wanted: "drain",
            }
        );
        assert!(refusal.render().contains("pausing"));
        assert!(swap.old_running());
    }

    #[test]
    fn a_failure_keeps_the_old_version_and_names_its_reason() {
        let mut swap = Swap::begin("logic-1", "logic-2", 300);
        swap.pause_ticked(1).expect("within the limit");
        swap.paused().expect("paused");
        swap.fail("the new version crashed while importing");
        assert_eq!(swap.phase(), Phase::Aborted);
        assert!(swap.old_running());
        assert!(
            swap.reason()
                .is_some_and(|reason| reason.contains("crashed"))
        );
    }

    #[test]
    fn the_imported_snapshot_is_the_one_that_was_taken() {
        let mut swap = Swap::begin("logic-1", "logic-2", 300);
        swap.pause_ticked(1).expect("within the limit");
        swap.paused().expect("paused");
        swap.drain().expect("drained");
        let snapshot = Snapshot::new(2);
        swap.snapshot(snapshot.clone()).expect("snapshotted");
        assert_eq!(swap.imported_snapshot(), Some(&snapshot));
    }
}
