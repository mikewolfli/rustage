// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Subprocesses this session owns, and taking them down again. Contract **HOST**.
//!
//! Roadmap T01.04 asks for subprocess start, logs, exit, targeted tree cleanup and a platform signal
//! policy; the acceptance line is **"only processes this session owns are cleaned up, and no external
//! process is killed"**. That line is why this module is built around one idea:
//!
//! > **A child is ours because we created its process group, not because it matches a name.**
//!
//! Every process started here is given a group of its own (`setpgid` through
//! `Command::process_group(0)`, so the child leads a group whose id is its own pid), and termination
//! signals **that group**. Nothing scans the process table, nothing matches on a program name, and
//! nothing can reach a process that was not started in a group this session created — no matter what
//! it is called or who its parent is. That is the difference between "the build's children are gone"
//! and "everything called `cargo` is gone", and the second one is a bug that kills a user's work.
//!
//! The three honesty rules that follow from the same place:
//!
//! * **A polite signal, then a deadline, then force.** `SIGTERM` to the group, a bounded grace period
//!   while the direct child is reaped, then `SIGKILL` to the group. Which of those happened is the
//!   [`Termination`] the caller gets; "it stopped" is not inferred from "we asked".
//! * **A process that was already gone is reported as gone**, not as terminated.
//! * **Where the platform cannot take down a tree, the report says so.** Off Unix there is no group
//!   signal here, so [`Termination::DirectOnly`] names what was achieved (the direct child) and what
//!   was not (its descendants). Claiming a tree cleanup that did not happen is exactly the placeholder
//!   success principle #22 rules out.
//!
//! **Output is captured, bounded and never dropped silently.** Both streams are read by their own
//! threads into a buffer capped by [`ProcessSpec::capture_limit`]; a stream that exceeds the cap is
//! truncated *and* the number of dropped bytes is reported, so a UI can say "output truncated" instead
//! of showing a prefix as if it were the whole log. A build that prints a hundred megabytes must not
//! be able to grow the host's memory with it.
//!
//! **Waiting for those readers is bounded too.** End-of-file comes when every copy of the pipe's
//! write end is gone, and a process *outside* this session can hold a copy: a child forked while
//! another thread is creating a pipe inherits its descriptors, the same race that makes `exec` fail
//! with `ETXTBSY`. Waiting for such a copy is waiting for an unrelated process to end — once, that
//! turned "the polite signal stopped it in 5ms" into "it stopped after 30s" and held the caller for
//! the same 30 seconds. [`DRAIN_DEADLINE`] bounds the wait, a shortfall is logged, and the time a
//! termination reports is measured on the process rather than on the host's own bookkeeping.
//!
//! **Starting a process requires a permit.** [`OwnedProcess::spawn`] takes an
//! [`super::permission::ExecutionPermit`], which only the permission service can mint and only after
//! a decision, so an untrusted project cannot run code by forgetting to check.
//!
//! **What this module is not.** It is not the build service (T16.x), not a PTY, not a terminal
//! emulator and not a scheduler: it starts one process, reports what it did and takes its tree down.
//! Attaching a subprocess to a task, a log panel and a project scope is T01.03's executor, T01.06's
//! notification centre and T01.07's registry, each of which consumes this rather than replacing it.

use std::collections::VecDeque;
use std::fmt;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::log::{Level, emit};
use super::permission::ExecutionPermit;
use crate::contracts::permission::EffectClass;

/// How long a terminated process is given to stop on its own before it is killed.
pub const DEFAULT_GRACE: Duration = Duration::from_secs(3);

/// How long the readers of a process's streams are given to reach end-of-file after it ends.
///
/// End-of-file arrives as soon as every copy of the pipe's write end is gone, and the only copy the
/// host can account for is the process it started. A *different* process can inherit a copy when a
/// child is forked while another thread is creating a pipe — the same race that makes `exec` fail
/// with `ETXTBSY` — and that copy is closed when that process ends, which may be much later or never.
/// Waiting for it unbounded turned a "the polite signal stopped it in 5ms" report into "it stopped
/// after 30s" and blocked the caller for the same 30s, so the wait is bounded and the shortfall is
/// logged. Everything the process itself wrote is readable well within this bound: it is already in
/// the kernel's buffer when the process ends.
pub const DRAIN_DEADLINE: Duration = Duration::from_secs(1);

/// How much of each stream is kept when the caller does not say.
pub const DEFAULT_CAPTURE_LIMIT: usize = 256 * 1024;

/// The environment variable every owned process is started with.
///
/// It marks the child as this session's work. It is **evidence for a person reading a process list or
/// a report** — a diagnostic that answers "who started this?" — and deliberately *not* the criterion
/// for termination: a process that merely carries the variable was not created in a group this session
/// owns, and killing it would be exactly the mistake this module exists to avoid.
pub const SESSION_ENVIRONMENT: &str = "RUSTAGE_SESSION";

/// A process this session started.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ProcessId(u64);

impl ProcessId {
    /// The number, for a report or a test assertion.
    pub const fn value(self) -> u64 {
        self.0
    }
}

impl fmt::Display for ProcessId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "process#{}", self.0)
    }
}

