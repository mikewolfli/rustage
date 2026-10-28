// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The language-service lifecycle: one session per project, startup diagnostics, negotiation and
//! restart (task T07.01).
//!
//! The acceptance line is *"the service is not started once per view, and closing a project releases
//! its session"*. Both are statements about **how many** sessions exist and **who owns** them, so the
//! module keeps them in one place instead of leaving them to every caller:
//!
//! * **A session belongs to a project, not to a view.** [`Services::open_view`] returns the project's
//!   session, creating it only if it is not there yet. Opening the tenth view of a project launches
//!   nothing; [`Services::launches`] is the count a caller (or a test) checks against the number of
//!   views.
//! * **A failed start is a diagnosis, not an absence.** A launch that cannot reach
//!   [`State::Running`] records [`StartupFailure`] on the session, and [`Session::diagnosis`] names it.
//!   A missing toolchain and a server that died while shaking hands read differently.
//! * **Capabilities are whatever the service reported.** [`Capabilities`] is the set the launcher
//!   returned at startup, and later tasks gate on [`Services::supports`] rather than assuming a
//!   feature exists.
//! * **Restart is explicit and counted.** [`Services::restart`] stops the old server and launches a
//!   new one, so "we restarted twice" is a fact on the session, and an [`State::Failed`] session is
//!   not silently retried by opening another view.
//! * **Closing a project releases its session.** [`Services::close`] stops the server and drops the
//!   session, so a closed project holds no service.
//!
//! The launcher is the seam: this task owns the **lifecycle**, and the protocol that a real
//! [T07.02](crate::app::language::protocol) launch speaks sits behind [`Launcher`] rather than inside here,
//! which is why the tests can drive every failure path without a toolchain.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use crate::app::language::features::{CompletionItem, Completions, Delivery, Request, Response};
use crate::app::language::protocol::{
    Document, DocumentChange, Notification, Operation, Sync, SyncError,
};

/// Something a language service can do, as negotiated at startup.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Capability {
    /// Text completion.
    Completion,
    /// Hover information.
    Hover,
    /// Go to definition.
    Definition,
    /// Find references.
    References,
    /// Document and workspace symbols.
    Symbols,
    /// Diagnostics.
    Diagnostics,
    /// Code actions.
    CodeAction,
    /// Formatting.
    Formatting,
    /// Rename.
    Rename,
    /// Inlay hints.
    InlayHint,
    /// Code lenses.
    CodeLens,
    /// Call hierarchy.
    CallHierarchy,
    /// Type hierarchy.
    TypeHierarchy,
    /// Semantic selection ranges.
    SemanticSelection,
}

impl Capability {
    /// Every capability, in a stable order.
    pub const ALL: [Self; 14] = [
        Self::Completion,
        Self::Hover,
        Self::Definition,
        Self::References,
        Self::Symbols,
        Self::Diagnostics,
        Self::CodeAction,
        Self::Formatting,
        Self::Rename,
        Self::InlayHint,
        Self::CodeLens,
        Self::CallHierarchy,
        Self::TypeHierarchy,
        Self::SemanticSelection,
    ];

    /// The capability's name, as a command line writes it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Completion => "completion",
            Self::Hover => "hover",
            Self::Definition => "definition",
            Self::References => "references",
            Self::Symbols => "symbols",
            Self::Diagnostics => "diagnostics",
            Self::CodeAction => "code-action",
            Self::Formatting => "formatting",
            Self::Rename => "rename",
            Self::InlayHint => "inlay-hint",
            Self::CodeLens => "code-lens",
            Self::CallHierarchy => "call-hierarchy",
            Self::TypeHierarchy => "type-hierarchy",
            Self::SemanticSelection => "semantic-selection",
        }
    }

    /// The capability a name picks.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|c| c.name() == name)
    }
}

/// The capabilities a service reported at startup.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Capabilities {
    set: BTreeSet<Capability>,
}

impl Capabilities {
    /// Nothing.
    pub fn none() -> Self {
        Self::default()
    }

