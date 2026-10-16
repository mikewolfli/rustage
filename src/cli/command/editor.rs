// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The editor's commands: text, highlighting, coordinates, multi-cursor editing, the
//! view, scale, snippets, the regression harness and structural selection.

// Every branch of the flag table answers with `return`, including the ones that could be a
// trailing expression: the branches are independent flags, and writing them alike is worth more
// than saving a keyword. The returns that are *not* needless — the ones inside a loop that refuses
// a bad value — are why the table reads the way it does.
#![allow(clippy::needless_return)]

use super::*;

/// How to replay in-place text edits (task T06.01).
#[derive(Debug, PartialEq, Eq)]
pub struct TextOptions {
    /// The file to read the starting text from, when there is one.
    pub file: Option<PathBuf>,
    /// The operations: `splice:<at>:<remove>:<insert>`, `undo`, `redo`, `show`.
    pub ops: Vec<String>,
}

impl TextOptions {
    /// Options for an empty text.
    pub fn new() -> Self {
        Self {
            file: None,
            ops: Vec::new(),
        }
    }
}

/// How to show syntax highlighting (task T06.02).
#[derive(Debug, PartialEq, Eq)]
pub struct HighlightOptions {
    /// The language, when the caller names one rather than relying on the file's extension.
    pub language: Option<String>,
    /// The theme the roles are coloured by.
    pub theme: Option<String>,
    /// The file to read the text from, when there is one.
    pub file: Option<PathBuf>,
    /// The operations: `line:<text>`, `step:<n>`, `set:<line>:<text>`, `language:<l>`, `show`.
    pub ops: Vec<String>,
}

impl HighlightOptions {
    /// Options for an empty plain-text document.
    pub fn new() -> Self {
        Self {
            language: None,
            theme: None,
            file: None,
            ops: Vec::new(),
        }
    }
}

/// How to convert between document offsets and visual positions (task T06.03).
#[derive(Debug, PartialEq, Eq)]
pub struct CoordinatesOptions {
    /// How many columns a tab advances to.
    pub tab: Option<u32>,
    /// Wrap at this many columns.
    pub wrap: Option<u32>,
    /// The file to read the text from, when there is one.
    pub file: Option<PathBuf>,
    /// The operations: `line:<text>`, `fold:<start>:<end>`, `unfold:<start>`, `tab:<n>`,
    /// `wrap:<n>`, `visual:<offset>`, `offset:<row>:<column>`, `mark:<kind>:<start>:<end>`, `show`.
    pub ops: Vec<String>,
}

impl CoordinatesOptions {
    /// Options for an empty document with a four-column tab.
    pub fn new() -> Self {
        Self {
            tab: None,
            wrap: None,
            file: None,
            ops: Vec::new(),
        }
    }
}

/// How to run multi-cursor editing (task T06.04).
#[derive(Debug, PartialEq, Eq)]
pub struct EditOpsOptions {
    /// The file to read the starting text from, when there is one.
    pub file: Option<PathBuf>,
    /// The operations: `line:<text>`, `carets:<o>:...`, `select:<anchor>:<cursor>`, `insert:<text>`,
    /// `type:<char>`, `backspace`, `delete`, `up`, `down`, `duplicate`, `delete-lines`,
    /// `comment:<prefix>`, `matching:<offset>`, `undo`, `redo`, `show`.
    pub ops: Vec<String>,
}

impl EditOpsOptions {
    /// Options for an empty document.
    pub fn new() -> Self {
        Self {
            file: None,
            ops: Vec::new(),
        }
    }
}

/// How to read a file's policy and show the editor's decoration (task T06.05).
#[derive(Debug, PartialEq, Eq)]
pub struct EditorViewOptions {
    /// How many columns a tab advances to.
    pub tab: Option<u32>,
    /// The file to read, when there is one; otherwise the `line:` operations build the text.
    pub file: Option<PathBuf>,
    /// The operations: `line:<text>`, `whitespace`, `numbers[:relative]`,
    /// `window:<first>:<count>[:<current>]`, `fold:indent|braces`, `minimap:<height>`,
    /// `encode`, `encode-with:<lf|crlf|cr>`, `show`.
    pub ops: Vec<String>,
}

