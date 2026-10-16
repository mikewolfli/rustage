// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Saving a document (task T03.04): writing the text a buffer holds back to disk, so that a failure
//! in the middle leaves what was already there.
//!
//! The acceptance line is *"after an injected disk-full / permission / interruption failure the
//! original content is still recoverable"*. That is a property of the **write**, not a thing to
//! remember to do, and it comes from one shape being the only shape:
//!
//! * **Prepare, write, commit.** The text goes into a temporary file *beside* the target and is only
//!   then renamed over it. A rename within a directory is atomic, so a reader sees either the whole
//!   old file or the whole new one — never a half-written one. There is no code path that truncates
//!   the target first, so there is no window in which the original is gone.
//! * **[`Saving::execute`] is [`Saving::stage`] then [`Staged::commit`].** The two steps are the two
//!   things that can go wrong, and they are named so that a test can stop between them: that is how
//!   the *interruption* failure is exercised (stage, then do not commit) rather than described.
//! * **[`Saving::stray`] finds a temporary a crash left behind.** An interrupted save is not a
//!   mystery: the temporary is hidden and named after its target, so a scan of the target's directory
//!   says what was in flight. Cleaning it up is the recovery task's subject (T03.06).
//!
//! The four verbs are one implementation with one difference. [`Verb::Save`] writes back over the
//! file the document was read from and **refuses when that file is gone** — a missing file is a
//! delete somebody has to decide about (T03.05), not a save this code may turn into a re-creation.
//! [`Verb::SaveAs`] and [`Verb::CopyTo`] write elsewhere; the first adopts the new file, the second
//! leaves the document where it was. "Save all" and "commit a set of resources together" are
//! [`Commit`], which stages every file first and only renames them once every stage succeeded, so a
//! set of files is all committed or all left as they were.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::host::paths::{PathError, ProjectRoot};

/// Which write a person asked for.
///
/// The three are one operation with one difference — where the text goes and whether the document
/// follows it there — so they are one type rather than three near-identical functions.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Verb {
    /// Back over the file the document was read from.
    Save,
    /// To another file, which becomes the document's file.
    SaveAs {
        /// The project-relative path to write.
        to: String,
    },
    /// To another file, leaving the document's own file alone.
    CopyTo {
        /// The project-relative path to write.
        to: String,
    },
}

impl Verb {
    /// The word a report uses.
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Save => "save",
            Self::SaveAs { .. } => "save as",
            Self::CopyTo { .. } => "save a copy",
        }
    }

    /// Whether the document follows the text to where it was written.
    pub const fn adopts(&self) -> bool {
        matches!(self, Self::Save | Self::SaveAs { .. })
    }

    /// The project-relative path to write, given the document's own.
    fn target_of<'a>(&'a self, document: &'a str) -> &'a str {
        match self {
            Self::Save => document,
            Self::SaveAs { to } | Self::CopyTo { to } => to,
        }
    }
}

/// Why a save could not even be planned.
///
/// Both variants mean nothing was touched: a problem is found by reading the paths, before any
/// temporary file exists. A failure *after* the write began is a [`Failure`], and it is a different
/// type because it carries what happened to the bytes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Problem {
    /// The path is not one this build may write, in the host's own words.
    Path {
        /// Why, from the path rules; the host's sentence already names the path.
        reason: String,
    },
    /// The document's own file is not there, and a plain save will not re-create it.
    Gone {
        /// The file that is missing.
        path: PathBuf,
    },
    /// The file changed on disk since the document last read it, so writing would lose that change.
    Changed {
        /// The file somebody else changed.
        path: PathBuf,
    },
}

impl Problem {
    /// The problem in one sentence.
    pub fn message(&self) -> String {
        match self {
            Self::Path { reason } => reason.clone(),
            Self::Gone { path } => format!(
                "{} is not there; a save writes back over the file a document was read from, and \
                 re-creating a deleted file is a decision somebody has to make (T03.05), so nothing \
                 was written",
                path.display()
            ),
            Self::Changed { path } => format!(
                "{} changed on disk since this document read it; nothing was written, so the change \
                 was not silently overwritten (compare the two and choose which to keep — T03.05)",
                path.display()
            ),
        }
    }
}

