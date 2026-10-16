// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Where things are on the screen: document offsets to visual rows and columns, and back
//! (task T06.03).
//!
//! The acceptance line is *"the cursor, diagnostics and protected regions all use the same
//! conversion"*. That is a statement about **one** conversion existing, so the module is built so
//! there cannot be three:
//!
//! * **[`Coordinates`] is the only conversion.** A byte offset becomes a [`Visual`] through
//!   [`Coordinates::visual_of`]; a byte range becomes a [`VisualSpan`] through
//!   [`Coordinates::span`]. The cursor, a diagnostic and a protected region are all a [`Mark`] — a
//!   range plus a [`Kind`] — and are all resolved through [`Coordinates::project`], which calls that
//!   one `span`. There is no second place a column is computed, so a cursor and an underline cannot
//!   disagree about where a character is.
//! * **A visual column is display width, not bytes.** [`char_width`] counts a wide character as two
//!   columns and a combining mark as none, and a tab advances to the next [`Metrics::tab_width`]
//!   stop. That is what makes a CJK document and a tab-indented document line up.
//! * **Wrapping and folding are rows, not text.** A line that wraps occupies several visual rows; a
//!   folded range hides its lines' rows and leaves one marker row. Both are decided here, so a
//!   cursor moving down a wrapped line and a diagnostic underlining across a fold agree by going
//!   through the same rows.
//!
//! The byte offsets are the ones [`crate::project::text`] edits with; this module never changes the
//! text, it only says where it is. Character width is a deliberate **approximation** of Unicode
//! `East_Asian_Width` (the common wide and combining ranges), which is what a terminal-grade editor
//! needs and what the acceptance asks for, rather than a full width table.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

/// How a document is laid out for the eye.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Metrics {
    /// How many columns a tab advances to.
    pub tab_width: u32,
    /// Wrap at this many columns, or `None` to never wrap.
    pub wrap: Option<u32>,
}

impl Metrics {
    /// Metrics with a tab width and no wrapping.
    pub const fn new(tab_width: u32, wrap: Option<u32>) -> Self {
        Self { tab_width, wrap }
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            tab_width: 4,
            wrap: None,
        }
    }
}

/// A point on the screen: which visual row, and which column in it.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Visual {
    /// The visual row, counting from the top (wrapped and folded rows included).
    pub row: usize,
    /// The display column within the row.
    pub column: u32,
}

impl Visual {
    /// A point.
    pub const fn new(row: usize, column: u32) -> Self {
        Self { row, column }
    }

    /// The point as one line.
    pub fn render(&self) -> String {
        format!("row {} col {}", self.row, self.column)
    }
}

/// What a mark is for. The three call sites of the acceptance are these three kinds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// The cursor.
    Cursor,
    /// A diagnostic.
    Diagnostic,
    /// A protected (generated) region.
    Protected,
}

impl Kind {
    /// The kind's name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Cursor => "cursor",
            Self::Diagnostic => "diagnostic",
            Self::Protected => "protected",
        }
    }
}

/// A byte range with a purpose: what to convert to the screen.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Mark {
    start: usize,
    end: usize,
    kind: Kind,
}

impl Mark {
    /// A cursor at an offset.
    pub const fn cursor(offset: usize) -> Self {
        Self {
            start: offset,
            end: offset,
            kind: Kind::Cursor,
        }
    }

    /// A diagnostic over a range.
    pub const fn diagnostic(start: usize, end: usize) -> Self {
        Self {
            start,
            end,
            kind: Kind::Diagnostic,
        }
    }

    /// A protected region over a range.
    pub const fn protected(start: usize, end: usize) -> Self {
        Self {
            start,
            end,
            kind: Kind::Protected,
        }
    }

    /// Where the range starts, as a byte offset.
    pub const fn start(&self) -> usize {
        self.start
    }

    /// Where it ends.
    pub const fn end(&self) -> usize {
        self.end
    }