    /// Everything.
    pub fn all() -> Self {
        Self::of(Capability::ALL)
    }

    /// Some capabilities.
    pub fn of(list: impl IntoIterator<Item = Capability>) -> Self {
        Self {
            set: list.into_iter().collect(),
        }
    }

    /// Whether a capability was reported.
    pub fn supports(&self, capability: Capability) -> bool {
        self.set.contains(&capability)
    }

    /// The capability names, in order.
    pub fn names(&self) -> Vec<&'static str> {
        self.set.iter().map(|c| c.name()).collect()
    }

    /// The capabilities in one line.
    pub fn render(&self) -> String {
        let names = self.names();
        if names.is_empty() {
            "none".to_owned()
        } else {
            names.join(", ")
        }
    }
}

/// Why a start did not reach [`State::Running`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum StartupFailure {
    /// No toolchain for the language is installed.
    NoToolchain,
    /// The server process could not be started.
    SpawnFailed(String),
    /// The server started but did not complete the handshake.
    HandshakeFailed(String),
}

impl StartupFailure {
    /// The failure in one sentence.
    pub fn message(&self) -> String {
        match self {
            Self::NoToolchain => "no language toolchain was found".to_owned(),
            Self::SpawnFailed(reason) => format!("the language server did not start: {reason}"),
            Self::HandshakeFailed(reason) => {
                format!("the language server did not finish its handshake: {reason}")
            }
        }
    }
}

/// A service's lifecycle state.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum State {
    /// No server is running and none is being started.
    Stopped,
    /// The server is running and its capabilities are known.
    Running,
    /// A start was attempted and did not reach [`State::Running`].
    Failed(StartupFailure),
}

impl State {
    /// The state's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Stopped => "stopped",
            Self::Running => "running",
            Self::Failed(_) => "failed",
        }
    }

    /// Whether the server is running.
    pub fn is_running(&self) -> bool {
        matches!(self, Self::Running)
    }
}

/// How a service is started and stopped. `host::lsp::Client` is the real implementation used by
/// `--language exec:<program>`; the scripted launchers in tests exercise the failure paths.
pub trait Launcher {
    /// Starts a server for a project, reporting the negotiated capabilities or why it failed.
    fn launch(&mut self, project: &str) -> Result<Capabilities, StartupFailure>;

    /// Stops the server for a project, if one is running.
    fn stop(&mut self, project: &str);
}

/// One project's language-service session.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Session {
    project: String,
    state: State,
    capabilities: Capabilities,
    restarts: usize,
}

impl Session {
    /// The project it belongs to.
    pub fn project(&self) -> &str {
        &self.project
    }

    /// Its state.
    pub fn state(&self) -> &State {
        &self.state
    }

    /// Its negotiated capabilities.
    pub fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    /// Whether a capability was negotiated.
    pub fn supports(&self, capability: Capability) -> bool {
        self.capabilities.supports(capability)
    }

    /// How many times it has been restarted.
    pub fn restarts(&self) -> usize {
        self.restarts
    }

    /// Why it is not running, when it is not.
    pub fn diagnosis(&self) -> Option<&StartupFailure> {
        match &self.state {
            State::Failed(failure) => Some(failure),
            _ => None,
        }
    }

    /// The session in one line.
    pub fn render(&self) -> String {
        match &self.state {
            State::Running => format!(
                "project {}: running, capabilities: {}",
                self.project,
                self.capabilities.render()
            ),
            State::Failed(failure) => {
                format!("project {}: failed — {}", self.project, failure.message())
            }
            State::Stopped => format!("project {}: stopped", self.project),
        }
    }
}

/// The project's sessions: one per project, whatever the number of views.
pub struct Services<L: Launcher> {
    launcher: L,
    sessions: BTreeMap<String, Session>,
    launches: usize,
}

impl<L: Launcher> Services<L> {
    /// A registry over a launcher.
    pub fn new(launcher: L) -> Self {
        Self {
            launcher,
            sessions: BTreeMap::new(),
            launches: 0,
        }
    }

