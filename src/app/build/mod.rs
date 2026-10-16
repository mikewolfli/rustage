// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Building, running, terminals and external tasks (feature family F16, roadmap MS07).
//!
//! F16 is the other half of "the first delivered source": F15 turns a design into Rust, and this
//! family turns that source into a running program and reads the result back. It is split the way the
//! work actually divides:
//!
//! * the toolchain and the feature selection ([`toolchain`]),
//! * check, build and rebuild and what their exit means ([`compile`]),
//! * the problems a build reports, mapped back to nodes ([`build_diagnostics`]),
//! * running the delivered program on its own ([`runner`]),
//! * configured tasks and their cache and cleanup ([`tasks`]),
//! * terminal sessions ([`terminal`]),
//! * external tools ([`external`]),
//! * the whole walk as one report ([`e2e`]),
//! * the typed task graph and problem matchers ([`dag`]),
//! * reproducible and offline builds ([`reproducible`]),
//! * and the headless walk a CI job runs ([`headless`]).
//!
//! Across all of them, one rule holds: a result is a value a caller can inspect and a person can act
//! on, and a failure is never shaped like a success. A missing SDK is a [`toolchain::Finding::Missing`]
//! with a remedy; an unsupported request is a [`toolchain::Finding::Unsupported`] with a reason; a
//! cancelled build is [`compile::Verdict::Cancelled`], not a green check.
pub mod build_diagnostics;
pub mod compile;
pub mod dag;
pub mod e2e;
pub mod external;
pub mod headless;
pub mod reproducible;
pub mod runner;
pub mod tasks;
pub mod terminal;
pub mod toolchain;
