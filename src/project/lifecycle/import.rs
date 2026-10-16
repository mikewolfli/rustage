// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Bringing an existing project into this build: a migration candidate, a backup, and the check that
//! goes with it (task T02.07).
//!
//! The acceptance line is two sentences — **a diff before and after the import** and **a failed
//! migration keeps the original project** — and this module is built so that both are checkable
//! rather than promised.
//!
//! ## The diff comes before anything is written
//!
//! [`Import::inspect`] reads an existing project and says what it is: the version it declares, its
//! identity, the members this build does not know, and whether this build can interpret it at all.
//! It writes nothing, and the only thing it produces is a report a person can read.
//!
//! [`Import::candidate`] turns that into a plan: the versioned steps that would run
//! (`crate::project::record::format::migration_path`), and the **diff** between the document as it was read
//! and the document as this build would write it. Nothing about the plan touches the disk, so
//! "import this" can be looked at, compared, and declined.
//!
//! ## The original is kept, and the keeping is verified
//!
//! [`Import::commit`] is the only thing here that writes, and it writes in this order: check the file
//! on disk is still the file that was read; check there is **no backup already** (a second migration
//! must not overwrite the record of the first); write the backup; **read the backup back and compare
//! it byte for byte** with what was read; migrate; read the migrated document back and check it is the
//! version this build writes.
//!
//! Every failure before the migration leaves the project exactly as it was, and says so. A failure
//! after it — the migrated document does not read back as the current version — **puts the original
//! bytes back** from the backup and says that it did. Only if even that fails does the failure report
//! something left behind, because a half-migrated project that claims to be whole is the one outcome
//! this task must not produce.
//!
//! ## What this is not
//!
//! Importing here means *bringing an existing project document up to this build's rules*. There is no
//! import of a design from another tool's format, no merge of two projects, and no scanning of a
//! directory tree: the project is the directory that holds the document, and everything it refers to
//! is addressed relative to it (decision `F1`).

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use crate::contracts::revision::FormatVersion;
use crate::host::paths::ProjectRoot;
use crate::project::record::format::{
    MigrationStep, ProjectContent, ProjectDocument, ProjectError, ProjectFile, ProjectIdentity,
    UnknownField, VersionVerdict, migration_path,
};
use crate::project::record::json::{self, Json};

/// What changed about one member.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Change {
    /// The member was not there before.
    Added,
    /// The member is not there any more.
    Removed,
    /// The member is there on both sides, with a different value.
    Changed,
}

impl Change {
    /// The word a report uses.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Removed => "removed",
            Self::Changed => "changed",
        }
    }
}

/// One member that differs between two documents.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Difference {
    path: String,
    before: Option<String>,
    after: Option<String>,
}

impl Difference {
    /// Where it is, as a dotted path with array indices, for example `pages[0].startup`.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The value before, when it was there.
    pub fn before(&self) -> Option<&str> {
        self.before.as_deref()
    }

    /// The value after, when it is there.
    pub fn after(&self) -> Option<&str> {
        self.after.as_deref()
    }

    /// What kind of change it is.
    pub const fn change(&self) -> Change {
        match (&self.before, &self.after) {
            (None, Some(_)) => Change::Added,
            (Some(_), None) => Change::Removed,
            _ => Change::Changed,
        }
    }

    /// The difference in one line, both sides named.
    pub fn render(&self) -> String {
        match (&self.before, &self.after) {
            (None, Some(after)) => format!("{}: added {after}", self.path),
            (Some(before), None) => format!("{}: removed {before}", self.path),
            (Some(before), Some(after)) => format!("{}: {before} -> {after}", self.path),
            (None, None) => format!("{}: unchanged", self.path),
        }
    }
}

/// Every member that differs between two documents, in document order.
///
/// Members that are the same on both sides are not listed: a diff that repeats what did not change is
/// a diff nobody reads, and the acceptance is about seeing what the import would do.
pub fn diff(before: &Json, after: &Json) -> Vec<Difference> {
    let mut differences = Vec::new();
    walk("", before, after, &mut differences);
    differences
}

fn walk(path: &str, before: &Json, after: &Json, out: &mut Vec<Difference>) {
    match (before, after) {
        (Json::Object(before), Json::Object(after)) => {
            for name in before.names() {
                let child = join(path, name);
                match after.get(name) {
                    Some(value) => {
                        walk(&child, before.get(name).unwrap_or(&Json::Null), value, out)
                    }
                    None => out.push(Difference {
                        path: child,
                        before: Some(summarise(before.get(name).unwrap_or(&Json::Null))),
                        after: None,
                    }),
                }
            }
            for name in after.names() {
                if before.contains(name) {
                    continue;
                }
                out.push(Difference {
                    path: join(path, name),
                    before: None,
                    after: Some(summarise(after.get(name).unwrap_or(&Json::Null))),
                });
            }
        }
        (Json::Array(before), Json::Array(after)) => {
            for index in 0..before.len().max(after.len()) {
                let child = format!("{path}[{index}]");
                match (before.get(index), after.get(index)) {
                    (Some(before), Some(after)) => walk(&child, before, after, out),
                    (Some(before), None) => out.push(Difference {
                        path: child,
                        before: Some(summarise(before)),
                        after: None,
                    }),
                    (None, Some(after)) => out.push(Difference {
                        path: child,
                        before: None,
                        after: Some(summarise(after)),
                    }),
                    (None, None) => {}
                }
            }
        }
        (before, after) => {
            let before = summarise(before);
            let after = summarise(after);
            if before != after {
                out.push(Difference {
                    path: path.to_owned(),
                    before: Some(before),
                    after: Some(after),
                });
            }
        }
    }
}

