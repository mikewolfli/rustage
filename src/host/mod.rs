// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Host services: the parts of rustage that belong to the desktop process rather than to the
//! design document. Contract **HOST**.
//!
//! Roadmap T01.01 asks for the host Cargo project, platform startup, module boundaries and the
//! error/logging foundation. The crate already exists; what this module adds is the boundary that
//! keeps host concerns from spreading into the contracts and the design model:
//!
//! | Concern | Module | What it owns |
//! |---|---|---|
//! | Platform startup | [`platform`] | the only place that calls the dependency's runtime |
//! | Startup orchestration | [`startup`] | the ordered steps, the outcome, and what "started" means |
//! | Errors | [`error`] | one error type and one exit-code mapping for the process |
//! | Logging | [`log`] | levelled records, an injectable clock, and a sink that can be captured |
//! | Paths | [`paths`] | system directories, and the project-root rule from decision F1 |
//!
//! **What "started" is allowed to mean.** The dependency publishes `RuntimeGuiMode`, and documents
//! why: a stub backend runs an event loop, prints progress, and creates no window, so a log alone
//! cannot tell it apart from a working one. [`startup::launch`] therefore asks for the mode before
//! claiming anything, and reports a refusal — not a success — when the backend is not interactive.
//! That is the point of T01.01's acceptance line "no placeholder success".
//!
//! Nothing here is a service registry yet: file dialogs, the clipboard, background tasks and
//! subprocesses arrive in T01.02–T01.04, and each of them gets its own module rather than a
//! growing `platform` file.

// Parts of this surface have no runtime producer yet, and the reason is worth writing down rather
// than hiding: the error variants for a failed init or loop are produced by platform adapters that
// can fail (the trait returns `Result`) and by the contract tests' fake platform, while the real
// `rust_widgets` adapter cannot report either failure because the pinned revision's entry points
// return `()`. The buffered logger and the fixed clock exist for tests now and for the output panel
// MS03 will build. Every one of them is exercised by a test; none of them is a placeholder standing
// in for behaviour, and not deleting them keeps the failure paths that the next adapter must fill.
#![allow(dead_code)]

pub mod error;
pub mod log;
pub mod paths;
pub mod platform;
pub mod startup;