impl From<PathError> for Problem {
    fn from(error: PathError) -> Self {
        Self::Path {
            reason: error.message(),
        }
    }
}

/// What a write did before it was stopped.
///
/// A [`Failure`] always has the same guarantee for a single file: the temporary was removed and the
/// target was never truncated, so the original is still exactly what it was. [`Self::original_intact`]
/// states that, and it is `false` only for a set write ([`Commit`]) where a rename already happened
/// and putting it back is what the failure is about.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Failure {
    reason: String,
    restored: Vec<PathBuf>,
    left_behind: Vec<String>,
}

impl Failure {
    /// Why the write stopped.
    pub fn reason(&self) -> &str {
        &self.reason
    }

    /// The files that had already been renamed and were put back.
    pub fn restored(&self) -> &[PathBuf] {
        &self.restored
    }

    /// What could not be put back, named file by file.
    pub fn left_behind(&self) -> &[String] {
        &self.left_behind
    }

    /// Whether what was on disk is exactly what was on disk before.
    pub fn original_intact(&self) -> bool {
        self.restored.is_empty() && self.left_behind.is_empty()
    }

    /// The failure in one sentence, saying whether the original is intact.
    pub fn message(&self) -> String {
        if self.original_intact() {
            format!(
                "{}; the original file is exactly as it was, so nothing was lost",
                self.reason
            )
        } else {
            format!(
                "{}; {} file(s) were put back and {} could not be ({})",
                self.reason,
                self.restored.len(),
                self.left_behind.len(),
                self.left_behind.join("; ")
            )
        }
    }
}

/// A file that was written.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Committed {
    target: PathBuf,
    created: bool,
    bytes: usize,
}

impl Committed {
    /// The file that was written.
    pub fn target(&self) -> &Path {
        &self.target
    }

    /// Whether the target did not exist before.
    pub const fn created(&self) -> bool {
        self.created
    }

    /// How many bytes were written.
    pub const fn bytes(&self) -> usize {
        self.bytes
    }
}

/// One document's text on its way to disk.
#[derive(Debug)]
pub struct Saving {
    target: PathBuf,
    temporary: PathBuf,
    text: String,
    previous: Option<Vec<u8>>,
    adopt: bool,
}

impl Saving {
    /// Plans a write of `text` for a document, without touching anything.
    ///
    /// `document` is the project-relative path the text belongs to. For [`Verb::Save`] the target is
    /// that file and it has to be there; for the other two it is the named path, which may be new.
    /// The target's own bytes are read here, so the plan knows whether a copy would overwrite
    /// something that is already there.
    pub fn plan(
        root: &ProjectRoot,
        document: &str,
        text: impl Into<String>,
        verb: Verb,
    ) -> Result<Self, Problem> {
        let origin = root.resolve(document)?;
        let target_relative = verb.target_of(document);
        let target = if matches!(verb, Verb::Save) {
            if !origin.is_file() {
                return Err(Problem::Gone { path: origin });
            }
            origin
        } else {
            root.resolve_for_creation(target_relative)?
        };
        let previous = std::fs::read(&target).ok();
        let temporary = temporary_beside(&target);
        Ok(Self {
            target,
            temporary,
            text: text.into(),
            previous,
            adopt: verb.adopts(),
        })
    }

    /// Plans a save that refuses to overwrite a change somebody else made (T03.05's guard).
    ///
    /// `agreed` is the bytes the document last read from the file. When they are not what is on
    /// disk now, the file was changed outside this designer, and the plan is refused rather than
    /// built — a save that went ahead would take the external change with it, silently. This is the
    /// same compare-and-swap the project document's writer performs, applied to a page document.
    pub fn plan_guarded(
        root: &ProjectRoot,
        document: &str,
        text: impl Into<String>,
        verb: Verb,
        agreed: Option<&[u8]>,
    ) -> Result<Self, Problem> {
        let saving = Self::plan(root, document, text, verb)?;
        if let Some(agreed) = agreed
            && let Some(previous) = &saving.previous
            && previous.as_slice() != agreed
        {
            return Err(Problem::Changed {
                path: saving.target.clone(),
            });
        }
        Ok(saving)
    }

