// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The workbench's tests panel (`panel.tests`, task F21).
//!
//! The panel is a view over a **real** cargo run: a worker thread lists the tests with
//! `cargo test -- --list`, then runs the whole selection through the existing
//! [`TestRunner`], and the window polls the snapshot channel and mirrors the last report into a
//! list. Nothing here invents a result: the discovery row is built from [`Discovery`] and the run
//! rows from [`RunReport`], so a test that reported no result stays "not run" rather than
//! becoming a pass.
//!
//! The worker sends at most three reports: `discovered`, then `done`; or `failed` in place of
//! either. The GUI never blocks on cargo.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};

use crate::app::quality::{
    Center, Discovery, RunReport, Selection, TestConfig, TestFailure, TestResult, TestRunner, Tests,
};
use crate::host::tasks::CancelToken;

/// The most detail rows one report carries.
///
/// A suite can have thousands of tests, and a panel that grows without end is a memory leak with a
/// scrollbar; everything past the limit is counted in an `omitted: N` row.
const ROW_LIMIT: usize = 200;

/// One report from the worker: the phase it reached, the one-line status and the panel rows.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct Update {
    /// `discovered`, `done` or `failed`.
    pub phase: &'static str,
    /// The one-line status.
    pub status: String,
    /// The rows the panel shows.
    pub rows: Vec<String>,
    /// The tests that failed in a finished run, so a rerun can name exactly those.
    pub failures: Vec<String>,
}

/// The panel's view of one run and the channel that connects it to the worker.
pub(super) struct TestsState {
    rows: Vec<String>,
    updates: Receiver<Update>,
    live: bool,
    /// The stop flag of the run in flight; a fresh one is made per run.
    cancel: CancelToken,
    /// The failed tests of the last finished run, by exact harness name.
    failures: Vec<String>,
}

impl TestsState {
    /// No run yet.
    pub(super) fn new() -> Self {
        let (_sender, updates) = channel();
        Self {
            rows: vec!["no test run yet; press Run tests".to_owned()],
            updates,
            live: false,
            cancel: CancelToken::default(),
            failures: Vec::new(),
        }
    }

    /// Starts discovery and a run of `selection` on a worker thread.
    ///
    /// Returns `false` when a worker is already live, because two cargo runs writing the same
    /// target directory would report each other's results.
    pub(super) fn start(&mut self, root: PathBuf, selection: Selection) -> bool {
        if self.live {
            return false;
        }
        let (updates, updates_rx) = channel();
        self.updates = updates_rx;
        self.live = true;
        self.failures.clear();
        self.cancel = CancelToken::default();
        let cancel = self.cancel.clone();
        self.rows = vec!["discovering and running cargo tests…".to_owned()];
        std::thread::spawn(move || {
            let _ = run_once(root, selection, cancel, &updates);
        });
        true
    }

    /// Asks the live run to stop; `false` when nothing is running.
    pub(super) fn stop(&mut self) -> bool {
        if !self.live {
            return false;
        }
        self.cancel.request();
        true
    }

    /// The failed tests of the last finished run.
    pub(super) fn failed_names(&self) -> Vec<String> {
        self.failures.clone()
    }

    /// Drains the worker's reports and returns the last one.
    pub(super) fn poll(&mut self) -> Option<Update> {
        let mut last = None;
        loop {
            match self.updates.try_recv() {
                Ok(update) => {
                    if matches!(update.phase, "done" | "failed") {
                        self.live = false;
                        self.failures = update.failures.clone();
                    }
                    last = Some(update);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.live = false;
                    break;
                }
            }
        }
        if let Some(update) = &last {
            self.rows = update.rows.clone();
        }
        last
    }

    /// The rows the list currently shows.
    pub(super) fn rows(&self) -> Vec<String> {
        self.rows.clone()
    }

    /// Whether a worker is still running.
    pub(super) fn is_live(&self) -> bool {
        self.live
    }

    /// Marks the state live without starting a worker, so the refusal path is testable.
    #[cfg(test)]
    fn mark_live(&mut self) {
        self.live = true;
    }
}

