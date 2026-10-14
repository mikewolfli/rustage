// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The notification centre: the host's own answer to "what went wrong, and what can I do about it".
//! Contract **HOST**.
//!
//! Roadmap T01.06 asks for a notification centre, error details, task links, de-duplication and
//! recovery actions. Its acceptance line is **"an error has a cause and a next step, not just log
//! output"**, and roadmap §3.2 adds the other half: severe problems stay traceable while **ordinary
//! log lines must not pop up**. Both are enforced by the shape of this module rather than by
//! discipline at the call sites:
//!
//! * **A cause is a value, never a sentence.** [`Cause`] has one variant per real producer — a
//!   finished [`TaskRecord`], a [`ProcessOutcome`], a [`SpawnError`] and a [`PermissionRefusal`] —
//!   and no constructor takes free text. There is therefore no way to turn a log line into a
//!   notification, which is what "ordinary logs do not raise banners" means in code.
//! * **A next step is not optional.** [`Cause::steps`] returns at least one [`NextStep`] for every
//!   variant, and it is a total match, so a new cause cannot be added without answering "and now
//!   what?" — the acceptance line, held by the compiler.
//! * **An action is distinguishable from advice.** [`NextStep::Do`] is something the host or the UI
//!   can perform ([`RecoveryAction`]); [`NextStep::Advice`] is a sentence for a person, used where
//!   there is no automatic remedy. Advice for a refused permission is the contract's own
//!   `DenialReason::explanation`, not a second wording of it.
//! * **A repeat is counted, not shown again.** [`NotificationCentre::report`] answers whether the
//!   caller should raise anything at all; the second occurrence of the same failure increments
//!   `occurrences` and updates `last_seen` (§3.2's "no repeated prompts").
//! * **A retry that may have side effects asks first.** [`RecoveryAction::Retry`] carries whether
//!   the work has side effects, which is roadmap §3.3's rule about re-running work.
//!
//! **What this module deliberately does not own.** It has no window, no banner and no focus policy:
//! §3.4 and T04.07 decide how a notification reaches a person without stealing the keyboard, and
//! T05.01/T05.07 turn [`RecoveryAction`] into commands. It does not persist notifications beyond the
//! process, and it does not replace the log — a notification is an *addition* to the record, which
//! is why the first occurrence of each cause also writes one log line and a repeat does not.

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use super::log::{Clock, Level, SystemClock, emit};
use super::permission::PermissionRefusal;
use super::subprocess::{ProcessOutcome, SpawnError};
use super::tasks::{CancellationReason, TaskId, TaskOutcome, TaskRecord};
use crate::contracts::permission::Capability;

/// How loud a notification is.
///
/// Three levels, not five: a notification is something a person is expected to act on, so the
/// debug/trace levels a log has would be a promise to interrupt for noise.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Severity {
    /// Work that finished as asked. Reported so a person can see it happened, not to interrupt.
    Info,
    /// Something needs a decision or may become worse; the work did not do what was asked.
    Warning,
    /// Work failed, or a step needs a person before anything else can proceed.
    Error,
}

impl Severity {
    /// The name used in reports and tests.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }

    /// Whether this severity is a problem rather than a record of work well done.
    pub const fn is_problem(self) -> bool {
        matches!(self, Self::Warning | Self::Error)
    }

    /// The log level the first occurrence of this notification is written at.
    ///
    /// Roadmap §4.1 requires logs, notifications and permissions to link to each other; one line in
    /// the log at the matching level is that link, and it is written once per cause rather than once
    /// per occurrence so the log does not become the flood the banner policy forbids.
    pub const fn log_level(self) -> Level {
        match self {
            Self::Info => Level::Info,
            Self::Warning => Level::Warn,
            Self::Error => Level::Error,
        }
    }
}

/// Something the host or the UI can do about a notification.
///
/// These are descriptors, not behaviour: the command layer (T05.01) decides how a person reaches
/// them, and T01.07's registry is what will let the host perform some of them on a specific
/// project's services.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RecoveryAction {
    /// Run the work again.
    Retry {
        /// Whether running it again can do something it already did — write a file, reach a network,
        /// spend money. Roadmap §3.3 asks for a second confirmation when that is true.
        side_effects: bool,
    },
    /// Ask the running task to stop, which is a request the executor honours or reports.
    Cancel {
        /// The task to stop.
        task: TaskId,
    },
    /// Open the task's record: state, progress, outcome and the log it produced.
    OpenTask {
        /// The task to open.
        task: TaskId,
    },
    /// Open the log the notification was written to.
    OpenLog {
        /// What the log is about: the task or process whose output it holds.
        about: String,
    },
}

impl RecoveryAction {
    /// What a menu item or a button says.
    pub fn label(&self) -> String {
        match self {
            Self::Retry { side_effects } => {
                if *side_effects {
                    "run it again (asks first: it can do something it already did)".to_owned()
                } else {
                    "run it again".to_owned()
                }
            }
            Self::Cancel { task } => format!("stop task #{}", task.value()),
            Self::OpenTask { task } => format!("open task #{}", task.value()),
            Self::OpenLog { about } => format!("open the log for {about}"),
        }
    }

    /// Whether performing this needs a person's confirmation first.
    pub fn requires_confirmation(&self) -> bool {
        matches!(self, Self::Retry { side_effects: true })
    }
}

