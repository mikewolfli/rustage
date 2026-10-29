// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Creating a project: the wizard's work, and the rules that keep a failure from leaving anything
//! behind (task T02.02).
//!
//! Creating a project is the first thing this build ever writes, and the acceptance line is blunt:
//! **a failure must not overwrite the directory that was there, and must not leave half a project**.
//! Both are properties of *how* the write happens, not of how careful the code is:
//!
//! 1. **Nothing is written into the project directory.** Everything goes into a staging directory
//!    beside it, on the same filesystem, and the project appears when that directory is renamed into
//!    place. A rename is one filesystem operation: either the project is there or the target is
//!    exactly as it was.
//! 2. **The target has to be free.** A directory that exists and has anything in it is refused
//!    before anything is written (roadmap §3.4: "a cancel must not leave half a project"), and an
//!    existing *empty* directory is filled only by that same rename.
//! 3. **Every refusal and every cancel cleans up the staging directory** and says what it removed, so
//!    the caller never has to guess whether the attempt left something behind.
//!
//! The wizard itself (§3.4: back and cancel are supported, the last step shows the final change and
//! the failure impact) is [`Creation`]: [`Creation::plan`] is the last step's material
//! ([`Creation::summary`]), [`Creation::stage`] writes the files where they are not yet a project,
//! [`Creation::commit`] puts them in place, and [`Creation::cancel`] takes them away again.
//!
//! ## What a template is, and what it is not
//!
//! A template describes a project **document** and the files it names — never code that this build
//! has not learned to generate yet. [`Template::Application`] therefore adds exactly one thing to
//! [`Template::BasicPage`]: the mapping from the page to the file the generator will write into
//! `src/generated/` (decision `F2`). Components, themes, windows and manifests arrive with the tasks
//! that own them, and a template that invented them would be a project nobody can build.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::contracts::identity::{NodeId, PageId};
use crate::contracts::model::NodeKind;
use crate::host::paths::ProjectRoot;
use crate::project::record::format::{PROJECT_FILE_NAME, ProjectDraft, ProjectError, ProjectFile};
use crate::project::record::page::PageDocument;

/// What a new project starts from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Template {
    /// A project with a name and nothing else: the designer's canvas is empty.
    Empty,
    /// A project with one page, its stylesheet and a target to build.
    BasicPage,
    /// [`Template::BasicPage`] plus the file the page is generated into.
    Application,
}

impl Template {
    /// Every template, in the order the wizard offers them.
    pub const ALL: [Self; 3] = [Self::Empty, Self::BasicPage, Self::Application];

    /// The name a command line or a saved choice uses.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::BasicPage => "basic-page",
            Self::Application => "application",
        }
    }

    /// The template a name means, when it means one.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|template| template.name() == name.trim())
    }
}

impl fmt::Display for Template {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.name())
    }
}

/// Where a new project's identity comes from.
///
/// Injected rather than read from the clock inside the creation path, for the same reason every
/// other clock in this build is a parameter: "the same request twice" has to be a thing a test can
/// do.
pub trait ProjectIds {
    /// The identity for the next project. Opaque: nothing but the project document reads it.
    fn next_id(&self) -> String;
}

/// The identity source this build ships: the time, the process and a counter, in hexadecimal.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemProjectIds;

impl ProjectIds for SystemProjectIds {
    fn next_id(&self) -> String {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let milliseconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|since| since.as_millis())
            .unwrap_or_default();
        format!(
            "p{:x}-{:x}-{:04x}",
            milliseconds,
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        )
    }
}

/// What the caller asked for.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct NewProjectRequest {
    directory: PathBuf,
    name: String,
    template: Template,
}

impl NewProjectRequest {
    /// A request for a project in a directory, named after the directory.
    ///
    /// A directory with no name of its own (a mount point, say) leaves the name empty, and the plan
    /// refuses it rather than this build inventing one.
    pub fn new(directory: impl Into<PathBuf>, template: Template) -> Self {
        let directory = directory.into();
        let name = directory
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        Self {
            directory,
            name,
            template,
        }
    }

    /// The same request, with the name a person typed.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// The directory the project would be created in.
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// What the project would be called.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// What it would start from.
    pub const fn template(&self) -> Template {
        self.template
    }
}

/// One file a plan would write.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PlannedFile {
    /// Where it goes, relative to the project root.
    path: String,
    /// What it is, in words, for the wizard's last step.
    what: &'static str,
    /// What would be in it.
    contents: String,
}

impl PlannedFile {
    /// The path, relative to the project root, with forward slashes.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// What the file is.
    pub const fn what(&self) -> &'static str {
        self.what
    }

    /// What would be written.
    pub fn contents(&self) -> &str {
        &self.contents
    }
}

