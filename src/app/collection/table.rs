// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The columns of a table control, and the honest mapping of a source row onto them (task T27.02).
//!
//! The acceptance line is *"a type mismatch / missing field is diagnosed; a row is never silently
//! truncated"*. The whole design is a refusal to drop anything:
//!
//! * **Mapping produces a diagnosis, not a shortened row.** [`map_row`] attempts *every* declared
//!   column. A field the source did not send is [`Diagnosis::MissingField`]; a field whose text does
//!   not parse as the column's type is [`Diagnosis::TypeMismatch`] with the type that was expected.
//! * **All or nothing.** When any column fails, the whole row is refused with the full list of
//!   diagnoses, so a caller can never mistake a partially filled row for a good one. A row that does
//!   map has one cell per declared column — never fewer.
//! * **No row disappears.** [`map_all`] answers one [`RowReport`] per source row, so a batch mapping
//!   cannot silently lose a row either.
//! * **The empty state is named.** [`Table::render`] states the empty message rather than showing a
//!   blank that could be mistaken for "no problem".

#![allow(dead_code)]

/// A column's type, which is how a source field is read.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ColumnType {
    /// Free text; any field is accepted.
    Text,
    /// A whole number.
    Integer,
    /// A fractional number.
    Number,
    /// A flag.
    Bool,
    /// A choice from a named set; an empty set accepts any text.
    Choice(Vec<String>),
}

impl ColumnType {
    /// The type's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Integer => "integer",
            Self::Number => "number",
            Self::Bool => "bool",
            Self::Choice(_) => "choice",
        }
    }

    /// Reads a type's name. A `choice` starts with no allowed values, so any text is accepted until
    /// the set is named.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "text" => Some(Self::Text),
            "integer" => Some(Self::Integer),
            "number" => Some(Self::Number),
            "bool" => Some(Self::Bool),
            "choice" => Some(Self::Choice(Vec::new())),
            _ => None,
        }
    }

    /// Reads a raw field as this type, or says why it does not fit.
    pub fn accepts(&self, raw: &str) -> Result<Value, String> {
        match self {
            Self::Text => Ok(Value::Text(raw.to_owned())),
            Self::Integer => raw
                .parse::<i64>()
                .map(Value::Integer)
                .map_err(|_| format!("{raw:?} is not a whole number")),
            Self::Number => raw
                .parse::<f64>()
                .map(Value::Number)
                .map_err(|_| format!("{raw:?} is not a number")),
            Self::Bool => match raw {
                "true" | "yes" | "1" => Ok(Value::Bool(true)),
                "false" | "no" | "0" => Ok(Value::Bool(false)),
                other => Err(format!("{other:?} is not a flag")),
            },
            Self::Choice(allowed) => {
                if allowed.is_empty() || allowed.iter().any(|choice| choice == raw) {
                    Ok(Value::Choice(raw.to_owned()))
                } else {
                    Err(format!("{raw:?} is not one of {}", allowed.join(", ")))
                }
            }
        }
    }
}

/// A typed cell value, one per [`ColumnType`] family.
#[derive(Clone, PartialEq, Debug)]
pub enum Value {
    /// Free text.
    Text(String),
    /// A whole number.
    Integer(i64),
    /// A fractional number.
    Number(f64),
    /// A flag.
    Bool(bool),
    /// A choice.
    Choice(String),
}

impl Value {
    /// The value in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Text(text) => text.clone(),
            Self::Integer(number) => number.to_string(),
            Self::Number(number) => number.to_string(),
            Self::Bool(flag) => flag.to_string(),
            Self::Choice(choice) => choice.clone(),
        }
    }
}

/// How a column is ordered.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sort {
    /// No sort.
    None,
    /// Smallest first.
    Ascending,
    /// Largest first.
    Descending,
}

impl Sort {
    /// The sort's name.
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Ascending => "asc",
            Self::Descending => "desc",
        }
    }

    /// Reads a sort's name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "none" => Some(Self::None),
            "asc" => Some(Self::Ascending),
            "desc" => Some(Self::Descending),
            _ => None,
        }
    }
}

/// A column's filter: a substring the value must contain.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Filter {
    /// The substring a value must contain.
    pub contains: String,
}

/// A table column and how it is edited.
#[derive(Clone, PartialEq, Debug)]
pub struct Column {
    /// The header shown above the column.
    pub name: String,
    /// The column's type.
    pub kind: ColumnType,
    /// The column's width in logical units; always positive.
    pub width: u32,
    /// How the column is ordered.
    pub sort: Sort,
    /// The column's filter, when one is set.
    pub filter: Option<Filter>,
}

