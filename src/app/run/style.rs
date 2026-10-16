// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! F11: attributes, CSS, design tokens, themes and transitions.
//!
//! One command handler per parsed command of this domain; the dispatcher lives in
//! [`super`](self).

use super::*;

/// Drives the schema-driven property editor (task T11.01).
pub(super) fn property_editor(options: &crate::cli::PropertyEditorOptions) -> bool {
    use crate::app::style::property_editor::{Editor, Schema};
    use crate::contracts::model::PropertyName;

    let mut editor = Editor::new(Schema::new());
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "schema" => match argument.split_once(':') {
                Some((property, kind)) => {
                    match (PropertyName::new(property), parse_property_type(kind)) {
                        (Some(name), Ok(kind)) => {
                            editor.describe(name.clone(), kind.clone());
                            Ok(vec![format!("schema {}: {}", name.as_str(), kind.render())])
                        }
                        (None, _) => Err(format!("{property:?} is not a property name")),
                        (_, Err(reason)) => Err(reason),
                    }
                }
                None => Err(format!("{argument:?} is not <property>:<type>")),
            },
            "set" => match parse_field(argument) {
                Ok((node, name, text)) => match editor.set(node, name.clone(), &text) {
                    Ok(()) => Ok(vec![format!(
                        "set {} {}: accepted",
                        node.get(),
                        name.as_str()
                    )]),
                    Err(reason) => Ok(vec![format!(
                        "set {} {}: refused — {reason}",
                        node.get(),
                        name.as_str()
                    )]),
                },
                Err(reason) => Err(reason),
            },
            "value" => match parse_node_property(argument) {
                Ok((node, name)) => Ok(vec![match editor.value(node, &name) {
                    Some(value) => {
                        format!("value {} {}: {}", node.get(), name.as_str(), value.family())
                    }
                    None => format!("value {} {}: unset", node.get(), name.as_str()),
                }]),
                Err(reason) => Err(reason),
            },
            "pending" => match parse_node_property(argument) {
                Ok((node, name)) => {
                    let mut lines = Vec::new();
                    if let Some(text) = editor.pending(node, &name) {
                        lines.push(format!(
                            "pending {} {}: {text:?}",
                            node.get(),
                            name.as_str()
                        ));
                    }
                    if let Some(reason) = editor.problem(node, &name) {
                        lines.push(format!(
                            "problem {} {}: {reason}",
                            node.get(),
                            name.as_str()
                        ));
                    }
                    if lines.is_empty() {
                        lines.push(format!("pending {} {}: nothing", node.get(), name.as_str()));
                    }
                    Ok(lines)
                }
                Err(reason) => Err(reason),
            },
            "state" => Ok(editor.render()),
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a property-editor operation")),
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
    for line in editor.render() {
        println!("{line}");
    }
    true
}

/// A `(node, property, text)` from `<node>:<property>:<text>`.
pub(super) fn parse_field(
    argument: &str,
) -> Result<
    (
        crate::contracts::identity::NodeId,
        crate::contracts::model::PropertyName,
        String,
    ),
    String,
> {
    let fields: Vec<&str> = argument.splitn(3, ':').collect();
    let [node, property, text] = fields.as_slice() else {
        return Err(format!("{argument:?} is not <node>:<property>:<text>"));
    };
    let node = parse_scene_node(node)?;
    let property = crate::contracts::model::PropertyName::new(property)
        .ok_or_else(|| format!("{property:?} is not a property name"))?;
    Ok((node, property, (*text).to_owned()))
}

/// A `(node, property)` from `<node>:<property>`.
pub(super) fn parse_node_property(
    argument: &str,
) -> Result<
    (
        crate::contracts::identity::NodeId,
        crate::contracts::model::PropertyName,
    ),
    String,
