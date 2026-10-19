// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The process boundary, exercised for real.
//!
//! The unit tests drive [`Worker`] over in-memory buffers; these two tests start the binary and talk
//! to it through pipes. They are the evidence that the T17.02/T17.06 channel is a process boundary
//! rather than a pair of function calls with a process-shaped name.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Output, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

/// How long any one step may take before the test calls the process hung.
const LIMIT: Duration = Duration::from_secs(20);

/// Reads one line from a child, bounded, so a hung worker fails the test instead of hanging it.
fn line(reader: &Receiver<String>, what: &str) -> String {
    reader
        .recv_timeout(LIMIT)
        .unwrap_or_else(|_| panic!("the worker did not answer {what} within {LIMIT:?}"))
}

/// Waits for a child, killing it and failing if it outlives the limit.
fn wait_within(child: &mut Child, limit: Duration) -> std::process::ExitStatus {
    let deadline = Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => {
                let _ = child.kill();
                panic!("the process outlived {limit:?}");
            }
            Err(error) => panic!("the worker's status could not be read: {error}"),
        }
    }
}

/// Waits for a child with the test limit.
fn wait(child: &mut Child) -> std::process::ExitStatus {
    wait_within(child, LIMIT)
}

/// Runs a command to completion, bounded, and collects its output.
fn run_bounded(mut command: Command, what: &str, limit: Duration) -> Output {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("{what} could not be started: {error}"));
    let status = wait_within(&mut child, limit);
    let mut output = child
        .wait_with_output()
        .unwrap_or_else(|error| panic!("{what} ended but its output could not be read: {error}"));
    output.status = status;
    output
}

/// The payload of an encoded frame, decoded from its hex field.
fn payload(encoded: &str) -> String {
    let hex = encoded.rsplit('|').next().expect("a payload field");
    assert_eq!(hex.len() % 2, 0, "odd hex payload in {encoded:?}");
    let bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).expect("hex"))
        .collect();
    String::from_utf8(bytes).expect("text")
}

