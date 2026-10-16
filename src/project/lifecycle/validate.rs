// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Validation and re-scan: what a project needs, whether it is there, and what changed since the
//! last look (task T02.08).
//!
//! Validation answers questions and changes nothing. That is not a rule written down here, it is the
//! shape of the types: [`Validation::of`] takes a document that is already read, and this module has
//! no writer at all, so "a missing SDK emptied my project" — the failure the acceptance names — is
//! not something this file can do even by accident.
//!
//! What it does instead is answer every need with one of four words, because collapsing them is how
//! a report starts lying (principle #22):
//!
//! * `present` — it is there, asked just now;
//! * `missing` — it is not there, asked just now;
//! * `not-checked` — nobody looked, and *why* nobody looked (the project is not trusted, the
//!   permission table said no, the store could not be reached). That is not the same finding as
//!   "missing": an SDK nobody looked for has not been shown to be absent;
//! * `refused` — the path leaves the project root, so it was never resolved.
//!
//! A need also says **where its answer came from**, and those sources are folded into one line, so
//! "opening a project offline" is a property of the answer set rather than a promise in prose:
//! nothing in a validation asks a far end, and a need that would is counted as such.
//!
//! Finally, [`Scan`] records what every reference resolved to, and [`Scan::between`] compares two of
//! them. A re-scan reads the file again rather than remembering: a recorded state reported as the
//! current one is the failure the whole module exists to avoid.

use std::{
    cmp::Ordering,
    fmt,
    io::ErrorKind,
    path::{Path, PathBuf},
    time::SystemTime,
};

use crate::contracts::revision::FormatVersion;
use crate::host::network::{CredentialHealth, CredentialStore};
use crate::project::build::configuration::{self, CredentialView};
use crate::project::lifecycle::session::{ToolUnavailable, ToolchainState};
use crate::project::record::format::{
    ProjectDocument, ProjectError, ProjectFile, Reference, ReferenceKind, ReferenceVerdict,
    TargetProfile, format_version,
};
use crate::project::record::json::{self, Json, JsonKind, JsonObject};

/// Where the answer to a need comes from.
///
/// It is a value rather than a comment because the report folds over it: a need answered by a far
/// end would change what "this project opens offline" means, so the one place a far end could be
/// named is the one place that gets counted.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Where {
    /// The project's own files, which are on this machine by definition.
    ProjectFiles,
    /// A program, a store or a file outside the project, on this machine.
    ThisMachine,
    /// The credential store this machine has.
    CredentialStore,
    /// A far end. Nothing in a validation is answered here, which is what makes an offline open a
    /// property of the answer set.
    Network,
}

impl Where {
    /// Every source, in the order a report lists them.
    pub const ALL: [Self; 4] = [
        Self::ProjectFiles,
        Self::ThisMachine,
        Self::CredentialStore,
        Self::Network,
    ];

    /// The name a report uses.
    pub const fn name(self) -> &'static str {
        match self {
            Self::ProjectFiles => "project-files",
            Self::ThisMachine => "this-machine",
            Self::CredentialStore => "credential-store",
            Self::Network => "network",
        }
    }

    /// The source in a person's words.
    pub const fn render(self) -> &'static str {
        match self {
            Self::ProjectFiles => "the project's own files",
            Self::ThisMachine => "this machine",
            Self::CredentialStore => "this machine's credential store",
            Self::Network => "a far end",
        }
    }

    /// Where the answer to a kind of need comes from.
    ///
    /// The mapping is total on purpose: a need cannot be given a source by hand, so a need cannot
    /// claim to be answered by the network and a need that is cannot be described as local.
    pub const fn of(kind: NeedKind) -> Self {
        match kind {
            NeedKind::File => Self::ProjectFiles,
            NeedKind::Toolchain | NeedKind::Target => Self::ThisMachine,
            NeedKind::Credential => Self::CredentialStore,
        }
    }
}

impl fmt::Display for Where {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.name())
    }
}

/// What kind of thing a project needs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NeedKind {
    /// A path the document names.
    File,
    /// A program a build runs.
    Toolchain,
    /// A target profile the project can be built for.
    Target,
    /// Material a service uses, by reference.
    Credential,
}

impl NeedKind {
    /// Every kind, in the order a report lists them.
    pub const ALL: [Self; 4] = [Self::File, Self::Toolchain, Self::Target, Self::Credential];

    /// The name a report uses.
    pub const fn name(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Toolchain => "toolchain",
            Self::Target => "target",
            Self::Credential => "credential",
        }
    }
}

impl fmt::Display for NeedKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.name())
    }
}

/// What was found about one need.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Availability {
    /// It is there.
    Present,
    /// It is not there, and what was looked at.
    Missing {
        /// What was looked for, and what was found instead.
        reason: String,
    },
    /// Nobody looked, and why not.
    NotChecked {
        /// What stopped the look.
        reason: String,
    },
    /// The answer is that this is not allowed, so nothing was looked for.
    Refused {
        /// Why, in the path layer's own words.
        reason: String,
    },
}

impl Availability {
    /// The name a scan record uses.
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Present => "present",
            Self::Missing { .. } => "missing",
            Self::NotChecked { .. } => "not-checked",
            Self::Refused { .. } => "refused",
        }
    }

    /// The word a report uses.
    pub const fn word(&self) -> &'static str {
        match self {
            Self::Present => "present",
            Self::Missing { .. } => "not there",
            Self::NotChecked { .. } => "not checked",
            Self::Refused { .. } => "refused",
        }
    }

    /// Whether whatever it is, is there.
    pub const fn is_present(&self) -> bool {
        matches!(self, Self::Present)
    }

    /// Whether it is known not to be there.
    pub const fn is_missing(&self) -> bool {
        matches!(self, Self::Missing { .. })
    }

    /// Whether nobody looked.
    pub const fn is_not_checked(&self) -> bool {
        matches!(self, Self::NotChecked { .. })
    }

    /// Whether it was refused.
    pub const fn is_refused(&self) -> bool {
        matches!(self, Self::Refused { .. })
    }

    /// Why, when there is a why.
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Present => None,
            Self::Missing { reason } | Self::NotChecked { reason } | Self::Refused { reason } => {
                Some(reason)
            }
        }
    }
}

impl fmt::Display for Availability {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.name())
    }
}

/// One thing a project needs, and what is known about it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Need {
    kind: NeedKind,
    subject: String,
    role: String,
    availability: Availability,
    note: Option<String>,
}

impl Need {
    /// What kind of need this is.
    pub const fn kind(&self) -> NeedKind {
        self.kind
    }

    /// What it is, as the document writes it.
    pub fn subject(&self) -> &str {
        &self.subject
    }

    /// What it is for.
    pub fn role(&self) -> &str {
        &self.role
    }

    /// What was found.
    pub const fn availability(&self) -> &Availability {
        &self.availability
    }

    /// Where the answer came from, which follows from the kind.
    pub const fn source(&self) -> Where {
        Where::of(self.kind)
    }

    /// Something else worth knowing, when there is something.
    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    /// One line: the kind, what it is, what was found, and what it is for.
    pub fn render(&self) -> String {
        let mut line = format!(
            "  {:<10} {:<34} [{:<11}] {}",
            self.kind.name(),
            self.subject,
            self.availability.word(),
            self.role
        );
        if let Some(reason) = self.availability.reason() {
            line.push_str(&format!(" — {reason}"));
        }
        if let Some(note) = self.note.as_deref() {
            line.push_str(&format!(" ({note})"));
        }
        line
    }
}

/// The far ends a project's **build product** reaches, with the credentials those use.
///
/// It is kept apart from the needs for one reason: opening the project asks none of these, so mixing
/// them in would say that designing this project needs a network. Their credentials are still
/// dependencies of a build, so they do appear among the needs — a missing token is a missing token
/// whether or not anybody is online.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Reach {
    services: Vec<String>,
    credentials: Vec<CredentialView>,
}

