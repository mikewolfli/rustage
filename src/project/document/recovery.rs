// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Automatic recovery copies, startup recovery and the session snapshot history (task T03.06).
//!
//! The acceptance line is *"a crash's recovery says its version and where it came from, and a person
//! can refuse it"*. Three decisions make that structural:
//!
//! * **A copy is not the project.** Recovery copies live **outside** the project root, under a
//!   directory the caller names (the host's own state directory in the real application). A copy the
//!   project could see would be a file the project would have to ignore, and a crash that left the
//!   project dirty would be, in part, this feature's fault. Keeping them apart is also what makes
//!   "the project is exactly what you saved, and here separately is what was in memory" true.
//! * **A copy carries its own provenance.** Each file begins with a small header — the format marker,
//!   the project root it belonged to, the document's project-relative path, the session it was written
//!   in, and when. Nothing is inferred from the directory layout or a file's name, so a copy that is
//!   moved, copied or inspected by hand still says what it is. Startup recovery is then just
//!   [`RecoveryStore::for_project`]: a list, each with its source.
//! * **Recovery is a question, not an action.** [`Copy::standing`] compares the copy with the file
//!   *now* — the same thing, the only copy there is, or a disagreement — so a person can decide to
//!   take it or [`RecoveryStore::discard`] it. Nothing adopts a copy on its own, because a recovery
//!   that silently replaced a file the user saved afterwards would be worse than no recovery at all.
//!
//! The format is one file per copy:
//!
//! ```text
//! rustage-recovery/1
//! root = /absolute/project/root
//! document = pages/home.json
//! session = 3
//! written = 1760000000000000000
//!
//! <the text, verbatim>
//! ```
//!
//! The header ends at the first blank line, so the text is copied byte for byte — including a leading
//! newline or a blank first line, which a rule that "skipped leading whitespace" would have eaten.
//! `written` is nanoseconds since the Unix epoch: seconds would not order two copies written in the
//! same second, and "the newest copy" is exactly what startup recovery and the retention policy ask
//! for.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// The first line of a recovery copy, so a file that is not one is refused rather than misread.
pub const RECOVERY_MARKER: &str = "rustage-recovery/1";

/// Why a recovery copy could not be written or read.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RecoveryError {
    /// The copy could not be written.
    NotWritten {
        /// What was being written.
        path: PathBuf,
        /// The OS's own reason.
        reason: String,
    },
    /// A file in the store is not a recovery copy.
    NotACopy {
        /// The file that was refused.
        path: PathBuf,
        /// Why it was refused.
        reason: String,
    },
}

impl RecoveryError {
    /// The error in one sentence.
    pub fn message(&self) -> String {
        match self {
            Self::NotWritten { path, reason } => {
                format!("{} could not be written: {reason}", path.display())
            }
            Self::NotACopy { path, reason } => {
                format!("{} is not a recovery copy: {reason}", path.display())
            }
        }
    }
}

/// How a recovery copy stands next to the file it belongs to now.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Standing {
    /// The file already holds exactly the copy's text: nothing to recover.
    Same,
    /// The file is gone: the copy is the only version left.
    OnlyCopy,
    /// The file holds different text: taking the copy would replace it.
    Differs {
        /// How many bytes the file has now.
        current: usize,
    },
}

impl Standing {
    /// Whether taking the copy would change the file.
    pub const fn needs_a_decision(&self) -> bool {
        !matches!(self, Self::Same)
    }

    /// The standing in one sentence.
    pub fn message(&self) -> String {
        match self {
            Self::Same => "the file already holds this text; nothing to recover".to_owned(),
            Self::OnlyCopy => {
                "the file is gone, so this copy is the only version there is".to_owned()
            }
            Self::Differs { current } => format!(
                "the file holds something else ({current} byte(s)); taking this copy would replace it"
            ),
        }
    }
}

/// A recovery copy that was written.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Recorded {
    path: PathBuf,
    document: String,
    session: String,
}

impl Recorded {
    /// Where the copy was written.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The document it belongs to.
    pub fn document(&self) -> &str {
        &self.document
    }

    /// The session it was written in.
    pub fn session(&self) -> &str {
        &self.session
    }

    /// The write in one sentence.
    pub fn message(&self) -> String {
        format!(
            "kept a recovery copy of {} from session {} at {}",
            self.document,
            self.session,
            self.path.display()
        )
    }
}

/// One recovery copy, read back from the store.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Copy {
    file: PathBuf,
    root: PathBuf,
    document: String,
    session: String,
    written: u64,
    text: String,
}

impl Copy {
    /// The file the copy is stored in, outside the project.
    pub fn file(&self) -> &Path {
        &self.file
    }

