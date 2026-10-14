// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Opening a project: what is read, what a person is asked, and what is never done (task T02.03).
//!
//! Opening a project must not do anything to it. The acceptance line is *"it can be opened
//! read-only, and a recent entry does not run anything"*, and it is met by where the decisions live:
//!
//! * [`ProjectSession::open`] reads the document and works out what a person *would* be asked. It
//!   writes nothing, trusts nothing and runs nothing.
//! * Trust is a decision, and only a decision makes it: [`ProjectSession::decide`] takes
//!   [`TrustDecision::Trust`] or [`TrustDecision::DoNotTrust`], records the answer in the host's
//!   authorization record (T01.05), and is the only way a session becomes trusted. A path is not a
//!   permission, and neither is a recent entry.
//! * The toolchain is read by *running* two programs, so it is asked for through the permission
//!   gate ([`ProjectSession::diagnose_toolchain`]): while the project is untrusted the answer is
//!   [`NotChecked::ProjectNotTrusted`], which is a diagnosis, not a failure.
//! * The question is re-read when it is answered. A directory can gain a `build.rs` between the
//!   prompt and the answer, and a person who was shown one directory must not end up having trusted
//!   another state of it: the answer is refused, trust is withdrawn, and the prompt has to be shown
//!   again (principle #22: a stale result must not pass itself off as the current one).

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::contracts::permission::Principal;
use crate::contracts::revision::FormatVersion;
use crate::host::paths::ProjectRoot;
use crate::host::permission::{PermissionService, Scope, TrustPrompt};
use crate::host::subprocess::{OwnedProcess, ProcessSpec};
use crate::project::format::{
    PROJECT_FILE_NAME, ProjectContent, ProjectError, ProjectFile, ProjectWriteRefusal,
};
use crate::project::recent::RecentEntry;

/// What an opened project is, from the format's own point of view.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SessionState {
    /// Readable, and this build writes this version.
    Readable,
    /// Readable, and this build will not write it.
    ReadOnly {
        /// Why, in the format's words.
        refusal: ProjectWriteRefusal,
    },
}

impl SessionState {
    /// Whether the format allows this build to write the project.
    pub fn is_writable(&self) -> bool {
        matches!(self, Self::Readable)
    }

    /// A sentence for a person.
    pub fn message(&self) -> String {
        match self {
            Self::Readable => "readable, and this build writes this format".to_owned(),
            Self::ReadOnly { refusal } => format!("readable, read-only: {}", refusal.message()),
        }
    }
}

/// The question a person is asked before a project is trusted.
///
/// It is built from the directory itself (a build script, a cargo configuration, a toolchain pin —
/// [`TrustPrompt`]), so it says what *would* happen rather than what somebody promised would not.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TrustQuestion {
    root: PathBuf,
    findings: Vec<String>,
    runs_code_at_build_time: bool,
    has_manifest: bool,
}

impl TrustQuestion {
    /// Reads the question a directory raises right now.
    pub fn read(root: &ProjectRoot) -> Self {
        let prompt = TrustPrompt::read(root);
        Self {
            root: root.as_path().to_path_buf(),
            findings: prompt.findings(),
            runs_code_at_build_time: prompt.runs_code_at_build_time(),
            has_manifest: prompt.has_manifest(),
        }
    }

    /// The directory the question is about.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// One sentence per finding: what is there, and what it would let the project do.
    pub fn findings(&self) -> &[String] {
        &self.findings
    }

    /// Whether anything in the directory can run or fetch a program during a build.
    pub const fn runs_code_at_build_time(&self) -> bool {
        self.runs_code_at_build_time
    }

    /// Whether cargo would find anything to build here.
    pub const fn has_manifest(&self) -> bool {
        self.has_manifest
    }

    /// What a person gets by answering yes.
    pub fn allows(&self) -> [&'static str; 2] {
        [
            "builds and runs, each of them still decided by the permission table",
            "writes into the project directory by a person's action",
        ]
    }

    /// What answering no leaves them with.
    pub fn refuses(&self) -> [&'static str; 2] {
        [
            "the project is open read-only: nothing is written to it",
            "nothing in it is run, so its toolchain is not read either",
        ]
    }

    /// The question as one text, for a terminal or a dialog.
    pub fn ask(&self) -> String {
        let mut text = format!("Open {} and let this build act on it?", self.root.display());
        for finding in &self.findings {
            text.push_str("\n  ");
            text.push_str(finding);
        }
        for allowed in self.allows() {
            text.push_str("\n  answering yes: ");
            text.push_str(allowed);
        }
        for refused in self.refuses() {
            text.push_str("\n  answering no: ");
            text.push_str(refused);
        }
        text
    }
}

/// What a person answered.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TrustDecision {
    /// Yes: this directory may be acted on for this session.
    Trust,
    /// No: it stays readable and nothing is done to it.
    DoNotTrust,
}

