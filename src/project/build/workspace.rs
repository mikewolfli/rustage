// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! A workspace: several roots that belong together (task T02.09).
//!
//! A project is a directory with a metadata document in it, and every path in it is relative to that
//! directory (decision `F1`). A workspace is one level up: several such directories, each its own
//! root, in one directory with a `workspace.json` that says which members there are. It is
//! deliberately **not** a second project document: pages, targets and services belong to a member,
//! and duplicating them here would be the second writable model the project forbids.
//!
//! Three things this module is arranged around, because they are the acceptance:
//!
//! * **A path is not an identity once there is more than one root.** Two members that both name
//!   `pages/main.json` name two different files, so every file here is identified by *the member that
//!   names it* and the path inside that member ([`WorkspaceFile`]). The file list is the acceptance
//!   made visible: one row per (member, path), never one row per path.
//! * **A path that leaves its member but stays in the workspace is not refused — it is qualified.**
//!   [`Workspace::resolve`] answers with the member it lands in, or with the reason it leaves the
//!   workspace. That is what cross-root references mean at this stage: the member's own reader refuses
//!   the path (it may not leave its root), and the workspace explains where it goes, without rewriting
//!   anybody's document.
//! * **Removing a member touches no directory.** A member is a line in the workspace document, so
//!   adding and removing are edits to that one document ([`WorkspacePlan`]); nothing is created and
//!   nothing is deleted. Removing is *blocked* while another member's document references the one
//!   being removed, and the references are listed rather than fixed silently.
//!
//! Members may not nest. If one member's root were inside another's, a path would belong to two of
//! them and "which member is this file in" would have two answers; the workspace refuses that at read
//! time, where it can be refused once instead of guessed at every lookup.
//!
//! Which member is *active* is this machine's state, not the document's: it is what somebody is
//! working on now, like the active target in [`crate::project::build::configuration`], so it lives in the
//! host's configuration directory keyed by the workspace's identity ([`WorkspaceState`]). Nothing in
//! the shared document changes when somebody switches.

use std::{
    cmp::Ordering,
    fmt,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::contracts::revision::FormatVersion;
use crate::host::paths::{PathError, ProjectRoot};
use crate::project::record::format::{
    PROJECT_FILE_NAME, ProjectError, ProjectFile, format_version,
};
use crate::project::record::json::{self, Json, JsonKind, JsonObject};

/// The document that says which members a workspace has.
pub const WORKSPACE_FILE_NAME: &str = "workspace.json";

/// The version of a workspace document this build writes.
pub const WORKSPACE_FORMAT: FormatVersion = FormatVersion::from_raw(1);

/// The members a workspace document has, in the order it is written.
pub const WORKSPACE_MEMBERS: [&str; 3] = ["format", "workspace", "members"];

/// The members the workspace's own identity has.
pub const IDENTITY_MEMBERS: [&str; 2] = ["id", "name"];

/// The members one workspace member has: what it is called, and where it is.
pub const MEMBER_MEMBERS: [&str; 2] = ["name", "root"];

/// What a member's directory holds, as the workspace read it.
///
/// The distinction that matters is between *no project document yet* and *a project document this
/// build cannot use*: the first is a directory somebody is about to make a project in, the second is
/// a project from a newer build, and reporting them the same way would either invent an identity or
/// hide one.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum MemberHolds {
    /// A project document this build reads, in a directory that is there.
    Project {
        /// The member's directory.
        root: PathBuf,
        /// The project's identity.
        id: String,
        /// The project's name.
        name: String,
        /// The format version the file declares, which is not always the one this build interprets.
        declared: FormatVersion,
    },
    /// A project document that declares a version this build does not interpret.
    Foreign {
        /// The member's directory.
        root: PathBuf,
        /// The version the document declares.
        version: FormatVersion,
    },
    /// A project document that could not be read at all.
    Unreadable {
        /// The member's directory.
        root: PathBuf,
        /// What the reader said.
        reason: String,
    },
    /// A directory that holds no project document.
    NothingYet {
        /// The member's directory.
        root: PathBuf,
    },
    /// The directory the document names cannot be resolved, or is not a directory.
    NoDirectory {
        /// Where it would be.
        path: PathBuf,
        /// Why not.
        reason: String,
    },
}

impl MemberHolds {
    /// The member's directory, when it is one.
    pub fn root(&self) -> Option<&Path> {
        match self {
            Self::Project { root, .. }
            | Self::Foreign { root, .. }
            | Self::Unreadable { root, .. }
            | Self::NothingYet { root } => Some(root),
            Self::NoDirectory { .. } => None,
        }
    }

    /// The member's directory, or where it would be when the directory is not there.
    ///
    /// Both questions are asked: whether the directory is one (which decides whether a project
    /// document can be read from it) and where the member is (which decides which member a path
    /// belongs to, and whether two members overlap). A member whose directory went missing is still
    /// a member, and it still owns the paths under it.
    pub fn path(&self) -> &Path {
        match self {
            Self::Project { root, .. }
            | Self::Foreign { root, .. }
            | Self::Unreadable { root, .. }
            | Self::NothingYet { root } => root,
            Self::NoDirectory { path, .. } => path,
        }
    }

    /// The project's identity, when this build read one.
    pub fn id(&self) -> Option<&str> {
        match self {
            Self::Project { id, .. } => Some(id),
            _ => None,
        }
    }

    /// The project's name, when this build read one.
    pub fn name(&self) -> Option<&str> {
        match self {
            Self::Project { name, .. } => Some(name),
            _ => None,
        }
    }

    /// Whether a project document this build interprets is there.
    pub const fn is_project(&self) -> bool {
        matches!(self, Self::Project { .. })
    }

    /// What the member holds, in a person's words.
    pub fn render(&self) -> String {
        match self {
            Self::Project {
                id, name, declared, ..
            } => format!("project `{id}` ({name}), {declared}"),
            Self::Foreign { version, .. } => {
                format!("format v{version}, which this build does not interpret")
            }
            Self::Unreadable { reason, .. } => {
                format!("a project document that cannot be read: {reason}")
            }
            Self::NothingYet { .. } => "no project document yet".to_owned(),
            Self::NoDirectory { reason, .. } => format!("no directory: {reason}"),
        }
    }
}

/// One root of a workspace: what it is called, and what is there.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Member {
    name: String,
    written: String,
    holds: MemberHolds,
}

impl Member {
    /// The name this member is addressed by, which is unique in its workspace.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The root as the document writes it, which is relative to the workspace root.
    pub fn written(&self) -> &str {
        &self.written
    }

    /// The member's directory, when it is one.
    pub fn root(&self) -> Option<&Path> {
        self.holds.root()
    }

    /// Where the member is, or would be.
    pub fn path(&self) -> &Path {
        self.holds.path()
    }

    /// What the member's directory holds.
    pub const fn holds(&self) -> &MemberHolds {
        &self.holds
    }

    /// The project's identity, when this build read one.
    pub fn identity(&self) -> Option<(&str, &str)> {
        self.holds.id().zip(self.holds.name())
    }

    /// The member's project document, opened again.
    ///
    /// It is opened on demand rather than held: a workspace read is about the workspace, and a
    /// document that changed since is read as it is now rather than as it was remembered.
    pub fn project_file(&self) -> Option<ProjectFile> {
        let root = self.holds.root()?;
        if !self.holds.is_project() && !matches!(self.holds, MemberHolds::NothingYet { .. }) {
            return None;
        }
        ProjectFile::open(root.join(PROJECT_FILE_NAME)).ok()
    }

    /// One line: the name, the path the document writes, and what is there.
    pub fn render(&self) -> String {
        format!(
            "{:<16} {:<28} {}",
            self.name,
            self.written,
            self.holds.render()
        )
    }
}

/// Where a path written by one member lands, in workspace terms.
///
/// The member's own reader answers "does it stay inside my root" and, for a path that does not,
/// refuses it. That refusal is not a dead end inside a workspace: this is the second question, and it
/// has three answers rather than two.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum CrossRoot {
    /// Inside the member that wrote it.
    WithinMember,
    /// In another member: the path leaves its own member and stays in the workspace.
    AnotherMember {
        /// The member it lands in.
        member: String,
        /// Where it is.
        path: PathBuf,
    },
    /// It leaves the workspace, so no member can be reached by it.
    Outside {
        /// Why the path was refused, in the path layer's own words.
        reason: String,
    },
    /// It is inside the workspace and outside every member, so no member owns it.
    ///
    /// This is the case adding a member answers: the path is reachable, and nothing in the workspace
    /// says whose it is.
    NoMemberOwns {
        /// The path, as it resolved inside the workspace.
        path: PathBuf,
    },
    /// Where it lands cannot be said, because the member that wrote it has no directory.
    Unknown {
        /// Why not.
        reason: String,
    },
}

impl CrossRoot {
    /// Whether it stays in the member that wrote it.
    pub const fn is_within(&self) -> bool {
        matches!(self, Self::WithinMember)
    }

    /// The member it lands in, when it lands in one.
    pub fn member(&self) -> Option<&str> {
        match self {
            Self::AnotherMember { member, .. } => Some(member),
            _ => None,
        }
    }

    /// One line for a report.
    pub fn render(&self) -> String {
        match self {
            Self::WithinMember => "inside the member".to_owned(),
            Self::AnotherMember { member, path } => {
                format!("in member `{member}` ({})", path.display())
            }
            Self::Outside { reason } => format!("outside the workspace: {reason}"),
            Self::NoMemberOwns { path } => format!(
                "no member owns it: {} is inside the workspace and outside every member",
                path.display()
            ),
            Self::Unknown { reason } => format!("cannot be said: {reason}"),
        }
    }
}

/// A file a member's document names, identified by the member and the path inside it.
///
/// This is the whole point of the type: `pages/main.json` is not a file, `app:pages/main.json` is.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct WorkspaceFile {
    member: String,
    written: String,
    kind: String,
    verdict: String,
    cross: CrossRoot,
}

