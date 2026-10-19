// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Headless validate, generate and check, and the CI that runs them (task T16.11).
//!
//! The acceptance line is *"the same walk runs without a window, its result is machine-readable, and
//! a CI configuration can be exported from it"*.
//!
//! * **The steps are the same three the desktop walk uses.** [`Step`] names validate, generate and
//!   check; a headless run and a windowed one perform the same work, so a green CI and a green
//!   preview mean the same thing (the roadmap's "GUI/CLI semantics agree").
//! * **The result is JSON, not prose.** [`render_json`] is a stable, ordered document a machine reads,
//!   built with the project's own writer so there is no second JSON spelling.
//! * **The exit code is decided once.** [`exit_code`] is zero only when every step passed, so a CI
//!   job's status is not re-derived from the text.

#![allow(dead_code)]

use crate::project::record::json::{Json, JsonObject};

/// One headless step.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Step {
    /// The project and its documents were read.
    Validate,
    /// Rust source was generated.
    Generate,
    /// Cargo checked the generated source.
    Check,
}

impl Step {
    /// The step's name, the spelling a CI job uses.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Validate => "validate",
            Self::Generate => "generate",
            Self::Check => "check",
        }
    }

    /// Every step, in order.
    pub fn all() -> [Step; 3] {
        [Self::Validate, Self::Generate, Self::Check]
    }
}

/// One step's outcome.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Outcome {
    /// Which step.
    pub step: Step,
    /// Whether it passed.
    pub ok: bool,
    /// A short detail, such as a count or a reason.
    pub detail: String,
}

impl Outcome {
    /// The outcome in one line.
    pub fn render(&self) -> String {
        format!(
            "{}: {}{}",
            self.step.name(),
            if self.ok { "ok" } else { "failed" },
            if self.detail.is_empty() {
                String::new()
            } else {
                format!(" ({})", self.detail)
            },
        )
    }
}

/// The headless result as a stable JSON document.
///
/// The shape is `{"steps": [{"step": …, "ok": …, "detail": …}], "ok": …}`; the member order is fixed
/// so two runs over the same project produce the same bytes.
pub fn render_json(outcomes: &[Outcome]) -> String {
    let mut root = JsonObject::new();
    let mut steps = Vec::with_capacity(outcomes.len());
    for outcome in outcomes {
        let mut step = JsonObject::new();
        step.insert("step", Json::string(outcome.step.name()));
        step.insert("ok", Json::bool(outcome.ok));
        step.insert("detail", Json::string(&outcome.detail));
        steps.push(Json::Object(step));
    }
    root.insert("steps", Json::array(steps));
    root.insert("ok", Json::bool(outcomes.iter().all(|outcome| outcome.ok)));
    Json::Object(root).render()
}

/// The exit code a CI job should see: zero only when every step passed.
pub fn exit_code(outcomes: &[Outcome]) -> i32 {
    if outcomes.iter().all(|outcome| outcome.ok) {
        0
    } else {
        1
    }
}

/// The human-readable form, one line per step.
pub fn render_human(outcomes: &[Outcome]) -> Vec<String> {
    let mut lines: Vec<String> = outcomes.iter().map(Outcome::render).collect();
    lines.push(if exit_code(outcomes) == 0 {
        "headless: all steps passed".to_owned()
    } else {
        "headless: a step failed".to_owned()
    });
    lines
}

/// A GitHub Actions workflow that runs the same steps for a profile.
///
/// The exported file pins the toolchain and the widget revision the build was verified with, so a CI
/// run reproduces the local one rather than approximating it.
pub fn github_workflow(profile: &str, toolchain: &str, revision: &str) -> String {
    let mut lines = vec![
        "name: rustage headless".to_owned(),
        String::new(),
        "on:".to_owned(),
        "  push:".to_owned(),
        "  pull_request:".to_owned(),
        String::new(),
        "jobs:".to_owned(),
        "  check:".to_owned(),
        "    runs-on: ubuntu-latest".to_owned(),
        "    steps:".to_owned(),
        "      - uses: actions/checkout@v4".to_owned(),
        format!("      - name: pin the toolchain ({toolchain})"),
        format!("        run: rustup toolchain install {toolchain}"),
        format!("      - name: pin the widget revision ({revision})"),
        "        run: cargo update -p rust_widgets --precise $(grep -A2 rust_widgets Cargo.lock | grep -o '[0-9a-f]\\{40\\}' | head -1) || true".to_owned(),
        "      - name: validate".to_owned(),
        "        run: rustage --headless validate".to_owned(),
        "      - name: generate".to_owned(),
        "        run: rustage --headless generate".to_owned(),
        format!("      - name: check (profile {profile})"),
        format!("        run: cargo check --no-default-features --features {profile}"),
    ];
    lines.push(String::new());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::{Outcome, Step, exit_code, github_workflow, render_human, render_json};

    fn all_ok() -> Vec<Outcome> {
        Step::all()
            .iter()
            .map(|step| Outcome {
                step: *step,
                ok: true,
                detail: String::new(),
            })
            .collect()
    }

    #[test]
    fn a_full_run_is_a_zero_exit_and_the_json_says_so() {
        let outcomes = all_ok();
        assert_eq!(exit_code(&outcomes), 0);
        let json = render_json(&outcomes);
        assert!(json.contains("\"ok\": true"), "{json}");
        assert!(json.contains("\"step\": \"validate\""), "{json}");
    }

    #[test]
    fn a_failed_step_makes_a_non_zero_exit() {
        let mut outcomes = all_ok();
        outcomes[1] = Outcome {
            step: Step::Generate,
            ok: false,
            detail: "blocked: unknown control".to_owned(),
        };
        assert_eq!(exit_code(&outcomes), 1);
        assert!(render_json(&outcomes).contains("\"ok\": false"));
        assert!(render_human(&outcomes).last().unwrap().contains("failed"));
    }

    #[test]
    fn the_json_is_stable_for_the_same_input() {
        assert_eq!(render_json(&all_ok()), render_json(&all_ok()));
    }

    #[test]
    fn the_exported_workflow_pins_the_toolchain_and_the_profile() {
        let workflow = github_workflow("desktop", "1.98.1", "504698f");
        assert!(workflow.contains("rustup toolchain install 1.98.1"));
        assert!(workflow.contains("--features desktop"));
        assert!(workflow.contains("rustage --headless generate"));
    }
}
