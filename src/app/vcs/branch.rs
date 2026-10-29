// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! What a branch switch invalidates (task T20.05).
//!
//! A switch changes the files under an open design, so three facts stay separate:
//!
//! * **An unsaved buffer is a refusal.** The text of an open document is not in Git and no stash can
//!   hold it; [`refuse_dirty_buffers`] stops the switch instead of letting a later save write one
//!   branch's text over another branch's file.
//! * **The model is read again, not carried across.** [`Snapshot::read`] reads the project document
//!   on each side of the switch and [`compare`] names what must be reloaded and which target state
//!   cannot be reused. A switch never reports the old target selection as still valid.
//! * **No project file is a fact, not an empty model.** [`Snapshot::read_optional`] separates "there
//!   is no file on this branch" from "the file cannot be read", so an unreadable project is a
//!   diagnosable failure rather than a project with nothing in it.

use std::path::{Path, PathBuf};

use crate::contracts::revision::FormatVersion;
use crate::project::record::format::ProjectFile;

/// One target profile, as the project document declares it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Target {
    name: String,
    profile: Option<String>,
    features: Vec<String>,
}

impl Target {
    /// What the target is called.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The build profile it names, when it names one.
    pub fn profile(&self) -> Option<&str> {
        self.profile.as_deref()
    }

    /// The features it enables, in document order.
    pub fn features(&self) -> &[String] {
        &self.features
    }

    /// The target in one line, for a reload report.
    fn render(&self) -> String {
        let profile = match self.profile() {
            Some(profile) => format!("profile {profile}"),
            None => "no profile".to_owned(),
        };
        if self.features().is_empty() {
            format!("{} ({profile})", self.name())
        } else {
            format!(
                "{} ({profile}, features {})",
                self.name(),
                self.features().join(",")
            )
        }
    }
}

/// The project facts a switch can invalidate.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Snapshot {
    path: PathBuf,
    version: FormatVersion,
    migrated_from: Option<FormatVersion>,
    targets: Vec<Target>,
    pages: Vec<String>,
    generated: Vec<String>,
}

impl Snapshot {
    /// Reads the project document at `path`.
    ///
    /// A document this build does not interpret is an error, not a snapshot with empty lists: the
    /// caller must not report "nothing changed" about a file it could not read.
    pub fn read(path: &Path) -> Result<Self, String> {
        let file = ProjectFile::open(path).map_err(|error| {
            format!("{} could not be read: {}", path.display(), error.message())
        })?;
        Self::from_file(path, &file)
    }

    /// Reads the project document at `path`, answering `None` when the path is not there at all.
    ///
    /// A path that exists but cannot be read is still an error. That is what keeps a branch without
    /// a project file distinct from a project file with no targets.
    pub fn read_optional(path: &Path) -> Result<Option<Self>, String> {
        match std::fs::metadata(path) {
            Ok(metadata) if metadata.is_file() => {}
            Ok(_) => return Err(format!("{} is not a file", path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(format!("{} could not be read: {error}", path.display()));
            }
        }
        Self::read(path).map(Some)
    }

    fn from_file(path: &Path, file: &ProjectFile) -> Result<Self, String> {
        let document = file.document().ok_or_else(|| {
            format!(
                "{} cannot be interpreted: {}",
                path.display(),
                file.verdict().message()
            )
        })?;
        Ok(Self {
            path: path.to_path_buf(),
            version: file.version(),
            migrated_from: file.migrated_from(),
            targets: document
                .targets()
                .iter()
                .map(|target| Target {
                    name: target.name().to_owned(),
                    profile: target.profile().map(str::to_owned),
                    features: target.features().to_vec(),
                })
                .collect(),
            pages: document
                .pages()
                .iter()
                .map(|page| page.document().written().to_owned())
                .collect(),
            generated: document
                .generated()
                .iter()
                .map(|generated| generated.output().written().to_owned())
                .collect(),
        })
    }

    /// The file this was read from.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The format the document declares.
    pub fn version(&self) -> FormatVersion {
        self.version
    }

    /// The target profiles, in document order.
    pub fn targets(&self) -> &[Target] {
        &self.targets
    }
}

/// What a switch requires the host to redo.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Invalidation {
    reload: Vec<String>,
    invalidated: Vec<String>,
    migration: Option<String>,
}

impl Invalidation {
    /// Work the host has to repeat because the files underneath it changed.
    pub fn reload(&self) -> &[String] {
        &self.reload
    }

    /// Old state that must not be reused on the new branch.
    pub fn invalidated(&self) -> &[String] {
        &self.invalidated
    }

    /// The migration a project on the new branch needs before it may be written.
    pub fn migration(&self) -> Option<&str> {
        self.migration.as_deref()
    }