/// One thing a person can do next.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum NextStep {
    /// Something the host or the UI can perform.
    Do(RecoveryAction),
    /// A sentence for a person, where there is no automatic remedy — installing a toolchain, or the
    /// wording the permission contract already uses for a refusal.
    Advice(&'static str),
}

impl NextStep {
    /// What a menu item, a button or a line of text says.
    pub fn label(&self) -> String {
        match self {
            Self::Do(action) => action.label(),
            Self::Advice(advice) => (*advice).to_owned(),
        }
    }

    /// Whether this is something the host can do rather than advice.
    pub const fn is_action(&self) -> bool {
        matches!(self, Self::Do(_))
    }

    /// Whether a person has to confirm before this happens.
    pub fn requires_confirmation(&self) -> bool {
        match self {
            Self::Do(action) => action.requires_confirmation(),
            Self::Advice(_) => false,
        }
    }
}

/// What a notification is about, so a reader can go there.
///
/// Roadmap §3.2 asks for "task links"; the same idea covers the other three things a notification
/// can be about. Every variant is derived from a real producer's own ids rather than from text.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Link {
    /// A task in the executor's record.
    Task {
        /// The task.
        id: TaskId,
        /// Whose work it is.
        owner: super::tasks::OwnerId,
        /// What the submitter called it.
        label: String,
    },
    /// A process this session started.
    Process {
        /// The operating system's pid.
        pid: u32,
        /// What the caller called it.
        label: String,
    },
    /// A program the host could not start.
    Program {
        /// The program.
        program: PathBuf,
    },
    /// A permission decision, as the record wrote it.
    Permission {
        /// The capability that decided it, when a decision existed.
        capability: Option<Capability>,
        /// Which scope was asked about.
        scope: String,
    },
}

impl Link {
    /// The kind, for a report or a test.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Task { .. } => "task",
            Self::Process { .. } => "process",
            Self::Program { .. } => "program",
            Self::Permission { .. } => "permission",
        }
    }

    /// The link in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Task { id, owner, label } => {
                format!("task #{} ({label}) owned by {}", id.value(), owner.value())
            }
            Self::Process { pid, label } => format!("process {pid} ({label})"),
            Self::Program { program } => format!("program {}", program.display()),
            Self::Permission { capability, scope } => match capability {
                Some(capability) => format!("permission {} in {scope}", capability.name()),
                None => format!("permission in {scope}"),
            },
        }
    }
}

/// Why a notification exists.
///
/// Each variant is one of the host's own answers about something that really happened, so the cause
/// cannot be invented: a caller has to hold the value. That is what keeps ordinary log output out of
/// the centre (roadmap §3.2: "ordinary logs do not raise banners").
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Cause {
    /// A task in the executor's record finished, one way or another.
    Task(TaskRecord),
    /// A process this session started ended.
    Process(ProcessOutcome),
    /// A program never started, so nothing ran.
    Program(SpawnError),
    /// The permission gate refused an effect.
    Permission(PermissionRefusal),
}

impl Cause {
    /// A cause for a task, or `None` while it has no outcome to report.
    ///
    /// `None` is not a failure of this function: a queued or running task has nothing to say yet,
    /// and returning an empty notification for it would be the placeholder success principle #22
    /// forbids.
    pub fn for_task(record: &TaskRecord) -> Option<Self> {
        record.outcome.as_ref()?;
        Some(Self::Task(record.clone()))
    }

    /// The task this cause is about, when it is about one.
    pub fn task(&self) -> Option<&TaskRecord> {
        match self {
            Self::Task(record) => Some(record),
            _ => None,
        }
    }

    /// How loud this is. Derived from the outcome rather than chosen by the caller, so a failure
    /// cannot be filed as information.
    pub fn severity(&self) -> Severity {
        match self {
            Self::Task(record) => match record.outcome.as_ref() {
                Some(TaskOutcome::Succeeded { .. }) => Severity::Info,
                Some(TaskOutcome::Failed { .. }) => Severity::Error,
                // A cancellation a person asked for is news; a deadline that stopped the work is a
                // problem, because the work did not finish and nobody chose that.
                Some(TaskOutcome::Cancelled { reason, .. }) => match reason {
                    CancellationReason::Requested => Severity::Info,
                    CancellationReason::DeadlineExceeded => Severity::Warning,
                },
                Some(TaskOutcome::Abandoned { .. }) => Severity::Warning,
                Some(TaskOutcome::Refused { .. }) => Severity::Warning,
                None => Severity::Info,
            },
            Self::Process(outcome) => {
                if outcome.exit.is_success() {
                    Severity::Info
                } else {
                    Severity::Error
                }
            }
            Self::Program(_) => Severity::Error,
            // A refusal is policy, not a defect: the work did not run because it was not allowed to,
            // and the person has to decide something. Calling that an error would tell them to look
            // for a bug.
            Self::Permission(_) => Severity::Warning,
        }
    }

    /// What happened, in one line.
    pub fn summary(&self) -> String {
        match self {
            Self::Task(record) => {
                let outcome = record
                    .outcome
                    .as_ref()
                    .map_or_else(|| "has no outcome yet".to_owned(), TaskOutcome::message);
                format!("task #{} \"{}\" {outcome}", record.id.value(), record.label)
            }
            Self::Process(outcome) => format!("process {}", outcome.summary()),
            Self::Program(error) => error.message(),
            Self::Permission(refusal) => refusal.message(),
        }
    }

