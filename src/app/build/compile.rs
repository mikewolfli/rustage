// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Check, build and rebuild: what they run and what their exit means (task T16.02).
//!
//! The acceptance line is *"success rests on the real exit, and a cancel does not leave a misleading
//! artifact"*. Both are in the types:
//!
//! * **The command is data.** [`Job`] holds the action and the selection, and [`Job::command`] is the
//!   one place the argv is built, so the check the designer runs and the build a person runs by hand
//!   are the same line (principle 51 for the feature selection).
//! * **The verdict comes from the exit, not from output.** [`Verdict::from_exit`] maps a real exit
//!   ([`ExitInfo`]) to a [`Verdict`]; a run that was cancelled has **no verdict of its own** and is
//!   [`Verdict::Cancelled`], which is never success — a partially written target that a cancel left
//!   behind is reported stale rather than presented as the build.
//! * **The run is a real, owned process.** [`execute`] starts cargo through
//!   [`OwnedProcess`](crate::host::subprocess::OwnedProcess), which leads a process group of its own,
//!   and polls the [`CancelToken`] on every turn: a cancel signals that group and reports
//!   [`Verdict::Cancelled`] only when the request is what ended it. Nothing here calls
//!   `Command::output`, and no cancel path invents an exit code.
//! * **A rebuild is a clean of one package, not of the tree.** [`Job::clean_command`] removes only the
//!   artifacts of the package being rebuilt (and the whole manifest's when no package was named), so
//!   a rebuild cannot invalidate an unrelated dependency's cache.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::reproducible::Reproducibility;
use super::toolchain::Selection;
use crate::contracts::permission::{EffectClass, Principal};
use crate::host::permission::{ExecutionPermit, PermissionService, Scope};
use crate::host::subprocess::{DEFAULT_GRACE, ExitInfo, OwnedProcess, ProcessSpec, Termination};
use crate::host::tasks::{
    CancelToken, Executor, JobControl, JobOutcome, OwnerId, SubmitError, SubmitRequest, TaskId,
};

/// The cargo flag that makes diagnostics machine-readable, so T16.03 can parse them.
pub const MESSAGE_FORMAT: &str = "--message-format=json";

/// How often a running build looks at its process and its cancellation token.
pub const CANCEL_POLL: Duration = Duration::from_millis(20);

/// How much of each stream a build keeps.
///
/// Larger than the host default because `--message-format=json` puts every diagnostic on stdout:
/// a 256 KiB default would silently drop the tail of a large project's diagnostics (T16.03).
pub const BUILD_CAPTURE_LIMIT: usize = 4 * 1024 * 1024;

/// What a build action asks cargo to do.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    /// Type-check without producing a binary.
    Check,
    /// Build the binary.
    Build,
    /// Remove this package's artifacts, then build.
    Rebuild,
}

impl Action {
    /// The action's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Check => "check",
            Self::Build => "build",
            Self::Rebuild => "rebuild",
        }
    }

    /// The cargo subcommand this action runs last.
    pub fn cargo_subcommand(&self) -> &'static str {
        match self {
            Self::Check => "check",
            Self::Build | Self::Rebuild => "build",
        }
    }
}

/// One build request: the action, the selection, and where it runs.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Job {
    /// What to do.
    pub action: Action,
    /// The profile and features.
    pub selection: Selection,
    /// The target triple, when it differs from the host.
    pub target: Option<String>,
    /// The package to build, for the `-p` flag and for a scoped clean.
    pub package: Option<String>,
    /// The manifest to build, when the working directory is not the one that holds it.
    pub manifest: Option<String>,
    /// The locked/offline/vendored settings this run must obey.
    pub reproducible: Reproducibility,
}

impl Job {
    /// The cargo argv for this job, without the program name.
    pub fn command(&self) -> Vec<String> {
        let mut argv = vec![self.action.cargo_subcommand().to_owned()];
        argv.extend(self.reproducible.flags());
        argv.extend(self.selection.feature_arguments());
        if let Some(target) = &self.target {
            argv.push("--target".to_owned());
            argv.push(target.clone());
        }
        if let Some(package) = &self.package {
            argv.push("-p".to_owned());
            argv.push(package.clone());
        }
        if let Some(manifest) = &self.manifest {
            argv.push("--manifest-path".to_owned());
            argv.push(manifest.clone());
        }
        // The diagnostics are a machine-readable stream; the human summary still goes to stderr.
        argv.push(MESSAGE_FORMAT.to_owned());
        argv
    }

