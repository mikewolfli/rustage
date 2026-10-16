// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Multi-cursor editing: selections, line operations, comments and brackets (task T06.04).
//!
//! The acceptance line is *"a multi-cursor edit is one transaction, and it does not split a
//! character"*. Both halves are properties of how an edit is delivered, so the module delivers
//! exactly that:
//!
//! * **One transaction.** Every command here builds a list of `(offset, remove, insert)` edits and
//!   hands them to [`TextStore::edit_many`] in one call, so several carets typing at once are one
//!   atomic change and one undo step. A batch that would split a character, or whose ranges overlap,
//!   is refused *as a whole*, leaving the text untouched. Nothing here calls [`TextStore::splice`] in
//!   a loop, which is what would turn one keystroke into several undo steps.
//! * **No character is truncated.** Every offset an edit uses comes from a [`Selection`], and a
//!   selection is snapped to character boundaries before it is used ([`Editor::set_carets`]) and the
//!   store refuses anything else. A selection that covers a wide character deletes it whole.
//!
//! The line operations — [`Editor::move_lines`], [`Editor::duplicate_lines`],
//! [`Editor::delete_lines`] and [`Editor::toggle_comment`] — are the same shape: the block of lines
//! the selections touch is rebuilt and spliced as **one** edit, because a person takes each back with
//! one undo. [`matching_bracket`] answers the other bracket; [`Editor::type_char`] is auto-pairing.
//!
//! **Known limit.** [`matching_bracket`] scans the text, so it does not yet skip a bracket inside a
//! string or a comment; that needs the per-line states of [`crate::app::highlight`] woven through the
//! scan, and is recorded as a boundary rather than claimed.

#![allow(dead_code)]

use crate::project::text::{TextError, TextStore};

/// One caret, or one selection: where it was anchored and where it is now, as byte offsets.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Selection {
    anchor: usize,
    cursor: usize,
}

impl Selection {
    /// A caret at an offset.
    pub const fn caret(offset: usize) -> Self {
        Self {
            anchor: offset,
            cursor: offset,
        }
    }

    /// A selection from one offset to another.
    pub const fn range(anchor: usize, cursor: usize) -> Self {
        Self { anchor, cursor }
    }

    /// Where it was anchored.
    pub const fn anchor(&self) -> usize {
        self.anchor
    }

    /// Where it is now.
    pub const fn cursor(&self) -> usize {
        self.cursor
    }

    /// The lower end of the range.
    pub fn start(&self) -> usize {
        self.anchor.min(self.cursor)
    }

    /// The higher end of the range.
    pub fn end(&self) -> usize {
        self.anchor.max(self.cursor)
    }

    /// Whether it covers nothing.
    pub fn is_caret(&self) -> bool {
        self.anchor == self.cursor
    }

    /// How many bytes it covers.
    pub fn len(&self) -> usize {
        self.end() - self.start()
    }

    /// The selection as one line.
    pub fn render(&self) -> String {
        if self.is_caret() {
            format!("caret at {}", self.cursor)
        } else {
            format!("selection {}..{}", self.start(), self.end())
        }
    }
}

/// The carets, ordered and disjoint.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Carets {
    selections: Vec<Selection>,
}

impl Carets {
    /// One caret.
    pub fn caret(offset: usize) -> Self {
        Self {
            selections: vec![Selection::caret(offset)],
        }
    }

    /// A set of selections, checked.
    pub fn of(selections: Vec<Selection>) -> Result<Self, String> {
        let mut carets = Self { selections };
        carets.normalise()?;
        Ok(carets)
    }

    /// Every selection, ordered by where it starts.
    pub fn selections(&self) -> &[Selection] {
        &self.selections
    }

    /// The primary selection: the last one, which a single-selection operation acts on.
    pub fn primary(&self) -> Selection {
        *self.selections.last().expect("there is always a caret")
    }

    /// How many carets there are.
    pub fn len(&self) -> usize {
        self.selections.len()
    }