    /// Whether the model and the target state remained exactly valid.
    pub fn keeps_state(&self) -> bool {
        self.reload.is_empty() && self.invalidated.is_empty() && self.migration.is_none()
    }

    /// The report, one line per fact.
    pub fn render(&self, from: &str, to: &str) -> Vec<String> {
        let mut lines = Vec::new();
        if self.keeps_state() {
            lines.push(format!(
                "the project model on {to} matches {from}; the target state remains valid"
            ));
            return lines;
        }
        lines.extend(self.reload().iter().cloned());
        lines.extend(self.invalidated().iter().cloned());
        if let Some(migration) = self.migration() {
            lines.push(migration.to_owned());
        }
        lines
    }
}

/// Compares the project on each side of a switch.
///
/// `None` means "there is no project file on that branch". Both sides absent is not a comparison:
/// the caller has no project state to reload, and the report says so by staying empty.
pub fn compare(
    before: Option<&Snapshot>,
    after: Option<&Snapshot>,
    from: &str,
    to: &str,
) -> Invalidation {
    let mut invalidation = Invalidation::default();
    match (before, after) {
        (None, None) => {}
        (None, Some(after)) => invalidation.reload.push(format!(
            "{} was not open before the switch; read the project on {to}",
            after.path().display()
        )),
        (Some(before), None) => invalidation.invalidated.push(format!(
            "{} was read from {from} and is not on {to}; its model and target state are discarded",
            before.path().display()
        )),
        (Some(before), Some(after)) => {
            compare_snapshots(before, after, from, to, &mut invalidation)
        }
    }
    invalidation
}

fn compare_snapshots(
    before: &Snapshot,
    after: &Snapshot,
    from: &str,
    to: &str,
    invalidation: &mut Invalidation,
) {
    if before.version() != after.version() {
        invalidation.reload.push(format!(
            "the project format read from {from} is {}, while {to} is {}; the document must be read \
             again",
            before.version(),
            after.version()
        ));
    } else if after.migrated_from.is_some() {
        invalidation.reload.push(format!(
            "the project on {to} declares {} and is read as {} by this build; the document must be \
             read again",
            after.migrated_from.expect("just checked"),
            after.version()
        ));
    }
    if let Some(found) = after.migrated_from {
        invalidation.migration = Some(format!(
            "the project on {to} is {} and must be migrated to {} before it may be written",
            found,
            after.version()
        ));
    }
    compare_targets(before, after, from, to, invalidation);
    compare_paths("page", &before.pages, &after.pages, from, to, invalidation);
    compare_paths(
        "generated file",
        &before.generated,
        &after.generated,
        from,
        to,
        invalidation,
    );
}

fn compare_targets(
    before: &Snapshot,
    after: &Snapshot,
    from: &str,
    to: &str,
    invalidation: &mut Invalidation,
) {
    for old in before.targets() {
        match after
            .targets()
            .iter()
            .find(|target| target.name() == old.name())
        {
            None => invalidation.invalidated.push(format!(
                "target {} was declared by {from} and not by {to}; its build state and selection \
                 cannot be carried over",
                old.render()
            )),
            Some(new) if new != old => invalidation.invalidated.push(format!(
                "target {} changed to {} on {to}; the previous build state is invalid",
                old.render(),
                new.render()
            )),
            Some(_) => {}
        }
    }
    for new in after.targets() {
        if !before.targets().iter().any(|old| old.name() == new.name()) {
            invalidation.reload.push(format!(
                "target {} exists only on {to}; it has no previous state",
                new.render()
            ));
        }
    }
}

fn compare_paths(
    what: &str,
    before: &[String],
    after: &[String],
    from: &str,
    to: &str,
    invalidation: &mut Invalidation,
) {
    for path in before.iter().filter(|path| !after.contains(path)) {
        invalidation.reload.push(format!(
            "{what} {path} was read from {from} but is not on {to}; close it"
        ));
    }
    for path in after.iter().filter(|path| !before.contains(path)) {
        invalidation.reload.push(format!(
            "{what} {path} is on {to} but was not read from {from}; open it"
        ));
    }
}