    /// The scoped clean a rebuild runs first, or `None` for a plain check or build.
    ///
    /// A rebuild cleans **this package**, not the workspace: cleaning the tree would throw away every
    /// dependency's cache to repair one crate. When no package was named there is no narrower scope
    /// to clean, so the whole manifest's artifacts are cleaned rather than inventing `-p .`.
    pub fn clean_command(&self) -> Option<Vec<String>> {
        if self.action != Action::Rebuild {
            return None;
        }
        let mut argv = vec!["clean".to_owned()];
        if let Some(package) = &self.package {
            argv.push("-p".to_owned());
            argv.push(package.clone());
        }
        if let Some(manifest) = &self.manifest {
            argv.push("--manifest-path".to_owned());
            argv.push(manifest.clone());
        }
        Some(argv)
    }

    /// The job in one line.
    pub fn render(&self) -> String {
        format!(
            "{} [{}]{}",
            self.action.name(),
            self.selection.render(),
            self.package
                .as_ref()
                .map(|package| format!(" package {package}"))
                .unwrap_or_default(),
        )
    }
}

/// What a build run ended as.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Verdict {
    /// Cargo exited zero.
    Succeeded,
    /// Cargo exited non-zero.
    Failed {
        /// The exit code it reported.
        code: i32,
    },
    /// The process was killed by a signal this run did not send as a stop.
    Signalled {
        /// The signal number the platform reported.
        signal: i32,
    },
    /// The process ended without either an exit code or a signal.
    NoStatus,
    /// The run was cancelled before it exited, so there is no exit code and nothing to trust.
    Cancelled,
}

impl Verdict {
    /// Derives the verdict from how the process really ended.
    pub fn from_exit(exit: &ExitInfo) -> Self {
        match (exit.code, exit.signal) {
            (Some(0), _) => Self::Succeeded,
            (Some(code), _) => Self::Failed { code },
            (None, Some(signal)) => Self::Signalled { signal },
            (None, None) => Self::NoStatus,
        }
    }

    /// Whether this verdict is a real success.
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Succeeded)
    }

    /// The verdict in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Succeeded => "succeeded".to_owned(),
            Self::Failed { code } => format!("failed with exit code {code}"),
            Self::Signalled { signal } => format!("killed by signal {signal}"),
            Self::NoStatus => "ended without an exit code or a signal".to_owned(),
            Self::Cancelled => "cancelled (no exit code; any artifact is stale)".to_owned(),
        }
    }
}

/// One build run's result.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Outcome {
    /// The job that ran.
    pub job: Job,
    /// What it ended as.
    pub verdict: Verdict,
    /// The operating system's process id, when a real process ran.
    pub pid: Option<u32>,
    /// Standard output, kept bounded by the caller.
    pub stdout: String,
    /// Standard error, kept bounded by the caller.
    pub stderr: String,
}

/// How much of a failed build's stderr the one-line report keeps.
const STDERR_EXCERPT: usize = 400;

impl Outcome {
    /// Builds the outcome of a run that exited with `exit` (`None` when it was cancelled).
    ///
    /// Kept for callers that already have a measured code; a real run uses
    /// [`Self::from_process`], which reads the code **and** signal the platform reported.
    pub fn from_exit(job: Job, exit: Option<i32>, stdout: String, stderr: String) -> Self {
        let verdict = match exit {
            Some(0) => Verdict::Succeeded,
            Some(code) => Verdict::Failed { code },
            None => Verdict::Cancelled,
        };
        Self {
            job,
            verdict,
            pid: None,
            stdout,
            stderr,
        }
    }

    /// Builds the outcome of a real process from how it ended.
    pub fn from_process(
        job: Job,
        exit: &ExitInfo,
        pid: Option<u32>,
        stdout: String,
        stderr: String,
    ) -> Self {
        Self {
            job,
            verdict: Verdict::from_exit(exit),
            pid,
            stdout,
            stderr,
        }
    }

    /// Builds the outcome of a run this host stopped on request.
    pub fn cancelled(job: Job, pid: Option<u32>, stdout: String, stderr: String) -> Self {
        Self {
            job,
            verdict: Verdict::Cancelled,
            pid,
            stdout,
            stderr,
        }
    }