    /// Adds a selection, refusing one that overlaps another.
    ///
    /// A refused add leaves the carets exactly as they were; it does not half-apply.
    pub fn add(&mut self, selection: Selection) -> Result<(), String> {
        let mut candidate = self.clone();
        candidate.selections.push(selection);
        candidate.normalise()?;
        *self = candidate;
        Ok(())
    }

    /// Every caret collapsed to a point at its cursor.
    pub fn collapsed(&self) -> Self {
        Self {
            selections: self
                .selections
                .iter()
                .map(|selection| Selection::caret(selection.cursor()))
                .collect(),
        }
    }

    /// Orders the selections, folds duplicates together, and refuses an overlap.
    fn normalise(&mut self) -> Result<(), String> {
        self.selections
            .sort_by_key(|selection| (selection.start(), selection.end()));
        // Two carets at the same place are one caret: folding them stops a keystroke from becoming
        // two overlapping edits that the store would refuse.
        self.selections.dedup();
        for window in self.selections.windows(2) {
            let (left, right) = (window[0], window[1]);
            if right.start() < left.end() {
                return Err(format!(
                    "the selections {}..{} and {}..{} overlap",
                    left.start(),
                    left.end(),
                    right.start(),
                    right.end()
                ));
            }
        }
        Ok(())
    }
}

/// The editor's multi-caret state over a [`TextStore`].
#[derive(Clone, Debug)]
pub struct Editor {
    carets: Carets,
}

impl Editor {
    /// An editor with one caret at the start.
    pub fn new() -> Self {
        Self {
            carets: Carets::caret(0),
        }
    }

    /// An editor with one caret at an offset.
    pub fn at(offset: usize) -> Self {
        Self {
            carets: Carets::caret(offset),
        }
    }

    /// The carets.
    pub const fn carets(&self) -> &Carets {
        &self.carets
    }

    /// Replaces the carets, snapping each end to a character boundary of the text.
    pub fn set_carets(&mut self, text: &str, carets: Carets) -> Result<(), String> {
        let snapped = carets
            .selections()
            .iter()
            .map(|selection| {
                Selection::range(
                    boundary(text, selection.anchor()),
                    boundary(text, selection.cursor()),
                )
            })
            .collect();
        self.carets = Carets::of(snapped)?;
        Ok(())
    }

