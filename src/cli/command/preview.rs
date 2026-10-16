// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The preview's commands: the service, live update, state keeping, the viewport matrix,
//! inspection, baselines, recovery and load.

// Every branch of the flag table answers with `return`, including the ones that could be a
// trailing expression: the branches are independent flags, and writing them alike is worth more
// than saving a keyword. The returns that are *not* needless — the ones inside a loop that refuses
// a bad value — are why the table reads the way it does.
#![allow(clippy::needless_return)]

use super::*;

/// How to drive the preview service's lifecycle (task T14.01).
#[derive(Debug, PartialEq, Eq)]
pub struct PreviewOptions {
    /// The operations: `start:<session>`, `ready`, `spawn:<pid>`, `install:<node>`, `rebuild:<node>`,
    /// `stop`, `finish-stop`, `clean`, `state`, `show`.
    pub ops: Vec<String>,
}

impl PreviewOptions {
    /// Options with a stopped service.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to classify edits as live updates or rebuilds (task T14.02).
#[derive(Debug, PartialEq, Eq)]
pub struct HotUpdateOptions {
    /// The operations: `self:<source>`, `from:<source>`, `property:<subject>`, `style:<subject>`,
    /// `existing:<subject>`, `add-binding:<subject>`, `remove-binding:<subject>`, `add-node:<subject>`,
    /// `remove-node:<subject>`, `text:<subject>`, `flush`, `check`, `show`.
    pub ops: Vec<String>,
}

impl HotUpdateOptions {
    /// Options with nothing pending.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to capture and restore the live UI state across a rebuild (task T14.03).
#[derive(Debug, PartialEq, Eq)]
pub struct StateKeepOptions {
    /// The operations: `keep:<node>:<kind>`, `focus:<node>`, `scroll:<node>:<n>`,
    /// `input:<node>:<text>`, `selection:<node>:<start>:<end>`, `present:<node>:<kind>`, `restore`,
    /// `check`, `show`.
    pub ops: Vec<String>,
}

impl StateKeepOptions {
    /// Options with an empty capture.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to drive the preview viewport matrix (task T14.04).
#[derive(Debug, PartialEq, Eq)]
pub struct MatrixOptions {
    /// The operations: `document:<name>`, `viewport:<name>:<w>x<h>:<orientation>`, `theme:<name>:<t>`,
    /// `language:<name>:<lang>`, `sample:<name>:<s>`, `commit`, `start:<name>`, `stop:<name>`,
    /// `restart:<name>`, `remove:<name>`, `check`, `show`.
    pub ops: Vec<String>,
}

impl MatrixOptions {
    /// Options with an empty matrix.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to drive a preview inspection (task T14.05).
#[derive(Debug, PartialEq, Eq)]
pub struct InspectOptions {
    /// The operations: `mode:<m>`, `adapter:<a>`, `edit:<version>`, `instance:<name>:<version>`,
    /// `stop`, `kind:<node>:<kind>`, `design:<node>:<property>:<text>`,
    /// `computed:<node>:<property>:<text>`, `runtime:<node>:<property>:<text>:<version>`,
    /// `source:<node>:<file>:<line>:<column>`, `child:<node>:<parent>`, `event:<name>:<payload>`,
    /// `data:<field>:<value>`, `values:<node>:<property>`, `locate:<node>`, `trigger:<action>`,
    /// `check`, `show`.
    pub ops: Vec<String>,
}

impl InspectOptions {
    /// Options with an empty inspection.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to compare a preview screenshot with its recorded baseline (task T14.06).
#[derive(Debug, PartialEq, Eq)]
pub struct VisualOptions {
    /// The operations: `platform:<p>`, `dpi:<n>`, `resources:<n>`, `fonts:<name>`,
    /// `tolerance:<percent>:<channel-step>`, `record:<name>:<w>x<h>:<digits>`,
    /// `shot:<name>:<w>x<h>:<digits>`, `drop:<name>`, `baseline`, `check`, `show`.
    pub ops: Vec<String>,
}

impl VisualOptions {
    /// Options with no baselines and no shots.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to drive the preview app's failure, crash and recovery (task T14.07).
#[derive(Debug, PartialEq, Eq)]
pub struct PreviewRecoveryOptions {
    /// The operations: `edit:<version>`, `launch`, `reload`, `rebuild`, `reset`, `exit:<code>`,
    /// `crash:<signal>`, `fault:<reason>`, `clear`, `check`, `show`.
    pub ops: Vec<String>,
}

impl PreviewRecoveryOptions {
    /// Options with a preview that has never started.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to run bind/rebuild cycles and check what must not grow (task T14.08).
#[derive(Debug, PartialEq, Eq)]
pub struct LoadOptions {
    /// The operations: `budget:<ms>`, `cycle:<viewport>:<subscriptions>:<nodes>[:<latency>]`,
    /// `repeat:<viewport>:<cycles>:<subscriptions>:<nodes>[:<latency>]`,
    /// `leak:<viewport>:<cycles>:<subscriptions>:<nodes>`, `invalid:<text>`, `check`, `verdict`,
    /// `show`.
    pub ops: Vec<String>,
}

impl LoadOptions {
    /// Options with no cycles and no budget.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// Parses this family's flags, answering `None` when the flag is another family's.
pub(super) fn parse_preview(
    text: &str,
    _argument: OsString,
    arguments: &mut std::vec::IntoIter<OsString>,
) -> Option<Result<Command, ParseError>> {
    match text {
        "--preview" => {
            let mut options = PreviewOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Preview(options)));
        }
        "--hot-update" => {
            let mut options = HotUpdateOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::HotUpdate(options)));
        }
        "--state-keep" => {
            let mut options = StateKeepOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::StateKeep(options)));
        }
        "--matrix" => {
            let mut options = MatrixOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Matrix(options)));
        }
        "--inspect" => {
            let mut options = InspectOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Inspect(options)));
        }
        "--visual" => {
            let mut options = VisualOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Visual(options)));
        }
        "--preview-recovery" => {
            let mut options = PreviewRecoveryOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::PreviewRecovery(options)));
        }
        "--load" => {
            let mut options = LoadOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Load(options)));
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;

    #[test]
    fn parses_the_preview_operations() {
        assert_eq!(
            parse(["--preview"]),
            Ok(Command::Preview(PreviewOptions::new())),
        );
        let mut expected = PreviewOptions::new();
        expected.ops = vec![
            "start:session-1".to_owned(),
            "ready".to_owned(),
            "install:1".to_owned(),
        ];
        assert_eq!(
            parse(["--preview", "start:session-1", "ready", "install:1"]),
            Ok(Command::Preview(expected)),
        );
        assert!(parse(["--preview", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_hot_update_operations() {
        assert_eq!(
            parse(["--hot-update"]),
            Ok(Command::HotUpdate(HotUpdateOptions::new())),
        );
        let mut expected = HotUpdateOptions::new();
        expected.ops = vec![
            "property:width".to_owned(),
            "add-node:label".to_owned(),
            "flush".to_owned(),
        ];
        assert_eq!(
            parse(["--hot-update", "property:width", "add-node:label", "flush"]),
            Ok(Command::HotUpdate(expected)),
        );
        assert!(parse(["--hot-update", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_state_keep_operations() {
        assert_eq!(
            parse(["--state-keep"]),
            Ok(Command::StateKeep(StateKeepOptions::new())),
        );
        let mut expected = StateKeepOptions::new();
        expected.ops = vec![
            "keep:1:text-field".to_owned(),
            "scroll:1:40".to_owned(),
            "restore".to_owned(),
        ];
        assert_eq!(
            parse([
                "--state-keep",
                "keep:1:text-field",
                "scroll:1:40",
                "restore"
            ]),
            Ok(Command::StateKeep(expected)),
        );
        assert!(parse(["--state-keep", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_matrix_operations() {
        assert_eq!(
            parse(["--matrix"]),
            Ok(Command::Matrix(MatrixOptions::new())),
        );
        let mut expected = MatrixOptions::new();
        expected.ops = vec![
            "document:design.json".to_owned(),
            "viewport:phone:390x844:portrait".to_owned(),
            "start:phone".to_owned(),
        ];
        assert_eq!(
            parse([
                "--matrix",
                "document:design.json",
                "viewport:phone:390x844:portrait",
                "start:phone"
            ]),
            Ok(Command::Matrix(expected)),
        );
        assert!(parse(["--matrix", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_inspect_operations() {
        assert_eq!(
            parse(["--inspect"]),
            Ok(Command::Inspect(InspectOptions::new())),
        );
        let mut expected = InspectOptions::new();
        expected.ops = vec![
            "adapter:real".to_owned(),
            "instance:phone:4".to_owned(),
            "values:1:width".to_owned(),
            "trigger:send-order".to_owned(),
            "check".to_owned(),
        ];
        assert_eq!(
            parse([
                "--inspect",
                "adapter:real",
                "instance:phone:4",
                "values:1:width",
                "trigger:send-order",
                "check"
            ]),
            Ok(Command::Inspect(expected)),
        );
        assert!(parse(["--inspect", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_visual_comparison_operations() {
        assert_eq!(
            parse(["--visual"]),
            Ok(Command::Visual(VisualOptions::new())),
        );
        let mut expected = VisualOptions::new();
        expected.ops = vec![
            "platform:linux/x86_64".to_owned(),
            "record:home:3x2:012345".to_owned(),
            "tolerance:50:2".to_owned(),
            "shot:home:3x2:012345".to_owned(),
        ];
        assert_eq!(
            parse([
                "--visual",
                "platform:linux/x86_64",
                "record:home:3x2:012345",
                "tolerance:50:2",
                "shot:home:3x2:012345"
            ]),
            Ok(Command::Visual(expected)),
        );
        assert!(parse(["--visual", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_preview_recovery_operations() {
        assert_eq!(
            parse(["--preview-recovery"]),
            Ok(Command::PreviewRecovery(PreviewRecoveryOptions::new())),
        );
        let mut expected = PreviewRecoveryOptions::new();
        expected.ops = vec![
            "edit:7".to_owned(),
            "launch".to_owned(),
            "crash:11".to_owned(),
            "reload".to_owned(),
            "check".to_owned(),
        ];
        assert_eq!(
            parse([
                "--preview-recovery",
                "edit:7",
                "launch",
                "crash:11",
                "reload",
                "check"
            ]),
            Ok(Command::PreviewRecovery(expected)),
        );
        assert!(parse(["--preview-recovery", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_load_operations() {
        assert_eq!(parse(["--load"]), Ok(Command::Load(LoadOptions::new())));
        let mut expected = LoadOptions::new();
        expected.ops = vec![
            "budget:16".to_owned(),
            "repeat:phone:100:12:40:8".to_owned(),
            "invalid:bind:phone:missing<-nothing".to_owned(),
            "check".to_owned(),
        ];
        assert_eq!(
            parse([
                "--load",
                "budget:16",
                "repeat:phone:100:12:40:8",
                "invalid:bind:phone:missing<-nothing",
                "check"
            ]),
            Ok(Command::Load(expected)),
        );
        assert!(parse(["--load", "--unknown"]).is_err());
    }
}
