// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The debug family's commands: adapter probes, a session script, breakpoints, execution, inspection,
//! navigation and the logic-swap boundary (F19).

// Every branch of the flag table answers with `return`, including the ones that could be a trailing
// expression: the branches are independent flags, and writing them alike is worth more than saving a
// keyword.
#![allow(clippy::needless_return)]

use super::*;

/// How to drive a debug session (tasks T19.01–T19.08).
#[derive(Debug, PartialEq, Eq)]
pub struct DebugOptions {
    /// The operations, in the order they run:
    ///
    /// `probe`, `host:<system>`, `adapter:<name|auto|path:<program>>`, `timeout:<ms>`, `authorize`,
    /// `launch:<program>`, `attach:<pid>`, `arg:<value>` (repeatable), `directory:<path>`,
    /// `source-version:<n>`, `stop-on-entry:<yes|no>`, `start`, `break:<file>:<line>[:<condition>]`,
    /// `logpoint:<file>:<line>:<message>`, `hit:<file>:<line>:<condition>`,
    /// `breakpoints`, `enable-all`, `disable-all`, `remove-breakpoint:<id>`,
    /// `continue`, `pause`, `step-over`, `step-in`, `step-out`, `run-to:<file>:<line>`,
    /// `wait[:<ms>]`, `threads`, `stack[:<thread>]`, `scopes:<frame>`,
    /// `variables:<reference>[:<count>]`, `evaluate:<expression>`, `watch:<expression>`,
    /// `watches`, `policy:<allow|ask|deny>`, `status`, `disconnect`, `stop`,
    /// `hot-swap[:confirm]`, `check`.
    pub ops: Vec<String>,
}

impl DebugOptions {
    /// Options with no session.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// Parses this family's flags, answering `None` when the flag is another family's.
pub(super) fn parse_debug(
    text: &str,
    _argument: OsString,
    arguments: &mut std::vec::IntoIter<OsString>,
) -> Option<Result<Command, ParseError>> {
    match text {
        "--debug" => {
            let mut options = DebugOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            Some(Ok(Command::Debug(options)))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;

    #[test]
    fn parses_the_debug_operations_in_order() {
        assert_eq!(parse(["--debug"]), Ok(Command::Debug(DebugOptions::new())));
        let mut expected = DebugOptions::new();
        expected.ops = vec![
            "adapter:auto".to_owned(),
            "launch:/tmp/program".to_owned(),
            "break:/tmp/ui.rs:12".to_owned(),
            "continue".to_owned(),
        ];
        assert_eq!(
            parse([
                "--debug",
                "adapter:auto",
                "launch:/tmp/program",
                "break:/tmp/ui.rs:12",
                "continue",
            ]),
            Ok(Command::Debug(expected))
        );
        assert!(parse(["--debug", "--oops"]).is_err());
        assert_ne!(parse(["--debug"]), Ok(Command::Status));
    }
}