    /// The file the text would be written to.
    pub fn target(&self) -> &Path {
        &self.target
    }

    /// The temporary the text would go through.
    pub fn temporary(&self) -> &Path {
        &self.temporary
    }

    /// Whether the target exists and already holds exactly this text.
    ///
    /// A save of what is already there is not a failure and not a change: it is the same file, and the
    /// report says so rather than writing it again.
    pub fn is_unchanged(&self) -> bool {
        self.previous.as_deref() == Some(self.text.as_bytes())
    }

    /// Whether the write would create the target.
    pub fn creates(&self) -> bool {
        self.previous.is_none()
    }

    /// Whether the document follows the text to the target.
    pub const fn adopts(&self) -> bool {
        self.adopt
    }

    /// The text that is about to be written.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Writes the text into the temporary, and stops there.
    ///
    /// This is the *interruption* seam: after it returns the target is untouched and the text is in
    /// [`Self::temporary`], exactly where a crash would leave it. [`Staged::commit`] is what makes it
    /// visible; a caller that does not commit has changed nothing a reader of the target can see.
    pub fn stage(self) -> Result<Staged, Failure> {
        if let Err(error) = std::fs::write(&self.temporary, &self.text) {
            // The temporary may exist in part; remove it, and say the original was never touched.
            let _ = std::fs::remove_file(&self.temporary);
            return Err(Failure {
                reason: format!(
                    "{} could not be written (through {}): {error}",
                    self.target.display(),
                    self.temporary.display()
                ),
                restored: Vec::new(),
                left_behind: Vec::new(),
            });
        }
        Ok(Staged { saving: self })
    }

    /// Stages the text and commits it in one step.
    pub fn execute(self) -> Result<Committed, Failure> {
        self.stage()?.commit()
    }

    /// The temporaries a crash left beside a target, in name order.
    ///
    /// An interrupted save is recognisable: the temporary is hidden (a leading dot) and named after
    /// its target with a `.tmp` tail, so scanning the target's directory says what was in flight
    /// without guessing. What to do about one is T03.06's subject.
    pub fn stray(target: &Path) -> Vec<PathBuf> {
        let Some(directory) = target.parent() else {
            return Vec::new();
        };
        let Some(name) = target
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
        else {
            return Vec::new();
        };
        let prefix = format!(".{name}.");
        let Ok(entries) = std::fs::read_dir(directory) else {
            return Vec::new();
        };
        let mut found: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .map(|candidate| candidate.to_string_lossy().into_owned())
                    .is_some_and(|candidate| {
                        candidate.starts_with(&prefix) && candidate.ends_with(".tmp")
                    })
            })
            .collect();
        found.sort();
        found
    }
}

/// The text, staged and waiting to be committed.
#[derive(Debug)]
pub struct Staged {
    saving: Saving,
}

impl Staged {
    /// The temporary holding the text.
    pub fn temporary(&self) -> &Path {
        &self.saving.temporary
    }

    /// The file the text will be written to.
    pub fn target(&self) -> &Path {
        &self.saving.target
    }

    /// Renames the temporary over the target.
    ///
    /// This is the one instant a reader could notice: a rename within a directory either happened or
    /// did not. A failure here removes the temporary and leaves the target as it was.
    pub fn commit(self) -> Result<Committed, Failure> {
        let created = self.saving.previous.is_none();
        let bytes = self.saving.text.len();
        if let Err(error) = std::fs::rename(&self.saving.temporary, &self.saving.target) {
            let _ = std::fs::remove_file(&self.saving.temporary);
            return Err(Failure {
                reason: format!(
                    "{} could not be replaced by the staged text: {error}",
                    self.saving.target.display()
                ),
                restored: Vec::new(),
                left_behind: Vec::new(),
            });
        }
        Ok(Committed {
            target: self.saving.target,
            created,
            bytes,
        })
    }