impl Reach {
    /// What a document declares, with what this machine's credential store says about each.
    pub fn of(document: &ProjectDocument, store: &dyn CredentialStore, now: SystemTime) -> Self {
        Self {
            services: document
                .services()
                .iter()
                .map(|service| service.policy().render())
                .collect(),
            credentials: configuration::credentials(document, store, now),
        }
    }

    /// The declared services, one line each.
    pub fn services(&self) -> &[String] {
        &self.services
    }

    /// The credentials those services use.
    pub fn credentials(&self) -> &[CredentialView] {
        &self.credentials
    }

    /// Whether the project declares nothing that reaches anywhere.
    pub fn is_empty(&self) -> bool {
        self.services.is_empty() && self.credentials.is_empty()
    }

    /// One line per thing the build product reaches.
    pub fn render(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for service in &self.services {
            lines.push(service.clone());
        }
        for view in &self.credentials {
            lines.push(view.render());
        }
        lines
    }
}

/// What a project needs, and what was found about each need.
///
/// Building one reads: the document that is already read, the toolchain state that was already
/// diagnosed, and the credential views that were already looked up. There is nothing here that
/// writes, so there is nothing here that can clear a document.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Validation {
    needs: Vec<Need>,
    reach: Reach,
}

impl Validation {
    /// Answers every need a document declares, from facts that were already gathered.
    ///
    /// A need is identified the way a scan identifies an item — what it is for, and how it is written
    /// — so a file that two members name is one need, the same way it is one item in a scan. A report
    /// that counted it twice would disagree with the scan of the same document about how much there
    /// is, and "four things are missing" when two are is exactly the arithmetic that hides a problem.
    pub fn of(document: &ProjectDocument, toolchain: &ToolchainState, reach: &Reach) -> Self {
        let mut needs: Vec<Need> = Vec::new();
        let mut add = |need: Need| {
            let kind = need.kind.name();
            if !needs
                .iter()
                .any(|there: &Need| there.kind.name() == kind && there.subject == need.subject)
            {
                needs.push(need);
            }
        };
        for reference in document.references() {
            add(need_of(reference));
        }
        for target in document.targets() {
            add(need_for_target(target));
        }
        for need in needs_of_toolchain(toolchain) {
            add(need);
        }
        for view in reach.credentials() {
            add(need_for_credential(view));
        }
        Self {
            needs,
            reach: reach.clone(),
        }
    }

    /// Every need, in the order a report lists them.
    pub fn needs(&self) -> &[Need] {
        &self.needs
    }

    /// The needs that are there.
    pub fn present(&self) -> Vec<&Need> {
        self.with(|availability| availability.is_present())
    }

    /// The needs that are not there.
    pub fn missing(&self) -> Vec<&Need> {
        self.with(|availability| availability.is_missing())
    }

    /// The needs nobody looked at.
    pub fn not_checked(&self) -> Vec<&Need> {
        self.with(|availability| availability.is_not_checked())
    }

    /// The needs that are refused.
    pub fn refused(&self) -> Vec<&Need> {
        self.with(|availability| availability.is_refused())
    }

    fn with(&self, wanted: impl Fn(&Availability) -> bool) -> Vec<&Need> {
        self.needs
            .iter()
            .filter(|need| wanted(&need.availability))
            .collect()
    }

    /// Whether nothing is missing and nothing was left unchecked.
    pub fn is_complete(&self) -> bool {
        self.missing().is_empty() && self.not_checked().is_empty() && self.refused().is_empty()
    }

    /// The sources the answers came from, each named once, in report order.
    pub fn answered_by(&self) -> Vec<Where> {
        Where::ALL
            .into_iter()
            .filter(|source| self.needs.iter().any(|need| need.source() == *source))
            .collect()
    }

    /// Whether none of the answers would need the network.
    pub fn needs_no_network(&self) -> bool {
        !self
            .needs
            .iter()
            .any(|need| need.source() == Where::Network)
    }

    /// What the build product reaches.
    pub const fn reach(&self) -> &Reach {
        &self.reach
    }

    /// One line per need, with the count and where the answers came from.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "needs: {} (present {}, missing {}, not checked {}, refused {})",
            self.needs.len(),
            self.present().len(),
            self.missing().len(),
            self.not_checked().len(),
            self.refused().len()
        )];
        for need in &self.needs {
            lines.push(need.render());
        }
        let sources: Vec<&str> = self.answered_by().into_iter().map(Where::render).collect();
        // The offline claim is stated on every run, and stated differently when it would not hold:
        // a line that says "yes" whatever happens is not evidence of anything.
        if self.needs_no_network() {
            lines.push(format!(
                "answers came from {}; no answer needs the network, so opening this project offline \
                 is the same path",
                joined(&sources)
            ));
        } else {
            let far = self
                .needs
                .iter()
                .filter(|need| need.source() == Where::Network)
                .count();
            lines.push(format!(
                "answers came from {}; {far} of them need the network, so this project cannot be \
                 opened the same way offline",
                joined(&sources)
            ));
        }
        if self.reach.is_empty() {
            lines.push("what this project builds reaches no far end".to_owned());
        } else {
            lines.push(
                "what this project builds reaches (opening it asks none of these):".to_owned(),
            );
            for line in self.reach.render() {
                lines.push(format!("  {line}"));
            }
        }
        lines
    }
}

/// What is known about the toolchain, as needs.
///
/// "Not read" and "not there" are kept apart: the first is a state somebody can change by answering
/// the trust question, the second is a machine without an SDK. Only the second is a missing
/// dependency.
fn needs_of_toolchain(toolchain: &ToolchainState) -> Vec<Need> {
    let role = "building this project".to_owned();
    match toolchain {
        ToolchainState::NotChecked(reason) => vec![Need {
            kind: NeedKind::Toolchain,
            subject: "the toolchain".to_owned(),
            role,
            availability: Availability::NotChecked {
                reason: reason.message(),
            },
            note: None,
        }],
        ToolchainState::Checked(report) => [("cargo", report.cargo()), ("rustc", report.rustc())]
            .into_iter()
            .map(|(program, version)| Need {
                kind: NeedKind::Toolchain,
                subject: program.to_owned(),
                role: role.clone(),
                availability: match version {
                    Some(_) => Availability::Present,
                    // The reason is the check's own typed finding, not a sentence written here: the
                    // report knows whether the program is not installed, ran and said nothing, or
                    // hung, and each of those is a different next step.
                    None => Availability::Missing {
                        reason: report
                            .unavailable(program)
                            .map(ToolUnavailable::line)
                            .unwrap_or_else(|| {
                                format!("`{program}` was run and reported no version")
                            }),
                    },
                },
                // The line the program printed is the evidence for the version, so it is kept as
                // the note rather than thrown away after being read once.
                note: version.map(|version| version.line().to_owned()),
            })
            .collect(),
    }
}

fn need_for_target(target: &TargetProfile) -> Need {
    let mut note = Vec::new();
    let availability = match target.profile() {
        Some(profile) => {
            note.push(format!("profile {profile}"));
            Availability::Present
        }
        None => Availability::Missing {
            reason: "the target declares no profile, so nothing says what it builds for".to_owned(),
        },
    };
    if !target.features().is_empty() {
        note.push(format!("features {}", target.features().join(" ")));
    }
    Need {
        kind: NeedKind::Target,
        subject: target.name().to_owned(),
        role: "a target this project can be built for".to_owned(),
        availability,
        note: (!note.is_empty()).then(|| note.join(", ")),
    }
}

fn need_for_credential(view: &CredentialView) -> Need {
    let (availability, note) = availability_of(view.health());
    Need {
        kind: NeedKind::Credential,
        subject: view.name().to_owned(),
        role: format!(
            "the credential service `{}` uses, from {}",
            view.service(),
            view.source().render()
        ),
        availability,
        note,
    }
}