/// What a caller wants started.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ProcessSpec {
    /// What this process is, in the caller's words. Used in reports and log lines.
    pub label: String,
    /// The program to start.
    pub program: PathBuf,
    /// Its arguments, each passed as its own argument so no shell parses them.
    pub arguments: Vec<String>,
    /// The working directory, when it matters.
    pub working_directory: Option<PathBuf>,
    /// Extra environment variables.
    pub environment: Vec<(String, String)>,
    /// How many bytes of each stream are kept.
    pub capture_limit: usize,
}

impl ProcessSpec {
    /// A spec for a program with its arguments.
    pub fn new(label: impl Into<String>, program: impl Into<PathBuf>) -> Self {
        Self {
            label: label.into(),
            program: program.into(),
            arguments: Vec::new(),
            working_directory: None,
            environment: Vec::new(),
            capture_limit: DEFAULT_CAPTURE_LIMIT,
        }
    }

    /// Adds one argument.
    pub fn arg(mut self, argument: impl Into<String>) -> Self {
        self.arguments.push(argument.into());
        self
    }

    /// Adds several arguments.
    pub fn args<I, S>(mut self, arguments: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.arguments.extend(arguments.into_iter().map(Into::into));
        self
    }

    /// Sets the working directory.
    pub fn in_directory(mut self, directory: impl Into<PathBuf>) -> Self {
        self.working_directory = Some(directory.into());
        self
    }

    /// Adds one environment variable.
    pub fn env(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.environment.push((name.into(), value.into()));
        self
    }

    /// Sets how much of each stream is kept.
    pub fn capture_limit(mut self, bytes: usize) -> Self {
        self.capture_limit = bytes;
        self
    }
}

/// Why a process could not be started.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SpawnError {
    /// Nothing to start.
    NoProgram,
    /// The program is not there (or not executable) — the OS's own reason is kept.
    Unavailable {
        /// The program the host tried to start.
        program: PathBuf,
        /// The OS's own reason.
        reason: String,
    },
    /// The working directory does not exist.
    WorkingDirectory {
        /// The directory the caller named.
        directory: PathBuf,
        /// The OS's own reason.
        reason: String,
    },
    /// The permit presented is for a different kind of effect than starting a program.
    PermitForAnotherEffect {
        /// The effect the permit was actually minted for.
        effect: EffectClass,
    },
}

impl SpawnError {
    /// A sentence naming what was found.
    pub fn message(&self) -> String {
        match self {
            Self::NoProgram => "no program was named, so there is nothing to start".to_owned(),
            Self::Unavailable { program, reason } => {
                format!("{} could not be started: {reason}", program.display())
            }
            Self::WorkingDirectory { directory, reason } => format!(
                "{} could not be used as the working directory: {reason}",
                directory.display(),
            ),
            Self::PermitForAnotherEffect { effect } => format!(
                "the permit is for {}, and starting a program is a command",
                effect.name(),
            ),
        }
    }
}

impl fmt::Display for SpawnError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message())
    }
}

impl std::error::Error for SpawnError {}

/// How a process ended.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ExitInfo {
    /// The exit code, when the platform reported one. A process killed by a signal has none.
    pub code: Option<i32>,
    /// The signal that killed it, when one did.
    pub signal: Option<i32>,
}

impl ExitInfo {
    /// Whether the process exited successfully (code zero, no signal).
    pub fn is_success(&self) -> bool {
        self.code == Some(0) && self.signal.is_none()
    }

    /// A sentence for a log line.
    pub fn message(&self) -> String {
        match (self.code, self.signal) {
            (Some(code), _) => format!("exit code {code}"),
            (None, Some(signal)) => format!("killed by signal {signal}"),
            (None, None) => "ended without an exit code".to_owned(),
        }
    }

    fn from_status(status: &ExitStatus) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            Self {
                code: status.code(),
                signal: status.signal(),
            }
        }
        #[cfg(not(unix))]
        {
            Self {
                code: status.code(),
                signal: None,
            }
        }
    }
}

/// What a process wrote.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct CapturedOutput {
    /// The standard output kept.
    pub stdout: String,
    /// The standard error kept.
    pub stderr: String,
    /// How many bytes were dropped because the stream exceeded the capture limit.
    ///
    /// A number rather than a flag: "truncated" without a size cannot tell a caller whether a little
    /// or almost everything is missing, and a UI that says "output truncated" while showing a prefix
    /// as the log is a failure shown as a success.
    pub dropped_bytes: usize,
}

impl CapturedOutput {
    /// Whether anything was dropped.
    pub fn is_truncated(&self) -> bool {
        self.dropped_bytes > 0
    }
}

/// What happened when the host asked a process to stop.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Termination {
    /// The process had already exited; nothing was signalled.
    AlreadyExited {
        /// How it had ended.
        exit: ExitInfo,
    },
    /// It stopped after the polite signal.
    Stopped {
        /// How it ended.
        exit: ExitInfo,
        /// How long it took to stop.
        after: Duration,
    },
    /// It ignored the polite signal and was killed.
    Killed {
        /// How it ended (normally "killed by signal 9").
        exit: ExitInfo,
        /// How long the whole termination took.
        after: Duration,
    },
    /// Only the direct child could be terminated; descendants may still be running. Reported with
    /// the reason, because "we cleaned up" would otherwise be a claim the platform cannot support.
    DirectOnly {
        /// How the direct child ended.
        exit: ExitInfo,
        /// Why the tree could not be reached.
        reason: String,
    },
    /// The signal could not be delivered at all.
    SignalFailed {
        /// The OS's own reason.
        reason: String,
    },
}

