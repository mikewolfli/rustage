// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The file and project history panel's rows (roadmap F03/F20, `panel.history`).
//!
//! The panel reads the repository's own commit history through the installed `git` program
//! ([`Repository::log`]); it does not keep a second history and it does not claim a comparison or a
//! restore it did not perform. A directory that is not a repository, or a machine without Git, is
//! the single `Discover::render` row; a failed `git log` keeps its reason in one row.
//!
//! **Boundary.** "Compare" and "restore" are commands (`Repository::diff`,
//! `Repository::restore_preview`) that need two revisions or a path the panel has no selection for,
//! so they are not fabricated here. The history itself is complete: hash, subject, author, e-mail
//! and the author time, formatted with the interface language's own rules.

use std::path::Path;

use crate::app::locale::format::Rules;
use crate::app::vcs::Repository;

/// The history rows for the repository containing `directory`.
pub(super) fn rows(directory: &Path, limit: usize, rules: Rules) -> Vec<String> {
    let repository = match Repository::discover(directory) {
        Ok(repository) => repository,
        Err(discover) => return vec![discover.render()],
    };
    match repository.log(Some(limit)) {
        Ok(commits) if commits.is_empty() => {
            vec!["history: no commit yet".to_owned()]
        }
        Ok(commits) => {
            let mut rows = vec![format!("commits: {} (at most {limit})", commits.len())];
            for commit in &commits {
                rows.push(format!(
                    "{} {} — {} <{}> {}",
                    commit.short(),
                    commit.subject,
                    commit.author,
                    commit.email,
                    rules.date_time(commit.time),
                ));
            }
            rows
        }
        Err(reason) => vec![format!("history unavailable: {reason}")],
    }
}

#[cfg(test)]
mod tests {
    use super::rows;
    use crate::app::locale::format::Rules;
    use crate::host::locale::Language;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    fn rules() -> Rules {
        Rules::for_language(Language::English)
    }

    fn git_available() -> bool {
        Command::new("git")
            .arg("--version")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    fn temporary(name: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};

        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "rustage-workbench-history-{name}-{}-{}-{}",
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

    fn git(directory: &Path, arguments: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(directory)
            .args(arguments)
            .status()
            .expect("git runs");
        assert!(status.success(), "git {arguments:?} succeeds");
    }

    #[test]
    fn a_non_repository_directory_reports_the_refusal() {
        let directory = temporary("plain");
        let lines = rows(&directory, 20, rules());
        let _ = std::fs::remove_dir_all(&directory);

        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            lines[0].contains("not a git repository") || lines[0].contains("git is not available"),
            "{lines:?}"
        );
    }

    #[test]
    fn a_committed_subject_is_reported() {
        if !git_available() {
            return;
        }
        let directory = temporary("commit");
        git(&directory, &["init", "-q"]);
        std::fs::write(directory.join("demo.txt"), "hello\n").expect("a file");
        git(&directory, &["add", "demo.txt"]);
        git(
            &directory,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "-q",
                "-m",
                "a real subject",
            ],
        );

        let lines = rows(&directory, 20, rules());
        let _ = std::fs::remove_dir_all(&directory);

        assert!(
            lines.iter().any(|line| line.contains("a real subject")),
            "{lines:?}"
        );
        assert!(
            lines.iter().any(|line| line.starts_with("commits: 1")),
            "{lines:?}"
        );
        assert!(lines.iter().all(|line| !line.is_empty()), "{lines:?}");
    }
}