    /// The outcome in one line per fact.
    pub fn render(&self) -> Vec<String> {
        let mut out = vec![format!("{}: {}", self.job.render(), self.verdict.render())];
        if !self.stderr.trim().is_empty() {
            out.push(format!("stderr: {}", excerpt(&self.stderr)));
        }
        out
    }
}

/// Runs this job as a host-owned process: a rebuild's scoped clean first, then the cargo command.
///
/// The permit is the host's own session decision (the person asked this host to build), and the
/// returned outcome is measured from the process, never assumed.
pub fn execute(job: &Job, token: &CancelToken) -> Result<Outcome, String> {
    let permit = PermissionService::new()
        .permit(EffectClass::Command, Scope::Session, Principal::Human)
        .map_err(|refusal| refusal.message())?;
    execute_owned(job, "cargo", token, &permit, DEFAULT_GRACE)
}

/// Runs the job's scoped clean (when it has one) and then its command, under one token.
pub fn execute_owned(
    job: &Job,
    program: &str,
    token: &CancelToken,
    permit: &ExecutionPermit,
    grace: Duration,
) -> Result<Outcome, String> {
    if let Some(clean) = job.clean_command() {
        let cleaned = execute_with(job, program, &clean, token, permit, grace)?;
        if !cleaned.verdict.is_success() {
            // A failed or cancelled clean means the build after it would measure the wrong tree.
            return Ok(cleaned);
        }
    }
    execute_with(job, program, &job.command(), token, permit, grace)
}

/// Runs `program` with `arguments` and maps its real exit onto a [`Verdict`].
///
/// The process leads its own group, so [`OwnedProcess::terminate`] reaches only what this run
/// started. While it runs, each turn asks two questions: was a stop requested, and has it ended.
/// A later request that loses the race to a real exit keeps the real exit; only a request that is
/// what ended the process reports [`Verdict::Cancelled`].
pub fn execute_with(
    job: &Job,
    program: &str,
    arguments: &[String],
    token: &CancelToken,
    permit: &ExecutionPermit,
    grace: Duration,
) -> Result<Outcome, String> {
    let spec = ProcessSpec::new(job.render(), program)
        .args(arguments.iter().cloned())
        .capture_limit(BUILD_CAPTURE_LIMIT);
    let mut process = OwnedProcess::spawn(spec, permit)
        .map_err(|error| format!("{program} could not be started: {error}"))?;
    let pid = process.pid();
    loop {
        if token.is_requested() {
            let termination = process.terminate(grace);
            let output = process.output_so_far();
            return match termination {
                Termination::SignalFailed { reason } => Err(format!(
                    "{program} was asked to stop but could not be signalled: {reason}; it may still \
                     be running (pid {pid})"
                )),
                // The request lost the race: the process had already ended, so its real exit is
                // the fact, not a cancellation this host cannot claim.
                Termination::AlreadyExited { exit } => Ok(Outcome::from_process(
                    job.clone(),
                    &exit,
                    Some(pid),
                    output.stdout,
                    output.stderr,
                )),
                Termination::Stopped { .. }
                | Termination::Killed { .. }
                | Termination::DirectOnly { .. } => Ok(Outcome::cancelled(
                    job.clone(),
                    Some(pid),
                    output.stdout,
                    output.stderr,
                )),
            };
        }
        if let Some(exit) = process.wait_for(CANCEL_POLL) {
            let output = process.output_so_far();
            return Ok(Outcome::from_process(
                job.clone(),
                &exit,
                Some(pid),
                output.stdout,
                output.stderr,
            ));
        }
    }
}

/// Where a submitted build leaves its result for the submitter.
type ResultSlot = Arc<Mutex<Option<Result<Outcome, String>>>>;

fn store(slot: &ResultSlot, value: Result<Outcome, String>) {
    *slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(value);
}

/// A build admitted to the host's task executor, which gives it an owner and a token.
///
/// Submitting is what makes "cancel the build" a request to the host rather than a flag the caller
/// invents: [`Executor::cancel`](crate::host::tasks::Executor::cancel) sets the token this job hands
/// to [`execute_with`], and the process group is what stops.
pub struct BuildTask {
    job: Job,
    program: String,
    result: ResultSlot,
}