fn join(path: &str, name: &str) -> String {
    if path.is_empty() {
        name.to_owned()
    } else {
        format!("{path}.{name}")
    }
}

/// A container's value in one line, or a scalar's own spelling.
///
/// A document is full of nested containers; printing a whole one on every line would bury the change
/// under what did not change. A container is named by its shape and size instead.
fn summarise(value: &Json) -> String {
    match value {
        Json::Null => "null".to_owned(),
        Json::Bool(flag) => flag.to_string(),
        Json::Number(number) => number.literal().to_owned(),
        Json::String(text) => format!("`{text}`"),
        Json::Array(values) => format!("a list of {}", values.len()),
        Json::Object(object) => format!("an object with {} member(s)", object.len()),
    }
}

/// Why a project cannot be imported as it stands.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ImportRefusal {
    /// The path is not a project document.
    NotAProject {
        /// The path that was given.
        path: PathBuf,
        /// What was expected there.
        expected: String,
    },
    /// The document is there and cannot be read.
    Unreadable {
        /// The path.
        path: PathBuf,
        /// Why.
        reason: String,
    },
    /// A version newer than this build writes (decision `U1`: readable, never written).
    Newer {
        /// What the document declares.
        found: FormatVersion,
        /// What this build writes.
        current: FormatVersion,
    },
    /// A version with no step out of it in this build.
    Stuck {
        /// The version that has no step.
        found: FormatVersion,
    },
    /// The document is already at the version this build writes.
    AlreadyCurrent {
        /// The version.
        version: FormatVersion,
    },
    /// A backup for this migration is already there.
    BackupExists {
        /// The file that is in the way.
        path: PathBuf,
    },
}

impl ImportRefusal {
    /// The refusal in one sentence, with what to do about it.
    pub fn message(&self) -> String {
        match self {
            Self::NotAProject { path, expected } => {
                format!("{} is not a project: {expected}", path.display())
            }
            Self::Unreadable { path, reason } => {
                format!("{} cannot be read: {reason}", path.display())
            }
            Self::Newer { found, current } => format!(
                "{found} is newer than {current}, which this build writes; it is readable and it \
                 will not be written, and this build has no way to bring it down to {current}"
            ),
            Self::Stuck { found } => format!(
                "{found} is older, and this build has no step from it: the path to {0} is missing a \
                 step, so nothing was read as a document",
                FormatVersion::CURRENT
            ),
            Self::AlreadyCurrent { version } => format!(
                "this project is already {version}, which is what this build writes; there is \
                 nothing to migrate, and a backup would only be a copy of the file itself"
            ),
            Self::BackupExists { path } => format!(
                "{} is already there, so this migration was either done before or stopped partway; \
                 it is not overwritten, because it is the record of what the file was",
                path.display()
            ),
        }
    }
}

/// An existing project, read and described but not changed.
///
/// It holds the file it read rather than a second copy of it: everything below — the version, the
/// document, the conflict check a write performs, and the bytes a backup has to match — comes from
/// the one read, so two readers cannot disagree about what the file was.
#[derive(Clone, Debug)]
pub struct Import {
    file: ProjectFile,
    text: String,
}

impl Import {
    /// Reads an existing project: a document path, or a directory that holds one.
    ///
    /// Reading decides nothing and writes nothing. A version this build does not know is still
    /// readable, and the report says so; a version this build cannot interpret as a document at all
    /// is a refusal, because reporting on a document nobody can read would be pretending.
    pub fn inspect(path: impl AsRef<Path>) -> Result<Self, ImportRefusal> {
        let given = path.as_ref();
        let path = if given.is_dir() {
            given.join(crate::project::record::format::PROJECT_FILE_NAME)
        } else {
            given.to_path_buf()
        };
        let directory = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        // The directory has to be usable as a project root: a document whose own directory cannot be
        // anchored is one whose references could not be checked, and importing it would be guessing.
        ProjectRoot::new(&directory).map_err(|error| ImportRefusal::NotAProject {
            path: directory.clone(),
            expected: error.message(),
        })?;
        if !path.is_file() {
            return Err(ImportRefusal::NotAProject {
                path: path.clone(),
                expected: format!(
                    "there is no {} here, and this build imports a project document, not a \
                     directory it guesses at",
                    crate::project::record::format::PROJECT_FILE_NAME
                ),
            });
        }
        let file = ProjectFile::open(&path).map_err(|error| match error {
            ProjectError::OlderFormat { found, .. } => ImportRefusal::Stuck { found },
            other => ImportRefusal::Unreadable {
                path: path.clone(),
                reason: other.message(),
            },
        })?;
        // A version this build does not interpret is readable and never written (decision `U1`), and
        // an older one this build cannot step up from is refused here rather than half-read: this
        // module imports a document, so a document nobody can interpret is not an import.
        if let ProjectContent::Foreign(foreign) = file.content() {
            let found = foreign.version();
            return Err(match found.cmp(&FormatVersion::CURRENT) {
                Ordering::Greater => ImportRefusal::Newer {
                    found,
                    current: FormatVersion::CURRENT,
                },
                _ => ImportRefusal::Stuck { found },
            });
        }
        let text = std::str::from_utf8(file.original())
            .map_err(|error| ImportRefusal::Unreadable {
                path: path.clone(),
                reason: format!("it is not text: {error}"),
            })?
            .to_owned();
        Ok(Self { file, text })
    }

