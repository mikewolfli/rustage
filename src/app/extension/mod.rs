// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Local extension packages, their contracts and templates (task F22).
//!
//! [`package`] holds the manifest, package, registry and template types; this boundary re-exports
//! them so callers keep the `extension::Manifest` path.

mod package;

pub use package::*;