> {
    let (node, property) = argument
        .split_once(':')
        .ok_or_else(|| format!("{argument:?} is not <node>:<property>"))?;
    let node = parse_scene_node(node)?;
    let property = crate::contracts::model::PropertyName::new(property)
        .ok_or_else(|| format!("{property:?} is not a property name"))?;
    Ok((node, property))
}

/// Inspects a multi-selection's shared properties and resets an override (task T11.02).
pub(super) fn multi_property(options: &crate::cli::MultiPropertyOptions) -> bool {
    use crate::app::style::multi_property::{Common, NodeProperties, common, reset, value_of};
    use crate::contracts::model::PropertyName;

    let mut nodes: Vec<NodeProperties> = Vec::new();
    let mut current: Option<usize> = None;
    let mut resets: Vec<String> = Vec::new();
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "node" => match parse_scene_node(argument) {
                Ok(node) => {
                    let index = nodes.iter().position(|existing| existing.node == node);
                    let index = index.unwrap_or_else(|| {
                        nodes.push(NodeProperties::new(node));
                        nodes.len() - 1
                    });
                    current = Some(index);
                    Ok(vec![format!("node: {}", node.get())])
                }
                Err(reason) => Err(reason),
            },
            "set" => match parse_setting(argument) {
                Ok((name, value, origin)) => match current {
                    Some(index) => {
                        nodes[index].settings.insert(
                            name.clone(),
                            crate::app::style::multi_property::Setting { value, origin },
                        );
                        Ok(vec![format!("set {} ({})", name.as_str(), origin.name())])
                    }
                    None => Err("no node is selected".to_owned()),
                },
                Err(reason) => Err(reason),
            },
            "common" => {
                let shared = common(&nodes);
                let names: Vec<&str> = shared.iter().map(PropertyName::as_str).collect();
                Ok(vec![format!("common: {}", names.join(", "))])
            }
            "value" => match PropertyName::new(argument) {
                Some(name) => Ok(vec![match value_of(&nodes, &name) {
                    Common::Agreed(value) => {
                        format!("value {}: agreed ({})", name.as_str(), value.family())
                    }
                    Common::Mixed => format!("value {}: mixed", name.as_str()),
                }]),
                None => Err(format!("{argument:?} is not a property name")),
            },
            "reset" => match parse_reset(argument) {
                Ok((origin, fallback)) => {
                    let outcome = reset(origin, fallback.as_ref());
                    resets.push(outcome.render());
                    Ok(vec![outcome.render()])
                }
                Err(reason) => Err(reason),
            },
            "state" => Ok(render_multi(&nodes, &resets)),
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a multi-property operation")),
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
    for line in render_multi(&nodes, &resets) {
        println!("{line}");
    }
    true
}