impl EditorViewOptions {
    /// Options for an empty document with a four-column tab.
    pub fn new() -> Self {
        Self {
            tab: None,
            file: None,
            ops: Vec::new(),
        }
    }
}

/// How to check writes against protected regions (task T06.06).
#[derive(Debug, PartialEq, Eq)]
pub struct WriteGuardOptions {
    /// The operations: `protect:<start>:<end>`, `writable`, `read-only[:<reason>]`,
    /// `input:<at>:<insert>`, `paste:<at>:<insert>`, `replace:<at>:<remove>:<insert>`,
    /// `format:<at>:<remove>:<insert>`, `external:<at>:<remove>`, `show`.
    pub ops: Vec<String>,
}

impl WriteGuardOptions {
    /// Options for a guard over no regions of a writable document.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to classify a document's size and show the degradation that follows (task T06.07).
#[derive(Debug, PartialEq, Eq)]
pub struct ScaleOptions {
    /// The file to classify, when there is one.
    pub file: Option<PathBuf>,
    /// The chunk size a cancellable read uses, in bytes.
    pub chunk: Option<usize>,
    /// The operations: `line:<text>`, `fill:<bytes>`, `long-line:<columns>`, `read`, `read-cancel`,
    /// `show`.
    pub ops: Vec<String>,
}

impl ScaleOptions {
    /// Options for an empty document.
    pub fn new() -> Self {
        Self {
            file: None,
            chunk: None,
            ops: Vec::new(),
        }
    }
}

/// How to expand a blueprint snippet and walk its placeholders (task T06.09).
#[derive(Debug, PartialEq, Eq)]
pub struct SnippetOptions {
    /// The operations, in the order they run: `text:<text>` and `snippet:<prefix>:<body>` set up the
    /// document and the snippet; `protect:<start>:<end>` and `read-only[:<reason>]` set the guard;
    /// `cursor:<at>`, `select:<start>:<end>`, `add-caret:<at>` and `add-select:<start>:<end>` place
    /// the carets; `expand:<prefix>`, `tab`, `shift-tab`, `type:<text>`, `cancel`, `undo` and
    /// `surround:<open>:<close>` act; `show` reports.
    pub ops: Vec<String>,
}

impl SnippetOptions {
    /// Options for an empty document with no snippet.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to replay the editing regression scenarios (task T06.08).
#[derive(Debug, PartialEq, Eq)]
pub struct RegressionOptions {
    /// The line count to use instead of each scenario's default.
    pub lines: Option<usize>,
    /// The scenarios to run: `normal`, `million`, `unicode`, `multi-view`; all of them when empty.
    pub scenarios: Vec<String>,
}

impl RegressionOptions {
    /// Options for every scenario at its default size.
    pub fn new() -> Self {
        Self {
            lines: None,
            scenarios: Vec::new(),
        }
    }
}

/// How to grow a selection, keep bookmarks, and toggle display elements (task T06.10).
#[derive(Debug, PartialEq, Eq)]
pub struct StructuralOptions {
    /// The operations, in the order they run: `text:<t>`, `cursor:<at>`, `select:<s>:<e>`,
    /// `expand:<n>`, `bookmark:<name>:<at>`, `drop:<name>`, `edit:<at>:<remove>:<insert>`,
    /// `toggle:<name>`, `baseline:<text>`, `inline`, `show`.
    pub ops: Vec<String>,
}

impl StructuralOptions {
    /// Options for an empty document.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// Parses this family's flags, answering `None` when the flag is another family's.
pub(super) fn parse_editor(
    text: &str,
    _argument: OsString,
    arguments: &mut std::vec::IntoIter<OsString>,
) -> Option<Result<Command, ParseError>> {
    match text {
        "--text" => {
            let mut options = TextOptions::new();
            let rest = arguments;
            while let Some(argument) = rest.next() {
                let value = argument.to_str().unwrap_or_default();
                if value == "--file" {
                    let Some(file) = rest.next() else {
                        return Some(Err(ParseError { argument }));
                    };
                    options.file = Some(PathBuf::from(file));
                    continue;
                }
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Text(options)));
        }
        "--highlight" => {
            let mut options = HighlightOptions::new();
            let rest = arguments;
            while let Some(argument) = rest.next() {
                let flag = argument.to_str().unwrap_or_default().to_owned();
                match flag.as_str() {
                    "--language" | "--theme" | "--file" => {
                        let Some(raw) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(text) = raw.to_str() else {
                            return Some(Err(ParseError { argument: raw }));
                        };
                        match flag.as_str() {
                            "--language" => options.language = Some(text.to_owned()),
                            "--theme" => options.theme = Some(text.to_owned()),
                            _ => options.file = Some(PathBuf::from(text)),
                        }
                    }
                    _ if flag.starts_with("--") => return Some(Err(ParseError { argument })),
                    _ => options.ops.push(flag),
                }
            }
            return Some(Ok(Command::Highlight(options)));
        }
        "--coordinates" => {
            let mut options = CoordinatesOptions::new();
            let rest = arguments;
            while let Some(argument) = rest.next() {
                let flag = argument.to_str().unwrap_or_default().to_owned();
                match flag.as_str() {
                    "--tab" | "--wrap" | "--file" => {
                        let Some(raw) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(text) = raw.to_str() else {
                            return Some(Err(ParseError { argument: raw }));
                        };
                        match flag.as_str() {
                            "--tab" | "--wrap" => {
                                let Some(number) = text.parse::<u32>().ok() else {
                                    return Some(Err(ParseError { argument: raw }));
                                };
                                if flag == "--tab" {
                                    options.tab = Some(number);
                                } else {
                                    options.wrap = Some(number);
                                }
                            }
                            _ => options.file = Some(PathBuf::from(text)),
                        }
                    }
                    _ if flag.starts_with("--") => return Some(Err(ParseError { argument })),
                    _ => options.ops.push(flag),
                }
            }
            return Some(Ok(Command::Coordinates(options)));
        }
        "--edit-ops" => {
            let mut options = EditOpsOptions::new();
            let rest = arguments;
            while let Some(argument) = rest.next() {
                let flag = argument.to_str().unwrap_or_default().to_owned();
                match flag.as_str() {
                    "--file" => {
                        let Some(raw) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(text) = raw.to_str() else {
                            return Some(Err(ParseError { argument: raw }));
                        };
                        options.file = Some(PathBuf::from(text));
                    }
                    _ if flag.starts_with("--") => return Some(Err(ParseError { argument })),
                    _ => options.ops.push(flag),
                }
            }
            return Some(Ok(Command::EditOps(options)));
        }
        "--editor-view" => {
            let mut options = EditorViewOptions::new();
            let rest = arguments;
            while let Some(argument) = rest.next() {
                let flag = argument.to_str().unwrap_or_default().to_owned();
                match flag.as_str() {
                    "--tab" | "--file" => {
                        let Some(raw) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(text) = raw.to_str() else {
                            return Some(Err(ParseError { argument: raw }));
                        };
                        if flag == "--tab" {
                            let Some(number) = text.parse::<u32>().ok() else {
                                return Some(Err(ParseError { argument: raw }));
                            };
                            options.tab = Some(number);
                        } else {
                            options.file = Some(PathBuf::from(text));
                        }
                    }
                    _ if flag.starts_with("--") => return Some(Err(ParseError { argument })),
                    _ => options.ops.push(flag),
                }
            }
            return Some(Ok(Command::EditorView(options)));
        }
        "--write-guard" => {
            let mut options = WriteGuardOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::WriteGuard(options)));
        }
        "--scale" => {
            let mut options = ScaleOptions::new();
            let rest = arguments;
            while let Some(argument) = rest.next() {
                let flag = argument.to_str().unwrap_or_default().to_owned();
                match flag.as_str() {
                    "--file" | "--chunk" => {
                        let Some(raw) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(text) = raw.to_str() else {
                            return Some(Err(ParseError { argument: raw }));
                        };
                        if flag == "--chunk" {
                            let Some(number) = text.parse::<usize>().ok() else {
                                return Some(Err(ParseError { argument: raw }));
                            };
                            options.chunk = Some(number);
                        } else {
                            options.file = Some(PathBuf::from(text));
                        }
                    }
                    _ if flag.starts_with("--") => return Some(Err(ParseError { argument })),
                    _ => options.ops.push(flag),
                }
            }
            return Some(Ok(Command::Scale(options)));
        }
        "--snippet" => {
            let mut options = SnippetOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Snippet(options)));
        }
        "--regression" => {
            let mut options = RegressionOptions::new();
            let rest = arguments;
            while let Some(argument) = rest.next() {
                let flag = argument.to_str().unwrap_or_default().to_owned();
                match flag.as_str() {
                    "--lines" => {
                        let Some(raw) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(text) = raw.to_str() else {
                            return Some(Err(ParseError { argument: raw }));
                        };
                        let Some(number) = text.parse::<usize>().ok() else {
                            return Some(Err(ParseError { argument: raw }));
                        };
                        options.lines = Some(number);
                    }
                    _ if flag.starts_with("--") => return Some(Err(ParseError { argument })),
                    _ => options.scenarios.push(flag),
                }
            }
            return Some(Ok(Command::Regression(options)));
        }
        "--structural" => {
            let mut options = StructuralOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Structural(options)));
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;

    #[test]
    fn parses_the_in_place_text_operations() {
        assert_eq!(parse(["--text"]), Ok(Command::Text(TextOptions::new())));
        let mut expected = TextOptions::new();
        expected.file = Some(PathBuf::from("pages/home.json"));
        expected.ops = vec!["splice:0:0:x".to_owned(), "undo".to_owned()];
        assert_eq!(
            parse([
                "--text",
                "--file",
                "pages/home.json",
                "splice:0:0:x",
                "undo"
            ]),
            Ok(Command::Text(expected)),
        );

        assert!(parse(["--text", "--file"]).is_err());
        assert!(parse(["--text", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_highlight_operations() {
        assert_eq!(
            parse(["--highlight"]),
            Ok(Command::Highlight(HighlightOptions::new())),
        );
        let mut expected = HighlightOptions::new();
        expected.language = Some("rust".to_owned());
        expected.theme = Some("light".to_owned());
        expected.ops = vec!["line:let x = 1;".to_owned(), "step:1".to_owned()];
        assert_eq!(
            parse([
                "--highlight",
                "--language",
                "rust",
                "--theme",
                "light",
                "line:let x = 1;",
                "step:1"
            ]),
            Ok(Command::Highlight(expected)),
        );

        assert!(parse(["--highlight", "--language"]).is_err());
        assert!(parse(["--highlight", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_coordinate_operations() {
        assert_eq!(
            parse(["--coordinates"]),
            Ok(Command::Coordinates(CoordinatesOptions::new())),
        );
        let mut expected = CoordinatesOptions::new();
        expected.tab = Some(8);
        expected.wrap = Some(80);
        expected.ops = vec![
            "line:\tlet x = 1;".to_owned(),
            "visual:5".to_owned(),
            "mark:cursor:3:3".to_owned(),
        ];
        assert_eq!(
            parse([
                "--coordinates",
                "--tab",
                "8",
                "--wrap",
                "80",
                "line:\tlet x = 1;",
                "visual:5",
                "mark:cursor:3:3"
            ]),
            Ok(Command::Coordinates(expected)),
        );

        assert!(parse(["--coordinates", "--tab"]).is_err());
        assert!(parse(["--coordinates", "--tab", "wide"]).is_err());
        assert!(parse(["--coordinates", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_editing_operations() {
        assert_eq!(
            parse(["--edit-ops"]),
            Ok(Command::EditOps(EditOpsOptions::new())),
        );
        let mut expected = EditOpsOptions::new();
        expected.file = Some(PathBuf::from("main.rs"));
        expected.ops = vec![
            "carets:1:3".to_owned(),
            "insert:-".to_owned(),
            "undo".to_owned(),
        ];
        assert_eq!(
            parse([
                "--edit-ops",
                "--file",
                "main.rs",
                "carets:1:3",
                "insert:-",
                "undo"
            ]),
            Ok(Command::EditOps(expected)),
        );

        assert!(parse(["--edit-ops", "--file"]).is_err());
        assert!(parse(["--edit-ops", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_editor_view_operations() {
        assert_eq!(
            parse(["--editor-view"]),
            Ok(Command::EditorView(EditorViewOptions::new())),
        );
        let mut expected = EditorViewOptions::new();
        expected.tab = Some(2);
        expected.file = Some(PathBuf::from("main.rs"));
        expected.ops = vec![
            "whitespace".to_owned(),
            "fold:indent".to_owned(),
            "minimap:4".to_owned(),
        ];
        assert_eq!(
            parse([
                "--editor-view",
                "--tab",
                "2",
                "--file",
                "main.rs",
                "whitespace",
                "fold:indent",
                "minimap:4"
            ]),
            Ok(Command::EditorView(expected)),
        );

        assert!(parse(["--editor-view", "--file"]).is_err());
        assert!(parse(["--editor-view", "--tab", "wide"]).is_err());
        assert!(parse(["--editor-view", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_write_guard_operations() {
        assert_eq!(
            parse(["--write-guard"]),
            Ok(Command::WriteGuard(WriteGuardOptions::new())),
        );
        let mut expected = WriteGuardOptions::new();
        expected.ops = vec![
            "protect:4:10".to_owned(),
            "input:6:1".to_owned(),
            "read-only".to_owned(),
            "show".to_owned(),
        ];
        assert_eq!(
            parse([
                "--write-guard",
                "protect:4:10",
                "input:6:1",
                "read-only",
                "show"
            ]),
            Ok(Command::WriteGuard(expected)),
        );

        assert!(parse(["--write-guard", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_scale_operations() {
        assert_eq!(parse(["--scale"]), Ok(Command::Scale(ScaleOptions::new())),);
        let mut expected = ScaleOptions::new();
        expected.file = Some(PathBuf::from("big.txt"));
        expected.chunk = Some(4096);
        expected.ops = vec!["read".to_owned(), "show".to_owned()];
        assert_eq!(
            parse([
                "--scale", "--file", "big.txt", "--chunk", "4096", "read", "show"
            ]),
            Ok(Command::Scale(expected)),
        );

        assert!(parse(["--scale", "--file"]).is_err());
        assert!(parse(["--scale", "--chunk", "lots"]).is_err());
        assert!(parse(["--scale", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_snippet_operations() {
        assert_eq!(
            parse(["--snippet"]),
            Ok(Command::Snippet(SnippetOptions::new())),
        );
        let mut expected = SnippetOptions::new();
        expected.ops = vec![
            "snippet:fn:fn ${1:name}".to_owned(),
            "expand:fn".to_owned(),
            "tab".to_owned(),
            "show".to_owned(),
        ];
        assert_eq!(
            parse([
                "--snippet",
                "snippet:fn:fn ${1:name}",
                "expand:fn",
                "tab",
                "show"
            ]),
            Ok(Command::Snippet(expected)),
        );
        assert!(parse(["--snippet", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_regression_operations() {
        assert_eq!(
            parse(["--regression"]),
            Ok(Command::Regression(RegressionOptions::new())),
        );
        let mut expected = RegressionOptions::new();
        expected.lines = Some(1000);
        expected.scenarios = vec!["normal".to_owned(), "unicode".to_owned()];
        assert_eq!(
            parse(["--regression", "--lines", "1000", "normal", "unicode"]),
            Ok(Command::Regression(expected)),
        );
        assert!(parse(["--regression", "--lines"]).is_err());
        assert!(parse(["--regression", "--lines", "lots"]).is_err());
        assert!(parse(["--regression", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_structural_operations() {
        assert_eq!(
            parse(["--structural"]),
            Ok(Command::Structural(StructuralOptions::new())),
        );
        let mut expected = StructuralOptions::new();
        expected.ops = vec![
            "text:hello".to_owned(),
            "cursor:2".to_owned(),
            "expand:3".to_owned(),
            "toggle:inline-diff".to_owned(),
        ];
        assert_eq!(
            parse([
                "--structural",
                "text:hello",
                "cursor:2",
                "expand:3",
                "toggle:inline-diff"
            ]),
            Ok(Command::Structural(expected)),
        );
        assert!(parse(["--structural", "--unknown"]).is_err());
    }
}
