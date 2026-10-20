// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The optional AI seam: authorized context, candidates and human review (task F23).

#![allow(clippy::needless_return)]

use super::*;

/// The optional AI operations (task T23.01–T23.12).
#[derive(Debug, PartialEq, Eq)]
pub struct AiOptions {
    /// The operations: `role:<coder|reviewer>`, `model:<name>`, `budget:<bytes>`,
    /// `tool:<name>`, `credential:<reference>`, `context:<path>:<text>`, `ignore:<pattern>`,
    /// `authorize`, `context-show`, `change:<path>:<before>:<after>`, `candidate:<id>`,
    /// `verify:<digest>:<state>`, `review:<text>`, `accept:<i[,j]>`, `base:<path>:<text>`,
    /// `apply`, `rollback`, `show`.
    pub ops: Vec<String>,
}

impl AiOptions {
    /// Options with nothing selected.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

impl Default for AiOptions {
    fn default() -> Self {
        Self::new()
    }
}

pub(super) fn parse_ai(
    text: &str,
    _argument: OsString,
    arguments: &mut std::vec::IntoIter<OsString>,
) -> Option<Result<Command, ParseError>> {
    match text {
        "--ai" => {
            let mut options = AiOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Ai(options)));
        }
        _ => None,
    }
}