/// What a store's answer means for a need.
///
/// A store that cannot be reached is `not-checked`, not `missing`: the difference between "the token
/// is not set" and "this machine has no keyring" decides whether somebody adds a variable or fixes a
/// machine, so the two are not folded together.
fn availability_of(health: &CredentialHealth) -> (Availability, Option<String>) {
    match health {
        CredentialHealth::Usable => (Availability::Present, None),
        CredentialHealth::ExpiringSoon { within } => (
            Availability::Present,
            Some(format!(
                "renew it: it expires within about {} day(s)",
                within.as_secs() / (24 * 60 * 60)
            )),
        ),
        CredentialHealth::Expired { since } => (
            Availability::Missing {
                reason: format!(
                    "it expired about {} hour(s) ago",
                    since.as_secs() / (60 * 60)
                ),
            },
            None,
        ),
        CredentialHealth::Missing { reason } => (
            Availability::Missing {
                reason: reason.clone(),
            },
            None,
        ),
        CredentialHealth::Unavailable { reason } => (
            Availability::NotChecked {
                reason: format!("the store could not be reached: {reason}"),
            },
            None,
        ),
    }
}

fn need_of(reference: &Reference) -> Need {
    let availability = current_state(reference);
    // The document holds what the *read* found, and this holds what is there now. They can differ
    // when the disk changed in between, and a report that hid that would be presenting an old
    // answer as the current one.
    let note = match reference.verdict() {
        ReferenceVerdict::Refused { .. } => None,
        verdict if agrees(verdict, &availability) => None,
        verdict => Some(format!(
            "the read said {}, and that is not what is there now",
            verdict.render()
        )),
    };
    Need {
        kind: NeedKind::File,
        subject: reference.written().to_owned(),
        role: role_of(reference.kind()).to_owned(),
        availability,
        note,
    }
}

/// Whether the document's own verdict and what is there now are the same finding.
fn agrees(verdict: &ReferenceVerdict, availability: &Availability) -> bool {
    match verdict {
        ReferenceVerdict::Present => availability.is_present(),
        ReferenceVerdict::Absent => availability.is_missing(),
        ReferenceVerdict::Refused { .. } => availability.is_refused(),
    }
}

fn role_of(kind: ReferenceKind) -> &'static str {
    match kind {
        ReferenceKind::PageDocument => "a page document",
        ReferenceKind::Stylesheet => "a stylesheet of that page",
        ReferenceKind::Resource => "a project file",
        ReferenceKind::GeneratedOutput => "a file this build writes",
        ReferenceKind::DesignSource => "the design a generated file comes from",
        ReferenceKind::CertificateAuthority => "a certificate authority the network policy names",
    }
}

/// What is at a reference's resolved path, asked now rather than remembered.
fn current_state(reference: &Reference) -> Availability {
    if let ReferenceVerdict::Refused { reason } = reference.verdict() {
        return Availability::Refused {
            reason: reason.clone(),
        };
    }
    let Some(path) = reference.resolved() else {
        return Availability::NotChecked {
            reason: "the document's read did not resolve this path".to_owned(),
        };
    };
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => Availability::Present,
        Ok(_) => Availability::Missing {
            reason: format!("`{}` is there, but it is not a file", path.display()),
        },
        Err(error) if error.kind() == ErrorKind::NotFound => Availability::Missing {
            reason: format!("there is nothing at `{}`", path.display()),
        },
        // A path that could not be looked at is not a path that is not there: a permission refusal
        // reported as "missing" would send somebody looking for a file that may be right there.
        Err(error) => Availability::NotChecked {
            reason: format!("`{}` could not be looked at: {error}", path.display()),
        },
    }
}

/// The format of a recorded scan. It is not the project's format: a record is this host's own file.
pub const SCAN_FORMAT: FormatVersion = FormatVersion::from_raw(1);

/// The members a recorded scan has, in the order it is written.
///
/// `format` is the record's own version, and `document` is the version of the project it scanned:
/// two different questions, so they are two different members with two different answers.
pub const SCAN_MEMBERS: [&str; 5] = ["format", "project", "name", "document", "items"];

/// The members one scanned reference has.
pub const SCANNED_MEMBERS: [&str; 3] = ["kind", "written", "state"];

/// One reference, as a scan saw it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Scanned {
    kind: String,
    written: String,
    state: String,
}

impl Scanned {
    /// What it is for, as the document's reader classified it.
    pub fn kind(&self) -> &str {
        &self.kind
    }

    /// How the document writes it.
    pub fn written(&self) -> &str {
        &self.written
    }

    /// What was found, as [`Availability::name`] spells it.
    pub fn state(&self) -> &str {
        &self.state
    }

    /// What identifies it across two scans: **the file**, as the document writes it.
    ///
    /// The state is a property of the path and not of the member that mentions it, so two members
    /// naming one file are one item — and a rewritten path is a different one, because a reference
    /// that moved is not the same reference with a new state.
    pub fn key(&self) -> &str {
        &self.written
    }

    /// What it is called in a line: one of the ways the document names it, and the path.
    ///
    /// It is the first way the document names it, in the document's own order, which is the order its
    /// members are written in.
    pub fn label(&self) -> String {
        format!("{} {}", self.kind, self.written)
    }

    /// One line, the way a scan record reads.
    pub fn render(&self) -> String {
        format!("{:<21} {:<40} {}", self.kind, self.written, self.state)
    }

    fn to_json(&self) -> Json {
        let mut out = JsonObject::new();
        out.insert("kind", Json::string(&self.kind));
        out.insert("written", Json::string(&self.written));
        out.insert("state", Json::string(&self.state));
        Json::Object(out)
    }
}

/// What every reference in a document resolved to, at one moment.
///
/// A scan is a record, not a cache: it is written to a path the caller names, and reading it back
/// never makes it the current answer — [`Scan::read`] answers that, by reading the file again.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Scan {
    project: String,
    name: String,
    document: u32,
    items: Vec<Scanned>,
}

impl Scan {
    /// What a project's references resolve to, asked now.
    ///
    /// It takes the file rather than the document because **the version a file declares is not the
    /// version a document value carries**: a version 1 file is interpreted as the version this build
    /// writes, and `migrated_from` is what remembers what the file said. A scan that recorded the
    /// document's own version would report every old file as a current one.
    ///
    /// Two members can name the same file — the startup entry is a page document that is already in
    /// `pages`, and a generated file is built from a page that is listed — and a scan records what is
    /// **there**, not how many members mention it. Counting it twice would make the same file appear
    /// and vanish twice in a comparison, and would let a comparison pair one mention with another
    /// mention of itself. The kind kept is the first way the document names it, in document order.
    pub fn of(file: &ProjectFile) -> Result<Self, ProjectError> {
        let Some(document) = file.document() else {
            return Err(ProjectError::Inconsistent {
                path: "the project document".to_owned(),
                reason: format!(
                    "it is {}, so nothing in it can be scanned",
                    file.verdict().message()
                ),
            });
        };
        let mut items: Vec<Scanned> = Vec::new();
        for reference in document.references() {
            let scanned = Scanned {
                kind: reference.kind().name().to_owned(),
                written: reference.written().to_owned(),
                state: current_state(reference).name().to_owned(),
            };
            if !items.iter().any(|item| item.key() == scanned.key()) {
                items.push(scanned);
            }
        }
        Ok(Self {
            project: document.identity().id().to_owned(),
            name: document.identity().name().to_owned(),
            document: file.migrated_from().unwrap_or_else(|| file.version()).get(),
            items,
        })
    }

    /// Reads the project from disk and scans it, without taking anybody's word for the state.
    pub fn read(path: impl AsRef<Path>) -> Result<Self, ProjectError> {
        let file = ProjectFile::open(path)?;
        Self::of(&file)
    }

