// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Supervision of an extension's calls (task T22.06).
//!
//! An extension is replaceable code the host does not control, so a call to one is a boundary and
//! not a function call. This module is that boundary:
//!
//! * **Messages are bounded.** A request past [`Limits::max_request_bytes`], an answer past
//!   [`Limits::max_response_bytes`], or a mailbox holding more than [`Limits::in_flight`] requests
//!   is refused before it can grow the host's memory. The worker channel is a `sync_channel`, so
//!   there is no unbounded queue anywhere on the path.
//! * **A call has a deadline.** The supervisor waits [`Limits::call_timeout`] for an answer and
//!   reports [`SupervisorError::Timeout`] — a distinct variant, not a protocol error.
//! * **A failure is contained.** A crash, a timeout, a malformed answer or an oversized answer
//!   marks the extension [`Status::Failed`], drops its worker, and leaves the host's design and
//!   preview surfaces available ([`Health`]). A later call may restart it.
//! * **Restarts are budgeted.** Each restart is counted and delayed by an exponential backoff;
//!   once [`Limits::max_restarts`] is reached the supervisor refuses to restart instead of looping
//!   forever ([`SupervisorError::RestartBudgetExhausted`]).
//! * **No answer is not success.** An empty answer is [`SupervisorError::NotAnswered`], and a
//!   denied permission is [`Status::Denied`] with [`SupervisorError::Denied`]: the caller cannot
//!   mistake either for a successful call.
//!
//! **The process path is a seam, not a claim.** [`Handler`] is the typed boundary a real child
//! process would implement: the worker thread that calls it enforces the timeout and the restart
//! budget, but a real process-backed handler (spawn, framed read/write, process-group teardown) is
//! not implemented here and remains unverified. [`LocalHarness`] is the in-process handler the
//! `--extension supervise` path and the tests use; it proves the supervision rules, not that a
//! child process obeys them.

use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// The limits a supervisor enforces.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Limits {
    /// The largest request the supervisor will send.
    pub max_request_bytes: usize,
    /// The largest answer the supervisor will accept.
    pub max_response_bytes: usize,
    /// How many requests the bounded worker channel may hold.
    pub in_flight: usize,
    /// How long one call may take before it is a timeout.
    pub call_timeout: Duration,
    /// How many restarts are allowed after failures.
    pub max_restarts: u32,
    /// The first restart delay; later delays double.
    pub restart_backoff: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_request_bytes: 16 * 1024,
            max_response_bytes: 64 * 1024,
            in_flight: 1,
            call_timeout: Duration::from_secs(2),
            max_restarts: 3,
            restart_backoff: Duration::from_millis(50),
        }
    }
}

impl Limits {
    /// Checks the limits are usable. A zero-sized mailbox or a zero timeout would make every call
    /// fail for a reason the caller did not ask for, so they are refused rather than accepted.
    pub fn validate(&self) -> Result<(), String> {
        if self.max_request_bytes == 0 {
            return Err("max_request_bytes must be greater than zero".to_owned());
        }
        if self.max_response_bytes == 0 {
            return Err("max_response_bytes must be greater than zero".to_owned());
        }
        if self.in_flight == 0 {
            return Err("in_flight must be greater than zero".to_owned());
        }
        if self.call_timeout.is_zero() {
            return Err("call_timeout must be greater than zero".to_owned());
        }
        Ok(())
    }
}

/// What the permission gate said before the supervisor was created.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Access {
    /// The extension may be called.
    Granted,
    /// The extension may not be called, and the reason is shown to the caller.
    Denied {
        /// Why the permission was refused.
        reason: String,
    },
}

/// The typed state of a supervised extension.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Status {
    /// A worker is up and the last call was answered.
    Running,
    /// No worker is up; the next call starts one.
    Stopped,
    /// The extension failed and is isolated until a restart within the budget.
    Failed {
        /// What went wrong.
        reason: String,
    },
    /// The extension never started because its permission was refused.
    Denied {
        /// Why the permission was refused.
        reason: String,
    },
}