impl WorkspaceFile {
    /// The member whose document names it.
    pub fn member(&self) -> &str {
        &self.member
    }

    /// How that member writes the path.
    pub fn written(&self) -> &str {
        &self.written
    }

    /// What the member's reader says it is for.
    pub fn kind(&self) -> &str {
        &self.kind
    }

    /// What the member's reader found there.
    pub fn verdict(&self) -> &str {
        &self.verdict
    }

    /// Where it lands, which for a path inside its member is that member.
    pub const fn cross(&self) -> &CrossRoot {
        &self.cross
    }

    /// The name this file is addressed by: `member:path`.
    pub fn name(&self) -> String {
        format!("{}:{}", self.member, self.written)
    }

    /// Reads a `member:path` name.
    ///
    /// A path with no member is refused rather than guessed at: which of two members holds
    /// `pages/main.json` is exactly what the name is for.
    pub fn parse(name: &str) -> Result<Self, WorkspaceFileRefusal> {
        let Some((member, written)) = name.split_once(':') else {
            return Err(WorkspaceFileRefusal::NoMember {
                name: name.to_owned(),
            });
        };
        if !name_is_addressable(member) || written.trim().is_empty() {
            return Err(WorkspaceFileRefusal::Empty {
                name: name.to_owned(),
            });
        }
        Ok(Self {
            member: member.trim().to_owned(),
            written: written.trim().to_owned(),
            kind: String::new(),
            verdict: String::new(),
            cross: CrossRoot::WithinMember,
        })
    }

    /// One line: the name, what it is for, what was found, and where it lands.
    pub fn render(&self) -> String {
        let mut line = format!("{:<44} {:<21} {}", self.name(), self.kind, self.verdict);
        if !self.cross.is_within() {
            line.push_str(&format!(" — {}", self.cross.render()));
        }
        line
    }
}

/// Why a `member:path` name could not be read.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum WorkspaceFileRefusal {
    /// There is no member in the name.
    NoMember {
        /// What was given.
        name: String,
    },
    /// The member or the path says nothing.
    Empty {
        /// What was given.
        name: String,
    },
}

impl WorkspaceFileRefusal {
    /// A sentence for a person.
    pub fn message(&self) -> String {
        match self {
            Self::NoMember { name } => format!(
                "`{name}` names no member; a file in a workspace is written `member:path`, because \
                 two members can each hold a path of the same name"
            ),
            Self::Empty { name } => format!("`{name}` leaves the member or the path out"),
        }
    }
}

impl fmt::Display for WorkspaceFileRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.message())
    }
}

impl std::error::Error for WorkspaceFileRefusal {}

/// A workspace document that was read: its identity, its root, and its members.
#[derive(Clone, Debug)]
pub struct Workspace {
    path: PathBuf,
    original: String,
    root: ProjectRoot,
    id: String,
    name: String,
    members: Vec<Member>,
}

