// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Project files: adding, excluding, moving, renaming and deleting them, with a preview and with a
//! way back (task T02.05).
//!
//! The acceptance line is *"paths, design references and file-operation failures are recoverable"*,
//! and it is met by three properties rather than by care:
//!
//! 1. **Nothing happens before a plan.** [`FilePlan::plan`] reads the document and the disk and
//!    produces what *would* happen: the files that move, the references that would be rewritten, and
//!    the reasons it cannot be done. It writes nothing — a caller can show it, print it or throw it
//!    away. [`FilePlan::execute`] is the only thing that touches anything.
//! 2. **Blocker before edit.** A design reference this build cannot rewrite *blocks* the operation
//!    instead of being dropped: an asset property in a page document names the resource, the page
//!    document is what the design holds, and this task does not rewrite it (that edit belongs with
//!    the page editor). The blocker names the page and the property, so the person knows what to
//!    change rather than being told "it failed".
//! 3. **A failure rolls the moves back.** Files move first and the document is written second; if the
//!    write fails (somebody edited the project meanwhile, the disk refuses), every move that was made
//!    is undone and the failure says so. What could not be undone is reported file by file, so a
//!    half-done state is never described as a clean one.
//!
//! What a plan can rewrite today is the project document's own references: a page's document and
//! stylesheet, a resource, a generated file and the entry it comes from, and the startup entry.
//! Undoing a *successful* operation is a separate feature (it needs a change log), and so is
//! importing a file from outside the project: this task moves files that are already inside the root.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::host::paths::ProjectRoot;
use crate::project::record::format::{ProjectDocument, ProjectError, ProjectFile};
use crate::project::record::page::PageDocument;

/// Where a deleted file goes until the document that named it has been written.
///
/// A delete has to be recoverable: the file is moved here first, the project is written, and only
/// then is the file removed. If the write fails the file goes back; if the removal fails the file
/// stays here and the failure says so, instead of a person's file being gone with no record of
/// where. The directory is inside the project root so that both directions are one `rename`.
const TRASH_DIRECTORY: &str = ".rustage-deleted";

/// The path a deleted file is held at, which keeps its own place inside the project.
fn trash_path(path: &str) -> String {
    format!("{TRASH_DIRECTORY}/{path}")
}

/// A file operation on a project.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum FileOperation {
    /// Declares a file that is already inside the project as one of its resources.
    Add {
        /// The project-relative path.
        path: String,
    },
    /// Stops managing a file. The file stays where it is.
    Exclude {
        /// The project-relative path.
        path: String,
    },
    /// Moves or renames a file, and rewrites what pointed at it.
    Move {
        /// Where it is now.
        from: String,
        /// Where it should be.
        to: String,
    },
    /// Deletes a file: it leaves the project *and* the disk, which is what separates this from
    /// [`Self::Exclude`]. The file is held in the project's trash directory until the document has
    /// been written, so a failed write puts it back.
    Delete {
        /// The project-relative path.
        path: String,
    },
}

impl FileOperation {
    /// The word a report uses.
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Add { .. } => "add",
            Self::Exclude { .. } => "exclude",
            Self::Move { .. } => "move",
            Self::Delete { .. } => "delete",
        }
    }

    /// The paths the operation names.
    pub fn paths(&self) -> Vec<&str> {
        match self {
            Self::Add { path } | Self::Exclude { path } | Self::Delete { path } => vec![path],
            Self::Move { from, to } => vec![from, to],
        }
    }
}

impl fmt::Display for FileOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Add { path } => write!(formatter, "add {path}"),
            Self::Exclude { path } => write!(formatter, "exclude {path}"),
            Self::Move { from, to } => write!(formatter, "move {from} to {to}"),
            Self::Delete { path } => write!(formatter, "delete {path}"),
        }
    }
}

/// A file that would move on disk.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Move {
    from: String,
    to: String,
}

impl Move {
    /// Where the file is now.
    pub fn from(&self) -> &str {
        &self.from
    }

    /// Where it would go.
    pub fn to(&self) -> &str {
        &self.to
    }
}

/// A design reference that names the file this plan is about.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DesignReference {
    page: String,
    property: String,
    written: String,
}

impl DesignReference {
    /// The page document that holds it.
    pub fn page(&self) -> &str {
        &self.page
    }

    /// The property that holds it.
    pub fn property(&self) -> &str {
        &self.property
    }

    /// The path it names.
    pub fn written(&self) -> &str {
        &self.written
    }

    /// One line for a report.
    pub fn render(&self) -> String {
        format!("{} in {} names {}", self.property, self.page, self.written)
    }
}

/// What a plan would do, and what stops it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FilePlan {
    operation: FileOperation,
    root: PathBuf,
    moves: Vec<Move>,
    rewrites: Vec<String>,
    declared: Vec<String>,
    undeclared: Vec<String>,
    design: Vec<DesignReference>,
    blockers: Vec<String>,
}

