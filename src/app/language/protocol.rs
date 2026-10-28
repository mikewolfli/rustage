// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Document synchronisation and the one place bytes and protocol positions meet (task T07.02).
//!
//! The acceptance line is *"incremental synchronisation of Chinese text and emoji is correct, and
//! the positions it returns are correct"*. Two encodings meet here — the editor's UTF-8 byte offsets
//! and the protocol's UTF-16 code-unit positions — so the module puts the conversion in exactly one
//! pair of functions rather than letting it leak:
//!
//! * **A protocol position is a line and a UTF-16 count.** [`Position`] is what the protocol speaks,
//!   and [`to_units`]/[`to_offset`] are the only conversions. A Chinese character is one UTF-8
//!   character of three bytes and one UTF-16 unit; an emoji is one character of four bytes and two
//!   UTF-16 units, so the count is taken per character rather than per byte or per scalar.
//! * **A position inside a character snaps to its boundary.** [`to_offset`] walks whole characters,
//!   so a protocol position that lands between an emoji's surrogate halves resolves to the start of
//!   that character rather than to invalid UTF-8.
//! * **A change is applied to the version it names.** [`Sync::change`] refuses an edit for a version
//!   that is not the document's current one, so a stale change cannot silently overwrite the buffer.
//! * **Several edits in one change are applied by the rules the protocol states.** All ranges refer to
//!   the version before the change, so they are applied from the highest offset down (the same shape
//!   as [`TextStore::edit_many`](crate::project::text::piece_table::TextStore::edit_many)), and an overlap is
//!   refused whole.
//! * **A typed operation returns the notification to send.** [`Operation`] and [`Notification`] keep
//!   the version rule in one place: [`Sync::apply`] opens at 1, increments on every full or
//!   incremental change, and leaves the version alone for a save. The transport turns the returned
//!   notification into `textDocument/*` JSON without deciding any version itself.
//!
//! **Boundary.** The document text is held as one string here; the piece table of
//! [T06.01](crate::project::text::piece_table) is the buffer the editor edits, and this module is the *protocol*
//! view of it, which is why a change is modelled as a described range rather than a store edit.

#![allow(dead_code)]

use std::collections::BTreeMap;

/// A position in the protocol's terms: a line, and a character counted in UTF-16 code units.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Position {
    /// The zero-based line.
    pub line: u32,
    /// The zero-based character, in UTF-16 code units.
    pub character: u32,
}

impl Position {
    /// A position of a line and a UTF-16 character count.
    pub const fn new(line: u32, character: u32) -> Self {
        Self { line, character }
    }

    /// The position in one line.
    pub fn render(&self) -> String {
        format!("{}:{}", self.line, self.character)
    }
}

/// A range the protocol describes, in positions.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Range {
    /// Where it starts.
    pub start: Position,
    /// Where it ends.
    pub end: Position,
}

impl Range {
    /// A range of two positions.
    pub const fn new(start: Position, end: Position) -> Self {
        Self { start, end }
    }
}

/// One edit of an incremental change: a range to replace, and the text to put there.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Change {
    /// The range being replaced.
    pub range: Range,
    /// The text replacing it.
    pub text: String,
}

impl Change {
    /// A change of a range and its replacement.
    pub fn new(range: Range, text: impl Into<String>) -> Self {
        Self {
            range,
            text: text.into(),
        }
    }

    /// The change in one line.
    pub fn render(&self) -> String {
        format!(
            "{}..{} -> {:?}",
            self.range.start.render(),
            self.range.end.render(),
            self.text
        )
    }
}

/// A change as the protocol describes it: the whole text, or the ranges that changed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum DocumentChange {
    /// Replace the whole document (a full synchronisation).
    Full {
        /// The new text.
        text: String,
    },
    /// Replace the listed ranges (an incremental synchronisation).
    Incremental {
        /// The edits, all resolved against the version before them.
        changes: Vec<Change>,
    },
}

impl DocumentChange {
    /// Whether this change is sent as ranges rather than as the whole text.
    pub fn is_incremental(&self) -> bool {
        matches!(self, Self::Incremental { .. })
    }
}

