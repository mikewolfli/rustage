// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The collection-contents panel's rows (roadmap F27, `panel.items`).
//!
//! The rows come from the page document's `collections` member (page format v3). The format types
//! are turned into the host's [`Collection`] only to reuse its own renderer and validation, so the
//! panel does not keep a second copy of "what a sample is" or "what a column role is".
//!
//! **Boundary.** Format v3 stores table/list content (columns, rows, roles and a source contract).
//! Tree items, container pages and cell edit rules are separate hosts models without a page member
//! yet; the panel says so instead of drawing an empty section as if it were complete.

use crate::app::collection::content::{
    Cell, CellRole, Collection, Column, ColumnId, Origin, Row, RowId, SourceContract,
};
use crate::app::data::adapters::Availability;
use crate::project::record::page::{PageAvailability, PageCellRole, PageCollection, PageOrigin};

/// The rows for one page's collection content.
pub(super) fn rows(source: &str, collections: &[PageCollection]) -> Vec<String> {
    let mut rows = vec![format!("page: {source}")];
    if collections.is_empty() {
        rows.push("no collection content is declared in this page".to_owned());
    } else {
        for collection in collections {
            match model(collection) {
                Ok(collection) => rows.extend(collection.render()),
                Err(reason) => rows.push(format!(
                    "collection on control {}: {reason}",
                    collection.node().get()
                )),
            }
        }
    }
    rows.push("tree items: not declared in page format v3".to_owned());
    rows.push("tab pages: not declared in page format v3".to_owned());
    rows.push("cell edit rules: not declared in page format v3".to_owned());
    rows
}

/// The host model for one document collection.
fn model(collection: &PageCollection) -> Result<Collection, String> {
    let origin = match collection.origin() {
        PageOrigin::Sample => Origin::Sample,
        PageOrigin::Source => Origin::Source,
    };
    let columns = collection
        .columns()
        .iter()
        .map(|column| Column {
            id: ColumnId::from_raw(column.id()),
            header: column.header().to_owned(),
            role: match column.role() {
                PageCellRole::Text => CellRole::Text,
                PageCellRole::Check => CellRole::Check,
                PageCellRole::Badge(label) => CellRole::Badge(label.clone()),
                PageCellRole::Reference(row) => CellRole::Reference(RowId::from_raw(*row)),
            },
        })
        .collect();
    let rows = collection
        .rows()
        .iter()
        .map(|row| Row {
            id: RowId::from_raw(row.id()),
            cells: row
                .cells()
                .iter()
                .map(|cell| Cell {
                    column: ColumnId::from_raw(cell.column()),
                    value: cell.value().to_owned(),
                })
                .collect(),
        })
        .collect();
    let source = collection.source().map(|source| SourceContract {
        adapter: source.adapter().to_owned(),
        provides: source.provides().to_vec(),
        availability: match source.availability() {
            PageAvailability::Online => Availability::Online,
            PageAvailability::Offline => Availability::Offline,
        },
    });
    Collection::restore(collection.node(), origin, columns, rows, source)
}

#[cfg(test)]
mod tests {
    use super::rows;
    use crate::contracts::identity::NodeId;
    use crate::project::record::page::{
        PageAvailability, PageCell, PageCellRole, PageCollection, PageColumn, PageOrigin, PageRow,
        PageSource,
    };

    #[test]
    fn a_page_without_collections_says_what_is_missing() {
        let rows = rows("pages/main.json", &[]);
        assert_eq!(rows[0], "page: pages/main.json");
        assert!(rows.iter().any(|row| row.contains("no collection content")));
        assert!(
            rows.iter()
                .any(|row| row.contains("tree items: not declared"))
        );
        assert!(
            rows.iter()
                .any(|row| row.contains("tab pages: not declared"))
        );
    }

    #[test]
    fn a_sample_collection_renders_its_columns_rows_and_roles() {
        let collection = PageCollection::new(
            NodeId::from_raw(2),
            PageOrigin::Sample,
            vec![
                PageColumn::new(1, "Name", PageCellRole::Text),
                PageColumn::new(2, "Done", PageCellRole::Check),
                PageColumn::new(3, "Next", PageCellRole::Reference(1)),
            ],
            vec![PageRow::new(
                1,
                vec![PageCell::new(1, "Write"), PageCell::new(2, "false")],
            )],
            None,
        )
        .expect("a valid collection");
        let rows = rows("pages/main.json", &[collection]);
        assert!(
            rows.iter()
                .any(|row| row.contains("sample collection on control 2")),
            "{rows:?}"
        );
        assert!(
            rows.iter()
                .any(|row| row.contains("Name [text] | Done [check] | Next [reference]")),
            "{rows:?}"
        );
        assert!(
            rows.iter()
                .any(|row| row.contains("Write (sample) | false (sample)")),
            "{rows:?}"
        );
    }

    #[test]
    fn a_source_collection_renders_its_contract() {
        let collection = PageCollection::new(
            NodeId::from_raw(2),
            PageOrigin::Source,
            Vec::new(),
            Vec::new(),
            Some(PageSource::new(
                "api",
                vec!["Name".to_owned()],
                PageAvailability::Offline,
            )),
        )
        .expect("a valid collection");
        let rows = rows("pages/main.json", &[collection]);
        assert!(
            rows.iter()
                .any(|row| row.contains("source api (offline, 1 column(s))")),
            "{rows:?}"
        );
    }
}