impl FilePlan {
    /// Plans an operation against a document and the disk behind it.
    ///
    /// It reads; it does not write, move or delete anything. Every reason the operation cannot be
    /// done is a *blocker*, and a plan with a blocker never executes.
    pub fn plan(document: &ProjectDocument, root: &ProjectRoot, operation: FileOperation) -> Self {
        let mut plan = Self {
            operation: operation.clone(),
            root: root.as_path().to_path_buf(),
            moves: Vec::new(),
            rewrites: Vec::new(),
            declared: Vec::new(),
            undeclared: Vec::new(),
            design: Vec::new(),
            blockers: Vec::new(),
        };
        let destination = match &operation {
            FileOperation::Move { to, .. } => Some(to.as_str()),
            _ => None,
        };
        for path in operation.paths() {
            if Some(path) == destination {
                // The destination may name a directory that does not exist yet: it is checked by
                // `resolve_for_creation` in the branch below, which is the rule that fits a path
                // that is about to be created.
                continue;
            }
            match root.resolve(path) {
                Ok(_) => {}
                Err(error) => plan
                    .blockers
                    .push(format!("`{path}` cannot be resolved: {}", error.message())),
            }
        }

        match &operation {
            FileOperation::Add { path } => {
                let resolved = root.resolve(path).ok();
                if let Some(resolved) = &resolved
                    && !resolved.is_file()
                {
                    plan.blockers.push(format!(
                        "`{path}` is not a file inside the project; adding it would declare a file \
                         nobody can find"
                    ));
                }
                if document
                    .resources()
                    .iter()
                    .any(|resource| resource.path().written() == path)
                {
                    plan.blockers.push(format!(
                        "`{path}` is already one of this project's resources"
                    ));
                } else {
                    plan.declared.push(path.clone());
                }
            }
            FileOperation::Exclude { path } => {
                plan.undeclared = plan.declared_resource(document, path);
                if plan.undeclared.is_empty() {
                    plan.blockers.push(format!(
                        "`{path}` is not one of this project's resources, so there is nothing to \
                         exclude"
                    ));
                }
                plan.design = plan.find_design_references(document, root, path);
            }
            FileOperation::Move { from, to } => {
                let source = root.resolve(from).ok();
                if let Some(source) = &source
                    && !source.exists()
                {
                    plan.blockers
                        .push(format!("`{from}` is not there, so it cannot move"));
                }
                match root.resolve_for_creation(to) {
                    Ok(target) if target.exists() => plan.blockers.push(format!(
                        "`{to}` is already there; this build does not move a file over an existing one"
                    )),
                    Ok(_) => {}
                    Err(error) => plan.blockers.push(format!(
                        "`{to}` cannot be resolved: {}",
                        error.message()
                    )),
                }
                if from == to {
                    plan.blockers
                        .push("the file would move onto itself".to_owned());
                }
                plan.moves.push(Move {
                    from: from.clone(),
                    to: to.clone(),
                });
                plan.rewrites = plan.rewrite_names(document, from, to);
                // A move undeclares nothing: the reference that named the old path names the new one
                // instead, so the project has not lost a resource. Reporting "stops being a resource"
                // here would say the resource list shrank when it did not.
                let declared_before = plan.declared_resource(document, from);
                plan.design = plan.find_design_references(document, root, from);
                if plan.rewrites.is_empty() && declared_before.is_empty() {
                    plan.blockers.push(format!(
                        "nothing in the project names `{from}`, so moving it would leave the file \
                         where the design does not look for it"
                    ));
                }
            }
            FileOperation::Delete { path } => {
                let resolved = root.resolve(path).ok();
                if let Some(resolved) = &resolved
                    && !resolved.exists()
                {
                    plan.blockers.push(format!(
                        "`{path}` is not there, so there is nothing to delete"
                    ));
                }
                plan.moves.push(Move {
                    from: path.clone(),
                    to: trash_path(path),
                });
                plan.undeclared = plan.declared_resource(document, path);
                plan.design = plan.find_design_references(document, root, path);
                // A move can rewrite what names a file; a delete has no new path to write, so every
                // member that would end up naming a file that is gone is a blocker instead.
                plan.blockers
                    .extend(plan.dangling_references(document, path));
                if let Some(page) = document
                    .pages()
                    .iter()
                    .find(|page| page.document().written() == path)
                {
                    let _ = page;
                    plan.blockers.push(format!(
                        "`{path}` is a page's own document; remove the entry instead, so the pages \
                         and what they generate stay in step"
                    ));
                }
                if let Some(startup) = document.startup_written()
                    && startup == path
                {
                    plan.blockers.push(format!(
                        "`{path}` is the entry the target starts with; choose another entry first"
                    ));
                }
            }
        }

        if !plan.design.is_empty() {
            plan.blockers.push(format!(
                "{} design reference(s) name this file, and a page document is not rewritten by this \
                 build yet",
                plan.design.len()
            ));
        }
        plan
    }

    /// The operation this plan is for.
    pub fn operation(&self) -> &FileOperation {
        &self.operation
    }

    /// The directory every path is relative to.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The files that would move.
    pub fn moves(&self) -> &[Move] {
        &self.moves
    }

    /// The references that would be rewritten.
    pub fn rewrites(&self) -> &[String] {
        &self.rewrites
    }

    /// The resources that would be declared.
    pub fn declared(&self) -> &[String] {
        &self.declared
    }

    /// The resource declarations that would be removed.
    pub fn undeclared(&self) -> &[String] {
        &self.undeclared
    }

    /// The design references that stop it.
    pub fn design_references(&self) -> &[DesignReference] {
        &self.design
    }

    /// Why it cannot be done.
    pub fn blockers(&self) -> &[String] {
        &self.blockers
    }

    /// Whether it can be done.
    pub fn is_executable(&self) -> bool {
        self.blockers.is_empty()
    }