impl Status {
    /// The stable name a report uses.
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Stopped => "stopped",
            Self::Failed { .. } => "failed",
            Self::Denied { .. } => "denied",
        }
    }

    /// The state in one line, with the reason when there is one.
    pub fn render(&self) -> String {
        match self {
            Self::Failed { reason } => format!("failed: {reason}"),
            Self::Denied { reason } => format!("denied: {reason}"),
            Self::Running | Self::Stopped => self.name().to_owned(),
        }
    }
}

/// What remains available when an extension fails.
///
/// The extension is the part that is allowed to disappear; the host's own design and preview are
/// not. Keeping them as separate fields makes the isolation claim checkable rather than implied.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Health {
    /// Whether the host's basic design surface is available.
    pub design: bool,
    /// Whether the host's preview surface is available.
    pub preview: bool,
    /// Whether the extension can be called right now.
    pub extension: bool,
}

impl Health {
    /// The health in one line.
    pub fn render(&self) -> String {
        format!(
            "design: {}, preview: {}, extension: {}",
            on_off(self.design),
            on_off(self.preview),
            on_off(self.extension),
        )
    }
}

/// Why a supervised call or a supervisor operation failed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SupervisorError {
    /// The extension's permission was refused.
    Denied {
        /// Why.
        reason: String,
    },
    /// The limits are not usable.
    InvalidLimits {
        /// Why.
        reason: String,
    },
    /// The request is larger than the supervisor sends.
    RequestTooLarge {
        /// The size that was offered.
        size: usize,
        /// The limit.
        limit: usize,
    },
    /// The answer is larger than the supervisor accepts.
    ResponseTooLarge {
        /// The size that arrived.
        size: usize,
        /// The limit.
        limit: usize,
    },
    /// The bounded mailbox already holds this many requests.
    InFlight {
        /// The limit.
        limit: usize,
    },
    /// The extension did not answer before the deadline.
    Timeout {
        /// The deadline.
        limit: Duration,
    },
    /// The extension answered, but not in the protocol.
    Protocol {
        /// Why the answer is unusable.
        reason: String,
    },
    /// The extension returned nothing. An empty answer is not a successful answer.
    NotAnswered,
    /// The extension stopped before answering.
    Crashed {
        /// Why it stopped.
        reason: String,
    },
    /// The restart backoff has not elapsed.
    Backoff {
        /// How much longer to wait.
        remaining: Duration,
    },
    /// The extension used its restart budget.
    RestartBudgetExhausted {
        /// How many restarts were performed.
        restarts: u32,
    },
    /// The worker could not be started.
    StartFailed {
        /// Why.
        reason: String,
    },
}

impl SupervisorError {
    /// The failure in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Denied { reason } => format!("the extension is denied: {reason}"),
            Self::InvalidLimits { reason } => {
                format!("the supervisor limits are invalid: {reason}")
            }
            Self::RequestTooLarge { size, limit } => {
                format!("the request is {size} byte(s); the supervisor allows {limit}")
            }
            Self::ResponseTooLarge { size, limit } => {
                format!("the answer is {size} byte(s); the supervisor allows {limit}")
            }
            Self::InFlight { limit } => {
                format!("the extension already holds {limit} request(s); the mailbox is bounded")
            }
            Self::Timeout { limit } => {
                format!(
                    "the extension did not answer within {} ms",
                    limit.as_millis()
                )
            }
            Self::Protocol { reason } => format!("the extension protocol failed: {reason}"),
            Self::NotAnswered => {
                "the extension returned no answer; an empty answer is not success".to_owned()
            }
            Self::Crashed { reason } => format!("the extension stopped: {reason}"),
            Self::Backoff { remaining } => {
                format!(
                    "the extension is in its restart backoff for another {} ms",
                    remaining.as_millis()
                )
            }
            Self::RestartBudgetExhausted { restarts } => format!(
                "the extension used its restart budget after {restarts} restart(s); it will not be restarted again"
            ),
            Self::StartFailed { reason } => {
                format!("the extension could not be started: {reason}")
            }
        }
    }
}

impl fmt::Display for SupervisorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.render())
    }
}

impl std::error::Error for SupervisorError {}

/// Why a [`Handler`] could not answer.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum HandlerFailure {
    /// The handler stopped, as a child process exiting would.
    Crashed {
        /// Why it stopped.
        reason: String,
    },
    /// The handler answered with something that is not the protocol.
    Protocol {
        /// Why the answer is unusable.
        reason: String,
    },
}