impl Termination {
    /// Whether the process is known to have ended.
    pub fn ended(&self) -> bool {
        !matches!(self, Self::SignalFailed { .. })
    }

    /// A sentence for a log line or a report.
    pub fn message(&self) -> String {
        match self {
            Self::AlreadyExited { exit } => {
                format!("had already ended ({})", exit.message())
            }
            Self::Stopped { exit, after } => format!(
                "stopped after {after:?}; the polite signal ended it ({})",
                exit.message(),
            ),
            Self::Killed { exit, after } => {
                format!(
                    "did not stop within its grace period and was killed after {after:?} ({})",
                    exit.message()
                )
            }
            Self::DirectOnly { exit, reason } => format!(
                "the direct process ended ({}) but its descendants were not reached: {reason}",
                exit.message(),
            ),
            Self::SignalFailed { reason } => format!("could not be signalled: {reason}"),
        }
    }
}

/// Whether a pid is running, without signalling it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProcessPresence {
    /// The process exists.
    Running,
    /// No such process.
    Gone,
    /// This platform cannot answer without a rights check this host does not make.
    Unverifiable,
}

/// Whether a process exists.
///
/// Asked with a zero signal, which performs the permission check without delivering anything. Used by
/// reports and tests to answer "is it really gone?" instead of inferring it from a signal that was
/// sent.
#[cfg(unix)]
pub fn presence(pid: u32) -> ProcessPresence {
    // SAFETY: `kill` with signal 0 delivers nothing; it only asks the kernel whether the process
    // exists and may be signalled by us.
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if result == 0 {
        return ProcessPresence::Running;
    }
    match std::io::Error::last_os_error().raw_os_error() {
        Some(libc::ESRCH) => ProcessPresence::Gone,
        // EPERM means it exists and belongs to somebody else — which is still "a process is there",
        // and calling it gone would be the mistake this whole module is about.
        Some(libc::EPERM) => ProcessPresence::Running,
        _ => ProcessPresence::Unverifiable,
    }
}

/// Whether a process exists.
///
/// Without `kill(2)` there is no rights check to make a liveness question answerable, so this host
/// says it cannot verify rather than guessing "gone".
#[cfg(not(unix))]
pub fn presence(pid: u32) -> ProcessPresence {
    let _ = pid;
    ProcessPresence::Unverifiable
}

/// A process this session started, with the group that makes it ours.
pub struct OwnedProcess {
    id: ProcessId,
    label: String,
    child: Child,
    pid: u32,
    /// The process group this session created for the child (Unix). `None` where the platform has no
    /// such notion, which is what makes [`Termination::DirectOnly`] the honest answer there.
    group: Option<i32>,
    started: Instant,
    captured: Arc<Mutex<Capture>>,
    readers: Vec<JoinHandle<()>>,
    /// One message per reader that reached end-of-file, so waiting for them can be bounded.
    drained: Receiver<()>,
}

impl OwnedProcess {
    /// Starts a process, capturing both of its streams.
    ///
    /// The child leads a process group of its own, so terminating it cannot reach anything this
    /// session did not start. It also cannot start without `permit`: the host's permission service
    /// mints one only after a decision, and an untrusted project never gets one, so "an untrusted
    /// project runs no code" is a property of the type rather than of a caller's care.
    pub fn spawn(spec: ProcessSpec, permit: &ExecutionPermit) -> Result<Self, SpawnError> {
        if permit.effect() != EffectClass::Command {
            return Err(SpawnError::PermitForAnotherEffect {
                effect: permit.effect(),
            });
        }
        if spec.program.as_os_str().is_empty() {
            return Err(SpawnError::NoProgram);
        }
        if let Some(directory) = &spec.working_directory
            && !directory.is_dir()
        {
            return Err(SpawnError::WorkingDirectory {
                directory: directory.clone(),
                reason: "it is not an existing directory".to_owned(),
            });
        }

        let mut command = Command::new(&spec.program);
        command.args(&spec.arguments);
        if let Some(directory) = &spec.working_directory {
            command.current_dir(directory);
        }
        for (name, value) in &spec.environment {
            command.env(name, value);
        }
        let session = session_marker();
        command.env(SESSION_ENVIRONMENT, &session);
        command.stdin(Stdio::null());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        command_group(&mut command);

        let mut child = command.spawn().map_err(|error| SpawnError::Unavailable {
            program: spec.program.clone(),
            reason: error.to_string(),
        })?;
        let pid = child.id();
        let captured = Arc::new(Mutex::new(Capture::new(spec.capture_limit)));
        let (drained_tx, drained) = mpsc::channel();
        let mut readers = Vec::new();
        if let Some(stdout) = child.stdout.take() {
            readers.push(read_into(
                captured.clone(),
                Stream::Stdout,
                stdout,
                drained_tx.clone(),
            ));
        }
        if let Some(stderr) = child.stderr.take() {
            readers.push(read_into(
                captured.clone(),
                Stream::Stderr,
                stderr,
                drained_tx.clone(),
            ));
        }
        drop(drained_tx);

        Ok(Self {
            id: ProcessId(NEXT_PROCESS.fetch_add(1, Ordering::Relaxed)),
            label: spec.label,
            child,
            pid,
            group: group_of(pid),
            started: Instant::now(),
            captured,
            readers,
            drained,
        })
    }

    /// The id this session gave the process.
    pub fn id(&self) -> ProcessId {
        self.id
    }