pub(super) fn css(options: &crate::cli::CssOptions) -> bool {
    use crate::app::style::css_view::{Css, Element, compute};

    let mut source = String::new();
    let mut element: Option<Element> = None;
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "css" => {
                source.push_str(argument);
                source.push('\n');
                Ok(Vec::new())
            }
            "element" => match Element::parse(argument) {
                Ok(parsed) => {
                    let line = format!("element: {}", parsed.render());
                    element = Some(parsed);
                    Ok(vec![line])
                }
                Err(reason) => Err(reason),
            },
            "value" | "source" | "properties" => match &element {
                Some(element) => {
                    let stylesheet = Css::parse(&source);
                    let computed = compute(&stylesheet, element);
                    match verb {
                        "value" => Ok(vec![match computed.value_of(argument) {
                            Some(value) => format!("{argument}: {value}"),
                            None => format!("{argument}: (no rule sets it)"),
                        }]),
                        "source" => Ok(vec![match computed.source_of(argument) {
                            Some(found) => format!("{argument}: {}", found.render()),
                            None => format!("{argument}: (no rule sets it)"),
                        }]),
                        _ => Ok(computed
                            .properties()
                            .iter()
                            .map(|p| (*p).to_owned())
                            .collect()),
                    }
                }
                None => Err("no element is selected".to_owned()),
            },
            "state" => {
                let stylesheet = Css::parse(&source);
                let mut lines = stylesheet.render();
                if let Some(element) = &element {
                    lines.push(format!("element: {}", element.render()));
                    lines.extend(compute(&stylesheet, element).render());
                }
                Ok(lines)
            }
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a CSS operation")),
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

pub(super) fn tokens(options: &crate::cli::TokensOptions) -> bool {
    use crate::app::style::design_tokens::{TokenSet, Usage, rename_references};

    let mut set = TokenSet::new();
    let mut usages: Vec<Usage> = Vec::new();
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "token" => define_token_from(&mut set, argument).map(|line| vec![line]),
            "theme" => define_theme_from(&mut set, argument).map(|line| vec![line]),
            "use" => match argument.split_once(':') {
                Some((property, value)) => {
                    usages.push(Usage {
                        property: property.to_owned(),
                        value: value.to_owned(),
                    });
                    Ok(vec![format!("use {property}: {value}")])
                }
                None => Err(format!("use needs <property>:<value>, got {argument:?}")),
            },
            "expand" => {
                let theme = if argument.is_empty() {
                    None
                } else {
                    Some(argument)
                };
                let mut lines = Vec::new();
                let mut refused = 0;
                for usage in &usages {
                    match set.expand(&usage.value, theme) {
                        Ok(expansion) => lines.push(format!(
                            "{}: {} -> {}",
                            usage.property,
                            usage.value,
                            expansion.render()
                        )),
                        Err(error) => {
                            refused += 1;
                            lines.push(format!(
                                "{}: {} -> REFUSED: {}",
                                usage.property,
                                usage.value,
                                error.message()
                            ));
                        }
                    }
                }
                if refused > 0 {
                    for line in &lines {
                        println!("{line}");
                    }
                    return report(format!("{refused} reference(s) could not be expanded"));
                }
                Ok(lines)
            }
            "rename" => match argument.split_once(':') {
                Some((from, to)) => match set.rename(from, to) {
                    Ok(()) => {
                        let mut updated = 0;
                        for usage in &mut usages {
                            let (text, count) = rename_references(&usage.value, from, to);
                            usage.value = text;
                            updated += count;
                        }
                        Ok(vec![format!(
                            "renamed {from} -> {to}, {updated} reference(s) updated"
                        )])
                    }
                    Err(reason) => Err(reason),
                },
                None => Err(format!("rename needs <from>:<to>, got {argument:?}")),
            },
            "state" => {
                let mut lines = set.render();
                lines.push(format!("usages: {}", usages.len()));
                for usage in &usages {
                    lines.push(format!("  {}: {}", usage.property, usage.value));
                }
                Ok(lines)
            }
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a token operation")),
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

pub(super) fn parse_design_token(
    argument: &str,
) -> Result<(crate::app::style::design_tokens::TokenKind, String, String), String> {
    use crate::app::style::design_tokens::TokenKind;
    let parts = argument.split_once(':').and_then(|(kind, rest)| {
        rest.split_once(':')
            .map(|(name, value)| (kind, name, value))
    });
    match parts {
        Some((kind, name, value)) => match TokenKind::parse(kind) {
            Some(kind) => Ok((kind, name.to_owned(), value.to_owned())),
            None => Err(format!("{kind:?} is not a token kind")),
        },
        None => Err(format!(
            "token needs <kind>:<name>:<value>, got {argument:?}"
        )),
    }
}

pub(super) fn define_token_from(
    set: &mut crate::app::style::design_tokens::TokenSet,
    argument: &str,
) -> Result<String, String> {
    let (kind, name, value) = parse_design_token(argument)?;
    set.define(kind, &name, &value);
    Ok(format!("token {name} ({})", kind.name()))
}

pub(super) fn parse_theme(argument: &str) -> Result<(String, Vec<(String, String)>), String> {
    match argument.split_once(':') {
        Some((name, pairs)) => {
            let mut values = Vec::new();
            for pair in pairs.split(',') {
                match pair.split_once('=') {
                    Some((token, value)) => values.push((token.to_owned(), value.to_owned())),
                    None => {
                        return Err(format!("a theme entry needs <token>=<value>, got {pair:?}"));
                    }
                }
            }
            Ok((name.to_owned(), values))
        }
        None => Err(format!(
            "theme needs <name>:<token>=<value>, got {argument:?}"
        )),
    }
}

pub(super) fn define_theme_from(
    set: &mut crate::app::style::design_tokens::TokenSet,
    argument: &str,
) -> Result<String, String> {
    let (name, values) = parse_theme(argument)?;
    set.define_theme(&name, &values);
    Ok(format!("theme {name}: {} override(s)", values.len()))
}

pub(super) fn states(options: &crate::cli::StatesOptions) -> bool {
    use crate::app::style::css_view::{Css, Element, State, compute_in};
    use crate::app::style::design_tokens::TokenSet;
    use crate::app::style::interaction_states::{Report, compare, literal, preview, resolved};

    let mut source = String::new();
    let mut element: Option<Element> = None;
    let mut requested: Vec<State> = Vec::new();
    let mut declared: Vec<String> = Vec::new();
    let mut set = TokenSet::new();
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "css" => {
                source.push_str(argument);
                source.push('\n');
                Ok(Vec::new())
            }
            "element" => match Element::parse(argument) {
                Ok(parsed) => {
                    let line = format!("element: {}", parsed.render());
                    element = Some(parsed);
                    Ok(vec![line])
                }
                Err(reason) => Err(reason),
            },
            "on" => match State::parse(argument) {
                Some(state) => {
                    if !requested.contains(&state) {
                        requested.push(state);
                    }
                    Ok(vec![format!("on: {}", state.name())])
                }
                None => Err(format!("{argument:?} is not an interaction state")),
            },
            "declared" => {
                declared.push(argument.to_owned());
                Ok(vec![format!("declared: {argument}")])
            }
            "token" => define_token_from(&mut set, argument).map(|line| vec![line]),
            "theme" => define_theme_from(&mut set, argument).map(|line| vec![line]),
            "preview" => match &element {
                Some(element) => {
                    let css = Css::parse(&source);
                    let report = Report {
                        previews: preview(&css, element, &requested),
                        declared: declared.clone(),
                    };
                    Ok(report.render())
                }
                None => Err("no element is selected".to_owned()),
            },
            "compare" => match &element {
                Some(element) => {
                    let (left_name, right_name) = match argument.split_once(':') {
                        Some((left, right)) => (left, Some(right)),
                        None => (argument, None),
                    };
                    let left_theme =
                        (left_name != "-" && !left_name.is_empty()).then_some(left_name);
                    let right_theme = right_name
                        .and_then(|name| (name != "-" && !name.is_empty()).then_some(name));
                    let css = Css::parse(&source);
                    let style = compute_in(&css, element, None);
                    match (
                        resolved(&style, &set, left_theme),
                        resolved(&style, &set, right_theme),
                    ) {
                        (Ok(left), Ok(right)) => {
                            let left_label = left_theme.unwrap_or("(no theme)");
                            let right_label = right_theme.unwrap_or("(no theme)");
                            let differences = compare(&left, &right);
                            if differences.is_empty() {
                                Ok(vec![format!(
                                    "the style is identical under {left_label} and {right_label}"
                                )])
                            } else {
                                Ok(differences
                                    .iter()
                                    .map(|difference| difference.render(left_label, right_label))
                                    .collect())
                            }
                        }
                        (Err(error), _) | (_, Err(error)) => Err(error.message()),
                    }
                }
                None => Err("no element is selected".to_owned()),
            },
            "diff" => match &element {
                Some(element) => match argument.split_once(':') {
                    Some((left, right)) => match (State::parse(left), State::parse(right)) {
                        (Some(left), Some(right)) => {
                            let css = Css::parse(&source);
                            let left_style = literal(&compute_in(&css, element, Some(left)));
                            let right_style = literal(&compute_in(&css, element, Some(right)));
                            let differences = compare(&left_style, &right_style);
                            if differences.is_empty() {
                                Ok(vec![format!(
                                    "{} and {} are identical",
                                    left.name(),
                                    right.name()
                                )])
                            } else {
                                Ok(differences
                                    .iter()
                                    .map(|difference| difference.render(left.name(), right.name()))
                                    .collect())
                            }
                        }
                        _ => Err(format!("{argument:?} needs two interaction states")),
                    },
                    None => Err(format!("diff needs <state>:<state>, got {argument:?}")),
                },
                None => Err("no element is selected".to_owned()),
            },
            "state" => {
                let mut lines = set.render();
                lines.push(format!(
                    "element: {}",
                    element
                        .as_ref()
                        .map(Element::render)
                        .unwrap_or_else(|| "(none)".to_owned())
                ));
                let names: Vec<&str> = requested.iter().map(State::name).collect();
                lines.push(format!("on: {}", names.join(", ")));
                lines.push(format!("declared: {}", declared.join(", ")));
                Ok(lines)
            }
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a state operation")),
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

pub(super) fn parse_motion(
    argument: &str,
    effect: crate::app::style::motion::Effect,
) -> Result<crate::app::style::motion::Motion, String> {
    use crate::app::style::motion::{Easing, Motion, Timing};
    let parts: Vec<&str> = argument.split(':').collect();
    if parts.len() != 4 {
        return Err(format!(
            "a motion needs <property>:<ms>:<delay_ms>:<easing>, got {argument:?}"
        ));
    }
    let duration: u32 = parts[1]
        .parse()
        .map_err(|_| format!("{:?} is not a duration in milliseconds", parts[1]))?;
    let delay: u32 = parts[2]
        .parse()
        .map_err(|_| format!("{:?} is not a delay in milliseconds", parts[2]))?;
    let easing =
        Easing::parse(parts[3]).ok_or_else(|| format!("{:?} is not an easing", parts[3]))?;
    Ok(Motion {
        effect,
        property: parts[0].to_owned(),
        timing: Timing::new(duration, delay, easing),
    })
}

pub(super) fn motion(options: &crate::cli::MotionOptions) -> bool {
    use crate::app::style::motion::{Capability, Control, Easing, Effect, Timeline};

    let mut capability = Capability::new();
    let mut timeline = Timeline::new();
    let mut control = Control::new(0);
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "transitions" | "animations" => {
                let on = match argument {
                    "on" => true,
                    "off" => false,
                    other => return report(format!("{other:?} is not on or off")),
                };
                if verb == "transitions" {
                    capability.transitions = on;
                } else {
                    capability.animations = on;
                }
                Ok(vec![format!("{verb}: {}", if on { "on" } else { "off" })])
            }
            "property" => {
                capability.properties.insert(argument.to_owned());
                Ok(vec![format!("property: {argument}")])
            }
            "easing" => match Easing::parse(argument) {
                Some(easing) => {
                    capability.easings.insert(easing.name().to_owned());
                    Ok(vec![format!("easing: {}", easing.name())])
                }
                None => Err(format!("{argument:?} is not an easing")),
            },
            "transition" | "animation" => {
                let effect = if verb == "transition" {
                    Effect::Transition
                } else {
                    Effect::Animation
                };
                match parse_motion(argument, effect) {
                    Ok(motion) => {
                        let line = format!(
                            "{} {} {}",
                            motion.effect.name(),
                            motion.property,
                            motion.timing.render()
                        );
                        timeline.motions.push(motion);
                        Ok(vec![line])
                    }
                    Err(reason) => Err(reason),
                }
            }
            "check" => {
                let verdicts = timeline.admit(&capability);
                let mut lines = Vec::new();
                let mut refused = 0;
                for (motion, verdict) in &verdicts {
                    if !verdict.is_accepted() {
                        refused += 1;
                    }
                    let label = format!("{} {}", motion.effect.name(), motion.property);
                    lines.push(verdict.render(&label));
                }
                if refused > 0 {
                    for line in &lines {
                        println!("{line}");
                    }
                    return report(format!("{refused} motion(s) the target cannot run"));
                }
                Ok(lines)
            }
            "play" => {
                control.duration_ms = timeline.duration_ms();
                control.play();
                Ok(vec![control.render()])
            }
            "pause" => {
                control.pause();
                Ok(vec![control.render()])
            }
            "seek" => match argument.parse::<u32>() {
                Ok(position) => {
                    control.duration_ms = timeline.duration_ms();
                    let clamped = control.seek(position);
                    let mut lines = vec![control.render()];
                    if clamped {
                        lines.push("the position was past the end, so it was clamped".to_owned());
                    }
                    Ok(lines)
                }
                Err(_) => Err(format!("{argument:?} is not a position in milliseconds")),
            },
            "state" => {
                control.duration_ms = timeline.duration_ms();
                let mut lines = capability.render();
                lines.push(format!(
                    "timeline: {} motion(s), {}ms",
                    timeline.motions.len(),
                    timeline.duration_ms()
                ));
                for item in &timeline.motions {
                    lines.push(format!(
                        "  {} {} {}",
                        item.effect.name(),
                        item.property,
                        item.timing.render()
                    ));
                }
                lines.push(format!("control: {}", control.render()));
                Ok(lines)
            }
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a motion operation")),
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

pub(super) fn style(options: &crate::cli::StyleOptions) -> bool {
    use std::collections::BTreeMap;

    use crate::app::style::css_view::{Css, Element};
    use crate::app::style::design_tokens::TokenSet;
    use crate::app::style::style_emit::{Input, Mode, emit, migrate};

    let mut source = String::new();
    let mut element: Option<Element> = None;
    let mut inline: BTreeMap<String, String> = BTreeMap::new();
    let mut tokens = TokenSet::new();
    let mut theme: Option<String> = None;
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "css" => {
                source.push_str(argument);
                source.push('\n');
                Ok(Vec::new())
            }
            "element" => match Element::parse(argument) {
                Ok(parsed) => {
                    let line = format!("element: {}", parsed.render());
                    element = Some(parsed);
                    Ok(vec![line])
                }
                Err(reason) => Err(reason),
            },
            "inline" => match argument.split_once(':') {
                Some((property, value)) => {
                    inline.insert(property.to_owned(), value.to_owned());
                    Ok(vec![format!("inline {property}: {value}")])
                }
                None => Err(format!("inline needs <property>:<value>, got {argument:?}")),
            },
            "token" => define_token_from(&mut tokens, argument).map(|line| vec![line]),
            "theme" => define_theme_from(&mut tokens, argument).map(|line| vec![line]),
            "under" => {
                if argument == "-" || argument == "none" || argument.is_empty() {
                    theme = None;
                    Ok(vec!["under: (no theme)".to_owned()])
                } else if tokens.has_theme(argument) {
                    theme = Some(argument.to_owned());
                    Ok(vec![format!("under: {argument}")])
                } else {
                    Err(format!("no theme {argument:?} is defined"))
                }
            }
            "preview" | "generate" | "migrate" => match &element {
                Some(element) => {
                    let css = Css::parse(&source);
                    let input = Input {
                        css: &css,
                        element,
                        inline: &inline,
                        tokens: &tokens,
                        theme: theme.as_deref(),
                    };
                    match emit(&input) {
                        Ok(style) => match verb {
                            "preview" => Ok(style.render(Mode::Preview)),
                            "generate" => Ok(style.render(Mode::Generate)),
                            _ => {
                                let (_, migrations) = migrate(&style, &tokens);
                                if migrations.is_empty() {
                                    Ok(vec!["nothing to migrate".to_owned()])
                                } else {
                                    Ok(migrations
                                        .iter()
                                        .map(|migration| migration.render())
                                        .collect())
                                }
                            }
                        },
                        Err(diagnostics) => {
                            for diagnostic in &diagnostics {
                                println!("{}", diagnostic.render());
                            }
                            return report(format!(
                                "{} declaration(s) could not be emitted",
                                diagnostics.len()
                            ));
                        }
                    }
                }
                None => Err("no element is selected".to_owned()),
            },
            "state" => {
                let mut lines = tokens.render();
                lines.push(format!(
                    "element: {}",
                    element
                        .as_ref()
                        .map(Element::render)
                        .unwrap_or_else(|| "(none)".to_owned())
                ));
                lines.push(format!("inline: {}", inline.len()));
                for (property, value) in &inline {
                    lines.push(format!("  {property}: {value}"));
                }
                lines.push(format!(
                    "under: {}",
                    theme.as_deref().unwrap_or("(no theme)")
                ));
                Ok(lines)
            }
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a style operation")),
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

pub(super) fn style_boundary(options: &crate::cli::StyleBoundaryOptions) -> bool {
    use crate::app::style::css_view::{Css, Element};
    use crate::app::style::design_tokens::TokenSet;
    use crate::app::style::style_boundary::{Level, Session, contrast, meets};
    use crate::app::style::style_emit::Mode;

    let mut session = Session::new(TokenSet::new());
    let mut source = String::new();
    let mut element: Option<Element> = None;
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "css" => {
                source.push_str(argument);
                source.push('\n');
                Ok(Vec::new())
            }
            "element" => match Element::parse(argument) {
                Ok(parsed) => {
                    let line = format!("element: {}", parsed.render());
                    element = Some(parsed);
                    Ok(vec![line])
                }
                Err(reason) => Err(reason),
            },
            "token" => match parse_design_token(argument) {
                Ok((kind, name, value)) => {
                    session.define_token(kind, name.clone(), value);
                    Ok(vec![format!("token {name} ({})", kind.name())])
                }
                Err(reason) => Err(reason),
            },
            "theme" => match parse_theme(argument) {
                Ok((name, values)) => {
                    session.define_theme(name.clone(), &values);
                    Ok(vec![format!("theme {name}: {} override(s)", values.len())])
                }
                Err(reason) => Err(reason),
            },
            "inline" => match argument.split_once(':') {
                Some((property, value)) => {
                    session.set_inline(property.to_owned(), value.to_owned());
                    Ok(vec![format!("inline {property}: {value}")])
                }
                None => Err(format!("inline needs <property>:<value>, got {argument:?}")),
            },
            "clear" => {
                session.clear_inline(argument);
                Ok(vec![format!("clear: {argument}")])
            }
            "rename" => match argument.split_once(':') {
                Some((from, to)) => match session.rename_token(from, to) {
                    Ok(()) => Ok(vec![format!("renamed {from} -> {to}")]),
                    Err(reason) => Err(reason),
                },
                None => Err(format!("rename needs <from>:<to>, got {argument:?}")),
            },
            "use-theme" => {
                let theme = if argument == "-" || argument.is_empty() {
                    None
                } else {
                    Some(argument)
                };
                match session.switch_theme(theme) {
                    Ok(()) => Ok(vec![format!("theme: {}", theme.unwrap_or("(no theme)"))]),
                    Err(reason) => Err(reason),
                }
            }
            "undo" => {
                if session.undo() {
                    Ok(vec!["undone".to_owned()])
                } else {
                    Ok(vec!["nothing to undo".to_owned()])
                }
            }
            "emit" => match &element {
                Some(element) => {
                    let css = Css::parse(&source);
                    match session.emit(&css, element) {
                        Ok(style) => Ok(style.render(Mode::Preview)),
                        Err(diagnostics) => {
                            for diagnostic in &diagnostics {
                                println!("{}", diagnostic.render());
                            }
                            return report(format!(
                                "{} declaration(s) could not be emitted",
                                diagnostics.len()
                            ));
                        }
                    }
                }
                None => Err("no element is selected".to_owned()),
            },
            "contrast" => match argument.split_once(':') {
                Some((a, b)) => match contrast(a, b) {
                    Some(ratio) => Ok(vec![
                        format!("{a} on {b}: {ratio:.2}:1"),
                        format!(
                            "  AA {}  AAA {}",
                            if meets(ratio, Level::Aa) {
                                "pass"
                            } else {
                                "fail"
                            },
                            if meets(ratio, Level::Aaa) {
                                "pass"
                            } else {
                                "fail"
                            }
                        ),
                    ]),
                    None => Err(format!("{a:?} or {b:?} is not a #rgb or #rrggbb colour")),
                },
                None => Err(format!(
                    "contrast needs <colour>:<colour>, got {argument:?}"
                )),
            },
            "state" => Ok(session.render()),
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a style-boundary operation")),
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

/// The selection and any resets in a report.
pub(super) fn render_multi(
    nodes: &[crate::app::style::multi_property::NodeProperties],
    resets: &[String],
) -> Vec<String> {
    let mut lines = vec![format!("nodes: {}", nodes.len())];
    for node in nodes {
        lines.push(format!(
            "  node {}: {} property(ies)",
            node.node.get(),
            node.settings.len()
        ));
    }
    lines.push(format!("resets: {}", resets.len()));
    lines
}

/// A `(property, value, origin)` from `<property>:<value>:<origin>`.
pub(super) fn parse_setting(
    argument: &str,
) -> Result<
    (
        crate::contracts::model::PropertyName,
        crate::contracts::model::PropertyValue,
        crate::app::layout::layout_plan::Origin,
    ),
    String,
> {
    use crate::app::layout::layout_plan::Origin;
    use crate::contracts::model::{PropertyName, PropertyValue};

    let fields: Vec<&str> = argument.splitn(3, ':').collect();
    let [property, value, origin] = fields.as_slice() else {
        return Err(format!("{argument:?} is not <property>:<value>:<origin>"));
    };
    let property = PropertyName::new(property)
        .ok_or_else(|| format!("{property:?} is not a property name"))?;
    let value = if let Ok(number) = value.parse::<f64>() {
        PropertyValue::Number(number)
    } else if let Ok(flag) = value.parse::<bool>() {
        PropertyValue::Bool(flag)
    } else {
        PropertyValue::Text((*value).to_owned())
    };
    let origin = Origin::from_name(origin).ok_or_else(|| format!("{origin:?} is not an origin"))?;
    Ok((property, value, origin))
}

/// An `(origin, fallback)` from `<origin>[:<fallback>]`.
pub(super) fn parse_reset(
    argument: &str,
) -> Result<
    (
        crate::app::layout::layout_plan::Origin,
        Option<crate::contracts::model::PropertyValue>,
    ),
    String,
> {
    use crate::app::layout::layout_plan::Origin;
    use crate::contracts::model::PropertyValue;

    let (origin, fallback) = match argument.split_once(':') {
        Some((origin, fallback)) => (origin, Some(fallback)),
        None => (argument, None),
    };
    let origin = Origin::from_name(origin).ok_or_else(|| format!("{origin:?} is not an origin"))?;
    let fallback = fallback.map(|text| {
        if let Ok(number) = text.parse::<f64>() {
            PropertyValue::Number(number)
        } else {
            PropertyValue::Text(text.to_owned())
        }
    });
    Ok((origin, fallback))
}
