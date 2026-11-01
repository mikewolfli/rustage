// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The `--ai` command: authorized context, candidates and review (task F23).

use std::collections::{BTreeMap, BTreeSet};

use crate::app::ai::{
    Candidate, Change, ChangeSet, Context, ContextFile, Handoff, InvocationOutcome, Journal,
    McpServer, Role, Session, SessionRecord, ToolCall, ToolPolicy, UsageRecord, Verification,
    Workspace,
};

/// Runs the optional AI operations, printing each answer as it is read.
pub(super) fn ai(options: &crate::cli::AiOptions) -> bool {
    let mut session = Session {
        role: Role::Coder,
        model: String::new(),
        budget: 65_536,
        tools: BTreeSet::new(),
        credential: String::new(),
        usage: Vec::new(),
    };
    let mut files = Vec::new();
    let mut ignored = Vec::new();
    let mut root: Option<std::path::PathBuf> = None;
    let mut context: Option<Context> = None;
    let mut prompt_bytes = 0usize;
    let mut completion_bytes = 0usize;
    let mut cost_micros = 0u64;
    let mut elapsed_ms = 0u64;
    let mut outcome = InvocationOutcome::Completed;
    let mut tool_arguments = String::new();
    let mut tool_preview: Option<ToolCall> = None;
    let mut tool_policy = ToolPolicy::default();
    // The stdio transport (T23.11). The program is named first, arguments follow, and only
    // `mcp-start` opens the process — naming it is not starting it.
    let mut mcp_program: Option<String> = None;
    let mut mcp_arguments: Vec<String> = Vec::new();
    let mut mcp_server: Option<McpServer> = None;
    let mut shared: Vec<String> = Vec::new();
    let mut handoff: Option<Handoff> = None;
    let mut journal: Option<Journal> = None;
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
            // The controlled external-tool seam (T23.11): the call is previewed against the
            // session allowlist, authorized by exact digest, and printed as an MCP `tools/call`
            // body. No transport is configured in this build, and the tool does not pretend one is.
            "tool-arguments" => {
                tool_arguments = argument.to_owned();
                Ok(vec![format!(
                    "tool arguments {} byte(s)",
                    tool_arguments.len()
                )])
            }
            "tool-preview" => match session.preview_tool(argument, &tool_arguments) {
                Ok(call) => {
                    let lines = vec![call.preview()];
                    tool_preview = Some(call);
                    Ok(lines)
                }
                Err(reason) => Err(reason),
            },
            "tool-authorize" => match tool_preview.as_ref() {
                Some(call) => {
                    tool_policy.authorize(call);
                    Ok(vec![format!("authorized {}", call.digest())])
                }
                None => Err("preview a tool call first".to_owned()),
            },
            "tool-decision" => match tool_preview.as_ref() {
                Some(call) => {
                    let decision = tool_policy.decide(call);
                    if decision.allowed() {
                        Ok(vec!["decision allowed".to_owned()])
                    } else {
                        match decision {
                            crate::app::ai::ToolDecision::Refused { reason } => {
                                Ok(vec![format!("decision refused: {reason}")])
                            }
                            crate::app::ai::ToolDecision::Allowed => {
                                Ok(vec!["decision allowed".to_owned()])
                            }
                        }
                    }
                }
                None => Err("preview a tool call first".to_owned()),
            },
            "tool-mcp" => match tool_preview.as_ref() {
                Some(call) => match call.mcp_json() {
                    Ok(body) => Ok(vec![body]),
                    Err(reason) => Err(reason),
                },
                None => Err("preview a tool call first".to_owned()),
            },
            "mcp-server" => {
                mcp_program = Some(argument.to_owned());
                mcp_arguments.clear();
                Ok(vec![format!("mcp server {argument}")])
            }
            "mcp-arg" => {
                mcp_arguments.push(argument.to_owned());
                Ok(vec![format!("mcp argument {argument}")])
            }
            "mcp-start" => {
                let Some(program) = mcp_program.clone() else {
                    return super::report("name the server with mcp-server first".to_owned());
                };
                let permit = match crate::host::permission::PermissionService::new().permit(
                    crate::contracts::permission::EffectClass::Command,
                    crate::host::permission::Scope::Session,
                    crate::contracts::permission::Principal::Human,
                ) {
                    Ok(permit) => permit,
                    Err(refusal) => return super::report(refusal.message()),
                };
                let spec = crate::host::subprocess::ProcessSpec::new("mcp-server", program)
                    .args(mcp_arguments.iter().cloned());
                match McpServer::start(spec, crate::app::ai::DEFAULT_REQUEST_TIMEOUT, &permit) {
                    Ok(mut server) => match server.initialize() {
                        Ok(result) => {
                            let lines = vec![
                                format!("mcp started {} (pid {})", server.label(), server.pid()),
                                format!("initialize: {}", result.render_compact()),
                            ];
                            mcp_server = Some(server);
                            Ok(lines)
                        }
                        Err(failure) => {
                            let stderr = server.stderr();
                            let mut message = failure.message();
                            if !stderr.is_empty() {
                                message.push_str(&format!("; stderr: {}", stderr.trim()));
                            }
                            server.shutdown(std::time::Duration::from_millis(500));
                            Err(message)
                        }
                    },
                    Err(failure) => Err(failure.message()),
                }
            }
            "mcp-tools" => match mcp_server.as_mut() {
                Some(server) => match server.list_tools() {
                    Ok(result) => Ok(vec![format!("tools: {}", result.render_compact())]),
                    Err(failure) => Err(failure.message()),
                },
                None => Err("start an MCP server with mcp-start first".to_owned()),
            },
            "mcp-call" => match (mcp_server.as_mut(), tool_preview.as_ref()) {
                (Some(server), Some(call)) => {
                    let decision = tool_policy.decide(call);
                    match server.call_tool(call, &decision) {
                        Ok(result) => Ok(vec![format!("result: {}", result.render_compact())]),
                        Err(failure) => Err(failure.message()),
                    }
                }
                (None, _) => Err("start an MCP server with mcp-start first".to_owned()),
                (_, None) => Err("preview a tool call first".to_owned()),
            },
            "mcp-stop" => match mcp_server.take() {
                Some(mut server) => {
                    let termination = server.shutdown(std::time::Duration::from_secs(3));
                    let stderr = server.stderr();
                    let mut lines = vec![format!("mcp stopped: {}", termination.message())];
                    if !stderr.trim().is_empty() {
                        lines.push(format!("stderr: {}", stderr.trim()));
                    }
                    Ok(lines)
                }
                None => Err("no MCP server is running".to_owned()),
            },
            "tool-clear" => {
                tool_policy.clear();
                tool_preview = None;
                Ok(vec!["tool grants cleared".to_owned()])
            }
            // Two-agent session governance (T23.12): a handoff carries the candidate digest and
            // its base revision, so a review cannot silently target another candidate or a
            // document that moved on; the shared-material paths are listed for cleanup.
            "share" => {
                shared.push(argument.to_owned());
                Ok(vec![format!("shared material {argument}")])
            }
            "handoff" => {
                let to = match argument {
                    "coder" => Role::Coder,
                    "reviewer" => Role::Reviewer,
                    other => return super::report(format!("{other:?} is not coder or reviewer")),
                };
                let Some(candidate) = candidate.as_ref() else {
                    return super::report("build a candidate before handing it over".to_owned());
                };
                match Handoff::new(session.role, to, candidate, shared.clone()) {
                    Ok(record) => {
                        let lines = vec![format!(
                            "handoff {} -> {} for candidate {}",
                            record.from.directory(),
                            record.to.directory(),
                            record.candidate
                        )];
                        handoff = Some(record);
                        Ok(lines)
                    }
                    Err(reason) => Err(reason),
                }
            }
            "handoff-check" => {
                let revision: u64 = match argument.parse() {
                    Ok(value) => value,
                    Err(_) => return super::report(format!("{argument:?} is not a revision")),
                };
                match (handoff.as_ref(), candidate.as_ref()) {
                    (Some(record), Some(candidate)) => record
                        .check(candidate, revision)
                        .map(|_| vec![format!("handoff valid at revision {revision}")]),
                    _ => Err("create a handoff before checking it".to_owned()),
                }
            }
            "handoff-cleanup" => match handoff.as_ref() {
                Some(record) => Ok(record
                    .cleanup()
                    .iter()
                    .map(|path| format!("cleanup {path}"))
                    .collect()),
                None => Err("create a handoff before listing its material".to_owned()),
            },
            // The durable half of a two-agent session: the journal records the configuration, the
            // candidates and the handoffs, and its save refuses a copy that another process has
            // already advanced (T23.12). Loading restores the last candidate and handoff so a
            // reviewer can continue in a new process.
            "session-save" => {
                if argument.is_empty() {
                    return super::report("session-save needs a path".to_owned());
                }
                if journal.is_none() {
                    journal = Some(Journal::new(SessionRecord::of(&session)));
                }
                let entry = journal.as_mut().expect("the journal was just created");
                entry.set_session(SessionRecord::of(&session));
                if let Some(candidate) = candidate.as_ref()
                    && let Err(reason) = entry.add_candidate(candidate.clone())
                {
                    return super::report(reason);
                }
                if let Some(record) = handoff.as_ref()
                    && !entry.handoffs().contains(record)
                {
                    entry.add_handoff(record.clone());
                }
                match entry.save(std::path::Path::new(argument)) {
                    Ok(seq) => Ok(vec![format!(
                        "journal {argument} saved at sequence {seq} (format {}): {} candidate(s), {} handoff(s), {} usage record(s)",
                        crate::app::ai::JOURNAL_VERSION,
                        entry.candidates().len(),
                        entry.handoffs().len(),
                        entry.session().usage.len()
                    )]),
                    Err(reason) => Err(reason),
                }
            }
            "session-load" => {
                if argument.is_empty() {
                    return super::report("session-load needs a path".to_owned());
                }
                match Journal::load(std::path::Path::new(argument)) {
                    Ok(loaded) => {
                        session = loaded.session().restore();
                        candidate = loaded.candidates().last().cloned();
                        handoff = loaded.handoffs().last().cloned();
                        let lines = vec![
                            format!(
                                "journal {argument} at sequence {}: {} candidate(s), {} handoff(s), {} usage record(s)",
                                loaded.seq(),
                                loaded.candidates().len(),
                                loaded.handoffs().len(),
                                loaded.session().usage.len()
                            ),
                            format!(
                                "restored role {}, model {:?}, budget {}",
                                match session.role {
                                    Role::Coder => "coder",
                                    Role::Reviewer => "reviewer",
                                },
                                session.model,
                                session.budget
                            ),
                        ];
                        journal = Some(loaded);
                        Ok(lines)
                    }
                    Err(reason) => Err(reason),
                }
            }
            "credential" => {
                session.credential = argument.to_owned();
                Ok(vec![format!("credential {argument}")])
            }
            // The usage ledger (T23.07): a call's bytes, reported cost, duration and how it
            // ended are recorded together, so a broken stream and a cancellation are distinct
            // facts rather than a failed request with no accounting.
            "prompt-bytes" => match argument.parse() {
                Ok(value) => {
                    prompt_bytes = value;
                    Ok(vec![format!("prompt bytes {value}")])
                }
                Err(_) => Err(format!("{argument:?} is not a byte count")),
            },
            "completion-bytes" => match argument.parse() {
                Ok(value) => {
                    completion_bytes = value;
                    Ok(vec![format!("completion bytes {value}")])
                }
                Err(_) => Err(format!("{argument:?} is not a byte count")),
            },
            "cost-micros" => match argument.parse() {
                Ok(value) => {
                    cost_micros = value;
                    Ok(vec![format!("cost {value} us")])
                }
                Err(_) => Err(format!("{argument:?} is not a micro-unit cost")),
            },
            "elapsed-ms" => match argument.parse() {
                Ok(value) => {
                    elapsed_ms = value;
                    Ok(vec![format!("elapsed {value} ms")])
                }
                Err(_) => Err(format!("{argument:?} is not a millisecond duration")),
            },
            "completed" => {
                outcome = InvocationOutcome::Completed;
                Ok(vec!["outcome completed".to_owned()])
            }
            "cancelled" => {
                outcome = InvocationOutcome::Cancelled;
                Ok(vec!["outcome cancelled".to_owned()])
            }
            "broken" => {
                let reason = if argument.is_empty() {
                    "the stream ended without a reason".to_owned()
                } else {
                    argument.to_owned()
                };
                outcome = InvocationOutcome::Broken { reason };
                Ok(vec!["outcome broken".to_owned()])
            }
            "record" => {
                let record = UsageRecord {
                    model: session.model.clone(),
                    role: session.role,
                    prompt_bytes,
                    completion_bytes,
                    cost_micros,
                    elapsed_ms,
                    outcome: outcome.clone(),
                };
                match session.record(record) {
                    Ok(()) => {
                        // One record is one call: the counters start clean so the next call cannot
                        // inherit this one's bytes or cost.
                        prompt_bytes = 0;
                        completion_bytes = 0;
                        cost_micros = 0;
                        elapsed_ms = 0;
                        outcome = InvocationOutcome::Completed;
                        Ok(vec!["recorded one call".to_owned()])
                    }
                    Err(reason) => Err(reason),
                }
            }
            "usage" => {
                let mut lines = vec![session.totals().render()];
                for record in session.usage() {
                    lines.push(record.render());
                }
                Ok(lines)
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
                let payload = selected
                    .sendable_payload()
                    .map(|payload| payload.len())
                    .unwrap_or(0);
                let lines = vec![
                    format!("authorized {} file(s)", selected.files.len()),
                    format!("ignored: {}", selected.ignored.join(", ")),
                    format!("redacted: {}", selected.redacted.join(", ")),
                    format!("payload {payload} byte(s)"),
                ];
                context = Some(selected);
                Ok(lines)
            }
            "context-show" => {
                let selected = match context.as_ref() {
                    Some(selected) => selected,
                    None => return super::report("authorize the context first".to_owned()),
                };
                let sendable = match selected.sendable_payload() {
                    Ok(payload) => format!("sendable {} byte(s)", payload.len()),
                    Err(error) => format!("not sendable: {error}"),
                };
                Ok(vec![
                    format!("files: {}", selected.files.len()),
                    format!("authorized: {}", selected.authorized),
                    format!("payload {} byte(s)", selected.payload_len()),
                    sendable,
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
            "root" => {
                root = Some(std::path::PathBuf::from(argument));
                Ok(vec![format!("project root {argument}")])
            }
            "candidate" => match argument.parse() {
                Ok(id) => {
                    let set = ChangeSet {
                        version: 1,
                        base_revision,
                        changes: changes.clone(),
                    };
                    if let Some(root) = &root
                        && let Err(reason) = set.validate_in(root)
                    {
                        return super::report(reason);
                    }
                    let built = match Candidate::new_at(id, set, base_revision) {
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