/// What a wizard shows on its last step: the change, and what could go wrong.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CreationPlan {
    root: PathBuf,
    name: String,
    id: String,
    template: Template,
    files: Vec<PlannedFile>,
    target_preexisted: bool,
    staging: PathBuf,
    impact: Vec<String>,
}

impl CreationPlan {
    /// The project directory that would be created.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// What the project would be called.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Which project it would be.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// What it starts from.
    pub const fn template(&self) -> Template {
        self.template
    }

    /// Every file that would be written, in the order it would be written.
    pub fn files(&self) -> &[PlannedFile] {
        &self.files
    }

    /// Whether the project directory already exists and is empty.
    pub const fn target_preexisted(&self) -> bool {
        self.target_preexisted
    }

    /// Where the files are written before they become a project.
    pub fn staging(&self) -> &Path {
        &self.staging
    }

    /// What can still go wrong, and what that would leave behind.
    pub fn impact(&self) -> &[String] {
        &self.impact
    }
}

/// Why a project was not created.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum CreationRefusal {
    /// The project would have no name.
    NameIsEmpty,
    /// The path is there, and it is not a directory.
    TargetIsAFile {
        /// The path.
        path: PathBuf,
    },
    /// The path is a directory with something in it.
    TargetNotEmpty {
        /// The path.
        path: PathBuf,
        /// How many entries are in the way.
        entries: usize,
    },
    /// The directory the project would go in is not there.
    ParentMissing {
        /// The path.
        path: PathBuf,
    },
    /// The staging directory is already there, which means another attempt is running or stopped.
    StagingExists {
        /// The path.
        path: PathBuf,
    },
    /// There is nothing staged to commit.
    NothingStaged,
    /// Something already staged cannot be staged again.
    AlreadyStaged,
    /// The files that were written are not a project this build can read back.
    Unsuitable {
        /// The path of the file that does not fit.
        path: PathBuf,
        /// What is wrong with it.
        reason: String,
    },
    /// The filesystem refused.
    Io {
        /// The path that was being written.
        path: PathBuf,
        /// What was being done.
        action: &'static str,
        /// The operating system's own words.
        reason: String,
    },
}

impl CreationRefusal {
    /// A sentence naming what was found, for a diagnostic or a terminal.
    pub fn message(&self) -> String {
        match self {
            Self::NameIsEmpty => {
                "the project would have no name; name it, and nothing has to be invented".to_owned()
            }
            Self::TargetIsAFile { path } => {
                format!("{} is a file, not a directory", path.display())
            }
            Self::TargetNotEmpty { path, entries } => format!(
                "{} already exists and holds {entries} entry/entries; this build never creates a \
                 project over something that is already there",
                path.display()
            ),
            Self::ParentMissing { path } => format!(
                "{} does not exist; create it first, so a typo cannot make a tree",
                path.display()
            ),
            Self::StagingExists { path } => format!(
                "{} is already there, so an earlier attempt did not finish or is running",
                path.display()
            ),
            Self::NothingStaged => "there is nothing staged to commit".to_owned(),
            Self::AlreadyStaged => "the files were already staged".to_owned(),
            Self::Unsuitable { path, reason } => format!(
                "{} was written, but this build cannot read it back: {reason}; nothing was put in \
                 place",
                path.display()
            ),
            Self::Io {
                path,
                action,
                reason,
            } => format!("{}: {action}: {reason}", path.display()),
        }
    }
}

impl fmt::Display for CreationRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.message())
    }
}

/// What an abandoned attempt left behind.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Cleanup {
    /// Whether a staging directory was there and is now gone.
    pub staging_removed: bool,
    /// How many files were taken away with it.
    pub files_removed: usize,
    /// Whether the project directory is exactly as it was.
    pub target_untouched: bool,
}

impl Cleanup {
    /// A sentence saying what was taken away.
    pub fn message(&self) -> String {
        let staging = if self.staging_removed {
            "the staged copy was removed"
        } else {
            "there was no staged copy to remove"
        };
        let target = if self.target_untouched {
            "the project directory is as it was"
        } else {
            "the project directory was created"
        };
        format!("{staging} ({} file(s)); {target}", self.files_removed)
    }
}

/// A creation that failed, and what it left behind.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CreationFailure {
    refusal: CreationRefusal,
    cleanup: Cleanup,
}

impl CreationFailure {
    /// Why it failed.
    pub fn refusal(&self) -> &CreationRefusal {
        &self.refusal
    }

    /// What was taken away.
    pub fn cleanup(&self) -> &Cleanup {
        &self.cleanup
    }