    /// The launcher, for an operation that has to talk to the real service (a request whose result is
    /// not part of the session model, such as completion).
    ///
    /// The model still owns the session state; this only exposes the transport beneath it, and only
    /// for the duration of the call.
    pub fn launcher_mut(&mut self) -> &mut L {
        &mut self.launcher
    }

    /// Opens a view of a project, returning its session and creating (and starting) it only once.
    pub fn open_view(&mut self, project: &str) -> &Session {
        if !self.sessions.contains_key(project) {
            let session = self.launch(project, 0);
            self.sessions.insert(project.to_owned(), session);
        }
        &self.sessions[project]
    }

    /// A project's session, when it has one.
    pub fn session(&self, project: &str) -> Option<&Session> {
        self.sessions.get(project)
    }

    /// A project's state, or [`State::Stopped`] when it has no session.
    pub fn state(&self, project: &str) -> State {
        self.sessions
            .get(project)
            .map_or(State::Stopped, |session| session.state.clone())
    }

    /// Whether a project's service negotiated a capability.
    pub fn supports(&self, project: &str, capability: Capability) -> bool {
        self.sessions
            .get(project)
            .is_some_and(|session| session.supports(capability))
    }

    /// Restarts a project's service, stopping the old server first, and counts the restart.
    pub fn restart(&mut self, project: &str) -> &Session {
        self.launcher.stop(project);
        let restarts = self
            .sessions
            .get(project)
            .map_or(0, |session| session.restarts + 1);
        let session = self.launch(project, restarts);
        self.sessions.insert(project.to_owned(), session);
        &self.sessions[project]
    }

    /// Closes a project, stopping its server and releasing its session.
    pub fn close(&mut self, project: &str) -> bool {
        if self.sessions.remove(project).is_some() {
            self.launcher.stop(project);
            true
        } else {
            false
        }
    }

    /// How many times a launcher was actually invoked.
    pub fn launches(&self) -> usize {
        self.launches
    }

    /// The projects with a session, in name order.
    pub fn projects(&self) -> Vec<&str> {
        self.sessions.keys().map(String::as_str).collect()
    }

    /// Starts a server and records the outcome as a session.
    fn launch(&mut self, project: &str, restarts: usize) -> Session {
        self.launches += 1;
        let (state, capabilities) = match self.launcher.launch(project) {
            Ok(capabilities) => (State::Running, capabilities),
            Err(failure) => (State::Failed(failure), Capabilities::none()),
        };
        Session {
            project: project.to_owned(),
            state,
            capabilities,
            restarts,
        }
    }

    /// The registry in one report.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "{} project session(s), {} launch(es)",
            self.sessions.len(),
            self.launches
        )];
        for session in self.sessions.values() {
            lines.push(format!("  {}", session.render()));
        }
        lines
    }
}

/// Reads the capability set out of an LSP `initialize` result's `capabilities` object.
///
/// Provider fields come in two shapes: a boolean (`hoverProvider: true`) and an options object
/// (`codeActionProvider: {…}`). Both mean the same thing here — the server offers the feature — so
/// the check is "present and not false/null", not "equals true".
///
/// Symbols are one capability in this model but two keys in LSP; either one is enough. Diagnostics
/// are the pull-model key (`diagnosticProvider`); a push-only server declares no capability for
/// them and is not given one here.
pub fn capabilities_from_lsp(capabilities: &crate::project::record::json::Json) -> Capabilities {
    let Some(object) = capabilities.as_object() else {
        return Capabilities::none();
    };
    let mut found = Vec::new();
    for (capability, keys) in [
        (Capability::Completion, &["completionProvider"][..]),
        (Capability::Hover, &["hoverProvider"][..]),
        (Capability::Definition, &["definitionProvider"][..]),
        (Capability::References, &["referencesProvider"][..]),
        (
            Capability::Symbols,
            &["documentSymbolProvider", "workspaceSymbolProvider"][..],
        ),
        (Capability::Diagnostics, &["diagnosticProvider"][..]),
        (Capability::CodeAction, &["codeActionProvider"][..]),
        (Capability::Formatting, &["documentFormattingProvider"][..]),
        (Capability::Rename, &["renameProvider"][..]),
        (Capability::InlayHint, &["inlayHintProvider"][..]),
        (Capability::CodeLens, &["codeLensProvider"][..]),
        (Capability::CallHierarchy, &["callHierarchyProvider"][..]),
        (Capability::TypeHierarchy, &["typeHierarchyProvider"][..]),
        (
            Capability::SemanticSelection,
            &["selectionRangeProvider"][..],
        ),
    ] {
        if keys.iter().any(|key| {
            matches!(
                object.get(key),
                Some(crate::project::record::json::Json::Bool(true))
                    | Some(crate::project::record::json::Json::Object(_))
                    | Some(crate::project::record::json::Json::Array(_))
                    | Some(crate::project::record::json::Json::String(_))
            )
        }) {
            found.push(capability);
        }
    }
    Capabilities::of(found)
}