/// Discovers the tests, then runs the whole selection, reporting each step.
///
/// The body is separate from the thread wrapper so its structure can be read (and eventually
/// tested) without a worker. Every error is also sent as a `failed` report; the returned reason is
/// for the caller and is deliberately the same text the panel shows.
fn run_once(
    root: PathBuf,
    selection: Selection,
    cancel: CancelToken,
    updates: &Sender<Update>,
) -> Result<(), String> {
    let config = TestConfig::in_directory(root);
    let runner = TestRunner::new(config);

    let discovery = match runner.discover() {
        Ok(discovery) => discovery,
        Err(reason) => return Err(failure(updates, reason)),
    };
    let discovered = Update {
        phase: "discovered",
        status: format!("discovered {} test(s)", discovery.tests.len()),
        rows: vec![discovery_row(&discovery)],
        failures: Vec::new(),
    };
    if updates.send(discovered).is_err() {
        // Nobody is listening: the panel is gone, so there is nothing left to run for.
        return Ok(());
    }

    let mut tests = Tests::new();
    let mut center = Center::new();
    let report =
        match runner.run_cancellable(&selection, 1, "desktop", &mut tests, &mut center, &cancel) {
            Ok(report) => report,
            Err(reason) => return Err(failure(updates, reason)),
        };

    let failures = report
        .failures
        .iter()
        .map(|failure| failure.name.clone())
        .collect();
    let _ = updates.send(Update {
        phase: "done",
        status: format!(
            "tests: {} passed, {} failed",
            report.passed(),
            report.failed()
        ),
        rows: report_rows(&report),
        failures,
    });
    Ok(())
}

/// Sends a `failed` report and returns the reason, keeping the worker body fallible.
fn failure(updates: &Sender<Update>, reason: String) -> String {
    let _ = updates.send(Update {
        phase: "failed",
        status: reason.clone(),
        rows: vec![reason.clone()],
        failures: Vec::new(),
    });
    reason
}

/// The discovery row: what the listing named, and why it may be incomplete.
fn discovery_row(discovery: &Discovery) -> String {
    let mut row = format!(
        "discovered {} test(s); {} ignored; {} unparsed line(s)",
        discovery.tests.len(),
        discovery.ignored(),
        discovery.unparsed,
    );
    if discovery.truncated {
        row.push_str("; listing truncated");
    }
    if let Some(reason) = &discovery.incomplete {
        row.push_str(&format!("; incomplete: {reason}"));
    }
    row
}

/// The rows one run report becomes: the summary, the failures, then every observed test.
fn report_rows(report: &RunReport) -> Vec<String> {
    let mut details = Vec::new();
    for failure in &report.failures {
        details.push(failure_row(failure));
    }
    for result in &report.observed {
        details.push(result_row(result));
    }

    let omitted = details.len().saturating_sub(ROW_LIMIT);
    let mut rows = Vec::with_capacity(details.len().min(ROW_LIMIT) + 2);
    rows.push(summary_row(report));
    rows.extend(details.into_iter().take(ROW_LIMIT));
    if omitted > 0 {
        rows.push(format!("omitted: {omitted}"));
    }
    rows
}

/// The run summary: the tally, the exit, whether it was cancelled, and any incompleteness.
fn summary_row(report: &RunReport) -> String {
    let exit = match report.exit {
        Some(code) => code.to_string(),
        None => "none".to_owned(),
    };
    let mut row = format!(
        "run: {} passed, {} failed, {} skipped, {} not run; exit {}; {}",
        report.passed(),
        report.failed(),
        report.skipped(),
        report.not_run.len(),
        exit,
        if report.cancelled {
            "cancelled"
        } else {
            "not cancelled"
        },
    );
    if report.truncated {
        row.push_str("; output truncated");
    }
    if let Some(reason) = &report.incomplete {
        row.push_str(&format!("; incomplete: {reason}"));
    }
    row
}

