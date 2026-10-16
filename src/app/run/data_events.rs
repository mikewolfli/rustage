// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! F13: data binding, events, state, navigation and the business seams.
//!
//! One command handler per parsed command of this domain; the dispatcher lives in
//! [`super`](self).

use super::*;

pub(super) fn parse_data_value(
    kind: Option<&crate::app::data_model::FieldKind>,
    text: &str,
) -> Result<crate::contracts::model::PropertyValue, String> {
    use crate::app::data_model::FieldKind;
    use crate::contracts::model::PropertyValue;
    Ok(match kind {
        Some(FieldKind::Integer) => PropertyValue::Integer(
            text.parse()
                .map_err(|_| format!("{text:?} is not a whole number"))?,
        ),
        Some(FieldKind::Number) => PropertyValue::Number(
            text.parse()
                .map_err(|_| format!("{text:?} is not a number"))?,
        ),
        Some(FieldKind::Bool) => match text {
            "true" => PropertyValue::Bool(true),
            "false" => PropertyValue::Bool(false),
            other => return Err(format!("{other:?} is not a flag")),
        },
        Some(FieldKind::Enum(_)) => PropertyValue::Variant(text.to_owned()),
        _ => PropertyValue::Text(text.to_owned()),
    })
}

pub(super) fn render_data_value(value: &crate::contracts::model::PropertyValue) -> String {
    use crate::contracts::model::PropertyValue;
    match value {
        PropertyValue::Bool(flag) => flag.to_string(),
        PropertyValue::Integer(number) => number.to_string(),
        PropertyValue::Number(number) => number.to_string(),
        PropertyValue::Text(text)
        | PropertyValue::Variant(text)
        | PropertyValue::Color(text)
        | PropertyValue::Asset(text) => text.clone(),
    }
}

pub(super) fn data(options: &crate::cli::DataOptions) -> bool {
    use crate::app::data_model::{
        Data, DataField, Dataset, FieldKind, Schema, active, secrets, validate,
    };

    let mut schema = Schema::new("(none)", 0);
    let mut sample = Data::new();
    let mut runtime = Data::new();
    let mut error: Option<String> = None;
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "schema" => match argument.split_once(':') {
                Some((name, version)) => match version.parse::<u32>() {
                    Ok(version) => {
                        schema = Schema::new(name, version);
                        Ok(vec![format!("schema {name} v{version}")])
                    }
                    Err(_) => Err(format!("{version:?} is not a schema version")),
                },
                None => Err(format!("schema needs <name>:<version>, got {argument:?}")),
            },
            "field" => match argument.split_once(':') {
                Some((name, rest)) => {
                    let (kind_text, required) = match rest.split_once(':') {
                        Some((kind, flag)) => match flag {
                            "required" => (kind, true),
                            "optional" => (kind, false),
                            other => {
                                return report(format!("{other:?} is not required or optional"));
                            }
                        },
                        None => (rest, false),
                    };
                    match FieldKind::parse(kind_text) {
                        Ok(kind) => {
                            let line = format!(
                                "field {name}: {}{}",
                                kind.render(),
                                if required { " (required)" } else { "" }
                            );
                            schema.fields.push(DataField {
                                name: name.to_owned(),
                                kind,
                                required,
                            });
                            Ok(vec![line])
                        }
                        Err(reason) => Err(reason),
                    }
                }
                None => Err(format!("field needs <name>:<kind>, got {argument:?}")),
            },
            "sample" | "runtime" => match argument.split_once('=') {
                Some((field, value)) => {
                    let kind = schema.field(field).map(|declared| &declared.kind);
                    match parse_data_value(kind, value) {
                        Ok(parsed) => {
                            let target = if verb == "sample" {
                                &mut sample
                            } else {
                                &mut runtime
                            };
                            target.values.insert(field.to_owned(), parsed.clone());
                            Ok(vec![format!(
                                "{verb} {field} = {}",
                                render_data_value(&parsed)
                            )])
                        }
                        Err(reason) => Err(reason),
                    }
                }
                None => Err(format!("{verb} needs <field>=<value>, got {argument:?}")),
            },
            "error" => {
                error = Some(argument.to_owned());
                Ok(vec![format!("error: {argument}")])
            }
            "check" => {
                let mut lines = Vec::new();
                if !sample.values.is_empty() {
                    match Dataset::sample(schema.version, sample.clone()) {
                        Ok(dataset) => {
                            lines.push(format!("sample: {}", dataset.label()));
                            for diagnostic in validate(&schema, &dataset) {
                                lines.push(format!("  {}", diagnostic.render()));
                            }
                        }
                        Err(found) => {
                            return report(format!(
                                "the sample holds credential field(s): {}",
                                found.join(", ")
                            ));
                        }
                    }
                }
                if !runtime.values.is_empty() {
                    let dataset = Dataset::runtime(schema.version, runtime.clone());
                    lines.push(format!("runtime: {}", dataset.label()));
                    for diagnostic in validate(&schema, &dataset) {
                        lines.push(format!("  {}", diagnostic.render()));
                    }
                }
                if lines.is_empty() {
                    lines.push("nothing to check".to_owned());
                }
                Ok(lines)
            }
            "active" => {
                let sample_dataset = if sample.values.is_empty() {
                    None
                } else {
                    match Dataset::sample(schema.version, sample.clone()) {
                        Ok(dataset) => Some(dataset),
                        Err(found) => {
                            return report(format!(
                                "the sample holds credential field(s): {}",
                                found.join(", ")
                            ));
                        }
                    }
                };
                let runtime_dataset = if let Some(reason) = &error {
                    Some(Dataset::error(schema.version, reason.clone()))
                } else if runtime.values.is_empty() {
                    None
                } else {
                    Some(Dataset::runtime(schema.version, runtime.clone()))
                };
                let selection = active(runtime_dataset.as_ref(), sample_dataset.as_ref());
                let mut lines = vec![format!("active: {}", selection.label())];
                if let Some(found) = selection.data() {
                    for (field, value) in &found.values {
                        lines.push(format!("  {field} = {}", render_data_value(value)));
                    }
                }
                if selection.is_sample() {
                    lines.push("the sample is shown because there is no runtime data".to_owned());
                }
                Ok(lines)
            }
            "state" => {
                let mut lines = schema.render();
                lines.push(format!("sample: {} value(s)", sample.values.len()));
                lines.push(format!("runtime: {} value(s)", runtime.values.len()));
                if let Some(reason) = &error {
                    lines.push(format!("runtime error: {reason}"));
                }
                let found = secrets(&sample);
                if !found.is_empty() {
                    lines.push(format!("sample credential field(s): {}", found.join(", ")));
                }
                Ok(lines)
            }
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a data operation")),
        };
        match step {
            Ok(notes) => {
                for note in notes {
                    println!("{note}");
                }
            }
            Err(reason) => return report(reason),
        }
    }
    true
}