impl HandlerFailure {
    /// The reason, whichever kind of failure it is.
    pub fn reason(&self) -> String {
        match self {
            Self::Crashed { reason } | Self::Protocol { reason } => reason.clone(),
        }
    }
}

/// The typed seam a real extension process implements.
///
/// A handler is called by the supervisor's worker thread, one request at a time. It must return
/// [`HandlerFailure::Crashed`] when the process is gone and [`HandlerFailure::Protocol`] when it
/// answered but the answer is unusable; it must not hide either as a successful empty answer.
pub trait Handler: Send {
    /// Answers one request.
    fn request(&mut self, request: &[u8]) -> Result<Vec<u8>, HandlerFailure>;
}

/// A supervisor for one extension id.
pub struct Supervisor {
    id: String,
    limits: Limits,
    access: Access,
    factory: Box<dyn FnMut() -> Result<Box<dyn Handler>, HandlerFailure> + Send>,
    worker: Option<Worker>,
    status: Status,
    restarts: u32,
    restart_at: Option<Instant>,
}

impl Supervisor {
    /// Creates a supervisor. The handler factory is called on the first call and on every restart.
    pub fn new<F>(
        id: impl Into<String>,
        limits: Limits,
        access: Access,
        factory: F,
    ) -> Result<Self, SupervisorError>
    where
        F: FnMut() -> Result<Box<dyn Handler>, HandlerFailure> + Send + 'static,
    {
        limits
            .validate()
            .map_err(|reason| SupervisorError::InvalidLimits { reason })?;
        let status = match &access {
            Access::Granted => Status::Stopped,
            Access::Denied { reason } => Status::Denied {
                reason: reason.clone(),
            },
        };
        Ok(Self {
            id: id.into(),
            limits,
            access,
            factory: Box::new(factory),
            worker: None,
            status,
            restarts: 0,
            restart_at: None,
        })
    }

    /// The extension's id.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The current typed status.
    pub fn status(&self) -> &Status {
        &self.status
    }

    /// How many restarts have been performed.
    pub fn restarts(&self) -> u32 {
        self.restarts
    }

    /// Whether the extension can be called right now.
    pub fn extension_available(&self) -> bool {
        matches!(&self.status, Status::Running)
    }

    /// What remains available after the extension's current state.
    pub fn health(&self) -> Health {
        Health {
            design: true,
            preview: true,
            extension: self.extension_available(),
        }
    }

    /// The supervisor in one line.
    pub fn render(&self) -> String {
        format!(
            "supervisor {}: {} (restarts {}/{})",
            self.id(),
            self.status().render(),
            self.restarts(),
            self.limits.max_restarts,
        )
    }

    /// Starts a worker if one is not already up.
    pub fn start(&mut self) -> Result<(), SupervisorError> {
        if let Access::Denied { reason } = &self.access {
            self.status = Status::Denied {
                reason: reason.clone(),
            };
            return Err(SupervisorError::Denied {
                reason: reason.clone(),
            });
        }
        if matches!(&self.status, Status::Failed { .. }) {
            // A public start must not be a way around the restart budget.
            return self.restart();
        }
        if self.worker.is_some() {
            self.status = Status::Running;
            return Ok(());
        }
        let handler = match (self.factory)() {
            Ok(handler) => handler,
            Err(failure) => {
                let reason = failure.reason();
                self.mark_failed(reason.clone());
                return Err(SupervisorError::StartFailed { reason });
            }
        };
        let worker = match Worker::spawn(handler, self.limits) {
            Ok(worker) => worker,
            Err(error) => {
                self.mark_failed(error.render());
                return Err(error);
            }
        };
        self.worker = Some(worker);
        self.status = Status::Running;
        self.restart_at = None;
        Ok(())
    }

