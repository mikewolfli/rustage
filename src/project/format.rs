// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The project metadata document, format version 1: the file a project *is* (contract
//! **C-PROJECT**).
//!
//! A version 1 document is a JSON object. These are its members:
//!
//! | member | kind | meaning |
//! |---|---|---|
//! | `format` | whole number | the format version; required, and this build writes `1` |
//! | `project` | object | identity: `id` and `name`, both required and non-empty |
//! | `pages` | array | one entry per page: `document` (required), `title`, `stylesheet` |
//! | `resources` | array | one entry per resource: `path` (required) |
//! | `targets` | array | one entry per target profile: `name` (required), `profile`, `features` |
//! | `generated` | array | one entry per generated file: `output` and `from`, both required |
//! | `network` | object | `services`: per service `name`, `host`, `port`, `tls`, `credential` |
//!
//! Every path in the document is **relative to the directory the document is in**, which is the
//! project root (decision `F1`). A reference that would leave that directory — through `..`, through
//! an absolute path, or through a link — is refused, and so is writing a document that contains one.
//! The root itself is deliberately **not** a member: a member holding an absolute root would be a
//! second answer to "where is this project", and the wrong one the moment the project is moved or
//! checked out somewhere else.
//!
//! ## Version policy
//!
//! The format version is mandatory, because it is what tells a reader how to read the rest.
//!
//! * Equal to the version this build writes: the document is interpreted, member by member.
//! * **Newer than this build**: the document opens, but nothing but the identity is read, and
//!   [`ProjectFile::writer`] returns an error for it — this build never writes a version it does not
//!   know (decision `U1`). Structure, not discipline, is what enforces that: a document of an
//!   unknown version never becomes a [`ProjectDocument`], and only a [`ProjectDocument`] can be
//!   rendered.
//! * Older than this build: refused with a reason, because migration is a step of its own
//!   (decision `U1`; task T02.07). Refusing beats guessing, and the message says which way the file
//!   is out of date.
//!
//! ## Unknown members
//!
//! A member this build does not know — at any level, inside `project` or inside `pages[2]` — is
//! **preserved as it was found and reported** in [`ProjectDocument::unknown_fields`]. It is never
//! dropped, so a build that is older than a file cannot damage it, and it is never interpreted, so a
//! build cannot act on a member whose meaning it does not know. Only the writer's canonical member
//! order changes: the writer writes every member version 1 defines (in the order above) and appends
//! the preserved ones, which is why saving a file this build wrote is byte-identical.
//!
//! ## Secrets
//!
//! A credential appears as a **reference**: a name, where the material lives (`system-keyring`, or
//! an environment variable), and when it is expected to expire. The material itself is not
//! representable, and an unknown member inside `credential` or inside its `source` is *refused*
//! rather than preserved — that is the one place where version 1 will not keep what it does not
//! know, because the member it would be keeping could be somebody's token.
//!
//! ## What is not here on purpose
//!
//! Machine-local state — window geometry, which page was open last, proxy and TLS overrides for this
//! machine — belongs to the host's configuration directory, not to the project. A project file is
//! meant to be shared and committed, and one machine's state in it would be wrong on the next
//! machine. Those members are therefore reported as unknown if they appear, instead of being read as
//! if they meant something here.

use std::cmp::Ordering;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::contracts::revision::FormatVersion;
use crate::host::network::{
    CertificateSource, CredentialRef, CredentialSource, ServicePolicy, TlsPolicy,
};
use crate::host::paths::{PathError, ProjectRoot};
use crate::project::json::{self, Json, JsonError, JsonKind, JsonObject, Number};

/// The name a project's metadata document has inside its root.
pub const PROJECT_FILE_NAME: &str = "project.json";

/// The member names the current version defines, in the order the writer writes them.
///
/// It is a value rather than a comment so that the cost of adding a member is visible in one place;
/// a member added here without a format version bump is exactly what decision `F1` forbids, and
/// `startup` is what that bump looked like: version 2 added it, and version 1 documents are read by
/// the migration table below.
pub const VERSION_MEMBERS: [&str; 8] = [
    "format",
    "project",
    "pages",
    "resources",
    "targets",
    "generated",
    "startup",
    "network",
];

/// One step between two format versions.
///
/// Decision `Q6` asks every format change to come with a **versioned** migration step and a
/// repeatable test, and decision `U1` makes migration forward-only: a step never goes back, and a
/// version with no step out of it is refused rather than guessed at.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MigrationStep {
    from: FormatVersion,
    to: FormatVersion,
    what: &'static str,
}

impl MigrationStep {
    /// The version a document has before the step.
    pub const fn from(self) -> FormatVersion {
        self.from
    }

    /// The version it has after.
    pub const fn to(self) -> FormatVersion {
        self.to
    }

    /// What the step does, in one sentence.
    pub const fn what(self) -> &'static str {
        self.what
    }
}

/// Every step this build can take, oldest first.
///
/// A step that needs no rewriting of the bytes is still a step: it has a version and it is tested,
/// which is what makes "which versions does this build read" answerable without reading the code.
pub const MIGRATION_STEPS: [MigrationStep; 1] = [MigrationStep {
    from: FormatVersion::from_raw(1),
    to: FormatVersion::from_raw(2),
    what: "version 2 added `startup`; a version 1 document is read unchanged, and its startup is \
           simply not chosen yet",
}];

/// The steps between a version and the one this build writes.
///
/// `Ok(steps)` when a path of steps exists, `Err(found)` when it does not: a version with no step out
/// of it is refused, which is what "forward-only, stepwise" means when the answer is no.
pub fn migration_path(from: FormatVersion) -> Result<Vec<MigrationStep>, FormatVersion> {
    let mut steps = Vec::new();
    let mut current = from;
    while current < FormatVersion::CURRENT {
        let Some(step) = MIGRATION_STEPS
            .into_iter()
            .find(|step| step.from == current)
        else {
            return Err(current);
        };
        steps.push(step);
        current = step.to;
    }
    Ok(steps)
}

/// What this build makes of the version a document declares.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VersionVerdict {
    /// The version this build writes: the document is interpreted, and writing it is allowed.
    Current {
        /// The version.
        version: FormatVersion,
    },
    /// An older version this build migrates on the way in (decision `Q6`, `U1`).
    ///
    /// The document is interpreted by this build's rules, and writing it needs the migration path:
    /// the file that was read is kept first (decision `Q6`: a migration never overwrites the
    /// original in place before it is done).
    Migrated {
        /// The version the document declares.
        found: FormatVersion,
        /// The version this build writes.
        current: FormatVersion,
        /// How many steps the path has.
        steps: usize,
    },
    /// A version this build does not know: readable, never written (decision `U1`).
    Newer {
        /// The version the document declares.
        found: FormatVersion,
        /// The version this build writes.
        current: FormatVersion,
    },
}

impl VersionVerdict {
    /// Whether this build may write the document.
    pub const fn is_writable(self) -> bool {
        matches!(self, Self::Current { .. } | Self::Migrated { .. })
    }

    /// Whether writing it means migrating it first, which keeps the file that was read.
    pub const fn needs_migration(self) -> bool {
        matches!(self, Self::Migrated { .. })
    }

    /// The version the document declares.
    pub const fn version(self) -> FormatVersion {
        match self {
            Self::Current { version }
            | Self::Migrated { found: version, .. }
            | Self::Newer { found: version, .. } => version,
        }
    }

    /// A sentence for a person, saying what will happen to the file.
    pub fn message(self) -> String {
        match self {
            Self::Current { version } => {
                format!("{version}, which this build writes")
            }
            Self::Migrated {
                found,
                current,
                steps,
            } => format!(
                "{found}, which this build reads and migrates to {current} in {steps} step(s); \
                 writing it keeps the file that was read"
            ),
            Self::Newer { found, current } => format!(
                "{found} is newer than {current}, which this build writes; \
                 it is opened read-only and this build will not write it"
            ),
        }
    }
}

/// Why a project may not be written.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ProjectWriteRefusal {
    /// The file declares a format this build does not know (decision `U1`).
    UnknownFormat {
        /// The version the document declares.
        found: FormatVersion,
        /// The version this build writes.
        current: FormatVersion,
    },
    /// A reference in the document would leave the project root (decision `F1`).
    RefusedReferences {
        /// How many references were refused.
        count: usize,
        /// The first one, so the message can name it.
        first: String,
    },
}

impl ProjectWriteRefusal {
    /// A sentence naming what has to change before the project can be written.
    pub fn message(&self) -> String {
        match self {
            Self::UnknownFormat { found, current } => format!(
                "{found} is newer than {current}, which this build writes; \
                 nothing was written — update rustage, or copy the project, to change it"
            ),
            Self::RefusedReferences { count, first } => format!(
                "{count} reference(s) leave the project root, starting with `{first}`; \
                 nothing was written — a project file may only point inside its own root"
            ),
        }
    }
}

impl fmt::Display for ProjectWriteRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.message())
    }
}

/// Why a project could not be read, or written.
///
/// Comparable and cloneable so a caller can hold one and still compare it: the value says what was
/// found, and a report that had to compare messages would be comparing prose.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ProjectError {
    /// The file could not be read or written.
    Io {
        /// The path.
        path: PathBuf,
        /// What was being done.
        action: &'static str,
        /// The operating system's own words.
        reason: String,
    },
    /// The text is not JSON.
    NotJson {
        /// The path.
        path: PathBuf,
        /// Where and why it stopped.
        error: JsonError,
    },
    /// A member this build knows is not the kind of value it has to be.
    WrongType {
        /// Where the member is.
        path: String,
        /// What it has to be.
        expected: &'static str,
        /// What it is.
        found: JsonKind,
    },
    /// A member this build needs is not there.
    MissingMember {
        /// Where the member would be.
        path: String,
    },
    /// A member that has to say something says nothing.
    EmptyValue {
        /// Where the member is.
        path: String,
    },
    /// The document declares a format this build is too new for.
    NewerFormat {
        /// The version the document declares.
        found: FormatVersion,
        /// The version this build writes.
        current: FormatVersion,
    },
    /// The document describes something the format does not allow.
    Inconsistent {
        /// Where it is wrong.
        path: String,
        /// What is wrong with it.
        reason: String,
    },
    /// A value has no spelling in this format.
    Unrepresentable {
        /// Where the value is.
        path: String,
        /// Why it cannot be written.
        reason: String,
    },
    /// The document declares a format this build is too old for (it is older than this build).
    OlderFormat {
        /// The version the document declares.
        found: FormatVersion,
        /// The version this build writes.
        current: FormatVersion,
    },
    /// Material for a credential is written into the project, which is not where it belongs.
    SecretInProject {
        /// Where the members are.
        path: String,
        /// The members that are not part of a credential reference.
        members: Vec<String>,
    },
    /// Two entries claim the same thing.
    Duplicate {
        /// Where the second one is.
        path: String,
        /// What they both claim.
        claim: String,
    },
    /// A member names something the document does not declare.
    Dangling {
        /// Where the member is.
        path: String,
        /// What it names.
        name: String,
    },
    /// The file changed on disk since it was read, so writing it would lose somebody's edit.
    ChangedOnDisk {
        /// The path.
        path: PathBuf,
    },
    /// The document is an older version, and writing it means migrating it first.
    MigrationNeedsBackup {
        /// The path.
        path: PathBuf,
        /// The version the document declares.
        from: FormatVersion,
        /// The version this build writes.
        to: FormatVersion,
    },
    /// There is nothing to migrate: the document is already the version this build writes.
    NothingToMigrate {
        /// The path.
        path: PathBuf,
    },
}

impl ProjectError {
    /// A sentence naming what was found, for a diagnostic or a terminal.
    pub fn message(&self) -> String {
        match self {
            Self::Io {
                path,
                action,
                reason,
            } => format!("{}: {action}: {reason}", path.display()),
            Self::NotJson { path, error } => {
                format!("{}: not JSON: {error}", path.display())
            }
            Self::WrongType {
                path,
                expected,
                found,
            } => format!("{path} must be {expected}, but it is {}", found.name()),
            Self::MissingMember { path } => format!("{path} is missing, and it is required"),
            Self::EmptyValue { path } => format!("{path} is empty, and it has to say something"),
            Self::NewerFormat { found, current } => format!(
                "{found} is newer than {current}, which this build writes; it was not read, and \
                 nothing was written"
            ),
            Self::Inconsistent { path, reason } => format!("{path} is inconsistent: {reason}"),
            Self::Unrepresentable { path, reason } => {
                format!("{path} cannot be written as JSON: {reason}")
            }
            Self::OlderFormat { found, current } => format!(
                "{found} is older than {current}, which this build writes, and no migration runs in \
                 this build (task T02.07); nothing was read"
            ),
            Self::SecretInProject { path, members } => format!(
                "{path} holds {}; a project file names a credential and never contains one — \
                 put the material in the system keyring or in an environment variable",
                members.join(", ")
            ),
            Self::Duplicate { path, claim } => {
                format!("{path} claims the same thing as an earlier entry: {claim}")
            }
            Self::Dangling { path, name } => {
                format!("{path} names `{name}`, which nothing in this project declares")
            }
            Self::ChangedOnDisk { path } => format!(
                "{} changed on disk since it was read; nothing was written, so no edit was lost",
                path.display()
            ),
            Self::MigrationNeedsBackup { path, from, to } => format!(
                "{} is {from} and this build writes {to}; nothing was written, because a migration \
                 keeps the file it read first (see the migrating save, which writes \
                 `.v{}.bak` beside it)",
                path.display(),
                from.get()
            ),
            Self::NothingToMigrate { path } => format!(
                "{} is already the version this build writes, so there is nothing to migrate",
                path.display()
            ),
        }
    }
}

impl fmt::Display for ProjectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.message())
    }
}

impl std::error::Error for ProjectError {}

/// A member of an object that this build does not know about.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct UnknownField {
    path: String,
}

impl UnknownField {
    /// Where it is, as a dotted path with array indices, for example `pages[0].customKey`.
    pub fn path(&self) -> &str {
        &self.path
    }

    fn member(parent: &str, name: &str) -> Self {
        Self {
            path: if parent.is_empty() {
                name.to_owned()
            } else {
                format!("{parent}.{name}")
            },
        }
    }
}

impl fmt::Display for UnknownField {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.path)
    }
}

/// The members of one object that interpretation left over, in the order they were found.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct Preserved {
    members: Vec<(String, Json)>,
}

impl Preserved {
    /// Whether interpretation left nothing over.
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// How many members were left over.
    pub fn len(&self) -> usize {
        self.members.len()
    }

    /// The names, in the order they appeared.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.members.iter().map(|(name, _)| name.as_str())
    }

    /// The members, in the order they appeared.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Json)> {
        self.members
            .iter()
            .map(|(name, value)| (name.as_str(), value))
    }

    /// One of them, by name.
    pub fn get(&self, name: &str) -> Option<&Json> {
        self.members
            .iter()
            .find(|(member, _)| member == name)
            .map(|(_, value)| value)
    }

    /// Whether a name was left over.
    pub fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    pub(crate) fn names_at(&self, path: &str, out: &mut Vec<UnknownField>) {
        for (name, _) in &self.members {
            out.push(UnknownField::member(path, name));
        }
    }

    pub(crate) fn adopt(&self, object: &mut JsonObject) {
        for (name, value) in &self.members {
            object.insert(name.clone(), value.clone());
        }
    }
}