#[test]
fn a_worker_process_serves_the_typed_channel_over_stdio() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustage"))
        .args([
            "--logic-worker",
            "name:orders",
            "version:0.3.0",
            "initial:idle",
            "transition:idle:go:running",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the worker starts");
    let mut stdin = child.stdin.take().expect("a writable stdin");
    let stdout = child.stdout.take().expect("a readable stdout");
    let (tx, lines) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            match line {
                Ok(line) => {
                    if tx.send(line).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    writeln!(stdin, "v1|handshake|logic-1|7|").expect("a handshake");
    stdin.flush().expect("a flush");
    let manifest = line(&lines, "the handshake");
    assert!(manifest.starts_with("v1|manifest|logic-1|7|"), "{manifest}");
    assert!(payload(&manifest).contains("name=orders"));

    writeln!(stdin, "v1|request|logic-1|7|6576656e7420676f").expect("an event"); // "event go"
    stdin.flush().expect("a flush");
    let answer = line(&lines, "the event");
    assert!(payload(&answer).starts_with("state=running"), "{answer}");

    writeln!(stdin, "v1|request|logic-1|7|71756974").expect("quit"); // "quit"
    stdin.flush().expect("a flush");
    assert_eq!(payload(&line(&lines, "quit")), "bye");

    drop(stdin);
    let status = wait(&mut child);
    assert!(status.success(), "{status:?}");
}

/// Builds the candidate fixture once into a shared test target and answers its artifact.
///
/// The two candidate tests may run in parallel; Cargo serialises builds to the same target
/// directory, and the second call is a no-op. The directory is left in `target/` (ignored) so a
/// concurrent test cannot delete the artifact another one is about to run.
fn candidate_binary() -> std::path::PathBuf {
    let target = std::path::Path::new("target").join("logic-candidate-tests");
    let mut build = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned()));
    build.args([
        "build",
        "--quiet",
        "--manifest-path",
        "fixtures/logic-candidate/Cargo.toml",
        "--target-dir",
        &target.to_string_lossy(),
        "--bin",
        "logic-candidate",
    ]);
    let built = run_bounded(build, "the candidate build", Duration::from_secs(180));
    assert!(
        built.status.success(),
        "candidate build failed: {}",
        String::from_utf8_lossy(&built.stderr)
    );
    let artifact = target
        .join("debug")
        .join(format!("logic-candidate{}", std::env::consts::EXE_SUFFIX));
    assert!(
        artifact.is_file(),
        "the artifact {} is missing",
        artifact.display()
    );
    artifact
}

#[test]
fn a_built_candidate_serves_as_the_second_generation() {
    let artifact = candidate_binary();

    let mut driver = Command::new(env!("CARGO_BIN_EXE_rustage"));
    driver.args([
        "--logic-process",
        "session:orders",
        "name:orders",
        "version:0.1.0",
        "generation:1",
        "new-generation:2",
        "first:idle",
        "transition:idle:go:running",
        "first-set:idle:go:count=number:3",
        "first-event:go",
        "second-event:approve",
        &format!("second-binary:{}", artifact.display()),
        "preserve",
        "expect:running",
        "expect:done",
        "check",
    ]);
    let driven = run_bounded(driver, "the two-generation walk", Duration::from_secs(60));
    let stdout = String::from_utf8_lossy(&driven.stdout);
    let stderr = String::from_utf8_lossy(&driven.stderr);
    assert!(
        driven.status.success(),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("generation 1 go: running [count=number:3]"),
        "{stdout}"
    );
    assert!(
        stdout.contains("snapshot generation 1: running (1 field(s))"),
        "{stdout}"
    );
    assert!(
        stdout.contains("restored generation 2: running (1 field(s))"),
        "{stdout}"
    );
    assert!(
        stdout.contains("generation 2 approve: done [count=number:3]"),
        "{stdout}"
    );
    assert!(
        stdout.contains("the stale generation is refused"),
        "{stdout}"
    );
}

#[test]
fn a_schema_migration_carries_a_retyped_field_and_adds_a_default() {
    let mut driver = Command::new(env!("CARGO_BIN_EXE_rustage"));
    driver.args([
        "--logic-process",
        "session:orders",
        "name:orders",
        "version:0.1.0",
        "generation:1",
        "new-generation:2",
        "first:idle",
        "transition:idle:go:running",
        "first-set:idle:go:count=number:3",
        "first-event:go",
        "second-transition:running:approve:done",
        "second-event:approve",
        "preserve",
        "from-version:1",
        "to-version:2",
        "first-schema:count:number:1",
        "second-schema:count:text:1",
        "second-schema:note:text:2",
        "expect:running",
        "expect:done",
        "check",
    ]);
    let output = run_bounded(driver, "the schema migration walk", Duration::from_secs(20));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(stdout.contains("migration v1 -> v2"), "{stdout}");
    assert!(stdout.contains("count: retyped number -> text"), "{stdout}");
    assert!(stdout.contains("note: added"), "{stdout}");
    assert!(
        stdout.contains("migrated generation 1 -> 2: 2 field(s) carried"),
        "{stdout}"
    );
    assert!(
        stdout.contains("restored generation 2: running (2 field(s))"),
        "{stdout}"
    );
    assert!(
        stdout.contains("generation 2 approve: done [count=text:3, note=text:]"),
        "{stdout}"
    );
}

#[test]
fn a_lossy_schema_migration_blocks_the_switch_and_the_old_generation_keeps_serving() {
    let mut driver = Command::new(env!("CARGO_BIN_EXE_rustage"));
    driver.args([
        "--logic-process",
        "name:orders",
        "generation:1",
        "new-generation:2",
        "first:idle",
        "transition:idle:go:running",
        "first-set:idle:go:count=number:3",
        "first-event:go",
        "preserve",
        "from-version:1",
        "to-version:2",
        "first-schema:count:number:1",
        "second-schema:mode:text:2",
        "check",
    ]);
    let output = run_bounded(
        driver,
        "the blocked migration walk",
        Duration::from_secs(20),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(4), "stdout:\n{stdout}");
    assert!(
        stdout.contains("count: dropped (v2 has no field count; value lost)"),
        "{stdout}"
    );
    assert!(
        stdout.contains("generation 1 still serving: running"),
        "{stdout}"
    );
    assert!(
        stdout.contains("the automatic switch is blocked: the migration would lose count"),
        "{stdout}"
    );
    assert!(
        !stdout.contains("new generation 2:"),
        "the new process must not start:\n{stdout}"
    );
}

#[test]
fn an_undeclared_snapshot_field_is_refused_rather_than_dropped() {
    let mut driver = Command::new(env!("CARGO_BIN_EXE_rustage"));
    driver.args([
        "--logic-process",
        "name:orders",
        "generation:1",
        "new-generation:2",
        "first:idle",
        "transition:idle:go:running",
        "first-set:idle:go:count=number:3",
        "first-event:go",
        "preserve",
        "from-version:1",
        "to-version:2",
        "first-schema:mode:text:1",
        "second-schema:mode:text:2",
        "check",
    ]);
    let output = run_bounded(driver, "the undeclared-field walk", Duration::from_secs(20));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(4), "stdout:\n{stdout}");
    assert!(
        stdout.contains("the snapshot holds field \"count\" that first-schema v1 does not declare; refusing to drop it silently"),
        "{stdout}"
    );
    assert!(
        stdout.contains("generation 1 still serving: running"),
        "{stdout}"
    );
    assert!(
        !stdout.contains("new generation 2:"),
        "the new process must not start:\n{stdout}"
    );
}