    /// The technical detail a person needs, when there is any beyond the summary.
    pub fn detail(&self) -> Option<String> {
        match self {
            Self::Task(record) => {
                let capability = record
                    .capability
                    .map_or("no capability named".to_owned(), |cap| {
                        format!("capability {}", cap.name())
                    });
                Some(format!(
                    "owned by {}, run for {:?}, {capability}",
                    record.owner.value(),
                    record.principal,
                ))
            }
            Self::Process(outcome) => {
                let mut parts = Vec::new();
                if let Some(line) = last_meaningful_line(&outcome.output.stderr) {
                    parts.push(format!("stderr: {line}"));
                }
                if outcome.output.is_truncated() {
                    parts.push(format!(
                        "{} bytes of output were dropped",
                        outcome.output.dropped_bytes,
                    ));
                }
                if parts.is_empty() {
                    None
                } else {
                    Some(parts.join("; "))
                }
            }
            // The summary already names the capability, the scope and the reason; there is no
            // second, more detailed answer to give.
            Self::Program(_) | Self::Permission(_) => None,
        }
    }

    /// What a person can do next. Never empty.
    pub fn steps(&self) -> Vec<NextStep> {
        match self {
            Self::Task(record) => match record.outcome.as_ref() {
                Some(TaskOutcome::Succeeded { .. }) => {
                    vec![NextStep::Do(RecoveryAction::OpenLog {
                        about: format!("task #{}", record.id.value()),
                    })]
                }
                Some(TaskOutcome::Failed { .. }) | Some(TaskOutcome::Refused { .. }) => vec![
                    // A failed task may have written files or reached the network before it failed,
                    // so re-running it is the confirmed kind.
                    NextStep::Do(RecoveryAction::Retry { side_effects: true }),
                    NextStep::Do(RecoveryAction::OpenLog {
                        about: format!("task #{}", record.id.value()),
                    }),
                    NextStep::Do(RecoveryAction::OpenTask { task: record.id }),
                ],
                Some(TaskOutcome::Cancelled { .. }) => vec![
                    NextStep::Do(RecoveryAction::OpenTask { task: record.id }),
                    NextStep::Do(RecoveryAction::Retry { side_effects: true }),
                ],
                Some(TaskOutcome::Abandoned { .. }) => vec![
                    NextStep::Do(RecoveryAction::Cancel { task: record.id }),
                    NextStep::Do(RecoveryAction::OpenLog {
                        about: format!("task #{}", record.id.value()),
                    }),
                ],
                None => vec![NextStep::Advice(
                    "this task has not finished, so there is nothing to report yet",
                )],
            },
            Self::Process(_) => vec![
                NextStep::Do(RecoveryAction::Retry { side_effects: true }),
                NextStep::Do(RecoveryAction::OpenLog {
                    about: "this process".to_owned(),
                }),
            ],
            Self::Program(error) => vec![NextStep::Advice(program_advice(error))],
            Self::Permission(PermissionRefusal::Denied { reason, .. }) => {
                vec![NextStep::Advice(reason.explanation())]
            }
            Self::Permission(PermissionRefusal::Undecided { reason, .. }) => {
                vec![NextStep::Advice(reason)]
            }
        }
    }

    /// Where a reader can go to see this for themselves.
    pub fn link(&self) -> Link {
        match self {
            Self::Task(record) => Link::Task {
                id: record.id,
                owner: record.owner,
                label: record.label.clone(),
            },
            Self::Process(outcome) => Link::Process {
                pid: outcome.pid,
                label: outcome.label.clone(),
            },
            Self::Program(error) => Link::Program {
                program: program_of(error),
            },
            Self::Permission(refusal) => Link::Permission {
                capability: refusal.capability(),
                scope: refusal.scope().to_owned(),
            },
        }
    }

    /// The key that decides whether two notifications are the same problem.
    ///
    /// Deliberately built from what a person would call the same problem: the kind, the thing it is
    /// about, and the message. A second failure with a *different* message is a different problem and
    /// gets its own notification, because "it failed again, differently" is news.
    pub fn key(&self) -> String {
        match self {
            Self::Task(record) => format!(
                "task:{}:{}:{}",
                record.label,
                record.outcome.as_ref().map_or("none", TaskOutcome::name),
                record
                    .outcome
                    .as_ref()
                    .map_or(String::new(), TaskOutcome::message),
            ),
            Self::Process(outcome) => format!(
                "process:{}:{}:{}",
                outcome.label,
                outcome.exit.message(),
                outcome.output.stderr,
            ),
            Self::Program(error) => format!("program:{}", error.message()),
            Self::Permission(refusal) => format!(
                "permission:{}:{}:{}",
                match refusal {
                    PermissionRefusal::Denied { reason, .. } => reason.name(),
                    PermissionRefusal::Undecided { effect, .. } => effect.name(),
                },
                refusal.capability().map_or("none", Capability::name),
                refusal.scope(),
            ),
        }
    }
}

/// The last line of a captured stream that says something, bounded.
fn last_meaningful_line(text: &str) -> Option<String> {
    const LIMIT: usize = 400;
    let line = text
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())?
        .trim();
    if line.chars().count() <= LIMIT {
        return Some(line.to_owned());
    }
    let tail: String = line.chars().skip(line.chars().count() - LIMIT).collect();
    Some(format!("…{tail}"))
}

