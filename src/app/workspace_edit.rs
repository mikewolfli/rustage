// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! A cross-file edit (format, rename) reviewed and committed as one transaction (task T07.06).
//!
//! The acceptance line is *"a protected region or an external conflict blocks a half-commit"*. A
//! workspace edit spans several files, so the danger is a commit that changed some of them and then
//! stopped; the module makes that impossible by **reviewing everything before it changes anything**:
//!
//! * **A review is a list of blocks.** [`review`] walks every file's edits and records every reason it
//!   cannot be applied — a region a generator owns ([`Block::Protected`]), a file that changed on disk
//!   since the edit was computed ([`Block::ExternalChange`]), a file that is not open, or edits that
//!   cannot be applied to the text at all. A ready review has an empty list.
//! * **A commit applies all of it or none of it.** [`commit`] reviews first and returns the blocks
//!   unchanged when there are any, so a partially-changed workspace is not a state the module can
//!   reach. When it applies, it applies each file through the one edit path of
//!   [`TextStore::edit_many`](crate::project::text::TextStore::edit_many).
//! * **The guard is [T06.06](crate::project::protection)'s, through [`WriteSource::Format`].** A
//!   formatter or a rename is a write, so it is refused by the same rule every other write is, and the
//!   refusal names the region.
//! * **Freshness is the file's version.** The edit carries the version each file had when the edit
//!   was computed; a file that moved past it is an [`Block::ExternalChange`], so a stale edit cannot
//!   overwrite a file someone else changed.

#![allow(dead_code)]

use std::collections::BTreeMap;

use crate::app::diagnostics::TextEdit;
use crate::project::protection::{Write, WriteGuard, WriteSource};
use crate::project::text::TextStore;

/// An open document the transaction can change: its text and the version it is at.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct OpenDocument {
    /// The text.
    pub text: String,
    /// The version.
    pub version: u64,
}

impl OpenDocument {
    /// A document at a version.
    pub fn new(text: impl Into<String>, version: u64) -> Self {
        Self {
            text: text.into(),
            version,
        }
    }
}

/// One file's part of a workspace edit.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FileEdit {
    /// The file.
    pub uri: String,
    /// The version the edits were computed against.
    pub baseline: u64,
    /// The edits.
    pub edits: Vec<TextEdit>,
}

impl FileEdit {
    /// A file's edits against a baseline version.
    pub fn new(uri: impl Into<String>, baseline: u64, edits: Vec<TextEdit>) -> Self {
        Self {
            uri: uri.into(),
            baseline,
            edits,
        }
    }
}

/// A workspace edit: a labelled set of files to change.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct WorkspaceEdit {
    /// What the edit is, for the review.
    pub label: String,
    /// The files.
    pub files: Vec<FileEdit>,
}

impl WorkspaceEdit {
    /// An empty edit with a label.
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            files: Vec::new(),
        }
    }

    /// Adds a file's edits.
    pub fn add(&mut self, file: FileEdit) {
        self.files.push(file);
    }
}

/// Why a workspace edit cannot be committed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Block {
    /// A file the edit names is not open.
    NotOpen {
        /// The file.
        uri: String,
    },
    /// A file changed on disk since the edit was computed.
    ExternalChange {
        /// The file.
        uri: String,
        /// The version the edit was computed against.
        baseline: u64,
        /// The version the file is at.
        current: u64,
    },
    /// An edit would change a region a generator owns.
    Protected {
        /// The file.
        uri: String,
        /// The refusal, in words.
        reason: String,
    },
    /// The edits cannot be applied to the text.
    Invalid {
        /// The file.
        uri: String,
        /// Why, in words.
        reason: String,
    },
}

impl Block {
    /// The block in one sentence.
    pub fn message(&self) -> String {
        match self {
            Self::NotOpen { uri } => format!("{uri} is not open"),
            Self::ExternalChange {
                uri,
                baseline,
                current,
            } => format!("{uri} changed outside the editor: version {current}, not {baseline}"),
            Self::Protected { uri, reason } => format!("{uri} has a protected region: {reason}"),
            Self::Invalid { uri, reason } => format!("{uri} cannot take the edits: {reason}"),
        }
    }
}