    /// A sentence for a person: what went wrong, and what is (not) left behind.
    pub fn message(&self) -> String {
        format!("{}; {}", self.refusal.message(), self.cleanup.message())
    }
}

impl fmt::Display for CreationFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.message())
    }
}

/// A project that exists.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CreatedProject {
    root: PathBuf,
    files: Vec<PathBuf>,
    target_preexisted: bool,
}

impl CreatedProject {
    /// The project directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Every file that was written, in the order it was written.
    pub fn files(&self) -> &[PathBuf] {
        &self.files
    }

    /// Whether the project directory was already there and empty.
    pub const fn target_preexisted(&self) -> bool {
        self.target_preexisted
    }

    /// Opens what was just created, which is what proves it is a project this build can read.
    pub fn open(&self) -> Result<ProjectFile, ProjectError> {
        let root = ProjectRoot::new(&self.root).map_err(|error| ProjectError::Io {
            path: self.root.clone(),
            action: "anchoring the created project",
            reason: error.message(),
        })?;
        ProjectFile::read_in(&root, Path::new(PROJECT_FILE_NAME))
    }
}

/// A creation in progress.
#[derive(Clone, Debug)]
pub struct Creation {
    request: NewProjectRequest,
    plan: CreationPlan,
    staged: bool,
}

impl Creation {
    /// Plan a project: the wizard's last step, and the only place a creation can be refused before
    /// anything is written.
    pub fn plan(request: NewProjectRequest, ids: &dyn ProjectIds) -> Result<Self, CreationRefusal> {
        let name = request.name.trim().to_owned();
        if name.is_empty() {
            return Err(CreationRefusal::NameIsEmpty);
        }
        let root = request.directory.clone();
        let mut target_preexisted = false;
        match std::fs::metadata(&root) {
            Ok(metadata) if metadata.is_dir() => {
                let entries = std::fs::read_dir(&root)
                    .map_err(|error| CreationRefusal::Io {
                        path: root.clone(),
                        action: "reading the project directory",
                        reason: error.to_string(),
                    })?
                    .count();
                if entries > 0 {
                    return Err(CreationRefusal::TargetNotEmpty {
                        path: root.clone(),
                        entries,
                    });
                }
                target_preexisted = true;
            }
            Ok(_) => return Err(CreationRefusal::TargetIsAFile { path: root.clone() }),
            Err(_) => {
                let parent = root
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| PathBuf::from("."));
                if !parent.is_dir() {
                    return Err(CreationRefusal::ParentMissing { path: parent });
                }
            }
        }

        let parent = root
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let directory_name = root
            .file_name()
            .map(|file| file.to_string_lossy().into_owned())
            .unwrap_or_else(|| "project".to_owned());
        let staging = parent.join(format!(
            ".{directory_name}.rustage-staging-{}-{:04x}",
            std::process::id(),
            next_counter()
        ));
        if staging.exists() {
            return Err(CreationRefusal::StagingExists { path: staging });
        }

        let id = ids.next_id();
        let files = template_files(request.template, &name, &id, &root)?;
        let mut impact = vec![format!(
            "nothing is written into {} until the last step: the files are written to {} and moved \
             into place in one step",
            root.display(),
            staging.display()
        )];
        if target_preexisted {
            impact.push(format!(
                "{} already exists and is empty; it is filled by that move and by nothing else",
                root.display()
            ));
        }
        impact.push(
            "a failure or a cancel removes the staged copy and leaves the project directory as it \
             was"
            .to_owned(),
        );

