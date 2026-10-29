// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The host adapter over the library's `CodeEditor` (tasks T06.01–T06.05).
//!
//! The widget owns the text, the carets, the undo history, folding and highlighting. This layer owns
//! only what a widget cannot: constructing the editor with a real geometry, handing it a document a
//! host read from disk, and translating the command line's byte offsets into the widget's
//! line/column positions. **No editing primitive is reimplemented here** — a multi-caret insert is
//! one `CodeEditor::insert` call and therefore one transaction and one undo step, which is the
//! acceptance line of T06.04.
//!
//! The widget is thread-affine (`Rc`/`RefCell` inside), so an adapter is created and used on the
//! thread that owns the view. `text()` clones the document and is for saving and reports, not for a
//! per-frame path: painting reads the widget's viewport, not a host copy.

#![allow(dead_code)]

use rust_widgets::core::Rect;
use rust_widgets::widget::special_widgets::code_editor::{
    CodeEditor, CodeEditorConfig, FoldRegion, LanguageId, ReadOnlySpan, TextPosition, VisualLine,
};

/// A geometry wide enough that the widget's viewport arithmetic is real in a command-line run.
fn geometry() -> Rect {
    Rect::new(0, 0, 400, 300)
}

/// The host's adapter over a live [`CodeEditor`].
pub struct Adapter {
    editor: CodeEditor,
}

impl Adapter {
    /// A new, empty adapter.
    pub fn new() -> Self {
        Self {
            editor: CodeEditor::new(geometry()),
        }
    }

    /// An adapter created with the widget's own wrap and tab settings.
    ///
    /// Soft wrap and the tab stop are read when the widget is built; the width wrapping uses is the
    /// widget's geometry, not a column number a caller can set independently.
    pub fn with_options(text: &str, word_wrap: bool, tab_width: usize) -> Result<Self, String> {
        let config = CodeEditorConfig::default()
            .word_wrap(word_wrap)
            .tab_width(tab_width.max(1));
        let mut editor =
            CodeEditor::with_config(geometry(), config).map_err(|reason| reason.to_owned())?;
        editor.set_text(text);
        Ok(Self { editor })
    }

    /// The widget itself, for callers that need an API this adapter does not wrap.
    pub fn editor(&self) -> &CodeEditor {
        &self.editor
    }

    /// The widget itself, mutably.
    pub fn editor_mut(&mut self) -> &mut CodeEditor {
        &mut self.editor
    }

    /// Replaces the document. The widget owns the resulting text; the caller keeps no copy.
    pub fn set_text(&mut self, text: &str) {
        self.editor.set_text(text);
    }

    /// Sets the language whose grammar and comment prefix the widget uses.
    pub fn set_language(&mut self, language: LanguageId) {
        self.editor.set_language(language);
    }

    /// The document text, for saving or a report. A per-frame path must not call this.
    pub fn text(&self) -> String {
        self.editor.text()
    }

    /// One line's text, without cloning the document.
    pub fn line_text(&self, line: usize) -> Option<String> {
        self.editor.line_text(line)
    }

    /// Folds every block the widget detects, answering how many it folded.
    pub fn fold_all(&mut self) -> usize {
        self.editor.fold_all()
    }

    /// The widget's visual rows: how each document line wraps and which lines folds hide.
    pub fn visual_lines(&self) -> Vec<VisualLine> {
        self.editor.visual_lines()
    }

    /// How many visual rows the document has.
    pub fn visual_row_count(&self) -> usize {
        self.editor.visual_row_count()
    }

    /// The first and last visible visual row.
    pub fn visible_rows(&self) -> (usize, usize) {
        self.editor.visible_row_range()
    }

    /// Folds `start_line..=end_line`.
    pub fn fold(&mut self, start_line: usize, end_line: usize) {
        self.editor.fold(start_line, end_line);
    }

    /// Unfolds the region that starts at `start_line`.
    pub fn unfold(&mut self, start_line: usize) -> bool {
        self.editor.unfold(start_line)
    }

    /// Every fold region the widget currently knows.
    pub fn fold_regions(&self) -> Vec<FoldRegion> {
        self.editor.fold_regions()
    }