/// Where a credential's material lives.
fn decode_credential_source(reader: &mut Reader<'_>) -> Result<CredentialSource, ProjectError> {
    let Some(mut member) = sub_object(reader, "source")? else {
        return Err(ProjectError::MissingMember {
            path: reader.member_path("source"),
        });
    };
    member.refuse_extra_members(&["kind", "variable"])?;
    let kind = member.required_string("kind")?;
    let source = match kind.as_str() {
        "system-keyring" => CredentialSource::SystemKeyring,
        "environment" => {
            let variable = member.required_string("variable")?;
            CredentialSource::Environment(variable)
        }
        _ => {
            return Err(ProjectError::WrongType {
                path: member.member_path("kind"),
                expected: "`system-keyring` or `environment`",
                found: JsonKind::String,
            });
        }
    };
    Ok(source)
}

/// A credential reference: a name, where the material lives, and when it is expected to expire.
fn encode_credential(credential: &CredentialRef) -> Json {
    let mut source = JsonObject::new();
    match credential.source() {
        CredentialSource::SystemKeyring => {
            source.insert("kind", Json::string("system-keyring"));
        }
        CredentialSource::Environment(variable) => {
            source.insert("kind", Json::string("environment"));
            source.insert("variable", Json::string(variable));
        }
    }
    let mut out = JsonObject::new();
    out.insert("name", Json::string(credential.name()));
    out.insert("source", Json::Object(source));
    if let Some(milliseconds) = credential.expires_at().and_then(unix_millis) {
        out.insert(
            "expires_at_unix_ms",
            Json::Number(Number::from_i64(milliseconds)),
        );
    }
    Json::Object(out)
}

/// When an instant is, in whole milliseconds since 1970, when the format can say it.
///
/// The format's own resolution is a millisecond: an instant that is finer than that is written
/// truncated rather than refused, because a credential expiring a microsecond later or earlier than
/// written is not a difference anybody can act on. An instant so far from 1970 that it does not fit
/// has no spelling here and the member is left out rather than saturated, so nothing is ever written
/// as a time it is not.
pub(crate) fn unix_millis(instant: SystemTime) -> Option<i64> {
    match instant.duration_since(UNIX_EPOCH) {
        Ok(duration) => i64::try_from(duration.as_millis()).ok(),
        Err(error) => i64::try_from(error.duration().as_millis())
            .ok()
            .map(|milliseconds| -milliseconds),
    }
}

fn instant_from_millis(milliseconds: i64) -> Option<SystemTime> {
    if milliseconds >= 0 {
        let milliseconds = u64::try_from(milliseconds).ok()?;
        UNIX_EPOCH.checked_add(Duration::from_millis(milliseconds))
    } else {
        let milliseconds = u64::try_from(milliseconds.checked_neg()?).ok()?;
        UNIX_EPOCH.checked_sub(Duration::from_millis(milliseconds))
    }
}

/// A path in the document, and what happened when it was resolved against the project root.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Reference {
    kind: ReferenceKind,
    written: String,
    resolved: Option<PathBuf>,
    verdict: ReferenceVerdict,
}

impl Reference {
    /// What this reference is for.
    pub const fn kind(&self) -> ReferenceKind {
        self.kind
    }

    /// The path as the document writes it.
    pub fn written(&self) -> &str {
        &self.written
    }

    /// Where it is, when it stayed inside the root.
    pub fn resolved(&self) -> Option<&Path> {
        self.resolved.as_deref()
    }

    /// What happened when it was resolved.
    pub const fn verdict(&self) -> &ReferenceVerdict {
        &self.verdict
    }

    /// Whether it was refused for leaving the root.
    pub const fn is_refused(&self) -> bool {
        matches!(self.verdict, ReferenceVerdict::Refused { .. })
    }

    pub(crate) fn resolves_like(&self, other: &Self) -> bool {
        match (self.resolved(), other.resolved()) {
            (Some(mine), Some(theirs)) => mine == theirs,
            _ => self.written == other.written,
        }
    }

    /// One line: what it is for, how it is written, and what happened.
    pub fn render(&self) -> String {
        format!(
            "{:<21} {:<40} {}",
            self.kind.name(),
            self.written,
            self.verdict.render()
        )
    }
}

/// What a reference is for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ReferenceKind {
    /// A page's own document.
    PageDocument,
    /// A page's stylesheet.
    Stylesheet,
    /// A resource file.
    Resource,
    /// A generated file the designer writes.
    GeneratedOutput,
    /// The design source a generated file comes from.
    DesignSource,
    /// A certificate authority file added to the system's anchors.
    CertificateAuthority,
}

impl ReferenceKind {
    /// The word a report uses.
    pub const fn name(self) -> &'static str {
        match self {
            Self::PageDocument => "page-document",
            Self::Stylesheet => "stylesheet",
            Self::Resource => "resource",
            Self::GeneratedOutput => "generated-output",
            Self::DesignSource => "design-source",
            Self::CertificateAuthority => "certificate-authority",
        }
    }
}

/// What happened when a reference was resolved against the project root.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ReferenceVerdict {
    /// Inside the root, and something is there.
    Present,
    /// Inside the root, and nothing is there yet.
    Absent,
    /// Refused: it does not stay inside the root.
    Refused {
        /// Why, in the path layer's own words.
        reason: String,
    },
}

impl ReferenceVerdict {
    /// A sentence for a person.
    pub fn render(&self) -> String {
        match self {
            Self::Present => "present".to_owned(),
            Self::Absent => "not there yet".to_owned(),
            Self::Refused { reason } => format!("refused: {reason}"),
        }
    }
}

/// A project's identity: what it is called, and which project it is.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ProjectIdentity {
    id: String,
    name: String,
    preserved: Preserved,
}

impl ProjectIdentity {
    /// The identifier, which stays the same for the life of the project.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The name a person sees.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// What interpretation left over.
    pub fn preserved(&self) -> &Preserved {
        &self.preserved
    }
}

/// One page: its document, and where its styles come from.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PageEntry {
    title: Option<String>,
    document: Reference,
    stylesheet: Option<Reference>,
    preserved: Preserved,
}

impl PageEntry {
    /// The name a person sees, when the document gives one.
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// The page's own document.
    pub fn document(&self) -> &Reference {
        &self.document
    }

    /// The stylesheet, when the page has one of its own.
    pub fn stylesheet(&self) -> Option<&Reference> {
        self.stylesheet.as_ref()
    }

    /// What interpretation left over.
    pub fn preserved(&self) -> &Preserved {
        &self.preserved
    }
}

/// One resource file.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ResourceEntry {
    path: Reference,
    preserved: Preserved,
}

impl ResourceEntry {
    /// The file, relative to the project root.
    pub fn path(&self) -> &Reference {
        &self.path
    }

    /// What interpretation left over.
    pub fn preserved(&self) -> &Preserved {
        &self.preserved
    }
}

/// One target profile: public configuration, never a secret.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TargetProfile {
    name: String,
    profile: Option<String>,
    features: Vec<String>,
    preserved: Preserved,
}

impl TargetProfile {
    /// What the target is called.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The build profile it uses, when it names one.
    pub fn profile(&self) -> Option<&str> {
        self.profile.as_deref()
    }

    /// The features it enables; empty when the document does not say, which is what "no features"
    /// means for a version 1 document.
    pub fn features(&self) -> &[String] {
        &self.features
    }

    /// What interpretation left over.
    pub fn preserved(&self) -> &Preserved {
        &self.preserved
    }
}

/// One generated file and the design source it comes from.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct GeneratedTarget {
    output: Reference,
    from: Reference,
    preserved: Preserved,
}

impl GeneratedTarget {
    /// The file the designer writes.
    pub fn output(&self) -> &Reference {
        &self.output
    }

    /// The design source it is generated from.
    pub fn from(&self) -> &Reference {
        &self.from
    }

    /// What interpretation left over.
    pub fn preserved(&self) -> &Preserved {
        &self.preserved
    }
}

/// One network service the project is allowed to use, with its credential by reference.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ServiceEntry {
    policy: ServicePolicy,
    roots_file: Option<Reference>,
    tls_preserved: Preserved,
    preserved: Preserved,
}

impl ServiceEntry {
    /// The policy the host enforces.
    pub fn policy(&self) -> &ServicePolicy {
        &self.policy
    }

    /// The certificate authority file it adds to the system's anchors.
    pub fn roots_file(&self) -> Option<&Reference> {
        self.roots_file.as_ref()
    }

    /// What interpretation left over inside its TLS configuration.
    pub fn tls_preserved(&self) -> &Preserved {
        &self.tls_preserved
    }

    /// What interpretation left over.
    pub fn preserved(&self) -> &Preserved {
        &self.preserved
    }
}

/// A document of the version this build writes.
///
/// There is no way to build one from a version this build does not know: [`read`] is the only
/// constructor and it returns [`ProjectContent::Foreign`] for those. That is what makes "never write
/// a version I do not know" a property of the type rather than a check somebody has to remember.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ProjectDocument {
    identity: ProjectIdentity,
    pages: Vec<PageEntry>,
    resources: Vec<ResourceEntry>,
    targets: Vec<TargetProfile>,
    generated: Vec<GeneratedTarget>,
    /// Which entry a target starts with, when one is chosen. Version 2 added this member.
    startup: Option<Reference>,
    services: Vec<ServiceEntry>,
    network_preserved: Preserved,
    preserved: Preserved,
    unknown: Vec<UnknownField>,
}

impl ProjectDocument {
    /// The format this document is written as, which is the version this build writes.
    pub const fn format(&self) -> FormatVersion {
        FormatVersion::CURRENT
    }

    /// What the project is called, and which project it is.
    pub fn identity(&self) -> &ProjectIdentity {
        &self.identity
    }

    /// The pages, in document order.
    pub fn pages(&self) -> &[PageEntry] {
        &self.pages
    }

    /// The resources, in document order.
    pub fn resources(&self) -> &[ResourceEntry] {
        &self.resources
    }

    /// The target profiles, in document order.
    pub fn targets(&self) -> &[TargetProfile] {
        &self.targets
    }

    /// The generated files, in document order.
    pub fn generated(&self) -> &[GeneratedTarget] {
        &self.generated
    }

    /// The entry a target starts with, when one is chosen.
    ///
    /// `None` is a real state, not a missing default: a project can have pages and nothing to start
    /// yet, and version 2 says so with `null` rather than by leaving the member out.
    pub fn startup(&self) -> Option<&Reference> {
        self.startup.as_ref()
    }

    /// Which entry starts, as the document writes it.
    pub fn startup_written(&self) -> Option<&str> {
        self.startup.as_ref().map(Reference::written)
    }

    /// The network services the project declares, in document order.
    pub fn services(&self) -> &[ServiceEntry] {
        &self.services
    }

    /// Every member this build did not know, wherever it was in the document.
    ///
    /// Reported rather than dropped: a person can see that their file holds something this build
    /// does not understand, and saving it keeps it.
    pub fn unknown_fields(&self) -> &[UnknownField] {
        &self.unknown
    }

    /// What interpretation left over at the top level.
    pub fn preserved(&self) -> &Preserved {
        &self.preserved
    }

    /// Every path in the document, in document order, with what resolving it found.
    pub fn references(&self) -> Vec<&Reference> {
        let mut out = Vec::new();
        for page in &self.pages {
            out.push(&page.document);
            if let Some(stylesheet) = &page.stylesheet {
                out.push(stylesheet);
            }
        }
        for resource in &self.resources {
            out.push(&resource.path);
        }
        for generated in &self.generated {
            out.push(&generated.output);
            out.push(&generated.from);
        }
        if let Some(startup) = &self.startup {
            out.push(startup);
        }
        for service in &self.services {
            if let Some(roots) = &service.roots_file {
                out.push(roots);
            }
        }
        out
    }

    /// The references that would leave the project root.
    pub fn refused_references(&self) -> Vec<&Reference> {
        self.references()
            .into_iter()
            .filter(|reference| reference.is_refused())
            .collect()
    }

    /// Chooses the entry a target starts with, or clears the choice.
    ///
    /// The entry has to be one the document declares, which is the same rule the reader applies: a
    /// document this build produces can always be read back by this build.
    pub fn with_startup(
        &self,
        root: &ProjectRoot,
        written: Option<&str>,
    ) -> Result<Self, ProjectError> {
        let mut document = self.clone();
        document.startup = match written {
            None => None,
            Some(written) => {
                let reference = reference(root, ReferenceKind::PageDocument, written.to_owned());
                check_startup(&document.pages, Some(&reference)).map_err(|error| {
                    ProjectError::Dangling {
                        path: "startup".to_owned(),
                        name: error.message(),
                    }
                })?;
                Some(reference)
            }
        };
        Ok(document)
    }

    /// Who points at an entry: what would break if it were removed.
    pub fn references_to(&self, entry: &PageEntry) -> EntryReferences {
        let target = entry.document();
        EntryReferences {
            generated: self
                .generated
                .iter()
                .filter(|generated| generated.from().resolves_like(target))
                .map(|generated| generated.output().written().to_owned())
                .collect(),
            startup: self
                .startup
                .as_ref()
                .is_some_and(|startup| startup.resolves_like(target)),
        }
    }

    /// Removes the page at an index, fixing what pointed at it or refusing when anything does.
    ///
    /// This is the acceptance of T02.04: an entry other things point at is **not** removed quietly.
    /// With [`Resolution::Block`] the removal is refused and the caller is told what still points at
    /// it; with [`Resolution::FixReferences`] the references are fixed — the generated mappings from
    /// that entry go, and the startup choice is cleared — and what was fixed is reported, so nothing
    /// disappears without a record.
    ///
    /// The result is checked with the reader's own rules before it is returned: a document this
    /// method produced is one this build can read back.
    pub fn remove_entry(
        &self,
        index: usize,
        resolution: Resolution,
    ) -> Result<(Self, Vec<String>), DeleteRefusal> {
        let Some(entry) = self.pages.get(index) else {
            return Err(DeleteRefusal::NoSuchEntry {
                index,
                known: self.pages.len(),
            });
        };
        let written = entry.document().written().to_owned();
        let references = self.references_to(entry);
        if !references.is_empty() && resolution == Resolution::Block {
            return Err(DeleteRefusal::Referenced {
                document: written,
                references,
            });
        }

        let target = entry.document().clone();
        let mut document = self.clone();
        document.pages.remove(index);
        let mut fixed = Vec::new();
        let generated: Vec<GeneratedTarget> = document
            .generated
            .iter()
            .filter(|generated| !generated.from().resolves_like(&target))
            .cloned()
            .collect();
        for removed in document
            .generated
            .iter()
            .filter(|generated| generated.from().resolves_like(&target))
        {
            fixed.push(format!(
                "the generated file {} no longer comes from {written}",
                removed.output().written()
            ));
        }
        document.generated = generated;
        if document
            .startup
            .as_ref()
            .is_some_and(|startup| startup.resolves_like(&target))
        {
            document.startup = None;
            fixed.push(format!(
                "{written} is no longer the entry the target starts with"
            ));
        }
        check_startup(&document.pages, document.startup.as_ref()).map_err(|error| {
            DeleteRefusal::WouldNotRead {
                reason: error.message(),
            }
        })?;
        Ok((document, fixed))
    }

    /// Declares a file inside the project as one of its resources.
    pub fn with_resource(&self, root: &ProjectRoot, written: &str) -> Result<Self, ProjectError> {
        if self
            .resources
            .iter()
            .any(|resource| resource.path().written() == written)
        {
            return Err(ProjectError::Duplicate {
                path: "resources".to_owned(),
                claim: format!("the resource `{written}`"),
            });
        }
        let mut document = self.clone();
        document.resources.push(ResourceEntry {
            path: reference(root, ReferenceKind::Resource, written.to_owned()),
            preserved: Preserved::default(),
        });
        check_unique(&document.resources)?;
        Ok(document)
    }