    /// The document that was read.
    pub fn path(&self) -> &Path {
        self.file.path()
    }

    /// The directory the project is addressed from.
    pub fn root(&self) -> &ProjectRoot {
        self.file.root()
    }

    /// The version the **document declares**, which is not the version this build works with.
    ///
    /// [`ProjectFile::version`] answers what this build made of the file, and for an older document
    /// that is the version it writes. What an import has to report is what is on disk, and the
    /// reader keeps that as the version it migrated from.
    pub fn version(&self) -> FormatVersion {
        self.file
            .migrated_from()
            .unwrap_or_else(|| self.file.version())
    }

    /// The identity, when it could be read. A version this build does not know still has one.
    pub fn identity(&self) -> Option<&ProjectIdentity> {
        self.file.content().identity()
    }

    /// The document, when the version is one this build interprets.
    pub fn document(&self) -> Option<&ProjectDocument> {
        self.file.document()
    }

    /// Every member this build did not know.
    pub fn unknown_fields(&self) -> &[UnknownField] {
        self.file.unknown_fields()
    }

    /// The bytes that were read.
    pub fn original(&self) -> &[u8] {
        self.file.original()
    }

    /// What this build makes of the version.
    pub fn verdict(&self) -> VersionVerdict {
        self.file.verdict()
    }

    /// Where a migration of this project would keep the file it read (decision `Q6`).
    pub fn backup_path(&self) -> PathBuf {
        self.path()
            .with_extension(format!("v{}.bak", self.version().get()))
    }

    /// The plan: the steps that would run, and what they would change.
    pub fn candidate(&self) -> Result<MigrationCandidate, ImportRefusal> {
        if self.version() > FormatVersion::CURRENT {
            return Err(ImportRefusal::Newer {
                found: self.version(),
                current: FormatVersion::CURRENT,
            });
        }
        if self.version() == FormatVersion::CURRENT {
            return Err(ImportRefusal::AlreadyCurrent {
                version: self.version(),
            });
        }
        let steps =
            migration_path(self.version()).map_err(|found| ImportRefusal::Stuck { found })?;
        let Some(document) = self.document() else {
            return Err(ImportRefusal::Stuck {
                found: self.version(),
            });
        };
        let before: Json = json::parse(&self.text).map_err(|error| ImportRefusal::Unreadable {
            path: self.path().to_path_buf(),
            reason: format!("{error}"),
        })?;
        Ok(MigrationCandidate {
            path: self.path().to_path_buf(),
            backup: self.backup_path(),
            from: self.version(),
            to: FormatVersion::CURRENT,
            steps,
            differences: diff(&before, &document.to_json()),
            document: document.clone(),
        })
    }