        let template = request.template;
        Ok(Self {
            request,
            plan: CreationPlan {
                root,
                name,
                id,
                template,
                files,
                target_preexisted,
                staging,
                impact,
            },
            staged: false,
        })
    }

    /// The last step's material: the change, and what could go wrong.
    pub fn summary(&self) -> &CreationPlan {
        &self.plan
    }

    /// What the caller asked for.
    pub fn request(&self) -> &NewProjectRequest {
        &self.request
    }

    /// Whether the files are staged and waiting to be committed.
    pub const fn is_staged(&self) -> bool {
        self.staged
    }

    /// Writes the planned files into the staging directory.
    ///
    /// A failure here takes the staging directory away again and reports both, so a caller that did
    /// not check the return value still cannot mistake a failure for progress.
    pub fn stage(&mut self) -> Result<&CreationPlan, CreationFailure> {
        if self.staged {
            return Err(CreationFailure {
                refusal: CreationRefusal::AlreadyStaged,
                cleanup: Cleanup {
                    staging_removed: false,
                    files_removed: 0,
                    target_untouched: true,
                },
            });
        }
        let outcome = write_files(&self.plan).and_then(|()| verify_staged(&self.plan));
        match outcome {
            Ok(()) => {
                self.staged = true;
                Ok(&self.plan)
            }
            Err(refusal) => Err(CreationFailure {
                refusal,
                cleanup: remove_staging(&self.plan),
            }),
        }
    }

    /// Moves the staged project into place.
    ///
    /// One rename: either the project is there or the directory that was there is untouched. If the
    /// move is refused — by the filesystem, or because something appeared in the project directory
    /// while the wizard was open — the staged copy is removed and the refusal says so.
    pub fn commit(self) -> Result<CreatedProject, CreationFailure> {
        if !self.staged {
            return Err(CreationFailure {
                refusal: CreationRefusal::NothingStaged,
                cleanup: Cleanup {
                    staging_removed: false,
                    files_removed: 0,
                    target_untouched: true,
                },
            });
        }
        if let Err(error) = std::fs::rename(&self.plan.staging, &self.plan.root) {
            return Err(CreationFailure {
                refusal: CreationRefusal::Io {
                    path: self.plan.root.clone(),
                    action: "moving the staged project into place",
                    reason: error.to_string(),
                },
                cleanup: remove_staging(&self.plan),
            });
        }
        Ok(CreatedProject {
            root: self.plan.root.clone(),
            files: self
                .plan
                .files()
                .iter()
                .map(|file| self.plan.root.join(file.path()))
                .collect(),
            target_preexisted: self.plan.target_preexisted,
        })
    }

    /// Takes the attempt away: the staged copy goes, the project directory is never touched.
    pub fn cancel(self) -> Cleanup {
        remove_staging(&self.plan)
    }
}

fn next_counter() -> u16 {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed) as u16
}

fn remove_staging(plan: &CreationPlan) -> Cleanup {
    let files_removed = count_files(&plan.staging);
    let staging_removed = std::fs::remove_dir_all(&plan.staging).is_ok();
    Cleanup {
        staging_removed,
        files_removed: if staging_removed { files_removed } else { 0 },
        target_untouched: !plan.root.exists() || plan.target_preexisted,
    }
}

/// How many files are really there, so an abandoned attempt reports what it took away rather than
/// what it meant to write.
fn count_files(directory: &Path) -> usize {
    let mut count = 0;
    let mut stack = vec![directory.to_path_buf()];
    while let Some(path) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&path) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                count += 1;
            }
        }
    }
    count
}

/// Reads back what was staged, in the place it was staged, before it becomes a project.
///
/// This is the check that makes "a created project is a project" a fact rather than a hope: the same
/// readers the designer uses read the staged files, so a writer that produced something its own
/// reader refuses stops the creation instead of shipping it.
fn verify_staged(plan: &CreationPlan) -> Result<(), CreationRefusal> {
    let root = ProjectRoot::new(&plan.staging).map_err(|error| CreationRefusal::Unsuitable {
        path: plan.staging.clone(),
        reason: error.message(),
    })?;
    let file = ProjectFile::read_in(&root, Path::new(PROJECT_FILE_NAME)).map_err(|error| {
        CreationRefusal::Unsuitable {
            path: plan.staging.join(PROJECT_FILE_NAME),
            reason: error.message(),
        }
    })?;
    let Some(document) = file.content().document() else {
        return Err(CreationRefusal::Unsuitable {
            path: plan.staging.join(PROJECT_FILE_NAME),
            reason: format!(
                "the project document declares {}, which this build does not write",
                file.version()
            ),
        });
    };
    if let Some(reference) = document.refused_references().first() {
        return Err(CreationRefusal::Unsuitable {
            path: plan.staging.join(PROJECT_FILE_NAME),
            reason: format!(
                "the project would reference `{}`, which leaves its own root",
                reference.written()
            ),
        });
    }
    for page in document.pages() {
        PageDocument::read(&root, page.document().written()).map_err(|error| {
            CreationRefusal::Unsuitable {
                path: plan.staging.join(page.document().written()),
                reason: error.message(),
            }
        })?;
    }
    Ok(())
}

fn write_files(plan: &CreationPlan) -> Result<(), CreationRefusal> {
    std::fs::create_dir_all(&plan.staging).map_err(|error| CreationRefusal::Io {
        path: plan.staging.clone(),
        action: "creating the staging directory",
        reason: error.to_string(),
    })?;
    for file in plan.files() {
        let path = plan.staging.join(file.path());
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| CreationRefusal::Io {
                path: parent.to_path_buf(),
                action: "creating a directory of the new project",
                reason: error.to_string(),
            })?;
        }
        std::fs::write(&path, file.contents()).map_err(|error| CreationRefusal::Io {
            path,
            action: "writing a file of the new project",
            reason: error.to_string(),
        })?;
    }
    Ok(())
}

