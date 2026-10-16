// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Text that is edited in place: a piece table, range undo, and a lazy full-text mirror
//! (task T06.01).
//!
//! The acceptance line is *"a one-character edit does not unconditionally clone the whole text"*.
//! That is a statement about what an edit **touches**, so the store keeps the two things an edit
//! must not do impossible:
//!
//! * **The original text is immutable and shared, never copied.** [`TextStore::new`] holds the text
//!   as one [`Arc<str>`] piece; a splice only splits and re-links *pieces* and appends the inserted
//!   bytes to an append-only side. Nothing in the edit path reads or copies `original`'s bytes, so
//!   editing one character in a large document costs one piece and one byte, not a document.
//! * **The full text is built only when asked for.** There is no eagerly-updated mirror that an edit
//!   must keep in step; [`TextStore::text`] walks the pieces and reassembles, and
//!   [`TextStore::materializations`] counts how many times it has — a test can therefore check that
//!   an **edit** built none, which is the difference between "one character" and "the whole text".
//!
//! Two more properties the store holds rather than promises:
//!
//! * **Undo is a range, not a snapshot.** Every splice records the exact [`Edit`] — where, what was
//!   removed, what was put back — and [`TextStore::undo`] reverses it. A keystroke costs one small
//!   record, not a copy of the document.
//! * **An edit that would split a character is refused.** Byte offsets are checked against the
//!   source string's char boundaries, so a splice can never leave the pieces holding invalid UTF-8,
//!   which is what lets [`TextStore::text`] slice by byte without checking later.
//!
//! This is the text the buffer and every view read: the shared buffer of [`super::document`] is one
//! record, and the two views of T04.04 both read it. Character and visual coordinates (columns, tabs,
//! folding) are T06.03's subject, built on these byte offsets rather than a second text.

#![allow(dead_code)]

use std::cell::Cell;
use std::sync::Arc;

/// Where a piece's bytes come from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Source {
    /// The text the store was created with. Immutable, and never copied by an edit.
    Original,
    /// Bytes appended by edits, in the order they were inserted.
    Added,
}

/// One piece of the text: a range of one of the two sources.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Piece {
    source: Source,
    start: usize,
    len: usize,
}

/// One change: enough to undo it exactly.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Edit {
    at: usize,
    removed: String,
    inserted: String,
}

impl Edit {
    /// Where the change begins, as a byte offset in the text before it.
    pub const fn at(&self) -> usize {
        self.at
    }

    /// What it removed.
    pub fn removed(&self) -> &str {
        &self.removed
    }

    /// What it put in place of the removal.
    pub fn inserted(&self) -> &str {
        &self.inserted
    }

    /// Whether it only inserted (no removal).
    pub fn is_insert(&self) -> bool {
        self.removed.is_empty()
    }

    /// The change as one line.
    pub fn render(&self) -> String {
        format!(
            "at {}: removed {:?}, inserted {:?}",
            self.at, self.removed, self.inserted
        )
    }
}

/// Why an edit was refused.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum TextError {
    /// The edit reaches past the end of the text.
    OutOfRange {
        /// Where it starts.
        at: usize,
        /// How much it removes.
        remove: usize,
        /// How long the text is.
        len: usize,
    },
    /// The offset falls inside a character, so the edit would split it.
    NotABoundary {
        /// The offending offset.
        at: usize,
    },
    /// Two edits in one transaction overlap, so which one wins would be undefined.
    Overlap {
        /// The start of the later edit.
        at: usize,
    },
}

impl TextError {
    /// The one-line explanation a person is shown.
    pub fn message(&self) -> String {
        match self {
            Self::OutOfRange { at, remove, len } => {
                format!("removing {remove} byte(s) at {at} reaches past the {len}-byte text")
            }
            Self::NotABoundary { at } => {
                format!("offset {at} is inside a character, so the edit would split it")
            }
            Self::Overlap { at } => {
                format!("the edits overlap at {at}, so which one wins is undefined")
            }
        }
    }
}

/// One undo step: the edits that were applied as one, in the order they landed.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Step {
    edits: Vec<Edit>,
}

/// A text edited in place: pieces over an immutable original and an append-only side.
#[derive(Clone, Debug)]
pub struct TextStore {
    original: Arc<str>,
    added: String,
    pieces: Vec<Piece>,
    history: Vec<Step>,
    applied: usize,
    materializations: Cell<usize>,
}

