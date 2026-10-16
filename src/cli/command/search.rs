// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The find-and-change commands: diffs, the index, search, quick open, replace,
//! batch replace, navigation history and the acceptance regression.

// Every branch of the flag table answers with `return`, including the ones that could be a
// trailing expression: the branches are independent flags, and writing them alike is worth more
// than saving a keyword. The returns that are *not* needless — the ones inside a loop that refuses
// a bad value — are why the table reads the way it does.
#![allow(clippy::needless_return)]

use super::*;

/// How to compare two texts or trees (task T08.07).
#[derive(Debug, PartialEq, Eq)]
pub struct DiffOptions {
    /// A file to read as the left side.
    pub left: Option<PathBuf>,
    /// A file to read as the right side.
    pub right: Option<PathBuf>,
    /// Treat `\r\n` and `\n` as the same ending; off by default, so an ending shows.
    pub ignore_endings: bool,
    /// Ignore trailing whitespace before an ending; off by default.
    pub ignore_trailing: bool,
    /// The operations: `left:<text>`, `right:<text>`, `tree-left:<path>=<text>`,
    /// `tree-right:<path>=<text>`, `show`, `trees`, `read-only`.
    pub ops: Vec<String>,
}

impl DiffOptions {
    /// Options for two empty texts, compared strictly.
    pub fn new() -> Self {
        Self {
            left: None,
            right: None,
            ignore_endings: false,
            ignore_trailing: false,
            ops: Vec::new(),
        }
    }
}

/// How to build the project file index (task T08.01).
#[derive(Debug, PartialEq, Eq)]
pub struct IndexOptions {
    /// The operations: `node:<path>:<file|dir|symlink>:<size|target>`, `ignore:<p1>,<p2>`,
    /// `binary:<e1>,<e2>`, `max-bytes:<n>`, `max-depth:<n>`, `max-entries:<n>`, `build`,
    /// `create:<path>:<size>`, `remove:<path>`, `modify:<path>:<size>`, `show`.
    pub ops: Vec<String>,
}

