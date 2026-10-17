// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! F12: definitions and instances, authoring, variants, custom widgets, resources, packages,
//! upgrade and the asset manifest.
//!
//! One command handler per parsed command of this domain; the dispatcher lives in [`super`](self).
//! Where a command can carry out the real work, it does; where it cannot, it reports the refusal
//! the domain returned rather than a success shape.

use super::*;

/// Defines components, places instances and checks for cycles (task T12.01).
pub(super) fn component(options: &crate::cli::ComponentOptions) -> bool {
    use crate::app::component::definition::{
        Definition, Instance, Parameter, Part, Registry, Slot,
    };
    use crate::contracts::identity::{IdAllocator, NodeId};
    use crate::contracts::model::{NodeKind, PropertyValue};
    use std::collections::BTreeMap;

    let mut registry = Registry::new();
    let mut allocator = IdAllocator::new();
    let mut defining = false;
    let mut name = String::new();
    let mut version = 1u32;
    let mut parameters: Vec<Parameter> = Vec::new();
    let mut slots: Vec<Slot> = Vec::new();
    let mut body: Vec<Part> = Vec::new();

    let mut placing = false;
    let mut instance_id = NodeId::from_raw(0);
    let mut instance_definition = String::new();
    let mut arguments: BTreeMap<String, PropertyValue> = BTreeMap::new();

    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        if !defining
            && matches!(
                verb,
                "version" | "param" | "slot" | "control" | "nested" | "useslot" | "commit"
            )
        {
            return report(format!(
                "{verb:?} needs a definition; start with define:<name>"
            ));
        }
        if !placing && matches!(verb, "of" | "arg" | "place") {
            return report(format!(
                "{verb:?} needs an instance; start with instance:<id>"
            ));
        }
        let step: Result<(), String> = match verb {
            "define" => {
                name = argument.to_owned();
                version = 1;
                parameters = Vec::new();
                slots = Vec::new();
                body = Vec::new();
                defining = true;
                Ok(())
            }
            "version" => match argument.parse::<u32>() {
                Ok(value) => {
                    version = value;
                    Ok(())
                }
                Err(_) => Err(format!("{argument:?} is not a version")),
            },
            "param" => {
                let (parameter, default) = match argument.split_once('=') {
                    Some((key, value)) => (key, Some(PropertyValue::Text(value.to_owned()))),
                    None => (argument, None),
                };
                match Parameter::new(parameter, default) {
                    Some(parameter) => {
                        parameters.push(parameter);
                        Ok(())
                    }
                    None => Err(format!("{parameter:?} is not a parameter name")),
                }
            }
            "slot" => match Slot::new(argument) {
                Some(slot) => {
                    slots.push(slot);
                    Ok(())
                }
                None => Err(format!("{argument:?} is not a slot name")),
            },
            "control" => match NodeKind::new(argument) {
                Some(kind) => {
                    body.push(Part::Control {
                        kind,
                        properties: BTreeMap::new(),
                    });
                    Ok(())
                }
                None => Err(format!("{argument:?} is not a control kind")),
            },
            "nested" => {
                body.push(Part::Nested {
                    definition: argument.to_owned(),
                });
                Ok(())
            }
            "useslot" => {
                body.push(Part::Slot {
                    name: argument.to_owned(),
                });
                Ok(())
            }
            "commit" => {
                match Definition::new(
                    &name,
                    version,
                    parameters.clone(),
                    slots.clone(),
                    body.clone(),
                ) {
                    Some(definition) => match registry.insert(definition) {
                        Ok(()) => {
                            defining = false;
                            Ok(())
                        }
                        Err(reason) => Err(reason),
                    },
                    None => Err(format!(
                        "definition {name:?} has a blank name or a duplicate member"
                    )),
                }
            }
            "instance" => match argument.parse::<u64>() {
                Ok(value) => {
                    instance_id = NodeId::from_raw(value);
                    instance_definition = String::new();
                    arguments = BTreeMap::new();
                    placing = true;
                    Ok(())
                }
                Err(_) => Err(format!("{argument:?} is not an instance id")),
            },
            "of" => {
                instance_definition = argument.to_owned();
                Ok(())
            }
            "arg" => match argument.split_once('=') {
                Some((key, value)) => {
                    arguments.insert(key.to_owned(), PropertyValue::Text(value.to_owned()));
                    Ok(())
                }
                None => Err(format!("arg needs <name>=<value>, got {argument:?}")),
            },
            "place" => {
                let mut instance = Instance::new(&instance_definition, instance_id);
                instance.arguments = arguments.clone();
                match registry.place(instance) {
                    Ok(()) => {
                        placing = false;
                        Ok(())
                    }
                    Err(reason) => Err(reason),
                }
            }
            "expand" => match argument.parse::<u64>() {
                Ok(value) => {
                    let id = NodeId::from_raw(value);
                    match registry.expand(id, &mut allocator) {
                        Ok(expansion) => {
                            for node in &expansion.nodes {
                                println!(
                                    "node {} {} owner {} ({} properties)",
                                    node.id.get(),
                                    node.kind.as_str(),
                                    node.owner.get(),
                                    node.properties.len()
                                );
                            }
                            println!(
                                "expanded {} node(s) from instance {}",
                                expansion.nodes.len(),
                                id.get()
                            );
                            Ok(())
                        }
                        Err(reason) => Err(reason),
                    }
                }
                Err(_) => Err(format!("{argument:?} is not an instance id")),
            },
            "show" => {
                println!("definitions: [{}]", registry.names().join(", "));
                Ok(())
            }
            other => Err(format!("{other:?} is not a component operation")),
        };
        if let Err(reason) = step {
            return report(reason);
        }
    }
    println!("registry: {} definition(s)", registry.names().len());
    true
}

