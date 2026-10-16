// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! A batch replace that can stop midway, be re-run, and be undone (task T08.05).
//!
//! The acceptance line is *"a batch that fails midway is recoverable and does not report all as
//! done"*. The danger is a batch that says "done" over a partial result, so the module makes the
//! partial state a first-class one:
//!
//! * **It applies one file at a time, and stops on the first problem.** [`Batch::run`] checks each
//!   file's **external baseline** (its version) and the [T06.06](crate::project::protection) guard
//!   before touching it; a mismatch or a refusal records a [`Failure`] and stops, leaving the earlier
//!   files applied and the later ones untouched.
//! * **"Complete" is proven, not assumed.** [`Batch::is_complete`] is true only when every planned
//!   file applied and no failure was recorded, so a partial batch cannot read as done.
//! * **The inverse is exact.** Each applied file keeps the [`OpenDocument`] it had before, and
//!   [`Batch::undo`] restores them in reverse — the recovery path, not a second guess.
//! * **A stopped batch can be resumed.** [`Batch::run`] continues from where it stopped, so once the
//!   external change is reconciled the rest applies, and the batch reaches completion honestly.

#![allow(dead_code)]

use std::collections::BTreeMap;

use crate::app::diagnostics::TextEdit;
use crate::app::workspace_edit::OpenDocument;
use crate::project::protection::{Write, WriteGuard};
use crate::project::text::TextStore;

/// One file's part of a batch.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BatchEdit {
    /// The file.
    pub uri: String,
    /// The version the edits were computed against.
    pub baseline: u64,
    /// The edits.
    pub edits: Vec<TextEdit>,
}

impl BatchEdit {
    /// A file's edits against a baseline version.
    pub fn new(uri: impl Into<String>, baseline: u64, edits: Vec<TextEdit>) -> Self {
        Self {
            uri: uri.into(),
            baseline,
            edits,
        }
    }
}

/// Why a batch stopped.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Failure {
    /// A file the batch names is not open.
    NotOpen {
        /// The file.
        uri: String,
    },
    /// A file changed on disk since the batch was computed.
    ExternalChange {
        /// The file.
        uri: String,
        /// The version the batch was computed against.
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

impl Failure {
    /// The failure in one sentence.
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

/// A file the batch applied, with what it was before.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Applied {
    /// The file.
    pub uri: String,
    /// The document as it was before the batch touched it.
    pub previous: OpenDocument,
}

/// The batch.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Batch {
    plan: Vec<BatchEdit>,
    applied: Vec<Applied>,
    failure: Option<Failure>,
    next: usize,
}

impl Batch {
    /// A batch over a plan.
    pub fn new(plan: Vec<BatchEdit>) -> Self {
        Self {
            plan,
            applied: Vec::new(),
            failure: None,
            next: 0,
        }
    }

    /// How many files the plan has.
    pub fn len(&self) -> usize {
        self.plan.len()
    }

    /// Whether the plan is empty.
    pub fn is_empty(&self) -> bool {
        self.plan.is_empty()
    }

    /// The files that were applied.
    pub fn applied(&self) -> &[Applied] {
        &self.applied
    }

    /// The failure that stopped it, when one did.
    pub fn failure(&self) -> Option<&Failure> {
        self.failure.as_ref()
    }

    /// How many files are still unapplied.
    pub fn remaining(&self) -> usize {
        self.plan.len().saturating_sub(self.next)
    }

    /// Whether every file applied and nothing failed.
    pub fn is_complete(&self) -> bool {
        self.failure.is_none() && self.next >= self.plan.len()
    }

    /// Runs (or resumes) the batch, applying files until a problem stops it.
    pub fn run(
        &mut self,
        documents: &mut BTreeMap<String, OpenDocument>,
        guards: &BTreeMap<String, WriteGuard>,
    ) {
        while self.next < self.plan.len() {
            let edit = self.plan[self.next].clone();
            match apply_one(&edit, documents, guards) {
                Ok(previous) => {
                    self.applied.push(Applied {
                        uri: edit.uri.clone(),
                        previous,
                    });
                    self.next += 1;
                }
                Err(failure) => {
                    self.failure = Some(failure);
                    return;
                }
            }
        }
        self.failure = None;
    }

    /// Restores every applied file to what it was, in reverse.
    pub fn undo(&mut self, documents: &mut BTreeMap<String, OpenDocument>) -> usize {
        let mut restored = 0;
        while let Some(applied) = self.applied.pop() {
            documents.insert(applied.uri.clone(), applied.previous.clone());
            restored += 1;
        }
        // After an undo, the batch is back at the start and can be re-run.
        self.next = 0;
        self.failure = None;
        restored
    }

    /// The batch in one report.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "batch: {}/{} applied, {}{}",
            self.applied.len(),
            self.plan.len(),
            if self.is_complete() {
                "complete".to_owned()
            } else {
                format!("{} remaining", self.remaining())
            },
            match &self.failure {
                Some(failure) => format!(", stopped: {}", failure.message()),
                None => String::new(),
            }
        )];
        for applied in &self.applied {
            lines.push(format!("  applied {}", applied.uri));
        }
        lines
    }
}

