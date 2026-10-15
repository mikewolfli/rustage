// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The application: what rustage does when it runs.
//!
//! This module is a boundary only. [`run`] executes a parsed command, and [`toolbar`] holds the
//! icon strip the host consumes until the designer itself exists.

pub mod audit;
pub mod command;
pub mod focus;
pub mod menu;
pub mod operations;
pub mod palette;
pub mod panels;
mod run;
pub mod shortcuts;
pub mod tabs;
pub mod toolbar;
pub mod workbench;

pub use run::run;
