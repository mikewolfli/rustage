// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The application: what rustage does when it runs.
//!
//! This module is a boundary only. [`run`] executes a parsed command, and [`toolbar`] holds the
//! icon strip the host consumes until the designer itself exists.

mod run;
pub mod toolbar;

pub use run::run;
