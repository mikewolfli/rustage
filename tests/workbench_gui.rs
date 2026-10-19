// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The workbench window's first slice, exercised as a real process.
//!
//! These tests need a desktop session (a window server), so they are macOS-only here: the window
//! is the thing under test, and pretending a headless run created one would be the placeholder
//! success the principles forbid. The smoke sequence drives the same `apply_action` path a toolbar
//! click or menu item uses, saves a real layout file, and then the process exits by itself.

#![cfg(target_os = "macos")]

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn layout_path(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a clock after the epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "rustage-workbench-{name}-{}-{unique}.layout",
        std::process::id()
    ))
}

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rustage"))
        .args(args)
        .output()
        .expect("the workbench command runs")
}

#[test]
fn the_workbench_window_opens_and_ends_a_bounded_session() {
    let output = run(&["--workbench-gui", "--exit-after-ms", "800"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let text = format!("{stdout}{stderr}");
    assert!(
        output.status.success(),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        text.contains("workbench window") && text.contains("1280x800"),
        "{text}"
    );
    assert!(text.contains("workbench session ended"), "{text}");
}

#[test]
fn the_smoke_sequence_toggles_panels_splits_and_saves_a_loadable_layout() {
    let path = layout_path("smoke");
    let output = run(&[
        "--workbench-gui",
        "--layout",
        path.to_str().expect("a UTF-8 path"),
        "--smoke",
        "--exit-after-ms",
        "1200",
    ]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let text = format!("{stdout}{stderr}");
    assert!(
        output.status.success(),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(text.contains("workbench smoke"), "{text}");
    let saved = std::fs::read_to_string(&path).expect("the smoke saved a layout");
    assert!(saved.contains("\"shape\""), "{saved}");
    assert!(
        saved.contains("\"left-right\""),
        "the split was not captured:\n{saved}"
    );
    assert!(saved.contains("\"shows\""), "{saved}");
    assert!(
        !saved.contains("\"id\": \"outline\"") && !saved.contains("\"id\": \"events\""),
        "the toggled-off panels were saved as shown:\n{saved}"
    );

    // The next process loads the same file; it must reach its own bounded end with exit 0.
    let second = run(&[
        "--workbench-gui",
        "--layout",
        path.to_str().expect("a UTF-8 path"),
        "--exit-after-ms",
        "500",
    ]);
    assert!(
        second.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr)
    );
    let _ = std::fs::remove_file(path);
}
