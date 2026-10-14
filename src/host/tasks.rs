// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The background executor: work that must not block the UI, and an honest record of what it did.
//! Contract **HOST**.
//!
//! Roadmap T01.03 asks for a background executor, task ownership, a queue, progress and a timeout
//! and cancellation protocol. The acceptance line is short and easy to fake: **"the cancelling
//! state is real, and a long task does not block the UI"**. Everything below follows from refusing
//! to fake it:
//!
//! * **Submitting returns; running does not happen on the caller's thread.** [`Executor::submit`]
//!   hands the job to a worker and returns its id. The caller — the UI thread — polls records or
//!   waits with its own deadline, and never blocks on the work itself.
//! * **`Cancelling` is a state, not a promise.** Requesting cancellation moves a running task to
//!   [`TaskState::Cancelling`] and sets a token; it becomes [`TaskOutcome::Cancelled`] only when the
//!   job actually returns. Until then the record says the truth: the request is in flight.
//! * **A job that will not stop is reported, not hidden.** The host cannot kill a thread, so a
//!   deadline can only *ask*. When the grace period passes and the job has not returned, the task is
//!   recorded as [`TaskOutcome::Abandoned`] with `job_still_running: true`, the host stops waiting,
//!   and any result the job produces later is discarded and logged. Calling that "cancelled" would
//!   be the placeholder success principle #22 forbids.
//! * **A job that reports cancellation nobody asked for is a defect**, and is recorded as a failure
//!   rather than as a cancellation. The reverse race — a cancel request that arrives while the job
//!   is finishing successfully — is recorded as a success with `cancel_requested` still set, because
//!   the work *did* happen and saying otherwise would misreport what is on disk.
//! * **The queue is bounded and says no.** Admission is refused with [`SubmitError::QueueFull`]
//!   rather than growing without limit (principle #23: cost follows the work, not the backlog).
//!
//! **What this module deliberately does not own.** Enforcing permissions is T01.05's contract; this
//! module *records* the capability and principal a submission names and documents that they are not
//! enforced yet, so nothing here reads as a permission check. Subprocess trees are T01.04, and the
//! service registry that gives executors a project scope is T01.07 — an [`OwnerId`] is the scope
//! marker those tasks attach to, and it deliberately carries no project data of its own.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::log::{Level, emit};
use crate::contracts::permission::{Capability, Principal};

/// Who owns a task: one session or workspace's worth of work.
///
/// Roadmap 4.1 puts "ownership" in the Task object because two projects must not be able to cancel
/// each other's work, and because closing a project has to be able to take its work down with it
/// (T01.07). The id is opaque on purpose: what a session *is* belongs to T02.01's project model, and
/// this module must not grow a second opinion about it.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct OwnerId(u64);

impl OwnerId {
    /// A fresh owner id, unique in this process.
    pub fn new() -> Self {
        Self(NEXT_OWNER.fetch_add(1, Ordering::Relaxed))
    }

    /// The number, for a log line or a test assertion.
    pub const fn value(self) -> u64 {
        self.0
    }
}

impl Default for OwnerId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for OwnerId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "owner#{}", self.0)
    }
}

/// A submitted task's identity.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct TaskId(u64);

impl TaskId {
    /// The number, for a log line or a test assertion.
    pub const fn value(self) -> u64 {
        self.0
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "task#{}", self.0)
    }
}

/// Where a task is in its lifecycle. Roadmap 4.2's build sequence is the model:
/// queued → running → cancelling/ended → succeeded/failed/cancelled.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TaskState {
    /// Accepted, not started.
    Queued,
    /// A worker is running the job.
    Running,
    /// Cancellation has been requested and the job has not returned yet.
    Cancelling,
    /// The task has an outcome and will not change again, except for the late-return flag.
    Finished,
}

impl TaskState {
    /// The name used in reports and tests.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Cancelling => "cancelling",
            Self::Finished => "finished",
        }
    }

    /// Whether this is a state the task can still leave.
    pub const fn is_final(self) -> bool {
        matches!(self, Self::Finished)
    }
}

/// How far along a task is.
///
/// Monotone by construction: [`ProgressSink`] keeps the largest `completed` it has been given, so a
/// job that reports progress out of order cannot make a finished-looking task look unfinished again
/// (principle #22: a stale number must not masquerade as the current one).
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Progress {
    completed: u64,
    total: Option<u64>,
    note: Option<String>,
}

impl Progress {
    /// Nothing done yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// How much is done.
    pub const fn completed(&self) -> u64 {
        self.completed
    }

    /// How much there is in total, when that is known.
    pub const fn total(&self) -> Option<u64> {
        self.total
    }

    /// The job's own note about the current step.
    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    /// The fraction done, when a total is known and not zero.
    pub fn fraction(&self) -> Option<f64> {
        match self.total {
            Some(total) if total > 0 => Some(self.completed as f64 / total as f64),
            _ => None,
        }
    }
}

/// Why a cancellation happened. Recorded because "someone pressed cancel" and "the deadline passed"
/// are different facts about the same file, and a reader has to be able to tell them apart.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CancellationReason {
    /// A person, a command or a shutdown asked for it.
    Requested,
    /// The task's own deadline passed, so the executor asked the job to stop.
    DeadlineExceeded,
}

impl CancellationReason {
    /// The name used in reports and tests.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Requested => "requested",
            Self::DeadlineExceeded => "deadline-exceeded",
        }
    }
}

/// How a task ended.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum TaskOutcome {
    /// The job did its work and said so.
    Succeeded {
        /// How long the job ran.
        after: Duration,
    },
    /// The job reported a failure, with its own message.
    Failed {
        /// What the job said went wrong.
        message: String,
        /// How long the job ran.
        after: Duration,
    },
    /// The job acknowledged a cancellation and stopped.
    Cancelled {
        /// How long the job ran.
        after: Duration,
        /// Who asked, or what did.
        reason: CancellationReason,
    },
    /// The deadline passed, the job did not stop, and the host stopped waiting for it.
    ///
    /// The job may still be running: `job_still_running` is true until it returns, and its result is
    /// discarded when it does. `deadline` is the limit that was exceeded, not the total runtime.
    Abandoned {
        /// The deadline the job failed to meet.
        deadline: Duration,
        /// Whether the job is still running now.
        job_still_running: bool,
    },
    /// The task never ran. The reason names what refused it.
    Refused {
        /// Why the task did not run.
        reason: String,
    },
}

impl TaskOutcome {
    /// The name used in reports and tests.
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Succeeded { .. } => "succeeded",
            Self::Failed { .. } => "failed",
            Self::Cancelled { .. } => "cancelled",
            Self::Abandoned { .. } => "abandoned",
            Self::Refused { .. } => "refused",
        }
    }

    /// A sentence for a log line, a problem panel or a terminal.
    pub fn message(&self) -> String {
        match self {
            Self::Succeeded { after } => format!("succeeded after {after:?}"),
            Self::Failed { message, after } => format!("failed after {after:?}: {message}"),
            Self::Cancelled { after, reason } => {
                format!("cancelled after {after:?} ({} )", reason.name())
            }
            Self::Abandoned {
                deadline,
                job_still_running,
            } => format!(
                "abandoned: the {deadline:?} deadline passed and the job {}",
                if *job_still_running {
                    "is still running; its result will be discarded"
                } else {
                    "has since returned; its result was discarded"
                },
            ),
            Self::Refused { reason } => format!("never ran: {reason}"),
        }
    }
}