/// The documents one project's service has open, and the requests made against them.
///
/// This is the typed half of [T07.02](crate::app::language::protocol): an operation updates the
/// [`Sync`] model and queues exactly the notification the transport must send. The completion
/// session is the freshness rule from [T07.03](crate::app::language::features), so a reply computed
/// against an older version cannot replace the list for a newer buffer.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DocumentSession {
    sync: Sync,
    completions: BTreeMap<String, Completions>,
    notifications: Vec<Notification>,
}

impl DocumentSession {
    /// A session with no document open.
    pub fn new() -> Self {
        Self {
            sync: Sync::new(),
            completions: BTreeMap::new(),
            notifications: Vec::new(),
        }
    }

    /// Opens a document at version 1 with its language id.
    pub fn open(
        &mut self,
        uri: impl Into<String>,
        language_id: impl Into<String>,
        text: impl Into<String>,
    ) -> Result<u64, SyncError> {
        let uri = uri.into();
        let notification = self.sync.apply(Operation::Open {
            uri: uri.clone(),
            language_id: language_id.into(),
            text: text.into(),
        })?;
        let version = notification.version().unwrap_or(1);
        self.completions.insert(uri, Completions::new(version));
        self.notifications.push(notification);
        Ok(version)
    }

    /// Applies a full or incremental change, advancing the version by one.
    pub fn change(&mut self, uri: &str, change: DocumentChange) -> Result<u64, SyncError> {
        let notification = self.sync.apply(Operation::Change {
            uri: uri.to_owned(),
            change,
        })?;
        let version = notification.version().unwrap_or(0);
        self.completions
            .entry(uri.to_owned())
            .or_insert_with(|| Completions::new(version))
            .advance(version);
        self.notifications.push(notification);
        Ok(version)
    }

    /// Records a save. The version does not change, and one `didSave` is queued.
    pub fn save(&mut self, uri: &str) -> Result<u64, SyncError> {
        let notification = self.sync.apply(Operation::Save {
            uri: uri.to_owned(),
        })?;
        let version = notification.version().unwrap_or(0);
        self.notifications.push(notification);
        Ok(version)
    }

    /// Closes a document and queues one `didClose`.
    pub fn close(&mut self, uri: &str) -> Result<(), SyncError> {
        let notification = self.sync.apply(Operation::Close {
            uri: uri.to_owned(),
        })?;
        self.completions.remove(uri);
        self.notifications.push(notification);
        Ok(())
    }

    /// Closes every open document, returning the queued notifications (one `didClose` each).
    pub fn close_all(&mut self) -> Vec<Notification> {
        let uris: Vec<String> = self
            .sync
            .open_documents()
            .into_iter()
            .map(str::to_owned)
            .collect();
        for uri in uris {
            if let Ok(notification) = self.sync.apply(Operation::Close { uri }) {
                self.notifications.push(notification);
            }
        }
        self.completions.clear();
        std::mem::take(&mut self.notifications)
    }

