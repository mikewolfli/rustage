// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The terminal's byte pump is a real POSIX pseudo-terminal, not a buffer with a process-shaped name.
//!
//! The model tests in `app::build::terminal` cover the session rules; these tests start `/bin/sh` and
//! `/bin/cat` on a master/slave pair through the real binary. They are Unix-only because the byte
//! pump is: a Windows host answers with an explicit refusal instead of pretending ConPTY exists.

#![cfg(unix)]

use std::process::Command;

fn rustage(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rustage"))
        .arg("--terminal")
        .args(args)
        .output()
        .expect("the terminal command runs")
}

#[test]
fn a_real_shell_writes_through_the_terminal() {
    let output = rustage(&[
        "authorize",
        "exec:/bin/sh",
        "arg:-c",
        "arg:printf pty-ok",
        "start",
        "wait:5000",
        "read",
    ]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(stdout.contains("started /bin/sh pid"), "{stdout}");
    assert!(stdout.contains("exit 0"), "{stdout}");
    assert!(stdout.contains("output \"pty-ok\""), "{stdout}");
    assert!(stdout.contains("dropped 0"), "{stdout}");
}

#[test]
fn input_reaches_a_real_program_and_its_output_comes_back() {
    let output = rustage(&[
        "authorize",
        "exec:/bin/cat",
        "start",
        "input:ping-through-pty\\n",
        "wait:500",
        "read",
        "terminate",
    ]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(stdout.contains("input: 17 byte(s)"), "{stdout}");
    assert!(stdout.contains("ping-through-pty"), "{stdout}");
    assert!(stdout.contains("terminated"), "{stdout}");
}

#[test]
fn running_without_authorization_is_refused() {
    let output = rustage(&["exec:/bin/true", "start"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(4), "{stdout}");
    assert!(
        stdout.contains("has not been authorized to run processes"),
        "{stdout}"
    );
}
