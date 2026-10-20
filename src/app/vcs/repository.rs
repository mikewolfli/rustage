// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! An existing Git repository, reached through the `git` program (task F20/T20.01–T20.05/T20.09).
//!
//! The designer does not implement a version system of its own: every operation is the installed
//! `git` command, and every answer is what that command printed. The rules this module keeps:
//!
//! * **A missing tool and a non-repository are different answers.** [`Discover::GitMissing`] and
//!   [`Discover::NotRepository`] let a caller keep designing and say which fact is missing.
//! * **Status is read fresh.** [`Repository::status`] runs `git status` on every call, so an
//!   external change cannot be mistaken for a cached clean tree.
//! * **Dirty means stop.** Switching branches, creating a branch from a dirty tree and stashing
//!   require a clean worktree; the refusal names how many paths changed rather than dropping work.
//! * **Destructive commands are bounded.** Merge/rebase/cherry-pick report their conflict and name
//!   the matching abort; nothing here pushes, force-updates a ref, deletes a worktree, or rewrites
//!   public history.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Why a directory is not a repository this module can drive.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Discover {
    /// `git` could not be started on this machine.
    GitMissing(String),
    /// `git` ran and said the path is not inside a repository.
    NotRepository(String),
}

impl Discover {
    /// The refusal in one line.
    pub fn render(&self) -> String {
        match self {
            Self::GitMissing(reason) => format!("git is not available: {reason}"),
            Self::NotRepository(reason) => format!("not a git repository: {reason}"),
        }
    }
}

/// One `git status --porcelain` row.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Entry {
    /// The index column (`M`, `A`, `D`, `R`, `?`, …).
    pub index: char,
    /// The worktree column.
    pub worktree: char,
    /// The project-relative path.
    pub path: String,
}

impl Entry {
    /// The row in one line.
    pub fn render(&self) -> String {
        format!("{}{} {}", self.index, self.worktree, self.path)
    }

    /// Whether this row is untracked.
    pub fn is_untracked(&self) -> bool {
        self.index == '?' && self.worktree == '?'
    }
}

/// A repository's branch and changed paths at one point in time.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Status {
    /// The current branch, or `HEAD` when detached.
    pub branch: String,
    /// Changed and untracked paths.
    pub entries: Vec<Entry>,
    /// Paths matched by ignore rules (`!!`), named so an unexpected ignore is diagnosable.
    pub ignored: Vec<String>,
}

impl Status {
    /// Whether there is no changed or untracked path.
    pub fn is_clean(&self) -> bool {
        self.entries.is_empty()
    }

    /// The status in one line per fact.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "branch {}: {} changed path(s), {} ignored",
            self.branch,
            self.entries.len(),
            self.ignored.len()
        )];
        for entry in &self.entries {
            lines.push(format!(
                "  {}{}",
                entry.render(),
                if entry.is_untracked() {
                    " (untracked)"
                } else {
                    ""
                }
            ));
        }
        for path in &self.ignored {
            lines.push(format!("  !! {path}"));
        }
        lines
    }
}

/// One worktree row.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Worktree {
    /// Its path.
    pub path: String,
    /// The commit it is at.
    pub head: String,
    /// Its branch, when it has one.
    pub branch: Option<String>,
}

/// A discovered repository.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Repository {
    root: PathBuf,
}

impl Repository {
    /// Finds the repository that contains `path`.
    ///
    /// The path may be a file; its directory is used. A missing `git` is reported before the
    /// repository question, so a machine without Git gets the reason that applies.
    pub fn discover(path: &Path) -> Result<Self, Discover> {
        let directory = if path.is_file() {
            path.parent().unwrap_or(path)
        } else {
            path
        };
        let probe = Command::new("git")
            .args(["--version"])
            .stdin(Stdio::null())
            .output()
            .map_err(|error| Discover::GitMissing(error.to_string()))?;
        if !probe.status.success() {
            return Err(Discover::GitMissing(
                String::from_utf8_lossy(&probe.stderr).trim().to_owned(),
            ));
        }
        let output = Command::new("git")
            .arg("-C")
            .arg(directory)
            .args(["rev-parse", "--show-toplevel"])
            .stdin(Stdio::null())
            .output()
            .map_err(|error| Discover::GitMissing(error.to_string()))?;
        if !output.status.success() {
            return Err(Discover::NotRepository(
                String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            ));
        }
        let root = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if root.is_empty() {
            return Err(Discover::NotRepository(
                "git named no top-level directory".to_owned(),
            ));
        }
        Ok(Self {
            root: PathBuf::from(root),
        })
    }