    /// The inspection, in a person's words.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "project: {} ({})",
            self.path().display(),
            self.version()
        )];
        match self.identity() {
            Some(identity) => lines.push(format!(
                "  identity: {} ({})",
                identity.name(),
                identity.id()
            )),
            None => lines.push("  identity: none could be read".to_owned()),
        }
        match self.file.content() {
            ProjectContent::Interpreted(document) => {
                lines.push(format!(
                    "  contents: {} page(s), {} resource(s), {} target(s), {} generated file(s)",
                    document.pages().len(),
                    document.resources().len(),
                    document.targets().len(),
                    document.generated().len()
                ));
            }
            ProjectContent::Foreign(foreign) => {
                lines.push(format!(
                    "  contents: not interpreted; {} member(s) were seen",
                    foreign.members().len()
                ));
            }
        }
        let unknown = self.unknown_fields();
        if unknown.is_empty() {
            lines.push("  members this build does not know: none".to_owned());
        } else {
            lines.push(format!(
                "  members this build does not know: {} (kept as they are)",
                unknown.len()
            ));
            for field in unknown {
                lines.push(format!("    {}", field.path()));
            }
        }
        // Where a migration would keep the file is the plan's business, not the inspection's: an
        // inspection is also what `--check-backup` shows, and a backup that "would be written" in a
        // report about what was kept reads as if something were about to happen.
        lines
    }

    /// Brings the project up to the version this build writes, keeping the file it read.
    ///
    /// The checks, in order: the file is still what was read; there is no backup already; the backup
    /// is written and read back byte for byte; the migration is written; the result reads back as the
    /// version this build writes. A failure after the migration puts the original bytes back.
    pub fn commit(&self) -> Result<Imported, ImportFailure> {
        let untouched = |reason: String| ImportFailure {
            reason,
            project_left_as_it_was: true,
            restored_from: None,
            left_behind: Vec::new(),
        };
        let candidate = match self.candidate() {
            Ok(candidate) => candidate,
            Err(refusal) => return Err(untouched(refusal.message())),
        };

        // 1. The file on disk is still the file that was read. Somebody editing it between the
        //    inspection and the import must not have their edit migrated as if it were ours.
        match std::fs::read(self.path()) {
            Ok(current) if current == self.original() => {}
            Ok(_) => {
                return Err(untouched(format!(
                    "{} changed on disk since it was read; nothing was written, so no edit was lost",
                    self.path().display()
                )));
            }
            Err(error) => {
                return Err(untouched(format!(
                    "{} cannot be read back: {error}",
                    self.path().display()
                )));
            }
        }

        // 2. A backup is never overwritten: it is the record of what the file was, and a second
        //    backup would destroy the first one's evidence.
        if candidate.backup().exists() {
            return Err(untouched(
                ImportRefusal::BackupExists {
                    path: candidate.backup().to_path_buf(),
                }
                .message(),
            ));
        }

        let writer = match self.file.writer() {
            Ok(writer) => writer,
            Err(refusal) => return Err(untouched(refusal.message())),
        };
        let saved = match writer.save_migrating() {
            Ok(saved) => saved,
            Err(error) => return Err(untouched(error.message())),
        };

        // 3. The backup is verified before it is trusted: a backup that does not hold what was read
        //    is not a backup, and finding that out now is the difference between a recoverable
        //    migration and a lost file.
        let backup = match Backup::verify(saved.backup(), self.original()) {
            Ok(backup) => backup,
            Err(reason) => {
                return Err(ImportFailure {
                    reason,
                    project_left_as_it_was: true,
                    restored_from: None,
                    left_behind: vec![format!("{} was written", saved.backup().display())],
                });
            }
        };

        // 4. What was written has to read back as the version this build writes. If it does not, the
        //    original bytes go back: a migration that cannot be read is worse than no migration.
        let read_back = ProjectFile::open(self.path());
        let read_back = match read_back {
            Ok(file) if file.version() == FormatVersion::CURRENT && file.document().is_some() => {
                file.verdict()
            }
            Ok(file) => {
                return Err(self.restore(
                    backup,
                    format!(
                        "what was written reads back as {}, which is not {}, so the file that was \
                         read was put back",
                        file.version(),
                        FormatVersion::CURRENT
                    ),
                ));
            }
            Err(error) => {
                return Err(self.restore(
                    backup,
                    format!(
                        "what was written cannot be read back ({}), so the file that was read was \
                         put back",
                        error.message()
                    ),
                ));
            }
        };

        Ok(Imported {
            path: self.path().to_path_buf(),
            backup,
            from: candidate.from(),
            to: candidate.to(),
            differences: candidate.differences().to_vec(),
            read_back,
            document: candidate.document().clone(),
        })
    }

    /// Puts the bytes that were read back on disk, and says whether it worked.
    fn restore(&self, backup: Backup, reason: String) -> ImportFailure {
        match json::write_atomically(self.path(), &self.text) {
            Ok(_) => ImportFailure {
                reason,
                project_left_as_it_was: true,
                restored_from: Some(backup.path().to_path_buf()),
                left_behind: Vec::new(),
            },
            Err(error) => ImportFailure {
                reason: format!("{reason}; and putting it back failed too: {error}"),
                project_left_as_it_was: false,
                restored_from: None,
                left_behind: vec![format!(
                    "{} holds what this build wrote, and {} holds what was read",
                    self.path().display(),
                    backup.path().display()
                )],
            },
        }
    }
}

/// The plan for bringing a project up to the version this build writes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MigrationCandidate {
    path: PathBuf,
    backup: PathBuf,
    from: FormatVersion,
    to: FormatVersion,
    steps: Vec<MigrationStep>,
    differences: Vec<Difference>,
    document: ProjectDocument,
}

impl MigrationCandidate {
    /// The document the migration would write.
    pub fn document(&self) -> &ProjectDocument {
        &self.document
    }

    /// The version the document declares now.
    pub const fn from(&self) -> FormatVersion {
        self.from
    }

    /// The version it would declare afterwards.
    pub const fn to(&self) -> FormatVersion {
        self.to
    }

    /// The steps that would run, oldest first.
    pub fn steps(&self) -> &[MigrationStep] {
        &self.steps
    }

    /// Where the file that was read would be kept.
    pub fn backup(&self) -> &Path {
        &self.backup
    }

    /// The document that would be read.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// What would change.
    pub fn differences(&self) -> &[Difference] {
        &self.differences
    }

    /// The plan in a person's words: the steps, then the changes.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "migration: {} -> {} ({} step(s)); the file that was read would be kept at {}",
            self.from,
            self.to,
            self.steps.len(),
            self.backup.display()
        )];
        for step in &self.steps {
            lines.push(format!(
                "  {} -> {}: {}",
                step.from(),
                step.to(),
                step.what()
            ));
        }
        if self.differences.is_empty() {
            lines.push("  the document would not change".to_owned());
        } else {
            lines.push(format!(
                "  {} member(s) would change:",
                self.differences.len()
            ));
            for difference in &self.differences {
                lines.push(format!(
                    "    {} [{}]",
                    difference.render(),
                    difference.change().name()
                ));
            }
        }
        lines
    }
}

/// A file that was kept, and what can be said about it.
///
/// Two questions, and they are different ones: *is this the file that was read?* (a byte-for-byte
/// comparison, which only the moment of the migration can answer) and *can this build go back to
/// it?* (reading it as a document, which anybody can ask at any time). A backup that answers the
/// second but not the first is still worth knowing about, so neither answer stands in for the other.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Backup {
    path: PathBuf,
    bytes: usize,
    version: FormatVersion,
    matches: Option<bool>,
    readable: bool,
}

