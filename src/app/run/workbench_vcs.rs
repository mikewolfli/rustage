// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The version-control panel's rows (task F20): the repository's real status.
//!
//! Every row is read from the installed Git program when the panel asks, never remembered from an
//! earlier run. The panel covers five facts:
//!
//! * the repository root and the current branch (`Repository::discover`, `Repository::status`);
//! * a summary of staged, unstaged and untracked paths, then one row per changed path;
//! * the unresolved conflicts (`Repository::conflicts`);
//! * the staged patch's file and hunk counts (`Repository::staged_diff`);
//! * how many changed generator-managed sources are generated, hand-written or mixed;
//!   a file with no region marker is not a generator artifact and is not counted as either
//!   (an unmarked file is not "generated" merely because the format treats unmarked bytes as
//!   generated).
//!
//! A directory that is not a repository, or a machine without Git, is reported as the
//! `Discover::render` row and nothing else: no empty status and no zero counts pretending a read
//! succeeded. A Git command that fails keeps its error in a row. A changed file that cannot be
//! read as UTF-8 is left out of the source counts rather than guessed at.

use std::path::Path;

use crate::app::generate::artifact;
use crate::app::vcs::Repository;

/// The version-control panel's rows for the repository containing `directory`.
///
/// The first call is discovery, so a plain directory or a missing `git` answers with the single
/// refusal row the panel can show; everything after it is read only once a repository exists.
pub(super) fn rows(directory: &Path) -> Vec<String> {
    let repository = match Repository::discover(directory) {
        Ok(repository) => repository,
        Err(discover) => return vec![discover.render()],
    };

    let mut rows = vec![format!("repository: {}", repository.root().display())];

    let status = match repository.status() {
        Ok(status) => Some(status),
        Err(reason) => {
            // A failed status is not an empty status: say why the branch and paths are missing.
            rows.push(format!("status unavailable: {reason}"));
            None
        }
    };

    if let Some(status) = &status {
        rows.push(format!("branch: {}", status.branch));

        let staged = status
            .entries
            .iter()
            .filter(|entry| entry.index != ' ' && entry.index != '?')
            .count();
        let unstaged = status
            .entries
            .iter()
            .filter(|entry| entry.worktree != ' ')
            .count();
        let untracked = status
            .entries
            .iter()
            .filter(|entry| entry.index == '?' || entry.worktree == '?')
            .count();
        rows.push(format!(
            "changes: {staged} staged, {unstaged} unstaged, {untracked} untracked"
        ));
        for entry in &status.entries {
            rows.push(entry.render());
        }
    }

    match repository.conflicts() {
        Ok(conflicts) if conflicts.is_empty() => rows.push("conflicts: none".to_owned()),
        Ok(conflicts) => {
            rows.push(format!("conflicts: {} unresolved", conflicts.len()));
            for conflict in &conflicts {
                rows.push(format!("conflict: {}", conflict.render()));
            }
        }
        Err(reason) => rows.push(format!("conflicts unavailable: {reason}")),
    }

    match repository.staged_diff(None) {
        Ok(patch) => {
            let files = patch
                .lines()
                .filter(|line| line.starts_with("+++ b/"))
                .count();
            let hunks = patch.lines().filter(|line| line.starts_with("@@")).count();
            rows.push(format!("commit preview: {files} file(s), {hunks} hunk(s)"));
        }
        Err(reason) => rows.push(format!("commit preview unavailable: {reason}")),
    }

    match &status {
        Some(status) => {
            let mut generated = 0;
            let mut handwritten = 0;
            let mut mixed = 0;
            for entry in &status.entries {
                let path = repository.root().join(&entry.path);
                if !path.is_file() {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                // Only a file that names a region is a generator artifact. `regions()` treats
                // unmarked bytes as generated, which is right for a generated file but would
                // mislabel an ordinary text file as generated; the marker is the evidence that
                // the generator wrote this file at all.
                let is_generated = text.contains(artifact::GENERATED_OPEN);
                let is_handwritten = text.contains(artifact::HANDWRITTEN_OPEN);
                match (is_generated, is_handwritten) {
                    (true, true) => mixed += 1,
                    (true, false) => generated += 1,
                    (false, true) => handwritten += 1,
                    (false, false) => {}
                }
            }
            rows.push(format!(
                "sources: {generated} generated, {handwritten} hand-written, {mixed} mixed"
            ));
        }
        None => rows.push("sources unavailable: the status could not be read".to_owned()),
    }

    rows
}

#[cfg(test)]
mod tests {
    use super::rows;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    fn git_available() -> bool {
        Command::new("git")
            .arg("--version")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    fn temporary(name: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};

        // Mirror the repository tests' naming: the clock alone can repeat inside one process, and
        // two tests sharing (then deleting) one directory would corrupt each other.
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "rustage-vcs-{name}-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).expect("a temporary directory");
        path
    }

    /// `git init` with `main`, falling back to the plain form when `-b` is not supported.
    fn init(directory: &Path) {
        let named = Command::new("git")
            .arg("-C")
            .arg(directory)
            .args(["init", "-q", "-b", "main"])
            .output();
        if matches!(&named, Ok(output) if output.status.success()) {
            return;
        }
        let status = Command::new("git")
            .arg("-C")
            .arg(directory)
            .args(["init", "-q"])
            .status()
            .expect("git runs");
        assert!(status.success(), "git init succeeds");
    }

    #[test]
    fn a_non_repository_directory_reports_the_refusal() {
        let directory = temporary("plain");
        let rows = rows(&directory);
        let _ = std::fs::remove_dir_all(&directory);

        assert_eq!(rows.len(), 1, "{rows:?}");
        let row = &rows[0];
        assert!(
            row.contains("not a git repository") || row.contains("git is not available"),
            "{row:?}"
        );
    }

    #[test]
    fn a_repository_reports_its_branch_and_change() {
        if !git_available() {
            return;
        }
        let directory = temporary("branch");
        init(&directory);
        std::fs::write(directory.join("demo.txt"), "hello\n").expect("a file");

        let rows = rows(&directory);
        let _ = std::fs::remove_dir_all(&directory);

        assert!(
            rows.iter().any(|row| row.starts_with("branch:")),
            "{rows:?}"
        );
        assert!(rows.iter().any(|row| row.contains("demo.txt")), "{rows:?}");
    }
}