    /// The editor in one report: how many carets there are and where each one is.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!("carets: {}", self.carets.len())];
        for (index, selection) in self.carets.selections().iter().enumerate() {
            lines.push(format!("  {index}: {}", selection.render()));
        }
        lines
    }

    /// Inserts text at every selection, replacing it, as one transaction.
    ///
    /// A caret at a higher offset is placed after the bytes the lower edits added or removed, so
    /// every caret lands where the person sees it move, not at its old offset.
    pub fn insert(&mut self, store: &mut TextStore, text: &str) -> Result<(), TextError> {
        let mut edits = Vec::new();
        let mut carets = Vec::new();
        let mut delta: isize = 0;
        for selection in self.carets.selections() {
            let start = selection.start();
            let caret = (start as isize + delta) as usize + text.len();
            edits.push((start, selection.len(), text.to_owned()));
            carets.push(Selection::caret(caret));
            delta += text.len() as isize - selection.len() as isize;
        }
        store.edit_many(edits)?;
        self.carets = Carets { selections: carets };
        Ok(())
    }

    /// Deletes one character before every caret (or the selection), as one transaction.
    pub fn backspace(&mut self, store: &mut TextStore) -> Result<(), TextError> {
        let text = store.text();
        let mut edits = Vec::new();
        let mut carets = Vec::new();
        let mut delta: isize = 0;
        for selection in self.carets.selections() {
            let start = selection.start();
            let base = (start as isize + delta) as usize;
            if !selection.is_caret() {
                edits.push((start, selection.len(), String::new()));
                carets.push(Selection::caret(base));
                delta -= selection.len() as isize;
            } else if let Some(previous) = previous_boundary(&text, selection.cursor()) {
                let removed = selection.cursor() - previous;
                edits.push((previous, removed, String::new()));
                carets.push(Selection::caret(base - removed));
                delta -= removed as isize;
            } else {
                carets.push(Selection::caret(base));
            }
        }
        store.edit_many(edits)?;
        self.carets = Carets { selections: carets };
        Ok(())
    }

    /// Deletes one character after every caret (or the selection), as one transaction.
    pub fn delete_forward(&mut self, store: &mut TextStore) -> Result<(), TextError> {
        let text = store.text();
        let mut edits = Vec::new();
        let mut carets = Vec::new();
        let mut delta: isize = 0;
        for selection in self.carets.selections() {
            let start = selection.start();
            let base = (start as isize + delta) as usize;
            if !selection.is_caret() {
                edits.push((start, selection.len(), String::new()));
                carets.push(Selection::caret(base));
                delta -= selection.len() as isize;
            } else if let Some(next) = next_boundary(&text, selection.cursor()) {
                let removed = next - selection.cursor();
                edits.push((selection.cursor(), removed, String::new()));
                carets.push(Selection::caret(base));
                delta -= removed as isize;
            } else {
                carets.push(Selection::caret(base));
            }
        }
        store.edit_many(edits)?;
        self.carets = Carets { selections: carets };
        Ok(())
    }

    /// Moves the block of lines the selections touch up or down by one, as one transaction.
    pub fn move_lines(&mut self, store: &mut TextStore, delta: i32) -> Result<(), TextError> {
        let text = store.text();
        let Some((first, last)) = self.line_span(&text) else {
            return Ok(());
        };
        let lines: Vec<&str> = text.split('\n').collect();
        if delta < 0 {
            if first == 0 {
                return Ok(());
            }
            let region_start = line_start(&text, first - 1);
            let region_end = line_end(&text, last);
            let mut block: Vec<&str> = lines[first - 1..=last].to_vec();
            let previous = block.remove(0);
            block.push(previous);
            let new_region = block.join("\n");
            store.splice(region_start, region_end - region_start, &new_region)?;
            self.carets = Carets::caret(region_start);
        } else if last + 1 < lines.len() {
            let region_start = line_start(&text, first);
            let region_end = line_end(&text, last + 1);
            let mut block: Vec<&str> = lines[first..=last + 1].to_vec();
            let next = block.pop().expect("the next line");
            block.insert(0, next);
            let new_region = block.join("\n");
            let caret = region_start + next.len() + 1;
            store.splice(region_start, region_end - region_start, &new_region)?;
            self.carets = Carets::caret(caret.min(region_start + new_region.len()));
        }
        Ok(())
    }

    /// Duplicates the block of lines the selections touch, as one transaction.
    pub fn duplicate_lines(&mut self, store: &mut TextStore) -> Result<(), TextError> {
        let text = store.text();
        let Some((first, last)) = self.line_span(&text) else {
            return Ok(());
        };
        let region_start = line_start(&text, first);
        let region_end = line_end(&text, last);
        let block = &text[region_start..region_end];
        let new_region = format!("{block}\n{block}");
        let caret = region_start + block.len() + 1;
        store.splice(region_start, region_end - region_start, &new_region)?;
        self.carets = Carets::caret(caret.min(region_start + new_region.len()));
        Ok(())
    }

    /// Deletes the block of lines the selections touch, as one transaction.
    pub fn delete_lines(&mut self, store: &mut TextStore) -> Result<(), TextError> {
        let text = store.text();
        let Some((first, last)) = self.line_span(&text) else {
            return Ok(());
        };
        let region_start = line_start(&text, first);
        let end = line_end(&text, last);
        // Take the newline with the block, so the line after it moves up rather than leaving a gap.
        let end = if end < text.len() { end + 1 } else { end };
        store.splice(region_start, end - region_start, "")?;
        self.carets = Carets::caret(region_start.min(store.len()));
        Ok(())
    }

    /// Prefixes every line the selections touch with a comment prefix, or removes one that all of
    /// them already have, as one transaction.
    pub fn toggle_comment(&mut self, store: &mut TextStore, prefix: &str) -> Result<(), TextError> {
        let text = store.text();
        let Some((first, last)) = self.line_span(&text) else {
            return Ok(());
        };
        let region_start = line_start(&text, first);
        let region_end = line_end(&text, last);
        let region = &text[region_start..region_end];
        let lines: Vec<&str> = region.split('\n').collect();
        let remove = lines
            .iter()
            .all(|line| line.trim_start().starts_with(prefix));
        let new_lines: Vec<String> = lines
            .iter()
            .map(|line| {
                if remove {
                    match line.find(prefix) {
                        Some(at) => {
                            let mut line = (*line).to_owned();
                            line.replace_range(at..at + prefix.len(), "");
                            line
                        }
                        None => (*line).to_owned(),
                    }
                } else {
                    let indent = line.len() - line.trim_start().len();
                    let mut line = (*line).to_owned();
                    line.insert_str(indent, prefix);
                    line
                }
            })
            .collect();
        let new_region = new_lines.join("\n");
        store.splice(region_start, region_end - region_start, &new_region)?;
        self.carets = Carets::caret(region_start);
        Ok(())
    }

    /// Types one character with auto-pairing.
    ///
    /// An opening bracket drops in its closer and leaves the caret between (wrapping the selection,
    /// when there is one). A closing character that is already at the caret is stepped over rather
    /// than duplicated. Everything else is inserted. One transaction, as every command here is.
    pub fn type_char(
        &mut self,
        store: &mut TextStore,
        character: char,
        pairs: &[(char, char)],
    ) -> Result<(), TextError> {
        if let Some(close) = pairs
            .iter()
            .find(|(open, _)| *open == character)
            .map(|(_, close)| *close)
        {
            return self.surround(store, character, close);
        }

        let text = store.text();
        if pairs.iter().any(|(_, close)| *close == character) {
            let mut edits = Vec::new();
            let mut carets = Vec::new();
            let mut delta: isize = 0;
            for selection in self.carets.selections() {
                let start = selection.start();
                let base = (start as isize + delta) as usize;
                if selection.is_caret() && text[selection.cursor()..].starts_with(character) {
                    // The closer is already there: step over it rather than duplicating it.
                    carets.push(Selection::caret(base + character.len_utf8()));
                } else {
                    edits.push((start, selection.len(), character.to_string()));
                    carets.push(Selection::caret(base + character.len_utf8()));
                    delta += character.len_utf8() as isize - selection.len() as isize;
                }
            }
            store.edit_many(edits)?;
            self.carets = Carets { selections: carets };
            return Ok(());
        }

        self.insert(store, &character.to_string())
    }

    /// Wraps every selection with `open` and `close` as one transaction.
    ///
    /// A caret becomes the pair with the caret between, and a selection is surrounded by the pair.
    /// This is the one wrap rule: [`Editor::type_char`] reaches it for its opening brackets, and the
    /// guarded surround of [T06.09](crate::app::snippets) reaches it through [`surround_edits`].
    pub fn surround(
        &mut self,
        store: &mut TextStore,
        open: char,
        close: char,
    ) -> Result<(), TextError> {
        let text = store.text();
        let (edits, carets) = surround_edits(&text, self.carets.selections(), open, close);
        store.edit_many(edits)?;
        self.carets = carets;
        Ok(())
    }

    /// The block of lines the selections touch, as `(first, last)` line indices.
    fn line_span(&self, text: &str) -> Option<(usize, usize)> {
        if self.carets.selections.is_empty() {
            return None;
        }
        let count = text.split('\n').count();
        let mut first = usize::MAX;
        let mut last = 0;
        for selection in self.carets.selections() {
            let start = line_index(text, selection.start());
            let end = if selection.is_caret() {
                start
            } else if text.as_bytes().get(selection.end() - 1) == Some(&b'\n') {
                // The selection ends exactly at a line start; it does not touch that line.
                line_index(text, selection.end() - 1)
            } else {
                line_index(text, selection.end())
            };
            first = first.min(start);
            last = last.max(end.min(count - 1));
        }
        Some((first, last))
    }
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