    /// Removes the temporary and changes nothing.
    pub fn abandon(self) -> PathBuf {
        let _ = std::fs::remove_file(&self.saving.temporary);
        self.saving.temporary
    }
}

/// Several files committed as one set: all of them land, or every one is left as it was.
///
/// This is what "save all" and "commit a set of resources together" are. Every file is staged first;
/// only when every stage succeeded are they renamed. A failure while renaming puts the ones that
/// landed back, from the bytes that were read when the set was planned, and names what could not be
/// put back — so a set write never reports a clean finish over a half-committed set.
#[derive(Debug)]
pub struct Commit {
    savings: Vec<Saving>,
}

/// The files a [`Commit`] wrote.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CommittedSet {
    targets: Vec<PathBuf>,
    bytes: usize,
}

impl CommittedSet {
    /// The files that were written, in the order they were given.
    pub fn targets(&self) -> &[PathBuf] {
        &self.targets
    }

    /// How many bytes were written in total.
    pub const fn bytes(&self) -> usize {
        self.bytes
    }
}

impl Commit {
    /// Plans a set of writes, without touching anything.
    ///
    /// Each entry is the project-relative path of a document and the text to write back. Every entry
    /// is a plain save over a file that is already there, because a set is "the project's files as
    /// they now stand": a file that has gone missing is a delete, and that is not a save.
    pub fn plan(
        root: &ProjectRoot,
        entries: impl IntoIterator<Item = (String, String)>,
    ) -> Result<Self, Problem> {
        let mut savings = Vec::new();
        for (document, text) in entries {
            savings.push(Saving::plan(root, &document, text, Verb::Save)?);
        }
        Ok(Self { savings })
    }

    /// The files this commit would write.
    pub fn targets(&self) -> Vec<PathBuf> {
        self.savings
            .iter()
            .map(|saving| saving.target().to_path_buf())
            .collect()
    }

    /// Whether none of the files would change.
    pub fn is_unchanged(&self) -> bool {
        self.savings.iter().all(Saving::is_unchanged)
    }

    /// Writes every file into its temporary, and stops there.
    ///
    /// This is the seam the joint failure test (T03.08) uses: a caller can stage the whole set and
    /// then change the disk underneath it, so that a rename fails after another rename has already
    /// landed and the put-back path is exercised rather than described.
    pub fn stage_all(self) -> Result<StagedSet, Failure> {
        let mut staged: Vec<Staged> = Vec::new();
        for saving in self.savings {
            match saving.stage() {
                Ok(one) => staged.push(one),
                Err(failure) => {
                    // Nothing has been renamed yet, so the set is exactly as it was: remove the
                    // temporaries that were staged and report the original untouched.
                    for one in staged {
                        one.abandon();
                    }
                    return Err(failure);
                }
            }
        }
        Ok(StagedSet { staged })
    }

    /// Stages every file, then commits them all.
    pub fn execute(self) -> Result<CommittedSet, Failure> {
        self.stage_all()?.commit_all()
    }
}

/// A whole set of files, staged and waiting to be committed.
#[derive(Debug)]
pub struct StagedSet {
    staged: Vec<Staged>,
}

impl StagedSet {
    /// The files the set will write, in the order they were given.
    pub fn targets(&self) -> Vec<PathBuf> {
        self.staged
            .iter()
            .map(|one| one.target().to_path_buf())
            .collect()
    }