/// The files a template describes.
///
/// Every document here is produced by the writer that owns the format, so a created project is the
/// same kind of file as one read back ([`ProjectDraft`], [`PageDocument`]).
fn template_files(
    template: Template,
    name: &str,
    id: &str,
    root: &Path,
) -> Result<Vec<PlannedFile>, CreationRefusal> {
    // The project does not exist yet, so the template is described against the directory it will go
    // in. The paths a template uses are its own literals, and what proves they stay inside the new
    // project is that the staged files are read back with the *project* as their root before
    // anything is put in place (see `verify_staged`).
    let parent = root
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let anchor = ProjectRoot::new(&parent).map_err(|error| CreationRefusal::Io {
        path: parent.clone(),
        action: "anchoring the directory the project goes in",
        reason: error.message(),
    })?;

    let page_document = "pages/main.json";
    let stylesheet = "styles/main.css";
    let generated = "src/generated/main.rs";

    let mut draft = match template {
        Template::Empty => ProjectDraft::new(&anchor, id, name),
        Template::BasicPage | Template::Application => {
            ProjectDraft::new(&anchor, id, name).map(|draft| {
                // A page project starts with its page: a project whose entry nobody chose would need
                // a person to choose one before anything could run.
                draft
                    .with_page(page_document, Some(name), Some(stylesheet))
                    .with_startup(page_document)
                    .with_target("desktop", Some("debug"), &["desktop"])
            })
        }
    }
    .map_err(|error| CreationRefusal::Io {
        path: root.to_path_buf(),
        action: "describing the new project",
        reason: error.message(),
    })?;
    if template == Template::Application {
        draft = draft.with_generated(generated, page_document);
    }
    let document = draft.build().map_err(|error| CreationRefusal::Io {
        path: root.to_path_buf(),
        action: "checking the new project",
        reason: error.message(),
    })?;
    let project_text = document.render();

    let mut files = vec![PlannedFile {
        path: PROJECT_FILE_NAME.to_owned(),
        what: "the project document",
        contents: project_text,
    }];
    if template != Template::Empty {
        let page = PageDocument::empty_page(
            PageId::from_raw(1),
            NodeId::from_raw(1),
            name,
            // A page document is also the unit a window is described by (decision Q5), and the
            // generator's document has a `window` root. A template whose root kind was `page`
            // produced a design that no generator could consume: J01's first-app chain ended at
            // "no control by that name". The root of a new page is therefore the window it is.
            NodeKind::new("window").expect("`window` is a kind"),
            page_document,
        )
        .ok_or(CreationRefusal::NameIsEmpty)?;
        let page_text = page.render().map_err(|error| CreationRefusal::Io {
            path: root.join(page_document),
            action: "writing the first page",
            reason: error.message(),
        })?;
        files.push(PlannedFile {
            path: page_document.to_owned(),
            what: "the first page",
            contents: page_text,
        });
        files.push(PlannedFile {
            path: stylesheet.to_owned(),
            what: "the page's stylesheet",
            contents: STYLESHEET.to_owned(),
        });
    }
    Ok(files)
}

/// The stylesheet a new page starts with.
///
/// One rule and no more: a template that shipped a design system would be deciding what somebody's
/// application looks like, and the theme tasks own that (§F13/F14).
const STYLESHEET: &str = "\
/* SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli */
/* SPDX-License-Identifier: MIT */

.page {
  display: flex;
  flex-direction: column;
}
";