/// Records the answer to a trust question about a directory.
///
/// The question is read again first. A directory can gain a `build.rs` between the prompt and the
/// answer, and a person who was shown one directory must not end up having trusted another state of
/// it: when the findings changed, nothing is trusted, trust is withdrawn, and the answer has to be
/// asked for again.
///
/// It takes the question rather than a session because the same question is asked about a directory
/// that holds no project document — a workspace root, which the host writes one document into — and
/// that directory has to be trusted by the same rule rather than by a second one beside it.
pub fn answer(
    question: &TrustQuestion,
    permissions: &PermissionService,
    decision: TrustDecision,
) -> TrustOutcome {
    match decision {
        TrustDecision::DoNotTrust => TrustOutcome {
            trusted: false,
            message:
                "not trusted: the project is open read-only, nothing is written and nothing is \
                      run"
                .to_owned(),
        },
        TrustDecision::Trust => {
            // The root is resolved again, from the same path the question was read from: a directory
            // that cannot be resolved now is not a directory an answer can apply to, and saying so
            // is better than applying the answer to something else.
            let Ok(root) = ProjectRoot::new(question.root()) else {
                return TrustOutcome {
                    trusted: false,
                    message: format!(
                        "{} cannot be resolved now, so no answer applies; the question has to be \
                         asked again",
                        question.root().display()
                    ),
                };
            };
            let prompt = permissions.trust(&root);
            if prompt.findings() != question.findings() {
                let withdrawn = permissions.distrust(&root);
                return TrustOutcome {
                    trusted: false,
                    message: format!(
                        "{} changed since the question was shown, so the answer was not applied; \
                         trust was withdrawn ({}) and the question has to be asked again",
                        root.as_path().display(),
                        withdrawn.len()
                    ),
                };
            }
            TrustOutcome {
                trusted: true,
                message: format!(
                    "{} is trusted for this session: builds and runs are decided by the permission \
                     table from here",
                    root.as_path().display()
                ),
            }
        }
    }
}

/// What happened when the question was answered.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TrustOutcome {
    trusted: bool,
    message: String,
}

impl TrustOutcome {
    /// Whether the project is trusted now.
    pub const fn trusted(&self) -> bool {
        self.trusted
    }

    /// What the answer did, in one sentence.
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// A toolchain program, and the version it reported.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ToolVersion {
    program: String,
    version: String,
    line: String,
}

impl ToolVersion {
    /// Reads the version out of the line a program printed.
    ///
    /// `cargo 1.99.0 (5f94df478 2026-08-27)`: the version is the word after the program's name, not
    /// the last word on the line, which is a commit date. A line with one word is that word, which
    /// is what a program that prints only a version looks like.
    pub fn from_line(program: &str, line: &str) -> Self {
        Self {
            program: program.to_owned(),
            version: line.split_whitespace().nth(1).unwrap_or(line).to_owned(),
            line: line.to_owned(),
        }
    }

    /// The program's name.
    pub fn program(&self) -> &str {
        &self.program
    }

    /// The version it reported.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// The line it printed, which is the evidence for the version.
    pub fn line(&self) -> &str {
        &self.line
    }
}

/// Why the toolchain was not read.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum NotChecked {
    /// Nobody asked yet.
    NotAsked,
    /// The project is not trusted, and reading a toolchain starts a program.
    ProjectNotTrusted,
    /// The permission table refused the command.
    Refused {
        /// Why, in the host's own words.
        reason: String,
    },
    /// The program is not there, or would not start.
    NotRunnable {
        /// The program that could not be started.
        program: String,
        /// What the operating system said.
        reason: String,
    },
}

impl NotChecked {
    /// A sentence for a person: what is not known, and what would make it known.
    pub fn message(&self) -> String {
        match self {
            Self::NotAsked => "not checked yet".to_owned(),
            Self::ProjectNotTrusted => {
                "not checked: reading a toolchain runs programs, and this project is not trusted"
                    .to_owned()
            }
            Self::Refused { reason } => format!("not checked: {reason}"),
            Self::NotRunnable { program, reason } => {
                format!("`{program}` could not be run: {reason}")
            }
        }
    }
}

/// What the toolchain check found.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ToolchainReport {
    cargo: Option<ToolVersion>,
    rustc: Option<ToolVersion>,
    checks: Vec<String>,
}

impl ToolchainReport {
    /// The report a toolchain check produced: what each program answered, and every command that was
    /// run, which is what makes the report checkable.
    pub(crate) fn from_checks(
        cargo: Option<ToolVersion>,
        rustc: Option<ToolVersion>,
        checks: Vec<String>,
    ) -> Self {
        Self {
            cargo,
            rustc,
            checks,
        }
    }

    /// Cargo, when it answered.
    pub fn cargo(&self) -> Option<&ToolVersion> {
        self.cargo.as_ref()
    }

    /// Rustc, when it answered.
    pub fn rustc(&self) -> Option<&ToolVersion> {
        self.rustc.as_ref()
    }