    /// What it is.
    pub const fn kind(&self) -> Kind {
        self.kind
    }
}

/// A mark placed on the screen.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct VisualSpan {
    start: Visual,
    end: Visual,
    kind: Kind,
}

impl VisualSpan {
    /// Where it starts.
    pub const fn start(&self) -> Visual {
        self.start
    }

    /// Where it ends.
    pub const fn end(&self) -> Visual {
        self.end
    }

    /// What it is.
    pub const fn kind(&self) -> Kind {
        self.kind
    }

    /// Whether it covers nothing (a cursor).
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// The span as one line.
    pub fn render(&self) -> String {
        format!(
            "{} {} → {}",
            self.kind.name(),
            self.start.render(),
            self.end.render()
        )
    }
}

/// One visual row: which line it shows, the byte range of that line, and whether it carries a fold
/// marker.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Row {
    line: usize,
    start: usize,
    end: usize,
    hides: bool,
}

/// The conversion between byte offsets and the screen.
#[derive(Clone, Debug)]
pub struct Coordinates {
    lines: Vec<String>,
    starts: Vec<usize>,
    metrics: Metrics,
    folds: BTreeMap<usize, usize>,
    hidden: BTreeSet<usize>,
    row_of_line: Vec<Option<usize>>,
    rows: Vec<Row>,
}

impl Coordinates {
    /// A conversion over a text.
    pub fn new(text: &str, metrics: Metrics) -> Self {
        let mut coordinates = Self {
            lines: Vec::new(),
            starts: Vec::new(),
            metrics,
            folds: BTreeMap::new(),
            hidden: BTreeSet::new(),
            row_of_line: Vec::new(),
            rows: Vec::new(),
        };
        coordinates.set_text(text);
        coordinates
    }

    /// Replaces the text.
    pub fn set_text(&mut self, text: &str) {
        self.lines = text.split('\n').map(str::to_owned).collect();
        self.starts.clear();
        let mut offset = 0;
        for line in &self.lines {
            self.starts.push(offset);
            offset += line.len() + 1;
        }
        // A fold that no longer names a line is dropped rather than kept dangling.
        self.folds
            .retain(|start, end| *start < self.lines.len() && *end < self.lines.len());
        self.rebuild();
    }

    /// Changes the metrics (tab width or wrapping), which re-lays the rows.
    pub fn set_metrics(&mut self, metrics: Metrics) {
        self.metrics = metrics;
        self.rebuild();
    }

    /// The metrics.
    pub const fn metrics(&self) -> Metrics {
        self.metrics
    }

    /// How many document lines there are.
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// How many visual rows there are.
    pub fn visual_rows(&self) -> usize {
        self.rows.len()
    }

    /// How long the text is, in bytes.
    pub fn text_len(&self) -> usize {
        match self.starts.last() {
            Some(start) => start + self.lines.last().map(String::len).unwrap_or(0),
            None => 0,
        }
    }

    /// Folds the lines after `start` up to and including `end`.
    ///
    /// A fold that does not name a real line, or that is empty, is refused rather than recorded:
    /// a fold here is a fact about which rows exist.
    pub fn fold(&mut self, start: usize, end: usize) -> Result<(), String> {
        if start >= self.lines.len() || end >= self.lines.len() {
            return Err(format!("there is no line {start}..{end}"));
        }
        if end <= start {
            return Err("a fold hides at least one line after the one it starts on".to_owned());
        }
        self.folds.insert(start, end);
        self.rebuild();
        Ok(())
    }

    /// Opens a fold.
    pub fn unfold(&mut self, start: usize) {
        self.folds.remove(&start);
        self.rebuild();
    }

    /// The folds, in line order.
    pub fn folds(&self) -> Vec<(usize, usize)> {
        self.folds
            .iter()
            .map(|(start, end)| (*start, *end))
            .collect()
    }

