// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Layered settings and keymap import (task F24).

#![allow(clippy::needless_return)]

use super::*;

/// The settings/profile operations (task T24.01, T24.09).
#[derive(Debug, PartialEq, Eq)]
pub struct SettingsOptions {
    /// The operations: `schema:<key>:<kind>`, `set:<layer>:<key>=<value>`, `get:<key>`,
    /// `search:<term>`, `reset:<layer>:<key>`, `migrate:<old>=<new>[,<old>=<new>]`,
    /// `keymap:<command>=<chord>`, `keymap-other:<command>=<chord>`, `keymap-diff`,
    /// `keymap-import`, `show`.
    pub ops: Vec<String>,
}

impl SettingsOptions {
    /// Options with an empty profile.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

impl Default for SettingsOptions {
    fn default() -> Self {
        Self::new()
    }
}

pub(super) fn parse_settings(
    text: &str,
    _argument: OsString,
    arguments: &mut std::vec::IntoIter<OsString>,
) -> Option<Result<Command, ParseError>> {
    match text {
        "--settings" => {
            let mut options = SettingsOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Settings(options)));
        }
        _ => None,
    }
}