/// The outcome of reviewing a workspace edit.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Review {
    /// The edit's label.
    pub label: String,
    /// Every reason the edit cannot be committed; empty when it can.
    pub blocks: Vec<Block>,
}

impl Review {
    /// Whether the edit can be committed.
    pub fn is_ready(&self) -> bool {
        self.blocks.is_empty()
    }

    /// The review in one report.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "review {}: {}",
            self.label,
            if self.is_ready() {
                "ready".to_owned()
            } else {
                format!("{} block(s)", self.blocks.len())
            }
        )];
        for block in &self.blocks {
            lines.push(format!("  blocked: {}", block.message()));
        }
        lines
    }
}

/// One file that a commit changed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Applied {
    /// The file.
    pub uri: String,
    /// The version it moved to.
    pub version: u64,
}

impl Applied {
    /// The file in one line.
    pub fn render(&self) -> String {
        format!("{}: version {}", self.uri, self.version)
    }
}

/// Reviews a workspace edit against the open documents and their guards.
pub fn review(
    edit: &WorkspaceEdit,
    documents: &BTreeMap<String, OpenDocument>,
    guards: &BTreeMap<String, WriteGuard>,
) -> Review {
    let mut blocks = Vec::new();
    for file in &edit.files {
        let Some(document) = documents.get(&file.uri) else {
            blocks.push(Block::NotOpen {
                uri: file.uri.clone(),
            });
            continue;
        };
        if document.version != file.baseline {
            blocks.push(Block::ExternalChange {
                uri: file.uri.clone(),
                baseline: file.baseline,
                current: document.version,
            });
            continue;
        }
        if let Some(guard) = guards.get(&file.uri) {
            let writes: Vec<Write> = file.edits.iter().map(TextEdit::write).collect();
            if let Err(refusal) = guard.check(WriteSource::Format, &writes) {
                blocks.push(Block::Protected {
                    uri: file.uri.clone(),
                    reason: refusal.message(),
                });
                continue;
            }
        }
        // The edits themselves must apply, or the file is invalid rather than merely blocked.
        let mut store = TextStore::new(document.text.clone());
        let edits: Vec<(usize, usize, String)> = file
            .edits
            .iter()
            .map(|edit| (edit.at, edit.remove, edit.insert.clone()))
            .collect();
        if let Err(error) = store.edit_many(edits) {
            blocks.push(Block::Invalid {
                uri: file.uri.clone(),
                reason: error.message(),
            });
        }
    }
    Review {
        label: edit.label.clone(),
        blocks,
    }
}

/// Reviews and commits a workspace edit. On any block, nothing is applied.
pub fn commit(
    edit: &WorkspaceEdit,
    documents: &mut BTreeMap<String, OpenDocument>,
    guards: &BTreeMap<String, WriteGuard>,
) -> Result<Vec<Applied>, Vec<Block>> {
    let review = review(edit, documents, guards);
    if !review.blocks.is_empty() {
        return Err(review.blocks);
    }
    let mut applied = Vec::new();
    for file in &edit.files {
        let document = documents
            .get_mut(&file.uri)
            .expect("a reviewed file is open");
        let mut store = TextStore::new(document.text.clone());
        let edits: Vec<(usize, usize, String)> = file
            .edits
            .iter()
            .map(|edit| (edit.at, edit.remove, edit.insert.clone()))
            .collect();
        store
            .edit_many(edits)
            .expect("a reviewed file takes its edits");
        document.text = store.text();
        document.version += 1;
        applied.push(Applied {
            uri: file.uri.clone(),
            version: document.version,
        });
    }
    Ok(applied)
}

#[cfg(test)]
mod tests {
    use super::{Block, FileEdit, OpenDocument, WorkspaceEdit, commit, review};
    use crate::app::diagnostics::TextEdit;
    use crate::project::conflict::Writability;
    use crate::project::protection::{ProtectedRegion, WriteGuard};
    use std::collections::BTreeMap;

    fn documents() -> BTreeMap<String, OpenDocument> {
        let mut documents = BTreeMap::new();
        documents.insert("a.rs".to_owned(), OpenDocument::new("aaa", 1));
        documents.insert("b.rs".to_owned(), OpenDocument::new("bbb", 1));
        documents
    }