impl TextStore {
    /// A store over a text, held as one immutable piece.
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let len = text.len();
        let pieces = if len == 0 {
            Vec::new()
        } else {
            vec![Piece {
                source: Source::Original,
                start: 0,
                len,
            }]
        };
        Self {
            original: Arc::from(text),
            added: String::new(),
            pieces,
            history: Vec::new(),
            applied: 0,
            materializations: Cell::new(0),
        }
    }

    /// How long the text is, in bytes.
    pub fn len(&self) -> usize {
        self.pieces.iter().map(|piece| piece.len).sum()
    }

    /// Whether the text is empty.
    pub fn is_empty(&self) -> bool {
        self.pieces.is_empty()
    }

    /// How many bytes the immutable original holds.
    pub fn original_len(&self) -> usize {
        self.original.len()
    }

    /// How many bytes have been appended by edits. Grows by exactly what was inserted.
    pub fn added_len(&self) -> usize {
        self.added.len()
    }

    /// How many pieces the text is currently held as.
    pub fn piece_count(&self) -> usize {
        self.pieces.len()
    }

    /// How many times the full text has been reassembled.
    ///
    /// An edit never reassembles, so this staying put across an edit is the acceptance in a number.
    pub fn materializations(&self) -> usize {
        self.materializations.get()
    }

    /// The full text, reassembled from the pieces.
    pub fn text(&self) -> String {
        self.materializations.set(self.materializations.get() + 1);
        let mut out = String::with_capacity(self.len());
        for piece in &self.pieces {
            let source = self.source(piece.source);
            out.push_str(&source[piece.start..piece.start + piece.len]);
        }
        out
    }

    /// How many characters the text holds.
    pub fn char_len(&self) -> usize {
        self.text().chars().count()
    }

    /// Whether an edit can be taken back.
    pub fn can_undo(&self) -> bool {
        self.applied > 0
    }

    /// Whether a taken-back edit can be put back.
    pub fn can_redo(&self) -> bool {
        self.applied < self.history.len()
    }

    /// Replaces `remove` bytes at `at` with `insert`, and returns the change.
    ///
    /// The original is not copied: the pieces are split and re-linked, and the inserted bytes are
    /// appended to the side. An out-of-range or character-splitting edit is refused and changes
    /// nothing.
    pub fn splice(&mut self, at: usize, remove: usize, insert: &str) -> Result<Edit, TextError> {
        let removed = self.apply(at, remove, insert)?;
        let edit = Edit {
            at,
            removed,
            inserted: insert.to_owned(),
        };
        self.record(vec![edit.clone()]);
        Ok(edit)
    }

    /// Applies several edits as **one transaction**, and returns them in the order they landed.
    ///
    /// This is what a multi-cursor edit is: the same keystroke at several places, which a person
    /// takes back with one undo. The edits are applied from the highest offset down, so an edit does
    /// not move the offsets of the ones below it; overlapping edits are refused before anything is
    /// applied, so a wrong batch changes nothing (and a batch whose offsets are not on character
    /// boundaries is refused for the same reason a single one is).
    pub fn edit_many(
        &mut self,
        edits: Vec<(usize, usize, String)>,
    ) -> Result<Vec<Edit>, TextError> {
        // A transaction with no edits changes nothing, so it must not become an undo step: a
        // backspace at the start of the text, or a closing bracket stepped over, is not undoable.
        if edits.is_empty() {
            return Ok(Vec::new());
        }
        let len = self.len();
        let mut ranges: Vec<(usize, usize)> = Vec::with_capacity(edits.len());
        for (at, remove, _) in &edits {
            if *at > len || at + remove > len {
                return Err(TextError::OutOfRange {
                    at: *at,
                    remove: *remove,
                    len,
                });
            }
            if !self.is_boundary(*at) {
                return Err(TextError::NotABoundary { at: *at });
            }
            if !self.is_boundary(at + remove) {
                return Err(TextError::NotABoundary { at: at + remove });
            }
            ranges.push((*at, at + remove));
        }
        ranges.sort_unstable();
        for window in ranges.windows(2) {
            if window[1].0 < window[0].1 {
                return Err(TextError::Overlap { at: window[1].0 });
            }
        }

        let mut order: Vec<usize> = (0..edits.len()).collect();
        order.sort_by(|a, b| edits[*b].0.cmp(&edits[*a].0));
        let mut applied = Vec::with_capacity(edits.len());
        for index in order {
            let (at, remove, insert) = &edits[index];
            let removed = self.apply(*at, *remove, insert)?;
            applied.push(Edit {
                at: *at,
                removed,
                inserted: insert.clone(),
            });
        }
        self.record(applied.clone());
        Ok(applied)
    }

    /// Pushes one step and drops any abandoned redo branch.
    fn record(&mut self, edits: Vec<Edit>) {
        self.history.truncate(self.applied);
        self.history.push(Step { edits });
        self.applied += 1;
    }

    /// Takes back the newest step, and says whether there was one.
    pub fn undo(&mut self) -> Result<bool, TextError> {
        if self.applied == 0 {
            return Ok(false);
        }
        let step = self.history[self.applied - 1].clone();
        for edit in step.edits.iter().rev() {
            self.apply(edit.at, edit.inserted.len(), &edit.removed)?;
        }
        self.applied -= 1;
        Ok(true)
    }

    /// Puts back the newest taken-back step, and says whether there was one.
    pub fn redo(&mut self) -> Result<bool, TextError> {
        if self.applied >= self.history.len() {
            return Ok(false);
        }
        let step = self.history[self.applied].clone();
        for edit in &step.edits {
            self.apply(edit.at, edit.removed.len(), &edit.inserted)?;
        }
        self.applied += 1;
        Ok(true)
    }

    /// The store in one report.
    pub fn render(&self) -> Vec<String> {
        vec![
            format!(
                "text: {} byte(s), {} character(s), {} piece(s)",
                self.len(),
                self.char_len(),
                self.piece_count()
            ),
            format!(
                "original {} byte(s), additions {} byte(s), reassembled {} time(s)",
                self.original_len(),
                self.added_len(),
                self.materializations()
            ),
            format!(
                "history: {} step(s), {} applied, {} to redo",
                self.history.len(),
                self.applied,
                self.history.len() - self.applied
            ),
        ]
    }

    /// The source string a piece reads from.
    fn source(&self, source: Source) -> &str {
        match source {
            Source::Original => &self.original,
            Source::Added => &self.added,
        }
    }

    /// The `(piece index, offset within it)` an offset falls at, when it is within the text.
    fn locate(&self, at: usize) -> Option<(usize, usize)> {
        let mut remaining = at;
        for (index, piece) in self.pieces.iter().enumerate() {
            if remaining <= piece.len {
                return Some((index, remaining));
            }
            remaining -= piece.len;
        }
        None
    }

    /// Whether a byte offset falls on a character boundary of the piece's source.
    fn is_boundary(&self, at: usize) -> bool {
        // An empty text has one boundary: its start. There is no piece to ask, so it is answered
        // here rather than letting an empty store refuse every insertion.
        if self.pieces.is_empty() {
            return at == 0;
        }
        let Some((index, offset)) = self.locate(at) else {
            return false;
        };
        let piece = self.pieces[index];
        self.source(piece.source)
            .is_char_boundary(piece.start + offset)
    }

    /// Splits a piece at an offset within it, when the offset is strictly inside.
    fn split_at(&mut self, index: usize, offset: usize) {
        let piece = self.pieces[index];
        if offset == 0 || offset >= piece.len {
            return;
        }
        self.pieces[index] = Piece {
            source: piece.source,
            start: piece.start,
            len: offset,
        };
        self.pieces.insert(
            index + 1,
            Piece {
                source: piece.source,
                start: piece.start + offset,
                len: piece.len - offset,
            },
        );
    }

    /// The bytes in a range, reassembled. Used to record what an edit removed.
    fn slice(&self, at: usize, len: usize) -> String {
        let mut out = String::with_capacity(len);
        let mut at = at;
        let mut left = len;
        for piece in &self.pieces {
            if left == 0 {
                break;
            }
            if at >= piece.len {
                at -= piece.len;
                continue;
            }
            let take = (piece.len - at).min(left);
            let source = self.source(piece.source);
            out.push_str(&source[piece.start + at..piece.start + at + take]);
            left -= take;
            at = 0;
        }
        out
    }

    /// The piece surgery of a splice, without touching the history. Returns what was removed.
    fn apply(&mut self, at: usize, remove: usize, insert: &str) -> Result<String, TextError> {
        let len = self.len();
        if at > len || at + remove > len {
            return Err(TextError::OutOfRange { at, remove, len });
        }
        if !self.is_boundary(at) {
            return Err(TextError::NotABoundary { at });
        }
        if !self.is_boundary(at + remove) {
            return Err(TextError::NotABoundary { at: at + remove });
        }

        let removed = self.slice(at, remove);

        // Split at the start so the offset is a piece boundary, then remove across pieces. An empty
        // text has no piece to split, and its only insertion point is the start.
        let cursor = if self.pieces.is_empty() {
            0
        } else {
            let (index, offset) = self.locate(at).expect("at is within the text");
            self.split_at(index, offset);
            if offset == 0 { index } else { index + 1 }
        };
        let mut left = remove;
        let at_piece = cursor;
        while left > 0 {
            let piece_len = self.pieces[at_piece].len;
            if piece_len <= left {
                left -= piece_len;
                self.pieces.remove(at_piece);
            } else {
                self.split_at(at_piece, left);
                self.pieces.remove(at_piece);
                left = 0;
            }
        }

        if !insert.is_empty() {
            let start = self.added.len();
            self.added.push_str(insert);
            self.pieces.insert(
                cursor,
                Piece {
                    source: Source::Added,
                    start,
                    len: insert.len(),
                },
            );
        }
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::{TextError, TextStore};

    #[test]
    fn a_new_store_holds_its_text_as_one_immutable_piece() {
        let store = TextStore::new("hello world");
        assert_eq!(store.len(), 11);
        assert_eq!(store.piece_count(), 1);
        assert_eq!(store.original_len(), 11);
        assert_eq!(store.added_len(), 0);
        assert_eq!(
            store.materializations(),
            0,
            "creating it reassembles nothing"
        );
        assert_eq!(store.text(), "hello world");
    }

    #[test]
    fn a_one_character_edit_does_not_clone_the_original_or_reassemble_it() {
        // A large text, to make "copied the whole document" observable rather than a rounding error.
        let big = "x".repeat(1 << 20);
        let mut store = TextStore::new(big);
        let before = store.materializations();

        store.splice(0, 0, "a").expect("insert at the start");

        // The edit touched one piece and appended one byte; it reassembled nothing.
        assert_eq!(store.materializations(), before, "the edit built no mirror");
        assert_eq!(store.added_len(), 1, "only the inserted byte was added");
        assert_eq!(store.original_len(), 1 << 20, "the original is uncopied");
        assert_eq!(store.piece_count(), 2);

        // And it is still correct.
        assert_eq!(store.text().len(), (1 << 20) + 1);
        assert!(store.text().starts_with('a'));
        assert_eq!(
            store.materializations(),
            before + 2,
            "asking for the text is what reassembles"
        );
    }

    #[test]
    fn an_empty_store_accepts_an_insertion_at_its_start() {
        let mut store = TextStore::new("");
        store.splice(0, 0, "hi").expect("an empty text has a start");
        assert_eq!(store.text(), "hi");
        assert_eq!(store.original_len(), 0);
        assert_eq!(store.added_len(), 2);
        // And an offset past the start of an empty text is still out of range.
        assert!(store.splice(0, 3, "x").is_err());
    }

    #[test]
    fn inserting_removing_and_replacing_all_land() {
        let mut store = TextStore::new("hello world");
        store.splice(5, 0, ",").unwrap();
        assert_eq!(store.text(), "hello, world");
        store.splice(0, 5, "goodbye").unwrap();
        assert_eq!(store.text(), "goodbye, world");
        store.splice(7, 1, "").unwrap();
        assert_eq!(store.text(), "goodbye world");
    }

    #[test]
    fn undo_and_redo_walk_the_range_history() {
        let mut store = TextStore::new("abc");
        store.splice(3, 0, "def").unwrap();
        assert_eq!(store.text(), "abcdef");
        store.splice(0, 1, "A").unwrap();
        assert_eq!(store.text(), "Abcdef");

        assert!(store.can_undo());
        assert!(store.undo().unwrap());
        assert_eq!(store.text(), "abcdef");
        assert!(store.undo().unwrap());
        assert_eq!(store.text(), "abc");
        assert!(!store.undo().unwrap(), "nothing left to take back");
        assert!(!store.can_undo());

        assert!(store.can_redo());
        assert!(store.redo().unwrap());
        assert_eq!(store.text(), "abcdef");
        assert!(store.redo().unwrap());
        assert_eq!(store.text(), "Abcdef");
        assert!(!store.redo().unwrap());
    }

    #[test]
    fn a_new_edit_after_an_undo_drops_the_redo() {
        let mut store = TextStore::new("abc");
        store.splice(3, 0, "d").unwrap();
        store.undo().unwrap();
        assert!(store.can_redo());
        store.splice(0, 0, "z").unwrap();
        assert!(!store.can_redo(), "the abandoned branch is gone");
        assert_eq!(store.text(), "zabc");
    }

    #[test]
    fn a_span_removed_across_pieces_is_recorded_whole_and_undoes_exactly() {
        let mut store = TextStore::new("hello world");
        store.splice(5, 0, " there").unwrap();
        store.splice(0, 5, "goodbye").unwrap();
        assert_eq!(store.text(), "goodbye there world");

        // Remove a span that crosses several pieces; undo must put it back byte for byte.
        let removed = store.splice(0, 12, "").unwrap();
        assert_eq!(removed.removed(), "goodbye ther");
        assert_eq!(store.text(), "e world");
        store.undo().unwrap();
        assert_eq!(store.text(), "goodbye there world");
    }

    #[test]
    fn an_out_of_range_or_character_splitting_edit_is_refused_and_changes_nothing() {
        let mut store = TextStore::new("héllo"); // 'é' is two bytes
        let before = store.text();

        // Past the end.
        assert_eq!(
            store.splice(100, 0, "x"),
            Err(TextError::OutOfRange {
                at: 100,
                remove: 0,
                len: 6
            })
        );
        // Byte 1 is the 'é'\'s start (a boundary); byte 2 is inside it (not one).
        assert!(store.splice(1, 0, "x").is_ok(), "byte 1 starts the 'é'");
        assert!(store.undo().unwrap());
        assert_eq!(
            store.splice(2, 0, "x"),
            Err(TextError::NotABoundary { at: 2 })
        );
        // Removing half of it is refused too.
        assert!(matches!(
            store.splice(2, 1, ""),
            Err(TextError::NotABoundary { .. })
        ));

        assert_eq!(store.text(), before);
        assert!(!store.can_undo(), "a refused edit is not a step");
    }

    #[test]
    fn a_group_of_edits_lands_as_one_undo_step() {
        let mut store = TextStore::new("aXbXc");
        // Insert the same character before each X — the shape of a multi-cursor keystroke.
        store
            .edit_many(vec![(1, 0, "-".to_owned()), (3, 0, "-".to_owned())])
            .expect("the batch lands");
        assert_eq!(store.text(), "a-Xb-Xc");

        // One undo takes back the whole transaction, not one edit of it.
        assert!(store.undo().unwrap());
        assert_eq!(store.text(), "aXbXc");
        assert!(!store.can_undo());
        // And one redo puts it all back.
        assert!(store.redo().unwrap());
        assert_eq!(store.text(), "a-Xb-Xc");
    }

    #[test]
    fn a_group_whose_offsets_are_out_of_order_still_lands_exactly() {
        let mut store = TextStore::new("0123456789");
        // Descending application keeps every offset valid without the caller sorting them.
        store
            .edit_many(vec![
                (2, 1, "aa".to_owned()),
                (7, 1, "bbb".to_owned()),
                (5, 0, "c".to_owned()),
            ])
            .unwrap();
        assert_eq!(store.text(), "01aa34c56bbb89");
    }

    #[test]
    fn an_overlapping_or_boundary_splitting_batch_changes_nothing() {
        // Overlapping edits: which one wins would be undefined.
        let mut store = TextStore::new("abcdef");
        assert_eq!(
            store.edit_many(vec![(0, 2, "x".to_owned()), (1, 1, "y".to_owned())]),
            Err(TextError::Overlap { at: 1 })
        );
        assert_eq!(store.text(), "abcdef");
        assert!(!store.can_undo(), "a refused batch is not a step");

        // An edit that splits a character ('é' is bytes 1..3).
        let mut store = TextStore::new("héllo");
        let before = store.text();
        assert_eq!(
            store.edit_many(vec![(2, 0, "x".to_owned())]),
            Err(TextError::NotABoundary { at: 2 })
        );
        // One good edit and one bad one: the good one does not land either.
        assert!(
            store
                .edit_many(vec![(0, 0, "x".to_owned()), (2, 0, "y".to_owned())])
                .is_err()
        );
        assert_eq!(store.text(), before);
        assert!(!store.can_undo(), "a refused batch is not a step");
    }

    #[test]
    fn the_report_names_the_pieces_the_lengths_and_the_reassemblies() {
        let mut store = TextStore::new("abc");
        store.splice(1, 0, "XY").unwrap();
        let report = store.render().join("\n");
        assert!(report.contains("5 byte(s)"), "{report}");
        assert!(report.contains("piece(s)"), "{report}");
        assert!(report.contains("additions 2 byte(s)"), "{report}");
        assert!(report.contains("1 applied"), "{report}");
    }
}