#[test]
fn malformed_schema_declarations_are_refused() {
    let mut empty = Command::new(env!("CARGO_BIN_EXE_rustage"));
    empty.args([
        "--logic-process",
        "generation:1",
        "new-generation:2",
        "first:idle",
        "first-transition:idle:go:running",
        "preserve",
        "first-schema::number:1",
        "second-schema:mode:text:2",
        "check",
    ]);
    let output = run_bounded(empty, "the empty-field walk", Duration::from_secs(20));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(4), "stdout:\n{stdout}");
    assert!(stdout.contains("non-empty name"), "{stdout}");

    let mut duplicate = Command::new(env!("CARGO_BIN_EXE_rustage"));
    duplicate.args([
        "--logic-process",
        "generation:1",
        "new-generation:2",
        "first:idle",
        "first-transition:idle:go:running",
        "preserve",
        "first-schema:count:number:1",
        "first-schema:count:text:1",
        "second-schema:count:text:2",
        "check",
    ]);
    let output = run_bounded(
        duplicate,
        "the duplicate-field walk",
        Duration::from_secs(20),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(4), "stdout:\n{stdout}");
    assert!(
        stdout.contains("first-schema declares field \"count\" more than once"),
        "{stdout}"
    );
}

#[test]
fn a_replacement_must_change_the_generation() {
    let mut driver = Command::new(env!("CARGO_BIN_EXE_rustage"));
    driver.args([
        "--logic-process",
        "generation:7",
        "new-generation:7",
        "first:idle",
        "first-transition:idle:go:running",
    ]);
    let output = run_bounded(driver, "the same-generation walk", Duration::from_secs(20));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!output.status.success(), "stdout:\n{stdout}");
    assert!(
        stdout.contains("a replacement needs a different generation"),
        "{stdout}"
    );
}

#[test]
fn two_real_generations_are_driven_and_the_stale_one_is_refused() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustage"))
        .args([
            "--logic-process",
            "session:logic-1",
            "name:orders",
            "version:0.3.0",
            "generation:7",
            "new-generation:8",
            "first:idle",
            "transition:idle:go:running",
            "first-set:idle:go:count=number:3",
            "second:idle",
            "second-transition:idle:go:running",
            "second-transition:running:approve:done",
            "first-event:go",
            "second-event:approve",
            "preserve",
            "expect:running",
            "expect:done",
            "check",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the driver starts");
    let status = wait(&mut child);
    let output = child.wait_with_output().expect("the output");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(status.success(), "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("generation 7 go: running"), "{stdout}");
    assert!(
        stdout.contains("restored generation 8: running (1 field(s))"),
        "{stdout}"
    );
    assert!(
        stdout.contains("generation 8 approve: done [count=number:3]"),
        "{stdout}"
    );
    assert!(
        stdout.contains("fault=the frame is from generation 7, not 8"),
        "{stdout}"
    );
    assert!(
        stdout.contains("the stale generation is refused"),
        "{stdout}"
    );
}

#[test]
fn a_generation_that_refuses_the_import_does_not_replace_the_old_one() {
    let artifact = candidate_binary();
    let mut driver = Command::new(env!("CARGO_BIN_EXE_rustage"));
    driver.args([
        "--logic-process",
        "session:orders",
        "name:orders",
        "version:0.1.0",
        "generation:1",
        "new-generation:2",
        "first:idle",
        "transition:idle:go:running",
        "first-set:idle:go:count=number:3",
        "first-event:go",
        &format!("second-binary:{}", artifact.display()),
        "preserve",
        "from-version:1",
        "to-version:2",
        // The driver migrates `count` to text; the candidate's own schema still declares a
        // number. An import that only trusted the sender's preview would let the new generation
        // hold a state it cannot reduce.
        "first-schema:count:number:1",
        "second-schema:count:text:1",
        "check",
    ]);
    let output = run_bounded(driver, "the refused import walk", Duration::from_secs(60));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(4),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(stdout.contains("migration v1 -> v2"), "{stdout}");
    assert!(stdout.contains("count: retyped number -> text"), "{stdout}");
    assert!(
        stdout.contains("the new generation refused the import: the state field \"count\" is text, but schema v2 declares number"),
        "{stdout}"
    );
    assert!(
        stdout.contains("generation 1 still serving: running"),
        "{stdout}"
    );
    assert!(
        !stdout.contains("restored generation 2"),
        "the refused import was reported as a restore:\n{stdout}"
    );
}