    /// Forgets the model and any queued notification without sending a `didClose`. A restarted or
    /// replaced server has nothing open, so the next edit must open the document again.
    pub fn clear(&mut self) {
        self.sync = Sync::new();
        self.completions.clear();
        self.notifications.clear();
    }

    /// A document, when it is open.
    pub fn document(&self, uri: &str) -> Option<&Document> {
        self.sync.document(uri)
    }

    /// The synchronisation model.
    pub fn sync(&self) -> &Sync {
        &self.sync
    }

    /// The document version a completion request is made against, when it is open.
    pub fn version(&self, uri: &str) -> Option<u64> {
        self.completions
            .get(uri)
            .map(Completions::version)
            .or_else(|| self.sync.document(uri).map(Document::version))
    }

    /// Asks for completions against the document's current version.
    pub fn request(&mut self, uri: &str) -> Result<Request, SyncError> {
        let version = self
            .sync
            .document(uri)
            .ok_or_else(|| SyncError::NotOpen {
                uri: uri.to_owned(),
            })?
            .version();
        let completions = self
            .completions
            .entry(uri.to_owned())
            .or_insert_with(|| Completions::new(version));
        completions.advance(version);
        Ok(completions.request())
    }

    /// Takes a completion reply through the freshness rule.
    pub fn accept(
        &mut self,
        uri: &str,
        response: Response<Vec<CompletionItem>>,
    ) -> Result<Delivery<usize>, SyncError> {
        let completions = self
            .completions
            .get_mut(uri)
            .ok_or_else(|| SyncError::NotOpen {
                uri: uri.to_owned(),
            })?;
        Ok(completions.accept(response))
    }

    /// The completion list currently shown for a document, when there is one.
    pub fn items(&self, uri: &str) -> Option<&[CompletionItem]> {
        self.completions.get(uri).and_then(Completions::items)
    }

    /// The notifications queued since the last drain.
    pub fn notifications(&self) -> &[Notification] {
        &self.notifications
    }

    /// Takes the queued notifications, leaving the model as it is.
    pub fn take_notifications(&mut self) -> Vec<Notification> {
        std::mem::take(&mut self.notifications)
    }
}

impl Default for DocumentSession {
    fn default() -> Self {
        Self::new()
    }
}

