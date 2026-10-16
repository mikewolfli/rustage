// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! How a text document is shown: whitespace, line numbers, fold regions and a minimap (task T06.05).
//!
//! The four things a reader sees around the text, kept apart from the text itself:
//!
//! * **Whitespace is drawn, not changed.** [`View::lines`] can show a space as `·`, a tab as `→`
//!   padded to the next tab stop and the end of a line as `¬`. The column walker uses
//!   [`crate::app::coordinates::char_width`], the same width the coordinate model uses, so a tab stop
//!   and a cursor column cannot disagree about where a character is.
//! * **Line numbers are a gutter, not a column of the text.** [`View::lines`] prefixes each line with
//!   its number, padded to the width of the document's line count so the gutter does not jump as an
//!   edit adds a line; [`gutter`] also answers relative numbers.
//! * **A fold region says which whole lines can be hidden.** [`fold_regions`] finds them from
//!   indentation or from braces, and hands [`crate::app::coordinates::Coordinates::fold`] a range it
//!   already understands rather than making it guess.
//! * **A minimap is a reduction, not a second document.** [`minimap`] folds the document into a fixed
//!   number of rows, each a band of lines with its first line, its least indentation and how many
//!   non-blank lines it holds — enough to draw a shape, and nothing that could disagree with the text.

#![allow(dead_code)]

use crate::app::coordinates::char_width;

/// The glyph a visible space is drawn as.
pub const SPACE: char = '·';
/// The glyph a visible tab is drawn as.
pub const TAB: char = '→';
/// The glyph a visible end of line is drawn as.
pub const EOL: char = '¬';

/// How a document's lines are shown.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct View {
    /// How many columns a tab advances to.
    pub tab_width: u32,
    /// Whether whitespace is shown as glyphs.
    pub show_whitespace: bool,
    /// Whether line numbers count from the cursor instead of from the top.
    pub relative_numbers: bool,
}

impl View {
    /// A view with a four-column tab, no visible whitespace and absolute numbers.
    pub const fn new() -> Self {
        Self {
            tab_width: 4,
            show_whitespace: false,
            relative_numbers: false,
        }
    }

    /// A view that shows whitespace.
    pub const fn with_whitespace(mut self) -> Self {
        self.show_whitespace = true;
        self
    }

    /// A view with a different tab width.
    pub const fn with_tab_width(mut self, tab_width: u32) -> Self {
        self.tab_width = tab_width;
        self
    }

    /// A view with relative numbers.
    pub const fn with_relative_numbers(mut self) -> Self {
        self.relative_numbers = true;
        self
    }

    /// The numbered, optionally whitespace-visible lines of a window of the document.
    ///
    /// `first` is the first line (0-based), `count` how many to show, and `current` the cursor's line
    /// for relative numbers.
    pub fn lines(
        &self,
        text: &str,
        first: usize,
        count: usize,
        current: Option<usize>,
    ) -> Vec<String> {
        let all: Vec<&str> = text.split('\n').collect();
        let numbers = gutter(all.len(), first, count, current, self.relative_numbers);
        numbers
            .into_iter()
            .enumerate()
            .filter_map(|(offset, number)| {
                all.get(first + offset).map(|line| {
                    let body = if self.show_whitespace {
                        whitespace(line, self.tab_width)
                    } else {
                        (*line).to_owned()
                    };
                    format!("{number} │ {body}")
                })
            })
            .collect()
    }
}

impl Default for View {
    fn default() -> Self {
        Self::new()
    }
}

/// One line with its whitespace visible and `EOL` at its end.
///
/// A space becomes `·`; a tab becomes `→` followed by spaces up to the next tab stop; a wide
/// character advances the column by two, so a tab after `中` still lines up.
pub fn whitespace(line: &str, tab_width: u32) -> String {
    let tab_width = tab_width.max(1);
    let mut out = String::new();
    let mut column = 0u32;
    for character in line.chars() {
        match character {
            ' ' => {
                out.push(SPACE);
                column += 1;
            }
            '\t' => {
                let next = (column / tab_width + 1) * tab_width;
                out.push(TAB);
                for _ in column + 1..next {
                    out.push(SPACE);
                }
                column = next;
            }
            _ => {
                out.push(character);
                column += char_width(character);
            }
        }
    }
    out.push(EOL);
    out
}

/// The gutter for a window of lines, one padded number per line.
///
/// Numbers are padded to the width of `total`, so the gutter does not shift when the document gains
/// or loses a line. With `relative`, every line but `current` counts its distance from `current`.
pub fn gutter(
    total: usize,
    first: usize,
    count: usize,
    current: Option<usize>,
    relative: bool,
) -> Vec<String> {
    let width = total.max(1).to_string().len();
    (first..first + count)
        .map(|line| {
            let number = if relative && Some(line) != current {
                match current {
                    Some(current) => line.abs_diff(current),
                    None => line + 1,
                }
            } else {
                line + 1
            };
            format!("{number:>width$}")
        })
        .collect()
}