/// Applies one file's edits, or says why it could not.
fn apply_one(
    edit: &BatchEdit,
    documents: &mut BTreeMap<String, OpenDocument>,
    guards: &BTreeMap<String, WriteGuard>,
) -> Result<OpenDocument, Failure> {
    let Some(document) = documents.get(&edit.uri) else {
        return Err(Failure::NotOpen {
            uri: edit.uri.clone(),
        });
    };
    if document.version != edit.baseline {
        return Err(Failure::ExternalChange {
            uri: edit.uri.clone(),
            baseline: edit.baseline,
            current: document.version,
        });
    }
    if let Some(guard) = guards.get(&edit.uri) {
        let writes: Vec<Write> = edit.edits.iter().map(TextEdit::write).collect();
        if let Err(refusal) = guard.check(crate::project::protection::WriteSource::Replace, &writes)
        {
            return Err(Failure::Protected {
                uri: edit.uri.clone(),
                reason: refusal.message(),
            });
        }
    }
    let previous = document.clone();
    let mut store = TextStore::new(previous.text.clone());
    let edits: Vec<(usize, usize, String)> = edit
        .edits
        .iter()
        .map(|edit| (edit.at, edit.remove, edit.insert.clone()))
        .collect();
    if let Err(error) = store.edit_many(edits) {
        return Err(Failure::Invalid {
            uri: edit.uri.clone(),
            reason: error.message(),
        });
    }
    let document = documents
        .get_mut(&edit.uri)
        .expect("a checked document is open");
    document.text = store.text();
    document.version += 1;
    Ok(previous)
}

#[cfg(test)]
mod tests {
    use super::{Batch, BatchEdit, Failure};
    use crate::app::diagnostics::TextEdit;
    use crate::app::workspace_edit::OpenDocument;
    use crate::project::conflict::Writability;
    use crate::project::protection::{ProtectedRegion, WriteGuard};
    use std::collections::BTreeMap;

    fn documents() -> BTreeMap<String, OpenDocument> {
        let mut documents = BTreeMap::new();
        documents.insert("a.rs".to_owned(), OpenDocument::new("aaa", 1));
        documents.insert("b.rs".to_owned(), OpenDocument::new("bbb", 1));
        documents.insert("c.rs".to_owned(), OpenDocument::new("ccc", 1));
        documents
    }

    fn plan() -> Vec<BatchEdit> {
        vec![
            BatchEdit::new("a.rs", 1, vec![TextEdit::new(0, 1, "X")]),
            BatchEdit::new("b.rs", 1, vec![TextEdit::new(0, 1, "Y")]),
            BatchEdit::new("c.rs", 1, vec![TextEdit::new(0, 1, "Z")]),
        ]
    }

    #[test]
    fn a_batch_with_no_problem_completes() {
        let mut documents = documents();
        let mut batch = Batch::new(plan());
        batch.run(&mut documents, &BTreeMap::new());
        assert!(batch.is_complete());
        assert_eq!(batch.applied().len(), 3);
        assert_eq!(documents["c.rs"].text, "Zcc");
    }

    #[test]
    fn a_failure_midway_is_partial_and_recoverable() {
        let mut documents = documents();
        // b.rs moved on, so the batch stops after a.rs.
        documents.insert("b.rs".to_owned(), OpenDocument::new("bbb", 2));
        let mut batch = Batch::new(plan());
        batch.run(&mut documents, &BTreeMap::new());
        assert!(!batch.is_complete());
        assert!(matches!(
            batch.failure(),
            Some(Failure::ExternalChange { .. })
        ));
        assert_eq!(batch.applied().len(), 1);
        assert_eq!(batch.remaining(), 2);
        assert_eq!(documents["a.rs"].text, "Xaa");
        // The later files were not touched.
        assert_eq!(documents["c.rs"].text, "ccc");
        // The inverse restores a.rs exactly.
        assert_eq!(batch.undo(&mut documents), 1);
        assert_eq!(documents["a.rs"].text, "aaa");
        assert_eq!(documents["a.rs"].version, 1);
    }

    #[test]
    fn a_stopped_batch_resumes_after_the_conflict_is_reconciled() {
        let mut documents = documents();
        documents.insert("b.rs".to_owned(), OpenDocument::new("bbb", 2));
        let mut batch = Batch::new(plan());
        batch.run(&mut documents, &BTreeMap::new());
        assert!(!batch.is_complete());
        // Reconcile: the file is back to the version the batch expected.
        documents.insert("b.rs".to_owned(), OpenDocument::new("bbb", 1));
        batch.run(&mut documents, &BTreeMap::new());
        assert!(batch.is_complete());
        assert_eq!(documents["a.rs"].text, "Xaa");
        assert_eq!(documents["b.rs"].text, "Ybb");
        assert_eq!(documents["c.rs"].text, "Zcc");
    }

    #[test]
    fn a_protected_region_stops_the_batch() {
        let mut documents = documents();
        let mut guards = BTreeMap::new();
        guards.insert(
            "b.rs".to_owned(),
            WriteGuard::new(vec![ProtectedRegion::new(0, 2)], Writability::Writable),
        );
        let mut batch = Batch::new(plan());
        batch.run(&mut documents, &guards);
        assert!(!batch.is_complete());
        assert!(matches!(batch.failure(), Some(Failure::Protected { .. })));
        assert_eq!(documents["b.rs"].text, "bbb");
    }

    #[test]
    fn a_partial_batch_never_reports_complete() {
        let mut documents = documents();
        documents.insert("a.rs".to_owned(), OpenDocument::new("aaa", 9));
        let mut batch = Batch::new(plan());
        batch.run(&mut documents, &BTreeMap::new());
        assert!(!batch.is_complete());
        assert!(batch.render()[0].contains("stopped"));
    }
}