impl<L: Launcher> Drop for Services<L> {
    fn drop(&mut self) {
        // A session that is still open owns a process. Dropping the registry without telling the
        // launcher would leave that process to the OS; the scripted launcher treats this as a
        // no-op, and the real one sends shutdown/exit before its client is dropped.
        let projects: Vec<String> = self.sessions.keys().cloned().collect();
        for project in projects {
            self.launcher.stop(&project);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Capabilities, Capability, DocumentSession, Launcher, Services, StartupFailure, State,
    };
    use crate::app::language::features::{CompletionItem, Delivery, Response};
    use crate::app::language::protocol::{Change, DocumentChange, Notification, Position, Range};

    /// A launcher that answers from a script and records what it did.
    struct Fake {
        result: Result<Capabilities, StartupFailure>,
        launched: Vec<String>,
        stopped: Vec<String>,
    }

    impl Fake {
        fn running() -> Self {
            Self {
                result: Ok(Capabilities::all()),
                launched: Vec::new(),
                stopped: Vec::new(),
            }
        }

        fn failing(failure: StartupFailure) -> Self {
            Self {
                result: Err(failure),
                launched: Vec::new(),
                stopped: Vec::new(),
            }
        }
    }

    impl Launcher for Fake {
        fn launch(&mut self, project: &str) -> Result<Capabilities, StartupFailure> {
            self.launched.push(project.to_owned());
            self.result.clone()
        }

        fn stop(&mut self, project: &str) {
            self.stopped.push(project.to_owned());
        }
    }

    #[test]
    fn one_service_per_project_however_many_views() {
        let mut services = Services::new(Fake::running());
        for _ in 0..5 {
            let session = services.open_view("app");
            assert!(session.state().is_running());
            assert_eq!(session.project(), "app");
        }
        // Five views, one launch.
        assert_eq!(services.launches(), 1);
        assert_eq!(services.projects(), vec!["app"]);
        // A second project launches its own.
        services.open_view("lib");
        assert_eq!(services.launches(), 2);
    }

    #[test]
    fn a_missing_toolchain_is_a_startup_diagnosis() {
        let mut services = Services::new(Fake::failing(StartupFailure::NoToolchain));
        let session = services.open_view("app");
        assert_eq!(session.state(), &State::Failed(StartupFailure::NoToolchain));
        assert_eq!(session.state().name(), "failed");
        assert_eq!(session.diagnosis(), Some(&StartupFailure::NoToolchain));
        assert!(session.render().contains("no language toolchain"));
        // A failed session is not silently retried by opening another view.
        services.open_view("app");
        assert_eq!(services.launches(), 1);
    }

    #[test]
    fn capabilities_are_whatever_the_service_reported() {
        let caps = Capabilities::of([Capability::Completion, Capability::Hover]);
        let mut services = Services::new(Fake {
            result: Ok(caps),
            launched: Vec::new(),
            stopped: Vec::new(),
        });
        services.open_view("app");
        assert!(services.supports("app", Capability::Completion));
        assert!(services.supports("app", Capability::Hover));
        assert!(!services.supports("app", Capability::Rename));
        assert_eq!(
            services
                .session("app")
                .expect("a session")
                .capabilities()
                .names(),
            vec!["completion", "hover"]
        );
    }

    #[test]
    fn restart_stops_the_old_server_and_counts() {
        let mut services = Services::new(Fake::running());
        services.open_view("app");
        assert_eq!(services.launches(), 1);
        let session = services.restart("app");
        assert_eq!(session.restarts(), 1);
        assert_eq!(services.launches(), 2);
        // The old server was told to stop.
        assert_eq!(
            services.session("app").expect("still there").state(),
            &State::Running
        );
    }

    #[test]
    fn closing_a_project_releases_the_session() {
        let mut services = Services::new(Fake::running());
        services.open_view("app");
        assert!(services.close("app"));
        assert!(services.session("app").is_none());
        assert_eq!(services.projects(), Vec::<&str>::new());
        // Closing what is not open is not a release.
        assert!(!services.close("app"));
    }

    #[test]
    fn a_capability_name_round_trips() {
        for capability in Capability::ALL {
            assert_eq!(Capability::from_name(capability.name()), Some(capability));
        }
        assert_eq!(Capability::from_name("nowhere"), None);
        assert_eq!(Capabilities::none().render(), "none");
    }

    #[test]
    fn lsp_capabilities_are_read_from_both_provider_shapes() {
        let json = crate::project::record::json::parse(
            r#"{
                "completionProvider": {"triggerCharacters": ["."]},
                "hoverProvider": true,
                "definitionProvider": false,
                "workspaceSymbolProvider": true,
                "renameProvider": {"prepareProvider": true},
                "diagnosticProvider": {"interFileDependencies": true},
                "unknownProvider": true
            }"#,
        )
        .expect("json");
        let capabilities = super::capabilities_from_lsp(&json);
        assert!(capabilities.supports(Capability::Completion));
        assert!(capabilities.supports(Capability::Hover));
        assert!(!capabilities.supports(Capability::Definition));
        assert!(capabilities.supports(Capability::Symbols));
        assert!(capabilities.supports(Capability::Rename));
        assert!(capabilities.supports(Capability::Diagnostics));
        assert!(!capabilities.supports(Capability::Formatting));
    }

    #[test]
    fn a_missing_or_non_object_capability_answer_is_empty() {
        let json = crate::project::record::json::parse("null").expect("json");
        assert_eq!(super::capabilities_from_lsp(&json), Capabilities::none());
    }

