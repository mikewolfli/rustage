// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! A batch replace that can stop midway, be re-run, and be undone (task T08.05).
//!
//! The acceptance line is *"a batch that fails midway is recoverable and does not report all as
//! done"*. The danger is a batch that says "done" over a partial result, so the module makes the
//! partial state a first-class one:
//!
//! * **It applies one file at a time, and stops on the first problem.** [`Batch::run`] checks each
//!   file's **external baseline** (its version) and the [T06.06](crate::project::text::protection) guard
//!   before touching it; a mismatch or a refusal records a [`Failure`] and stops, leaving the earlier
//!   files applied and the later ones untouched.
//! * **"Complete" is proven, not assumed.** [`Batch::is_complete`] is true only when every planned
//!   file applied and no failure was recorded, so a partial batch cannot read as done.
//! * **The inverse is exact.** Each applied file keeps the [`OpenDocument`] it had before, and
//!   [`Batch::undo`] restores them in reverse — the recovery path, not a second guess.
//! * **A stopped batch can be resumed.** [`Batch::run`] continues from where it stopped, so once the
//!   external change is reconciled the rest applies, and the batch reaches completion honestly.
//! * **Applied is not the same as written.** [`Batch::commit`] writes each applied file through a
//!   temporary beside its destination and renames it into place. A URI that is absolute or walks out
//!   with `..` is refused before anything is written; a failure after some renames keeps those files
//!   on disk and names them in the report, so the batch never reads as done over a partial write.
//!   [`Batch::rollback`] reuses the same staged writer to put the previous text back.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use crate::app::language::diagnostics::TextEdit;
use crate::app::language::workspace_edit::OpenDocument;
use crate::project::text::piece_table::TextStore;
use crate::project::text::protection::{Write, WriteGuard};

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

/// A file the batch applied, with what it was before and after.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Applied {
    /// The file.
    pub uri: String,
    /// The document as it was before the batch touched it.
    pub previous: OpenDocument,
    /// The document as the batch left it; this is what [`Batch::commit`] writes.
    pub current: OpenDocument,
}

/// Why writing a set of files to disk stopped.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum DiskFailure {
    /// The URI is absolute or walks out of the root, so it is not a file the root owns.
    EscapesRoot {
        /// The file.
        uri: String,
    },
    /// The new text could not be put into a temporary beside its destination.
    NotStaged {
        /// The file.
        uri: String,
        /// Why, in words.
        reason: String,
    },
    /// The staged text could not be renamed over its destination.
    NotRenamed {
        /// The file.
        uri: String,
        /// Why, in words.
        reason: String,
    },
    /// A rollback could not remove a file the batch created.
    NotRemoved {
        /// The file.
        uri: String,
        /// Why, in words.
        reason: String,
    },
}

impl DiskFailure {
    /// The failure in one sentence.
    pub fn message(&self) -> String {
        match self {
            Self::EscapesRoot { uri } => format!("{uri} is not inside the commit root"),
            Self::NotStaged { uri, reason } => format!("{uri} could not be staged: {reason}"),
            Self::NotRenamed { uri, reason } => {
                format!("{uri} could not be moved into place: {reason}")
            }
            Self::NotRemoved { uri, reason } => {
                format!("{uri} could not be removed for the rollback: {reason}")
            }
        }
    }
}

/// A file that reached disk, with the text it had before the batch wrote it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Written {
    /// The file.
    pub uri: String,
    /// Where it was written.
    pub path: PathBuf,
    /// The text the file had before this write, when it existed and could be read.
    pub previous: Option<String>,
}

/// What writing a set of files to disk did.
///
/// It is used for both [`Batch::commit`] and [`Batch::rollback`]: `written` is what reached disk,
/// `pending` is what did not, and `failure` says why the set stopped. A partial set is exactly as
/// traceable as a complete one, which is what lets recovery name the files it still has to handle.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct CommitReport {
    written: Vec<Written>,
    pending: Vec<String>,
    failure: Option<DiskFailure>,
}

impl CommitReport {
    /// The files that reached disk.
    pub fn written(&self) -> &[Written] {
        &self.written
    }

    /// The files that were planned but did not reach disk.
    pub fn pending(&self) -> &[String] {
        &self.pending
    }

    /// The failure that stopped the write, when one did.
    pub fn failure(&self) -> Option<&DiskFailure> {
        self.failure.as_ref()
    }

