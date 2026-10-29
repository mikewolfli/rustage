// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Release freeze, artifacts, SBOM and updates (task F25).
//!
//! [`manifest`] holds the release types and operations; [`lockfile`] derives an SBOM from the
//! `Cargo.lock` a release was built from (T25.09); [`update_journal`] persists update state across
//! processes (T25.03). This boundary re-exports `manifest`.

pub mod lockfile;
mod manifest;
pub mod update_journal;

pub use manifest::*;