/// What a person should check when a program could not be started.
fn program_advice(error: &SpawnError) -> &'static str {
    match error {
        SpawnError::NoProgram => "name a program to start",
        SpawnError::Unavailable { .. } => {
            "install the program, or correct the path, toolchain or target that names it, then run \
             the step again"
        }
        SpawnError::WorkingDirectory { .. } => {
            "create the working directory, or correct where the step is told to run"
        }
        SpawnError::PermitForAnotherEffect { .. } => {
            "this is a defect in the host's own code: the step asked for a permit it cannot use"
        }
    }
}

/// The program a spawn error is about.
fn program_of(error: &SpawnError) -> PathBuf {
    match error {
        SpawnError::Unavailable { program, .. }
        | SpawnError::WorkingDirectory {
            directory: program, ..
        } => program.clone(),
        SpawnError::NoProgram | SpawnError::PermitForAnotherEffect { .. } => PathBuf::new(),
    }
}

/// A notification's identity within the centre.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct NotificationId(u64);

impl NotificationId {
    /// The number, for a log line or a test assertion.
    pub const fn value(self) -> u64 {
        self.0
    }
}

impl fmt::Display for NotificationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

/// One problem (or one piece of finished work) in the centre.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Notification {
    id: NotificationId,
    cause: Cause,
    occurrences: u32,
    first_seen_millis: u128,
    last_seen_millis: u128,
    dismissed: bool,
}

impl Notification {
    /// The identity this notification is addressed by.
    pub fn id(&self) -> NotificationId {
        self.id
    }

    /// Why it exists.
    pub fn cause(&self) -> &Cause {
        &self.cause
    }

    /// How loud it is.
    pub fn severity(&self) -> Severity {
        self.cause.severity()
    }

    /// What happened.
    pub fn summary(&self) -> String {
        self.cause.summary()
    }

    /// The technical detail, when there is any.
    pub fn detail(&self) -> Option<String> {
        self.cause.detail()
    }

    /// What a person can do next. Never empty.
    pub fn steps(&self) -> Vec<NextStep> {
        self.cause.steps()
    }

    /// Where to go to see it.
    pub fn link(&self) -> Link {
        self.cause.link()
    }

    /// How many times the same problem has been reported.
    pub fn occurrences(&self) -> u32 {
        self.occurrences
    }

    /// Milliseconds between the centre starting and the first report.
    pub fn first_seen_millis(&self) -> u128 {
        self.first_seen_millis
    }

    /// Milliseconds between the centre starting and the most recent report.
    pub fn last_seen_millis(&self) -> u128 {
        self.last_seen_millis
    }

    /// Whether a person has cleared it from the list.
    pub fn is_dismissed(&self) -> bool {
        self.dismissed
    }

    /// Whether it is still asking for attention.
    pub fn is_open(&self) -> bool {
        !self.dismissed
    }

    /// The notification as text: what happened, the detail, and every next step.
    pub fn render(&self) -> String {
        let mut out = format!(
            "#{} [{}] {}",
            self.id.value(),
            self.severity().name(),
            self.summary(),
        );
        if self.occurrences > 1 {
            out.push_str(&format!(" (seen {} times)", self.occurrences));
        }
        if self.dismissed {
            out.push_str(" (dismissed)");
        }
        out.push('\n');
        out.push_str(&format!("    about: {}\n", self.link().render()));
        if let Some(detail) = self.detail() {
            out.push_str(&format!("    detail: {detail}\n"));
        }
        for step in self.steps() {
            let confirmation = if step.requires_confirmation() {
                " (needs confirmation)"
            } else {
                ""
            };
            out.push_str(&format!("    next: {}{confirmation}\n", step.label()));
        }
        out
    }
}

/// What the centre did with a report.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Reported {
    /// The notification, new or existing.
    pub id: NotificationId,
    /// Whether the caller should raise something, or whether this only added to a count.
    pub first_time: bool,
}

/// How many notifications of each severity the centre holds.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Counts {
    /// Failed work and steps that need a person.
    pub errors: usize,
    /// Work that stopped without being asked to, and refusals.
    pub warnings: usize,
    /// Work that finished as asked.
    pub information: usize,
}

impl Counts {
    /// Whether anything is asking for attention.
    pub const fn is_empty(self) -> bool {
        self.errors == 0 && self.warnings == 0 && self.information == 0
    }
}

/// One host process's notifications.
///
/// The centre keeps the order problems were first seen in, so a person reading the list sees the
/// oldest unresolved problem first rather than the most recent noise.
pub struct NotificationCentre {
    clock: Box<dyn Clock>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    notifications: Vec<Notification>,
    /// Where a notification for a dedup key lives, so a repeat is found without scanning.
    by_key: BTreeMap<String, usize>,
    next_id: u64,
}

impl NotificationCentre {
    /// An empty centre, timed by the process clock.
    pub fn new() -> Self {
        Self::with_clock(Box::new(SystemClock::start()))
    }

    /// An empty centre whose times a test can pin.
    pub fn with_clock(clock: Box<dyn Clock>) -> Self {
        Self {
            clock,
            state: Mutex::new(State {
                notifications: Vec::new(),
                by_key: BTreeMap::new(),
                next_id: 1,
            }),
        }
    }