impl IndexOptions {
    /// Options for an empty tree under the usual policy.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to search file and project text (task T08.02).
#[derive(Debug, PartialEq, Eq)]
pub struct SearchOptions {
    /// The operations: `file:<path>=<text>`, `pattern:<text>`, `regex`, `literal`, `no-case`,
    /// `whole-word`, `range:<start>:<end>`, `max:<n>`, `search`, `cancel`, `show`.
    pub ops: Vec<String>,
}

impl SearchOptions {
    /// Options for a literal search over no files.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to open files and symbols by a query (task T08.03).
#[derive(Debug, PartialEq, Eq)]
pub struct QuickOpenOptions {
    /// The operations: `file:<path>`, `touch:<path>`, `caps:<c1>,<c2>`, `symbol:<name>:<path>`,
    /// `open:<query>`, `file-symbols:<query>`, `workspace-symbols:<query>`, `show`.
    pub ops: Vec<String>,
}

impl QuickOpenOptions {
    /// Options for no files and every capability.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to build replace candidates (task T08.04).
#[derive(Debug, PartialEq, Eq)]
pub struct ReplaceOptions {
    /// The operations: `file:<path>=<text>`, `pattern:<t>`, `regex`, `no-case`, `whole-word`,
    /// `template:<t>`, `protect:<uri>:<s>:<e>`, `read-only:<uri>`, `build`, `check:<i>`,
    /// `uncheck:<i>`, `check-all`, `uncheck-all`, `show`.
    pub ops: Vec<String>,
}

impl ReplaceOptions {
    /// Options for no files and no pattern.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to run a batch replace (task T08.05).
#[derive(Debug, PartialEq, Eq)]
pub struct BatchOptions {
    /// The operations: `open:<uri>:<version>=<text>`, `protect:<uri>:<s>:<e>`,
    /// `edit:<uri>:<baseline>:<at>:<remove>:<insert>`, `run`, `undo`, `show`.
    pub ops: Vec<String>,
}

impl BatchOptions {
    /// Options for an empty batch over no documents.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to walk the navigation history (task T08.06).
#[derive(Debug, PartialEq, Eq)]
pub struct HistoryNavOptions {
    /// The operations: `visit:<uri>:<offset>`, `edit:<uri>:<offset>`, `problem:<uri>:<offset>`,
    /// `back`, `forward`, `rename:<from>:<to>`, `vanished:<uri>`, `resolve`, `show`.
    pub ops: Vec<String>,
}

impl HistoryNavOptions {
    /// Options for an empty history.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to run the search/replace acceptance regression (task T08.08).
#[derive(Debug, PartialEq, Eq)]
pub struct SearchHealthOptions {
    /// The scenarios to run: `big-project`, `continuous-edits`, `conflict`, `cancel`; all when empty.
    pub scenarios: Vec<String>,
}

impl SearchHealthOptions {
    /// Options for every scenario.
    pub fn new() -> Self {
        Self {
            scenarios: Vec::new(),
        }
    }
}

/// Parses this family's flags, answering `None` when the flag is another family's.
pub(super) fn parse_search(
    text: &str,
    _argument: OsString,
    arguments: &mut std::vec::IntoIter<OsString>,
) -> Option<Result<Command, ParseError>> {
    match text {
        "--diff" => {
            let mut options = DiffOptions::new();
            let rest = arguments;
            while let Some(argument) = rest.next() {
                let flag = argument.to_str().unwrap_or_default().to_owned();
                match flag.as_str() {
                    "--left" | "--right" => {
                        let Some(raw) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(value) = raw.to_str() else {
                            return Some(Err(ParseError { argument: raw }));
                        };
                        if flag == "--left" {
                            options.left = Some(PathBuf::from(value));
                        } else {
                            options.right = Some(PathBuf::from(value));
                        }
                    }
                    "--ignore-endings" => options.ignore_endings = true,
                    "--ignore-trailing" => options.ignore_trailing = true,
                    _ if flag.starts_with("--") => return Some(Err(ParseError { argument })),
                    _ => options.ops.push(flag),
                }
            }
            return Some(Ok(Command::Diff(options)));
        }
        "--index" => {
            let mut options = IndexOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Index(options)));
        }
        "--search" => {
            let mut options = SearchOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Search(options)));
        }
        "--quick-open" => {
            let mut options = QuickOpenOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::QuickOpen(options)));
        }
        "--replace" => {
            let mut options = ReplaceOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Replace(options)));
        }
        "--batch" => {
            let mut options = BatchOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Batch(options)));
        }
        "--history-nav" => {
            let mut options = HistoryNavOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::HistoryNav(options)));
        }
        "--search-health" => {
            let mut options = SearchHealthOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.scenarios.push(value.to_owned());
            }
            return Some(Ok(Command::SearchHealth(options)));
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;

    #[test]
    fn parses_the_diff_operations() {
        assert_eq!(parse(["--diff"]), Ok(Command::Diff(DiffOptions::new())));
        let mut expected = DiffOptions::new();
        expected.left = Some(PathBuf::from("a.txt"));
        expected.right = Some(PathBuf::from("b.txt"));
        expected.ignore_endings = true;
        expected.ops = vec!["show".to_owned(), "trees".to_owned()];
        assert_eq!(
            parse([
                "--diff",
                "--left",
                "a.txt",
                "--right",
                "b.txt",
                "--ignore-endings",
                "show",
                "trees"
            ]),
            Ok(Command::Diff(expected)),
        );
        assert!(parse(["--diff", "--left"]).is_err());
        assert!(parse(["--diff", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_index_operations() {
        assert_eq!(parse(["--index"]), Ok(Command::Index(IndexOptions::new())));
        let mut expected = IndexOptions::new();
        expected.ops = vec![
            "node:src/main.rs:file:100".to_owned(),
            "node:loop:symlink:..".to_owned(),
            "build".to_owned(),
        ];
        assert_eq!(
            parse([
                "--index",
                "node:src/main.rs:file:100",
                "node:loop:symlink:..",
                "build"
            ]),
            Ok(Command::Index(expected)),
        );
        assert!(parse(["--index", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_search_operations() {
        assert_eq!(
            parse(["--search"]),
            Ok(Command::Search(SearchOptions::new()))
        );
        let mut expected = SearchOptions::new();
        expected.ops = vec![
            "pattern:foo".to_owned(),
            "no-case".to_owned(),
            "search".to_owned(),
        ];
        assert_eq!(
            parse(["--search", "pattern:foo", "no-case", "search"]),
            Ok(Command::Search(expected)),
        );
        assert!(parse(["--search", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_quick_open_operations() {
        assert_eq!(
            parse(["--quick-open"]),
            Ok(Command::QuickOpen(QuickOpenOptions::new())),
        );
        let mut expected = QuickOpenOptions::new();
        expected.ops = vec!["file:src/main.rs".to_owned(), "open:main".to_owned()];
        assert_eq!(
            parse(["--quick-open", "file:src/main.rs", "open:main"]),
            Ok(Command::QuickOpen(expected)),
        );
        assert!(parse(["--quick-open", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_replace_operations() {
        assert_eq!(
            parse(["--replace"]),
            Ok(Command::Replace(ReplaceOptions::new())),
        );
        let mut expected = ReplaceOptions::new();
        expected.ops = vec![
            "pattern:x".to_owned(),
            "template:N".to_owned(),
            "build".to_owned(),
        ];
        assert_eq!(
            parse(["--replace", "pattern:x", "template:N", "build"]),
            Ok(Command::Replace(expected)),
        );
        assert!(parse(["--replace", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_batch_operations() {
        assert_eq!(parse(["--batch"]), Ok(Command::Batch(BatchOptions::new())));
        let mut expected = BatchOptions::new();
        expected.ops = vec![
            "open:a.rs:1=aaa".to_owned(),
            "edit:a.rs:1:0:1:X".to_owned(),
            "run".to_owned(),
        ];
        assert_eq!(
            parse(["--batch", "open:a.rs:1=aaa", "edit:a.rs:1:0:1:X", "run"]),
            Ok(Command::Batch(expected)),
        );
        assert!(parse(["--batch", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_history_nav_operations() {
        assert_eq!(
            parse(["--history-nav"]),
            Ok(Command::HistoryNav(HistoryNavOptions::new())),
        );
        let mut expected = HistoryNavOptions::new();
        expected.ops = vec![
            "visit:a.rs:0".to_owned(),
            "back".to_owned(),
            "resolve".to_owned(),
        ];
        assert_eq!(
            parse(["--history-nav", "visit:a.rs:0", "back", "resolve"]),
            Ok(Command::HistoryNav(expected)),
        );
        assert!(parse(["--history-nav", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_search_health_operations() {
        assert_eq!(
            parse(["--search-health"]),
            Ok(Command::SearchHealth(SearchHealthOptions::new())),
        );
        let mut expected = SearchHealthOptions::new();
        expected.scenarios = vec!["conflict".to_owned(), "cancel".to_owned()];
        assert_eq!(
            parse(["--search-health", "conflict", "cancel"]),
            Ok(Command::SearchHealth(expected)),
        );
        assert!(parse(["--search-health", "--unknown"]).is_err());
    }
}