/// Extracts, inserts and edits a definition (task T12.02).
pub(super) fn component_author(options: &crate::cli::ComponentAuthorOptions) -> bool {
    use crate::app::component::authoring::Library;
    use crate::contracts::identity::NodeId;
    use crate::contracts::model::{NodeKind, PropertyName, PropertyValue};
    use std::collections::BTreeMap;

    let mut library = Library::new();
    let mut pending_kind: Option<NodeKind> = None;
    let mut pending_properties: BTreeMap<PropertyName, PropertyValue> = BTreeMap::new();
    let mut current_definition = String::new();
    let mut edit_name: Option<String> = None;
    let mut changes: BTreeMap<PropertyName, PropertyValue> = BTreeMap::new();

    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<(), String> = match verb {
            "kind" => match NodeKind::new(argument) {
                Some(kind) => {
                    pending_kind = Some(kind);
                    Ok(())
                }
                None => Err(format!("{argument:?} is not a control kind")),
            },
            "property" => match argument.split_once('=') {
                Some((property, value)) => match PropertyName::new(property) {
                    Some(property) => {
                        pending_properties.insert(property, PropertyValue::Text(value.to_owned()));
                        Ok(())
                    }
                    None => Err(format!("{property:?} is not a property name")),
                },
                None => Err(format!("property needs <name>=<value>, got {argument:?}")),
            },
            "extract" => match pending_kind.clone() {
                Some(kind) => match library.extract(argument, kind, pending_properties.clone()) {
                    Ok(()) => {
                        pending_properties.clear();
                        println!("extracted {argument}");
                        Ok(())
                    }
                    Err(reason) => Err(reason),
                },
                None => Err("extract needs a kind; use kind:<kind> first".to_owned()),
            },
            "use" => {
                current_definition = argument.to_owned();
                Ok(())
            }
            "insert" => match argument.parse::<u64>() {
                Ok(id) => match library.insert(&current_definition, NodeId::from_raw(id)) {
                    Ok(_) => Ok(()),
                    Err(reason) => Err(reason),
                },
                Err(_) => Err(format!("{argument:?} is not an instance id")),
            },
            "override" => match argument.split_once(':') {
                Some((id, assignment)) => match (id.parse::<u64>(), assignment.split_once('=')) {
                    (Ok(id), Some((property, value))) => match PropertyName::new(property) {
                        Some(property) => library.override_property(
                            NodeId::from_raw(id),
                            property,
                            PropertyValue::Text(value.to_owned()),
                        ),
                        None => Err(format!("{property:?} is not a property name")),
                    },
                    _ => Err(format!(
                        "override needs <id>:<name>=<value>, got {argument:?}"
                    )),
                },
                None => Err(format!(
                    "override needs <id>:<name>=<value>, got {argument:?}"
                )),
            },
            "edit" => {
                edit_name = Some(argument.to_owned());
                changes.clear();
                Ok(())
            }
            "change" => match argument.split_once('=') {
                Some((property, value)) => match PropertyName::new(property) {
                    Some(property) => {
                        changes.insert(property, PropertyValue::Text(value.to_owned()));
                        Ok(())
                    }
                    None => Err(format!("{property:?} is not a property name")),
                },
                None => Err(format!("change needs <name>=<value>, got {argument:?}")),
            },
            "commit" => match edit_name.clone() {
                Some(name) => match library.edit(&name, changes.clone()) {
                    Ok(edit) => {
                        let affected: Vec<String> = edit
                            .affected
                            .iter()
                            .map(|id| id.get().to_string())
                            .collect();
                        println!("affected: [{}]", affected.join(", "));
                        for preserved in &edit.preserved {
                            println!(
                                "preserved: instance {} keeps {} = {}",
                                preserved.instance.get(),
                                preserved.name.as_str(),
                                preserved.value.family()
                            );
                        }
                        changes.clear();
                        edit_name = None;
                        Ok(())
                    }
                    Err(reason) => Err(reason),
                },
                None => Err("commit needs an edit; start with edit:<name>".to_owned()),
            },
            "resolve" => match argument.parse::<u64>() {
                Ok(id) => match library.resolve(NodeId::from_raw(id)) {
                    Ok(properties) => {
                        println!("resolved {} propert(ies)", properties.len());
                        Ok(())
                    }
                    Err(reason) => Err(reason),
                },
                Err(_) => Err(format!("{argument:?} is not an instance id")),
            },
            "show" => {
                println!("definitions: [{}]", library.names().join(", "));
                Ok(())
            }
            other => Err(format!("{other:?} is not a component-author operation")),
        };
        if let Err(reason) = step {
            return report(reason);
        }
    }
    true
}