    /// The display width of a piece of text, with tab stops.
    pub fn columns(&self, text: &str) -> u32 {
        let mut column = 0;
        for character in text.chars() {
            column = self.advance(column, character);
        }
        column
    }

    /// Where a byte offset is on the screen.
    ///
    /// An offset inside a folded-away line is reported at the fold: a hidden line has no row of its
    /// own, and inventing one would be a place a cursor could not be drawn.
    pub fn visual_of(&self, offset: usize) -> Visual {
        let offset = offset.min(self.text_len());
        let line = self.line_of(offset);
        if self.hidden.contains(&line) {
            let start = self.fold_start_of(line).unwrap_or(line);
            return Visual::new(self.row_of_line[start].unwrap_or(0), 0);
        }
        let within = offset - self.starts[line];
        // A byte offset inside a character is reported at the nearest boundary before it, so a
        // caller that hands this a mid-character offset gets a defined place and no panic.
        let mut within = within;
        while within > 0 && !self.lines[line].is_char_boundary(within) {
            within -= 1;
        }
        for (index, row) in self.rows.iter().enumerate() {
            if row.line != line {
                continue;
            }
            if within >= row.start && (within < row.end || row.end == self.lines[line].len()) {
                let column = self.columns(&self.lines[line][row.start..within]);
                return Visual::new(index, column);
            }
        }
        // A line that has no rows (only possible when it is hidden, handled above).
        Visual::new(self.row_of_line[line].unwrap_or(0), 0)
    }

    /// The byte offset under a screen point (hit-testing).
    ///
    /// A column past the end of a row lands at the row's end; a row past the last lands at the end of
    /// the text. It never returns an offset that is not a character boundary.
    pub fn offset_at(&self, visual: Visual) -> usize {
        if self.rows.is_empty() {
            return 0;
        }
        if visual.row >= self.rows.len() {
            // Below the last row: the end of the text, which is where a click past the end lands.
            return self.text_len();
        }
        let row = self.rows[visual.row];
        let line = row.line;
        let base = self.starts[line];
        if row.hides {
            // The marker row of a fold: the caret belongs at the fold's own line.
            return base;
        }
        let (byte, _) = self.byte_at_column(line, row.start, visual.column);
        base + byte.min(row.end)
    }

    /// A byte range as a span on the screen.
    pub fn span(&self, start: usize, end: usize) -> VisualSpan {
        VisualSpan {
            start: self.visual_of(start),
            end: self.visual_of(end),
            kind: Kind::Cursor,
        }
    }

    /// Places a mark on the screen. The one conversion every caller goes through.
    pub fn project(&self, mark: &Mark) -> VisualSpan {
        let mut span = self.span(mark.start(), mark.end());
        span.kind = mark.kind();
        span
    }

