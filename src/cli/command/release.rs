// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Release freeze, artifacts, SBOM, updates and maintenance (task F25).

#![allow(clippy::needless_return)]

use super::*;

/// The release operations (task T25.01–T25.09).
#[derive(Debug, PartialEq, Eq)]
pub struct ReleaseOptions {
    /// The operations: `freeze:<revision>:<generator>`, `format:<name>`, `component:<name>:<version>`,
    /// `support:<revision>`, `freeze-check`, `artifact:<name>:<digest>:<size>:<signed|unsigned>`,
    /// `license:<text>`, `verify-file:<path>`, `sbom:<file>`, `update:<from>:<to>`,
    /// `verify-update:<path>`, `install`, `rollback`, `channel:<stable|candidate>`,
    /// `maintenance:<branch>:<matrix>`, `limit:<text>`, `show`.
    pub ops: Vec<String>,
}

impl ReleaseOptions {
    /// Options with no release.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

impl Default for ReleaseOptions {
    fn default() -> Self {
        Self::new()
    }
}

pub(super) fn parse_release(
    text: &str,
    _argument: OsString,
    arguments: &mut std::vec::IntoIter<OsString>,
) -> Option<Result<Command, ParseError>> {
    match text {
        "--release" => {
            let mut options = ReleaseOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Release(options)));
        }
        _ => None,
    }
}
