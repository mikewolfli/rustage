// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The content inside a collection control: its items, its columns and its cell roles, and the line
//! between a design-time sample and a runtime business value (task T27.01).
//!
//! The acceptance line is *"moving rows/columns keeps references; sample content does not masquerade
//! as business data"*. Both halves are about identity rather than position:
//!
//! * **A reference is an identity, not an index.** A [`Row`] carries a [`RowId`] and a [`Column`] a
//!   [`ColumnId`]; [`CellRole::Reference`] points at a [`RowId`]. [`Collection::move_row`] and
//!   [`Collection::move_column`] reorder the vectors and change no id, so every reference resolves to
//!   the same item before and after a move.
//! * **A sample is labelled.** [`Origin`] separates [`Origin::Sample`], content typed into the design
//!   as an example, from [`Origin::Source`], content the project's data source provides.
//!   [`Collection::business_rows`] refuses a sample collection instead of handing it back as if it
//!   were real data, and [`Collection::render`] marks sample content so a person reading the design is
//!   never misled.
//! * **The source is a separate contract.** [`SourceContract`] says which adapter answers at runtime
//!   and which columns it promises; it is not a second copy of the sample, and the design schema that
//!   holds the sample never doubles as the business data.

#![allow(dead_code)]

use std::collections::BTreeSet;

use crate::app::data::adapters::Availability;
use crate::contracts::identity::NodeId;

/// A row's stable identity inside one collection.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct RowId(u64);

impl RowId {
    /// Rebuilds a row id from the number it was written as.
    pub const fn from_raw(value: u64) -> Self {
        Self(value)
    }

    /// The number this id is written as.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// A column's stable identity inside one collection.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ColumnId(u64);

impl ColumnId {
    /// Rebuilds a column id from the number it was written as.
    pub const fn from_raw(value: u64) -> Self {
        Self(value)
    }

    /// The number this id is written as.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// What a cell is for, which decides how the collection draws it.
///
/// [`CellRole::Reference`] holds a [`RowId`], so a cell that points at another item keeps pointing at
/// that item however the collection is reordered.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum CellRole {
    /// Plain display text.
    Text,
    /// A checkbox-like flag shown as set or unset.
    Check,
    /// A badge drawn from a named set.
    Badge(String),
    /// A link to another item, held by identity.
    Reference(RowId),
}

impl CellRole {
    /// The role's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Check => "check",
            Self::Badge(_) => "badge",
            Self::Reference(_) => "reference",
        }
    }
}

/// Where a collection's content came from.
///
/// The distinction is the design's honesty about data: sample content is an example a person typed
/// in, and it is never presented as a business value.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Origin {
    /// An example typed into the design; never business data.
    Sample,
    /// Content the project's data source provides.
    Source,
}

impl Origin {
    /// The origin's name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Sample => "sample",
            Self::Source => "source",
        }
    }

    /// The label rendered next to the content, so a sample is visible as one.
    pub fn label(self) -> &'static str {
        self.name()
    }

    /// Whether this origin is real business data.
    pub fn is_business(self) -> bool {
        matches!(self, Self::Source)
    }

    /// Reads an origin's name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "sample" => Some(Self::Sample),
            "source" => Some(Self::Source),
            _ => None,
        }
    }
}

/// One column of a collection.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Column {
    /// The column's stable identity.
    pub id: ColumnId,
    /// The header shown above the column.
    pub header: String,
    /// What the column's cells are for.
    pub role: CellRole,
}

/// One cell's value, held against the column's identity.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Cell {
    /// The column this cell belongs to.
    pub column: ColumnId,
    /// The cell's text as written.
    pub value: String,
}

/// One row of a collection.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Row {
    /// The row's stable identity.
    pub id: RowId,
    /// The row's cells, each held against a column identity.
    pub cells: Vec<Cell>,
}

/// The contract that says where a collection's real rows come from.
///
/// This is deliberately not the sample: it names the adapter that answers at runtime and the columns
/// it promises, so the design schema and the business data stay two things.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SourceContract {
    /// The adapter that answers for this collection at runtime.
    pub adapter: String,
    /// The column headers the source promises to provide.
    pub provides: Vec<String>,
    /// Whether the source can currently be reached.
    pub availability: Availability,
}