    /// Whether every planned file reached disk and nothing failed.
    pub fn is_complete(&self) -> bool {
        self.failure.is_none() && self.pending.is_empty()
    }

    /// The report in lines.
    pub fn render(&self) -> Vec<String> {
        self.render_for("commit")
    }

    /// The report in lines, headed by the verb that produced it.
    pub fn render_for(&self, verb: &str) -> Vec<String> {
        let total = self.written.len() + self.pending.len();
        let state = match &self.failure {
            Some(failure) => format!(", stopped: {}", failure.message()),
            None if self.pending.is_empty() => ", complete".to_owned(),
            None => format!(", {} not written", self.pending.len()),
        };
        let mut lines = vec![format!(
            "{verb}: {}/{total} written{state}",
            self.written.len()
        )];
        for written in &self.written {
            lines.push(format!("  wrote {}", written.uri));
        }
        for uri in &self.pending {
            lines.push(format!("  not written {uri}"));
        }
        lines
    }
}

/// The batch.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Batch {
    plan: Vec<BatchEdit>,
    applied: Vec<Applied>,
    committed: Vec<Written>,
    failure: Option<Failure>,
    next: usize,
}

impl Batch {
    /// A batch over a plan.
    pub fn new(plan: Vec<BatchEdit>) -> Self {
        Self {
            plan,
            applied: Vec::new(),
            committed: Vec::new(),
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

    /// The files that reached disk.
    pub fn committed(&self) -> &[Written] {
        &self.committed
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
                Ok((previous, current)) => {
                    self.applied.push(Applied {
                        uri: edit.uri.clone(),
                        previous,
                        current,
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

    /// Writes every applied file that is not yet on disk.
    ///
    /// Every destination is checked against `root` first: an absolute URI or one with a `..`
    /// component is refused before any temporary is created, so a batch cannot write outside the
    /// project. The texts are then staged beside their destinations and only renamed once every
    /// stage succeeded; a failure while staging leaves every file as it was. A failure while
    /// renaming keeps the files that already landed — they are returned in the report and retained
    /// on the batch — and names every file that did not.
    pub fn commit(&mut self, root: &Path) -> CommitReport {
        let already: BTreeSet<&str> = self
            .committed
            .iter()
            .map(|written| written.uri.as_str())
            .collect();
        let pending: Vec<Applied> = self
            .applied
            .iter()
            .filter(|applied| !already.contains(applied.uri.as_str()))
            .cloned()
            .collect();
        let names: Vec<String> = pending.iter().map(|applied| applied.uri.clone()).collect();

        // Refuse the whole set before writing anything: a batch that would escape the root is not a
        // batch this root can take, and writing the other files first would hide that.
        if let Some(escaping) = pending
            .iter()
            .find(|applied| relative_within(&applied.uri).is_none())
        {
            return CommitReport {
                written: Vec::new(),
                pending: names,
                failure: Some(DiskFailure::EscapesRoot {
                    uri: escaping.uri.clone(),
                }),
            };
        }

        let mut staged: Vec<Staged> = Vec::new();
        for applied in &pending {
            let relative = relative_within(&applied.uri).expect("checked above");
            let target = root.join(relative);
            if let Some(parent) = target.parent()
                && let Err(error) = std::fs::create_dir_all(parent)
            {
                cleanup(&staged);
                return CommitReport {
                    written: Vec::new(),
                    pending: names,
                    failure: Some(DiskFailure::NotStaged {
                        uri: applied.uri.clone(),
                        reason: error.to_string(),
                    }),
                };
            }
            let previous = std::fs::read_to_string(&target).ok();
            let temporary = temporary_beside(&target);
            if let Err(error) = std::fs::write(&temporary, &applied.current.text) {
                let _ = std::fs::remove_file(&temporary);
                cleanup(&staged);
                return CommitReport {
                    written: Vec::new(),
                    pending: names,
                    failure: Some(DiskFailure::NotStaged {
                        uri: applied.uri.clone(),
                        reason: error.to_string(),
                    }),
                };
            }
            staged.push(Staged {
                uri: applied.uri.clone(),
                temporary,
                target,
                previous,
            });
        }

        // Everything is staged; the renames are the only part a reader can notice.
        let mut written = Vec::new();
        let mut rest = staged.into_iter();
        while let Some(staged) = rest.next() {
            if let Err(error) = std::fs::rename(&staged.temporary, &staged.target) {
                let _ = std::fs::remove_file(&staged.temporary);
                cleanup(&rest.collect::<Vec<_>>());
                self.committed.extend(written.clone());
                let landed: BTreeSet<&str> =
                    written.iter().map(|entry| entry.uri.as_str()).collect();
                let pending = names
                    .into_iter()
                    .filter(|uri| !landed.contains(uri.as_str()))
                    .collect();
                return CommitReport {
                    written,
                    pending,
                    failure: Some(DiskFailure::NotRenamed {
                        uri: staged.uri,
                        reason: error.to_string(),
                    }),
                };
            }
            written.push(Written {
                uri: staged.uri,
                path: staged.target,
                previous: staged.previous,
            });
        }
        self.committed.extend(written.clone());
        CommitReport {
            written,
            pending: Vec::new(),
            failure: None,
        }
    }

    /// Puts the previous text of every committed file back, through the same staged writer.
    ///
    /// A file the batch created has no previous text, so it is removed instead. Files whose restore
    /// already landed are dropped from the batch's committed list; a failed restore leaves the rest
    /// recorded, so a rollback can be resumed without guessing.
    pub fn rollback(&mut self, root: &Path) -> CommitReport {
        let names: Vec<String> = self
            .committed
            .iter()
            .map(|written| written.uri.clone())
            .collect();
        if let Some(escaping) = self
            .committed
            .iter()
            .find(|written| relative_within(&written.uri).is_none())
        {
            return CommitReport {
                written: Vec::new(),
                pending: names,
                failure: Some(DiskFailure::EscapesRoot {
                    uri: escaping.uri.clone(),
                }),
            };
        }

        let mut staged: Vec<Staged> = Vec::new();
        let mut removals: Vec<(String, PathBuf)> = Vec::new();
        for written in &self.committed {
            let relative = relative_within(&written.uri).expect("checked above");
            match &written.previous {
                Some(text) => {
                    let target = root.join(relative);
                    let temporary = temporary_beside(&target);
                    if let Err(error) = std::fs::write(&temporary, text) {
                        let _ = std::fs::remove_file(&temporary);
                        cleanup(&staged);
                        return CommitReport {
                            written: Vec::new(),
                            pending: names,
                            failure: Some(DiskFailure::NotStaged {
                                uri: written.uri.clone(),
                                reason: error.to_string(),
                            }),
                        };
                    }
                    staged.push(Staged {
                        uri: written.uri.clone(),
                        temporary,
                        target,
                        previous: written.previous.clone(),
                    });
                }
                None => removals.push((written.uri.clone(), root.join(relative))),
            }
        }

        let mut restored = Vec::new();
        let mut rest = staged.into_iter();
        while let Some(staged) = rest.next() {
            if let Err(error) = std::fs::rename(&staged.temporary, &staged.target) {
                let _ = std::fs::remove_file(&staged.temporary);
                cleanup(&rest.collect::<Vec<_>>());
                self.retain_unrestored(&restored);
                return CommitReport {
                    written: restored.clone(),
                    pending: names_after(&names, &restored),
                    failure: Some(DiskFailure::NotRenamed {
                        uri: staged.uri,
                        reason: error.to_string(),
                    }),
                };
            }
            restored.push(Written {
                uri: staged.uri,
                path: staged.target,
                previous: staged.previous,
            });
        }
        for (uri, target) in removals {
            if let Err(error) = std::fs::remove_file(&target) {
                self.retain_unrestored(&restored);
                return CommitReport {
                    written: restored.clone(),
                    pending: names_after(&names, &restored)
                        .into_iter()
                        .filter(|name| name != &uri)
                        .chain(std::iter::once(uri.clone()))
                        .collect(),
                    failure: Some(DiskFailure::NotRemoved {
                        uri,
                        reason: error.to_string(),
                    }),
                };
            }
            restored.push(Written {
                uri,
                path: target,
                previous: None,
            });
        }
        self.retain_unrestored(&restored);
        CommitReport {
            written: restored,
            pending: Vec::new(),
            failure: None,
        }
    }

    /// Drops the files a rollback restored, so the rest stay traceable.
    fn retain_unrestored(&mut self, restored: &[Written]) {
        let done: BTreeSet<&str> = restored.iter().map(|entry| entry.uri.as_str()).collect();
        self.committed
            .retain(|written| !done.contains(written.uri.as_str()));
    }

    /// Restores every applied file to what it was, in reverse.
    ///
    /// This is the in-memory inverse only; [`Batch::rollback`] is the disk counterpart.
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
        // Applying and writing are separate phases, so the report says when they disagree.
        if self.committed.len() < self.applied.len() {
            lines.push(format!(
                "  on disk {}/{} applied, {} not written",
                self.committed.len(),
                self.applied.len(),
                self.applied.len() - self.committed.len()
            ));
        }
        for written in &self.committed {
            lines.push(format!("  on disk {}", written.uri));
        }
        lines
    }
}

/// A file staged beside its destination, waiting for the rename.
struct Staged {
    uri: String,
    temporary: PathBuf,
    target: PathBuf,
    previous: Option<String>,
}

/// Whether a URI names a file inside the root, by refusing everything a relative path may not say.
fn relative_within(uri: &str) -> Option<&Path> {
    let path = Path::new(uri);
    if path.as_os_str().is_empty() {
        return None;
    }
    let mut normal = 0;
    for component in path.components() {
        match component {
            Component::Normal(_) => normal += 1,
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    // `.` and `./` name the root itself, which is not a file this root owns.
    (normal > 0).then_some(path)
}

/// The temporary a staged write goes through, beside its destination and hidden like a save.
fn temporary_beside(target: &Path) -> PathBuf {
    match target.file_name() {
        Some(name) => target.with_file_name(format!(".{}.batch.tmp", name.to_string_lossy())),
        None => target.with_extension("batch.tmp"),
    }
}

/// Removes the temporaries staged so far, on a failure path.
fn cleanup(staged: &[Staged]) {
    for one in staged {
        let _ = std::fs::remove_file(&one.temporary);
    }
}

/// The names that did not reach disk, in plan order.
fn names_after(names: &[String], restored: &[Written]) -> Vec<String> {
    let done: BTreeSet<&str> = restored.iter().map(|entry| entry.uri.as_str()).collect();
    names
        .iter()
        .filter(|uri| !done.contains(uri.as_str()))
        .cloned()
        .collect()
}

/// Applies one file's edits, or says why it could not.
fn apply_one(
    edit: &BatchEdit,
    documents: &mut BTreeMap<String, OpenDocument>,
    guards: &BTreeMap<String, WriteGuard>,
) -> Result<(OpenDocument, OpenDocument), Failure> {
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
        if let Err(refusal) = guard.check(
            crate::project::text::protection::WriteSource::Replace,
            &writes,
        ) {
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
    Ok((previous, document.clone()))
}

#[cfg(test)]
mod tests {
    use super::{Batch, BatchEdit, DiskFailure, Failure};
    use crate::app::language::diagnostics::TextEdit;
    use crate::app::language::workspace_edit::OpenDocument;
    use crate::project::document::conflict::Writability;
    use crate::project::text::protection::{ProtectedRegion, WriteGuard};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

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

    struct Temp(PathBuf);

    impl Temp {
        fn new(name: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("rustage-batch-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("a temp directory");
            Self(path)
        }

        fn root(&self) -> &std::path::Path {
            &self.0
        }

        fn write(&self, name: &str, text: &str) {
            std::fs::write(self.root().join(name), text).expect("a seed file");
        }

        fn read(&self, name: &str) -> String {
            std::fs::read_to_string(self.root().join(name)).expect("a written file")
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
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

    #[test]
    fn a_commit_writes_every_applied_file_to_disk() {
        let temp = Temp::new("commit");
        let mut documents = documents();
        let mut batch = Batch::new(plan());
        batch.run(&mut documents, &BTreeMap::new());

        let report = batch.commit(temp.root());
        assert!(report.is_complete(), "{}", report.render().join("\n"));
        assert_eq!(report.written().len(), 3);
        assert!(report.pending().is_empty());
        assert_eq!(temp.read("a.rs"), "Xaa");
        assert_eq!(temp.read("b.rs"), "Ybb");
        assert_eq!(temp.read("c.rs"), "Zcc");
        assert_eq!(batch.committed().len(), 3);
        // The temporary is not left beside the destination.
        assert!(!temp.root().join(".a.rs.batch.tmp").exists());
    }

    #[test]
    fn a_path_that_escapes_the_root_is_refused_before_anything_is_written() {
        let temp = Temp::new("escape");
        temp.write("safe.rs", "safe");
        let mut documents = BTreeMap::new();
        documents.insert("../outside.rs".to_owned(), OpenDocument::new("aaa", 1));
        documents.insert("/absolute.rs".to_owned(), OpenDocument::new("bbb", 1));
        documents.insert("safe.rs".to_owned(), OpenDocument::new("safe", 1));
        let mut batch = Batch::new(vec![
            BatchEdit::new("safe.rs", 1, vec![TextEdit::new(0, 1, "S")]),
            BatchEdit::new("../outside.rs", 1, vec![TextEdit::new(0, 1, "X")]),
            BatchEdit::new("/absolute.rs", 1, vec![TextEdit::new(0, 1, "Y")]),
        ]);
        batch.run(&mut documents, &BTreeMap::new());

        let report = batch.commit(temp.root());
        assert!(!report.is_complete());
        assert!(matches!(
            report.failure(),
            Some(DiskFailure::EscapesRoot { .. })
        ));
        assert!(report.written().is_empty());
        assert_eq!(report.pending().len(), 3);
        // The refusal happens before any stage, so even the safe file is untouched.
        assert_eq!(temp.read("safe.rs"), "safe");
        assert!(!temp.root().parent().unwrap().join("outside.rs").exists());

        // The root itself is not a file the root owns.
        let mut dot_documents = BTreeMap::new();
        dot_documents.insert(".".to_owned(), OpenDocument::new("aaa", 1));
        let mut dot_batch =
            Batch::new(vec![BatchEdit::new(".", 1, vec![TextEdit::new(0, 1, "X")])]);
        dot_batch.run(&mut dot_documents, &BTreeMap::new());
        let dot_report = dot_batch.commit(temp.root());
        assert!(matches!(
            dot_report.failure(),
            Some(DiskFailure::EscapesRoot { .. })
        ));
        assert!(dot_report.written().is_empty());
    }

    #[test]
    fn a_write_failure_midway_keeps_earlier_files_and_says_it_is_partial() {
        let temp = Temp::new("partial");
        temp.write("a.rs", "aaa");
        temp.write("c.rs", "ccc");
        // A directory where b.rs belongs makes the rename fail after a.rs has landed.
        std::fs::create_dir_all(temp.root().join("b.rs")).expect("a blocking directory");

        let mut documents = documents();
        let mut batch = Batch::new(plan());
        batch.run(&mut documents, &BTreeMap::new());
        let report = batch.commit(temp.root());

        assert!(!report.is_complete());
        assert!(matches!(
            report.failure(),
            Some(DiskFailure::NotRenamed { .. })
        ));
        assert_eq!(report.written().len(), 1);
        assert_eq!(report.written()[0].uri, "a.rs");
        assert_eq!(report.pending(), ["b.rs".to_owned(), "c.rs".to_owned()]);
        // The earlier file stayed on disk; the failed and later ones were not written over.
        assert_eq!(temp.read("a.rs"), "Xaa");
        assert_eq!(temp.read("c.rs"), "ccc");
        assert!(temp.root().join("b.rs").is_dir());
        // The batch keeps what landed traceable rather than reporting all done.
        assert_eq!(batch.committed().len(), 1);
        assert!(
            report.render()[0].contains("not written") || report.render()[0].contains("stopped")
        );
    }

    #[test]
    fn undo_restores_the_previous_text_in_memory_after_a_commit() {
        let temp = Temp::new("undo-memory");
        let mut documents = documents();
        let mut batch = Batch::new(plan());
        batch.run(&mut documents, &BTreeMap::new());
        assert!(batch.commit(temp.root()).is_complete());

        assert_eq!(batch.undo(&mut documents), 3);
        assert_eq!(documents["a.rs"].text, "aaa");
        assert_eq!(documents["b.rs"].text, "bbb");
        assert_eq!(documents["c.rs"].text, "ccc");
        // The in-memory undo does not pretend the disk changed.
        assert_eq!(temp.read("a.rs"), "Xaa");
    }

    #[test]
    fn a_rollback_puts_the_previous_text_back_on_disk() {
        let temp = Temp::new("rollback");
        temp.write("a.rs", "aaa");
        temp.write("b.rs", "bbb");
        temp.write("c.rs", "ccc");
        let mut documents = documents();
        let mut batch = Batch::new(plan());
        batch.run(&mut documents, &BTreeMap::new());
        assert!(batch.commit(temp.root()).is_complete());

        let report = batch.rollback(temp.root());
        assert!(report.is_complete(), "{}", report.render().join("\n"));
        assert_eq!(report.written().len(), 3);
        assert_eq!(temp.read("a.rs"), "aaa");
        assert_eq!(temp.read("b.rs"), "bbb");
        assert_eq!(temp.read("c.rs"), "ccc");
        assert!(batch.committed().is_empty());
    }
}