    /// Renames every temporary over its target.
    ///
    /// A failure part-way puts the files that already landed back, from the bytes that were read when
    /// the set was planned, and names what could not be put back.
    pub fn commit_all(self) -> Result<CommittedSet, Failure> {
        let mut targets = Vec::new();
        let mut bytes = 0usize;
        let mut done: Vec<(PathBuf, Option<Vec<u8>>)> = Vec::new();
        for one in self.staged {
            let target = one.target().to_path_buf();
            let previous = one.saving.previous.clone();
            match one.commit() {
                Ok(committed) => {
                    targets.push(committed.target);
                    bytes += committed.bytes;
                    done.push((target, previous));
                }
                Err(failure) => {
                    let restored = put_back(&done);
                    let mut left_behind = Vec::new();
                    for (path, _) in &done {
                        if !restored.contains(path) {
                            left_behind.push(format!(
                                "{} was written and could not be put back",
                                path.display()
                            ));
                        }
                    }
                    return Err(Failure {
                        reason: failure.reason().to_owned(),
                        restored,
                        left_behind,
                    });
                }
            }
        }
        Ok(CommittedSet { targets, bytes })
    }

    /// Removes every temporary and changes nothing.
    pub fn abandon_all(self) {
        for one in self.staged {
            one.abandon();
        }
    }
}

/// Puts a set of already-committed files back from the bytes that were read, and reports which landed.
fn put_back(done: &[(PathBuf, Option<Vec<u8>>)]) -> Vec<PathBuf> {
    let mut restored = Vec::new();
    for (path, previous) in done.iter().rev() {
        let ok = match previous {
            Some(bytes) => write_through_temporary(path, bytes),
            // The file did not exist before this commit, so putting it back means removing it.
            None => std::fs::remove_file(path).is_ok(),
        };
        if ok {
            restored.push(path.clone());
        }
    }
    restored
}

/// Writes bytes to a path through a temporary beside it.
fn write_through_temporary(path: &Path, bytes: &[u8]) -> bool {
    let temporary = temporary_beside(path);
    if std::fs::write(&temporary, bytes).is_err() {
        return false;
    }
    if std::fs::rename(&temporary, path).is_err() {
        let _ = std::fs::remove_file(&temporary);
        return false;
    }
    true
}

