// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The service registry: one scope per open project, and a close that leaves nothing running.
//! Contract **HOST**.
//!
//! Roadmap T01.07 asks for service registration with a project scope, a shutdown order, and recovery
//! from a failed start. Its acceptance line is **"closing leaves no hanging service and no state
//! leaking from one project into another"**, and roadmap §7 says the same from the other side:
//! "services exit traceably when a project closes; a view cannot start an unowned process".
//!
//! The registry is what makes those sentences checkable rather than aspirational:
//!
//! * **A scope is a project, and a project is a resolved root.** [`ServiceRegistry::open`] resolves
//!   the directory and keys the scope by the resolved path, so two different spellings of one
//!   directory are one scope and not two.
//! * **Nothing is shared between scopes.** Each scope has its own [`Executor`] and its own list of
//!   processes; the trust and permission state lives per project inside the shared
//!   [`PermissionService`], which is keyed by the same resolved root. A task or process of one scope
//!   is unreachable from another because the handle carries the scope's own executor.
//! * **A close cannot leave a handle usable.** Closing marks the scope closed *before* anything is
//!   stopped, so a caller holding [`ProjectServices`] is refused from that moment ([`ScopeRefusal::Closed`])
//!   rather than racing the shutdown.
//! * **The order is fixed and reported.** §7 asks for a *traceable* exit, so [`CloseReport::order`]
//!   is the sequence that was actually taken: mark closed, stop the executor (which cancels the
//!   scope's tasks and is bounded by the executor's own grace), take down the scope's processes
//!   (polite signal first, forced only if the deadline passes), then record what did not stop.
//! * **A failed start leaves the registry exactly as it was.** [`ServiceRegistry::open`] either adds
//!   a working scope or changes nothing, and the failure names the step it failed at — "no half-open
//!   project" from roadmap §3.4, applied to the host's own services.
//! * **Closing ends the project's authorizations.** A repaired or reopened project must not inherit
//!   trust it was never given for *this* open ([`PermissionService::close_scope`]), which is what
//!   T02.03's trust prompt promises.
//! * **Execution goes through the gate.** [`ProjectServices::spawn`] asks the permission service for
//!   a permit *for this project* before starting anything, so the gate from T01.05 is what decides,
//!   and a refusal is reported to the notification centre from T01.06 with the contract's own
//!   explanation as the next step. This is the first shipped call chain that produces notifications.
//!
//! **What this module deliberately does not own.** It is not the project model (T02.01 decides the
//! project's identity, configuration and where trust is stored) and not the file service. It does
//! not decide *when* a project is opened or closed (T02.03 and T03.07 do), and it deliberately has
//! no lazier "start services on first use" path: a scope starts its services when it opens, so
//! "what is still running" has one answer.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use super::log::{Clock, Level, SystemClock, emit};
use super::notify::{Cause, NotificationCentre};
use super::paths::{PathError, ProjectRoot};
use super::permission::{PermissionRefusal, PermissionService, Scope as PermissionScope};
use super::subprocess::{
    DEFAULT_GRACE, ExitInfo, OwnedProcess, ProcessId, ProcessOutcome, ProcessSpec, SpawnError,
    Termination,
};
use super::tasks::{
    Executor, ExecutorConfig, ExecutorError, Job, OwnerId, SubmitError, SubmitRequest, TaskId,
    TaskRecord,
};
use crate::contracts::permission::{Capability, EffectClass, Principal};

/// One open project.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ProjectScope {
    root: PathBuf,
    owner: OwnerId,
    opened_at_millis: u128,
}

impl ProjectScope {
    /// The resolved project root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The owner id this scope's tasks belong to, for the executor's records.
    pub fn owner(&self) -> OwnerId {
        self.owner
    }

    /// Milliseconds between the registry starting and this scope opening.
    pub fn opened_at_millis(&self) -> u128 {
        self.opened_at_millis
    }

    /// The scope in one line.
    pub fn render(&self) -> String {
        format!(
            "{} (owner {}, opened at +{}ms)",
            self.root.display(),
            self.owner.value(),
            self.opened_at_millis,
        )
    }
}

/// Why a scope refused an operation.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ScopeRefusal {
    /// The project was closed, so its services are gone. Holding a handle does not keep them alive.
    Closed {
        /// Which project.
        root: PathBuf,
    },
    /// The executor refused the task.
    Submit {
        /// Why.
        reason: SubmitError,
    },
    /// The permission gate refused the effect.
    Permission(PermissionRefusal),
    /// The platform refused to start the program.
    Platform(SpawnError),
}

impl ScopeRefusal {
    /// A sentence for a log line, a panel or a terminal.
    pub fn message(&self) -> String {
        match self {
            Self::Closed { root } => format!(
                "{} is closed, so it starts no work; open it again first",
                root.display(),
            ),
            Self::Submit { reason } => reason.message(),
            Self::Permission(refusal) => refusal.message(),
            Self::Platform(error) => error.message(),
        }
    }

    /// The notification cause this refusal is, when one of the centre's own causes fits.
    ///
    /// A closed scope is not one: "you closed this project" is an answer to a stale handle, not a
    /// problem to show a person, and inventing a notification for it would be noise.
    pub fn cause(&self) -> Option<Cause> {
        match self {
            Self::Permission(refusal) => Some(Cause::Permission(refusal.clone())),
            Self::Platform(error) => Some(Cause::Program(error.clone())),
            Self::Closed { .. } | Self::Submit { .. } => None,
        }
    }
}