impl SourceContract {
    /// A contract for an adapter, the columns it promises, and whether it is reachable.
    pub fn new(adapter: &str, provides: &[&str], availability: Availability) -> Self {
        Self {
            adapter: adapter.to_owned(),
            provides: provides.iter().map(|header| (*header).to_owned()).collect(),
            availability,
        }
    }

    /// Whether the source promises a column with this header.
    pub fn covers(&self, header: &str) -> bool {
        self.provides.iter().any(|provided| provided == header)
    }

    /// The contract in one line.
    pub fn render(&self) -> String {
        let reach = match self.availability {
            Availability::Online => "online",
            Availability::Offline => "offline",
        };
        format!(
            "source {} ({reach}, {} column(s))",
            self.adapter,
            self.provides.len()
        )
    }
}

/// A collection control's content: its columns, its rows, and where the content came from.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Collection {
    /// The control node this content belongs to.
    pub control: NodeId,
    /// Whether the content is a sample or source content.
    pub origin: Origin,
    /// The columns, in display order.
    pub columns: Vec<Column>,
    /// The rows, in display order.
    pub rows: Vec<Row>,
    /// The source contract, when the collection is filled from a data source.
    pub source: Option<SourceContract>,
    next_row: u64,
    next_column: u64,
}

impl Collection {
    /// An empty collection for a control, with the given origin.
    pub fn new(control: NodeId, origin: Origin) -> Self {
        Self {
            control,
            origin,
            columns: Vec::new(),
            rows: Vec::new(),
            source: None,
            next_row: 1,
            next_column: 1,
        }
    }

    /// A collection of sample content: the design's own example, labelled as such.
    pub fn sample(control: NodeId) -> Self {
        Self::new(control, Origin::Sample)
    }

    /// A collection filled from a source contract; nothing in it is a sample.
    pub fn from_source(control: NodeId, contract: SourceContract) -> Self {
        let mut collection = Self::new(control, Origin::Source);
        collection.source = Some(contract);
        collection
    }

    /// Adds a column and answers its identity.
    pub fn add_column(&mut self, header: &str, role: CellRole) -> ColumnId {
        let id = ColumnId(self.next_column);
        self.next_column += 1;
        self.columns.push(Column {
            id,
            header: header.to_owned(),
            role,
        });
        id
    }

    /// Adds an empty row and answers its identity.
    pub fn add_row(&mut self) -> RowId {
        let id = RowId(self.next_row);
        self.next_row += 1;
        self.rows.push(Row {
            id,
            cells: Vec::new(),
        });
        id
    }

    /// The column with this identity, wherever it sits.
    pub fn column(&self, id: ColumnId) -> Option<&Column> {
        self.columns.iter().find(|column| column.id == id)
    }

    /// The row with this identity, wherever it sits.
    pub fn row(&self, id: RowId) -> Option<&Row> {
        self.rows.iter().find(|row| row.id == id)
    }

    /// The cell at a row and a column, resolved by identity.
    pub fn cell(&self, row: RowId, column: ColumnId) -> Option<&Cell> {
        self.row(row)?
            .cells
            .iter()
            .find(|cell| cell.column == column)
    }

    /// Where a row sits now, which a move changes.
    pub fn position_of_row(&self, id: RowId) -> Option<usize> {
        self.rows.iter().position(|row| row.id == id)
    }

    /// Where a column sits now, which a move changes.
    pub fn position_of_column(&self, id: ColumnId) -> Option<usize> {
        self.columns.iter().position(|column| column.id == id)
    }

