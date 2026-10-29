// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Cargo test discovery, selection and execution (task T21.04).
//!
//! Three things this module refuses to do:
//!
//! * **Invent a result.** A test is recorded only from a line the test harness actually printed.
//!   A test that was filtered out, ignored, or still undiscovered stays [`TestStatus::NotRun`] or
//!   [`TestStatus::Skipped`]; nothing is promoted to "passed" because the command exited zero.
//! * **Parse a shell line.** Commands are an argument vector, each argument already split. The
//!   directory is a parameter, not a `cd … &&` string, so a path with a space or a quote is just a
//!   path.
//! * **Hide a cancellation.** A stop request that ended the process is reported as cancelled; a
//!   request that lost the race to a real exit keeps the real exit. Either way the tests that did
//!   not report a result stay not run.
//!
//! The command boundary is a trait ([`CommandRunner`]) so the parser and the state transitions are
//! tested with canned output and no compiler is started. The production implementation
//! ([`ProcessRunner`]) goes through the host's owned process: its own process group, a captured
//! and bounded output buffer, and a cancellation flag polled while it runs.
//!
//! Failure text is fed back into [`Center`] as [`Source::Test`] problems at the revision the run
//! measured, with the panic's source position when the harness printed one.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::Duration;

use super::report::{Center, Problem, Severity, Source, TestResult, TestStatus, Tests};
use crate::contracts::permission::{EffectClass, Principal};
use crate::host::permission::{PermissionService, Scope};
use crate::host::subprocess::{OwnedProcess, ProcessSpec, Termination};
use crate::host::tasks::CancelToken;

/// How much of each stream is kept for one cargo invocation.
///
/// A test binary's panic text is what the failure feedback is built from, so the limit is larger
/// than a one-line build message but still bounded: a test that prints without end must not grow
/// the host's memory.
pub const TEST_CAPTURE_LIMIT: usize = 512 * 1024;

/// How long a cancelled cargo invocation is given to stop before it is killed.
pub const TEST_CANCEL_GRACE: Duration = Duration::from_secs(3);

/// How often a running invocation is asked whether it has ended.
const CANCEL_POLL: Duration = Duration::from_millis(25);

/// The longest failure detail kept for one test.
const MAX_DETAIL_CHARS: usize = 2_000;

/// The longest first failure line used in a problem message.
const MAX_REASON_CHARS: usize = 240;

/// How this runner invokes cargo.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TestConfig {
    /// The program to start. `cargo` by default.
    pub program: String,
    /// The project directory the command runs in.
    pub directory: Option<PathBuf>,
    /// How many bytes of each stream are kept.
    pub capture_limit: usize,
    /// How long a cancelled invocation may take to stop.
    pub grace: Duration,
}

impl Default for TestConfig {
    fn default() -> Self {
        Self {
            program: "cargo".to_owned(),
            directory: None,
            capture_limit: TEST_CAPTURE_LIMIT,
            grace: TEST_CANCEL_GRACE,
        }
    }
}

impl TestConfig {
    /// A configuration that runs cargo in `directory`.
    pub fn in_directory(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: Some(directory.into()),
            ..Self::default()
        }
    }
}

/// One argv-only command for the runner.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Command {
    /// What this invocation is, in the caller's words.
    pub label: String,
    /// The program to start.
    pub program: String,
    /// Each argument already split; no shell parses them.
    pub arguments: Vec<String>,
    /// The working directory, when it matters.
    pub directory: Option<PathBuf>,
    /// How many bytes of each stream are kept.
    pub capture_limit: usize,
}

impl Command {
    fn new(label: impl Into<String>, program: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            program: program.into(),
            arguments: Vec::new(),
            directory: None,
            capture_limit: TEST_CAPTURE_LIMIT,
        }
    }

    fn args<I, S>(mut self, arguments: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.arguments.extend(arguments.into_iter().map(Into::into));
        self
    }
}

/// What one invocation ended with.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct CommandOutput {
    /// The exit code, when the platform reported one. A signalled process has none.
    pub exit: Option<i32>,
    /// The captured standard output.
    pub stdout: String,
    /// The captured standard error.
    pub stderr: String,
    /// Whether the capture limit dropped bytes.
    pub truncated: bool,
    /// Whether the stop request is what ended the process.
    ///
    /// False when the process had already exited, so a request that lost the race does not relabel
    /// a real result as cancelled.
    pub cancelled: bool,
}

/// The command boundary. Injectable so the state transitions are testable without cargo.
pub trait CommandRunner: Send + Sync {
    /// Runs `command`, stopping when `cancel` is requested.
    fn run(&self, command: &Command, cancel: &CancelToken) -> Result<CommandOutput, String>;
}

/// The production runner: a host-owned process with a group of its own.
#[derive(Clone, Copy, Debug)]
pub struct ProcessRunner {
    grace: Duration,
    poll: Duration,
}

impl ProcessRunner {
    /// A runner that gives a stopped invocation `grace` to exit.
    pub fn new(grace: Duration) -> Self {
        Self {
            grace,
            poll: CANCEL_POLL,
        }
    }
}