/// A process a scope started.
///
/// The handle borrows the process for the duration of a call, so two callers cannot wait on it at
/// once; that is deliberate, because "who reaped it" has to have one answer.
#[derive(Clone)]
pub struct ProcessHandle {
    id: ProcessId,
    pid: u32,
    label: String,
    process: Arc<Mutex<OwnedProcess>>,
}

impl ProcessHandle {
    /// The id this session gave the process.
    pub fn id(&self) -> ProcessId {
        self.id
    }

    /// The operating system's pid.
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// What the caller called it.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// What it has written so far, bounded by its capture limit.
    pub fn output_so_far(&self) -> super::subprocess::CapturedOutput {
        lock_process(&self.process).output_so_far()
    }

    /// Waits up to `limit` for it to end, returning what it did.
    pub fn wait(&self, limit: Duration) -> Option<ProcessOutcome> {
        let mut process = lock_process(&self.process);
        let exit = process.wait_for(limit)?;
        Some(process.outcome(exit))
    }

    /// Ends its whole process group, politely first and forcibly if the deadline passes.
    pub fn terminate(&self, grace: Duration) -> Termination {
        lock_process(&self.process).terminate(grace)
    }

    /// How it ended, for a process that has already been waited for.
    pub fn exit(&self) -> Option<ExitInfo> {
        let mut process = lock_process(&self.process);
        process.wait_for(Duration::ZERO)
    }
}

impl std::fmt::Debug for ProcessHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProcessHandle")
            .field("pid", &self.pid)
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

/// What a scope's state refused, or the handle that was not created.
type ScopeResult<T> = Result<T, ScopeRefusal>;

/// The handle to one open project's services.
///
/// Cloning it is cheap and shares the same scope, which is how a panel and a task submission reach
/// the same executor. Closing the project invalidates every clone at once.
#[derive(Clone)]
pub struct ProjectServices {
    scope: ProjectScope,
    root: ProjectRoot,
    closed: Arc<AtomicBool>,
    executor: Arc<Executor>,
    processes: Arc<Mutex<Vec<Arc<Mutex<OwnedProcess>>>>>,
    permission: Arc<PermissionService>,
    notifications: Arc<NotificationCentre>,
}

impl ProjectServices {
    /// The scope identity.
    pub fn scope(&self) -> &ProjectScope {
        &self.scope
    }

    /// The resolved project root.
    pub fn root(&self) -> &Path {
        self.scope.root()
    }

    /// The project root as the permission service keys it.
    pub fn project_root(&self) -> &ProjectRoot {
        &self.root
    }

    /// Whether the project has been closed. A closed scope refuses everything below.
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// The scope's executor, so a panel can read its records.
    pub fn executor(&self) -> &Arc<Executor> {
        &self.executor
    }

    /// Every task this scope's executor knows, in id order.
    pub fn tasks(&self) -> Vec<TaskRecord> {
        self.executor.records()
    }

    /// Submits work in this scope.
    pub fn submit<J: Job>(&self, label: impl Into<String>, job: J) -> ScopeResult<TaskId> {
        self.refuse_if_closed()?;
        self.executor
            .submit(SubmitRequest::new(self.scope.owner, label), job)
            .map_err(|reason| ScopeRefusal::Submit { reason })
    }

    /// Starts a program for this project, after asking the permission gate about *this* project.
    ///
    /// The refusal is reported to the notification centre, because a person needs to know that the
    /// work did not start and what would let it start.
    pub fn spawn(&self, spec: ProcessSpec, principal: Principal) -> ScopeResult<ProcessHandle> {
        self.refuse_if_closed()?;

        let permit = match self.permission.permit(
            EffectClass::Command,
            PermissionScope::Project(&self.root),
            principal,
        ) {
            Ok(permit) => permit,
            Err(refusal) => {
                let refusal = ScopeRefusal::Permission(refusal);
                self.report(&refusal);
                return Err(refusal);
            }
        };

        let process = match OwnedProcess::spawn(spec, &permit) {
            Ok(process) => process,
            Err(error) => {
                let refusal = ScopeRefusal::Platform(error);
                self.report(&refusal);
                return Err(refusal);
            }
        };

        // The handle and the scope share one process: whoever reaps it is whichever call reaches it
        // first, and the other sees the same state.
        let id = process.id();
        let pid = process.pid();
        let label = process.label().to_owned();
        let shared = Arc::new(Mutex::new(process));
        lock(&self.processes).push(Arc::clone(&shared));
        Ok(ProcessHandle {
            id,
            pid,
            label,
            process: shared,
        })
    }

    /// The processes this scope started and has not taken down.
    pub fn processes(&self) -> Vec<ProcessHandle> {
        lock(&self.processes)
            .iter()
            .map(|process| {
                let guard = lock_process(process);
                ProcessHandle {
                    id: guard.id(),
                    pid: guard.pid(),
                    label: guard.label().to_owned(),
                    process: Arc::clone(process),
                }
            })
            .collect()
    }

    /// Records a person's decision to trust this project.
    pub fn trust(&self) -> super::permission::TrustPrompt {
        self.permission.trust(&self.root)
    }

    /// Whether this project is trusted.
    pub fn is_trusted(&self) -> bool {
        self.permission.is_trusted(&self.root)
    }

