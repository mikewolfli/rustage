// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The problem centre, design checks, test evidence and visual conditions (task F21).

#![allow(clippy::needless_return)]

use super::*;

/// The quality evidence operations (task T21.01–T21.11).
#[derive(Debug, PartialEq, Eq)]
pub struct QualityOptions {
    /// The operations: `source:<name>`, `target:<name>`, `revision:<n>`, `node:<id>`, `location:<text>`,
    /// `severity:<error|warning|note>`, `problem:<message>`, `filter:<source>`, `filter-target:<name>`,
    /// `filter-revision:<n>`, `duplicate:<id>`, `parent:<id>:<parent>`, `capability:<name>:<true|false>`,
    /// `capacity:<used>:<limit>`, `resource:<name>:<true|false>`, `focus:<id>`, `check`,
    /// `contrast:<fg>:<bg>:<min>`, `test:<name>:<status>[:<seconds>]`, `tests`, `junit:<file>`,
    /// `visual:<platform>:<scale>:<font>`, `visual-compare:<platform>:<scale>:<font>`,
    /// `evidence:<file>`, `state`, `show`.
    pub ops: Vec<String>,
}

impl QualityOptions {
    /// Options with no facts.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

impl Default for QualityOptions {
    fn default() -> Self {
        Self::new()
    }
}

pub(super) fn parse_quality(
    text: &str,
    _argument: OsString,
    arguments: &mut std::vec::IntoIter<OsString>,
) -> Option<Result<Command, ParseError>> {
    match text {
        "--quality" => {
            let mut options = QualityOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Quality(options)));
        }
        _ => None,
    }
}