    /// The project root the copy belonged to.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The document's project-relative path.
    pub fn document(&self) -> &str {
        &self.document
    }

    /// The session the copy was written in.
    pub fn session(&self) -> &str {
        &self.session
    }

    /// When it was written, as nanoseconds since the Unix epoch.
    pub const fn written(&self) -> u64 {
        self.written
    }

    /// The text the copy holds.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// How the copy stands next to the file it belongs to now.
    pub fn standing(&self) -> Standing {
        let path = self.root.join(&self.document);
        match std::fs::read(&path) {
            Ok(bytes) if bytes == self.text.as_bytes() => Standing::Same,
            Ok(bytes) => Standing::Differs {
                current: bytes.len(),
            },
            Err(_) => Standing::OnlyCopy,
        }
    }

    /// The copy in one report: where it came from, when, and how it stands.
    pub fn render(&self) -> Vec<String> {
        vec![
            format!("{} ({} byte(s))", self.document, self.text.len()),
            format!(
                "  from session {}, written at {} (nanoseconds since the unix epoch), project {}",
                self.session,
                self.written,
                self.root.display()
            ),
            format!("  kept at {}", self.file.display()),
            format!("  {}", self.standing().message()),
        ]
    }
}

/// Where recovery copies are kept.
///
/// The directory is the caller's choice and is deliberately not inside a project (see the module
/// documentation). Copies are filed by session, then by a hash of the project root, so two projects
/// with the same document path cannot collide — and the header carries the truth either way.
#[derive(Clone, Debug)]
pub struct RecoveryStore {
    base: PathBuf,
}

impl RecoveryStore {
    /// A store under `base`.
    pub fn new(base: impl Into<PathBuf>) -> Self {
        Self { base: base.into() }
    }

    /// The directory the store keeps its copies in.
    pub fn base(&self) -> &Path {
        &self.base
    }

    /// Writes a recovery copy of `text` for a document, and says where it went.
    ///
    /// The copy is written through a temporary and renamed, so an interrupted write leaves no
    /// half-copy behind — a recovery store that could itself be half-written would be a poor place to
    /// look for a lost document.
    pub fn record(
        &self,
        session: &str,
        root: &Path,
        document: &str,
        text: &str,
        written: SystemTime,
    ) -> Result<Recorded, RecoveryError> {
        let directory = self.base.join(sanitize(session)).join(root_hash(root));
        let file = directory.join(document);
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent).map_err(|error| RecoveryError::NotWritten {
                path: parent.to_path_buf(),
                reason: error.to_string(),
            })?;
        }
        let nanos = written
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos() as u64)
            .unwrap_or(0);
        let header = format!(
            "{RECOVERY_MARKER}\nroot = {}\ndocument = {document}\nsession = {session}\nwritten = {nanos}\n\n",
            root.display()
        );
        let bytes = format!("{header}{text}");
        let temporary = match file.file_name() {
            Some(name) => file.with_file_name(format!("{}.copy.tmp", name.to_string_lossy())),
            None => file.with_extension("copy.tmp"),
        };
        std::fs::write(&temporary, bytes.as_bytes()).map_err(|error| {
            RecoveryError::NotWritten {
                path: temporary.clone(),
                reason: error.to_string(),
            }
        })?;
        std::fs::rename(&temporary, &file).map_err(|error| {
            let _ = std::fs::remove_file(&temporary);
            RecoveryError::NotWritten {
                path: file.clone(),
                reason: error.to_string(),
            }
        })?;
        Ok(Recorded {
            path: file,
            document: document.to_owned(),
            session: session.to_owned(),
        })
    }

    /// Every copy in the store, newest first.
    pub fn all(&self) -> Vec<Copy> {
        let mut copies = Vec::new();
        collect(&self.base, &mut copies);
        copies.sort_by(|a, b| {
            b.written()
                .cmp(&a.written())
                .then_with(|| a.document().cmp(b.document()))
        });
        copies
    }

    /// Every copy that belongs to a project, newest first.
    pub fn for_project(&self, root: &Path) -> Vec<Copy> {
        self.all()
            .into_iter()
            .filter(|copy| copy.root() == root)
            .collect()
    }

    /// The newest copy for one document, when there is one.
    pub fn newest(&self, root: &Path, document: &str) -> Option<Copy> {
        self.for_project(root)
            .into_iter()
            .find(|copy| copy.document() == document)
    }

    /// Refuses every copy for one document: they are removed, and how many were removed is returned.
    ///
    /// This is a person's "no" made concrete. It deletes only the copies, never the document: refusing
    /// a recovery must not be a way to lose the file.
    pub fn discard(&self, root: &Path, document: &str) -> usize {
        let mut removed = 0;
        for copy in self.for_project(root) {
            if copy.document() == document && std::fs::remove_file(copy.file()).is_ok() {
                removed += 1;
            }
        }
        removed
    }
}