pub(super) fn binding(options: &crate::cli::BindingOptions) -> bool {
    use crate::app::binding::{Binding, Conversion, Direction, Environment, Graph, SlotKind};
    use crate::app::data_model::{DataField, FieldKind, Schema};
    use crate::contracts::identity::NodeId;
    use crate::contracts::model::PropertyName;

    let mut environment = Environment::new(Schema::new("(none)", 0));
    let mut graph = Graph::new();
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "schema" => match argument.split_once(':') {
                Some((name, version)) => match version.parse::<u32>() {
                    Ok(version) => {
                        environment.schema = Schema::new(name, version);
                        Ok(vec![format!("schema {name} v{version}")])
                    }
                    Err(_) => Err(format!("{version:?} is not a schema version")),
                },
                None => Err(format!("schema needs <name>:<version>, got {argument:?}")),
            },
            "field" => match argument.split_once(':') {
                Some((name, kind_text)) => match FieldKind::parse(kind_text) {
                    Ok(kind) => {
                        let line = format!("field {name}: {}", kind.render());
                        environment.schema.fields.push(DataField {
                            name: name.to_owned(),
                            kind,
                            required: false,
                        });
                        Ok(vec![line])
                    }
                    Err(reason) => Err(reason),
                },
                None => Err(format!("field needs <name>:<kind>, got {argument:?}")),
            },
            "slot" => match argument.split_once(':') {
                Some((target, kind_text)) => match target.split_once('.') {
                    Some((node, property)) => {
                        let node = match node.parse::<u64>() {
                            Ok(node) => node,
                            Err(_) => {
                                return report(format!("{node:?} is not a node id"));
                            }
                        };
                        let Some(property) = PropertyName::new(property) else {
                            return report(format!("{property:?} is not a property name"));
                        };
                        match parse_property_type(kind_text) {
                            Ok(kind) => {
                                let line =
                                    format!("slot {node}.{} : {}", property.as_str(), kind.name());
                                environment.slots.push(SlotKind {
                                    node: NodeId::from_raw(node),
                                    property,
                                    kind,
                                });
                                Ok(vec![line])
                            }
                            Err(reason) => Err(reason),
                        }
                    }
                    None => Err(format!("{target:?} is not <node>.<property>")),
                },
                None => Err(format!(
                    "slot needs <node>.<property>:<type>, got {argument:?}"
                )),
            },
            "bind" => match argument.split_once('=') {
                Some((source, rest)) => match rest.split_once(':') {
                    Some((target, rest)) => match rest.split_once(':') {
                        Some((direction, conversion)) => {
                            let direction = match Direction::parse(direction) {
                                Some(direction) => direction,
                                None => {
                                    return report(format!(
                                        "{direction:?} is not one-way or two-way"
                                    ));
                                }
                            };
                            let conversion = match Conversion::parse(conversion) {
                                Some(conversion) => conversion,
                                None => {
                                    return report(format!("{conversion:?} is not a conversion"));
                                }
                            };
                            match (parse_origin(source), parse_origin(target)) {
                                (Ok(source), Ok(target)) => {
                                    let one = Binding {
                                        source,
                                        target,
                                        direction,
                                        conversion,
                                    };
                                    let verdict = graph.admit(&one, &environment);
                                    let label = "binding";
                                    let line = verdict.render(label);
                                    if verdict.is_accepted() {
                                        graph.bindings.push(one);
                                    }
                                    Ok(vec![line])
                                }
                                (Err(reason), _) | (_, Err(reason)) => Err(reason),
                            }
                        }
                        None => Err(format!(
                            "bind needs <source>=<target>:<direction>:<conversion>, got {argument:?}"
                        )),
                    },
                    None => Err(format!(
                        "bind needs <source>=<target>:<direction>:<conversion>, got {argument:?}"
                    )),
                },
                None => Err(format!(
                    "bind needs <source>=<target>:<direction>:<conversion>, got {argument:?}"
                )),
            },
            "check" => {
                let diagnostics = graph.check(&environment);
                if diagnostics.is_empty() {
                    Ok(vec![format!(
                        "{} binding(s), all admitted",
                        graph.bindings.len()
                    )])
                } else {
                    let mut lines: Vec<String> = diagnostics
                        .iter()
                        .map(|diagnostic| diagnostic.render())
                        .collect();
                    let count = diagnostics.len();
                    lines.insert(0, format!("{count} binding(s) refused"));
                    for line in &lines {
                        println!("{line}");
                    }
                    return report(format!("{count} binding(s) could not be admitted"));
                }
            }
            "state" => {
                let mut lines = environment.schema.render();
                lines.push(format!("slots: {}", environment.slots.len()));
                for slot in &environment.slots {
                    lines.push(format!(
                        "  {}.{} : {}",
                        slot.node.get(),
                        slot.property.as_str(),
                        slot.kind.name()
                    ));
                }
                lines.extend(graph.render());
                Ok(lines)
            }
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a binding operation")),
        };
        match step {
            Ok(notes) => {
                for note in notes {
                    println!("{note}");
                }
            }
            Err(reason) => return report(reason),
        }
    }
    true
}

