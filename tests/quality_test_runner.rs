// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! T21.04 end to end: the `--quality test-*` operations against a real `cargo` project.
//!
//! The module tests drive the parser with canned output. These tests do the thing the roadmap
//! asked for instead: they write a minimal std-only cargo project to a temporary directory, run the
//! built binary against it, and check what the command line actually printed and exited with.
//! Nothing here depends on the rustage crate's own test suite, so a machine that can run the tests
//! can rerun this evidence:
//!
//! ```text
//! cargo test --test quality_test_runner
//! ```
//!
//! Each fixture's build goes to `CARGO_TARGET_DIR` inside the fixture. That is not tidiness: an
//! inherited `CARGO_TARGET_DIR` would make the inner cargo wait for the build-directory lock the
//! outer `cargo test` is holding, and the two would wait for each other.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

/// How long one rustage invocation may take before the test calls it hung.
///
/// The cancel case stops a test that would sleep for 30 s, so a working cancellation finishes well
/// inside this; a hung one fails here instead of stalling the suite.
const LIMIT: Duration = Duration::from_secs(120);

/// A project with one passing, one failing and one ignored test.
const MIXED: &str = r#"
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

#[cfg(test)]
mod tests {
    #[test]
    fn passing() {
        assert_eq!(super::add(1, 2), 3);
    }

    #[test]
    fn failing() {
        assert_eq!(super::add(1, 2), 4, "one plus two is not four");
    }

    #[test]
    #[ignore]
    fn ignored_one() {
        panic!("an ignored test must not run");
    }
}
"#;

/// A project whose only test outlives any sensible deadline.
const SLOW: &str = r#"
#[cfg(test)]
mod tests {
    #[test]
    fn slow() {
        std::thread::sleep(std::time::Duration::from_secs(30));
    }
}
"#;

/// A project that does not compile, so the tests cannot even be listed.
const BROKEN: &str = r#"
pub fn broken() -> i32 {
    "not a number"
}
"#;

/// Writes a minimal cargo project and returns its directory.
fn project(root: &Path, name: &str, source: &str) -> PathBuf {
    let directory = root.join(name);
    fs::create_dir_all(directory.join("src")).expect("the fixture source directory");
    fs::write(
        directory.join("Cargo.toml"),
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
             [workspace]\n\n[dependencies]\n"
        ),
    )
    .expect("the fixture manifest");
    fs::write(directory.join("src/lib.rs"), source).expect("the fixture source");
    directory
}

/// A fresh temporary root for one test, removed when the test ends.
struct Root(PathBuf);

impl Root {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("rustage-t21-04-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("a temporary root");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Waits for `child`, killing it and failing the test if it outlives [`LIMIT`].
fn wait_within(child: &mut Child, what: &str) -> std::process::ExitStatus {
    let deadline = Instant::now() + LIMIT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("{what} outlived {LIMIT:?}");
            }
            Err(error) => panic!("{what}: its status could not be read: {error}"),
        }
    }
}

/// Runs `rustage --quality <ops>` against `project`, bounded, and returns its output and code.
fn quality(project: &Path, ops: &[String]) -> (Output, i32) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rustage"));
    command.arg("--quality").args(ops);
    // The inner cargo must not contend with the outer `cargo test` build lock.
    command.env("CARGO_TARGET_DIR", project.join("target"));
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .unwrap_or_else(|error| panic!("rustage could not be started: {error}"));
    let status = wait_within(&mut child, "rustage");
    let output = child
        .wait_with_output()
        .expect("the command ended but its output could not be read");
    let code = status.code().unwrap_or(-1);
    (output, code)
}

/// The text a person would read, from both streams.
fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn contains(output: &Output, needle: &str) {
    let text = text(output);
    assert!(
        text.contains(needle),
        "expected {needle:?} in:\n{text}\n---"
    );
}

fn lacks(output: &Output, needle: &str) {
    let text = text(output);
    assert!(
        !text.contains(needle),
        "did not expect {needle:?} in:\n{text}\n---"
    );
}