/// A typed `textDocument/*` notification the transport must send.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Notification {
    /// `textDocument/didOpen` with the language id, version and text.
    Open {
        /// The document's uri.
        uri: String,
        /// The language id the server should analyse it as.
        language_id: String,
        /// The version the document opens at.
        version: u64,
        /// The whole text.
        text: String,
    },
    /// `textDocument/didChange` with the version *after* the change.
    Change {
        /// The document's uri.
        uri: String,
        /// The version after the change.
        version: u64,
        /// The full or incremental content.
        change: DocumentChange,
    },
    /// `textDocument/didSave` with the saved text.
    Save {
        /// The document's uri.
        uri: String,
        /// The version the save was made at.
        version: u64,
        /// The text that was saved.
        text: String,
    },
    /// `textDocument/didClose`.
    Close {
        /// The document's uri.
        uri: String,
    },
}

impl Notification {
    /// The method this notification is sent as.
    pub fn method(&self) -> &'static str {
        match self {
            Self::Open { .. } => "textDocument/didOpen",
            Self::Change { .. } => "textDocument/didChange",
            Self::Save { .. } => "textDocument/didSave",
            Self::Close { .. } => "textDocument/didClose",
        }
    }

    /// The document the notification concerns.
    pub fn uri(&self) -> &str {
        match self {
            Self::Open { uri, .. }
            | Self::Change { uri, .. }
            | Self::Save { uri, .. }
            | Self::Close { uri } => uri,
        }
    }

    /// The document version after the operation, when the notification carries one.
    pub fn version(&self) -> Option<u64> {
        match self {
            Self::Open { version, .. }
            | Self::Change { version, .. }
            | Self::Save { version, .. } => Some(*version),
            Self::Close { .. } => None,
        }
    }
}

/// One typed document-synchronisation operation.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Operation {
    /// Open a document at version 1.
    Open {
        /// The document's uri.
        uri: String,
        /// The language id the server should analyse it as.
        language_id: String,
        /// The whole text.
        text: String,
    },
    /// Change an open document. The version is the model's, so it cannot go backwards.
    Change {
        /// The document's uri.
        uri: String,
        /// The full or incremental content.
        change: DocumentChange,
    },
    /// Record a save of an open document.
    Save {
        /// The document's uri.
        uri: String,
    },
    /// Close an open document.
    Close {
        /// The document's uri.
        uri: String,
    },
}

/// Why a synchronisation operation was refused.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SyncError {
    /// The document is not open.
    NotOpen {
        /// The document's uri.
        uri: String,
    },
    /// The document is already open, so a second `didOpen` would be a protocol error.
    AlreadyOpen {
        /// The document's uri.
        uri: String,
    },
    /// The change names a version the document is not at.
    StaleVersion {
        /// The document's uri.
        uri: String,
        /// The version the change named.
        named: u64,
        /// The version the document is at.
        current: u64,
    },
    /// Two edits of one change overlap.
    Overlap {
        /// The uri.
        uri: String,
    },
    /// A range ends before it starts.
    Backwards {
        /// The uri.
        uri: String,
    },
}

impl SyncError {
    /// The refusal in one sentence.
    pub fn message(&self) -> String {
        match self {
            Self::NotOpen { uri } => format!("{uri} is not open"),
            Self::AlreadyOpen { uri } => format!("{uri} is already open"),
            Self::StaleVersion {
                uri,
                named,
                current,
            } => format!("{uri} is at version {current}, not {named}"),
            Self::Overlap { uri } => format!("two edits of the change to {uri} overlap"),
            Self::Backwards { uri } => {
                format!("a range of the change to {uri} ends before it starts")
            }
        }
    }
}

/// What an editor asks the protocol about the document it has open.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Document {
    uri: String,
    language_id: String,
    text: String,
    version: u64,
    dirty: bool,
}

impl Document {
    /// The uri.
    pub fn uri(&self) -> &str {
        &self.uri
    }

    /// The language id the document was opened with.
    pub fn language_id(&self) -> &str {
        &self.language_id
    }

    /// The text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The version. Starts at 1 when opened and grows by one for each change.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Whether it has unsaved changes.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// The document in one line.
    pub fn render(&self) -> String {
        format!(
            "{}: version {}, {} byte(s), {}",
            self.uri,
            self.version,
            self.text.len(),
            if self.dirty { "dirty" } else { "saved" }
        )
    }
}

/// The line a byte offset is on, and its character count there in UTF-16 code units.
pub fn to_units(text: &str, offset: usize) -> Position {
    let offset = boundary(text, offset);
    let line = text[..offset].matches('\n').count() as u32;
    let line_start = text[..offset].rfind('\n').map_or(0, |index| index + 1);
    let character = text[line_start..offset]
        .chars()
        .map(|c| c.len_utf16() as u32)
        .sum();
    Position { line, character }
}

