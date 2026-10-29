// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Local extension packages, their contracts and templates (task F22).
//!
//! [`package`] holds the manifest, package, registry and template types; this boundary re-exports
//! them so callers keep the `extension::Manifest` path. [`supervisor`] holds the bounded, timed and
//! restart-budgeted call boundary for an extension that is running, and [`manager`] is the panel and
//! the project-dependency check a removal goes through (T22.05).

pub mod adapters;
mod diagnostics;
pub mod manager;
mod package;
pub mod startup;
pub mod supervisor;

pub use diagnostics::*;
pub use package::*;
