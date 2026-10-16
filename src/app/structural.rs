// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Structural selection, bookmarks/edit locations, and display toggles (task T06.10).
//!
//! The acceptance line is *"when there is no syntax capability, selection falls back to text, and it
//! changes neither source coordinates nor protection ranges"*. Selection here is a **read** of the
//! text, so it can do neither by construction, and the module makes the fallback a named outcome
//! rather than a silent one:
//!
//! * **Selection grows over a ladder, and the rung is named.** [`expand`] takes a selection and
//!   returns the next one up — [`Kind::Word`], [`Kind::Line`], [`Kind::Paragraph`],
//!   [`Kind::Document`] — each produced from the text alone. Nothing is written, so no coordinate and
//!   no protected range can move.
//! * **A syntax capability is an input, and its absence is explicit.** A caller that *has* a language
//!   service passes [`Capability::Syntax`] with the spans it knows; selection then grows to the
//!   smallest enclosing span ([`Kind::Syntax`]). With [`Capability::TextOnly`] — which is where the
//!   designer is until [F07](crate::app) is wired — the result is a text selection and
//!   [`Expanded::fell_back`] is `true`, so "we fell back to text" is a fact in the result, not an
//!   assumption.
//! * **A bookmark is an edit location that moves with the edits.** [`Bookmarks`] names positions, and
//!   [`Bookmarks::translate`] shifts them across an edit exactly as the text moves, so a mark stays on
//!   the thing it marked.
//! * **An inline diff is shown only against a real revision.** The display toggle for it answers
//!   [`Toggles::reason`] when there is no baseline, and the baseline is a read-only
//!   [`Revision`](crate::app::diff::Revision) from [T08.07](crate::app::diff), so this task's inline
//!   diff is the one that task built rather than a second, weaker one.

#![allow(dead_code)]

use crate::app::diff::{Revision, Row, diff};
use crate::app::editing::Selection;

/// A structural span a language service knows about (its absence is why selection falls back to text).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Span {
    /// What the span is, for the report.
    pub label: String,
    /// Where it starts.
    pub start: usize,
    /// Where it ends.
    pub end: usize,
}

impl Span {
    /// A labelled span over `[start, end)`.
    pub fn new(label: impl Into<String>, start: usize, end: usize) -> Self {
        Self {
            label: label.into(),
            start: start.min(end),
            end: start.max(end),
        }
    }

    /// How many bytes it covers.
    pub fn len(&self) -> usize {
        self.end - self.start
    }

    /// Whether it is empty.
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }
}

/// What a selection can grow over.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Capability {
    /// No language service: selection is structural over the text only.
    TextOnly,
    /// A language service that knows some spans; selection grows to the smallest enclosing one.
    Syntax(Vec<Span>),
}

/// What rung of the ladder a selection is on.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Kind {
    /// A run of word characters.
    Word,
    /// A whole line, without its ending.
    Line,
    /// A block of non-blank lines.
    Paragraph,
    /// The whole document.
    Document,
    /// A labelled span from a syntax capability.
    Syntax(String),
}

impl Kind {
    /// The kind's name.
    pub fn name(&self) -> String {
        match self {
            Self::Word => "word".to_owned(),
            Self::Line => "line".to_owned(),
            Self::Paragraph => "paragraph".to_owned(),
            Self::Document => "document".to_owned(),
            Self::Syntax(label) => format!("syntax:{label}"),
        }
    }
}

/// The outcome of growing a selection.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Expanded {
    /// The selection grown to.
    pub selection: Selection,
    /// The rung it is on.
    pub kind: Kind,
    /// Whether a syntax capability was asked for and not available.
    pub fell_back: bool,
}

impl Expanded {
    /// The outcome in one line.
    pub fn render(&self) -> String {
        let fallback = if self.fell_back {
            " (text selection: no syntax capability)"
        } else {
            ""
        };
        format!("{} {}{fallback}", self.kind.name(), self.selection.render())
    }
}