/// Detaches, resets and checks a variant (task T12.03).
pub(super) fn component_override(options: &crate::cli::ComponentOverrideOptions) -> bool {
    use crate::app::component::variant::{Variant, detach, reset, target_compatibility};
    use crate::contracts::identity::NodeId;
    use crate::contracts::model::{NodeKind, PropertyName, PropertyValue};
    use std::collections::BTreeMap;

    let mut instance = NodeId::from_raw(0);
    let mut definition = String::new();
    let mut kind: Option<NodeKind> = None;
    let mut targets: Vec<String> = Vec::new();
    let mut overrides: BTreeMap<PropertyName, PropertyValue> = BTreeMap::new();
    let mut base: BTreeMap<PropertyName, PropertyValue> = BTreeMap::new();

    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<(), String> = match verb {
            "variant" => match argument.parse::<u64>() {
                Ok(value) => {
                    instance = NodeId::from_raw(value);
                    definition = String::new();
                    kind = None;
                    targets = Vec::new();
                    overrides = BTreeMap::new();
                    Ok(())
                }
                Err(_) => Err(format!("{argument:?} is not an instance id")),
            },
            "of" => {
                definition = argument.to_owned();
                Ok(())
            }
            "kind" => match NodeKind::new(argument) {
                Some(value) => {
                    kind = Some(value);
                    Ok(())
                }
                None => Err(format!("{argument:?} is not a control kind")),
            },
            "target" => {
                targets.push(argument.to_owned());
                Ok(())
            }
            "override" => match argument.split_once('=') {
                Some((property, value)) => match PropertyName::new(property) {
                    Some(property) => {
                        overrides.insert(property, PropertyValue::Text(value.to_owned()));
                        Ok(())
                    }
                    None => Err(format!("{property:?} is not a property name")),
                },
                None => Err(format!("override needs <name>=<value>, got {argument:?}")),
            },
            "base" => match argument.split_once('=') {
                Some((property, value)) => match PropertyName::new(property) {
                    Some(property) => {
                        base.insert(property, PropertyValue::Text(value.to_owned()));
                        Ok(())
                    }
                    None => Err(format!("{property:?} is not a property name")),
                },
                None => Err(format!("base needs <name>=<value>, got {argument:?}")),
            },
            "detach" => match kind.clone() {
                Some(kind) => {
                    let mut variant = Variant::new(instance, &definition, kind);
                    variant.targets = targets.clone();
                    variant.overrides = overrides.clone();
                    let detached = detach(&variant, &base);
                    println!(
                        "detached {} with {} propert(ies)",
                        detached.copy.kind.as_str(),
                        detached.copy.properties.len()
                    );
                    if detached.invert() != variant {
                        return report("detach was not reversible".to_owned());
                    }
                    Ok(())
                }
                None => Err("detach needs a kind; use kind:<kind> first".to_owned()),
            },
            "reset" => match kind.clone() {
                Some(kind) => {
                    let mut variant = Variant::new(instance, &definition, kind);
                    variant.targets = targets.clone();
                    variant.overrides = overrides.clone();
                    let undone = reset(&variant);
                    println!(
                        "reset: dropped {} override(s), kept {} propert(ies)",
                        undone.dropped.len(),
                        undone.reset.overrides.len()
                    );
                    if undone.invert() != variant {
                        return report("reset was not reversible".to_owned());
                    }
                    Ok(())
                }
                None => Err("reset needs a kind; use kind:<kind> first".to_owned()),
            },
            "check" => match kind.clone() {
                Some(kind) => {
                    let mut variant = Variant::new(instance, &definition, kind);
                    variant.targets = targets.clone();
                    match target_compatibility(&variant, argument) {
                        Ok(()) => {
                            println!("{argument}: the variant covers it");
                            Ok(())
                        }
                        Err(reason) => Err(reason),
                    }
                }
                None => Err("check needs a kind; use kind:<kind> first".to_owned()),
            },
            "show" => {
                println!(
                    "variant {} of {:?} on [{}] with {} override(s)",
                    instance.get(),
                    definition,
                    targets.join(", "),
                    overrides.len()
                );
                Ok(())
            }
            other => Err(format!("{other:?} is not a component-override operation")),
        };
        if let Err(reason) = step {
            return report(reason);
        }
    }
    true
}