    /// Sets a cell, refusing a row or column this collection does not have.
    pub fn set(&mut self, row: RowId, column: ColumnId, value: &str) -> Result<(), String> {
        if self.column(column).is_none() {
            return Err(format!("column {} is not in this collection", column.get()));
        }
        let target = self
            .rows
            .iter_mut()
            .find(|candidate| candidate.id == row)
            .ok_or_else(|| format!("row {} is not in this collection", row.get()))?;
        match target.cells.iter_mut().find(|cell| cell.column == column) {
            Some(cell) => cell.value = value.to_owned(),
            None => target.cells.push(Cell {
                column,
                value: value.to_owned(),
            }),
        }
        Ok(())
    }

    /// Moves a row from one position to another.
    ///
    /// The row keeps its [`RowId`], so any [`CellRole::Reference`] that names it still resolves. Only
    /// the positions change.
    pub fn move_row(&mut self, from: usize, to: usize) -> Result<(), String> {
        let len = self.rows.len();
        if from >= len || to >= len {
            return Err(format!("row move {from} -> {to} is outside 0..{len}"));
        }
        let row = self.rows.remove(from);
        self.rows.insert(to, row);
        Ok(())
    }

    /// Moves a column from one position to another. Every cell keeps pointing at the same column
    /// identity, so nothing has to be rewritten.
    pub fn move_column(&mut self, from: usize, to: usize) -> Result<(), String> {
        let len = self.columns.len();
        if from >= len || to >= len {
            return Err(format!("column move {from} -> {to} is outside 0..{len}"));
        }
        let column = self.columns.remove(from);
        self.columns.insert(to, column);
        Ok(())
    }

    /// The rows as business data.
    ///
    /// A sample collection is refused rather than passed off as data: the acceptance line is that a
    /// sample does not masquerade as a business value, and this is where that is enforced.
    pub fn business_rows(&self) -> Result<&[Row], String> {
        match self.origin {
            Origin::Source => Ok(&self.rows),
            Origin::Sample => Err(
                "this collection holds sample content; it is not business data until the data \
                 source fills it"
                    .to_owned(),
            ),
        }
    }

    /// Checks unique ids and that every cell names a column this collection has.
    pub fn validate(&self) -> Result<(), String> {
        let mut columns = BTreeSet::new();
        for column in &self.columns {
            if !columns.insert(column.id) {
                return Err(format!("column id {} appears twice", column.id.get()));
            }
        }
        let mut rows = BTreeSet::new();
        for row in &self.rows {
            if !rows.insert(row.id) {
                return Err(format!("row id {} appears twice", row.id.get()));
            }
            for cell in &row.cells {
                if !columns.contains(&cell.column) {
                    return Err(format!(
                        "row {} has a cell for unknown column {}",
                        row.id.get(),
                        cell.column.get()
                    ));
                }
            }
        }
        Ok(())
    }