    /// Sets the tab stop the widget expands a tab to.
    pub fn set_tab_width(&mut self, width: usize) {
        self.editor.set_tab_width(width.max(1));
    }

    /// The revision the widget bumps on every committed change.
    pub fn revision(&self) -> u64 {
        self.editor.revision()
    }

    /// How many lines the document has.
    pub fn line_count(&self) -> usize {
        self.editor.line_count()
    }

    /// The primary caret.
    pub fn caret(&self) -> TextPosition {
        self.editor.caret()
    }

    /// Every caret, primary included, in document order.
    pub fn carets(&self) -> Vec<TextPosition> {
        self.editor
            .cursors()
            .carets()
            .iter()
            .map(|caret| caret.head)
            .collect()
    }

    /// How many carets are active.
    pub fn caret_count(&self) -> usize {
        self.editor.cursors().len()
    }

    /// The primary selection's normalized bounds, when there is one.
    pub fn selection(&self) -> Option<(TextPosition, TextPosition)> {
        self.editor
            .has_selection()
            .then(|| self.editor.selection_bounds())
    }

    /// Places the primary caret and collapses every secondary caret.
    pub fn set_caret(&mut self, position: TextPosition) {
        self.editor.collapse_cursors();
        self.editor.set_caret(position, false);
    }

    /// Adds a caret, answering whether it was new.
    pub fn add_caret(&mut self, position: TextPosition) -> bool {
        self.editor.add_cursor_at(position)
    }

    /// Selects `anchor..cursor` with one caret.
    pub fn select(&mut self, anchor: TextPosition, cursor: TextPosition) {
        self.set_caret(anchor);
        self.editor.set_caret(cursor, true);
    }

    /// Inserts text at every caret as one widget transaction.
    pub fn insert(&mut self, text: &str) {
        self.editor.insert(text);
    }

    /// Types text through the widget's input layer, which applies auto-pairing and indentation.
    pub fn type_text(&mut self, text: &str) {
        self.editor.input_text(text);
    }

    /// Backspaces at every caret.
    pub fn backspace(&mut self) {
        self.editor.backspace();
    }

    /// Deletes forward at every caret.
    pub fn delete_forward(&mut self) {
        self.editor.delete_forward();
    }

    /// Moves the selected lines, or the caret's line, up (`-1`) or down (`1`).
    pub fn move_line(&mut self, direction: isize) {
        self.editor.move_line(direction);
    }

    /// Duplicates the selected lines, or the caret's line.
    pub fn duplicate_line(&mut self) {
        self.editor.duplicate_line();
    }

    /// Deletes the selected lines, or the caret's line.
    pub fn delete_line(&mut self) {
        self.editor.delete_line();
    }

    /// Toggles the line comment of the selected lines with the language's own prefix.
    pub fn toggle_comment(&mut self) {
        self.editor.toggle_line_comment();
    }

    /// Takes the last transaction back, answering whether there was one.
    pub fn undo(&mut self) -> bool {
        self.editor.undo()
    }

    /// Puts the last undone transaction back, answering whether there was one.
    pub fn redo(&mut self) -> bool {
        self.editor.redo()
    }

    /// The bracket pair the caret is next to.
    pub fn matching_brackets(&self) -> Option<(TextPosition, TextPosition)> {
        self.editor.matching_brackets()
    }

    /// Hands the widget the spans an edit must not cross.
    pub fn protect(&mut self, spans: Vec<ReadOnlySpan>) {
        self.editor.set_read_only_spans(spans);
    }

    /// Removes every protected span.
    pub fn clear_protection(&mut self) {
        self.editor.clear_read_only_spans();
    }

    /// Whether an edit at `position` would land outside every protected span.
    pub fn is_editable_at(&self, position: TextPosition) -> bool {
        self.editor.is_position_editable(position)
    }
}