    /// Every command that was run, which is what makes the report checkable.
    pub fn checks(&self) -> &[String] {
        &self.checks
    }

    /// Whether both programs answered.
    pub fn is_complete(&self) -> bool {
        self.cargo.is_some() && self.rustc.is_some()
    }

    /// One line per finding.
    pub fn render(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for version in [self.cargo.as_ref(), self.rustc.as_ref()]
            .into_iter()
            .flatten()
        {
            lines.push(format!("{}: {}", version.program, version.version));
        }
        if self.cargo.is_none() {
            lines.push("cargo: not found or did not answer".to_owned());
        }
        if self.rustc.is_none() {
            lines.push("rustc: not found or did not answer".to_owned());
        }
        lines
    }
}

/// What is known about the toolchain.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ToolchainState {
    /// Nothing was read, and why.
    NotChecked(NotChecked),
    /// What the two programs said.
    Checked(ToolchainReport),
}

impl ToolchainState {
    /// The report, when there is one.
    pub fn report(&self) -> Option<&ToolchainReport> {
        match self {
            Self::Checked(report) => Some(report),
            Self::NotChecked(_) => None,
        }
    }

    /// One sentence for a person.
    pub fn message(&self) -> String {
        match self {
            Self::NotChecked(reason) => reason.message(),
            Self::Checked(report) => report.render().join("; "),
        }
    }
}

/// A project that is open.
///
/// Holding one is not a permission: [`Self::may_write`] still asks the trust decision, and every
/// external effect goes through the host's gate.
#[derive(Clone, Debug)]
pub struct ProjectSession {
    file: ProjectFile,
    question: TrustQuestion,
    state: SessionState,
    toolchain: ToolchainState,
}