    /// What would happen, one line at a time: the preview a person reads before saying yes.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "{}: {}",
            self.operation,
            if self.is_executable() {
                "can be done"
            } else {
                "cannot be done as it stands"
            }
        )];
        let deleting = matches!(self.operation, FileOperation::Delete { .. });
        for movement in &self.moves {
            lines.push(if deleting {
                format!("  the file {} is deleted", movement.from)
            } else {
                format!("  the file {} moves to {}", movement.from, movement.to)
            });
        }
        for rewrite in &self.rewrites {
            lines.push(format!("  {rewrite}"));
        }
        for declared in &self.declared {
            lines.push(format!("  {declared} becomes a project resource"));
        }
        for undeclared in &self.undeclared {
            // Excluding leaves the file where it is; deleting takes it with it. The same list of
            // declarations feeds both, so the words have to separate them.
            lines.push(if deleting {
                format!("  {undeclared} is no longer a project resource")
            } else {
                format!(
                    "  {undeclared} stops being a project resource (the file itself is left alone)"
                )
            });
        }
        for reference in &self.design {
            lines.push(format!("  a design reference: {}", reference.render()));
        }
        for blocker in &self.blockers {
            lines.push(format!("  blocked: {blocker}"));
        }
        lines
    }

    /// Carries the operation out: the files move, then the document is written.
    ///
    /// A failure after the moves undoes them and reports what, if anything, could not be undone, so
    /// a caller that only looks at the outcome cannot mistake a half-done state for a clean one.
    pub fn execute(&self, file: &ProjectFile) -> Result<Executed, FileFailure> {
        if !self.is_executable() {
            return Err(FileFailure {
                reason: format!(
                    "{} cannot be done as it stands: {}",
                    self.operation,
                    self.blockers.join("; ")
                ),
                undone: Vec::new(),
                left_behind: Vec::new(),
            });
        }
        let Some(document) = file.document() else {
            return Err(FileFailure {
                reason: "the document cannot be interpreted, so nothing was changed".to_owned(),
                undone: Vec::new(),
                left_behind: Vec::new(),
            });
        };
        let root = file.root();

        let mut moved: Vec<Move> = Vec::new();
        for movement in &self.moves {
            let source = root.resolve(&movement.from).map_err(|error| FileFailure {
                reason: error.message(),
                undone: undo(root, &moved),
                left_behind: Vec::new(),
            })?;
            let target = root
                .resolve_for_creation(&movement.to)
                .map_err(|error| FileFailure {
                    reason: error.message(),
                    undone: undo(root, &moved),
                    left_behind: Vec::new(),
                })?;
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|error| FileFailure {
                    reason: format!("{}: {}", parent.display(), error),
                    undone: undo(root, &moved),
                    left_behind: Vec::new(),
                })?;
            }
            std::fs::rename(&source, &target).map_err(|error| FileFailure {
                reason: format!(
                    "{} could not move to {}: {error}",
                    movement.from, movement.to
                ),
                undone: undo(root, &moved),
                left_behind: Vec::new(),
            })?;
            moved.push(movement.clone());
        }

        // The document is edited to describe what just happened, and written only if it reads back.
        let edited = match self.edit(document, root) {
            Ok(edited) => edited,
            Err(error) => {
                return Err(FileFailure {
                    reason: error.message(),
                    undone: undo(root, &moved),
                    left_behind: Vec::new(),
                });
            }
        };
        match file.write_document(&edited) {
            Ok(written) => {
                // The project no longer names what was deleted, so the held copy can go. A failure
                // here is reported rather than swallowed: the file is still on disk, at a path the
                // report names, and the deletion did not fully happen.
                let left_behind = if matches!(self.operation, FileOperation::Delete { .. }) {
                    remove_held(root, &moved)
                } else {
                    Vec::new()
                };
                if !left_behind.is_empty() {
                    return Err(FileFailure {
                        reason: format!(
                            "the project was written, but {} file(s) could not be removed from \
                             `{TRASH_DIRECTORY}`",
                            left_behind.len()
                        ),
                        undone: Vec::new(),
                        left_behind,
                    });
                }
                Ok(Executed {
                    operation: self.operation.clone(),
                    moves: moved,
                    rewrites: self.rewrites.clone(),
                    declared: self.declared.clone(),
                    undeclared: self.undeclared.clone(),
                    written: written.target().to_path_buf(),
                    migrated_from: written.migrated_from(),
                })
            }
            Err(error) => {
                // The files moved but the project was not written: put them back, and say exactly
                // what is where when even that fails.
                let undone = undo(root, &moved);
                let mut left_behind = Vec::new();
                if undone.len() != moved.len() {
                    for movement in &moved {
                        if !undone.contains(movement) {
                            left_behind.push(format!(
                                "{} is at {} and could not be moved back",
                                movement.from, movement.to
                            ));
                        }
                    }
                }
                Err(FileFailure {
                    reason: error.message(),
                    undone,
                    left_behind,
                })
            }
        }
    }

    fn rewrite_names(&self, document: &ProjectDocument, from: &str, to: &str) -> Vec<String> {
        let Some(root) = ProjectRoot::new(&self.root).ok() else {
            return Vec::new();
        };
        document
            .rewire(&root, from, to)
            .map(|(_, rewrites)| rewrites)
            .unwrap_or_default()
    }

    /// What a delete would leave naming a file that is gone.
    ///
    /// The companion of [`ProjectDocument::rewire`], which is what keeps these same members up to
    /// date when a file moves: a site that is missed there is missed here, so a test moves a file
    /// every site names and checks that the rewrites and these lines cover the same members.
    fn dangling_references(&self, document: &ProjectDocument, path: &str) -> Vec<String> {
        let mut dangling = Vec::new();
        for (index, page) in document.pages().iter().enumerate() {
            if page
                .stylesheet()
                .is_some_and(|sheet| sheet.written() == path)
            {
                dangling.push(format!(
                    "pages[{index}] uses `{path}` as its stylesheet; choose another one first"
                ));
            }
        }
        for (index, generated) in document.generated().iter().enumerate() {
            if generated.output().written() == path {
                dangling.push(format!(
                    "generated[{index}] writes `{path}`; stop generating it first"
                ));
            }
            if generated.from().written() == path {
                dangling.push(format!(
                    "generated[{index}] comes from `{path}`; point it at another entry first"
                ));
            }
        }
        dangling
    }

    fn declared_resource(&self, document: &ProjectDocument, path: &str) -> Vec<String> {
        document
            .resources()
            .iter()
            .filter(|resource| resource.path().written() == path)
            .map(|resource| resource.path().written().to_owned())
            .collect()
    }

    /// The design references in the page documents that name a path.
    ///
    /// A page document is where a design holds what it points at; this build reads it and says so,
    /// and does not rewrite it (the page editor owns that edit).
    fn find_design_references(
        &self,
        document: &ProjectDocument,
        root: &ProjectRoot,
        path: &str,
    ) -> Vec<DesignReference> {
        let mut found = Vec::new();
        for page in document.pages() {
            if page.document().written() == path {
                // The page's own document naming itself is the entry, not an asset reference.
                continue;
            }
            let Ok(page_document) = PageDocument::read(root, page.document().written()) else {
                continue;
            };
            for reference in page_document.references() {
                if reference.written() != path {
                    continue;
                }
                found.push(DesignReference {
                    page: page.document().written().to_owned(),
                    property: "an asset property".to_owned(),
                    written: reference.written().to_owned(),
                });
            }
        }
        found
    }

    /// The document this operation produces.
    fn edit(
        &self,
        document: &ProjectDocument,
        root: &ProjectRoot,
    ) -> Result<ProjectDocument, ProjectError> {
        match &self.operation {
            FileOperation::Add { path } => document.with_resource(root, path),
            FileOperation::Exclude { path } => Ok(document.without_resource(path).0),
            FileOperation::Move { from, to } => Ok(document.rewire(root, from, to)?.0),
            FileOperation::Delete { path } => Ok(document.without_resource(path).0),
        }
    }
}