/// A hidden temporary beside a target, named after it so an interrupted write is recognisable.
fn temporary_beside(path: &Path) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let directory = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_owned());
    directory.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A directory that removes itself, so a test leaves nothing behind.
    struct TempDirectory(PathBuf);

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct Sandbox {
        root: ProjectRoot,
        path: PathBuf,
        _guard: TempDirectory,
    }

    impl Sandbox {
        fn write(&self, relative: &str, text: &str) {
            let path = self.path.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("a directory is creatable");
            }
            std::fs::write(path, text).expect("a file is writable");
        }

        fn read(&self, relative: &str) -> String {
            std::fs::read_to_string(self.path.join(relative)).expect("a file is readable")
        }
    }

    fn sandbox() -> Sandbox {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("rustage-save-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&path).expect("a temporary directory is creatable");
        std::fs::write(path.join("project.json"), "{\"version\":1}").expect("a project document");
        let root = ProjectRoot::new(&path).expect("the root resolves");
        Sandbox {
            root,
            path: path.clone(),
            _guard: TempDirectory(path),
        }
    }

    const PAGE: &str = "{\"version\":1,\"page\":{\"id\":\"p\",\"kind\":\"page\"}}\n";

    #[test]
    fn a_save_replaces_the_file_through_a_temporary_and_leaves_none_behind() {
        let sandbox = sandbox();
        sandbox.write("pages/home.json", PAGE);
        let saving = Saving::plan(
            &sandbox.root,
            "pages/home.json",
            PAGE.replace("p", "q"),
            Verb::Save,
        )
        .expect("a plan");
        assert!(!saving.creates());
        assert!(!saving.is_unchanged());
        let committed = saving.execute().expect("the save lands");
        assert_eq!(committed.target(), sandbox.path.join("pages/home.json"));
        assert!(!committed.created());
        assert!(sandbox.read("pages/home.json").contains("\"q\""));
        assert!(
            Saving::stray(&sandbox.path.join("pages/home.json")).is_empty(),
            "a completed save leaves no temporary"
        );
    }

    #[test]
    fn a_save_of_what_is_already_there_is_not_a_change() {
        let sandbox = sandbox();
        sandbox.write("pages/home.json", PAGE);
        let saving = Saving::plan(&sandbox.root, "pages/home.json", PAGE, Verb::Save).unwrap();
        assert!(saving.is_unchanged());
    }

    #[test]
    fn a_save_as_writes_elsewhere_and_adopts_it_while_a_copy_does_not() {
        let sandbox = sandbox();
        sandbox.write("pages/home.json", PAGE);
        let as_new = Saving::plan(
            &sandbox.root,
            "pages/home.json",
            PAGE,
            Verb::SaveAs {
                to: "pages/home-2.json".to_owned(),
            },
        )
        .expect("a plan");
        assert!(as_new.adopts());
        assert!(as_new.creates());
        let committed = as_new.execute().expect("the save-as lands");
        assert!(committed.created());
        assert!(sandbox.path.join("pages/home-2.json").is_file());
        assert!(sandbox.path.join("pages/home.json").is_file());

        let copy = Saving::plan(
            &sandbox.root,
            "pages/home.json",
            PAGE,
            Verb::CopyTo {
                to: "pages/home-3.json".to_owned(),
            },
        )
        .expect("a plan");
        assert!(!copy.adopts());
        copy.execute().expect("the copy lands");
        assert!(sandbox.path.join("pages/home-3.json").is_file());
    }

    #[test]
    fn a_plain_save_refuses_when_the_file_has_gone() {
        let sandbox = sandbox();
        let problem = Saving::plan(&sandbox.root, "gone.json", PAGE, Verb::Save)
            .expect_err("a missing file is refused");
        assert!(
            problem.message().contains("not there"),
            "{}",
            problem.message()
        );
    }

    #[test]
    fn an_interrupted_save_leaves_the_original_and_a_temporary_that_can_be_found() {
        let sandbox = sandbox();
        sandbox.write("pages/home.json", PAGE);
        let saving =
            Saving::plan(&sandbox.root, "pages/home.json", "half-written", Verb::Save).unwrap();
        let staged = saving.stage().expect("the temporary is written");
        let temporary = staged.temporary().to_path_buf();
        // Stop here, the way a crash would: do not commit, and do not clean up either.
        std::mem::forget(staged);

        assert_eq!(
            sandbox.read("pages/home.json"),
            PAGE,
            "the target is exactly as it was, because a rename never happened"
        );
        assert!(temporary.is_file(), "the temporary is what a crash leaves");
        let stray = Saving::stray(&sandbox.path.join("pages/home.json"));
        assert_eq!(stray, vec![temporary], "the temporary is recognisable");
        std::fs::remove_file(&stray[0]).expect("a caller can clean it up");
    }

    #[test]
    fn abandoning_a_staged_save_removes_the_temporary_and_changes_nothing() {
        let sandbox = sandbox();
        sandbox.write("pages/home.json", PAGE);
        let saving = Saving::plan(&sandbox.root, "pages/home.json", "other", Verb::Save).unwrap();
        let staged = saving.stage().expect("the temporary is written");
        let temporary = staged.abandon();
        assert!(!temporary.exists());
        assert_eq!(sandbox.read("pages/home.json"), PAGE);
    }

    #[cfg(unix)]
    #[test]
    fn a_write_the_directory_refuses_leaves_the_original_intact() {
        use std::os::unix::fs::PermissionsExt;

        // A root user ignores directory permissions, so this check only means something when it runs
        // as somebody else. Skipping is honest: the assertion would be vacuous as root.
        if libc_geteuid() == 0 {
            return;
        }

        let sandbox = sandbox();
        sandbox.write("pages/home.json", PAGE);
        let directory = sandbox.path.join("pages");
        let original = std::fs::metadata(&directory).unwrap().permissions();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o555))
            .expect("the directory can be made read-only");

        let saving = Saving::plan(&sandbox.root, "pages/home.json", "refused", Verb::Save).unwrap();
        let failure = saving.execute().expect_err("the write is refused");

        // Put the directory back before asserting, so a failure cleans up after itself.
        std::fs::set_permissions(&directory, original).expect("the directory is writable again");
        assert!(failure.original_intact(), "{}", failure.message());
        assert!(failure.message().contains("nothing was lost"));
        assert_eq!(sandbox.read("pages/home.json"), PAGE);
        assert!(Saving::stray(&sandbox.path.join("pages/home.json")).is_empty());
    }

    #[cfg(unix)]
    fn libc_geteuid() -> u32 {
        // Read through `id -u` rather than linking libc: the build has no libc dependency, and this
        // is a test-only check.
        let output = std::process::Command::new("id").arg("-u").output();
        match output {
            Ok(output) => String::from_utf8_lossy(&output.stdout)
                .trim()
                .parse()
                .unwrap_or(0),
            Err(_) => 0,
        }
    }

    #[test]
    fn a_save_refuses_to_overwrite_a_change_somebody_else_made() {
        let sandbox = sandbox();
        sandbox.write("pages/home.json", PAGE);
        let agreed = PAGE.as_bytes().to_vec();
        // Somebody else edits the file after the buffer read it.
        sandbox.write("pages/home.json", "{\"version\":1,\"edited\":true}\n");

        let problem = Saving::plan_guarded(
            &sandbox.root,
            "pages/home.json",
            "mine",
            Verb::Save,
            Some(&agreed),
        )
        .expect_err("the external change is refused");
        assert!(matches!(problem, Problem::Changed { .. }));
        assert!(problem.message().contains("not silently overwritten"));
        assert_eq!(
            sandbox.read("pages/home.json"),
            "{\"version\":1,\"edited\":true}\n",
            "the file nobody wrote is exactly as the other change left it"
        );

        // The same plan without the guard writes, which is why the guard is a choice a caller makes.
        let plain = Saving::plan(&sandbox.root, "pages/home.json", "mine", Verb::Save).unwrap();
        plain.execute().expect("an unguarded save writes");
        assert_eq!(sandbox.read("pages/home.json"), "mine");
    }

    #[test]
    fn a_set_of_files_is_committed_together() {
        let sandbox = sandbox();
        sandbox.write("pages/a.json", PAGE);
        sandbox.write("pages/b.json", PAGE);
        let commit = Commit::plan(
            &sandbox.root,
            [
                ("pages/a.json".to_owned(), "A".to_owned()),
                ("pages/b.json".to_owned(), "B".to_owned()),
            ],
        )
        .expect("a plan");
        let committed = commit.execute().expect("the set lands");
        assert_eq!(committed.targets().len(), 2);
        assert_eq!(sandbox.read("pages/a.json"), "A");
        assert_eq!(sandbox.read("pages/b.json"), "B");
    }

    #[test]
    fn a_set_whose_second_stage_fails_leaves_the_first_exactly_as_it_was() {
        let sandbox = sandbox();
        sandbox.write("pages/a.json", PAGE);
        sandbox.write("pages/b.json", PAGE);
        let commit = Commit::plan(
            &sandbox.root,
            [
                ("pages/a.json".to_owned(), "A".to_owned()),
                ("pages/b.json".to_owned(), "B".to_owned()),
            ],
        )
        .expect("a plan");

        // Stage everything, then make the second target impossible to rename over: this is the
        // interruption the put-back path exists for, and it happens *after* the first file staged.
        let staged = commit.stage_all().expect("every file stages");
        let second = sandbox.path.join("pages/b.json");
        std::fs::remove_file(&second).expect("the second target can be replaced");
        std::fs::create_dir(&second).expect("a directory now sits where the file was");

        let failure = staged.commit_all().expect_err("the second rename fails");
        assert!(
            !failure.original_intact(),
            "a file did land before the failure"
        );
        assert_eq!(failure.restored().len(), 1);
        assert_eq!(
            sandbox.read("pages/a.json"),
            PAGE,
            "the first file was put back from the bytes that were read"
        );
    }
}