/// Grows a selection one rung: to a syntax span when one is available, otherwise over the text.
pub fn expand(text: &str, selection: Selection, capability: &Capability) -> Expanded {
    if let Capability::Syntax(spans) = capability {
        // The smallest span that encloses the selection and is strictly larger than it.
        let enclosing = spans
            .iter()
            .filter(|span| span.start <= selection.start() && selection.end() <= span.end)
            .filter(|span| {
                selection.is_caret() || span.start < selection.start() || span.end > selection.end()
            })
            .min_by_key(|span| span.len());
        if let Some(span) = enclosing {
            let start = boundary(text, span.start);
            let end = boundary(text, span.end);
            return Expanded {
                selection: Selection::range(start, end),
                kind: Kind::Syntax(span.label.clone()),
                fell_back: false,
            };
        }
    }
    let (selection, kind) = text_step(text, selection);
    Expanded {
        selection,
        kind,
        fell_back: matches!(capability, Capability::Syntax(_)),
    }
}

/// The next text structure above a selection: the first span that strictly contains it.
fn text_step(text: &str, selection: Selection) -> (Selection, Kind) {
    let candidates = [
        (Kind::Word, word_span(text, selection)),
        (Kind::Line, line_span(text, selection)),
        (Kind::Paragraph, paragraph_span(text, selection)),
        (Kind::Document, Selection::range(0, text.len())),
    ];
    for (kind, span) in candidates {
        let contains = span.start() <= selection.start() && selection.end() <= span.end();
        // Strictly larger: a rung the selection is already on is skipped, so the ladder always grows.
        if contains && span.len() > selection.len() {
            return (span, kind);
        }
    }
    (Selection::range(0, text.len()), Kind::Document)
}

/// The run of word characters around a selection.
fn word_span(text: &str, selection: Selection) -> Selection {
    let at = selection.cursor().min(text.len());
    let mut start = at;
    while let Some(previous) = previous_boundary(text, start) {
        if is_word_char(text[previous..start].chars().next().expect("one character")) {
            start = previous;
        } else {
            break;
        }
    }
    let mut end = at;
    while let Some(next) = next_boundary(text, end) {
        if is_word_char(text[end..next].chars().next().expect("one character")) {
            end = next;
        } else {
            break;
        }
    }
    Selection::range(start, end)
}

/// The whole line the selection is on, without its ending.
fn line_span(text: &str, selection: Selection) -> Selection {
    Selection::range(
        line_start_of(text, selection.start()),
        line_end_of(text, selection.end()),
    )
}

/// The block of non-blank lines around the selection.
fn paragraph_span(text: &str, selection: Selection) -> Selection {
    let mut start = line_start_of(text, selection.start());
    loop {
        if start == 0 {
            break;
        }
        let previous_end = start - 1;
        let previous_start = line_start_of(text, previous_end);
        if text[previous_start..previous_end].trim().is_empty() {
            break;
        }
        start = previous_start;
    }
    let mut end = line_end_of(text, selection.end());
    loop {
        if end >= text.len() {
            break;
        }
        let next_start = end + 1;
        let next_end = line_end_of(text, next_start);
        if text[next_start..next_end].trim().is_empty() {
            break;
        }
        end = next_end;
    }
    Selection::range(start, end)
}

/// A bookmark: a name and the position it marks, an edit location that moves with the edits.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Bookmark {
    /// The name.
    pub name: String,
    /// The position, as a byte offset.
    pub offset: usize,
}

/// Named positions that move with the edits.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Bookmarks {
    marks: Vec<Bookmark>,
}

impl Bookmarks {
    /// No bookmarks.
    pub fn new() -> Self {
        Self { marks: Vec::new() }
    }

    /// Adds a bookmark, replacing a mark of the same name.
    pub fn add(&mut self, name: impl Into<String>, offset: usize) {
        let name = name.into();
        self.marks.retain(|mark| mark.name != name);
        self.marks.push(Bookmark { name, offset });
        self.marks
            .sort_by(|a, b| (a.offset, &a.name).cmp(&(b.offset, &b.name)));
    }

