// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! A release journey against the real binary (task T25.05/T25.02).
//!
//! The walk is: freeze the inputs, stage the binary and its runtime language packs, then verify the
//! staged bytes against a declared artifact digest. It runs the built binary as a separate process,
//! so it exercises the CLI wiring, the filesystem and the digest manifest rather than a model in
//! isolation.

use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

fn scratch(tag: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rustage-release-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("a clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&path).expect("a scratch directory");
    path
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

fn digest_of(path: &Path) -> (String, u64) {
    let bytes = std::fs::read(path).expect("the staged binary is readable");
    let digest = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    (digest, bytes.len() as u64)
}

#[test]
fn a_release_can_be_frozen_staged_and_verified_end_to_end() {
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_rustage"));
    let stage = scratch("stage");
    let stage_argument = format!("stage:{}:{}", binary.display(), stage.display());
    let (ok, output) = rustage(&["--release", "freeze:t25.05:generator-1", &stage_argument]);
    assert!(ok, "stage failed: {output}");
    assert!(output.contains("staged rustage"), "{output}");
    assert!(output.contains("staged language/en.json"), "{output}");
    assert!(stage.join("stage.json").is_file());
    assert!(stage.join("language/en.json").is_file());
    assert!(stage.join("language/zh-Hans.json").is_file());
    let manifest = std::fs::read_to_string(stage.join("stage.json")).expect("stage.json");
    assert!(manifest.contains("\"revision\": \"t25.05\""), "{manifest}");
    assert!(manifest.contains("\"digest\":"), "{manifest}");

    let staged_binary = stage.join(binary.file_name().expect("a file name"));
    let (digest, size) = digest_of(&staged_binary);
    let artifact = format!("artifact:rustage:{digest}:{size}:unsigned");
    let verify = format!("verify-file:{}", staged_binary.display());
    let (ok, output) = rustage(&["--release", &artifact, &verify]);
    assert!(ok, "verify failed: {output}");
    assert!(output.contains("verified"), "{output}");

    let _ = std::fs::remove_dir_all(stage);
}