impl BuildTask {
    /// Submits `job` under `owner` and returns the handle its result will appear in.
    pub fn submit(
        executor: &Executor,
        owner: OwnerId,
        job: Job,
        program: impl Into<String>,
    ) -> Result<Submitted, SubmitError> {
        let result: ResultSlot = Arc::new(Mutex::new(None));
        let task = executor.submit(
            SubmitRequest::new(owner, job.render()),
            Self {
                job,
                program: program.into(),
                result: result.clone(),
            },
        )?;
        Ok(Submitted { task, result })
    }
}

impl crate::host::tasks::Job for BuildTask {
    fn run(&mut self, control: JobControl) -> JobOutcome {
        let permit = match PermissionService::new().permit(
            EffectClass::Command,
            Scope::Session,
            Principal::Human,
        ) {
            Ok(permit) => permit,
            Err(refusal) => {
                let reason = refusal.message();
                store(&self.result, Err(reason.clone()));
                return JobOutcome::failed(reason);
            }
        };
        match execute_owned(
            &self.job,
            &self.program,
            control.token(),
            &permit,
            DEFAULT_GRACE,
        ) {
            Ok(outcome) => {
                let verdict = outcome.verdict.clone();
                let message = format!("{}: {}", outcome.job.render(), verdict.render());
                let cancelled = verdict == Verdict::Cancelled;
                store(&self.result, Ok(outcome));
                if cancelled && control.is_cancelled() {
                    JobOutcome::Cancelled
                } else if verdict.is_success() {
                    JobOutcome::Succeeded
                } else {
                    JobOutcome::failed(message)
                }
            }
            Err(reason) => {
                store(&self.result, Err(reason.clone()));
                JobOutcome::failed(reason)
            }
        }
    }
}

/// A submitted build: its task id, and where its measured outcome appears.
#[derive(Clone, Debug)]
pub struct Submitted {
    task: TaskId,
    result: ResultSlot,
}

impl Submitted {
    /// The task's id in the executor.
    pub fn task(&self) -> TaskId {
        self.task
    }