impl CommandRunner for ProcessRunner {
    fn run(&self, command: &Command, cancel: &CancelToken) -> Result<CommandOutput, String> {
        if cancel.is_requested() {
            // Nothing was started, so nothing ran. Reporting an empty successful run here would
            // let a caller read "no output" as "no failures".
            return Ok(CommandOutput {
                cancelled: true,
                ..CommandOutput::default()
            });
        }
        if command.program.trim().is_empty() {
            return Err("no program to run".to_owned());
        }

        // Running the project's own tests is the session's decision, like the debug runner's
        // `start`: the permit names a human and this session, not fetched project code.
        let permit = PermissionService::new()
            .permit(EffectClass::Command, Scope::Session, Principal::Human)
            .map_err(|refusal| refusal.message())?;
        let mut spec = ProcessSpec::new(command.label.as_str(), command.program.as_str())
            .args(command.arguments.iter().cloned())
            .capture_limit(command.capture_limit);
        if let Some(directory) = &command.directory {
            spec = spec.in_directory(directory);
        }
        let mut process = OwnedProcess::spawn(spec, &permit)
            .map_err(|error| format!("{} could not be started: {error}", command.label.as_str()))?;

        loop {
            if cancel.is_requested() {
                return match process.terminate(self.grace) {
                    Termination::SignalFailed { reason } => Err(format!(
                        "{} was asked to stop but could not be signalled: {reason}; it may still be \
                         running",
                        command.label.as_str()
                    )),
                    // The request lost the race: the process had already ended, so its real exit
                    // is the fact and this is not a cancellation.
                    Termination::AlreadyExited { exit } => Ok(ended(&process, exit.code, false)),
                    Termination::Stopped { .. }
                    | Termination::Killed { .. }
                    | Termination::DirectOnly { .. } => {
                        let output = process.output_so_far();
                        let truncated = output.is_truncated();
                        Ok(CommandOutput {
                            exit: None,
                            stdout: output.stdout,
                            stderr: output.stderr,
                            truncated,
                            cancelled: true,
                        })
                    }
                };
            }
            if let Some(exit) = process.wait_for(self.poll) {
                return Ok(ended(&process, exit.code, false));
            }
        }
    }
}

/// The final output of a process that ended on its own.
fn ended(process: &OwnedProcess, exit: Option<i32>, cancelled: bool) -> CommandOutput {
    let output = process.output_so_far();
    let truncated = output.is_truncated();
    CommandOutput {
        exit,
        stdout: output.stdout,
        stderr: output.stderr,
        truncated,
        cancelled,
    }
}

/// One test the harness listed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DiscoveredTest {
    /// The test path, as the harness prints it.
    pub name: String,
    /// Whether the listing marked it ignored.
    pub ignored: bool,
}

/// What `cargo test -- --list` reported.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Discovery {
    /// The tests, in output order.
    pub tests: Vec<DiscoveredTest>,
    /// How many benchmarks the listing named. Benchmarks are not tests and are not recorded.
    pub benchmarks: usize,
    /// Whether the capture limit dropped output, so the list may be incomplete.
    pub truncated: bool,
    /// How many non-empty lines were not understood. A listing with these is not treated as
    /// complete: an ignored line is an unknown test, not a missing one.
    pub unparsed: usize,
    /// Why the list is not complete even though it parsed: the ignored-test listing failed, so
    /// which of these tests are ignored is unknown.
    pub incomplete: Option<String>,
}

impl Discovery {
    /// How many listed tests are marked ignored.
    pub fn ignored(&self) -> usize {
        self.tests.iter().filter(|test| test.ignored).count()
    }

    /// Marks the tests the ignored listing named, keeping the full listing's order.
    ///
    /// Only ever sets the flag: a toolchain that also marks the plain listing keeps its marks.
    fn mark_ignored(&mut self, ignored: &[DiscoveredTest]) {
        let names: BTreeSet<&str> = ignored.iter().map(|test| test.name.as_str()).collect();
        for test in &mut self.tests {
            test.ignored |= names.contains(test.name.as_str());
        }
    }
}

/// Which tests to run.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Selection {
    /// A substring filter the harness applies.
    pub filter: Option<String>,
    /// Exact test paths. When present, the harness is asked for these and nothing else.
    pub names: Vec<String>,
}

impl Selection {
    /// Every test the harness will run.
    pub fn all() -> Self {
        Self::default()
    }

    /// The tests whose path contains `filter`.
    pub fn filter(filter: impl Into<String>) -> Self {
        Self {
            filter: Some(filter.into()),
            names: Vec::new(),
        }
    }

    /// Exactly these test paths.
    pub fn exact<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            filter: None,
            names: names.into_iter().map(Into::into).collect(),
        }
    }

    /// The harness arguments after the separating `--`.
    fn arguments(&self) -> Vec<String> {
        let mut arguments = Vec::new();
        if !self.names.is_empty() {
            // `--exact` keeps a selected name from also selecting every test whose path contains
            // it, which would report results for tests the caller did not choose.
            arguments.push("--exact".to_owned());
            arguments.extend(self.names.iter().cloned());
        } else if let Some(filter) = &self.filter {
            arguments.push(filter.clone());
        }
        arguments
    }
}

/// One failure with the text the harness printed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TestFailure {
    /// The test path.
    pub name: String,
    /// The first line that explains it.
    pub reason: String,
    /// The captured detail block, bounded.
    pub detail: String,
    /// The panic's source position, when the harness printed one.
    pub location: Option<String>,
}

/// What one invocation did.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct RunReport {
    /// Whether a stop request ended the run.
    pub cancelled: bool,
    /// The exit code the harness reported, when it ended on its own.
    pub exit: Option<i32>,
    /// The tests this run recorded, in name order.
    pub observed: Vec<TestResult>,
    /// Tests with no result after the run: the exact names the selection asked for, plus every
    /// known test this session has not run. It never includes a test that reported passed, failed
    /// or skipped.
    pub not_run: Vec<String>,
    /// Whether the capture limit dropped output.
    pub truncated: bool,
    /// The failures, in name order.
    pub failures: Vec<TestFailure>,
    /// A reason this run's result is not a complete pass, when there is one.
    pub incomplete: Option<String>,
}

impl RunReport {
    /// The number of tests that ran and passed.
    pub fn passed(&self) -> usize {
        self.count(TestStatus::Passed)
    }

    /// The number of tests that ran and failed.
    pub fn failed(&self) -> usize {
        self.count(TestStatus::Failed)
    }

    /// The number of tests the harness skipped.
    pub fn skipped(&self) -> usize {
        self.count(TestStatus::Skipped)
    }

    fn count(&self, status: TestStatus) -> usize {
        self.observed
            .iter()
            .filter(|result| result.status == status)
            .count()
    }

