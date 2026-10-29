// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The T14.08 load run measured in a process of its own.
//!
//! The unit test for `app::preview::load` shares its process with the rest of the suite, so a
//! thread another test starts would be counted as growth here. This test starts the real binary,
//! drives 100 mount/update/stop cycles in that process, and reads the process's own memory and
//! thread line: the reading is only meaningful when nothing else is running in the process. It is
//! not a windowed or preview-instance measurement — there is no mounted window in this build — and
//! it says nothing about other platforms.

use std::process::Command;

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn a_hundred_cycles_leave_the_process_memory_and_threads_steady() {
    let binary = env!("CARGO_BIN_EXE_rustage");
    let output = Command::new(binary)
        .args(["--quality", "load:100"])
        .output()
        .expect("the binary runs");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "the load run failed:\n{stdout}\n{stderr}"
    );
    assert!(
        stdout.contains("load passed: 100 real cycle(s)"),
        "{stdout}"
    );
    assert!(stdout.contains("steady: yes"), "{stdout}");
    assert!(stdout.contains("residual controls: 0"), "{stdout}");

    let resources = stdout
        .lines()
        .find(|line| line.starts_with("resources: rss "))
        .unwrap_or_else(|| panic!("no resource line in:\n{stdout}"));
    // `resources: rss <a> -> <b> bytes (growth <n>); threads <x> -> <y> (growth <m>)`.
    let threads = resources
        .split("threads ")
        .nth(1)
        .unwrap_or_else(|| panic!("no thread part in {resources:?}"));
    assert!(
        threads.ends_with("(growth 0)"),
        "the run left a thread behind: {resources}"
    );
    let growth = resources
        .split("bytes (growth ")
        .nth(1)
        .and_then(|rest| rest.split(')').next())
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or_else(|| panic!("no resident growth in {resources:?}"));
    assert!(
        growth <= 64 * 1024 * 1024,
        "resident memory grew by {growth} byte(s): {resources}"
    );
}
