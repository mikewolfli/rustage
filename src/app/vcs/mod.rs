// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! An existing Git repository, reached through the installed `git` program (task F20).
//!
//! The module declaration is the whole file: [`repository`] holds the types and operations, and
//! this boundary re-exports them so callers keep the `vcs::Repository` path.

mod repository;

pub use repository::*;