impl Workspace {
    /// Reads a workspace document.
    pub fn read(path: impl AsRef<Path>) -> Result<Self, ProjectError> {
        let path = path.as_ref().to_path_buf();
        let text = std::fs::read_to_string(&path).map_err(|error| ProjectError::Io {
            path: path.clone(),
            action: "reading the workspace document",
            reason: error.to_string(),
        })?;
        let directory = match path.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
            _ => PathBuf::from("."),
        };
        let root = ProjectRoot::new(&directory).map_err(|error| ProjectError::Io {
            path: path.clone(),
            action: "resolving the workspace root",
            reason: error.message(),
        })?;
        Self::interpret(path, root, text)
    }

    /// A workspace that does not exist yet, at a directory, with an identity.
    ///
    /// It has no members: a workspace is made by naming the roots that are already there, so there is
    /// nothing to guess at creation time.
    pub fn draft(directory: impl AsRef<Path>, name: &str, id: &str) -> Result<Self, ProjectError> {
        let directory = directory.as_ref();
        let path = directory.join(WORKSPACE_FILE_NAME);
        let root = ProjectRoot::new(directory).map_err(|error| ProjectError::Io {
            path: directory.to_path_buf(),
            action: "resolving the workspace root",
            reason: error.message(),
        })?;
        Ok(Self {
            path,
            original: String::new(),
            root,
            id: id.to_owned(),
            name: name.to_owned(),
            members: Vec::new(),
        })
    }

    /// Writes a workspace document that is not there yet.
    ///
    /// One file through a temporary file and a rename is the whole of it: a project needs a staging
    /// directory because it is a tree, and this is not one. What it shares with project creation is
    /// the rule that matters — a document that already exists is never overwritten, and what was
    /// written is read back before it is reported as a workspace.
    pub fn create(&self) -> Result<Self, ProjectError> {
        if self.path.exists() {
            return Err(ProjectError::Io {
                path: self.path.clone(),
                action: "creating the workspace document",
                reason: "it is already there, and this build does not overwrite a workspace"
                    .to_owned(),
            });
        }
        if let Some(parent) = self.path.parent()
            && !parent.as_os_str().is_empty()
            && !parent.is_dir()
        {
            return Err(ProjectError::Io {
                path: parent.to_path_buf(),
                action: "creating the workspace document",
                reason: "the directory it would go in is not there".to_owned(),
            });
        }
        json::write_atomically(&self.path, &self.to_json().render()).map_err(|error| {
            ProjectError::Io {
                path: self.path.clone(),
                action: "writing the workspace document",
                reason: error.to_string(),
            }
        })?;
        Self::read(&self.path)
    }

    fn interpret(path: PathBuf, root: ProjectRoot, text: String) -> Result<Self, ProjectError> {
        let value = json::parse(&text).map_err(|error| ProjectError::NotJson {
            path: path.clone(),
            error,
        })?;
        let version = format_version(&value)?;
        match version.get().cmp(&WORKSPACE_FORMAT.get()) {
            Ordering::Equal => {}
            Ordering::Greater => {
                return Err(ProjectError::NewerFormat {
                    found: version,
                    current: WORKSPACE_FORMAT,
                });
            }
            Ordering::Less => {
                return Err(ProjectError::OlderFormat {
                    found: version,
                    current: WORKSPACE_FORMAT,
                });
            }
        }
        let Some(object) = value.as_object() else {
            return Err(ProjectError::WrongType {
                path: "the workspace document".to_owned(),
                expected: "an object",
                found: value.kind(),
            });
        };
        for member in WORKSPACE_MEMBERS {
            if !object.contains(member) {
                return Err(ProjectError::MissingMember {
                    path: format!("`{member}` in the workspace document"),
                });
            }
        }
        let identity = object
            .get("workspace")
            .and_then(Json::as_object)
            .ok_or_else(|| ProjectError::WrongType {
                path: "`workspace`".to_owned(),
                expected: "an object with the workspace's identity",
                found: object
                    .get("workspace")
                    .map(Json::kind)
                    .unwrap_or(JsonKind::Null),
            })?;
        for member in IDENTITY_MEMBERS {
            if identity.get(member).and_then(Json::as_str).is_none() {
                return Err(ProjectError::MissingMember {
                    path: format!("`workspace.{member}`"),
                });
            }
        }
        let id = text_member(identity, "id", "workspace.id")?;
        let name = text_member(identity, "name", "workspace.name")?;
        let Some(entries) = object.get("members").and_then(Json::as_array) else {
            return Err(ProjectError::WrongType {
                path: "`members`".to_owned(),
                expected: "an array",
                found: object
                    .get("members")
                    .map(Json::kind)
                    .unwrap_or(JsonKind::Null),
            });
        };

        let mut members: Vec<Member> = Vec::new();
        for (index, entry) in entries.iter().enumerate() {
            let at = format!("members[{index}]");
            let Some(entry) = entry.as_object() else {
                return Err(ProjectError::WrongType {
                    path: format!("`{at}`"),
                    expected: "an object",
                    found: entry.kind(),
                });
            };
            for member in MEMBER_MEMBERS {
                if entry.get(member).and_then(Json::as_str).is_none() {
                    return Err(ProjectError::MissingMember {
                        path: format!("`{at}.{member}`"),
                    });
                }
            }
            let member_name = text_member(entry, "name", &format!("{at}.name"))?;
            let written = text_member(entry, "root", &format!("{at}.root"))?;
            if member_name.trim().is_empty() {
                return Err(ProjectError::EmptyValue {
                    path: format!("`{at}.name`"),
                });
            }
            if members.iter().any(|there| there.name == member_name) {
                return Err(ProjectError::Duplicate {
                    path: format!("`{at}.name`"),
                    claim: member_name,
                });
            }
            let holds = holds_of(&root, &member_name, &written)?;
            members.push(Member {
                name: member_name,
                written,
                holds,
            });
        }

        // Nesting is refused here rather than resolved at every lookup: with one member inside
        // another, "which member holds this file" would have two answers and neither would be wrong.
        for (index, outer) in members.iter().enumerate() {
            for inner in &members[index + 1..] {
                let (a, b) = (outer.path(), inner.path());
                if a.starts_with(b) || b.starts_with(a) {
                    return Err(ProjectError::Inconsistent {
                        path: format!("`members[{index}].root`"),
                        reason: format!(
                            "`{}` (in `{}`) and `{}` (in `{}`) overlap, and a file would then \
                             belong to two members; a member may not contain another member",
                            outer.name, outer.written, inner.name, inner.written
                        ),
                    });
                }
            }
        }

        Ok(Self {
            path,
            original: text,
            root,
            id,
            name,
            members,
        })
    }

    /// Where the workspace document is.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The directory every member root is relative to.
    pub fn root(&self) -> &ProjectRoot {
        &self.root
    }

    /// The workspace's identity.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The workspace's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The members, in document order.
    pub fn members(&self) -> &[Member] {
        &self.members
    }

    /// The member with this name.
    pub fn member(&self, name: &str) -> Option<&Member> {
        self.members.iter().find(|member| member.name == name)
    }

    /// Where a path written by a member lands.
    pub fn resolve(&self, from: &Member, written: &str) -> CrossRoot {
        // First the member's own answer: a path that stays inside it is its own business, and the
        // workspace has nothing to add.
        if let Some(root) = from.root() {
            match ProjectRoot::new(root) {
                Ok(root) => {
                    // The rule about what stays inside a root is the path layer's, and it is the same
                    // rule for a file that is there and for one that is not: a project may reference
                    // a file it is about to create, and reading that as "it went somewhere else"
                    // would be wrong.
                    if root.resolve_for_creation(written).is_ok() {
                        return CrossRoot::WithinMember;
                    }
                }
                Err(error) => {
                    return CrossRoot::Unknown {
                        reason: format!(
                            "member `{}` cannot be resolved: {}",
                            from.name,
                            error.message()
                        ),
                    };
                }
            }
        } else {
            return CrossRoot::Unknown {
                reason: format!("member `{}` has no directory", from.name),
            };
        }
        // It leaves the member, so the question is where it lands. The path is resolved against the
        // workspace root with the member's own prefix in front of it, which is what makes the path
        // leave one member and enter another rather than leave the workspace.
        match self
            .root
            .resolve_for_creation(format!("{}/{}", from.written, written))
        {
            Ok(path) => match self.holder_of(&path) {
                Some(member) if member.name != from.name => CrossRoot::AnotherMember {
                    member: member.name.clone(),
                    path,
                },
                Some(_) => CrossRoot::WithinMember,
                None => CrossRoot::NoMemberOwns { path },
            },
            Err(error) => CrossRoot::Outside {
                reason: error.message(),
            },
        }
    }

    /// The member a path belongs to, when one holds it.
    fn holder_of(&self, path: &Path) -> Option<&Member> {
        self.members
            .iter()
            .find(|member| path.starts_with(member.path()))
    }

    /// Every file the members' documents name, qualified by the member that names it.
    ///
    /// A file is one row per member, not one per member that mentions it: a page document is named by
    /// `pages` and by `startup`, and it is one file. The same path in two members is two rows, which
    /// is the point of the type.
    pub fn files(&self) -> Vec<WorkspaceFile> {
        let mut files: Vec<WorkspaceFile> = Vec::new();
        for member in &self.members {
            let Some(file) = member.project_file() else {
                continue;
            };
            let Some(document) = file.document() else {
                continue;
            };
            for reference in document.references() {
                let written = reference.written();
                if files
                    .iter()
                    .any(|there| there.member == member.name && there.written == written)
                {
                    continue;
                }
                files.push(WorkspaceFile {
                    member: member.name.clone(),
                    written: written.to_owned(),
                    kind: reference.kind().name().to_owned(),
                    verdict: reference.verdict().render(),
                    cross: self.resolve(member, written),
                });
            }
        }
        files
    }

    /// One file, by the member and path it is identified by.
    ///
    /// A file nothing names is reported as such rather than as an empty record: the members' own
    /// readers are the authority on what they name, and this does not add entries of its own.
    pub fn file(&self, name: &str) -> Result<WorkspaceFile, WorkspaceLookup> {
        let wanted = WorkspaceFile::parse(name).map_err(WorkspaceLookup::UnreadableName)?;
        let Some(member) = self.member(&wanted.member) else {
            return Err(WorkspaceLookup::NoSuchMember {
                member: wanted.member,
                members: self
                    .members
                    .iter()
                    .map(|member| member.name.clone())
                    .collect(),
            });
        };
        Ok(self
            .files()
            .into_iter()
            .find(|file| file.member == wanted.member && file.written == wanted.written)
            .unwrap_or(WorkspaceFile {
                cross: self.resolve(member, &wanted.written),
                ..wanted
            }))
    }

    /// The members' documents' references that leave their own member, with where each lands.
    pub fn cross_root_references(&self) -> Vec<(String, String, CrossRoot)> {
        self.files()
            .into_iter()
            .filter(|file| !file.cross.is_within())
            .map(|file| (file.member, file.written, file.cross))
            .collect()
    }

    /// One line per member, then the files that leave their member, then where each lands.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "workspace `{}` ({}), {} member(s), root {}",
            self.name,
            self.id,
            self.members.len(),
            self.root.as_path().display()
        )];
        if self.members.is_empty() {
            lines.push(
                "  no members yet: a workspace is made by naming the roots that are there"
                    .to_owned(),
            );
        }
        for member in &self.members {
            lines.push(format!("  {}", member.render()));
        }
        let files = self.files();
        lines.push(format!("files: {}", files.len()));
        for file in &files {
            lines.push(format!("  {}", file.render()));
        }
        let crossing: Vec<&WorkspaceFile> = files
            .iter()
            .filter(|file| !file.cross.is_within())
            .collect();
        if crossing.is_empty() {
            lines.push("no member's document reaches another member".to_owned());
        } else {
            lines.push(format!(
                "{} reference(s) leave their own member:",
                crossing.len()
            ));
            for file in crossing {
                lines.push(format!(
                    "  {}:{} — {}",
                    file.member,
                    file.written,
                    file.cross.render()
                ));
            }
        }
        lines
    }

    /// The document as JSON.
    pub fn to_json(&self) -> Json {
        let mut identity = JsonObject::new();
        identity.insert("id", Json::string(&self.id));
        identity.insert("name", Json::string(&self.name));
        let mut out = JsonObject::new();
        out.insert("format", Json::number(u64::from(WORKSPACE_FORMAT.get())));
        out.insert("workspace", Json::Object(identity));
        out.insert(
            "members",
            Json::array(
                self.members
                    .iter()
                    .map(|member| {
                        let mut entry = JsonObject::new();
                        entry.insert("name", Json::string(&member.name));
                        entry.insert("root", Json::string(&member.written));
                        Json::Object(entry)
                    })
                    .collect::<Vec<_>>(),
            ),
        );
        Json::Object(out)
    }

    /// The workspace with a different member list, written to the same path.
    fn with_members(&self, members: Vec<Member>) -> Self {
        Self {
            members,
            ..self.clone()
        }
    }

    /// Writes the document, refusing when the file changed since it was read.
    ///
    /// This is the only thing an add or a remove writes. No directory is created and none is
    /// removed: a member is a line in this document, and the roots are the person's own.
    fn write(&self) -> Result<PathBuf, ProjectError> {
        let current = std::fs::read_to_string(&self.path).map_err(|error| ProjectError::Io {
            path: self.path.clone(),
            action: "reading the workspace document before writing it",
            reason: error.to_string(),
        })?;
        if current != self.original {
            return Err(ProjectError::ChangedOnDisk {
                path: self.path.clone(),
            });
        }
        json::write_atomically(&self.path, &self.to_json().render()).map_err(|error| {
            ProjectError::Io {
                path: self.path.clone(),
                action: "writing the workspace document",
                reason: error.to_string(),
            }
        })
    }

    /// A plan for one change, which reads and decides nothing.
    pub fn plan(&self, change: WorkspaceChange) -> WorkspacePlan {
        WorkspacePlan::of(self, change)
    }

    /// Records which member is active, for this workspace, on this machine.
    pub fn activate(
        &self,
        state: &mut WorkspaceState,
        name: &str,
        at: SystemTime,
    ) -> Result<ActiveMemberChoice, WorkspaceRefusal> {
        if self.member(name).is_none() {
            return Err(WorkspaceRefusal::NoSuchMember {
                member: name.to_owned(),
                members: self
                    .members
                    .iter()
                    .map(|member| member.name.clone())
                    .collect(),
            });
        }
        let choice = ActiveMemberChoice::new(&self.id, &self.name, name, at);
        state.record(choice.clone());
        Ok(choice)
    }
}

fn holds_of(root: &ProjectRoot, name: &str, written: &str) -> Result<MemberHolds, ProjectError> {
    let resolved = match root.resolve(written) {
        Ok(resolved) => resolved,
        Err(error @ PathError::MissingParent { .. }) => {
            let path = match &error {
                PathError::MissingParent { path } => path.clone(),
                _ => unreachable!("the arm above matched"),
            };
            return Ok(MemberHolds::NoDirectory {
                path,
                reason: error.message(),
            });
        }
        Err(error) => {
            return Err(ProjectError::Inconsistent {
                path: format!("the root of member `{name}`"),
                reason: error.message(),
            });
        }
    };
    match std::fs::metadata(&resolved) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => {
            return Ok(MemberHolds::NoDirectory {
                path: resolved,
                reason: "it is there, but it is not a directory".to_owned(),
            });
        }
        // A directory that is not there and one that could not be looked at are different findings:
        // "somebody moved it" and "this machine will not let me look" call for different actions.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let reason = format!("there is nothing at {}", resolved.display());
            return Ok(MemberHolds::NoDirectory {
                path: resolved,
                reason,
            });
        }
        Err(error) => {
            return Ok(MemberHolds::NoDirectory {
                path: resolved,
                reason: format!("it could not be looked at: {error}"),
            });
        }
    }
    let document = resolved.join(PROJECT_FILE_NAME);
    if !document.is_file() {
        return Ok(MemberHolds::NothingYet { root: resolved });
    }
    match ProjectFile::open(&document) {
        Ok(file) => match file.document() {
            Some(project) => Ok(MemberHolds::Project {
                root: resolved,
                id: project.identity().id().to_owned(),
                name: project.identity().name().to_owned(),
                declared: file.migrated_from().unwrap_or_else(|| file.version()),
            }),
            None => Ok(MemberHolds::Foreign {
                root: resolved,
                version: file.version(),
            }),
        },
        Err(error) => Ok(MemberHolds::Unreadable {
            root: resolved,
            reason: error.message(),
        }),
    }
}

fn text_member(object: &JsonObject, member: &str, where_: &str) -> Result<String, ProjectError> {
    object
        .get(member)
        .and_then(Json::as_str)
        .map(str::to_owned)
        .ok_or_else(|| ProjectError::MissingMember {
            path: format!("`{where_}`"),
        })
}

