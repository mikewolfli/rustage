// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Closing documents, a project or a window: the dirty-content decision and stopping the work that
//! belongs to what is being closed (task T03.07).
//!
//! The acceptance line is *"cancelling a close keeps every view and task consistent"*. That is a
//! promise about what a **cancel** and a **failure** leave behind, so the shape here is chosen to
//! make it true rather than to make it likely:
//!
//! * **Closing is planned before it is decided.** [`Closing::plan`] answers one question — is anything
//!   dirty — and nothing is asked when nothing is. A person is only interrupted when there is
//!   genuinely a choice.
//! * **A cancel is the absence of an action.** [`Closing::close`] takes the state **by value** and
//!   gives it back; [`Decision::Cancel`] returns the same state with [`Outcome::Cancelled`], so
//!   "cancelling kept everything" is not a claim about care but the identity of two values. The same
//!   is true of a failure while saving: [`Outcome::Blocked`] returns the state, nothing was closed,
//!   and the tasks are still there.
//! * **Saving happens before closing, and one failure stops the close.** [`Decision::SaveAll`] writes
//!   each dirty document through the save path ([`crate::project::save`]) — the same atomic write that
//!   refuses to overwrite an external change — and if any write fails the close does **not** happen.
//!   Documents whose write succeeded are reported as saved (their content is on disk); they are not
//!   silently reported as unsaved, and the rest are named.
//! * **Work stops only when the close actually happens.** [`Task`]s are stopped after the decision
//!   succeeds, never before: stopping work for a close that is then cancelled would be its own
//!   inconsistency.

use crate::host::paths::ProjectRoot;
use crate::project::save::{Saving, Verb};

/// One open document and the version it agreed with on disk.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Document {
    path: String,
    buffer: String,
    base: Option<Vec<u8>>,
    writable: bool,
}

impl Document {
    /// A document as it is on disk: its buffer is the file, so it is not dirty.
    pub fn clean(root: &ProjectRoot, relative: &str) -> Result<Self, String> {
        let path = root.resolve(relative).map_err(|error| error.message())?;
        let bytes = std::fs::read(&path)
            .map_err(|error| format!("{} could not be read: {error}", path.display()))?;
        let buffer = String::from_utf8(bytes.clone())
            .map_err(|error| format!("{} is not text this build holds: {error}", path.display()))?;
        Ok(Self {
            path: relative.to_owned(),
            buffer,
            base: Some(bytes),
            writable: true,
        })
    }

    /// A document with unsaved changes: a buffer that differs from the file it read.
    pub fn unsaved(
        root: &ProjectRoot,
        relative: &str,
        buffer: impl Into<String>,
    ) -> Result<Self, String> {
        let path = root.resolve(relative).map_err(|error| error.message())?;
        let base = std::fs::read(&path).ok();
        Ok(Self {
            path: relative.to_owned(),
            buffer: buffer.into(),
            base,
            writable: true,
        })
    }

    /// The document's project-relative path.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The text the document holds.
    pub fn buffer(&self) -> &str {
        &self.buffer
    }

    /// Whether the document holds a change the file does not have.
    pub fn is_dirty(&self) -> bool {
        self.base.as_deref() != Some(self.buffer.as_bytes())
    }

    /// Whether the document may be written.
    pub const fn is_writable(&self) -> bool {
        self.writable
    }

    /// Marks the document as holding no unsaved change.
    fn settle(&mut self) {
        self.base = Some(self.buffer.clone().into_bytes());
    }
}

/// Work that belongs to what is being closed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Task {
    name: String,
    running: bool,
}

impl Task {
    /// A running task.
    pub fn running(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            running: true,
        }
    }

    /// The task's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether it is still running.
    pub const fn is_running(&self) -> bool {
        self.running
    }
}

/// What closing is waiting for.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Plan {
    /// Nothing is dirty: the close needs no decision.
    Clean,
    /// Some documents are dirty; the person has to choose.
    NeedsDecision {
        /// The documents with unsaved changes, in order.
        dirty: Vec<String>,
        /// How many tasks would be stopped.
        tasks: usize,
    },
}