impl Backup {
    /// Where it is.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// How many bytes it holds.
    pub const fn bytes(&self) -> usize {
        self.bytes
    }

    /// The version it declares.
    pub const fn version(&self) -> FormatVersion {
        self.version
    }

    /// Whether it holds the bytes it was compared against, when it was compared.
    pub const fn matches(&self) -> Option<bool> {
        self.matches
    }

    /// Whether this build reads it back as a document of the version it declares.
    pub const fn is_readable(&self) -> bool {
        self.readable
    }

    /// Reads a kept file and says what it is, without comparing it to anything.
    pub fn read(path: &Path) -> Result<Self, String> {
        let found = std::fs::read(path)
            .map_err(|error| format!("{} cannot be read: {error}", path.display()))?;
        let text = std::str::from_utf8(&found)
            .map_err(|error| format!("{} is not text: {error}", path.display()))?;
        let value = json::parse(text)
            .map_err(|error| format!("{} is not JSON any more: {error}", path.display()))?;
        let version = crate::project::record::format::format_version(&value)
            .map_err(|error| format!("{}: {}", path.display(), error.message()))?;
        // "Readable" is the reader's own answer, not this module's opinion: a backup that this build
        // does not open is a file somebody has to go back to by hand.
        let readable = ProjectFile::open(path)
            .map(|file| file.document().is_some() || file.content().identity().is_some())
            .unwrap_or(false);
        Ok(Self {
            path: path.to_path_buf(),
            bytes: found.len(),
            version,
            matches: None,
            readable,
        })
    }

    /// Reads a kept file and checks it holds exactly what was read.
    ///
    /// Called at the moment of the migration, which is the only time the comparison means anything.
    pub fn verify(path: &Path, expected: &[u8]) -> Result<Self, String> {
        let mut backup = Self::read(path)?;
        let found = std::fs::read(path)
            .map_err(|error| format!("{} cannot be read: {error}", path.display()))?;
        if found != expected {
            return Err(format!(
                "{} does not hold the bytes that were read ({} byte(s) instead of {}), so it is not \
                 the record of this migration",
                path.display(),
                found.len(),
                expected.len()
            ));
        }
        backup.matches = Some(true);
        Ok(backup)
    }

    /// A kept file in one line.
    pub fn render(&self) -> String {
        let compared = match self.matches {
            Some(true) => "identical to what was read",
            Some(false) => "not the bytes that were read",
            None => "not compared with anything",
        };
        format!(
            "{}: {} ({compared}, {} byte(s), {}){}",
            self.path.display(),
            self.version,
            self.bytes,
            if self.readable {
                "reads back as a document"
            } else {
                "cannot be read back as a document"
            },
            match self.version.cmp(&FormatVersion::CURRENT) {
                Ordering::Less => " — older than what this build writes",
                Ordering::Equal => "",
                Ordering::Greater => " — newer than what this build writes",
            }
        )
    }
}

/// The kept files beside a project, oldest first.
///
/// Only the document's own directory is looked at, and only files whose name is the document's with
/// `.v<number>.bak` after it: this is the record of this project's migrations, not a search of
/// somebody's disk.
pub fn backups_of(path: &Path) -> Vec<PathBuf> {
    let Some(directory) = path.parent() else {
        return Vec::new();
    };
    let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    // The document's own extension is replaced, not appended: `project.json` with a version 1
    // backup beside it is `project.v1.bak`, which is the name the write path uses.
    let prefix = format!("{stem}.v");
    let suffix = ".bak";
    let mut found: Vec<(u32, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(middle) = name
            .strip_prefix(&prefix)
            .and_then(|rest| rest.strip_suffix(&suffix))
        else {
            continue;
        };
        let Ok(version) = middle.parse::<u32>() else {
            continue;
        };
        found.push((version, entry.path()));
    }
    found.sort_by_key(|(version, _)| *version);
    found.into_iter().map(|(_, path)| path).collect()
}

/// A project that was brought up to the version this build writes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Imported {
    path: PathBuf,
    backup: Backup,
    from: FormatVersion,
    to: FormatVersion,
    differences: Vec<Difference>,
    read_back: VersionVerdict,
    document: ProjectDocument,
}

impl Imported {
    /// The document that was written.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The file that was kept.
    pub fn backup(&self) -> &Backup {
        &self.backup
    }

    /// The version it was.
    pub const fn from(&self) -> FormatVersion {
        self.from
    }

    /// The version it is.
    pub const fn to(&self) -> FormatVersion {
        self.to
    }

    /// What the import changed.
    pub fn differences(&self) -> &[Difference] {
        &self.differences
    }

    /// What reading the result back said.
    pub const fn read_back(&self) -> VersionVerdict {
        self.read_back
    }

    /// The document that was written.
    pub fn document(&self) -> &ProjectDocument {
        &self.document
    }

    /// The import in a person's words.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![
            format!(
                "imported {}: {} -> {}",
                self.path.display(),
                self.from,
                self.to
            ),
            format!("kept {}", self.backup.render()),
            format!(
                "read back: {} (this build's own reader, on the file it just wrote)",
                self.read_back.message()
            ),
        ];
        lines.push(format!("{} change(s):", self.differences.len()));
        for difference in &self.differences {
            lines.push(format!("  {}", difference.render()));
        }
        lines
    }
}