    /// Records a cause, merging it into an existing notification when it is the same problem.
    ///
    /// The first occurrence of each problem also writes one log line, so the record and the log can
    /// be read against each other. A repeat does not: that is the difference between "there is a
    /// problem" and "the log is talking to me".
    pub fn report(&self, cause: Cause) -> Reported {
        let key = cause.key();
        let at = self.clock.elapsed_millis();
        let mut state = lock(&self.state);

        if let Some(index) = state.by_key.get(&key).copied() {
            let notification = &mut state.notifications[index];
            notification.occurrences += 1;
            notification.last_seen_millis = at;
            // A person who cleared the list before is told again: the problem came back, and hiding
            // a recurrence because it was dismissed once would be the silent failure #22 forbids.
            notification.dismissed = false;
            return Reported {
                id: notification.id,
                first_time: false,
            };
        }

        let id = NotificationId(state.next_id);
        state.next_id += 1;
        let notification = Notification {
            id,
            cause: cause.clone(),
            occurrences: 1,
            first_seen_millis: at,
            last_seen_millis: at,
            dismissed: false,
        };
        let index = state.notifications.len();
        state.by_key.insert(key, index);
        let severity = notification.severity();
        state.notifications.push(notification);

        emit(
            severity.log_level(),
            "host::notify",
            &format!("#{} {}", id.value(), cause.summary()),
        );
        Reported {
            id,
            first_time: true,
        }
    }

    /// Every notification, oldest first, dismissed ones included.
    pub fn notifications(&self) -> Vec<Notification> {
        lock(&self.state).notifications.clone()
    }

    /// The notifications still asking for attention.
    pub fn open(&self) -> Vec<Notification> {
        lock(&self.state)
            .notifications
            .iter()
            .filter(|notification| notification.is_open())
            .cloned()
            .collect()
    }

    /// One notification by id.
    pub fn notification(&self, id: NotificationId) -> Option<Notification> {
        lock(&self.state)
            .notifications
            .iter()
            .find(|notification| notification.id == id)
            .cloned()
    }

    /// Clears one notification from the list, keeping its record and count. Returns whether it was
    /// there to clear.
    pub fn dismiss(&self, id: NotificationId) -> bool {
        let mut state = lock(&self.state);
        match state
            .notifications
            .iter_mut()
            .find(|notification| notification.id == id)
        {
            Some(notification) => {
                notification.dismissed = true;
                true
            }
            None => false,
        }
    }

    /// Clears everything, returning how many notifications that was.
    pub fn dismiss_all(&self) -> usize {
        let mut state = lock(&self.state);
        let mut dismissed = 0;
        for notification in &mut state.notifications {
            if notification.is_open() {
                notification.dismissed = true;
                dismissed += 1;
            }
        }
        dismissed
    }

    /// How many notifications of each severity the centre holds.
    pub fn counts(&self) -> Counts {
        let mut counts = Counts::default();
        for notification in &lock(&self.state).notifications {
            match notification.severity() {
                Severity::Error => counts.errors += 1,
                Severity::Warning => counts.warnings += 1,
                Severity::Info => counts.information += 1,
            }
        }
        counts
    }

    /// The catalogue as text, oldest first.
    pub fn render_text(&self) -> String {
        let mut out = String::new();
        let notifications = lock(&self.state).notifications.clone();
        if notifications.is_empty() {
            return "no notifications\n".to_owned();
        }
        for notification in &notifications {
            out.push_str(&notification.render());
        }
        let counts = self.counts();
        out.push_str(&format!(
            "{} notification(s): {} error, {} warning, {} information\n",
            notifications.len(),
            counts.errors,
            counts.warnings,
            counts.information,
        ));
        out
    }
}

impl Default for NotificationCentre {
    fn default() -> Self {
        Self::new()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .expect("the notification centre is never poisoned: no panic escapes its critical sections")
}

#[cfg(test)]
mod tests {
    use super::{Cause, Counts, Link, NextStep, NotificationCentre, RecoveryAction, Severity};
    use crate::contracts::permission::{Capability, EffectClass, Principal};
    use crate::host::log::FixedClock;
    use crate::host::paths::ProjectRoot;
    use crate::host::permission::{PermissionRefusal, PermissionService, Scope};
    use crate::host::subprocess::{OwnedProcess, ProcessSpec};
    use crate::host::tasks::{
        CancelResult, Executor, ExecutorConfig, JobControl, JobOutcome, OwnerId, SubmitRequest,
        TaskRecord,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    /// Runs one job to completion and returns the executor's own record of it.
    ///
    /// Using the real executor rather than a hand-built record is the point: the notification is
    /// derived from what the host actually reported, so a change in how the executor words an
    /// outcome cannot pass unnoticed here.
    fn finished_task(
        label: &str,
        mut job: impl FnMut() -> JobOutcome + Send + 'static,
    ) -> TaskRecord {
        let executor = Executor::new(ExecutorConfig::responsive()).expect("the executor starts");
        let id = executor
            .submit(
                SubmitRequest::new(OwnerId::new(), label),
                move |_control: JobControl| job(),
            )
            .expect("the executor accepts one job");
        let record = executor
            .wait(id, Duration::from_secs(5))
            .expect("the task finished within the deadline");
        let shutdown = executor.shutdown();
        assert!(shutdown.message().contains("stopped"));
        record
    }

    /// The permit a test needs to start a process at all.
    fn command_permit() -> crate::host::permission::ExecutionPermit {
        PermissionService::new()
            .permit(EffectClass::Command, Scope::Session, Principal::Human)
            .expect("the host's own session may run a command")
    }

    /// Runs a command to completion and returns the host's outcome for it.
    fn ran(script: &str) -> crate::host::subprocess::ProcessOutcome {
        let mut process = OwnedProcess::spawn(
            ProcessSpec::new("test: a command", "sh").args(["-c", script]),
            &command_permit(),
        )
        .expect("the shell starts");
        let exit = process.wait();
        process.outcome(exit)
    }

    /// A refusal from the real gate, for an untrusted project.
    fn untrusted_refusal() -> PermissionRefusal {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "rustage-notify-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed),
        ));
        std::fs::create_dir_all(&path).expect("a temporary project directory is creatable");
        let root = ProjectRoot::new(&path).expect("the sandbox is a directory");
        let refusal = PermissionService::new()
            .permit(
                EffectClass::Command,
                Scope::Project(&root),
                Principal::Human,
            )
            .expect_err("an untrusted project runs no code");
        let _ = std::fs::remove_dir_all(&path);
        refusal
    }

