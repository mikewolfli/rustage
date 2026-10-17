// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The target application's shell, menus, actions, tool bars and dialogs (feature family F26,
//! roadmap MS08).
//!
//! This family designs the *end-user application's* window shell and its chrome — not the designer's
//! own menus, which live in [`crate::app::commands::menu`]. The split is the same one the roadmap
//! draws:
//!
//! * the shell schema and its capability seam ([`shell`]),
//! * the shared action model every entry point references ([`action`]),
//! * the menu tree and its platform roles ([`menu`]),
//! * the target application's tool bar and status bar ([`toolbar`]),
//! * context menus, scope and dynamic enablement ([`context`]),
//! * dialogs, modality and focus ([`dialog`]),
//! * docking and persisted layout as a target capability ([`dock`]),
//! * and the end-to-end walk that proves one action fires once ([`regression`]).
//!
//! Two rules run through all of them: a bad value is refused at the edge rather than kept as a
//! placeholder, and a target capability that is missing is reported as a gap or an unsupported
//! outcome — never silently replaced by a stand-in.
pub mod action;
pub mod context;
pub mod dialog;
pub mod dock;
pub mod menu;
pub mod regression;
pub mod shell;
pub mod toolbar;