/// What an executed plan did.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Executed {
    operation: FileOperation,
    moves: Vec<Move>,
    rewrites: Vec<String>,
    declared: Vec<String>,
    undeclared: Vec<String>,
    written: PathBuf,
    migrated_from: Option<crate::contracts::revision::FormatVersion>,
}

impl Executed {
    /// The operation that was carried out.
    pub fn operation(&self) -> &FileOperation {
        &self.operation
    }

    /// The files that moved.
    pub fn moves(&self) -> &[Move] {
        &self.moves
    }

    /// The references that were rewritten.
    pub fn rewrites(&self) -> &[String] {
        &self.rewrites
    }

    /// The resources that were declared.
    pub fn declared(&self) -> &[String] {
        &self.declared
    }

    /// The resource declarations that were removed.
    pub fn undeclared(&self) -> &[String] {
        &self.undeclared
    }

    /// The project document that was written.
    pub fn written(&self) -> &Path {
        &self.written
    }

    /// The version it was read as, when writing it meant migrating it.
    pub const fn migrated_from(&self) -> Option<crate::contracts::revision::FormatVersion> {
        self.migrated_from
    }

    /// One line per thing that happened.
    pub fn render(&self) -> Vec<String> {
        let deleting = matches!(self.operation, FileOperation::Delete { .. });
        let mut lines: Vec<String> = self
            .moves
            .iter()
            .map(|movement| {
                if deleting {
                    format!("deleted {}", movement.from)
                } else {
                    format!("moved {} to {}", movement.from, movement.to)
                }
            })
            .collect();
        for rewrite in &self.rewrites {
            lines.push(format!("rewrote {rewrite}"));
        }
        for declared in &self.declared {
            lines.push(format!("declared {declared}"));
        }
        for undeclared in &self.undeclared {
            lines.push(if deleting {
                format!("{undeclared} is no longer a project resource")
            } else {
                format!("excluded {undeclared}")
            });
        }
        lines.push(format!("wrote {}", self.written.display()));
        lines
    }
}

/// Why an operation failed, and what was put back.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FileFailure {
    reason: String,
    undone: Vec<Move>,
    left_behind: Vec<String>,
}

impl FileFailure {
    /// Why it failed.
    pub fn reason(&self) -> &str {
        &self.reason
    }

    /// The moves that were undone.
    pub fn undone(&self) -> &[Move] {
        &self.undone
    }

    /// What could not be undone, file by file.
    pub fn left_behind(&self) -> &[String] {
        &self.left_behind
    }

    /// Whether everything that was done is undone.
    pub fn is_recovered(&self) -> bool {
        self.left_behind.is_empty()
    }

    /// A sentence for a person: what went wrong, and what state things are in.
    pub fn message(&self) -> String {
        if self.undone.is_empty() && self.left_behind.is_empty() {
            return format!("{}; nothing was changed", self.reason);
        }
        if self.left_behind.is_empty() {
            return format!(
                "{}; the {} file move(s) that were made were undone, so the project is as it was",
                self.reason,
                self.undone.len()
            );
        }
        format!(
            "{}; {} move(s) were undone and {} could not be: {}",
            self.reason,
            self.undone.len(),
            self.left_behind.len(),
            self.left_behind.join("; ")
        )
    }
}

impl fmt::Display for FileFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.message())
    }
}