/// Which rows are selected, by display position.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Selection {
    /// The selected row positions, in the order they were selected.
    pub rows: Vec<usize>,
}

impl Selection {
    /// An empty selection.
    pub fn new() -> Self {
        Self::default()
    }

    /// Selects a row, or deselects it when it was already selected.
    pub fn toggle(&mut self, index: usize) {
        match self.rows.iter().position(|row| *row == index) {
            Some(position) => {
                self.rows.remove(position);
            }
            None => self.rows.push(index),
        }
    }

    /// Whether a row is selected.
    pub fn contains(&self, index: usize) -> bool {
        self.rows.contains(&index)
    }

    /// Whether nothing is selected.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// A table's columns, its selection and its empty state.
#[derive(Clone, PartialEq, Debug)]
pub struct Table {
    /// The columns, in display order.
    pub columns: Vec<Column>,
    /// Which rows are selected.
    pub selection: Selection,
    /// The message shown when the table has no rows.
    pub empty_message: String,
}

impl Table {
    /// A table with no columns.
    pub fn new() -> Self {
        Self {
            columns: Vec::new(),
            selection: Selection::new(),
            empty_message: "no rows".to_owned(),
        }
    }

    /// Adds a column, refusing a duplicate header or a non-positive width.
    pub fn add_column(&mut self, name: &str, kind: ColumnType, width: u32) -> Result<(), String> {
        if self.columns.iter().any(|column| column.name == name) {
            return Err(format!("column {name:?} already exists"));
        }
        if width == 0 {
            return Err(format!("column {name:?} needs a positive width"));
        }
        self.columns.push(Column {
            name: name.to_owned(),
            kind,
            width,
            sort: Sort::None,
            filter: None,
        });
        Ok(())
    }

    /// The column with this header.
    pub fn column(&self, name: &str) -> Option<&Column> {
        self.columns.iter().find(|column| column.name == name)
    }

    /// Sets a column's width, refusing an unknown column or a non-positive width.
    pub fn set_width(&mut self, name: &str, width: u32) -> Result<(), String> {
        if width == 0 {
            return Err(format!("column {name:?} needs a positive width"));
        }
        let column = self
            .columns
            .iter_mut()
            .find(|column| column.name == name)
            .ok_or_else(|| format!("column {name:?} is not in this table"))?;
        column.width = width;
        Ok(())
    }

    /// Sets a column's sort, refusing an unknown column.
    pub fn sort(&mut self, name: &str, direction: Sort) -> Result<(), String> {
        let column = self
            .columns
            .iter_mut()
            .find(|column| column.name == name)
            .ok_or_else(|| format!("column {name:?} is not in this table"))?;
        column.sort = direction;
        Ok(())
    }

    /// Sets a column's filter, refusing an unknown column.
    pub fn filter(&mut self, name: &str, contains: &str) -> Result<(), String> {
        let column = self
            .columns
            .iter_mut()
            .find(|column| column.name == name)
            .ok_or_else(|| format!("column {name:?} is not in this table"))?;
        column.filter = Some(Filter {
            contains: contains.to_owned(),
        });
        Ok(())
    }

    /// The message shown when the table has no rows, named rather than left blank.
    pub fn empty_state(&self) -> String {
        self.empty_message.clone()
    }

