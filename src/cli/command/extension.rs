// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Local extension packages, contracts and templates (task F22).

#![allow(clippy::needless_return)]

use super::*;

/// The extension/template operations (task T22.01–T22.10).
#[derive(Debug, PartialEq, Eq)]
pub struct ExtensionOptions {
    /// The operations: `id:<value>`, `version:<x.y.z>`, `host-min:<x.y.z>`, `host-max:<x.y.z>`,
    /// `license:<text>`, `capability:<name>`, `permission:<name>`, `provides:<name>[,<name>...]`,
    /// `file:<name>:<text>`, `digest`, `install`, `install-dir:<path>`, `authorize:<fs|network>`,
    /// `pin:<id>`, `unpin:<id>`, `rollback:<id>`, `lookup:<id>`, `status`, `disable:<id>`,
    /// `enable:<id>`, `remove:<id>`, `remove-preflight:<id>`, `update-preflight`, `panel`,
    /// `usage:<name>[,<name>...]`, `adapter:<id>:<kind>:<version>:<host-min>:<host-max>`,
    /// `adapter-needs:<capability>`, `adapter-register`, `adapter-probe`,
    /// `adapter-fail:<id>:<reason>`, `adapter-ok:<id>`, `adapters`, `template:<name>`,
    /// `param:<name>:<kind>:<required>`, `template-file:<name>:<text>`, `render:<k=v,...>`, `show`,
    /// the `supervise*` operations, `service-log:<service>:<level>:<text>`,
    /// `bisect:<program>:<id>[,<id>...]`, `bundle-preview`, `bundle:<file>`,
    /// `bundle-send:<destination>`.
    ///
    /// `usage` is the open project's own report of the names it references; `panel` and a removal
    /// match it against what each installed extension provides, and a removal is refused while the
    /// project still uses any of it (T22.05).
    pub ops: Vec<String>,
}

impl ExtensionOptions {
    /// Options with no package.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

impl Default for ExtensionOptions {
    fn default() -> Self {
        Self::new()
    }
}

pub(super) fn parse_extension(
    text: &str,
    _argument: OsString,
    arguments: &mut std::vec::IntoIter<OsString>,
) -> Option<Result<Command, ParseError>> {
    match text {
        "--extension" => {
            let mut options = ExtensionOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Extension(options)));
        }
        _ => None,
    }
}