impl ProjectSession {
    /// Opens the project at a path: reads it, and decides nothing.
    ///
    /// Nothing is written, nothing is trusted, nothing is run. What the caller gets is the project,
    /// the state the format is in, and the question a person would be asked.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ProjectError> {
        let file = ProjectFile::open(path)?;
        Self::from_file(file)
    }

    fn from_file(file: ProjectFile) -> Result<Self, ProjectError> {
        let question = TrustQuestion::read(file.root());
        let state = match file.writer() {
            Ok(_) => SessionState::Readable,
            Err(refusal) => SessionState::ReadOnly { refusal },
        };
        Ok(Self {
            file,
            question,
            state,
            toolchain: ToolchainState::NotChecked(NotChecked::NotAsked),
        })
    }

    /// Opens the project a recent entry names.
    ///
    /// A recent entry is a *place*, not a permission: this opens the project the same way
    /// [`Self::open`] does — untrusted, read-only until somebody decides otherwise, running nothing —
    /// and it checks that the place still holds the project that was recorded there, because a path
    /// is not an identity.
    pub fn from_recent(entry: &RecentEntry) -> Result<Self, RecentOpen> {
        if !entry.path().is_dir() {
            return Err(RecentOpen::MissingPath {
                path: entry.path().to_path_buf(),
                name: entry.name().to_owned(),
            });
        }
        // A recent entry remembers the project *directory*, because that is what moves: the
        // document inside it has one name, and the root is the thing a person recognises.
        let session =
            Self::open(entry.path().join(PROJECT_FILE_NAME)).map_err(RecentOpen::Unreadable)?;
        let found = session
            .id()
            .map(str::to_owned)
            .unwrap_or_else(|| "<no identity>".to_owned());
        if found != entry.id() {
            return Err(RecentOpen::DifferentProject {
                path: entry.path().to_path_buf(),
                recorded: entry.id().to_owned(),
                found,
            });
        }
        Ok(session)
    }

    /// The file this session was read from.
    pub fn file(&self) -> &ProjectFile {
        &self.file
    }

    /// The directory every path in the project is relative to.
    pub fn root(&self) -> &ProjectRoot {
        self.file.root()
    }

    /// The file the project was read from.
    pub fn path(&self) -> &Path {
        self.file.path()
    }

    /// The version the document declares.
    pub fn version(&self) -> FormatVersion {
        self.file.version()
    }

    /// What the project is called, when the identity could be read.
    pub fn name(&self) -> Option<&str> {
        self.file
            .content()
            .identity()
            .map(|identity| identity.name())
    }

    /// Which project it is, when the identity could be read.
    pub fn id(&self) -> Option<&str> {
        self.file.content().identity().map(|identity| identity.id())
    }

    /// What the format makes of the project.
    pub const fn state(&self) -> &SessionState {
        &self.state
    }

    /// The question a person would be asked.
    pub const fn question(&self) -> &TrustQuestion {
        &self.question
    }

    /// Whether a person has trusted this project in this session.
    pub fn is_trusted(&self, permissions: &PermissionService) -> bool {
        permissions.is_trusted(self.root())
    }

    /// Whether the project can be written right now: the format allows it *and* it is trusted.
    pub fn may_write(&self, permissions: &PermissionService) -> bool {
        self.state.is_writable() && self.is_trusted(permissions)
    }

    /// Records the answer to the question.
    ///
    /// The rule itself is [`answer`], which is also what a directory that holds no project document
    /// goes through: one rule for "this directory may be acted on", rather than a second one written
    /// next to it that can drift away from it.
    pub fn decide(&self, permissions: &PermissionService, decision: TrustDecision) -> TrustOutcome {
        answer(&self.question, permissions, decision)
    }

    /// What is known about the toolchain.
    pub const fn toolchain(&self) -> &ToolchainState {
        &self.toolchain
    }

    /// Reads the toolchain, which means running two programs.
    ///
    /// Gated on purpose: `cargo --version` and `rustc --version` are programs this build starts, so
    /// they need the permission the decision table gives a command, and a project that is not
    /// trusted gets a diagnosis that says exactly that instead of a version nobody authorized.
    pub fn diagnose_toolchain(&mut self, permissions: &PermissionService) -> &ToolchainState {
        if !self.is_trusted(permissions) {
            self.toolchain = ToolchainState::NotChecked(NotChecked::ProjectNotTrusted);
            return &self.toolchain;
        }
        let scope = Scope::Project(self.root());
        let permit = match permissions.permit(
            crate::contracts::permission::EffectClass::Command,
            scope,
            Principal::Human,
        ) {
            Ok(permit) => permit,
            Err(refusal) => {
                self.toolchain = ToolchainState::NotChecked(NotChecked::Refused {
                    reason: refusal.message(),
                });
                return &self.toolchain;
            }
        };
        let mut checks = Vec::new();
        let mut read = |program: &str| -> Result<ToolVersion, NotChecked> {
            let spec = ProcessSpec::new(format!("{program} --version"), program).arg("--version");
            checks.push(format!("{program} --version"));
            let process =
                OwnedProcess::spawn(spec, &permit).map_err(|error| NotChecked::NotRunnable {
                    program: program.to_owned(),
                    reason: error.to_string(),
                })?;
            let mut process = process;
            let exit = process.wait();
            let output = process.output_so_far();
            let line = output
                .stdout
                .lines()
                .chain(output.stderr.lines())
                .map(str::trim)
                .find(|line| !line.is_empty())
                .unwrap_or_default()
                .to_owned();
            if !exit.is_success() || line.is_empty() {
                return Err(NotChecked::NotRunnable {
                    program: program.to_owned(),
                    reason: format!("it ended with {} and said `{}`", exit.message(), line),
                });
            }
            Ok(ToolVersion::from_line(program, &line))
        };

        let cargo = read("cargo");
        let rustc = read("rustc");
        // A program that would not start is a finding about the machine, and the report keeps it:
        // returning "not checked" here would throw away the one half that did answer.
        for failure in [cargo.as_ref().err(), rustc.as_ref().err()]
            .into_iter()
            .flatten()
        {
            checks.push(failure.message());
        }
        let report = ToolchainReport::from_checks(
            cargo.as_ref().ok().cloned(),
            rustc.as_ref().ok().cloned(),
            checks,
        );
        self.toolchain = ToolchainState::Checked(report);
        &self.toolchain
    }

    /// The entry this session adds to the recent list.
    ///
    /// It remembers the project directory, not the file: the directory is what a person moves and
    /// recognises, and a missing directory is the state the repair step answers.
    pub fn recent_entry(&self, at: SystemTime) -> RecentEntry {
        RecentEntry::new(
            self.root().as_path().to_path_buf(),
            self.name().unwrap_or("<unnamed>").to_owned(),
            self.id().unwrap_or("<no identity>").to_owned(),
            at,
        )
    }
}

/// Why opening a recent entry did not produce a session.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RecentOpen {
    /// The path is not there any more, which is what the repair step answers.
    MissingPath {
        /// The path that was recorded.
        path: PathBuf,
        /// What the project was called when it was recorded.
        name: String,
    },
    /// The path is there and holds a different project than the one recorded.
    DifferentProject {
        /// The path.
        path: PathBuf,
        /// The identity that was recorded.
        recorded: String,
        /// The identity that is there now.
        found: String,
    },
    /// The path holds something this build cannot read as a project.
    Unreadable(ProjectError),
}

impl RecentOpen {
    /// A sentence naming what was found, for a diagnostic or a terminal.
    pub fn message(&self) -> String {
        match self {
            Self::MissingPath { path, name } => format!(
                "{} is not there any more (`{name}` was last seen there); point the entry at where \
                 the project is now, or forget it",
                path.display()
            ),
            Self::DifferentProject {
                path,
                recorded,
                found,
            } => format!(
                "{} holds a different project now (recorded {recorded}, found {found}); nothing was \
                 opened, because a path is not an identity",
                path.display()
            ),
            Self::Unreadable(error) => error.message(),
        }
    }
}

