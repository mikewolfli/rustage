// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The build family's commands: the toolchain, the build, its problems, the run, tasks, terminals,
//! external tools, the end-to-end walk, the task graph, reproducibility and the headless walk.

#![allow(clippy::needless_return)]

use super::*;

/// How to probe a toolchain and choose a feature set (task T16.01).
#[derive(Debug, PartialEq, Eq)]
pub struct ToolchainOptions {
    /// The operations: `profile:<name>`, `feature:<name>` (repeatable), `target:<triple>`, `cargo:<version>`,
    /// `rustc:<version>`, `installed:<triple>` (repeatable), `sdk:<name>`, `check`, `show`.
    pub ops: Vec<String>,
}

impl ToolchainOptions {
    /// Options with nothing probed.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to check, build or rebuild (task T16.02).
#[derive(Debug, PartialEq, Eq)]
pub struct CompileOptions {
    /// The operations: `action:<check|build|rebuild>`, `profile:<name>`, `feature:<name>` (repeatable),
    /// `target:<triple>`, `package:<name>`, `manifest:<path>`, `locked:<yes|no>`,
    /// `offline:<yes|no>`, `vendor:<path>`, `cache:<path>`, `source:<path>`, `exit:<code>`, `cancel`,
    /// `show`.
    pub ops: Vec<String>,
}

impl CompileOptions {
    /// Options with no job.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to read a build's problems and link them to nodes (task T16.03).
#[derive(Debug, PartialEq, Eq)]
pub struct BuildDiagOptions {
    /// The operations: `output:<file>`, `model:<n>`, `source:<file>`, `key:<node>:<key>` (repeatable),
    /// `span:<node>:<file>:<start>:<end>`, `current:<n>`, `show`.
    pub ops: Vec<String>,
}

impl BuildDiagOptions {
    /// Options with no output.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to run the delivered program (task T16.04).
#[derive(Debug, PartialEq, Eq)]
pub struct RunOptions {
    /// The operations: `program:<path>`, `arg:<value>` (repeatable), `env:<name>=<value>` (repeatable),
    /// `directory:<path>`, `standalone:<yes|no>`, `start`, `exit:<code>`, `signal:<n>`, `stop`, `restart`,
    /// `show`.
    pub ops: Vec<String>,
}

impl RunOptions {
    /// Options with no program.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to order configured tasks and check their cache (task T16.05).
#[derive(Debug, PartialEq, Eq)]
pub struct TasksOptions {
    /// The operations: `task:<name>:<command>` (repeatable), `after:<name>:<dependency>` (repeatable),
    /// `pre:<name>:<task>`, `post:<name>:<task>`, `input:<name>:<path>` (repeatable),
    /// `fingerprint:<path>:<value>`, `output:<name>:<path>`, `root:<path>`, `default:<name>`,
    /// `order`, `schedule`, `key:<name>`, `cleanup:<name>`, `clean-preview:<name>`, `clean:<name>`,
    /// `show`.
    ///
    /// `schedule` prints the waves that may run at once; `clean` deletes only what `clean-preview`
    /// showed in the same invocation (T16.05).
    pub ops: Vec<String>,
}

impl TasksOptions {
    /// Options with no tasks.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to drive a terminal session (task T16.06).
#[derive(Debug, PartialEq, Eq)]
pub struct TerminalOptions {
    /// The operations: `session:<name>`, `directory:<path>`, `env:<name>=<value>` (repeatable),
    /// `size:<columns>x<rows>`, `platform:<macos|linux|windows>`, `authorize`, `start`, `write:<text>`,
    /// `copy:<start>:<end>`, `resize:<columns>x<rows>`, `show`.
    pub ops: Vec<String>,
}

impl TerminalOptions {
    /// Options with no session.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to configure and invoke an external tool (task T16.07).
#[derive(Debug, PartialEq, Eq)]
pub struct ExternalOptions {
    /// The operations: `tool:<name>`, `program:<path>`, `arg:<value>` (repeatable), `input:<path>`,
    /// `output:<path>`, `confirm:<yes|no>`, `placeholder:<name>=<value>` (repeatable), `invoke`,
    /// `confirmed`, `show`.
    pub ops: Vec<String>,
}

impl ExternalOptions {
    /// Options with no tool.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to record the generate → check → build → run → feedback walk (task T16.08).
#[derive(Debug, PartialEq, Eq)]
pub struct E2eOptions {
    /// The operations: `generate:<ok|fail>:<note>`, `check:<ok|fail>:<note>`, `build:<ok|fail>:<note>`,
    /// `run:<ok|fail>:<note>`, `feedback:<ok|fail>:<note>`, `walk:<directory>:<program>`, `skip`,
    /// `show`.
    ///
    /// `walk` performs the walk for real against a cargo project: `cargo check`, `cargo build`, then
    /// the built binary; a failing check records the compiler's own first error line as the feedback.
    /// Generation is not part of it (the generator has its own tests), and a walk with `skip` is not
    /// a success — a stage that did not run is not a stage that passed (T16.08).
    pub ops: Vec<String>,
}

impl E2eOptions {
    /// Options with no steps recorded.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to build a typed task graph and match its output (task T16.09).
#[derive(Debug, PartialEq, Eq)]
pub struct DagOptions {
    /// The operations: `node:<name>`, `after:<name>:<dependency>`, `together:<name>:<dependency>`,
    /// `background:<name>:<marker>`, `terminal:<name>:<terminal>`, `default:<name>`,
    /// `pattern:<owner>:<pattern>:<file-group>:<line-group>:<message-group>`, `match:<line>`, `waves`,
    /// `target:<name>`, `revision:<n>`, `ready:<name>:<text>`, `show`.
    ///
    /// A matched line keeps the tool that read it and is recorded in the problem centre as a build
    /// problem for the current target and revision (T16.09).
    pub ops: Vec<String>,
}

impl DagOptions {
    /// Options with no graph.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to state a reproducible, offline build (task T16.10).
#[derive(Debug, PartialEq, Eq)]
pub struct ReproducibleOptions {
    /// The operations: `locked:<yes|no>`, `offline:<yes|no>`, `vendor:<path>`, `cache:<path>`,
    /// `source:<path>`, `show`.
    pub ops: Vec<String>,
}

impl ReproducibleOptions {
    /// Options with a mutable lock file and a reachable network.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to run the headless walk and export its CI (task T16.11).
#[derive(Debug, PartialEq, Eq)]
pub struct HeadlessOptions {
    /// The operations: `validate:<ok|fail>` (repeatable), `generate:<ok|fail>`, `check:<ok|fail>`,
    /// `profile:<name>`, `workflow`, `json`, `show`.
    pub ops: Vec<String>,
}

impl HeadlessOptions {
    /// Options with no recorded steps.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// Parses this family's flags, answering `None` when the flag is another family's.
pub(super) fn parse_build(
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
        "--toolchain" => ops!(ToolchainOptions::new(), Command::Toolchain),
        "--compile" => ops!(CompileOptions::new(), Command::Compile),
        "--build-diag" => ops!(BuildDiagOptions::new(), Command::BuildDiag),
        "--run" => ops!(RunOptions::new(), Command::Run),
        "--tasks" => ops!(TasksOptions::new(), Command::Tasks),
        "--terminal" => ops!(TerminalOptions::new(), Command::Terminal),
        "--external" => ops!(ExternalOptions::new(), Command::External),
        "--e2e" => ops!(E2eOptions::new(), Command::E2e),
        "--dag" => ops!(DagOptions::new(), Command::Dag),
        "--reproducible" => ops!(ReproducibleOptions::new(), Command::Reproducible),
        "--headless" => ops!(HeadlessOptions::new(), Command::Headless),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;

    #[test]
    fn parses_the_build_operations() {
        assert_eq!(
            parse(["--compile"]),
            Ok(Command::Compile(CompileOptions::new()))
        );
        let mut expected = CompileOptions::new();
        expected.ops = vec!["action:build".to_owned(), "profile:desktop".to_owned()];
        assert_eq!(
            parse(["--compile", "action:build", "profile:desktop"]),
            Ok(Command::Compile(expected)),
        );
        assert!(parse(["--compile", "--oops"]).is_err());
    }

    #[test]
    fn parses_each_build_flag() {
        assert_eq!(
            parse(["--toolchain"]),
            Ok(Command::Toolchain(ToolchainOptions::new()))
        );
        assert_eq!(
            parse(["--build-diag"]),
            Ok(Command::BuildDiag(BuildDiagOptions::new()))
        );
        assert_eq!(parse(["--run"]), Ok(Command::Run(RunOptions::new())));
        assert_eq!(parse(["--tasks"]), Ok(Command::Tasks(TasksOptions::new())));
        assert_eq!(
            parse(["--terminal"]),
            Ok(Command::Terminal(TerminalOptions::new()))
        );
        assert_eq!(
            parse(["--external"]),
            Ok(Command::External(ExternalOptions::new()))
        );
        assert_eq!(parse(["--e2e"]), Ok(Command::E2e(E2eOptions::new())));
        assert_eq!(parse(["--dag"]), Ok(Command::Dag(DagOptions::new())));
        assert_eq!(
            parse(["--reproducible"]),
            Ok(Command::Reproducible(ReproducibleOptions::new())),
        );
        assert_eq!(
            parse(["--headless"]),
            Ok(Command::Headless(HeadlessOptions::new()))
        );
    }
}