/// Moves back the files that were moved, and returns the ones that made it back.
///
/// Undoing is done in reverse order, so a chain of moves ends where it started. A move that cannot be
/// undone is left out of the result, and the caller reports it rather than claiming a clean state.
/// Removes the files a delete is holding, and reports the ones that could not be removed.
///
/// The directories the hold used are pruned as far as they are empty, so a project does not keep a
/// tree of empty `.rustage-deleted/...` directories after deleting a file.
fn remove_held(root: &ProjectRoot, moved: &[Move]) -> Vec<String> {
    let mut left_behind = Vec::new();
    for movement in moved {
        let Ok(held) = root.resolve(&movement.to) else {
            left_behind.push(format!(
                "{} is held at {} and this build cannot resolve that path",
                movement.from, movement.to
            ));
            continue;
        };
        match std::fs::remove_file(&held) {
            Ok(()) => prune_empty_directories(root, &held),
            Err(error) => left_behind.push(format!(
                "{} is held at {} and could not be removed: {error}",
                movement.from, movement.to
            )),
        }
    }
    left_behind
}

/// Removes the directories a held file left empty, up to but not including the project root.
fn prune_empty_directories(root: &ProjectRoot, held: &Path) {
    let mut directory = held.parent();
    while let Some(current) = directory {
        if current == root.as_path() || !current.starts_with(root.as_path()) {
            break;
        }
        if std::fs::remove_dir(current).is_err() {
            break;
        }
        directory = current.parent();
    }
}

fn undo(root: &ProjectRoot, moved: &[Move]) -> Vec<Move> {
    let mut undone = Vec::new();
    for movement in moved.iter().rev() {
        let (Ok(target), Ok(source)) = (root.resolve(&movement.to), root.resolve(&movement.from))
        else {
            continue;
        };
        if let Some(parent) = source.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if std::fs::rename(&target, &source).is_ok() {
            // A held copy that goes back leaves the trash tree empty behind it. An ordinary move
            // does not: pruning there could remove a project directory the move did not touch.
            if movement.to.starts_with(TRASH_DIRECTORY) {
                prune_empty_directories(root, &target);
            }
            undone.push(movement.clone());
        }
    }
    undone
}

#[cfg(test)]
mod tests {
    use super::{FileOperation, FilePlan};
    use crate::host::paths::ProjectRoot;
    use crate::project::record::format::ProjectFile;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

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
        let root = std::env::temp_dir().join(format!("rustage-files-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&root).expect("a temporary directory is creatable");
        Sandbox {
            root: root.clone(),
            _guard: TempDirectory(root),
        }
    }

    const PROJECT: &str = "{\"format\": 2, \"project\": {\"id\": \"3f2b\", \"name\": \"Demo\"}, \
         \"pages\": [{\"document\": \"pages/main.json\", \"stylesheet\": \"styles/main.css\"}], \
         \"resources\": [{\"path\": \"resources/icon.svg\"}], \
         \"generated\": [{\"output\": \"src/generated/main.rs\", \"from\": \"pages/main.json\"}], \
         \"startup\": \"pages/main.json\"}";

    /// A page document that points at a resource through an asset property.
    const PAGE_WITH_ASSET: &str = "{\n  \"format\": 1,\n  \"page\": {\n    \"id\": 1,\n    \"name\": \"Main\",\n    \"root\": 1\n  },\n  \"nodes\": [\n    {\n      \"id\": 1,\n      \"type\": \"page\",\n      \"children\": [],\n      \"properties\": [\n        {\n          \"name\": \"icon\",\n          \"value\": {\n            \"asset\": \"resources/icon.svg\"\n          }\n        }\n      ]\n    }\n  ]\n}\n";

    impl Sandbox {
        fn root(&self) -> ProjectRoot {
            ProjectRoot::new(&self.root).expect("the sandbox is a directory")
        }

        fn write(&self, relative: &str, text: &str) -> PathBuf {
            let path = self.root.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("a directory");
            }
            std::fs::write(&path, text).expect("a file");
            path
        }

        fn project(&self, page: Option<&str>) -> ProjectFile {
            let path = self.write("project.json", PROJECT);
            if let Some(page) = page {
                self.write("pages/main.json", page);
            }
            ProjectFile::open(&path).expect("the project opens")
        }
    }

    #[test]
    fn a_plan_is_a_preview_that_touches_nothing() {
        let sandbox = sandbox();
        sandbox.write("resources/icon.svg", "<svg/>");
        let file = sandbox.project(None);
        let root = sandbox.root();
        let document = file.document().expect("a document");

        let plan = FilePlan::plan(
            document,
            &root,
            FileOperation::Move {
                from: "resources/icon.svg".to_owned(),
                to: "resources/icons/app.svg".to_owned(),
            },
        );

        assert!(plan.is_executable(), "{:?}", plan.blockers());
        assert_eq!(plan.operation().name(), "move");
        assert_eq!(plan.moves().len(), 1);
        assert_eq!(plan.moves()[0].from(), "resources/icon.svg");
        assert!(
            plan.rewrites()
                .iter()
                .any(|line| line.contains("a resource"))
        );
        assert_eq!(plan.root(), root.as_path());
        let rendered = plan.render();
        assert!(rendered[0].contains("move resources/icon.svg to resources/icons/app.svg"));
        assert!(rendered[0].contains("can be done"));
        assert!(
            rendered
                .iter()
                .any(|line| line.contains("the file resources/icon.svg moves to"))
        );

        // Nothing happened: the file is where it was and the document is unchanged.
        assert!(sandbox.root.join("resources/icon.svg").is_file());
        assert!(!sandbox.root.join("resources/icons/app.svg").exists());
        assert_eq!(
            std::fs::read_to_string(sandbox.root.join("project.json")).unwrap(),
            PROJECT
        );
    }