    /// What the caller called it.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// The operating system's process id.
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// The process group this session created for it, where the platform has one.
    pub fn group(&self) -> Option<i32> {
        self.group
    }

    /// How long it has been running.
    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// Whether it is still running, without signalling it.
    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// What it has written so far.
    ///
    /// A snapshot, so a caller can show progress while the process runs; [`Self::outcome`] gives the
    /// final text after it has ended.
    pub fn output_so_far(&self) -> CapturedOutput {
        capture(&self.captured).snapshot()
    }

    /// Waits for it to end on its own and returns how it ended.
    pub fn wait(&mut self) -> ExitInfo {
        let exit = self.reap();
        self.drain_readers();
        exit
    }

    /// Waits for the direct child to end, without draining its streams.
    ///
    /// Split out so a caller that reports how long something took can measure the process and not
    /// its own bookkeeping; see [`DRAIN_DEADLINE`].
    fn reap(&mut self) -> ExitInfo {
        match self.child.wait() {
            Ok(status) => ExitInfo::from_status(&status),
            Err(error) => ExitInfo {
                code: None,
                signal: None,
            }
            .with_note(error.to_string()),
        }
    }

    /// Waits up to `limit` for it to end, returning `None` if it is still running.
    pub fn wait_for(&mut self, limit: Duration) -> Option<ExitInfo> {
        let deadline = Instant::now() + limit;
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => {
                    self.drain_readers();
                    return Some(ExitInfo::from_status(&status));
                }
                Ok(None) => {
                    if Instant::now() >= deadline {
                        return None;
                    }
                    thread::sleep(Duration::from_millis(5));
                }
                Err(_) => return None,
            }
        }
    }

    /// Asks it to stop: a polite signal to its group, then a deadline, then force.
    ///
    /// The caller gets which of those happened, so "we asked" and "it stopped" never merge.
    pub fn terminate(&mut self, grace: Duration) -> Termination {
        if let Ok(Some(status)) = self.child.try_wait() {
            self.drain_readers();
            return Termination::AlreadyExited {
                exit: ExitInfo::from_status(&status),
            };
        }

        let began = Instant::now();
        #[cfg(unix)]
        {
            let Some(group) = self.group else {
                return Termination::DirectOnly {
                    exit: self.kill_directly(),
                    reason: "this process has no group of its own".to_owned(),
                };
            };

            match signal_group(group, libc::SIGTERM) {
                Ok(()) => {}
                Err(error) if error.raw_os_error() == Some(libc::ESRCH) => {
                    // The group is already gone; reaping the direct child is all that is left.
                    let exit = self.wait();
                    return Termination::AlreadyExited { exit };
                }
                Err(error) => {
                    return Termination::SignalFailed {
                        reason: error.to_string(),
                    };
                }
            }

            let deadline = Instant::now() + grace;
            loop {
                match self.child.try_wait() {
                    Ok(Some(status)) => {
                        // Measured before the streams are drained: "how long did the polite signal
                        // take" is a fact about the process, and draining is the host's own work
                        // (which can be delayed by a pipe another process inherited).
                        let after = began.elapsed();
                        self.drain_readers();
                        return Termination::Stopped {
                            exit: ExitInfo::from_status(&status),
                            after,
                        };
                    }
                    Ok(None) if Instant::now() < deadline => {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Ok(None) => break,
                    Err(error) => {
                        return Termination::SignalFailed {
                            reason: format!("the process's status could not be read: {error}"),
                        };
                    }
                }
            }

            if let Err(error) = signal_group(group, libc::SIGKILL)
                && error.raw_os_error() != Some(libc::ESRCH)
            {
                return Termination::SignalFailed {
                    reason: error.to_string(),
                };
            }
            let exit = self.reap();
            // The kill-to-reaped time, before the streams are drained: the same reason the polite
            // path measures before draining.
            let after = began.elapsed();
            self.drain_readers();
            Termination::Killed { exit, after }
        }
        #[cfg(not(unix))]
        {
            let _ = began;
            let exit = self.kill_directly();
            Termination::DirectOnly {
                exit,
                reason: format!(
                    "this host knows no way to signal a process group on {}; the direct process was \
                     killed and its descendants were not",
                    std::env::consts::OS,
                ),
            }
        }
    }

    /// A sentence describing why terminating this process cannot reach anything else.
    ///
    /// This is the evidence for T01.04's acceptance line, and it is a value rather than a comment so a
    /// report can carry it.
    pub fn ownership_evidence(&self) -> String {
        match self.group {
            Some(group) => format!(
                "{} leads process group {group}, created by this session; termination signals the \
                 group, so only processes started in it can be reached (marker {SESSION_ENVIRONMENT}=\
                 {})",
                self.pid,
                session_marker(),
            ),
            None => format!(
                "{} has no process group on this platform; only the direct process can be terminated \
                 (marker {SESSION_ENVIRONMENT}={})",
                self.pid,
                session_marker(),
            ),
        }
    }

    /// The final state: exit, captured output and how long it ran.
    pub fn outcome(&self, exit: ExitInfo) -> ProcessOutcome {
        ProcessOutcome {
            id: self.id,
            label: self.label.clone(),
            pid: self.pid,
            exit,
            output: capture(&self.captured).snapshot(),
            run_for: self.started.elapsed(),
        }
    }

    /// Waits for the readers to reach end-of-file, for a bounded time.
    ///
    /// Bounded because end-of-file is not in this host's control: a descriptor inherited by an
    /// unrelated process keeps the pipe open, and joining without a limit would hang the caller for
    /// as long as that process lives (see [`DRAIN_DEADLINE`]).
    fn drain_readers(&mut self) {
        let expected = self.readers.len();
        if expected == 0 {
            return;
        }
        let finished = wait_for_readers(&self.drained, expected, Instant::now() + DRAIN_DEADLINE);
        if finished < expected {
            emit(
                Level::Warn,
                "host::subprocess",
                &format!(
                    "{} of {expected} stream readers for {} did not reach end-of-file within \
                     {DRAIN_DEADLINE:?} of the process ending; they are left to finish on their own \
                     rather than blocking the caller",
                    expected - finished,
                    self.label,
                ),
            );
            // Dropping a handle detaches the thread, so the caller is not held by a pipe that a
            // process outside this session inherited.
            self.readers.clear();
            return;
        }
        for reader in self.readers.drain(..) {
            let _ = reader.join();
        }
    }

    fn kill_directly(&mut self) -> ExitInfo {
        let _ = self.child.kill();
        self.wait()
    }
}