pub(super) fn events(options: &crate::cli::EventsOptions) -> bool {
    use crate::app::data_model::{Data, DataField, FieldKind};
    use crate::app::events::{EventDecl, Registry, Trigger, Wiring, fire, status};
    use crate::contracts::identity::NodeId;

    let mut registry = Registry::new();
    let mut wiring = Wiring::new();
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "event" => {
                if registry.has(argument) {
                    Ok(vec![format!("event {argument}: already declared")])
                } else {
                    registry.events.push(EventDecl {
                        name: argument.to_owned(),
                        payload: Vec::new(),
                    });
                    Ok(vec![format!("event {argument}")])
                }
            }
            "field" => match argument.split_once(':') {
                Some((event, rest)) => match rest.split_once(':') {
                    Some((field, rest)) => {
                        let (kind_text, required) = match rest.split_once(':') {
                            Some((kind, "required")) => (kind, true),
                            Some((kind, "optional")) => (kind, false),
                            Some((_, other)) => {
                                return report(format!("{other:?} is not required or optional"));
                            }
                            None => (rest, false),
                        };
                        let kind = match FieldKind::parse(kind_text) {
                            Ok(kind) => kind,
                            Err(reason) => return report(reason),
                        };
                        match registry.events.iter_mut().find(|decl| decl.name == event) {
                            Some(declaration) => {
                                let line = format!("field {event}.{field}: {}", kind.render());
                                declaration.payload.push(DataField {
                                    name: field.to_owned(),
                                    kind,
                                    required,
                                });
                                Ok(vec![line])
                            }
                            None => Err(format!("no event {event:?} is declared")),
                        }
                    }
                    None => Err(format!(
                        "field needs <event>:<field>:<kind>, got {argument:?}"
                    )),
                },
                None => Err(format!(
                    "field needs <event>:<field>:<kind>, got {argument:?}"
                )),
            },
            "attach" => match argument.split_once(':') {
                Some((node, rest)) => match rest.split_once(':') {
                    Some((event, handler)) => match node.parse::<u64>() {
                        Ok(node) => {
                            match wiring.attach(&registry, NodeId::from_raw(node), event, handler) {
                                Ok(()) => Ok(vec![format!("node {node}: {event} -> {handler}")]),
                                Err(reason) => Err(reason),
                            }
                        }
                        Err(_) => Err(format!("{node:?} is not a node id")),
                    },
                    None => Err(format!(
                        "attach needs <node>:<event>:<handler>, got {argument:?}"
                    )),
                },
                None => Err(format!(
                    "attach needs <node>:<event>:<handler>, got {argument:?}"
                )),
            },
            "status" => match argument.split_once(':') {
                Some((node, event)) => match node.parse::<u64>() {
                    Ok(node) => {
                        let node = NodeId::from_raw(node);
                        Ok(vec![status(&registry, &wiring, node, event).render(event)])
                    }
                    Err(_) => Err(format!("{node:?} is not a node id")),
                },
                None => Err(format!("status needs <node>:<event>, got {argument:?}")),
            },
            "fire" => match argument.split_once(':') {
                Some((node, rest)) => match rest.split_once(':') {
                    Some((event, pairs)) => {
                        let node = match node.parse::<u64>() {
                            Ok(node) => NodeId::from_raw(node),
                            Err(_) => {
                                return report(format!("{node:?} is not a node id"));
                            }
                        };
                        let mut payload = Data::new();
                        for pair in pairs.split(',') {
                            let Some((field, value)) = pair.split_once('=') else {
                                return report(format!(
                                    "a payload entry needs <field>=<value>, got {pair:?}"
                                ));
                            };
                            let kind = registry
                                .get(event)
                                .and_then(|declaration| {
                                    declaration
                                        .payload
                                        .iter()
                                        .find(|declared| declared.name == field)
                                })
                                .map(|declared| &declared.kind);
                            match parse_data_value(kind, value) {
                                Ok(parsed) => {
                                    payload.values.insert(field.to_owned(), parsed);
                                }
                                Err(reason) => return report(reason),
                            }
                        }
                        let trigger = Trigger {
                            event: event.to_owned(),
                            payload,
                        };
                        match fire(&registry, &wiring, node, &trigger) {
                            Ok(firing) => Ok(firing.render(node)),
                            Err(reason) => Err(reason),
                        }
                    }
                    None => Err(format!(
                        "fire needs <node>:<event>:<field>=<value>, got {argument:?}"
                    )),
                },
                None => Err(format!(
                    "fire needs <node>:<event>:<field>=<value>, got {argument:?}"
                )),
            },
            "check" => {
                let mut lines = Vec::new();
                for declaration in &registry.events {
                    let wired = wiring
                        .attachments
                        .iter()
                        .filter(|attachment| attachment.event == declaration.name)
                        .count();
                    lines.push(format!(
                        "{}: declared; wired on {} node(s)",
                        declaration.name, wired
                    ));
                }
                lines.extend(wiring.render());
                Ok(lines)
            }
            "state" => {
                let mut lines = registry.render();
                lines.extend(wiring.render());
                let handlers: Vec<&str> = wiring.handlers();
                lines.push(format!("handlers: {}", handlers.join(", ")));
                Ok(lines)
            }
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not an events operation")),
        };
        match step {
            Ok(notes) => {
                for note in notes {
                    println!("{note}");
                }
            }
            Err(reason) => return report(reason),
        }
    }
    true
}