    /// Stops declaring a resource, and says whether one was declared at all.
    ///
    /// The file itself is not touched: excluding a file from a project is a statement about the
    /// project, not about the disk.
    pub fn without_resource(&self, written: &str) -> (Self, bool) {
        let mut document = self.clone();
        let before = document.resources.len();
        document
            .resources
            .retain(|resource| resource.path().written() != written);
        let removed = document.resources.len() != before;
        (document, removed)
    }

    /// Rewrites every reference that names one path so that it names another.
    ///
    /// This is the reference half of moving a file: the document is edited where it pointed at the
    /// old path and nowhere else, and every rewrite is reported by name, so nothing changes silently.
    /// The result is checked with the reader's own rules before it is returned.
    pub fn rewire(
        &self,
        root: &ProjectRoot,
        from: &str,
        to: &str,
    ) -> Result<(Self, Vec<String>), ProjectError> {
        let replace = |named: &Reference, what: &str, rewrites: &mut Vec<String>| {
            if named.written() != from {
                return named.clone();
            }
            rewrites.push(format!("{what} now names `{to}`"));
            reference(root, named.kind(), to.to_owned())
        };
        let mut rewrites = Vec::new();
        let mut document = self.clone();
        document.pages = document
            .pages
            .iter()
            .map(|page| PageEntry {
                title: page.title().map(str::to_owned),
                document: replace(page.document(), "the page document", &mut rewrites),
                stylesheet: page
                    .stylesheet()
                    .map(|sheet| replace(sheet, "a page stylesheet", &mut rewrites)),
                preserved: page.preserved().clone(),
            })
            .collect();
        document.resources = document
            .resources
            .iter()
            .map(|resource| ResourceEntry {
                path: replace(resource.path(), "a resource", &mut rewrites),
                preserved: resource.preserved().clone(),
            })
            .collect();
        document.generated = document
            .generated
            .iter()
            .map(|generated| GeneratedTarget {
                output: replace(generated.output(), "a generated file", &mut rewrites),
                from: replace(generated.from(), "a generated file's source", &mut rewrites),
                preserved: generated.preserved().clone(),
            })
            .collect();
        document.startup = document
            .startup
            .as_ref()
            .map(|startup| replace(startup, "the startup entry", &mut rewrites));
        check_unique(&document.pages)?;
        check_unique(&document.resources)?;
        check_unique(&document.targets)?;
        check_generated_sources(&document.pages, &document.generated)?;
        check_startup(&document.pages, document.startup.as_ref())?;
        Ok((document, rewrites))
    }

    /// The document as JSON, with the members this build did not know put back where they were.
    ///
    /// Writing is a separate decision from rendering: [`ProjectFile::writer`] is what refuses a
    /// document this build may not put on disk.
    pub fn to_json(&self) -> Json {
        let mut out = JsonObject::new();
        out.insert(
            "format",
            Json::number(u64::from(FormatVersion::CURRENT.get())),
        );
        out.insert("project", encode_identity(&self.identity));
        out.insert(
            "pages",
            Json::array(self.pages.iter().map(encode_page).collect::<Vec<_>>()),
        );
        out.insert(
            "resources",
            Json::array(
                self.resources
                    .iter()
                    .map(encode_resource)
                    .collect::<Vec<_>>(),
            ),
        );
        out.insert(
            "targets",
            Json::array(self.targets.iter().map(encode_target).collect::<Vec<_>>()),
        );
        out.insert(
            "generated",
            Json::array(
                self.generated
                    .iter()
                    .map(encode_generated)
                    .collect::<Vec<_>>(),
            ),
        );
        out.insert(
            "startup",
            match &self.startup {
                Some(reference) => Json::string(reference.written()),
                None => Json::Null,
            },
        );
        let mut network = JsonObject::new();
        network.insert(
            "services",
            Json::array(self.services.iter().map(encode_service).collect::<Vec<_>>()),
        );
        self.network_preserved.adopt(&mut network);
        out.insert("network", Json::Object(network));
        self.preserved.adopt(&mut out);
        Json::Object(out)
    }

    /// The document as text.
    pub fn render(&self) -> String {
        self.to_json().render()
    }
}

/// Who points at an entry that is about to be removed.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct EntryReferences {
    generated: Vec<String>,
    startup: bool,
}

impl EntryReferences {
    /// The generated files that come from this entry.
    pub fn generated(&self) -> &[String] {
        &self.generated
    }

    /// Whether the target starts with this entry.
    pub const fn startup(&self) -> bool {
        self.startup
    }

    /// Whether anything points at it at all.
    pub fn is_empty(&self) -> bool {
        self.generated.is_empty() && !self.startup
    }

    /// How many things point at it.
    pub fn len(&self) -> usize {
        self.generated.len() + usize::from(self.startup)
    }

    /// One line per reference, for a report.
    pub fn lines(&self) -> Vec<String> {
        let mut lines: Vec<String> = self
            .generated
            .iter()
            .map(|output| format!("generated[{output}] comes from this entry"))
            .collect();
        if self.startup {
            lines.push("the target's startup entry is this one".to_owned());
        }
        lines
    }
}

/// What to do with an entry that other things still point at.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Resolution {
    /// Refuse the removal and say what still points at it: the caller decides what happens to those.
    Block,
    /// Remove it and fix what pointed at it, reporting every reference that was rewritten.
    FixReferences,
}

/// Why an entry was not removed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum DeleteRefusal {
    /// No page is there at that index.
    NoSuchEntry {
        /// The index that was asked for.
        index: usize,
        /// How many pages the document has.
        known: usize,
    },
    /// Something still points at it, and the caller asked to be blocked rather than to fix them.
    Referenced {
        /// The entry, as the document writes it.
        document: String,
        /// What points at it.
        references: EntryReferences,
    },
    /// Removing it would leave a document this build could not read back.
    WouldNotRead {
        /// Why.
        reason: String,
    },
}

impl DeleteRefusal {
    /// A sentence naming what was found, for a diagnostic or a terminal.
    pub fn message(&self) -> String {
        match self {
            Self::NoSuchEntry { index, known } => {
                format!("there is no page at index {index} (the document declares {known})")
            }
            Self::Referenced {
                document,
                references,
            } => {
                let mut message = format!(
                    "{document} is still pointed at by {} thing(s); nothing was removed. \
                     Fix those references, or ask for them to be fixed:",
                    references.len()
                );
                for line in references.lines() {
                    message.push_str("\n  ");
                    message.push_str(&line);
                }
                message
            }
            Self::WouldNotRead { reason } => format!(
                "removing it would leave a document this build could not read back: {reason}"
            ),
        }
    }
}

impl fmt::Display for DeleteRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.message())
    }
}

/// A project whose format version this build does not know.
///
/// Only the identity is read, and only when it is still where version 1 put it; everything else is
/// left alone. Nothing here can be written, because there is no rendering for it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ForeignProject {
    version: FormatVersion,
    identity: Option<ProjectIdentity>,
    members: Vec<String>,
    unknown: Vec<UnknownField>,
}

impl ForeignProject {
    /// The version the document declares.
    pub const fn version(&self) -> FormatVersion {
        self.version
    }

    /// The identity, when it could be read without interpreting the rest.
    ///
    /// Best effort by definition: it is read from a layout this build does not know, so a caller
    /// that shows it has to say so.
    pub fn identity(&self) -> Option<&ProjectIdentity> {
        self.identity.as_ref()
    }

    /// The members the document has, in the order they appear.
    pub fn members(&self) -> &[String] {
        &self.members
    }

    /// The members that were not interpreted: all of them but the version, and the identity when it
    /// could be read.
    pub fn unknown_fields(&self) -> &[UnknownField] {
        &self.unknown
    }
}

/// What a project file holds.
///
/// The interpreted variant is much larger than the other one, and deliberately not boxed: a document
/// is built once per opened file and moved a handful of times, so an allocation on the path every
/// read takes would buy nothing.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ProjectContent {
    /// A version this build knows, member by member.
    Interpreted(ProjectDocument),
    /// A version this build does not know.
    Foreign(ForeignProject),
}

impl ProjectContent {
    /// The version the document declares.
    pub const fn version(&self) -> FormatVersion {
        match self {
            Self::Interpreted(_) => FormatVersion::CURRENT,
            Self::Foreign(foreign) => foreign.version,
        }
    }

    /// The identity, when it could be read.
    pub fn identity(&self) -> Option<&ProjectIdentity> {
        match self {
            Self::Interpreted(document) => Some(document.identity()),
            Self::Foreign(foreign) => foreign.identity(),
        }
    }

    /// The document, when this build knows the version.
    pub fn document(&self) -> Option<&ProjectDocument> {
        match self {
            Self::Interpreted(document) => Some(document),
            Self::Foreign(_) => None,
        }
    }

    /// The foreign project, when this build does not know the version.
    pub fn foreign(&self) -> Option<&ForeignProject> {
        match self {
            Self::Interpreted(_) => None,
            Self::Foreign(foreign) => Some(foreign),
        }
    }

    /// Every member this build did not know.
    pub fn unknown_fields(&self) -> &[UnknownField] {
        match self {
            Self::Interpreted(document) => document.unknown_fields(),
            Self::Foreign(foreign) => foreign.unknown_fields(),
        }
    }

    /// What this build makes of the version.
    pub fn verdict(&self) -> VersionVerdict {
        let version = self.version();
        match version.get().cmp(&FormatVersion::CURRENT.get()) {
            Ordering::Greater => VersionVerdict::Newer {
                found: version,
                current: FormatVersion::CURRENT,
            },
            _ => VersionVerdict::Current { version },
        }
    }
}

/// A project file that was read, with what it came from.
#[derive(Clone, Debug)]
pub struct ProjectFile {
    path: PathBuf,
    root: ProjectRoot,
    original: Vec<u8>,
    content: ProjectContent,
    /// The version the document was read as, when it is older than the one this build writes.
    migrated_from: Option<FormatVersion>,
}

impl ProjectFile {
    /// Opens the project file at a path, with the directory it is in as the project root.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ProjectError> {
        let path = path.as_ref();
        let directory = match path.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
            _ => PathBuf::from("."),
        };
        let root = ProjectRoot::new(&directory).map_err(|error| ProjectError::Io {
            path: path.to_path_buf(),
            action: "anchoring the project root",
            reason: error.message(),
        })?;
        let name = path.file_name().ok_or_else(|| ProjectError::Io {
            path: path.to_path_buf(),
            action: "reading the project file",
            reason: "the path names no file".to_owned(),
        })?;
        Self::read_in(&root, Path::new(name))
    }

    /// Opens a project file inside a root, by a path relative to it.
    pub fn read_in(root: &ProjectRoot, relative: impl AsRef<Path>) -> Result<Self, ProjectError> {
        let relative = relative.as_ref();
        let path = root.resolve(relative).map_err(|error| ProjectError::Io {
            path: root.as_path().join(relative),
            action: "resolving the project file",
            reason: error.message(),
        })?;
        let bytes = std::fs::read(&path).map_err(|error| ProjectError::Io {
            path: path.clone(),
            action: "reading",
            reason: error.to_string(),
        })?;
        Self::from_bytes(root, path, bytes)
    }

    /// Reads a project file from text that was already obtained, for tests and for tools.
    pub fn parse(
        root: &ProjectRoot,
        path: impl Into<PathBuf>,
        text: &str,
    ) -> Result<Self, ProjectError> {
        Self::from_bytes(root, path.into(), text.as_bytes().to_vec())
    }

    fn from_bytes(root: &ProjectRoot, path: PathBuf, bytes: Vec<u8>) -> Result<Self, ProjectError> {
        let text = String::from_utf8(bytes.clone()).map_err(|error| ProjectError::Io {
            path: path.clone(),
            action: "reading",
            reason: format!(
                "it is not UTF-8 text, so it is not a project file ({})",
                error.utf8_error()
            ),
        })?;
        let value = json::parse(&text).map_err(|error| ProjectError::NotJson {
            path: path.clone(),
            error,
        })?;
        let declared = format_version(&value)?;
        let content = read(root, &value)?;
        let migrated_from = (declared < FormatVersion::CURRENT).then_some(declared);
        Ok(Self {
            path,
            root: root.clone(),
            original: bytes,
            content,
            migrated_from,
        })
    }

    /// The file this was read from.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The directory every path in the document is relative to.
    pub fn root(&self) -> &ProjectRoot {
        &self.root
    }

    /// What the file holds.
    pub const fn content(&self) -> &ProjectContent {
        &self.content
    }

    /// The bytes that were read.
    ///
    /// Exposed because two things have to be able to compare themselves with what was read: the
    /// conflict check inside a write, and a backup that claims to be the record of this file.
    pub fn original(&self) -> &[u8] {
        &self.original
    }

    /// The version the document declares.
    pub const fn version(&self) -> FormatVersion {
        self.content.version()
    }

    /// What this build makes of the version.
    pub fn verdict(&self) -> VersionVerdict {
        match self.migrated_from {
            Some(found) => VersionVerdict::Migrated {
                found,
                current: FormatVersion::CURRENT,
                steps: migration_path(found)
                    .map(|steps| steps.len())
                    .unwrap_or_default(),
            },
            None => self.content.verdict(),
        }
    }

    /// The version the document was read as, when it is older than the one this build writes.
    pub const fn migrated_from(&self) -> Option<FormatVersion> {
        self.migrated_from
    }

    /// The document as it was read, when the version is one this build interprets.
    pub fn document(&self) -> Option<&ProjectDocument> {
        self.content.document()
    }

    /// Writes a document of this project, under the same rules a save follows.
    ///
    /// The conflict check, the atomic write and the migration backup are one implementation: a
    /// caller that changed the document (removing an entry, T02.04) puts the result in place through
    /// the same path a plain save uses, so it cannot skip any of them.
    pub fn write_document(&self, document: &ProjectDocument) -> Result<Written, ProjectError> {
        let backup = match self.migrated_from {
            Some(from) => {
                let backup = self.path.with_extension(format!("v{}.bak", from.get()));
                std::fs::write(&backup, &self.original).map_err(|error| ProjectError::Io {
                    path: backup.clone(),
                    action: "keeping the file that was read",
                    reason: error.to_string(),
                })?;
                Some(backup)
            }
            None => None,
        };
        let target = self.write(document, &self.path)?;
        Ok(Written {
            target,
            backup,
            migrated_from: self.migrated_from,
        })
    }

    /// The one place a project document is put on disk.
    fn write(&self, document: &ProjectDocument, path: &Path) -> Result<PathBuf, ProjectError> {
        if path == self.path {
            let current = std::fs::read(path).map_err(|error| ProjectError::Io {
                path: path.to_path_buf(),
                action: "reading before writing",
                reason: error.to_string(),
            })?;
            if current != self.original {
                return Err(ProjectError::ChangedOnDisk {
                    path: path.to_path_buf(),
                });
            }
        }
        json::write_atomically(path, &document.render()).map_err(|error| ProjectError::Io {
            path: path.to_path_buf(),
            action: "writing",
            reason: error.to_string(),
        })
    }

    /// Every member this build did not know.
    pub fn unknown_fields(&self) -> &[UnknownField] {
        self.content.unknown_fields()
    }

    /// Whether this build may write the file.
    pub fn is_writable(&self) -> bool {
        self.writer().is_ok()
    }

    /// A writer for the file, when this build may write it.
    ///
    /// The two refusals are the version policy (this build never writes a version it does not know,
    /// decision `U1`) and the path rule (a project file may only point inside its own root, decision
    /// `F1`). Both make writing impossible rather than discouraged: without a [`ProjectWriter`] there
    /// is no save path to call.
    pub fn writer(&self) -> Result<ProjectWriter<'_>, ProjectWriteRefusal> {
        match &self.content {
            ProjectContent::Foreign(foreign) => Err(ProjectWriteRefusal::UnknownFormat {
                found: foreign.version,
                current: FormatVersion::CURRENT,
            }),
            ProjectContent::Interpreted(document) => {
                let refused = document.refused_references();
                if let Some(first) = refused.first() {
                    return Err(ProjectWriteRefusal::RefusedReferences {
                        count: refused.len(),
                        first: first.written().to_owned(),
                    });
                }
                Ok(ProjectWriter {
                    file: self,
                    document,
                    migrated_from: self.migrated_from,
                })
            }
        }
    }
}