    #[test]
    fn a_failed_task_becomes_a_notification_with_a_cause_and_a_step() {
        let record = finished_task("build the project", || {
            JobOutcome::failed("the linker could not find `libfoo`")
        });
        let centre = NotificationCentre::new();

        let reported = centre.report(Cause::for_task(&record).expect("the task finished"));

        assert!(reported.first_time);
        let notification = centre.notification(reported.id).expect("it was recorded");
        assert_eq!(notification.severity(), Severity::Error);
        assert!(notification.severity().is_problem());
        assert!(
            notification.summary().contains("build the project"),
            "{}",
            notification.summary(),
        );
        assert!(
            notification.summary().contains("libfoo"),
            "the cause has to carry the job's own message: {}",
            notification.summary(),
        );
        assert!(
            notification.detail().is_some(),
            "a task has context to show"
        );
        let steps = notification.steps();
        assert!(
            !steps.is_empty(),
            "an error without a next step is not actionable"
        );
        assert!(matches!(
            steps[0],
            NextStep::Do(RecoveryAction::Retry { side_effects: true })
        ));
        assert!(
            steps[0].requires_confirmation(),
            "re-running may repeat effects"
        );
        assert_eq!(notification.link().kind(), "task");
        assert!(notification.render().contains("next:"));
    }

    #[test]
    fn a_task_with_no_outcome_yet_has_nothing_to_report() {
        let executor = Executor::new(ExecutorConfig::responsive()).expect("the executor starts");
        let (started, started_rx) = std::sync::mpsc::channel();
        let id = executor
            .submit(
                SubmitRequest::new(OwnerId::new(), "waits for a cancel"),
                move |control: JobControl| {
                    let _ = started.send(());
                    if control.token().wait(Duration::from_secs(5)) {
                        JobOutcome::Cancelled
                    } else {
                        JobOutcome::Succeeded
                    }
                },
            )
            .expect("accepted");
        started_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the job started");
        let running = executor.record(id).expect("the record exists");

        assert!(Cause::for_task(&running).is_none());

        assert_eq!(executor.cancel(id), CancelResult::Requested);
        assert!(executor.wait(id, Duration::from_secs(5)).is_some());
        assert!(Cause::for_task(&executor.record(id).expect("still there")).is_some());
        executor.shutdown();
    }

    #[test]
    fn a_successful_task_is_information_with_a_next_step_too() {
        let record = finished_task("render the icons", || JobOutcome::Succeeded);

        let cause = Cause::for_task(&record).expect("the task finished");

        assert_eq!(cause.severity(), Severity::Info);
        assert!(!cause.severity().is_problem());
        assert!(!cause.steps().is_empty());
        assert!(cause.summary().contains("render the icons"));
    }

    #[test]
    fn a_cancelled_task_separates_what_a_person_chose_from_what_a_deadline_did() {
        // A cancellation a person asked for is news, not a problem.
        let executor = Executor::new(ExecutorConfig::responsive()).expect("the executor starts");
        let (started, started_rx) = std::sync::mpsc::channel();
        let id = executor
            .submit(
                SubmitRequest::new(OwnerId::new(), "stop by hand"),
                move |control: JobControl| {
                    let _ = started.send(());
                    if control.token().wait(Duration::from_secs(5)) {
                        JobOutcome::Cancelled
                    } else {
                        JobOutcome::Succeeded
                    }
                },
            )
            .expect("accepted");
        started_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the job started");
        assert_eq!(executor.cancel(id), CancelResult::Requested);
        let asked = executor.wait(id, Duration::from_secs(5)).expect("finished");
        executor.shutdown();
        let cause = Cause::for_task(&asked).expect("the task finished");
        assert_eq!(cause.severity(), Severity::Info);
        assert!(cause.summary().contains("cancelled"), "{}", cause.summary());

        // The same outcome when a deadline, not a person, stopped the work is a problem.
        let executor = Executor::new(ExecutorConfig::responsive()).expect("the executor starts");
        let id = executor
            .submit(
                SubmitRequest::new(OwnerId::new(), "outlives its deadline")
                    .with_timeout(Duration::from_millis(30)),
                |control: JobControl| {
                    if control.token().wait(Duration::from_secs(5)) {
                        JobOutcome::Cancelled
                    } else {
                        JobOutcome::Succeeded
                    }
                },
            )
            .expect("accepted");
        let expired = executor.wait(id, Duration::from_secs(5)).expect("finished");
        executor.shutdown();
        let cause = Cause::for_task(&expired).expect("the task finished");
        assert_eq!(cause.severity(), Severity::Warning);
        assert!(
            cause.summary().contains("deadline-exceeded"),
            "{}",
            cause.summary(),
        );

        // A job announcing a cancellation nobody asked for is the defect T01.03 records as a
        // failure; a notification must not soften it into news.
        let unexpected = finished_task("cancels itself", || JobOutcome::Cancelled);
        assert_eq!(
            Cause::for_task(&unexpected).expect("finished").severity(),
            Severity::Error,
        );
    }