/// Refuses a switch while an open document has unsaved changes.
///
/// `dirty` is the caller's list of open documents with unsaved changes. Those bytes are only in the
/// buffer: Git cannot stash them, and a switch would leave the buffer's base file from another
/// branch. The names are repeated so the person can save or discard each one.
pub fn refuse_dirty_buffers(dirty: &[String], branch: &str) -> Result<(), String> {
    if dirty.is_empty() {
        return Ok(());
    }
    Err(format!(
        "switching to {branch} is refused: {} open document(s) have unsaved changes ({}); save or \
         discard them first, because a buffer is not a file Git can stash",
        dirty.len(),
        dirty.join(", ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DESKTOP: &str = r#"{"format": 2, "project": {"id": "3f2b", "name": "Demo"},
        "pages": [{"document": "pages/main.json"}],
        "targets": [{"name": "desktop", "profile": "desktop"}],
        "generated": []}"#;

    const TABLET: &str = r#"{"format": 2, "project": {"id": "3f2b", "name": "Demo"},
        "pages": [{"document": "pages/main.json"}, {"document": "pages/tablet.json"}],
        "targets": [{"name": "desktop", "profile": "tablet"}, {"name": "tablet"}],
        "generated": []}"#;

    const OLD: &str = r#"{"format": 1, "project": {"id": "3f2b", "name": "Demo"},
        "pages": [{"document": "pages/main.json"}],
        "targets": [{"name": "desktop", "profile": "desktop"}],
        "generated": []}"#;

    fn write(name: &str, text: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "rustage-branch-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("a temporary directory");
        let path = directory.join("project.json");
        std::fs::write(&path, text).expect("a project file");
        path
    }

    #[test]
    fn an_identical_project_keeps_the_target_state() {
        let before = Snapshot::read(&write("same-before", DESKTOP)).expect("a snapshot");
        let after = Snapshot::read(&write("same-after", DESKTOP)).expect("a snapshot");
        let invalidation = compare(Some(&before), Some(&after), "main", "feature");
        assert!(invalidation.keeps_state(), "{invalidation:?}");
        let report = invalidation.render("main", "feature");
        assert_eq!(report.len(), 1);
        assert!(report[0].contains("remains valid"), "{report:?}");
    }

    #[test]
    fn a_changed_target_is_reported_as_invalid_and_a_new_one_as_unset() {
        let before = Snapshot::read(&write("changed-before", DESKTOP)).expect("a snapshot");
        let after = Snapshot::read(&write("changed-after", TABLET)).expect("a snapshot");
        let invalidation = compare(Some(&before), Some(&after), "main", "feature");
        assert!(!invalidation.keeps_state());
        let invalid = invalidation.invalidated().join("\n");
        assert!(invalid.contains("target desktop"), "{invalid}");
        assert!(
            invalid.contains("changed to desktop (profile tablet)"),
            "{invalid}"
        );
        let reload = invalidation.reload().join("\n");
        assert!(reload.contains("target tablet"), "{reload}");
        assert!(reload.contains("page pages/tablet.json"), "{reload}");
    }

    #[test]
    fn a_project_absent_from_the_new_branch_discards_the_old_state() {
        let before = Snapshot::read(&write("absent-before", DESKTOP)).expect("a snapshot");
        let invalidation = compare(Some(&before), None, "main", "empty");
        let invalid = invalidation.invalidated().join("\n");
        assert!(
            invalid.contains("read from main and is not on empty"),
            "{invalid}"
        );
        assert!(invalid.contains("discarded"), "{invalid}");
    }

    #[test]
    fn an_old_format_on_the_new_branch_needs_a_migration() {
        let before = Snapshot::read(&write("format-before", DESKTOP)).expect("a snapshot");
        let after = Snapshot::read(&write("format-after", OLD)).expect("a snapshot");
        let invalidation = compare(Some(&before), Some(&after), "main", "old");
        assert!(!invalidation.keeps_state());
        let migration = invalidation.migration().expect("a migration");
        assert!(migration.contains("format v1"), "{migration}");
        assert!(migration.contains("format v2"), "{migration}");
        let reload = invalidation.reload().join("\n");
        assert!(reload.contains("declares format v1"), "{reload}");
        assert!(reload.contains("read again"), "{reload}");
    }

    #[test]
    fn a_path_that_is_not_there_is_absence_and_unreadable_content_is_an_error() {
        let path = write("optional", DESKTOP);
        let missing = path.with_file_name("gone.json");
        assert_eq!(Snapshot::read_optional(&missing).expect("absence"), None);
        std::fs::write(&path, "not json").expect("a file");
        assert!(Snapshot::read_optional(&path).is_err());
    }

    #[test]
    fn an_unsaved_buffer_refuses_the_switch_and_names_every_document() {
        assert!(refuse_dirty_buffers(&[], "feature").is_ok());
        let reason = refuse_dirty_buffers(
            &["pages/main.json".to_owned(), "styles/app.css".to_owned()],
            "feature",
        )
        .expect_err("a refusal");
        assert!(reason.contains("2 open document(s)"), "{reason}");
        assert!(reason.contains("pages/main.json"), "{reason}");
        assert!(reason.contains("styles/app.css"), "{reason}");
        assert!(reason.contains("Git can stash"), "{reason}");
    }
}
