// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! F17: the shared business core, the typed channel, the action queue, the candidate build, the
//! migration, the swap, the readiness rules and the vector regression.
//!
//! One command handler per parsed command of this domain; the dispatcher lives in [`super`](self). A
//! refusal prints the reason and answers false, so a command that found a problem exits through the
//! host's `Reported` code rather than looking like a success.

use super::*;
use crate::app::logic::core::{Action, Core, Event, Guard, Need, Seam, Transition, Value};
use crate::app::logic::ipc::Kind as FrameKind;

/// Splits `text` into `count` fields on `:`, the last field absorbing any remaining colons.
fn split_fields(text: &str, count: usize) -> Option<Vec<&str>> {
    let mut parts = Vec::with_capacity(count);
    let mut rest = text;
    for _ in 1..count {
        let (head, tail) = rest.split_once(':')?;
        parts.push(head);
        rest = tail;
    }
    parts.push(rest);
    Some(parts)
}

/// Parses `yes`/`true` and `no`/`false`.
fn flag(value: &str) -> Option<bool> {
    match value {
        "yes" | "true" => Some(true),
        "no" | "false" => Some(false),
        _ => None,
    }
}

/// Parses a whole number.
fn count(value: &str) -> Result<u32, String> {
    value
        .parse()
        .map_err(|_| format!("{value:?} is not a whole number"))
}

/// Parses `field=<type>:<value>`.
fn assignment(text: &str) -> Result<(String, Value), String> {
    let (field, value) = text
        .split_once('=')
        .ok_or_else(|| format!("{text:?} is not <field>=<type>:<value>"))?;
    let value = Value::parse(value).ok_or_else(|| format!("{value:?} is not <type>:<value>"))?;
    Ok((field.to_owned(), value))
}

/// Reduces events against the shared business core (task T17.01).
pub(super) fn logic_core(options: &crate::cli::LogicCoreOptions) -> bool {
    use crate::app::logic::core::drive;

    fn find<'a>(
        transitions: &'a mut [Transition],
        state: &str,
        event: &str,
    ) -> Option<&'a mut Transition> {
        transitions
            .iter_mut()
            .find(|transition| transition.state == state && transition.event == event)
    }

    let mut initial = String::from("idle");
    let mut transitions: Vec<Transition> = Vec::new();
    let mut seam = Seam::new();
    let mut events: Vec<Event> = Vec::new();
    let mut execute = false;

    for op in &options.ops {
        let (verb, rest) = op.split_once(':').unwrap_or((op.as_str(), ""));
        match verb {
            "init" => initial = rest.to_owned(),
            "transition" => {
                let Some(fields) = split_fields(rest, 3) else {
                    return report(format!(
                        "transition needs <state>:<event>:<next>, got {rest:?}"
                    ));
                };
                let (state, event, next) = (fields[0], fields[1], fields[2]);
                if find(&mut transitions, state, event).is_some() {
                    return report(format!("{state}/{event} already has a transition"));
                }
                transitions.push(Transition::new(state, event, next));
            }
            "guard" => {
                let Some(fields) = split_fields(rest, 3) else {
                    return report(format!(
                        "guard needs <state>:<event>:<field>=<type>:<value>, got {rest:?}"
                    ));
                };
                let (state, event, spec) = (fields[0], fields[1], fields[2]);
                let Ok((field, equals)) = assignment(spec) else {
                    return report(format!("{spec:?} is not <field>=<type>:<value>"));
                };
                let Some(transition) = find(&mut transitions, state, event) else {
                    return report(format!("there is no transition {state}/{event} to guard"));
                };
                transition.guard = Guard::Field { field, equals };
            }
            "set" => {
                let Some(fields) = split_fields(rest, 3) else {
                    return report(format!(
                        "set needs <state>:<event>:<field>=<type>:<value>, got {rest:?}"
                    ));
                };
                let (state, event, spec) = (fields[0], fields[1], fields[2]);
                let Ok((field, value)) = assignment(spec) else {
                    return report(format!("{spec:?} is not <field>=<type>:<value>"));
                };
                let Some(transition) = find(&mut transitions, state, event) else {
                    return report(format!("there is no transition {state}/{event} to change"));
                };
                transition.actions.push(Action::Set { field, value });
            }
            "need" => {
                let Some(fields) = split_fields(rest, 4) else {
                    return report(format!(
                        "need needs <state>:<event>:<function>:<into>=<type>:<value>, got {rest:?}"
                    ));
                };
                let (state, event, function, spec) = (fields[0], fields[1], fields[2], fields[3]);
                let Ok((into, argument)) = assignment(spec) else {
                    return report(format!("{spec:?} is not <into>=<type>:<value>"));
                };
                let Some(transition) = find(&mut transitions, state, event) else {
                    return report(format!(
                        "there is no transition {state}/{event} to ask from"
                    ));
                };
                transition.need = Some(Need {
                    function: function.to_owned(),
                    argument,
                    into,
                });
            }
            "seam" => {
                let Some((function, value)) = rest.split_once('=') else {
                    return report(format!(
                        "seam needs <function>=<type>:<value>, got {rest:?}"
                    ));
                };
                let Some(value) = Value::parse(value) else {
                    return report(format!("{value:?} is not <type>:<value>"));
                };
                seam = seam.with(function, value);
            }
            "event" => {
                let Some((name, value)) = rest.split_once('=') else {
                    return report(format!("event needs <name>=<type>:<value>, got {rest:?}"));
                };
                let Some(value) = Value::parse(value) else {
                    return report(format!("{value:?} is not <type>:<value>"));
                };
                events.push(Event::new(name, value));
            }
            "run" => execute = true,
            "show" => {}
            other => return report(format!("{other:?} is not a logic-core operation")),
        }
    }

    let declared = transitions.len();
    let mut core = Core::new(initial.clone());
    for transition in transitions {
        if let Err(refusal) = core.add(transition) {
            return report(refusal.render());
        }
    }
    println!("core: {declared} transition(s), starts in {initial}");
    if !execute {
        return true;
    }
    match drive(&core, &core.start(), &events, &seam) {
        Ok(steps) => {
            for step in &steps {
                for line in step.render() {
                    println!("{line}");
                }
            }
            println!("{} step(s)", steps.len());
            true
        }
        Err(refusal) => report(refusal.render()),
    }
}

/// One thing to do on the channel, collected so the session can be opened first.
enum Act {
    /// Send a frame.
    Send(FrameKind, u64, String),
    /// Receive a frame the caller wrote out.
    Receive(String),
}

/// Opens a typed local channel and moves frames (task T17.02).
pub(super) fn logic_ipc(options: &crate::cli::LogicIpcOptions) -> bool {
    use crate::app::logic::ipc::{Frame, Kind, Manifest, Protocol, Session};

    let mut id = String::new();
    let mut version = Protocol::VERSION;
    let mut generation = 0u64;
    let mut name = String::new();
    let mut process_version = String::new();
    let mut capabilities: Vec<String> = Vec::new();
    let mut acts: Vec<Act> = Vec::new();

    for op in &options.ops {
        let (verb, rest) = op.split_once(':').unwrap_or((op.as_str(), ""));
        match verb {
            "session" => id = rest.to_owned(),
            "version" => match count(rest) {
                Ok(value) => version = value,
                Err(reason) => return report(reason),
            },
            "generation" => match rest.parse() {
                Ok(value) => generation = value,
                Err(_) => return report(format!("{rest:?} is not a generation")),
            },
            "manifest" => {
                let Some(fields) = split_fields(rest, 2) else {
                    return report(format!("manifest needs <name>:<version>, got {rest:?}"));
                };
                let (manifest_name, manifest_version) = (fields[0], fields[1]);
                name = manifest_name.to_owned();
                process_version = manifest_version.to_owned();
            }
            "capability" => capabilities.push(rest.to_owned()),
            "frame" => {
                let Some(fields) = split_fields(rest, 3) else {
                    return report(format!(
                        "frame needs <kind>:<generation>:<payload>, got {rest:?}"
                    ));
                };
                let (kind, at, payload) = (fields[0], fields[1], fields[2]);
                let Some(kind) = Kind::parse(kind) else {
                    return report(format!("{kind:?} is not a frame kind"));
                };
                let Ok(at) = at.parse() else {
                    return report(format!("{at:?} is not a generation"));
                };
                acts.push(Act::Send(kind, at, payload.to_owned()));
            }
            "receive" => acts.push(Act::Receive(rest.to_owned())),
            "open" => {}
            other => return report(format!("{other:?} is not a logic-ipc operation")),
        }
    }

    if id.is_empty() {
        return report("a session needs `session:<id>`".to_owned());
    }
    let mut manifest = Manifest::new(name.clone(), process_version.clone(), generation);
    for capability in capabilities {
        manifest = manifest.with(capability);
    }
    let session = match Session::open(id.clone(), version, manifest, generation) {
        Ok(session) => session,
        Err(fault) => return report(fault.render()),
    };
    println!(
        "session {id}: protocol v{}, generation {generation}, manifest {name} {process_version}",
        session.protocol
    );

    for act in &acts {
        match act {
            Act::Send(kind, at, payload) => {
                let frame = Frame::new(*kind, id.clone(), *at, payload.clone().into_bytes());
                match session.send(&frame) {
                    Ok(encoded) => println!("{encoded}"),
                    Err(fault) => return report(fault.render()),
                }
            }
            Act::Receive(text) => match session.receive(text) {
                Ok(frame) => println!(
                    "received {} for {} at generation {} ({} bytes)",
                    frame.kind.name(),
                    frame.session,
                    frame.generation,
                    frame.payload.len()
                ),
                Err(fault) => return report(fault.render()),
            },
        }
    }
    true
}