    /// Whether this report is a complete pass for `selection`.
    ///
    /// A run that was stopped, that reported a failure, or whose output could not be trusted is
    /// not a pass. Neither is a test the selection asked about that reported no result — filtered
    /// out by the harness, skipped by the capture limit, or never reached. "Not run" is not
    /// "passed", so this is the test a caller may use for an exit code.
    pub fn complete_for(&self, selection: &Selection) -> bool {
        if self.cancelled || !self.failures.is_empty() || self.incomplete.is_some() {
            return false;
        }
        if selection.names.is_empty() {
            // A whole-suite (or filtered) run is a pass only when every test this session knows
            // about reported a result.
            self.not_run.is_empty()
        } else {
            // An exact selection asks about the tests it named, and only those.
            selection
                .names
                .iter()
                .all(|name| !self.not_run.contains(name))
        }
    }
}

/// Discovers and runs cargo tests.
pub struct TestRunner {
    config: TestConfig,
    runner: Box<dyn CommandRunner>,
}

impl TestRunner {
    /// A runner that starts real processes.
    pub fn new(config: TestConfig) -> Self {
        let grace = config.grace;
        Self {
            config,
            runner: Box::new(ProcessRunner::new(grace)),
        }
    }

    /// A runner that uses `runner` instead of starting processes.
    ///
    /// The test seam: the parser and the state transitions are exercised with canned output and
    /// no compiler starts. Production always uses [`ProcessRunner`], so a fake cargo cannot make
    /// "the tests ran" true without a process having run.
    #[cfg(test)]
    pub fn with_runner(config: TestConfig, runner: impl CommandRunner + 'static) -> Self {
        Self {
            config,
            runner: Box::new(runner),
        }
    }

    /// Lists the tests, without starting a test.
    pub fn discover(&self) -> Result<Discovery, String> {
        self.discover_cancellable(&CancelToken::default())
    }

    /// Lists the tests, stopping if `cancel` is requested.
    pub fn discover_cancellable(&self, cancel: &CancelToken) -> Result<Discovery, String> {
        let command = self.command("cargo test -- --list", ["test", "--", "--list"]);
        let output = self.runner.run(&command, cancel)?;
        if output.cancelled {
            return Err("test discovery was cancelled before it finished".to_owned());
        }
        let mut discovery = match output.exit {
            Some(0) => parse_listing(&output.stdout, output.truncated),
            Some(code) => {
                return Err(format!(
                    "cargo test -- --list exited with code {code}: {}",
                    tail(&output.stderr)
                ));
            }
            None => {
                return Err(format!(
                    "cargo test -- --list ended without an exit code: {}",
                    tail(&output.stderr)
                ));
            }
        };

        // libtest 1.98 prints `name: test` for an ignored test in the plain listing too — verified
        // against a real project, not assumed — so the plain listing cannot say which tests are
        // ignored. `--list --ignored` is the harness filter that names only those. A failure here
        // does not throw away the tests that were listed; it says which part is unknown.
        let ignored_command = self.command(
            "cargo test -- --list --ignored",
            ["test", "--", "--list", "--ignored"],
        );
        let ignored = self.runner.run(&ignored_command, cancel)?;
        if ignored.cancelled {
            return Err("test discovery was cancelled before it finished".to_owned());
        }
        match ignored.exit {
            Some(0) => {
                let listing = parse_listing(&ignored.stdout, ignored.truncated);
                discovery.mark_ignored(&listing.tests);
                discovery.truncated |= listing.truncated;
                discovery.unparsed += listing.unparsed;
            }
            _ => {
                discovery.incomplete = Some(format!(
                    "cargo test -- --list --ignored exited with {}; which tests are ignored is \
                     unknown",
                    exit_text(ignored.exit)
                ));
            }
        }
        Ok(discovery)
    }

    /// Lists the tests and records each newly seen one as not run.
    ///
    /// A test that already has a result keeps it: re-listing a suite is not a reason to forget that
    /// a test passed a moment ago.
    pub fn discover_into(&self, tests: &mut Tests) -> Result<Discovery, String> {
        let discovery = self.discover()?;
        for test in &discovery.tests {
            tests.note_discovered(test.name.clone());
        }
        Ok(discovery)
    }

    /// Runs the selection and records the results.
    ///
    /// Results are recorded for the tests the harness explicitly reported. Everything else in the
    /// selection stays not run, and a failure is added to `center` as a test problem at `revision`.
    ///
    /// Only the tests use this: every production caller names a stop policy it owns, so an
    /// operation that cannot be stopped is not built into the binary.
    #[cfg(test)]
    pub fn run(
        &self,
        selection: &Selection,
        revision: u64,
        target: &str,
        tests: &mut Tests,
        center: &mut Center,
    ) -> Result<RunReport, String> {
        self.run_cancellable(
            selection,
            revision,
            target,
            tests,
            center,
            &CancelToken::default(),
        )
    }

