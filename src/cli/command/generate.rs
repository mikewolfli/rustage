// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The generator's commands: the plan, the pipeline, the mapping, the merge, the reverse path, the
//! position recalculator, the export and the golden comparisons.
//!
//! Every command here is a sequence of `verb:argument` operations, the same shape the other model
//! commands use, so a demonstration is one line and its parts are named rather than positional.

#![allow(clippy::needless_return)]

use super::*;

/// How to plan a project's generated files (task T15.01).
#[derive(Debug, PartialEq, Eq)]
pub struct ArtifactOptions {
    /// The operations: `entry:<source>=<output>` (repeatable), `output:<source>`, `show`.
    pub ops: Vec<String>,
}

impl ArtifactOptions {
    /// Options with no entries.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to run the generation pipeline (task T15.02).
#[derive(Debug, PartialEq, Eq)]
pub struct GenerateOptions {
    /// The operations: `document:<file>`, `node:<kind>:<children>` (repeatable), `target:<default|stripped>`,
    /// `size:<w>x<h>`, `fn:<name>`, `style:<node>:<reason>`, `layout:<node>:<reason>`, `check`, `run`,
    /// `twice`, `show`.
    pub ops: Vec<String>,
}

impl GenerateOptions {
    /// Options with no document.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to build a source map (task T15.03).
#[derive(Debug, PartialEq, Eq)]
pub struct MappingOptions {
    /// The operations: `source:<file>`, `file:<path>`, `key:<node>:<key>` (repeatable), `model:<n>`,
    /// `generator:<maj>.<min>.<patch>`, `locate:<node>`, `at:<byte>`, `staleness:<n>`, `show`.
    pub ops: Vec<String>,
}

impl MappingOptions {
    /// Options with no source and no keys.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to merge a regenerated body into a committed file (task T15.04).
#[derive(Debug, PartialEq, Eq)]
pub struct GenMergeOptions {
    /// The operations: `old:<file>`, `new:<file>`, `show`.
    pub ops: Vec<String>,
}

impl GenMergeOptions {
    /// Options with no files.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to read a generated file back into a model candidate (task T15.05).
#[derive(Debug, PartialEq, Eq)]
pub struct ReverseOptions {
    /// The operations: `base:<file>`, `edited:<file>`, `mine:<file>`, `theirs:<file>`, `support`,
    /// `show`.
    pub ops: Vec<String>,
}

impl ReverseOptions {
    /// Options with no files.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to recalculate positions and guard against loops (task T15.06).
#[derive(Debug, PartialEq, Eq)]
pub struct PositionOptions {
    /// The operations: `text:<file>`, `at:<byte>`, `line:<line>:<column>`, `edit:<start>:<end>:<inserted>`,
    /// `apply:<byte>`, `guard:<limit>`, `enter`, `leave`, `undo`, `redo`, `show`.
    pub ops: Vec<String>,
}

impl PositionOptions {
    /// Options with no text.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to commit generated files and state what they need (task T15.07).
#[derive(Debug, PartialEq, Eq)]
pub struct ExportOptions {
    /// The operations: `root:<directory>`, `artifact:<path>=<file>` (repeatable), `commit`, `contract`,
    /// `show`.
    pub ops: Vec<String>,
}

impl ExportOptions {
    /// Options with nothing to write.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to compare generation against recorded goldens (task T15.08).
#[derive(Debug, PartialEq, Eq)]
pub struct GoldenOptions {
    /// The operations: `golden:<name>=<file>` (repeatable), `produced:<name>=<file>` (repeatable),
    /// `semantics:<file>`, `check`, `show`.
    pub ops: Vec<String>,
}

impl GoldenOptions {
    /// Options with no goldens.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// Parses this family's flags, answering `None` when the flag is another family's.
pub(super) fn parse_generate(
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
        "--artifact" => ops!(ArtifactOptions::new(), Command::Artifact),
        "--generate" => ops!(GenerateOptions::new(), Command::Generate),
        "--mapping" => ops!(MappingOptions::new(), Command::Mapping),
        "--gen-merge" => ops!(GenMergeOptions::new(), Command::GenMerge),
        "--reverse" => ops!(ReverseOptions::new(), Command::Reverse),
        "--position" => ops!(PositionOptions::new(), Command::Position),
        "--export" => ops!(ExportOptions::new(), Command::Export),
        "--golden" => ops!(GoldenOptions::new(), Command::Golden),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;

    #[test]
    fn parses_the_generator_operations() {
        assert_eq!(
            parse(["--artifact"]),
            Ok(Command::Artifact(ArtifactOptions::new()))
        );
        let mut expected = ArtifactOptions::new();
        expected.ops = vec!["entry:pages/main.json=src/generated/main.rs".to_owned()];
        assert_eq!(
            parse(["--artifact", "entry:pages/main.json=src/generated/main.rs"]),
            Ok(Command::Artifact(expected)),
        );
        assert!(parse(["--artifact", "--unknown"]).is_err());
    }

    #[test]
    fn parses_each_generator_flag() {
        assert_eq!(
            parse(["--generate"]),
            Ok(Command::Generate(GenerateOptions::new()))
        );
        assert_eq!(
            parse(["--mapping"]),
            Ok(Command::Mapping(MappingOptions::new()))
        );
        assert_eq!(
            parse(["--gen-merge"]),
            Ok(Command::GenMerge(GenMergeOptions::new()))
        );
        assert_eq!(
            parse(["--reverse"]),
            Ok(Command::Reverse(ReverseOptions::new()))
        );
        assert_eq!(
            parse(["--position"]),
            Ok(Command::Position(PositionOptions::new()))
        );
        assert_eq!(
            parse(["--export"]),
            Ok(Command::Export(ExportOptions::new()))
        );
        assert_eq!(
            parse(["--golden"]),
            Ok(Command::Golden(GoldenOptions::new()))
        );
    }

    #[test]
    fn a_generator_op_that_looks_like_a_flag_is_refused() {
        assert!(parse(["--generate", "run", "--oops"]).is_err());
        assert!(parse(["--golden", "--oops"]).is_err());
    }
}
