// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The generate → check → build → run → feedback walk against a real `cargo` project (task T16.08).
//!
//! This is a separate process on purpose: it starts real builds, and a test binary that runs many
//! tests in parallel must not have its own timing mixed in. Each test builds a project of its own.
//!
//! It skips when `cargo` is missing. It does **not** skip when a build fails: a failing build is the
//! point of the second test.

use std::path::PathBuf;
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rustage-e2e-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("a clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&path).expect("a directory");
    path
}

/// A one-file project whose binary prints one word, with no dependencies.
fn project(tag: &str, source: &str) -> PathBuf {
    let directory = scratch(tag);
    std::fs::write(
        directory.join("Cargo.toml"),
        "[package]\nname = \"e2e-fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .expect("a manifest");
    std::fs::create_dir_all(directory.join("src")).expect("a source directory");
    std::fs::write(directory.join("src/main.rs"), source).expect("a source file");
    directory
}

fn rustage(arguments: &[&str]) -> (bool, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_rustage"))
        .args(arguments)
        .output()
        .expect("the host binary runs");
    (
        output.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
    )
}

fn cargo_available() -> bool {
    Command::new("cargo")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

#[test]
fn a_real_project_is_checked_built_and_run_and_the_walk_says_so() {
    if !cargo_available() {
        println!("skipped: cargo is not on PATH");
        return;
    }
    let directory = project("ok", "fn main() { println!(\"built\"); }\n");
    let target = directory.display().to_string();
    let (ok, output) = rustage(&["--e2e", &format!("walk:{target}:e2e-fixture")]);
    assert!(ok, "{output}");
    assert!(output.contains("check (cargo check): ok"), "{output}");
    assert!(output.contains("build (cargo build): ok"), "{output}");
    assert!(
        output.contains("run (") && output.contains("): ok"),
        "{output}"
    );
    assert!(output.contains("built"), "{output}");
    assert!(
        output.contains("feedback (") && output.contains("): ok"),
        "{output}"
    );
    // The generate stage is not part of this walk: it is exercised by the generator's own tests, and
    // this test does not claim it happened.
    assert!(!output.contains("generate"), "{output}");
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn a_broken_project_stops_at_the_check_and_reports_the_compilers_line() {
    if !cargo_available() {
        println!("skipped: cargo is not on PATH");
        return;
    }
    let directory = project("broken", "fn main() { let x: u32 = \"no\"; }\n");
    let target = directory.display().to_string();
    // `skip` fills the stages that never ran as skipped, so "not run" is visible rather than absent.
    let (ok, output) = rustage(&["--e2e", &format!("walk:{target}:e2e-fixture"), "skip"]);
    // A failing walk is a failure, and the stages after the failure are skipped, not passed.
    assert!(!ok, "{output}");
    assert!(output.contains("check (cargo check): failed"), "{output}");
    assert!(output.contains("build: skipped"), "{output}");
    assert!(output.contains("run: skipped"), "{output}");
    assert!(
        output.contains("feedback (") && output.contains("): failed"),
        "{output}"
    );
    assert!(
        output.contains("src/main.rs"),
        "the compiler's own line is the feedback: {output}"
    );
    assert!(output.contains("blocked at check"), "{output}");
    let _ = std::fs::remove_dir_all(&directory);
}