    /// Sends one request and waits for one bounded answer.
    pub fn call(&mut self, request: &[u8]) -> Result<Vec<u8>, SupervisorError> {
        if let Access::Denied { reason } = &self.access {
            self.status = Status::Denied {
                reason: reason.clone(),
            };
            return Err(SupervisorError::Denied {
                reason: reason.clone(),
            });
        }
        if request.len() > self.limits.max_request_bytes {
            return Err(SupervisorError::RequestTooLarge {
                size: request.len(),
                limit: self.limits.max_request_bytes,
            });
        }
        if matches!(&self.status, Status::Failed { .. }) {
            self.restart()?;
        } else if self.status == Status::Stopped || self.worker.is_none() {
            self.start()?;
        }

        let outcome = match self.worker.as_ref() {
            Some(worker) => worker.call(request, self.limits.call_timeout),
            None => {
                return Err(SupervisorError::StartFailed {
                    reason: "the supervisor has no worker".to_owned(),
                });
            }
        };

        match outcome {
            Ok(Ok(answer)) => {
                if answer.len() > self.limits.max_response_bytes {
                    let reason = format!(
                        "the answer is {} bytes; the supervisor allows {}",
                        answer.len(),
                        self.limits.max_response_bytes
                    );
                    self.mark_failed(reason);
                    return Err(SupervisorError::ResponseTooLarge {
                        size: answer.len(),
                        limit: self.limits.max_response_bytes,
                    });
                }
                if answer.is_empty() {
                    self.mark_failed("the extension returned an empty answer".to_owned());
                    return Err(SupervisorError::NotAnswered);
                }
                self.status = Status::Running;
                Ok(answer)
            }
            Ok(Err(failure)) => {
                let reason = failure.reason();
                self.mark_failed(reason.clone());
                match failure {
                    HandlerFailure::Crashed { .. } => Err(SupervisorError::Crashed { reason }),
                    HandlerFailure::Protocol { .. } => Err(SupervisorError::Protocol { reason }),
                }
            }
            Err(WorkerFault::Full) => Err(SupervisorError::InFlight {
                limit: self.limits.in_flight,
            }),
            Err(WorkerFault::Timeout) => {
                self.mark_failed(format!(
                    "the call timed out after {} ms",
                    self.limits.call_timeout.as_millis()
                ));
                Err(SupervisorError::Timeout {
                    limit: self.limits.call_timeout,
                })
            }
            Err(WorkerFault::Gone) => {
                let reason = "the extension worker stopped without answering".to_owned();
                self.mark_failed(reason.clone());
                Err(SupervisorError::Crashed { reason })
            }
        }
    }

    /// Stops the worker and returns the extension to [`Status::Stopped`].
    ///
    /// The restart count is reset because this is an explicit reclaim, not a failure loop. A denied
    /// extension stays denied: stopping it does not grant the permission that was refused.
    pub fn stop(&mut self) {
        self.worker = None;
        self.restarts = 0;
        self.restart_at = None;
        if !matches!(&self.status, Status::Denied { .. }) {
            self.status = Status::Stopped;
        }
    }

    fn restart(&mut self) -> Result<(), SupervisorError> {
        if let Access::Denied { reason } = &self.access {
            self.status = Status::Denied {
                reason: reason.clone(),
            };
            return Err(SupervisorError::Denied {
                reason: reason.clone(),
            });
        }
        if self.restarts >= self.limits.max_restarts {
            let reason = format!(
                "the extension used its restart budget of {}",
                self.limits.max_restarts
            );
            self.status = Status::Failed { reason };
            self.restart_at = None;
            return Err(SupervisorError::RestartBudgetExhausted {
                restarts: self.restarts,
            });
        }
        if let Some(at) = self.restart_at {
            let now = Instant::now();
            if now < at {
                return Err(SupervisorError::Backoff {
                    remaining: at.saturating_duration_since(now),
                });
            }
        }

        // Count the attempt before the factory runs, so a factory that keeps failing consumes the
        // budget too; otherwise a broken extension could be retried forever.
        self.restarts += 1;
        let handler = match (self.factory)() {
            Ok(handler) => handler,
            Err(failure) => {
                let reason = failure.reason();
                self.mark_failed(reason.clone());
                return Err(SupervisorError::StartFailed { reason });
            }
        };
        let worker = match Worker::spawn(handler, self.limits) {
            Ok(worker) => worker,
            Err(error) => {
                self.mark_failed(error.render());
                return Err(error);
            }
        };
        self.worker = Some(worker);
        self.status = Status::Running;
        self.restart_at = None;
        Ok(())
    }