    /// Which project this is a scan of.
    pub fn project(&self) -> &str {
        &self.project
    }

    /// The project's name, which is what a person recognises.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The project document's format version when it was scanned.
    pub fn document(&self) -> u32 {
        self.document
    }

    /// Every reference, in document order.
    pub fn items(&self) -> &[Scanned] {
        &self.items
    }

    /// What a scan is, one line per reference.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "scan of {} ({}) at format v{}, {} reference(s)",
            self.name,
            self.project,
            self.document,
            self.items.len()
        )];
        for item in &self.items {
            lines.push(format!("  {}", item.render()));
        }
        lines
    }

    /// What changed between this scan and a later one.
    ///
    /// A scan of another project is refused rather than compared: two projects' `pages/main.json`
    /// have the same name and are not the same file, and reporting one as the other is exactly the
    /// kind of cross-project leak a machine's own state is kept apart for.
    pub fn between(&self, later: &Scan) -> Result<Rescan, RescanRefusal> {
        if self.project != later.project {
            return Err(RescanRefusal::AnotherProject {
                expected: format!("{} ({})", self.name, self.project),
                found: format!("{} ({})", later.name, later.project),
            });
        }
        let mut changes = Vec::new();
        let mut unchanged = 0usize;
        let mut seen = vec![false; later.items.len()];
        for before in &self.items {
            let found = later
                .items
                .iter()
                .position(|after| after.key() == before.key());
            match found {
                Some(index) => {
                    seen[index] = true;
                    let after = &later.items[index];
                    if after.state == before.state {
                        unchanged += 1;
                    } else {
                        changes.push(Change {
                            kind: ChangeKind::State,
                            item: before.label(),
                            from: Some(before.state.clone()),
                            to: Some(after.state.clone()),
                        });
                    }
                }
                None => changes.push(Change {
                    kind: ChangeKind::Gone,
                    item: before.label(),
                    from: Some(before.state.clone()),
                    to: None,
                }),
            }
        }
        for (index, after) in later.items.iter().enumerate() {
            if !seen[index] {
                changes.push(Change {
                    kind: ChangeKind::New,
                    item: after.label(),
                    from: None,
                    to: Some(after.state.clone()),
                });
            }
        }
        Ok(Rescan {
            project: self.project.clone(),
            document: (self.document, later.document),
            changes,
            unchanged,
        })
    }

    /// The record as JSON.
    pub fn to_json(&self) -> Json {
        let mut out = JsonObject::new();
        out.insert("format", Json::number(u64::from(SCAN_FORMAT.get())));
        out.insert("project", Json::string(&self.project));
        out.insert("name", Json::string(&self.name));
        out.insert("document", Json::number(u64::from(self.document)));
        out.insert(
            "items",
            Json::array(self.items.iter().map(Scanned::to_json).collect::<Vec<_>>()),
        );
        Json::Object(out)
    }

    /// Writes the record to a path, through a temporary file and a rename.
    ///
    /// This is the host's own artifact, written where the caller says, and it is never the project
    /// document: the caller refuses that path before getting here.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<PathBuf, ProjectError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|error| ProjectError::Io {
                path: parent.to_path_buf(),
                action: "creating the directory the scan record goes in",
                reason: error.to_string(),
            })?;
        }
        json::write_atomically(&path, &self.to_json().render()).map_err(|error| {
            ProjectError::Io {
                path: path.clone(),
                action: "writing the scan record",
                reason: error.to_string(),
            }
        })?;
        Ok(path)
    }

    /// Reads a record written by [`Scan::save`].
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ProjectError> {
        let path = path.as_ref().to_path_buf();
        let text = std::fs::read_to_string(&path).map_err(|error| ProjectError::Io {
            path: path.clone(),
            action: "reading the scan record to compare against",
            reason: error.to_string(),
        })?;
        let value = json::parse(&text).map_err(|error| ProjectError::NotJson {
            path: path.clone(),
            error,
        })?;
        let version = format_version(&value)?;
        match version.get().cmp(&SCAN_FORMAT.get()) {
            Ordering::Equal => {}
            Ordering::Greater => {
                return Err(ProjectError::NewerFormat {
                    found: version,
                    current: SCAN_FORMAT,
                });
            }
            Ordering::Less => {
                return Err(ProjectError::OlderFormat {
                    found: version,
                    current: SCAN_FORMAT,
                });
            }
        }
        let Some(object) = value.as_object() else {
            return Err(ProjectError::WrongType {
                path: "the scan record".to_owned(),
                expected: "an object",
                found: value.kind(),
            });
        };
        let Some(items) = object.get("items").and_then(Json::as_array) else {
            return Err(ProjectError::MissingMember {
                path: "`items` in the scan record".to_owned(),
            });
        };
        let mut scanned = Vec::new();
        for (index, item) in items.iter().enumerate() {
            let Some(fields) = item.as_object() else {
                return Err(ProjectError::WrongType {
                    path: format!("items[{index}]"),
                    expected: "an object",
                    found: item.kind(),
                });
            };
            scanned.push(Scanned {
                kind: member_text(fields, "kind")?,
                written: member_text(fields, "written")?,
                state: member_text(fields, "state")?,
            });
        }
        Ok(Self {
            project: member_text(object, "project")?,
            name: member_text(object, "name")?,
            document: u32::try_from(member_number(object, "document")?).map_err(|_| {
                ProjectError::WrongType {
                    path: "`document` in the scan record".to_owned(),
                    expected: "a format version",
                    found: JsonKind::Number,
                }
            })?,
            items: scanned,
        })
    }
}

/// Why a path cannot hold a scan record for a project.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RecordRefusal {
    /// It is the project document.
    TheProjectDocument {
        /// The path.
        path: PathBuf,
    },
    /// It is a file the document declares.
    AFileTheProjectDeclares {
        /// The path.
        path: PathBuf,
        /// How the document writes it.
        written: String,
    },
}

impl RecordRefusal {
    /// A sentence for a person.
    pub fn message(&self) -> String {
        match self {
            Self::TheProjectDocument { path } => format!(
                "{} is the project document itself; a scan record goes to its own file",
                path.display()
            ),
            Self::AFileTheProjectDeclares { path, written } => format!(
                "{} is `{written}`, which this project declares; recording over it would destroy a \
                 file to write a report about it",
                path.display()
            ),
        }
    }
}

impl fmt::Display for RecordRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.message())
    }
}

impl std::error::Error for RecordRefusal {}

/// Whether a path can hold a project's scan record.
///
/// A record is this host's artifact about a project, so it is not part of the project and there are
/// two places it must never be written: the project document, and a file the document declares.
/// Overwriting either would destroy the project in order to write a report about it, and both are
/// one `canonicalize` away from looking like an ordinary path.
pub fn may_record_at(file: &ProjectFile, path: &Path) -> Result<(), RecordRefusal> {
    if same_file(path, file.path()) {
        return Err(RecordRefusal::TheProjectDocument {
            path: path.to_path_buf(),
        });
    }
    for reference in file
        .document()
        .map(ProjectDocument::references)
        .unwrap_or_default()
    {
        let Some(resolved) = reference.resolved() else {
            continue;
        };
        if same_file(path, resolved) {
            return Err(RecordRefusal::AFileTheProjectDeclares {
                path: path.to_path_buf(),
                written: reference.written().to_owned(),
            });
        }
    }
    Ok(())
}

/// Whether two paths name the same file, including when one of them does not exist yet.
///
/// `canonicalize` answers for a file that is there; for the one about to be written the directory is
/// resolved and the name is kept, which is what catches `demo/project.json` being asked for as
/// `./demo/../demo/project.json`.
fn same_file(a: &Path, b: &Path) -> bool {
    fn resolved(path: &Path) -> PathBuf {
        if let Ok(canonical) = std::fs::canonicalize(path) {
            return canonical;
        }
        match (path.parent(), path.file_name()) {
            (Some(parent), Some(name)) => match std::fs::canonicalize(parent) {
                Ok(parent) => parent.join(name),
                Err(_) => path.to_path_buf(),
            },
            _ => path.to_path_buf(),
        }
    }
    resolved(a) == resolved(b)
}