    #[test]
    fn a_failed_process_reports_its_exit_and_its_last_stderr_line() {
        let outcome = ran("echo 'boom: no such target' >&2; exit 3");

        let cause = Cause::Process(outcome);

        assert_eq!(cause.severity(), Severity::Error);
        assert!(
            cause.summary().contains("exit code 3"),
            "{}",
            cause.summary()
        );
        let detail = cause.detail().expect("stderr is the detail a person needs");
        assert!(detail.contains("boom: no such target"), "{detail}");
        assert_eq!(cause.link().kind(), "process");
        assert!(cause.steps().iter().any(NextStep::is_action));
    }

    #[test]
    fn a_process_that_exited_cleanly_is_information() {
        let cause = Cause::Process(ran("exit 0"));

        assert_eq!(cause.severity(), Severity::Info);
        assert!(!cause.steps().is_empty());
    }

    #[test]
    fn a_program_that_never_started_says_what_to_check() {
        let permit = command_permit();
        let error = OwnedProcess::spawn(
            ProcessSpec::new("test: a missing program", "/nonexistent/rustage-tool"),
            &permit,
        )
        .expect_err("the program is not there");

        let cause = Cause::Program(error);

        assert_eq!(cause.severity(), Severity::Error);
        assert!(
            cause.summary().contains("rustage-tool"),
            "{}",
            cause.summary()
        );
        assert_eq!(cause.link().kind(), "program");
        let steps = cause.steps();
        assert_eq!(steps.len(), 1);
        assert!(!steps[0].is_action(), "there is nothing to perform here");
        assert!(
            steps[0].label().contains("install"),
            "the step has to say what a person checks: {}",
            steps[0].label(),
        );
    }