    fn mark_failed(&mut self, reason: String) {
        self.worker = None;
        let factor = 1u32 << self.restarts.min(5);
        let backoff = self.limits.restart_backoff.saturating_mul(factor);
        self.restart_at = Some(Instant::now() + backoff);
        self.status = Status::Failed { reason };
    }
}

/// The supervisor's bounded channel to one handler.
struct Worker {
    requests: SyncSender<Vec<u8>>,
    responses: Receiver<Result<Vec<u8>, HandlerFailure>>,
}

impl Worker {
    fn spawn(mut handler: Box<dyn Handler>, limits: Limits) -> Result<Self, SupervisorError> {
        let (requests, request_queue) = sync_channel::<Vec<u8>>(limits.in_flight);
        let (response_sender, responses) =
            sync_channel::<Result<Vec<u8>, HandlerFailure>>(limits.in_flight);
        thread::Builder::new()
            .name("extension-supervisor".to_owned())
            .spawn(move || {
                while let Ok(request) = request_queue.recv() {
                    let answer = handler.request(&request);
                    let stop = answer.is_err();
                    if response_sender.send(answer).is_err() || stop {
                        break;
                    }
                }
            })
            .map_err(|error| SupervisorError::StartFailed {
                reason: format!("cannot start the extension worker: {error}"),
            })?;
        Ok(Self {
            requests,
            responses,
        })
    }

    /// Enqueues one request on the bounded channel.
    fn submit(&self, request: Vec<u8>) -> Result<(), WorkerFault> {
        match self.requests.try_send(request) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(WorkerFault::Full),
            Err(TrySendError::Disconnected(_)) => Err(WorkerFault::Gone),
        }
    }

    /// Waits for one answer, bounded by the deadline.
    fn receive(&self, timeout: Duration) -> Result<Result<Vec<u8>, HandlerFailure>, WorkerFault> {
        match self.responses.recv_timeout(timeout) {
            Ok(answer) => Ok(answer),
            Err(RecvTimeoutError::Timeout) => Err(WorkerFault::Timeout),
            Err(RecvTimeoutError::Disconnected) => Err(WorkerFault::Gone),
        }
    }

    fn call(
        &self,
        request: &[u8],
        timeout: Duration,
    ) -> Result<Result<Vec<u8>, HandlerFailure>, WorkerFault> {
        self.submit(request.to_vec())?;
        self.receive(timeout)
    }
}

/// Why the worker could not carry a request.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum WorkerFault {
    /// The bounded mailbox is full.
    Full,
    /// The deadline passed.
    Timeout,
    /// The worker stopped.
    Gone,
}

/// The behaviour the in-process harness exhibits on its next calls.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub enum LocalBehavior {
    /// Answer with the request.
    #[default]
    Echo,
    /// Stop, as a child process exiting would.
    Crash {
        /// Why it stopped.
        reason: String,
    },
    /// Answer with something that is not the protocol.
    Protocol {
        /// Why the answer is unusable.
        reason: String,
    },
    /// Sleep before answering, so a short deadline becomes a timeout.
    Sleep {
        /// How long to sleep.
        millis: u64,
    },
    /// Answer with this many bytes, to cross the response limit.
    Oversize {
        /// The answer size.
        bytes: usize,
    },
    /// Answer with nothing, which is not a successful answer.
    Empty,
}

impl LocalBehavior {
    /// A handler that stops with this reason.
    pub fn crash(reason: impl Into<String>) -> Self {
        Self::Crash {
            reason: reason.into(),
        }
    }

    /// A handler that answers outside the protocol.
    pub fn protocol(reason: impl Into<String>) -> Self {
        Self::Protocol {
            reason: reason.into(),
        }
    }

    /// A handler that sleeps for this long before answering.
    pub const fn sleep(millis: u64) -> Self {
        Self::Sleep { millis }
    }

    /// A handler that answers with this many bytes.
    pub const fn oversize(bytes: usize) -> Self {
        Self::Oversize { bytes }
    }

    /// The behaviour's stable name.
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Echo => "echo",
            Self::Crash { .. } => "crash",
            Self::Protocol { .. } => "protocol",
            Self::Sleep { .. } => "sleep",
            Self::Oversize { .. } => "oversize",
            Self::Empty => "empty",
        }
    }
}