/// What a workspace change should do.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum WorkspaceChange {
    /// Name a root that is already there as a member.
    Add {
        /// The name to address it by.
        name: String,
        /// The root, as a path relative to the workspace root.
        root: String,
    },
    /// Take a member out of the document, and touch nothing else.
    Remove {
        /// The member's name.
        name: String,
    },
}

impl fmt::Display for WorkspaceChange {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Add { name, root } => write!(formatter, "add member `{name}` at `{root}`"),
            Self::Remove { name } => write!(formatter, "remove member `{name}`"),
        }
    }
}

/// One reference another member's document makes into the member being removed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Referrer {
    from: String,
    written: String,
    kind: String,
}

impl Referrer {
    /// The member whose document makes the reference.
    pub fn from(&self) -> &str {
        &self.from
    }

    /// How that member writes the path.
    pub fn written(&self) -> &str {
        &self.written
    }

    /// What the member's reader says it is for.
    pub fn kind(&self) -> &str {
        &self.kind
    }

    /// One line.
    pub fn render(&self) -> String {
        format!("{}:{} ({})", self.from, self.written, self.kind)
    }
}

/// What a workspace change would do, and what stops it.
#[derive(Clone, Debug)]
pub struct WorkspacePlan {
    change: WorkspaceChange,
    blockers: Vec<String>,
    referrers: Vec<Referrer>,
    resulting: Option<Workspace>,
}

impl WorkspacePlan {
    /// Plans one change. Reading, and deciding nothing.
    pub fn of(workspace: &Workspace, change: WorkspaceChange) -> Self {
        let mut blockers = Vec::new();
        let mut referrers = Vec::new();
        let mut members = workspace.members().to_vec();
        match &change {
            WorkspaceChange::Add { name, root } => {
                if !name_is_addressable(name) {
                    blockers.push(format!(
                        "`{name}` cannot be a member's name: a name is addressed as `member:path` \
                         and typed as an argument, so it holds no separator, no space and no slash"
                    ));
                }
                if workspace.member(name).is_some() {
                    blockers.push(format!(
                        "a member called `{name}` is already there; names address members, so two of \
                         them cannot share one"
                    ));
                }
                // Every reason not to add it is listed, not just the first: a plan that stops at
                // the first problem makes a person fix one thing at a time to find the next.
                match workspace.root().resolve(root) {
                    Ok(path) => {
                        // Naming a root is not creating one: a member that points at nothing would
                        // be a member with no answer to any question about it, and creating
                        // directories belongs to the creation path.
                        if !path.is_dir() {
                            blockers.push(format!(
                                "there is nothing to add at `{root}`: a member names a root that is \
                                 there, and making one belongs to the creation path"
                            ));
                        }
                        if let Some(holder) = members.iter().find(|member| {
                            let there = member.path();
                            path.starts_with(there) || there.starts_with(&path)
                        }) {
                            blockers.push(format!(
                                "`{root}` overlaps `{}` (in `{}`); members may not contain one \
                                 another, because a file would then belong to two of them",
                                holder.name(),
                                holder.written()
                            ));
                        }
                        if blockers.is_empty() {
                            match holds_of(workspace.root(), name, root) {
                                Ok(holds) => members.push(Member {
                                    name: name.clone(),
                                    written: root.clone(),
                                    holds,
                                }),
                                Err(error) => blockers.push(error.message()),
                            }
                        }
                    }
                    Err(error) => blockers.push(format!("`{root}`: {}", error.message())),
                }
            }
            WorkspaceChange::Remove { name } => {
                if workspace.member(name).is_none() {
                    blockers.push(format!(
                        "there is no member called `{name}`; the members are {}",
                        names_of(workspace.members())
                    ));
                } else {
                    // Anything that points into the member being removed is listed before the
                    // removal is allowed: taking a member out while another member's document still
                    // names a file in it would leave a reference that nothing can answer.
                    for file in workspace.files() {
                        if file.cross().member() == Some(name.as_str()) {
                            referrers.push(Referrer {
                                from: file.member().to_owned(),
                                written: file.written().to_owned(),
                                kind: file.kind().to_owned(),
                            });
                        }
                    }
                    if !referrers.is_empty() {
                        blockers.push(format!(
                            "{} reference(s) in other members' documents name a file in it",
                            referrers.len()
                        ));
                    }
                    members.retain(|member| &member.name != name);
                }
            }
        }
        let resulting = blockers.is_empty().then(|| workspace.with_members(members));
        Self {
            change,
            blockers,
            referrers,
            resulting,
        }
    }

    /// What the plan would do.
    pub const fn change(&self) -> &WorkspaceChange {
        &self.change
    }

    /// Why it cannot be done, one line each.
    pub fn blockers(&self) -> &[String] {
        &self.blockers
    }

    /// The references that stop a removal, when that is what stopped it.
    pub fn referrers(&self) -> &[Referrer] {
        &self.referrers
    }

    /// Whether the change can be carried out.
    pub fn is_executable(&self) -> bool {
        self.blockers.is_empty() && self.resulting.is_some()
    }

    /// The members the workspace would have afterwards.
    pub fn members_after(&self) -> Vec<&str> {
        match &self.resulting {
            Some(workspace) => workspace.members().iter().map(Member::name).collect(),
            None => Vec::new(),
        }
    }

    /// The plan, one line per thing it would do and one per thing that stops it.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!("would {}", self.change)];
        match &self.resulting {
            Some(workspace) => {
                lines.push(format!(
                    "  members after: {}",
                    names_of(workspace.members())
                ));
                lines.push(
                    "  nothing else is written: no directory is created and none is removed"
                        .to_owned(),
                );
            }
            None => lines.push("  nothing would change".to_owned()),
        }
        for referrer in &self.referrers {
            lines.push(format!("  it is named by {}", referrer.render()));
        }
        for blocker in &self.blockers {
            lines.push(format!("  blocked: {blocker}"));
        }
        lines
    }

    /// Carries the change out: writes the workspace document, and nothing else.
    ///
    /// There is nothing else to write and nothing to put back: the document is one file written
    /// through a temporary file and a rename, and the write refuses while the file on disk is not the
    /// one this plan was made from. A blocked plan and a failed write both leave the workspace
    /// exactly as it was, which is why neither is reported as a partial success.
    pub fn execute(&self) -> Result<WorkspaceWritten, WorkspaceFailure> {
        let Some(resulting) = &self.resulting else {
            return Err(WorkspaceFailure::Blocked {
                blockers: self.blockers.clone(),
            });
        };
        let path = resulting.write().map_err(WorkspaceFailure::Write)?;
        // What was written is read back: a plan that says it did something has to be able to show
        // the document it produced, and the reader is the only thing that can say whether it can.
        let after = Workspace::read(&path).map_err(WorkspaceFailure::Write)?;
        Ok(WorkspaceWritten {
            change: self.change.clone(),
            path,
            members: after
                .members()
                .iter()
                .map(|member| member.name().to_owned())
                .collect(),
            written: after,
        })
    }
}

fn names_of(members: &[Member]) -> String {
    match members {
        [] => "none".to_owned(),
        [one] => format!("`{}`", one.name),
        [rest @ .., last] => format!(
            "{} and `{}`",
            rest.iter()
                .map(|member| format!("`{}`", member.name))
                .collect::<Vec<_>>()
                .join(", "),
            last.name
        ),
    }
}

/// What a change did.
#[derive(Clone, Debug)]
pub struct WorkspaceWritten {
    change: WorkspaceChange,
    path: PathBuf,
    members: Vec<String>,
    written: Workspace,
}

impl WorkspaceWritten {
    /// What changed.
    pub const fn change(&self) -> &WorkspaceChange {
        &self.change
    }

    /// Where the document is.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The members the workspace has now.
    pub fn members(&self) -> &[String] {
        &self.members
    }

    /// The workspace as it was read back.
    pub const fn workspace(&self) -> &Workspace {
        &self.written
    }

    /// One line per thing that happened.
    pub fn render(&self) -> Vec<String> {
        vec![
            format!("{}: wrote {}", self.change, self.path.display()),
            format!("  members: {}", self.members.join(", ")),
        ]
    }
}

/// Why a change did not happen.
#[derive(Clone, Debug)]
pub enum WorkspaceFailure {
    /// The plan had blockers, so nothing was written.
    Blocked {
        /// What stopped it.
        blockers: Vec<String>,
    },
    /// The document could not be written, so nothing changed.
    Write(ProjectError),
}

impl WorkspaceFailure {
    /// A sentence for a person, which always says what the workspace looks like now.
    pub fn message(&self) -> String {
        match self {
            Self::Blocked { blockers } => format!(
                "the change was not made: {}; the workspace is as it was",
                blockers.join("; ")
            ),
            Self::Write(error) => format!("{}; the workspace is as it was", error.message()),
        }
    }

    /// What stopped it, one line each.
    pub fn blockers(&self) -> Vec<&str> {
        match self {
            Self::Blocked { blockers } => blockers.iter().map(String::as_str).collect(),
            Self::Write(_) => Vec::new(),
        }
    }
}

impl fmt::Display for WorkspaceFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.message())
    }
}

impl std::error::Error for WorkspaceFailure {}

/// Whether a word can be the name of a member.
///
/// A name has to survive being addressed as `member:path` and being typed as a command-line
/// argument, so the separators those use are not part of it, and neither is leading or trailing
/// space: `  app` and `app` would look like two members while reading like one.
pub fn name_is_addressable(name: &str) -> bool {
    !name.is_empty()
        && name == name.trim()
        && !name.contains([':', '=', ' ', '\t', '\n'])
        && !name.contains(['/', '\\'])
}

/// A lookup that found nothing.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum WorkspaceLookup {
    /// There is no member by that name.
    NoSuchMember {
        /// The name that was asked for.
        member: String,
        /// The members there are.
        members: Vec<String>,
    },
    /// The name could not be read.
    UnreadableName(WorkspaceFileRefusal),
}