/// How a fold region was found.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FoldKind {
    /// A block found by indentation: lines more indented than the line that opens it.
    Indent,
    /// A block found by braces: the lines between a `{` and its matching `}`.
    Braces,
}

impl FoldKind {
    /// The kind's name, for a command line.
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "indent" => Some(Self::Indent),
            "braces" => Some(Self::Braces),
            _ => None,
        }
    }

    /// The kind, as it is written in a report.
    pub fn render(&self) -> &'static str {
        match self {
            Self::Indent => "indent",
            Self::Braces => "braces",
        }
    }
}

/// A run of whole lines that can be hidden: `start` and `end` are line numbers, both inclusive.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FoldRegion {
    start: usize,
    end: usize,
    kind: FoldKind,
}

impl FoldRegion {
    /// The first line of the region.
    pub fn start(&self) -> usize {
        self.start
    }

    /// The last line of the region.
    pub fn end(&self) -> usize {
        self.end
    }

    /// How it was found.
    pub fn kind(&self) -> FoldKind {
        self.kind
    }

    /// How many lines it hides.
    pub fn len(&self) -> usize {
        self.end - self.start + 1
    }

    /// The region as one line.
    pub fn render(&self) -> String {
        format!(
            "{} fold {}..{} ({} line(s))",
            self.kind.render(),
            self.start,
            self.end,
            self.len()
        )
    }
}

/// Finds the foldable regions in a text, by indentation or by braces.
pub fn fold_regions(text: &str, kind: FoldKind) -> Vec<FoldRegion> {
    match kind {
        FoldKind::Braces => brace_regions(text),
        FoldKind::Indent => indent_regions(text),
    }
}

/// Regions from `{` to its matching `}`: at least two lines, so a one-line block does not fold.
fn brace_regions(text: &str) -> Vec<FoldRegion> {
    let mut regions = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    for (index, line) in text.split('\n').enumerate() {
        for character in line.chars() {
            match character {
                '{' => stack.push(index),
                '}' => {
                    if let Some(opened) = stack.pop()
                        && index > opened
                    {
                        regions.push(FoldRegion {
                            start: opened,
                            end: index,
                            kind: FoldKind::Braces,
                        });
                    }
                }
                _ => {}
            }
        }
    }
    regions.sort_by_key(|region| (region.start, region.end));
    regions
}

/// Regions from a line to the last following line that is indented more than it.
fn indent_regions(text: &str) -> Vec<FoldRegion> {
    let lines: Vec<&str> = text.split('\n').collect();
    let indents: Vec<Option<usize>> = lines.iter().map(|line| indent_depth(line)).collect();
    let mut regions = Vec::new();
    for index in 0..lines.len() {
        let Some(base) = indents[index] else {
            continue;
        };
        let mut end = index;
        let mut cursor = index + 1;
        while cursor < lines.len() {
            match indents[cursor] {
                None => cursor += 1,
                Some(depth) if depth > base => {
                    end = cursor;
                    cursor += 1;
                }
                Some(_) => break,
            }
        }
        if end > index {
            regions.push(FoldRegion {
                start: index,
                end,
                kind: FoldKind::Indent,
            });
        }
    }
    regions
}

/// The indentation depth of a line, or `None` when the line is blank.
fn indent_depth(line: &str) -> Option<usize> {
    let trimmed = line.trim_start_matches([' ', '\t']);
    if trimmed.is_empty() {
        return None;
    }
    Some(line.len() - trimmed.len())
}

/// One row of a minimap: a band of lines reduced to the shape it makes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MinimapRow {
    /// The band's first line.
    pub line: usize,
    /// The least indentation among the band's non-blank lines.
    pub indent: u32,
    /// How many of the band's lines are not blank.
    pub ink: u32,
}

impl MinimapRow {
    /// The row as one line.
    pub fn render(&self) -> String {
        format!("line {} indent {} ink {}", self.line, self.indent, self.ink)
    }
}