/// Registers a custom widget and asks whether it is publishable (task T12.04).
pub(super) fn custom_widget(options: &crate::cli::CustomWidgetOptions) -> bool {
    use crate::app::component::custom::{
        CustomWidget, Event, Field, Registry, ValueType, validate,
    };
    use crate::contracts::model::{PropertyName, PropertyValue};
    use std::collections::BTreeMap;

    let mut registry = Registry::new();
    let mut name = String::new();
    let mut widget = CustomWidget::new("widget").expect("a default widget");
    let mut registered = false;
    let mut binds: BTreeMap<String, String> = BTreeMap::new();

    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<(), String> = match verb {
            "widget" => match CustomWidget::new(argument) {
                Some(created) => {
                    name = created.name.clone();
                    widget = created;
                    registered = false;
                    binds.clear();
                    Ok(())
                }
                None => Err(format!("{argument:?} is not a widget name")),
            },
            "field" => match argument.split_once('=') {
                Some((field, value_type)) => {
                    match (PropertyName::new(field), ValueType::parse(value_type)) {
                        (Some(field), Some(value_type)) => {
                            widget.fields.push(Field {
                                name: field,
                                value_type,
                                required: false,
                            });
                            Ok(())
                        }
                        _ => Err(format!("field needs <name>=<type>, got {argument:?}")),
                    }
                }
                None => Err(format!("field needs <name>=<type>, got {argument:?}")),
            },
            "event" => match argument.split_once('=') {
                Some((event, payload)) => match ValueType::parse(payload) {
                    Some(payload) => {
                        widget.events.push(Event {
                            name: event.to_owned(),
                            payload,
                        });
                        Ok(())
                    }
                    None => Err(format!("{payload:?} is not a value type")),
                },
                None => Err(format!("event needs <name>=<type>, got {argument:?}")),
            },
            "impl" => {
                widget.implementations.insert(
                    argument.to_owned(),
                    format!("implementation for {argument}"),
                );
                Ok(())
            }
            "register" => match registry.register(widget.clone()) {
                Ok(()) => {
                    registered = true;
                    Ok(())
                }
                Err(reason) => Err(reason),
            },
            "bind" => match argument.split_once('=') {
                Some((field, value)) => {
                    binds.insert(field.to_owned(), value.to_owned());
                    Ok(())
                }
                None => Err(format!("bind needs <name>=<value>, got {argument:?}")),
            },
            "publish" => match registry.publishable(&name, argument) {
                Ok(verdict) => {
                    println!("{}", verdict.render());
                    if verdict.is_publishable() {
                        Ok(())
                    } else {
                        Err(format!("{name} is not publishable yet"))
                    }
                }
                Err(reason) => Err(reason),
            },
            "validate" => {
                if !registered {
                    return report(
                        "validate needs a registered widget; run register first".to_owned(),
                    );
                }
                let Some(widget) = registry.widget(&name) else {
                    return report(format!("no custom widget {name:?}"));
                };
                let mut values: BTreeMap<PropertyName, PropertyValue> = BTreeMap::new();
                for (field, text) in &binds {
                    let Some(declared) = widget.fields.iter().find(|f| f.name.as_str() == field)
                    else {
                        return report(format!("{field:?} is not a field of {name:?}"));
                    };
                    let value = match declared.value_type {
                        ValueType::Bool => PropertyValue::Bool(text == "true"),
                        ValueType::Integer => match text.parse::<i64>() {
                            Ok(number) => PropertyValue::Integer(number),
                            Err(_) => return report(format!("{text:?} is not an integer")),
                        },
                        ValueType::Number => match text.parse::<f64>() {
                            Ok(number) => PropertyValue::Number(number),
                            Err(_) => return report(format!("{text:?} is not a number")),
                        },
                        ValueType::Text => PropertyValue::Text(text.clone()),
                        ValueType::Color => PropertyValue::Color(text.clone()),
                        ValueType::Asset => PropertyValue::Asset(text.clone()),
                    };
                    values.insert(declared.name.clone(), value);
                }
                match validate(widget, &values) {
                    Ok(()) => {
                        println!("{name}: {} value(s) valid", values.len());
                        Ok(())
                    }
                    Err(reason) => Err(reason),
                }
            }
            "show" => {
                println!("custom widgets: [{}]", registry.names().join(", "));
                Ok(())
            }
            other => Err(format!("{other:?} is not a custom-widget operation")),
        };
        if let Err(reason) = step {
            return report(reason);
        }
    }
    true
}

