// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The debug client against a real DAP process (tasks T19.01–T19.08).
//!
//! The adapter used here is `tests/fixtures/dap_stub.py`: a small DAP server with fixed answers. It
//! is **not** a debugger and these tests do not claim platform debugging evidence; they exercise the
//! client's real call chain — framing, the launch/initialized/configurationDone order, breakpoint
//! binding, paged inspection, evaluation and disconnect — against a separate process that speaks
//! the protocol. The platform adapter's own minimum example is recorded separately and is blocked
//! here because macOS Developer Mode is off.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/dap_stub.py")
}

fn executable_stub() -> PathBuf {
    let python = Command::new("python3")
        .arg("--version")
        .output()
        .expect("python3 is required to run the DAP transport test");
    assert!(
        python.status.success(),
        "python3 is required to run the DAP transport test"
    );
    let directory = std::env::temp_dir().join(format!("rustage-dap-stub-{}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("a temporary directory");
    let stub = directory.join("dap_stub.py");
    std::fs::copy(fixture(), &stub).expect("a copy of the stub");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755))
        .expect("an executable stub");
    stub
}

fn rustage(ops: &[&str]) -> std::process::Output {
    let stub = executable_stub();
    rustage_with_adapter(&format!("adapter:path:{}", stub.display()), ops)
}

fn rustage_with_adapter(adapter: &str, ops: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rustage"))
        .arg("--debug")
        .arg(adapter)
        .args(ops)
        .output()
        .expect("the debug command runs")
}

#[test]
fn a_session_runs_through_launch_breakpoints_inspection_and_exit() {
    let output = rustage(&[
        "authorize",
        "launch:/tmp/rustage-dap-stub/program",
        "break:/tmp/rustage-dap-stub/main.rs:7",
        "start",
        "wait:3000",
        "breakpoints",
        "status",
        "threads",
        "stack",
        "scopes:1",
        "variables:100",
        "evaluate:n",
        "continue",
        "wait:3000",
        "disconnect",
        "check",
    ]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    for expected in [
        "launched /tmp/rustage-dap-stub/program",
        "stopped: breakpoint at line 7",
        "breakpoint 1 at /tmp/rustage-dap-stub/main.rs:7: bound at 7",
        "thread 1 main",
        "tick at /tmp/rustage-dap-stub/main.rs:7:5",
        "scope Locals reference 100",
        "variable n: i32 = 41",
        "evaluate i32 42",
        "exited 0",
        "disconnected",
        "debug: clean",
    ] {
        assert!(
            stdout.contains(expected),
            "missing {expected:?} in:\n{stdout}"
        );
    }
    assert!(stdout.contains("source version 0"), "{stdout}");
}

#[test]
fn a_session_without_authorization_never_starts_the_adapter() {
    let output = rustage(&["launch:/tmp/rustage-dap-stub/program", "start"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(4), "{stdout}");
    assert!(stdout.contains("was not authorized"), "{stdout}");
    assert!(!stdout.contains("launched"), "{stdout}");
}

#[test]
fn replacing_logic_while_debugging_asks_before_ending_the_session() {
    let output = rustage(&[
        "authorize",
        "launch:/tmp/rustage-dap-stub/program",
        "start",
        "wait:3000",
        "hot-swap",
        "hot-swap:confirm",
        "status",
    ]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(4), "{stdout}");
    assert!(stdout.contains("confirm:"), "{stdout}");
    assert!(stdout.contains("call stack"), "{stdout}");
    assert!(stdout.contains("start a new session"), "{stdout}");
    assert!(stdout.contains("source version 0"), "{stdout}");
}

#[test]
fn a_silent_adapter_is_a_failure_with_no_session_left_running() {
    let output = rustage_with_adapter(
        "adapter:path:/bin/cat",
        &[
            "authorize",
            "timeout:300",
            "launch:/tmp/rustage-dap-stub/program",
            "start",
        ],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(4), "{stdout}");
    assert!(stdout.contains("timed out"), "{stdout}");
    assert!(!stdout.contains("launched"), "{stdout}");
}

#[test]
fn the_platform_probe_reports_what_this_machine_has() {
    let output = Command::new(env!("CARGO_BIN_EXE_rustage"))
        .args(["--debug", "probe", "check"])
        .output()
        .expect("the probe runs");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("lldb-dap"), "{stdout}");
    assert!(stdout.contains("adapters:"), "{stdout}");
    // Readiness is a fact about the machine. When the adapter cannot start, the probe has to say
    // why; it may not report a session that would fail as one that would work.
    if stdout.contains("unavailable") {
        assert!(stdout.contains("unavailable ("), "{stdout}");
    }
}