/// The byte offset a protocol position names, snapping to the character boundary at or before it.
///
/// Answers `None` only when the line does not exist. A character count past the line's end clamps to
/// the line's end, and one inside a character (between an emoji's surrogate halves) snaps down.
pub fn to_offset(text: &str, position: Position) -> Option<usize> {
    let mut line_start = 0usize;
    if position.line != 0 {
        let mut line = 0u32;
        for (index, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                line += 1;
                if line == position.line {
                    line_start = index + 1;
                    break;
                }
            }
        }
        if line != position.line {
            return None;
        }
    }
    let mut units = 0u32;
    let mut offset = line_start;
    for character in text[line_start..].chars() {
        if character == '\n' {
            break;
        }
        let width = character.len_utf16() as u32;
        if units + width > position.character {
            break;
        }
        units += width;
        offset += character.len_utf8();
        if units == position.character {
            break;
        }
    }
    Some(offset)
}

/// The documents the protocol has open, each with its version.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Sync {
    documents: BTreeMap<String, Document>,
}

impl Sync {
    /// No documents open.
    pub fn new() -> Self {
        Self::default()
    }

    /// Opens a document at version 1 with the language id the editor would send.
    pub fn open_with(
        &mut self,
        uri: impl Into<String>,
        language_id: impl Into<String>,
        text: impl Into<String>,
    ) -> u64 {
        let uri = uri.into();
        self.documents.insert(
            uri.clone(),
            Document {
                uri,
                language_id: language_id.into(),
                text: text.into(),
                version: 1,
                dirty: false,
            },
        );
        1
    }

    /// Opens a document at version 1.
    pub fn open(&mut self, uri: impl Into<String>, text: impl Into<String>) -> u64 {
        self.open_with(uri, "plaintext", text)
    }

    /// A document, when it is open.
    pub fn document(&self, uri: &str) -> Option<&Document> {
        self.documents.get(uri)
    }

    /// Replaces a document's whole text (a full synchronisation), bumping its version.
    pub fn replace(&mut self, uri: &str, text: impl Into<String>) -> Result<u64, SyncError> {
        let document = self
            .documents
            .get_mut(uri)
            .ok_or_else(|| SyncError::NotOpen {
                uri: uri.to_owned(),
            })?;
        document.text = text.into();
        document.version += 1;
        document.dirty = true;
        Ok(document.version)
    }

    /// Applies an incremental change to the version it names.
    ///
    /// All of the change's ranges refer to that version, so they are applied by descending offset.
    pub fn change(
        &mut self,
        uri: &str,
        version: u64,
        changes: &[Change],
    ) -> Result<u64, SyncError> {
        let document = self
            .documents
            .get_mut(uri)
            .ok_or_else(|| SyncError::NotOpen {
                uri: uri.to_owned(),
            })?;
        if version != document.version {
            return Err(SyncError::StaleVersion {
                uri: uri.to_owned(),
                named: version,
                current: document.version,
            });
        }
        // Resolve every range against the text *before* the change.
        let text = document.text.clone();
        let mut resolved = Vec::with_capacity(changes.len());
        for change in changes {
            let start = to_offset(&text, change.range.start).ok_or_else(|| SyncError::NotOpen {
                uri: uri.to_owned(),
            })?;
            let end = to_offset(&text, change.range.end).ok_or_else(|| SyncError::NotOpen {
                uri: uri.to_owned(),
            })?;
            if end < start {
                return Err(SyncError::Backwards {
                    uri: uri.to_owned(),
                });
            }
            resolved.push((start, end, change.text.clone()));
        }
        let mut ranges: Vec<(usize, usize)> = resolved
            .iter()
            .map(|(start, end, _)| (*start, *end))
            .collect();
        ranges.sort_unstable();
        for window in ranges.windows(2) {
            if window[1].0 < window[0].1 {
                return Err(SyncError::Overlap {
                    uri: uri.to_owned(),
                });
            }
        }
        // Highest offset first, so an edit does not move the ones below it.
        resolved.sort_by_key(|(start, _, _)| std::cmp::Reverse(*start));
        for (start, end, replacement) in resolved {
            document.text.replace_range(start..end, &replacement);
        }
        document.version += 1;
        document.dirty = true;
        Ok(document.version)
    }