/// An in-process handler whose behaviour can be changed between calls.
///
/// This is the harness the `--extension supervise` path uses, and the fake extension the tests use.
/// It deliberately proves the supervisor's rules, not that a real child process obeys them; the
/// [`Handler`] implementation is the seam a process-backed handler would fill.
#[derive(Clone)]
pub struct LocalHarness {
    behavior: Arc<Mutex<LocalBehavior>>,
    starts: Arc<AtomicUsize>,
}

impl LocalHarness {
    /// A harness that echoes requests.
    pub fn new() -> Self {
        Self {
            behavior: Arc::new(Mutex::new(LocalBehavior::Echo)),
            starts: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Changes what the next handler instance does.
    pub fn set_behavior(&self, behavior: LocalBehavior) {
        if let Ok(mut current) = self.behavior.lock() {
            *current = behavior;
        }
    }

    /// The behaviour the next handler instance will use.
    pub fn behavior(&self) -> LocalBehavior {
        match self.behavior.lock() {
            Ok(current) => current.clone(),
            Err(_) => LocalBehavior::crash("the local harness lock is poisoned"),
        }
    }

    /// How many handler instances the factory has created.
    pub fn starts(&self) -> usize {
        self.starts.load(Ordering::SeqCst)
    }

    /// A supervisor whose handler is this harness.
    pub fn supervisor(
        &self,
        id: impl Into<String>,
        limits: Limits,
        access: Access,
    ) -> Result<Supervisor, SupervisorError> {
        let behavior = Arc::clone(&self.behavior);
        let starts = Arc::clone(&self.starts);
        Supervisor::new(id, limits, access, move || {
            starts.fetch_add(1, Ordering::SeqCst);
            let behavior = behavior
                .lock()
                .map_err(|_| HandlerFailure::Crashed {
                    reason: "the local harness lock is poisoned".to_owned(),
                })?
                .clone();
            Ok(Box::new(LocalHandler { behavior }) as Box<dyn Handler>)
        })
    }
}

impl Default for LocalHarness {
    fn default() -> Self {
        Self::new()
    }
}

struct LocalHandler {
    behavior: LocalBehavior,
}

impl Handler for LocalHandler {
    fn request(&mut self, request: &[u8]) -> Result<Vec<u8>, HandlerFailure> {
        match &self.behavior {
            LocalBehavior::Echo => Ok(request.to_vec()),
            LocalBehavior::Crash { reason } => Err(HandlerFailure::Crashed {
                reason: reason.clone(),
            }),
            LocalBehavior::Protocol { reason } => Err(HandlerFailure::Protocol {
                reason: reason.clone(),
            }),
            LocalBehavior::Sleep { millis } => {
                thread::sleep(Duration::from_millis(*millis));
                Ok(request.to_vec())
            }
            LocalBehavior::Oversize { bytes } => Ok(vec![b'x'; *bytes]),
            LocalBehavior::Empty => Ok(Vec::new()),
        }
    }
}

const fn on_off(value: bool) -> &'static str {
    if value { "on" } else { "off" }
}

#[cfg(test)]
mod tests {
    use super::{
        Access, Handler, HandlerFailure, Health, Limits, LocalBehavior, LocalHarness, Status,
        Supervisor, SupervisorError, Worker, WorkerFault,
    };
    use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
    use std::time::Duration;

    fn limits() -> Limits {
        Limits {
            max_request_bytes: 32,
            max_response_bytes: 64,
            in_flight: 1,
            call_timeout: Duration::from_millis(50),
            max_restarts: 1,
            restart_backoff: Duration::ZERO,
        }
    }

    fn supervisor(harness: &LocalHarness) -> Supervisor {
        harness
            .supervisor("demo", limits(), Access::Granted)
            .expect("the limits are usable")
    }

    #[test]
    fn a_normal_request_round_trips_and_the_supervisor_is_running() {
        let harness = LocalHarness::new();
        let mut supervisor = supervisor(&harness);

        assert_eq!(supervisor.status(), &Status::Stopped);
        assert_eq!(
            supervisor.call(b"ping").expect("an answer"),
            b"ping".to_vec()
        );
        assert_eq!(supervisor.status(), &Status::Running);
        assert_eq!(
            supervisor.health(),
            Health {
                design: true,
                preview: true,
                extension: true,
            }
        );
    }