impl WorkspaceLookup {
    /// A sentence for a person.
    pub fn message(&self) -> String {
        match self {
            Self::NoSuchMember { member, members } => format!(
                "there is no member called `{member}`; the members are {}",
                match members.len() {
                    0 => "none".to_owned(),
                    _ => members
                        .iter()
                        .map(|name| format!("`{name}`"))
                        .collect::<Vec<_>>()
                        .join(", "),
                }
            ),
            Self::UnreadableName(refusal) => refusal.message(),
        }
    }
}

impl fmt::Display for WorkspaceLookup {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.message())
    }
}

impl std::error::Error for WorkspaceLookup {}

/// Something that could not be done.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum WorkspaceRefusal {
    /// There is no member by that name.
    NoSuchMember {
        /// The name that was asked for.
        member: String,
        /// The members there are.
        members: Vec<String>,
    },
}

impl WorkspaceRefusal {
    /// A sentence for a person.
    pub fn message(&self) -> String {
        match self {
            Self::NoSuchMember { member, members } => format!(
                "there is no member called `{member}`, so nothing was recorded; the members are {}",
                names_of(
                    &members
                        .iter()
                        .map(|name| Member {
                            name: name.clone(),
                            written: String::new(),
                            holds: MemberHolds::NothingYet {
                                root: PathBuf::new(),
                            },
                        })
                        .collect::<Vec<_>>()
                )
            ),
        }
    }
}

impl fmt::Display for WorkspaceRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.message())
    }
}

impl std::error::Error for WorkspaceRefusal {}

/// Which member is active for a workspace, on one machine.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ActiveMemberChoice {
    workspace: String,
    name: String,
    member: String,
    chosen_at_millis: i64,
}

impl ActiveMemberChoice {
    fn new(workspace: &str, name: &str, member: &str, at: SystemTime) -> Self {
        Self {
            workspace: workspace.to_owned(),
            name: name.to_owned(),
            member: member.to_owned(),
            chosen_at_millis: at
                .duration_since(UNIX_EPOCH)
                .map(|since| i64::try_from(since.as_millis()).unwrap_or(i64::MAX))
                .unwrap_or(0),
        }
    }

    /// The workspace this belongs to.
    pub fn workspace(&self) -> &str {
        &self.workspace
    }

    /// The workspace's name, which is what a person recognises.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The member that was made active.
    pub fn member(&self) -> &str {
        &self.member
    }

    /// When it was made active.
    pub const fn chosen_at_millis(&self) -> i64 {
        self.chosen_at_millis
    }
}

/// Which member is active, and whether that is usable here.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ActiveMember<'a> {
    /// A member of this workspace.
    Named(&'a Member, &'a ActiveMemberChoice),
    /// A choice was recorded for this workspace and names a member that is not there any more.
    MemberIsGone {
        /// The name that is not there.
        name: String,
        /// What was recorded.
        recorded: &'a ActiveMemberChoice,
    },
    /// The only choice this machine has is for another workspace.
    AnotherWorkspace(&'a ActiveMemberChoice),
    /// Nothing is recorded for this workspace on this machine.
    NeverChosen,
}

impl<'a> ActiveMember<'a> {
    /// The member that is active, when there is one.
    pub fn member(&self) -> Option<&'a Member> {
        match self {
            Self::Named(member, _) => Some(member),
            _ => None,
        }
    }

    /// The name that is active or was recorded, when there is one.
    pub fn name(&self) -> Option<&str> {
        match self {
            Self::Named(member, _) => Some(member.name()),
            Self::MemberIsGone { name, .. } => Some(name),
            Self::AnotherWorkspace(choice) => Some(choice.member()),
            Self::NeverChosen => None,
        }
    }

    /// Whether the choice is for this workspace.
    pub const fn is_for_this_workspace(&self) -> bool {
        matches!(self, Self::Named(..) | Self::MemberIsGone { .. })
    }

    /// One line for a report.
    pub fn render(&self) -> String {
        match self {
            Self::Named(member, _) => format!("`{}` (chosen for this workspace)", member.name()),
            Self::MemberIsGone { name, .. } => format!(
                "none: `{name}` was chosen and is not a member any more, and a choice is not moved \
                 to another member by itself"
            ),
            Self::AnotherWorkspace(choice) => format!(
                "none for this workspace: `{}` is chosen for {} ({}), and a choice belongs to one \
                 workspace",
                choice.member(),
                choice.name(),
                choice.workspace()
            ),
            Self::NeverChosen => {
                "none has been chosen for this workspace on this machine".to_owned()
            }
        }
    }
}

/// This machine's workspace state: which member is active, per workspace.
///
/// It is machine-local for the same reason the active target is: it is what somebody is working on
/// now, and a shared document that said so would change under everyone's feet. It is written the way
/// the other machine-local files are — a temporary file and a rename — and it needs no permission,
/// because it is the host's own state and not the project's.
#[derive(Clone, Debug)]
pub struct WorkspaceState {
    path: PathBuf,
    active: Vec<ActiveMemberChoice>,
    capacity: usize,
}

/// The format of the machine-local workspace state.
pub const STATE_FORMAT: FormatVersion = FormatVersion::from_raw(1);

/// The members the machine-local workspace state has.
pub const STATE_MEMBERS: [&str; 2] = ["format", "active"];

/// The members one recorded choice has.
pub const CHOICE_MEMBERS: [&str; 4] = ["workspace", "name", "member", "chosen_at_unix_ms"];

impl WorkspaceState {
    /// How many workspaces' choices the file keeps. Older ones fall off the end.
    pub const CAPACITY: usize = 10;