    /// Removes a bookmark, saying whether there was one.
    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.marks.len();
        self.marks.retain(|mark| mark.name != name);
        self.marks.len() != before
    }

    /// A bookmark's position, when it exists.
    pub fn at(&self, name: &str) -> Option<usize> {
        self.marks
            .iter()
            .find(|mark| mark.name == name)
            .map(|mark| mark.offset)
    }

    /// Every bookmark, ordered by position.
    pub fn list(&self) -> &[Bookmark] {
        &self.marks
    }

    /// Moves every bookmark across an edit that replaces `remove` bytes at `at` with `insert` bytes.
    ///
    /// A mark strictly before the edit stays; a mark at or after it moves by the edit's length change
    /// (an insertion at the mark's own offset moves the mark with the text it typed); a mark inside
    /// the replaced range lands at the edit's start, since its text is gone.
    pub fn translate(&mut self, at: usize, remove: usize, insert: usize) {
        for mark in &mut self.marks {
            if mark.offset < at {
                continue;
            }
            if mark.offset >= at + remove {
                mark.offset = mark.offset - remove + insert;
            } else {
                mark.offset = at + insert;
            }
        }
        self.marks
            .sort_by(|a, b| (a.offset, &a.name).cmp(&(b.offset, &b.name)));
    }

    /// The bookmarks in one report.
    pub fn render(&self) -> Vec<String> {
        if self.marks.is_empty() {
            return vec!["bookmarks: none".to_owned()];
        }
        let mut lines = vec![format!("bookmarks: {}", self.marks.len())];
        for mark in &self.marks {
            lines.push(format!("  {} at {}", mark.name, mark.offset));
        }
        lines
    }
}

/// A display element that can be turned on or off.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Toggle {
    /// Whitespace markers.
    Whitespace,
    /// The line-number gutter.
    Gutters,
    /// Fold regions.
    Folding,
    /// The minimap.
    Minimap,
    /// An inline diff against a baseline revision.
    InlineDiff,
}

impl Toggle {
    /// Every toggle, in a stable order.
    pub const ALL: [Self; 5] = [
        Self::Whitespace,
        Self::Gutters,
        Self::Folding,
        Self::Minimap,
        Self::InlineDiff,
    ];

    /// The toggle's name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Whitespace => "whitespace",
            Self::Gutters => "gutters",
            Self::Folding => "folding",
            Self::Minimap => "minimap",
            Self::InlineDiff => "inline-diff",
        }
    }

    /// The toggle a name picks.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|toggle| toggle.name() == name)
    }
}

/// Which display elements are on, and the baseline an inline diff needs.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Toggles {
    whitespace: bool,
    gutters: bool,
    folding: bool,
    minimap: bool,
    inline_diff: bool,
    baseline: Option<Revision>,
}

impl Toggles {
    /// The editor's usual set: whitespace, gutters, folding and the minimap on; inline diff off.
    pub fn new() -> Self {
        Self {
            whitespace: true,
            gutters: true,
            folding: true,
            minimap: true,
            inline_diff: false,
            baseline: None,
        }
    }

    /// Sets the baseline an inline diff compares against (a read-only revision from T08.07).
    pub fn set_baseline(&mut self, revision: Revision) {
        self.baseline = Some(revision);
    }

    /// Turns a toggle on or off, refusing to turn the inline diff on with no baseline to compare.
    pub fn set(&mut self, toggle: Toggle, on: bool) -> Result<(), String> {
        if toggle == Toggle::InlineDiff && on && self.baseline.is_none() {
            return Err("an inline diff needs a baseline revision".to_owned());
        }
        match toggle {
            Toggle::Whitespace => self.whitespace = on,
            Toggle::Gutters => self.gutters = on,
            Toggle::Folding => self.folding = on,
            Toggle::Minimap => self.minimap = on,
            Toggle::InlineDiff => self.inline_diff = on,
        }
        Ok(())
    }

    /// Whether a toggle is on.
    pub fn is_on(&self, toggle: Toggle) -> bool {
        match toggle {
            Toggle::Whitespace => self.whitespace,
            Toggle::Gutters => self.gutters,
            Toggle::Folding => self.folding,
            Toggle::Minimap => self.minimap,
            Toggle::InlineDiff => self.inline_diff,
        }
    }

    /// Why a toggle is off, when there is a reason beyond the person turning it off.
    pub fn reason(&self, toggle: Toggle) -> Option<String> {
        if toggle == Toggle::InlineDiff && self.baseline.is_none() {
            return Some("no baseline revision to diff against".to_owned());
        }
        None
    }