fn member_text(object: &JsonObject, member: &str) -> Result<String, ProjectError> {
    object
        .get(member)
        .and_then(Json::as_str)
        .map(str::to_owned)
        .ok_or_else(|| ProjectError::MissingMember {
            path: format!("`{member}` in the scan record"),
        })
}

fn member_number(object: &JsonObject, member: &str) -> Result<u64, ProjectError> {
    object
        .get(member)
        .and_then(Json::as_u64)
        .ok_or_else(|| ProjectError::MissingMember {
            path: format!("`{member}` in the scan record"),
        })
}

/// How one item changed between two scans.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChangeKind {
    /// It is in the newer scan and was not in the older one.
    New,
    /// It was in the older scan and is not in the newer one.
    Gone,
    /// It is in both, and what was found about it is not the same.
    State,
}

impl ChangeKind {
    /// The name a report uses.
    pub const fn name(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Gone => "gone",
            Self::State => "state",
        }
    }
}

impl fmt::Display for ChangeKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.name())
    }
}

/// One item that is not the same in two scans.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Change {
    kind: ChangeKind,
    item: String,
    from: Option<String>,
    to: Option<String>,
}

impl Change {
    /// How it changed.
    pub const fn kind(&self) -> ChangeKind {
        self.kind
    }

    /// What it is: what it is for, and how it is written.
    pub fn item(&self) -> &str {
        &self.item
    }

    /// What was found before, when it was there before.
    pub fn from(&self) -> Option<&str> {
        self.from.as_deref()
    }

    /// What is found now, when it is there now.
    pub fn to(&self) -> Option<&str> {
        self.to.as_deref()
    }

    /// One line.
    pub fn render(&self) -> String {
        match (self.from.as_deref(), self.to.as_deref()) {
            (None, Some(to)) => format!("  {:<6} {} ({to})", self.kind.name(), self.item),
            (Some(from), None) => format!("  {:<6} {} (was {from})", self.kind.name(), self.item),
            (Some(from), Some(to)) => {
                format!("  {:<6} {} ({from} → {to})", self.kind.name(), self.item)
            }
            // A change always has one side or the other; an item that is neither new, gone nor
            // different is not a change and never reaches this value.
            (None, None) => format!("  {:<6} {}", self.kind.name(), self.item),
        }
    }
}

/// What a re-scan found: what changed, and how much did not.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Rescan {
    project: String,
    document: (u32, u32),
    changes: Vec<Change>,
    unchanged: usize,
}

impl Rescan {
    /// What is not the same since the scan that was recorded.
    pub fn changes(&self) -> &[Change] {
        &self.changes
    }

    /// How many references are exactly as they were.
    pub const fn unchanged(&self) -> usize {
        self.unchanged
    }

    /// Whether nothing changed.
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty() && self.same_document()
    }

    /// Whether the document itself is the format version it was when scanned.
    pub const fn same_document(&self) -> bool {
        self.document.0 == self.document.1
    }

    /// The project the two scans were of.
    pub fn project(&self) -> &str {
        &self.project
    }

    /// One line per change, then how much did not change.
    ///
    /// "Nothing changed" is said out loud rather than left as an empty list: a re-scan that prints
    /// nothing is indistinguishable from one that did not run.
    pub fn render(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if !self.same_document() {
            lines.push(format!(
                "the document itself changed: it was format v{} when scanned, and v{} now",
                self.document.0, self.document.1
            ));
        }
        if self.changes.is_empty() {
            lines.push("since the scan that was recorded, nothing changed".to_owned());
        } else {
            lines.push(format!(
                "since the scan that was recorded, {} changed:",
                self.changes.len()
            ));
            for change in &self.changes {
                lines.push(change.render());
            }
        }
        lines.push(format!("{} reference(s) are as they were", self.unchanged));
        lines
    }
}

/// Why two scans could not be compared.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RescanRefusal {
    /// The recorded scan is of another project.
    AnotherProject {
        /// The project the scan is for, and the one being looked at.
        expected: String,
        /// The project the record is of.
        found: String,
    },
}

impl RescanRefusal {
    /// A sentence for a person.
    pub fn message(&self) -> String {
        match self {
            Self::AnotherProject { expected, found } => format!(
                "the recorded scan is of {found}, and this project is {expected}; a scan of one \
                 project is not a baseline for another, so nothing was compared"
            ),
        }
    }
}

impl fmt::Display for RescanRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.message())
    }
}

impl std::error::Error for RescanRefusal {}

