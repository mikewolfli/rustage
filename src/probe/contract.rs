// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The contract probe vocabulary: what a probe is, what it can report, and the fixed revision it
//! reports against.
//!
//! # What a probe reports
//!
//! A probe either proves the contract or records why it could not:
//!
//! | Outcome | Meaning |
//! |---|---|
//! | [`ProbeOutcome::Verified`] | The contract behaved as the designer requires. |
//! | [`ProbeOutcome::Gap`] | The requirement was not met, and the string says how. A gap is either a finding **about the library** (filed under `docs/rw-feedback/`) or a change in the contract the designer must adopt. |
//!
//! There is deliberately no third "could not run here" state yet: every probe in this module runs
//! wherever the crate builds, so a state nothing could produce would be vocabulary without a
//! producer (principle #39/#61). It is added together with the first probe that needs a display
//! session or a device, and until then "an unrun probe is not a pass" is upheld by the probes
//! having no way to be skipped.
//!
//! A probe never reports success for a shape it did not observe: each one reads values back and
//! records a [`ProbeOutcome::Gap`] when the observation does not match, rather than panicking. The
//! unit tests then fail if a probe that must be `Verified` turns into a `Gap`, so the report cannot
//! silently drift into "everything is fine".
//!
//! # Why the revision is a constant with a drift test
//!
//! [`DEPENDENCY_REVISION`] duplicates the `rev` in `Cargo.toml`. That duplication is deliberate
//! and gated: a drift test in [`crate::probe::report`] re-reads `Cargo.lock` and fails if the two
//! disagree. Reading the lockfile at run time instead would make the report's revision depend on
//! whatever lockfile happens to sit next to the binary, which is the "stale result presented as
//! current" failure principle #22 forbids.

/// The crate name the probes exercise.
pub const DEPENDENCY_CRATE: &str = "rust_widgets";

/// The pinned dependency version, matching `Cargo.lock`.
pub const DEPENDENCY_VERSION: &str = "2.8.6";

/// The pinned dependency revision, matching the `rev` in `Cargo.toml`.
///
/// Kept in step with the lockfile by a drift test in [`crate::probe::report`], which re-reads
/// `Cargo.lock` and fails if the two disagree.
pub const DEPENDENCY_REVISION: &str = "58012c0508b2a4eb095e7623e685f5c9889e9835";

/// The API families on the designer's critical path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiFamily {
    /// Static layout solved with the library's own layout engine.
    Layout,
    /// Mode 2 source generation and target-capability queries.
    Generation,
    /// Signal connect and dispatch.
    Event,
    /// Connection teardown.
    Unbind,
    /// The text editor: edit primitives, highlight state, coordinates and graded scale.
    Editing,
}

impl ApiFamily {
    /// Every family, in report order.
    pub const ALL: [ApiFamily; 5] = [
        ApiFamily::Layout,
        ApiFamily::Generation,
        ApiFamily::Event,
        ApiFamily::Unbind,
        ApiFamily::Editing,
    ];

    /// The stable identifier used in the report and its JSON form.
    pub fn as_str(self) -> &'static str {
        match self {
            ApiFamily::Layout => "layout",
            ApiFamily::Generation => "generation",
            ApiFamily::Event => "event",
            ApiFamily::Unbind => "unbind",
            ApiFamily::Editing => "editing",
        }
    }
}

/// What a probe observed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeOutcome {
    /// The contract behaved as required.
    Verified,
    /// The requirement was not met; the string says how.
    Gap(String),
}

impl ProbeOutcome {
    /// The stable identifier used in the report and its JSON form.
    pub fn as_str(&self) -> &'static str {
        match self {
            ProbeOutcome::Verified => "verified",
            ProbeOutcome::Gap(_) => "gap",
        }
    }

    /// The explanatory text, for a gap.
    pub fn reason(&self) -> Option<&str> {
        match self {
            ProbeOutcome::Verified => None,
            ProbeOutcome::Gap(reason) => Some(reason),
        }
    }

    /// Whether the probe proved the contract.
    pub fn is_verified(&self) -> bool {
        matches!(self, ProbeOutcome::Verified)
    }
}

/// One probe's result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeResult {
    /// Which API family this probe covers.
    pub family: ApiFamily,
    /// The specific item exercised, spelled as it appears in the library.
    pub api: &'static str,
    /// What was observed.
    pub outcome: ProbeOutcome,
    /// Values read back from the library, in the order they were observed.
    pub notes: Vec<String>,
}

impl ProbeResult {
    /// A probe that behaved as required, with the values it read back.
    pub fn verified(family: ApiFamily, api: &'static str, notes: Vec<String>) -> Self {
        Self {
            family,
            api,
            outcome: ProbeOutcome::Verified,
            notes,
        }
    }

    /// A probe whose observation did not match the requirement.
    pub fn gap(family: ApiFamily, api: &'static str, reason: impl Into<String>) -> Self {
        Self {
            family,
            api,
            outcome: ProbeOutcome::Gap(reason.into()),
            notes: Vec::new(),
        }
    }
}

/// Records a gap and returns from the enclosing probe when `condition` is false.
///
/// # Why not `assert!`
///
/// A panicking probe would abort the whole run at the first surprise, so the report could never
/// say "layout is fine, generation is broken" — the two facts it exists to keep apart. This turns
/// the surprise into a recorded observation so the remaining probes still run.
macro_rules! require {
    ($condition:expr, $family:expr, $api:expr, $reason:expr) => {
        if !$condition {
            return ProbeResult::gap($family, $api, $reason);
        }
    };
}

pub(crate) use require;

/// Runs every probe and returns the results in family order.
///
/// Deterministic: the order of families and of the probes within them is fixed, and no probe reads
/// a clock, the filesystem or the network, so two runs over one revision produce the same report.
pub fn run_all() -> Vec<ProbeResult> {
    let mut results = Vec::new();
    results.extend(super::layout::probes());
    results.extend(super::generation::probes());
    results.extend(super::event::probes());
    results.extend(super::unbind::probes());
    results.extend(super::editing::probes());
    results
}