    #[test]
    fn a_design_reference_blocks_the_operation_instead_of_being_dropped() {
        let sandbox = sandbox();
        sandbox.write("resources/icon.svg", "<svg/>");
        let file = sandbox.project(Some(PAGE_WITH_ASSET));
        let root = sandbox.root();
        let document = file.document().expect("a document");

        for operation in [
            FileOperation::Delete {
                path: "resources/icon.svg".to_owned(),
            },
            FileOperation::Exclude {
                path: "resources/icon.svg".to_owned(),
            },
            FileOperation::Move {
                from: "resources/icon.svg".to_owned(),
                to: "resources/other.svg".to_owned(),
            },
        ] {
            let plan = FilePlan::plan(document, &root, operation.clone());
            assert!(!plan.is_executable(), "{operation} should be blocked");
            assert_eq!(plan.design_references().len(), 1);
            let reference = &plan.design_references()[0];
            assert_eq!(reference.page(), "pages/main.json");
            assert_eq!(reference.written(), "resources/icon.svg");
            assert!(reference.render().contains("pages/main.json"));
            assert!(
                plan.blockers()
                    .iter()
                    .any(|blocker| blocker.contains("design reference")),
                "{:?}",
                plan.blockers()
            );
            assert!(
                plan.render()
                    .iter()
                    .any(|line| line.contains("a design reference:"))
            );

            // Executing it changes nothing at all.
            let failure = plan
                .execute(&file)
                .expect_err("a blocked plan does not run");
            assert!(failure.is_recovered());
            assert!(failure.message().contains("nothing was changed"));
            assert!(sandbox.root.join("resources/icon.svg").is_file());
            assert_eq!(
                std::fs::read_to_string(sandbox.root.join("project.json")).unwrap(),
                PROJECT
            );
        }
    }

    #[test]
    fn moving_a_resource_rewrites_the_reference_and_moves_the_file() {
        let sandbox = sandbox();
        sandbox.write("resources/icon.svg", "<svg/>");
        let file = sandbox.project(None);
        let root = sandbox.root();
        let document = file.document().expect("a document");

        let plan = FilePlan::plan(
            document,
            &root,
            FileOperation::Move {
                from: "resources/icon.svg".to_owned(),
                to: "resources/icons/app.svg".to_owned(),
            },
        );
        let executed = plan.execute(&file).expect("the move happens");

        assert_eq!(executed.moves().len(), 1);
        assert!(
            executed
                .rewrites()
                .iter()
                .any(|line| line.contains("a resource"))
        );
        assert_eq!(executed.written(), file.path());
        assert!(executed.render()[0].contains("moved resources/icon.svg"));
        assert!(!sandbox.root.join("resources/icon.svg").exists());
        assert_eq!(
            std::fs::read_to_string(sandbox.root.join("resources/icons/app.svg")).unwrap(),
            "<svg/>"
        );

        // The document was written with the new path, and reads back.
        let again = ProjectFile::open(file.path()).expect("the project opens");
        let document = again.document().expect("a document");
        assert_eq!(
            document.resources()[0].path().written(),
            "resources/icons/app.svg"
        );
        assert!(again.is_writable());
    }

    #[test]
    fn moving_a_page_document_rewrites_every_reference_that_named_it() {
        let sandbox = sandbox();
        sandbox.write("pages/main.json", "{}");
        let file = sandbox.project(None);
        let root = sandbox.root();
        let document = file.document().expect("a document");

        let plan = FilePlan::plan(
            document,
            &root,
            FileOperation::Move {
                from: "pages/main.json".to_owned(),
                to: "pages/windows/main.json".to_owned(),
            },
        );
        assert!(plan.is_executable(), "{:?}", plan.blockers());
        assert_eq!(
            plan.rewrites().len(),
            3,
            "the page's own document, the source the generated file comes from, and the startup \
             entry: {:?}",
            plan.rewrites()
        );
        let executed = plan.execute(&file).expect("the move happens");
        assert_eq!(executed.moves().len(), 1);

        let again = ProjectFile::open(file.path()).expect("the project opens");
        let document = again.document().expect("a document");
        assert_eq!(
            document.pages()[0].document().written(),
            "pages/windows/main.json"
        );
        assert_eq!(
            document.generated()[0].from().written(),
            "pages/windows/main.json"
        );
        assert_eq!(document.startup_written(), Some("pages/windows/main.json"));
        assert!(sandbox.root.join("pages/windows/main.json").is_file());
        // A document that names a page it does not have would be refused by the reader; this one
        // reads back, which is what proves the rewrites were complete.
        assert!(again.is_writable());
    }