pub(super) fn parse_capability(text: &str) -> Option<crate::contracts::permission::Capability> {
    use crate::contracts::permission::Capability;
    Some(match text {
        "edit-property" => Capability::EditWidgetProperty,
        "add-node" => Capability::AddOrRemoveNode,
        "edit-generated" => Capability::EditGeneratedCode,
        "edit-handwritten" => Capability::EditHandwrittenCode,
        "write-file" => Capability::WriteFile,
        "run-cargo" => Capability::RunCargoCheck,
        "network" => Capability::RequestNetworkService,
        "dependencies" => Capability::ChangeDependencies,
        _ => return None,
    })
}

pub(super) fn dispatch_actions(options: &crate::cli::DispatchOptions) -> bool {
    use crate::app::binding::Environment;
    use crate::app::data_model::Schema;
    use crate::app::dispatch::{Action, Dispatcher};
    use crate::contracts::identity::NodeId;
    use crate::contracts::model::PropertyName;
    use crate::contracts::permission::{PermissionGate, Principal};

    let mut dispatcher = Dispatcher::new(Environment::new(Schema::new("(none)", 0)));
    let mut gate = PermissionGate::new();
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "slot" => match argument.split_once(':') {
                Some((target, kind_text)) => match target.split_once('.') {
                    Some((node, property)) => {
                        let node = match node.parse::<u64>() {
                            Ok(node) => node,
                            Err(_) => return report(format!("{node:?} is not a node id")),
                        };
                        let Some(property) = PropertyName::new(property) else {
                            return report(format!("{property:?} is not a property name"));
                        };
                        match parse_property_type(kind_text) {
                            Ok(kind) => {
                                let line =
                                    format!("slot {node}.{} : {}", property.as_str(), kind.name());
                                dispatcher
                                    .environment
                                    .slots
                                    .push(crate::app::binding::SlotKind {
                                        node: NodeId::from_raw(node),
                                        property,
                                        kind,
                                    });
                                Ok(vec![line])
                            }
                            Err(reason) => Err(reason),
                        }
                    }
                    None => Err(format!("{target:?} is not <node>.<property>")),
                },
                None => Err(format!(
                    "slot needs <node>.<property>:<type>, got {argument:?}"
                )),
            },
            "command" => {
                dispatcher.commands.insert(argument.to_owned());
                Ok(vec![format!("command: {argument}")])
            }
            "function" => {
                dispatcher.functions.insert(argument.to_owned());
                Ok(vec![format!("function: {argument}")])
            }
            "trust" => {
                gate.trust_project();
                Ok(vec!["the project is trusted".to_owned()])
            }
            "authorize" => match parse_capability(argument) {
                Some(capability) => {
                    gate.authorize(capability);
                    Ok(vec![format!("authorized: {argument}")])
                }
                None => Err(format!("{argument:?} is not a capability")),
            },
            "bind" => match argument.split_once(':') {
                Some((node, rest)) => match rest.split_once(':') {
                    Some((event, rest)) => {
                        let node = match node.parse::<u64>() {
                            Ok(node) => NodeId::from_raw(node),
                            Err(_) => return report(format!("{node:?} is not a node id")),
                        };
                        let action = match rest.split_once(':') {
                            Some(("set", body)) => match body.split_once('=') {
                                Some((property, value)) => {
                                    let Some(property) = PropertyName::new(property) else {
                                        return report(format!(
                                            "{property:?} is not a property name"
                                        ));
                                    };
                                    let kind =
                                        dispatcher.environment.slot_kind(node, &property).cloned();
                                    let Some(kind) = kind else {
                                        return report(format!(
                                            "node {} declares no property {:?}",
                                            node.get(),
                                            property.as_str()
                                        ));
                                    };
                                    match kind.parse(value) {
                                        Ok(value) => Action::Set {
                                            node,
                                            property,
                                            value,
                                        },
                                        Err(reason) => return report(reason),
                                    }
                                }
                                None => {
                                    return report(format!(
                                        "a set action needs <property>=<value>, got {body:?}"
                                    ));
                                }
                            },
                            Some(("command", id)) => Action::Command { id: id.to_owned() },
                            Some(("call", body)) => match body.split_once(':') {
                                Some((function, capability)) => {
                                    match parse_capability(capability) {
                                        Some(needs) => Action::Call {
                                            function: function.to_owned(),
                                            needs,
                                        },
                                        None => {
                                            return report(format!(
                                                "{capability:?} is not a capability"
                                            ));
                                        }
                                    }
                                }
                                None => {
                                    return report(format!(
                                        "a call action needs <function>:<capability>, got {body:?}"
                                    ));
                                }
                            },
                            _ => {
                                return report(format!("{rest:?} is not set/command/call"));
                            }
                        };
                        match dispatcher.install(&gate, Principal::Human, node, event, action) {
                            Ok(install) => Ok(vec![format!(
                                "node {}: {event}: {}",
                                node.get(),
                                install.render()
                            )]),
                            Err(reason) => Err(reason),
                        }
                    }
                    None => Err(format!(
                        "bind needs <node>:<event>:<action>, got {argument:?}"
                    )),
                },
                None => Err(format!(
                    "bind needs <node>:<event>:<action>, got {argument:?}"
                )),
            },
            "unbind" => match argument.split_once(':') {
                Some((node, event)) => match node.parse::<u64>() {
                    Ok(node) => match dispatcher.unbind(NodeId::from_raw(node), event) {
                        Some(released) => Ok(vec![format!("released {}", released.render())]),
                        None => Ok(vec![format!("nothing was bound to {event}")]),
                    },
                    Err(_) => Err(format!("{node:?} is not a node id")),
                },
                None => Err(format!("unbind needs <node>:<event>, got {argument:?}")),
            },
            "dispatch" => match argument.split_once(':') {
                Some((node, event)) => match node.parse::<u64>() {
                    Ok(node) => match dispatcher.dispatch(NodeId::from_raw(node), event) {
                        Ok(dispatch) => Ok(vec![dispatch.render()]),
                        Err(reason) => Err(reason),
                    },
                    Err(_) => Err(format!("{node:?} is not a node id")),
                },
                None => Err(format!("dispatch needs <node>:<event>, got {argument:?}")),
            },
            "check" => Ok(dispatcher.render()),
            "state" => {
                let mut lines = dispatcher.render();
                for released in dispatcher.released() {
                    lines.push(format!("released {}", released.render()));
                }
                Ok(lines)
            }
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a dispatch operation")),
        };
        match step {
            Ok(notes) => {
                for note in notes {
                    println!("{note}");
                }
            }
            Err(reason) => return report(reason),
        }
    }
    true
}