/// Everything the host knows about one task.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TaskRecord {
    /// The task.
    pub id: TaskId,
    /// Who owns it.
    pub owner: OwnerId,
    /// The input summary roadmap 4.1 asks for: what the task is, in the words the submitter used.
    pub label: String,
    /// On whose behalf it runs.
    pub principal: Principal,
    /// The capability the submitter says this work needs, when it named one.
    ///
    /// **Recorded, not enforced.** Enforcement is T01.05's contract; this field is where its decision
    /// will be attached, and saying so here is what keeps "the executor does not check permissions
    /// yet" from being discovered later by reading the source.
    pub capability: Option<Capability>,
    /// Where it is in its lifecycle.
    pub state: TaskState,
    /// How far along it is.
    pub progress: Progress,
    /// How it ended, once it has.
    pub outcome: Option<TaskOutcome>,
    /// When it was accepted.
    pub submitted_at: Instant,
    /// When a worker started it.
    pub started_at: Option<Instant>,
    /// When it finished, or when the host stopped waiting for it.
    pub finished_at: Option<Instant>,
    /// Whether a cancellation has been asked for, whether or not the job obeyed.
    pub cancel_requested: bool,
    /// The deadline the submitter named, if any.
    pub deadline: Option<Duration>,
    /// Whether that deadline was reached and the executor asked the job to stop.
    pub deadline_reached: bool,
}

impl TaskRecord {
    /// Whether the task has an outcome and will not change again, apart from the late-return flag.
    pub fn is_final(&self) -> bool {
        self.state.is_final()
    }

    /// How long the task has been alive at `now`.
    pub fn elapsed(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.submitted_at)
    }

    /// One line: what it is, where it is, and how it ended.
    pub fn summary(&self) -> String {
        let mut line = format!(
            "{} [{}] {} ({})",
            self.id,
            self.owner,
            self.label,
            self.state.name(),
        );
        if let Some(outcome) = &self.outcome {
            line.push_str(" — ");
            line.push_str(&outcome.message());
        }
        if self.cancel_requested && !matches!(self.outcome, Some(TaskOutcome::Cancelled { .. })) {
            line.push_str(" (a cancellation was requested)");
        }
        line
    }
}

/// What a submitter says about the work.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SubmitRequest {
    /// Who owns the task.
    pub owner: OwnerId,
    /// The input summary: what this task is.
    pub label: String,
    /// On whose behalf it runs.
    pub principal: Principal,
    /// The capability the work needs, when the submitter knows it.
    pub capability: Option<Capability>,
    /// How long the job may take before the executor asks it to stop.
    pub timeout: Option<Duration>,
}

impl SubmitRequest {
    /// A request from a person, with no deadline and no named capability.
    pub fn new(owner: OwnerId, label: impl Into<String>) -> Self {
        Self {
            owner,
            label: label.into(),
            principal: Principal::Human,
            capability: None,
            timeout: None,
        }
    }

    /// Sets the deadline.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Names the capability the work needs, and who is asking.
    pub fn with_capability(mut self, principal: Principal, capability: Capability) -> Self {
        self.principal = principal;
        self.capability = Some(capability);
        self
    }
}

/// Why a submission was refused.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SubmitError {
    /// The queue cannot hold more work. Named with the capacity so a caller can say what to do.
    QueueFull {
        /// How many tasks the queue can hold.
        capacity: usize,
    },
    /// The executor has been shut down and accepts no more work.
    Stopped,
}

impl SubmitError {
    /// A sentence naming what was found.
    pub fn message(&self) -> String {
        match self {
            Self::QueueFull { capacity } => format!(
                "the executor's queue is full ({capacity} waiting); wait for work to finish or \
                 cancel some",
            ),
            Self::Stopped => "the executor has been shut down and accepts no more work".to_owned(),
        }
    }
}

impl fmt::Display for SubmitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message())
    }
}

impl std::error::Error for SubmitError {}

/// Why an executor could not be built.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ExecutorError {
    /// A configuration that could never do the work it was asked for.
    InvalidConfig {
        /// What is wrong with it.
        reason: String,
    },
    /// A worker or the deadline monitor could not be started.
    SpawnFailed {
        /// The operating system's own reason.
        reason: String,
    },
}

impl ExecutorError {
    /// A sentence naming what was found.
    pub fn message(&self) -> String {
        match self {
            Self::InvalidConfig { reason } => {
                format!("the executor configuration is unusable: {reason}")
            }
            Self::SpawnFailed { reason } => {
                format!("an executor thread could not be started: {reason}")
            }
        }
    }
}

impl fmt::Display for ExecutorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message())
    }
}

impl std::error::Error for ExecutorError {}

/// What a job reports when it returns.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum JobOutcome {
    /// The work is done.
    Succeeded,
    /// The work failed, with a message worth showing to a person.
    Failed {
        /// What went wrong.
        message: String,
    },
    /// The job noticed the cancellation request and stopped.
    Cancelled,
}

impl JobOutcome {
    /// A failing outcome with a message.
    pub fn failed(message: impl Into<String>) -> Self {
        Self::Failed {
            message: message.into(),
        }
    }
}

/// Work the executor can run.
///
/// `Send + 'static` because it runs on another thread and outlives the call that submitted it. The
/// job can be slow; it must check its token if it wants to be cancellable, which is why
/// [`JobControl`] is handed to it rather than being a global.
///
/// The control is passed **by value** — it is two reference-counted handles and an `Option`, so
/// passing it costs a clone — so that a plain closure works as a job without a lifetime annotation:
/// `executor.submit(request, |control| { … })`. Taking it by reference would force every call site
/// to write `|control: JobControl|` for the closure to be higher-ranked.
pub trait Job: Send + 'static {
    /// Does the work and reports what happened.
    fn run(&mut self, control: JobControl) -> JobOutcome;
}

impl<F> Job for F
where
    F: FnMut(JobControl) -> JobOutcome + Send + 'static,
{
    fn run(&mut self, control: JobControl) -> JobOutcome {
        self(control)
    }
}

/// The cancellation flag a job polls.
///
/// A flag rather than a channel because a job may check it in a tight loop; and cloneable because a
/// job that starts its own sub-work (T01.04's subprocess tree) has to be able to pass the same
/// request down instead of inventing a second cancellation path.
#[derive(Clone, Debug, Default)]
pub struct CancelToken {
    flag: Arc<AtomicBool>,
}

impl CancelToken {
    fn new() -> Self {
        Self::default()
    }

    /// Asks the job to stop. Idempotent.
    pub fn request(&self) {
        self.flag.store(true, Ordering::SeqCst);
    }

    /// Whether a stop has been asked for.
    pub fn is_requested(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    /// Sleeps up to `limit`, returning early when a stop is asked for.
    ///
    /// Returns whether cancellation was requested. Sliced rather than one long sleep so a job that
    /// waits has a bounded reaction time without polling in a spin loop.
    pub fn wait(&self, limit: Duration) -> bool {
        let slice = Duration::from_millis(5);
        let deadline = Instant::now() + limit;
        while Instant::now() < deadline {
            if self.is_requested() {
                return true;
            }
            std::thread::sleep(slice.min(deadline.saturating_duration_since(Instant::now())));
        }
        self.is_requested()
    }
}

/// The progress channel a job reports through.
#[derive(Clone)]
pub struct ProgressSink {
    shared: Arc<Shared>,
    id: TaskId,
}

impl fmt::Debug for ProgressSink {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProgressSink")
            .field("task", &self.id)
            .finish_non_exhaustive()
    }
}