/// A list in a person's words: `a and b`.
fn joined(words: &[&str]) -> String {
    match words {
        [] => "nowhere".to_owned(),
        [one] => (*one).to_owned(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
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
        Availability, ChangeKind, Need, Reach, RescanRefusal, SCAN_FORMAT, Scan, Validation, Where,
    };
    use crate::host::network::{EnvironmentStore, UnavailableStore};
    use crate::host::permission::PermissionService;
    use crate::project::lifecycle::session::{
        NotChecked, ProjectSession, ToolUnavailable, ToolchainReport, ToolchainState,
    };
    use crate::project::record::format::ProjectFile;

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
            std::env::temp_dir().join(format!("rustage-validate-{}-{id}", std::process::id()));
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

        fn project(&self, name: &str, text: &str) -> ProjectFile {
            let path = self.write(&format!("{name}/project.json"), text);
            ProjectFile::open(&path).expect("the project opens")
        }
    }

    /// A project that names more than is there: one page document that exists, one that does not, a
    /// stylesheet that does not, a resource that does not, a target with a profile and one without,
    /// and a service whose credential comes from the environment.
    ///
    /// Nothing here leaves the root, because a document that names a path outside it is not writable
    /// at all — the reader refuses to write back a reference it had to refuse (see [`ESCAPING`]).
    const PROJECT: &str = "{\"format\": 2, \"project\": {\"id\": \"3f2b\", \"name\": \"Demo\"}, \
         \"pages\": [{\"document\": \"pages/main.json\", \"stylesheet\": \"styles/app.css\"}, \
                     {\"document\": \"pages/gone.json\"}], \
         \"resources\": [{\"path\": \"assets/icon.svg\"}], \
         \"targets\": [{\"name\": \"desktop\", \"profile\": \"debug\", \"features\": [\"desktop\"]}, \
                       {\"name\": \"quiet\"}], \
         \"generated\": [{\"output\": \"src/generated/main.rs\", \"from\": \"pages/main.json\"}], \
         \"startup\": \"pages/main.json\", \
         \"network\": {\"services\": [{\"name\": \"build-agent\", \"host\": \"agent.example\", \
            \"port\": 8443, \"tls\": {\"roots\": \"system\"}, \
            \"credential\": {\"name\": \"RUSTAGE_T08_TOKEN_UNSET\", \
              \"source\": {\"kind\": \"environment\", \"variable\": \"RUSTAGE_T08_TOKEN_UNSET\"}}}]}}";

    /// The same project, with a resource that climbs out of the root.
    const ESCAPING: &str = "{\"format\": 2, \"project\": {\"id\": \"3f2b\", \"name\": \"Demo\"}, \
         \"pages\": [{\"document\": \"pages/main.json\"}], \
         \"resources\": [{\"path\": \"../outside.png\"}], \
         \"targets\": [{\"name\": \"desktop\", \"profile\": \"debug\"}], \
         \"generated\": [], \
         \"network\": {\"services\": []}}";

    fn at(seconds: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(seconds)
    }

    fn validate(file: &ProjectFile, toolchain: &ToolchainState) -> Validation {
        let document = file.document().expect("a document this build reads");
        let reach = Reach::of(document, &EnvironmentStore, at(1_000));
        Validation::of(document, toolchain, &reach)
    }

    #[test]
    fn validating_a_project_with_nothing_present_is_a_report_and_not_an_edit() {
        let sandbox = sandbox();
        let file = sandbox.project("demo", PROJECT);
        let path = file.path().to_path_buf();
        let before = std::fs::read(&path).expect("the document is there");
        // Nothing is on disk except the page document the project declares, so the missing things
        // are a missing resource, a missing stylesheet and a missing SDK — the acceptance's case.
        sandbox.write("demo/pages/main.json", "{}");
        let validation = validate(
            &file,
            &ToolchainState::Checked(ToolchainReport::from_checks(
                Err(ToolUnavailable::NotInstalled {
                    program: "cargo".to_owned(),
                    reason: "No such file or directory (os error 2)".to_owned(),
                }),
                Err(ToolUnavailable::NotInstalled {
                    program: "rustc".to_owned(),
                    reason: "No such file or directory (os error 2)".to_owned(),
                }),
                vec!["cargo --version".to_owned(), "rustc --version".to_owned()],
            )),
        );
        assert!(
            !validation.missing().is_empty(),
            "a project with nothing present has missing needs: {:?}",
            validation.render()
        );
        let report = validation.render().join("\n");
        assert!(report.contains("assets/icon.svg"), "{report}");
        assert!(report.contains("styles/app.css"), "{report}");

        // The document is byte for byte what it was: validating is not an edit, so it cannot have
        // cleared anything.
        assert_eq!(
            std::fs::read(&path).expect("still there"),
            before,
            "validation wrote the document"
        );

        // And it is still editable: the ordinary edit path adds a resource, and everything the
        // document had is still in it afterwards.
        let document = file.document().expect("a document").clone();
        let root = file.root().clone();
        sandbox.write("demo/assets/added.svg", "<svg/>");
        let edited = document
            .with_resource(&root, "assets/added.svg")
            .expect("a resource inside the root is addable");
        assert_eq!(edited.pages().len(), 2);
        assert_eq!(edited.targets().len(), 2);
        assert_eq!(edited.services().len(), 1);
        assert_eq!(edited.resources().len(), 2);
        assert_eq!(edited.identity().id(), "3f2b");

        let written = file
            .write_document(&edited)
            .expect("the edit is written through the same path a plain save uses");
        // What is read back is compared member by member, not as a whole value: a reference's verdict
        // is what the *read* found, and the fresh read of the page document that was written in the
        // meantime legitimately says `present` where this in-memory document still says `absent`.
        // The written file carries the members, not the verdicts.
        let again = ProjectFile::open(written.target()).expect("the edit reads back");
        let after = again.document().expect("a document");
        assert_eq!(after.identity().id(), "3f2b");
        assert_eq!(after.pages().len(), 2);
        assert_eq!(after.targets().len(), 2);
        assert_eq!(after.services().len(), 1);
        assert_eq!(after.generated().len(), 1);
        assert_eq!(after.startup_written(), Some("pages/main.json"));
        assert_eq!(after.resources().len(), 2);
        assert!(
            after
                .resources()
                .iter()
                .any(|resource| resource.path().written() == "assets/added.svg")
        );
    }

    #[test]
    fn not_looking_at_the_toolchain_is_not_the_same_finding_as_a_missing_sdk() {
        let sandbox = sandbox();
        let file = sandbox.project("demo", PROJECT);

        let unasked = validate(&file, &ToolchainState::NotChecked(NotChecked::NotAsked));
        assert_eq!(unasked.not_checked().len(), 1);
        assert!(
            unasked
                .missing()
                .iter()
                .all(|need| need.kind().name() != "toolchain"),
            "an unread toolchain is not a missing toolchain: {:?}",
            unasked.render()
        );
        assert!(unasked.render().join("\n").contains("not checked yet"));

        let untrusted = validate(
            &file,
            &ToolchainState::NotChecked(NotChecked::ProjectNotTrusted),
        );
        assert_eq!(untrusted.not_checked().len(), 1);

        // A report where the program is not there is a missing SDK, and the reason is the check's
        // own typed finding rather than a sentence written here.
        let checked = ToolchainState::Checked(ToolchainReport::from_checks(
            Err(ToolUnavailable::NotInstalled {
                program: "cargo".to_owned(),
                reason: "No such file or directory (os error 2)".to_owned(),
            }),
            Err(ToolUnavailable::DidNotAnswer {
                program: "rustc".to_owned(),
                exit: "exit code 1".to_owned(),
                said: "error: no version here".to_owned(),
            }),
            vec![
                "cargo --version".to_owned(),
                "rustc --version".to_owned(),
                "cargo: not installed (No such file or directory (os error 2))".to_owned(),
                "rustc: ran and did not answer with a version (exit code 1; it said `error: no \
                 version here`)"
                    .to_owned(),
            ],
        ));
        let missing = validate(&file, &checked);
        let sdk: Vec<&str> = missing
            .missing()
            .iter()
            .filter(|need| need.kind().name() == "toolchain")
            .map(|need| need.subject())
            .collect();
        assert_eq!(sdk, vec!["cargo", "rustc"]);
        // The two cases are told apart in the report a person reads, which is the whole point of
        // the typed finding: one is a machine to fix, the other a wrapper to look at.
        let report = missing.render().join("\n");
        assert!(
            report.contains("cargo: not installed (No such file or directory (os error 2))"),
            "{report}"
        );
        assert!(
            report.contains("rustc: ran and did not answer with a version"),
            "{report}"
        );
    }

    #[test]
    fn an_sdk_this_machine_has_is_present_and_carries_the_line_it_printed() {
        let sandbox = sandbox();
        let directory = sandbox.write("demo/project.json", PROJECT);
        let permissions = PermissionService::new();
        let mut session = ProjectSession::open(&directory).expect("readable");
        session.decide(
            &permissions,
            crate::project::lifecycle::session::TrustDecision::Trust,
        );
        let state = session.diagnose_toolchain(&permissions).clone();
        let file = ProjectFile::open(&directory).expect("readable");

        let validation = validate(&file, &state);
        let present: Vec<&str> = validation
            .present()
            .iter()
            .filter(|need| need.kind().name() == "toolchain")
            .map(|need| need.subject())
            .collect();
        assert_eq!(
            present,
            vec!["cargo", "rustc"],
            "this machine has a Rust toolchain: {:?}",
            validation.render()
        );
        let note = validation
            .present()
            .iter()
            .find(|need| need.kind().name() == "toolchain")
            .and_then(|need| need.note())
            .expect("the version line is kept as evidence")
            .to_owned();
        assert!(note.contains("cargo"), "{note}");
    }

    #[test]
    fn a_file_two_members_name_is_one_need_the_way_it_is_one_item_in_a_scan() {
        let sandbox = sandbox();
        let file = sandbox.project("demo", PROJECT);
        let scan = Scan::of(&file).expect("scannable");
        let validation = validate(&file, &ToolchainState::NotChecked(NotChecked::NotAsked));

        // Seven references, and `pages/main.json` is three of them: the page, the design the
        // generated file comes from, and the startup entry.
        assert_eq!(file.document().unwrap().references().len(), 7);
        let named: Vec<&Need> = validation
            .needs()
            .iter()
            .filter(|need| need.subject() == "pages/main.json")
            .collect();
        assert_eq!(named.len(), 1, "one file, so one need");
        assert_eq!(named[0].role(), "a page document");
        assert_eq!(
            scan.items()
                .iter()
                .filter(|item| item.key() == "pages/main.json")
                .count(),
            1,
            "the scan counts the same thing the same way"
        );
        // Five files, two targets, one toolchain and the service's credential: the report and the
        // scan agree about how much there is, so neither can report "four things are missing" when
        // two are.
        assert_eq!(validation.needs().len(), 9);
        assert_eq!(
            validation.present().len()
                + validation.missing().len()
                + validation.not_checked().len()
                + validation.refused().len(),
            9
        );
    }

    #[test]
    fn a_reference_that_leaves_the_root_is_refused_rather_than_missing() {
        let sandbox = sandbox();
        let file = sandbox.project("demo", ESCAPING);
        let validation = validate(&file, &ToolchainState::NotChecked(NotChecked::NotAsked));
        let refused = validation.refused();
        assert_eq!(refused.len(), 1, "{:?}", validation.render());
        assert_eq!(refused[0].subject(), "../outside.png");
        assert_eq!(refused[0].source(), Where::ProjectFiles);
        assert!(
            refused[0]
                .availability()
                .reason()
                .expect("a refusal says why")
                .contains("climbs above the project root"),
            "{:?}",
            refused[0]
        );
        assert!(matches!(
            refused[0].availability(),
            Availability::Refused { .. }
        ));
    }

    #[test]
    fn every_answer_says_where_it_came_from_and_none_of_them_needs_the_network() {
        let sandbox = sandbox();
        let file = sandbox.project("demo", PROJECT);
        let validation = validate(&file, &ToolchainState::NotChecked(NotChecked::NotAsked));

        assert_eq!(
            validation.answered_by(),
            vec![
                Where::ProjectFiles,
                Where::ThisMachine,
                Where::CredentialStore
            ]
        );
        assert!(validation.needs_no_network());
        assert!(
            validation
                .needs()
                .iter()
                .all(|need| need.source() != Where::Network)
        );
        // A far end is a source this report can name, and no kind of need is answered by one: that
        // is the whole of the offline claim, and it is checked here rather than promised in prose.
        assert!(Where::ALL.contains(&Where::Network));
        for kind in super::NeedKind::ALL {
            assert_ne!(Where::of(kind), Where::Network);
        }
        assert_eq!(Where::of(super::NeedKind::File), Where::ProjectFiles);
    }

    #[test]
    fn what_the_project_builds_reaches_is_listed_apart_from_what_opening_it_needs() {
        let sandbox = sandbox();
        let file = sandbox.project("demo", PROJECT);
        let validation = validate(&file, &ToolchainState::NotChecked(NotChecked::NotAsked));

        assert_eq!(validation.reach().services().len(), 1);
        assert!(validation.reach().services()[0].contains("agent.example"));
        // The credential of that service is a dependency, so it is a need; whether it is there does
        // not change whether the project can be opened.
        let credential = validation
            .needs()
            .iter()
            .find(|need| need.kind().name() == "credential")
            .expect("the service's credential is a need");
        assert_eq!(credential.subject(), "RUSTAGE_T08_TOKEN_UNSET");
        assert!(credential.availability().is_missing());
        assert!(credential.source() == Where::CredentialStore);
        let rendered = validation.render().join("\n");
        assert!(
            rendered.contains("what this project builds reaches"),
            "{rendered}"
        );
        assert!(
            rendered.contains("environment variable is not set"),
            "{rendered}"
        );
    }

    #[test]
    fn a_credential_store_that_cannot_be_reached_is_not_a_missing_credential() {
        let sandbox = sandbox();
        let keyring = PROJECT.replace(
            "{\"kind\": \"environment\", \"variable\": \"RUSTAGE_T08_TOKEN_UNSET\"}",
            "{\"kind\": \"system-keyring\"}",
        );
        let file = sandbox.project("demo", &keyring);
        let document = file.document().expect("a document");
        let reach = Reach::of(
            document,
            &UnavailableStore::new("this machine has no keyring"),
            at(1_000),
        );
        let validation = Validation::of(
            document,
            &ToolchainState::NotChecked(NotChecked::NotAsked),
            &reach,
        );
        let credential = validation
            .needs()
            .iter()
            .find(|need| need.kind().name() == "credential")
            .expect("a need");
        assert!(credential.availability().is_not_checked());
        assert!(
            credential
                .availability()
                .reason()
                .expect("a reason")
                .contains("no keyring"),
            "{:?}",
            credential
        );
    }

    #[test]
    fn a_target_without_a_profile_says_so_and_the_others_are_present() {
        let sandbox = sandbox();
        let file = sandbox.project("demo", PROJECT);
        let validation = validate(&file, &ToolchainState::NotChecked(NotChecked::NotAsked));
        let targets: Vec<&str> = validation
            .needs()
            .iter()
            .filter(|need| need.kind().name() == "target")
            .map(|need| need.subject())
            .collect();
        assert_eq!(targets, vec!["desktop", "quiet"]);
        let quiet = validation
            .needs()
            .iter()
            .find(|need| need.subject() == "quiet")
            .expect("a need");
        assert!(quiet.availability().is_missing());
        assert!(
            quiet
                .availability()
                .reason()
                .expect("a reason")
                .contains("declares no profile"),
            "{quiet:?}"
        );
        let desktop = validation
            .needs()
            .iter()
            .find(|need| need.subject() == "desktop")
            .expect("a need");
        assert!(desktop.availability().is_present());
        assert_eq!(
            desktop.note().expect("a note"),
            "profile debug, features desktop"
        );
    }

    #[test]
    fn a_file_that_appeared_after_the_read_is_reported_as_there_now() {
        let sandbox = sandbox();
        let file = sandbox.project("demo", PROJECT);
        // The read found nothing at the stylesheet, and it is written afterwards.
        assert!(
            file.document()
                .expect("a document")
                .references()
                .iter()
                .any(|reference| reference.written() == "styles/app.css"
                    && matches!(
                        reference.verdict(),
                        crate::project::record::format::ReferenceVerdict::Absent
                    ))
        );
        sandbox.write("demo/styles/app.css", "body {}");
        let validation = validate(&file, &ToolchainState::NotChecked(NotChecked::NotAsked));
        let sheet = validation
            .needs()
            .iter()
            .find(|need| need.subject() == "styles/app.css")
            .expect("a need");
        assert!(
            sheet.availability().is_present(),
            "the answer is asked now: {sheet:?}"
        );
        assert!(
            sheet
                .note()
                .expect("the disagreement is disclosed")
                .contains("the read said"),
            "{sheet:?}"
        );
    }

    #[test]
    fn a_scan_records_every_reference_and_round_trips_through_a_file() {
        let sandbox = sandbox();
        let file = sandbox.project("demo", PROJECT);
        sandbox.write("demo/pages/main.json", "{}");
        let scan = Scan::of(&file).expect("the project is scannable");

        assert_eq!(scan.project(), "3f2b");
        assert_eq!(scan.name(), "Demo");
        assert_eq!(scan.document(), 2);
        // Seven references, five files: `pages/main.json` is a page document, the design a generated
        // file comes from and the startup entry, and a scan records the file once.
        assert_eq!(file.document().unwrap().references().len(), 7);
        assert_eq!(scan.items().len(), 5);
        let keys: Vec<&str> = scan.items().iter().map(|item| item.key()).collect();
        let mut unique = keys.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(
            unique.len(),
            keys.len(),
            "a scan holds each file once: {keys:?}"
        );
        assert_eq!(
            scan.items()
                .iter()
                .find(|item| item.key() == "pages/main.json")
                .expect("the file is scanned")
                .kind(),
            "page-document",
            "the first way the document names it is what it is called"
        );
        let states: Vec<&str> = scan.items().iter().map(|item| item.state()).collect();
        assert!(states.contains(&"present"), "{states:?}");
        assert!(states.contains(&"missing"), "{states:?}");

        let path = sandbox.root.join("demo-scan.json");
        let written = scan.save(&path).expect("the record is written");
        assert_eq!(written, path);
        let text = std::fs::read_to_string(&path).expect("the record is there");
        for member in super::SCAN_MEMBERS {
            assert!(text.contains(&format!("\"{member}\"")), "{member}: {text}");
        }
        assert_eq!(Scan::load(&path).expect("the record reads back"), scan);
        // A record from a newer build is refused rather than half-read.
        let newer = text.replace("\"format\": 1", "\"format\": 3");
        std::fs::write(&path, newer).expect("a newer record");
        assert!(Scan::load(&path).is_err());
        // And a record with no items is not an empty scan, it is a broken one: an empty baseline
        // would report every reference as new.
        std::fs::write(&path, "{\"format\": 1}").expect("a broken record");
        assert!(Scan::load(&path).is_err());

        // A refused reference is scanned as refused, and it is the same list the needs use.
        let escaping = sandbox.project("escaping", ESCAPING);
        let scan = Scan::of(&escaping).expect("the project is scannable");
        let refused = scan
            .items()
            .iter()
            .find(|item| item.key() == "../outside.png")
            .expect("the resource is scanned");
        assert_eq!(refused.label(), "resource ../outside.png");
        assert_eq!(refused.state(), "refused");
    }

    #[test]
    fn a_scan_reads_the_project_off_the_disk_rather_than_remembering_it() {
        let sandbox = sandbox();
        let file = sandbox.project("demo", PROJECT);
        let read = Scan::read(file.path()).expect("the project is scannable");
        assert_eq!(read, Scan::of(&file).expect("the project is scannable"));

        // The same path, with a reference that is now there: the scan taken from disk says so, and
        // the one taken from the document that was read earlier does not. That difference is why a
        // re-scan reads again.
        sandbox.write("demo/pages/gone.json", "{}");
        let again = Scan::read(file.path()).expect("still scannable");
        assert_ne!(again, read);
        assert_eq!(
            again
                .items()
                .iter()
                .find(|item| item.written() == "pages/gone.json")
                .expect("a scanned reference")
                .state(),
            "present"
        );
        let rescan = read.between(&again).expect("the same project");
        let rendered = rescan.render().join("\n");
        assert_eq!(rescan.changes().len(), 1, "{rendered}");
        assert_eq!(rescan.changes()[0].kind(), ChangeKind::State);
        assert_eq!(rescan.changes()[0].item(), "page-document pages/gone.json");
        assert_eq!(rescan.changes()[0].from(), Some("missing"));
        assert_eq!(rescan.changes()[0].to(), Some("present"));
        assert!(rescan.same_document());
        assert!(rescan.render()[0].contains("1 changed"), "{rendered}");
        assert!(
            rescan
                .render()
                .last()
                .expect("a summary")
                .contains("as they were"),
            "{rendered}"
        );
    }

    #[test]
    fn a_re_scan_names_what_appeared_what_vanished_and_what_nothing_touched() {
        let sandbox = sandbox();
        let file = sandbox.project("demo", PROJECT);
        let before = Scan::of(&file).expect("the project is scannable");

        // A resource is added to the document, the stylesheet reference is dropped, and the page
        // document appears on disk: one of each kind of change.
        let text = PROJECT
            .replace(
                "\"resources\": [{\"path\": \"assets/icon.svg\"}]",
                "\"resources\": [{\"path\": \"assets/icon.svg\"}, {\"path\": \"assets/new.svg\"}]",
            )
            .replace(", \"stylesheet\": \"styles/app.css\"", "");
        let edited = sandbox.project("edited", &text);
        sandbox.write("edited/pages/main.json", "{}");
        let after = Scan::read(edited.path()).expect("scannable");
        let rescan = before.between(&after).expect("the same project");

        let rendered = rescan.render().join("\n");
        assert!(
            rendered.contains("new    resource assets/new.svg"),
            "{rendered}"
        );
        assert!(
            rendered.contains("gone   stylesheet styles/app.css"),
            "{rendered}"
        );
        assert!(
            rendered.contains("state  page-document pages/main.json (missing → present)"),
            "{rendered}"
        );
        // The generated file is built from that same page document, and the file that appeared is
        // reported once: a scan records what is at a path, not how many members name it.
        assert_eq!(
            rendered
                .matches("pages/main.json (missing → present)")
                .count(),
            1,
            "{rendered}"
        );
        assert_eq!(rescan.changes().len(), 3, "{rendered}");
        assert!(!rescan.is_empty());
        assert!(rescan.same_document(), "both are format v2");

        // The document's own format version is part of the comparison: a project that was scanned as
        // version 1 and is version 2 now says so, because everything else in the file may well be the
        // same while what the document *is* has changed.
        // Version 1 has no `startup` member, so the older document is the same project without one.
        let older = sandbox.project(
            "older",
            &PROJECT
                .replace("\"format\": 2", "\"format\": 1")
                .replace(", \"startup\": \"pages/main.json\"", ""),
        );
        let recorded = Scan::read(older.path()).expect("scannable");
        let rescan = match recorded.between(&before) {
            Ok(rescan) => rescan,
            Err(refusal) => panic!("the same project id: {}", refusal.message()),
        };
        assert_eq!(recorded.document(), 1, "the file declares version 1");
        assert!(!rescan.same_document());
        assert!(
            rescan.render()[0].contains("it was format v1 when scanned, and v2 now"),
            "{:?}",
            rescan.render()
        );

        // Scanning the same project twice in a row finds nothing, and says so rather than printing
        // an empty list.
        let quiet = after
            .between(&Scan::read(edited.path()).expect("scannable"))
            .expect("same");
        assert!(quiet.is_empty());
        assert_eq!(quiet.changes().len(), 0);
        assert!(quiet.render()[0].contains("nothing changed"));
    }

    #[test]
    fn a_record_may_not_be_written_over_the_project_or_a_file_it_declares() {
        let sandbox = sandbox();
        let file = sandbox.project("demo", PROJECT);
        sandbox.write("demo/pages/main.json", "{}");

        let refusal =
            super::may_record_at(&file, file.path()).expect_err("the document is refused");
        assert_eq!(
            refusal,
            super::RecordRefusal::TheProjectDocument {
                path: file.path().to_path_buf()
            }
        );
        assert!(refusal.message().contains("goes to its own file"));

        // A file the document declares is refused too, however the path is spelled: the same file
        // reached by another name is the same file.
        let declared = sandbox.root.join("demo/pages/../pages/main.json");
        let refusal =
            super::may_record_at(&file, &declared).expect_err("a declared file is refused");
        assert_eq!(
            refusal,
            super::RecordRefusal::AFileTheProjectDeclares {
                path: declared.clone(),
                written: "pages/main.json".to_owned(),
            }
        );
        assert!(refusal.message().contains("would destroy a file"));

        // Anywhere else is fine, inside the project or not: the record is this host's artifact.
        for allowed in [
            sandbox.root.join("demo/scan.json"),
            sandbox.root.join("scan.json"),
        ] {
            assert!(
                super::may_record_at(&file, &allowed).is_ok(),
                "{} should be allowed",
                allowed.display()
            );
        }
    }

    #[test]
    fn a_scan_of_another_project_is_refused_as_a_baseline() {
        let sandbox = sandbox();
        let file = sandbox.project("demo", PROJECT);
        let other = sandbox.project(
            "other",
            &PROJECT.replace("\"id\": \"3f2b\"", "\"id\": \"9c77\""),
        );
        let scan = Scan::of(&file).expect("the project is scannable");
        let theirs = Scan::of(&other).expect("scannable");

        let refusal = scan
            .between(&theirs)
            .expect_err("another project is refused");
        assert_eq!(
            refusal,
            RescanRefusal::AnotherProject {
                expected: "Demo (3f2b)".to_owned(),
                found: "Demo (9c77)".to_owned(),
            }
        );
        assert!(refusal.message().contains("not a baseline for another"));
        assert_eq!(SCAN_FORMAT.get(), 1);
    }
}