impl fmt::Display for RecentOpen {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.message())
    }
}

/// Whether the project's format is one this build reads, in one word, for a report.
pub fn content_of(file: &ProjectFile) -> &'static str {
    match file.content() {
        ProjectContent::Interpreted(_) => "interpreted",
        ProjectContent::Foreign(_) => "a format this build does not know",
    }
}

#[cfg(test)]
mod tests {
    use super::{
        NotChecked, ProjectSession, RecentOpen, SessionState, ToolchainState, TrustDecision,
        TrustQuestion,
    };
    use crate::host::permission::PermissionService;
    use crate::project::recent::{RecentProjects, RepairRefusal};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::SystemTime;
    use std::time::{Duration, UNIX_EPOCH};

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
            std::env::temp_dir().join(format!("rustage-session-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&root).expect("a temporary directory is creatable");
        Sandbox {
            root: root.clone(),
            _guard: TempDirectory(root),
        }
    }

    const PROJECT: &str = "{\n  \"format\": 2,\n  \"project\": {\n    \"id\": \"3f2b\",\n    \"name\": \"Demo\"\n  },\n  \"pages\": [],\n  \"resources\": [],\n  \"targets\": [],\n  \"generated\": [],\n  \"startup\": null,\n  \"network\": {\n    \"services\": []\n  }\n}\n";

    /// A project document as version 1 wrote it, for the migration path.
    const VERSION_1: &str = "{\n  \"format\": 1,\n  \"project\": {\n    \"id\": \"3f2b\",\n    \"name\": \"Older\"\n  },\n  \"pages\": [],\n  \"resources\": [],\n  \"targets\": [],\n  \"generated\": [],\n  \"network\": {\n    \"services\": []\n  }\n}\n";

    impl Sandbox {
        fn project(&self, name: &str, text: &str) -> PathBuf {
            let directory = self.root.join(name);
            std::fs::create_dir_all(&directory).expect("a project directory");
            std::fs::write(directory.join("project.json"), text).expect("a project document");
            directory
        }
    }

    /// Whether the gate would let a person run a checked command in this session at all.
    ///
    /// Asked through the service's own report rather than a second gate: `permit` records what a
    /// refusal was about, and a service that granted nothing has an empty record.
    fn trusted_can_run(permissions: &PermissionService) -> bool {
        permissions.record().events().iter().any(|event| {
            matches!(
                event.kind,
                crate::host::permission::AuthorizationEventKind::Trusted { .. }
            )
        })
    }

    #[test]
    fn opening_reads_the_project_and_decides_nothing() {
        let sandbox = sandbox();
        let directory = sandbox.project("demo", PROJECT);
        let permissions = PermissionService::new();

        let session = ProjectSession::open(directory.join("project.json")).expect("readable");

        assert_eq!(session.name(), Some("Demo"));
        assert_eq!(session.id(), Some("3f2b"));
        assert_eq!(session.version().get(), 2);
        assert!(session.state().is_writable());
        assert!(
            session
                .state()
                .message()
                .contains("this build writes this format")
        );
        assert!(!session.is_trusted(&permissions), "opening is not trusting");
        assert!(
            !session.may_write(&permissions),
            "an untrusted project is not written"
        );
        assert!(
            permissions.record().events().is_empty(),
            "opening a project records no decision at all"
        );
        assert!(matches!(
            session.toolchain(),
            ToolchainState::NotChecked(NotChecked::NotAsked)
        ));
    }

    #[test]
    fn a_project_this_build_must_not_write_is_still_readable() {
        let sandbox = sandbox();
        let newer = sandbox.project("newer", &PROJECT.replace("\"format\": 2", "\"format\": 3"));
        let escaping = sandbox.project(
            "escaping",
            &PROJECT.replace(
                "\"pages\": []",
                "\"pages\": [{\"document\": \"../outside.json\"}]",
            ),
        );

        let session = ProjectSession::open(newer.join("project.json")).expect("readable");
        assert_eq!(
            session.name(),
            Some("Demo"),
            "the identity is still readable"
        );
        assert!(matches!(session.state(), SessionState::ReadOnly { .. }));
        assert!(!session.state().is_writable());
        assert!(session.state().message().contains("read-only"));
        assert!(!session.may_write(&PermissionService::new()));

        let session = ProjectSession::open(escaping.join("project.json")).expect("readable");
        assert!(matches!(session.state(), SessionState::ReadOnly { .. }));
        assert!(session.state().message().contains("leave the project root"));
    }

    #[test]
    fn the_question_says_what_would_happen_and_what_answering_no_costs() {
        let sandbox = sandbox();
        let directory = sandbox.project("demo", PROJECT);
        std::fs::write(directory.join("build.rs"), "fn main() {}").expect("a build script");
        std::fs::create_dir_all(directory.join(".cargo")).expect("a directory");
        std::fs::write(directory.join(".cargo/config.toml"), "[build]\n").expect("a config");

        let session = ProjectSession::open(directory.join("project.json")).expect("readable");
        let question = session.question();
        assert!(question.runs_code_at_build_time());
        assert!(!question.has_manifest());
        assert_eq!(
            question.findings().len(),
            3,
            "two findings and the missing manifest"
        );
        assert!(question.findings()[0].contains("build.rs"));
        assert!(question.ask().contains("Open "));
        assert!(question.ask().contains("answering yes:"));
        assert!(question.ask().contains("answering no:"));
        assert!(question.ask().contains("read-only"));
        assert_eq!(
            question.root(),
            crate::host::paths::ProjectRoot::new(&directory)
                .expect("a root")
                .as_path()
        );
    }

    #[test]
    fn answering_the_question_is_the_only_way_a_project_becomes_trusted() {
        let sandbox = sandbox();
        let directory = sandbox.project("demo", PROJECT);
        let permissions = PermissionService::new();
        let session = ProjectSession::open(directory.join("project.json")).expect("readable");

        // Answering no leaves the project readable, records no decision and grants nothing.
        let declined = session.decide(&permissions, TrustDecision::DoNotTrust);
        assert!(!declined.trusted());
        assert!(declined.message().contains("read-only"));
        assert!(!session.is_trusted(&permissions));
        assert!(permissions.record().events().is_empty());

        // Answering yes records the prompt that was shown, and only then may anything happen.
        let accepted = session.decide(&permissions, TrustDecision::Trust);
        assert!(accepted.trusted(), "{}", accepted.message());
        assert!(session.is_trusted(&permissions));
        assert!(session.may_write(&permissions));
        assert_eq!(permissions.record().events().len(), 1);
        match &permissions.record().events()[0].kind {
            crate::host::permission::AuthorizationEventKind::Trusted { prompt } => {
                assert_eq!(prompt.findings(), session.question().findings());
            }
            other => panic!("expected a trust decision, found {other:?}"),
        }
    }

    #[test]
    fn a_directory_that_changed_while_the_question_was_on_screen_is_not_trusted() {
        let sandbox = sandbox();
        let directory = sandbox.project("demo", PROJECT);
        let permissions = PermissionService::new();
        let session = ProjectSession::open(directory.join("project.json")).expect("readable");
        assert!(!session.question().runs_code_at_build_time());

        // Something lands in the directory between the prompt and the answer.
        std::fs::write(directory.join("build.rs"), "fn main() {}").expect("a build script");

        let outcome = session.decide(&permissions, TrustDecision::Trust);
        assert!(
            !outcome.trusted(),
            "the answer applied to a state that is gone"
        );
        assert!(
            outcome
                .message()
                .contains("changed since the question was shown")
        );
        assert!(
            !session.is_trusted(&permissions),
            "trust was withdrawn again"
        );
        assert_eq!(
            permissions.record().events().len(),
            2,
            "the decision and its withdrawal are both recorded"
        );
    }

    #[test]
    fn the_toolchain_is_not_read_for_a_project_nobody_trusted() {
        let sandbox = sandbox();
        let directory = sandbox.project("demo", PROJECT);
        let permissions = PermissionService::new();
        let mut session = ProjectSession::open(directory.join("project.json")).expect("readable");

        let state = session.diagnose_toolchain(&permissions);
        assert!(matches!(
            state,
            ToolchainState::NotChecked(NotChecked::ProjectNotTrusted)
        ));
        assert!(state.message().contains("runs programs"));
        assert!(state.report().is_none());
        assert!(permissions.record().events().is_empty());
    }

    #[test]
    fn the_toolchain_is_read_once_the_project_is_trusted() {
        let sandbox = sandbox();
        let directory = sandbox.project("demo", PROJECT);
        let permissions = PermissionService::new();
        let mut session = ProjectSession::open(directory.join("project.json")).expect("readable");
        session.decide(&permissions, TrustDecision::Trust);

        let state = session.diagnose_toolchain(&permissions);
        let ToolchainState::Checked(report) = state else {
            panic!(
                "the toolchain should have been read, found {}",
                state.message()
            );
        };
        // The checks are recorded whether or not the programs answered, so the report is evidence
        // rather than a claim: this machine has a Rust toolchain, and the runner is a Rust build.
        assert!(
            report
                .checks()
                .iter()
                .any(|check| check.contains("cargo --version"))
        );
        assert!(
            report
                .checks()
                .iter()
                .any(|check| check.contains("rustc --version"))
        );
        if let Some(cargo) = report.cargo() {
            assert_eq!(cargo.program(), "cargo");
            assert!(cargo.line().contains("cargo"));
            // The version is the word after the program: a build that read the last word would
            // report the commit date instead, which is a number nobody can act on.
            assert!(
                cargo.version().split('.').count() >= 2
                    && cargo
                        .version()
                        .split('.')
                        .all(|part| part.starts_with(char::is_numeric)),
                "`{}` is not a version, and the commit date at the end of the line is not one either",
                cargo.version()
            );
        }
        if let Some(rustc) = report.rustc() {
            assert_eq!(rustc.program(), "rustc");
            assert!(rustc.version().contains('.'), "{}", rustc.version());
        }
        assert!(report.render().iter().any(|line| line.starts_with("cargo")));
    }

    #[test]
    fn a_recent_entry_opens_the_project_without_trusting_or_running_anything() {
        let sandbox = sandbox();
        let directory = sandbox.project("demo", PROJECT);
        std::fs::write(directory.join("build.rs"), "fn main() {}").expect("a build script");
        let permissions = PermissionService::new();
        let entries = sandbox.root.join("recent.json");
        let mut recent = RecentProjects::load(&entries).expect("an empty list");

        let session = ProjectSession::open(directory.join("project.json")).expect("readable");
        let entry = session.recent_entry(SystemTime::now());
        recent.record(entry.clone());
        assert_eq!(recent.len(), 1);
        assert_eq!(recent.entries()[0].name(), "Demo");

        // "Using" the entry is opening a path: the project is untrusted, nothing was recorded, and
        // the toolchain was not read either.
        let reopened = ProjectSession::from_recent(&entry).expect("the same project");
        assert!(!reopened.is_trusted(&permissions));
        assert!(!reopened.may_write(&permissions));
        assert!(matches!(
            reopened.toolchain(),
            ToolchainState::NotChecked(NotChecked::NotAsked)
        ));
        assert!(
            permissions.record().events().is_empty(),
            "a recent entry is a place, not a permission"
        );
    }

    #[test]
    fn a_recent_entry_whose_path_holds_another_project_is_refused() {
        let sandbox = sandbox();
        let directory = sandbox.project("demo", PROJECT);
        let session = ProjectSession::open(directory.join("project.json")).expect("readable");
        let entry = session.recent_entry(SystemTime::now());

        // The directory is gone: that is the missing-path case the repair step answers.
        std::fs::remove_dir_all(&directory).expect("removed");
        assert!(matches!(
            ProjectSession::from_recent(&entry),
            Err(RecentOpen::MissingPath { .. })
        ));

        // The directory is there again and holds another project.
        std::fs::create_dir_all(&directory).expect("a directory");
        std::fs::write(
            directory.join("project.json"),
            PROJECT.replace("\"3f2b\"", "\"9999\""),
        )
        .expect("another project");
        match ProjectSession::from_recent(&entry) {
            Err(RecentOpen::DifferentProject {
                recorded, found, ..
            }) => {
                assert_eq!(recorded, "3f2b");
                assert_eq!(found, "9999");
            }
            other => panic!("expected a different project, found {other:?}"),
        }
    }

    #[test]
    fn a_missing_entry_is_repaired_only_onto_the_same_project() {
        let sandbox = sandbox();
        let directory = sandbox.project("demo", PROJECT);
        let moved = sandbox.root.join("moved-away");
        let session = ProjectSession::open(directory.join("project.json")).expect("readable");
        let mut recent = RecentProjects::load(sandbox.root.join("recent.json")).expect("empty");
        recent.record(session.recent_entry(SystemTime::now()));

        // While the path is there, there is nothing to repair.
        let root = crate::host::paths::ProjectRoot::new(&directory).expect("a root");
        let refusal = recent
            .repair(root.as_path(), &moved)
            .expect_err("a path that is still there is not repaired");
        assert!(matches!(refusal, RepairRefusal::NotMissing { .. }));

        // The project moves; the entry is repaired onto it because it is the same project.
        std::fs::rename(&directory, &moved).expect("the project moves");
        let mut recent = RecentProjects::load(sandbox.root.join("recent.json")).expect("empty");
        recent.record(session.recent_entry(SystemTime::now()));
        assert!(matches!(
            recent.state_of(&recent.entries()[0].clone()),
            crate::project::recent::RecentState::Missing
        ));
        let moved_root = crate::host::paths::ProjectRoot::new(&moved).expect("a root");
        recent
            .repair(root.as_path(), &moved)
            .expect("the same project");
        assert_eq!(recent.entries()[0].path(), moved_root.as_path());
        assert!(ProjectSession::from_recent(&recent.entries()[0].clone()).is_ok());

        // A different project in the new place is refused, with both identities named.
        let other = sandbox.project("other", &PROJECT.replace("\"3f2b\"", "\"aaaa\""));
        std::fs::remove_dir_all(&moved).expect("removed");
        let refusal = recent
            .repair(&moved, &other)
            .expect_err("a different project is not the one that moved");
        match refusal {
            RepairRefusal::DifferentProject {
                recorded, found, ..
            } => {
                assert_eq!(recorded, "3f2b");
                assert_eq!(found, "aaaa");
            }
            other => panic!("expected a different project, found {other:?}"),
        }
        assert_eq!(
            recent.entries()[0].path(),
            moved_root.as_path(),
            "the entry did not move"
        );
    }

    #[test]
    fn the_recent_list_keeps_one_entry_per_path_and_a_bounded_number_of_them() {
        let sandbox = sandbox();
        let at = UNIX_EPOCH + Duration::from_millis(1_700_000_000_000);
        let mut recent = RecentProjects::load(sandbox.root.join("recent.json"))
            .expect("empty")
            .with_capacity(2);

        let first = crate::project::recent::RecentEntry::new(sandbox.root.join("a"), "A", "1", at);
        let second = crate::project::recent::RecentEntry::new(sandbox.root.join("b"), "B", "2", at);
        let third = crate::project::recent::RecentEntry::new(sandbox.root.join("c"), "C", "3", at);
        recent.record(first.clone());
        recent.record(second.clone());
        recent.record(third.clone());
        assert_eq!(recent.len(), 2, "the oldest fell off the end");
        assert_eq!(recent.entries()[0].path(), third.path());
        assert_eq!(recent.entries()[1].path(), second.path());

        // Opening a project again moves its entry to the front instead of adding a second line.
        recent.record(second.clone());
        assert_eq!(recent.len(), 2);
        assert_eq!(recent.entries()[0].path(), second.path());
        assert!(recent.forget(second.path()));
        assert!(!recent.forget(second.path()));
        assert_eq!(recent.len(), 1);
        assert!(!recent.is_empty());
    }

    #[test]
    fn the_recent_list_is_written_and_read_back() {
        let sandbox = sandbox();
        let path = sandbox.root.join("config/recent.json");
        let at = UNIX_EPOCH + Duration::from_millis(1_700_000_000_000);
        let mut recent = RecentProjects::load(&path).expect("an absent file is an empty list");
        assert!(recent.path().ends_with("recent.json"));
        assert!(recent.render()[0].contains("no projects"));

        recent.record(crate::project::recent::RecentEntry::new(
            sandbox.root.join("demo"),
            "Demo",
            "3f2b",
            at,
        ));
        let written = recent.save().expect("the list is written");
        assert_eq!(written, path);
        assert!(path.is_file());

        let again = RecentProjects::load(&path).expect("readable");
        assert_eq!(again.entries(), recent.entries());
        assert_eq!(again.entries()[0].opened_at_millis(), 1_700_000_000_000);
        assert_eq!(again.entries()[0].missing_since_millis(), None);
        assert!(
            again.render()[0].starts_with("missing"),
            "{}",
            again.render()[0]
        );

        // A file that is there but is not a list is an error, not a silent empty list.
        std::fs::write(&path, "{\"format\": 1, \"recent\": {}}").expect("a broken list");
        assert!(matches!(
            RecentProjects::load(&path),
            Err(crate::project::format::ProjectError::WrongType { .. })
        ));
        std::fs::write(&path, "{\"format\": 2, \"recent\": []}").expect("a newer list");
        assert!(matches!(
            RecentProjects::load(&path),
            Err(crate::project::format::ProjectError::NewerFormat { .. })
        ));
        std::fs::write(&path, "not json").expect("not JSON");
        assert!(matches!(
            RecentProjects::load(&path),
            Err(crate::project::format::ProjectError::NotJson { .. })
        ));
    }

    #[test]
    fn a_project_that_is_not_there_is_refused_with_a_reason_a_person_can_act_on() {
        let sandbox = sandbox();
        let entry = crate::project::recent::RecentEntry::new(
            sandbox.root.join("nowhere"),
            "Demo",
            "3f2b",
            SystemTime::now(),
        );
        let refusal = ProjectSession::from_recent(&entry).expect_err("nothing is there");
        assert!(refusal.message().contains("not there any more"));
        assert!(refusal.message().contains("forget it"));
        let broken = sandbox.project("broken", "not a project");
        let refusal = ProjectSession::from_recent(&crate::project::recent::RecentEntry::new(
            broken,
            "Broken",
            "3f2b",
            SystemTime::now(),
        ))
        .expect_err("not a project");
        assert!(matches!(refusal, RecentOpen::Unreadable(_)));
    }

    #[test]
    fn the_question_is_a_value_that_can_be_shown_again() {
        let sandbox = sandbox();
        let directory = sandbox.project("demo", PROJECT);
        let root = crate::host::paths::ProjectRoot::new(&directory).expect("a root");
        let question = TrustQuestion::read(&root);
        assert_eq!(question.allows().len(), 2);
        assert_eq!(question.refuses().len(), 2);
        assert!(question.findings()[0].contains("no Cargo.toml"));
        assert!(question.ask().contains(&sandbox.root.display().to_string()));
        assert_eq!(
            question,
            TrustQuestion::read(&root),
            "the question is stable here"
        );
    }
}
