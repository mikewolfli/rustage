// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The logic family's commands: the shared core, the typed channel, the action queue, the candidate
//! build, the migration, the swap, the readiness/combination rules and the vector regression.

// Every branch of the flag table answers with `return`, including the ones that could be a trailing
// expression: the branches are independent flags, and writing them alike is worth more than saving a
// keyword.
#![allow(clippy::needless_return)]

use super::*;

/// How to reduce events against the shared business core (task T17.01).
#[derive(Debug, PartialEq, Eq)]
pub struct LogicCoreOptions {
    /// The operations: `init:<state>`, `transition:<state>:<event>:<next>`,
    /// `guard:<state>:<event>:<field>=<type>:<value>`, `set:<state>:<event>:<field>=<type>:<value>`,
    /// `need:<state>:<event>:<function>:<into>=<type>:<value>`, `seam:<function>=<type>:<value>`,
    /// `event:<name>=<type>:<value>` (repeatable), `run`, `show`.
    pub ops: Vec<String>,
}

impl LogicCoreOptions {
    /// Options with no transitions and no events.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to open a typed local channel and move frames (task T17.02).
#[derive(Debug, PartialEq, Eq)]
pub struct LogicIpcOptions {
    /// The operations: `session:<id>`, `version:<n>`, `generation:<n>`, `manifest:<name>:<version>`,
    /// `capability:<name>` (repeatable), `frame:<kind>:<generation>:<payload>`, `receive:<encoded>`,
    /// `open`, `show`.
    pub ops: Vec<String>,
}

impl LogicIpcOptions {
    /// Options with no session.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to validate actions and queue them across the boundary (task T17.03).
#[derive(Debug, PartialEq, Eq)]
pub struct LogicActionsOptions {
    /// The operations: `capacity:<n>`, `item:<event>:<class>:<needs|ui>[:<deadline>]`, `take`,
    /// `tick:<now>`, `check`, `show`.
    pub ops: Vec<String>,
}

impl LogicActionsOptions {
    /// Options with an empty queue.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to debounce edits and admit a candidate build (task T17.04).
#[derive(Debug, PartialEq, Eq)]
pub struct LogicCandidateOptions {
    /// The operations: `revision:<n>`, `window:<ms>`, `edit:<source>`, `flush:<now>`,
    /// `action:<check|build|rebuild>`, `outcome:<success|failure>`, `stamp:<revision>`,
    /// `artifact:<path>`, `running:<path>`, `admit`, `check`, `show`.
    pub ops: Vec<String>,
}

impl LogicCandidateOptions {
    /// Options with no build.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to plan a state migration and confirm a reset (task T17.05).
#[derive(Debug, PartialEq, Eq)]
pub struct LogicMigrationOptions {
    /// The operations: `from:<n>`, `to:<n>`, `from-field:<name>:<type>:<since>`,
    /// `to-field:<name>:<type>:<since>`, `value:<name>=<type>:<value>`, `owner:<name>`,
    /// `task:<name>`, `plan`, `reset`, `confirm`, `check`, `show`.
    pub ops: Vec<String>,
}

impl LogicMigrationOptions {
    /// Options with no migration.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to walk the controlled replacement (task T17.06).
#[derive(Debug, PartialEq, Eq)]
pub struct LogicSwapOptions {
    /// The operations: `old:<name>`, `new:<name>`, `limit:<ms>`, `pause:<elapsed>`, `drain`,
    /// `snapshot:<version>`, `import:<yes|no>`, `confirm`, `resume`, `retire`, `fail:<reason>`,
    /// `check`, `show`.
    pub ops: Vec<String>,
}

impl LogicSwapOptions {
    /// Options with no swap.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to judge readiness and isolate a late result (task T17.07).
#[derive(Debug, PartialEq, Eq)]
pub struct LogicCombinationOptions {
    /// The operations: `building:<yes|no>`, `protocol:<yes|no>`,
    /// `plan:<style|document|binding|logic|protocol>` (repeatable), `interaction:<name>`,
    /// `needs:<style|document|binding|logic|protocol>` (repeatable), `generation:<n>`,
    /// `result:<generation>:<value>`, `check`, `show`.
    pub ops: Vec<String>,
}

impl LogicCombinationOptions {
    /// Options with no changes and no interactions.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to run the event vectors and decide on a fault (task T17.08).
#[derive(Debug, PartialEq, Eq)]
pub struct LogicRegressionOptions {
    /// The operations: `init:<state>`, `transition:<state>:<event>:<next>`, `vector:<name>`,
    /// `say:<event>`, `fault:unknown:<call>`, `fault:timeout:<step>`, `fault:crash:<generation>`,
    /// `fault:rapid:<count>`, `run`, `decide`, `check`, `show`.
    pub ops: Vec<String>,
}

impl LogicRegressionOptions {
    /// Options with no vectors.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// Serves one business program over standard input/output (task T17.02).
///
/// This is the replaceable side of a hot update, started by `--logic-process` (and available to a
/// person debugging a channel by hand). It reports nothing on standard output except encoded frames.
#[derive(Debug, PartialEq, Eq)]
pub struct LogicWorkerOptions {
    /// The operations: `name:<name>`, `version:<version>`, `initial:<state>`,
    /// `transition:<state>:<event>:<next>` and `set:<state>:<event>:<field>=<type>:<value>`
    /// (both repeatable), plus `schema-version:<n>` and `schema:<name>:<type>:<since>`
    /// (repeatable). A declared schema is this generation's own `restore` contract: an undeclared
    /// field or a wrongly typed value is refused.
    pub ops: Vec<String>,
}

impl LogicWorkerOptions {
    /// A worker with no program.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// Drives two real business processes across the typed channel (tasks T17.02 and T17.06).
#[derive(Debug, PartialEq, Eq)]
pub struct LogicProcessOptions {
    /// The operations: `session:<id>`, `name:<name>`, `version:<version>`,
    /// `generation:<n>`, `new-generation:<n>`, `first:<state>`, `second:<state>`,
    /// `first-transition:<state>:<event>:<next>` (repeatable),
    /// `second-transition:<state>:<event>:<next>` (repeatable),
    /// `first-set:<state>:<event>:<field>=<type>:<value>` (repeatable),
    /// `second-set:<state>:<event>:<field>=<type>:<value>` (repeatable),
    /// `first-event:<name>[:<type>:<value>]` (repeatable),
    /// `second-event:<name>[:<type>:<value>]` (repeatable),
    /// `second-binary:<path>` (run a built candidate instead of this executable),
    /// `first-schema:<name>:<type>:<since>` (repeatable, declares the old state schema),
    /// `second-schema:<name>:<type>:<since>` (repeatable, declares the new state schema),
    /// `from-version:<n>` / `to-version:<n>` (the schema versions around the migration),
    /// `preserve` (snapshot the old state and restore it into the new generation),
    /// `expect:<state>` (repeatable, in the order the events run), `check`, `show`.
    ///
    /// A schema migration needs `preserve` and both schema sides: an undeclared side would classify
    /// no field and silently drop the state it does not know about.
    pub ops: Vec<String>,
}

impl LogicProcessOptions {
    /// A driver with no programs.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// Parses this family's flags, answering `None` when the flag is another family's.
pub(super) fn parse_logic(
    text: &str,
    _argument: OsString,
    arguments: &mut std::vec::IntoIter<OsString>,
) -> Option<Result<Command, ParseError>> {
    macro_rules! ops {
        ($options:expr, $variant:path) => {{
            let mut options = $options;
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok($variant(options)));
        }};
    }
    match text {
        "--logic-core" => ops!(LogicCoreOptions::new(), Command::LogicCore),
        "--logic-ipc" => ops!(LogicIpcOptions::new(), Command::LogicIpc),
        "--logic-actions" => ops!(LogicActionsOptions::new(), Command::LogicActions),
        "--logic-candidate" => ops!(LogicCandidateOptions::new(), Command::LogicCandidate),
        "--logic-migration" => ops!(LogicMigrationOptions::new(), Command::LogicMigration),
        "--logic-swap" => ops!(LogicSwapOptions::new(), Command::LogicSwap),
        "--logic-combination" => ops!(LogicCombinationOptions::new(), Command::LogicCombination),
        "--logic-regression" => ops!(LogicRegressionOptions::new(), Command::LogicRegression),
        "--logic-worker" => ops!(LogicWorkerOptions::new(), Command::LogicWorker),
        "--logic-process" => ops!(LogicProcessOptions::new(), Command::LogicProcess),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;

    #[test]
    fn parses_the_logic_core_operations() {
        assert_eq!(
            parse(["--logic-core"]),
            Ok(Command::LogicCore(LogicCoreOptions::new())),
        );
        let mut expected = LogicCoreOptions::new();
        expected.ops = vec![
            "init:idle".to_owned(),
            "transition:idle:go:running".to_owned(),
            "run".to_owned(),
        ];
        assert_eq!(
            parse([
                "--logic-core",
                "init:idle",
                "transition:idle:go:running",
                "run"
            ]),
            Ok(Command::LogicCore(expected)),
        );
        assert!(parse(["--logic-core", "--oops"]).is_err());
    }

    #[test]
    fn parses_each_logic_flag() {
        assert_eq!(
            parse(["--logic-ipc"]),
            Ok(Command::LogicIpc(LogicIpcOptions::new()))
        );
        assert_eq!(
            parse(["--logic-actions"]),
            Ok(Command::LogicActions(LogicActionsOptions::new())),
        );
        assert_eq!(
            parse(["--logic-candidate"]),
            Ok(Command::LogicCandidate(LogicCandidateOptions::new())),
        );
        assert_eq!(
            parse(["--logic-migration"]),
            Ok(Command::LogicMigration(LogicMigrationOptions::new())),
        );
        assert_eq!(
            parse(["--logic-swap"]),
            Ok(Command::LogicSwap(LogicSwapOptions::new())),
        );
        assert_eq!(
            parse(["--logic-combination"]),
            Ok(Command::LogicCombination(LogicCombinationOptions::new())),
        );
        assert_eq!(
            parse(["--logic-regression"]),
            Ok(Command::LogicRegression(LogicRegressionOptions::new())),
        );
        assert_eq!(
            parse(["--logic-worker"]),
            Ok(Command::LogicWorker(LogicWorkerOptions::new())),
        );
        assert_eq!(
            parse(["--logic-process"]),
            Ok(Command::LogicProcess(LogicProcessOptions::new())),
        );
    }
}