    /// Runs the selection, stopping if `cancel` is requested.
    pub fn run_cancellable(
        &self,
        selection: &Selection,
        revision: u64,
        target: &str,
        tests: &mut Tests,
        center: &mut Center,
        cancel: &CancelToken,
    ) -> Result<RunReport, String> {
        if cancel.is_requested() {
            // Nothing was started, so no test ran. The tests this session already knows about are
            // listed as not run for the same reason a terminated run lists them: a stop is the
            // opposite of a pass, and "0 not run" here would read as "nothing was missed".
            return Ok(RunReport {
                cancelled: true,
                not_run: outstanding(tests, selection, &BTreeSet::new()),
                ..RunReport::default()
            });
        }

        let mut arguments = vec!["test".to_owned(), "--".to_owned()];
        arguments.extend(selection.arguments());
        let command = self.command("cargo test", arguments);
        let output = self.runner.run(&command, cancel)?;
        let parsed = parse_run_output(&output.stdout);

        for result in &parsed.results {
            tests.record(result.name.clone(), result.status, result.seconds);
        }
        for failure in &parsed.failures {
            center.add(failure.problem(target, revision));
        }

        let observed: BTreeSet<&str> = parsed
            .results
            .iter()
            .map(|result| result.name.as_str())
            .collect();
        let not_run = outstanding(tests, selection, &observed);

        let mut notes = Vec::new();
        if output.truncated {
            notes.push(
                "the capture limit dropped output, so tests without an explicit result stay not \
                 run"
                .to_owned(),
            );
        }
        if parsed.unparsed > 0 {
            notes.push(format!(
                "{} output line(s) looked like test results but were not understood; tests without \
                 an explicit result stay not run",
                parsed.unparsed
            ));
        }
        if !output.cancelled && output.exit != Some(0) && parsed.failures.is_empty() {
            notes.push(format!(
                "cargo test exited with {} and no failing test could be parsed; the run is not a \
                 pass: {}",
                exit_text(output.exit),
                tail(&output.stderr)
            ));
        }
        if output.exit == Some(0) && parsed.results.is_empty() && !selection.names.is_empty() {
            notes.push(format!(
                "cargo test reported success but ran none of the {} selected test(s); they stay \
                 not run",
                selection.names.len()
            ));
        }

        Ok(RunReport {
            cancelled: output.cancelled,
            exit: output.exit,
            observed: parsed.results,
            not_run,
            truncated: output.truncated,
            failures: parsed.failures,
            incomplete: if notes.is_empty() {
                None
            } else {
                Some(notes.join("; "))
            },
        })
    }

    /// Re-runs exactly the tests whose last recorded status is failed.
    ///
    /// Only the tests use this, for the same reason [`TestRunner::run`] does.
    #[cfg(test)]
    pub fn rerun_failed(
        &self,
        revision: u64,
        target: &str,
        tests: &mut Tests,
        center: &mut Center,
    ) -> Result<RunReport, String> {
        self.rerun_failed_cancellable(revision, target, tests, center, &CancelToken::default())
            .map(|(_, report)| report)
    }

    /// Re-runs the failed tests, stopping if `cancel` is requested.
    ///
    /// The selection is returned with the report because the caller that prints the result also has
    /// to say whether the re-run covered everything it asked for
    /// ([`RunReport::complete_for`]), and only this call knows which tests were still failing when
    /// it started.
    pub fn rerun_failed_cancellable(
        &self,
        revision: u64,
        target: &str,
        tests: &mut Tests,
        center: &mut Center,
        cancel: &CancelToken,
    ) -> Result<(Selection, RunReport), String> {
        let failed: Vec<String> = tests
            .results()
            .into_iter()
            .filter(|result| result.status == TestStatus::Failed)
            .map(|result| result.name.clone())
            .collect();
        if failed.is_empty() {
            return Err("there is no failed test to re-run".to_owned());
        }
        let selection = Selection::exact(failed);
        let report = self.run_cancellable(&selection, revision, target, tests, center, cancel)?;
        Ok((selection, report))
    }

    fn command<I, S>(&self, label: &str, arguments: I) -> Command
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Command::new(label, self.config.program.as_str())
            .args(arguments)
            .in_directory(self.config.directory.clone())
            .capture_limit(self.config.capture_limit)
    }
}

impl Command {
    fn in_directory(mut self, directory: Option<PathBuf>) -> Self {
        self.directory = directory;
        self
    }

    fn capture_limit(mut self, limit: usize) -> Self {
        self.capture_limit = limit;
        self
    }
}

impl TestFailure {
    fn problem(&self, target: &str, revision: u64) -> Problem {
        Problem {
            source: Source::Test,
            target: target.to_owned(),
            revision,
            node: None,
            location: self.location.clone(),
            severity: Severity::Error,
            message: format!("test {} failed: {}", self.name, self.reason),
        }
    }
}

/// The tests a run left without a result, in name order.
///
/// Every test this session knows about that has not run, plus every selected name that did not
/// report one. `observed` is what this run's output proved, so a selected test that reported stays
/// out of the list even when it was already not run before.
fn outstanding(tests: &Tests, selection: &Selection, observed: &BTreeSet<&str>) -> Vec<String> {
    let mut not_run: Vec<String> = tests
        .results()
        .into_iter()
        .filter(|result| result.status == TestStatus::NotRun)
        .map(|result| result.name.clone())
        .collect();
    for name in &selection.names {
        if !observed.contains(name.as_str()) && !not_run.contains(name) {
            not_run.push(name.clone());
        }
    }
    not_run.sort();
    not_run.dedup();
    not_run
}

/// The results one run's output proves.
#[derive(Clone, PartialEq, Debug, Default)]
struct ParsedRun {
    results: Vec<TestResult>,
    failures: Vec<TestFailure>,
    unparsed: usize,
}