impl fmt::Debug for OwnedProcess {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OwnedProcess")
            .field("id", &self.id)
            .field("label", &self.label)
            .field("pid", &self.pid)
            .field("group", &self.group)
            .field("elapsed", &self.elapsed())
            .finish_non_exhaustive()
    }
}

impl Drop for OwnedProcess {
    fn drop(&mut self) {
        // Dropping does not kill: a caller that stops caring about a process has not asked for it to
        // die, and a silent kill on drop would make "the process is still running" impossible to
        // report. The reader threads are collected so they cannot outlive the process handle.
        self.drain_readers();
    }
}

/// What a finished process leaves behind.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ProcessOutcome {
    /// The session's id for it.
    pub id: ProcessId,
    /// What the caller called it.
    pub label: String,
    /// The operating system's pid.
    pub pid: u32,
    /// How it ended.
    pub exit: ExitInfo,
    /// What it wrote, bounded by the capture limit.
    pub output: CapturedOutput,
    /// How long it ran.
    pub run_for: Duration,
}

impl ProcessOutcome {
    /// One line: what it was, how it ended, and whether the log is complete.
    pub fn summary(&self) -> String {
        let truncation = if self.output.is_truncated() {
            format!(", {} bytes of output dropped", self.output.dropped_bytes)
        } else {
            String::new()
        };
        format!(
            "{} ({}) {} after {:?}{truncation}",
            self.label,
            self.pid,
            self.exit.message(),
            self.run_for,
        )
    }
}

/// One stream of captured output.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stream {
    Stdout,
    Stderr,
}

/// The bounded buffers behind the capture.
struct Capture {
    limit: usize,
    stdout: VecDeque<u8>,
    stderr: VecDeque<u8>,
    dropped: usize,
}

impl Capture {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            stdout: VecDeque::new(),
            stderr: VecDeque::new(),
            dropped: 0,
        }
    }

    fn push(&mut self, stream: Stream, bytes: &[u8]) {
        let (buffer, room) = match stream {
            Stream::Stdout => {
                let room = self.limit.saturating_sub(self.stdout.len());
                (&mut self.stdout, room)
            }
            Stream::Stderr => {
                let room = self.limit.saturating_sub(self.stderr.len());
                (&mut self.stderr, room)
            }
        };
        let keep = bytes.len().min(room);
        buffer.extend(&bytes[..keep]);
        self.dropped += bytes.len() - keep;
    }

    fn snapshot(&self) -> CapturedOutput {
        CapturedOutput {
            stdout: String::from_utf8_lossy(&self.stdout.iter().copied().collect::<Vec<u8>>())
                .into_owned(),
            stderr: String::from_utf8_lossy(&self.stderr.iter().copied().collect::<Vec<u8>>())
                .into_owned(),
            dropped_bytes: self.dropped,
        }
    }
}

/// Locks the capture without letting a poisoned mutex become a second panic.
fn capture(captured: &Arc<Mutex<Capture>>) -> std::sync::MutexGuard<'_, Capture> {
    captured
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Reads one stream into the capture until it ends.
fn read_into<R: Read + Send + 'static>(
    captured: Arc<Mutex<Capture>>,
    stream: Stream,
    mut source: R,
    drained: mpsc::Sender<()>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut buffer = [0u8; 4096];
        loop {
            match source.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => capture(&captured).push(stream, &buffer[..read]),
                Err(_) => break,
            }
        }
        // Sent whether the stream ended or errored: the host's question is "has this reader
        // stopped reading", and a reader that gave up has.
        let _ = drained.send(());
    })
}

/// Waits for `expected` readers to report that they stopped, until `deadline`.
///
/// Returns how many reported. A caller compares that with `expected` rather than assuming: the
/// whole reason this is bounded is that the last reader may never report.
fn wait_for_readers(drained: &Receiver<()>, expected: usize, deadline: Instant) -> usize {
    let mut finished = 0;
    while finished < expected {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            break;
        };
        match drained.recv_timeout(remaining) {
            Ok(()) => finished += 1,
            // Out of time, or every sender is gone without having reported — a reader that panicked
            // is the only way the second one happens. Neither arrives later, so stop waiting.
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => break,
        }
    }
    finished
}

