// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Host resources: what rustage owns, as opposed to what the widget dependency publishes.
//!
//! The committed sources live in `resources/` and the packaging definitions in `packaging/`;
//! this module is the Rust side that reads them. It contains no binary data and no derived
//! assets: the one generated file it declares is rebuilt from `resources/` by
//! `node tools/resources/generate-resources.mjs` and checked by
//! `node tools/resources/check-resources.mjs`.
//!
//! See [`icons`] for the icon seam.

// The icon seam is a vocabulary, and part of it has no runtime consumer *yet*: `IconRole::ALL` and
// `IconRole::from_str` are what the vocabulary cross-check test walks, `HostIcon::record` is what a
// caller that needs the manifest record (not just the drawable source) will use when the property
// panel assigns a role, and the generated `ICON_ROLE_NAMES` is the list the test compares the enum
// against. Deleting any of them would delete a claim that is already checked; the consumer that
// makes them runtime-reachable is T12.04's role assignment on the property panel. Silence the lint
// once here with that reason, the way `contracts`, `host` and `project` do — rather than at each
// item, where the reason would read as four unrelated silences.
#![allow(dead_code)]

pub mod icons;

/// Machine-generated data: the host icon table and the role vocabulary. Rebuilt from
/// `resources/` by the resource generator; edit the sources, never this file.
pub(crate) mod generated_host_icons;
