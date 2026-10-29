// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! End-to-end evidence for T20.07: a real merge conflict on a generated file is not a resolution
//! until both the bytes are staged and `regenerate:<project-file>` proves the design still generates
//! them.
//!
//! Each test builds a throwaway Git repository from the fixture project under
//! `tests/fixtures/vcs-conflict`, generates the committed `src/generated/main.rs` through the real
//! binary, appends a hand-written region, and creates a genuine conflict by committing different
//! regenerations on two branches. The conflict is then resolved through the `--vcs` operations the
//! product exposes, not through test-only helpers.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

const PROJECT: &str = "project.json";
const PAGE: &str = "pages/main.json";
const GENERATED: &str = "src/generated/main.rs";

/// A hand-written region that both sides carry verbatim, so only the generated half conflicts.
const HANDWRITTEN: &str = "// region:handwritten save\n\
    fn save() { /* mine */ }\n\
    // endregion:handwritten save\n";

/// A temporary repository that is removed however the test ends.
struct Workspace {
    root: PathBuf,
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

impl Workspace {
    /// A repository with the fixture's base page and its generated file, before any commit.
    fn new(name: &str) -> Option<Self> {
        if !git_available() {
            eprintln!("git is not installed; skipping the T20.07 end-to-end fixture");
            return None;
        }
        let root = std::env::temp_dir().join(format!(
            "rustage-vcs-conflict-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("pages")).expect("the fixture directories");
        std::fs::copy(fixture("project.json"), root.join(PROJECT)).expect("the fixture project");
        std::fs::copy(fixture("pages/base.json"), root.join(PAGE)).expect("the base page");

        let workspace = Self { root };
        workspace.git_ok(&["init", "-q", "--template="]);
        workspace.git_ok(&["config", "user.email", "rustage@example.invalid"]);
        workspace.git_ok(&["config", "user.name", "rustage"]);
        workspace.git_ok(&["config", "commit.gpgsign", "false"]);

        let generated = workspace.generate();
        assert_success(&generated, "the base generation");
        let path = workspace.path(GENERATED);
        let mut text = std::fs::read_to_string(&path).expect("the generated file");
        text.push_str(HANDWRITTEN);
        std::fs::write(&path, text).expect("the hand-written region");
        Some(workspace)
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }

    fn copy_page(&self, variant: &str) {
        let text = std::fs::read_to_string(fixture(&format!("pages/{variant}.json")))
            .expect("the page variant");
        // Write rather than copy: `fs::copy` may preserve the fixture's timestamp, and a page with
        // the same size and timestamp as the committed one would look unchanged to Git's index.
        std::fs::write(self.path(PAGE), text).expect("the page variant");
    }

    /// Generates `pages/main.json` into the project's `src/generated/main.rs`.
    fn generate(&self) -> Output {
        let page = self.path(PAGE);
        let generated = self.path(GENERATED);
        Command::new(env!("CARGO_BIN_EXE_rustage"))
            .current_dir(&self.root)
            .args([
                "--generate",
                &format!("document:{}", page.display()),
                "target:default",
                "size:320x200",
                "fn:build_ui",
                &format!("write:{}", generated.display()),
                "run",
            ])
            .output()
            .expect("the generator runs")
    }

    fn git(&self, arguments: &[&str]) -> Output {
        Command::new("git")
            .arg("-C")
            .arg(&self.root)
            .args(["-c", "core.quotepath=false"])
            .args(arguments)
            .output()
            .expect("git runs")
    }

    fn git_ok(&self, arguments: &[&str]) {
        let output = self.git(arguments);
        assert!(
            output.status.success(),
            "git {arguments:?} failed\n{}",
            rendered(&output)
        );
    }

    /// Git's stdout exactly as printed, including a file's trailing newline.
    fn git_text(&self, arguments: &[&str]) -> String {
        let output = self.git(arguments);
        assert!(
            output.status.success(),
            "git {arguments:?} failed\n{}",
            rendered(&output)
        );
        String::from_utf8(output.stdout).expect("git printed UTF-8")
    }

    /// One `--vcs` invocation over this repository, with the given operations in order.
    fn vcs(&self, ops: &[String]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_rustage"));
        command
            .current_dir(&self.root)
            .arg("--vcs")
            .arg(format!("repo:{}", self.root.display()));
        for op in ops {
            command.arg(op);
        }
        command.output().expect("the vcs command runs")
    }
}

/// The fixture project copied into every test, by relative path.
fn fixture(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/vcs-conflict")
        .join(relative)
}

fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn rendered(output: &Output) -> String {
    format!(
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn assert_success(output: &Output, what: &str) -> String {
    let text = rendered(output);
    assert!(output.status.success(), "{what} failed\n{text}");
    text
}

fn assert_failure(output: &Output, what: &str) -> String {
    let text = rendered(output);
    assert!(
        !output.status.success(),
        "{what} unexpectedly passed\n{text}"
    );
    text
}

/// A repository whose `side` branch conflicts with the current branch on both the page and the
/// generated file. The merge is expected to fail and is left in the conflicted state.
fn conflicted(name: &str) -> Option<Workspace> {
    let workspace = Workspace::new(name)?;
    workspace.git_ok(&["add", "--", "."]);
    workspace.git_ok(&["commit", "-q", "-m", "base"]);
    let branch = workspace
        .git_text(&["rev-parse", "--abbrev-ref", "HEAD"])
        .trim()
        .to_owned();
    assert!(!branch.is_empty(), "the current branch has a name");
    workspace.git_ok(&["branch", "side"]);

    workspace.copy_page("ours");
    assert_success(&workspace.generate(), "the ours generation");
    workspace.git_ok(&["add", "--", "."]);
    workspace.git_ok(&["commit", "-q", "-m", "ours"]);

    workspace.git_ok(&["switch", "-q", "side"]);
    workspace.copy_page("theirs");
    assert_success(&workspace.generate(), "the theirs generation");
    workspace.git_ok(&["add", "--", "."]);
    workspace.git_ok(&["commit", "-q", "-m", "theirs"]);

    workspace.git_ok(&["switch", "-q", &branch]);
    let merge = workspace.git(&["merge", "side"]);
    assert!(
        !merge.status.success(),
        "the merge must conflict\n{}",
        rendered(&merge)
    );
    Some(workspace)
}

#[test]
fn a_generated_conflict_is_complete_only_after_regeneration_passes() {
    let Some(workspace) = conflicted("valid") else {
        return;
    };

    let conflicts = assert_success(
        &workspace.vcs(&["conflicts".to_owned()]),
        "conflict discovery",
    );
    assert!(
        conflicts.contains("pages/main.json (json"),
        "the page conflict is discovered: {conflicts}"
    );
    assert!(
        conflicts.contains("src/generated/main.rs (text"),
        "the generated-file conflict is discovered: {conflicts}"
    );

    let detail = assert_success(
        &workspace.vcs(&["conflict:src/generated/main.rs".to_owned()]),
        "conflict detail",
    );
    assert!(
        detail.contains("conflicting hunk") && detail.contains("hunk 0"),
        "the text conflict exposes its hunks: {detail}"
    );

    let resolved_page = workspace.path("resolved-main.json");
    std::fs::copy(fixture("pages/ours.json"), &resolved_page).expect("the resolved page document");

    let output = assert_success(
        &workspace.vcs(&[
            format!(
                "resolve-document:pages/main.json:{}",
                resolved_page.display()
            ),
            "resolve-ours:src/generated/main.rs".to_owned(),
            "complete".to_owned(),
            "regenerate:project.json".to_owned(),
        ]),
        "the resolve and regenerate chain",
    );
    assert!(
        output.contains("conflicts complete: no unmerged path remains"),
        "both paths are staged: {output}"
    );
    assert!(
        output.contains("regenerated src/generated/main.rs"),
        "regeneration compared the committed file: {output}"
    );
    assert!(
        output.contains("regeneration: passed"),
        "the resolution is reported complete only after regeneration passes: {output}"
    );
    assert_eq!(
        workspace.git_text(&["ls-files", "-u"]).trim(),
        "",
        "no unmerged stage remains"
    );

    let page = std::fs::read_to_string(workspace.path(PAGE)).expect("the resolved page");
    assert!(
        page.contains("Ours"),
        "the page resolution was written: {page}"
    );
    let generated =
        std::fs::read_to_string(workspace.path(GENERATED)).expect("the resolved generated file");
    assert!(
        generated.contains("Ours"),
        "the generated resolution was written: {generated}"
    );
    assert!(
        generated.contains("region:handwritten save") && generated.contains("fn save"),
        "the hand-written region survived: {generated}"
    );
}

#[test]
fn a_stale_resolution_stays_incomplete_and_the_previous_content_is_recoverable() {
    let Some(workspace) = conflicted("stale") else {
        return;
    };
    let previous_generated = workspace.git_text(&["show", &format!("HEAD:{GENERATED}")]);
    let previous_page = workspace.git_text(&["show", &format!("HEAD:{PAGE}")]);

    let resolved_page = workspace.path("resolved-main.json");
    std::fs::copy(fixture("pages/ours.json"), &resolved_page).expect("the resolved page document");

    // The page says "Ours" but the generated file is resolved to their stale bytes.
    let output = assert_failure(
        &workspace.vcs(&[
            format!(
                "resolve-document:pages/main.json:{}",
                resolved_page.display()
            ),
            "resolve-theirs:src/generated/main.rs".to_owned(),
            "regenerate:project.json".to_owned(),
        ]),
        "a stale regeneration",
    );
    assert!(
        output.contains("regeneration differs from the committed file(s): src/generated/main.rs"),
        "the drift is named: {output}"
    );
    assert!(
        output.contains("remain incomplete"),
        "the resolution is not reported complete: {output}"
    );
    assert!(
        !output.contains("regeneration: passed"),
        "a stale regeneration must not pass: {output}"
    );

    assert_success(
        &workspace.vcs(&["merge-abort".to_owned()]),
        "the merge abort",
    );
    assert_eq!(
        workspace.git_text(&["ls-files", "-u"]).trim(),
        "",
        "abort leaves no unmerged stage"
    );
    assert_eq!(
        std::fs::read_to_string(workspace.path(GENERATED)).expect("the restored generated file"),
        previous_generated,
        "the pre-merge generated content is recoverable"
    );
    assert_eq!(
        std::fs::read_to_string(workspace.path(PAGE)).expect("the restored page"),
        previous_page,
        "the pre-merge page is recoverable"
    );
}

#[test]
fn a_generator_gap_is_not_reported_as_a_successful_resolution() {
    let Some(workspace) = conflicted("gap") else {
        return;
    };

    // A syntactically valid JSON page that is not a design this build can generate. Resolving the
    // conflict to it is allowed, but regeneration must report the gap instead of passing.
    let gap_page = workspace.path("gap-page.json");
    std::fs::write(&gap_page, "{\"not_a_page\": true}\n").expect("the gap document");

    let output = assert_failure(
        &workspace.vcs(&[
            format!("resolve-document:pages/main.json:{}", gap_page.display()),
            "resolve-ours:src/generated/main.rs".to_owned(),
            "regenerate:project.json".to_owned(),
        ]),
        "a refused generation",
    );
    assert!(
        output.contains("remain incomplete"),
        "the resolution stays incomplete when the generator refuses: {output}"
    );
    assert!(
        !output.contains("regeneration: passed"),
        "a generator gap must not pass: {output}"
    );
}