#[cfg(test)]
mod tests {
    use super::{Creation, CreationRefusal, NewProjectRequest, ProjectIds, Template};
    use crate::project::record::format::PROJECT_FILE_NAME;
    use crate::project::record::page::PageDocument;
    use std::path::{Path, PathBuf};
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
        let root = std::env::temp_dir().join(format!("rustage-create-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&root).expect("a temporary directory is creatable");
        Sandbox {
            root: root.clone(),
            _guard: TempDirectory(root),
        }
    }

    /// An identity source that says the same thing every time, so a test can name the identity it
    /// expects instead of reading it out of the result.
    struct FixedIds;

    impl ProjectIds for FixedIds {
        fn next_id(&self) -> String {
            "test-id-0001".to_owned()
        }
    }

    impl Sandbox {
        fn project(&self, template: Template) -> NewProjectRequest {
            NewProjectRequest::new(self.root.join("demo"), template).with_name("Demo")
        }

        /// One directory per template, so a test can create every template in one sandbox.
        fn project_of(&self, template: Template) -> NewProjectRequest {
            NewProjectRequest::new(
                self.root.join(format!("demo-{}", template.name())),
                template,
            )
            .with_name("Demo")
        }

        fn entries(&self, path: &Path) -> Vec<String> {
            let mut names: Vec<String> = std::fs::read_dir(path)
                .map(|entries| {
                    entries
                        .filter_map(|entry| entry.ok())
                        .map(|entry| entry.file_name().to_string_lossy().into_owned())
                        .collect()
                })
                .unwrap_or_default();
            names.sort();
            names
        }
    }

    #[test]
    fn the_last_step_names_every_file_and_what_could_go_wrong() {
        let sandbox = sandbox();
        let creation = Creation::plan(sandbox.project(Template::Application), &FixedIds)
            .expect("a plan for a directory that is not there");
        let plan = creation.summary();

        assert_eq!(plan.name(), "Demo");
        assert_eq!(plan.id(), "test-id-0001");
        assert_eq!(plan.template(), Template::Application);
        assert!(!plan.target_preexisted());
        assert_eq!(plan.root(), sandbox.root.join("demo"));
        assert_eq!(
            plan.files()
                .iter()
                .map(|file| file.path())
                .collect::<Vec<_>>(),
            [PROJECT_FILE_NAME, "pages/main.json", "styles/main.css"]
        );
        assert_eq!(plan.files()[0].what(), "the project document");
        assert!(plan.files()[0].contents().contains("\"name\": \"Demo\""));
        assert!(plan.files()[0].contents().contains("\"generated\""));
        // The first page's root is the window the page is; the generator consumes `window`,
        // not the document's own `page` metadata member.
        assert!(plan.files()[1].contents().contains("\"type\": \"window\""));
        // The staging directory is beside the project, on the same filesystem, so the move is one
        // operation.
        assert_eq!(plan.staging().parent(), Some(sandbox.root.as_path()));
        assert!(plan.staging().is_absolute() || plan.staging().parent().is_some());
        assert!(
            plan.impact()
                .iter()
                .any(|line| line.contains("nothing is written"))
        );
        assert!(
            plan.impact()
                .iter()
                .any(|line| line.contains("cancel removes"))
        );
        assert!(!Path::new(plan.root()).exists(), "planning writes nothing");
    }

    #[test]
    fn the_templates_differ_by_what_the_project_document_names() {
        let cases = [
            (Template::Empty, vec![PROJECT_FILE_NAME], false, false),
            (
                Template::BasicPage,
                vec![PROJECT_FILE_NAME, "pages/main.json", "styles/main.css"],
                true,
                false,
            ),
            (
                Template::Application,
                vec![PROJECT_FILE_NAME, "pages/main.json", "styles/main.css"],
                true,
                true,
            ),
        ];
        for (template, files, has_page, has_generated) in cases {
            let sandbox = sandbox();
            let creation = Creation::plan(sandbox.project(template), &FixedIds).expect("a plan");
            let document = &creation.summary().files()[0].contents();
            assert_eq!(
                creation
                    .summary()
                    .files()
                    .iter()
                    .map(|file| file.path())
                    .collect::<Vec<_>>(),
                files,
                "{template} writes these files"
            );
            assert_eq!(
                document.contains("\"pages\": []"),
                !has_page,
                "{template}: an empty project has an empty page list, a page template has a page"
            );
            assert_eq!(
                document.contains("\"generated\": []"),
                !has_generated,
                "{template}: the generated mapping"
            );
            assert_eq!(
                document.contains("src/generated/main.rs"),
                has_generated,
                "{template}: where generation writes"
            );
            // Never an empty member list: every member the format defines is written.
            assert!(document.contains("\"resources\": []"));
            assert_eq!(Template::from_name(template.name()), Some(template));
        }
        assert_eq!(Template::from_name("nowhere"), None);
        assert_eq!(Template::ALL.len(), 3);
    }

    #[test]
    fn a_project_that_is_created_is_a_project_this_build_reads_back() {
        let sandbox = sandbox();
        for template in Template::ALL {
            let creation = Creation::plan(sandbox.project_of(template), &FixedIds).expect("a plan");
            let root = creation.summary().root().to_path_buf();
            let mut creation = creation;
            creation
                .stage()
                .expect("the files are written where they are not a project yet");
            assert!(creation.is_staged());
            assert!(
                !root.join(PROJECT_FILE_NAME).exists(),
                "staging is beside the project, not in it"
            );

            let created = creation
                .commit()
                .expect("the staged project moves into place");
            assert_eq!(created.root(), root);
            assert!(!created.target_preexisted());
            assert!(root.join(PROJECT_FILE_NAME).is_file());
            assert_eq!(
                sandbox
                    .entries(&sandbox.root)
                    .into_iter()
                    .filter(|name| name.contains("staging"))
                    .count(),
                0,
                "no staging directory is left behind"
            );

            let file = created.open().expect("what was created is readable");
            let document = file.content().document().expect("an interpreted document");
            assert_eq!(document.identity().name(), "Demo");
            assert_eq!(document.identity().id(), "test-id-0001");
            assert_eq!(
                document.pages().len(),
                usize::from(template != Template::Empty)
            );
            if template != Template::Empty {
                // The page document the project names is there, and its own reader reads it.
                let project_root =
                    crate::host::paths::ProjectRoot::new(&root).expect("the project root");
                let page =
                    PageDocument::read(&project_root, document.pages()[0].document().written())
                        .expect("the page reads back");
                assert_eq!(page.page().name(), "Demo");
                assert_eq!(page.nodes().len(), 1);
                assert!(document.pages()[0].stylesheet().is_some());
            }
        }
    }

    #[test]
    fn a_failure_or_a_cancel_leaves_the_directory_as_it_was() {
        let sandbox = sandbox();
        let target = sandbox.root.join("demo");
        std::fs::create_dir_all(&target).expect("an empty directory");

        // An empty directory is allowed, and is filled by the move.
        let creation = Creation::plan(
            NewProjectRequest::new(&target, Template::BasicPage).with_name("Demo"),
            &FixedIds,
        )
        .expect("an empty directory is a place for a project");
        assert!(creation.summary().target_preexisted());
        let cancelled = creation.cancel();
        assert!(cancelled.target_untouched);
        assert_eq!(sandbox.entries(&target), Vec::<String>::new());

        // A cancel after staging takes the staged copy away and leaves the empty directory empty.
        let mut creation = Creation::plan(
            NewProjectRequest::new(&target, Template::BasicPage).with_name("Demo"),
            &FixedIds,
        )
        .expect("a plan");
        creation.stage().expect("staged");
        let staging = creation.summary().staging().to_path_buf();
        assert!(staging.is_dir());
        let cancelled = creation.cancel();
        assert!(cancelled.staging_removed);
        assert_eq!(cancelled.files_removed, 3);
        assert!(cancelled.target_untouched);
        assert!(!staging.exists());
        assert_eq!(sandbox.entries(&target), Vec::<String>::new());
        assert!(cancelled.message().contains("the staged copy was removed"));

        // The second cancel has nothing to do, and says so instead of claiming a removal.
        let creation = Creation::plan(
            NewProjectRequest::new(&target, Template::BasicPage).with_name("Demo"),
            &FixedIds,
        )
        .expect("a plan");
        let again = creation.cancel();
        assert!(!again.staging_removed);
        assert!(again.message().contains("there was no staged copy"));
    }

    #[test]
    fn a_failure_does_not_overwrite_a_directory_that_has_something_in_it() {
        let sandbox = sandbox();
        let target = sandbox.root.join("demo");
        std::fs::create_dir_all(&target).expect("a directory");
        std::fs::write(target.join("important.txt"), "keep me").expect("a file");

        let refusal = Creation::plan(
            NewProjectRequest::new(&target, Template::Application).with_name("Demo"),
            &FixedIds,
        )
        .expect_err("a directory with something in it is refused");
        match &refusal {
            CreationRefusal::TargetNotEmpty { entries, .. } => assert_eq!(*entries, 1),
            other => panic!("expected a non-empty target, found {other:?}"),
        }
        assert!(refusal.message().contains("never creates a project over"));
        assert_eq!(
            std::fs::read_to_string(target.join("important.txt")).unwrap(),
            "keep me"
        );
        assert_eq!(sandbox.entries(&target), ["important.txt"]);
        assert_eq!(
            sandbox.entries(&sandbox.root),
            ["demo"],
            "nothing was staged either"
        );

        // A file where the project should go is refused too.
        let as_file = sandbox.root.join("a-file");
        std::fs::write(&as_file, "x").expect("a file");
        let refusal = Creation::plan(
            NewProjectRequest::new(&as_file, Template::Empty).with_name("Demo"),
            &FixedIds,
        )
        .expect_err("a file is not a directory");
        assert!(matches!(refusal, CreationRefusal::TargetIsAFile { .. }));

        // And a parent that is not there is refused rather than created.
        let refusal = Creation::plan(
            NewProjectRequest::new(sandbox.root.join("nowhere/demo"), Template::Empty)
                .with_name("Demo"),
            &FixedIds,
        )
        .expect_err("a missing parent is refused");
        assert!(matches!(refusal, CreationRefusal::ParentMissing { .. }));
        assert!(refusal.message().contains("create it first"));
    }

    #[test]
    fn a_path_that_appeared_while_the_wizard_was_open_stops_the_move() {
        let sandbox = sandbox();
        let target = sandbox.root.join("demo");
        std::fs::create_dir_all(&target).expect("an empty directory");
        let mut creation = Creation::plan(
            NewProjectRequest::new(&target, Template::BasicPage).with_name("Demo"),
            &FixedIds,
        )
        .expect("a plan");
        creation.stage().expect("staged");

        // Somebody else writes into the directory between the last step and the move.
        std::fs::write(target.join("somebody-elses.txt"), "mine").expect("a file");

        let failure = creation.commit().expect_err("the move is refused");
        assert!(matches!(failure.refusal(), CreationRefusal::Io { .. }));
        assert_eq!(
            std::fs::read_to_string(target.join("somebody-elses.txt")).unwrap(),
            "mine",
            "what was there is still there"
        );
        assert_eq!(sandbox.entries(&target), ["somebody-elses.txt"]);
        assert!(failure.cleanup().staging_removed);
        assert!(
            failure
                .message()
                .contains("the project directory is as it was")
        );
        assert!(
            !sandbox
                .entries(&sandbox.root)
                .iter()
                .any(|name| name.contains("staging"))
        );
    }

    #[test]
    fn committing_something_that_was_never_staged_is_refused_with_a_reason() {
        let sandbox = sandbox();
        let creation = Creation::plan(sandbox.project(Template::Empty), &FixedIds).expect("a plan");
        let failure = creation.commit().expect_err("there is nothing to commit");
        assert_eq!(failure.refusal(), &CreationRefusal::NothingStaged);
        assert!(!sandbox.root.join("demo").exists());
    }

    #[test]
    fn a_project_without_a_name_is_refused_rather_than_named_by_us() {
        let sandbox = sandbox();
        let request =
            NewProjectRequest::new(sandbox.root.join("demo"), Template::Empty).with_name("   ");
        let refusal = Creation::plan(request, &FixedIds).expect_err("a blank name");
        assert_eq!(refusal, CreationRefusal::NameIsEmpty);
        assert!(refusal.message().contains("nothing has to be invented"));

        // A directory whose own name is the project's name needs nothing added.
        let derived = NewProjectRequest::new(sandbox.root.join("demo"), Template::Empty);
        assert_eq!(derived.name(), "demo");
        assert_eq!(derived.directory(), sandbox.root.join("demo"));
        assert_eq!(derived.template(), Template::Empty);
    }

    #[test]
    fn the_identity_source_is_asked_once_and_its_answer_is_written_down() {
        struct CountingIds(AtomicUsize);

        impl ProjectIds for CountingIds {
            fn next_id(&self) -> String {
                let count = self.0.fetch_add(1, Ordering::Relaxed);
                format!("id-{count}")
            }
        }

        let sandbox = sandbox();
        let ids = CountingIds(AtomicUsize::new(0));
        let first = Creation::plan(sandbox.project(Template::Empty), &ids).expect("a plan");
        let second = Creation::plan(sandbox.project(Template::Empty), &ids).expect("a plan");
        assert_eq!(first.summary().id(), "id-0");
        assert_eq!(second.summary().id(), "id-1");
        assert_eq!(ids.0.load(Ordering::Relaxed), 2);

        // What the built-in source produces is opaque but usable, and two calls differ.
        let system = super::SystemProjectIds;
        let one = system.next_id();
        let two = system.next_id();
        assert_ne!(one, two);
        assert!(one.starts_with('p'));
    }

    #[test]
    fn a_staged_creation_is_read_back_before_it_is_put_in_place() {
        let sandbox = sandbox();
        let mut creation =
            Creation::plan(sandbox.project(Template::Application), &FixedIds).expect("a plan");
        creation.stage().expect("the files are readable back");
        let plan = creation.summary();
        let staging = plan.staging().to_path_buf();
        // The staged copy is a whole project: its document, its page and its stylesheet.
        assert!(staging.join(PROJECT_FILE_NAME).is_file());
        assert!(staging.join("pages/main.json").is_file());
        assert!(staging.join("styles/main.css").is_file());
        assert_eq!(creation.request().template(), Template::Application);

        // Staging twice is refused instead of writing over what is there.
        let failure = creation.stage().expect_err("already staged");
        assert_eq!(failure.refusal(), &CreationRefusal::AlreadyStaged);
        assert_eq!(failure.cleanup().files_removed, 0);
        assert!(failure.cleanup().target_untouched);

        let created = creation.commit().expect("committed");
        assert_eq!(created.files().len(), 3);
        assert!(created.files()[0].ends_with(PROJECT_FILE_NAME));
    }
}