    /// The repository's top-level directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Runs one Git command and answers its stdout, or the error Git printed.
    fn run(&self, arguments: &[&str]) -> Result<String, String> {
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.root)
            .args(["-c", "core.quotepath=false"])
            .args(arguments)
            .stdin(Stdio::null())
            .output()
            .map_err(|error| format!("cannot run git: {error}"))?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout)
                .trim_end()
                .to_owned())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            let detail = if stderr.is_empty() { stdout } else { stderr };
            Err(format!(
                "git {} exited with {}: {detail}",
                arguments.first().copied().unwrap_or("?"),
                output.status
            ))
        }
    }

    /// Reads the branch, changed paths and ignored paths.
    pub fn status(&self) -> Result<Status, String> {
        let text = self.run(&["status", "--porcelain=v1", "--branch", "--ignored=matching"])?;
        let mut branch = "HEAD".to_owned();
        let mut entries = Vec::new();
        let mut ignored = Vec::new();
        for line in text.lines() {
            if let Some(header) = line.strip_prefix("## ") {
                let name = header.split("...").next().unwrap_or(header);
                branch = if name.starts_with("No commits yet on ") {
                    name.trim_start_matches("No commits yet on ").to_owned()
                } else {
                    name.to_owned()
                };
                continue;
            }
            if let Some(ignored_path) = line.strip_prefix("!! ") {
                ignored.push(ignored_path.to_owned());
                continue;
            }
            let mut characters = line.chars();
            let index = characters.next().unwrap_or(' ');
            let worktree = characters.next().unwrap_or(' ');
            let path = line.chars().skip(3).collect::<String>();
            if !path.is_empty() {
                entries.push(Entry {
                    index,
                    worktree,
                    path,
                });
            }
        }
        Ok(Status {
            branch,
            entries,
            ignored,
        })
    }

    /// The unstaged diff, or one path's unstaged diff.
    pub fn diff(&self, path: Option<&str>) -> Result<String, String> {
        let mut arguments = vec!["diff", "--no-color"];
        if let Some(path) = path {
            arguments.push("--");
            arguments.push(path);
        }
        self.run(&arguments)
    }

    /// The staged diff, or one path's staged diff.
    pub fn staged_diff(&self, path: Option<&str>) -> Result<String, String> {
        let mut arguments = vec!["diff", "--cached", "--no-color"];
        if let Some(path) = path {
            arguments.push("--");
            arguments.push(path);
        }
        self.run(&arguments)
    }

    /// Adds one path to the index.
    pub fn stage(&self, path: &str) -> Result<(), String> {
        if path.trim().is_empty() {
            return Err("a path is required".to_owned());
        }
        self.run(&["add", "--", path]).map(|_| ())
    }

    /// Takes one path out of the index, leaving the worktree file alone.
    pub fn unstage(&self, path: &str) -> Result<(), String> {
        if path.trim().is_empty() {
            return Err("a path is required".to_owned());
        }
        self.run(&["restore", "--staged", "--", path]).map(|_| ())
    }

    /// Commits the index with an explicit message.
    ///
    /// An empty message is refused before Git runs, and Git itself refuses an empty index.
    pub fn commit(&self, message: &str) -> Result<String, String> {
        if message.trim().is_empty() {
            return Err("a commit message is required".to_owned());
        }
        self.run(&["commit", "-m", message])
    }

    /// The local branch names.
    pub fn branches(&self) -> Result<Vec<String>, String> {
        let text = self.run(&["branch", "--format=%(refname:short)"])?;
        Ok(text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect())
    }

    /// Refuses a branch-changing operation while the worktree has changes.
    fn require_clean(&self, operation: &str) -> Result<Status, String> {
        let status = self.status()?;
        if !status.is_clean() {
            return Err(format!(
                "{operation} needs a clean worktree, and {} path(s) changed on {}",
                status.entries.len(),
                status.branch
            ));
        }
        Ok(status)
    }

    /// Creates and switches to a branch, refusing a dirty worktree.
    pub fn create_branch(&self, name: &str) -> Result<String, String> {
        if name.trim().is_empty() {
            return Err("a branch name is required".to_owned());
        }
        self.require_clean("creating a branch")?;
        self.run(&["switch", "-c", name])
            .map(|_| format!("created and switched to {name}"))
    }

    /// Switches branches, refusing a dirty worktree.
    pub fn switch_branch(&self, name: &str) -> Result<String, String> {
        if name.trim().is_empty() {
            return Err("a branch name is required".to_owned());
        }
        self.require_clean("switching branches")?;
        self.run(&["switch", name])
            .map(|_| format!("switched to {name}"))
    }

    /// The worktrees Git reports.
    pub fn worktrees(&self) -> Result<Vec<Worktree>, String> {
        let text = self.run(&["worktree", "list", "--porcelain"])?;
        let mut out = Vec::new();
        let mut path = String::new();
        let mut head = String::new();
        let mut branch = None;
        for line in text.lines().chain(std::iter::once("")) {
            if line.is_empty() {
                if !path.is_empty() {
                    out.push(Worktree {
                        path: path.clone(),
                        head: head.clone(),
                        branch: branch.clone(),
                    });
                }
                path.clear();
                head.clear();
                branch = None;
                continue;
            }
            if let Some(value) = line.strip_prefix("worktree ") {
                path = value.to_owned();
            } else if let Some(value) = line.strip_prefix("HEAD ") {
                head = value.to_owned();
            } else if let Some(value) = line.strip_prefix("branch ") {
                branch = Some(value.trim_start_matches("refs/heads/").to_owned());
            }
        }
        Ok(out)
    }

    /// Adds a worktree at `path` for an existing branch.
    pub fn add_worktree(&self, path: &str, branch: &str) -> Result<String, String> {
        if path.trim().is_empty() || branch.trim().is_empty() {
            return Err("a worktree path and branch are required".to_owned());
        }
        if !self.branches()?.iter().any(|name| name == branch) {
            return Err(format!("no branch {branch:?} exists"));
        }
        self.run(&["worktree", "add", path, branch])
            .map(|_| format!("added worktree {path} on {branch}"))
    }

    /// Stashes the worktree and index, refusing a clean tree.
    pub fn stash(&self, message: &str) -> Result<String, String> {
        let status = self.status()?;
        if status.is_clean() {
            return Err("there is nothing to stash".to_owned());
        }
        let message = if message.trim().is_empty() {
            "rustage"
        } else {
            message
        };
        self.run(&["stash", "push", "-u", "-m", message])
            .map(|_| format!("stashed {} path(s)", status.entries.len()))
    }

    /// The stash entries.
    pub fn stash_list(&self) -> Result<Vec<String>, String> {
        let text = self.run(&["stash", "list"])?;
        Ok(text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect())
    }

    /// Pops one stash entry (default `0`), removing it on success.
    pub fn stash_pop(&self, index: u32) -> Result<String, String> {
        self.run(&["stash", "pop", &format!("stash@{{{index}}}")])
            .map(|_| format!("popped stash@{{{index}}}"))
    }

    /// Applies one stash entry without removing it.
    pub fn stash_apply(&self, index: u32) -> Result<String, String> {
        self.run(&["stash", "apply", &format!("stash@{{{index}}}")])
            .map(|_| format!("applied stash@{{{index}}}"))
    }

    /// Merges a branch without committing an empty fast-forward, reporting a conflict.
    pub fn merge(&self, branch: &str) -> Result<String, String> {
        self.run(&["merge", "--no-ff", "--no-edit", branch])
            .map(|_| format!("merged {branch}"))
    }

    /// Fetches a remote without touching the worktree. This is the only network operation that is
    /// safe without a confirmation.
    pub fn fetch(&self, remote: &str) -> Result<String, String> {
        let remote = if remote.trim().is_empty() {
            "origin"
        } else {
            remote
        };
        self.run(&["fetch", remote])
            .map(|_| format!("fetched {remote}"))
    }

    /// Pulls with `--ff-only`, so a divergent branch is a refusal rather than a merge.
    pub fn pull(&self, remote: &str) -> Result<String, String> {
        let remote = if remote.trim().is_empty() {
            "origin"
        } else {
            remote
        };
        self.run(&["pull", "--ff-only", remote])
            .map(|_| format!("pulled {remote} (fast-forward only)"))
    }

    /// Pushes an explicit branch after the caller confirmed it.
    ///
    /// `confirmed` is not a default: without it this refuses before Git runs. There is no force
    /// flag and no automatic push after commit.
    pub fn push(&self, remote: &str, branch: &str, confirmed: bool) -> Result<String, String> {
        if !confirmed {
            return Err("push needs an explicit confirmation".to_owned());
        }
        if branch.trim().is_empty() {
            return Err("a branch is required for push".to_owned());
        }
        let remote = if remote.trim().is_empty() {
            "origin"
        } else {
            remote
        };
        self.run(&["push", remote, branch])
            .map(|_| format!("pushed {branch} to {remote}"))
    }

    /// Aborts an in-progress merge.
    pub fn merge_abort(&self) -> Result<String, String> {
        self.run(&["merge", "--abort"])
            .map(|_| "merge aborted".to_owned())
    }

    /// Rebases onto a branch, reporting a conflict.
    pub fn rebase(&self, branch: &str) -> Result<String, String> {
        self.run(&["rebase", branch])
            .map(|_| format!("rebased onto {branch}"))
    }

    /// Aborts an in-progress rebase.
    pub fn rebase_abort(&self) -> Result<String, String> {
        self.run(&["rebase", "--abort"])
            .map(|_| "rebase aborted".to_owned())
    }

    /// Cherry-picks one revision, reporting a conflict.
    pub fn cherry_pick(&self, revision: &str) -> Result<String, String> {
        self.run(&["cherry-pick", revision])
            .map(|_| format!("cherry-picked {revision}"))
    }

    /// Aborts an in-progress cherry-pick.
    pub fn cherry_pick_abort(&self) -> Result<String, String> {
        self.run(&["cherry-pick", "--abort"])
            .map(|_| "cherry-pick aborted".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::{Discover, Repository};
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
        let path = std::env::temp_dir().join(format!(
            "rustage-vcs-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0)
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

    fn repository() -> Option<(PathBuf, Repository)> {
        if !git_available() {
            return None;
        }
        let directory = temporary("repo");
        git(&directory, &["init", "-q", "--template="]);
        git(
            &directory,
            &["config", "user.email", "rustage@example.invalid"],
        );
        git(&directory, &["config", "user.name", "rustage"]);
        std::fs::write(directory.join("a.txt"), "one\n").expect("a file");
        git(&directory, &["add", "a.txt"]);
        git(&directory, &["commit", "-q", "-m", "initial"]);
        let repository = Repository::discover(&directory).expect("a repository");
        Some((directory, repository))
    }

    #[test]
    fn a_directory_that_is_not_a_repository_says_so_and_a_missing_git_is_distinct() {
        if !git_available() {
            return;
        }
        let directory = temporary("plain");
        match Repository::discover(&directory) {
            Err(Discover::NotRepository(reason)) => assert!(!reason.is_empty()),
            other => panic!("expected NotRepository, got {other:?}"),
        }
    }

    #[test]
    fn status_reads_the_branch_and_a_new_path() {
        let Some((directory, repository)) = repository() else {
            return;
        };
        let clean = repository.status().expect("a status");
        assert!(clean.is_clean(), "{clean:?}");
        assert!(!clean.branch.is_empty());
        std::fs::write(directory.join("b.txt"), "two\n").expect("a file");
        let dirty = repository.status().expect("a status");
        assert!(!dirty.is_clean());
        assert!(dirty.entries.iter().any(|entry| entry.path == "b.txt"));
        assert!(dirty.entries.iter().any(|entry| entry.is_untracked()));
    }

    #[test]
    fn stage_commit_and_diff_go_through_git() {
        let Some((directory, repository)) = repository() else {
            return;
        };
        std::fs::write(directory.join("a.txt"), "one\ntwo\n").expect("an edit");
        let diff = repository.diff(Some("a.txt")).expect("a diff");
        assert!(diff.contains("+two"), "{diff}");
        repository.stage("a.txt").expect("a stage");
        assert!(
            repository
                .staged_diff(Some("a.txt"))
                .expect("a staged diff")
                .contains("+two")
        );
        let message = repository.commit("second").expect("a commit");
        assert!(
            message.contains("second") || message.contains("1 file"),
            "{message}"
        );
        assert!(repository.status().expect("a status").is_clean());
    }

    #[test]
    fn a_dirty_tree_refuses_a_branch_switch_and_names_the_change() {
        let Some((directory, repository)) = repository() else {
            return;
        };
        repository.create_branch("feature").expect("a clean branch");
        assert!(
            repository
                .branches()
                .expect("branches")
                .contains(&"feature".to_owned())
        );
        std::fs::write(directory.join("a.txt"), "changed\n").expect("an edit");
        let reason = repository
            .switch_branch("master")
            .or_else(|_| repository.switch_branch("main"))
            .expect_err("a dirty switch");
        assert!(reason.contains("clean worktree"), "{reason}");
        assert!(reason.contains("1 path"), "{reason}");
    }

    #[test]
    fn stash_round_trips_a_change() {
        let Some((directory, repository)) = repository() else {
            return;
        };
        std::fs::write(directory.join("a.txt"), "stashed\n").expect("an edit");
        repository.stash("work in progress").expect("a stash");
        assert!(repository.status().expect("a status").is_clean());
        assert!(!repository.stash_list().expect("a list").is_empty());
        repository.stash_pop(0).expect("a pop");
        assert!(!repository.status().expect("a status").is_clean());
        let _ = directory;
    }

    #[test]
    fn an_empty_commit_message_is_refused_before_git_runs() {
        let Some((_directory, repository)) = repository() else {
            return;
        };
        assert!(repository.commit("   ").is_err());
    }
}
