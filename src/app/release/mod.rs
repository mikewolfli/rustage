// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Release freeze, artifacts, SBOM and updates (task F25).
//!
//! [`manifest`] holds the release types and operations; this boundary re-exports them.

mod manifest;

pub use manifest::*;