/// Reduces a document to at most `height` rows, each a band of lines.
///
/// The rows are a summary: the band's first line, its least indentation and how many of its lines are
/// not blank. A document shorter than `height` gives one row per line; a longer one is sampled into
/// `height` even bands, so the shape of a large file costs a fixed number of rows to draw.
pub fn minimap(text: &str, height: usize) -> Vec<MinimapRow> {
    let lines: Vec<&str> = text.split('\n').collect();
    let height = height.max(1);
    let band = lines.len().div_ceil(height).max(1);
    let mut rows = Vec::new();
    let mut start = 0;
    while start < lines.len() && rows.len() < height {
        let end = (start + band).min(lines.len());
        let mut indent = u32::MAX;
        let mut ink = 0u32;
        for line in &lines[start..end] {
            if let Some(depth) = indent_depth(line) {
                indent = indent.min(depth as u32);
                ink += 1;
            }
        }
        rows.push(MinimapRow {
            line: start,
            indent: if indent == u32::MAX { 0 } else { indent },
            ink,
        });
        start = end;
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::{EOL, FoldKind, SPACE, TAB, View, fold_regions, gutter, minimap, whitespace};

    #[test]
    fn whitespace_shows_spaces_tabs_and_the_end_of_a_line() {
        assert_eq!(whitespace("a b", 4), format!("a{SPACE}b{EOL}"));
        assert_eq!(whitespace("  ", 4), format!("{SPACE}{SPACE}{EOL}"));
        // A tab from column 0 with a width of 4 is the glyph and three spaces.
        assert_eq!(
            whitespace("\tx", 4),
            format!("{TAB}{SPACE}{SPACE}{SPACE}x{EOL}")
        );
        // A tab from column 2 reaches the same tab stop, so it is the glyph and one space.
        assert_eq!(whitespace("ab\tx", 4), format!("ab{TAB}{SPACE}x{EOL}"));
    }

    #[test]
    fn a_wide_character_pushes_a_tab_to_the_next_stop() {
        // 中 is two columns wide, so with a width of 2 the tab lands on column 4.
        assert_eq!(whitespace("中\tx", 2), format!("中{TAB}{SPACE}x{EOL}"));
    }

    #[test]
    fn the_gutter_pads_absolute_and_relative_numbers() {
        assert_eq!(
            gutter(12, 0, 3, Some(1), false),
            vec![" 1".to_owned(), " 2".to_owned(), " 3".to_owned()]
        );
        // Relative: every line but the cursor shows its distance from it.
        assert_eq!(
            gutter(12, 0, 3, Some(1), true),
            vec![" 1".to_owned(), " 2".to_owned(), " 1".to_owned()]
        );
        // The width follows the document's line count.
        assert_eq!(
            gutter(100, 98, 2, None, false),
            vec![" 99".to_owned(), "100".to_owned()]
        );
    }

    #[test]
    fn a_view_composes_the_gutter_and_the_visible_whitespace() {
        let view = View::new().with_whitespace();
        assert_eq!(
            view.lines("a b\nc\td", 0, 2, None),
            vec![
                format!("1 │ a{SPACE}b{EOL}"),
                format!("2 │ c{TAB}{SPACE}{SPACE}d{EOL}"),
            ]
        );
        // Without the option, the text is drawn as it is.
        assert_eq!(
            View::new().lines("a b\nc", 0, 2, None),
            vec!["1 │ a b".to_owned(), "2 │ c".to_owned()]
        );
    }

    #[test]
    fn brace_fold_regions_include_nested_blocks() {
        let text = "fn f() {\n    a;\n    b;\n}\nafter\n";
        let regions = fold_regions(text, FoldKind::Braces);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].render(), "braces fold 0..3 (4 line(s))");

        // A block inside a block is a region too, and a one-line block is not. Regions are
        // ordered by where they start, so the outer block comes first.
        let nested = "{\n    {\n    }\n}\n";
        let regions = fold_regions(nested, FoldKind::Braces);
        assert_eq!(regions.len(), 2);
        assert_eq!(regions[0].start(), 0);
        assert_eq!(regions[0].end(), 3);
        assert_eq!(regions[1].start(), 1);
        assert_eq!(regions[1].end(), 2);
    }

    #[test]
    fn indent_fold_regions_follow_the_nesting() {
        let text = "if x {\n    a;\n    b;\n}\nc\n";
        let regions = fold_regions(text, FoldKind::Indent);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].render(), "indent fold 0..2 (3 line(s))");

        // A more-indented block inside a block is a region in its own right.
        let nested = "a\n    b\n        c\n    d\ne\n";
        let regions = fold_regions(nested, FoldKind::Indent);
        assert_eq!(regions.len(), 2);
        assert_eq!(regions[0].start(), 0);
        assert_eq!(regions[0].end(), 3);
        assert_eq!(regions[1].start(), 1);
        assert_eq!(regions[1].end(), 2);
    }

    #[test]
    fn a_minimap_reduces_a_document_to_a_fixed_number_of_rows() {
        let text = "a\n    b\n\nc\nd\ne\nf\ng\nh";
        let rows = minimap(text, 3);
        assert_eq!(rows.len(), 3);
        // Nine lines into three bands of three: the shape, not the text.
        assert_eq!(rows[0].render(), "line 0 indent 0 ink 2");
        assert_eq!(rows[1].render(), "line 3 indent 0 ink 3");
        assert_eq!(rows[2].render(), "line 6 indent 0 ink 3");

        // A document shorter than the height is one row per line.
        let rows = minimap("a\n    b", 8);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].render(), "line 1 indent 4 ink 1");
    }
}