impl ProgressSink {
    /// Reports how much is done, and optionally how much there is.
    ///
    /// A value below what was already reported is ignored and logged: progress that goes backwards
    /// would let a nearly finished task look nearly unstarted, and the record is what the UI draws.
    pub fn set(&self, completed: u64, total: Option<u64>) {
        let mut state = lock(&self.shared.state);
        let Some(record) = state.records.get_mut(&self.id) else {
            return;
        };
        if completed < record.progress.completed {
            let dropped = completed;
            let kept = record.progress.completed;
            drop(state);
            emit(
                Level::Warn,
                "host::tasks",
                &format!(
                    "{} reported progress {dropped} below the recorded {kept}; the higher value is \
                     kept",
                    self.id,
                ),
            );
            return;
        }
        record.progress.completed = completed;
        if total.is_some() {
            record.progress.total = total;
        }
    }

    /// Records what the job is doing right now.
    pub fn note(&self, note: impl Into<String>) {
        let mut state = lock(&self.shared.state);
        if let Some(record) = state.records.get_mut(&self.id) {
            record.progress.note = Some(note.into());
        }
    }
}

/// What a running job may ask about its own execution.
#[derive(Clone, Debug)]
pub struct JobControl {
    token: CancelToken,
    progress: ProgressSink,
    deadline: Option<Duration>,
}

impl JobControl {
    /// The cancellation flag. Check it, or hand it to sub-work.
    pub fn token(&self) -> &CancelToken {
        &self.token
    }

    /// Whether a stop has already been asked for.
    pub fn is_cancelled(&self) -> bool {
        self.token.is_requested()
    }

    /// The deadline the submitter named, so a job can budget its own work.
    pub fn deadline(&self) -> Option<Duration> {
        self.deadline
    }

    /// The progress channel.
    pub fn progress(&self) -> &ProgressSink {
        &self.progress
    }
}

/// How the executor is built.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ExecutorConfig {
    /// How many jobs may run at once.
    pub workers: usize,
    /// How many tasks may wait for a worker.
    pub queue_capacity: usize,
    /// How often the deadline monitor looks at running tasks.
    pub monitor_interval: Duration,
    /// How long a job may take to stop after its deadline before its task is called abandoned.
    pub timeout_grace: Duration,
    /// How long `shutdown` waits for workers before it reports them as abandoned.
    pub shutdown_grace: Duration,
}

impl Default for ExecutorConfig {
    fn default() -> Self {
        Self {
            workers: 2,
            queue_capacity: 64,
            monitor_interval: Duration::from_millis(25),
            timeout_grace: Duration::from_millis(500),
            shutdown_grace: Duration::from_secs(2),
        }
    }
}

impl ExecutorConfig {
    /// A configuration for tests and demos: one worker and short intervals.
    pub fn responsive() -> Self {
        Self {
            workers: 1,
            queue_capacity: 8,
            monitor_interval: Duration::from_millis(2),
            timeout_grace: Duration::from_millis(20),
            shutdown_grace: Duration::from_millis(200),
        }
    }
}

/// What a cancel request did.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum CancelResult {
    /// The request is in flight: a queued task is finishing now, a running task is `cancelling`.
    Requested,
    /// The task had already ended, with the outcome it ended with.
    AlreadyFinished {
        /// How it ended.
        outcome: Option<TaskOutcome>,
    },
    /// No task with that id.
    Unknown,
}

impl CancelResult {
    /// Whether the request was taken.
    pub fn is_requested(&self) -> bool {
        matches!(self, Self::Requested)
    }
}

/// What a shutdown managed to do.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ShutdownReport {
    /// How many tasks were still alive and had a cancellation requested.
    pub cancelled: usize,
    /// How many tasks had already finished before the shutdown.
    pub finished: usize,
    /// The tasks whose jobs had not returned when the grace period passed.
    pub abandoned: Vec<TaskId>,
    /// How many worker threads the executor had.
    pub workers: usize,
}

impl ShutdownReport {
    /// Whether every job stopped within the grace period.
    pub fn is_clean(&self) -> bool {
        self.abandoned.is_empty()
    }

    /// A sentence for a log line.
    pub fn message(&self) -> String {
        if self.is_clean() {
            return format!(
                "{} workers stopped; {} tasks had finished, {} were cancelled",
                self.workers, self.finished, self.cancelled,
            );
        }
        format!(
            "{} workers stopped; {} tasks had finished, {} were cancelled, and {} jobs did not stop \
             within the grace period: {:?}",
            self.workers,
            self.finished,
            self.cancelled,
            self.abandoned.len(),
            self.abandoned,
        )
    }
}

impl fmt::Display for ShutdownReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message())
    }
}

/// A background executor: a fixed set of worker threads, a bounded queue, and a task record per
/// submission.
///
/// Owned by the session that created it (T01.07's service registry will place it there). Dropping an
/// `Executor` **does not** stop its workers: [`Executor::shutdown`] does, and says what it could not
/// stop. That is deliberate — a silent drop would make "the project closed and its work ended" a
/// guess, which is exactly what T01.07 has to be able to check.
pub struct Executor {
    shared: Arc<Shared>,
    workers: Mutex<Vec<JoinHandle<()>>>,
    monitor: Mutex<Option<JoinHandle<()>>>,
}

