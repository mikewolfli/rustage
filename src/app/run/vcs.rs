// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The `--vcs` command: drive the installed Git program over a real repository (task F20).

use std::path::Path;

use crate::app::vcs::Repository;

/// Runs the version-control operations, printing each answer as it is read.
pub(super) fn vcs(options: &crate::cli::VcsOptions) -> bool {
    let mut path = ".".to_owned();
    let mut repository: Option<Repository> = None;
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "repo" => {
                if argument.trim().is_empty() {
                    Err("repo needs a path, got an empty one".to_owned())
                } else {
                    path = argument.to_owned();
                    repository = None;
                    Ok(vec![format!("repository path {path}")])
                }
            }
            "status" => with(&mut repository, &path, |repo| Ok(repo.status()?.render())),
            "diff" => with(&mut repository, &path, |repo| {
                let text = repo.diff(if argument.is_empty() {
                    None
                } else {
                    Some(argument)
                })?;
                Ok(if text.is_empty() {
                    vec!["(no unstaged changes)".to_owned()]
                } else {
                    text.lines().map(str::to_owned).collect()
                })
            }),
            "staged-diff" => with(&mut repository, &path, |repo| {
                let text = repo.staged_diff(if argument.is_empty() {
                    None
                } else {
                    Some(argument)
                })?;
                Ok(if text.is_empty() {
                    vec!["(no staged changes)".to_owned()]
                } else {
                    text.lines().map(str::to_owned).collect()
                })
            }),
            "stage" => with(&mut repository, &path, |repo| {
                repo.stage(argument)?;
                Ok(vec![format!("staged {argument}")])
            }),
            "unstage" => with(&mut repository, &path, |repo| {
                repo.unstage(argument)?;
                Ok(vec![format!("unstaged {argument}")])
            }),
            "commit" => with(&mut repository, &path, |repo| {
                let message = repo.commit(argument)?;
                Ok(message.lines().map(str::to_owned).collect())
            }),
            "branches" => with(&mut repository, &path, |repo| {
                let branches = repo.branches()?;
                Ok(if branches.is_empty() {
                    vec!["(no local branches yet)".to_owned()]
                } else {
                    branches
                        .into_iter()
                        .map(|branch| format!("branch {branch}"))
                        .collect()
                })
            }),
            "create" => with(&mut repository, &path, |repo| {
                Ok(vec![repo.create_branch(argument)?])
            }),
            "switch" => with(&mut repository, &path, |repo| {
                Ok(vec![repo.switch_branch(argument)?])
            }),
            "worktrees" => with(&mut repository, &path, |repo| {
                let trees = repo.worktrees()?;
                Ok(trees
                    .into_iter()
                    .map(|tree| {
                        format!(
                            "worktree {} {} {}",
                            tree.path,
                            tree.head,
                            tree.branch.unwrap_or_else(|| "(detached)".to_owned())
                        )
                    })
                    .collect())
            }),
            "worktree-add" => match argument.rsplit_once(':') {
                Some((tree_path, branch)) => with(&mut repository, &path, |repo| {
                    Ok(vec![repo.add_worktree(tree_path, branch)?])
                }),
                None => Err(format!(
                    "worktree-add needs <path>:<branch>, got {argument:?}"
                )),
            },
            "stash" => with(&mut repository, &path, |repo| {
                Ok(vec![repo.stash(argument)?])
            }),
            "stash-list" => with(&mut repository, &path, |repo| {
                let entries = repo.stash_list()?;
                Ok(if entries.is_empty() {
                    vec!["(no stash entries)".to_owned()]
                } else {
                    entries
                })
            }),
            "stash-pop" => with(&mut repository, &path, |repo| {
                Ok(vec![repo.stash_pop(stash_index(argument)?)?])
            }),
            "stash-apply" => with(&mut repository, &path, |repo| {
                Ok(vec![repo.stash_apply(stash_index(argument)?)?])
            }),
            "merge" => with(&mut repository, &path, |repo| {
                Ok(vec![repo.merge(argument)?])
            }),
            "merge-abort" => with(&mut repository, &path, |repo| Ok(vec![repo.merge_abort()?])),
            "rebase" => with(&mut repository, &path, |repo| {
                Ok(vec![repo.rebase(argument)?])
            }),
            "rebase-abort" => with(&mut repository, &path, |repo| {
                Ok(vec![repo.rebase_abort()?])
            }),
            "cherry-pick" => with(&mut repository, &path, |repo| {
                Ok(vec![repo.cherry_pick(argument)?])
            }),
            "cherry-pick-abort" => with(&mut repository, &path, |repo| {
                Ok(vec![repo.cherry_pick_abort()?])
            }),
            "fetch" => with(&mut repository, &path, |repo| {
                Ok(vec![repo.fetch(argument)?])
            }),
            "pull" => with(&mut repository, &path, |repo| {
                Ok(vec![repo.pull(argument)?])
            }),
            "push-confirm" => match argument.split_once(':') {
                Some((remote, branch)) => with(&mut repository, &path, |repo| {
                    Ok(vec![repo.push(remote, branch, true)?])
                }),
                None => Err(format!(
                    "push-confirm needs <remote>:<branch>, got {argument:?}"
                )),
            },
            "show" => with(&mut repository, &path, |repo| {
                let status = repo.status()?;
                Ok(vec![
                    format!("root: {}", repo.root().display()),
                    format!(
                        "branch {}: {} changed, {} ignored",
                        status.branch,
                        status.entries.len(),
                        status.ignored.len()
                    ),
                ])
            }),
            other => Err(format!("{other:?} is not a vcs operation")),
        };
        match step {
            Ok(lines) => {
                for line in lines {
                    println!("{line}");
                }
            }
            Err(reason) => return super::report(reason),
        }
    }
    true
}

/// Discovers the repository on first use, then runs `f` against it.
fn with<T>(
    repository: &mut Option<Repository>,
    path: &str,
    f: impl FnOnce(&Repository) -> Result<T, String>,
) -> Result<T, String> {
    if repository.is_none() {
        *repository =
            Some(Repository::discover(Path::new(path)).map_err(|reason| reason.render())?);
    }
    f(repository.as_ref().expect("just discovered"))
}

/// Reads a stash index, with `0` as the default.
fn stash_index(argument: &str) -> Result<u32, String> {
    if argument.is_empty() {
        return Ok(0);
    }
    argument
        .parse()
        .map_err(|_| format!("{argument:?} is not a stash index"))
}

#[cfg(test)]
mod tests {
    use super::stash_index;

    #[test]
    fn a_stash_index_defaults_to_zero_and_refuses_letters() {
        assert_eq!(stash_index("").expect("default"), 0);
        assert_eq!(stash_index("2").expect("an index"), 2);
        assert!(stash_index("two").is_err());
    }
}