impl Plan {
    /// Whether a person has to choose before the close can happen.
    pub const fn needs_a_decision(&self) -> bool {
        matches!(self, Self::NeedsDecision { .. })
    }

    /// The plan in one sentence.
    pub fn message(&self) -> String {
        match self {
            Self::Clean => "nothing has unsaved changes; the close needs no decision".to_owned(),
            Self::NeedsDecision { dirty, tasks } => format!(
                "{} document(s) have unsaved changes ({}) and {tasks} task(s) would stop: save all, \
                 discard all, or cancel",
                dirty.len(),
                dirty.join(", ")
            ),
        }
    }
}

/// What the person chose, or what should happen when nobody had to choose.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Decision {
    /// Write every dirty document, then close.
    SaveAll,
    /// Close without writing; the unsaved changes are gone.
    DiscardAll,
    /// Do nothing at all.
    Cancel,
}

impl Decision {
    /// The name a command line uses.
    pub const fn name(self) -> &'static str {
        match self {
            Self::SaveAll => "save-all",
            Self::DiscardAll => "discard-all",
            Self::Cancel => "cancel",
        }
    }

    /// The decision a name asks for, when it is one.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "save-all" => Some(Self::SaveAll),
            "discard-all" => Some(Self::DiscardAll),
            "cancel" => Some(Self::Cancel),
            _ => None,
        }
    }
}

/// What closing did.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Outcome {
    /// Everything closed: the documents that were written and the ones whose changes were discarded,
    /// and the tasks that were stopped. `discarded` names documents whose unsaved changes were lost
    /// on purpose, so a report never has to say "nothing was lost" when something was.
    Closed {
        /// Documents written on the way out.
        saved: Vec<String>,
        /// Documents closed without writing.
        discarded: Vec<String>,
        /// Tasks that were stopped.
        stopped: Vec<String>,
    },
    /// Nothing happened: every document and task is exactly as it was.
    Cancelled,
    /// A write failed, so the close did not happen. `saved` names the documents whose write succeeded
    /// anyway (their content is on disk), and `unsaved` the ones that still hold unsaved changes.
    Blocked {
        /// Why the close stopped.
        reason: String,
        /// Documents that were written before the failure.
        saved: Vec<String>,
        /// Documents that still hold unsaved changes.
        unsaved: Vec<String>,
    },
}

impl Outcome {
    /// Whether the close happened.
    pub const fn happened(&self) -> bool {
        matches!(self, Self::Closed { .. })
    }

    /// The outcome in one sentence.
    pub fn message(&self) -> String {
        match self {
            Self::Closed {
                saved,
                discarded,
                stopped,
            } => format!(
                "closed: {} written, {} discarded, {} task(s) stopped",
                saved.len(),
                discarded.len(),
                stopped.len()
            ),
            Self::Cancelled => {
                "the close was cancelled; every document and task is as it was".to_owned()
            }
            Self::Blocked {
                reason,
                saved,
                unsaved,
            } => format!(
                "the close did not happen: {reason}; {} document(s) were written and {} still have \
                 unsaved changes",
                saved.len(),
                unsaved.len()
            ),
        }
    }
}

/// A set of open documents and their tasks, and what to do when it is closed.
#[derive(Clone, Debug)]
pub struct Closing {
    root: ProjectRoot,
    documents: Vec<Document>,
    tasks: Vec<Task>,
}

impl Closing {
    /// Opens a closing over a project's documents and tasks.
    pub fn new(root: ProjectRoot, documents: Vec<Document>, tasks: Vec<Task>) -> Self {
        Self {
            root,
            documents,
            tasks,
        }
    }

    /// The documents being closed.
    pub fn documents(&self) -> &[Document] {
        &self.documents
    }

    /// The tasks being stopped.
    pub fn tasks(&self) -> &[Task] {
        &self.tasks
    }