impl Executor {
    /// Starts the workers and the deadline monitor.
    pub fn new(config: ExecutorConfig) -> Result<Self, ExecutorError> {
        if config.workers == 0 {
            return Err(ExecutorError::InvalidConfig {
                reason: "no workers, so nothing submitted would ever run".to_owned(),
            });
        }
        if config.queue_capacity == 0 {
            return Err(ExecutorError::InvalidConfig {
                reason: "a queue that holds nothing would refuse every submission".to_owned(),
            });
        }

        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            wake: Condvar::new(),
            config: config.clone(),
        });

        let mut workers = Vec::with_capacity(config.workers);
        for index in 0..config.workers {
            let worker = shared.clone();
            let handle = thread::Builder::new()
                .name(format!("rustage-worker-{index}"))
                .spawn(move || worker_loop(worker))
                .map_err(|error| ExecutorError::SpawnFailed {
                    reason: error.to_string(),
                })?;
            workers.push(handle);
        }

        let monitor = {
            let monitor_shared = shared.clone();
            thread::Builder::new()
                .name("rustage-deadlines".to_owned())
                .spawn(move || monitor_loop(monitor_shared))
                .map_err(|error| ExecutorError::SpawnFailed {
                    reason: error.to_string(),
                })?
        };

        Ok(Self {
            shared,
            workers: Mutex::new(workers),
            monitor: Mutex::new(Some(monitor)),
        })
    }

    /// Accepts work and returns its id.
    ///
    /// The job runs on a worker; this call returns as soon as the task is queued, which is what
    /// keeps a long build off the caller's thread.
    pub fn submit<J: Job>(&self, request: SubmitRequest, job: J) -> Result<TaskId, SubmitError> {
        let mut state = lock(&self.shared.state);
        if state.stopping {
            return Err(SubmitError::Stopped);
        }
        if state.queue.len() >= self.shared.config.queue_capacity {
            return Err(SubmitError::QueueFull {
                capacity: self.shared.config.queue_capacity,
            });
        }

        let id = TaskId(NEXT_TASK.fetch_add(1, Ordering::Relaxed));
        state.records.insert(
            id,
            TaskRecord {
                id,
                owner: request.owner,
                label: request.label,
                principal: request.principal,
                capability: request.capability,
                state: TaskState::Queued,
                progress: Progress::new(),
                outcome: None,
                submitted_at: Instant::now(),
                started_at: None,
                finished_at: None,
                cancel_requested: false,
                deadline: request.timeout,
                deadline_reached: false,
            },
        );
        state.jobs.insert(id, Box::new(job));
        state.queue.push_back(id);
        self.shared.wake.notify_one();
        Ok(id)
    }

    /// Asks a task to stop, and says what the request did.
    pub fn cancel(&self, id: TaskId) -> CancelResult {
        let mut state = lock(&self.shared.state);
        let Some(record) = state.records.get_mut(&id) else {
            return CancelResult::Unknown;
        };
        if record.state.is_final() {
            return CancelResult::AlreadyFinished {
                outcome: record.outcome.clone(),
            };
        }

        record.cancel_requested = true;
        if record.state == TaskState::Queued {
            // Nothing has started, so there is nothing to wait for: the task is cancelled now and
            // its job is dropped without ever running.
            let now = Instant::now();
            record.outcome = Some(TaskOutcome::Cancelled {
                after: Duration::ZERO,
                reason: CancellationReason::Requested,
            });
            record.state = TaskState::Finished;
            record.finished_at = Some(now);
            state.jobs.remove(&id);
            return CancelResult::Requested;
        }

        // A running job is asked, not stopped. The record says `cancelling` until it returns, so a
        // caller can tell "the request is in flight" from "the work stopped".
        record.state = TaskState::Cancelling;
        if let Some(token) = state.tokens.get(&id) {
            token.request();
        }
        CancelResult::Requested
    }

    /// Asks every task of one owner to stop, and returns the tasks it asked.
    ///
    /// This is what "closing one project does not touch another project's work" rests on (T01.07).
    pub fn cancel_owner(&self, owner: OwnerId) -> Vec<TaskId> {
        let ids: Vec<TaskId> = {
            let state = lock(&self.shared.state);
            state
                .records
                .values()
                .filter(|record| record.owner == owner && !record.state.is_final())
                .map(|record| record.id)
                .collect()
        };
        let mut asked = Vec::new();
        for id in ids {
            if self.cancel(id).is_requested() {
                asked.push(id);
            }
        }
        asked
    }

    /// One task's record, as it is now.
    pub fn record(&self, id: TaskId) -> Option<TaskRecord> {
        lock(&self.shared.state).records.get(&id).cloned()
    }

    /// Every record, in submission order.
    pub fn records(&self) -> Vec<TaskRecord> {
        lock(&self.shared.state).records.values().cloned().collect()
    }

    /// Every record of one owner, in submission order.
    pub fn records_for(&self, owner: OwnerId) -> Vec<TaskRecord> {
        lock(&self.shared.state)
            .records
            .values()
            .filter(|record| record.owner == owner)
            .cloned()
            .collect()
    }

    /// Waits up to `limit` for a task to finish, returning the newest record either way.
    ///
    /// Returns `None` only when there is no such task. A task that is still running when the limit
    /// passes is returned as it is, not as an error and not as a finished task: the caller asked
    /// "what do we know now", and that is the honest answer.
    pub fn wait(&self, id: TaskId, limit: Duration) -> Option<TaskRecord> {
        let deadline = Instant::now() + limit;
        loop {
            let record = self.record(id)?;
            if record.is_final() || Instant::now() >= deadline {
                return Some(record);
            }
            thread::sleep(Duration::from_millis(2));
        }
    }

    /// Whether the executor has been shut down.
    pub fn is_stopped(&self) -> bool {
        lock(&self.shared.state).stopping
    }

    /// Cancels everything, waits for the workers, and reports what did not stop.
    ///
    /// The host cannot kill a thread, so a job that ignores its token is *reported*: its task is
    /// finalised, its worker handle is dropped with a warning, and the id appears in
    /// [`ShutdownReport::abandoned`]. A clean report is therefore evidence, and an unclean one is a
    /// finding rather than a hang.
    pub fn shutdown(&self) -> ShutdownReport {
        let mut cancelled = 0;
        let mut finished = 0;
        {
            let mut state = lock(&self.shared.state);
            state.stopping = true;
            let ids: Vec<TaskId> = state.records.keys().copied().collect();
            for id in ids {
                let Some(record) = state.records.get_mut(&id) else {
                    continue;
                };
                if record.state.is_final() {
                    finished += 1;
                    continue;
                }
                record.cancel_requested = true;
                if record.state == TaskState::Queued {
                    record.outcome = Some(TaskOutcome::Cancelled {
                        after: Duration::ZERO,
                        reason: CancellationReason::Requested,
                    });
                    record.state = TaskState::Finished;
                    record.finished_at = Some(Instant::now());
                    state.jobs.remove(&id);
                    cancelled += 1;
                    continue;
                }
                record.state = TaskState::Cancelling;
                if let Some(token) = state.tokens.get(&id) {
                    token.request();
                }
                cancelled += 1;
            }
            self.shared.wake.notify_all();
        }

        let deadline = Instant::now() + self.shared.config.shutdown_grace;
        let mut abandoned = Vec::new();
        {
            let mut workers = lock(&self.workers);
            while Instant::now() < deadline {
                if workers.iter().all(JoinHandle::is_finished) {
                    break;
                }
                thread::sleep(Duration::from_millis(5));
            }
            for handle in workers.drain(..) {
                if handle.is_finished() {
                    let _ = handle.join();
                } else {
                    emit(
                        Level::Warn,
                        "host::tasks",
                        "a worker did not stop within the shutdown grace period; the thread is left \
                         running and its task is reported as abandoned",
                    );
                }
            }
        }

        if let Some(monitor) = lock(&self.monitor).take() {
            let monitor_deadline = Instant::now() + Duration::from_millis(200);
            while !monitor.is_finished() && Instant::now() < monitor_deadline {
                thread::sleep(Duration::from_millis(2));
            }
            if monitor.is_finished() {
                let _ = monitor.join();
            }
        }

        {
            let mut state = lock(&self.shared.state);
            let now = Instant::now();
            for record in state.records.values_mut() {
                if !record.state.is_final() {
                    let deadline = record.deadline.unwrap_or_default();
                    record.outcome = Some(TaskOutcome::Abandoned {
                        deadline,
                        job_still_running: true,
                    });
                    record.state = TaskState::Finished;
                    record.finished_at = Some(now);
                    abandoned.push(record.id);
                }
            }
        }

        ShutdownReport {
            cancelled,
            finished,
            abandoned,
            workers: self.shared.config.workers,
        }
    }
}

impl fmt::Debug for Executor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let records = lock(&self.shared.state).records.len();
        formatter
            .debug_struct("Executor")
            .field("workers", &self.shared.config.workers)
            .field("queue_capacity", &self.shared.config.queue_capacity)
            .field("tasks", &records)
            .finish_non_exhaustive()
    }
}

/// The shared state of one executor.
struct Shared {
    state: Mutex<State>,
    wake: Condvar,
    config: ExecutorConfig,
}

