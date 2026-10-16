// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Style's commands: the property editor, multi-selection, CSS, tokens, states, motion and
//! style emission.

// Every branch of the flag table answers with `return`, including the ones that could be a
// trailing expression: the branches are independent flags, and writing them alike is worth more
// than saving a keyword. The returns that are *not* needless — the ones inside a loop that refuses
// a bad value — are why the table reads the way it does.
#![allow(clippy::needless_return)]

use super::*;

/// How to drive the schema-driven property editor (task T11.01).
#[derive(Debug, PartialEq, Eq)]
pub struct PropertyEditorOptions {
    /// The operations: `schema:<property>:<type>`, `set:<node>:<property>:<text>`,
    /// `value:<node>:<property>`, `pending:<node>:<property>`, `state`, `show`.
    pub ops: Vec<String>,
}

impl PropertyEditorOptions {
    /// Options for an editor with an empty schema.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to inspect a multi-selection's shared properties and reset an override (task T11.02).
#[derive(Debug, PartialEq, Eq)]
pub struct MultiPropertyOptions {
    /// The operations: `node:<id>`, `set:<property>:<value>:<origin>`, `common`,
    /// `value:<property>`, `reset:<node>:<property>[:<fallback>]`, `state`, `show`.
    pub ops: Vec<String>,
}

impl MultiPropertyOptions {
    /// Options for an empty selection.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to read a CSS rule view and compute an element's style (task T11.03).
#[derive(Debug, PartialEq, Eq)]
pub struct CssOptions {
    /// The operations: `css:<rule text>` (append CSS text), `element:<compound>`,
    /// `value:<property>`, `source:<property>`, `properties`, `state`, `show`.
    pub ops: Vec<String>,
}

impl CssOptions {
    /// Options for an empty sheet.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to define design tokens and expand references at generation time (task T11.04).
#[derive(Debug, PartialEq, Eq)]
pub struct TokensOptions {
    /// The operations: `token:<kind>:<name>:<value>`, `theme:<name>:<token>=<value>[,...]`,
    /// `use:<property>:<value>`, `expand[:<theme>]`, `rename:<from>:<to>`, `state`, `show`.
    pub ops: Vec<String>,
}

impl TokensOptions {
    /// Options for an empty token set.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to preview interaction states and review theme/state differences (task T11.05).
#[derive(Debug, PartialEq, Eq)]
pub struct StatesOptions {
    /// The operations: `css:<rule text>`, `element:<compound>`, `on:<state>`,
    /// `declared:<event>`, `token:<kind>:<name>:<value>`, `theme:<name>:<token>=<value>[,...]`,
    /// `preview`, `compare:<left>[:<right>]`, `diff:<state>:<state>`, `state`, `show`.
    pub ops: Vec<String>,
}

impl StatesOptions {
    /// Options with no states requested.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to configure transitions/animations and control their preview (task T11.06).
#[derive(Debug, PartialEq, Eq)]
pub struct MotionOptions {
    /// The operations: `transitions:<on|off>`, `animations:<on|off>`, `property:<name>`,
    /// `easing:<name>`, `transition:<property>:<ms>:<delay_ms>:<easing>`,
    /// `animation:<property>:<ms>:<delay_ms>:<easing>`, `check`, `play`, `pause`, `seek:<ms>`,
    /// `state`, `show`.
    pub ops: Vec<String>,
}

impl MotionOptions {
    /// Options with an empty support table.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to emit a style for the preview and the generator, and migrate literals to tokens (T11.07).
#[derive(Debug, PartialEq, Eq)]
pub struct StyleOptions {
    /// The operations: `css:<rule text>`, `element:<compound>`, `inline:<property>:<value>`,
    /// `token:<kind>:<name>:<value>`, `theme:<name>:<token>=<value>[,...]`, `under:<theme|->`,
    /// `preview`, `generate`, `migrate`, `state`, `show`.
    pub ops: Vec<String>,
}

impl StyleOptions {
    /// Options with no input.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to exercise the style boundaries and undo a style edit (task T11.08).
#[derive(Debug, PartialEq, Eq)]
pub struct StyleBoundaryOptions {
    /// The operations: `css:<rule text>`, `element:<compound>`, `token:<kind>:<name>:<value>`,
    /// `theme:<name>:<token>=<value>[,...]`, `inline:<property>:<value>`, `clear:<property>`,
    /// `rename:<from>:<to>`, `use-theme:<theme|->`, `undo`, `emit`, `contrast:<a>:<b>`, `state`,
    /// `show`.
    pub ops: Vec<String>,
}

impl StyleBoundaryOptions {
    /// Options with an empty session.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// Parses this family's flags, answering `None` when the flag is another family's.
pub(super) fn parse_style(
    text: &str,
    _argument: OsString,
    arguments: &mut std::vec::IntoIter<OsString>,
) -> Option<Result<Command, ParseError>> {
    match text {
        "--property-editor" => {
            let mut options = PropertyEditorOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::PropertyEditor(options)));
        }
        "--multi-property" => {
            let mut options = MultiPropertyOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::MultiProperty(options)));
        }
        "--css" => {
            let mut options = CssOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Css(options)));
        }
        "--tokens" => {
            let mut options = TokensOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Tokens(options)));
        }
        "--states" => {
            let mut options = StatesOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::States(options)));
        }
        "--motion" => {
            let mut options = MotionOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Motion(options)));
        }
        "--style" => {
            let mut options = StyleOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Style(options)));
        }
        "--style-boundary" => {
            let mut options = StyleBoundaryOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::StyleBoundary(options)));
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;

    #[test]
    fn parses_the_property_editor_operations() {
        assert_eq!(
            parse(["--property-editor"]),
            Ok(Command::PropertyEditor(PropertyEditorOptions::new())),
        );
        let mut expected = PropertyEditorOptions::new();
        expected.ops = vec![
            "schema:width:number:0:100".to_owned(),
            "set:1:width:120".to_owned(),
            "value:1:width".to_owned(),
        ];
        assert_eq!(
            parse([
                "--property-editor",
                "schema:width:number:0:100",
                "set:1:width:120",
                "value:1:width"
            ]),
            Ok(Command::PropertyEditor(expected)),
        );
        assert!(parse(["--property-editor", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_multi_property_operations() {
        assert_eq!(
            parse(["--multi-property"]),
            Ok(Command::MultiProperty(MultiPropertyOptions::new())),
        );
        let mut expected = MultiPropertyOptions::new();
        expected.ops = vec![
            "node:1".to_owned(),
            "set:width:120:explicit".to_owned(),
            "common".to_owned(),
        ];
        assert_eq!(
            parse([
                "--multi-property",
                "node:1",
                "set:width:120:explicit",
                "common"
            ]),
            Ok(Command::MultiProperty(expected)),
        );
        assert!(parse(["--multi-property", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_css_operations() {
        assert_eq!(parse(["--css"]), Ok(Command::Css(CssOptions::new())),);
        let mut expected = CssOptions::new();
        expected.ops = vec![
            "css:.a { color: red }".to_owned(),
            "element:button.a".to_owned(),
            "value:color".to_owned(),
        ];
        assert_eq!(
            parse([
                "--css",
                "css:.a { color: red }",
                "element:button.a",
                "value:color"
            ]),
            Ok(Command::Css(expected)),
        );
        assert!(parse(["--css", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_tokens_operations() {
        assert_eq!(
            parse(["--tokens"]),
            Ok(Command::Tokens(TokensOptions::new())),
        );
        let mut expected = TokensOptions::new();
        expected.ops = vec![
            "token:colour:--accent:#3b82f6".to_owned(),
            "use:color:var(--accent)".to_owned(),
            "expand".to_owned(),
        ];
        assert_eq!(
            parse([
                "--tokens",
                "token:colour:--accent:#3b82f6",
                "use:color:var(--accent)",
                "expand"
            ]),
            Ok(Command::Tokens(expected)),
        );
        assert!(parse(["--tokens", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_states_operations() {
        assert_eq!(
            parse(["--states"]),
            Ok(Command::States(StatesOptions::new())),
        );
        let mut expected = StatesOptions::new();
        expected.ops = vec![
            "css:button:hover { color: blue }".to_owned(),
            "on:hover".to_owned(),
            "preview".to_owned(),
        ];
        assert_eq!(
            parse([
                "--states",
                "css:button:hover { color: blue }",
                "on:hover",
                "preview"
            ]),
            Ok(Command::States(expected)),
        );
        assert!(parse(["--states", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_motion_operations() {
        assert_eq!(
            parse(["--motion"]),
            Ok(Command::Motion(MotionOptions::new())),
        );
        let mut expected = MotionOptions::new();
        expected.ops = vec![
            "transitions:on".to_owned(),
            "property:color".to_owned(),
            "transition:color:200:0:linear".to_owned(),
            "check".to_owned(),
        ];
        assert_eq!(
            parse([
                "--motion",
                "transitions:on",
                "property:color",
                "transition:color:200:0:linear",
                "check"
            ]),
            Ok(Command::Motion(expected)),
        );
        assert!(parse(["--motion", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_style_operations() {
        assert_eq!(parse(["--style"]), Ok(Command::Style(StyleOptions::new())),);
        let mut expected = StyleOptions::new();
        expected.ops = vec![
            "css:button { color: #3b82f6 }".to_owned(),
            "inline:gap:8".to_owned(),
            "generate".to_owned(),
        ];
        assert_eq!(
            parse([
                "--style",
                "css:button { color: #3b82f6 }",
                "inline:gap:8",
                "generate"
            ]),
            Ok(Command::Style(expected)),
        );
        assert!(parse(["--style", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_style_boundary_operations() {
        assert_eq!(
            parse(["--style-boundary"]),
            Ok(Command::StyleBoundary(StyleBoundaryOptions::new())),
        );
        let mut expected = StyleBoundaryOptions::new();
        expected.ops = vec![
            "inline:color:var(--accent)".to_owned(),
            "undo".to_owned(),
            "emit".to_owned(),
        ];
        assert_eq!(
            parse([
                "--style-boundary",
                "inline:color:var(--accent)",
                "undo",
                "emit"
            ]),
            Ok(Command::StyleBoundary(expected)),
        );
        assert!(parse(["--style-boundary", "--unknown"]).is_err());
    }
}