/// Imports a resource and replaces or moves it (task T12.05).
pub(super) fn resource_import(options: &crate::cli::ResourceImportOptions) -> bool {
    use crate::app::component::resource::{AssetId, AssetKind, Library};
    use crate::contracts::identity::NodeId;

    let mut library = Library::new();
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<(), String> = match verb {
            "asset" => match argument.split(':').collect::<Vec<&str>>().as_slice() {
                [id, path, kind, capacity, bytes] => {
                    match (
                        id.parse::<u64>(),
                        AssetKind::parse(kind),
                        capacity.parse::<u64>(),
                        bytes.parse::<u64>(),
                    ) {
                        (Ok(id), Some(kind), Ok(capacity), Ok(bytes)) => {
                            library.add(AssetId::from_raw(id), path, kind, capacity, bytes)
                        }
                        _ => Err(format!(
                            "asset needs <id>:<path>:<kind>:<capacity>:<bytes>, got {argument:?}"
                        )),
                    }
                }
                _ => Err(format!(
                    "asset needs <id>:<path>:<kind>:<capacity>:<bytes>, got {argument:?}"
                )),
            },
            "refer" => match argument.split(':').collect::<Vec<&str>>().as_slice() {
                [path, node] => match node.parse::<u64>() {
                    Ok(node) => {
                        library.reference(path, NodeId::from_raw(node));
                        Ok(())
                    }
                    Err(_) => Err(format!("{node:?} is not a node id")),
                },
                _ => Err(format!("refer needs <path>:<node>, got {argument:?}")),
            },
            "replace" => match argument.split(':').collect::<Vec<&str>>().as_slice() {
                [id, path, bytes] => match (id.parse::<u64>(), bytes.parse::<u64>()) {
                    (Ok(id), Ok(bytes)) => library.replace(AssetId::from_raw(id), path, bytes),
                    _ => Err(format!(
                        "replace needs <id>:<path>:<bytes>, got {argument:?}"
                    )),
                },
                _ => Err(format!(
                    "replace needs <id>:<path>:<bytes>, got {argument:?}"
                )),
            },
            "move" => match argument.split(':').collect::<Vec<&str>>().as_slice() {
                [id, path] => match id.parse::<u64>() {
                    Ok(id) => library.move_asset(AssetId::from_raw(id), path),
                    Err(_) => Err(format!("{id:?} is not an asset id")),
                },
                _ => Err(format!("move needs <id>:<path>, got {argument:?}")),
            },
            "repoint" => match argument.split(':').collect::<Vec<&str>>().as_slice() {
                [from, to] => match library.repoint(from, to) {
                    Ok(moved) => {
                        println!("repointed {moved} reference(s) from {from} to {to}");
                        Ok(())
                    }
                    Err(reason) => Err(reason),
                },
                _ => Err(format!("repoint needs <from>:<to>, got {argument:?}")),
            },
            "missing" => {
                let missing = library.missing();
                for entry in &missing {
                    println!(
                        "missing {} referenced by {} node(s)",
                        entry.path,
                        entry.referrers.len()
                    );
                }
                if missing.is_empty() {
                    println!("no missing assets");
                }
                Ok(())
            }
            "show" => {
                for asset in library.assets() {
                    println!(
                        "asset {} {} {} ({}/{} bytes)",
                        asset.id.get(),
                        asset.kind.name(),
                        asset.path,
                        asset.bytes,
                        asset.capacity
                    );
                }
                Ok(())
            }
            other => Err(format!("{other:?} is not a resource-import operation")),
        };
        if let Err(reason) = step {
            return report(reason);
        }
    }
    true
}

/// Gives a resource per-target variants and checks them (task T12.06).
pub(super) fn resource_target(options: &crate::cli::ResourceTargetOptions) -> bool {
    use crate::app::component::targets::{
        Resource, ResourceKind, TargetVariant, blocks_generation, check,
    };

    let mut resources: Vec<Resource> = Vec::new();
    let mut verdict = true;
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<(), String> = match verb {
            "resource" => match argument.split(':').collect::<Vec<&str>>().as_slice() {
                [name, kind] => match ResourceKind::parse(kind) {
                    Some(kind) => {
                        resources.push(Resource {
                            name: name.to_string(),
                            kind,
                            variants: Vec::new(),
                        });
                        Ok(())
                    }
                    None => Err(format!("{kind:?} is not a resource kind")),
                },
                _ => Err(format!("resource needs <name>:<kind>, got {argument:?}")),
            },
            "variant" => match argument.split(':').collect::<Vec<&str>>().as_slice() {
                [target, encoding, bytes] => match bytes.parse::<u64>() {
                    Ok(bytes) => match resources.last_mut() {
                        Some(resource) => {
                            resource.variants.push(TargetVariant {
                                target: target.to_string(),
                                encoding: encoding.to_string(),
                                bytes,
                            });
                            Ok(())
                        }
                        None => Err(
                            "variant needs a resource; start with resource:<name>:<kind>"
                                .to_owned(),
                        ),
                    },
                    Err(_) => Err(format!("{bytes:?} is not a size")),
                },
                _ => Err(format!(
                    "variant needs <target>:<encoding>:<bytes>, got {argument:?}"
                )),
            },
            "check" => {
                let findings = check(&resources, argument);
                if findings.is_empty() {
                    println!("{argument}: no blocking findings");
                } else {
                    for finding in &findings {
                        println!("{}", finding.render());
                    }
                }
                verdict = !blocks_generation(&resources, argument);
                Ok(())
            }
            "block" => {
                let blocked = blocks_generation(&resources, argument);
                println!("{argument}: generation blocked = {blocked}");
                verdict = !blocked;
                Ok(())
            }
            "show" => {
                for resource in &resources {
                    println!(
                        "{} {} ({} variants)",
                        resource.name,
                        resource.kind.name(),
                        resource.variants.len()
                    );
                }
                Ok(())
            }
            other => Err(format!("{other:?} is not a resource-target operation")),
        };
        if let Err(reason) = step {
            return report(reason);
        }
    }
    verdict
}

