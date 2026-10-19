// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The generation pipeline has one write point, and the bytes it writes are the bytes it reported.
//!
//! The independent-project harness (`tools/generated-project/run.py`) needs a file to build, and
//! T15.07's export step reads generated files. Before `--generate write:<path>` there was no command
//! that materialized the pipeline's output, so "the generated project builds on its own" could only
//! be reached through a test that called the pipeline directly. This keeps the seam honest.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn work_dir(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a clock after the epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "rustage-generate-{name}-{}-{unique}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("a temporary directory");
    dir
}

fn generate(dir: &Path, output: &Path) -> std::process::Output {
    let page = dir.join("page.json");
    std::fs::write(
        &page,
        r#"{"window":{"id":"root","title":"Probe","children":[
            {"label":{"id":"title","text":"Hello"}},
            {"button":{"id":"go","text":"Go"}}
        ]}}"#,
    )
    .expect("a page document");
    Command::new(env!("CARGO_BIN_EXE_rustage"))
        .args([
            "--generate",
            &format!("document:{}", page.display()),
            "node:window:2",
            "node:label:0",
            "node:button:0",
            "target:default",
            "size:320x200",
            "fn:build_ui",
            &format!("write:{}", output.display()),
            "run",
        ])
        .output()
        .expect("the generator runs")
}

#[test]
fn generation_can_write_its_source_and_a_second_run_is_byte_identical() {
    let dir = work_dir("stable");
    let first_path = dir.join("first.rs");
    let second_path = dir.join("second.rs");

    let first = generate(&dir, &first_path);
    assert!(
        first.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let first_stdout = String::from_utf8_lossy(&first.stdout);
    let first_source = std::fs::read_to_string(&first_path).expect("the written source");
    assert!(
        first_source.contains("fn build_ui"),
        "the written file is not the generated source"
    );
    assert!(
        first_stdout.contains(&format!(
            "wrote {} ({} bytes)",
            first_path.display(),
            first_source.len()
        )),
        "the report does not name the write:\n{first_stdout}"
    );

    let second = generate(&dir, &second_path);
    assert!(second.status.success());
    let second_source = std::fs::read_to_string(&second_path).expect("the written source");
    assert_eq!(
        first_source, second_source,
        "the same input written twice must be byte-identical"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_blocked_precheck_writes_nothing() {
    let dir = work_dir("blocked");
    let output = dir.join("blocked.rs");
    let page = dir.join("page.json");
    std::fs::write(&page, r#"{"window":{"id":"root","title":"Probe"}}"#).expect("a page");
    let result = Command::new(env!("CARGO_BIN_EXE_rustage"))
        .args([
            "--generate",
            &format!("document:{}", page.display()),
            "node:window:0",
            "layout:root:an unsupported layout",
            &format!("write:{}", output.display()),
            "run",
        ])
        .output()
        .expect("the generator runs");
    assert_eq!(
        result.status.code(),
        Some(4),
        "a blocked precheck is exit 4"
    );
    assert!(
        !output.exists(),
        "a blocked precheck emitted a file anyway: {}",
        output.display()
    );
    let _ = std::fs::remove_dir_all(&dir);
}