impl Default for Adapter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adapter(text: &str) -> Adapter {
        let mut adapter = Adapter::new();
        adapter.set_text(text);
        adapter
    }

    #[test]
    #[ignore = "million-line benchmark; run explicitly and record the numbers"]
    fn million_line_benchmark() {
        use crate::app::editor::highlight::{Highlights, Language as HighlightLanguage};
        use crate::project::text::piece_table::TextStore;
        use std::time::Instant;

        let mut text = String::with_capacity(20 * 1024 * 1024);
        for index in 0..1_000_000usize {
            use std::fmt::Write as _;
            let _ = writeln!(text, "let v{index} = {index};");
        }
        println!("document: {} bytes, 1000000 lines", text.len());

        let started = Instant::now();
        let mut adapter = Adapter::with_options(&text, false, 4).expect("a widget");
        println!("widget open (set_text): {:?}", started.elapsed());

        adapter.set_caret(TextPosition::new(0, 0));
        let started = Instant::now();
        adapter.insert("x");
        println!("widget insert at start: {:?}", started.elapsed());
        let started = Instant::now();
        assert!(adapter.undo());
        println!("widget undo: {:?}", started.elapsed());

        let started = Instant::now();
        let line = adapter.line_text(500_000).expect("a line");
        println!(
            "widget line_text(500000): {:?} ({line:?})",
            started.elapsed()
        );

        let started = Instant::now();
        let rows = adapter.visual_lines().len();
        println!(
            "widget visual_lines(): {:?} ({rows} rows)",
            started.elapsed()
        );

        let mut highlights = Highlights::new(HighlightLanguage::Rust);
        let started = Instant::now();
        highlights.set_text(&text);
        let built = started.elapsed();
        let started = Instant::now();
        let stepped = highlights.step(1000);
        println!(
            "host highlight cache: set_text {built:?}, step(1000) {:?} ({stepped} lines)",
            started.elapsed()
        );

        let started = Instant::now();
        let mut store = TextStore::new(text.clone());
        println!("piece table open: {:?}", started.elapsed());
        let started = Instant::now();
        store.splice(0, 0, "x").expect("a splice");
        println!("piece table splice at start: {:?}", started.elapsed());
        let started = Instant::now();
        store.undo().expect("an undo");
        println!("piece table undo: {:?}", started.elapsed());
    }

    #[test]
    fn a_multi_caret_insert_is_one_undo_step() {
        let mut adapter = adapter("a\nb\nc");
        adapter.set_caret(TextPosition::new(0, 0));
        assert!(adapter.add_caret(TextPosition::new(2, 0)));
        assert_eq!(adapter.caret_count(), 2);

        adapter.insert("x");
        assert_eq!(adapter.text(), "xa\nb\nxc");
        assert!(adapter.undo(), "the whole keystroke is one transaction");
        assert_eq!(adapter.text(), "a\nb\nc");
    }

    #[test]
    fn a_protected_span_refuses_the_edit_and_leaves_the_text_alone() {
        let mut adapter = adapter("generated\nhandwritten");
        adapter.protect(vec![ReadOnlySpan::lines(0, 0)]);
        assert!(!adapter.is_editable_at(TextPosition::new(0, 0)));
        assert!(adapter.is_editable_at(TextPosition::new(1, 0)));

        adapter.set_caret(TextPosition::new(0, 0));
        adapter.insert("x");
        assert_eq!(adapter.text(), "generated\nhandwritten");

        adapter.clear_protection();
        adapter.set_caret(TextPosition::new(0, 0));
        adapter.insert("x");
        assert_eq!(adapter.text(), "xgenerated\nhandwritten");
    }

    #[test]
    fn a_comment_toggle_uses_the_languages_own_prefix() {
        let mut rust = adapter("let x = 1;");
        rust.set_language(LanguageId::Rust);
        rust.set_caret(TextPosition::new(0, 0));
        rust.toggle_comment();
        assert_eq!(rust.text(), "// let x = 1;");
        rust.toggle_comment();
        assert_eq!(rust.text(), "let x = 1;");

        let mut python = adapter("x = 1");
        python.set_language(LanguageId::Python);
        python.set_caret(TextPosition::new(0, 0));
        python.toggle_comment();
        assert_eq!(python.text(), "# x = 1");
    }

    #[test]
    fn a_selection_is_placed_and_reported_with_one_conversion() {
        let mut adapter = adapter("hello");
        adapter.select(TextPosition::new(0, 1), TextPosition::new(0, 4));
        assert_eq!(
            adapter.selection(),
            Some((TextPosition::new(0, 1), TextPosition::new(0, 4)))
        );
        adapter.insert("i");
        assert_eq!(adapter.text(), "hio");
    }
}