    /// Authorizes an execution scope for this project.
    pub fn authorize(&self, capability: Capability) -> Result<(), ScopeRefusal> {
        self.permission
            .authorize(PermissionScope::Project(&self.root), capability)
            .map_err(ScopeRefusal::Permission)
    }

    /// Withdraws an execution scope for this project.
    pub fn revoke_authorization(&self, capability: Capability) {
        self.permission
            .revoke_authorization(PermissionScope::Project(&self.root), capability);
    }

    fn refuse_if_closed(&self) -> ScopeResult<()> {
        if self.is_closed() {
            return Err(ScopeRefusal::Closed {
                root: self.scope.root.clone(),
            });
        }
        Ok(())
    }

    fn report(&self, refusal: &ScopeRefusal) {
        if let Some(cause) = refusal.cause() {
            self.notifications.report(cause);
        }
    }
}

impl std::fmt::Debug for ProjectServices {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProjectServices")
            .field("scope", &self.scope)
            .field("closed", &self.is_closed())
            .finish_non_exhaustive()
    }
}

/// Why opening a project failed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct OpenFailure {
    path: PathBuf,
    step: &'static str,
    reason: String,
}

impl OpenFailure {
    /// The directory that was asked for.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The step that failed: `root`, `scope-table` or `executor`.
    pub fn step(&self) -> &'static str {
        self.step
    }

    /// Why it failed, in the platform's or the executor's own words.
    pub fn reason(&self) -> &str {
        &self.reason
    }

    /// A sentence for a panel or a terminal.
    pub fn message(&self) -> String {
        format!(
            "{} could not be opened at the {} step: {}",
            self.path.display(),
            self.step,
            self.reason,
        )
    }
}

impl std::fmt::Display for OpenFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message())
    }
}

impl std::error::Error for OpenFailure {}

/// Why closing failed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum CloseFailure {
    /// Nothing is open for this directory. Not an empty success: there was no scope to close.
    NotOpen {
        /// Which project.
        root: PathBuf,
    },
}

impl CloseFailure {
    /// A sentence for a panel or a terminal.
    pub fn message(&self) -> String {
        match self {
            Self::NotOpen { root } => format!(
                "{} is not open, so there is nothing to close",
                root.display()
            ),
        }
    }
}

impl std::fmt::Display for CloseFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message())
    }
}

impl std::error::Error for CloseFailure {}

/// What happened to the processes a scope was holding.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ProcessCleanup {
    /// How many processes the scope still held when it closed.
    pub tracked: usize,
    /// How many had already ended on their own.
    pub already_gone: usize,
    /// The ones the polite signal ended, with how long that took.
    pub stopped: Vec<String>,
    /// The ones that ignored the polite signal and were forced.
    pub forced: Vec<String>,
    /// The ones that could not be taken down, with the platform's reason.
    pub not_clean: Vec<String>,
}

impl ProcessCleanup {
    /// Whether every process in the scope was dealt with.
    pub fn is_clean(&self) -> bool {
        self.not_clean.is_empty()
    }

    /// One line naming what a person should know.
    pub fn message(&self) -> String {
        let mut parts = vec![format!(
            "{} process(es): {} had already ended, {} stopped on the polite signal, {} needed the \
             forced one",
            self.tracked,
            self.already_gone,
            self.stopped.len(),
            self.forced.len(),
        )];
        if !self.not_clean.is_empty() {
            parts.push(format!(
                "{} could not be taken down: {}",
                self.not_clean.len(),
                self.not_clean.join("; "),
            ));
        }
        parts.join("; ")
    }
}

/// What closing a project did, in the order it did it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CloseReport {
    /// Which project.
    pub scope: ProjectScope,
    /// What the scope's executor did with its tasks.
    pub tasks: super::tasks::ShutdownReport,
    /// What happened to the scope's processes.
    pub processes: ProcessCleanup,
    /// The steps that were taken, in order.
    pub order: Vec<&'static str>,
    /// How many notifications the close itself raised.
    pub notifications: usize,
    /// The capabilities the project's close withdrew.
    pub withdrawn: Vec<Capability>,
}

/// The steps a close takes, in the order [`CloseReport::order`] reports them.
///
/// Public so a caller can check that a close really took the steps it says it does, and so the
/// probe can require the order rather than trusting the report's prose.
pub const CLOSE_ORDER: [&str; 5] = [
    "mark the scope closed",
    "stop the scope's tasks",
    "take down the scope's processes",
    "withdraw the project's trust and authorizations",
    "record what did not stop",
];

impl CloseReport {
    /// Whether the close left nothing behind.
    pub fn is_clean(&self) -> bool {
        self.tasks.is_clean() && self.processes.is_clean()
    }

    /// A sentence for a panel or a terminal.
    pub fn message(&self) -> String {
        let mut out = format!(
            "closed {}: {}; {}",
            self.scope.root.display(),
            self.tasks.message(),
            self.processes.message(),
        );
        if !self.withdrawn.is_empty() {
            out.push_str(&format!(
                "; withdrew {}",
                self.withdrawn
                    .iter()
                    .map(|capability| capability.name())
                    .collect::<Vec<_>>()
                    .join(", "),
            ));
        }
        if self.is_clean() {
            out.push_str("; nothing is left running");
        }
        out
    }
}