    /// The inline diff of the current text against the baseline, when there is one.
    pub fn inline_rows(&self, text: &str) -> Vec<Row> {
        match &self.baseline {
            Some(revision) => diff(revision.text(), text, crate::app::diff::Options::strict()),
            None => Vec::new(),
        }
    }

    /// The toggles in one report.
    pub fn render(&self) -> Vec<String> {
        let baseline = match &self.baseline {
            Some(revision) => revision.name().to_owned(),
            None => "none".to_owned(),
        };
        let mut lines = vec![format!("toggles (baseline: {baseline}):")];
        for toggle in Toggle::ALL {
            let state = if self.is_on(toggle) { "on" } else { "off" };
            match self.reason(toggle) {
                Some(reason) => lines.push(format!("  {}: {state} — {reason}", toggle.name())),
                None => lines.push(format!("  {}: {state}", toggle.name())),
            }
        }
        lines
    }
}

impl Default for Toggles {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether a character is part of a word for selection.
fn is_word_char(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

/// The byte offset a line starts at.
fn line_start_of(text: &str, offset: usize) -> usize {
    let offset = boundary(text, offset);
    text[..offset].rfind('\n').map_or(0, |index| index + 1)
}

/// The byte offset a line ends at (its newline, or the end of the text).
fn line_end_of(text: &str, offset: usize) -> usize {
    let offset = boundary(text, offset.min(text.len()));
    text[offset..]
        .find('\n')
        .map_or(text.len(), |index| offset + index)
}

/// The character boundary at or before an offset.
fn boundary(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

/// The boundary before an offset.
fn previous_boundary(text: &str, offset: usize) -> Option<usize> {
    if offset == 0 {
        return None;
    }
    Some(boundary(text, offset - 1))
}

/// The boundary after an offset.
fn next_boundary(text: &str, offset: usize) -> Option<usize> {
    if offset >= text.len() {
        return None;
    }
    text[offset..]
        .chars()
        .next()
        .map(|character| offset + character.len_utf8())
}

#[cfg(test)]
mod tests {
    use super::{Bookmarks, Capability, Kind, Span, Toggle, Toggles, expand};
    use crate::app::diff::Revision;
    use crate::app::editing::Selection;

    #[test]
    fn a_selection_grows_word_line_paragraph_document() {
        // Each line is a word plus another word, and the third line is blank, so the four rungs are
        // all distinct: word 3..5, line 0..5, paragraph 0..11, document 0..16.
        let text = "aa bb\ncc dd\n\nee\n";
        let word = expand(text, Selection::caret(4), &Capability::TextOnly);
        assert_eq!(word.kind, Kind::Word);
        assert_eq!(word.selection, Selection::range(3, 5));
        assert!(!word.fell_back);

        let line = expand(text, word.selection, &Capability::TextOnly);
        assert_eq!(line.kind, Kind::Line);
        assert_eq!(line.selection, Selection::range(0, 5));

        let paragraph = expand(text, line.selection, &Capability::TextOnly);
        assert_eq!(paragraph.kind, Kind::Paragraph);
        assert_eq!(paragraph.selection, Selection::range(0, 11));

        let document = expand(text, paragraph.selection, &Capability::TextOnly);
        assert_eq!(document.kind, Kind::Document);
        assert_eq!(document.selection, Selection::range(0, text.len()));
    }

    #[test]
    fn a_word_that_fills_its_line_grows_straight_to_the_paragraph() {
        // "one" exactly fills its line, so the line rung would be a no-op and is skipped.
        let text = "a\none\ntwo\nb\n";
        let word = expand(text, Selection::caret(3), &Capability::TextOnly);
        assert_eq!(word.kind, Kind::Word);
        assert_eq!(word.selection, Selection::range(2, 5));
        let paragraph = expand(text, word.selection, &Capability::TextOnly);
        assert_eq!(paragraph.kind, Kind::Paragraph);
        assert_eq!(paragraph.selection, Selection::range(0, 11));
    }

    #[test]
    fn a_syntax_span_is_used_when_one_is_available() {
        let text = "fn f() { g(); }\n";
        let spans = vec![Span::new("function", 0, 16), Span::new("body", 8, 14)];
        let capability = Capability::Syntax(spans);
        // A caret inside the body grows to the smallest enclosing span (the body), not the function.
        let grown = expand(text, Selection::caret(9), &capability);
        assert_eq!(grown.kind, Kind::Syntax("body".to_owned()));
        assert_eq!(grown.selection, Selection::range(8, 14));
        assert!(!grown.fell_back);
        // Growing again reaches the enclosing function.
        let grown = expand(text, grown.selection, &capability);
        assert_eq!(grown.kind, Kind::Syntax("function".to_owned()));
        assert_eq!(grown.selection, Selection::range(0, 16));
    }

    #[test]
    fn without_a_syntax_span_the_growth_falls_back_to_text() {
        let text = "fn f() { g(); }\n";
        // A syntax capability that knows nothing about this place falls back to text, and says so.
        let capability = Capability::Syntax(vec![Span::new("elsewhere", 100, 200)]);
        let grown = expand(text, Selection::caret(9), &capability);
        assert!(grown.fell_back);
        assert_eq!(grown.kind, Kind::Word);
    }

    #[test]
    fn growing_a_selection_never_moves_off_the_text() {
        let text = "héllo wörld\n";
        let mut selection = Selection::caret(1);
        for _ in 0..6 {
            let grown = expand(text, selection, &Capability::TextOnly);
            assert!(grown.selection.start() <= text.len());
            assert!(grown.selection.end() <= text.len());
            assert!(text.is_char_boundary(grown.selection.start()));
            assert!(text.is_char_boundary(grown.selection.end()));
            selection = grown.selection;
        }
    }

    #[test]
    fn a_bookmark_moves_with_the_edits() {
        let mut bookmarks = Bookmarks::new();
        bookmarks.add("start", 0);
        bookmarks.add("middle", 5);
        bookmarks.add("end", 10);
        // Delete three bytes at 4: the middle mark is inside and lands at 4; the end mark shifts by -3.
        bookmarks.translate(4, 3, 0);
        assert_eq!(bookmarks.at("start"), Some(0));
        assert_eq!(bookmarks.at("middle"), Some(4));
        assert_eq!(bookmarks.at("end"), Some(7));
        // Insert two bytes at 0: every mark moves right by two.
        bookmarks.translate(0, 0, 2);
        assert_eq!(bookmarks.at("start"), Some(2));
        assert_eq!(bookmarks.at("middle"), Some(6));
        assert_eq!(bookmarks.at("end"), Some(9));
        assert!(bookmarks.remove("middle"));
        assert!(!bookmarks.remove("middle"));
        assert_eq!(bookmarks.list().len(), 2);
    }

    #[test]
    fn an_inline_diff_needs_a_baseline_and_is_then_the_one_from_t08_07() {
        let mut toggles = Toggles::new();
        // Without a baseline the toggle cannot be turned on, and says why.
        assert!(toggles.set(Toggle::InlineDiff, true).is_err());
        assert!(!toggles.is_on(Toggle::InlineDiff));
        assert_eq!(
            toggles.reason(Toggle::InlineDiff).as_deref(),
            Some("no baseline revision to diff against")
        );
        // With a baseline it turns on and diffs against it.
        toggles.set_baseline(Revision::new("HEAD", "a\nb\n"));
        assert!(toggles.set(Toggle::InlineDiff, true).is_ok());
        assert!(toggles.is_on(Toggle::InlineDiff));
        assert_eq!(toggles.reason(Toggle::InlineDiff), None);
        let rows = toggles.inline_rows("a\nB\n");
        assert_eq!(rows.len(), 2);
        assert!(rows[1].is_change());
    }

    #[test]
    fn the_other_toggles_turn_on_and_off_freely() {
        let mut toggles = Toggles::new();
        for toggle in [
            Toggle::Whitespace,
            Toggle::Gutters,
            Toggle::Folding,
            Toggle::Minimap,
        ] {
            assert!(toggles.is_on(toggle));
            toggles.set(toggle, false).expect("turning off");
            assert!(!toggles.is_on(toggle));
            assert_eq!(toggles.reason(toggle), None);
        }
    }

    #[test]
    fn a_toggle_name_round_trips() {
        for toggle in Toggle::ALL {
            assert_eq!(Toggle::from_name(toggle.name()), Some(toggle));
        }
        assert_eq!(Toggle::from_name("nowhere"), None);
    }
}