    /// The conversion in one report.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "{} line(s), {} visual row(s), tab {} , wrap {}",
            self.lines.len(),
            self.rows.len(),
            self.metrics.tab_width,
            match self.metrics.wrap {
                Some(width) => width.to_string(),
                None => "off".to_owned(),
            }
        )];
        for (index, row) in self.rows.iter().enumerate() {
            let text = &self.lines[row.line][row.start..row.end];
            let folded = if row.hides { "  ⋯" } else { "" };
            lines.push(format!(
                "  row {index} ← line {} [{}..{}] {} column(s){folded}",
                row.line,
                row.start,
                row.end,
                self.columns(text)
            ));
        }
        lines
    }

    /// The column after one character, honouring tab stops.
    fn advance(&self, column: u32, character: char) -> u32 {
        if character == '\t' {
            let width = self.metrics.tab_width.max(1);
            column + (width - column % width)
        } else {
            column + char_width(character)
        }
    }

    /// The line a byte offset falls in.
    fn line_of(&self, offset: usize) -> usize {
        match self.starts.binary_search(&offset) {
            Ok(index) => index,
            Err(index) => index.saturating_sub(1),
        }
    }

    /// The fold a hidden line belongs to.
    fn fold_start_of(&self, line: usize) -> Option<usize> {
        self.folds
            .iter()
            .find(|(start, end)| **start < line && line <= **end)
            .map(|(start, _)| *start)
    }

    /// The byte within a line where a column is reached, and the column actually reached.
    fn byte_at_column(&self, line: usize, from: usize, columns: u32) -> (usize, u32) {
        let text = &self.lines[line];
        let mut column = 0;
        let mut byte = from;
        for (offset, character) in text[from..].char_indices() {
            if column >= columns {
                return (from + offset, column);
            }
            column = self.advance(column, character);
            byte = from + offset + character.len_utf8();
        }
        (byte, column)
    }

    /// Rebuilds the rows from the text, the metrics and the folds.
    fn rebuild(&mut self) {
        self.hidden.clear();
        for (start, end) in &self.folds {
            for line in (start + 1)..=*end {
                self.hidden.insert(line);
            }
        }
        self.row_of_line = vec![None; self.lines.len()];
        self.rows = Vec::new();
        for line in 0..self.lines.len() {
            if self.hidden.contains(&line) {
                continue;
            }
            self.row_of_line[line] = Some(self.rows.len());
            let ranges = self.wrap_ranges(line);
            let hides = self.folds.contains_key(&line);
            let last = ranges.len().saturating_sub(1);
            for (index, (start, end)) in ranges.into_iter().enumerate() {
                self.rows.push(Row {
                    line,
                    start,
                    end,
                    hides: hides && index == last,
                });
            }
        }
    }

    /// The byte ranges a line occupies, split to the wrap width.
    fn wrap_ranges(&self, line: usize) -> Vec<(usize, usize)> {
        let text = &self.lines[line];
        let Some(limit) = self.metrics.wrap.filter(|limit| *limit > 0) else {
            return vec![(0, text.len())];
        };
        let mut ranges = Vec::new();
        let mut row_start = 0;
        let mut column = 0;
        for (byte, character) in text.char_indices() {
            let next = self.advance(column, character);
            if next > limit && byte > row_start {
                ranges.push((row_start, byte));
                row_start = byte;
                column = self.advance(0, character);
            } else {
                column = next;
            }
        }
        ranges.push((row_start, text.len()));
        ranges
    }
}

/// The display width of one character, as an approximation of Unicode `East_Asian_Width`.
///
/// Two for the wide ranges (CJK, Hangul, fullwidth forms, emoji), none for a combining mark or a
/// zero-width joiner, one otherwise. A tab is handled by the column walker, not here.
pub fn char_width(character: char) -> u32 {
    let code = character as u32;
    if character == '\t' || character == '\n' || character == '\r' {
        return 0;
    }
    if code < 0x20 || (0x7f..0xa0).contains(&code) {
        return 0;
    }
    let combining = (0x0300..=0x036f).contains(&code)
        || (0x1ab0..=0x1aff).contains(&code)
        || (0x1dc0..=0x1dff).contains(&code)
        || (0x20d0..=0x20ff).contains(&code)
        || (0xfe20..=0xfe2f).contains(&code)
        || code == 0x200b
        || code == 0x200c
        || code == 0x200d
        || code == 0xfeff;
    if combining {
        return 0;
    }
    if is_wide(code) { 2 } else { 1 }
}

fn is_wide(code: u32) -> bool {
    (0x1100..=0x115f).contains(&code)
        || (0x2e80..=0x303e).contains(&code)
        || (0x3041..=0x33ff).contains(&code)
        || (0x3400..=0x4dbf).contains(&code)
        || (0x4e00..=0x9fff).contains(&code)
        || (0xa000..=0xa4cf).contains(&code)
        || (0xac00..=0xd7a3).contains(&code)
        || (0xf900..=0xfaff).contains(&code)
        || (0xfe30..=0xfe4f).contains(&code)
        || (0xff00..=0xff60).contains(&code)
        || (0xffe0..=0xffe6).contains(&code)
        || (0x1f300..=0x1faff).contains(&code)
        || (0x20000..=0x3fffd).contains(&code)
}

