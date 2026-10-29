// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Version-control operations over the installed `git` program (task F20).

#![allow(clippy::needless_return)]

use super::*;

/// Which repository and operations to run (task T20.01–T20.05/T20.09).
#[derive(Debug, PartialEq, Eq)]
pub struct VcsOptions {
    /// The operations: `repo:<path>` (default `.`), `project:<path>`, `status`, `diff[:<path>]`,
    /// `staged-diff[:<path>]`, `stage:<path>`, `unstage:<path>`, `commit:<message>`, `branches`,
    /// `create:<branch>`, `switch:<branch>`, `switch-stash:<branch>`, `worktrees`,
    /// `worktree-add:<path>:<branch>`, `stash[:<message>]`, `stash-list`, `stash-pop[:<index>]`,
    /// `stash-apply[:<index>]`, `merge:<branch>`, `merge-abort`, `rebase:<branch>`, `rebase-abort`,
    /// `cherry-pick:<revision>`, `cherry-pick-abort`, `show`, `log[:<n>]`,
    /// `commit-show:<revision>[:<path>]`, `restore-preview:<revision>:<path>`, `dirty:<path>`,
    /// `commit-preview`, `hunks:<path>`, `stage-hunk:<path>:<n>`, `design-diff:<revision>:<path>`,
    /// `code-diff:<revision>:<path>`.
    ///
    /// `dirty` names an open document with unsaved changes; `commit-preview` (and `commit`, which
    /// runs it first) reports what is staged, what stays out, that warning, and every secret in the
    /// staged content (T20.03). `project:<path>` is the project document a switch reads on both
    /// sides, `switch` refuses while `dirty` is non-empty, and `switch-stash` sets a dirty worktree
    /// aside in a named stash before switching (T20.05).
    pub ops: Vec<String>,
}

impl VcsOptions {
    /// Options with no operation and the current directory as the repository.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

impl Default for VcsOptions {
    fn default() -> Self {
        Self::new()
    }
}

pub(super) fn parse_vcs(
    text: &str,
    _argument: OsString,
    arguments: &mut std::vec::IntoIter<OsString>,
) -> Option<Result<Command, ParseError>> {
    match text {
        "--vcs" => {
            let mut options = VcsOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Vcs(options)));
        }
        _ => None,
    }
}
