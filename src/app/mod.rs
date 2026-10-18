// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The application: what rustage does when it runs.
//!
//! This module is a boundary only. [`run`] executes a parsed command, and [`shell::toolbar`] holds
//! the icon strip the host consumes until the designer itself exists.
//!
//! * The designer is grouped into one module per feature family — `shell`, `commands`, `editor`,
//!   `language`, `search`, `canvas`, `layout`, `style`, `data`, `preview`, `generate`, `build`,
//!   `component`, `application`, `collection`, `logic`, `targets` and `locale` — because a flat
//!   directory of eighty modules cannot be navigated. The group names are the domains, and the task
//!   numbers in each module (`T04.01`, `T05.01`, …) are the same families the roadmap uses, so a
//!   reader can get from a roadmap entry to a file without a second index.
//! * `application` designs the *target* application's shell, menus and dialogs (F26); it is kept
//!   apart from `commands`, which is the *designer's* own menu tree.
//! * `locale` holds the designer's *own* interface vocabulary and language packs (F24); it is kept
//!   apart from `language`, which is the code language service.
//! * `targets` is the output side of the product (F18): the five profiles, their triples, the local
//!   device tools and the certification evidence. It is kept apart from `build`, which runs the
//!   toolchain, and from `generate`, which writes one profile's source.
//! * [`run`] stays at the top rather than inside a group: it is the dispatcher every command passes
//!   through, and its own submodules already follow this same domain split.
pub mod application;
pub mod build;
pub mod canvas;
pub mod collection;
pub mod commands;
pub mod component;
pub mod data;
pub mod editor;
pub mod generate;
pub mod language;
pub mod layout;
pub mod locale;
pub mod logic;
pub mod preview;
pub mod search;
pub mod shell;
pub mod style;
pub mod targets;

// The dispatcher stays at the top level rather than inside a group: every command passes through it,
// and its own submodules already follow the same domain split.
mod run;

pub use run::run;
