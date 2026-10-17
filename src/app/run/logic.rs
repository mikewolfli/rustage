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
            "admit" => admit = true,
            "check" | "show" => {}
            other => return report(format!("{other:?} is not a logic-candidate operation")),
        }
    }

    if !admit {
        println!(
            "candidate: revision {current}, {} source(s) debounced",
            debounce.pending()
        );
        return true;
    }

    let Some(action) = action else {
        return report("a build needs `action:<check|build|rebuild>`".to_owned());
    };
    let Some(outcome) = outcome else {
        return report("a build needs `outcome:<success|failure>`".to_owned());
    };
    let built_from = stamp.unwrap_or(current);
    let stamp = VersionStamp::current(built_from);
    let build = Build {
        action,
        outcome,
        artifact: artifact.map(|path| Artifact { path, stamp }),
        stamp,
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