/// Validates actions and queues them across the boundary (task T17.03).
pub(super) fn logic_actions(options: &crate::cli::LogicActionsOptions) -> bool {
    use crate::app::logic::actions::{Admission, Class, Item, Queue};

    let mut queue = Queue::new(8);
    let mut now = 0u32;
    let mut finding = false;

    for op in &options.ops {
        let (verb, rest) = op.split_once(':').unwrap_or((op.as_str(), ""));
        match verb {
            "capacity" => match rest.parse() {
                Ok(capacity) => queue = Queue::new(capacity),
                Err(_) => return report(format!("{rest:?} is not a capacity")),
            },
            "item" => {
                let Some(fields) = split_fields(rest, 3) else {
                    return report(format!(
                        "item needs <event>:<class>:<needs|ui>[:<deadline>], got {rest:?}"
                    ));
                };
                let (event, class, spec) = (fields[0], fields[1], fields[2]);
                let Some(class) = Class::parse(class) else {
                    return report(format!("{class:?} is not an event class"));
                };
                let (mode, deadline) = match spec.split_once(':') {
                    Some((mode, deadline)) => (mode, Some(deadline)),
                    None => (spec, None),
                };
                let needs_logic = match mode {
                    "needs" => true,
                    "ui" => false,
                    other => return report(format!("{other:?} is not needs or ui")),
                };
                let mut item = Item::new(event, class, needs_logic);
                if let Some(deadline) = deadline {
                    match count(deadline) {
                        Ok(deadline) => item = item.by(deadline),
                        Err(reason) => return report(reason),
                    }
                }
                match queue.push(item, now) {
                    Ok(Admission::Admitted) => println!("{event}: admitted"),
                    Ok(Admission::StaysInUi) => println!("{event}: stays in the UI"),
                    Ok(Admission::Overflow { dropped }) => {
                        finding = true;
                        println!("{event}: admitted, dropped {}", dropped.render());
                    }
                    Ok(Admission::Expired { item }) => {
                        finding = true;
                        println!(
                            "{event}: expired at {} (deadline {})",
                            now,
                            item.deadline.unwrap_or_default()
                        );
                    }
                    Err(refusal) => return report(refusal.render()),
                }
            }
            "take" => match queue.take() {
                Some(item) => println!("taken {}", item.render()),
                None => println!("nothing is waiting"),
            },
            "tick" => {
                let Ok(at) = count(rest) else {
                    return report(format!("{rest:?} is not a time"));
                };
                now = at;
                let expired = queue.expire(now);
                if !expired.is_empty() {
                    finding = true;
                }
                for expiry in &expired {
                    println!("{}", expiry.render());
                }
            }
            "check" | "show" => {}
            other => return report(format!("{other:?} is not a logic-actions operation")),
        }
    }
    println!("queue: {} of {} waiting", queue.len(), queue.capacity);
    !finding
}

/// Debounces edits and admits a candidate build (task T17.04).
pub(super) fn logic_candidate(options: &crate::cli::LogicCandidateOptions) -> bool {
    use crate::app::logic::candidate::{
        Artifact, Build, BuildAction, Candidate, Debounce, Outcome,
    };
    use crate::contracts::revision::{Revision, VersionStamp};

    let mut current = Revision::INITIAL;
    let mut debounce = Debounce::new(0);
    let mut action = None;
    let mut outcome = None;
    let mut stamp = None;
    let mut artifact = None;
    let mut running = None;
    let mut admit = false;
    let mut build_manifest = None;
    let mut target_dir = None;
    let mut binary = None;

    for op in &options.ops {
        let (verb, rest) = op.split_once(':').unwrap_or((op.as_str(), ""));
        match verb {
            "revision" => match rest.parse() {
                Ok(revision) => current = Revision::from_raw(revision),
                Err(_) => return report(format!("{rest:?} is not a revision")),
            },
            "window" => match rest.parse() {
                Ok(window) => debounce = Debounce::new(window),
                Err(_) => return report(format!("{rest:?} is not a window")),
            },
            "edit" => debounce.push(rest, 0),
            "flush" => {
                let Ok(at) = rest.parse() else {
                    return report(format!("{rest:?} is not a time"));
                };
                match debounce.flush(at) {
                    Some(batch) => println!("build triggered by {}", batch.join(", ")),
                    None => println!("still debouncing ({} waiting)", debounce.pending()),
                }
            }
            "action" => match BuildAction::parse(rest) {
                Some(value) => action = Some(value),
                None => return report(format!("{rest:?} is not a build action")),
            },
            "outcome" => match Outcome::parse(rest) {
                Some(value) => outcome = Some(value),
                None => return report(format!("{rest:?} is not an outcome")),
            },
            "stamp" => match rest.parse() {
                Ok(revision) => stamp = Some(Revision::from_raw(revision)),
                Err(_) => return report(format!("{rest:?} is not a revision")),
            },
            "artifact" => artifact = Some(rest.to_owned()),
            "running" => running = Some(rest.to_owned()),
            "build" => build_manifest = Some(rest.to_owned()),
            "target-dir" => target_dir = Some(rest.to_owned()),
            "bin" => binary = Some(rest.to_owned()),
            "admit" => admit = true,
            "check" | "show" => {}
            other => return report(format!("{other:?} is not a logic-candidate operation")),
        }
    }

    if build_manifest.is_some()
        && (action.is_some() || outcome.is_some() || stamp.is_some() || artifact.is_some())
    {
        return report(
            "`build:` determines the action, outcome and artifact; pass `revision:` for the version \
             instead of action:/outcome:/stamp:/artifact:"
                .to_owned(),
        );
    }
    let build = if let Some(manifest) = build_manifest {
        match build_candidate(&manifest, target_dir.as_deref(), binary.as_deref(), current) {
            Ok(build) => Some(build),
            Err(reason) => return report(reason),
        }
    } else {
        None
    };

    if !admit {
        match &build {
            Some(build) => {
                if build.outcome != Outcome::Success {
                    return report(
                        "the candidate build failed; there is nothing to admit".to_owned(),
                    );
                }
                println!(
                    "candidate build: {} (revision {})",
                    build
                        .artifact
                        .as_ref()
                        .map(|artifact| artifact.path.as_str())
                        .unwrap_or("no artifact"),
                    current
                );
            }
            None => println!(
                "candidate: revision {current}, {} source(s) debounced",
                debounce.pending()
            ),
        }
        return true;
    }

    let build = if let Some(build) = build {
        build
    } else {
        let Some(action) = action else {
            return report(
                "a build needs `action:<check|build|rebuild>` or `build:<manifest>`".to_owned(),
            );
        };
        let Some(outcome) = outcome else {
            return report("a build needs `outcome:<success|failure>`".to_owned());
        };
        let built_from = stamp.unwrap_or(current);
        let stamp = VersionStamp::current(built_from);
        Build {
            action,
            outcome,
            artifact: artifact.map(|path| Artifact { path, stamp }),
            stamp,
        }
    };
    let candidate = match Candidate::from_build(&build, current) {
        Ok(candidate) => candidate,
        Err(refusal) => return report(refusal.render()),
    };
    match candidate.may_replace(running.as_deref().unwrap_or_default()) {
        Ok(()) => println!(
            "candidate {} may replace the running program",
            candidate.path
        ),
        Err(refusal) => return report(refusal.render()),
    }
    true
}

