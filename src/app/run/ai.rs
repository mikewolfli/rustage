// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The `--ai` command: authorized context, candidates and review (task F23).

use std::collections::{BTreeMap, BTreeSet};

use crate::app::ai::{
    Candidate, Change, ChangeSet, Context, ContextFile, Role, Session, Verification, Workspace,
};

/// Runs the optional AI operations, printing each answer as it is read.
pub(super) fn ai(options: &crate::cli::AiOptions) -> bool {
    let mut session = Session {
        role: Role::Coder,
        model: String::new(),
        budget: 65_536,
        tools: BTreeSet::new(),
        credential: String::new(),
    };
    let mut files = Vec::new();
    let mut ignored = Vec::new();
    let mut context: Option<Context> = None;
    let mut changes = Vec::new();
    let mut base_revision = 1;
    let mut candidate: Option<Candidate> = None;
    let mut accepted: Vec<Change> = Vec::new();
    let mut workspace = Workspace::default();
    let mut undo: BTreeMap<String, String> = BTreeMap::new();

    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "role" => match argument {
                "coder" => {
                    session.role = Role::Coder;
                    Ok(vec!["role coder".to_owned()])
                }
                "reviewer" => {
                    session.role = Role::Reviewer;
                    Ok(vec!["role reviewer".to_owned()])
                }
                other => Err(format!("{other:?} is not coder or reviewer")),
            },
            "model" => {
                session.model = argument.to_owned();
                Ok(vec![format!("model {argument}")])
            }
            "budget" => match argument.parse() {
                Ok(value) => {
                    session.budget = value;
                    Ok(vec![format!("budget {value}")])
                }
                Err(_) => Err(format!("{argument:?} is not a byte budget")),
            },
            "tool" => {
                session.tools.insert(argument.to_owned());
                Ok(vec![format!("tool {argument}")])
            }
            "tool-check" => Ok(vec![format!(
                "tool {argument}: {}",
                if session.allows(argument) {
                    "allowed"
                } else {
                    "not allowed"
                }
            )]),
            "credential" => {
                session.credential = argument.to_owned();
                Ok(vec![format!("credential {argument}")])
            }
            "context" => match argument.split_once(':') {
                Some((path, text)) => {
                    files.push(ContextFile {
                        path: path.to_owned(),
                        text: text.to_owned(),
                    });
                    context = None;
                    Ok(vec![format!("context {path}")])
                }
                None => Err(format!("context needs <path>:<text>, got {argument:?}")),
            },
            "ignore" => {
                ignored.push(argument.to_owned());
                context = None;
                Ok(vec![format!("ignore {argument}")])
            }
            "authorize" => {
                let mut selected = Context::select(files.clone(), &ignored);
                selected.authorize();
                let lines = vec![
                    format!("authorized {} file(s)", selected.files.len()),
                    format!("ignored: {}", selected.ignored.join(", ")),
                    format!("redacted: {}", selected.redacted.join(", ")),
                    format!("payload {} byte(s)", selected.payload().len()),
                ];
                context = Some(selected);
                Ok(lines)
            }
            "context-show" => {
                let selected = match context.as_ref() {
                    Some(selected) => selected,
                    None => return super::report("authorize the context first".to_owned()),
                };
                Ok(vec![
                    format!("files: {}", selected.files.len()),
                    format!("authorized: {}", selected.authorized),
                    format!("payload {} byte(s)", selected.payload().len()),
                    format!("fits budget: {}", session.fits(selected)),
                ])
            }
            "base-revision" => match argument.parse() {
                Ok(value) => {
                    base_revision = value;
                    Ok(vec![format!("base revision {value}")])
                }
                Err(_) => Err(format!("{argument:?} is not a revision")),
            },
            "change" => match argument.splitn(3, ':').collect::<Vec<_>>().as_slice() {
                [path, before, after] => {
                    changes.push(Change {
                        path: (*path).to_owned(),
                        before: (*before).to_owned(),
                        after: (*after).to_owned(),
                    });
                    Ok(vec![format!("change {path}")])
                }
                _ => Err(format!(
                    "change needs <path>:<before>:<after>, got {argument:?}"
                )),
            },
            "candidate" => match argument.parse() {
                Ok(id) => {
                    let set = ChangeSet {
                        version: 1,
                        base_revision,
                        changes: changes.clone(),
                    };
                    let built = match Candidate::new(id, set) {
                        Ok(built) => built,
                        Err(reason) => return super::report(reason),
                    };
                    let lines = vec![
                        format!("candidate {id} digest {}", built.digest),
                        format!("verification {:?}", built.verification),
                    ];
                    candidate = Some(built);
                    Ok(lines)
                }
                Err(_) => Err(format!("{argument:?} is not a candidate id")),
            },
            "verify" => match argument.split_once(':') {
                Some((digest, state)) => {
                    let result = match state {
                        "unverified" => Verification::Unverified,
                        "compiled" => Verification::Compiled,
                        "tested" => Verification::Tested,
                        "failed" => Verification::Failed,
                        other => return super::report(format!("{other:?} is not a verification")),
                    };
                    let candidate = match candidate.as_mut() {
                        Some(candidate) => candidate,
                        None => return super::report("create a candidate first".to_owned()),
                    };
                    if let Err(reason) = candidate.verify(digest, result) {
                        return super::report(reason);
                    }
                    Ok(vec![format!("verified {}", candidate.digest)])
                }
                None => Err(format!("verify needs <digest>:<state>, got {argument:?}")),
            },
            "review" => {
                let candidate = match candidate.as_mut() {
                    Some(candidate) => candidate,
                    None => return super::report("create a candidate first".to_owned()),
                };
                candidate.review(argument);
                Ok(vec![format!("review comment: {argument}")])
            }
            "accept" => {
                let candidate = match candidate.as_ref() {
                    Some(candidate) => candidate,
                    None => return super::report("create a candidate first".to_owned()),
                };
                let indices = match parse_indices(argument) {
                    Ok(indices) => indices,
                    Err(reason) => return super::report(reason),
                };
                accepted = match candidate.accept(&indices) {
                    Ok(accepted) => accepted,
                    Err(reason) => return super::report(reason),
                };
                Ok(accepted
                    .iter()
                    .map(|change| format!("accepted {}", change.path))
                    .collect())
            }
            "base" => match argument.split_once(':') {
                Some((path, text)) => {
                    workspace = Workspace::new(
                        [(path.to_owned(), text.to_owned())]
                            .into_iter()
                            .collect::<BTreeMap<_, _>>(),
                    );
                    Ok(vec![format!("base {path}")])
                }
                None => Err(format!("base needs <path>:<text>, got {argument:?}")),
            },
            "apply" => {
                undo = match workspace.apply(&accepted) {
                    Ok(undo) => undo,
                    Err(reason) => return super::report(reason),
                };
                let mut lines = vec![format!("applied {} change(s)", accepted.len())];
                for change in &accepted {
                    lines.push(format!(
                        "{} -> {:?}",
                        change.path,
                        workspace.get(&change.path).unwrap_or("")
                    ));
                }
                Ok(lines)
            }
            "rollback" => {
                workspace.rollback(&undo);
                Ok(vec![format!("rolled back {} path(s)", undo.len())])
            }
            "show" => Ok(vec![
                format!("role: {:?}", session.role),
                format!("workspace: {}", session.role.directory()),
                format!("shared: {}", Role::shared()),
                format!("model: {}", session.model),
                format!("credential: {}", session.credential),
                format!("budget: {} bytes", session.budget),
                format!("tools: {}", session.tools.len()),
                format!(
                    "candidate: {}",
                    candidate
                        .as_ref()
                        .map(|candidate| candidate.digest.clone())
                        .unwrap_or_else(|| "(none)".to_owned())
                ),
            ]),
            other => Err(format!("{other:?} is not an ai operation")),
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

fn parse_indices(text: &str) -> Result<Vec<usize>, String> {
    let mut indices = Vec::new();
    for part in text.split(',') {
        indices.push(
            part.parse()
                .map_err(|_| format!("{part:?} is not a change index"))?,
        );
    }
    Ok(indices)
}