pub(super) fn routes(options: &crate::cli::RoutesOptions) -> bool {
    use crate::app::routes::{Navigator, Route, Window};

    let mut navigator = Navigator::new();
    let mut window: Option<Window> = None;
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "page" => {
                navigator = navigator.with_page(argument);
                Ok(vec![format!("page: {argument}")])
            }
            "entry" => {
                navigator = navigator.with_entry(argument);
                Ok(vec![format!("entry: {argument}")])
            }
            "edge" => match argument.split_once(':') {
                Some((from, to)) => {
                    navigator = navigator.with_edge(from, to);
                    Ok(vec![format!("edge: {from} -> {to}")])
                }
                None => Err(format!("edge needs <from>:<to>, got {argument:?}")),
            },
            "open" => {
                window = Some(Window::open(argument));
                Ok(vec![format!("open: {argument}")])
            }
            "ready" => match &mut window {
                Some(window) => {
                    window.ready();
                    Ok(vec![format!("lifecycle: {}", window.lifecycle.name())])
                }
                None => Err("no window is open".to_owned()),
            },
            "go" => match &mut window {
                Some(window) => match window.navigate(&navigator, Route::new(argument)) {
                    Ok(()) => Ok(vec![format!(
                        "current: {}",
                        window.current().map(Route::render).unwrap_or_default()
                    )]),
                    Err(reason) => Err(reason),
                },
                None => Err("no window is open".to_owned()),
            },
            "back" | "forward" => match &mut window {
                Some(window) => {
                    let moved = if verb == "back" {
                        window.back()
                    } else {
                        window.forward()
                    };
                    match moved {
                        Some(route) => Ok(vec![format!("current: {}", route.render())]),
                        None => Ok(vec![format!("nothing to go {verb} to")]),
                    }
                }
                None => Err("no window is open".to_owned()),
            },
            "close" => match &mut window {
                Some(window) => match window.close() {
                    Ok(()) => Ok(vec![format!("lifecycle: {}", window.lifecycle.name())]),
                    Err(reason) => Err(reason),
                },
                None => Err("no window is open".to_owned()),
            },
            "finish-close" => match &mut window {
                Some(window) => match window.finish_close() {
                    Ok(()) => Ok(vec![format!("lifecycle: {}", window.lifecycle.name())]),
                    Err(reason) => Err(reason),
                },
                None => Err("no window is open".to_owned()),
            },
            "check" => Ok(navigator.render()),
            "state" => {
                let mut lines = navigator.render();
                if let Some(window) = &window {
                    lines.extend(window.render());
                }
                Ok(lines)
            }
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a routes operation")),
        };
        match step {
            Ok(notes) => {
                for note in notes {
                    println!("{note}");
                }
            }
            Err(reason) => return report(reason),
        }
    }
    true
}