/// Writing a project file back, under the rules that allow it.
#[derive(Debug)]
pub struct ProjectWriter<'a> {
    file: &'a ProjectFile,
    document: &'a ProjectDocument,
    migrated_from: Option<FormatVersion>,
}

impl ProjectWriter<'_> {
    /// The document as it would be written, unknown members included.
    pub fn render(&self) -> String {
        self.document.render()
    }

    /// Writes the document back where it came from.
    ///
    /// A writer is a ticket for one write of what was read: it refuses when the file changed on disk
    /// in the meantime, so an editor's or a script's change is never overwritten by a stale copy.
    pub fn save(&self) -> Result<PathBuf, ProjectError> {
        if let Some(from) = self.migrated_from {
            return Err(ProjectError::MigrationNeedsBackup {
                path: self.file.path.clone(),
                from,
                to: FormatVersion::CURRENT,
            });
        }
        self.save_to(self.file.path.clone())
    }

    /// Writes a migrated document, keeping the file that was read first.
    ///
    /// Decision `Q6`: a migration never overwrites the original in place before it is done. The
    /// document as it was read is written beside it under its own version's name, and only then is
    /// the migrated document put in place — so a migration that goes wrong leaves the version a
    /// person could still read.
    pub fn save_migrating(&self) -> Result<MigrationSave, ProjectError> {
        let Some(from) = self.migrated_from else {
            return Err(ProjectError::NothingToMigrate {
                path: self.file.path.clone(),
            });
        };
        let backup = self
            .file
            .path
            .with_extension(format!("v{}.bak", from.get()));
        std::fs::write(&backup, &self.file.original).map_err(|error| ProjectError::Io {
            path: backup.clone(),
            action: "keeping the file that was read",
            reason: error.to_string(),
        })?;
        let target = self.save_to(self.file.path.clone())?;
        Ok(MigrationSave {
            target,
            backup,
            from,
            to: FormatVersion::CURRENT,
        })
    }

    /// Writes the document to a path of the caller's choosing.
    ///
    /// The write goes to a temporary file in the target directory and is then renamed over the
    /// target, which is the save path the roadmap asks for: prepare, write, commit, confirm. A
    /// failure in the middle leaves the target as it was.
    pub fn save_to(&self, path: impl AsRef<Path>) -> Result<PathBuf, ProjectError> {
        self.file.write(self.document, path.as_ref())
    }
}

/// What writing a document did.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Written {
    target: PathBuf,
    backup: Option<PathBuf>,
    migrated_from: Option<FormatVersion>,
}

impl Written {
    /// The file that was written.
    pub fn target(&self) -> &Path {
        &self.target
    }

    /// The file that was kept before a migration, when one happened.
    pub fn backup(&self) -> Option<&Path> {
        self.backup.as_deref()
    }

    /// The version the document was read as, when it was older than the one this build writes.
    pub const fn migrated_from(&self) -> Option<FormatVersion> {
        self.migrated_from
    }

    /// One sentence for a person.
    pub fn message(&self) -> String {
        match &self.backup {
            Some(backup) => format!(
                "wrote {} (the {from} document is kept at {backup})",
                self.target.display(),
                from = self
                    .migrated_from
                    .map(|version| version.to_string())
                    .unwrap_or_else(|| "older".to_owned()),
                backup = backup.display()
            ),
            None => format!("wrote {}", self.target.display()),
        }
    }
}

/// What a migrating save did: the file that was kept, and the file that was put in place.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MigrationSave {
    target: PathBuf,
    backup: PathBuf,
    from: FormatVersion,
    to: FormatVersion,
}

impl MigrationSave {
    /// The migrated document's path.
    pub fn target(&self) -> &Path {
        &self.target
    }

    /// The file that was kept, which holds the document as it was read.
    pub fn backup(&self) -> &Path {
        &self.backup
    }

    /// The version the document was read as.
    pub const fn from(&self) -> FormatVersion {
        self.from
    }

    /// The version it was written as.
    pub const fn to(&self) -> FormatVersion {
        self.to
    }

    /// One sentence for a person: what moved where.
    pub fn message(&self) -> String {
        format!(
            "migrated {} to {}: the {from} document is kept at {backup}",
            self.target.display(),
            self.to,
            from = self.from,
            backup = self.backup.display()
        )
    }
}

/// Reads a document that was already parsed, and says what this build makes of it.
pub fn read(root: &ProjectRoot, value: &Json) -> Result<ProjectContent, ProjectError> {
    let version = format_version(value)?;
    match version.get().cmp(&FormatVersion::CURRENT.get()) {
        Ordering::Equal => Ok(ProjectContent::Interpreted(interpret(
            root,
            value,
            FormatVersion::CURRENT,
        )?)),
        Ordering::Greater => Ok(ProjectContent::Foreign(foreign(version, value))),
        Ordering::Less => {
            // Decision `Q6`: an older version is migrated by versioned steps before it is used, and a
            // version with no step out of it is refused rather than guessed at.
            migration_path(version).map_err(|stuck| ProjectError::OlderFormat {
                found: stuck,
                current: FormatVersion::CURRENT,
            })?;
            Ok(ProjectContent::Interpreted(interpret(
                root, value, version,
            )?))
        }
    }
}

pub(crate) fn format_version(value: &Json) -> Result<FormatVersion, ProjectError> {
    let Some(object) = value.as_object() else {
        return Err(ProjectError::WrongType {
            path: "the document".to_owned(),
            expected: "an object",
            found: value.kind(),
        });
    };
    let Some(member) = object.get("format") else {
        return Err(ProjectError::MissingMember {
            path: "format".to_owned(),
        });
    };
    let Some(number) = member.as_u64() else {
        return Err(ProjectError::WrongType {
            path: "format".to_owned(),
            expected: "a whole number",
            found: member.kind(),
        });
    };
    let version = u32::try_from(number).map_err(|_| ProjectError::WrongType {
        path: "format".to_owned(),
        expected: "a whole number that fits in 32 bits",
        found: JsonKind::Number,
    })?;
    Ok(FormatVersion::from_raw(version))
}

/// A version this build does not know: the identity is read, nothing else is interpreted.
fn foreign(version: FormatVersion, value: &Json) -> ForeignProject {
    let mut members = Vec::new();
    let mut unknown = Vec::new();
    let mut identity = None;
    if let Some(object) = value.as_object() {
        for name in object.names() {
            members.push(name.to_owned());
        }
        identity = object.get("project").and_then(best_effort_identity);
        for name in object.names() {
            if name == "format" {
                continue;
            }
            if name == "project" && identity.is_some() {
                continue;
            }
            unknown.push(UnknownField::member("", name));
        }
    }
    ForeignProject {
        version,
        identity,
        members,
        unknown,
    }
}

/// An identity read from a layout this build does not know.
///
/// Only what a person needs to recognise the project, and only when it is unambiguous: both members
/// present, both strings, neither empty.
fn best_effort_identity(value: &Json) -> Option<ProjectIdentity> {
    let object = value.as_object()?;
    let id = object.get("id")?.as_str()?;
    let name = object.get("name")?.as_str()?;
    if id.is_empty() || name.is_empty() {
        return None;
    }
    Some(ProjectIdentity {
        id: id.to_owned(),
        name: name.to_owned(),
        preserved: Preserved::default(),
    })
}

fn interpret(
    root: &ProjectRoot,
    value: &Json,
    declared: FormatVersion,
) -> Result<ProjectDocument, ProjectError> {
    let context = Context { root };
    let mut reader = Reader::object(String::new(), value)?;
    // The version was read before the dispatch; taking it here is what stops it being reported as a
    // member this build does not know.
    let _ = reader.take("format");
    let identity = decode_identity(&mut reader)?;
    let pages = decode_pages(&context, &mut reader)?;
    let resources = decode_resources(&context, &mut reader)?;
    let targets = decode_targets(&mut reader)?;
    let generated = decode_generated(&context, &mut reader)?;
    let startup = decode_startup(&context, &mut reader, declared)?;
    let (services, network_preserved) = decode_services(&context, &mut reader)?;
    let preserved = reader.finish();
    check_unique(&pages)?;
    check_unique(&resources)?;
    check_unique(&targets)?;
    check_generated_sources(&pages, &generated)?;
    check_startup(&pages, startup.as_ref())?;

    let mut unknown = Vec::new();
    preserved.names_at("", &mut unknown);
    identity.preserved.names_at("project", &mut unknown);
    for (index, page) in pages.iter().enumerate() {
        page.preserved
            .names_at(&format!("pages[{index}]"), &mut unknown);
    }
    for (index, resource) in resources.iter().enumerate() {
        resource
            .preserved
            .names_at(&format!("resources[{index}]"), &mut unknown);
    }
    for (index, target) in targets.iter().enumerate() {
        target
            .preserved
            .names_at(&format!("targets[{index}]"), &mut unknown);
    }
    for (index, entry) in generated.iter().enumerate() {
        entry
            .preserved
            .names_at(&format!("generated[{index}]"), &mut unknown);
    }
    network_preserved.names_at("network", &mut unknown);
    for (index, service) in services.iter().enumerate() {
        service
            .preserved
            .names_at(&format!("network.services[{index}]"), &mut unknown);
        service
            .tls_preserved
            .names_at(&format!("network.services[{index}].tls"), &mut unknown);
    }

    Ok(ProjectDocument {
        identity,
        pages,
        resources,
        targets,
        generated,
        startup,
        services,
        network_preserved,
        preserved,
        unknown,
    })
}

/// What a reader needs to turn a written path into a place.
struct Context<'a> {
    root: &'a ProjectRoot,
}

impl Context<'_> {
    fn reference(&self, kind: ReferenceKind, written: String) -> Reference {
        reference(self.root, kind, written)
    }
}

/// Resolves a written path against a project root, and says what it found.
///
/// One definition for both documents of the format: the project document and a page document must
/// agree about what "inside the project" means, and two copies of this rule would drift.
pub(crate) fn reference(root: &ProjectRoot, kind: ReferenceKind, written: String) -> Reference {
    match root.resolve(&written) {
        Ok(path) => {
            let verdict = if path.exists() {
                ReferenceVerdict::Present
            } else {
                ReferenceVerdict::Absent
            };
            Reference {
                kind,
                written,
                resolved: Some(path),
                verdict,
            }
        }
        // A parent that is not there yet is "not there yet", not a refusal: a resource in a
        // directory nobody has created is a normal state of a project being built up. Every other
        // reason — an absolute path, `..`, a link that leaves the root, an empty path — means the
        // reference does not belong to this project.
        Err(PathError::MissingParent { path }) => Reference {
            kind,
            written,
            resolved: Some(path),
            verdict: ReferenceVerdict::Absent,
        },
        Err(error) => Reference {
            kind,
            written,
            resolved: None,
            verdict: ReferenceVerdict::Refused {
                reason: error.message(),
            },
        },
    }
}

fn decode_identity(reader: &mut Reader<'_>) -> Result<ProjectIdentity, ProjectError> {
    let Some(mut member) = sub_object(reader, "project")? else {
        return Err(ProjectError::MissingMember {
            path: "project".to_owned(),
        });
    };
    let id = member.required_string("id")?;
    let name = member.required_string("name")?;
    Ok(ProjectIdentity {
        id,
        name,
        preserved: member.finish(),
    })
}

fn decode_pages(
    context: &Context<'_>,
    reader: &mut Reader<'_>,
) -> Result<Vec<PageEntry>, ProjectError> {
    let mut pages = Vec::new();
    for mut entry in sub_objects(reader, "pages")? {
        let document = entry
            .required_string("document")
            .map(|written| context.reference(ReferenceKind::PageDocument, written))?;
        let title = entry.take_string("title")?;
        let stylesheet = entry
            .take_string("stylesheet")?
            .map(|written| context.reference(ReferenceKind::Stylesheet, written));
        let preserved = entry.finish();
        pages.push(PageEntry {
            title,
            document,
            stylesheet,
            preserved,
        });
    }
    Ok(pages)
}

fn decode_resources(
    context: &Context<'_>,
    reader: &mut Reader<'_>,
) -> Result<Vec<ResourceEntry>, ProjectError> {
    let mut resources = Vec::new();
    for mut entry in sub_objects(reader, "resources")? {
        let path = entry
            .required_string("path")
            .map(|written| context.reference(ReferenceKind::Resource, written))?;
        let preserved = entry.finish();
        resources.push(ResourceEntry { path, preserved });
    }
    Ok(resources)
}

fn decode_targets(reader: &mut Reader<'_>) -> Result<Vec<TargetProfile>, ProjectError> {
    let mut targets = Vec::new();
    for mut entry in sub_objects(reader, "targets")? {
        let name = entry.required_string("name")?;
        let profile = entry.take_string("profile")?;
        let features = entry.take_string_array("features")?;
        let preserved = entry.finish();
        targets.push(TargetProfile {
            name,
            profile,
            features,
            preserved,
        });
    }
    Ok(targets)
}

fn decode_generated(
    context: &Context<'_>,
    reader: &mut Reader<'_>,
) -> Result<Vec<GeneratedTarget>, ProjectError> {
    let mut generated = Vec::new();
    for mut entry in sub_objects(reader, "generated")? {
        let output = entry
            .required_string("output")
            .map(|written| context.reference(ReferenceKind::GeneratedOutput, written))?;
        let from = entry
            .required_string("from")
            .map(|written| context.reference(ReferenceKind::DesignSource, written))?;
        let preserved = entry.finish();
        generated.push(GeneratedTarget {
            output,
            from,
            preserved,
        });
    }
    Ok(generated)
}

fn decode_services(
    context: &Context<'_>,
    reader: &mut Reader<'_>,
) -> Result<(Vec<ServiceEntry>, Preserved), ProjectError> {
    let Some(mut network) = sub_object(reader, "network")? else {
        return Ok((Vec::new(), Preserved::default()));
    };
    let mut services = Vec::new();
    for mut entry in sub_objects(&mut network, "services")? {
        let name = entry.required_string("name")?;
        let host = entry.required_string("host")?;
        let port = entry.required_u16("port")?;
        let credential = match entry.take("credential") {
            None => None,
            Some(value) => {
                let path = entry.member_path("credential");
                Some(decode_credential(value, &path)?)
            }
        };
        let (tls, roots_file, tls_preserved) = decode_tls(context, &mut entry)?;
        let preserved = entry.finish();
        let mut policy = ServicePolicy::new(name, host, port, tls);
        if let Some(credential) = credential {
            policy = policy.with_credential(credential);
        }
        services.push(ServiceEntry {
            policy,
            roots_file,
            tls_preserved,
            preserved,
        });
    }
    Ok((services, network.finish()))
}