    #[test]
    fn adding_and_excluding_a_resource_are_a_statement_about_the_project() {
        let sandbox = sandbox();
        sandbox.write("resources/new.svg", "<svg/>");
        let file = sandbox.project(None);
        let root = sandbox.root();
        let document = file.document().expect("a document");

        let plan = FilePlan::plan(
            document,
            &root,
            FileOperation::Add {
                path: "resources/new.svg".to_owned(),
            },
        );
        assert!(plan.is_executable(), "{:?}", plan.blockers());
        assert_eq!(plan.declared(), ["resources/new.svg"]);
        let executed = plan.execute(&file).expect("the resource is declared");
        assert_eq!(executed.declared(), ["resources/new.svg"]);
        assert!(
            executed
                .render()
                .iter()
                .any(|line| line.contains("declared resources/new.svg"))
        );

        let again = ProjectFile::open(file.path()).expect("the project opens");
        let document = again.document().expect("a document");
        assert_eq!(document.resources().len(), 2);

        // Adding it again is refused: a resource is declared once.
        let refusal = FilePlan::plan(
            document,
            &root,
            FileOperation::Add {
                path: "resources/new.svg".to_owned(),
            },
        );
        assert!(refusal.blockers()[0].contains("already one of this project's resources"));

        // Adding a file nobody can find is refused too.
        let missing = FilePlan::plan(
            document,
            &root,
            FileOperation::Add {
                path: "resources/nowhere.svg".to_owned(),
            },
        );
        assert!(missing.blockers()[0].contains("is not a file inside the project"));

        // Excluding leaves the file alone and takes the declaration away.
        // The project is opened again: a save is a ticket for one write of what was read, so the
        // handle the first operation used cannot be used for the second.
        let again = ProjectFile::open(file.path()).expect("the project opens");
        let plan = FilePlan::plan(
            again.document().expect("a document"),
            &root,
            FileOperation::Exclude {
                path: "resources/new.svg".to_owned(),
            },
        );
        assert!(plan.is_executable(), "{:?}", plan.blockers());
        assert_eq!(plan.undeclared(), ["resources/new.svg"]);
        assert_eq!(
            plan.render()[1],
            "  resources/new.svg stops being a project resource (the file itself is left alone)"
        );
        let excluded = plan.execute(&again).expect("the resource is excluded");
        assert!(
            excluded
                .render()
                .iter()
                .any(|line| line == "excluded resources/new.svg"),
            "{:?}",
            excluded.render()
        );
        assert!(
            sandbox.root.join("resources/new.svg").is_file(),
            "excluding a file from a project is not deleting it"
        );
        let again = ProjectFile::open(file.path()).expect("the project opens");
        assert_eq!(again.document().expect("a document").resources().len(), 1);

        // Excluding something that was never declared is refused.
        let refusal = FilePlan::plan(
            again.document().expect("a document"),
            &root,
            FileOperation::Exclude {
                path: "resources/new.svg".to_owned(),
            },
        );
        assert!(refusal.blockers()[0].contains("nothing to exclude"));
    }

    #[test]
    fn deleting_a_file_takes_it_off_disk_and_out_of_the_project() {
        let sandbox = sandbox();
        sandbox.write("resources/icon.svg", "<svg/>");
        let file = sandbox.project(None);
        let root = sandbox.root();
        let document = file.document().expect("a document");

        let plan = FilePlan::plan(
            document,
            &root,
            FileOperation::Delete {
                path: "resources/icon.svg".to_owned(),
            },
        );
        assert!(plan.is_executable(), "{:?}", plan.blockers());
        // The plan says what a person asked for, not where the file is held while it happens.
        assert_eq!(plan.render()[1], "  the file resources/icon.svg is deleted");
        let executed = plan.execute(&file).expect("the file is deleted");
        assert_eq!(executed.moves().len(), 1);
        assert_eq!(executed.undeclared(), ["resources/icon.svg"]);
        assert!(
            executed
                .render()
                .iter()
                .any(|line| line == "deleted resources/icon.svg")
        );
        assert!(
            executed
                .render()
                .iter()
                .any(|line| line == "resources/icon.svg is no longer a project resource")
        );
        // Excluding a file and deleting it are different operations: this one takes it off the disk.
        assert!(!sandbox.root.join("resources/icon.svg").exists());
        assert!(
            !sandbox.root.join(".rustage-deleted").exists(),
            "the held copy is removed, and the directories it needed are pruned"
        );
        let again = ProjectFile::open(file.path()).expect("the project opens");
        assert!(again.document().expect("a document").resources().is_empty());

        // A page's own document is not deleted by this operation: the entry is what says it exists.
        let refusal = FilePlan::plan(
            again.document().expect("a document"),
            &root,
            FileOperation::Delete {
                path: "pages/main.json".to_owned(),
            },
        );
        assert!(
            refusal
                .blockers()
                .iter()
                .any(|blocker| blocker.contains("remove the entry instead")),
            "{:?}",
            refusal.blockers()
        );
    }

    #[test]
    fn a_delete_whose_write_fails_puts_the_file_back() {
        let sandbox = sandbox();
        sandbox.write("resources/icon.svg", "<svg/>");
        let file = sandbox.project(None);
        let root = sandbox.root();
        let document = file.document().expect("a document");

        let plan = FilePlan::plan(
            document,
            &root,
            FileOperation::Delete {
                path: "resources/icon.svg".to_owned(),
            },
        );
        assert!(plan.is_executable(), "{:?}", plan.blockers());

        // Somebody edits the project between the plan and the execution: the file was already held,
        // and a failed write has to bring it back rather than leave the project without it.
        std::fs::write(
            sandbox.root.join("project.json"),
            PROJECT.replace("\"Demo\"", "\"Edited\""),
        )
        .expect("an edit");

        let failure = plan.execute(&file).expect_err("the write is refused");
        assert!(failure.reason().contains("changed on disk"));
        assert_eq!(failure.undone().len(), 1);
        assert!(failure.is_recovered());
        assert!(
            sandbox.root.join("resources/icon.svg").is_file(),
            "the file is back where it was"
        );
        assert!(
            !sandbox.root.join(".rustage-deleted").exists(),
            "and nothing is left in the trash"
        );
    }