pub(super) fn machine(options: &crate::cli::MachineOptions) -> bool {
    use std::collections::BTreeMap;

    use crate::app::state_machine::{Answer, Interaction, Machine, Mode, Session, send};

    let mut machine = Machine::new();
    let mut session: Option<Session> = None;
    let mut answers: BTreeMap<String, Answer> = BTreeMap::new();
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "state" => {
                machine = machine.with_state(argument);
                Ok(vec![format!("state: {argument}")])
            }
            "initial" => {
                machine = machine.starting_at(argument);
                Ok(vec![format!("initial: {argument}")])
            }
            "final" => {
                machine = machine.final_state(argument);
                Ok(vec![format!("final: {argument}")])
            }
            "transition" => match argument.split_once(':') {
                Some((from, rest)) => match rest.split_once(':') {
                    Some((on, rest)) => add_transition(rest, from, on, &mut machine),
                    None => Err(format!(
                        "transition needs <from>:<on>:<to>[:<guard>], got {argument:?}"
                    )),
                },
                None => Err(format!(
                    "transition needs <from>:<on>:<to>[:<guard>], got {argument:?}"
                )),
            },
            "answer" => match argument.split_once(':') {
                Some((guard, rest)) => match rest.split_once(':') {
                    Some((value, interaction)) => {
                        let value = match value {
                            "true" => true,
                            "false" => false,
                            other => {
                                return report(format!("{other:?} is not true or false"));
                            }
                        };
                        let interaction = match interaction {
                            "sync" => Interaction::Sync,
                            "async" => Interaction::Async,
                            other => {
                                return report(format!("{other:?} is not sync or async"));
                            }
                        };
                        answers.insert(guard.to_owned(), Answer { value, interaction });
                        Ok(vec![format!("answer {guard}: {value} ({interaction:?})")])
                    }
                    None => Err(format!(
                        "answer needs <guard>:<true|false>:<sync|async>, got {argument:?}"
                    )),
                },
                None => Err(format!(
                    "answer needs <guard>:<true|false>:<sync|async>, got {argument:?}"
                )),
            },
            "start" => match Session::start(&machine) {
                Ok(started) => {
                    let line = format!("started in {}", started.current);
                    session = Some(started);
                    Ok(vec![line])
                }
                Err(reason) => Err(reason),
            },
            "send" => match &mut session {
                Some(session) => {
                    let (event, mode) = match argument.split_once(':') {
                        Some((event, "sync")) => (event, Mode::Sync),
                        Some((event, "async")) => (event, Mode::Async),
                        Some((_event, other)) => {
                            return report(format!("{other:?} is not sync or async"));
                        }
                        None => (argument, Mode::Sync),
                    };
                    match send(&machine, session, event, &answers, mode) {
                        Ok(step) => Ok(vec![step.render()]),
                        Err(reason) => Err(reason),
                    }
                }
                None => Err("no session has started".to_owned()),
            },
            "inspect" => {
                let mut lines = machine.render();
                match &session {
                    Some(session) => lines.push(format!("current: {}", session.current)),
                    None => lines.push("current: (no session)".to_owned()),
                }
                Ok(lines)
            }
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a machine operation")),
        };
        match step {
            Ok(notes) => {
                for note in notes {
                    println!("{note}");
                }
            }
            Err(reason) => return report(reason),
        }
    }
    true
}