    /// What closing is waiting for.
    pub fn plan(&self) -> Plan {
        let dirty: Vec<String> = self
            .documents
            .iter()
            .filter(|document| document.is_dirty())
            .map(|document| document.path().to_owned())
            .collect();
        if dirty.is_empty() {
            Plan::Clean
        } else {
            Plan::NeedsDecision {
                dirty,
                tasks: self.tasks.iter().filter(|task| task.running).count(),
            }
        }
    }

    /// Carries out the close, and gives the (possibly unchanged) state back.
    ///
    /// The state is taken by value and returned so that "nothing happened" is a value, not a promise:
    /// a cancel, and a close whose write failed, both return the state unchanged and an outcome that
    /// names what did not happen. Work stops only in the branch where the close really closed.
    pub fn close(mut self, decision: Decision) -> (Self, Outcome) {
        match decision {
            Decision::Cancel => (self, Outcome::Cancelled),
            Decision::DiscardAll => {
                let discarded = self.dirty_paths();
                for document in &mut self.documents {
                    document.settle();
                }
                let stopped = self.stop_tasks();
                (
                    self,
                    Outcome::Closed {
                        saved: Vec::new(),
                        discarded,
                        stopped,
                    },
                )
            }
            Decision::SaveAll => {
                let mut saved = Vec::new();
                for index in 0..self.documents.len() {
                    if !self.documents[index].is_dirty() {
                        continue;
                    }
                    let path = self.documents[index].path.clone();
                    let text = self.documents[index].buffer.clone();
                    let agreed = self.documents[index].base.clone();
                    let saving = match Saving::plan_guarded(
                        &self.root,
                        &path,
                        text,
                        Verb::Save,
                        agreed.as_deref(),
                    ) {
                        Ok(saving) => saving,
                        Err(problem) => {
                            let unsaved = self.dirty_paths();
                            return (
                                self,
                                Outcome::Blocked {
                                    reason: problem.message(),
                                    saved,
                                    unsaved,
                                },
                            );
                        }
                    };
                    match saving.execute() {
                        Ok(_) => {
                            self.documents[index].settle();
                            saved.push(path);
                        }
                        Err(failure) => {
                            let unsaved = self.dirty_paths();
                            return (
                                self,
                                Outcome::Blocked {
                                    reason: failure.message(),
                                    saved,
                                    unsaved,
                                },
                            );
                        }
                    }
                }
                let stopped = self.stop_tasks();
                (
                    self,
                    Outcome::Closed {
                        saved,
                        discarded: Vec::new(),
                        stopped,
                    },
                )
            }
        }
    }

    fn dirty_paths(&self) -> Vec<String> {
        self.documents
            .iter()
            .filter(|document| document.is_dirty())
            .map(|document| document.path().to_owned())
            .collect()
    }

    fn stop_tasks(&mut self) -> Vec<String> {
        let mut stopped = Vec::new();
        for task in &mut self.tasks {
            if task.running {
                task.running = false;
                stopped.push(task.name.clone());
            }
        }
        stopped
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TempDirectory(PathBuf);

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn sandbox() -> (ProjectRoot, PathBuf, TempDirectory) {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("rustage-closing-{}-{id}", std::process::id()));
        std::fs::create_dir_all(path.join("pages")).expect("a directory");
        std::fs::write(path.join("project.json"), "{\"version\":1}").expect("a project");
        std::fs::write(path.join("pages/home.json"), "disk\n").expect("writable");
        std::fs::write(path.join("pages/about.json"), "other\n").expect("writable");
        let root = ProjectRoot::new(&path).expect("the root resolves");
        (root, path.clone(), TempDirectory(path))
    }

    fn closing(root: &ProjectRoot, dirty: bool) -> Closing {
        let home = if dirty {
            Document::unsaved(root, "pages/home.json", "edited\n").expect("a document")
        } else {
            Document::clean(root, "pages/home.json").expect("a document")
        };
        Closing::new(
            root.clone(),
            vec![home],
            vec![Task::running("build"), Task::running("preview")],
        )
    }