    /// Records a save. The version does not change; the document is no longer dirty.
    pub fn save(&mut self, uri: &str) -> Result<u64, SyncError> {
        let document = self
            .documents
            .get_mut(uri)
            .ok_or_else(|| SyncError::NotOpen {
                uri: uri.to_owned(),
            })?;
        document.dirty = false;
        Ok(document.version)
    }

    /// Applies a typed operation to the model and returns the notification the transport sends.
    ///
    /// The version rule lives here rather than in each caller: `open` starts at 1, every change is
    /// applied to the current version and increments it, and save/close leave it alone. A duplicate
    /// open and a close of a document that is not open are refusals with no notification.
    pub fn apply(&mut self, operation: Operation) -> Result<Notification, SyncError> {
        match operation {
            Operation::Open {
                uri,
                language_id,
                text,
            } => {
                if self.documents.contains_key(&uri) {
                    return Err(SyncError::AlreadyOpen { uri });
                }
                self.documents.insert(
                    uri.clone(),
                    Document {
                        uri: uri.clone(),
                        language_id: language_id.clone(),
                        text: text.clone(),
                        version: 1,
                        dirty: false,
                    },
                );
                Ok(Notification::Open {
                    uri,
                    language_id,
                    version: 1,
                    text,
                })
            }
            Operation::Change { uri, change } => {
                let current = self
                    .documents
                    .get(&uri)
                    .ok_or_else(|| SyncError::NotOpen { uri: uri.clone() })?
                    .version;
                let version = match &change {
                    DocumentChange::Full { text } => self.replace(&uri, text.clone())?,
                    DocumentChange::Incremental { changes } => {
                        self.change(&uri, current, changes)?
                    }
                };
                Ok(Notification::Change {
                    uri,
                    version,
                    change,
                })
            }
            Operation::Save { uri } => {
                let version = self.save(&uri)?;
                let text = self
                    .documents
                    .get(&uri)
                    .map(|document| document.text.clone())
                    .unwrap_or_default();
                Ok(Notification::Save { uri, version, text })
            }
            Operation::Close { uri } => {
                self.close(&uri)?;
                Ok(Notification::Close { uri })
            }
        }
    }

    /// Closes a document, releasing it.
    pub fn close(&mut self, uri: &str) -> Result<(), SyncError> {
        if self.documents.remove(uri).is_some() {
            Ok(())
        } else {
            Err(SyncError::NotOpen {
                uri: uri.to_owned(),
            })
        }
    }

    /// The open uris, in order.
    pub fn open_documents(&self) -> Vec<&str> {
        self.documents.keys().map(String::as_str).collect()
    }

    /// The sync state in one report.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!("{} document(s) open", self.documents.len())];
        for document in self.documents.values() {
            lines.push(format!("  {}", document.render()));
        }
        lines
    }
}