/// Plans a package import and commits or cancels it (task T12.07).
pub(super) fn asset_package(options: &crate::cli::AssetPackageOptions) -> bool {
    use crate::app::component::package::{Entry, Package, Plan, import_plan};
    use std::collections::{BTreeMap, BTreeSet};

    let mut pending: Option<Package> = None;
    let mut existing: BTreeSet<String> = BTreeSet::new();
    let mut registry: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut plan: Option<Plan> = None;

    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<(), String> = match verb {
            "package" => match argument.split(':').collect::<Vec<&str>>().as_slice() {
                [name, version, license] => match version.parse::<u32>() {
                    Ok(version) => match Package::new(name, version, license) {
                        Some(created) => {
                            pending = Some(created);
                            plan = None;
                            Ok(())
                        }
                        None => Err("a package name cannot be blank".to_owned()),
                    },
                    Err(_) => Err(format!("{version:?} is not a version")),
                },
                _ => Err(format!(
                    "package needs <name>:<version>:<license>, got {argument:?}"
                )),
            },
            "depends" => match pending.as_mut() {
                Some(package) => {
                    package.depends_on.push(argument.to_owned());
                    Ok(())
                }
                None => Err(
                    "depends needs a package; start with package:<name>:<version>:<license>"
                        .to_owned(),
                ),
            },
            "entry" => match argument.split(':').collect::<Vec<&str>>().as_slice() {
                [path, version] => match version.parse::<u32>() {
                    Ok(version) => match pending.as_mut() {
                        Some(package) => {
                            let license = package.license.clone();
                            package.entries.push(Entry {
                                path: path.to_string(),
                                version,
                                license,
                            });
                            Ok(())
                        }
                        None => Err(
                            "entry needs a package; start with package:<name>:<version>:<license>"
                                .to_owned(),
                        ),
                    },
                    Err(_) => Err(format!("{version:?} is not a version")),
                },
                _ => Err(format!("entry needs <path>:<version>, got {argument:?}")),
            },
            "existing" => {
                existing.insert(argument.to_owned());
                Ok(())
            }
            "dependency" => match argument.split(':').collect::<Vec<&str>>().as_slice() {
                [name, dependency] => {
                    registry
                        .entry(name.to_string())
                        .or_default()
                        .push(dependency.to_string());
                    Ok(())
                }
                _ => Err(format!(
                    "dependency needs <name>:<dependency>, got {argument:?}"
                )),
            },
            "plan" => match pending.as_ref() {
                Some(package) => match import_plan(package, &registry, &existing) {
                    Ok(built) => {
                        for entry in built.writes() {
                            println!("would write {}", entry.path);
                        }
                        plan = Some(built);
                        Ok(())
                    }
                    Err(refusal) => Err(refusal.render()),
                },
                None => Err(
                    "plan needs a package; start with package:<name>:<version>:<license>"
                        .to_owned(),
                ),
            },
            "commit" => match plan.as_mut() {
                Some(plan) => {
                    plan.commit();
                    println!("applied {} write(s)", plan.applied().len());
                    Ok(())
                }
                None => Err("commit needs a plan; run plan first".to_owned()),
            },
            "cancel" => match plan.as_mut() {
                Some(plan) => {
                    plan.cancel();
                    println!("cancelled: {} write(s) applied", plan.applied().len());
                    Ok(())
                }
                None => Err("cancel needs a plan; run plan first".to_owned()),
            },
            "show" => {
                match &pending {
                    Some(package) => println!("package {} v{}", package.name, package.version),
                    None => println!("no package"),
                }
                Ok(())
            }
            other => Err(format!("{other:?} is not an asset-package operation")),
        };
        if let Err(reason) = step {
            return report(reason);
        }
    }
    true
}