    /// The collection as lines, marking sample content so it is never mistaken for data.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "{} collection on control {} ({} row(s), {} column(s))",
            self.origin.label(),
            self.control.get(),
            self.rows.len(),
            self.columns.len()
        )];
        if let Some(source) = &self.source {
            lines.push(source.render());
        }
        if self.columns.is_empty() {
            lines.push("(no columns)".to_owned());
            return lines;
        }
        lines.push(
            self.columns
                .iter()
                .map(|column| format!("{} [{}]", column.header, column.role.name()))
                .collect::<Vec<_>>()
                .join(" | "),
        );
        for row in &self.rows {
            let cells = self
                .columns
                .iter()
                .map(|column| match self.cell(row.id, column.id) {
                    Some(cell) if self.origin == Origin::Sample => {
                        format!("{} (sample)", cell.value)
                    }
                    Some(cell) => cell.value.clone(),
                    None => "—".to_owned(),
                })
                .collect::<Vec<_>>()
                .join(" | ");
            lines.push(cells);
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Availability, Cell, CellRole, Collection, ColumnId, Origin, Row, RowId, SourceContract,
    };
    use crate::contracts::identity::NodeId;

    fn control() -> NodeId {
        NodeId::from_raw(7)
    }

    fn sample() -> Collection {
        let mut collection = Collection::sample(control());
        let name = collection.add_column("name", CellRole::Text);
        let _flag = collection.add_column("done", CellRole::Check);
        let first = collection.add_row();
        collection.set(first, name, "first").expect("a cell");
        let second = collection.add_row();
        collection.set(second, name, "second").expect("a cell");
        let third = collection.add_row();
        collection.set(third, name, "third").expect("a cell");
        collection
    }

    #[test]
    fn moving_a_row_keeps_every_reference_valid() {
        let mut collection = sample();
        // A column whose cells point at the third row by identity.
        let third = collection.rows[2].id;
        let link = collection.add_column("links to", CellRole::Reference(third));

        let before = collection.position_of_row(third).expect("a position");
        collection.move_row(0, 2).expect("a move");
        let after = collection.position_of_row(third).expect("still there");

        // The row moved, so its position changed...
        assert_ne!(before, after);
        // ...but the reference still resolves to the same row.
        let Some(CellRole::Reference(target)) = collection.column(link).map(|c| c.role.clone())
        else {
            panic!("the link column keeps its reference role");
        };
        assert_eq!(target, third);
        assert!(collection.row(target).is_some());
    }

    #[test]
    fn moving_a_column_keeps_cells_pointing_at_the_same_column() {
        let mut collection = sample();
        let name = collection.columns[0].id;
        let first = collection.rows[0].id;

        collection.move_column(0, 1).expect("a move");
        // The header moved to a new position...
        assert_eq!(collection.position_of_column(name), Some(1));
        // ...and the cell still resolves against the same column identity.
        assert_eq!(
            collection.cell(first, name).map(|cell| cell.value.as_str()),
            Some("first")
        );
    }

    #[test]
    fn an_out_of_range_move_is_refused() {
        let mut collection = sample();
        assert!(collection.move_row(0, 9).is_err());
        assert!(collection.move_column(3, 0).is_err());
    }

    #[test]
    fn a_sample_is_refused_as_business_data() {
        let collection = sample();
        assert_eq!(collection.origin, Origin::Sample);
        assert!(collection.business_rows().is_err());
    }

    #[test]
    fn a_source_collection_yields_its_business_rows() {
        let contract = SourceContract::new("orders-api", &["name", "done"], Availability::Online);
        assert!(contract.covers("name"));
        assert!(!contract.covers("total"));

        let mut collection = Collection::from_source(control(), contract);
        let name = collection.add_column("name", CellRole::Text);
        let row = collection.add_row();
        collection.set(row, name, "real").expect("a cell");

        let rows = collection.business_rows().expect("business data");
        assert_eq!(rows.len(), 1);
        assert!(
            collection
                .render()
                .iter()
                .any(|line| line.contains("orders-api"))
        );
    }

    #[test]
    fn a_source_origin_is_business_and_a_sample_origin_is_not() {
        assert!(Origin::Source.is_business());
        assert!(!Origin::Sample.is_business());
        assert_eq!(Origin::parse("sample"), Some(Origin::Sample));
        assert_eq!(Origin::parse("live"), None);
    }

    #[test]
    fn render_labels_sample_content() {
        let collection = sample();
        let rendered = collection.render().join("\n");
        assert!(rendered.contains("sample collection"));
        assert!(rendered.contains("first (sample)"));
    }

    #[test]
    fn set_refuses_an_unknown_row_or_column() {
        let mut collection = sample();
        let name = collection.columns[0].id;
        let row = collection.rows[0].id;
        assert!(
            collection
                .set(RowId::from_raw(99), name, "x")
                .unwrap_err()
                .contains("row 99")
        );
        assert!(
            collection
                .set(row, ColumnId::from_raw(99), "x")
                .unwrap_err()
                .contains("column 99")
        );
    }

    #[test]
    fn validate_rejects_a_cell_for_an_unknown_column() {
        let mut collection = sample();
        collection.rows.push(Row {
            id: RowId::from_raw(50),
            cells: vec![Cell {
                column: ColumnId::from_raw(99),
                value: "orphan".to_owned(),
            }],
        });
        assert!(
            collection
                .validate()
                .unwrap_err()
                .contains("unknown column")
        );
    }

    #[test]
    fn a_well_formed_collection_validates() {
        assert!(sample().validate().is_ok());
    }
}