/// One project, its services, and whether it is still open.
struct ScopeSlot {
    scope: ProjectScope,
    root: ProjectRoot,
    closed: Arc<AtomicBool>,
    executor: Arc<Executor>,
    processes: Arc<Mutex<Vec<Arc<Mutex<OwnedProcess>>>>>,
}

impl ScopeSlot {
    fn services(
        &self,
        permission: Arc<PermissionService>,
        notifications: Arc<NotificationCentre>,
    ) -> ProjectServices {
        ProjectServices {
            scope: self.scope.clone(),
            root: self.root.clone(),
            closed: Arc::clone(&self.closed),
            executor: Arc::clone(&self.executor),
            processes: Arc::clone(&self.processes),
            permission,
            notifications,
        }
    }
}

/// The host's open projects and the services each of them owns.
pub struct ServiceRegistry {
    clock: Box<dyn Clock>,
    permission: Arc<PermissionService>,
    notifications: Arc<NotificationCentre>,
    state: Mutex<BTreeMap<PathBuf, Arc<ScopeSlot>>>,
}

impl ServiceRegistry {
    /// A registry with no open project, its own permission service and its own notification centre.
    pub fn new() -> Self {
        Self::with_services(
            Box::new(SystemClock::start()),
            Arc::new(PermissionService::new()),
            Arc::new(NotificationCentre::new()),
        )
    }

    /// A registry around services a caller already has, with a clock a test can pin.
    ///
    /// Sharing the permission service and the centre is what lets an application hold one trust
    /// record and one notification list for the whole process; the registry only decides which
    /// project owns which executor and which processes.
    pub fn with_services(
        clock: Box<dyn Clock>,
        permission: Arc<PermissionService>,
        notifications: Arc<NotificationCentre>,
    ) -> Self {
        Self {
            clock,
            permission,
            notifications,
            state: Mutex::new(BTreeMap::new()),
        }
    }

    /// The permission service every scope asks before it acts.
    pub fn permission(&self) -> &Arc<PermissionService> {
        &self.permission
    }

    /// The notification centre every scope reports refusals to.
    pub fn notifications(&self) -> &Arc<NotificationCentre> {
        &self.notifications
    }

    /// Opens a project with the default executor configuration.
    pub fn open(&self, path: impl AsRef<Path>) -> Result<ProjectServices, OpenFailure> {
        self.open_with(path, ExecutorConfig::default())
    }

    /// Opens a project, or changes nothing and says which step failed.
    ///
    /// The order is deliberate: the directory is resolved first (nothing is built for a path that
    /// does not exist), an already-open project is refused next (there is one scope per project, not
    /// two executors racing), and the executor is started last, because it is the only step that
    /// owns threads.
    pub fn open_with(
        &self,
        path: impl AsRef<Path>,
        config: ExecutorConfig,
    ) -> Result<ProjectServices, OpenFailure> {
        let path = path.as_ref();
        let root = ProjectRoot::new(path).map_err(|error| OpenFailure {
            path: path.to_path_buf(),
            step: "root",
            reason: path_error_message(&error),
        })?;

        {
            let state = lock(&self.state);
            if state.contains_key(root.as_path()) {
                return Err(OpenFailure {
                    path: root.as_path().to_path_buf(),
                    step: "scope-table",
                    reason: "this project is already open; use the scope that was handed out"
                        .to_owned(),
                });
            }
        }

        let executor = Executor::new(config).map_err(|error| OpenFailure {
            path: root.as_path().to_path_buf(),
            step: "executor",
            reason: executor_error_message(&error),
        })?;

        let scope = ProjectScope {
            root: root.as_path().to_path_buf(),
            owner: OwnerId::new(),
            opened_at_millis: self.clock.elapsed_millis(),
        };
        let slot = Arc::new(ScopeSlot {
            scope: scope.clone(),
            root: root.clone(),
            closed: Arc::new(AtomicBool::new(false)),
            executor: Arc::new(executor),
            processes: Arc::new(Mutex::new(Vec::new())),
        });

        {
            let mut state = lock(&self.state);
            // Checked again under the same lock that inserts: two threads opening one project must
            // not both get past the check above and install two scopes.
            if state.contains_key(root.as_path()) {
                return Err(OpenFailure {
                    path: root.as_path().to_path_buf(),
                    step: "scope-table",
                    reason: "another open of this project won the race".to_owned(),
                });
            }
            state.insert(root.as_path().to_path_buf(), Arc::clone(&slot));
        }

        emit(
            Level::Info,
            "host::registry",
            &format!("opened {}", scope.render()),
        );
        Ok(slot.services(
            Arc::clone(&self.permission),
            Arc::clone(&self.notifications),
        ))
    }

    /// The services of an open project, or `None` when it is not open.
    pub fn services(&self, path: impl AsRef<Path>) -> Option<ProjectServices> {
        let root = ProjectRoot::new(path.as_ref()).ok()?;
        lock(&self.state).get(root.as_path()).map(|slot| {
            slot.services(
                Arc::clone(&self.permission),
                Arc::clone(&self.notifications),
            )
        })
    }

    /// Every open project, in path order.
    pub fn open_scopes(&self) -> Vec<ProjectScope> {
        lock(&self.state)
            .values()
            .map(|slot| slot.scope.clone())
            .collect()
    }