    #[test]
    fn an_oversized_request_is_refused_before_the_handler_starts() {
        let harness = LocalHarness::new();
        let mut supervisor = supervisor(&harness);

        let error = supervisor
            .call(&[b'x'; 33])
            .expect_err("the request is past the limit");
        assert_eq!(
            error,
            SupervisorError::RequestTooLarge {
                size: 33,
                limit: 32,
            }
        );
        assert_eq!(harness.starts(), 0, "nothing should have been started");
    }

    #[test]
    fn an_oversized_answer_is_refused_and_isolates_the_extension() {
        let harness = LocalHarness::new();
        harness.set_behavior(LocalBehavior::oversize(65));
        let mut supervisor = supervisor(&harness);

        let error = supervisor
            .call(b"ping")
            .expect_err("the answer is too large");
        assert_eq!(
            error,
            SupervisorError::ResponseTooLarge {
                size: 65,
                limit: 64,
            }
        );
        assert!(matches!(supervisor.status(), Status::Failed { .. }));
        assert!(!supervisor.health().extension);
        assert!(supervisor.health().design && supervisor.health().preview);
    }

    #[test]
    fn a_slow_answer_is_a_timeout_not_a_protocol_error() {
        let harness = LocalHarness::new();
        harness.set_behavior(LocalBehavior::sleep(500));
        let mut supervisor = supervisor(&harness);

        let error = supervisor.call(b"ping").expect_err("the answer is late");
        assert!(
            matches!(error, SupervisorError::Timeout { .. }),
            "{error:?}"
        );
        assert!(
            !matches!(error, SupervisorError::Protocol { .. }),
            "a timeout must stay distinct from a protocol error"
        );
        assert!(matches!(supervisor.status(), Status::Failed { .. }));
        assert!(supervisor.health().design && supervisor.health().preview);
    }

    #[test]
    fn a_crash_is_isolated_and_the_next_call_restarts_within_budget() {
        let harness = LocalHarness::new();
        harness.set_behavior(LocalBehavior::crash("the extension process exited"));
        let mut supervisor = supervisor(&harness);

        let error = supervisor.call(b"first").expect_err("the handler stopped");
        assert!(
            matches!(error, SupervisorError::Crashed { .. }),
            "{error:?}"
        );
        assert!(matches!(supervisor.status(), Status::Failed { .. }));
        assert!(!supervisor.health().extension);
        assert!(supervisor.health().design && supervisor.health().preview);
        assert_eq!(supervisor.restarts(), 0);

        harness.set_behavior(LocalBehavior::Echo);
        assert_eq!(
            supervisor.call(b"second").expect("the restart answered"),
            b"second".to_vec()
        );
        assert_eq!(supervisor.restarts(), 1);
        assert_eq!(supervisor.status(), &Status::Running);
        assert_eq!(harness.starts(), 2, "the first start and one restart");
    }

    #[test]
    fn the_restart_budget_stops_an_endless_restart_loop() {
        let harness = LocalHarness::new();
        harness.set_behavior(LocalBehavior::crash("the extension process exited"));
        let mut supervisor = supervisor(&harness);

        let first = supervisor.call(b"one").expect_err("the handler stopped");
        assert!(matches!(first, SupervisorError::Crashed { .. }));
        assert_eq!(supervisor.restarts(), 0);

        let second = supervisor.call(b"two").expect_err("the restart stopped");
        assert!(matches!(second, SupervisorError::Crashed { .. }));
        assert_eq!(supervisor.restarts(), 1);

        let third = supervisor.call(b"three").expect_err("the budget is gone");
        assert_eq!(
            third,
            SupervisorError::RestartBudgetExhausted { restarts: 1 }
        );
        assert_eq!(harness.starts(), 2, "no restart past the budget");
        assert!(matches!(supervisor.status(), Status::Failed { .. }));
    }