fn parse_run_output(text: &str) -> ParsedRun {
    let mut results: BTreeMap<String, TestStatus> = BTreeMap::new();
    let mut details: BTreeMap<String, String> = BTreeMap::new();
    let mut current: Option<String> = None;
    let mut unparsed = 0usize;

    for raw in text.lines() {
        let line = raw.trim_end_matches('\r');
        let trimmed = line.trim();

        if let Some(name) = block_header(trimmed) {
            details.entry(name.clone()).or_default();
            current = Some(name);
            continue;
        }
        if trimmed == "----" {
            continue;
        }
        if trimmed == "failures:" {
            // The summary section repeats the failing names; appending it to the last detail block
            // would put the summary into the failure's own text.
            current = None;
            continue;
        }

        // Inside a captured-output block every line belongs to the test's own output, even one
        // that happens to begin with "test ". Parsing it as a result would let a test's own print
        // claim another test ran; the harness's result lines all precede the summary.
        if let Some(name) = &current {
            if !trimmed.is_empty() && !trimmed.starts_with("note: run with") {
                let detail = details.entry(name.clone()).or_default();
                if detail.len() < MAX_DETAIL_CHARS {
                    if !detail.is_empty() {
                        detail.push('\n');
                    }
                    detail.push_str(trimmed);
                    truncate_in_place(detail, MAX_DETAIL_CHARS);
                }
            }
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix("test ") {
            if let Some((name, outcome)) = rest.rsplit_once(" ... ") {
                let name = name.trim();
                if let Some(status) = status_of(outcome) {
                    results.insert(name.to_owned(), status);
                    continue;
                }
                if outcome.starts_with("filtered out") || outcome.starts_with("bench") {
                    // A filtered test did not run, and a benchmark is not a test: neither is a
                    // result this module may record.
                    continue;
                }
                unparsed += 1;
                continue;
            }
            if trimmed.starts_with("test result:") || trimmed.contains("has been running for over")
            {
                continue;
            }
            unparsed += 1;
            continue;
        }
    }

    // The failure summary is printed after the per-test lines. A block for a test whose `FAILED`
    // line was lost to the capture limit is still evidence that the test failed.
    for name in details.keys() {
        results.entry(name.clone()).or_insert(TestStatus::Failed);
    }

    let results: Vec<TestResult> = results
        .into_iter()
        .map(|(name, status)| TestResult {
            name,
            status,
            seconds: None,
        })
        .collect();
    let failures = results
        .iter()
        .filter(|result| result.status == TestStatus::Failed)
        .map(|result| {
            let detail = details.get(&result.name).cloned().unwrap_or_default();
            TestFailure {
                name: result.name.clone(),
                reason: first_reason(&detail),
                location: panic_location(&detail),
                detail,
            }
        })
        .collect();

    ParsedRun {
        results,
        failures,
        unparsed,
    }
}

/// The name in a `---- name stdout ----` failure block header.
fn block_header(line: &str) -> Option<String> {
    let rest = line.strip_prefix("---- ")?.trim_end();
    let name = rest
        .strip_suffix(" stdout ----")
        .or_else(|| rest.strip_suffix(" stderr ----"))?;
    let name = name.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_owned())
    }
}

fn status_of(outcome: &str) -> Option<TestStatus> {
    if outcome == "ok" || outcome.starts_with("ok ") {
        Some(TestStatus::Passed)
    } else if outcome.starts_with("FAILED") {
        Some(TestStatus::Failed)
    } else if outcome.starts_with("ignored") {
        Some(TestStatus::Skipped)
    } else {
        None
    }
}

/// The first line that explains a failure.
///
/// The harness's first line is `thread '…' panicked at path:line:col:`; the assertion message on
/// the next line is what a person needs, so it is preferred when present.
fn first_reason(detail: &str) -> String {
    let mut lines = detail
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());
    let Some(first) = lines.next() else {
        return "no detail was captured".to_owned();
    };
    let chosen = if first.starts_with("thread ") && first.contains("panicked at") {
        lines.next().unwrap_or(first)
    } else {
        first
    };
    truncate(chosen, MAX_REASON_CHARS)
}

/// The source position in a panic line, when there is one.
fn panic_location(detail: &str) -> Option<String> {
    let (_, after) = detail.split_once("panicked at ")?;
    let token = after.split_whitespace().next()?;
    let token = token.trim_end_matches(':');
    if token.contains(':') {
        Some(token.to_owned())
    } else {
        None
    }
}

/// The harness's own count line, `3 tests, 0 benchmarks` or `1 test, 0 benchmarks`.
///
/// Singular and plural both occur: the ignored-test listing is usually a single test, and counting
/// its summary as an unparsed line would mark a perfectly readable list as incomplete.
fn is_listing_summary(line: &str) -> bool {
    if !(line.ends_with("benchmark") || line.ends_with("benchmarks")) {
        return false;
    }
    let Some((count, rest)) = line.split_once(' ') else {
        return false;
    };
    !count.is_empty()
        && count.chars().all(|digit| digit.is_ascii_digit())
        && (rest.starts_with("test, ") || rest.starts_with("tests, "))
}

/// Parses `cargo test -- --list`.
fn parse_listing(text: &str, truncated: bool) -> Discovery {
    let mut discovery = Discovery {
        truncated,
        ..Discovery::default()
    };
    for raw in text.lines() {
        let mut line = raw.trim_end_matches('\r').trim();
        if line.is_empty() || line.starts_with("running ") {
            continue;
        }
        if is_listing_summary(line) {
            continue;
        }
        let mut ignored = false;
        if let Some(stripped) = line.strip_suffix(" ... ignored") {
            line = stripped.trim_end();
            ignored = true;
        }
        match line.rsplit_once(": ") {
            Some((name, "test")) if !name.trim().is_empty() => {
                discovery.tests.push(DiscoveredTest {
                    name: name.trim().to_owned(),
                    ignored,
                });
            }
            Some((_, "benchmark")) => discovery.benchmarks += 1,
            _ => discovery.unparsed += 1,
        }
    }
    discovery
}

fn exit_text(exit: Option<i32>) -> String {
    match exit {
        Some(code) => format!("code {code}"),
        None => "no exit code".to_owned(),
    }
}

/// The last useful lines of a stream, for an error message.
fn tail(text: &str) -> String {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let start = lines.len().saturating_sub(8);
    if lines.is_empty() {
        return "no diagnostics were captured".to_owned();
    }
    lines[start..].join(" | ")
}

fn truncate(text: &str, limit: usize) -> String {
    let mut text = text.to_owned();
    truncate_in_place(&mut text, limit);
    text
}