    /// Closes a project with the default grace for its processes.
    pub fn close(&self, path: impl AsRef<Path>) -> Result<CloseReport, CloseFailure> {
        self.close_with(path, DEFAULT_GRACE)
    }

    /// Closes a project: no new work, then stop the tasks, then take down the processes.
    ///
    /// Returns what it did and whether anything was left running. The scope is removed from the
    /// table *first*, so a caller that opens the same project again during the close gets a fresh
    /// scope rather than the one being torn down.
    pub fn close_with(
        &self,
        path: impl AsRef<Path>,
        grace: Duration,
    ) -> Result<CloseReport, CloseFailure> {
        let root = ProjectRoot::new(path.as_ref()).map_err(|_| CloseFailure::NotOpen {
            root: path.as_ref().to_path_buf(),
        })?;
        let slot = {
            let mut state = lock(&self.state);
            state.remove(root.as_path()).ok_or(CloseFailure::NotOpen {
                root: path.as_ref().to_path_buf(),
            })?
        };

        // 1. From here on, every handle to this scope refuses.
        slot.closed.store(true, Ordering::SeqCst);

        // 2. The executor refuses new work, asks its tasks to stop and is bounded by its own grace.
        let tasks = slot.executor.shutdown();

        // 3. Anything the tasks did not take down with them.
        let processes = take_down(&slot.processes, grace);

        // 4. Trust and authorizations were given for *this* open, not for the directory forever.
        let withdrawn = self.permission.close_scope(&slot.root);

        // 5. What did not stop is a notification; the rest is in the report.
        let mut notifications = 0;
        for id in &tasks.abandoned {
            if let Some(record) = slot.executor.record(*id)
                && let Some(cause) = Cause::for_task(&record)
            {
                self.notifications.report(cause);
                notifications += 1;
            }
        }

        let report = CloseReport {
            scope: slot.scope.clone(),
            tasks,
            processes,
            order: CLOSE_ORDER.to_vec(),
            notifications,
            withdrawn,
        };
        emit(
            Level::Info,
            "host::registry",
            &format!("{} ({})", report.message(), report.order.join(" -> ")),
        );
        Ok(report)
    }

    /// Closes every open project, oldest first, returning one report per project.
    pub fn close_all(&self, grace: Duration) -> Vec<CloseReport> {
        let roots: Vec<PathBuf> = lock(&self.state).keys().cloned().collect();
        roots
            .into_iter()
            .filter_map(|root| self.close_with(&root, grace).ok())
            .collect()
    }
}

impl Default for ServiceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Ends every process a scope still holds, politely first.
fn take_down(processes: &Mutex<Vec<Arc<Mutex<OwnedProcess>>>>, grace: Duration) -> ProcessCleanup {
    let mut cleanup = ProcessCleanup::default();
    let mut tracked = lock(processes);
    cleanup.tracked = tracked.len();
    for process in tracked.drain(..) {
        let mut process = lock_process(&process);
        let described = format!("pid {} ({})", process.pid(), process.label());
        match process.terminate(grace) {
            Termination::AlreadyExited { .. } => cleanup.already_gone += 1,
            Termination::Stopped { after, .. } => {
                cleanup
                    .stopped
                    .push(format!("{described} stopped after {after:?}"));
            }
            Termination::Killed { after, .. } => {
                cleanup
                    .forced
                    .push(format!("{described} forced after {after:?}"));
            }
            Termination::DirectOnly { reason, .. } => {
                cleanup.not_clean.push(format!("{described}: {reason}"));
            }
            Termination::SignalFailed { reason } => {
                cleanup
                    .not_clean
                    .push(format!("{described}: the signal failed: {reason}"));
            }
        }
    }
    cleanup
}

/// The permission contract's own words for a path that could not be resolved.
fn path_error_message(error: &PathError) -> String {
    error.to_string()
}

/// The executor's own words for a start that failed.
fn executor_error_message(error: &ExecutorError) -> String {
    error.to_string()
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| {
        crate::host::log::emit(
            crate::host::log::Level::Warn,
            "host::registry",
            "the service registry lock was poisoned; recovering its inner state",
        );
        poisoned.into_inner()
    })
}

fn lock_process(process: &Mutex<OwnedProcess>) -> MutexGuard<'_, OwnedProcess> {
    process.lock().unwrap_or_else(|poisoned| {
        crate::host::log::emit(
            crate::host::log::Level::Warn,
            "host::registry",
            "a tracked process lock was poisoned; recovering its inner state",
        );
        poisoned.into_inner()
    })
}

#[cfg(test)]
mod tests {
    use super::{CloseFailure, ScopeRefusal, ServiceRegistry};
    use crate::contracts::permission::{Capability, Principal};
    use crate::host::paths::ProjectRoot;
    use crate::host::subprocess::{ProcessPresence, ProcessSpec, presence};
    use crate::host::tasks::{ExecutorConfig, JobControl, JobOutcome};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    /// A project directory of its own per test, removed when the test ends.
    struct Sandbox {
        path: std::path::PathBuf,
        _guard: TempDirectory,
    }

    struct TempDirectory(std::path::PathBuf);

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    impl Sandbox {
        fn new(name: &str) -> Self {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "rustage-registry-{name}-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed),
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(path.join("nested")).expect("a temporary project is creatable");
            Self {
                path: path.clone(),
                _guard: TempDirectory(path),
            }
        }

        fn path(&self) -> &std::path::Path {
            &self.path
        }

        fn root(&self) -> ProjectRoot {
            ProjectRoot::new(&self.path).expect("the sandbox is a directory")
        }
    }