    #[test]
    fn a_clean_set_needs_no_decision() {
        let (root, _path, _guard) = sandbox();
        assert_eq!(closing(&root, false).plan(), Plan::Clean);
        let plan = closing(&root, true).plan();
        assert_eq!(
            plan,
            Plan::NeedsDecision {
                dirty: vec!["pages/home.json".to_owned()],
                tasks: 2
            }
        );
        assert!(plan.message().contains("save all, discard all, or cancel"));
    }

    #[test]
    fn cancelling_keeps_every_document_and_task_exactly_as_it_was() {
        let (root, _path, _guard) = sandbox();
        let before = closing(&root, true);
        let (after, outcome) = before.clone().close(Decision::Cancel);
        assert_eq!(outcome, Outcome::Cancelled);
        assert_eq!(after.documents(), before.documents());
        assert_eq!(after.tasks(), before.tasks());
        assert!(after.plan().needs_a_decision());
    }

    #[test]
    fn discarding_closes_and_stops_the_work() {
        let (root, path, _guard) = sandbox();
        let (after, outcome) = closing(&root, true).close(Decision::DiscardAll);
        assert!(outcome.happened());
        assert_eq!(
            std::fs::read_to_string(path.join("pages/home.json")).unwrap(),
            "disk\n",
            "discarding writes nothing"
        );
        assert!(after.tasks().iter().all(|task| !task.is_running()));
        if let Outcome::Closed { discarded, .. } = outcome {
            assert_eq!(discarded, ["pages/home.json"]);
        }
    }

    #[test]
    fn saving_writes_every_dirty_document_then_closes() {
        let (root, path, _guard) = sandbox();
        let (after, outcome) = closing(&root, true).close(Decision::SaveAll);
        assert!(outcome.happened());
        assert_eq!(
            std::fs::read_to_string(path.join("pages/home.json")).unwrap(),
            "edited\n"
        );
        assert!(
            after
                .documents()
                .iter()
                .all(|document| !document.is_dirty())
        );
        assert!(after.tasks().iter().all(|task| !task.is_running()));
    }

    #[cfg(unix)]
    #[test]
    fn a_write_that_fails_stops_the_close_and_leaves_the_tasks_running() {
        use std::os::unix::fs::PermissionsExt;
        if is_root() {
            return;
        }
        let (root, path, _guard) = sandbox();
        let mut documents = vec![
            Document::unsaved(&root, "pages/home.json", "edited\n").expect("a document"),
            Document::unsaved(&root, "pages/about.json", "also edited\n").expect("a document"),
        ];
        // The second document cannot be written, so the close must stop rather than half-close.
        std::fs::set_permissions(path.join("pages"), std::fs::Permissions::from_mode(0o555))
            .expect("chmod");
        // Make the first document's buffer already equal to the file, so the failing write is the
        // only one attempted for it and the failure is the second document's.
        documents[0] = Document::clean(&root, "pages/home.json").expect("a document");

        let all_tasks_running =
            |closing: &Closing| closing.tasks().iter().all(|task| task.is_running());
        let closing = Closing::new(
            root.clone(),
            documents,
            vec![Task::running("build"), Task::running("preview")],
        );
        let (after, outcome) = closing.close(Decision::SaveAll);
        std::fs::set_permissions(path.join("pages"), std::fs::Permissions::from_mode(0o755))
            .expect("chmod back");

        assert!(!outcome.happened());
        assert!(outcome.message().contains("did not happen"));
        assert!(
            all_tasks_running(&after),
            "the work must not stop for a close that did not happen"
        );
    }

    #[cfg(unix)]
    fn is_root() -> bool {
        std::process::Command::new("id")
            .arg("-u")
            .output()
            .ok()
            .and_then(|output| {
                String::from_utf8_lossy(&output.stdout)
                    .trim()
                    .parse::<u32>()
                    .ok()
            })
            == Some(0)
    }
}