    /// The table's columns and selection as lines.
    pub fn render(&self) -> Vec<String> {
        if self.columns.is_empty() {
            return vec![format!("empty: {}", self.empty_state())];
        }
        let mut lines = vec![
            self.columns
                .iter()
                .map(|column| {
                    format!(
                        "{} [{}] w{} {}",
                        column.name,
                        column.kind.name(),
                        column.width,
                        column.sort.name()
                    )
                })
                .collect::<Vec<_>>()
                .join(" | "),
        ];
        for column in &self.columns {
            if let Some(filter) = &column.filter {
                lines.push(format!(
                    "filter {} contains {:?}",
                    column.name, filter.contains
                ));
            }
        }
        if !self.selection.is_empty() {
            lines.push(format!(
                "selected rows: {}",
                self.selection
                    .rows
                    .iter()
                    .map(|row| row.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        lines
    }
}

impl Default for Table {
    fn default() -> Self {
        Self::new()
    }
}

/// One row as the data source answers it: a key and the fields the source sent.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SourceRow {
    /// The row's key, which identifies it in the source.
    pub key: String,
    /// The fields the source sent, by name, in the order they arrived.
    pub fields: Vec<(String, String)>,
}

impl SourceRow {
    /// A source row with a key and no fields.
    pub fn new(key: &str) -> Self {
        Self {
            key: key.to_owned(),
            fields: Vec::new(),
        }
    }

    /// Records a field, replacing any earlier value with the same name.
    pub fn set(&mut self, name: &str, value: &str) {
        match self.fields.iter_mut().find(|(field, _)| field == name) {
            Some((_, existing)) => *existing = value.to_owned(),
            None => self.fields.push((name.to_owned(), value.to_owned())),
        }
    }

    /// The raw text of a field, when the source sent it.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(field, _)| field == name)
            .map(|(_, value)| value.as_str())
    }
}

/// A mapped row's cell.
#[derive(Clone, PartialEq, Debug)]
pub struct Cell {
    /// The column the cell belongs to.
    pub column: String,
    /// The typed value.
    pub value: Value,
}

/// A source row mapped onto the table's typed columns.
#[derive(Clone, PartialEq, Debug)]
pub struct Row {
    /// The source row's key.
    pub key: String,
    /// One cell per declared column, in column order.
    pub cells: Vec<Cell>,
}

/// One reason a source row could not be mapped.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Diagnosis {
    /// The source did not send a field the column needs.
    MissingField {
        /// The column whose field is missing.
        column: String,
    },
    /// The source sent a field that is not the column's type.
    TypeMismatch {
        /// The column that could not read the field.
        column: String,
        /// The raw text the source sent.
        raw: String,
        /// The type the column expected.
        expected: &'static str,
        /// Why the raw text does not fit.
        reason: String,
    },
}

impl Diagnosis {
    /// The column the diagnosis is about.
    pub fn column(&self) -> &str {
        match self {
            Self::MissingField { column } | Self::TypeMismatch { column, .. } => column,
        }
    }