/// Builds a real candidate artifact from a crate, beside the running program (task T17.04).
///
/// This is the difference between "a build was described" and "a build ran": the exit code is the
/// verdict, the artifact is checked to exist, and the target directory is separate from the running
/// binary so starting the candidate cannot overwrite the program that is serving. The artifact name
/// follows cargo's `--bin` (or the manifest's directory name when the caller does not name one).
fn build_candidate(
    manifest: &str,
    target_dir: Option<&str>,
    binary: Option<&str>,
    revision: crate::contracts::revision::Revision,
) -> Result<crate::app::logic::candidate::Build, String> {
    use crate::app::logic::candidate::{Artifact, Build, BuildAction, Outcome};
    use crate::contracts::revision::VersionStamp;
    use std::path::PathBuf;

    let manifest_path = PathBuf::from(manifest);
    if !manifest_path.is_file() {
        return Err(format!("{manifest:?} is not a crate manifest"));
    }
    let target = match target_dir {
        Some(path) => PathBuf::from(path),
        None => std::env::temp_dir().join("rustage-logic-candidate"),
    };
    let name = match binary {
        Some(name) => name.to_owned(),
        None => manifest_path
            .parent()
            .and_then(|parent| parent.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .ok_or_else(|| format!("{manifest:?} has no directory to name its binary"))?,
    };

    let mut command = std::process::Command::new("cargo");
    command
        .arg("build")
        .arg("--manifest-path")
        .arg(&manifest_path)
        .arg("--target-dir")
        .arg(&target)
        .arg("--bin")
        .arg(&name);
    println!(
        "cargo build --manifest-path {} --target-dir {} --bin {name}",
        manifest_path.display(),
        target.display()
    );
    let output = command
        .output()
        .map_err(|error| format!("cargo could not be started: {error}"))?;
    let artifact = target
        .join("debug")
        .join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    let success = output.status.success() && artifact.is_file();
    if !success {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let tail: Vec<&str> = stderr.lines().rev().take(8).collect();
        for line in tail.iter().rev() {
            println!("  {line}");
        }
    }
    let stamp = VersionStamp::current(revision);
    Ok(Build {
        action: BuildAction::Build,
        outcome: if success {
            Outcome::Success
        } else {
            Outcome::Failure
        },
        artifact: artifact.is_file().then(|| Artifact {
            path: artifact.display().to_string(),
            stamp,
        }),
        stamp,
    })
}

/// Plans a state migration and confirms a reset (task T17.05).
pub(super) fn logic_migration(options: &crate::cli::LogicMigrationOptions) -> bool {
    use crate::app::logic::migration::{Ownership, Schema, Snapshot, apply, plan, reset};

    let mut from_version = 1u32;
    let mut to_version = 1u32;
    let mut source = Schema::new(from_version);
    let mut target = Schema::new(to_version);
    let mut snapshot = Snapshot::new(from_version);
    let mut ownership = Ownership::new("generation");
    let mut confirm = false;
    let mut actions: Vec<&str> = Vec::new();

    for op in &options.ops {
        let (verb, rest) = op.split_once(':').unwrap_or((op.as_str(), ""));
        match verb {
            "from" => match rest.parse() {
                Ok(version) => {
                    from_version = version;
                    source = Schema::new(version);
                    snapshot = Snapshot::new(version);
                }
                Err(_) => return report(format!("{rest:?} is not a version")),
            },
            "to" => match rest.parse() {
                Ok(version) => {
                    to_version = version;
                    target = Schema::new(version);
                }
                Err(_) => return report(format!("{rest:?} is not a version")),
            },
            "from-field" => match field(rest) {
                Ok(field) => source.fields.push(field),
                Err(reason) => return report(reason),
            },
            "to-field" => match field(rest) {
                Ok(field) => target.fields.push(field),
                Err(reason) => return report(reason),
            },
            "value" => {
                let Ok((name, value)) = assignment(rest) else {
                    return report(format!("{rest:?} is not <name>=<type>:<value>"));
                };
                snapshot = snapshot.with(name, value);
            }
            "owner" => ownership = Ownership::new(rest),
            "task" => ownership = ownership.own(rest),
            "plan" => actions.push("plan"),
            "reset" => actions.push("reset"),
            "confirm" => confirm = true,
            "check" | "show" => {}
            other => return report(format!("{other:?} is not a logic-migration operation")),
        }
    }

    println!(
        "schema v{from_version} -> v{to_version}: {} source field(s), {} target field(s)",
        source.fields.len(),
        target.fields.len()
    );
    for action in actions {
        match action {
            "plan" => {
                let migration = match plan(&source, &target, &snapshot) {
                    Ok(migration) => migration,
                    Err(fault) => return report(fault.render()),
                };
                for line in migration.render() {
                    println!("{line}");
                }
                if migration.blocks_auto_switch() {
                    return report(format!(
                        "a migration loses {}; the switch must not be automatic",
                        migration.lost().join(", ")
                    ));
                }
                let migrated = apply(&migration, &snapshot);
                println!("migrated: {} field(s) carried", migrated.values.len());
            }
            _ => match reset(&ownership, confirm) {
                Ok(reset) => println!("reset discarded {} task(s)", reset.discarded.len()),
                Err(refusal) => return report(refusal.render()),
            },
        }
    }
    true
}

/// A field from `<name>:<type>:<since>`.
fn field(text: &str) -> Result<crate::app::logic::migration::Field, String> {
    use crate::app::logic::migration::{Field, FieldType};
    let Some(fields) = split_fields(text, 3) else {
        return Err(format!("a field needs <name>:<type>:<since>, got {text:?}"));
    };
    let (name, kind, since) = (fields[0], fields[1], fields[2]);
    if name.is_empty() {
        return Err(format!("a field needs a non-empty name, got {text:?}"));
    }
    let kind = FieldType::parse(kind).ok_or_else(|| format!("{kind:?} is not a field type"))?;
    let since = since
        .parse()
        .map_err(|_| format!("{since:?} is not a version"))?;
    Ok(Field {
        name: name.to_owned(),
        kind,
        since,
    })
}

/// Walks the controlled replacement (task T17.06).
pub(super) fn logic_swap(options: &crate::cli::LogicSwapOptions) -> bool {
    use crate::app::logic::migration::Snapshot;
    use crate::app::logic::swap::Swap;

    let mut old = String::from("logic-1");
    let mut new = String::from("logic-2");
    let mut limit = 300u32;
    let mut swap: Option<Swap> = None;

    for op in &options.ops {
        let (verb, rest) = op.split_once(':').unwrap_or((op.as_str(), ""));
        match verb {
            "old" => old = rest.to_owned(),
            "new" => new = rest.to_owned(),
            "limit" => match count(rest) {
                Ok(value) => limit = value,
                Err(reason) => return report(reason),
            },
            _ => {
                let swap = swap.get_or_insert_with(|| Swap::begin(old.clone(), new.clone(), limit));
                match verb {
                    "pause" => {
                        let Ok(elapsed) = count(rest) else {
                            return report(format!("{rest:?} is not a time"));
                        };
                        if let Err(outcome) = swap.pause_ticked(elapsed) {
                            return report(outcome.render());
                        }
                        match swap.paused() {
                            Ok(phase) => {
                                println!("pause at {elapsed}, limit {limit}: {}", phase.name());
                            }
                            Err(refusal) => return report(refusal.render()),
                        }
                    }
                    "drain" => match swap.drain() {
                        Ok(phase) => println!("{}", phase.name()),
                        Err(refusal) => return report(refusal.render()),
                    },
                    "snapshot" => {
                        let Ok(version) = rest.parse() else {
                            return report(format!("{rest:?} is not a version"));
                        };
                        match swap.snapshot(Snapshot::new(version)) {
                            Ok(phase) => println!("{} (v{version})", phase.name()),
                            Err(refusal) => return report(refusal.render()),
                        }
                    }
                    "import" => match flag(rest) {
                        Some(imported) => {
                            if let Err(outcome) = swap.import(imported) {
                                return report(outcome.render());
                            }
                            println!("{}", swap.phase().name());
                        }
                        None => return report(format!("{rest:?} is not yes or no")),
                    },
                    "confirm" => match swap.confirm() {
                        Ok(phase) => println!("{}", phase.name()),
                        Err(refusal) => return report(refusal.render()),
                    },
                    "resume" => match swap.resume() {
                        Ok(phase) => println!("{}", phase.name()),
                        Err(refusal) => return report(refusal.render()),
                    },
                    "retire" => {
                        if !swap.retire_old() {
                            return report(format!(
                                "the old version cannot be retired while the swap is {}",
                                swap.phase().name()
                            ));
                        }
                        println!("retired; the old version is gone");
                    }
                    "fail" => {
                        swap.fail(rest);
                        println!("aborted: {}", swap.phase().name());
                    }
                    "check" | "show" => {}
                    other => {
                        return report(format!("{other:?} is not a logic-swap operation"));
                    }
                }
            }
        }
    }

    let Some(swap) = swap else {
        return report("a swap needs `old:<name>`, `new:<name>` and `limit:<ms>` first".to_owned());
    };
    println!(
        "swap {} -> {}: phase {}, old version {}",
        swap.old,
        swap.new,
        swap.phase().name(),
        if swap.old_running() {
            "still running"
        } else {
            "retired"
        }
    );
    if let Some(reason) = swap.reason() {
        println!("reason: {reason}");
    }
    swap.phase() != crate::app::logic::swap::Phase::Aborted
}

/// Judges readiness and isolates a late result (task T17.07).
pub(super) fn logic_combination(options: &crate::cli::LogicCombinationOptions) -> bool {
    use crate::app::logic::combination::{
        Change, Computed, Conditions, Interaction, Plan, deliver, readiness,
    };

    let mut conditions = Conditions {
        building: false,
        protocol_matches: true,
    };
    let mut plan = Plan::new();
    let mut interactions: Vec<Interaction> = Vec::new();
    let mut generation = 1u64;
    let mut result: Option<Computed> = None;

    for op in &options.ops {
        let (verb, rest) = op.split_once(':').unwrap_or((op.as_str(), ""));
        match verb {
            "building" => match flag(rest) {
                Some(value) => conditions.building = value,
                None => return report(format!("{rest:?} is not yes or no")),
            },
            "protocol" => match flag(rest) {
                Some(value) => conditions.protocol_matches = value,
                None => return report(format!("{rest:?} is not yes or no")),
            },
            "plan" => match Change::parse(rest) {
                Some(change) => plan = plan.with(change),
                None => return report(format!("{rest:?} is not a change")),
            },
            "interaction" => interactions.push(Interaction::new(rest)),
            "needs" => {
                let Some(change) = Change::parse(rest) else {
                    return report(format!("{rest:?} is not a change"));
                };
                let Some(interaction) = interactions.last_mut() else {
                    return report("needs needs an interaction first".to_owned());
                };
                interaction.needs.push(change);
            }
            "generation" => match rest.parse() {
                Ok(value) => generation = value,
                Err(_) => return report(format!("{rest:?} is not a generation")),
            },
            "result" => {
                let Some((at, value)) = rest.split_once(':') else {
                    return report(format!("result needs <generation>:<value>, got {rest:?}"));
                };
                let Ok(at) = at.parse() else {
                    return report(format!("{at:?} is not a generation"));
                };
                result = Some(Computed::new(at, value));
            }
            "check" | "show" => {}
            other => return report(format!("{other:?} is not a logic-combination operation")),
        }
    }

    println!(
        "plan: {} change(s); needs a logic build: {}",
        plan.changes.len(),
        if plan.requires_logic_build() {
            "yes"
        } else {
            "no"
        }
    );
    for interaction in &interactions {
        println!(
            "{}: {}",
            interaction.name,
            readiness(interaction, &conditions).render()
        );
    }
    if let Some(computed) = result {
        println!("{}", deliver(computed, generation).render());
    }
    true
}

/// Runs the event vectors and decides on a fault (task T17.08).
pub(super) fn logic_regression(options: &crate::cli::LogicRegressionOptions) -> bool {
    use crate::app::logic::regression::{Fault, Vector, decide, run};

    let mut initial = String::from("idle");
    let mut transitions: Vec<Transition> = Vec::new();
    let mut vectors: Vec<Vector> = Vec::new();
    let mut faults: Vec<Fault> = Vec::new();
    let mut execute = false;

    for op in &options.ops {
        let (verb, rest) = op.split_once(':').unwrap_or((op.as_str(), ""));
        match verb {
            "init" => initial = rest.to_owned(),
            "transition" => {
                let Some(fields) = split_fields(rest, 3) else {
                    return report(format!(
                        "transition needs <state>:<event>:<next>, got {rest:?}"
                    ));
                };
                let (state, event, next) = (fields[0], fields[1], fields[2]);
                if transitions
                    .iter()
                    .any(|known| known.state == state && known.event == event)
                {
                    return report(format!("{state}/{event} already has a transition"));
                }
                transitions.push(Transition::new(state, event, next));
            }
            "vector" => vectors.push(Vector::new(rest)),
            "say" => {
                let Some(vector) = vectors.last_mut() else {
                    return report("say needs a vector first".to_owned());
                };
                vector.events.push(Event::new(rest, Value::Flag(true)));
            }
            "fault" => {
                let Some((kind, argument)) = rest.split_once(':') else {
                    return report(format!("fault needs <kind>:<argument>, got {rest:?}"));
                };
                let fault = match kind {
                    "unknown" => Fault::UnknownSideEffect {
                        call: argument.to_owned(),
                    },
                    "timeout" => Fault::Timeout {
                        step: argument.to_owned(),
                    },
                    "crash" => match argument.parse() {
                        Ok(generation) => Fault::PostSwapCrash { generation },
                        Err(_) => return report(format!("{argument:?} is not a generation")),
                    },
                    "rapid" => match argument.parse() {
                        Ok(count) => Fault::RapidChange { count },
                        Err(_) => return report(format!("{argument:?} is not a count")),
                    },
                    other => return report(format!("{other:?} is not a fault kind")),
                };
                println!("{}", decide(&fault).render());
                faults.push(fault);
            }
            "run" => execute = true,
            "decide" => {}
            "check" | "show" => {}
            other => return report(format!("{other:?} is not a logic-regression operation")),
        }
    }

    let declared = transitions.len();
    let mut core = Core::new(initial.clone());
    for transition in transitions {
        if let Err(refusal) = core.add(transition) {
            return report(refusal.render());
        }
    }
    println!(
        "core: {declared} transition(s); {} vector(s), {} fault(s)",
        vectors.len(),
        faults.len()
    );
    if !execute {
        return true;
    }
    match run(&vectors, &core, &core.start(), &Seam::new()) {
        Ok(reports) => {
            let mut identical = true;
            for report in &reports {
                println!("{}", report.render());
                if !report.identical {
                    identical = false;
                }
            }
            if identical {
                true
            } else {
                report("the channel-carried walk diverged from the direct one".to_owned())
            }
        }
        Err(error) => report(error.render()),
    }
}

/// Serves one business program over standard input/output (task T17.02).
///
/// Nothing but encoded frames is written to standard output, because this end of the channel is a
/// process rather than a report: a stray line would be read as a malformed frame by the host.
pub(super) fn logic_worker(options: &crate::cli::LogicWorkerOptions) -> bool {
    use crate::app::logic::worker::{Program, Worker, serve};

    let mut name = String::from("logic");
    let mut version = String::from("0.1.0");
    let mut initial = String::from("idle");
    let mut transitions = Vec::new();
    let mut schema_fields = Vec::new();
    let mut schema_version = 1u32;
    for op in &options.ops {
        let (verb, rest) = op.split_once(':').unwrap_or((op.as_str(), ""));
        match verb {
            "name" => name = rest.to_owned(),
            "version" => version = rest.to_owned(),
            "initial" => initial = rest.to_owned(),
            "schema-version" => match rest.parse() {
                Ok(value) => schema_version = value,
                Err(_) => {
                    eprintln!("rustage: {rest:?} is not a schema version");
                    return false;
                }
            },
            "schema" => match field(rest) {
                Ok(field) => schema_fields.push(field),
                Err(reason) => {
                    eprintln!("rustage: {reason}");
                    return false;
                }
            },
            "transition" => match parse_transition(rest) {
                Ok(transition) => transitions.push(transition),
                Err(reason) => {
                    eprintln!("rustage: {reason}");
                    return false;
                }
            },
            "set" => {
                let Some(fields) = split_fields(rest, 3) else {
                    eprintln!(
                        "rustage: a set needs <state>:<event>:<field>=<type>:<value>, got {rest:?}"
                    );
                    return false;
                };
                let (state, event, spec) = (fields[0], fields[1], fields[2]);
                let Ok((field, value)) = assignment(spec) else {
                    eprintln!("rustage: {spec:?} is not <field>=<type>:<value>");
                    return false;
                };
                let Some(transition) = transitions
                    .iter_mut()
                    .find(|transition| transition.state == state && transition.event == event)
                else {
                    eprintln!("rustage: there is no transition {state}/{event} to change");
                    return false;
                };
                transition.actions.push(Action::Set { field, value });
            }
            other => {
                eprintln!("rustage: {other:?} is not a logic-worker operation");
                return false;
            }
        }
    }
    let mut program = Program::new(name, version, initial);
    if !schema_fields.is_empty() {
        let mut schema = crate::app::logic::migration::Schema::new(schema_version);
        schema.fields = schema_fields;
        program = program.with_schema(schema);
    }
    for transition in transitions {
        match program.with(transition) {
            Ok(next) => program = next,
            Err(refusal) => {
                eprintln!("rustage: {}", refusal.render());
                return false;
            }
        }
    }

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut input = stdin.lock();
    let mut output = stdout.lock();
    match serve(Worker::new(program), &mut input, &mut output) {
        Ok(()) => true,
        Err(error) => {
            eprintln!("rustage: {}", error.render());
            false
        }
    }
}

/// A transition from `<state>:<event>:<next>`.
fn parse_transition(text: &str) -> Result<Transition, String> {
    let Some(fields) = split_fields(text, 3) else {
        return Err(format!(
            "a transition needs <state>:<event>:<next>, got {text:?}"
        ));
    };
    Ok(Transition::new(fields[0], fields[1], fields[2]))
}

/// A set from `<state>:<event>:<field>=<type>:<value>`.
fn parse_set(text: &str) -> Result<(String, String, String), String> {
    let Some(fields) = split_fields(text, 3) else {
        return Err(format!(
            "a set needs <state>:<event>:<field>=<type>:<value>, got {text:?}"
        ));
    };
    Ok((
        fields[0].to_owned(),
        fields[1].to_owned(),
        fields[2].to_owned(),
    ))
}

/// Applies `set` operations to the transitions they name, refusing an unknown transition.
fn apply_sets(
    transitions: &mut [Transition],
    sets: Vec<(String, String, String)>,
) -> Result<(), String> {
    for (state, event, spec) in sets {
        let Some(transition) = transitions
            .iter_mut()
            .find(|transition| transition.state == state && transition.event == event)
        else {
            return Err(format!("there is no transition {state}/{event} to change"));
        };
        let Ok((field, value)) = assignment(&spec) else {
            return Err(format!("{spec:?} is not <field>=<type>:<value>"));
        };
        transition.actions.push(Action::Set { field, value });
    }
    Ok(())
}

/// Drives two real business processes across the typed channel (tasks T17.02 and T17.06).
///
/// The first program is the running version; the second is the candidate. Both are real child
/// processes started from this executable with `--logic-worker`, and every request and answer crosses
/// a pipe as an encoded frame. The walk also sends one frame stamped with the retired generation to
/// the new process, because a generation check only one side performs is not a boundary.
pub(super) fn logic_process(options: &crate::cli::LogicProcessOptions) -> bool {
    use crate::app::logic::ipc::{Frame, Kind};
    use crate::app::logic::migration::Snapshot;
    use crate::app::logic::swap::Swap;
    use crate::app::logic::worker::Program;
    use std::time::{Duration, Instant};

    let mut session_id = String::from("logic-1");
    let mut name = String::from("orders");
    let mut version = String::from("0.1.0");
    let mut first_generation = 1u64;
    let mut second_generation = 2u64;
    let mut first_initial = String::from("idle");
    let mut second_initial = String::from("idle");
    let mut first = Vec::new();
    let mut second = Vec::new();
    let mut first_events = Vec::new();
    let mut second_events = Vec::new();
    let mut first_sets = Vec::new();
    let mut second_sets = Vec::new();
    let mut expected: Vec<String> = Vec::new();
    let mut second_binary = None;
    let mut handoff = false;
    let mut first_schema = Vec::new();
    let mut second_schema = Vec::new();
    let mut from_version = 1u32;
    let mut to_version = None;
    let mut schema_declared = false;
    let mut pause_limit: Option<u32> = None;

    for op in &options.ops {
        let (verb, rest) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let parsed = match verb {
            "transition" => parse_transition(rest).map(|transition| first.push(transition)),
            "first-transition" => parse_transition(rest).map(|transition| first.push(transition)),
            "second-transition" => parse_transition(rest).map(|transition| second.push(transition)),
            "first-set" => parse_set(rest).map(|set| first_sets.push(set)),
            "second-set" => parse_set(rest).map(|set| second_sets.push(set)),
            _ => Ok(()),
        };
        if let Err(reason) = parsed {
            return report(reason);
        }
        match verb {
            "session" => session_id = rest.to_owned(),
            "name" => name = rest.to_owned(),
            "version" => version = rest.to_owned(),
            "generation" => match rest.parse() {
                Ok(value) => first_generation = value,
                Err(_) => return report(format!("{rest:?} is not a generation")),
            },
            "new-generation" => match rest.parse() {
                Ok(value) => second_generation = value,
                Err(_) => return report(format!("{rest:?} is not a generation")),
            },
            "first" => first_initial = rest.to_owned(),
            "second" => second_initial = rest.to_owned(),
            "first-event" => first_events.push(rest.to_owned()),
            "second-event" => second_events.push(rest.to_owned()),
            "second-binary" => second_binary = Some(rest.to_owned()),
            "first-schema" => match field(rest) {
                Ok(field) => {
                    schema_declared = true;
                    first_schema.push(field);
                }
                Err(reason) => return report(reason),
            },
            "second-schema" => match field(rest) {
                Ok(field) => {
                    schema_declared = true;
                    second_schema.push(field);
                }
                Err(reason) => return report(reason),
            },
            "from-version" => match rest.parse() {
                Ok(version) => {
                    from_version = version;
                    schema_declared = true;
                }
                Err(_) => return report(format!("{rest:?} is not a schema version")),
            },
            "to-version" => match rest.parse() {
                Ok(version) => {
                    to_version = Some(version);
                    schema_declared = true;
                }
                Err(_) => return report(format!("{rest:?} is not a schema version")),
            },
            "preserve" | "handoff" => handoff = true,
            "pause-limit" => match rest.parse() {
                Ok(limit) => {
                    pause_limit = Some(limit);
                    // A bounded pause only has a subject when a snapshot is handed off; declaring
                    // one without `preserve` would describe a switch that never happens.
                    handoff = true;
                }
                Err(_) => return report(format!("{rest:?} is not a pause limit in milliseconds")),
            },
            "expect" => expected.push(rest.to_owned()),
            "transition" | "first-transition" | "second-transition" | "first-set"
            | "second-set" | "check" | "show" => {}
            other => return report(format!("{other:?} is not a logic-process operation")),
        }
    }
    if first_generation == second_generation {
        return report(format!(
            "a replacement needs a different generation; both are {first_generation}"
        ));
    }
    if schema_declared {
        if first_schema.is_empty() || second_schema.is_empty() {
            return report(
                "a schema migration needs both first-schema and second-schema; an undeclared side \
                 would silently drop the fields it does not know about"
                    .to_owned(),
            );
        }
        for (label, fields) in [
            ("first-schema", first_schema.as_slice()),
            ("second-schema", second_schema.as_slice()),
        ] {
            let mut seen = std::collections::BTreeSet::new();
            for field in fields {
                if !seen.insert(field.name.as_str()) {
                    return report(format!(
                        "{label} declares field {:?} more than once",
                        field.name
                    ));
                }
            }
        }
        if !handoff {
            return report(
                "a schema migration needs preserve; without a snapshot there is no state to migrate"
                    .to_owned(),
            );
        }
    }
    let target_version = match to_version {
        Some(version) => version,
        None => match from_version.checked_add(1) {
            Some(version) => version,
            None => return report(format!("{from_version} has no next schema version")),
        },
    };
    let source_schema = {
        let mut schema = crate::app::logic::migration::Schema::new(from_version);
        schema.fields = first_schema;
        schema
    };
    let target_schema = {
        let mut schema = crate::app::logic::migration::Schema::new(target_version);
        schema.fields = second_schema;
        schema
    };
    if first.is_empty() {
        return report("a process walk needs at least one first-transition".to_owned());
    }
    if let Err(reason) = apply_sets(&mut first, first_sets) {
        return report(reason);
    }
    if second.is_empty() {
        second = first.clone();
    }
    if let Err(reason) = apply_sets(&mut second, second_sets) {
        return report(reason);
    }

    let mut old_program = Program::new(name.clone(), version.clone(), first_initial);
    for transition in first {
        match old_program.with(transition) {
            Ok(next) => old_program = next,
            Err(refusal) => return report(refusal.render()),
        }
    }
    let mut new_program = Program::new(name, version.clone(), second_initial);
    for transition in second {
        match new_program.with(transition) {
            Ok(next) => new_program = next,
            Err(refusal) => return report(refusal.render()),
        }
    }

    let timeout = Duration::from_secs(10);
    let mut old = match start_logic_process(
        &session_id,
        "logic-old",
        &old_program,
        None,
        first_generation,
        if schema_declared {
            Some(&source_schema)
        } else {
            None
        },
        timeout,
    ) {
        Ok(started) => started,
        Err(reason) => return report(reason),
    };
    println!(
        "old generation {first_generation}: pid {}, {}",
        old.process.pid(),
        old.session.manifest.name
    );
    let first_states = match run_process_events(&mut old, &first_events, timeout) {
        Ok(states) => states,
        Err(reason) => {
            let stderr = old.process.stderr();
            finish_process(&mut old.process, "the old generation");
            return report(format!("{reason}\n{}", stderr.trim()));
        }
    };

    // T17.06's pause -> drain steps, on the real process. The model (`swap::Swap`) decides
    // whether the walk is in order and whether the pause missed its limit; the process is what
    // actually stops applying business events. `pause-limit` is opt-in so the existing
    // snapshot/restore walk keeps its shape.
    let mut swap = pause_limit.map(|limit| Swap::begin(version.clone(), version.clone(), limit));
    if let Some(active) = swap.as_mut() {
        println!("swap phase: {}", active.phase().name());
        let paused_at = Instant::now();
        match ask(&mut old, "pause", timeout) {
            Ok(reply) if reply == "paused" => {
                let elapsed = u32::try_from(paused_at.elapsed().as_millis()).unwrap_or(u32::MAX);
                if let Err(outcome) = active.pause_ticked(elapsed) {
                    // The deadline passed while the old process was answering. It is still
                    // serving (the pause reply proves the process is alive), so report the
                    // bound instead of taking a snapshot of a half-paused generation.
                    note_old_still_serving(&mut old, first_generation, timeout);
                    finish_process(&mut old.process, "the old generation");
                    return report(outcome.render());
                }
                match active.paused() {
                    Ok(phase) => println!("swap phase: {}", phase.name()),
                    Err(refusal) => return report(refusal.render()),
                }
                match ask(&mut old, "drain", timeout) {
                    Ok(reply) if reply == "drained" => match active.drain() {
                        Ok(phase) => println!("swap phase: {}", phase.name()),
                        Err(refusal) => return report(refusal.render()),
                    },
                    Ok(reply) => {
                        finish_process(&mut old.process, "the old generation");
                        return report(format!("the old generation did not drain: {reply:?}"));
                    }
                    Err(reason) => {
                        finish_process(&mut old.process, "the old generation");
                        return report(reason);
                    }
                }
            }
            Ok(reply) => {
                finish_process(&mut old.process, "the old generation");
                return report(format!("the old generation did not pause: {reply:?}"));
            }
            Err(reason) => {
                finish_process(&mut old.process, "the old generation");
                return report(reason);
            }
        }
    }
    let snapshot = if handoff {
        match ask(&mut old, "snapshot", timeout) {
            Ok(text) if text.starts_with("state=") => {
                let state = text.lines().next().unwrap_or_default();
                println!(
                    "snapshot generation {first_generation}: {} ({} field(s))",
                    unescape_text(state.strip_prefix("state=").unwrap_or(state)),
                    text.lines().count().saturating_sub(1)
                );
                Some(text)
            }
            Ok(text) => {
                finish_process(&mut old.process, "the old generation");
                return report(format!("the snapshot has no state: {text:?}"));
            }
            Err(reason) => {
                finish_process(&mut old.process, "the old generation");
                return report(reason);
            }
        }
    } else {
        None
    };
    let snapshot = match snapshot {
        Some(raw) if !schema_declared => Some(raw),
        Some(raw) => match migrate_state(
            &raw,
            &source_schema,
            &target_schema,
            first_generation,
            second_generation,
        ) {
            Ok(migrated) => Some(migrated),
            Err(reason) => {
                // The switch is abandoned before the new process exists. Ask the old one for its
                // state once more, so "the old generation stays in service" is an observation
                // rather than a promise in the refusal text.
                note_old_still_serving(&mut old, first_generation, timeout);
                finish_process(&mut old.process, "the old generation");
                return report(reason);
            }
        },
        None => None,
    };
    if let Some(active) = swap.as_mut() {
        // The model records the migrated snapshot's schema version; the raw text is what the new
        // process receives. Keeping both means a failed migration (above) never reaches here and
        // an import that succeeds is recorded as the same step the user confirmed.
        match active.snapshot(Snapshot::new(target_version)) {
            Ok(phase) => println!("swap phase: {}", phase.name()),
            Err(refusal) => return report(refusal.render()),
        }
    }

    let mut new = match start_logic_process(
        &session_id,
        "logic-new",
        &new_program,
        second_binary.as_deref(),
        second_generation,
        if schema_declared {
            Some(&target_schema)
        } else {
            None
        },
        timeout,
    ) {
        Ok(started) => started,
        Err(reason) => {
            finish_process(&mut old.process, "the old generation");
            return report(reason);
        }
    };
    println!(
        "new generation {second_generation}: pid {}, {}",
        new.process.pid(),
        new.session.manifest.name
    );
    if let Some(snapshot) = &snapshot {
        match ask(&mut new, &format!("restore\n{snapshot}"), timeout) {
            Ok(restored) if restored == *snapshot => {
                let state = restored.lines().next().unwrap_or_default();
                println!(
                    "restored generation {second_generation}: {} ({} field(s))",
                    unescape_text(state.strip_prefix("state=").unwrap_or(state)),
                    restored.lines().count().saturating_sub(1)
                );
                if let Some(active) = swap.as_mut() {
                    match active.import(true) {
                        Ok(phase) => println!("swap phase: {}", phase.name()),
                        Err(outcome) => return report(outcome.render()),
                    }
                    match active.confirm() {
                        Ok(phase) => println!("swap phase: {}", phase.name()),
                        Err(refusal) => return report(refusal.render()),
                    }
                    match active.resume() {
                        Ok(phase) => println!("swap phase: {}", phase.name()),
                        Err(refusal) => return report(refusal.render()),
                    }
                }
            }
            Ok(restored) if restored.starts_with("error=") => {
                // The new generation refused the import: its own schema is the receiving side's
                // check, not a copy of the sender's preview. The switch has not happened, so the
                // old process is asked for its state before both ends are closed.
                if let Some(active) = swap.as_mut() {
                    // `ImportFailed` is the expected answer here: this call records the new
                    // generation's refusal and aborts the walk. There is no separate cleanup failure
                    // to report, because the phase is the evidence.
                    let _ = active.import(false);
                }
                let reason =
                    unescape_text(restored.strip_prefix("error=").unwrap_or(restored.as_str()));
                note_old_still_serving(&mut old, first_generation, timeout);
                finish_process(&mut old.process, "the old generation");
                finish_process(&mut new.process, "the new generation");
                return report(format!(
                    "the new generation refused the import: {reason}; generation {first_generation} \
                     was not replaced"
                ));
            }
            Ok(restored) => {
                finish_process(&mut old.process, "the old generation");
                finish_process(&mut new.process, "the new generation");
                return report(format!(
                    "the new generation restored {restored:?}, not the snapshot {snapshot:?}"
                ));
            }
            Err(reason) => {
                finish_process(&mut old.process, "the old generation");
                finish_process(&mut new.process, "the new generation");
                return report(reason);
            }
        }
    }
    let second_states = match run_process_events(&mut new, &second_events, timeout) {
        Ok(states) => states,
        Err(reason) => {
            let stderr = new.process.stderr();
            finish_process(&mut old.process, "the old generation");
            finish_process(&mut new.process, "the new generation");
            return report(format!("{reason}\n{}", stderr.trim()));
        }
    };

    // The new process must refuse a frame from the version it replaced. The frame is encoded without
    // a local session check on purpose: the point is what the *worker* does with it.
    let stale = Frame::new(
        Kind::Request,
        session_id.clone(),
        first_generation,
        b"state".to_vec(),
    );
    let stale_line = match stale.encode() {
        Ok(line) => line,
        Err(fault) => return report(fault.render()),
    };
    let refusal = match new.process.request(&stale_line, timeout) {
        Ok(line) => match new.session.receive(&line) {
            Ok(frame) => String::from_utf8_lossy(&frame.payload).into_owned(),
            Err(fault) => return report(fault.render()),
        },
        Err(reason) => return report(reason),
    };
    if !refusal.starts_with("fault=") || !refusal.contains("generation") {
        return report(format!(
            "the new process did not refuse generation {first_generation}: {refusal:?}"
        ));
    }
    println!(
        "generation {first_generation} sent to generation {second_generation}: {}",
        refusal.trim()
    );

    let old_exit = stop_logic_process(&mut old, timeout);
    let new_exit = stop_logic_process(&mut new, timeout);
    for (label, exit) in [("old", old_exit), ("new", new_exit)] {
        match exit {
            Ok(message) => println!("{label} process: {message}"),
            Err(reason) => return report(reason),
        }
    }
    if let Some(active) = swap.as_mut() {
        // Retirement is last on purpose: the old process has actually exited before the model
        // says it is gone, so "old version retired" cannot be printed over a live process.
        if active.retire_old() {
            println!("swap phase: {}", active.phase().name());
        } else {
            return report(format!(
                "the old version cannot be retired while the swap is {}",
                active.phase().name()
            ));
        }
    }

    let actual: Vec<String> = first_states.into_iter().chain(second_states).collect();
    if expected.len() != actual.len() {
        return report(format!(
            "{} state expectation(s) were declared for {} event(s): {}",
            expected.len(),
            actual.len(),
            actual.join(", ")
        ));
    }
    for (index, (expected, actual)) in expected.iter().zip(actual.iter()).enumerate() {
        if expected != actual {
            return report(format!(
                "event {} ended in {actual}, not {expected}",
                index + 1
            ));
        }
    }
    println!(
        "process: {} event(s) over 2 generation(s); the replacement serves and the stale generation is refused",
        actual.len()
    );
    true
}

/// Plans and applies a schema migration to the snapshot text the old generation reported.
///
/// Every field is classified by
/// [`plan`](crate::app::logic::migration::plan); a snapshot field the declared source schema does
/// not know is refused instead of being dropped silently. A loss blocks the switch, and the caller
/// keeps the old generation in service rather than pretending the state survived.
fn migrate_state(
    text: &str,
    source: &crate::app::logic::migration::Schema,
    target: &crate::app::logic::migration::Schema,
    first_generation: u64,
    second_generation: u64,
) -> Result<String, String> {
    use crate::app::logic::core::Value;
    use crate::app::logic::migration::{Snapshot, apply, plan};
    use crate::app::logic::worker::{escape, unescape};

    let mut state_line = None;
    let mut snapshot = Snapshot::new(source.version);
    for line in text.lines() {
        if line.starts_with("state=") {
            state_line = Some(line.to_owned());
        } else if let Some(rest) = line.strip_prefix("field=") {
            let Some((name, typed)) = rest.split_once('=') else {
                return Err(format!(
                    "the snapshot field {line:?} is not field=<name>=<type>:<value>"
                ));
            };
            let name = unescape(name)
                .ok_or_else(|| "a snapshot field name has an unfinished escape".to_owned())?;
            let typed = unescape(typed)
                .ok_or_else(|| "a snapshot field value has an unfinished escape".to_owned())?;
            let value =
                Value::parse(&typed).ok_or_else(|| format!("{typed:?} is not <type>:<value>"))?;
            if snapshot.holds(&name) {
                return Err(format!("the snapshot names field {name:?} more than once"));
            }
            snapshot = snapshot.with(name, value);
        } else if !line.is_empty() {
            return Err(format!(
                "the snapshot line {line:?} is not state= or field="
            ));
        }
    }
    let state_line = state_line.ok_or_else(|| "the snapshot has no state= line".to_owned())?;
    for name in snapshot.values.keys() {
        if source.field(name).is_none() {
            return Err(format!(
                "the snapshot holds field {name:?} that first-schema v{} does not declare; \
                 refusing to drop it silently",
                source.version
            ));
        }
    }
    let migration = plan(source, target, &snapshot).map_err(|fault| fault.render())?;
    for line in migration.render() {
        println!("{line}");
    }
    if migration.blocks_auto_switch() {
        return Err(format!(
            "the automatic switch is blocked: the migration would lose {} (v{} -> v{})",
            migration.lost().join(", "),
            source.version,
            target.version
        ));
    }
    let migrated = apply(&migration, &snapshot);
    println!(
        "migrated generation {first_generation} -> {second_generation}: {} field(s) carried",
        migrated.values.len()
    );
    let mut restored = state_line;
    for (name, value) in &migrated.values {
        restored.push_str(&format!(
            "\nfield={}={}:{}",
            escape(name),
            value.type_name(),
            escape(&value.render())
        ));
    }
    Ok(restored)
}

/// Prints whether the old generation still answers `state` after a refused switch.
/// Stops a generation's process after a refusal and reports a stop that failed.
///
/// `DuplexProcess::finish` returns an outcome rather than a `Result`; a `SignalFailed` outcome is
/// the one case where the process may still be running, and swallowing it would make a cleanup
/// failure look like a clean stop.
fn finish_process(process: &mut crate::host::subprocess::DuplexProcess, label: &str) {
    if let crate::host::subprocess::Termination::SignalFailed { reason } =
        process.finish(std::time::Duration::from_secs(2))
    {
        crate::host::log::emit(
            crate::host::log::Level::Warn,
            "app::run::logic",
            &format!("{label} could not be stopped: {reason}"),
        );
    }
}

fn note_old_still_serving(old: &mut StartedLogic, generation: u64, timeout: std::time::Duration) {
    match ask(old, "state", timeout) {
        Ok(state) => println!(
            "generation {generation} still serving: {}",
            unescape_text(
                state
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .strip_prefix("state=")
                    .unwrap_or_default()
            )
        ),
        Err(problem) => {
            println!("generation {generation} could not answer after the refusal: {problem}")
        }
    }
}

/// One started worker and the session the host opened with it.
pub(super) struct StartedLogic {
    process: crate::host::subprocess::DuplexProcess,
    session: crate::app::logic::ipc::Session,
}

impl StartedLogic {
    /// Sends one request to the worker and answers its payload.
    pub(super) fn request(&mut self, payload: &str) -> Result<String, String> {
        ask(self, payload, std::time::Duration::from_secs(2))
    }

    /// Asks the worker to quit and waits for its real exit.
    pub(super) fn quit(&mut self) -> Result<String, String> {
        stop_logic_process(self, std::time::Duration::from_secs(2))
    }
}

/// Starts the real business process behind the GUI event smoke.
///
/// The program is total for the three declared events (`click`, `change`, `submit`) in every state,
/// so a real control event that the designer's model accepted always has an observable reduction;
/// a state that could not answer one would turn a routing success into a worker error. `change`
/// keeps the state in `editing` because a field can raise it once per keystroke, and a second edit
/// must reduce rather than be refused.
pub(super) fn start_smoke_link() -> Result<StartedLogic, String> {
    use crate::app::logic::core::Transition;
    use crate::app::logic::worker::Program;
    use std::time::Duration;

    let mut program = Program::new("orders", "0.1.0", "idle");
    for transition in [
        Transition::new("idle", "click", "running"),
        Transition::new("idle", "submit", "submitted"),
        Transition::new("idle", "change", "editing"),
        Transition::new("running", "click", "idle"),
        Transition::new("running", "submit", "submitted"),
        Transition::new("running", "change", "editing"),
        Transition::new("editing", "click", "running"),
        Transition::new("editing", "submit", "submitted"),
        Transition::new("editing", "change", "editing"),
        Transition::new("submitted", "click", "running"),
        Transition::new("submitted", "submit", "idle"),
        Transition::new("submitted", "change", "editing"),
    ] {
        program = program
            .with(transition)
            .map_err(|refusal| refusal.render())?;
    }
    start_logic_process(
        "gui-smoke",
        "logic-smoke",
        &program,
        None,
        1,
        None,
        Duration::from_secs(2),
    )
}

/// Starts this executable as a logic worker and performs the handshake.
fn start_logic_process(
    session_id: &str,
    label: &str,
    program: &crate::app::logic::worker::Program,
    binary: Option<&str>,
    generation: u64,
    schema: Option<&crate::app::logic::migration::Schema>,
    timeout: std::time::Duration,
) -> Result<StartedLogic, String> {
    use crate::app::logic::ipc::{Frame, Kind, Manifest, Session};
    use crate::contracts::permission::{EffectClass, Principal};
    use crate::host::permission::{PermissionService, Scope};
    use crate::host::subprocess::{DuplexProcess, ProcessSpec};

    let spec = match binary {
        Some(path) => {
            if !std::path::Path::new(path).is_file() {
                return Err(format!("the candidate binary {path:?} does not exist"));
            }
            ProcessSpec::new(label, path)
        }
        None => {
            let executable = std::env::current_exe()
                .map_err(|error| format!("this executable could not be located: {error}"))?;
            let mut spec = ProcessSpec::new(label, executable).arg("--logic-worker");
            spec = spec
                .arg(format!("name:{}", program.name))
                .arg(format!("version:{}", program.version))
                .arg(format!("initial:{}", program.initial()));
            for transition in program.core.transitions() {
                spec = spec.arg(format!(
                    "transition:{}:{}:{}",
                    transition.state, transition.event, transition.next
                ));
                if transition.guard != Guard::Always {
                    return Err(format!(
                        "the worker command cannot carry a guard on {}/{}",
                        transition.state, transition.event
                    ));
                }
                if transition.need.is_some() {
                    return Err(format!(
                        "the worker command cannot carry a seam need on {}/{}",
                        transition.state, transition.event
                    ));
                }
                for action in &transition.actions {
                    match action {
                        Action::Set { field, value } => {
                            spec = spec.arg(format!(
                                "set:{}:{}:{}={}:{}",
                                transition.state,
                                transition.event,
                                field,
                                value.type_name(),
                                value.render()
                            ))
                        }
                        other => {
                            return Err(format!(
                                "the worker command cannot carry `{}` on {}/{}",
                                other.render(),
                                transition.state,
                                transition.event
                            ));
                        }
                    }
                }
            }
            // The receiving generation gets the same schema the host used for the migration, so
            // the two sides do not disagree about what a legal state is. The candidate binary is
            // excluded above: its schema is compiled into it, and a driver-declared schema that
            // differs is exactly the mismatch an import refusal must catch.
            if let Some(schema) = schema {
                spec = spec.arg(format!("schema-version:{}", schema.version));
                for field in &schema.fields {
                    spec = spec.arg(format!(
                        "schema:{}:{}:{}",
                        field.name,
                        field.kind.name(),
                        field.since
                    ));
                }
            }
            spec
        }
    };
    let permit = PermissionService::new()
        .permit(EffectClass::Command, Scope::Session, Principal::Human)
        .map_err(|refusal| refusal.message())?;
    let mut process = DuplexProcess::spawn(spec, &permit)
        .map_err(|error| format!("{label} could not be started: {error}"))?;

    let handshake = Frame::new(Kind::Handshake, session_id, generation, Vec::new())
        .encode()
        .map_err(|fault| fault.render())?;
    let line = process
        .request(&handshake, timeout)
        .map_err(|error| format!("{label} did not answer the handshake: {error}"))?;
    let frame = Frame::decode(&line).map_err(|fault| fault.render())?;
    if frame.kind != Kind::Manifest {
        return Err(format!(
            "{label} answered the handshake with a {} frame",
            frame.kind.name()
        ));
    }
    let manifest = Manifest::parse(&String::from_utf8_lossy(&frame.payload))
        .map_err(|fault| fault.render())?;
    let session = Session::open(session_id, frame.protocol, manifest, generation)
        .map_err(|fault| fault.render())?;
    Ok(StartedLogic { process, session })
}

/// Sends one payload on the open session and answers the response text.
fn ask(
    started: &mut StartedLogic,
    payload: &str,
    timeout: std::time::Duration,
) -> Result<String, String> {
    use crate::app::logic::ipc::{Frame, Kind};

    let frame = Frame::new(
        Kind::Request,
        started.session.id.clone(),
        started.session.generation,
        payload.as_bytes().to_vec(),
    );
    let line = frame.encode().map_err(|fault| fault.render())?;
    let answer = started.process.request(&line, timeout)?;
    let frame = started
        .session
        .receive(&answer)
        .map_err(|fault| fault.render())?;
    Ok(String::from_utf8_lossy(&frame.payload).into_owned())
}

/// Sends each event and answers the states the worker reported.
fn run_process_events(
    started: &mut StartedLogic,
    events: &[String],
    timeout: std::time::Duration,
) -> Result<Vec<String>, String> {
    use crate::app::logic::core::Value;

    let mut states = Vec::new();
    for event in events {
        let (name, payload) = match event.split_once(':') {
            Some((name, typed)) => (
                name,
                Value::parse(typed).ok_or_else(|| format!("{typed:?} is not <type>:<value>"))?,
            ),
            None => (event.as_str(), Value::Flag(true)),
        };
        let request = match &payload {
            Value::Flag(true) => format!("event {name}"),
            Value::Flag(false) => format!("event {name} flag:no"),
            Value::Number(value) => format!("event {name} number:{value}"),
            Value::Text(value) => format!("event {name} text:{}", escape_text(value)),
        };
        let text = ask(started, &request, timeout)?;
        if let Some(reason) = text.strip_prefix("error=") {
            return Err(format!("{name}: {}", unescape_text(reason)));
        }
        if let Some(reason) = text.strip_prefix("fault=") {
            return Err(format!("{name}: {}", unescape_text(reason)));
        }
        let Some(state) = text
            .lines()
            .next()
            .and_then(|line| line.strip_prefix("state="))
        else {
            return Err(format!("{name}: the answer has no state: {text:?}"));
        };
        let fields: Vec<String> = text
            .lines()
            .filter_map(|line| line.strip_prefix("field="))
            .map(unescape_text)
            .collect();
        let state = unescape_text(state);
        if fields.is_empty() {
            println!("generation {} {name}: {state}", started.session.generation);
        } else {
            println!(
                "generation {} {name}: {state} [{}]",
                started.session.generation,
                fields.join(", ")
            );
        }
        states.push(state);
    }
    Ok(states)
}

/// Asks a worker to quit and waits (bounded) for it to end.
fn stop_logic_process(
    started: &mut StartedLogic,
    timeout: std::time::Duration,
) -> Result<String, String> {
    use crate::app::logic::ipc::{Frame, Kind};

    let frame = Frame::new(
        Kind::Request,
        started.session.id.clone(),
        started.session.generation,
        b"quit".to_vec(),
    );
    let line = frame.encode().map_err(|fault| fault.render())?;
    let answer = started.process.request(&line, timeout)?;
    let frame = started
        .session
        .receive(&answer)
        .map_err(|fault| fault.render())?;
    if String::from_utf8_lossy(&frame.payload).trim() != "bye" {
        return Err(format!(
            "{} did not answer quit with bye: {:?}",
            started.process.label(),
            String::from_utf8_lossy(&frame.payload)
        ));
    }
    let termination = started.process.finish(std::time::Duration::from_secs(5));
    if !termination.ended() {
        return Err(format!(
            "{} did not end: {}",
            started.process.label(),
            termination.message()
        ));
    }
    let success = match &termination {
        crate::host::subprocess::Termination::AlreadyExited { exit }
        | crate::host::subprocess::Termination::Stopped { exit, .. } => exit.is_success(),
        crate::host::subprocess::Termination::Killed { exit, .. } => exit.is_success(),
        crate::host::subprocess::Termination::DirectOnly { exit, .. } => exit.is_success(),
        crate::host::subprocess::Termination::SignalFailed { .. } => false,
    };
    if !success {
        return Err(format!(
            "{} ended without a successful exit: {}",
            started.process.label(),
            termination.message()
        ));
    }
    Ok(termination.message())
}

/// Escapes the characters that would split a request record.
///
/// Delegates to the worker's codec so the host and the process cannot drift into different escape
/// rules (principle #35).
fn escape_text(text: &str) -> String {
    crate::app::logic::worker::escape(text)
}

/// Reads what [`escape_text`] wrote; an unfinished escape is kept as written.
fn unescape_text(text: &str) -> String {
    crate::app::logic::worker::unescape(text).unwrap_or_else(|| text.to_owned())
}