    /// The job's own result, once the worker has stored it.
    pub fn result(&self) -> Option<Result<Outcome, String>> {
        self.result
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

/// A bounded excerpt, so a failure names its cause without dumping an unbounded log.
fn excerpt(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() <= STDERR_EXCERPT {
        return text.to_owned();
    }
    let head: String = text.chars().take(STDERR_EXCERPT).collect();
    format!("{head}… ({} more byte(s))", text.len() - head.len())
}

#[cfg(test)]
mod tests {
    use super::{Action, BuildTask, Job, Outcome, Verdict, execute_with};
    use crate::app::build::toolchain::Selection;
    use crate::contracts::permission::{EffectClass, Principal};
    use crate::host::permission::{ExecutionPermit, PermissionService, Scope};
    use crate::host::subprocess::{ProcessPresence, presence};
    use crate::host::tasks::{
        CancelResult, CancelToken, Executor, ExecutorConfig, OwnerId, TaskOutcome,
    };
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    fn job(action: Action) -> Job {
        Job {
            action,
            selection: Selection {
                profile: "desktop".to_owned(),
                extra: Vec::new(),
            },
            target: None,
            package: Some("demo".to_owned()),
            manifest: None,
            reproducible: crate::app::build::reproducible::Reproducibility::default(),
        }
    }

    #[test]
    fn a_check_runs_cargo_check_with_the_profile_feature_and_json_diagnostics() {
        assert_eq!(
            job(Action::Check).command(),
            vec![
                "check",
                "--no-default-features",
                "--features",
                "desktop",
                "-p",
                "demo",
                "--message-format=json"
            ],
        );
    }

    #[test]
    fn a_build_also_asks_for_machine_readable_diagnostics() {
        let argv = job(Action::Build).command();
        assert_eq!(
            argv.last().map(String::as_str),
            Some("--message-format=json")
        );
        // The clean stays human-readable: it prints no diagnostics at all.
        assert!(
            !job(Action::Rebuild)
                .clean_command()
                .expect("a clean")
                .contains(&"--message-format=json".to_owned())
        );
    }

    #[test]
    fn a_build_runs_cargo_build_and_a_cross_target_adds_target() {
        let mut cross = job(Action::Build);
        cross.target = Some("x86_64-pc-windows-msvc".to_owned());
        let argv = cross.command();
        assert_eq!(argv[0], "build");
        assert!(
            argv.windows(2)
                .any(|pair| pair == ["--target", "x86_64-pc-windows-msvc"])
        );
    }

    #[test]
    fn only_a_rebuild_cleans_first_and_only_the_package() {
        assert!(job(Action::Check).clean_command().is_none());
        assert!(job(Action::Build).clean_command().is_none());
        assert_eq!(
            job(Action::Rebuild).clean_command(),
            Some(vec!["clean".to_owned(), "-p".to_owned(), "demo".to_owned()]),
        );
    }

    #[test]
    fn a_rebuild_without_a_package_cleans_the_manifest_rather_than_inventing_one() {
        let mut unnamed = job(Action::Rebuild);
        unnamed.package = None;
        assert_eq!(unnamed.clean_command(), Some(vec!["clean".to_owned()]));
        assert!(
            !unnamed
                .clean_command()
                .expect("a clean")
                .contains(&".".to_owned())
        );
    }

    #[test]
    fn reproducibility_flags_reach_the_real_cargo_argv() {
        let mut locked = job(Action::Build);
        locked.reproducible.locked = true;
        assert!(locked.command().contains(&"--locked".to_owned()));
        let mut frozen = job(Action::Build);
        frozen.reproducible.locked = true;
        frozen.reproducible.offline = true;
        assert!(frozen.command().contains(&"--frozen".to_owned()));
        assert!(!frozen.command().contains(&"--locked".to_owned()));
    }

    #[test]
    fn a_failure_names_its_stderr_without_dumping_it() {
        let outcome = Outcome::from_exit(
            job(Action::Check),
            Some(101),
            String::new(),
            "error: the package does not contain this feature: desktop".to_owned(),
        );
        let rendered = outcome.render().join("\n");
        assert!(
            rendered.contains("does not contain this feature"),
            "{rendered}"
        );
    }

    #[test]
    fn success_rests_on_a_real_zero_exit() {
        let outcome = Outcome::from_exit(job(Action::Build), Some(0), String::new(), String::new());
        assert_eq!(outcome.verdict, Verdict::Succeeded);
        assert!(outcome.verdict.is_success());

        let failed = Outcome::from_exit(job(Action::Build), Some(101), String::new(), "e".into());
        assert_eq!(failed.verdict, Verdict::Failed { code: 101 });
        assert!(!failed.verdict.is_success());
    }

    #[test]
    fn a_cancel_is_not_a_success_and_says_its_artifact_is_stale() {
        let cancelled = Outcome::from_exit(job(Action::Build), None, String::new(), String::new());
        assert_eq!(cancelled.verdict, Verdict::Cancelled);
        assert!(!cancelled.verdict.is_success());
        assert!(cancelled.verdict.render().contains("stale"));
    }

    /// The permit every real start in these tests goes through — the same session decision a caller
    /// gets, not a bypass.
    fn command_permit() -> ExecutionPermit {
        PermissionService::new()
            .permit(EffectClass::Command, Scope::Session, Principal::Human)
            .expect("the host's own session may run a command")
    }

    /// A tiny executable that ignores the cargo argv and does what `body` says.
    #[cfg(unix)]
    fn script(name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "rustage-compile-{name}-{}-{}.sh",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed),
        ));
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("a script");
        let mut permissions = std::fs::metadata(&path).expect("metadata").permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).expect("an executable script");
        path
    }

    /// Waits for a pid to disappear, returning what was last seen.
    #[cfg(unix)]
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

    #[cfg(unix)]
    #[test]
    fn a_real_zero_and_non_zero_exit_become_success_and_failure() {
        let permit = command_permit();
        let token = CancelToken::default();

        let zero = script("zero", "exit 0");
        let outcome = execute_with(
            &job(Action::Check),
            zero.to_str().expect("a utf-8 path"),
            &[],
            &token,
            &permit,
            Duration::from_secs(3),
        )
        .expect("a run");
        assert_eq!(outcome.verdict, Verdict::Succeeded);
        assert!(outcome.verdict.is_success());
        let pid = outcome.pid.expect("a real process id");
        assert_eq!(
            wait_until_gone(pid, Duration::from_secs(5)),
            ProcessPresence::Gone,
        );

        let seven = script("seven", "exit 7");
        let outcome = execute_with(
            &job(Action::Build),
            seven.to_str().expect("a utf-8 path"),
            &[],
            &token,
            &permit,
            Duration::from_secs(3),
        )
        .expect("a run");
        assert_eq!(outcome.verdict, Verdict::Failed { code: 7 });
        assert!(!outcome.verdict.is_success());

        std::fs::remove_file(zero).ok();
        std::fs::remove_file(seven).ok();
    }

    #[cfg(unix)]
    #[test]
    fn cancelling_kills_the_process_group_and_leaves_no_orphan() {
        let permit = command_permit();
        let token = CancelToken::default();
        let directory = std::env::temp_dir().join(format!(
            "rustage-compile-cancel-{}-{:?}",
            std::process::id(),
            std::thread::current().id(),
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("a temporary directory");
        let pid_file = directory.join("grandchild.pid");
        let program = script(
            "cancel",
            &format!("sleep 30 &\necho $! > '{}'\nwait", pid_file.display()),
        );

        // Ask only once the child exists, so this measures a cancel of a running process rather
        // than a request that happened to arrive before the spawn.
        let asking = token.clone();
        let observe = pid_file.clone();
        let waiter = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline {
                let written = std::fs::read_to_string(&observe)
                    .map(|text| !text.trim().is_empty())
                    .unwrap_or(false);
                if written {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            asking.request();
        });

        let began = Instant::now();
        let outcome = execute_with(
            &job(Action::Build),
            program.to_str().expect("a utf-8 path"),
            &[],
            &token,
            &permit,
            Duration::from_secs(3),
        )
        .expect("a run");
        waiter.join().expect("the asking thread");

        assert_eq!(outcome.verdict, Verdict::Cancelled);
        assert!(!outcome.verdict.is_success());
        assert!(
            began.elapsed() < Duration::from_secs(15),
            "a cancel must not wait for the sleeping child: {:?}",
            began.elapsed(),
        );

        let parent = outcome.pid.expect("a real process id");
        assert_eq!(
            wait_until_gone(parent, Duration::from_secs(5)),
            ProcessPresence::Gone,
            "the build process outlived its cancellation",
        );
        let child: u32 = std::fs::read_to_string(&pid_file)
            .expect("the background child wrote its pid")
            .trim()
            .parse()
            .expect("a pid");
        assert_eq!(
            wait_until_gone(child, Duration::from_secs(5)),
            ProcessPresence::Gone,
            "the process group's child outlived its cancelled parent",
        );

        std::fs::remove_dir_all(&directory).ok();
        std::fs::remove_file(program).ok();
    }

    #[cfg(unix)]
    #[test]
    fn a_submitted_build_belongs_to_an_owner_and_a_cancel_takes_its_process_down() {
        let executor = Executor::new(ExecutorConfig::responsive()).expect("an executor");
        let owner = OwnerId::new();
        let directory = std::env::temp_dir().join(format!(
            "rustage-compile-task-{}-{:?}",
            std::process::id(),
            std::thread::current().id(),
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("a temporary directory");
        let pid_file = directory.join("task.pid");
        let program = script(
            "task",
            &format!("echo $$ > '{}'\nsleep 30", pid_file.display()),
        );
        let submitted = BuildTask::submit(
            &executor,
            owner,
            job(Action::Build),
            program.to_str().expect("a utf-8 path"),
        )
        .expect("a submission");

        let deadline = Instant::now() + Duration::from_secs(10);
        let mut pid = None;
        while Instant::now() < deadline {
            if let Ok(text) = std::fs::read_to_string(&pid_file)
                && let Ok(value) = text.trim().parse::<u32>()
            {
                pid = Some(value);
                break;
            }
            let final_already = executor
                .record(submitted.task())
                .map(|record| record.is_final())
                .unwrap_or(false);
            assert!(
                !final_already,
                "the build ended before it could be cancelled"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        let pid = pid.expect("the task wrote its pid");

        assert_eq!(executor.cancel(submitted.task()), CancelResult::Requested);
        let record = executor
            .wait(submitted.task(), Duration::from_secs(10))
            .expect("a record");
        assert!(
            matches!(record.outcome, Some(TaskOutcome::Cancelled { .. })),
            "{:?}",
            record.outcome,
        );
        assert_eq!(
            wait_until_gone(pid, Duration::from_secs(5)),
            ProcessPresence::Gone,
            "the task's process outlived its cancellation",
        );
        let outcome = submitted
            .result()
            .expect("the task left a result")
            .expect("a measured run");
        assert_eq!(outcome.verdict, Verdict::Cancelled);

        let report = executor.shutdown();
        assert!(report.is_clean(), "{}", report.message());
        std::fs::remove_dir_all(&directory).ok();
        std::fs::remove_file(program).ok();
    }
}