/// Upgrades a definition and compares the round trip with the static artifact (task T12.08).
pub(super) fn component_upgrade(options: &crate::cli::ComponentUpgradeOptions) -> bool {
    use crate::app::component::upgrade::{
        Definition, Instance, Project, StaticArtifact, delete_asset, parse, resolve, serialize,
        upgrade,
    };
    use crate::contracts::identity::NodeId;
    use crate::contracts::model::{PropertyName, PropertyValue};
    use std::collections::BTreeMap;

    let mut project = Project::default();
    let mut current_definition: Option<String> = None;
    let mut changes: Vec<(PropertyName, PropertyValue)> = Vec::new();

    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<(), String> = match verb {
            "definition" => match argument.split(':').collect::<Vec<&str>>().as_slice() {
                [name, version] => match version.parse::<u32>() {
                    Ok(version) => {
                        project.definitions.insert(
                            name.to_string(),
                            Definition {
                                name: name.to_string(),
                                version,
                                properties: BTreeMap::new(),
                            },
                        );
                        current_definition = Some(name.to_string());
                        Ok(())
                    }
                    Err(_) => Err(format!("{version:?} is not a version")),
                },
                _ => Err(format!(
                    "definition needs <name>:<version>, got {argument:?}"
                )),
            },
            "property" => match current_definition.clone() {
                Some(name) => match argument.split_once('=') {
                    Some((property, value)) => match PropertyName::new(property) {
                        Some(property) => {
                            if let Some(definition) = project.definitions.get_mut(&name) {
                                definition
                                    .properties
                                    .insert(property, PropertyValue::Text(value.to_owned()));
                            }
                            Ok(())
                        }
                        None => Err(format!("{property:?} is not a property name")),
                    },
                    None => Err(format!("property needs <name>=<value>, got {argument:?}")),
                },
                None => Err(
                    "property needs a definition; start with definition:<name>:<version>"
                        .to_owned(),
                ),
            },
            "instance" => match argument.split(':').collect::<Vec<&str>>().as_slice() {
                [id, definition] => match id.parse::<u64>() {
                    Ok(id) => {
                        project.instances.push(Instance {
                            id: NodeId::from_raw(id),
                            definition: definition.to_string(),
                            overrides: BTreeMap::new(),
                        });
                        Ok(())
                    }
                    Err(_) => Err(format!("{id:?} is not an instance id")),
                },
                _ => Err(format!(
                    "instance needs <id>:<definition>, got {argument:?}"
                )),
            },
            "override" => match argument.split_once(':') {
                Some((id, assignment)) => match (id.parse::<u64>(), assignment.split_once('=')) {
                    (Ok(id), Some((property, value))) => match PropertyName::new(property) {
                        Some(property) => match project
                            .instances
                            .iter_mut()
                            .find(|instance| instance.id == NodeId::from_raw(id))
                        {
                            Some(instance) => {
                                instance
                                    .overrides
                                    .insert(property, PropertyValue::Text(value.to_owned()));
                                Ok(())
                            }
                            None => Err(format!("no instance {id}")),
                        },
                        None => Err(format!("{property:?} is not a property name")),
                    },
                    _ => Err(format!(
                        "override needs <id>:<name>=<value>, got {argument:?}"
                    )),
                },
                None => Err(format!(
                    "override needs <id>:<name>=<value>, got {argument:?}"
                )),
            },
            "asset" => {
                project.assets.insert(argument.to_owned());
                Ok(())
            }
            "change" => match argument.split_once('=') {
                Some((property, value)) => match PropertyName::new(property) {
                    Some(property) => {
                        changes.push((property, PropertyValue::Text(value.to_owned())));
                        Ok(())
                    }
                    None => Err(format!("{property:?} is not a property name")),
                },
                None => Err(format!("change needs <name>=<value>, got {argument:?}")),
            },
            "upgrade" => match upgrade(&mut project, argument, &changes) {
                Ok(outcome) => {
                    let affected: Vec<String> = outcome
                        .affected
                        .iter()
                        .map(|id| id.get().to_string())
                        .collect();
                    println!(
                        "upgrade {}: v{} -> v{}, affected [{}], preserved {} override(s)",
                        outcome.name,
                        outcome.from,
                        outcome.to,
                        affected.join(", "),
                        outcome.preserved.len()
                    );
                    changes.clear();
                    Ok(())
                }
                Err(reason) => Err(reason),
            },
            "delete-asset" => delete_asset(&mut project, argument),
            "roundtrip" => match parse(&serialize(&project)) {
                Ok(parsed) => {
                    let mut equal = parsed == project;
                    for instance in &project.instances {
                        match (
                            resolve(&project, instance.id),
                            resolve(&parsed, instance.id),
                        ) {
                            (Ok(left), Ok(right)) => {
                                if left != right {
                                    equal = false;
                                }
                            }
                            _ => equal = false,
                        }
                    }
                    println!(
                        "roundtrip: {}",
                        if equal { "identical" } else { "different" }
                    );
                    if equal {
                        Ok(())
                    } else {
                        Err("the round trip changed the project".to_owned())
                    }
                }
                Err(reason) => Err(reason),
            },
            "static" => match StaticArtifact::expand(&project) {
                Ok(artifact) => {
                    let mut equal = true;
                    for instance in &project.instances {
                        match (resolve(&project, instance.id), artifact.get(instance.id)) {
                            (Ok(live), Some(pre)) => {
                                if live != *pre {
                                    equal = false;
                                }
                            }
                            _ => equal = false,
                        }
                    }
                    println!(
                        "static artifact: {}",
                        if equal {
                            "same instance semantics"
                        } else {
                            "different"
                        }
                    );
                    if equal {
                        Ok(())
                    } else {
                        Err("the static artifact disagrees with the project".to_owned())
                    }
                }
                Err(reason) => Err(reason),
            },
            "show" => {
                println!(
                    "project: {} definition(s), {} instance(s), {} asset(s)",
                    project.definitions.len(),
                    project.instances.len(),
                    project.assets.len()
                );
                Ok(())
            }
            other => Err(format!("{other:?} is not a component-upgrade operation")),
        };
        if let Err(reason) = step {
            return report(reason);
        }
    }
    true
}