pub(super) fn add_transition(
    rest: &str,
    from: &str,
    on: &str,
    machine: &mut crate::app::state_machine::Machine,
) -> Result<Vec<String>, String> {
    let (to, guard) = match rest.split_once(':') {
        Some((to, guard)) => (to, Some(guard.to_owned())),
        None => (rest, None),
    };
    let line = format!(
        "transition {from} --{on}--> {to}{}",
        match &guard {
            Some(guard) => format!(" [{guard}]"),
            None => String::new(),
        }
    );
    machine
        .transitions
        .push(crate::app::state_machine::Transition {
            from: from.to_owned(),
            on: on.to_owned(),
            to: to.to_owned(),
            guard,
        });
    Ok(vec![line])
}

pub(super) fn parse_outcome(text: &str) -> Option<crate::app::adapters::Outcome> {
    use crate::app::adapters::Outcome;
    let (kind, rest) = text.split_once(':').unwrap_or((text, ""));
    Some(match kind {
        "ok" => Outcome::Ok(rest.to_owned()),
        "failed" => Outcome::Failed(rest.to_owned()),
        "unknown" => Outcome::Unknown(rest.to_owned()),
        "cancelled" => Outcome::Cancelled,
        _ => return None,
    })
}

pub(super) fn adapter(options: &crate::cli::AdapterOptions) -> bool {
    use crate::app::adapters::{
        Availability, Cancel, Kind, Outcome, Policy, Request, attempt, run,
    };
    use crate::contracts::permission::{PermissionGate, Principal};

    let mut sample: Vec<Outcome> = Vec::new();
    let mut live: Vec<(String, Outcome)> = Vec::new();
    let mut request: Option<Request> = None;
    let mut cancel = Cancel::new();
    let mut availability = Availability::Online;
    let gate = PermissionGate::new();
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "sample" => match parse_outcome(argument) {
                Some(outcome) => {
                    let line = outcome.render();
                    sample.push(outcome);
                    Ok(vec![format!("sample: {line}")])
                }
                None => Err(format!("{argument:?} is not an outcome")),
            },
            "live" => match argument.split_once(':') {
                Some((target, rest)) => match parse_outcome(rest) {
                    Some(outcome) => {
                        let line = outcome.render();
                        live.push((target.to_owned(), outcome));
                        Ok(vec![format!("live {target}: {line}")])
                    }
                    None => Err(format!("{rest:?} is not an outcome")),
                },
                None => Err(format!("live needs <target>:<outcome>, got {argument:?}")),
            },
            "request" => match argument.split_once(':') {
                Some((kind, target)) => match Kind::parse(kind) {
                    Some(kind) => {
                        request = Some(Request::new(kind, target));
                        Ok(vec![format!("request: {} {target}", kind.name())])
                    }
                    None => Err(format!("{kind:?} is not a request kind")),
                },
                None => Err(format!("request needs <kind>:<target>, got {argument:?}")),
            },
            "cancel" => {
                cancel = Cancel::set();
                Ok(vec!["cancelled".to_owned()])
            }
            "online" => {
                availability = Availability::Online;
                Ok(vec!["online".to_owned()])
            }
            "offline" => {
                availability = Availability::Offline;
                Ok(vec!["offline".to_owned()])
            }
            "run" => match &request {
                Some(request) => {
                    let mut adapter = if live.is_empty() {
                        crate::app::adapters::Adapter::sample(&sample)
                    } else {
                        crate::app::adapters::Adapter::live_owned(&live)
                    };
                    match run(
                        &mut adapter,
                        request,
                        &cancel,
                        &gate,
                        Principal::Human,
                        availability,
                    ) {
                        Ok(outcome) => Ok(vec![outcome.render()]),
                        Err(reason) => Err(reason),
                    }
                }
                None => Err("no request was set".to_owned()),
            },
            "attempt" => match &request {
                Some(request) => {
                    let attempts = match argument.parse::<u32>() {
                        Ok(attempts) => attempts,
                        Err(_) => return report(format!("{argument:?} is not a number")),
                    };
                    let mut adapter = if live.is_empty() {
                        crate::app::adapters::Adapter::sample(&sample)
                    } else {
                        crate::app::adapters::Adapter::live_owned(&live)
                    };
                    let outcomes = attempt(&mut adapter, request, &cancel, Policy::new(attempts));
                    let mut lines: Vec<String> = outcomes
                        .iter()
                        .enumerate()
                        .map(|(index, outcome)| format!("#{} {}", index + 1, outcome.render()))
                        .collect();
                    lines.push(format!("{} attempt(s)", outcomes.len()));
                    Ok(lines)
                }
                None => Err("no request was set".to_owned()),
            },
            "check" | "state" => {
                let mut lines = vec![format!(
                    "adapter: {}",
                    if live.is_empty() { "sample" } else { "live" }
                )];
                lines.push(format!(
                    "request: {}",
                    request
                        .as_ref()
                        .map(Request::render)
                        .unwrap_or_else(|| "(none)".to_owned())
                ));
                lines.push(format!(
                    "availability: {}",
                    if availability == Availability::Online {
                        "online"
                    } else {
                        "offline"
                    }
                ));
                lines.push(format!(
                    "cancelled: {}",
                    if cancel.cancelled { "yes" } else { "no" }
                ));
                lines.push(format!(
                    "answers: {} sample, {} live",
                    sample.len(),
                    live.len()
                ));
                Ok(lines)
            }
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not an adapter operation")),
        };
        match step {
            Ok(notes) => {
                for note in notes {
                    println!("{note}");
                }
            }
            Err(reason) => return report(reason),
        }
    }
    true
}

