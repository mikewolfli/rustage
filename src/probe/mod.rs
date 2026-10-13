// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Runnable contract probes against the pinned widget dependency.
//!
//! The designer is built *on* `rust_widgets`: its preview must reuse the library's layout
//! semantics, its mode 2 output comes from the library's generator, and its event wiring and
//! teardown must follow the library's signal and unbind contracts. Each of those is an assumption
//! about a dependency, and this project does not accept assumptions as support (principle #25: no
//! claim without evidence; #26: no placeholder that stands in for a closed loop).
//!
//! So the four contracts on the designer's critical path are exercised by **code that actually
//! calls them**, and each call records the upstream revision it ran against. That is T00.01's
//! deliverable: "every API has a revision and a runnable probe, or an explicit gap".
//!
//! This module is a boundary only. [`contract`] defines what a probe is and runs the set;
//! [`report`] renders the result; the four family modules hold the probes themselves.

pub mod contract;
pub mod editing;
pub mod event;
pub mod generation;
pub mod layout;
pub mod report;
pub mod unbind;

pub(crate) use contract::require;
pub use contract::{
    ApiFamily, DEPENDENCY_CRATE, DEPENDENCY_REVISION, DEPENDENCY_VERSION, ProbeOutcome,
    ProbeResult, run_all,
};
