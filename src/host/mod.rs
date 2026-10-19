// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Host services: the parts of rustage that belong to the desktop process rather than to the
//! design document. Contract **HOST**.
//!
//! Roadmap T01.01 asks for the host Cargo project, platform startup, module boundaries and the
//! error/logging foundation; T01.02 adds the platform services a host needs before it can open a
//! project. What this module owns is the boundary that keeps host concerns from spreading into the
//! contracts and the design model:
//!
//! | Concern | Module | What it owns |
//! |---|---|---|
//! | Platform startup | [`platform`] | the only place that calls the dependency's runtime |
//! | Startup orchestration | [`startup`] | the ordered steps, the outcome, and what "started" means |
//! | Errors | [`error`] | one error type and one exit-code mapping for the process |
//! | Logging | [`log`] | levelled records, an injectable clock, and a sink that can be captured |
//! | Language | [`locale`] | the interface languages the build knows, the one the system asks for, and which are bundled |
//! | Paths | [`paths`] | system directories, the project-root rule from decision F1, and the four host directories |
//! | File dialogs | [`dialog`] | the chooser request, its typed answer, the modal handoff, and what makes a chosen path usable |
//! | Clipboard | [`clipboard`] | plain-text round trips that keep "empty", "not text" and "refused" apart |
//! | URL opening | [`open_url`] | handing a URL or a path to the operating system's handler, and refusing what must not be handed over |
//! | File events | [`watch`] | the adapter T03.05's external-modification detection reads |
//! | Tasks | [`tasks`] | the background executor, task ownership, the bounded queue, progress and the timeout/cancellation protocol |
//! | Subprocesses | [`subprocess`] | starting a process, capturing its output, and taking down only the group this session created |
//! | Permissions | [`permission`] | project trust, the one gate that decides what may run, the permits that gate mints, and the record of every decision |
//! | Notifications | [`notify`] | the actionable half of a failure: its cause, its next step, the link to what it is about, and the de-duplication that keeps a repeat from prompting twice |
//! | Service registry | [`registry`] | one scope per open project, the order a close takes, and the guarantee that a failed start changes nothing |
//! | Network services | [`network`] | credential references, proxy and TLS configuration, the permit a declared service is reached under, and the redaction that keeps a credential out of what is shown |
//! | Readiness | [`readiness`] | what those services can actually do in this environment, with the evidence |
//!
//! Each service is its own module rather than another `platform` file, and each is behind a small
//! trait so its failure paths are reachable in a test without a display: the dependency's own entry
//! points cannot report every failure, and a service whose only testable path needs a GUI session
//! is a service whose failures are never seen.
//!
//! **What "started" is allowed to mean.** The dependency publishes `RuntimeGuiMode`, and documents
//! why: a stub backend runs an event loop, prints progress, and creates no window, so a log alone
//! cannot tell it apart from a working one. [`startup::launch`] therefore asks for the mode before
//! claiming anything, and reports a refusal — not a success — when the backend is not interactive.
//! That is the point of T01.01's acceptance line "no placeholder success".
//!
//! **The registry is [`registry::ServiceRegistry`]**, and it deliberately has no `Service` trait: a
//! service is a value with a lifetime the registry owns, not an object with four methods every type
//! has to grow. A scope holds the executor its tasks run on, the processes it started, and the
//! handles to the shared permission service and notification centre; closing it takes those down in
//! a fixed, reported order and invalidates the handles it handed out.
//!
//! The direction of the dependencies is the part worth keeping: [`permission::PermissionService`]
//! decides, [`subprocess::OwnedProcess::spawn`] requires a permit from it, and
//! [`registry::ProjectServices::spawn`] is the only route that mints one for a project — so a
//! project's own trust and authorization are what decide whether its code runs.

// Parts of this surface have no runtime producer yet, and the reason is worth writing down rather
// than hiding: the error variants for a failed init or loop are produced by platform adapters that
// can fail (the trait returns `Result`) and by the contract tests' fake platform, while the real
// `rust_widgets` adapter cannot report either failure because the pinned revision's entry points
// return `()`. The buffered logger and the fixed clock exist for tests now and for the output panel
// MS03 will build. Every one of them is exercised by a test; none of them is a placeholder standing
// in for behaviour, and not deleting them keeps the failure paths that the next adapter must fill.
#![allow(dead_code)]

pub mod clipboard;
pub mod dialog;
pub mod error;
pub mod locale;
pub mod log;
pub mod network;
pub mod notify;
pub mod open_url;
pub mod paths;
pub mod permission;
pub mod platform;
pub mod readiness;
pub mod registry;
pub mod smoke;
pub mod startup;
pub mod subprocess;
pub mod tasks;
pub mod watch;
