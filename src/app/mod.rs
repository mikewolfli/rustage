// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The application: what rustage does when it runs.
//!
//! This module is a boundary only. [`run`] executes a parsed command, and [`shell::toolbar`] holds
//! the icon strip the host consumes until the designer itself exists.
//!
//! * The designer is grouped into one module per feature family — `shell`, `commands`, `editor`,
//!   `language`, `search`, `canvas`, `layout`, `style`, `data` and `preview` — because a flat
//!   directory of eighty modules cannot be navigated. The group names are the domains, and the task
//!   numbers in each module (`T04.01`, `T05.01`, …) are the same families the roadmap uses, so a
//!   reader can get from a roadmap entry to a file without a second index.
//! * [`run`] stays at the top rather than inside a group: it is the dispatcher every command passes
//!   through, and its own submodules already follow this same domain split.
pub mod build;
pub mod canvas;
pub mod commands;
pub mod data;
pub mod editor;
pub mod generate;
pub mod language;
pub mod layout;
pub mod preview;
pub mod search;
pub mod shell;
pub mod style;

// The dispatcher stays at the top level rather than inside a group: every command passes through it,
// and its own submodules already follow the same domain split.
mod run;

pub use run::run;