#[cfg(test)]
mod tests {
    use super::{Coordinates, Kind, Mark, Metrics, Visual, char_width};

    fn plain(text: &str) -> Coordinates {
        Coordinates::new(text, Metrics::new(4, None))
    }

    #[test]
    fn tabs_advance_to_the_next_stop() {
        let coordinates = plain("x");
        assert_eq!(coordinates.columns("\tx"), 5);
        assert_eq!(coordinates.columns("\t"), 4);
        assert_eq!(coordinates.columns("a\tb"), 5);
        assert_eq!(coordinates.columns("\t\t"), 8);
        assert_eq!(coordinates.columns("abc"), 3);
    }

    #[test]
    fn wide_and_combining_characters_are_counted_by_display_width() {
        assert_eq!(char_width('中'), 2);
        assert_eq!(char_width('a'), 1);
        assert_eq!(char_width('\u{301}'), 0, "a combining acute adds no column");
        assert_eq!(
            char_width('\u{200d}'),
            0,
            "a zero-width joiner adds no column"
        );
        assert_eq!(char_width('\t'), 0, "a tab is the walker's business");
        // "e" + combining acute is two code points, one column.
        assert_eq!(plain("").columns("e\u{301}"), 1);
        // Two CJK characters are four columns.
        assert_eq!(plain("").columns("中文"), 4);
    }

    #[test]
    fn a_document_offset_maps_to_a_visual_row_and_column() {
        // With a tab on the first line and a wide character on the second.
        let coordinates = plain("\tlet x = 1;\n中文");
        let line_two = "\tlet x = 1;\n".len();

        // Just after the tab: column 4.
        assert_eq!(coordinates.visual_of(1), Visual::new(0, 4));
        // Just after "let": column 4 + 3 = 7.
        assert_eq!(coordinates.visual_of(1 + 3), Visual::new(0, 7));
        // The second line begins at row 1, column 0.
        assert_eq!(coordinates.visual_of(line_two), Visual::new(1, 0));
        // After the first CJK character: column 2.
        assert_eq!(coordinates.visual_of(line_two + 3), Visual::new(1, 2));
        // After both: column 4.
        assert_eq!(coordinates.visual_of(line_two + 6), Visual::new(1, 4));
    }

    #[test]
    fn wrapping_splits_a_line_into_rows_and_offsets_round_trip() {
        let coordinates = Coordinates::new("abcdefgh", Metrics::new(4, Some(4)));
        assert_eq!(coordinates.visual_rows(), 2);
        assert_eq!(coordinates.visual_of(0), Visual::new(0, 0));
        assert_eq!(coordinates.visual_of(3), Visual::new(0, 3));
        // The fourth character begins the second row.
        assert_eq!(coordinates.visual_of(4), Visual::new(1, 0));
        assert_eq!(coordinates.visual_of(5), Visual::new(1, 1));

        // Hit-testing back: a column in a row finds the byte under it.
        assert_eq!(coordinates.offset_at(Visual::new(0, 2)), 2);
        assert_eq!(coordinates.offset_at(Visual::new(1, 2)), 6);
        // A column past the row's end lands at the row's end.
        assert_eq!(coordinates.offset_at(Visual::new(0, 99)), 4);
        assert_eq!(coordinates.offset_at(Visual::new(9, 0)), 8);
    }