    #[test]
    fn what_names_a_file_is_the_same_list_that_a_move_rewrites() {
        let sandbox = sandbox();
        sandbox.write("pages/main.json", "{}");
        sandbox.write("resources/icon.svg", "<svg/>");
        let file = sandbox.project(None);
        let root = sandbox.root();
        let document = file.document().expect("a document");

        // Each named path, how many members of the document name it, and the words those members are
        // described with. A move rewrites exactly those members; a delete is refused for exactly the
        // ones that would end up naming a file that is gone, and a resource is not one of them
        // (deleting it removes the declaration, which is what the operation is for).
        let cases: [(&str, usize, &[&str]); 4] = [
            ("styles/main.css", 1, &["a page stylesheet"]),
            ("src/generated/main.rs", 1, &["a generated file"]),
            (
                "pages/main.json",
                3,
                &[
                    "the page document",
                    "a generated file's source",
                    "the startup entry",
                ],
            ),
            ("resources/icon.svg", 1, &["a resource"]),
        ];
        for (path, named, words) in cases {
            let plan = FilePlan::plan(
                document,
                &root,
                FileOperation::Move {
                    from: path.to_owned(),
                    to: format!("moved/{path}"),
                },
            );
            assert_eq!(
                plan.rewrites().len(),
                named,
                "{path} is named {named} time(s): {:?}",
                plan.rewrites()
            );
            for word in words {
                assert!(
                    plan.rewrites().iter().any(|line| line.contains(word)),
                    "{path} should be rewritten as `{word}`: {:?}",
                    plan.rewrites()
                );
            }

            // The same path, deleted: what a move would have rewritten is what a delete has to
            // refuse over, except the resource declaration the delete removes anyway.
            let refusal = FilePlan::plan(
                document,
                &root,
                FileOperation::Delete {
                    path: path.to_owned(),
                },
            );
            let dangling: Vec<_> = refusal
                .blockers()
                .iter()
                .filter(|blocker| {
                    blocker.contains("; choose another")
                        || blocker.contains("; stop generating")
                        || blocker.contains("; point it at another")
                        || blocker.contains("remove the entry instead")
                        || blocker.contains("choose another entry first")
                })
                .collect();
            if path == "resources/icon.svg" {
                assert!(
                    refusal.is_executable(),
                    "nothing but the declaration names a resource: {:?}",
                    refusal.blockers()
                );
            } else {
                assert_eq!(
                    dangling.len(),
                    named,
                    "a delete of {path} has to answer for every member that names it: {:?}",
                    refusal.blockers()
                );
            }
        }
    }

    #[test]
    fn a_move_that_would_overwrite_something_or_leave_the_root_is_refused() {
        let sandbox = sandbox();
        sandbox.write("resources/icon.svg", "<svg/>");
        sandbox.write("resources/other.svg", "<svg/>");
        let file = sandbox.project(None);
        let root = sandbox.root();
        let document = file.document().expect("a document");

        let over = FilePlan::plan(
            document,
            &root,
            FileOperation::Move {
                from: "resources/icon.svg".to_owned(),
                to: "resources/other.svg".to_owned(),
            },
        );
        assert!(over.blockers()[0].contains("already there"));

        let outside = FilePlan::plan(
            document,
            &root,
            FileOperation::Move {
                from: "resources/icon.svg".to_owned(),
                to: "../outside.svg".to_owned(),
            },
        );
        assert!(outside.blockers()[0].contains("cannot be resolved"));
        assert!(outside.blockers()[0].contains("climbs above the project root"));

        sandbox.write("pages/main.json", "{}");
        let named = FilePlan::plan(
            document,
            &root,
            FileOperation::Move {
                from: "pages/main.json".to_owned(),
                to: "pages/other.json".to_owned(),
            },
        );
        // A page document is named by the entry that points at it, so moving it is allowed; a file
        // nothing names is what gets refused.
        assert!(named.is_executable(), "{:?}", named.blockers());
        let unclaimed = sandbox.write("loose.txt", "x");
        assert!(unclaimed.is_file());
        let plan = FilePlan::plan(
            document,
            &root,
            FileOperation::Move {
                from: "loose.txt".to_owned(),
                to: "loose2.txt".to_owned(),
            },
        );
        assert!(
            plan.blockers()
                .iter()
                .any(|blocker| blocker.contains("nothing in the project names"))
        );
    }

    #[test]
    fn a_failure_after_the_moves_puts_them_back() {
        let sandbox = sandbox();
        sandbox.write("resources/icon.svg", "<svg/>");
        let file = sandbox.project(None);
        let root = sandbox.root();
        let document = file.document().expect("a document");

        let plan = FilePlan::plan(
            document,
            &root,
            FileOperation::Move {
                from: "resources/icon.svg".to_owned(),
                to: "resources/icons/app.svg".to_owned(),
            },
        );
        assert!(plan.is_executable());

        // Somebody edits the project between the plan and the execution: the write is refused, and
        // the move that was already made is undone.
        std::fs::write(
            sandbox.root.join("project.json"),
            PROJECT.replace("\"Demo\"", "\"Edited\""),
        )
        .expect("an edit");

        let failure = plan.execute(&file).expect_err("the write is refused");
        assert!(failure.reason().contains("changed on disk"));
        assert_eq!(failure.undone().len(), 1);
        assert!(failure.is_recovered());
        assert!(
            failure
                .message()
                .contains("were undone, so the project is as it was"),
            "{}",
            failure.message()
        );
        assert!(sandbox.root.join("resources/icon.svg").is_file());
        assert!(!sandbox.root.join("resources/icons/app.svg").exists());
        assert!(
            std::fs::read_to_string(sandbox.root.join("project.json"))
                .unwrap()
                .contains("Edited"),
            "the edit that stopped it is still the file on disk"
        );
    }

    #[test]
    fn the_paths_an_operation_names_are_what_a_report_shows() {
        let add = FileOperation::Add {
            path: "a".to_owned(),
        };
        let moved = FileOperation::Move {
            from: "a".to_owned(),
            to: "b".to_owned(),
        };
        assert_eq!(add.paths(), ["a"]);
        assert_eq!(moved.paths(), ["a", "b"]);
        assert_eq!(add.to_string(), "add a");
        assert_eq!(moved.to_string(), "move a to b");
        assert_eq!(
            FileOperation::Delete {
                path: "a".to_owned()
            }
            .name(),
            "delete"
        );
    }
}