/// Builds the asset manifest and its derived resources (task T12.09).
pub(super) fn asset_manifest(options: &crate::cli::AssetManifestOptions) -> bool {
    use crate::app::component::manifest::{Asset, AssetId, AssetManifest, Packaging, parse_static};

    let mut manifest = AssetManifest::new();
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<(), String> = match verb {
            "asset" => match argument.split(':').collect::<Vec<&str>>().as_slice() {
                [id, path, packaging] => match (id.parse::<u64>(), Packaging::parse(packaging)) {
                    (Ok(id), Some(packaging)) => manifest
                        .insert(Asset {
                            id: AssetId::from_raw(id),
                            path: path.to_string(),
                            packaging,
                            provenance: None,
                            transform: None,
                        })
                        .map_err(|finding| finding.render()),
                    _ => Err(format!(
                        "asset needs <id>:<path>:<packaging>, got {argument:?}"
                    )),
                },
                _ => Err(format!(
                    "asset needs <id>:<path>:<packaging>, got {argument:?}"
                )),
            },
            "derive" => match argument.split(':').collect::<Vec<&str>>().as_slice() {
                [id, source, transform, path, packaging] => {
                    match (
                        id.parse::<u64>(),
                        source.parse::<u64>(),
                        Packaging::parse(packaging),
                    ) {
                        (Ok(id), Ok(source), Some(packaging)) => manifest
                            .derive(
                                AssetId::from_raw(source),
                                AssetId::from_raw(id),
                                path,
                                transform,
                                packaging,
                            )
                            .map_err(|finding| finding.render()),
                        _ => Err(format!(
                            "derive needs <id>:<source>:<transform>:<path>:<packaging>, got {argument:?}"
                        )),
                    }
                }
                _ => Err(format!(
                    "derive needs <id>:<source>:<transform>:<path>:<packaging>, got {argument:?}"
                )),
            },
            "move" => match argument.split(':').collect::<Vec<&str>>().as_slice() {
                [id, path] => match id.parse::<u64>() {
                    Ok(id) => manifest
                        .move_asset(AssetId::from_raw(id), path)
                        .map_err(|finding| finding.render()),
                    Err(_) => Err(format!("{id:?} is not an asset id")),
                },
                _ => Err(format!("move needs <id>:<path>, got {argument:?}")),
            },
            "resolve" => match argument.parse::<u64>() {
                Ok(id) => {
                    match manifest.resolve(AssetId::from_raw(id)) {
                        Some(path) => println!("asset {id} -> {path}"),
                        None => println!("asset {id} is unresolved"),
                    }
                    Ok(())
                }
                Err(_) => Err(format!("{argument:?} is not an asset id")),
            },
            "missing" => match argument.parse::<u64>() {
                Ok(id) => {
                    if manifest.asset(AssetId::from_raw(id)).is_some() {
                        println!("asset {id} is present");
                    } else {
                        println!("asset {id} is missing; fix: import or re-derive it");
                    }
                    Ok(())
                }
                Err(_) => Err(format!("{argument:?} is not an asset id")),
            },
            "static" => match parse_static(&manifest.static_manifest()) {
                Some(static_entries) => {
                    let equal = static_entries == manifest.runtime_index();
                    println!(
                        "static manifest: {}",
                        if equal {
                            "resolves the same identities as the runtime lookup"
                        } else {
                            "differs from the runtime lookup"
                        }
                    );
                    if equal {
                        Ok(())
                    } else {
                        Err("the static manifest disagrees with the runtime lookup".to_owned())
                    }
                }
                None => Err("the static manifest could not be read".to_owned()),
            },
            "show" => {
                println!("manifest: {} asset(s)", manifest.runtime_index().len());
                Ok(())
            }
            other => Err(format!("{other:?} is not an asset-manifest operation")),
        };
        if let Err(reason) = step {
            return report(reason);
        }
    }
    true
}