    #[test]
    fn a_folded_range_hides_its_lines_and_keeps_one_marker_row() {
        // Line starts: a=0, b=2, c=4, d=6, e=8.
        let mut coordinates = Coordinates::new("a\nb\nc\nd\ne", Metrics::default());
        assert_eq!(coordinates.visual_rows(), 5);
        assert_eq!(
            coordinates.visual_of(2),
            Visual::new(1, 0),
            "the start of b"
        );

        // Fold b's body (lines c and d away).
        coordinates.fold(1, 3).expect("lines 2..=3 hide");
        assert_eq!(coordinates.visual_rows(), 3, "a, b's marker, and e");
        // A position on a hidden line is reported at the fold, not on a row that is not drawn.
        assert_eq!(
            coordinates.visual_of(4),
            Visual::new(1, 0),
            "c is reported at b"
        );
        assert_eq!(coordinates.visual_of(6), Visual::new(1, 0), "d too");
        // Hit-testing the marker row lands on the fold's own line.
        assert_eq!(coordinates.offset_at(Visual::new(1, 0)), 2);
        // And the line after the fold is still its own row.
        assert_eq!(coordinates.visual_of(8), Visual::new(2, 0));

        coordinates.unfold(1);
        assert_eq!(coordinates.visual_rows(), 5);
        assert_eq!(coordinates.visual_of(4), Visual::new(2, 0));
    }

    #[test]
    fn the_cursor_a_diagnostic_and_a_protected_region_all_use_one_conversion() {
        let coordinates = plain("let x = 1;\n中文 = 2;");

        let cursor = coordinates.project(&Mark::cursor(4));
        let diagnostic = coordinates.project(&Mark::diagnostic(0, 3));
        let protected = coordinates.project(&Mark::protected(0, 3));

        // The cursor is a point, placed by the same conversion the ranges use.
        assert_eq!(cursor.kind(), Kind::Cursor);
        assert!(cursor.is_empty());
        assert_eq!(cursor.start(), coordinates.visual_of(4));

        // A diagnostic and a protected region over the same bytes are placed identically: there is
        // one conversion, so they cannot disagree.
        assert_eq!(diagnostic.start(), protected.start());
        assert_eq!(diagnostic.end(), protected.end());
        assert_eq!(diagnostic.start(), Visual::new(0, 0));
        assert_eq!(diagnostic.end(), coordinates.visual_of(3));

        // Across lines and a wide character, still the same conversion. Line 1 begins at byte 11;
        // `中` is bytes 11..14 and `文` is 14..17.
        let wide = coordinates.project(&Mark::diagnostic(11, 17));
        assert_eq!(wide.start(), coordinates.visual_of(11));
        assert_eq!(wide.end(), coordinates.visual_of(17));
        assert_eq!(
            wide.end(),
            Visual::new(1, 4),
            "two wide characters are four columns"
        );
    }

    #[test]
    fn an_offset_past_the_end_is_clamped_rather_than_panicking() {
        let coordinates = plain("abc");
        assert_eq!(coordinates.visual_of(999), Visual::new(0, 3));
        assert_eq!(coordinates.offset_at(Visual::new(999, 999)), 3);
        // An empty document has one empty line and one row.
        let empty = plain("");
        assert_eq!(empty.line_count(), 1);
        assert_eq!(empty.visual_rows(), 1);
        assert_eq!(empty.visual_of(0), Visual::new(0, 0));
    }

    #[test]
    fn a_fold_that_names_no_line_is_refused() {
        let mut coordinates = plain("a\nb\nc");
        assert!(coordinates.fold(2, 5).is_err());
        assert!(coordinates.fold(2, 2).is_err());
        assert!(coordinates.folds().is_empty());
    }

    #[test]
    fn the_report_names_the_lines_the_rows_and_the_metrics() {
        let coordinates = Coordinates::new("ab\ncd", Metrics::new(8, None));
        let report = coordinates.render().join("\n");
        assert!(report.contains("2 line(s), 2 visual row(s)"), "{report}");
        assert!(report.contains("tab 8"), "{report}");
        assert!(report.contains("wrap off"), "{report}");
    }
}
