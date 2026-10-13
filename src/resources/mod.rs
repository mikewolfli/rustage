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

pub mod icons;

/// Machine-generated data: the host icon table and the role vocabulary. Rebuilt from
/// `resources/` by the resource generator; edit the sources, never this file.
pub(crate) mod generated_host_icons;