/// Gives the child a process group of its own, where the platform has the notion.
fn command_group(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // `0` means "put the child in a new group whose id is its own pid".
        command.process_group(0);
    }
    #[cfg(not(unix))]
    {
        let _ = command;
    }
}

/// The group id of a process this host started.
///
/// On Unix it is the child's own pid, because [`command_group`] made the child its group's leader.
fn group_of(pid: u32) -> Option<i32> {
    #[cfg(unix)]
    {
        Some(pid as i32)
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        None
    }
}

/// Signals a whole process group.
#[cfg(unix)]
fn signal_group(group: i32, signal: i32) -> std::io::Result<()> {
    // SAFETY: `kill` takes a pid and a signal. A negative pid means "this process group", and the
    // group named here is one this session created for a child it started — never a group discovered
    // by scanning, and never a pid read from a file.
    let result = unsafe { libc::kill(-group, signal) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// The marker this session writes into every process it starts.
///
/// Derived from the process id, so two hosts on one machine cannot be confused for each other when a
/// person looks at a process list.
pub fn session_marker() -> String {
    format!("rustage-{}", std::process::id())
}

/// Adds a note to an exit the platform could not describe.
impl ExitInfo {
    fn with_note(self, note: String) -> Self {
        crate::host::log::emit(
            crate::host::log::Level::Warn,
            "host::subprocess",
            &format!("a process's exit status could not be read: {note}"),
        );
        self
    }
}

static NEXT_PROCESS: AtomicU64 = AtomicU64::new(1);

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_GRACE, ExitInfo, OwnedProcess, ProcessPresence, ProcessSpec, SESSION_ENVIRONMENT,
        SpawnError, Termination, presence, session_marker,
    };
    use crate::contracts::permission::{EffectClass, Principal};
    use crate::host::permission::{ExecutionPermit, PermissionService, Scope};
    use crate::host::tasks::{
        CancelResult, Executor, ExecutorConfig, JobControl, JobOutcome, OwnerId, SubmitRequest,
        TaskOutcome,
    };
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    /// A shell running `script`, which is enough to make a process that sleeps, fails, signals or
    /// spawns a child without depending on any tool being installed.
    fn shell(label: &str, script: &str) -> ProcessSpec {
        ProcessSpec::new(label, "sh").args(["-c", script])
    }

    /// The permit every start in these tests goes through.
    ///
    /// Not a bypass: it is the same decision a caller gets, for the session scope, and a test that
    /// needed a different decision would fail here rather than quietly start a process.
    fn command_permit() -> ExecutionPermit {
        PermissionService::new()
            .permit(EffectClass::Command, Scope::Session, Principal::Human)
            .expect("the host's own session may run a command")
    }

    #[test]
    fn a_permit_for_another_effect_cannot_start_a_program() {
        let permit = PermissionService::new()
            .permit(EffectClass::FileWrite, Scope::Session, Principal::Human)
            .expect("a human may write a file");

        let error = OwnedProcess::spawn(shell("wrong-permit", "exit 0"), &permit)
            .expect_err("a file-write permit does not authorize a command");

        assert_eq!(
            error,
            SpawnError::PermitForAnotherEffect {
                effect: EffectClass::FileWrite
            },
        );
        assert!(error.message().contains("file-write"));
    }

    /// Waits for a pid to disappear, returning what was last seen.
    fn wait_until_gone(pid: u32, limit: Duration) -> ProcessPresence {
        let deadline = Instant::now() + limit;
        loop {
            let seen = presence(pid);
            if seen != ProcessPresence::Running || Instant::now() >= deadline {
                return seen;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn waiting_for_readers_stops_at_its_deadline() {
        let (tx, rx) = mpsc::channel::<()>();
        let slow = tx.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(2));
            let _ = slow.send(());
        });
        tx.send(()).expect("the receiver is alive");

        let began = Instant::now();
        let finished = super::wait_for_readers(&rx, 2, began + Duration::from_millis(50));
        let took = began.elapsed();

        assert_eq!(
            finished, 1,
            "the reader that is still blocked must not be counted",
        );
        assert!(
            took < Duration::from_secs(1),
            "the wait has to end at its deadline, not when the reader does: {took:?}",
        );
    }

    #[test]
    fn waiting_for_readers_counts_them_all_when_they_finish() {
        let (tx, rx) = mpsc::channel::<()>();
        for _ in 0..2 {
            let sender = tx.clone();
            std::thread::spawn(move || {
                let _ = sender.send(());
            });
        }
        drop(tx);

        let finished = super::wait_for_readers(&rx, 2, Instant::now() + Duration::from_secs(5));

        assert_eq!(finished, 2);
    }

    #[test]
    fn a_reader_that_never_reports_is_not_counted_as_finished() {
        // Every sender gone without a message is what a panicking reader looks like. The wait has to
        // end then too, rather than sitting out the whole deadline.
        let (tx, rx) = mpsc::channel::<()>();
        drop(tx);

        let began = Instant::now();
        let finished = super::wait_for_readers(&rx, 2, began + Duration::from_secs(5));

        assert_eq!(finished, 0);
        assert!(began.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn a_finished_process_reports_its_exit_code_and_both_streams() {
        let mut process = OwnedProcess::spawn(
            shell("prints", "echo out; echo err >&2; exit 5"),
            &command_permit(),
        )
        .expect("the shell starts");

        let exit = process.wait();
        let outcome = process.outcome(exit);

        assert_eq!(outcome.exit.code, Some(5));
        assert!(!outcome.exit.is_success());
        assert_eq!(outcome.output.stdout, "out\n");
        assert_eq!(outcome.output.stderr, "err\n");
        assert!(!outcome.output.is_truncated());
        assert!(
            outcome.summary().contains("exit code 5"),
            "{}",
            outcome.summary()
        );
    }

    #[test]
    fn a_process_that_has_already_ended_is_reported_as_gone_not_terminated() {
        let mut process = OwnedProcess::spawn(shell("quick", "exit 0"), &command_permit())
            .expect("the shell starts");
        let _ = process.wait();

        let termination = process.terminate(Duration::from_secs(1));

        assert!(
            matches!(termination, Termination::AlreadyExited { .. }),
            "{termination:?}",
        );
        assert!(termination.ended());
    }

    #[cfg(unix)]
    #[test]
    fn a_cooperative_process_stops_after_the_polite_signal() {
        let mut process = OwnedProcess::spawn(shell("waits", "sleep 30"), &command_permit())
            .expect("the shell starts");

        let termination = process.terminate(Duration::from_secs(5));

        match termination {
            Termination::Stopped { exit, after } => {
                assert_eq!(exit.signal, Some(libc::SIGTERM));
                assert!(after < Duration::from_secs(5), "{after:?}");
            }
            other => panic!("expected Stopped, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_process_that_ignores_the_polite_signal_is_killed_and_says_so() {
        // The trap makes the shell ignore SIGTERM, and the loop keeps it alive so the deadline has
        // something to bite: only the forced signal can end it. The shell announces the moment the
        // trap is installed, because a `SIGTERM` that arrives *before* a program has installed its
        // handler kills it — which is correct behaviour, not something to assert about here, and
        // racing it made this test flaky at first.
        let mut process = OwnedProcess::spawn(
            shell(
                "stubborn",
                "trap '' TERM; echo ready; while true; do sleep 0.2; done",
            ),
            &command_permit(),
        )
        .expect("starts");
        let deadline = Instant::now() + Duration::from_secs(5);
        while !process.output_so_far().stdout.contains("ready") {
            assert!(
                Instant::now() < deadline,
                "the shell never reached the point where it ignores SIGTERM",
            );
            std::thread::sleep(Duration::from_millis(10));
        }

        let termination = process.terminate(Duration::from_millis(200));

        match termination {
            Termination::Killed { exit, after } => {
                assert_eq!(exit.signal, Some(libc::SIGKILL));
                assert!(after >= Duration::from_millis(200), "{after:?}");
            }
            other => panic!("expected Killed, got {other:?}"),
        }
        assert_eq!(
            wait_until_gone(process.pid(), Duration::from_secs(2)),
            ProcessPresence::Gone
        );
    }

    #[test]
    fn the_child_can_see_which_session_started_it() {
        let mut process = OwnedProcess::spawn(
            shell("marker", &format!("echo ${SESSION_ENVIRONMENT}")),
            &command_permit(),
        )
        .expect("the shell starts");

        let exit = process.wait();
        let outcome = process.outcome(exit);

        assert_eq!(outcome.output.stdout.trim(), session_marker());
        assert!(process.ownership_evidence().contains(SESSION_ENVIRONMENT));
    }

    #[cfg(unix)]
    #[test]
    fn terminating_one_process_leaves_a_process_outside_the_group_alone() {
        // The acceptance line, tested the only way it can be: an unrelated process is started
        // *without* this service, so it is not in the group the service created, and it has to
        // survive. Matching on a name would fail this test, which is the point.
        let mut foreign = std::process::Command::new("sleep")
            .arg("30")
            .stdin(std::process::Stdio::null())
            .spawn()
            .expect("the unrelated process starts");
        let foreign_pid = foreign.id();

        let mut owned =
            OwnedProcess::spawn(shell("ours", "sleep 30"), &command_permit()).expect("ours starts");
        let owned_pid = owned.pid();
        let termination = owned.terminate(Duration::from_secs(5));

        assert!(termination.ended(), "{termination:?}");
        assert_eq!(
            wait_until_gone(owned_pid, Duration::from_secs(2)),
            ProcessPresence::Gone,
        );
        assert_eq!(
            presence(foreign_pid),
            ProcessPresence::Running,
            "an unrelated process was killed by our cleanup",
        );

        let _ = foreign.kill();
        let _ = foreign.wait();
    }

    #[cfg(unix)]
    #[test]
    fn a_descendant_inside_the_group_is_taken_down_with_its_parent() {
        // A build script's children are the reason cleanup targets a group: the direct child is not
        // the process that keeps a compiler running.
        let mut process =
            OwnedProcess::spawn(shell("tree", "sleep 30 & echo $!; wait"), &command_permit())
                .expect("the shell starts");

        let grandchild = {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let seen = process.output_so_far().stdout;
                if let Ok(pid) = seen.trim().parse::<u32>() {
                    break pid;
                }
                assert!(
                    Instant::now() < deadline,
                    "the shell never reported its child"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        };
        assert_eq!(presence(grandchild), ProcessPresence::Running);

        let termination = process.terminate(Duration::from_secs(5));

        assert!(termination.ended(), "{termination:?}");
        assert_eq!(
            wait_until_gone(grandchild, Duration::from_secs(5)),
            ProcessPresence::Gone,
            "the descendant outlived the cleanup",
        );
    }

    #[test]
    fn a_program_that_is_not_there_is_reported_rather_than_panicking() {
        let error = OwnedProcess::spawn(
            ProcessSpec::new("missing", "/nonexistent/rustage-tool"),
            &command_permit(),
        )
        .expect_err("the program is not there");

        match &error {
            SpawnError::Unavailable { program, reason } => {
                assert_eq!(program.to_string_lossy(), "/nonexistent/rustage-tool");
                assert!(!reason.is_empty());
            }
            other => panic!("expected Unavailable, got {other:?}"),
        }
        assert!(error.message().contains("could not be started"));
    }

    #[test]
    fn a_working_directory_that_is_not_there_is_refused_before_anything_starts() {
        let error = OwnedProcess::spawn(
            shell("nowhere", "exit 0").in_directory("/nonexistent/rustage-directory"),
            &command_permit(),
        )
        .expect_err("the directory is not there");

        assert!(
            matches!(error, SpawnError::WorkingDirectory { .. }),
            "{error:?}"
        );
        assert_eq!(
            OwnedProcess::spawn(ProcessSpec::new("empty", ""), &command_permit())
                .expect_err("no program"),
            SpawnError::NoProgram,
        );
    }

    #[test]
    fn output_beyond_the_capture_limit_is_dropped_and_the_amount_is_reported() {
        let mut process = OwnedProcess::spawn(
            shell("noisy", "head -c 4000 /dev/zero | tr '\\0' 'x'").capture_limit(64),
            &command_permit(),
        )
        .expect("the shell starts");

        let exit = process.wait();
        let outcome = process.outcome(exit);

        assert!(
            outcome.output.stdout.len() <= 64,
            "{}",
            outcome.output.stdout.len()
        );
        assert!(outcome.output.is_truncated());
        assert!(
            outcome.output.dropped_bytes > 3000,
            "{}",
            outcome.output.dropped_bytes
        );
        assert!(
            outcome.summary().contains("dropped"),
            "{}",
            outcome.summary()
        );
    }

    #[test]
    fn a_process_still_running_reports_no_exit_within_the_limit() {
        let mut process = OwnedProcess::spawn(shell("slow", "sleep 30"), &command_permit())
            .expect("the shell starts");

        assert!(process.wait_for(Duration::from_millis(50)).is_none());
        assert!(process.is_running());
        assert!(process.elapsed() >= Duration::from_millis(50));

        let termination = process.terminate(Duration::from_secs(5));
        assert!(termination.ended(), "{termination:?}");
    }

    #[test]
    fn two_processes_get_different_ids_and_groups_of_their_own() {
        let first =
            OwnedProcess::spawn(shell("first", "sleep 1"), &command_permit()).expect("starts");
        let second =
            OwnedProcess::spawn(shell("second", "sleep 1"), &command_permit()).expect("starts");

        assert_ne!(first.id(), second.id());
        assert_eq!(
            first.pid(),
            first.group().expect("unix gives a group") as u32
        );
        assert_ne!(first.group(), second.group());

        drop(second);
        drop(first);
    }

    #[cfg(unix)]
    #[test]
    fn cancelling_a_task_that_owns_a_process_takes_the_process_down() {
        // T01.03 and T01.04 together, which is how a build cancel will actually work: the job owns
        // the process, watches its token, and terminates what it started before returning.
        let executor = Executor::new(ExecutorConfig::responsive()).expect("the executor starts");
        let (pid_tx, pid_rx) = mpsc::channel();
        let task = executor
            .submit(
                SubmitRequest::new(OwnerId::new(), "a cancellable long job"),
                move |control: JobControl| {
                    let mut child =
                        match OwnedProcess::spawn(shell("owned", "sleep 30"), &command_permit()) {
                            Ok(child) => child,
                            Err(error) => return JobOutcome::failed(error.message()),
                        };
                    let _ = pid_tx.send(child.pid());
                    // A real build polls both: the process, and the request to stop it.
                    loop {
                        if control.token().is_requested() {
                            let termination = child.terminate(DEFAULT_GRACE);
                            crate::host::log::emit(
                                crate::host::log::Level::Info,
                                "host::subprocess",
                                &format!("cancelled: {}", termination.message()),
                            );
                            return JobOutcome::Cancelled;
                        }
                        if let Some(exit) = child.wait_for(Duration::from_millis(20)) {
                            return if exit.is_success() {
                                JobOutcome::Succeeded
                            } else {
                                JobOutcome::failed(exit.message())
                            };
                        }
                    }
                },
            )
            .expect("the queue has room");
        let pid = pid_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the process started");

        assert_eq!(executor.cancel(task), CancelResult::Requested);
        let record = executor
            .wait(task, Duration::from_secs(10))
            .expect("a record");

        assert!(
            matches!(record.outcome, Some(TaskOutcome::Cancelled { .. })),
            "{:?}",
            record.outcome,
        );
        assert_eq!(
            wait_until_gone(pid, Duration::from_secs(5)),
            ProcessPresence::Gone,
            "the process outlived its cancelled task",
        );
        let _ = executor.shutdown();
    }

    #[test]
    fn an_exit_without_a_code_explains_itself() {
        let signal = ExitInfo {
            code: None,
            signal: Some(9),
        };
        assert!(!signal.is_success());
        assert_eq!(signal.message(), "killed by signal 9");
        assert_eq!(
            ExitInfo {
                code: None,
                signal: None
            }
            .message(),
            "ended without an exit code",
        );
    }
}