/// The `(open, close)` pairs auto-pairing knows.
pub const PAIRS: [(char, char); 4] = [('(', ')'), ('[', ']'), ('{', '}'), ('"', '"')];

/// The edits that wrap every selection with `open` and `close`, and where the carets land.
///
/// This is the single wrap rule. [`Editor::surround`] applies it, and the guarded surround of
/// [T06.09](crate::app::snippets) checks the same edits against the protected regions before it is
/// applied, so a wrap cannot be allowed by one path and refused by another.
pub fn surround_edits(
    text: &str,
    selections: &[Selection],
    open: char,
    close: char,
) -> (Vec<(usize, usize, String)>, Carets) {
    let mut edits = Vec::new();
    let mut carets = Vec::new();
    let mut delta: isize = 0;
    for selection in selections {
        let start = selection.start();
        let base = (start as isize + delta) as usize;
        let covered = &text[start..selection.end()];
        edits.push((start, selection.len(), format!("{open}{covered}{close}")));
        carets.push(Selection::caret(base + open.len_utf8() + selection.len()));
        delta += (open.len_utf8() + close.len_utf8()) as isize;
    }
    (edits, Carets { selections: carets })
}

/// The offset of the bracket matching the one at `offset`, when there is one.
///
/// The bracket considered is the one at `offset`, or the one just before it when the offset is
/// between characters. The scan does not skip strings or comments (a recorded boundary).
pub fn matching_bracket(text: &str, offset: usize) -> Option<usize> {
    let (at, bracket) = bracket_at(text, offset)?;
    let (open, close) = pair_of(bracket)?;
    if bracket == open {
        let mut depth = 0i32;
        for (index, character) in text[at..].char_indices() {
            if character == open {
                depth += 1;
            } else if character == close {
                depth -= 1;
                if depth == 0 {
                    return Some(at + index);
                }
            }
        }
        None
    } else {
        let mut depth = 0i32;
        for (index, character) in text[..at + bracket.len_utf8()].char_indices().rev() {
            if character == close {
                depth += 1;
            } else if character == open {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
        }
        None
    }
}

/// The bracket at an offset, or just before it.
fn bracket_at(text: &str, offset: usize) -> Option<(usize, char)> {
    for candidate in [offset, offset.saturating_sub(1)] {
        if let Some(character) = text[candidate..].chars().next()
            && pair_of(character).is_some()
        {
            return Some((candidate, character));
        }
    }
    None
}

fn pair_of(character: char) -> Option<(char, char)> {
    PAIRS
        .iter()
        .copied()
        .find(|(open, close)| *open == character || *close == character)
}

/// The nearest character boundary at or before an offset.
fn boundary(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

/// The boundary before an offset (the start of the character ending there).
fn previous_boundary(text: &str, offset: usize) -> Option<usize> {
    if offset == 0 {
        return None;
    }
    Some(boundary(text, offset - 1))
}

/// The boundary after an offset (the end of the character starting there).
fn next_boundary(text: &str, offset: usize) -> Option<usize> {
    if offset >= text.len() {
        return None;
    }
    text[offset.min(text.len())..]
        .chars()
        .next()
        .map(|character| offset + character.len_utf8())
}

/// The line index a byte offset falls in.
fn line_index(text: &str, offset: usize) -> usize {
    let offset = boundary(text, offset);
    text[..offset].matches('\n').count()
}

/// The byte offset a line starts at.
fn line_start(text: &str, line: usize) -> usize {
    if line == 0 {
        return 0;
    }
    let mut count = 0;
    for (index, character) in text.char_indices() {
        if character == '\n' {
            count += 1;
            if count == line {
                return index + 1;
            }
        }
    }
    text.len()
}

/// The byte offset a line ends at (its newline, or the end of the text).
fn line_end(text: &str, line: usize) -> usize {
    let mut count = 0;
    for (index, character) in text.char_indices() {
        if character == '\n' {
            if count == line {
                return index;
            }
            count += 1;
        }
    }
    text.len()
}

#[cfg(test)]
mod tests {
    use super::{Carets, Editor, PAIRS, Selection, matching_bracket};
    use crate::project::text::TextStore;

    fn text_store(text: &str) -> TextStore {
        TextStore::new(text)
    }

    /// An editor with exactly these selections, not snapped to boundaries, so a test can hand the
    /// store an offset that is deliberately wrong and check that it refuses.
    fn editor_with(selections: Vec<Selection>) -> Editor {
        Editor {
            carets: Carets::of(selections).expect("valid carets"),
        }
    }

    #[test]
    fn a_multi_caret_insert_is_one_transaction() {
        let mut store = text_store("axbxc");
        let mut editor = editor_with(vec![Selection::caret(1), Selection::caret(3)]);

        editor.insert(&mut store, "-").unwrap();
        assert_eq!(store.text(), "a-xb-xc");
        // One undo takes back the whole transaction, not one caret of it.
        assert!(store.undo().unwrap());
        assert_eq!(store.text(), "axbxc");
        assert!(!store.can_undo());
        assert!(store.redo().unwrap());
        assert_eq!(store.text(), "a-xb-xc");
    }

    #[test]
    fn a_selection_replaced_by_typing_lands_at_every_caret() {
        let mut store = text_store("one two three");
        let mut editor = editor_with(vec![Selection::range(0, 3), Selection::range(8, 13)]);
        editor.insert(&mut store, "X").unwrap();
        assert_eq!(store.text(), "X two X");
    }

    #[test]
    fn a_multi_caret_edit_never_splits_a_character() {
        // 'é' is two bytes; '中' is three. Delete a whole wide character from each side.
        let mut store = text_store("é中x");
        let mut editor = editor_with(vec![Selection::range(0, 2), Selection::range(2, 5)]);
        editor.backspace(&mut store).unwrap();
        assert_eq!(store.text(), "x");

        // A caret handed an offset inside a character is snapped, not used to split it.
        let mut store = text_store("é中");
        let mut editor = Editor::at(0);
        editor
            .set_carets("é中", Carets::of(vec![Selection::caret(1)]).unwrap())
            .expect("snaps to a boundary");
        assert_eq!(editor.carets().primary().cursor(), 0);
        // And an edit that would split a character is refused as a whole by the store.
        let mut editor = editor_with(vec![Selection::caret(0), Selection::caret(1)]);
        // Snap is not applied here, so the second caret is mid-character; the store refuses.
        assert!(editor.insert(&mut store, "x").is_err());
        assert_eq!(store.text(), "é中");
    }

    #[test]
    fn carets_refuse_an_overlap() {
        let mut carets = Carets::caret(0);
        carets.add(Selection::range(2, 5)).unwrap();
        assert!(carets.add(Selection::range(3, 6)).is_err(), "overlaps 2..5");
        // Two carets at the same place are fine.
        assert!(carets.add(Selection::caret(10)).is_ok());
        assert!(carets.add(Selection::caret(10)).is_ok());
    }

    #[test]
    fn backspace_deletes_one_character_at_every_caret_as_one_transaction() {
        let mut store = text_store("aXbYc");
        let mut editor = editor_with(vec![Selection::caret(2), Selection::caret(4)]);
        editor.backspace(&mut store).unwrap();
        assert_eq!(store.text(), "abc");
        assert!(store.undo().unwrap());
        assert_eq!(store.text(), "aXbYc");
    }

    #[test]
    fn line_operations_move_duplicate_and_delete_the_selected_lines() {
        let mut store = text_store("one\ntwo\nthree\nfour");
        // The caret is on line 1 ("two"); move it up.
        let mut editor = Editor::at(5);
        editor.move_lines(&mut store, -1).unwrap();
        assert_eq!(store.text(), "two\none\nthree\nfour");

        // Move it back down.
        editor.move_lines(&mut store, 1).unwrap();
        assert_eq!(store.text(), "one\ntwo\nthree\nfour");

        // Duplicate the line.
        let mut editor = Editor::at(5);
        editor.duplicate_lines(&mut store).unwrap();
        assert_eq!(store.text(), "one\ntwo\ntwo\nthree\nfour");

        // Delete the two "two" lines.
        let mut editor = Editor::at(5);
        editor
            .set_carets(
                &store.text(),
                Carets::of(vec![Selection::range(4, 11)]).unwrap(),
            )
            .unwrap();
        editor.delete_lines(&mut store).unwrap();
        assert_eq!(store.text(), "one\nthree\nfour");
    }

    #[test]
    fn toggling_a_comment_prefixes_then_unprefixes_the_lines() {
        let mut store = text_store("let a = 1;\n  let b = 2;");
        let mut editor = Editor::at(0);
        // Select both lines.
        editor
            .set_carets(
                &store.text(),
                Carets::of(vec![Selection::range(0, 24)]).unwrap(),
            )
            .unwrap();
        editor.toggle_comment(&mut store, "// ").unwrap();
        assert_eq!(store.text(), "// let a = 1;\n  // let b = 2;");
        // One undo takes back all the prefixes.
        assert!(store.undo().unwrap());
        assert_eq!(store.text(), "let a = 1;\n  let b = 2;");
        // Toggling again removes the prefixes.
        editor.toggle_comment(&mut store, "// ").unwrap();
        editor.toggle_comment(&mut store, "// ").unwrap();
        assert_eq!(store.text(), "let a = 1;\n  let b = 2;");
    }

    #[test]
    fn matching_bracket_handles_nesting_and_an_unmatched_one() {
        let text = "fn f() { g(a[1]) }";
        let open = text.find('{').unwrap();
        let close = text.rfind('}').unwrap();
        assert_eq!(matching_bracket(text, open), Some(close));
        assert_eq!(matching_bracket(text, close), Some(open));
        // Nested parentheses: the `(` of `g(a[1])` matches its `)`.
        let paren = text.find("(a").unwrap();
        let matching = matching_bracket(text, paren).unwrap();
        assert_eq!(&text[paren..=matching], "(a[1])");
        // An unmatched opening bracket has no match.
        assert_eq!(matching_bracket("fn f() {", 7), None);
        // A non-bracket has none.
        assert_eq!(matching_bracket("abc", 1), None);
    }

    #[test]
    fn typing_an_opener_inserts_its_pair_and_typing_the_closer_steps_over_it() {
        let mut store = text_store("");
        let mut editor = Editor::at(0);
        editor.type_char(&mut store, '(', &PAIRS).unwrap();
        assert_eq!(store.text(), "()");
        assert_eq!(
            editor.carets().primary().cursor(),
            1,
            "the caret is between"
        );

        // Typing the closer that is already there steps over it.
        editor.type_char(&mut store, ')', &PAIRS).unwrap();
        assert_eq!(store.text(), "()");
        assert_eq!(editor.carets().primary().cursor(), 2);

        // Typing an opener around a selection wraps it.
        let mut store = text_store("word");
        let mut editor = Editor::at(0);
        editor
            .set_carets(
                &store.text(),
                Carets::of(vec![Selection::range(0, 4)]).unwrap(),
            )
            .unwrap();
        editor.type_char(&mut store, '(', &PAIRS).unwrap();
        assert_eq!(store.text(), "(word)");
        assert_eq!(editor.carets().primary().cursor(), 5);
    }
}