    #[test]
    fn a_restart_waits_for_the_backoff() {
        let harness = LocalHarness::new();
        harness.set_behavior(LocalBehavior::crash("the extension process exited"));
        let limits = Limits {
            restart_backoff: Duration::from_secs(60),
            ..limits()
        };
        let mut supervisor = harness
            .supervisor("demo", limits, Access::Granted)
            .expect("the limits are usable");

        let first = supervisor.call(b"one").expect_err("the handler stopped");
        assert!(matches!(first, SupervisorError::Crashed { .. }));

        harness.set_behavior(LocalBehavior::Echo);
        let second = supervisor.call(b"two").expect_err("the backoff is running");
        assert!(
            matches!(second, SupervisorError::Backoff { .. }),
            "{second:?}"
        );
        assert_eq!(supervisor.restarts(), 0, "a backoff is not a restart yet");
    }

    #[test]
    fn a_denied_permission_fails_visibly_and_starts_nothing() {
        let harness = LocalHarness::new();
        let mut supervisor = harness
            .supervisor(
                "demo",
                limits(),
                Access::Denied {
                    reason: "the extension was not granted the command permission".to_owned(),
                },
            )
            .expect("the limits are usable");

        assert!(matches!(supervisor.status(), Status::Denied { .. }));
        let error = supervisor
            .call(b"ping")
            .expect_err("the extension is denied");
        assert_eq!(
            error,
            SupervisorError::Denied {
                reason: "the extension was not granted the command permission".to_owned(),
            }
        );
        assert!(error.render().contains("denied"), "{}", error.render());
        assert_eq!(harness.starts(), 0);
        assert!(!supervisor.health().extension);
        assert!(supervisor.health().design && supervisor.health().preview);
    }

    #[test]
    fn an_empty_answer_is_not_a_success() {
        let harness = LocalHarness::new();
        harness.set_behavior(LocalBehavior::Empty);
        let mut supervisor = supervisor(&harness);

        let error = supervisor.call(b"ping").expect_err("there was no answer");
        assert_eq!(error, SupervisorError::NotAnswered);
        assert!(matches!(supervisor.status(), Status::Failed { .. }));
    }

    #[test]
    fn a_protocol_failure_is_reported_separately_from_a_timeout() {
        let harness = LocalHarness::new();
        harness.set_behavior(LocalBehavior::protocol("the answer is not a frame"));
        let mut supervisor = supervisor(&harness);

        let error = supervisor
            .call(b"ping")
            .expect_err("the answer is unusable");
        assert_eq!(
            error,
            SupervisorError::Protocol {
                reason: "the answer is not a frame".to_owned(),
            }
        );
    }

    #[test]
    fn stopping_reclaims_the_worker_and_a_later_call_starts_again() {
        let harness = LocalHarness::new();
        let mut supervisor = supervisor(&harness);

        supervisor.call(b"ping").expect("an answer");
        assert_eq!(supervisor.status(), &Status::Running);
        supervisor.stop();
        assert_eq!(supervisor.status(), &Status::Stopped);
        assert_eq!(supervisor.restarts(), 0);

        supervisor.call(b"again").expect("a fresh answer");
        assert_eq!(supervisor.status(), &Status::Running);
        assert_eq!(harness.starts(), 2);
    }

    struct BlockingHandler {
        started: SyncSender<()>,
        release: Receiver<()>,
    }

    impl Handler for BlockingHandler {
        fn request(&mut self, _request: &[u8]) -> Result<Vec<u8>, HandlerFailure> {
            let _ = self.started.send(());
            let _ = self.release.recv();
            Ok(b"ok".to_vec())
        }
    }

    #[test]
    fn the_worker_mailbox_refuses_more_requests_than_it_holds() {
        let (started, started_queue) = sync_channel(1);
        let (release, release_queue) = sync_channel(1);
        let worker = Worker::spawn(
            Box::new(BlockingHandler {
                started,
                release: release_queue,
            }),
            limits(),
        )
        .expect("the worker starts");

        worker.submit(b"one".to_vec()).expect("the first request");
        started_queue
            .recv_timeout(Duration::from_secs(1))
            .expect("the handler started");
        worker
            .submit(b"two".to_vec())
            .expect("the second request fits");
        assert_eq!(
            worker.submit(b"three".to_vec()),
            Err(WorkerFault::Full),
            "the channel is bounded"
        );
        let _ = release.send(());
    }
}