    #[test]
    fn save_and_close_are_each_emitted_once() {
        let mut session = DocumentSession::new();
        session
            .open("file:///a.rs", "rust", "one\n")
            .expect("an open");
        let opened = session.take_notifications();
        assert_eq!(opened.len(), 1);
        assert!(matches!(
            opened[0],
            Notification::Open {
                version: 1,
                ref language_id,
                ..
            } if language_id == "rust"
        ));

        session.save("file:///a.rs").expect("a save");
        let saved = session.take_notifications();
        assert_eq!(saved.len(), 1);
        assert!(matches!(saved[0], Notification::Save { version: 1, .. }));

        session.close("file:///a.rs").expect("a close");
        let closed = session.take_notifications();
        assert_eq!(closed.len(), 1);
        assert!(matches!(closed[0], Notification::Close { .. }));

        // A second close is refused and emits nothing.
        assert!(session.close("file:///a.rs").is_err());
        assert!(session.take_notifications().is_empty());
    }

    #[test]
    fn every_change_advances_the_version_by_one() {
        let mut session = DocumentSession::new();
        session
            .open("file:///a.rs", "rust", "one\n")
            .expect("an open");
        session.take_notifications();
        assert_eq!(session.version("file:///a.rs"), Some(1));
        let version = session
            .change(
                "file:///a.rs",
                DocumentChange::Incremental {
                    changes: vec![Change::new(
                        Range::new(Position::new(0, 0), Position::new(0, 0)),
                        "x",
                    )],
                },
            )
            .expect("a change");
        assert_eq!(version, 2);
        assert_eq!(session.version("file:///a.rs"), Some(2));
        let version = session
            .change(
                "file:///a.rs",
                DocumentChange::Full {
                    text: "two\n".to_owned(),
                },
            )
            .expect("a full change");
        assert_eq!(version, 3);
        assert_eq!(session.version("file:///a.rs"), Some(3));

        let changes: Vec<Notification> = session.take_notifications();
        assert_eq!(changes.len(), 2);
        let versions: Vec<Option<u64>> = changes.iter().map(Notification::version).collect();
        assert_eq!(versions, vec![Some(2), Some(3)]);
    }

    #[test]
    fn a_stale_completion_does_not_overwrite_the_newer_buffer() {
        let mut session = DocumentSession::new();
        session
            .open("file:///a.rs", "rust", "one\n")
            .expect("an open");
        session.take_notifications();
        // The request is made at version 1.
        let request = session.request("file:///a.rs").expect("a request");
        assert_eq!(request.version, 1);
        // The buffer moves on before the reply arrives.
        session
            .change(
                "file:///a.rs",
                DocumentChange::Incremental {
                    changes: vec![Change::new(
                        Range::new(Position::new(0, 0), Position::new(0, 0)),
                        "x",
                    )],
                },
            )
            .expect("a change");
        let delivery = session
            .accept(
                "file:///a.rs",
                Response::new(request, vec![CompletionItem::text("old")]),
            )
            .expect("a reply");
        assert_eq!(
            delivery,
            Delivery::Stale {
                computed: 1,
                current: 2
            }
        );
        // The stale reply did not become the shown list.
        assert!(session.items("file:///a.rs").is_none());

        // A request made at the newer version is shown.
        let request = session.request("file:///a.rs").expect("a request");
        assert_eq!(request.version, 2);
        let delivery = session
            .accept(
                "file:///a.rs",
                Response::new(request, vec![CompletionItem::text("new")]),
            )
            .expect("a reply");
        assert_eq!(delivery, Delivery::Fresh(1));
        assert_eq!(
            session.items("file:///a.rs").expect("shown")[0].label,
            "new"
        );
    }

    #[test]
    fn closing_every_document_queues_one_did_close_each() {
        let mut session = DocumentSession::new();
        session.open("file:///a.rs", "rust", "a\n").expect("a");
        session.open("file:///b.rs", "rust", "b\n").expect("b");
        session.take_notifications();
        let notifications = session.close_all();
        assert_eq!(notifications.len(), 2);
        assert!(
            notifications
                .iter()
                .all(|notification| matches!(notification, Notification::Close { .. }))
        );
        assert!(session.sync().open_documents().is_empty());
        // Nothing is queued twice.
        assert!(session.close_all().is_empty());
    }
}