fn truncate_in_place(text: &mut String, limit: usize) {
    if text.chars().count() > limit {
        let cut = text
            .char_indices()
            .nth(limit)
            .map(|(index, _)| index)
            .unwrap_or(text.len());
        text.truncate(cut);
        text.push('…');
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use super::{
        Command, CommandOutput, CommandRunner, Selection, TestConfig, TestRunner, parse_listing,
        parse_run_output,
    };
    use crate::app::quality::report::{Center, Filter, Severity, Source, TestStatus, Tests};
    use crate::host::tasks::CancelToken;

    struct FakeInner {
        replies: Mutex<VecDeque<Result<CommandOutput, String>>>,
        seen: Mutex<Vec<Command>>,
    }

    /// A runner that answers with canned output and remembers what it was asked to run.
    #[derive(Clone)]
    struct FakeRunner {
        inner: Arc<FakeInner>,
    }

    impl FakeRunner {
        fn replying(replies: Vec<CommandOutput>) -> Self {
            Self {
                inner: Arc::new(FakeInner {
                    replies: Mutex::new(replies.into_iter().map(Ok).collect()),
                    seen: Mutex::new(Vec::new()),
                }),
            }
        }

        fn commands(&self) -> Vec<Command> {
            self.inner
                .seen
                .lock()
                .expect("the log is not poisoned")
                .clone()
        }
    }

    impl CommandRunner for FakeRunner {
        fn run(&self, command: &Command, _cancel: &CancelToken) -> Result<CommandOutput, String> {
            self.inner
                .seen
                .lock()
                .expect("the log is not poisoned")
                .push(command.clone());
            self.inner
                .replies
                .lock()
                .expect("the replies are not poisoned")
                .pop_front()
                .unwrap_or_else(|| Ok(CommandOutput::default()))
        }
    }

    fn output(stdout: &str, exit: i32) -> CommandOutput {
        CommandOutput {
            exit: Some(exit),
            stdout: stdout.to_owned(),
            ..CommandOutput::default()
        }
    }

    fn runner(replies: Vec<CommandOutput>) -> (TestRunner, FakeRunner) {
        let fake = FakeRunner::replying(replies);
        let runner = TestRunner::with_runner(
            TestConfig {
                directory: Some("demo".into()),
                ..TestConfig::default()
            },
            fake.clone(),
        );
        (runner, fake)
    }

    #[test]
    fn discovery_parses_a_listing_and_records_what_it_saw() {
        let (runner, fake) = runner(vec![
            output(
                "running 0 tests\napp::quality::report::tests::one: test\napp::quality::report::tests::two: test\n2 tests, 0 benchmarks\n",
                0,
            ),
            output(
                "app::quality::report::tests::two: test\n1 test, 0 benchmarks\n",
                0,
            ),
        ]);
        let mut tests = Tests::new();
        let discovery = runner.discover_into(&mut tests).expect("a listing");

        assert_eq!(discovery.tests.len(), 2);
        assert_eq!(discovery.tests[0].name, "app::quality::report::tests::one");
        assert_eq!(discovery.benchmarks, 0);
        assert_eq!(discovery.unparsed, 0);
        assert!(!discovery.truncated);
        assert_eq!(discovery.ignored(), 1);
        assert!(discovery.tests[1].ignored);
        assert!(discovery.incomplete.is_none());
        assert_eq!(tests.tally().get("not-run"), Some(&2));

        let commands = fake.commands();
        assert_eq!(commands.len(), 2);
        assert_eq!(commands[0].arguments, ["test", "--", "--list"]);
        assert_eq!(commands[0].directory.as_deref(), Some("demo".as_ref()));
        assert_eq!(
            commands[1].arguments,
            ["test", "--", "--list", "--ignored"],
            "the plain listing does not mark ignored tests, so they are listed separately"
        );
    }

    #[test]
    fn an_unreadable_ignored_listing_leaves_the_rest_of_the_list_in_place() {
        let (runner, _fake) = runner(vec![
            output("app::tests::one: test\napp::tests::two: test\n", 0),
            output("", 101),
        ]);
        let discovery = runner.discover().expect("a listing");

        assert_eq!(
            discovery.tests.len(),
            2,
            "the tests that were listed are kept"
        );
        let reason = discovery.incomplete.expect("the gap is named");
        assert!(
            reason.contains("which tests are ignored is unknown"),
            "{reason}"
        );
    }

    #[test]
    fn a_truncated_listing_is_not_reported_as_complete() {
        let (runner, _fake) = runner(vec![
            CommandOutput {
                exit: Some(0),
                stdout: "app::tests::one: test\n".to_owned(),
                truncated: true,
                ..CommandOutput::default()
            },
            output("", 0),
        ]);
        let discovery = runner.discover().expect("a listing");
        assert_eq!(discovery.tests.len(), 1);
        assert!(discovery.truncated);
    }

    #[test]
    fn a_listing_that_fails_to_compile_reports_the_diagnostics() {
        let (runner, fake) = runner(vec![CommandOutput {
            exit: Some(101),
            stderr: "error[E0425]: cannot find value `x` in this scope\n".to_owned(),
            ..CommandOutput::default()
        }]);
        let error = runner.discover().expect_err("a failed listing");
        assert!(error.contains("exited with code 101"), "{error}");
        assert!(error.contains("error[E0425]"), "{error}");
        assert_eq!(
            fake.commands().len(),
            1,
            "a listing that failed is not asked twice"
        );
    }

    #[test]
    fn a_filtered_run_records_passed_failed_and_ignored_separately() {
        let (runner, fake) = runner(vec![output(
            "running 3 tests\n\
             test app::quality::one ... ok\n\
             test app::quality::two ... FAILED\n\
             test app::quality::three ... ignored, needs a display\n\
             \n\
             failures:\n\
             \n\
             ---- app::quality::two stdout ----\n\
             thread 'app::quality::two' panicked at src/quality.rs:12:5:\n\
             assert_eq!(1, 2)\n\
             note: run with `RUST_BACKTRACE=1` for a backtrace\n\
             \n\
             failures:\n\
             \x20   app::quality::two\n\
             \n\
             test result: FAILED. 1 passed; 1 failed; 1 ignored; 0 measured; 0 filtered out; \
             finished in 0.01s\n",
            101,
        )]);
        let mut tests = Tests::new();
        let mut center = Center::new();
        let report = runner
            .run(
                &Selection::filter("app::quality"),
                4,
                "desktop",
                &mut tests,
                &mut center,
            )
            .expect("a run");

        assert_eq!(report.passed(), 1);
        assert_eq!(report.failed(), 1);
        assert_eq!(report.skipped(), 1);
        assert!(!report.cancelled);
        assert!(report.incomplete.is_none(), "{:?}", report.incomplete);
        assert_eq!(
            tests
                .results()
                .iter()
                .map(|r| (r.name.as_str(), r.status))
                .collect::<Vec<_>>(),
            vec![
                ("app::quality::one", TestStatus::Passed),
                ("app::quality::three", TestStatus::Skipped),
                ("app::quality::two", TestStatus::Failed),
            ]
        );

        let commands = fake.commands();
        assert_eq!(commands[0].arguments, ["test", "--", "app::quality"]);
        assert!(commands[0].arguments.iter().all(|arg| !arg.contains(' ')));
    }

    #[test]
    fn an_exact_selection_asks_for_each_name_and_nothing_else() {
        let (runner, fake) = runner(vec![output(
            "running 2 tests\ntest a ... ok\ntest b ... ok\ntest result: ok. 2 passed; 0 failed; \
             0 ignored; 0 measured; 4 filtered out; finished in 0.00s\n",
            0,
        )]);
        let mut tests = Tests::new();
        let mut center = Center::new();
        runner
            .run(
                &Selection::exact(["a", "b"]),
                1,
                "desktop",
                &mut tests,
                &mut center,
            )
            .expect("a run");

        let commands = fake.commands();
        assert_eq!(commands[0].arguments, ["test", "--", "--exact", "a", "b"]);
    }

    #[test]
    fn a_cancelled_run_leaves_the_unreported_tests_not_run() {
        let (runner, _fake) = runner(vec![CommandOutput {
            exit: None,
            stdout: "test a ... ok\n".to_owned(),
            cancelled: true,
            ..CommandOutput::default()
        }]);
        let mut tests = Tests::new();
        tests.note_discovered("a");
        tests.note_discovered("b");
        tests.note_discovered("c");
        let mut center = Center::new();
        let report = runner
            .run(
                &Selection::exact(["a", "b", "c"]),
                2,
                "desktop",
                &mut tests,
                &mut center,
            )
            .expect("a cancelled run");

        assert!(report.cancelled);
        assert_eq!(report.exit, None);
        assert_eq!(report.passed(), 1);
        assert_eq!(report.not_run, ["b".to_owned(), "c".to_owned()]);
        assert_eq!(tests.results()[0].status, TestStatus::Passed);
        assert_eq!(tests.results()[1].status, TestStatus::NotRun);
        assert_eq!(tests.results()[2].status, TestStatus::NotRun);
        assert!(center.matching(&Filter::default()).is_empty());
    }

    #[test]
    fn a_stop_request_before_the_run_starts_no_process() {
        let (runner, fake) = runner(Vec::new());
        let cancel = CancelToken::default();
        cancel.request();
        let mut tests = Tests::new();
        tests.note_discovered("c");
        let mut center = Center::new();
        let report = runner
            .run_cancellable(
                &Selection::exact(["a", "b"]),
                0,
                "desktop",
                &mut tests,
                &mut center,
                &cancel,
            )
            .expect("a cancelled run");

        assert!(report.cancelled);
        assert!(report.observed.is_empty());
        assert_eq!(
            report.not_run,
            ["a".to_owned(), "b".to_owned(), "c".to_owned()],
            "a test discovery already knew about is not run either"
        );
        assert!(!report.complete_for(&Selection::exact(["a", "b"])));
        assert!(fake.commands().is_empty(), "nothing was started");
    }

    #[test]
    fn a_failure_reaches_the_centre_with_its_source_revision_and_location() {
        let (runner, _fake) = runner(vec![output(
            "test suite::fails ... FAILED\n\
             \n\
             ---- suite::fails stdout ----\n\
             thread 'suite::fails' panicked at src/app/quality/report.rs:42:9:\n\
             the widget count differs\n\
             note: run with `RUST_BACKTRACE=1` for a backtrace\n\
             \n\
             test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out\n",
            101,
        )]);
        let mut tests = Tests::new();
        let mut center = Center::new();
        runner
            .run(&Selection::all(), 7, "desktop", &mut tests, &mut center)
            .expect("a run");

        let problems = center.matching(&Filter {
            source: Some(Source::Test),
            ..Filter::default()
        });
        assert_eq!(problems.len(), 1);
        let problem = problems[0];
        assert_eq!(problem.target, "desktop");
        assert_eq!(problem.revision, 7);
        assert_eq!(problem.severity, Severity::Error);
        assert_eq!(
            problem.location.as_deref(),
            Some("src/app/quality/report.rs:42:9")
        );
        assert!(
            problem.message.contains("suite::fails"),
            "{}",
            problem.message
        );
        assert!(
            problem.message.contains("the widget count differs"),
            "{}",
            problem.message
        );
    }

    #[test]
    fn rerun_failed_reruns_only_the_failed_tests() {
        let (runner, fake) = runner(vec![output(
            "test b ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered \
             out\n",
            0,
        )]);
        let mut tests = Tests::new();
        tests.record("a", TestStatus::Passed, Some(0.1));
        tests.record("b", TestStatus::Failed, None);
        tests.record("c", TestStatus::Skipped, None);
        let mut center = Center::new();
        let report = runner
            .rerun_failed(8, "desktop", &mut tests, &mut center)
            .expect("a rerun");

        assert_eq!(report.passed(), 1);
        let commands = fake.commands();
        assert_eq!(commands[0].arguments, ["test", "--", "--exact", "b"]);
        assert_eq!(tests.results()[0].status, TestStatus::Passed);
        assert_eq!(tests.results()[1].status, TestStatus::Passed);
        assert_eq!(tests.results()[2].status, TestStatus::Skipped);
    }

    #[test]
    fn a_run_that_exits_without_a_parsable_failure_is_not_a_pass() {
        let (runner, _fake) = runner(vec![CommandOutput {
            exit: Some(101),
            stderr: "error: could not compile `demo`\n".to_owned(),
            ..CommandOutput::default()
        }]);
        let mut tests = Tests::new();
        let mut center = Center::new();
        let report = runner
            .run(&Selection::all(), 1, "desktop", &mut tests, &mut center)
            .expect("a run");

        assert!(!report.cancelled);
        assert_eq!(report.passed(), 0);
        assert!(report.observed.is_empty());
        let note = report.incomplete.expect("a limitation is reported");
        assert!(note.contains("not a pass"), "{note}");
    }

    #[test]
    fn discovery_does_not_forget_an_existing_result() {
        let (runner, _fake) = runner(vec![output("a: test\nb: test\n", 0), output("", 0)]);
        let mut tests = Tests::new();
        tests.record("a", TestStatus::Passed, Some(0.2));
        runner.discover_into(&mut tests).expect("a listing");

        assert_eq!(tests.results()[0].status, TestStatus::Passed);
        assert_eq!(tests.results()[1].status, TestStatus::NotRun);
    }

    #[test]
    fn listing_parses_ignored_benchmarks_and_noise_conservatively() {
        let discovery = parse_listing(
            "running 0 tests\n\
             module::plain: test\n\
             module::ignored: test ... ignored\n\
             module::bench: benchmark\n\
             2 tests, 1 benchmarks\n\
             1 test, 0 benchmarks\n\
             not a listing line\n",
            false,
        );
        assert_eq!(discovery.tests.len(), 2);
        assert!(!discovery.tests[0].ignored);
        assert!(discovery.tests[1].ignored);
        assert_eq!(discovery.benchmarks, 1);
        assert_eq!(discovery.unparsed, 1);
    }

    #[test]
    fn a_result_line_without_a_result_is_not_recorded() {
        let parsed = parse_run_output(
            "running 1 test\n\
             test suite::odd ... some new harness state\n\
             test result: ok. 0 passed; 0 failed; 0 ignored\n\
             ---- suite::lost stdout ----\n\
             thread 'suite::lost' panicked at src/lib.rs:3:1:\n\
             boom\n",
        );
        assert_eq!(parsed.results.len(), 1);
        assert_eq!(parsed.results[0].name, "suite::lost");
        assert_eq!(parsed.results[0].status, TestStatus::Failed);
        assert_eq!(parsed.unparsed, 1);
    }

    #[test]
    fn a_test_s_own_output_cannot_claim_another_test_ran() {
        let parsed = parse_run_output(
            "test suite::noisy ... FAILED\n\
             \n\
             ---- suite::noisy stdout ----\n\
             test suite::victim ... ok\n\
             the real assertion message\n\
             \n\
             failures:\n\
             \x20   suite::noisy\n",
        );
        assert_eq!(parsed.results.len(), 1);
        assert_eq!(parsed.results[0].name, "suite::noisy");
        assert_eq!(parsed.results[0].status, TestStatus::Failed);
        assert!(
            parsed.failures[0]
                .detail
                .contains("test suite::victim ... ok")
        );
    }

    #[test]
    fn a_complete_pass_asks_about_every_test_it_named() {
        let (runner, _fake) = runner(vec![
            output(
                "test a ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 1 filtered out\n",
                0,
            ),
            output(
                "test a ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 1 filtered out\n",
                0,
            ),
            output(
                "test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n",
                0,
            ),
        ]);
        let mut tests = Tests::new();
        let mut center = Center::new();

        // A whole-suite run where a known test reported nothing is not a pass.
        tests.note_discovered("a");
        tests.note_discovered("b");
        let report = runner
            .run(&Selection::all(), 0, "desktop", &mut tests, &mut center)
            .expect("a run");
        assert!(!report.complete_for(&Selection::all()));
        assert_eq!(report.not_run, ["b".to_owned()]);

        // An exact selection asks only about its own names, so an unrelated test that never ran is
        // not a reason to call it incomplete.
        let report = runner
            .run(
                &Selection::exact(["a"]),
                0,
                "desktop",
                &mut tests,
                &mut center,
            )
            .expect("a run");
        assert!(report.complete_for(&Selection::exact(["a"])));
        assert!(!report.complete_for(&Selection::exact(["a", "b"])));

        // Nothing printed at all: not a pass, however the command exited.
        let report = runner
            .run(
                &Selection::exact(["gone"]),
                0,
                "desktop",
                &mut tests,
                &mut center,
            )
            .expect("a run");
        assert!(!report.complete_for(&Selection::exact(["gone"])));
    }

    #[test]
    fn the_runner_does_not_mutate_the_project_directory() {
        use std::sync::atomic::{AtomicU64, Ordering};

        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "rustage-test-runner-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&directory).expect("a temporary directory");
        std::fs::write(directory.join("marker.txt"), "unchanged").expect("a marker file");

        let fake = FakeRunner::replying(vec![
            output("suite::one: test\n", 0),
            output("", 0),
            output(
                "test suite::one ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored\n",
                0,
            ),
        ]);
        let runner = TestRunner::with_runner(TestConfig::in_directory(&directory), fake);
        let mut tests = Tests::new();
        let mut center = Center::new();
        runner.discover_into(&mut tests).expect("a listing");
        runner
            .run(&Selection::all(), 1, "desktop", &mut tests, &mut center)
            .expect("a run");

        let entries: Vec<String> = std::fs::read_dir(&directory)
            .expect("the directory")
            .map(|entry| {
                entry
                    .expect("an entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(entries, ["marker.txt"]);
        assert_eq!(
            std::fs::read_to_string(directory.join("marker.txt")).expect("the marker"),
            "unchanged"
        );
        std::fs::remove_dir_all(&directory).expect("cleanup");
    }
}