pub(super) fn session_log(options: &crate::cli::SessionLogOptions) -> bool {
    use std::collections::BTreeMap;

    use crate::app::session_log::{Log, Loop, Origin, Transform, run_vector};
    use crate::app::state_machine::{Answer, Machine, Mode, Session};

    let mut log = Log::new();
    let mut machine = Machine::new();
    let mut vector: Vec<(String, String)> = Vec::new();
    let answers: BTreeMap<String, Answer> = BTreeMap::new();
    let mut loop_result: Option<String> = None;
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "human" | "business" => match argument.split_once(':') {
                Some((label, rest)) => match rest.split_once(':') {
                    Some((before, after)) => {
                        let origin = if verb == "human" {
                            Origin::Human
                        } else {
                            Origin::Business
                        };
                        let line = format!("[{}] {label}: {before} -> {after}", origin.name());
                        log.record(label, origin, before, after);
                        Ok(vec![line])
                    }
                    None => Err(format!(
                        "{verb} needs <label>:<before>:<after>, got {argument:?}"
                    )),
                },
                None => Err(format!(
                    "{verb} needs <label>:<before>:<after>, got {argument:?}"
                )),
            },
            "undo" => match log.undo() {
                Ok(undone) => Ok(vec![undone.render()]),
                Err(reason) => Err(reason),
            },
            "loop" => match argument.split_once(':') {
                Some((transform, value)) => {
                    let transform = match transform {
                        "identity" => Transform::Identity,
                        "drifting" => Transform::Drifting,
                        other => {
                            return report(format!("{other:?} is not identity or drifting"));
                        }
                    };
                    let mut loop_ = Loop::new(value, transform);
                    let settled = loop_.settle(8);
                    let line = settled.render();
                    loop_result = Some(line.clone());
                    Ok(vec![line])
                }
                None => Err(format!(
                    "loop needs <identity|drifting>:<value>, got {argument:?}"
                )),
            },
            "state" => {
                machine = machine.with_state(argument);
                Ok(vec![format!("state: {argument}")])
            }
            "initial" => {
                machine = machine.starting_at(argument);
                Ok(vec![format!("initial: {argument}")])
            }
            "final" => {
                machine = machine.final_state(argument);
                Ok(vec![format!("final: {argument}")])
            }
            "transition" => match argument.split_once(':') {
                Some((from, rest)) => match rest.split_once(':') {
                    Some((on, to)) => {
                        machine = machine.with_transition(from, on, to, None);
                        Ok(vec![format!("transition {from} --{on}--> {to}")])
                    }
                    None => Err(format!(
                        "transition needs <from>:<on>:<to>, got {argument:?}"
                    )),
                },
                None => Err(format!(
                    "transition needs <from>:<on>:<to>, got {argument:?}"
                )),
            },
            "expect" => match argument.split_once(':') {
                Some((event, state)) => {
                    vector.push((event.to_owned(), state.to_owned()));
                    Ok(vec![format!("expect {event} -> {state}")])
                }
                None => Err(format!("expect needs <event>:<state>, got {argument:?}")),
            },
            "vector" => match Session::start(&machine) {
                Ok(mut session) => {
                    match run_vector(&machine, &mut session, &vector, &answers, Mode::Sync) {
                        Ok(taken) => {
                            let mut lines: Vec<String> = taken
                                .iter()
                                .map(|step| format!("{} -> {}", step.event, step.actual))
                                .collect();
                            lines.push(format!("reached: {}", session.current));
                            Ok(lines)
                        }
                        Err(reason) => Err(reason),
                    }
                }
                Err(reason) => Err(reason),
            },
            "check" => {
                let mut lines = log.render();
                if let Some(result) = &loop_result {
                    lines.push(format!("loop: {result}"));
                }
                Ok(lines)
            }
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a session-log operation")),
        };
        match step {
            Ok(notes) => {
                for note in notes {
                    println!("{note}");
                }
            }
            Err(reason) => return report(reason),
        }
    }
    true
}