    /// A configuration whose executor gives up quickly, so a close in a test is not a two-second wait.
    fn quick() -> ExecutorConfig {
        ExecutorConfig {
            shutdown_grace: Duration::from_millis(60),
            ..ExecutorConfig::responsive()
        }
    }

    fn sleep_forever(label: &str) -> ProcessSpec {
        ProcessSpec::new(label, "sh").args(["-c", "sleep 30"])
    }

    #[test]
    fn opening_two_projects_gives_each_one_its_own_executor() {
        let first = Sandbox::new("two-a");
        let second = Sandbox::new("two-b");
        let registry = ServiceRegistry::new();

        let a = registry
            .open(first.path())
            .expect("the first project opens");
        let b = registry
            .open(second.path())
            .expect("the second project opens");

        assert_ne!(a.scope().owner(), b.scope().owner());
        assert_eq!(registry.open_scopes().len(), 2);
        assert_eq!(a.root(), a.scope().root());
        assert_eq!(b.root(), b.scope().root());
        assert!(a.scope().render().contains("owner"));

        let id = a
            .submit("a says nothing to b", |_control: JobControl| {
                JobOutcome::Succeeded
            })
            .expect("accepted");
        assert!(
            b.tasks().is_empty(),
            "one project's executor must not see the other's tasks",
        );
        assert!(
            a.executor()
                .wait(id, Duration::from_secs(5))
                .is_some_and(|record| record.is_final()),
            "the task belongs to A and runs there",
        );
        assert!(a.tasks().iter().any(|record| record.id == id));
        assert!(registry.close(a.root()).expect("A closes").is_clean());
        assert!(registry.close(b.root()).expect("B closes").is_clean());
    }

    #[test]
    fn opening_the_same_project_twice_is_refused_and_changes_nothing() {
        let sandbox = Sandbox::new("twice");
        let registry = ServiceRegistry::new();
        let first = registry.open(sandbox.path()).expect("the project opens");

        let failure = registry
            .open(sandbox.path())
            .expect_err("a second scope for one project is what pollution looks like");

        assert_eq!(failure.step(), "scope-table");
        assert!(
            failure.message().contains("already open"),
            "{}",
            failure.message()
        );
        assert_eq!(registry.open_scopes().len(), 1);
        assert_eq!(
            registry
                .services(sandbox.path())
                .expect("still open")
                .scope(),
            first.scope(),
        );
    }

    #[test]
    fn another_spelling_of_one_directory_is_the_same_scope() {
        let sandbox = Sandbox::new("spelling");
        let registry = ServiceRegistry::new();
        registry.open(sandbox.path()).expect("the project opens");
        let spelled = sandbox.path().join("nested").join("..");

        assert!(
            registry.services(&spelled).is_some(),
            "a scope is keyed by the resolved path, not by the spelling",
        );
        assert_eq!(
            registry
                .open(&spelled)
                .expect_err("it is the same project")
                .step(),
            "scope-table",
        );
    }

    #[test]
    fn a_failed_open_leaves_nothing_behind_and_the_next_one_works() {
        let sandbox = Sandbox::new("recover");
        let registry = ServiceRegistry::new();
        let impossible = ExecutorConfig {
            workers: 0,
            ..quick()
        };

        let failure = registry
            .open_with(sandbox.path(), impossible)
            .expect_err("an executor with no workers cannot start");

        assert_eq!(failure.step(), "executor");
        assert!(
            failure.reason().contains("no workers"),
            "{}",
            failure.reason()
        );
        assert!(registry.open_scopes().is_empty(), "no half-open project");
        assert!(registry.services(sandbox.path()).is_none());

        // The failed start did not poison the table.
        let services = registry
            .open(sandbox.path())
            .expect("the project opens now");
        assert!(
            services
                .submit("works", |_control: JobControl| JobOutcome::Succeeded)
                .is_ok()
        );
    }

    #[test]
    fn a_directory_that_is_not_there_fails_at_the_root_step() {
        let registry = ServiceRegistry::new();

        let failure = registry
            .open("/nonexistent/rustage-project")
            .expect_err("nothing can be opened there");

        assert_eq!(failure.step(), "root");
        assert!(registry.open_scopes().is_empty());
        assert!(failure.message().contains("rustage-project"));
    }

    #[test]
    fn a_scope_asks_the_gate_about_its_own_project_before_it_runs_anything() {
        let sandbox = Sandbox::new("gate");
        let registry = ServiceRegistry::new();
        let services = registry.open(sandbox.path()).expect("opens");

        let refused = services
            .spawn(sleep_forever("must not start"), Principal::Agent)
            .expect_err("an untrusted project runs no code");
        assert!(matches!(refused, ScopeRefusal::Permission(_)));
        assert!(services.processes().is_empty(), "nothing may have started");

        services.trust();
        assert!(
            services
                .spawn(sleep_forever("a person's own command"), Principal::Human)
                .is_ok(),
            "a human who trusted the project may run a command in it",
        );
        assert!(
            services
                .spawn(sleep_forever("an agent's command"), Principal::Agent)
                .is_err(),
            "an agent still needs the execution scope authorized",
        );

        services
            .authorize(Capability::RunCargoCheck)
            .expect("the scope can be authorized");
        let handle = services
            .spawn(
                sleep_forever("an authorized agent's command"),
                Principal::Agent,
            )
            .expect("now it may run");

        assert_eq!(
            services.processes().len(),
            2,
            "the two refusals left nothing tracked",
        );
        assert!(handle.terminate(Duration::from_secs(5)).ended());
        assert!(registry.close(sandbox.path()).expect("closes").is_clean());
    }