    /// An empty state that would be saved at a path.
    pub fn empty(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            active: Vec::new(),
            capacity: Self::CAPACITY,
        }
    }

    /// A state with a different bound, which is what a test uses.
    pub fn with_capacity(mut self, capacity: usize) -> Self {
        self.capacity = capacity.max(1);
        self.active.truncate(self.capacity);
        self
    }

    /// Where the state is.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Reads the state, or an empty one when the file is not there yet.
    ///
    /// A file that is there and cannot be read is an error rather than an empty state: forgetting
    /// which member somebody was working in because one file went wrong is a loss nobody sees.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ProjectError> {
        let path = path.as_ref().to_path_buf();
        let mut state = Self::empty(path.clone());
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(state),
            Err(error) => {
                return Err(ProjectError::Io {
                    path: path.clone(),
                    action: "reading this machine's workspace state",
                    reason: error.to_string(),
                });
            }
        };
        let value = json::parse(&text).map_err(|error| ProjectError::NotJson {
            path: path.clone(),
            error,
        })?;
        let version = format_version(&value)?;
        match version.get().cmp(&STATE_FORMAT.get()) {
            Ordering::Equal => {}
            Ordering::Greater => {
                return Err(ProjectError::NewerFormat {
                    found: version,
                    current: STATE_FORMAT,
                });
            }
            Ordering::Less => {
                return Err(ProjectError::OlderFormat {
                    found: version,
                    current: STATE_FORMAT,
                });
            }
        }
        let Some(object) = value.as_object() else {
            return Err(ProjectError::WrongType {
                path: "the workspace state".to_owned(),
                expected: "an object",
                found: value.kind(),
            });
        };
        let Some(entries) = object.get("active").and_then(Json::as_array) else {
            return Err(ProjectError::MissingMember {
                path: "`active` in the workspace state".to_owned(),
            });
        };
        for (index, entry) in entries.iter().enumerate() {
            let at = format!("active[{index}]");
            let Some(entry) = entry.as_object() else {
                return Err(ProjectError::WrongType {
                    path: format!("`{at}`"),
                    expected: "an object",
                    found: entry.kind(),
                });
            };
            let workspace = text_member(entry, "workspace", &format!("{at}.workspace"))?;
            let name = text_member(entry, "name", &format!("{at}.name"))?;
            let member = text_member(entry, "member", &format!("{at}.member"))?;
            let chosen_at_millis = entry
                .get("chosen_at_unix_ms")
                .and_then(Json::as_i64)
                .ok_or_else(|| ProjectError::MissingMember {
                    path: format!("`{at}.chosen_at_unix_ms`"),
                })?;
            state.active.push(ActiveMemberChoice {
                workspace,
                name,
                member,
                chosen_at_millis,
            });
        }
        state.active.truncate(state.capacity);
        Ok(state)
    }

    /// Writes the state, through a temporary file and a rename.
    pub fn save(&self) -> Result<PathBuf, ProjectError> {
        if let Some(parent) = self.path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|error| ProjectError::Io {
                path: parent.to_path_buf(),
                action: "creating the directory this machine's workspace state lives in",
                reason: error.to_string(),
            })?;
        }
        json::write_atomically(&self.path, &self.to_json().render()).map_err(|error| {
            ProjectError::Io {
                path: self.path.clone(),
                action: "writing this machine's workspace state",
                reason: error.to_string(),
            }
        })
    }

    /// Records a choice, newest first, and forgets the oldest beyond the bound.
    pub fn record(&mut self, choice: ActiveMemberChoice) {
        self.active
            .retain(|there| there.workspace != choice.workspace);
        self.active.insert(0, choice);
        self.active.truncate(self.capacity);
    }

    /// What is recorded for a workspace.
    pub fn active_for<'a>(&'a self, workspace: &'a Workspace) -> ActiveMember<'a> {
        match self
            .active
            .iter()
            .find(|choice| choice.workspace == workspace.id())
        {
            Some(choice) => match workspace.member(&choice.member) {
                Some(member) => ActiveMember::Named(member, choice),
                None => ActiveMember::MemberIsGone {
                    name: choice.member.clone(),
                    recorded: choice,
                },
            },
            None => match self.active.first() {
                Some(choice) => ActiveMember::AnotherWorkspace(choice),
                None => ActiveMember::NeverChosen,
            },
        }
    }

    /// The choices, which is what a report shows.
    pub fn choices(&self) -> &[ActiveMemberChoice] {
        &self.active
    }

    /// The state as JSON.
    pub fn to_json(&self) -> Json {
        let mut out = JsonObject::new();
        out.insert("format", Json::number(u64::from(STATE_FORMAT.get())));
        out.insert(
            "active",
            Json::array(
                self.active
                    .iter()
                    .map(|choice| {
                        let mut entry = JsonObject::new();
                        entry.insert("workspace", Json::string(&choice.workspace));
                        entry.insert("name", Json::string(&choice.name));
                        entry.insert("member", Json::string(&choice.member));
                        entry.insert(
                            "chosen_at_unix_ms",
                            Json::number(choice.chosen_at_millis.unsigned_abs()),
                        );
                        Json::Object(entry)
                    })
                    .collect::<Vec<_>>(),
            ),
        );
        Json::Object(out)
    }

    /// One line per recorded choice.
    pub fn render(&self) -> Vec<String> {
        if self.active.is_empty() {
            return vec!["nothing is recorded for any workspace on this machine".to_owned()];
        }
        self.active
            .iter()
            .map(|choice| {
                format!(
                    "member `{}` for {} ({})",
                    choice.member, choice.name, choice.workspace
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use super::{
        ActiveMember, CrossRoot, MemberHolds, WORKSPACE_FILE_NAME, Workspace, WorkspaceChange,
        WorkspaceFailure, WorkspaceFile, WorkspaceFileRefusal, WorkspaceLookup, WorkspaceState,
    };
    use crate::host::paths::ProjectRoot;
    use crate::host::permission::PermissionService;
    use crate::project::build::configuration::{
        ConfigurationRefusal, MachineConfiguration, ResolvedConfiguration,
    };

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
        let root =
            std::env::temp_dir().join(format!("rustage-workspace-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&root).expect("a temporary directory is creatable");
        Sandbox {
            root: root.clone(),
            _guard: TempDirectory(root),
        }
    }

    impl Sandbox {
        fn write(&self, relative: &str, text: &str) -> PathBuf {
            let path = self.root.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("a directory");
            }
            std::fs::write(&path, text).expect("a file");
            path
        }

        fn directory(&self, relative: &str) -> PathBuf {
            let path = self.root.join(relative);
            std::fs::create_dir_all(&path).expect("a directory");
            path
        }

        fn workspace(&self, members: &[(&str, &str)]) -> Workspace {
            let text = workspace_json(members);
            let path = self.write(WORKSPACE_FILE_NAME, &text);
            Workspace::read(&path).expect("the workspace reads")
        }
    }

    /// A project document with an identity, one page and one target.
    fn project(id: &str, name: &str, resources: &str) -> String {
        format!(
            "{{\"format\": 2, \"project\": {{\"id\": \"{id}\", \"name\": \"{name}\"}}, \
             \"pages\": [{{\"document\": \"pages/main.json\"}}], \
             \"resources\": [{resources}], \
             \"targets\": [{{\"name\": \"desktop\", \"profile\": \"debug\"}}], \
             \"generated\": [], \
             \"startup\": \"pages/main.json\", \"network\": {{\"services\": []}}}}"
        )
    }

    fn workspace_json(members: &[(&str, &str)]) -> String {
        workspace_json_for("ws-0001", "Demo workspace", members)
    }

    /// A workspace document with its own identity, so a test can hold two of them.
    fn workspace_json_for(id: &str, name: &str, members: &[(&str, &str)]) -> String {
        let list = members
            .iter()
            .map(|(name, root)| format!("{{\"name\": \"{name}\", \"root\": \"{root}\"}}"))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "{{\"format\": 1, \"workspace\": {{\"id\": \"{id}\", \"name\": \"{name}\"}}, \
             \"members\": [{list}]}}"
        )
    }

    fn at(seconds: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(seconds)
    }

    #[test]
    fn a_workspace_reads_its_members_and_what_each_one_holds() {
        let sandbox = sandbox();
        sandbox.write("app/project.json", &project("app-id", "App", ""));
        sandbox.directory("empty");
        sandbox.write(
            "newer/project.json",
            &project("newer-id", "Newer", "").replace("\"format\": 2", "\"format\": 3"),
        );
        let workspace = sandbox.workspace(&[
            ("app", "app"),
            ("empty", "empty"),
            ("newer", "newer"),
            ("gone", "gone"),
        ]);

        assert_eq!(workspace.id(), "ws-0001");
        assert_eq!(workspace.name(), "Demo workspace");
        assert_eq!(workspace.members().len(), 4);
        assert_eq!(
            workspace.member("app").expect("a member").identity(),
            Some(("app-id", "App"))
        );
        assert!(matches!(
            workspace.member("empty").expect("a member").holds(),
            MemberHolds::NothingYet { .. }
        ));
        // A project from a newer build is reported as what it is, not as an unreadable file and not
        // as an empty member: the workspace can hold such a member and say so.
        assert!(matches!(
            workspace.member("newer").expect("a member").holds(),
            MemberHolds::Foreign { version, .. } if version.get() == 3
        ));
        // A member whose directory is gone is a finding about the workspace, not a reason to refuse
        // reading it.
        let gone = workspace.member("gone").expect("a member");
        assert!(matches!(gone.holds(), MemberHolds::NoDirectory { .. }));
        assert!(gone.root().is_none());
        assert!(workspace.render().join("\n").contains("no directory"));
    }

    #[test]
    fn a_member_root_that_leaves_the_workspace_is_a_document_refusal() {
        let sandbox = sandbox();
        let text = workspace_json(&[("outside", "../elsewhere")]);
        let path = sandbox.write(WORKSPACE_FILE_NAME, &text);
        let error = Workspace::read(&path).expect_err("a root that leaves the workspace");
        assert!(
            error.message().contains("climbs above the project root"),
            "{}",
            error.message()
        );

        // A root that is a file rather than a directory is reported rather than resolved.
        sandbox.write("not-a-dir", "a file");
        let workspace = sandbox.workspace(&[("thing", "not-a-dir")]);
        assert!(matches!(
            workspace.member("thing").expect("a member").holds(),
            MemberHolds::NoDirectory { .. }
        ));
    }

    #[test]
    fn two_members_may_not_overlap() {
        let sandbox = sandbox();
        sandbox.write("app/project.json", &project("app-id", "App", ""));
        let nested = workspace_json(&[("app", "app"), ("inner", "app/internal")]);
        let path = sandbox.write(WORKSPACE_FILE_NAME, &nested);
        let error = Workspace::read(&path).expect_err("overlapping members");
        assert!(error.message().contains("overlap"), "{}", error.message());
        assert!(
            error.message().contains("two members"),
            "{}",
            error.message()
        );

        // A member that *is* the workspace root overlaps everything else in it.
        let root_member = workspace_json(&[("whole", "."), ("app", "app")]);
        let path = sandbox.write(WORKSPACE_FILE_NAME, &root_member);
        assert!(Workspace::read(&path).is_err());
    }

    #[test]
    fn a_document_the_members_cannot_share_is_refused_at_read_time() {
        let sandbox = sandbox();
        let path = sandbox.write(WORKSPACE_FILE_NAME, &workspace_json(&[]));
        // A name is how a member is addressed, so two members cannot claim one.
        let duplicated = workspace_json(&[("app", "app"), ("app", "app")]);
        sandbox.write(WORKSPACE_FILE_NAME, &duplicated);
        assert!(Workspace::read(&path).is_err());

        // Versions this build does not write are refused rather than emptied.
        for version in [0, 3] {
            let text =
                workspace_json(&[]).replace("\"format\": 1", &format!("\"format\": {version}"));
            sandbox.write(WORKSPACE_FILE_NAME, &text);
            assert!(
                Workspace::read(&path).is_err(),
                "format {version} should be refused"
            );
        }
        // A member with no name, and a document with no members list, are both refused.
        sandbox.write(
            WORKSPACE_FILE_NAME,
            "{\"format\": 1, \"workspace\": {\"id\": \"a\", \"name\": \"b\"}}",
        );
        assert!(Workspace::read(&path).is_err());
        sandbox.write(
            WORKSPACE_FILE_NAME,
            "{\"format\": 1, \"workspace\": {\"id\": \"a\", \"name\": \"b\"}, \"members\": [{\"root\": \"app\"}]}",
        );
        assert!(Workspace::read(&path).is_err());
    }

    #[test]
    fn a_file_is_identified_by_its_member_and_not_by_its_path_alone() {
        let sandbox = sandbox();
        sandbox.write("app/project.json", &project("app-id", "App", ""));
        sandbox.write("lib/project.json", &project("lib-id", "Lib", ""));
        sandbox.write("app/pages/main.json", "{}");
        sandbox.write("lib/pages/main.json", "{}");
        let workspace = sandbox.workspace(&[("app", "app"), ("lib", "lib")]);

        let files = workspace.files();
        let same_named: Vec<&WorkspaceFile> = files
            .iter()
            .filter(|file| file.written() == "pages/main.json")
            .collect();
        assert_eq!(
            same_named.len(),
            2,
            "two members name it, so there are two files: {files:#?}"
        );
        assert_eq!(
            same_named
                .iter()
                .map(|file| file.name())
                .collect::<Vec<_>>(),
            vec!["app:pages/main.json", "lib:pages/main.json"]
        );
        assert!(
            same_named
                .iter()
                .all(|file| file.verdict().contains("present")),
            "{same_named:#?}"
        );

        // The name addresses one of them, and the two are never confused for one another.
        let found = workspace.file("lib:pages/main.json").expect("a file");
        assert_eq!(found.member(), "lib");
        assert_eq!(found.name(), "lib:pages/main.json");
        let unknown = workspace
            .file("nope:pages/main.json")
            .expect_err("no such member");
        assert!(matches!(unknown, WorkspaceLookup::NoSuchMember { .. }));
        assert!(unknown.message().contains("`app`"), "{}", unknown.message());

        // A path with no member is refused rather than guessed at.
        let refusal = WorkspaceFile::parse("pages/main.json").expect_err("no member in the name");
        assert!(matches!(refusal, WorkspaceFileRefusal::NoMember { .. }));
        assert!(refusal.message().contains("member:path"));
        assert!(WorkspaceFile::parse("app:").is_err());
    }

    #[test]
    fn a_path_that_leaves_its_member_lands_in_another_one() {
        let sandbox = sandbox();
        sandbox.write(
            "app/project.json",
            &project(
                "app-id",
                "App",
                "{\"path\": \"../shared/logo.svg\"}, {\"path\": \"../../outside.svg\"}",
            ),
        );
        sandbox.write(
            "shared/project.json",
            &project("shared-id", "Shared", "{\"path\": \"logo.svg\"}"),
        );
        sandbox.directory("shared");
        sandbox.write("shared/logo.svg", "<svg/>");
        sandbox.directory("outside");
        let workspace = sandbox.workspace(&[("app", "app"), ("shared", "shared")]);

        let app = workspace.member("app").expect("a member");
        // The member's own reader refused both paths: they leave its root.
        let files = workspace.files();
        let refused: Vec<&WorkspaceFile> = files
            .iter()
            .filter(|file| file.member() == "app" && file.verdict().contains("refused"))
            .collect();
        assert_eq!(refused.len(), 2, "{files:#?}");

        // The workspace answers the second question: one lands in another member, one leaves.
        let shared = workspace.resolve(app, "../shared/logo.svg");
        assert_eq!(shared.member(), Some("shared"));
        assert!(shared.render().contains("logo.svg"), "{}", shared.render());
        let outside = workspace.resolve(app, "../../outside.svg");
        assert!(matches!(outside, CrossRoot::Outside { .. }));
        assert!(
            outside.render().contains("outside the workspace"),
            "{}",
            outside.render()
        );
        // A path inside its own member is its own business, and the workspace adds nothing.
        assert!(workspace.resolve(app, "pages/main.json").is_within());
        // A path whose parent is not there yet is inside its member too: it is a file the project is
        // about to create, not a path that went somewhere else.
        assert!(
            workspace
                .resolve(app, "assets/not-written-yet.svg")
                .is_within()
        );

        // Inside the workspace and outside every member is its own answer: the path is reachable and
        // nothing says whose it is, which is what adding a member answers.
        sandbox.write(
            "app/project.json",
            &project(
                "app-id",
                "App",
                "{\"path\": \"../shared/logo.svg\"}, {\"path\": \"../../outside.svg\"}, \
                 {\"path\": \"../loose.svg\"}",
            ),
        );
        sandbox.write("loose.svg", "<svg/>");
        let loose = workspace.resolve(workspace.member("app").expect("a member"), "../loose.svg");
        assert!(matches!(loose, CrossRoot::NoMemberOwns { .. }), "{loose:?}");
        assert!(
            loose.render().contains("no member owns it"),
            "{}",
            loose.render()
        );
        // Three references leave their member: the one that lands in another member, the one that
        // leaves the workspace, and the one nothing owns.
        // A member whose directory somebody moved is still the member a path lands in: whose file
        // it is does not change because the directory is not there right now.
        std::fs::rename(
            sandbox.root.join("shared"),
            sandbox.root.join("shared-away"),
        )
        .expect("the directory is moved");
        // The workspace read before the rename still says the member is a project — it was, when it
        // was read — so the question is asked of a workspace read now.
        let after = Workspace::read(workspace.path()).expect("the workspace reads");
        let missing = after.member("shared").expect("a member");
        assert!(matches!(missing.holds(), MemberHolds::NoDirectory { .. }));
        assert!(
            missing.holds().render().contains("there is nothing at"),
            "{}",
            missing.holds().render()
        );
        assert_eq!(
            after.resolve(after.member("app").expect("a member"), "../shared/logo.svg"),
            CrossRoot::AnotherMember {
                member: "shared".to_owned(),
                path: sandbox.root.join("shared/logo.svg"),
            }
        );

        let crossing = workspace.cross_root_references();
        assert_eq!(crossing.len(), 3, "{crossing:#?}");
        assert!(
            workspace
                .render()
                .join("\n")
                .contains("leave their own member")
        );
    }

    #[test]
    fn removing_a_member_writes_the_document_and_touches_no_directory() {
        let sandbox = sandbox();
        sandbox.write("app/project.json", &project("app-id", "App", ""));
        sandbox.write("lib/project.json", &project("lib-id", "Lib", ""));
        sandbox.write("lib/pages/main.json", "{}");
        let workspace = sandbox.workspace(&[("app", "app"), ("lib", "lib")]);
        let before_lib = std::fs::read(sandbox.root.join("lib/pages/main.json")).expect("a file");
        let before_document = std::fs::read_to_string(workspace.path()).expect("the document");

        let plan = workspace.plan(WorkspaceChange::Remove {
            name: "lib".to_owned(),
        });
        assert!(plan.is_executable(), "{:?}", plan.render());
        assert_eq!(plan.members_after(), vec!["app"]);
        let written = plan.execute().expect("the removal is written");
        assert_eq!(written.members(), ["app"]);

        // The acceptance: the directory, and everything in it, is exactly as it was.
        assert!(
            sandbox.root.join("lib").is_dir(),
            "the directory is still there"
        );
        assert_eq!(
            std::fs::read(sandbox.root.join("lib/pages/main.json")).expect("still there"),
            before_lib
        );
        assert_ne!(
            std::fs::read_to_string(workspace.path()).expect("the document"),
            before_document,
            "the document is what changed"
        );

        // And what was written is read back: the member is gone from the document, the other one is
        // untouched, and a document this build writes can be read by this build.
        let after = written.workspace();
        assert_eq!(after.members().len(), 1);
        assert_eq!(
            after.member("app").expect("a member").identity(),
            Some(("app-id", "App"))
        );
        assert!(after.member("lib").is_none());

        // A plan made from a document that has changed since is refused, and nothing is written.
        let stale = workspace.plan(WorkspaceChange::Remove {
            name: "lib".to_owned(),
        });
        sandbox.write(
            WORKSPACE_FILE_NAME,
            &workspace_json(&[("app", "app"), ("lib", "lib"), ("third", "third")]),
        );
        let failure = stale.execute().expect_err("the document changed");
        assert!(
            failure.message().contains("as it was"),
            "{}",
            failure.message()
        );
        assert!(
            std::fs::read_to_string(workspace.path())
                .expect("the document")
                .contains("third"),
            "the edit somebody else made is the file that stays"
        );
    }

    #[test]
    fn removing_a_member_another_member_names_is_blocked_and_lists_the_references() {
        let sandbox = sandbox();
        sandbox.write(
            "app/project.json",
            &project("app-id", "App", "{\"path\": \"../shared/logo.svg\"}"),
        );
        sandbox.write("shared/project.json", &project("shared-id", "Shared", ""));
        let workspace = sandbox.workspace(&[("app", "app"), ("shared", "shared")]);

        let plan = workspace.plan(WorkspaceChange::Remove {
            name: "shared".to_owned(),
        });
        assert!(!plan.is_executable());
        assert_eq!(plan.referrers().len(), 1, "{:?}", plan.render());
        assert_eq!(plan.referrers()[0].from(), "app");
        assert_eq!(plan.referrers()[0].written(), "../shared/logo.svg");
        assert!(plan.members_after().is_empty());
        let rendered = plan.render().join("\n");
        assert!(
            rendered.contains("it is named by app:../shared/logo.svg"),
            "{rendered}"
        );
        assert!(rendered.contains("blocked:"), "{rendered}");

        let before = std::fs::read_to_string(workspace.path()).expect("the document");
        let failure = plan.execute().expect_err("blocked");
        assert!(matches!(failure, WorkspaceFailure::Blocked { .. }));
        assert_eq!(failure.blockers().len(), 1);
        assert!(failure.message().contains("the workspace is as it was"));
        assert_eq!(
            std::fs::read_to_string(workspace.path()).expect("the document"),
            before,
            "a blocked removal writes nothing at all"
        );
    }

    #[test]
    fn adding_a_member_names_a_root_that_is_there_and_never_creates_one() {
        let sandbox = sandbox();
        sandbox.write("app/project.json", &project("app-id", "App", ""));
        sandbox.write("lib/project.json", &project("lib-id", "Lib", ""));
        sandbox.directory("fresh");
        let workspace = sandbox.workspace(&[("app", "app")]);

        // A name that is taken, a root that is not there, and a root that overlaps: all three are
        // refusals rather than something the command works around.
        let taken = workspace.plan(WorkspaceChange::Add {
            name: "app".to_owned(),
            root: "lib".to_owned(),
        });
        assert!(!taken.is_executable());
        assert!(
            taken.blockers()[0].contains("already there"),
            "{:?}",
            taken.render()
        );

        let nowhere = workspace.plan(WorkspaceChange::Add {
            name: "nowhere".to_owned(),
            root: "nowhere".to_owned(),
        });
        assert!(!nowhere.is_executable());
        assert!(
            nowhere.blockers()[0].contains("names a root that is there"),
            "{:?}",
            nowhere.render()
        );
        assert!(
            !sandbox.root.join("nowhere").exists(),
            "nothing was created"
        );

        // Two things are wrong with this one, and both are said: there is nothing at the root, and
        // it would sit inside another member. A plan that reported only the first would make a
        // person fix one problem, try again, and find the next.
        let overlapping = workspace.plan(WorkspaceChange::Add {
            name: "inside".to_owned(),
            root: "app/pages".to_owned(),
        });
        assert!(!overlapping.is_executable());
        assert_eq!(
            overlapping.blockers().len(),
            2,
            "{:?}",
            overlapping.render()
        );
        let said = overlapping.blockers().join(" ");
        assert!(said.contains("nothing to add"), "{said}");
        assert!(said.contains("overlaps"), "{said}");

        // The one that is possible: a directory that is there, and a document written and read back.
        let adding = workspace.plan(WorkspaceChange::Add {
            name: "lib".to_owned(),
            root: "lib".to_owned(),
        });
        assert!(adding.is_executable(), "{:?}", adding.render());
        assert_eq!(adding.members_after(), vec!["app", "lib"]);
        assert!(
            adding
                .render()
                .join("\n")
                .contains("no directory is created")
        );
        let written = adding.execute().expect("the addition is written");
        assert_eq!(written.members(), ["app", "lib"]);
        assert_eq!(
            written
                .workspace()
                .member("lib")
                .expect("a member")
                .identity(),
            Some(("lib-id", "Lib"))
        );
        // An empty directory can be a member too, and it is reported as a directory with no project
        // document rather than as a project with an invented identity.
        let adding = written.workspace().plan(WorkspaceChange::Add {
            name: "fresh".to_owned(),
            root: "fresh".to_owned(),
        });
        let written = adding.execute().expect("the addition is written");
        assert!(matches!(
            written
                .workspace()
                .member("fresh")
                .expect("a member")
                .holds(),
            MemberHolds::NothingYet { .. }
        ));
    }

    #[test]
    fn trust_and_targets_belong_to_one_member_and_do_not_leak() {
        let sandbox = sandbox();
        sandbox.write("app/project.json", &project("app-id", "App", ""));
        sandbox.write("lib/project.json", &project("lib-id", "Lib", ""));
        let workspace = sandbox.workspace(&[("app", "app"), ("lib", "lib")]);

        // Trust is per root, which is what "one member's trust is not another's" means here: the
        // workspace does not add a second notion of trust, it uses the one the host already keys by
        // the root.
        let app_root = ProjectRoot::new(sandbox.root.join("app")).expect("a root");
        let lib_root = ProjectRoot::new(sandbox.root.join("lib")).expect("a root");
        let permissions = PermissionService::new();
        permissions.trust(&app_root);
        assert!(permissions.is_trusted(&app_root));
        assert!(
            !permissions.is_trusted(&lib_root),
            "trusting one member must not trust another"
        );
        assert!(permissions.permission(&lib_root).is_none());

        // Targets are per member too: a resolution carries the identity it was made for and refuses
        // to be handed to another member.
        let state = MachineConfiguration::empty(sandbox.root.join("configuration.json"));
        let app = workspace
            .member("app")
            .expect("a member")
            .project_file()
            .expect("a project");
        let lib = workspace
            .member("lib")
            .expect("a member")
            .project_file()
            .expect("a project");
        let app_document = app.document().expect("a document");
        let lib_document = lib.document().expect("a document");
        assert_eq!(app_document.identity().id(), "app-id");
        assert_eq!(lib_document.identity().id(), "lib-id");

        let resolved = ResolvedConfiguration::of(app_document, None, &state).expect("resolved");
        assert!(matches!(
            resolved.for_project("lib-id"),
            Err(ConfigurationRefusal::AnotherProject { .. })
        ));
        assert!(resolved.for_project("app-id").is_ok());
        assert_ne!(
            ResolvedConfiguration::of(lib_document, None, &state)
                .expect("resolved")
                .target(),
            "",
            "the other member resolves on its own"
        );
    }

    #[test]
    fn the_active_member_is_machine_state_and_belongs_to_one_workspace() {
        let sandbox = sandbox();
        sandbox.write("app/project.json", &project("app-id", "App", ""));
        sandbox.write("lib/project.json", &project("lib-id", "Lib", ""));
        let workspace = sandbox.workspace(&[("app", "app"), ("lib", "lib")]);
        let mut state = WorkspaceState::empty(sandbox.root.join("workspace-state.json"));

        assert!(matches!(
            state.active_for(&workspace),
            ActiveMember::NeverChosen
        ));
        let choice = workspace
            .activate(&mut state, "lib", at(1_000))
            .expect("a member of this workspace");
        assert_eq!(choice.member(), "lib");
        let active = state.active_for(&workspace);
        assert_eq!(active.name(), Some("lib"));
        assert!(active.is_for_this_workspace());
        assert_eq!(
            active.member().map(|member| member.identity()),
            Some(Some(("lib-id", "Lib")))
        );
        assert!(active.render().contains("chosen for this workspace"));

        // The state survives a reload, and another workspace does not get this choice.
        let path = state.save().expect("the state is written");
        let reloaded = WorkspaceState::load(&path).expect("the state reads back");
        assert_eq!(reloaded.choices().len(), 1);
        // A second workspace, with its own identity: a choice belongs to one of them.
        sandbox.write(
            "other/app/project.json",
            &project("other-app-id", "Other app", ""),
        );
        let other_path = sandbox.write(
            "other/workspace.json",
            &workspace_json_for("ws-0002", "Another workspace", &[("app", "app")]),
        );
        let other = Workspace::read(&other_path).expect("the other workspace reads");
        assert_eq!(other.id(), "ws-0002");
        assert!(matches!(
            reloaded.active_for(&other),
            ActiveMember::AnotherWorkspace(_)
        ));
        assert!(
            reloaded
                .active_for(&other)
                .render()
                .contains("belongs to one workspace")
        );

        // A choice that names a member which is no longer there is reported, not moved to another.
        let without = other
            .plan(WorkspaceChange::Remove {
                name: "app".to_owned(),
            })
            .execute()
            .expect("the removal is written");
        let emptied = Workspace::read(without.path()).expect("reads");
        let mut state = WorkspaceState::empty(sandbox.root.join("workspace-state-2.json"));
        assert!(
            emptied.activate(&mut state, "nothing", at(1_000)).is_err(),
            "an unknown member is refused"
        );
        assert!(
            emptied
                .activate(&mut state, "nothing", at(1_000))
                .expect_err("refused")
                .message()
                .contains("nothing was recorded")
        );
        // The other way round: a choice recorded for a member, from a document that still has it,
        // and then read against a document that does not.
        let mut state = WorkspaceState::empty(sandbox.root.join("workspace-state-3.json"));
        workspace
            .activate(&mut state, "lib", at(2_000))
            .expect("recorded");
        let removed = workspace
            .plan(WorkspaceChange::Remove {
                name: "lib".to_owned(),
            })
            .execute()
            .expect("the removal is written");
        let active = state.active_for(removed.workspace());
        assert!(matches!(active, ActiveMember::MemberIsGone { .. }));
        assert_eq!(active.name(), Some("lib"));
        assert!(
            active.render().contains("not a member any more"),
            "{}",
            active.render()
        );

        // A state file from a newer build is refused rather than read as empty.
        let text = std::fs::read_to_string(&path).expect("the state");
        std::fs::write(&path, text.replace("\"format\": 1", "\"format\": 3"))
            .expect("a newer state");
        assert!(WorkspaceState::load(&path).is_err());
    }

    #[test]
    fn a_workspace_is_created_once_and_read_back() {
        let sandbox = sandbox();
        sandbox.directory("area");
        let draft =
            Workspace::draft(sandbox.root.join("area"), "Area", "area-0001").expect("a draft");
        assert!(draft.members().is_empty());
        let created = draft.create().expect("the document is created");
        assert_eq!(created.id(), "area-0001");
        assert!(created.render().join("\n").contains("no members yet"));

        // Creating it again refuses: this build does not overwrite a workspace document.
        let again =
            Workspace::draft(sandbox.root.join("area"), "Area", "area-0001").expect("a draft");
        let error = again.create().expect_err("it is already there");
        assert!(
            error.message().contains("already there"),
            "{}",
            error.message()
        );

        // A directory that is not there is refused, and the document is not made anywhere else.
        let missing = Workspace::draft(sandbox.root.join("nowhere"), "Nowhere", "n-0001")
            .expect_err("there is nothing to make a workspace in");
        assert!(
            missing.message().contains("resolving the workspace root"),
            "{}",
            missing.message()
        );
        assert!(!sandbox.root.join("nowhere").exists());
    }

    #[test]
    fn a_name_that_cannot_be_addressed_is_refused_where_it_is_used() {
        use super::name_is_addressable;

        for good in ["app", "lib-core", "app.member", "a"] {
            assert!(name_is_addressable(good), "{good} should be usable");
        }
        for bad in [
            "", " ", "app ", " app", "app:path", "app=lib", "app lib", "app/lib",
        ] {
            assert!(!name_is_addressable(bad), "{bad:?} should be refused");
        }

        // The model refuses it too, so a name that could not be addressed cannot get into a document
        // through some other door.
        let sandbox = sandbox();
        sandbox.write("app/project.json", &project("app-id", "App", ""));
        let workspace = sandbox.workspace(&[("app", "app")]);
        for bad in ["two members", "with:colon", "with=equals"] {
            let plan = workspace.plan(WorkspaceChange::Add {
                name: bad.to_owned(),
                root: "app".to_owned(),
            });
            assert!(!plan.is_executable(), "{bad:?} should be refused");
            let said = plan.blockers().join(" ");
            assert!(said.contains("cannot be a member's name"), "{said}");
        }
    }

    #[test]
    fn a_workspace_document_round_trips_as_json() {
        let sandbox = sandbox();
        sandbox.write("app/project.json", &project("app-id", "App", ""));
        sandbox.directory("fresh");
        let workspace = sandbox.workspace(&[("app", "app")]);
        let plan = workspace.plan(WorkspaceChange::Add {
            name: "fresh".to_owned(),
            root: "fresh".to_owned(),
        });
        let written = plan.execute().expect("written");
        let text = std::fs::read_to_string(written.path()).expect("the document");
        for member in super::WORKSPACE_MEMBERS {
            assert!(text.contains(&format!("\"{member}\"")), "{member}: {text}");
        }
        for member in super::MEMBER_MEMBERS {
            assert!(text.contains(&format!("\"{member}\"")), "{member}: {text}");
        }
        let again = Workspace::read(written.path()).expect("reads back");
        assert_eq!(again.id(), "ws-0001");
        assert_eq!(
            again.members().iter().map(|m| m.name()).collect::<Vec<_>>(),
            vec!["app", "fresh"]
        );
        // The document is a value that writes to what it reads: no member is invented and none is
        // dropped by the round trip.
        let text = again.to_json().render();
        let reparsed = Workspace::read(written.path()).expect("reads back");
        assert_eq!(reparsed.to_json().render(), text);
        assert_eq!(reparsed.members().len(), 2);
    }
}