/// One failure row: the test, its first reason and, when known, the panic location.
fn failure_row(failure: &TestFailure) -> String {
    match &failure.location {
        Some(location) => format!(
            "failure: {}: {} at {location}",
            failure.name, failure.reason
        ),
        None => format!("failure: {}: {}", failure.name, failure.reason),
    }
}

/// One observed test row: its status, name and timing when the harness reported one.
fn result_row(result: &TestResult) -> String {
    let timing = match result.seconds {
        Some(seconds) => format!("{seconds:.2}s"),
        None => "no timing".to_owned(),
    };
    format!("{}: {} ({timing})", result.status.name(), result.name)
}

#[cfg(test)]
mod tests {
    use super::{TestsState, discovery_row, report_rows, summary_row};
    use crate::app::quality::{
        DiscoveredTest, Discovery, RunReport, Selection, TestFailure, TestResult, TestStatus,
    };
    use std::path::PathBuf;

    #[test]
    fn a_new_state_has_no_run() {
        let state = TestsState::new();
        assert!(!state.is_live());
        assert_eq!(
            state.rows(),
            vec!["no test run yet; press Run tests".to_owned()]
        );
    }

    #[test]
    fn start_refuses_a_second_worker_while_live() {
        let mut state = TestsState::new();
        state.mark_live();
        assert!(!state.start(PathBuf::from("."), Selection::all()));
        assert!(state.is_live());
    }

    #[test]
    fn a_live_run_can_be_stopped_and_a_finished_one_says_so() {
        let mut state = TestsState::new();
        assert!(!state.stop(), "nothing is running yet");
        state.mark_live();
        assert!(state.stop(), "a live run accepts a stop request");
    }

    #[test]
    fn a_new_state_has_no_failed_tests_to_rerun() {
        assert!(TestsState::new().failed_names().is_empty());
    }

    #[test]
    fn the_summary_row_names_the_counts() {
        let report = RunReport {
            cancelled: true,
            exit: Some(101),
            observed: vec![
                TestResult {
                    name: "tests::a".to_owned(),
                    status: TestStatus::Passed,
                    seconds: Some(0.25),
                },
                TestResult {
                    name: "tests::b".to_owned(),
                    status: TestStatus::Failed,
                    seconds: Some(1.5),
                },
                TestResult {
                    name: "tests::c".to_owned(),
                    status: TestStatus::Skipped,
                    seconds: None,
                },
            ],
            not_run: vec!["tests::d".to_owned()],
            truncated: true,
            failures: vec![TestFailure {
                name: "tests::b".to_owned(),
                reason: "assertion failed".to_owned(),
                detail: "left: 1, right: 2".to_owned(),
                location: Some("src/lib.rs:9:5".to_owned()),
            }],
            incomplete: Some("one output line was not understood".to_owned()),
        };

        let row = summary_row(&report);
        assert!(!row.is_empty());
        assert!(row.contains("1 passed"));
        assert!(row.contains("1 failed"));
        assert!(row.contains("1 skipped"));
        assert!(row.contains("1 not run"));
        assert!(row.contains("exit 101"));
        assert!(row.contains("cancelled"));
        assert!(row.contains("one output line was not understood"));

        let rows = report_rows(&report);
        assert!(!rows.is_empty());
        for row in rows {
            assert!(!row.is_empty(), "a panel row must not be empty");
        }
    }

    #[test]
    fn a_discovery_row_names_what_was_listed() {
        let discovery = Discovery {
            tests: vec![
                DiscoveredTest {
                    name: "tests::a".to_owned(),
                    ignored: false,
                },
                DiscoveredTest {
                    name: "tests::b".to_owned(),
                    ignored: true,
                },
            ],
            benchmarks: 1,
            truncated: true,
            unparsed: 2,
            incomplete: Some("the ignored listing failed".to_owned()),
        };

        let row = discovery_row(&discovery);
        assert!(!row.is_empty());
        assert!(row.contains("2 test"));
        assert!(row.contains("1 ignored"));
        assert!(row.contains("2 unparsed"));
        assert!(row.contains("listing truncated"));
        assert!(row.contains("the ignored listing failed"));
    }
}