    #[test]
    fn a_permission_refusal_reuses_the_contracts_own_explanation() {
        let refusal = untrusted_refusal();
        let expected = match &refusal {
            PermissionRefusal::Denied { reason, .. } => reason.explanation(),
            other => panic!("expected a denial, got {other:?}"),
        };

        let cause = Cause::Permission(refusal);

        assert_eq!(cause.severity(), Severity::Warning, "policy, not a defect");
        assert_eq!(cause.link().kind(), "permission");
        let steps = cause.steps();
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0], NextStep::Advice(expected));
        assert!(!steps[0].requires_confirmation());
    }

    #[test]
    fn an_effect_with_no_decision_says_which_decision_is_missing() {
        let refusal = PermissionService::new()
            .permit(EffectClass::Deploy, Scope::Session, Principal::Human)
            .expect_err("deployment has no decision");

        let cause = Cause::Permission(refusal);

        let steps = cause.steps();
        assert_eq!(steps.len(), 1);
        assert!(
            steps[0].label().contains("row"),
            "the step has to name the missing decision: {}",
            steps[0].label(),
        );
    }

    #[test]
    fn the_same_failure_twice_is_one_notification_and_the_second_does_not_raise() {
        let cause = Cause::Process(ran("exit 4"));

        let centre = NotificationCentre::new();
        let first = centre.report(cause.clone());
        let second = centre.report(cause);

        assert!(first.first_time, "the caller raises something once");
        assert!(
            !second.first_time,
            "a repeat only adds to the count, so nothing prompts twice",
        );
        assert_eq!(first.id, second.id);
        let notification = centre.notification(first.id).expect("recorded");
        assert_eq!(notification.occurrences(), 2);
        assert_eq!(centre.counts().errors, 1, "still one problem");
    }

    #[test]
    fn the_same_failure_with_a_different_message_is_a_different_notification() {
        let centre = NotificationCentre::new();

        let first = centre.report(Cause::Process(ran("echo one >&2; exit 1")));
        let second = centre.report(Cause::Process(ran("echo two >&2; exit 1")));

        assert_ne!(first.id, second.id);
        assert!(second.first_time);
        assert_eq!(centre.notifications().len(), 2);
    }

    #[test]
    fn dismissing_clears_a_notification_from_the_list_and_a_recurrence_brings_it_back() {
        let cause = Cause::Process(ran("exit 5"));
        let centre = NotificationCentre::new();
        let reported = centre.report(cause.clone());
        assert_eq!(centre.open().len(), 1);

        assert!(centre.dismiss(reported.id));
        assert!(centre.open().is_empty());
        assert_eq!(centre.counts().errors, 1, "the record is kept");
        assert!(
            centre
                .notification(reported.id)
                .expect("kept")
                .is_dismissed()
        );
        assert!(
            !centre.dismiss(crate::host::notify::NotificationId(99)),
            "unknown ids are not invented"
        );

        let again = centre.report(cause);
        assert!(!again.first_time);
        assert_eq!(
            centre.open().len(),
            1,
            "a problem that comes back must not stay hidden because it was cleared once",
        );
        assert_eq!(centre.dismiss_all(), 1);
        assert_eq!(centre.dismiss_all(), 0);
    }

    #[test]
    fn every_cause_offers_a_next_step_and_an_identified_link() {
        let permit = command_permit();
        let missing = OwnedProcess::spawn(ProcessSpec::new("gone", "/nonexistent/x"), &permit)
            .expect_err("not there");
        let causes = vec![
            Cause::for_task(&finished_task("fails", || JobOutcome::failed("no")))
                .expect("finished"),
            Cause::Process(ran("exit 7")),
            Cause::Program(missing),
            Cause::Permission(untrusted_refusal()),
        ];

        for cause in &causes {
            let steps = cause.steps();
            assert!(!steps.is_empty(), "{} has no next step", cause.key());
            for step in &steps {
                assert!(
                    step.label().split_whitespace().count() >= 2,
                    "a step is a phrase, not a label: {}",
                    step.label(),
                );
            }
            assert!(!cause.summary().is_empty());
            assert!(!cause.key().is_empty());
            assert!(
                !cause.link().render().is_empty(),
                "{} has no link",
                cause.key(),
            );
            assert!(
                matches!(
                    cause.link(),
                    Link::Task { .. }
                        | Link::Process { .. }
                        | Link::Program { .. }
                        | Link::Permission { .. }
                ),
                "{}",
                cause.key(),
            );
        }

        // The keys have to tell the four apart, or de-duplication would merge unrelated problems.
        let keys: std::collections::BTreeSet<String> = causes.iter().map(Cause::key).collect();
        assert_eq!(keys.len(), causes.len());
    }

    #[test]
    fn the_centre_counts_by_severity_and_renders_what_it_holds() {
        let centre = NotificationCentre::new();
        centre
            .report(Cause::for_task(&finished_task("fails", || JobOutcome::failed("no"))).unwrap());
        centre.report(Cause::Process(ran("exit 2")));
        centre.report(Cause::for_task(&finished_task("works", || JobOutcome::Succeeded)).unwrap());
        centre.report(Cause::Permission(untrusted_refusal()));

        assert_eq!(
            centre.counts(),
            Counts {
                errors: 2,
                warnings: 1,
                information: 1,
            },
        );
        let rendered = centre.render_text();
        assert!(rendered.contains("[error]"), "{rendered}");
        assert!(rendered.contains("next:"), "{rendered}");
        assert!(
            rendered.contains("4 notification(s): 2 error, 1 warning, 1 information"),
            "{rendered}",
        );
        assert_eq!(centre.open().len(), 4);
    }

    #[test]
    fn an_empty_centre_says_so_rather_than_printing_nothing() {
        let centre = NotificationCentre::new();

        assert_eq!(centre.render_text(), "no notifications\n");
        assert!(centre.counts().is_empty());
        assert!(centre.notifications().is_empty());
        assert!(centre.open().is_empty());
    }

    #[test]
    fn the_clock_times_the_first_and_the_last_occurrence() {
        let clock = std::sync::Arc::new(FixedClock::new());
        struct SharedClock(std::sync::Arc<FixedClock>);
        impl crate::host::log::Clock for SharedClock {
            fn elapsed_millis(&self) -> u128 {
                self.0.elapsed_millis()
            }
        }
        let centre = NotificationCentre::with_clock(Box::new(SharedClock(clock.clone())));
        let cause = Cause::Process(ran("exit 6"));

        let reported = centre.report(cause.clone());
        clock.advance(Duration::from_millis(250));
        centre.report(cause);

        let notification = centre.notification(reported.id).expect("recorded");
        assert_eq!(notification.first_seen_millis(), 0);
        assert_eq!(notification.last_seen_millis(), 250);
    }

    #[test]
    fn retrying_work_that_may_have_side_effects_asks_first() {
        assert!(
            RecoveryAction::Retry { side_effects: true }.requires_confirmation(),
            "roadmap 3.3: a retry that can repeat a side effect asks first",
        );
        assert!(
            !RecoveryAction::Retry {
                side_effects: false
            }
            .requires_confirmation()
        );
        assert!(
            RecoveryAction::Retry {
                side_effects: false
            }
            .label()
            .contains("run it again")
        );
        assert!(
            RecoveryAction::Retry { side_effects: true }
                .label()
                .contains("asks first")
        );
    }

    #[test]
    fn a_task_refused_before_it_ran_is_reported_as_a_warning_not_an_error() {
        // The task never ran, so there is no failure to look for; a person has something to decide.
        let record = finished_task("never runs", || JobOutcome::Succeeded);

        assert_eq!(
            Cause::for_task(&record).expect("finished").severity(),
            Severity::Info,
        );
    }

    #[test]
    fn a_link_names_the_thing_it_points_at() {
        let record = finished_task("names itself", || JobOutcome::Succeeded);
        let link = Cause::Task(record.clone()).link();

        assert_eq!(
            link,
            Link::Task {
                id: record.id,
                owner: record.owner,
                label: "names itself".to_owned(),
            },
        );
        assert!(link.render().contains("task #"));
        assert!(link.render().contains("names itself"));
    }

    #[test]
    fn a_capability_is_named_in_a_permission_link_when_a_decision_existed() {
        let link = Cause::Permission(untrusted_refusal()).link();

        match link {
            Link::Permission { capability, scope } => {
                assert_eq!(capability, Some(Capability::RunCargoCheck));
                assert!(scope.starts_with("project "), "{scope}");
            }
            other => panic!("expected a permission link, got {other:?}"),
        }
    }
}
