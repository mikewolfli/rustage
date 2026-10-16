// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The language service's commands: the session, protocol, completion, navigation,
//! diagnostics, workspace edits, schema docs, health, hints and analysis configuration.

// Every branch of the flag table answers with `return`, including the ones that could be a
// trailing expression: the branches are independent flags, and writing them alike is worth more
// than saving a keyword. The returns that are *not* needless — the ones inside a loop that refuses
// a bad value — are why the table reads the way it does.
#![allow(clippy::needless_return)]

use super::*;

/// How to drive the language-service lifecycle (task T07.01).
#[derive(Debug, PartialEq, Eq)]
pub struct LanguageOptions {
    /// The operations: `caps:<c1>,<c2>`, `fail:<toolchain|spawn|handshake>`, `view:<project>`,
    /// `restart:<project>`, `close:<project>`, `show`.
    pub ops: Vec<String>,
}

impl LanguageOptions {
    /// Options for a registry with nothing open.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to synchronise documents and convert positions (task T07.02).
#[derive(Debug, PartialEq, Eq)]
pub struct ProtocolOptions {
    /// The operations: `open:<uri>=<text>`, `replace:<uri>=<text>`,
    /// `edit:<uri>:<version>:<sl>,<sc>:<el>,<ec>=<text>`, `save:<uri>`, `close:<uri>`,
    /// `units:<uri>:<byte>`, `offset:<uri>:<line>,<char>`, `show`.
    pub ops: Vec<String>,
}

impl ProtocolOptions {
    /// Options for no documents open.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to drive completion freshness (task T07.03).
#[derive(Debug, PartialEq, Eq)]
pub struct FeaturesOptions {
    /// The operations: `version:<n>`, `request`, `cancel:<id>`, `respond:<id>:<version>:<a,b>`,
    /// `show`.
    pub ops: Vec<String>,
}

impl FeaturesOptions {
    /// Options for a session at version 1.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to ask a project's navigation (task T07.04).
#[derive(Debug, PartialEq, Eq)]
pub struct NavigationOptions {
    /// The operations: `caps:<c1>,<c2>`, `symbol:<uri>:<name>:<kind>:<s>:<e>:<ns>:<ne>`,
    /// `definition:<name>`, `references:<name>`, `declaration:<name>`, `implementation:<name>`,
    /// `document-symbol:<uri>`, `workspace-symbol:<name>`, `outline:<uri>`,
    /// `breadcrumbs:<uri>:<offset>`, `show`.
    pub ops: Vec<String>,
}

impl NavigationOptions {
    /// Options with every capability negotiated.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to drive diagnostics, tokens and code actions (task T07.05).
#[derive(Debug, PartialEq, Eq)]
pub struct DiagnosticsOptions {
    /// The operations: `version:<n>`, `diag:<severity>:<source>:<s>:<e>:<message>`,
    /// `publish:<version>`, `protect:<s>:<e>`, `mark`, `token:<kind>:<s>:<e>`,
    /// `tokens:<version>`, `action:<title>:<at>:<remove>:<insert>`, `apply`, `show`.
    pub ops: Vec<String>,
}

impl DiagnosticsOptions {
    /// Options for an empty document at version 1.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to review and commit a cross-file edit (task T07.06).
#[derive(Debug, PartialEq, Eq)]
pub struct WorkspaceEditOptions {
    /// The operations: `label:<text>`, `open:<uri>:<version>=<text>`, `protect:<uri>:<s>:<e>`,
    /// `edit:<uri>:<baseline>:<at>:<remove>:<insert>`, `review`, `commit`, `show`.
    pub ops: Vec<String>,
}

impl WorkspaceEditOptions {
    /// Options for an empty edit over no documents.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to show schema documentation and target-relative completion (task T07.07).
#[derive(Debug, PartialEq, Eq)]
pub struct SchemaOptions {
    /// The operations: `lang:<rust|json|css>`, `entry:<key>:<requires|->:<detail>`,
    /// `target:<name>:<cap1>,<cap2>`, `complete:<prefix>`, `doc:<key>`, `show`.
    pub ops: Vec<String>,
}

impl SchemaOptions {
    /// Options for an empty CSS catalog and no target.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to run the language-service failure regression (task T07.08).
#[derive(Debug, PartialEq, Eq)]
pub struct HealthOptions {
    /// The scenarios to run: `crash`, `slow`, `no-toolchain`, `multi-project`; all of them when empty.
    pub scenarios: Vec<String>,
}

impl HealthOptions {
    /// Options for every scenario.
    pub fn new() -> Self {
        Self {
            scenarios: Vec::new(),
        }
    }
}

/// How to show hints, lenses and hierarchies (task T07.09).
#[derive(Debug, PartialEq, Eq)]
pub struct HintsOptions {
    /// The operations: `caps:<c1>,<c2>`, `version:<n>`, `inlay:<s>:<e>:<label>`,
    /// `lens:<s>:<e>:<label>:<id>:<title>`, `publish:<version>`, `dismiss:<kind>`,
    /// `invoke:<id>`, `hierarchy:<call|type>:<name>`, `show`.
    pub ops: Vec<String>,
}

impl HintsOptions {
    /// Options for a layer at version 1 with every capability.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to rebuild the analysis configuration (task T07.10).
#[derive(Debug, PartialEq, Eq)]
pub struct AnalysisOptions {
    /// The operations: `target:<name>`, `known:<name>`, `member:<name>`, `feature:<name>`,
    /// `kit:<name>`, `trust:<trusted|untrusted>`, `configure`, `publish:<generation>`, `show`.
    pub ops: Vec<String>,
}

impl AnalysisOptions {
    /// Options for no configuration yet.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// Parses this family's flags, answering `None` when the flag is another family's.
pub(super) fn parse_language(
    text: &str,
    _argument: OsString,
    arguments: &mut std::vec::IntoIter<OsString>,
) -> Option<Result<Command, ParseError>> {
    match text {
        "--language" => {
            let mut options = LanguageOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Language(options)));
        }
        "--protocol" => {
            let mut options = ProtocolOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Protocol(options)));
        }
        "--features" => {
            let mut options = FeaturesOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Features(options)));
        }
        "--navigation" => {
            let mut options = NavigationOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Navigation(options)));
        }
        "--diagnostics" => {
            let mut options = DiagnosticsOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Diagnostics(options)));
        }
        "--workspace-edit" => {
            let mut options = WorkspaceEditOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::WorkspaceEdit(options)));
        }
        "--schema" => {
            let mut options = SchemaOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Schema(options)));
        }
        "--health" => {
            let mut options = HealthOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.scenarios.push(value.to_owned());
            }
            return Some(Ok(Command::Health(options)));
        }
        "--hints" => {
            let mut options = HintsOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Hints(options)));
        }
        "--analysis" => {
            let mut options = AnalysisOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Analysis(options)));
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;

    #[test]
    fn parses_the_language_operations() {
        assert_eq!(
            parse(["--language"]),
            Ok(Command::Language(LanguageOptions::new())),
        );
        let mut expected = LanguageOptions::new();
        expected.ops = vec![
            "caps:completion,hover".to_owned(),
            "view:app".to_owned(),
            "restart:app".to_owned(),
        ];
        assert_eq!(
            parse([
                "--language",
                "caps:completion,hover",
                "view:app",
                "restart:app"
            ]),
            Ok(Command::Language(expected)),
        );
        assert!(parse(["--language", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_protocol_operations() {
        assert_eq!(
            parse(["--protocol"]),
            Ok(Command::Protocol(ProtocolOptions::new())),
        );
        let mut expected = ProtocolOptions::new();
        expected.ops = vec![
            "open:file:///a.rs=中a".to_owned(),
            "edit:file:///a.rs:1:0,0:0,1=字".to_owned(),
            "units:file:///a.rs:4".to_owned(),
        ];
        assert_eq!(
            parse([
                "--protocol",
                "open:file:///a.rs=中a",
                "edit:file:///a.rs:1:0,0:0,1=字",
                "units:file:///a.rs:4"
            ]),
            Ok(Command::Protocol(expected)),
        );
        assert!(parse(["--protocol", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_features_operations() {
        assert_eq!(
            parse(["--features"]),
            Ok(Command::Features(FeaturesOptions::new())),
        );
        let mut expected = FeaturesOptions::new();
        expected.ops = vec![
            "version:1".to_owned(),
            "request".to_owned(),
            "respond:0:1:alpha,beta".to_owned(),
        ];
        assert_eq!(
            parse([
                "--features",
                "version:1",
                "request",
                "respond:0:1:alpha,beta"
            ]),
            Ok(Command::Features(expected)),
        );
        assert!(parse(["--features", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_navigation_operations() {
        assert_eq!(
            parse(["--navigation"]),
            Ok(Command::Navigation(NavigationOptions::new())),
        );
        let mut expected = NavigationOptions::new();
        expected.ops = vec![
            "caps:definition,references".to_owned(),
            "symbol:file:///a.rs:foo:function:0:10:0:3".to_owned(),
            "definition:foo".to_owned(),
        ];
        assert_eq!(
            parse([
                "--navigation",
                "caps:definition,references",
                "symbol:file:///a.rs:foo:function:0:10:0:3",
                "definition:foo"
            ]),
            Ok(Command::Navigation(expected)),
        );
        assert!(parse(["--navigation", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_diagnostics_operations() {
        assert_eq!(
            parse(["--diagnostics"]),
            Ok(Command::Diagnostics(DiagnosticsOptions::new())),
        );
        let mut expected = DiagnosticsOptions::new();
        expected.ops = vec![
            "diag:error:compiler:0:3:oops".to_owned(),
            "publish:1".to_owned(),
        ];
        assert_eq!(
            parse(["--diagnostics", "diag:error:compiler:0:3:oops", "publish:1"]),
            Ok(Command::Diagnostics(expected)),
        );
        assert!(parse(["--diagnostics", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_workspace_edit_operations() {
        assert_eq!(
            parse(["--workspace-edit"]),
            Ok(Command::WorkspaceEdit(WorkspaceEditOptions::new())),
        );
        let mut expected = WorkspaceEditOptions::new();
        expected.ops = vec![
            "label:rename".to_owned(),
            "edit:a.rs:1:0:1:X".to_owned(),
            "commit".to_owned(),
        ];
        assert_eq!(
            parse([
                "--workspace-edit",
                "label:rename",
                "edit:a.rs:1:0:1:X",
                "commit"
            ]),
            Ok(Command::WorkspaceEdit(expected)),
        );
        assert!(parse(["--workspace-edit", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_schema_operations() {
        assert_eq!(
            parse(["--schema"]),
            Ok(Command::Schema(SchemaOptions::new()))
        );
        let mut expected = SchemaOptions::new();
        expected.ops = vec![
            "lang:css".to_owned(),
            "entry:color:-:text colour".to_owned(),
            "target:embedded:".to_owned(),
            "complete:c".to_owned(),
        ];
        assert_eq!(
            parse([
                "--schema",
                "lang:css",
                "entry:color:-:text colour",
                "target:embedded:",
                "complete:c"
            ]),
            Ok(Command::Schema(expected)),
        );
        assert!(parse(["--schema", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_health_operations() {
        assert_eq!(
            parse(["--health"]),
            Ok(Command::Health(HealthOptions::new()))
        );
        let mut expected = HealthOptions::new();
        expected.scenarios = vec!["crash".to_owned(), "slow".to_owned()];
        assert_eq!(
            parse(["--health", "crash", "slow"]),
            Ok(Command::Health(expected)),
        );
        assert!(parse(["--health", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_hints_operations() {
        assert_eq!(parse(["--hints"]), Ok(Command::Hints(HintsOptions::new())));
        let mut expected = HintsOptions::new();
        expected.ops = vec![
            "inlay:2:2:: i32".to_owned(),
            "invoke:showReferences".to_owned(),
        ];
        assert_eq!(
            parse(["--hints", "inlay:2:2:: i32", "invoke:showReferences"]),
            Ok(Command::Hints(expected)),
        );
        assert!(parse(["--hints", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_analysis_operations() {
        assert_eq!(
            parse(["--analysis"]),
            Ok(Command::Analysis(AnalysisOptions::new())),
        );
        let mut expected = AnalysisOptions::new();
        expected.ops = vec![
            "target:wasm32-unknown-unknown".to_owned(),
            "trust:untrusted".to_owned(),
            "configure".to_owned(),
        ];
        assert_eq!(
            parse([
                "--analysis",
                "target:wasm32-unknown-unknown",
                "trust:untrusted",
                "configure"
            ]),
            Ok(Command::Analysis(expected)),
        );
        assert!(parse(["--analysis", "--unknown"]).is_err());
    }
}
