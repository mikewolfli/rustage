// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The project model: what a project is on disk and how it is addressed (milestone F02).
//!
//! A project is a directory with a metadata document in it. Everything a project contains is
//! addressed **relative to that directory** (decision `F1`), so a project can be moved, copied or
//! checked into a repository without rewriting anything, and a reference that would leave the
//! directory is refused instead of resolved.
//!
//! The layers are separate on purpose:
//!
//! * [`json`] reads and writes JSON without losing members it does not know.
//! * [`format`] says which members a version 1 document has, interprets them, and enforces the
//!   version policy (a version this build does not know is readable and never written, decision
//!   `U1`).
//!
//! Machine-local state — window geometry, the last page that was open, proxy and TLS overrides for
//! this machine — deliberately does **not** belong to the metadata document: it lives in the host's
//! configuration directory, so a project file can be shared without carrying one machine's state
//! and without becoming a place where a secret could hide.

#![allow(dead_code)]

pub mod format;
pub mod json;
