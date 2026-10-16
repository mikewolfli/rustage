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
//! [T07.02](crate::app::protocol) launch speaks sits behind [`Launcher`] rather than inside here,
//! which is why the tests can drive every failure path without a toolchain.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

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

/// How a service is started and stopped. A real LSP client fills this in T07.02.
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

#[cfg(test)]
mod tests {
    use super::{Capabilities, Capability, Launcher, Services, StartupFailure, State};

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
}