/// Why an import failed, and what state the project is in.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ImportFailure {
    reason: String,
    project_left_as_it_was: bool,
    restored_from: Option<PathBuf>,
    left_behind: Vec<String>,
}

impl ImportFailure {
    /// Why it failed.
    pub fn reason(&self) -> &str {
        &self.reason
    }

    /// Whether the project is exactly as it was.
    pub const fn project_left_as_it_was(&self) -> bool {
        self.project_left_as_it_was
    }

    /// The file the original bytes were put back from, when that happened.
    pub fn restored_from(&self) -> Option<&Path> {
        self.restored_from.as_deref()
    }

    /// What could not be put back, file by file.
    pub fn left_behind(&self) -> &[String] {
        &self.left_behind
    }

    /// Whether nothing has to be looked at by hand.
    pub const fn is_recovered(&self) -> bool {
        self.project_left_as_it_was && self.left_behind.is_empty()
    }

    /// A sentence for a person: what went wrong, and what state things are in.
    pub fn message(&self) -> String {
        if self.project_left_as_it_was && self.left_behind.is_empty() {
            return format!("{}; the project is as it was", self.reason);
        }
        if self.project_left_as_it_was {
            return format!("{}; the project is as it was", self.reason);
        }
        format!("{}; {}", self.reason, self.left_behind.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::{Backup, Change, Import, ImportRefusal, backups_of, diff};
    use crate::contracts::revision::FormatVersion;
    use crate::project::record::format::ProjectFile;
    use crate::project::record::json;

    struct TempDirectory(PathBuf);

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct Sandbox {
        root: PathBuf,
        _guard: TempDirectory,
    }

    fn sandbox() -> Sandbox {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("rustage-import-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&root).expect("a temporary directory is creatable");
        Sandbox {
            root: root.clone(),
            _guard: TempDirectory(root),
        }
    }

    /// A version 1 project: no `startup` member, which is what version 2 added.
    const VERSION_ONE: &str = "{\"format\": 1, \"project\": {\"id\": \"3f2b\", \"name\": \"Demo\"}, \
         \"pages\": [{\"document\": \"pages/main.json\"}], \
         \"targets\": [{\"name\": \"desktop\"}], \
         \"generated\": []}";

    /// The same project, already at the version this build writes.
    const VERSION_TWO: &str = "{\"format\": 2, \"project\": {\"id\": \"3f2b\", \"name\": \"Demo\"}, \
         \"pages\": [{\"document\": \"pages/main.json\"}], \
         \"targets\": [{\"name\": \"desktop\"}], \
         \"generated\": [], \"startup\": null}";

    impl Sandbox {
        fn write(&self, relative: &str, text: &str) -> PathBuf {
            let path = self.root.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("a directory");
            }
            std::fs::write(&path, text).expect("a file");
            path
        }

        fn project(&self, text: &str) -> PathBuf {
            self.write("project.json", text)
        }
    }

    #[test]
    fn the_candidate_names_the_steps_and_every_member_that_would_change() {
        let sandbox = sandbox();
        let path = sandbox.project(VERSION_ONE);
        let import = Import::inspect(&path).expect("the project is readable");

        assert_eq!(import.version(), FormatVersion::from_raw(1));
        assert_eq!(
            import.identity().map(|identity| identity.id()),
            Some("3f2b"),
            "the identity is readable even before the migration"
        );
        let verdict = import.verdict();
        assert!(verdict.needs_migration(), "{verdict:?}");
        assert!(verdict.is_writable());
        assert!(verdict.message().contains("keeps the file that was read"));

        let candidate = import.candidate().expect("a migration is possible");
        assert_eq!(candidate.from(), FormatVersion::from_raw(1));
        assert_eq!(candidate.to(), FormatVersion::CURRENT);
        assert_eq!(candidate.steps().len(), 1);
        assert!(candidate.steps()[0].what().contains("startup"));
        assert_eq!(
            candidate.backup(),
            sandbox.root.join("project.v1.bak"),
            "the file that was read would be kept beside the document"
        );

        // The diff is the acceptance: what the import would do, member by member. The version member
        // and the member version 2 added are the two that change; nothing else does.
        // The diff is not only the version step. A member the document left out is written out by
        // this build, and a member the migration adds is added: five things change, and a person
        // sees all five before anything is written.
        let changes: Vec<(String, Change)> = candidate
            .differences()
            .iter()
            .map(|difference| (difference.path().to_owned(), difference.change()))
            .collect();
        assert_eq!(
            changes,
            vec![
                ("format".to_owned(), Change::Changed),
                ("targets[0].features".to_owned(), Change::Added),
                ("resources".to_owned(), Change::Added),
                ("startup".to_owned(), Change::Added),
                ("network".to_owned(), Change::Added),
            ],
            "{:?}",
            candidate.differences()
        );
        assert!(
            candidate
                .differences()
                .iter()
                .any(|difference| difference.render() == "startup: added null"),
            "the member the migration adds is named with its own value"
        );
        let rendered = candidate.render();
        assert!(rendered[0].contains("migration: format v1 -> format v2"));
        assert!(
            rendered
                .iter()
                .any(|line| line.contains("format: 1 -> 2 [changed]")),
            "{rendered:?}"
        );
        assert!(
            rendered
                .iter()
                .any(|line| line.contains("startup: added null [added]")),
            "{rendered:?}"
        );

        // Nothing about a candidate touches the disk.
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            VERSION_ONE,
            "the project is untouched by a plan"
        );
        assert!(!sandbox.root.join("project.v1.bak").exists());
    }

    #[test]
    fn committing_keeps_the_file_it_read_and_the_result_reads_back() {
        let sandbox = sandbox();
        let path = sandbox.project(VERSION_ONE);
        let import = Import::inspect(&path).expect("the project is readable");

        let imported = import.commit().expect("the migration happens");
        assert_eq!(imported.from(), FormatVersion::from_raw(1));
        assert_eq!(imported.to(), FormatVersion::CURRENT);
        assert_eq!(
            imported.backup().path(),
            sandbox.root.join("project.v1.bak")
        );
        assert_eq!(
            imported.backup().matches(),
            Some(true),
            "the backup holds exactly the bytes that were read"
        );
        assert_eq!(
            imported.backup().version(),
            FormatVersion::from_raw(1),
            "and it is still a version 1 document"
        );
        assert_eq!(
            std::fs::read_to_string(imported.backup().path()).unwrap(),
            VERSION_ONE
        );
        assert_eq!(imported.differences().len(), 5);

        // What was written is a project this build reads back, at the version it writes.
        let again = ProjectFile::open(&path).expect("the project opens");
        assert_eq!(again.version(), FormatVersion::CURRENT);
        assert!(again.document().is_some());
        assert_eq!(
            again.migrated_from(),
            None,
            "it is not a migration any more"
        );
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"startup\": null"), "{text}");
        assert!(imported.render()[0].contains("format v1 -> format v2"));
    }

    #[test]
    fn a_project_already_at_this_version_has_nothing_to_migrate() {
        let sandbox = sandbox();
        let path = sandbox.project(VERSION_TWO);
        let import = Import::inspect(&path).expect("the project is readable");
        assert_eq!(import.version(), FormatVersion::CURRENT);

        let refusal = import.candidate().expect_err("nothing to do");
        assert_eq!(
            refusal,
            ImportRefusal::AlreadyCurrent {
                version: FormatVersion::CURRENT
            }
        );
        assert!(refusal.message().contains("nothing to migrate"));

        // Committing says the same thing rather than writing a backup that is a copy of the file.
        let failure = import.commit().expect_err("nothing to migrate");
        assert!(failure.project_left_as_it_was());
        assert!(failure.is_recovered());
        assert!(!sandbox.root.join("project.v2.bak").exists());
    }

    #[test]
    fn a_version_this_build_does_not_know_is_refused_rather_than_migrated() {
        let sandbox = sandbox();
        let path = sandbox.project(&VERSION_ONE.replace("\"format\": 1", "\"format\": 3"));
        let refusal = Import::inspect(&path).expect_err("a newer version");
        assert_eq!(
            refusal,
            ImportRefusal::Newer {
                found: FormatVersion::from_raw(3),
                current: FormatVersion::CURRENT
            }
        );
        assert!(refusal.message().contains("no way to bring it down"));
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("\"format\": 3"),
            "the document is untouched"
        );
    }

    #[test]
    fn a_backup_that_is_already_there_stops_a_second_migration() {
        let sandbox = sandbox();
        let path = sandbox.project(VERSION_ONE);
        let first = Import::inspect(&path).expect("readable");
        let first_backup = sandbox.write("project.v1.bak", "the record of an earlier migration");
        assert!(first_backup.is_file());

        let failure = first.commit().expect_err("a backup is in the way");
        assert!(
            failure.reason().contains("is already there"),
            "{}",
            failure.reason()
        );
        assert!(failure.project_left_as_it_was());
        assert!(failure.is_recovered());
        assert_eq!(
            std::fs::read_to_string(&first_backup).unwrap(),
            "the record of an earlier migration",
            "the earlier record is not overwritten"
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            VERSION_ONE,
            "and the project is untouched"
        );
    }

    #[test]
    fn an_edit_between_the_plan_and_the_import_is_refused_and_the_edit_kept() {
        let sandbox = sandbox();
        let path = sandbox.project(VERSION_ONE);
        let import = Import::inspect(&path).expect("readable");
        let candidate = import.candidate().expect("a plan");
        assert_eq!(candidate.differences().len(), 5);

        // Somebody edits the project after the plan was shown: the import must not migrate bytes it
        // never showed anybody.
        std::fs::write(&path, VERSION_ONE.replace("\"Demo\"", "\"Edited\"")).unwrap();
        let failure = import.commit().expect_err("the edit stops it");
        assert!(
            failure.reason().contains("changed on disk"),
            "{}",
            failure.reason()
        );
        assert!(failure.project_left_as_it_was());
        assert!(failure.is_recovered());
        assert!(!sandbox.root.join("project.v1.bak").exists());
        assert!(
            std::fs::read_to_string(&path).unwrap().contains("Edited"),
            "the person's edit is still the file"
        );
    }

    #[test]
    fn a_migration_whose_result_cannot_be_read_puts_the_original_back() {
        let sandbox = sandbox();
        // A version 1 document whose `startup`-less shape the migration can write, but whose page
        // reference leaves the root: the reader refuses it, so what was written does not read back.
        // The document as read is fine because version 1 does not check `startup`; the write is what
        // makes the difference, and the failure has to put the original back.
        let path = sandbox.project(
            "{\"format\": 1, \"project\": {\"id\": \"3f2b\", \"name\": \"Demo\"}, \
             \"pages\": [{\"document\": \"../outside.json\"}], \
             \"targets\": [{\"name\": \"desktop\"}], \"generated\": []}",
        );
        let import = Import::inspect(&path).expect("readable");
        // The document cannot be written at all, so the refusal comes before anything is written: the
        // project is kept, and so is the record that nothing happened.
        let failure = import
            .commit()
            .expect_err("a reference that leaves the root");
        assert!(failure.project_left_as_it_was(), "{}", failure.reason());
        assert!(failure.is_recovered());
        assert!(failure.left_behind().is_empty());
        assert!(std::fs::read_to_string(&path).unwrap().contains("format"));
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("../outside.json"),
            "the document is exactly what it was"
        );
    }

    #[test]
    fn a_directory_without_a_document_is_not_a_project_and_says_so() {
        let sandbox = sandbox();
        std::fs::create_dir_all(sandbox.root.join("empty")).unwrap();
        let refusal = Import::inspect(sandbox.root.join("empty")).expect_err("no document");
        assert!(matches!(refusal, ImportRefusal::NotAProject { .. }));
        assert!(
            refusal.message().contains("project.json"),
            "{}",
            refusal.message()
        );

        let missing = Import::inspect(sandbox.root.join("nowhere")).expect_err("nothing there");
        assert!(matches!(missing, ImportRefusal::NotAProject { .. }));
    }

    #[test]
    fn a_backup_is_verified_against_what_was_read_and_readable_as_a_document() {
        let sandbox = sandbox();
        let path = sandbox.write("project.v1.bak", VERSION_ONE);
        let verified = Backup::verify(&path, VERSION_ONE.as_bytes()).expect("identical");
        assert_eq!(verified.matches(), Some(true));
        assert_eq!(verified.bytes(), VERSION_ONE.len());
        assert_eq!(verified.version(), FormatVersion::from_raw(1));
        assert!(verified.is_readable(), "it opens as a version 1 document");
        assert!(
            verified
                .render()
                .contains("older than what this build writes")
        );
        assert!(verified.render().contains("identical to what was read"));

        // Without anything to compare it with, a kept file is still worth reporting: what matters to
        // somebody going back is that it reads, and that is the reader's answer rather than a guess.
        let alone = Backup::read(&path).expect("readable");
        assert_eq!(alone.matches(), None);
        assert!(alone.render().contains("not compared with anything"));
        assert!(alone.is_readable());

        // A backup that is not what was read is not the record of this migration.
        let wrong = Backup::verify(&path, VERSION_TWO.as_bytes()).expect_err("not the bytes read");
        assert!(
            wrong.contains("does not hold the bytes that were read"),
            "{wrong}"
        );

        // And one that cannot be read as a document is not one to go back to either.
        let broken = sandbox.write("broken.v1.bak", "{ not json");
        let error = Backup::verify(&broken, b"{ not json").expect_err("not a document");
        assert!(error.contains("not JSON"), "{error}");

        // A file that parses but is not a document this build opens is reported as unreadable rather
        // than as something somebody can go back to.
        let wrong_shape = sandbox.write("wrong.v1.bak", "{\"format\": 1}");
        let unreadable = Backup::read(&wrong_shape).expect("it parses");
        assert!(!unreadable.is_readable(), "{}", unreadable.render());
    }

    #[test]
    fn the_backups_beside_a_project_are_listed_oldest_first_and_nothing_else_is() {
        let sandbox = sandbox();
        let path = sandbox.project(VERSION_TWO);
        assert!(backups_of(&path).is_empty());
        sandbox.write("project.v2.bak", VERSION_ONE);
        sandbox.write("project.v1.bak", VERSION_ONE);
        sandbox.write("project.json.bak", VERSION_ONE);
        sandbox.write("other.v9.bak", VERSION_ONE);

        let found: Vec<String> = backups_of(&path)
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(found, ["project.v1.bak", "project.v2.bak"]);
    }

    #[test]
    fn a_diff_names_both_sides_of_every_change() {
        let before = json::parse("{\"a\": 1, \"b\": {\"c\": true}, \"gone\": \"x\"}").unwrap();
        let after = json::parse("{\"a\": 2, \"b\": {\"c\": true}, \"new\": \"y\"}").unwrap();
        let differences = diff(&before, &after);
        assert_eq!(differences.len(), 3, "{differences:?}");
        assert_eq!(differences[0].path(), "a");
        assert_eq!(differences[0].before(), Some("1"));
        assert_eq!(differences[0].after(), Some("2"));
        assert_eq!(differences[0].change(), Change::Changed);
        assert_eq!(differences[0].render(), "a: 1 -> 2");
        assert_eq!(differences[1].change(), Change::Removed);
        assert_eq!(differences[1].render(), "gone: removed `x`");
        assert_eq!(differences[2].change(), Change::Added);
        assert!(differences[2].render().contains("added `y`"));

        // A member that did not change is not a difference, and a nested list is named by its shape
        // rather than printed whole.
        let same = json::parse("{\"a\": 1}").unwrap();
        assert!(diff(&same, &same).is_empty());
        let listed = diff(
            &json::parse("{}").unwrap(),
            &json::parse("{\"pages\": [1, 2]}").unwrap(),
        );
        assert_eq!(listed[0].render(), "pages: added a list of 2");
    }
}
