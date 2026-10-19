// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The language service's real path: a child process, the LSP handshake, and a bounded shutdown.
//!
//! The scripted model tests live in `app::language::session`. This test starts a real server when
//! one is installed, and skips (rather than failing) on a machine without `rust-analyzer`: the
//! transport is the thing under test, not a toolchain prerequisite.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn rustage(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rustage"))
        .args(args)
        .output()
        .expect("the language command runs")
}

fn has_rust_analyzer() -> bool {
    Command::new("rust-analyzer")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn project() -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a clock after the epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rustage-lsp-{}-{unique}", std::process::id()));
    std::fs::create_dir_all(dir.join("src")).expect("a project directory");
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"lsp-probe\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .expect("a manifest");
    std::fs::write(dir.join("src/main.rs"), "fn main() {}\n").expect("a source");
    dir
}

#[test]
fn a_real_server_is_started_asked_for_capabilities_and_stopped() {
    if !has_rust_analyzer() {
        eprintln!("rust-analyzer is not installed; the real transport test is skipped");
        return;
    }

    let dir = project();
    let root = dir.display().to_string();
    let output = rustage(&[
        "--language",
        "exec:rust-analyzer",
        &format!("root:{root}"),
        &format!("view:{root}"),
        &format!("close:{root}"),
        "show",
    ]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains(&format!("view: project {root}: running, capabilities:")),
        "{stdout}"
    );
    assert!(stdout.contains("completion"), "{stdout}");
    assert!(stdout.contains("hover"), "{stdout}");
    assert!(
        stdout.contains(&format!("close {root}: released=true")),
        "{stdout}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn completion_from_a_real_server_becomes_typed_items() {
    if !has_rust_analyzer() {
        eprintln!("rust-analyzer is not installed; the completion test is skipped");
        return;
    }
    let dir = project();
    let source = "fn main() {\n    let alpha = 1;\n    let beta = alpha;\n    let _ = be;\n}\n";
    std::fs::write(dir.join("src/main.rs"), source).expect("a source");
    let offset = source.find("    let _ = be").expect("the probe line") + "    let _ = be".len();
    let root = dir.display().to_string();
    let file = dir.join("src/main.rs").display().to_string();
    let output = rustage(&[
        "--language",
        "exec:rust-analyzer",
        &format!("root:{root}"),
        &format!("view:{root}"),
        // The handshake returns before the workspace is indexed; waiting is the explicit op.
        "wait:3000",
        &format!("complete:{file}:{offset}"),
        "show",
    ]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(stdout.contains("completion: beta [variable]"), "{stdout}");
    assert!(stdout.contains("completion: alpha [variable]"), "{stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_program_that_is_not_a_server_is_a_spawn_failure_not_a_hang() {
    let dir = project();
    let root = dir.display().to_string();
    let output = rustage(&[
        "--language",
        "exec:/definitely/not/a/language-server",
        &format!("view:{root}"),
    ]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(4), "{stdout}");
    assert!(
        stdout.contains("could not be started"),
        "the spawn failure was not named:\n{stdout}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