/// The TLS configuration and the certificate file it adds, when it adds one.
fn decode_tls(
    context: &Context<'_>,
    entry: &mut Reader<'_>,
) -> Result<(TlsPolicy, Option<Reference>, Preserved), ProjectError> {
    let Some(mut member) = sub_object(entry, "tls")? else {
        return Ok((TlsPolicy::new(), None, Preserved::default()));
    };
    let roots = member.required_string("roots")?;
    let mut policy = TlsPolicy::new();
    let mut reference = None;
    match roots.as_str() {
        "system" => {}
        "system-and-file" => {
            let written = member.required_string("roots_file")?;
            let resolved = context.reference(ReferenceKind::CertificateAuthority, written);
            // A refused reference leaves the policy at the system's own anchors: the document is not
            // writable in that state anyway, and the alternative would be handing the host a path
            // that was never resolved against the root.
            if let Some(path) = resolved.resolved() {
                policy = policy.with_additional_roots(path.to_path_buf());
            }
            reference = Some(resolved);
        }
        _ => {
            return Err(ProjectError::WrongType {
                path: member.member_path("roots"),
                expected: "`system` or `system-and-file`",
                found: JsonKind::String,
            });
        }
    }
    if let Some(value) = member.take("client_certificate") {
        let path = member.member_path("client_certificate");
        let certificate = decode_credential(value, &path)?;
        policy = policy.with_client_certificate(certificate);
    }
    Ok((policy, reference, member.finish()))
}

fn decode_credential(value: &Json, path: &str) -> Result<CredentialRef, ProjectError> {
    let mut reader = Reader::object(path.to_owned(), value)?;
    reader.refuse_extra_members(&["name", "source", "expires_at_unix_ms"])?;
    let name = reader.required_string("name")?;
    let source = decode_credential_source(&mut reader)?;
    let expires_at = match reader.take_i64("expires_at_unix_ms")? {
        Some(milliseconds) => {
            Some(
                instant_from_millis(milliseconds).ok_or_else(|| ProjectError::WrongType {
                    path: reader.member_path("expires_at_unix_ms"),
                    expected: "a time this build can hold",
                    found: JsonKind::Number,
                })?,
            )
        }
        None => None,
    };
    let mut credential = CredentialRef::new(name, source);
    if let Some(at) = expires_at {
        credential = credential.expiring_at(at);
    }
    Ok(credential)
}

/// A project document being described before it exists: what creating a project needs (T02.02).
///
/// It is deliberately not an editing path. A draft starts from nothing, and what it produces is the
/// same kind of value a file reader produces, written by the same writer — so a created project
/// cannot be a second kind of file. Every path a draft is given resolves against the root exactly
/// the way a path read from a file does, and the integrity rules are the reader's own, so a draft
/// cannot describe a project that could not be read back.
pub struct ProjectDraft<'a> {
    root: &'a ProjectRoot,
    id: String,
    name: String,
    pages: Vec<PageEntry>,
    resources: Vec<ResourceEntry>,
    targets: Vec<TargetProfile>,
    generated: Vec<GeneratedTarget>,
    startup: Option<Reference>,
    services: Vec<ServiceEntry>,
}

impl<'a> ProjectDraft<'a> {
    /// A draft with nothing in it but its identity.
    pub fn new(root: &'a ProjectRoot, id: &str, name: &str) -> Result<Self, ProjectError> {
        if id.trim().is_empty() {
            return Err(ProjectError::EmptyValue {
                path: "project.id".to_owned(),
            });
        }
        if name.trim().is_empty() {
            return Err(ProjectError::EmptyValue {
                path: "project.name".to_owned(),
            });
        }
        Ok(Self {
            root,
            id: id.to_owned(),
            name: name.to_owned(),
            pages: Vec::new(),
            resources: Vec::new(),
            targets: Vec::new(),
            generated: Vec::new(),
            startup: None,
            services: Vec::new(),
        })
    }

    /// Adds a page, with the document it lives in and the stylesheet it uses.
    pub fn with_page(
        mut self,
        document: &str,
        title: Option<&str>,
        stylesheet: Option<&str>,
    ) -> Self {
        self.pages.push(PageEntry {
            title: title.map(str::to_owned),
            document: reference(self.root, ReferenceKind::PageDocument, document.to_owned()),
            stylesheet: stylesheet
                .map(|written| reference(self.root, ReferenceKind::Stylesheet, written.to_owned())),
            preserved: Preserved::default(),
        });
        self
    }

    /// Adds a resource to a project that is being created.
    ///
    /// A project that already exists is edited with [`ProjectDocument::with_resource`] instead,
    /// which can refuse: a draft cannot be wrong about a file it is the first to declare.
    pub fn with_resource(mut self, path: &str) -> Self {
        self.resources.push(ResourceEntry {
            path: reference(self.root, ReferenceKind::Resource, path.to_owned()),
            preserved: Preserved::default(),
        });
        self
    }

    /// Adds a target profile.
    pub fn with_target(mut self, name: &str, profile: Option<&str>, features: &[&str]) -> Self {
        self.targets.push(TargetProfile {
            name: name.to_owned(),
            profile: profile.map(str::to_owned),
            features: features
                .iter()
                .map(|feature| (*feature).to_owned())
                .collect(),
            preserved: Preserved::default(),
        });
        self
    }

    /// Adds the file a page is generated into.
    pub fn with_generated(mut self, output: &str, from: &str) -> Self {
        self.generated.push(GeneratedTarget {
            output: reference(self.root, ReferenceKind::GeneratedOutput, output.to_owned()),
            from: reference(self.root, ReferenceKind::DesignSource, from.to_owned()),
            preserved: Preserved::default(),
        });
        self
    }

    /// Chooses the entry a target starts with.
    pub fn with_startup(mut self, document: &str) -> Self {
        self.startup = Some(reference(
            self.root,
            ReferenceKind::PageDocument,
            document.to_owned(),
        ));
        self
    }

    /// The document, checked by the reader's own rules.
    pub fn build(self) -> Result<ProjectDocument, ProjectError> {
        check_unique(&self.pages)?;
        check_unique(&self.resources)?;
        check_unique(&self.targets)?;
        check_generated_sources(&self.pages, &self.generated)?;
        check_startup(&self.pages, self.startup.as_ref())?;
        Ok(ProjectDocument {
            identity: ProjectIdentity {
                id: self.id,
                name: self.name,
                preserved: Preserved::default(),
            },
            pages: self.pages,
            resources: self.resources,
            targets: self.targets,
            generated: self.generated,
            startup: self.startup,
            services: self.services,
            network_preserved: Preserved::default(),
            preserved: Preserved::default(),
            unknown: Vec::new(),
        })
    }
}

fn encode_identity(identity: &ProjectIdentity) -> Json {
    let mut out = JsonObject::new();
    out.insert("id", Json::string(identity.id()));
    out.insert("name", Json::string(identity.name()));
    identity.preserved.adopt(&mut out);
    Json::Object(out)
}

fn encode_page(page: &PageEntry) -> Json {
    let mut out = JsonObject::new();
    out.insert("document", Json::string(page.document.written()));
    if let Some(title) = page.title() {
        out.insert("title", Json::string(title));
    }
    if let Some(stylesheet) = &page.stylesheet {
        out.insert("stylesheet", Json::string(stylesheet.written()));
    }
    page.preserved.adopt(&mut out);
    Json::Object(out)
}

fn encode_resource(resource: &ResourceEntry) -> Json {
    let mut out = JsonObject::new();
    out.insert("path", Json::string(resource.path.written()));
    resource.preserved.adopt(&mut out);
    Json::Object(out)
}

fn encode_target(target: &TargetProfile) -> Json {
    let mut out = JsonObject::new();
    out.insert("name", Json::string(target.name()));
    if let Some(profile) = target.profile() {
        out.insert("profile", Json::string(profile));
    }
    out.insert(
        "features",
        Json::array(
            target
                .features()
                .iter()
                .map(Json::string)
                .collect::<Vec<_>>(),
        ),
    );
    target.preserved.adopt(&mut out);
    Json::Object(out)
}

fn encode_generated(entry: &GeneratedTarget) -> Json {
    let mut out = JsonObject::new();
    out.insert("output", Json::string(entry.output.written()));
    out.insert("from", Json::string(entry.from.written()));
    entry.preserved.adopt(&mut out);
    Json::Object(out)
}

/// A service: public configuration, and credentials by reference.
///
/// The member names here are the format's, and they have to stay in step with [`ServicePolicy`],
/// [`TlsPolicy`] and [`CredentialRef`]. The round-trip test builds a policy with every field set, so
/// a field added to those types without a member here fails that test rather than being silently
/// dropped from every project file.
fn encode_service(entry: &ServiceEntry) -> Json {
    let policy = &entry.policy;
    let mut out = JsonObject::new();
    out.insert("name", Json::string(policy.service()));
    out.insert("host", Json::string(policy.host()));
    out.insert("port", Json::number(u64::from(policy.port())));
    let mut tls = JsonObject::new();
    match policy.tls().roots() {
        CertificateSource::System => {
            tls.insert("roots", Json::string("system"));
        }
        CertificateSource::AdditionalFile(path) => {
            tls.insert("roots", Json::string("system-and-file"));
            let written = entry
                .roots_file
                .as_ref()
                .map(|reference| reference.written().to_owned())
                .unwrap_or_else(|| path.to_string_lossy().into_owned());
            tls.insert("roots_file", Json::string(written));
        }
    }
    if let Some(certificate) = policy.tls().client_certificate() {
        tls.insert("client_certificate", encode_credential(certificate));
    }
    // The TLS configuration's own unknown members are put back where they were.
    entry.tls_preserved.adopt(&mut tls);
    out.insert("tls", Json::Object(tls));
    if let Some(credential) = policy.credential() {
        out.insert("credential", encode_credential(credential));
    }
    entry.preserved.adopt(&mut out);
    Json::Object(out)
}

pub(crate) fn check_unique<T: Unique>(entries: &[T]) -> Result<(), ProjectError> {
    let mut seen = Vec::new();
    for entry in entries {
        let claim = entry.claim();
        let path = entry.claim_path();
        if seen.contains(&claim) {
            return Err(ProjectError::Duplicate { path, claim });
        }
        seen.push(claim);
    }
    Ok(())
}

/// Something that two entries may not both claim.
pub(crate) trait Unique {
    fn claim(&self) -> String;

    fn claim_path(&self) -> String;
}

impl Unique for PageEntry {
    fn claim(&self) -> String {
        self.document.written().to_owned()
    }

    fn claim_path(&self) -> String {
        format!("pages[].document `{}`", self.document.written())
    }
}

impl Unique for ResourceEntry {
    fn claim(&self) -> String {
        self.path.written().to_owned()
    }

    fn claim_path(&self) -> String {
        format!("resources[].path `{}`", self.path.written())
    }
}

impl Unique for TargetProfile {
    fn claim(&self) -> String {
        self.name.clone()
    }

    fn claim_path(&self) -> String {
        format!("targets[].name `{}`", self.name)
    }
}

/// The entry a target starts with: a reference to a page document, or `null` for "not chosen yet".
fn decode_startup(
    context: &Context<'_>,
    reader: &mut Reader<'_>,
    declared: FormatVersion,
) -> Result<Option<Reference>, ProjectError> {
    let path = reader.member_path("startup");
    let Some(value) = reader.take("startup") else {
        // A version 1 document has no such member, and its absence means the same as `null`:
        // nothing is chosen. That equivalence is what the version 1 -> 2 step in `MIGRATION_STEPS`
        // is: it needs no rewriting because the member was added as optional.
        return Ok(None);
    };
    if declared < FormatVersion::CURRENT {
        // The member version 2 added cannot be in a version 1 document: reading it would be believing
        // a file that contradicts its own version, and the migration step is about a document that
        // does not have it.
        return Err(ProjectError::Inconsistent {
            path,
            reason: format!(
                "{declared} does not define `startup`, so a document of that version cannot choose \
                 an entry"
            ),
        });
    }
    if value.kind() == JsonKind::Null {
        return Ok(None);
    }
    let Some(written) = value.as_str() else {
        return Err(ProjectError::WrongType {
            path,
            expected: "a path to a page document, or `null`",
            found: value.kind(),
        });
    };
    if written.is_empty() {
        return Err(ProjectError::EmptyValue { path });
    }
    Ok(Some(context.reference(
        ReferenceKind::PageDocument,
        written.to_owned(),
    )))
}

/// The startup entry has to be a page the document declares, or nothing may save it.
pub(crate) fn check_startup(
    pages: &[PageEntry],
    startup: Option<&Reference>,
) -> Result<(), ProjectError> {
    let Some(startup) = startup else {
        return Ok(());
    };
    let declared = pages
        .iter()
        .any(|page| page.document.resolves_like(startup));
    if declared {
        return Ok(());
    }
    Err(ProjectError::Dangling {
        path: "startup".to_owned(),
        name: startup.written().to_owned(),
    })
}

pub(crate) fn check_generated_sources(
    pages: &[PageEntry],
    generated: &[GeneratedTarget],
) -> Result<(), ProjectError> {
    for (index, entry) in generated.iter().enumerate() {
        let known = pages
            .iter()
            .any(|page| page.document.resolves_like(&entry.from));
        if !known {
            return Err(ProjectError::Dangling {
                path: format!("generated[{index}].from"),
                name: entry.from.written().to_owned(),
            });
        }
    }
    Ok(())
}

/// Reads one object, remembering which members were interpreted.
///
/// Shared with [`crate::project::page`]: both documents of the format read JSON the same way, and a
/// second reader would be a second opinion about what an unknown member is (principle #35).
pub(crate) struct Reader<'a> {
    path: String,
    members: Vec<(String, &'a Json)>,
}

impl<'a> Reader<'a> {
    pub(crate) fn object(path: impl Into<String>, value: &'a Json) -> Result<Self, ProjectError> {
        let path = path.into();
        let Some(object) = value.as_object() else {
            return Err(ProjectError::WrongType {
                path: if path.is_empty() {
                    "the document".to_owned()
                } else {
                    path
                },
                expected: "an object",
                found: value.kind(),
            });
        };
        Ok(Self {
            path,
            members: object
                .iter()
                .map(|(name, value)| (name.to_owned(), value))
                .collect(),
        })
    }

    pub(crate) fn path(&self) -> &str {
        &self.path
    }

    pub(crate) fn member_path(&self, name: &str) -> String {
        if self.path.is_empty() {
            name.to_owned()
        } else {
            format!("{}.{name}", self.path)
        }
    }

    pub(crate) fn entry_path(&self, name: &str, index: usize) -> String {
        format!("{}[{index}]", self.member_path(name))
    }

    /// The member names this object may have, and nothing else.
    ///
    /// Used where the set is closed on purpose: a credential reference may not carry a member
    /// nobody has seen before, because the member it would be carrying could be the material
    /// itself.
    fn refuse_extra_members(&self, allowed: &[&str]) -> Result<(), ProjectError> {
        let extra: Vec<String> = self
            .members
            .iter()
            .filter(|(name, _)| !allowed.contains(&name.as_str()))
            .map(|(name, _)| format!("`{name}`"))
            .collect();
        if extra.is_empty() {
            return Ok(());
        }
        Err(ProjectError::SecretInProject {
            path: self.path.clone(),
            members: extra,
        })
    }