/// Reads a recovery copy file, or says why it is not one.
fn read_copy(file: &Path) -> Result<Copy, RecoveryError> {
    let bytes = std::fs::read(file).map_err(|error| RecoveryError::NotACopy {
        path: file.to_path_buf(),
        reason: error.to_string(),
    })?;
    let text_bytes = match bytes.windows(2).position(|pair| pair == b"\n\n") {
        Some(at) => at + 2,
        None => {
            return Err(RecoveryError::NotACopy {
                path: file.to_path_buf(),
                reason: "there is no header/body separator".to_owned(),
            });
        }
    };
    let header = String::from_utf8_lossy(&bytes[..text_bytes]);
    let body = String::from_utf8_lossy(&bytes[text_bytes..]).into_owned();
    let mut lines = header.lines();
    match lines.next() {
        Some(marker) if marker == RECOVERY_MARKER => {}
        other => {
            return Err(RecoveryError::NotACopy {
                path: file.to_path_buf(),
                reason: format!("its first line is {other:?}, not {RECOVERY_MARKER:?}"),
            });
        }
    }
    let mut root = None;
    let mut document = None;
    let mut session = None;
    let mut written = None;
    for line in lines {
        let Some((key, value)) = line.split_once(" = ") else {
            continue;
        };
        match key.trim() {
            "root" => root = Some(PathBuf::from(value)),
            "document" => document = Some(value.to_owned()),
            "session" => session = Some(value.to_owned()),
            "written" => written = value.trim().parse::<u64>().ok(),
            _ => {}
        }
    }
    let (Some(root), Some(document), Some(session)) = (root, document, session) else {
        return Err(RecoveryError::NotACopy {
            path: file.to_path_buf(),
            reason: "its header does not name a root, document and session".to_owned(),
        });
    };
    Ok(Copy {
        file: file.to_path_buf(),
        root,
        document,
        session,
        written: written.unwrap_or(0),
        text: body,
    })
}

/// Walks a store directory and collects the copies in it, ignoring anything that is not one.
fn collect(directory: &Path, copies: &mut Vec<Copy>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, copies);
        } else if let Ok(copy) = read_copy(&path) {
            copies.push(copy);
        }
    }
}

/// One directory-name-safe piece of a session name.
fn sanitize(name: &str) -> String {
    name.chars()
        .map(|character| match character {
            '/' | '\\' | ':' | '\0' => '_',
            other => other,
        })
        .collect()
}