#[derive(Default)]
struct State {
    records: BTreeMap<TaskId, TaskRecord>,
    queue: VecDeque<TaskId>,
    /// Jobs that have not been handed to a worker yet.
    jobs: BTreeMap<TaskId, Box<dyn Job>>,
    /// The cancellation flag of every running task.
    tokens: BTreeMap<TaskId, CancelToken>,
    stopping: bool,
}

/// Locks without the `unwrap` that turns a poisoned mutex into a panic cascade.
///
/// A panic in a job must not take the executor down with it: the record of that task is what a user
/// needs to see, and the host keeps running. The panic itself is still reported by the worker, so
/// recovery here does not hide it.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// One worker: take a job, run it, record what it said.
fn worker_loop(shared: Arc<Shared>) {
    loop {
        // Wait for work rather than exiting on an empty queue: a worker that returned here would
        // make the executor accept a task and never run it.
        let Some((id, mut job, token)) = wait_for_work(&shared) else {
            return;
        };
        let deadline = {
            let state = lock(&shared.state);
            state.records.get(&id).and_then(|record| record.deadline)
        };
        let control = JobControl {
            token: token.clone(),
            progress: ProgressSink {
                shared: shared.clone(),
                id,
            },
            deadline,
        };

        let started = Instant::now();
        // A job that panics must not take its worker down with it: the worker is shared capacity,
        // and losing it would make every later task hang. The panic is turned into a recorded
        // failure, and the panic hook still prints it, so nothing is hidden.
        let outcome =
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job.run(control))) {
                Ok(outcome) => outcome,
                Err(payload) => JobOutcome::failed(format!(
                    "the job panicked: {}",
                    panic_message(payload.as_ref()),
                )),
            };
        let elapsed = started.elapsed();

        let mut state = lock(&shared.state);
        state.tokens.remove(&id);
        let Some(record) = state.records.get_mut(&id) else {
            continue;
        };
        if record.state.is_final() {
            // The deadline monitor already finalised this task. The job's late answer is discarded,
            // and saying so in the record is the difference between "abandoned" and a silent lie.
            if let Some(TaskOutcome::Abandoned {
                job_still_running, ..
            }) = record.outcome.as_mut()
            {
                *job_still_running = false;
            }
            let label = record.label.clone();
            drop(state);
            emit(
                Level::Warn,
                "host::tasks",
                &format!(
                    "{id} ({label}) returned after its task had been recorded as abandoned; the \
                     result was discarded",
                ),
            );
            shared.wake.notify_all();
            continue;
        }

        let final_outcome = match outcome {
            JobOutcome::Succeeded => {
                if record.cancel_requested {
                    // The work happened; a cancellation arrived too late. Reporting `cancelled`
                    // would misdescribe what is on disk, so the success is recorded and the request
                    // stays visible on the record.
                    emit(
                        Level::Warn,
                        "host::tasks",
                        &format!(
                            "{id} succeeded after a cancellation was requested; the request lost the race"
                        ),
                    );
                }
                TaskOutcome::Succeeded { after: elapsed }
            }
            JobOutcome::Failed { message } => TaskOutcome::Failed {
                message,
                after: elapsed,
            },
            JobOutcome::Cancelled => {
                if !record.cancel_requested {
                    // A job claiming a cancellation nobody asked for is a job defect, not a fact.
                    TaskOutcome::Failed {
                        message: "the job reported a cancellation that was never requested"
                            .to_owned(),
                        after: elapsed,
                    }
                } else {
                    TaskOutcome::Cancelled {
                        after: elapsed,
                        reason: if record.deadline_reached {
                            CancellationReason::DeadlineExceeded
                        } else {
                            CancellationReason::Requested
                        },
                    }
                }
            }
        };
        record.outcome = Some(final_outcome);
        record.state = TaskState::Finished;
        record.finished_at = Some(Instant::now());
        drop(state);
        shared.wake.notify_all();
    }
}

/// Waits for a job, returning `None` only when the executor is shutting down.
fn wait_for_work(shared: &Arc<Shared>) -> Option<(TaskId, Box<dyn Job>, CancelToken)> {
    let mut state = lock(&shared.state);
    loop {
        if state.stopping {
            return None;
        }
        if let Some(taken) = take_next(&mut state) {
            return Some(taken);
        }
        let (guard, _) = shared
            .wake
            .wait_timeout(state, WORKER_IDLE_POLL)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state = guard;
    }
}

/// Takes the next job that is ready to run, finalising any that were cancelled while queued.
fn take_next(state: &mut State) -> Option<(TaskId, Box<dyn Job>, CancelToken)> {
    while let Some(id) = state.queue.pop_front() {
        let Some(job) = state.jobs.remove(&id) else {
            continue;
        };
        let now = Instant::now();
        let Some(record) = state.records.get_mut(&id) else {
            continue;
        };
        if record.cancel_requested {
            // Cancelled before a worker picked it up: it never ran, and the record says so.
            record.outcome = Some(TaskOutcome::Cancelled {
                after: Duration::ZERO,
                reason: CancellationReason::Requested,
            });
            record.state = TaskState::Finished;
            record.finished_at = Some(now);
            drop(job);
            continue;
        }
        record.state = TaskState::Running;
        record.started_at = Some(now);
        let token = CancelToken::new();
        state.tokens.insert(id, token.clone());
        return Some((id, job, token));
    }
    None
}

/// The text of a caught panic, if it has one.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        return (*text).to_owned();
    }
    if let Some(text) = payload.downcast_ref::<String>() {
        return text.clone();
    }
    "no message".to_owned()
}

/// How long an idle worker sleeps before looking for work again.
///
/// A backstop, not the mechanism: a submission wakes a worker through the condvar, and this only
/// bounds how long a worker that missed a wake-up stays idle.
const WORKER_IDLE_POLL: Duration = Duration::from_millis(20);

/// The deadline monitor: turns a passed deadline into a cancellation request, then into an honest
/// record when the job does not obey.
fn monitor_loop(shared: Arc<Shared>) {
    loop {
        {
            let state = lock(&shared.state);
            if state.stopping {
                return;
            }
        }
        thread::sleep(shared.config.monitor_interval);

        let now = Instant::now();
        let mut reached = Vec::new();
        {
            let mut state = lock(&shared.state);
            if state.stopping {
                return;
            }
            for record in state.records.values_mut() {
                if record.state.is_final() || record.deadline_reached {
                    continue;
                }
                let Some(deadline) = record.deadline else {
                    continue;
                };
                let started = record.started_at.unwrap_or(record.submitted_at);
                if now.saturating_duration_since(started) < deadline {
                    continue;
                }
                record.deadline_reached = true;
                record.cancel_requested = true;
                // `queued` keeps its place in the queue's own logic; a running job is asked now.
                if record.state == TaskState::Running {
                    record.state = TaskState::Cancelling;
                }
                reached.push((record.id, deadline));
            }
            for (id, _) in &reached {
                if let Some(token) = state.tokens.get(id) {
                    token.request();
                }
            }
        }
        for (id, deadline) in reached {
            emit(
                Level::Warn,
                "host::tasks",
                &format!("{id} passed its {deadline:?} deadline; the job has been asked to stop"),
            );
        }

        let now = Instant::now();
        let mut abandoned = Vec::new();
        {
            let mut state = lock(&shared.state);
            for record in state.records.values_mut() {
                if record.state.is_final() || !record.deadline_reached {
                    continue;
                }
                let Some(deadline) = record.deadline else {
                    continue;
                };
                let started = record.started_at.unwrap_or(record.submitted_at);
                let overdue = now
                    .saturating_duration_since(started)
                    .saturating_sub(deadline);
                if overdue < shared.config.timeout_grace {
                    continue;
                }
                record.outcome = Some(TaskOutcome::Abandoned {
                    deadline,
                    job_still_running: true,
                });
                record.state = TaskState::Finished;
                record.finished_at = Some(now);
                abandoned.push(record.id);
            }
        }
        for id in abandoned {
            emit(
                Level::Warn,
                "host::tasks",
                &format!(
                    "{id} did not stop within the grace period; it is recorded as abandoned and its \
                     result will be discarded",
                ),
            );
        }
    }
}