/// The character boundary at or before an offset.
fn boundary(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

#[cfg(test)]
mod tests {
    use super::{
        Change, DocumentChange, Notification, Operation, Position, Range, Sync, SyncError,
        to_offset, to_units,
    };

    #[test]
    fn chinese_and_emoji_are_counted_in_utf16_units() {
        // "a" is 1 byte / 1 unit, "中" is 3 bytes / 1 unit, "b" 1 unit, "🎉" 4 bytes / 2 units.
        let text = "a中b🎉c\n";
        assert_eq!(to_units(text, 0), Position::new(0, 0));
        assert_eq!(to_units(text, 1), Position::new(0, 1)); // after "a"
        assert_eq!(to_units(text, 4), Position::new(0, 2)); // after "中"
        assert_eq!(to_units(text, 5), Position::new(0, 3)); // after "b"
        assert_eq!(to_units(text, 9), Position::new(0, 5)); // after the emoji (2 units)
        assert_eq!(to_units(text, 10), Position::new(0, 6)); // after "c"
        // And back.
        assert_eq!(to_offset(text, Position::new(0, 0)), Some(0));
        assert_eq!(to_offset(text, Position::new(0, 2)), Some(4));
        assert_eq!(to_offset(text, Position::new(0, 5)), Some(9));
    }

    #[test]
    fn a_position_between_a_surrogate_pair_snaps_to_the_character() {
        let text = "🎉x";
        // The emoji is units 0..2; unit 1 is inside it, so it snaps to the emoji's start.
        assert_eq!(to_offset(text, Position::new(0, 1)), Some(0));
        assert_eq!(to_offset(text, Position::new(0, 2)), Some(4));
    }

    #[test]
    fn a_position_past_the_line_end_clamps_to_it() {
        let text = "ab\ncd\n";
        assert_eq!(to_offset(text, Position::new(0, 99)), Some(2)); // end of line 0
        assert_eq!(to_offset(text, Position::new(1, 1)), Some(4));
        // A line that does not exist has no offset.
        assert_eq!(to_offset(text, Position::new(9, 0)), None);
    }

    #[test]
    fn an_incremental_change_of_chinese_text_matches_a_splice() {
        let mut sync = Sync::new();
        let version = sync.open("file:///a.rs", "中a\n");
        assert_eq!(version, 1);
        // Replace the first character "中" with "字".
        let change = Change::new(Range::new(Position::new(0, 0), Position::new(0, 1)), "字");
        let version = sync.change("file:///a.rs", 1, &[change]).expect("a change");
        assert_eq!(version, 2);
        assert_eq!(sync.document("file:///a.rs").expect("open").text(), "字a\n");
        assert!(sync.document("file:///a.rs").expect("open").is_dirty());
    }

    #[test]
    fn several_edits_of_one_change_all_refer_to_the_version_before_it() {
        let mut sync = Sync::new();
        sync.open("file:///a.rs", "line1\nline2\nline3\n");
        // Two edits, both in the original: replace "1" on line 0 and "3" on line 2.
        let changes = vec![
            Change::new(Range::new(Position::new(0, 4), Position::new(0, 5)), "X"),
            Change::new(Range::new(Position::new(2, 4), Position::new(2, 5)), "Y"),
        ];
        sync.change("file:///a.rs", 1, &changes).expect("a change");
        assert_eq!(
            sync.document("file:///a.rs").expect("open").text(),
            "lineX\nline2\nlineY\n"
        );
    }

    #[test]
    fn a_stale_change_is_refused() {
        let mut sync = Sync::new();
        sync.open("file:///a.rs", "one\n");
        sync.change(
            "file:///a.rs",
            1,
            &[Change::new(
                Range::new(Position::new(0, 0), Position::new(0, 0)),
                "x",
            )],
        )
        .expect("first change");
        // A second change that still names version 1 is stale.
        match sync.change(
            "file:///a.rs",
            1,
            &[Change::new(
                Range::new(Position::new(0, 0), Position::new(0, 0)),
                "y",
            )],
        ) {
            Err(SyncError::StaleVersion { named, current, .. }) => {
                assert_eq!((named, current), (1, 2));
            }
            other => panic!("expected a stale refusal, got {other:?}"),
        }
        assert_eq!(
            sync.document("file:///a.rs").expect("open").text(),
            "xone\n"
        );
    }

    #[test]
    fn overlapping_edits_are_refused_and_change_nothing() {
        let mut sync = Sync::new();
        sync.open("file:///a.rs", "abcdef\n");
        let changes = vec![
            Change::new(Range::new(Position::new(0, 1), Position::new(0, 4)), "X"),
            Change::new(Range::new(Position::new(0, 2), Position::new(0, 3)), "Y"),
        ];
        assert_eq!(
            sync.change("file:///a.rs", 1, &changes),
            Err(SyncError::Overlap {
                uri: "file:///a.rs".to_owned()
            })
        );
        assert_eq!(
            sync.document("file:///a.rs").expect("open").text(),
            "abcdef\n"
        );
    }

    #[test]
    fn save_keeps_the_version_and_close_releases_the_document() {
        let mut sync = Sync::new();
        sync.open("file:///a.rs", "one\n");
        sync.change(
            "file:///a.rs",
            1,
            &[Change::new(
                Range::new(Position::new(0, 0), Position::new(0, 0)),
                "x",
            )],
        )
        .expect("a change");
        assert_eq!(sync.save("file:///a.rs").expect("save"), 2);
        assert!(!sync.document("file:///a.rs").expect("open").is_dirty());
        assert!(sync.close("file:///a.rs").is_ok());
        assert!(sync.document("file:///a.rs").is_none());
        assert_eq!(
            sync.close("file:///a.rs"),
            Err(SyncError::NotOpen {
                uri: "file:///a.rs".to_owned()
            })
        );
    }

    #[test]
    fn typed_operations_emit_one_notification_and_increment_the_version_per_edit() {
        let mut sync = Sync::new();
        let opened = sync
            .apply(Operation::Open {
                uri: "file:///a.rs".to_owned(),
                language_id: "rust".to_owned(),
                text: "one\n".to_owned(),
            })
            .expect("an open");
        match &opened {
            Notification::Open {
                uri,
                language_id,
                version,
                text,
            } => {
                assert_eq!(uri, "file:///a.rs");
                assert_eq!(language_id, "rust");
                assert_eq!(*version, 1);
                assert_eq!(text, "one\n");
            }
            other => panic!("expected didOpen, got {other:?}"),
        }

        let changed = sync
            .apply(Operation::Change {
                uri: "file:///a.rs".to_owned(),
                change: DocumentChange::Incremental {
                    changes: vec![Change::new(
                        Range::new(Position::new(0, 0), Position::new(0, 0)),
                        "x",
                    )],
                },
            })
            .expect("a change");
        assert_eq!(changed.version(), Some(2));
        assert!(matches!(
            changed,
            Notification::Change {
                version: 2,
                change: DocumentChange::Incremental { .. },
                ..
            }
        ));

        let replaced = sync
            .apply(Operation::Change {
                uri: "file:///a.rs".to_owned(),
                change: DocumentChange::Full {
                    text: "two\n".to_owned(),
                },
            })
            .expect("a full change");
        assert_eq!(replaced.version(), Some(3));
        assert!(matches!(
            replaced,
            Notification::Change {
                version: 3,
                change: DocumentChange::Full { .. },
                ..
            }
        ));
        assert_eq!(sync.document("file:///a.rs").expect("open").text(), "two\n");

        let saved = sync
            .apply(Operation::Save {
                uri: "file:///a.rs".to_owned(),
            })
            .expect("a save");
        assert_eq!(saved.version(), Some(3));
        assert!(matches!(saved, Notification::Save { .. }));

        let closed = sync
            .apply(Operation::Close {
                uri: "file:///a.rs".to_owned(),
            })
            .expect("a close");
        assert!(matches!(closed, Notification::Close { .. }));
        // A second close is a refusal and emits nothing.
        assert_eq!(
            sync.apply(Operation::Close {
                uri: "file:///a.rs".to_owned(),
            }),
            Err(SyncError::NotOpen {
                uri: "file:///a.rs".to_owned()
            })
        );
    }

    #[test]
    fn a_duplicate_open_is_refused_without_a_second_did_open() {
        let mut sync = Sync::new();
        sync.apply(Operation::Open {
            uri: "file:///a.rs".to_owned(),
            language_id: "rust".to_owned(),
            text: "one\n".to_owned(),
        })
        .expect("the first open");
        assert_eq!(
            sync.apply(Operation::Open {
                uri: "file:///a.rs".to_owned(),
                language_id: "rust".to_owned(),
                text: "two\n".to_owned(),
            }),
            Err(SyncError::AlreadyOpen {
                uri: "file:///a.rs".to_owned()
            })
        );
        assert_eq!(sync.document("file:///a.rs").expect("open").text(), "one\n");
    }

    #[test]
    fn a_typed_incremental_change_of_chinese_and_emoji_keeps_utf16_ranges() {
        let mut sync = Sync::new();
        sync.apply(Operation::Open {
            uri: "file:///a.rs".to_owned(),
            language_id: "rust".to_owned(),
            text: "a🎉中\n".to_owned(),
        })
        .expect("an open");
        // The emoji is UTF-16 units 1..3; replace it with one Chinese character (1 unit).
        let changed = sync
            .apply(Operation::Change {
                uri: "file:///a.rs".to_owned(),
                change: DocumentChange::Incremental {
                    changes: vec![Change::new(
                        Range::new(Position::new(0, 1), Position::new(0, 3)),
                        "字",
                    )],
                },
            })
            .expect("a change");
        assert!(matches!(
            changed,
            Notification::Change {
                version: 2,
                change: DocumentChange::Incremental { ref changes },
                ..
            } if changes[0].range == Range::new(Position::new(0, 1), Position::new(0, 3))
        ));
        assert_eq!(
            sync.document("file:///a.rs").expect("open").text(),
            "a字中\n"
        );
        // And the positions round-trip through the byte/UTF-16 conversion.
        assert_eq!(to_units("a字中\n", 4), Position::new(0, 2));
        assert_eq!(to_offset("a字中\n", Position::new(0, 2)), Some(4));
    }
}