    /// The diagnosis in one line.
    pub fn render(&self) -> String {
        match self {
            Self::MissingField { column } => format!("{column}: the source sent no such field"),
            Self::TypeMismatch {
                column,
                raw,
                expected,
                reason,
            } => format!("{column}: {raw:?} is not {expected} ({reason})"),
        }
    }
}

/// Maps one source row onto the table's typed columns.
///
/// Every declared column is attempted and every problem is reported, so a missing field and a wrong
/// type are two [`Diagnosis`] values rather than a quietly shortened row. When any column fails the
/// whole row is refused with all of its diagnoses; a row that maps has one cell per column.
pub fn map_row(columns: &[Column], source: &SourceRow) -> Result<Row, Vec<Diagnosis>> {
    let mut diagnoses = Vec::new();
    let mut cells = Vec::new();
    for column in columns {
        match source.get(&column.name) {
            None => diagnoses.push(Diagnosis::MissingField {
                column: column.name.clone(),
            }),
            Some(raw) => match column.kind.accepts(raw) {
                Ok(value) => cells.push(Cell {
                    column: column.name.clone(),
                    value,
                }),
                Err(reason) => diagnoses.push(Diagnosis::TypeMismatch {
                    column: column.name.clone(),
                    raw: raw.to_owned(),
                    expected: column.kind.name(),
                    reason,
                }),
            },
        }
    }
    if diagnoses.is_empty() {
        Ok(Row {
            key: source.key.clone(),
            cells,
        })
    } else {
        Err(diagnoses)
    }
}

/// One source row's mapping outcome, kept under its key so no row is dropped.
#[derive(Clone, PartialEq, Debug)]
pub struct RowReport {
    /// The source row's key.
    pub key: String,
    /// The mapped row, or every reason it could not be mapped.
    pub row: Result<Row, Vec<Diagnosis>>,
}

/// Maps every source row, answering one report per row in the order they arrived.
pub fn map_all(columns: &[Column], sources: &[SourceRow]) -> Vec<RowReport> {
    sources
        .iter()
        .map(|source| RowReport {
            key: source.key.clone(),
            row: map_row(columns, source),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        ColumnType, Diagnosis, Selection, Sort, SourceRow, Table, Value, map_all, map_row,
    };

    fn typed_table() -> Table {
        let mut table = Table::new();
        table
            .add_column("name", ColumnType::Text, 160)
            .expect("a column");
        table
            .add_column("count", ColumnType::Integer, 80)
            .expect("a column");
        table
            .add_column("done", ColumnType::Bool, 60)
            .expect("a column");
        table
    }

    #[test]
    fn a_missing_field_is_diagnosed_and_the_row_is_refused() {
        let table = typed_table();
        let mut source = SourceRow::new("a");
        source.set("name", "widget");
        source.set("done", "true");
        // `count` was never sent.
        let diagnoses = map_row(&table.columns, &source).expect_err("a diagnosis");
        assert_eq!(diagnoses.len(), 1);
        assert_eq!(diagnoses[0].column(), "count");
        assert!(matches!(diagnoses[0], Diagnosis::MissingField { .. }));
    }

    #[test]
    fn a_type_mismatch_is_diagnosed_with_the_expected_type() {
        let table = typed_table();
        let mut source = SourceRow::new("a");
        source.set("name", "widget");
        source.set("count", "many"); // not a whole number
        source.set("done", "true");
        let diagnoses = map_row(&table.columns, &source).expect_err("a diagnosis");
        assert_eq!(diagnoses.len(), 1);
        assert!(diagnoses[0].render().contains("integer"));
        assert!(diagnoses[0].render().contains("\"many\""));
    }

    #[test]
    fn every_declared_column_appears_in_a_mapped_row() {
        let table = typed_table();
        let mut source = SourceRow::new("a");
        source.set("name", "widget");
        source.set("count", "3");
        source.set("done", "yes");
        // An extra field the table does not declare is ignored, and the row is not shortened.
        source.set("notes", "extra");
        let row = map_row(&table.columns, &source).expect("a row");
        assert_eq!(row.cells.len(), table.columns.len());
        assert_eq!(row.cells[1].value, Value::Integer(3));
        assert_eq!(row.cells[2].value, Value::Bool(true));
    }

    #[test]
    fn a_row_with_several_problems_reports_them_all() {
        let table = typed_table();
        let source = SourceRow::new("a"); // nothing sent at all
        let diagnoses = map_row(&table.columns, &source).expect_err("diagnoses");
        assert_eq!(diagnoses.len(), table.columns.len());
    }

    #[test]
    fn every_source_row_produces_a_report_so_none_is_dropped() {
        let table = typed_table();
        let mut good = SourceRow::new("good");
        good.set("name", "ok");
        good.set("count", "1");
        good.set("done", "true");
        let bad = SourceRow::new("bad");
        let reports = map_all(&table.columns, &[good, bad]);
        assert_eq!(reports.len(), 2);
        assert_eq!(reports[0].key, "good");
        assert!(reports[0].row.is_ok());
        assert!(reports[1].row.is_err());
    }

    #[test]
    fn a_zero_width_column_is_refused() {
        let mut table = Table::new();
        assert!(table.add_column("name", ColumnType::Text, 0).is_err());
    }

    #[test]
    fn an_unknown_column_cannot_be_edited() {
        let mut table = typed_table();
        assert!(table.set_width("nope", 100).is_err());
        assert!(table.sort("nope", Sort::Ascending).is_err());
        assert!(table.filter("nope", "x").is_err());
    }

    #[test]
    fn a_duplicate_column_is_refused() {
        let mut table = typed_table();
        assert!(
            table
                .add_column("name", ColumnType::Text, 100)
                .unwrap_err()
                .contains("already exists")
        );
    }

    #[test]
    fn sort_and_filter_are_recorded_on_the_column() {
        let mut table = typed_table();
        table.sort("name", Sort::Descending).expect("a sort");
        table.filter("name", "wid").expect("a filter");
        let column = table.column("name").expect("a column");
        assert_eq!(column.sort, Sort::Descending);
        assert_eq!(
            column
                .filter
                .as_ref()
                .map(|filter| filter.contains.as_str()),
            Some("wid")
        );
        assert!(
            table
                .render()
                .iter()
                .any(|line| line.contains("filter name"))
        );
    }

    #[test]
    fn the_selection_toggles_rows() {
        let mut selection = Selection::new();
        selection.toggle(2);
        assert!(selection.contains(2));
        selection.toggle(2);
        assert!(selection.is_empty());
    }

    #[test]
    fn the_empty_state_is_explicit() {
        let table = Table::new();
        assert_eq!(table.empty_state(), "no rows");
        assert!(table.render()[0].starts_with("empty:"));
    }

    #[test]
    fn reading_a_type_and_a_sort_by_name() {
        assert_eq!(ColumnType::parse("integer"), Some(ColumnType::Integer));
        assert_eq!(ColumnType::parse("blob"), None);
        assert_eq!(Sort::parse("asc"), Some(Sort::Ascending));
        assert_eq!(Sort::parse("sideways"), None);
    }
}