static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);
static NEXT_TASK: AtomicU64 = AtomicU64::new(1);

#[cfg(test)]
mod tests {
    use super::{
        CancelResult, CancellationReason, Executor, ExecutorConfig, JobControl, JobOutcome,
        OwnerId, SubmitError, SubmitRequest, TaskOutcome, TaskState,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::{Duration, Instant};

    fn executor(config: ExecutorConfig) -> Executor {
        Executor::new(config).expect("the executor starts")
    }

    /// A job that waits until the test releases it, so a state can be observed mid-flight.
    struct Blocking {
        started: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
        observed: Arc<AtomicUsize>,
        on_release: fn(JobControl) -> JobOutcome,
    }

    impl super::Job for Blocking {
        fn run(&mut self, control: JobControl) -> JobOutcome {
            self.observed.fetch_add(1, Ordering::SeqCst);
            let _ = self.started.send(());
            let _ = self.release.recv_timeout(Duration::from_secs(10));
            (self.on_release)(control)
        }
    }

    #[test]
    fn submitting_returns_before_the_job_finishes() {
        let executor = executor(ExecutorConfig::responsive());
        let (started, started_rx) = mpsc::channel();
        let (release, release_rx) = mpsc::channel();

        let began = Instant::now();
        let id = executor
            .submit(
                SubmitRequest::new(OwnerId::new(), "a slow job"),
                Blocking {
                    started,
                    release: release_rx,
                    observed: Arc::new(AtomicUsize::new(0)),
                    on_release: |_| JobOutcome::Succeeded,
                },
            )
            .expect("the queue has room");
        let submit_took = began.elapsed();
        started_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the job started");

        // The point of the executor: the caller returns while the work is still going on.
        assert!(
            submit_took < Duration::from_millis(50),
            "submit blocked for {submit_took:?}",
        );
        assert!(!executor.record(id).expect("the record exists").is_final());

        release.send(()).expect("the job is released");
        let record = executor
            .wait(id, Duration::from_secs(5))
            .expect("the record exists");
        assert!(matches!(
            record.outcome,
            Some(TaskOutcome::Succeeded { .. })
        ));
    }

    #[test]
    fn a_running_task_is_cancelling_until_the_job_actually_stops() {
        let executor = executor(ExecutorConfig::responsive());
        let owner = OwnerId::new();
        let (started, started_rx) = mpsc::channel();
        let (release, release_rx) = mpsc::channel();
        let id = executor
            .submit(
                SubmitRequest::new(owner, "ignores the request for a moment"),
                Blocking {
                    started,
                    release: release_rx,
                    observed: Arc::new(AtomicUsize::new(0)),
                    // A job that finished its work and only then looked at the token.
                    on_release: |_| JobOutcome::Succeeded,
                },
            )
            .expect("the queue has room");
        started_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the job started");

        assert_eq!(executor.cancel(id), CancelResult::Requested);

        // The acceptance line: `cancelling` is a real state, not a synonym for `cancelled`.
        let record = executor.record(id).expect("the record exists");
        assert_eq!(record.state, TaskState::Cancelling);
        assert!(record.cancel_requested);
        assert!(record.outcome.is_none());

        release.send(()).expect("the job is released");
        let final_record = executor
            .wait(id, Duration::from_secs(5))
            .expect("the record exists");
        // The work did happen, so the success is recorded — and the request stays visible.
        assert!(
            matches!(final_record.outcome, Some(TaskOutcome::Succeeded { .. })),
            "{:?}",
            final_record.outcome,
        );
        assert!(
            final_record.cancel_requested,
            "the lost race must stay visible"
        );
    }

    #[test]
    fn a_job_that_notices_the_request_is_cancelled() {
        let executor = executor(ExecutorConfig::responsive());
        let owner = OwnerId::new();
        let (started, started_rx) = mpsc::channel();
        let id = executor
            .submit(
                SubmitRequest::new(owner, "waits for the request"),
                move |control: JobControl| {
                    let _ = started.send(());
                    if control.token().wait(Duration::from_secs(10)) {
                        JobOutcome::Cancelled
                    } else {
                        JobOutcome::Succeeded
                    }
                },
            )
            .expect("the queue has room");
        started_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the job started");

        assert_eq!(executor.cancel(id), CancelResult::Requested);

        let record = executor
            .wait(id, Duration::from_secs(5))
            .expect("the record exists");
        match record.outcome {
            Some(TaskOutcome::Cancelled { reason, .. }) => {
                assert_eq!(reason, CancellationReason::Requested);
            }
            other => panic!("expected Cancelled, got {other:?}"),
        }
    }

    #[test]
    fn an_invalid_configuration_is_refused_rather_than_silently_doing_nothing() {
        assert!(
            Executor::new(ExecutorConfig {
                workers: 0,
                ..ExecutorConfig::responsive()
            })
            .is_err()
        );
        assert!(
            Executor::new(ExecutorConfig {
                queue_capacity: 0,
                ..ExecutorConfig::responsive()
            })
            .is_err()
        );
        let Err(error) = Executor::new(ExecutorConfig {
            workers: 0,
            ..ExecutorConfig::responsive()
        }) else {
            panic!("zero workers must be refused");
        };
        assert!(error.message().contains("ever run"));
    }

    #[test]
    fn a_queued_task_that_is_cancelled_never_runs() {
        let executor = executor(ExecutorConfig::responsive());
        let owner = OwnerId::new();
        let ran = Arc::new(AtomicUsize::new(0));
        let (started, started_rx) = mpsc::channel();
        let (release, release_rx) = mpsc::channel();

        let first = executor
            .submit(
                SubmitRequest::new(owner, "holds the only worker"),
                Blocking {
                    started,
                    release: release_rx,
                    observed: Arc::new(AtomicUsize::new(0)),
                    on_release: |_| JobOutcome::Succeeded,
                },
            )
            .expect("the queue has room");
        started_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the first job started");

        let second = {
            let ran = ran.clone();
            executor
                .submit(
                    SubmitRequest::new(owner, "queued then cancelled"),
                    move |_| {
                        ran.fetch_add(1, Ordering::SeqCst);
                        JobOutcome::Succeeded
                    },
                )
                .expect("the queue has room")
        };

        assert_eq!(executor.cancel(second), CancelResult::Requested);
        let record = executor.record(second).expect("the record exists");
        assert!(record.is_final(), "a queued task is cancelled at once");
        assert_eq!(record.state, TaskState::Finished);

        release.send(()).expect("the first job is released");
        let _ = executor.wait(first, Duration::from_secs(5));
        // Give the worker every chance to run the cancelled job, so the assertion means something.
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(
            ran.load(Ordering::SeqCst),
            0,
            "the cancelled job ran anyway"
        );
    }

    #[test]
    fn the_queue_refuses_work_it_cannot_hold() {
        let executor = executor(ExecutorConfig {
            workers: 1,
            queue_capacity: 1,
            ..ExecutorConfig::responsive()
        });
        let owner = OwnerId::new();
        let (started, started_rx) = mpsc::channel();
        let (_release, release_rx) = mpsc::channel();

        executor
            .submit(
                SubmitRequest::new(owner, "running"),
                Blocking {
                    started,
                    release: release_rx,
                    observed: Arc::new(AtomicUsize::new(0)),
                    on_release: |_| JobOutcome::Succeeded,
                },
            )
            .expect("the first submission fits");
        started_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the first job started");
        executor
            .submit(SubmitRequest::new(owner, "waiting"), |_| {
                JobOutcome::Succeeded
            })
            .expect("the queue has one slot");

        let refused = executor
            .submit(SubmitRequest::new(owner, "one too many"), |_| {
                JobOutcome::Succeeded
            })
            .expect_err("the queue is full");

        assert_eq!(refused, SubmitError::QueueFull { capacity: 1 });
        assert!(refused.message().contains("queue is full"));
    }

    #[test]
    fn progress_never_goes_backwards() {
        let executor = executor(ExecutorConfig::responsive());
        let (started, started_rx) = mpsc::channel();
        let (release, release_rx) = mpsc::channel();
        let id = executor
            .submit(
                SubmitRequest::new(OwnerId::new(), "reports progress"),
                move |control: JobControl| {
                    control.progress().set(5, Some(10));
                    control.progress().note("halfway");
                    control.progress().set(2, None);
                    let _ = started.send(());
                    let _ = release_rx.recv_timeout(Duration::from_secs(10));
                    JobOutcome::Succeeded
                },
            )
            .expect("the queue has room");
        started_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the job started");

        let record = executor.record(id).expect("the record exists");
        assert_eq!(record.progress.completed(), 5, "a lower value must not win");
        assert_eq!(record.progress.total(), Some(10));
        assert_eq!(record.progress.fraction(), Some(0.5));
        assert_eq!(record.progress.note(), Some("halfway"));

        release.send(()).expect("the job is released");
        let final_record = executor
            .wait(id, Duration::from_secs(5))
            .expect("the record exists");
        assert_eq!(
            final_record.progress.completed(),
            5,
            "the recorded progress must survive the job's own regression",
        );
    }

    #[test]
    fn jobs_run_in_submission_order() {
        let executor = executor(ExecutorConfig::responsive());
        let order = Arc::new(Mutex::new(Vec::new()));
        let mut ids = Vec::new();
        for index in 0..5 {
            let order = order.clone();
            ids.push(
                executor
                    .submit(
                        SubmitRequest::new(OwnerId::new(), format!("job {index}")),
                        move |_| {
                            order.lock().expect("the lock is not poisoned").push(index);
                            JobOutcome::Succeeded
                        },
                    )
                    .expect("the queue has room"),
            );
        }
        for id in ids {
            let record = executor.wait(id, Duration::from_secs(5)).expect("a record");
            assert!(record.is_final(), "{} did not finish", record.id);
        }
        assert_eq!(
            *order.lock().expect("the lock is not poisoned"),
            vec![0, 1, 2, 3, 4]
        );
    }

    #[test]
    fn cancelling_one_owner_leaves_another_owner_s_work_alone() {
        let executor = executor(ExecutorConfig {
            workers: 2,
            ..ExecutorConfig::responsive()
        });
        let (mine, other) = (OwnerId::new(), OwnerId::new());
        let (started_tx, started_rx) = mpsc::channel();
        let (release, release_rx) = mpsc::channel();
        let release_rx = Arc::new(Mutex::new(release_rx));

        let mut mine_tasks = Vec::new();
        for index in 0..2 {
            let started = started_tx.clone();
            let release = release_rx.clone();
            mine_tasks.push(
                executor
                    .submit(
                        SubmitRequest::new(mine, format!("mine {index}")),
                        move |_| {
                            let _ = started.send(());
                            let _ = release
                                .lock()
                                .expect("not poisoned")
                                .recv_timeout(Duration::from_secs(20));
                            JobOutcome::Succeeded
                        },
                    )
                    .expect("the queue has room"),
            );
        }
        for _ in 0..2 {
            started_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("both jobs started");
        }

        // The other owner's work waits behind the two jobs that hold the workers.
        let other_task = executor
            .submit(SubmitRequest::new(other, "the other owner's work"), |_| {
                JobOutcome::Succeeded
            })
            .expect("the queue has room");

        assert_eq!(executor.cancel_owner(mine).len(), 2);
        for id in &mine_tasks {
            assert_eq!(
                executor.record(*id).expect("a record").state,
                TaskState::Cancelling,
                "{id} should be cancelling",
            );
        }
        let queued = executor.record(other_task).expect("a record");
        assert_eq!(queued.state, TaskState::Queued);
        assert!(
            !queued.cancel_requested,
            "cancelling one owner asked another owner's task to stop",
        );

        for _ in 0..2 {
            release.send(()).expect("both jobs are released");
        }
        for id in &mine_tasks {
            let record = executor
                .wait(*id, Duration::from_secs(20))
                .expect("a record");
            assert!(
                matches!(record.outcome, Some(TaskOutcome::Succeeded { .. })),
                "a released job should finish: {:?}",
                record.outcome,
            );
        }
        let other_record = executor
            .wait(other_task, Duration::from_secs(20))
            .expect("the other owner's record");
        assert!(
            matches!(other_record.outcome, Some(TaskOutcome::Succeeded { .. })),
            "the other owner's work was disturbed: {:?}",
            other_record.outcome,
        );
        assert!(!other_record.cancel_requested);
    }

    #[test]
    fn a_job_that_will_not_stop_is_abandoned_and_then_reported_as_returned() {
        let executor = executor(ExecutorConfig {
            timeout_grace: Duration::from_millis(30),
            ..ExecutorConfig::responsive()
        });
        let (release, release_rx) = mpsc::channel();
        let (started, started_rx) = mpsc::channel();
        let id = executor
            .submit(
                SubmitRequest::new(OwnerId::new(), "ignores its deadline")
                    .with_timeout(Duration::from_millis(30)),
                move |_control: JobControl| {
                    let _ = started.send(());
                    // Deliberately ignores the token: this is the job the host cannot stop.
                    let _ = release_rx.recv_timeout(Duration::from_secs(10));
                    JobOutcome::Succeeded
                },
            )
            .expect("the queue has room");
        started_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("started");

        let record = executor
            .wait(id, Duration::from_secs(5))
            .expect("the record exists");
        match record.outcome {
            Some(TaskOutcome::Abandoned {
                deadline,
                job_still_running,
            }) => {
                assert_eq!(deadline, Duration::from_millis(30));
                assert!(job_still_running, "the job has not returned yet");
            }
            other => panic!("expected Abandoned, got {other:?}"),
        }
        assert!(record.deadline_reached);
        assert!(record.cancel_requested);
        assert!(
            record
                .outcome
                .as_ref()
                .expect("an outcome")
                .message()
                .contains("abandoned")
        );

        // When the job finally returns, the flag is cleared and the result is not resurrected.
        release.send(()).expect("the job is released");
        let deadline = Instant::now() + Duration::from_secs(5);
        let settled = loop {
            let record = executor.record(id).expect("the record exists");
            match record.outcome {
                Some(TaskOutcome::Abandoned {
                    job_still_running: false,
                    ..
                }) => break record,
                _ if Instant::now() >= deadline => {
                    panic!("the late return was never recorded: {record:?}")
                }
                _ => std::thread::sleep(Duration::from_millis(10)),
            }
        };
        assert_eq!(settled.state, TaskState::Finished);
        assert!(
            settled
                .outcome
                .as_ref()
                .expect("an outcome")
                .message()
                .contains("discarded")
        );
    }

    #[test]
    fn a_job_that_stops_at_its_deadline_is_cancelled_for_that_reason() {
        let executor = executor(ExecutorConfig {
            timeout_grace: Duration::from_millis(200),
            ..ExecutorConfig::responsive()
        });
        let id = executor
            .submit(
                SubmitRequest::new(OwnerId::new(), "stops when asked")
                    .with_timeout(Duration::from_millis(20)),
                |control: JobControl| {
                    if control.token().wait(Duration::from_secs(10)) {
                        JobOutcome::Cancelled
                    } else {
                        JobOutcome::Succeeded
                    }
                },
            )
            .expect("the queue has room");

        let record = executor
            .wait(id, Duration::from_secs(5))
            .expect("the record exists");
        match record.outcome {
            Some(TaskOutcome::Cancelled { reason, .. }) => {
                assert_eq!(reason, CancellationReason::DeadlineExceeded);
            }
            other => panic!("expected a deadline cancellation, got {other:?}"),
        }
        assert!(
            record.deadline_reached,
            "the reason must be visible on the record"
        );
    }

    #[test]
    fn a_failing_job_keeps_its_own_message() {
        let executor = executor(ExecutorConfig::responsive());
        let id = executor
            .submit(SubmitRequest::new(OwnerId::new(), "fails"), |_| {
                JobOutcome::failed("the toolchain is not installed")
            })
            .expect("the queue has room");

        let record = executor
            .wait(id, Duration::from_secs(5))
            .expect("the record exists");
        match &record.outcome {
            Some(TaskOutcome::Failed { message, .. }) => {
                assert_eq!(message, "the toolchain is not installed");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
        assert!(
            record.summary().contains("the toolchain is not installed"),
            "{}",
            record.summary(),
        );
    }

    #[test]
    fn a_job_claiming_an_unrequested_cancellation_is_a_defect() {
        let executor = executor(ExecutorConfig::responsive());
        let id = executor
            .submit(
                SubmitRequest::new(OwnerId::new(), "lies about being cancelled"),
                |_| JobOutcome::Cancelled,
            )
            .expect("the queue has room");

        let record = executor
            .wait(id, Duration::from_secs(5))
            .expect("the record exists");
        match record.outcome {
            Some(TaskOutcome::Failed { message, .. }) => {
                assert!(message.contains("never requested"), "{message}");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn cancelling_an_unknown_or_finished_task_says_which_it_was() {
        let executor = executor(ExecutorConfig::responsive());
        let id = executor
            .submit(SubmitRequest::new(OwnerId::new(), "quick"), |_| {
                JobOutcome::Succeeded
            })
            .expect("the queue has room");
        let record = executor
            .wait(id, Duration::from_secs(5))
            .expect("the record exists");
        assert!(record.is_final());

        match executor.cancel(id) {
            CancelResult::AlreadyFinished { outcome } => {
                assert!(matches!(outcome, Some(TaskOutcome::Succeeded { .. })));
            }
            other => panic!("expected AlreadyFinished, got {other:?}"),
        }
        assert_eq!(
            executor.cancel(super::TaskId(9_999_999)),
            CancelResult::Unknown
        );
    }

    #[test]
    fn shutdown_reports_what_it_could_not_stop_and_refuses_new_work() {
        let executor = executor(ExecutorConfig {
            shutdown_grace: Duration::from_millis(50),
            ..ExecutorConfig::responsive()
        });
        let owner = OwnerId::new();
        let (started, started_rx) = mpsc::channel();
        let (_release, release_rx) = mpsc::channel();
        let stubborn = executor
            .submit(
                SubmitRequest::new(owner, "will not stop"),
                Blocking {
                    started,
                    release: release_rx,
                    observed: Arc::new(AtomicUsize::new(0)),
                    on_release: |_| JobOutcome::Succeeded,
                },
            )
            .expect("the queue has room");
        started_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("started");

        let report = executor.shutdown();

        assert!(
            !report.is_clean(),
            "an uncooperative job cannot be a clean shutdown"
        );
        assert_eq!(report.abandoned, vec![stubborn]);
        assert!(report.message().contains("did not stop"));
        assert!(executor.is_stopped());
        assert_eq!(
            executor
                .submit(SubmitRequest::new(owner, "too late"), |_| {
                    JobOutcome::Succeeded
                })
                .expect_err("a stopped executor refuses work"),
            SubmitError::Stopped,
        );
        let record = executor.record(stubborn).expect("the record exists");
        assert!(matches!(
            record.outcome,
            Some(TaskOutcome::Abandoned { .. })
        ));
    }

    #[test]
    fn shutdown_of_idle_work_is_clean() {
        let executor = executor(ExecutorConfig::responsive());
        let id = executor
            .submit(SubmitRequest::new(OwnerId::new(), "quick"), |_| {
                JobOutcome::Succeeded
            })
            .expect("the queue has room");
        let _ = executor.wait(id, Duration::from_secs(5));

        let report = executor.shutdown();

        assert!(report.is_clean());
        assert_eq!(report.finished, 1);
        assert_eq!(report.cancelled, 0);
    }

    #[test]
    fn a_panicking_job_does_not_take_the_executor_down() {
        // The record is what a user needs to see; a panic in one job must not stop the others.
        let executor = executor(ExecutorConfig::responsive());
        let (started, started_rx) = mpsc::channel();
        let (release, release_rx) = mpsc::channel();
        let first = executor
            .submit(
                SubmitRequest::new(OwnerId::new(), "will panic"),
                Blocking {
                    started,
                    release: release_rx,
                    observed: Arc::new(AtomicUsize::new(0)),
                    on_release: |_| panic!("this job panicked on purpose"),
                },
            )
            .expect("the queue has room");
        started_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("started");

        release.send(()).expect("the job is released");
        let panicked = executor
            .wait(first, Duration::from_secs(5))
            .expect("the record exists");
        match &panicked.outcome {
            Some(TaskOutcome::Failed { message, .. }) => {
                assert!(message.contains("panicked"), "{message}");
                assert!(message.contains("on purpose"), "{message}");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
        let second = executor
            .submit(
                SubmitRequest::new(OwnerId::new(), "after the panic"),
                |_| JobOutcome::Succeeded,
            )
            .expect("the executor still accepts work");
        let record = executor
            .wait(second, Duration::from_secs(5))
            .expect("the record exists");
        assert!(matches!(
            record.outcome,
            Some(TaskOutcome::Succeeded { .. })
        ));
        let _ = executor.record(first);
        let _ = executor.shutdown();
    }

    #[test]
    fn the_executor_is_shareable_across_threads() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Executor>();
        assert_send_sync::<super::JobControl>();
        assert_send_sync::<super::OwnerId>();
        assert_send_sync::<super::TaskId>();
    }
}