    #[test]
    fn a_ready_edit_commits_every_file() {
        let mut edit = WorkspaceEdit::new("rename");
        edit.add(FileEdit::new("a.rs", 1, vec![TextEdit::new(0, 1, "X")]));
        edit.add(FileEdit::new("b.rs", 1, vec![TextEdit::new(0, 1, "Y")]));
        let mut documents = documents();
        let applied = commit(&edit, &mut documents, &BTreeMap::new()).expect("commits");
        assert_eq!(applied.len(), 2);
        assert_eq!(documents["a.rs"].text, "Xaa");
        assert_eq!(documents["b.rs"].text, "Ybb");
        assert_eq!(documents["a.rs"].version, 2);
    }

    #[test]
    fn a_protected_region_blocks_a_half_commit() {
        let mut edit = WorkspaceEdit::new("format");
        // The first file is fine; the second reaches into a protected region.
        edit.add(FileEdit::new("a.rs", 1, vec![TextEdit::new(0, 1, "X")]));
        edit.add(FileEdit::new("b.rs", 1, vec![TextEdit::new(1, 1, "Y")]));
        let mut guards = BTreeMap::new();
        guards.insert(
            "b.rs".to_owned(),
            WriteGuard::new(vec![ProtectedRegion::new(1, 3)], Writability::Writable),
        );
        let mut documents = documents();
        let blocks = commit(&edit, &mut documents, &guards).expect_err("blocked");
        assert!(matches!(blocks.as_slice(), [Block::Protected { .. }]));
        // Nothing was applied: not even the good file.
        assert_eq!(documents["a.rs"].text, "aaa");
        assert_eq!(documents["a.rs"].version, 1);
        assert_eq!(documents["b.rs"].text, "bbb");
    }

    #[test]
    fn an_external_change_blocks_the_commit() {
        let mut edit = WorkspaceEdit::new("rename");
        edit.add(FileEdit::new("a.rs", 1, vec![TextEdit::new(0, 1, "X")]));
        let mut documents = documents();
        // The file moved on without the editor.
        documents.insert("a.rs".to_owned(), OpenDocument::new("changed", 2));
        let blocks = commit(&edit, &mut documents, &BTreeMap::new()).expect_err("blocked");
        assert_eq!(
            blocks,
            vec![Block::ExternalChange {
                uri: "a.rs".to_owned(),
                baseline: 1,
                current: 2
            }]
        );
        assert_eq!(documents["a.rs"].text, "changed");
    }

    #[test]
    fn a_file_that_is_not_open_blocks_the_commit() {
        let mut edit = WorkspaceEdit::new("rename");
        edit.add(FileEdit::new("gone.rs", 1, vec![TextEdit::new(0, 1, "X")]));
        let mut documents = documents();
        let blocks = commit(&edit, &mut documents, &BTreeMap::new()).expect_err("blocked");
        assert_eq!(
            blocks,
            vec![Block::NotOpen {
                uri: "gone.rs".to_owned()
            }]
        );
        // The other files were untouched.
        assert_eq!(documents["a.rs"].text, "aaa");
    }

    #[test]
    fn overlapping_edits_are_invalid_not_silently_merged() {
        let mut edit = WorkspaceEdit::new("bad");
        edit.add(FileEdit::new(
            "a.rs",
            1,
            vec![TextEdit::new(0, 2, "X"), TextEdit::new(1, 1, "Y")],
        ));
        let mut documents = documents();
        let blocks = commit(&edit, &mut documents, &BTreeMap::new()).expect_err("blocked");
        assert!(matches!(blocks.as_slice(), [Block::Invalid { .. }]));
        assert_eq!(documents["a.rs"].text, "aaa");
    }

    #[test]
    fn a_review_of_a_ready_edit_has_no_blocks() {
        let mut edit = WorkspaceEdit::new("rename");
        edit.add(FileEdit::new("a.rs", 1, vec![TextEdit::new(0, 1, "X")]));
        let review = review(&edit, &documents(), &BTreeMap::new());
        assert!(review.is_ready());
        assert!(review.render()[0].contains("ready"));
    }
}
