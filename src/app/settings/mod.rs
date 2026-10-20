// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Layered settings, migration and keymap import (task F24).
//!
//! [`profile`] holds the settings schema, layered values and keymap types; this boundary re-exports
//! them.

mod profile;

pub use profile::*;