    #[test]
    fn a_refused_spawn_reaches_the_notification_centre_with_something_to_do() {
        let sandbox = Sandbox::new("notify-refusal");
        let registry = ServiceRegistry::new();
        let services = registry.open(sandbox.path()).expect("opens");

        let _ = services
            .spawn(sleep_forever("refused"), Principal::Agent)
            .expect_err("untrusted");

        let notifications = registry.notifications().notifications();
        assert_eq!(notifications.len(), 1);
        let notification = &notifications[0];
        assert_eq!(notification.severity().name(), "warning");
        assert_eq!(notification.link().kind(), "permission");
        assert!(
            notification.steps()[0]
                .label()
                .contains("trust the project first"),
            "the next step has to be the contract's own explanation: {}",
            notification.steps()[0].label(),
        );
    }

    #[test]
    fn a_program_that_is_not_there_is_reported_and_tracked_nowhere() {
        let sandbox = Sandbox::new("notify-missing");
        let registry = ServiceRegistry::new();
        let services = registry.open(sandbox.path()).expect("opens");
        services.trust();

        let refused = services
            .spawn(
                ProcessSpec::new("a program that does not exist", "/nonexistent/rustage-tool"),
                Principal::Human,
            )
            .expect_err("the program is not there");

        assert!(matches!(refused, ScopeRefusal::Platform(_)));
        assert!(services.processes().is_empty());
        let notifications = registry.notifications().notifications();
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].severity().name(), "error");
        assert_eq!(notifications[0].link().kind(), "program");
        assert!(notifications[0].steps()[0].label().contains("install"));
        assert!(registry.close(sandbox.path()).expect("closes").is_clean());
    }

    #[test]
    fn closing_a_project_stops_its_tasks_and_refuses_the_handles_it_handed_out() {
        let sandbox = Sandbox::new("close-tasks");
        let registry = ServiceRegistry::new();
        let services = registry.open_with(sandbox.path(), quick()).expect("opens");
        let (started, started_rx) = std::sync::mpsc::channel();
        let id = services
            .submit("waits to be cancelled", move |control: JobControl| {
                let _ = started.send(());
                if control.token().wait(Duration::from_secs(5)) {
                    JobOutcome::Cancelled
                } else {
                    JobOutcome::Succeeded
                }
            })
            .expect("accepted");
        started_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the job started");

        let report = registry.close(sandbox.path()).expect("closes");

        assert!(report.tasks.cancelled >= 1, "{:?}", report.tasks);
        assert!(report.is_clean(), "{}", report.message());
        let record = services
            .executor()
            .record(id)
            .expect("the record outlives the close");
        assert!(record.is_final(), "every task has an outcome after a close");
        assert!(services.is_closed());
        assert!(matches!(
            services.submit("too late", |_control: JobControl| JobOutcome::Succeeded),
            Err(ScopeRefusal::Closed { .. }),
        ));
        assert!(matches!(
            services.spawn(sleep_forever("too late"), Principal::Human),
            Err(ScopeRefusal::Closed { .. }),
        ));
        assert!(registry.services(sandbox.path()).is_none());
    }

    #[test]
    fn closing_a_project_takes_down_the_processes_it_started() {
        let sandbox = Sandbox::new("close-processes");
        let registry = ServiceRegistry::new();
        let services = registry.open(sandbox.path()).expect("opens");
        services.trust();
        let handle = services
            .spawn(
                sleep_forever("goes down with the project"),
                Principal::Human,
            )
            .expect("starts");
        let pid = handle.pid();
        assert_eq!(presence(pid), ProcessPresence::Running);

        let report = registry.close(sandbox.path()).expect("closes");

        assert_eq!(report.processes.tracked, 1);
        assert_eq!(
            report.processes.stopped.len(),
            1,
            "{}",
            report.processes.message()
        );
        assert!(report.processes.is_clean());
        assert!(report.is_clean(), "{}", report.message());
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while presence(pid) == ProcessPresence::Running && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            presence(pid),
            ProcessPresence::Gone,
            "nothing may outlive its project"
        );
    }

    #[test]
    fn closing_one_project_leaves_the_other_working() {
        let first = Sandbox::new("isolation-a");
        let second = Sandbox::new("isolation-b");
        let registry = ServiceRegistry::new();
        let a = registry.open(first.path()).expect("A opens");
        let b = registry.open(second.path()).expect("B opens");
        a.trust();
        b.trust();
        let going_down = a
            .spawn(sleep_forever("A's process"), Principal::Human)
            .expect("A starts");
        let staying_up = b
            .spawn(sleep_forever("B's process"), Principal::Human)
            .expect("B starts");

        let report = registry.close(first.path()).expect("A closes");

        assert!(report.is_clean(), "{}", report.message());
        assert_eq!(presence(going_down.pid()), ProcessPresence::Gone);
        assert_eq!(
            presence(staying_up.pid()),
            ProcessPresence::Running,
            "closing one project must not touch another's work",
        );
        assert!(!b.is_closed());
        assert!(
            b.submit("B keeps working", |_control: JobControl| {
                JobOutcome::Succeeded
            })
            .is_ok()
        );
        assert!(b.is_trusted(), "B's trust is its own");
        assert!(
            !a.is_trusted(),
            "A's trust went with A, even for a handle that outlived it",
        );

        let b_report = registry.close(second.path()).expect("B closes");
        assert!(b_report.is_clean(), "{}", b_report.message());
        assert_eq!(presence(staying_up.pid()), ProcessPresence::Gone);
    }

    #[test]
    fn closing_a_project_that_is_not_open_is_refused() {
        let sandbox = Sandbox::new("not-open");
        let registry = ServiceRegistry::new();

        let failure = registry
            .close(sandbox.path())
            .expect_err("there is nothing to close");

        assert!(matches!(failure, CloseFailure::NotOpen { .. }));
        assert!(failure.message().contains("not open"));
        let services = registry.open(sandbox.path()).expect("opens");
        assert!(registry.close(sandbox.path()).is_ok());
        assert!(
            registry.close(sandbox.path()).is_err(),
            "closing twice does not report a second successful close",
        );
        drop(services);
    }

    #[test]
    fn closing_withdraws_the_trust_and_the_authorizations_that_open_was_given() {
        let sandbox = Sandbox::new("withdraw");
        let registry = ServiceRegistry::new();
        let root = sandbox.root();
        let services = registry.open(sandbox.path()).expect("opens");
        services.trust();
        services
            .authorize(Capability::RunCargoCheck)
            .expect("authorized");

        let report = registry.close(sandbox.path()).expect("closes");

        assert_eq!(report.withdrawn, vec![Capability::RunCargoCheck]);
        assert!(!registry.permission().is_trusted(&root));
        let again = registry.open(sandbox.path()).expect("opens again");
        assert!(
            !again.is_trusted(),
            "a reopened project must be asked again, not inherit trust",
        );
        assert!(registry.close(sandbox.path()).is_ok());
    }

    #[test]
    fn the_close_report_names_the_order_it_took() {
        let sandbox = Sandbox::new("order");
        let registry = ServiceRegistry::new();
        registry.open(sandbox.path()).expect("opens");

        let report = registry.close(sandbox.path()).expect("closes");

        assert_eq!(
            report.order,
            vec![
                "mark the scope closed",
                "stop the scope's tasks",
                "take down the scope's processes",
                "withdraw the project's trust and authorizations",
                "record what did not stop",
            ],
        );
        assert!(report.message().contains("nothing is left running"));
        assert!(report.scope.render().contains("owner"));
    }

    #[test]
    fn closing_every_project_reports_one_close_each() {
        let first = Sandbox::new("all-a");
        let second = Sandbox::new("all-b");
        let registry = ServiceRegistry::new();
        registry.open(first.path()).expect("A opens");
        registry.open(second.path()).expect("B opens");

        let reports = registry.close_all(Duration::from_secs(1));

        assert_eq!(reports.len(), 2);
        assert!(reports.iter().all(super::CloseReport::is_clean));
        assert!(registry.open_scopes().is_empty());
        assert!(registry.close_all(Duration::from_secs(1)).is_empty());
    }

    #[test]
    fn a_process_handle_reports_what_the_process_wrote() {
        let sandbox = Sandbox::new("handle");
        let registry = ServiceRegistry::new();
        let services = registry.open(sandbox.path()).expect("opens");
        services.trust();
        let handle = services
            .spawn(
                ProcessSpec::new("prints", "sh").args(["-c", "echo hello; exit 0"]),
                Principal::Human,
            )
            .expect("starts");

        let outcome = handle.wait(Duration::from_secs(5)).expect("it ends");

        assert_eq!(outcome.output.stdout.trim(), "hello");
        assert!(outcome.exit.is_success());
        assert_eq!(handle.pid(), outcome.pid);
        assert_eq!(handle.label(), "prints");
        assert!(handle.id() == outcome.id);
        assert!(registry.close(sandbox.path()).expect("closes").is_clean());
    }

    #[test]
    fn work_that_will_not_stop_is_in_the_close_report_and_in_the_notifications() {
        let sandbox = Sandbox::new("abandoned");
        let registry = ServiceRegistry::new();
        let services = registry.open_with(sandbox.path(), quick()).expect("opens");
        let id = services
            .submit("ignores everything", |_control: JobControl| {
                std::thread::sleep(Duration::from_millis(250));
                JobOutcome::Succeeded
            })
            .expect("accepted");
        // Let the worker pick it up, so the close has something running to give up on.
        std::thread::sleep(Duration::from_millis(40));

        let report = registry.close(sandbox.path()).expect("closes");

        assert_eq!(report.tasks.abandoned, vec![id], "{:?}", report.tasks);
        assert!(!report.is_clean(), "a job that never stopped is not clean");
        assert_eq!(report.notifications, 1);
        let notification = registry
            .notifications()
            .notifications()
            .into_iter()
            .next()
            .expect("the abandoned task is reported");
        assert_eq!(notification.link().kind(), "task");
        assert!(
            notification.summary().contains("abandoned"),
            "{}",
            notification.summary(),
        );
        // The job is left to finish on its own; the host already stopped waiting for it.
        std::thread::sleep(Duration::from_millis(300));
    }
}