    pub(crate) fn take(&mut self, name: &str) -> Option<&'a Json> {
        let index = self.members.iter().position(|(member, _)| member == name)?;
        Some(self.members.remove(index).1)
    }

    pub(crate) fn take_string(&mut self, name: &str) -> Result<Option<String>, ProjectError> {
        match self.take(name) {
            None => Ok(None),
            Some(value) => match value.as_str() {
                Some(text) => Ok(Some(text.to_owned())),
                None => Err(ProjectError::WrongType {
                    path: self.member_path(name),
                    expected: "a string",
                    found: value.kind(),
                }),
            },
        }
    }

    pub(crate) fn required_string(&mut self, name: &str) -> Result<String, ProjectError> {
        let path = self.member_path(name);
        match self.take_string(name)? {
            None => Err(ProjectError::MissingMember { path }),
            Some(text) if text.is_empty() => Err(ProjectError::EmptyValue { path }),
            Some(text) => Ok(text),
        }
    }

    fn required_u16(&mut self, name: &str) -> Result<u16, ProjectError> {
        let path = self.member_path(name);
        let Some(value) = self.take(name) else {
            return Err(ProjectError::MissingMember { path });
        };
        let Some(number) = value.as_u64() else {
            return Err(ProjectError::WrongType {
                path,
                expected: "a whole number",
                found: value.kind(),
            });
        };
        u16::try_from(number).map_err(|_| ProjectError::WrongType {
            path,
            expected: "a port between 0 and 65535",
            found: JsonKind::Number,
        })
    }

    fn take_i64(&mut self, name: &str) -> Result<Option<i64>, ProjectError> {
        match self.take(name) {
            None => Ok(None),
            Some(value) => match value.as_i64() {
                Some(number) => Ok(Some(number)),
                None => Err(ProjectError::WrongType {
                    path: self.member_path(name),
                    expected: "a whole number of milliseconds",
                    found: value.kind(),
                }),
            },
        }
    }

    /// A whole number, required.
    pub(crate) fn required_u64(&mut self, name: &str) -> Result<u64, ProjectError> {
        let path = self.member_path(name);
        let Some(value) = self.take(name) else {
            return Err(ProjectError::MissingMember { path });
        };
        value.as_u64().ok_or(ProjectError::WrongType {
            path,
            expected: "a whole number",
            found: value.kind(),
        })
    }

    /// An optional array, so that a member a document leaves out means "empty" in one place.
    pub(crate) fn take_array(&mut self, name: &str) -> Result<Option<&'a [Json]>, ProjectError> {
        let path = self.member_path(name);
        let Some(value) = self.take(name) else {
            return Ok(None);
        };
        value.as_array().map(Some).ok_or(ProjectError::WrongType {
            path,
            expected: "an array",
            found: value.kind(),
        })
    }

    pub(crate) fn take_string_array(&mut self, name: &str) -> Result<Vec<String>, ProjectError> {
        let path = self.member_path(name);
        let Some(value) = self.take(name) else {
            return Ok(Vec::new());
        };
        let Some(values) = value.as_array() else {
            return Err(ProjectError::WrongType {
                path,
                expected: "an array of strings",
                found: value.kind(),
            });
        };
        let mut out = Vec::with_capacity(values.len());
        for (index, entry) in values.iter().enumerate() {
            let Some(text) = entry.as_str() else {
                return Err(ProjectError::WrongType {
                    path: format!("{path}[{index}]"),
                    expected: "a string",
                    found: entry.kind(),
                });
            };
            out.push(text.to_owned());
        }
        Ok(out)
    }

    pub(crate) fn finish(self) -> Preserved {
        Preserved {
            members: self
                .members
                .into_iter()
                .map(|(name, value)| (name, value.clone()))
                .collect(),
        }
    }
}

pub(crate) fn sub_object<'a>(
    reader: &mut Reader<'a>,
    name: &str,
) -> Result<Option<Reader<'a>>, ProjectError> {
    let path = reader.member_path(name);
    let Some(value) = reader.take(name) else {
        return Ok(None);
    };
    Ok(Some(Reader::object(path, value)?))
}

