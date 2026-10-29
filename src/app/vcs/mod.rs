// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! An existing Git repository, reached through the installed `git` program (task F20).
//!
//! [`repository`] holds the types and operations and this boundary re-exports them so callers keep
//! the `vcs::Repository` path; [`secrets`] is the scan a commit preview runs before anything is
//! committed (T20.03), [`design_diff`] is the structural document diff and the
//! generated/hand-written split (T20.02), and [`branch`] is what a branch switch invalidates
//! (T20.05).

pub mod branch;
pub mod design_diff;
mod repository;
pub mod secrets;

pub use repository::*;