/// A stable hash of a project root, so two projects cannot share a file path.
///
/// FNV-1a, written out rather than pulled in: the store only needs the hash to keep two roots apart,
/// and a copy whose header names the wrong root is refused by the header anyway.
fn root_hash(root: &Path) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in root.to_string_lossy().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TempDirectory(PathBuf);

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct Sandbox {
        base: PathBuf,
        store_root: PathBuf,
        project: PathBuf,
        _guard: TempDirectory,
    }

    fn sandbox() -> Sandbox {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let base =
            std::env::temp_dir().join(format!("rustage-recovery-{}-{id}", std::process::id()));
        let store_root = base.join("store");
        let project = base.join("project");
        std::fs::create_dir_all(&project).expect("a project directory");
        Sandbox {
            base: base.clone(),
            store_root,
            project,
            _guard: TempDirectory(base),
        }
    }

    fn at(seconds: u64) -> SystemTime {
        UNIX_EPOCH + std::time::Duration::from_secs(seconds)
    }

    #[test]
    fn a_copy_says_where_it_came_from_and_when() {
        let sandbox = sandbox();
        let store = RecoveryStore::new(&sandbox.store_root);
        let recorded = store
            .record(
                "3",
                &sandbox.project,
                "pages/home.json",
                "hello\n",
                at(1000),
            )
            .expect("written");
        assert!(recorded.path().is_file());

        let copies = store.for_project(&sandbox.project);
        assert_eq!(copies.len(), 1);
        let copy = &copies[0];
        assert_eq!(copy.document(), "pages/home.json");
        assert_eq!(copy.session(), "3");
        assert_eq!(copy.written(), 1000 * 1_000_000_000);
        assert_eq!(copy.text(), "hello\n");
        assert_eq!(copy.standing(), Standing::OnlyCopy);
        assert!(copy.render()[1].contains("session 3"));
    }

    #[test]
    fn the_text_is_kept_byte_for_byte_including_a_leading_blank_line() {
        let sandbox = sandbox();
        let store = RecoveryStore::new(&sandbox.store_root);
        for text in ["\nleading newline\n", "a\n\nb\n", "", "no trailing newline"] {
            store
                .record("1", &sandbox.project, "pages/x.json", text, at(1))
                .expect("written");
            let copy = store
                .newest(&sandbox.project, "pages/x.json")
                .expect("a copy");
            assert_eq!(copy.text(), text, "the body must be verbatim");
        }
    }

    #[test]
    fn standing_says_whether_taking_the_copy_would_change_the_file() {
        let sandbox = sandbox();
        let store = RecoveryStore::new(&sandbox.store_root);
        let file = sandbox.project.join("pages/home.json");
        std::fs::create_dir_all(sandbox.project.join("pages")).expect("a directory");

        store
            .record("1", &sandbox.project, "pages/home.json", "one\n", at(1))
            .expect("written");
        let copy = store
            .newest(&sandbox.project, "pages/home.json")
            .expect("a copy");
        assert_eq!(copy.standing(), Standing::OnlyCopy);

        std::fs::write(&file, "one\n").expect("writable");
        assert_eq!(copy.standing(), Standing::Same);
        assert!(!copy.standing().needs_a_decision());

        std::fs::write(&file, "two\n").expect("writable");
        assert_eq!(copy.standing(), Standing::Differs { current: 4 });
        assert!(copy.standing().needs_a_decision());
    }

    #[test]
    fn the_newest_copy_is_the_one_a_recovery_offers() {
        let sandbox = sandbox();
        let store = RecoveryStore::new(&sandbox.store_root);
        store
            .record("1", &sandbox.project, "pages/home.json", "old\n", at(100))
            .expect("written");
        store
            .record("2", &sandbox.project, "pages/home.json", "new\n", at(200))
            .expect("written");

        let newest = store
            .newest(&sandbox.project, "pages/home.json")
            .expect("a copy");
        assert_eq!(newest.text(), "new\n");
        assert_eq!(newest.session(), "2");
        assert_eq!(store.for_project(&sandbox.project).len(), 2);
    }

    #[test]
    fn refusing_a_recovery_deletes_the_copies_and_never_the_document() {
        let sandbox = sandbox();
        let store = RecoveryStore::new(&sandbox.store_root);
        let file = sandbox.project.join("pages/home.json");
        std::fs::create_dir_all(sandbox.project.join("pages")).expect("a directory");
        std::fs::write(&file, "kept\n").expect("writable");
        store
            .record("1", &sandbox.project, "pages/home.json", "copy\n", at(1))
            .expect("written");

        assert_eq!(store.discard(&sandbox.project, "pages/home.json"), 1);
        assert!(store.newest(&sandbox.project, "pages/home.json").is_none());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "kept\n");
    }

    #[test]
    fn copies_for_two_projects_do_not_collide() {
        let sandbox = sandbox();
        let store = RecoveryStore::new(&sandbox.store_root);
        let other = sandbox.base.join("other");
        std::fs::create_dir_all(&other).expect("a directory");
        store
            .record("1", &sandbox.project, "pages/home.json", "mine\n", at(1))
            .expect("written");
        store
            .record("1", &other, "pages/home.json", "theirs\n", at(1))
            .expect("written");

        assert_eq!(store.for_project(&sandbox.project).len(), 1);
        assert_eq!(
            store
                .newest(&sandbox.project, "pages/home.json")
                .unwrap()
                .text(),
            "mine\n"
        );
        assert_eq!(
            store.newest(&other, "pages/home.json").unwrap().text(),
            "theirs\n"
        );
        assert_eq!(store.all().len(), 2);
    }

    #[test]
    fn a_file_that_is_not_a_copy_is_ignored_rather_than_misread() {
        let sandbox = sandbox();
        let store = RecoveryStore::new(&sandbox.store_root);
        store
            .record("1", &sandbox.project, "pages/home.json", "real\n", at(1))
            .expect("written");
        // Something else lands in the store directory: a user's note, a stray file.
        std::fs::write(sandbox.store_root.join("README"), "not a copy").expect("writable");
        // And one that looks like a copy but has no body.
        std::fs::write(
            sandbox.store_root.join("fake"),
            format!("{RECOVERY_MARKER}\nroot = /x\n"),
        )
        .expect("writable");

        let copies = store.all();
        assert_eq!(copies.len(), 1, "only the real copy is read");
        assert_eq!(copies[0].text(), "real\n");
    }
}