pub(crate) fn sub_objects<'a>(
    reader: &mut Reader<'a>,
    name: &str,
) -> Result<Vec<Reader<'a>>, ProjectError> {
    let path = reader.member_path(name);
    let Some(value) = reader.take(name) else {
        return Ok(Vec::new());
    };
    let Some(values) = value.as_array() else {
        return Err(ProjectError::WrongType {
            path,
            expected: "an array",
            found: value.kind(),
        });
    };
    let mut out = Vec::with_capacity(values.len());
    for (index, entry) in values.iter().enumerate() {
        out.push(Reader::object(reader.entry_path(name, index), entry)?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{
        DeleteRefusal, MIGRATION_STEPS, PROJECT_FILE_NAME, ProjectContent, ProjectDocument,
        ProjectDraft, ProjectError, ProjectFile, ProjectWriteRefusal, ReferenceKind,
        ReferenceVerdict, Resolution, VERSION_MEMBERS, VersionVerdict, migration_path,
    };
    use crate::contracts::revision::FormatVersion;
    use crate::host::network::{CredentialRef, CredentialSource, ServicePolicy, TlsPolicy};
    use crate::host::paths::ProjectRoot;
    use crate::project::format::{check_generated_sources, check_startup, check_unique};
    use crate::project::json;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, UNIX_EPOCH};

    /// The smallest document this build writes, spelled exactly the way the writer spells it.
    const MINIMAL: &str = "{\n  \"format\": 2,\n  \"project\": {\n    \"id\": \"3f2b\",\n    \"name\": \"Demo\"\n  },\n  \"pages\": [],\n  \"resources\": [],\n  \"targets\": [],\n  \"generated\": [],\n  \"startup\": null,\n  \"network\": {\n    \"services\": []\n  }\n}\n";

    /// A document that uses every member version 1 defines.
    const FULL: &str = r#"{
  "format": 2,
  "project": {
    "id": "9c1f",
    "name": "Demo"
  },
  "pages": [
    {
      "document": "pages/main.json",
      "title": "Main window",
      "stylesheet": "styles/main.css"
    },
    {
      "document": "pages/settings.json"
    }
  ],
  "resources": [
    {
      "path": "resources/icon.png"
    }
  ],
  "targets": [
    {
      "name": "desktop",
      "profile": "debug",
      "features": [
        "desktop"
      ]
    }
  ],
  "generated": [
    {
      "output": "src/generated/main.rs",
      "from": "pages/main.json"
    }
  ],
  "startup": "pages/main.json",
  "network": {
    "services": [
      {
        "name": "api",
        "host": "api.example.com",
        "port": 443,
        "tls": {
          "roots": "system-and-file",
          "roots_file": "certs/internal-ca.pem",
          "client_certificate": {
            "name": "client",
            "source": {
              "kind": "environment",
              "variable": "CLIENT_CERT"
            },
            "expires_at_unix_ms": 1700000000000
          }
        },
        "credential": {
          "name": "API_TOKEN",
          "source": {
            "kind": "environment",
            "variable": "API_TOKEN"
          },
          "expires_at_unix_ms": 1700000000000
        }
      }
    ]
  }
}
"#;

    struct TempDirectory(PathBuf);

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A project root of its own per test, removed when the test ends.
    struct Sandbox {
        root: PathBuf,
        _guard: TempDirectory,
    }

    fn sandbox() -> Sandbox {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("rustage-project-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&root).expect("a temporary directory is creatable");
        Sandbox {
            root: root.clone(),
            _guard: TempDirectory(root),
        }
    }

    impl Sandbox {
        fn root(&self) -> ProjectRoot {
            ProjectRoot::new(&self.root).expect("the sandbox is a directory")
        }

        fn root_path(&self) -> PathBuf {
            self.root().as_path().to_path_buf()
        }

        fn write(&self, relative: &str, text: &str) -> PathBuf {
            let path = self.root.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("a parent directory is creatable");
            }
            std::fs::write(&path, text).expect("a file is writable");
            path
        }

        fn parse(&self, text: &str) -> Result<ProjectFile, ProjectError> {
            ProjectFile::parse(&self.root(), self.root.join(PROJECT_FILE_NAME), text)
        }

        fn document(&self, text: &str) -> ProjectDocument {
            let file = self.parse(text).expect("the document is readable");
            match file.content() {
                ProjectContent::Interpreted(document) => document.clone(),
                other => panic!("expected an interpreted document, found {other:?}"),
            }
        }

        fn failure(&self, text: &str) -> ProjectError {
            self.parse(text).expect_err("the document is refused")
        }

        fn unknown_paths(&self, text: &str) -> Vec<String> {
            self.document(text)
                .unknown_fields()
                .iter()
                .map(|field| field.path().to_owned())
                .collect()
        }
    }

    #[test]
    fn a_document_is_interpreted_member_by_member() {
        let sandbox = sandbox();
        sandbox.write("pages/main.json", "{}");
        sandbox.write("pages/settings.json", "{}");
        sandbox.write("styles/main.css", "");
        sandbox.write("resources/icon.png", "");
        sandbox.write("src/generated/main.rs", "");
        sandbox.write("certs/internal-ca.pem", "");

        let document = sandbox.document(FULL);

        assert_eq!(document.format().get(), 2);
        assert_eq!(document.identity().id(), "9c1f");
        assert_eq!(document.identity().name(), "Demo");
        assert_eq!(document.pages().len(), 2);
        assert_eq!(document.pages()[0].title(), Some("Main window"));
        assert_eq!(document.pages()[0].document().written(), "pages/main.json");
        assert_eq!(
            document.pages()[0].document().kind(),
            ReferenceKind::PageDocument
        );
        assert_eq!(
            document.pages()[0]
                .stylesheet()
                .map(|sheet| sheet.written()),
            Some("styles/main.css")
        );
        assert_eq!(document.pages()[1].title(), None);
        assert_eq!(document.pages()[1].stylesheet(), None);
        assert_eq!(
            document.resources()[0].path().written(),
            "resources/icon.png"
        );
        assert_eq!(document.targets()[0].name(), "desktop");
        assert_eq!(document.targets()[0].profile(), Some("debug"));
        assert_eq!(document.targets()[0].features(), ["desktop"]);
        assert_eq!(
            document.generated()[0].output().written(),
            "src/generated/main.rs"
        );
        assert_eq!(document.generated()[0].from().written(), "pages/main.json");
        assert_eq!(document.startup_written(), Some("pages/main.json"));
        assert_eq!(
            document.startup().map(|entry| entry.kind()),
            Some(ReferenceKind::PageDocument)
        );
        assert_eq!(
            document.startup().map(|entry| entry.verdict()),
            Some(&ReferenceVerdict::Present),
            "the page the demo starts with is there"
        );

        let service = document.services()[0].policy();
        assert_eq!(service.service(), "api");
        assert_eq!(service.host(), "api.example.com");
        assert_eq!(service.port(), 443);
        let credential = service.credential().expect("a credential reference");
        assert_eq!(credential.name(), "API_TOKEN");
        assert_eq!(
            credential.source(),
            &CredentialSource::Environment("API_TOKEN".to_owned())
        );
        assert_eq!(
            credential.expires_at(),
            Some(UNIX_EPOCH + Duration::from_millis(1_700_000_000_000))
        );
        let client = service
            .tls()
            .client_certificate()
            .expect("a client certificate reference");
        assert_eq!(client.name(), "client");
        assert!(service.tls().verification_required());
        let roots = document.services()[0]
            .roots_file()
            .expect("the certificate file");
        assert_eq!(roots.written(), "certs/internal-ca.pem");
        assert_eq!(roots.verdict(), &ReferenceVerdict::Present);

        assert!(document.unknown_fields().is_empty());
        assert!(document.refused_references().is_empty());
        // A document this build understands comes back exactly as it was written.
        assert_eq!(document.render(), FULL);
    }

    #[test]
    fn the_writer_writes_every_member_version_1_defines() {
        let sandbox = sandbox();
        let document = sandbox.document(MINIMAL);
        assert_eq!(document.render(), MINIMAL);
        // Members a document does not have mean "empty", which is what a version 1 document says
        // and what decision `F1` forbids turning into a silent default; the two spellings therefore
        // read the same and are written the same.
        let sparse = sandbox
            .document("{\"format\": 1, \"project\": {\"id\": \"3f2b\", \"name\": \"Demo\"}}");
        assert!(sparse.pages().is_empty());
        assert!(sparse.resources().is_empty());
        assert!(sparse.targets().is_empty());
        assert!(sparse.generated().is_empty());
        assert!(sparse.services().is_empty());
        assert_eq!(sparse.render(), MINIMAL);

        let json = document.to_json();
        let names: Vec<&str> = json.as_object().expect("an object").names().collect();
        assert_eq!(names, VERSION_MEMBERS);
        assert_eq!(VERSION_MEMBERS.len(), 8);
    }

    #[test]
    fn unknown_members_survive_a_round_trip_and_are_reported() {
        let sandbox = sandbox();
        let text = r#"{
  "format": 1,
  "project": {
    "id": "3f2b",
    "name": "Demo",
    "owner": "someone else's build wrote this"
  },
  "future": {
    "nested": [
      1,
      2
    ]
  },
  "pages": [
    {
      "document": "pages/main.json",
      "customKey": "kept"
    }
  ],
  "network": {
    "services": [],
    "proxy": "http://proxy.example.com:8080"
  }
}
"#;
        let document = sandbox.document(text);
        assert_eq!(
            document
                .unknown_fields()
                .iter()
                .map(|field| field.path())
                .collect::<Vec<_>>(),
            [
                "future",
                "project.owner",
                "pages[0].customKey",
                "network.proxy"
            ]
        );
        assert_eq!(
            document.preserved().get("future"),
            json::parse("{\"nested\": [1, 2]}").ok().as_ref(),
            "the value a person wrote is kept as it was"
        );

        // Saving it puts every unknown member back, and reading that again reports the same thing.
        let saved = document.render();
        assert!(saved.contains("\"customKey\": \"kept\""));
        assert!(saved.contains("http://proxy.example.com:8080"));
        let again = sandbox.document(&saved);
        assert_eq!(again.unknown_fields(), document.unknown_fields());
        assert_eq!(again.preserved(), document.preserved());
        assert_eq!(again.render(), saved);
    }

    #[test]
    fn a_newer_format_version_refuses_to_write_the_project() {
        let sandbox = sandbox();
        let path = sandbox.write(
            PROJECT_FILE_NAME,
            "{\"format\": 3, \"project\": {\"id\": \"3f2b\", \"name\": \"Demo\"}}\n",
        );
        let file = ProjectFile::open(&path).expect("a newer version still opens");

        assert_eq!(file.version().get(), 3);
        assert_eq!(
            file.verdict(),
            VersionVerdict::Newer {
                found: crate::contracts::revision::FormatVersion::from_raw(3),
                current: crate::contracts::revision::FormatVersion::CURRENT,
            }
        );
        assert!(!file.is_writable());
        let refusal = file.writer().expect_err("a newer version is never written");
        assert_eq!(
            refusal,
            ProjectWriteRefusal::UnknownFormat {
                found: crate::contracts::revision::FormatVersion::from_raw(3),
                current: crate::contracts::revision::FormatVersion::CURRENT,
            }
        );
        assert!(refusal.message().contains("newer"));
        assert!(refusal.message().contains("nothing was written"));
        // The only way to write is through a writer, so there was nothing to call and nothing on
        // disk changed.
        assert_eq!(
            std::fs::read_to_string(&path).expect("the file is still there"),
            "{\"format\": 3, \"project\": {\"id\": \"3f2b\", \"name\": \"Demo\"}}\n"
        );
    }

    #[test]
    fn a_future_version_is_read_without_being_interpreted() {
        let sandbox = sandbox();
        let text = r#"{
  "format": 3,
  "project": {
    "id": "3f2b",
    "name": "Demo"
  },
  "pages": {
    "main": "pages/main.json"
  },
  "window": {
    "width": 1200
  }
}
"#;
        let file = sandbox
            .parse(text)
            .expect("a future version opens read-only");
        let foreign = match file.content() {
            ProjectContent::Foreign(foreign) => foreign,
            other => panic!("expected a foreign project, found {other:?}"),
        };
        assert_eq!(foreign.version().get(), 3);
        assert_eq!(
            foreign.identity().map(|identity| identity.name()),
            Some("Demo")
        );
        assert_eq!(
            foreign.members(),
            ["format", "project", "pages", "window"],
            "every member is listed, so a person can see what the file holds"
        );
        assert_eq!(
            foreign
                .unknown_fields()
                .iter()
                .map(|field| field.path())
                .collect::<Vec<_>>(),
            ["pages", "window"],
            "nothing but the version and the identity is read"
        );
        assert!(file.content().document().is_none());
    }

    #[test]
    fn a_future_version_without_a_readable_identity_still_opens() {
        let sandbox = sandbox();
        let file = sandbox
            .parse("{\"format\": 9, \"project\": \"somewhere else\"}")
            .expect("a future version opens read-only");
        assert_eq!(file.content().identity(), None);
        assert_eq!(
            file.unknown_fields()
                .iter()
                .map(|field| field.path())
                .collect::<Vec<_>>(),
            ["project"]
        );
    }

    #[test]
    fn an_older_format_is_refused_with_a_reason() {
        let sandbox = sandbox();
        let error =
            sandbox.failure("{\"format\": 0, \"project\": {\"id\": \"a\", \"name\": \"b\"}}");
        assert!(matches!(error, ProjectError::OlderFormat { .. }));
        assert!(error.message().contains("older"));
        assert!(error.message().contains("migration"));
    }

    #[test]
    fn a_version_that_is_missing_or_not_a_whole_number_is_refused() {
        let sandbox = sandbox();
        for (text, expected) in [
            ("{}", "format is missing"),
            ("{\"format\": \"1\"}", "format must be a whole number"),
            ("{\"format\": 1.5}", "format must be a whole number"),
            ("{\"format\": -1}", "format must be a whole number"),
            ("{\"format\": 5000000000}", "fits in 32 bits"),
            ("[]", "the document must be an object"),
        ] {
            let error = sandbox.failure(text);
            assert!(
                error.message().contains(expected),
                "{text} was refused as `{}`",
                error.message()
            );
        }
    }

    #[test]
    fn a_member_of_the_wrong_kind_is_refused_with_the_path_that_holds_it() {
        let sandbox = sandbox();
        for (text, expected) in [
            (
                "{\"format\": 1, \"project\": {}, \"pages\": {}}",
                "project.id is missing",
            ),
            (
                "{\"format\": 1, \"project\": {\"id\": \"a\", \"name\": \"b\"}, \"pages\": {}}",
                "pages must be an array",
            ),
            (
                "{\"format\": 1, \"project\": {\"id\": \"a\", \"name\": \"b\"}, \"pages\": [{\"document\": 3}]}",
                "pages[0].document must be a string",
            ),
            (
                "{\"format\": 1, \"project\": {\"id\": \"a\", \"name\": \"b\"}, \"targets\": [{\"name\": \"x\", \"features\": \"y\"}]}",
                "targets[0].features must be an array of strings",
            ),
            (
                "{\"format\": 1, \"project\": {\"id\": \"a\", \"name\": \"b\"}, \"network\": []}",
                "network must be an object",
            ),
            (
                "{\"format\": 1, \"project\": {\"id\": \"a\", \"name\": \"b\"}, \"generated\": [{\"output\": \"a.rs\"}]}",
                "generated[0].from is missing",
            ),
        ] {
            let error = sandbox.failure(text);
            assert!(
                error.message().contains(expected),
                "{text} was refused as `{}`",
                error.message()
            );
        }
    }

    #[test]
    fn a_member_that_has_to_say_something_may_not_be_empty() {
        let sandbox = sandbox();
        let error =
            sandbox.failure("{\"format\": 1, \"project\": {\"id\": \"a\", \"name\": \"\"}}");
        assert!(matches!(error, ProjectError::EmptyValue { .. }));
        assert!(error.message().contains("project.name"));
    }

    #[test]
    fn a_port_outside_the_possible_range_is_refused() {
        let sandbox = sandbox();
        let error = sandbox.failure(
            "{\"format\": 1, \"project\": {\"id\": \"a\", \"name\": \"b\"}, \
             \"network\": {\"services\": [{\"name\": \"api\", \"host\": \"h\", \"port\": 70000}]}}",
        );
        assert!(error.message().contains("port between 0 and 65535"));
    }

    #[test]
    fn material_for_a_credential_may_not_be_written_into_the_project() {
        let sandbox = sandbox();
        let error = sandbox.failure(
            "{\"format\": 1, \"project\": {\"id\": \"a\", \"name\": \"b\"}, \
             \"network\": {\"services\": [{\"name\": \"api\", \"host\": \"h\", \"port\": 443, \
             \"credential\": {\"name\": \"API_TOKEN\", \"source\": {\"kind\": \"system-keyring\"}, \"value\": \"s3cr3t\"}}]}}",
        );
        match &error {
            ProjectError::SecretInProject { path, members } => {
                assert_eq!(path, "network.services[0].credential");
                assert_eq!(members, &["`value`".to_owned()]);
            }
            other => panic!("expected a refused secret, found {other:?}"),
        }
        assert!(error.message().contains("keyring"));
        assert!(error.message().contains("environment variable"));

        let in_source = sandbox.failure(
            "{\"format\": 1, \"project\": {\"id\": \"a\", \"name\": \"b\"}, \
             \"network\": {\"services\": [{\"name\": \"api\", \"host\": \"h\", \"port\": 443, \
             \"credential\": {\"name\": \"t\", \"source\": {\"kind\": \"environment\", \
             \"variable\": \"V\", \"token\": \"s3cr3t\"}}}]}}",
        );
        assert!(matches!(in_source, ProjectError::SecretInProject { .. }));
    }

    #[test]
    fn a_credential_source_this_build_does_not_know_is_refused() {
        let sandbox = sandbox();
        let error = sandbox.failure(
            "{\"format\": 1, \"project\": {\"id\": \"a\", \"name\": \"b\"}, \
             \"network\": {\"services\": [{\"name\": \"api\", \"host\": \"h\", \"port\": 443, \
             \"credential\": {\"name\": \"t\", \"source\": {\"kind\": \"a-file\"}}}]}}",
        );
        assert!(error.message().contains("system-keyring"));
        let roots = sandbox.failure(
            "{\"format\": 1, \"project\": {\"id\": \"a\", \"name\": \"b\"}, \
             \"network\": {\"services\": [{\"name\": \"api\", \"host\": \"h\", \"port\": 443, \
             \"tls\": {\"roots\": \"everywhere\"}}]}}",
        );
        assert!(roots.message().contains("system-and-file"));
    }

    #[test]
    fn a_reference_that_leaves_the_root_is_refused_and_blocks_writing() {
        let sandbox = sandbox();
        let text = "{\"format\": 1, \"project\": {\"id\": \"a\", \"name\": \"b\"}, \
             \"pages\": [{\"document\": \"../outside.json\"}], \
             \"resources\": [{\"path\": \"/etc/passwd\"}]}";
        let file = sandbox
            .parse(text)
            .expect("the document is readable, so it can be fixed");
        let document = file.content().document().expect("an interpreted document");
        let refused = document.refused_references();
        assert_eq!(refused.len(), 2);
        assert_eq!(refused[0].written(), "../outside.json");
        assert_eq!(refused[1].written(), "/etc/passwd");
        assert!(refused[0].resolved().is_none());
        let refusal = file
            .writer()
            .expect_err("a document like this is not written");
        match refusal {
            ProjectWriteRefusal::RefusedReferences { count, first } => {
                assert_eq!(count, 2);
                assert_eq!(first, "../outside.json");
            }
            other => panic!("expected refused references, found {other:?}"),
        }
        assert!(!file.is_writable());
    }

    #[test]
    fn a_reference_inside_the_root_is_reported_present_or_absent() {
        let sandbox = sandbox();
        sandbox.write("pages/main.json", "{}");
        std::fs::create_dir_all(sandbox.root.join("resources")).unwrap();
        let text = "{\"format\": 1, \"project\": {\"id\": \"a\", \"name\": \"b\"}, \
             \"pages\": [{\"document\": \"pages/main.json\"}], \
             \"resources\": [{\"path\": \"resources/missing.png\"}, \
             {\"path\": \"nothing-here/x.png\"}]}";
        let document = sandbox.document(text);
        let references = document.references();
        assert_eq!(references[0].verdict(), &ReferenceVerdict::Present);
        assert!(references[0].resolved().is_some());
        assert_eq!(references[1].verdict(), &ReferenceVerdict::Absent);
        assert_eq!(references[2].verdict(), &ReferenceVerdict::Absent);
        assert!(references[1].render().contains("not there yet"));
        assert!(document.refused_references().is_empty());
    }

    #[test]
    fn every_reference_is_reported_in_document_order() {
        let sandbox = sandbox();
        let document = sandbox.document(FULL);
        let kinds: Vec<ReferenceKind> = document
            .references()
            .into_iter()
            .map(|reference| reference.kind())
            .collect();
        assert_eq!(
            kinds,
            [
                ReferenceKind::PageDocument,
                ReferenceKind::Stylesheet,
                ReferenceKind::PageDocument,
                ReferenceKind::Resource,
                ReferenceKind::GeneratedOutput,
                ReferenceKind::DesignSource,
                ReferenceKind::PageDocument,
                ReferenceKind::CertificateAuthority,
            ]
        );
        assert_eq!(
            document.startup().map(|entry| entry.written()),
            Some("pages/main.json")
        );
    }

    #[test]
    fn a_reference_is_resolved_against_the_root_the_document_is_in() {
        let first = sandbox();
        let second = sandbox();
        let text = "{\"format\": 1, \"project\": {\"id\": \"a\", \"name\": \"b\"}, \
             \"pages\": [{\"document\": \"pages/main.json\"}]}";
        let here = first.document(text);
        let there = second.document(text);
        assert_eq!(here.pages()[0].document().written(), "pages/main.json");
        assert_eq!(there.pages()[0].document().written(), "pages/main.json");
        assert_ne!(
            here.pages()[0].document().resolved(),
            there.pages()[0].document().resolved(),
            "the same text means a different place in a different root"
        );
        assert!(
            here.pages()[0]
                .document()
                .resolved()
                .expect("resolved")
                .starts_with(first.root_path())
        );
    }

    #[test]
    fn a_generated_file_that_names_no_page_is_refused() {
        let sandbox = sandbox();
        let error = sandbox.failure(
            "{\"format\": 1, \"project\": {\"id\": \"a\", \"name\": \"b\"}, \
             \"pages\": [{\"document\": \"pages/main.json\"}], \
             \"generated\": [{\"output\": \"src/generated/main.rs\", \"from\": \"pages/other.json\"}]}",
        );
        match error {
            ProjectError::Dangling { path, name } => {
                assert_eq!(path, "generated[0].from");
                assert_eq!(name, "pages/other.json");
            }
            other => panic!("expected a dangling reference, found {other:?}"),
        }

        // The same page written two ways is the same page.
        let accepted = sandbox.document(
            "{\"format\": 1, \"project\": {\"id\": \"a\", \"name\": \"b\"}, \
             \"pages\": [{\"document\": \"pages/main.json\"}], \
             \"generated\": [{\"output\": \"src/generated/main.rs\", \"from\": \"./pages/main.json\"}]}",
        );
        assert_eq!(
            accepted.generated()[0].from().written(),
            "./pages/main.json"
        );
    }

    #[test]
    fn two_entries_that_claim_the_same_thing_are_refused() {
        let sandbox = sandbox();
        for (text, expected) in [
            (
                "{\"format\": 1, \"project\": {\"id\": \"a\", \"name\": \"b\"}, \"pages\": \
                 [{\"document\": \"p.json\"}, {\"document\": \"p.json\"}]}",
                "pages[].document `p.json`",
            ),
            (
                "{\"format\": 1, \"project\": {\"id\": \"a\", \"name\": \"b\"}, \"resources\": \
                 [{\"path\": \"r.png\"}, {\"path\": \"r.png\"}]}",
                "resources[].path `r.png`",
            ),
            (
                "{\"format\": 1, \"project\": {\"id\": \"a\", \"name\": \"b\"}, \"targets\": \
                 [{\"name\": \"desktop\"}, {\"name\": \"desktop\"}]}",
                "targets[].name `desktop`",
            ),
        ] {
            let error = sandbox.failure(text);
            assert!(matches!(error, ProjectError::Duplicate { .. }));
            assert!(
                error.message().contains(expected),
                "refused as `{}`",
                error.message()
            );
        }
    }

    #[test]
    fn a_service_carries_its_credential_by_reference_and_every_field_round_trips() {
        let sandbox = sandbox();
        sandbox.write("certs/internal-ca.pem", "");
        let document = sandbox.document(FULL);
        let entry = &document.services()[0];

        let expiry = UNIX_EPOCH + Duration::from_millis(1_700_000_000_000);
        let roots = sandbox
            .root()
            .resolve("certs/internal-ca.pem")
            .expect("the file is inside the root");
        let expected = ServicePolicy::new(
            "api",
            "api.example.com",
            443,
            TlsPolicy::new()
                .with_additional_roots(roots)
                .with_client_certificate(
                    CredentialRef::new(
                        "client",
                        CredentialSource::Environment("CLIENT_CERT".to_owned()),
                    )
                    .expiring_at(expiry),
                ),
        )
        .with_credential(
            CredentialRef::new(
                "API_TOKEN",
                CredentialSource::Environment("API_TOKEN".to_owned()),
            )
            .expiring_at(expiry),
        );
        // A field added to ServicePolicy, TlsPolicy or CredentialRef without a member in the codec
        // fails here, rather than being dropped from every project file.
        assert_eq!(entry.policy(), &expected);

        // And what was read is written back as something that reads the same again.
        let saved = document.render();
        let again = sandbox.document(&saved);
        assert_eq!(again.services()[0].policy(), &expected);
        assert_eq!(again.services()[0].roots_file(), entry.roots_file());
    }

    #[test]
    fn an_instant_written_as_milliseconds_comes_back_the_same_millisecond() {
        let sandbox = sandbox();
        let document = sandbox.document(FULL);
        let credential = document.services()[0]
            .policy()
            .credential()
            .expect("a credential reference");
        assert_eq!(
            credential.expires_at(),
            Some(UNIX_EPOCH + Duration::from_millis(1_700_000_000_000))
        );
        // The format's resolution is a millisecond, and a finer instant is written truncated rather
        // than refused: written here so the behaviour is pinned instead of assumed.
        let coarse = sandbox.document(&document.render());
        assert_eq!(
            coarse.services()[0]
                .policy()
                .credential()
                .and_then(|c| c.expires_at()),
            Some(UNIX_EPOCH + Duration::from_millis(1_700_000_000_000))
        );
    }

    #[test]
    fn machine_local_members_are_reported_rather_than_read() {
        let sandbox = sandbox();
        let paths = sandbox.unknown_paths(
            "{\"format\": 1, \"project\": {\"id\": \"a\", \"name\": \"b\"}, \
             \"window\": {\"width\": 1200}, \"session\": {\"last_page\": \"pages/main.json\"}}",
        );
        assert_eq!(paths, ["window", "session"]);
    }

    #[test]
    fn saving_commits_through_a_temporary_file_and_refuses_a_changed_file() {
        let sandbox = sandbox();
        let path = sandbox.write(PROJECT_FILE_NAME, MINIMAL);
        let file = ProjectFile::open(&path).expect("the project opens");
        file.writer()
            .expect("a version 1 document may be written")
            .save()
            .expect("the save succeeds");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), MINIMAL);

        let mut names: Vec<String> = std::fs::read_dir(&sandbox.root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(
            names,
            [PROJECT_FILE_NAME.to_owned()],
            "no temporary file is left behind"
        );

        // Somebody else writes the file after it was read: the stale copy is not written back.
        std::fs::write(
            &path,
            "{\"format\": 2, \"project\": {\"id\": \"a\", \"name\": \"edited\"}}",
        )
        .unwrap();
        let stale = ProjectFile::open(&path).expect("the edited project opens");
        std::fs::write(
            &path,
            "{\"format\": 2, \"project\": {\"id\": \"a\", \"name\": \"newer\"}}",
        )
        .unwrap();
        let error = stale
            .writer()
            .expect("the version is still one this build knows")
            .save()
            .expect_err("writing over somebody's edit is refused");
        assert!(matches!(error, ProjectError::ChangedOnDisk { .. }));
        assert!(error.message().contains("no edit was lost"));
        assert!(
            std::fs::read_to_string(&path).unwrap().contains("newer"),
            "the file that changed on disk is still the one on disk"
        );

        // Writing somewhere else is what a round trip check needs, and it is not a conflict.
        let elsewhere = sandbox.root.join("roundtrip.json");
        let file = ProjectFile::open(&path).expect("the project opens");
        let written = file
            .writer()
            .expect("writable")
            .save_to(&elsewhere)
            .expect("a new path is writable");
        assert_eq!(written, elsewhere);
        assert_eq!(
            std::fs::read_to_string(&elsewhere).unwrap(),
            file.content().document().expect("a document").render()
        );
    }

    #[test]
    fn a_project_file_is_opened_from_the_directory_it_is_in() {
        let sandbox = sandbox();
        let path = sandbox.write(PROJECT_FILE_NAME, MINIMAL);
        let file = ProjectFile::open(&path).expect("the project opens");
        assert_eq!(file.root().as_path(), sandbox.root_path());
        assert_eq!(file.path(), std::fs::canonicalize(&path).unwrap());
        assert_eq!(
            file.verdict(),
            VersionVerdict::Current {
                version: crate::contracts::revision::FormatVersion::CURRENT,
            }
        );
        assert!(file.verdict().is_writable());
        assert!(file.verdict().message().contains("which this build writes"));

        let missing = ProjectFile::open(sandbox.root.join("nowhere.json"));
        assert!(matches!(missing, Err(ProjectError::Io { .. })));
    }

    #[test]
    fn text_that_is_not_json_is_refused_with_the_position() {
        let sandbox = sandbox();
        let error = sandbox.failure("{\n  \"format\": 1,\n");
        match error {
            ProjectError::NotJson { path, error } => {
                assert!(path.ends_with(PROJECT_FILE_NAME));
                assert_eq!(error.line(), 3);
            }
            other => panic!("expected a JSON refusal, found {other:?}"),
        }
        assert!(
            sandbox
                .failure("{\"format\": 1, \"format\": 1}")
                .message()
                .contains("twice")
        );
    }

    #[test]
    fn the_demo_project_in_the_repository_is_a_document_this_build_writes() {
        const DEMO: &str = include_str!("../../examples/demo-project/project.json");
        let root = ProjectRoot::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/examples/demo-project"
        ))
        .expect("the demo directory is in the repository");
        let file = ProjectFile::parse(&root, root.as_path().join(PROJECT_FILE_NAME), DEMO)
            .expect("the demo project is a version 1 document this build understands");
        let document = file.content().document().expect("an interpreted document");

        assert!(
            document.unknown_fields().is_empty(),
            "the committed demo holds nothing this build does not know"
        );
        assert_eq!(
            document.render(),
            DEMO,
            "the committed demo is exactly what this build writes"
        );
        assert!(
            file.is_writable(),
            "every demo reference stays inside its root"
        );
        assert_eq!(document.pages().len(), 1);
        assert_eq!(
            document.pages()[0]
                .stylesheet()
                .expect("the demo names a stylesheet")
                .verdict(),
            &ReferenceVerdict::Present,
            "the committed stylesheet is in the repository"
        );
        assert_eq!(
            document.pages()[0].document().verdict(),
            &ReferenceVerdict::Absent,
            "a page's own document arrives with T02.02, so the demo has none yet"
        );
    }

    #[test]
    fn an_older_version_is_migrated_by_versioned_steps() {
        // Decision `Q6`: every format change comes with a versioned migration step, and decision
        // `U1`: the steps only go forward and a version with no step out of it is refused.
        assert_eq!(
            migration_path(FormatVersion::from_raw(1))
                .expect("one step")
                .len(),
            1
        );
        assert!(
            migration_path(FormatVersion::from_raw(2))
                .expect("nothing to do")
                .is_empty()
        );
        assert_eq!(
            migration_path(FormatVersion::from_raw(0)).expect_err("no step out of version 0"),
            FormatVersion::from_raw(0)
        );
        let step = MIGRATION_STEPS[0];
        assert_eq!(step.from().get(), 1);
        assert_eq!(step.to().get(), 2);
        assert!(step.what().contains("startup"));
        assert_eq!(FormatVersion::OLDEST_READABLE.get(), 1);

        let sandbox = sandbox();
        let version_1 = "{\n  \"format\": 1,\n  \"project\": {\n    \"id\": \"3f2b\",\n    \"name\": \"Older\"\n  },\n  \"pages\": [],\n  \"resources\": [],\n  \"targets\": [],\n  \"generated\": [],\n  \"network\": {\n    \"services\": []\n  }\n}\n";
        let path = sandbox.write(PROJECT_FILE_NAME, version_1);
        let file = ProjectFile::open(&path).expect("an older version is read, then migrated");

        assert_eq!(file.migrated_from(), Some(FormatVersion::from_raw(1)));
        assert_eq!(
            file.content().version().get(),
            2,
            "it is read by this build's rules"
        );
        assert_eq!(
            file.verdict(),
            VersionVerdict::Migrated {
                found: FormatVersion::from_raw(1),
                current: FormatVersion::CURRENT,
                steps: 1,
            }
        );
        assert!(file.verdict().needs_migration());
        assert!(
            file.is_writable(),
            "a version with a step is writable after migrating"
        );
        let document = file.content().document().expect("interpreted");
        assert_eq!(document.identity().name(), "Older");
        assert_eq!(document.startup_written(), None, "version 1 chose no entry");

        // Writing it does not overwrite the file that was read: it is kept first (decision `Q6`).
        let writer = file.writer().expect("a version with a step is writable");
        let refusal = writer
            .save()
            .expect_err("a migration keeps the file it read");
        assert!(matches!(refusal, ProjectError::MigrationNeedsBackup { .. }));
        assert!(refusal.message().contains("keeps the file it read first"));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            version_1,
            "nothing was written"
        );

        let migrated = writer.save_migrating().expect("the migration writes");
        assert_eq!(migrated.from().get(), 1);
        assert_eq!(migrated.to().get(), 2);
        assert_eq!(migrated.target(), file.path());
        assert_eq!(
            std::fs::read_to_string(migrated.backup()).unwrap(),
            version_1,
            "the version 1 document is at the backup"
        );
        assert!(migrated.message().contains("migrated"));

        let again = ProjectFile::open(&path).expect("what was written reads back");
        assert_eq!(again.migrated_from(), None);
        assert!(matches!(again.verdict(), VersionVerdict::Current { .. }));
        assert!(again.content().document().is_some());

        // There is nothing to migrate in a document this build writes.
        let refusal = again
            .writer()
            .expect("writable")
            .save_migrating()
            .expect_err("nothing to migrate");
        assert!(matches!(refusal, ProjectError::NothingToMigrate { .. }));
    }

    #[test]
    fn a_version_the_migration_table_does_not_reach_is_refused() {
        let sandbox = sandbox();
        let error =
            sandbox.failure("{\"format\": 0, \"project\": {\"id\": \"a\", \"name\": \"b\"}}");
        match &error {
            ProjectError::OlderFormat { found, current } => {
                assert_eq!(found.get(), 0);
                assert_eq!(current.get(), 2);
            }
            other => panic!("expected an unreachable version, found {other:?}"),
        }
    }

    #[test]
    fn a_document_may_not_choose_an_entry_its_version_does_not_define() {
        let sandbox = sandbox();
        // A version 1 document with the member version 2 added: believing it would be reading a file
        // that contradicts its own version.
        let error = sandbox.failure(
            "{\"format\": 1, \"project\": {\"id\": \"a\", \"name\": \"b\"}, \
             \"startup\": \"pages/main.json\"}",
        );
        assert!(
            error.message().contains("does not define `startup`"),
            "{}",
            error.message()
        );
    }

    #[test]
    fn the_startup_entry_has_to_be_a_page_the_document_declares() {
        let sandbox = sandbox();
        let declared = sandbox.document(
            "{\"format\": 2, \"project\": {\"id\": \"a\", \"name\": \"b\"}, \
             \"pages\": [{\"document\": \"pages/main.json\"}], \
             \"startup\": \"pages/main.json\"}",
        );
        assert_eq!(declared.startup_written(), Some("pages/main.json"));

        // `null` is a state, not a missing member: a project can have pages and nothing to start yet.
        let nothing = sandbox.document(
            "{\"format\": 2, \"project\": {\"id\": \"a\", \"name\": \"b\"}, \
             \"pages\": [{\"document\": \"pages/main.json\"}], \"startup\": null}",
        );
        assert_eq!(nothing.startup_written(), None);
        assert!(nothing.render().contains("\"startup\": null"));

        let dangling = sandbox.failure(
            "{\"format\": 2, \"project\": {\"id\": \"a\", \"name\": \"b\"}, \
             \"pages\": [{\"document\": \"pages/main.json\"}], \
             \"startup\": \"pages/other.json\"}",
        );
        match dangling {
            ProjectError::Dangling { path, name } => {
                assert_eq!(path, "startup");
                assert_eq!(name, "pages/other.json");
            }
            other => panic!("expected a dangling entry, found {other:?}"),
        }

        let wrong = sandbox.failure(
            "{\"format\": 2, \"project\": {\"id\": \"a\", \"name\": \"b\"}, \"startup\": 3}",
        );
        assert!(
            wrong
                .message()
                .contains("must be a path to a page document")
        );
        let empty = sandbox.failure(
            "{\"format\": 2, \"project\": {\"id\": \"a\", \"name\": \"b\"}, \"startup\": \"\"}",
        );
        assert!(empty.message().contains("startup is empty"));

        // A draft cannot choose an entry it does not declare either.
        let root = sandbox.root();
        let refusal = ProjectDraft::new(&root, "a", "b")
            .expect("a draft")
            .with_startup("pages/main.json")
            .build()
            .expect_err("a draft may not start with a page it does not declare");
        assert!(
            refusal
                .message()
                .contains("nothing in this project declares")
        );
    }

    #[test]
    fn removing_an_entry_that_is_pointed_at_is_refused_until_the_references_are_dealt_with() {
        let sandbox = sandbox();
        sandbox.write("pages/main.json", "{}");
        sandbox.write("pages/settings.json", "{}");
        let document = sandbox.document(
            "{\"format\": 2, \"project\": {\"id\": \"a\", \"name\": \"b\"}, \
             \"pages\": [{\"document\": \"pages/main.json\"}, {\"document\": \"pages/settings.json\"}], \
             \"generated\": [{\"output\": \"src/generated/main.rs\", \"from\": \"pages/main.json\"}], \
             \"startup\": \"pages/main.json\"}",
        );

        // What points at the first entry: the generated mapping and the startup choice.
        let references = document.references_to(&document.pages()[0]);
        assert_eq!(references.len(), 2);
        assert_eq!(references.generated(), ["src/generated/main.rs"]);
        assert!(references.startup());
        assert!(!references.is_empty());
        assert_eq!(references.lines().len(), 2);
        assert!(document.references_to(&document.pages()[1]).is_empty());

        // Blocking is the default answer, and it says what still points at it.
        let refusal = document
            .remove_entry(0, Resolution::Block)
            .expect_err("an entry other things point at is not removed");
        let DeleteRefusal::Referenced {
            document: named,
            references,
        } = &refusal
        else {
            unreachable!("expected a refused removal, found {refusal:?}");
        };
        assert_eq!(named, "pages/main.json");
        assert_eq!(references.len(), 2);
        assert!(refusal.message().contains("nothing was removed"));
        assert!(
            refusal
                .message()
                .contains("generated[src/generated/main.rs]")
        );

        // Fixing the references removes it and reports every one of them.
        let (removed, fixed) = document
            .remove_entry(0, Resolution::FixReferences)
            .expect("the references can be fixed");
        assert_eq!(removed.pages().len(), 1);
        assert_eq!(
            removed.pages()[0].document().written(),
            "pages/settings.json"
        );
        assert!(removed.generated().is_empty());
        assert_eq!(removed.startup_written(), None);
        assert_eq!(fixed.len(), 2);
        assert!(fixed[0].contains("src/generated/main.rs"));
        assert!(fixed[1].contains("no longer the entry the target starts with"));
        // What was produced is a document this build can read back: the same checks the reader runs.
        check_unique(&removed.pages).expect("the pages are still unique");
        check_generated_sources(&removed.pages, &removed.generated)
            .expect("the generated mappings still point at a declared page");
        check_startup(&removed.pages, removed.startup.as_ref())
            .expect("the startup still names a declared page");
        let text = removed.render();
        let again = sandbox
            .parse(&text)
            .expect("the document reads back")
            .content()
            .document()
            .expect("interpreted")
            .clone();
        assert_eq!(again, removed);

        // An entry nothing points at is removed without any fixing, and an index that is not there
        // is refused for that reason.
        let (only, fixed) = removed
            .remove_entry(0, Resolution::Block)
            .expect("nothing points at it");
        assert!(only.pages().is_empty());
        assert!(fixed.is_empty());
        let refusal = only
            .remove_entry(3, Resolution::Block)
            .expect_err("no such page");
        assert!(matches!(
            refusal,
            DeleteRefusal::NoSuchEntry { index: 3, known: 0 }
        ));
        assert!(refusal.message().contains("no page at index 3"));
    }

    #[test]
    fn the_startup_entry_is_chosen_and_cleared_by_the_document_that_owns_it() {
        let sandbox = sandbox();
        sandbox.write("pages/main.json", "{}");
        sandbox.write("pages/settings.json", "{}");
        let document = sandbox.document(
            "{\"format\": 2, \"project\": {\"id\": \"a\", \"name\": \"b\"}, \
             \"pages\": [{\"document\": \"pages/main.json\"}, {\"document\": \"pages/settings.json\"}], \
             \"startup\": null}",
        );
        let root = sandbox.root();

        let chosen = document
            .with_startup(&root, Some("pages/settings.json"))
            .expect("a declared page can be chosen");
        assert_eq!(chosen.startup_written(), Some("pages/settings.json"));
        assert!(
            chosen
                .render()
                .contains("\"startup\": \"pages/settings.json\"")
        );
        // Everything else is untouched: choosing an entry is not an edit of the rest.
        assert_eq!(chosen.pages(), document.pages());

        let cleared = chosen
            .with_startup(&root, None)
            .expect("the choice can be cleared");
        assert_eq!(cleared.startup_written(), None);
        assert_eq!(cleared.render(), document.render());

        let refusal = document
            .with_startup(&root, Some("pages/nowhere.json"))
            .expect_err("a page the document does not declare cannot start");
        assert!(refusal.message().contains("startup"));
    }

    #[test]
    fn a_document_only_exists_for_a_version_this_build_writes() {
        let sandbox = sandbox();
        let file = sandbox
            .parse("{\"format\": 7, \"project\": {\"id\": \"a\", \"name\": \"b\"}}")
            .expect("readable");
        assert_eq!(file.content().document(), None);
        assert_eq!(file.content().version().get(), 7);
        assert_eq!(file.verdict().version().get(), 7);
        assert!(!file.is_writable());
    }
}
