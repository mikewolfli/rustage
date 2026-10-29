// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The `--vcs` command: drive the installed Git program over a real repository (task F20).

use std::path::{Path, PathBuf};

use crate::app::generate::merge::{self, Prepared};
use crate::app::locale::catalog;
use crate::app::locale::format::Rules;
use crate::app::vcs::branch;
use crate::app::vcs::design_diff;
use crate::app::vcs::{HunkChoice, Repository, Resolution, Switched};

/// Runs the version-control operations, printing each answer as it is read.
pub(super) fn vcs(options: &crate::cli::VcsOptions) -> bool {
    let mut path = ".".to_owned();
    let mut repository: Option<Repository> = None;
    let mut resolutions: Vec<Resolution> = Vec::new();
    // The open documents the caller reports as having unsaved changes (T20.03).
    let mut dirty: Vec<String> = Vec::new();
    // The project document whose model must be read again after a switch (T20.05).
    let mut project: Option<String> = None;
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
            "hunks" => with(&mut repository, &path, |repo| {
                if argument.trim().is_empty() {
                    return Err("hunks needs a path".to_owned());
                }
                let hunks = repo.diff_hunks(argument)?;
                if hunks.is_empty() {
                    return Ok(vec![format!("no unstaged text change in {argument}")]);
                }
                Ok(hunks
                    .iter()
                    .enumerate()
                    .map(|(index, hunk)| {
                        let header = hunk.lines().next().unwrap_or("");
                        format!("hunk {} of {argument}: {header}", index + 1)
                    })
                    .collect())
            }),
            "stage-hunk" => match argument.rsplit_once(':') {
                Some((target, index)) => match index.parse::<usize>() {
                    Ok(index) => with(&mut repository, &path, |repo| {
                        repo.stage_hunk(target, index).map(|line| vec![line])
                    }),
                    Err(_) => Err(format!("{index:?} is not a hunk number")),
                },
                None => Err(format!("stage-hunk needs <path>:<hunk>, got {argument:?}")),
            },
            "stage" => with(&mut repository, &path, |repo| {
                repo.stage(argument)?;
                Ok(vec![format!("staged {argument}")])
            }),
            "conflicts" => with(&mut repository, &path, |repo| {
                let conflicts = repo.conflicts()?;
                Ok(if conflicts.is_empty() {
                    vec!["(no unresolved conflicts)".to_owned()]
                } else {
                    conflicts.iter().map(|conflict| conflict.render()).collect()
                })
            }),
            "conflict" => with(&mut repository, &path, |repo| {
                let conflict = repo.conflict(argument)?;
                let mut lines = vec![conflict.render()];
                if let Some(diff) = conflict.structured_diff() {
                    lines.extend(diff.render());
                } else {
                    lines.extend(conflict.text_conflict()?.render_hunks());
                }
                Ok(lines)
            }),
            "complete" => with(&mut repository, &path, |repo| {
                Ok(repo.completion()?.render())
            }),
            "resolve-hunks" => match argument.rsplit_once(':') {
                Some((conflict_path, choices)) => with(&mut repository, &path, |repo| {
                    let conflict = repo.conflict(conflict_path)?;
                    let text = conflict.text_conflict()?;
                    let choices = hunk_choices(choices)?;
                    let resolution = repo.resolve_text(&text, &choices, None)?;
                    let lines = resolution.render();
                    resolutions.push(resolution);
                    Ok(lines)
                }),
                None => Err(format!(
                    "resolve-hunks needs <path>:<choices>, got {argument:?}"
                )),
            },
            "resolve-ours" | "resolve-theirs" => with(&mut repository, &path, |repo| {
                let conflict = repo.conflict(argument)?;
                let text = conflict.text_conflict()?;
                let choice = if verb == "resolve-ours" {
                    HunkChoice::Ours
                } else {
                    HunkChoice::Theirs
                };
                let choices = vec![choice; text.hunk_count()];
                let resolution = repo.resolve_text(&text, &choices, None)?;
                let lines = resolution.render();
                resolutions.push(resolution);
                Ok(lines)
            }),
            "resolve-file" => match argument.rsplit_once(':') {
                Some((conflict_path, source)) => match read_resolution(source) {
                    Ok(document) => with(&mut repository, &path, |repo| {
                        let resolution = repo.resolve_text_file(conflict_path, &document, None)?;
                        let lines = resolution.render();
                        resolutions.push(resolution);
                        Ok(lines)
                    }),
                    Err(reason) => Err(reason),
                },
                None => Err(format!(
                    "resolve-file needs <path>:<resolved-file>, got {argument:?}"
                )),
            },
            "resolve-document" => match argument.rsplit_once(':') {
                Some((conflict_path, source)) => match read_resolution(source) {
                    Ok(document) => with(&mut repository, &path, |repo| {
                        let resolution = repo.resolve_document(conflict_path, &document, None)?;
                        let lines = resolution.render();
                        resolutions.push(resolution);
                        Ok(lines)
                    }),
                    Err(reason) => Err(reason),
                },
                None => Err(format!(
                    "resolve-document needs <path>:<resolved-file>, got {argument:?}"
                )),
            },
            "regenerate" => with(&mut repository, &path, |repo| {
                let checked = check_generation(repo.root(), argument);
                // The check is recorded on every resolution even when it fails: a stale or
                // refused regeneration must leave `Resolution::is_complete` false rather than
                // disappearing behind an early return.
                let recorded = match &checked {
                    Ok(lines) => Ok(lines.join("; ")),
                    Err(reason) => Err(reason.clone()),
                };
                for resolution in &mut resolutions {
                    resolution.regeneration = Some(recorded.clone());
                }
                match checked {
                    Ok(mut lines) => {
                        for resolution in &resolutions {
                            lines.extend(resolution.render());
                        }
                        Ok(lines)
                    }
                    Err(reason) if resolutions.is_empty() => Err(reason),
                    Err(reason) => Err(format!(
                        "{reason}; {} resolved path(s) remain incomplete because regeneration did \
                         not pass",
                        resolutions.len()
                    )),
                }
            }),
            "unstage" => with(&mut repository, &path, |repo| {
                repo.unstage(argument)?;
                Ok(vec![format!("unstaged {argument}")])
            }),
            "dirty" => {
                if argument.trim().is_empty() {
                    return super::report("dirty needs a path".to_owned());
                }
                dirty.push(argument.to_owned());
                Ok(vec![format!("dirty buffer {argument}")])
            }
            "commit-preview" => with(&mut repository, &path, |repo| {
                repo.commit_preview(&dirty).map(|preview| preview.render())
            }),
            "commit" => with(&mut repository, &path, |repo| {
                // The preview runs first: a secret in the staged content is a refusal, not a warning
                // that is printed while the commit goes ahead.
                let preview = repo.commit_preview(&dirty)?;
                if !preview.can_commit() {
                    for line in preview.render() {
                        println!("{line}");
                    }
                    return Err(if preview.staged.is_empty() {
                        "there is nothing staged to commit".to_owned()
                    } else {
                        format!(
                            "refusing to commit {} secret(s) in the staged content",
                            preview.secrets.len()
                        )
                    });
                }
                for line in preview.render() {
                    println!("{line}");
                }
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
            "project" => {
                if argument.trim().is_empty() {
                    return super::report("project needs a path".to_owned());
                }
                project = Some(argument.to_owned());
                Ok(vec![format!("project {argument}")])
            }
            "switch" => with(&mut repository, &path, |repo| {
                switch(repo, &dirty, project.as_deref(), argument, false)
            }),
            "switch-stash" => with(&mut repository, &path, |repo| {
                switch(repo, &dirty, project.as_deref(), argument, true)
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
            "log" => with(&mut repository, &path, |repo| {
                let limit: Option<usize> = if argument.is_empty() {
                    None
                } else {
                    match argument.parse() {
                        Ok(limit) => Some(limit),
                        Err(_) => {
                            return Err(format!("{argument:?} is not a commit count"));
                        }
                    }
                };
                let history = repo.log(limit)?;
                if history.is_empty() {
                    return Ok(vec!["no commits yet".to_owned()]);
                }
                // The commit's date is written the way the interface language writes dates. The
                // timestamp itself is a fact; only its written form follows the language.
                let rules = Rules::for_language(catalog::installed().language());
                Ok(history
                    .iter()
                    .map(|commit| {
                        format!(
                            "{} {} {} <{}>: {}",
                            commit.short(),
                            rules.date_time(commit.time),
                            commit.author,
                            commit.email,
                            commit.subject
                        )
                    })
                    .collect())
            }),
            "commit-show" => with(&mut repository, &path, |repo| {
                let (revision, target) = match argument.split_once(':') {
                    Some((revision, target)) => (revision, Some(target)),
                    None => (argument, None),
                };
                if revision.trim().is_empty() {
                    return Err("commit-show needs a revision".to_owned());
                }
                let text = repo.commit_show(revision, target)?;
                Ok(text.lines().map(str::to_owned).collect())
            }),
            "restore-preview" => match argument.split_once(':') {
                Some((revision, target)) => with(&mut repository, &path, |repo| {
                    repo.restore_preview(revision, target)
                        .map(|preview| preview.render())
                }),
                None => Err(format!(
                    "restore-preview needs <revision>:<path>, got {argument:?}"
                )),
            },
            "design-diff" => match argument.rsplit_once(':') {
                Some((revision, target)) => with(&mut repository, &path, |repo| {
                    let before_text = repo.commit_show(revision, Some(target))?;
                    let before = crate::project::record::json::parse(&before_text)
                        .map_err(|error| format!("{target} at {revision} is not JSON: {error}"))?;
                    let after_text = read_working(repo, target)?;
                    let after = crate::project::record::json::parse(&after_text)
                        .map_err(|error| format!("the working {target} is not JSON: {error}"))?;
                    Ok(design_diff::compare(&before, &after).render())
                }),
                None => Err(format!(
                    "design-diff needs <revision>:<path>, got {argument:?}"
                )),
            },
            "code-diff" => match argument.rsplit_once(':') {
                Some((revision, target)) => with(&mut repository, &path, |repo| {
                    let before = repo.commit_show(revision, Some(target))?;
                    let after = read_working(repo, target)?;
                    design_diff::split_code(&before, &after).map(|diff| diff.render())
                }),
                None => Err(format!(
                    "code-diff needs <revision>:<path>, got {argument:?}"
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
    if !resolutions.is_empty() {
        if !resolutions.iter().all(Resolution::is_complete) {
            let incomplete = resolutions
                .iter()
                .filter(|resolution| !resolution.is_complete())
                .count();
            return super::report(format!(
                "{incomplete} resolved path(s) were staged, but the generation/check step has \
                 not passed; run regenerate:<project-file> and do not report success until it \
                 does"
            ));
        }
        // A resolution is only one path. Reporting success while another conflict is still
        // unmerged would claim a complete merge that Git does not have.
        let completion = match with(&mut repository, &path, |repo| repo.completion()) {
            Ok(completion) => completion,
            Err(reason) => return super::report(reason),
        };
        if !completion.is_ready() {
            return super::report(
                completion
                    .render()
                    .into_iter()
                    .chain(std::iter::once(
                        "resolved paths are not a complete merge; resolve and stage every remaining \
                         path before reporting success"
                            .to_owned(),
                    ))
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
        }
    }
    true
}

/// Reads a caller-written resolved file.
fn read_resolution(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|error| format!("{path:?} could not be read: {error}"))
}

/// Reads one choice per hunk: `ours`, `theirs`, `both`, `none`, or `lines=<ours>|<theirs>`.
///
/// Line indexes are separated by `+`, and an empty side is written as an empty string, so
/// `lines=0+2|1` takes our first and third lines plus their second line of a hunk.
fn hunk_choices(text: &str) -> Result<Vec<HunkChoice>, String> {
    if text.trim().is_empty() {
        return Err(
            "a hunk resolution needs one choice per hunk, for example ours,theirs".to_owned(),
        );
    }
    text.split(',')
        .map(|choice| {
            let choice = choice.trim();
            match choice {
                "ours" => Ok(HunkChoice::Ours),
                "theirs" => Ok(HunkChoice::Theirs),
                "both" => Ok(HunkChoice::Both),
                "none" | "omit" => Ok(HunkChoice::Omit),
                _ => {
                    let Some(rest) = choice.strip_prefix("lines=") else {
                        return Err(format!(
                            "{choice:?} is not a hunk choice (ours, theirs, both, none, \
                             lines=<ours>|<theirs>)"
                        ));
                    };
                    let Some((ours, theirs)) = rest.split_once('|') else {
                        return Err(format!(
                            "{choice:?} needs both sides, as lines=<ours>|<theirs>"
                        ));
                    };
                    Ok(HunkChoice::Lines {
                        ours: line_indexes(ours)?,
                        theirs: line_indexes(theirs)?,
                    })
                }
            }
        })
        .collect()
}

fn line_indexes(text: &str) -> Result<Vec<usize>, String> {
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    text.split('+')
        .map(|value| {
            value
                .trim()
                .parse::<usize>()
                .map_err(|_| format!("{value:?} is not a line index"))
        })
        .collect()
}

/// Regenerates a project's artifacts and compares them with what is committed.
///
/// This is the step a resolution must pass before it can be reported as complete: a staged conflict
/// resolution does not by itself prove the model still generates. Nothing is written; a mismatch is
/// a refusal that names the stale paths.
fn check_generation(root: &Path, project: &str) -> Result<Vec<String>, String> {
    let artifacts = super::generate::project_artifacts(root, Path::new(project))?;
    let mut lines = Vec::new();
    let mut stale = Vec::new();
    for artifact in artifacts {
        let path = root.join(&artifact.path);
        let committed = std::fs::read_to_string(&path)
            .map_err(|error| format!("{} could not be read: {error}", path.display()))?;
        match merge::prepare(&committed, &artifact.source) {
            Ok(Prepared::Unchanged) => lines.push(format!("regenerated {}", artifact.path)),
            Ok(Prepared::Write(_)) => {
                lines.push(format!("stale {}", artifact.path));
                stale.push(artifact.path);
            }
            Err(reason) => return Err(format!("{}: {reason}", artifact.path)),
        }
    }
    if stale.is_empty() {
        Ok(lines)
    } else {
        Err(format!(
            "regeneration differs from the committed file(s): {}; the conflict is not resolved \
             until the design regenerates the same bytes",
            stale.join(", ")
        ))
    }
}

/// Switches branches and reports what the switch invalidated (task T20.05).
///
/// The project document is read before and after the switch, so the answer is a comparison of two
/// real models rather than a promise that the old one still applies. A caller that names no project
/// gets that fact stated; it is not reported as "nothing changed".
fn switch(
    repository: &Repository,
    dirty: &[String],
    project: Option<&str>,
    name: &str,
    stash: bool,
) -> Result<Vec<String>, String> {
    branch::refuse_dirty_buffers(dirty, name)?;
    if name.trim().is_empty() {
        return Err("a branch name is required".to_owned());
    }
    let from = repository.status()?.branch;
    let project = project.map(|project| project_file(repository.root(), project));
    let before = match project.as_deref() {
        Some(path) => branch::Snapshot::read_optional(path)?,
        None => None,
    };
    let switched = if stash {
        repository.switch_branch_stashing(name)?
    } else {
        repository.switch_branch(name).map(|message| Switched {
            message,
            stash: None,
        })?
    };
    let source = from_label(&switched, &from);
    let mut lines = vec![switched.message];
    if let Some(stash) = &switched.stash {
        lines.push(format!("work set aside in stash: {stash}"));
    }
    let Some(path) = project else {
        lines.push(
            "no project file was named; the target state was not compared (add project:<path>)"
                .to_owned(),
        );
        return Ok(lines);
    };
    let after = match branch::Snapshot::read_optional(&path) {
        Ok(after) => after,
        Err(reason) => {
            let mut failure = lines.join("; ");
            failure.push_str(&format!(
                "; the project state cannot be read: {reason}; the target state from {from} is \
                 invalid"
            ));
            return Err(failure);
        }
    };
    let invalidation = branch::compare(before.as_ref(), after.as_ref(), &source, name);
    lines.extend(invalidation.render(&source, name));
    Ok(lines)
}

/// What the model before the switch actually was.
///
/// A clean switch read the branch that was checked out; a stashing switch read the dirty worktree,
/// which is now in the stash, so calling that model "the branch" would name the wrong file.
fn from_label(switched: &Switched, from: &str) -> String {
    if switched.stash.is_some() {
        "the worktree before the switch".to_owned()
    } else {
        from.to_owned()
    }
}

/// Resolves a project path the caller wrote against the repository it belongs to.
fn project_file(root: &Path, project: &str) -> PathBuf {
    let path = Path::new(project);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
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

/// Reads one file from the repository's working tree.
fn read_working(repository: &Repository, path: &str) -> Result<String, String> {
    let file = repository.root().join(path);
    std::fs::read_to_string(&file)
        .map_err(|error| format!("cannot read {}: {error}", file.display()))
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
    use super::{hunk_choices, stash_index};
    use crate::app::vcs::HunkChoice;

    #[test]
    fn a_stash_index_defaults_to_zero_and_refuses_letters() {
        assert_eq!(stash_index("").expect("default"), 0);
        assert_eq!(stash_index("2").expect("an index"), 2);
        assert!(stash_index("two").is_err());
    }

    #[test]
    fn a_hunk_choice_names_a_side_or_an_indexed_line_set() {
        assert_eq!(
            hunk_choices("ours,theirs,both,none").expect("choices"),
            vec![
                HunkChoice::Ours,
                HunkChoice::Theirs,
                HunkChoice::Both,
                HunkChoice::Omit
            ]
        );
        assert_eq!(
            hunk_choices("lines=0+2|1").expect("a line choice"),
            vec![HunkChoice::Lines {
                ours: vec![0, 2],
                theirs: vec![1]
            }]
        );
        assert_eq!(
            hunk_choices("lines=|0").expect("one empty side"),
            vec![HunkChoice::Lines {
                ours: Vec::new(),
                theirs: vec![0]
            }]
        );
        assert!(hunk_choices("").is_err());
        assert!(hunk_choices("mine").is_err());
        assert!(hunk_choices("lines=0").is_err());
        assert!(hunk_choices("lines=x|1").is_err());
    }
}
