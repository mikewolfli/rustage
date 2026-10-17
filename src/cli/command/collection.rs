// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The collection family's commands: the content model, table columns, tree items, cell delegates,
//! navigation containers, rich content, specialised views, and the end-to-end walk.

#![allow(clippy::needless_return)]

use super::*;

/// How to edit a collection's content model (task T27.01).
#[derive(Debug, PartialEq, Eq)]
pub struct CollectionOptions {
    /// The operations: `control:<n>`, `origin:<sample|source>`, `column:<role>:<header>` (repeatable),
    /// `row` (repeatable), `set:<row>:<column>:<value>`, `move-row:<from>:<to>`,
    /// `move-column:<from>:<to>`, `business`, `show`.
    pub ops: Vec<String>,
}

impl CollectionOptions {
    /// Options with no content.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to edit a table's columns and map its rows (task T27.02).
#[derive(Debug, PartialEq, Eq)]
pub struct TableOptions {
    /// The operations: `column:<name>:<type>`, `width:<name>:<n>`, `sort:<name>:<asc|desc|none>`,
    /// `filter:<name>:<text>`, `select:<index>` (repeatable), `empty:<message>`, `source:<key>`,
    /// `field:<name>:<raw>`, `map`, `show`.
    pub ops: Vec<String>,
}

impl TableOptions {
    /// Options with no columns.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to edit a tree's items and load them (task T27.03).
#[derive(Debug, PartialEq, Eq)]
pub struct TreeOptions {
    /// The operations: `node:<id>:<parent|->:<label>`, `icon:<id>:<role>`, `expand:<id>`,
    /// `collapse:<id>`, `select:<id>`, `reparent:<id>:<parent|->`, `lazy:<id>`,
    /// `answer:<ok|fail|cancel|unknown>:<text>`, `load:<id>:<target>`, `show`.
    pub ops: Vec<String>,
}

impl TreeOptions {
    /// Options with no items.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to edit a cell and commit or cancel it (task T27.04).
#[derive(Debug, PartialEq, Eq)]
pub struct CellEditOptions {
    /// The operations: `row:<n>`, `column:<name>`, `current:<value>`, `candidate:<value>`,
    /// `delegate:<text|number|check|choice>`, `commit`, `cancel`, `show`.
    pub ops: Vec<String>,
}

impl CellEditOptions {
    /// Options with no edit.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to manage a container's pages (task T27.05).
#[derive(Debug, PartialEq, Eq)]
pub struct ContainerOptions {
    /// The operations: `kind:<tabs|stack|scroll|split>`, `page:<id>:<title>:<node>`, `activate:<id>`,
    /// `remove:<id>`, `reorder:<id>:<to>`, `next`, `previous`, `show`.
    pub ops: Vec<String>,
}

impl ContainerOptions {
    /// Options with no pages.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to edit rich content and execute its links (task T27.06).
#[derive(Debug, PartialEq, Eq)]
pub struct RichContentOptions {
    /// The operations: `font:<family>:<size>:<weight>`, `run:<format>:<text>`, `link:<target>:<alt>`,
    /// `image:<asset>:<alt>`, `limit-length:<n>`, `limit-format:<format>` (repeatable), `trust`,
    /// `network:<yes|no>`, `authorize`, `execute-link`, `show`.
    pub ops: Vec<String>,
}

impl RichContentOptions {
    /// Options with no content.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to describe a specialised view and its support (task T27.07).
#[derive(Debug, PartialEq, Eq)]
pub struct ViewOptions {
    /// The operations: `kind:<gauge|bar-chart|line-chart|sparkline|image|video|map>`, `data:<text>`
    /// (repeatable), `capability`, `editor`, `show`.
    pub ops: Vec<String>,
}

impl ViewOptions {
    /// Options with no view.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to run the collection content regression (task T27.08).
#[derive(Debug, PartialEq, Eq)]
pub struct CollectionRegressionOptions {
    /// The operations: `total:<n>`, `viewport:<height>`, `first:<n>`, `mode:<static|live>`,
    /// `locale:<tag>`, `empty:<message>`, `error:<reason>`, `select:<index>` (repeatable),
    /// `edit:<index>:<value>`, `key:<command>`, `check`, `show`.
    pub ops: Vec<String>,
}

impl CollectionRegressionOptions {
    /// Options with no scenario.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// Parses this family's flags, answering `None` when the flag is another family's.
pub(super) fn parse_collection(
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
        "--collection" => ops!(CollectionOptions::new(), Command::Collection),
        "--table" => ops!(TableOptions::new(), Command::Table),
        "--tree" => ops!(TreeOptions::new(), Command::Tree),
        "--cell-edit" => ops!(CellEditOptions::new(), Command::CellEdit),
        "--container" => ops!(ContainerOptions::new(), Command::Container),
        "--rich-content" => ops!(RichContentOptions::new(), Command::RichContent),
        "--view" => ops!(ViewOptions::new(), Command::View),
        "--collection-regression" => {
            ops!(
                CollectionRegressionOptions::new(),
                Command::CollectionRegression
            )
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;

    #[test]
    fn parses_the_collection_operations() {
        assert_eq!(
            parse(["--collection"]),
            Ok(Command::Collection(CollectionOptions::new()))
        );
        let mut expected = CollectionOptions::new();
        expected.ops = vec!["control:1".to_owned(), "origin:sample".to_owned()];
        assert_eq!(
            parse(["--collection", "control:1", "origin:sample"]),
            Ok(Command::Collection(expected)),
        );
        assert!(parse(["--collection", "--oops"]).is_err());
    }

    #[test]
    fn parses_each_collection_flag() {
        assert_eq!(parse(["--table"]), Ok(Command::Table(TableOptions::new())));
        assert_eq!(parse(["--tree"]), Ok(Command::Tree(TreeOptions::new())));
        assert_eq!(
            parse(["--cell-edit"]),
            Ok(Command::CellEdit(CellEditOptions::new()))
        );
        assert_eq!(
            parse(["--container"]),
            Ok(Command::Container(ContainerOptions::new()))
        );
        assert_eq!(
            parse(["--rich-content"]),
            Ok(Command::RichContent(RichContentOptions::new()))
        );
        assert_eq!(parse(["--view"]), Ok(Command::View(ViewOptions::new())));
        assert_eq!(
            parse(["--collection-regression"]),
            Ok(Command::CollectionRegression(
                CollectionRegressionOptions::new()
            )),
        );
    }
}