/// The whole command line path over a real project: discovery, a failing run, a selection, a
/// re-run, and the problem centre.
#[test]
fn a_real_cargo_project_reports_pass_fail_ignored_and_not_run() {
    let root = Root::new("mixed");
    let directory = project(root.path(), "mixed", MIXED);
    let at = directory.to_string_lossy().into_owned();

    // Discovery is real: libtest's plain listing does not mark ignored tests, so the runner has to
    // ask twice, and this is the evidence that it does.
    let (output, code) = quality(&directory, &[format!("test-discover:{at}")]);
    assert_eq!(code, 0, "discovery is not a run: {}", text(&output));
    contains(&output, "discovered 3 test(s), 1 ignored, 0 benchmark(s)");
    lacks(&output, "not understood");

    // A whole-suite run: the three statuses are told apart, and the failure is not a pass.
    let (output, code) = quality(&directory, &[format!("test-run:{at}")]);
    assert_eq!(code, 4, "a failing suite must not exit like a pass");
    contains(&output, "tests: 1 passed, 1 failed, 1 skipped, 0 not run");
    contains(&output, "test tests::passing: passed");
    contains(&output, "test tests::ignored_one: skipped");
    contains(&output, "test tests::failing: failed");
    contains(&output, "failed tests::failing at src/lib.rs:");

    // The failure is in the problem centre at the revision and target the command was told.
    let (output, code) = quality(
        &directory,
        &[
            "revision:7".to_owned(),
            "target:linux".to_owned(),
            format!("test-run:{at}"),
            "state".to_owned(),
        ],
    );
    assert_eq!(code, 4, "a reported problem is not success");
    contains(&output, "[error] test linux@r7 src/lib.rs:");
    contains(&output, "test tests::failing failed:");

    // An exact selection of a passing test is a pass, and only that test is reported.
    let (output, code) = quality(&directory, &[format!("test-select:{at}:tests::passing")]);
    assert_eq!(code, 0, "{}", text(&output));
    contains(&output, "tests: 1 passed, 0 failed, 0 skipped, 0 not run");
    lacks(&output, "tests::ignored_one");

    // A name that does not exist: cargo exits zero, and the run is still not a pass.
    let (output, code) = quality(&directory, &[format!("test-select:{at}:tests::missing")]);
    assert_eq!(code, 4, "a test that did not run is not a pass");
    contains(&output, "tests: 0 passed, 0 failed, 0 skipped, 1 not run");
    contains(&output, "not run: tests::missing");

    // Re-running asks for the failed test and nothing else.
    let (output, code) = quality(
        &directory,
        &[format!("test-run:{at}"), format!("test-rerun:{at}")],
    );
    assert_eq!(code, 4, "the re-run still fails");
    contains(&output, "tests: 0 passed, 1 failed, 0 skipped, 0 not run");
}

/// A project that cannot compile is an error, not an empty passing suite.
#[test]
fn a_project_that_does_not_compile_is_reported_as_an_error() {
    let root = Root::new("broken");
    let directory = project(root.path(), "broken", BROKEN);
    let at = directory.to_string_lossy().into_owned();

    let (output, code) = quality(&directory, &[format!("test-discover:{at}")]);
    assert_eq!(code, 4, "{}", text(&output));
    contains(&output, "exited with code 101");
}

/// The cancellation case, on a real process: a test that would sleep for 30 s is stopped, and it is
/// left not run rather than reported as anything else.
#[test]
fn a_cancelled_real_run_leaves_the_test_not_run() {
    let root = Root::new("slow");
    let directory = project(root.path(), "slow", SLOW);
    let at = directory.to_string_lossy().into_owned();

    let began = Instant::now();
    let (output, code) = quality(
        &directory,
        &[
            format!("test-discover:{at}"),
            "test-cancel:1".to_owned(),
            format!("test-run:{at}"),
        ],
    );
    let elapsed = began.elapsed();

    assert_eq!(code, 4, "a cancelled run must not exit like a pass");
    contains(&output, "discovered 1 test(s), 0 ignored, 0 benchmark(s)");
    contains(&output, "cancelled: tests without a result stay not run");
    contains(&output, "not run: tests::slow");
    lacks(&output, "tests::slow: passed");
    assert!(
        elapsed < Duration::from_secs(25),
        "the 30 s test was not stopped: the invocation took {elapsed:?}"
    );
}

/// A stop requested before the run starts no process, so every selected test stays not run.
#[test]
fn a_stop_before_the_start_starts_nothing() {
    let root = Root::new("stop");
    let directory = project(root.path(), "stop", MIXED);
    let at = directory.to_string_lossy().into_owned();

    let (output, code) = quality(
        &directory,
        &[
            format!("test-discover:{at}"),
            "test-cancel".to_owned(),
            format!("test-run:{at}"),
        ],
    );
    assert_eq!(code, 4, "{}", text(&output));
    contains(&output, "cancelled: tests without a result stay not run");
    contains(&output, "tests: 0 passed, 0 failed, 0 skipped, 3 not run");
    contains(
        &output,
        "not run: tests::failing, tests::ignored_one, tests::passing",
    );
    lacks(&output, "test tests::passing: passed");
}
