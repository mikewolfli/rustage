// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! F10: layout, constraints and the focus order.
//!
//! One command handler per parsed command of this domain; the dispatcher lives in
//! [`super`](self).

use super::*;

/// Exercises the layout contract and its diagnostics through the library engine (task T10.01).
pub(super) fn layout_plan(options: &crate::cli::LayoutPlanOptions) -> bool {
    use crate::app::layout::layout_choice::LayoutKind;
    use crate::app::layout::layout_plan::{
        Computed, LayoutInput, LibraryEngine, Role, Size, layout,
    };

    let mut kind = LayoutKind::Row;
    let mut container: Option<Size> = None;
    let mut gap = 0u32;
    let mut padding = 0u32;
    let mut inputs: Vec<LayoutInput> = Vec::new();
    let mut caller_boxes: Vec<Computed> = Vec::new();
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "available" => match argument.split_once(':') {
                Some((node, size)) => match (parse_scene_node(node), parse_floats(size).as_deref())
                {
                    (Ok(node), Ok([width, height])) => {
                        inputs.retain(|input| input.node != node);
                        inputs.push(LayoutInput::new(node, Size::new(*width, *height)));
                        Ok(vec![format!("available {}: {width},{height}", node.get())])
                    }
                    _ => Err(format!("{argument:?} is not <node>:<w>,<h>")),
                },
                None => Err(format!("{argument:?} is not <node>:<w>,<h>")),
            },
            "constraint" => match parse_constraint(argument) {
                Ok(constraint) => {
                    let node = constraint.node;
                    match inputs.iter_mut().find(|input| input.node == node) {
                        Some(input) => {
                            input.constraints.push(constraint);
                            Ok(vec![format!("constraint on node {}", node.get())])
                        }
                        None => Err(format!("no space was offered to node {}", node.get())),
                    }
                }
                Err(reason) => Err(reason),
            },
            "layout" => match LayoutKind::from_name(argument) {
                Some(selected) => {
                    kind = selected;
                    let engine = LibraryEngine::new(selected);
                    Ok(vec![format!(
                        "layout: {} ({})",
                        selected.name(),
                        engine.selection().name()
                    )])
                }
                None => Err(format!("{argument:?} is not a layout kind")),
            },
            "container" => match parse_floats(argument).as_deref() {
                Ok([width, height]) => {
                    container = Some(Size::new(*width, *height));
                    Ok(vec![format!("container: {width},{height}")])
                }
                _ => Err(format!("{argument:?} is not <w>,<h>")),
            },
            "gap" | "padding" => match argument.parse::<u32>() {
                Ok(value) => {
                    if verb == "gap" {
                        gap = value;
                    } else {
                        padding = value;
                    }
                    Ok(vec![format!("{verb}: {value}")])
                }
                Err(_) => Err(format!("{argument:?} is not a non-negative pixel count")),
            },
            // The boxes a caller used to hand the stand-in are kept only so an old
            // command still parses; the plan is the library's, and this list is not it.
            "box" => match parse_box(argument) {
                Ok(box_) => {
                    caller_boxes.retain(|existing| existing.node != box_.node);
                    caller_boxes.push(box_);
                    Ok(vec![format!(
                        "box: caller-supplied boxes are not used by the library engine (kept {})",
                        box_.render()
                    )])
                }
                Err(reason) => Err(reason),
            },
            "run" => match Role::from_name(argument) {
                Some(role) => {
                    let mut engine = LibraryEngine::new(kind).with_gap(gap).with_padding(padding);
                    if let Some(container) = container {
                        engine = engine.with_container(container);
                    }
                    let plan = layout(&engine, role, &inputs);
                    let mut lines = vec![format!("engine: {}", engine.selection().name())];
                    lines.extend(plan.render());
                    Ok(lines)
                }
                None => Err(format!("{argument:?} is not preview or generation")),
            },
            "source" => match parse_scene_node(argument) {
                Ok(node) => match inputs.iter().find(|input| input.node == node) {
                    Some(input) => {
                        let mut lines = vec![format!(
                            "source node {}: {} constraint(s)",
                            node.get(),
                            input.constraints.len()
                        )];
                        lines.extend(
                            input
                                .constraints
                                .iter()
                                .map(|constraint| format!("  {}", constraint.source().render())),
                        );
                        Ok(lines)
                    }
                    None => Err(format!("no space was offered to node {}", node.get())),
                },
                Err(reason) => Err(reason),
            },
            "state" => Ok(render_layout_state(&inputs, kind, &caller_boxes)),
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a layout-plan operation")),
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
    for line in render_layout_state(&inputs, kind, &caller_boxes) {
        println!("{line}");
    }
    true
}

/// The inputs, the selected library layout and any caller-supplied boxes in a report.
pub(super) fn render_layout_state(
    inputs: &[crate::app::layout::layout_plan::LayoutInput],
    kind: crate::app::layout::layout_choice::LayoutKind,
    caller_boxes: &[crate::app::layout::layout_plan::Computed],
) -> Vec<String> {
    let selection = crate::app::layout::layout_plan::LibraryEngine::new(kind).selection();
    let mut lines = vec![format!("inputs: {}", inputs.len())];
    for input in inputs {
        lines.push(format!(
            "  node {} available {},{} ({} constraint(s))",
            input.node.get(),
            input.available.width,
            input.available.height,
            input.constraints.len()
        ));
        for constraint in &input.constraints {
            let value = constraint
                .value
                .as_ref()
                .map_or_else(|| "(no value)".to_owned(), |value| value.render());
            lines.push(format!("    {} = {value}", constraint.property.as_str()));
        }
    }
    lines.push(format!("layout: {} ({})", kind.name(), selection.name()));
    lines.push(format!(
        "caller boxes: {} (not used by the library engine)",
        caller_boxes.len()
    ));
    for box_ in caller_boxes {
        lines.push(format!("  {}", box_.render()));
    }
    lines
}

/// A constraint from `<node>:<property>:<origin>[:<value>]`.
pub(super) fn parse_constraint(
    argument: &str,
) -> Result<crate::app::layout::layout_plan::Constraint, String> {
    let fields: Vec<&str> = argument.splitn(4, ':').collect();
    let (node, property, origin, value) = match fields.as_slice() {
        [node, property, origin] => (*node, *property, *origin, None),
        [node, property, origin, value] => (*node, *property, *origin, Some(*value)),
        _ => {
            return Err(format!(
                "{argument:?} is not <node>:<property>:<origin>[:<value>]"
            ));
        }
    };
    let node = parse_scene_node(node)?;
    let property = crate::contracts::model::PropertyName::new(property)
        .ok_or_else(|| format!("{property:?} is not a property name"))?;
    let origin = crate::app::layout::layout_plan::Origin::from_name(origin)
        .ok_or_else(|| format!("{origin:?} is not an origin"))?;
    let constraint = crate::app::layout::layout_plan::Constraint::new(node, property, origin);
    match value {
        Some(value) => {
            let typed = parse_constraint_value(constraint.property.as_str(), value)?;
            Ok(constraint.with_value(typed))
        }
        None => Ok(constraint),
    }
}

/// The typed value a constraint property carries, from its written form.
pub(super) fn parse_constraint_value(
    property: &str,
    raw: &str,
) -> Result<crate::app::layout::layout_plan::Value, String> {
    use crate::app::layout::layout_plan::Value;
    match property {
        "min" | "max" | "preferred" => match parse_floats(raw).as_deref() {
            Ok([width, height]) => Ok(Value::Size {
                width: *width,
                height: *height,
            }),
            _ => Err(format!("{raw:?} is not <width>,<height>")),
        },
        "cell" => match raw.split_once(',') {
            Some((row, column)) => {
                let row = row
                    .parse::<u32>()
                    .map_err(|_| format!("{row:?} is not a row"))?;
                let column = column
                    .parse::<u32>()
                    .map_err(|_| format!("{column:?} is not a column"))?;
                Ok(Value::Cell { row, column })
            }
            None => Err(format!("{raw:?} is not <row>,<column>")),
        },
        "align" | "overflow" => Ok(Value::Word(raw.to_owned())),
        "width" | "height" | "gap" | "margin" | "padding" | "grow" | "x" | "y" | "order" => raw
            .parse::<f64>()
            .map(Value::Scalar)
            .map_err(|_| format!("{raw:?} is not a number")),
        other => Err(format!("{other:?} has no value form")),
    }
}

/// A computed box from `<node>:<x>,<y>,<w>,<h>`.
pub(super) fn parse_box(
    argument: &str,
) -> Result<crate::app::layout::layout_plan::Computed, String> {
    let (node, rect) = argument
        .split_once(':')
        .ok_or_else(|| format!("{argument:?} is not <node>:<x>,<y>,<w>,<h>"))?;
    let node = parse_scene_node(node)?;
    match parse_floats(rect).as_deref() {
        Ok([x, y, width, height]) => Ok(crate::app::layout::layout_plan::Computed {
            node,
            x: *x,
            y: *y,
            width: *width,
            height: *height,
        }),
        _ => Err(format!("{rect:?} is not <x>,<y>,<w>,<h>")),
    }
}

/// Chooses, switches and detaches a container's layout (task T10.02).
pub(super) fn layout_switch(options: &crate::cli::LayoutSwitchOptions) -> bool {
    use crate::app::layout::layout_choice::{LayoutKind, detach, switch};

    let mut kind: Option<LayoutKind> = None;
    let mut children: Vec<crate::app::layout::layout_plan::Constraint> = Vec::new();
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "layout" => match LayoutKind::from_name(argument) {
                Some(layout) => {
                    kind = Some(layout);
                    Ok(vec![format!("layout: {}", layout.name())])
                }
                None => Err(format!("{argument:?} is not a layout kind")),
            },
            "child" => match parse_child_constraint(argument) {
                Ok(constraint) => {
                    children.push(constraint);
                    Ok(vec![format!("child: {argument}")])
                }
                Err(reason) => Err(reason),
            },
            "switch" => match kind {
                Some(to) => match switch(to, &children) {
                    Ok(migration) => Ok(migration.render()),
                    Err(refusal) => Ok(refusal.render()),
                },
                None => Err("no layout is chosen".to_owned()),
            },
            "detach" => match detach(&children) {
                Ok(migration) => Ok(migration.render()),
                Err(refusal) => Ok(refusal.render()),
            },
            "state" => Ok(render_layout_choice(kind, &children)),
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a layout-switch operation")),
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
    for line in render_layout_choice(kind, &children) {
        println!("{line}");
    }
    true
}

/// The chosen layout and the children in a report.
pub(super) fn render_layout_choice(
    kind: Option<crate::app::layout::layout_choice::LayoutKind>,
    children: &[crate::app::layout::layout_plan::Constraint],
) -> Vec<String> {
    let mut lines = vec![format!(
        "layout: {}",
        kind.map_or("none", crate::app::layout::layout_choice::LayoutKind::name)
    )];
    lines.push(format!("children: {}", children.len()));
    for constraint in children {
        lines.push(format!(
            "  node {} {}",
            constraint.node.get(),
            constraint.property.as_str()
        ));
    }
    lines
}

/// A child constraint from `<node>:<property>`.
pub(super) fn parse_child_constraint(
    argument: &str,
) -> Result<crate::app::layout::layout_plan::Constraint, String> {
    let (node, property) = argument
        .split_once(':')
        .ok_or_else(|| format!("{argument:?} is not <node>:<property>"))?;
    let node = parse_scene_node(node)?;
    let property = crate::contracts::model::PropertyName::new(property)
        .ok_or_else(|| format!("{property:?} is not a property name"))?;
    Ok(crate::app::layout::layout_plan::Constraint::new(
        node,
        property,
        crate::app::layout::layout_plan::Origin::Explicit,
    ))
}

/// Sets and validates a node's box constraints (task T10.03).
pub(super) fn constraints(options: &crate::cli::ConstraintsOptions) -> bool {
    use crate::app::layout::constraints::{BoxConstraints, Overflow};
    use crate::contracts::identity::NodeId;
    use std::collections::BTreeMap;

    let mut sets: BTreeMap<NodeId, BoxConstraints> = BTreeMap::new();
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "min" | "max" | "preferred" => match parse_node_size(argument) {
                Ok((node, width, height)) => {
                    let set = sets
                        .entry(node)
                        .or_insert_with(|| BoxConstraints::new(node));
                    let size = crate::app::layout::layout_plan::Size::new(width, height);
                    match verb {
                        "min" => set.minimum = Some(size),
                        "max" => set.maximum = Some(size),
                        _ => set.preferred = Some(size),
                    }
                    Ok(vec![format!("{verb} {}: {width},{height}", node.get())])
                }
                Err(reason) => Err(reason),
            },
            "grow" | "margin" | "gap" => match parse_node_scalar(argument) {
                Ok((node, value)) => {
                    let set = sets
                        .entry(node)
                        .or_insert_with(|| BoxConstraints::new(node));
                    match verb {
                        "grow" => set.grow = Some(value),
                        "margin" => set.margin = Some(value),
                        _ => set.gap = Some(value),
                    }
                    Ok(vec![format!("{verb} {}: {value}", node.get())])
                }
                Err(reason) => Err(reason),
            },
            "overflow" => match argument.split_once(':') {
                Some((node, kind)) => match (parse_scene_node(node), Overflow::from_name(kind)) {
                    (Ok(node), Some(overflow)) => {
                        sets.entry(node)
                            .or_insert_with(|| BoxConstraints::new(node))
                            .overflow = Some(overflow);
                        Ok(vec![format!("overflow {}: {kind}", node.get())])
                    }
                    _ => Err(format!("{argument:?} is not <node>:<kind>")),
                },
                None => Err(format!("{argument:?} is not <node>:<kind>")),
            },
            "validate" => {
                let mut lines = Vec::new();
                let mut problems = 0usize;
                for set in sets.values() {
                    for diagnostic in set.validate() {
                        problems += 1;
                        lines.push(diagnostic.render());
                    }
                }
                let mut rendered = vec![format!("validate: {problems} problem(s)")];
                rendered.extend(lines);
                Ok(rendered)
            }
            "state" => Ok(render_constraints(&sets)),
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a constraints operation")),
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
    for line in render_constraints(&sets) {
        println!("{line}");
    }
    true
}

/// The constraint sets in a report.
pub(super) fn render_constraints(
    sets: &std::collections::BTreeMap<
        crate::contracts::identity::NodeId,
        crate::app::layout::constraints::BoxConstraints,
    >,
) -> Vec<String> {
    let mut lines = vec![format!("constraints: {} node(s)", sets.len())];
    for set in sets.values() {
        lines.extend(set.render());
    }
    lines
}

/// A `(node, width, height)` from `<node>:<w>,<h>`.
pub(super) fn parse_node_size(
    argument: &str,
) -> Result<(crate::contracts::identity::NodeId, f64, f64), String> {
    let (node, size) = argument
        .split_once(':')
        .ok_or_else(|| format!("{argument:?} is not <node>:<w>,<h>"))?;
    let node = parse_scene_node(node)?;
    match parse_floats(size).as_deref() {
        Ok([width, height]) => Ok((node, *width, *height)),
        _ => Err(format!("{size:?} is not <w>,<h>")),
    }
}

/// A `(node, value)` from `<node>:<value>`.
pub(super) fn parse_node_scalar(
    argument: &str,
) -> Result<(crate::contracts::identity::NodeId, f64), String> {
    let (node, value) = argument
        .split_once(':')
        .ok_or_else(|| format!("{argument:?} is not <node>:<value>"))?;
    let node = parse_scene_node(node)?;
    let value = value
        .parse::<f64>()
        .map_err(|_| format!("{value:?} is not a number"))?;
    Ok((node, value))
}

/// Aligns, distributes and nudges a selection as constraint writes (task T10.04).
pub(super) fn arrange(options: &crate::cli::ArrangeOptions) -> bool {
    use crate::app::layout::arrange::{Axis, Document, Operation};
    use crate::app::layout::layout_choice::LayoutKind;
    use crate::contracts::identity::NodeId;

    let mut kind = LayoutKind::Row;
    let mut selected: Vec<NodeId> = Vec::new();
    let mut document = Document::new();
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "layout" => match LayoutKind::from_name(argument) {
                Some(layout) => {
                    kind = layout;
                    Ok(vec![format!("layout: {}", layout.name())])
                }
                None => Err(format!("{argument:?} is not a layout kind")),
            },
            "select" => match parse_scene_node(argument) {
                Ok(node) => {
                    if !selected.contains(&node) {
                        selected.push(node);
                    }
                    Ok(vec![format!("select: {}", node.get())])
                }
                Err(reason) => Err(reason),
            },
            // The box and container are the geometry an alignment, distribution or size match
            // reads. Without them the operation is refused rather than guessed.
            "frame" => match parse_box(argument) {
                Ok(box_) => {
                    document = document.with_bounds(box_);
                    Ok(vec![format!("frame: {argument}")])
                }
                Err(reason) => Err(reason),
            },
            "container" => match parse_floats(argument).as_deref() {
                Ok([width, height]) => {
                    document = document.with_container(crate::app::layout::layout_plan::Size::new(
                        *width, *height,
                    ));
                    Ok(vec![format!("container: {width},{height}")])
                }
                _ => Err(format!("{argument:?} is not <w>,<h>")),
            },
            "align" => match parse_axis_mode(argument) {
                Ok((axis, mode)) => Ok(run_arrange(
                    kind,
                    &mut document,
                    &selected,
                    Operation::Align { axis, mode },
                )),
                Err(reason) => Err(reason),
            },
            "distribute" => match Axis::from_name(argument) {
                Some(axis) => Ok(run_arrange(
                    kind,
                    &mut document,
                    &selected,
                    Operation::Distribute { axis },
                )),
                None => Err(format!("{argument:?} is not an axis")),
            },
            "match" => match Axis::from_name(argument) {
                Some(axis) => Ok(run_arrange(
                    kind,
                    &mut document,
                    &selected,
                    Operation::MatchSize { axis },
                )),
                None => Err(format!("{argument:?} is not an axis")),
            },
            "nudge" => match parse_nudge(argument) {
                Ok(operation) => Ok(run_arrange(kind, &mut document, &selected, operation)),
                Err(reason) => Err(reason),
            },
            "state" => Ok(render_arrange(kind, &selected, &document)),
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not an arrange operation")),
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
    for line in render_arrange(kind, &selected, &document) {
        println!("{line}");
    }
    true
}

/// Runs one arrangement against the document, keeping the new document on success.
pub(super) fn run_arrange(
    kind: crate::app::layout::layout_choice::LayoutKind,
    document: &mut crate::app::layout::arrange::Document,
    selected: &[crate::contracts::identity::NodeId],
    operation: crate::app::layout::arrange::Operation,
) -> Vec<String> {
    match crate::app::layout::arrange::apply_to(kind, document, selected, operation) {
        Ok(arrangement) => {
            let lines = arrangement.render();
            *document = arrangement.document;
            lines
        }
        Err(refusal) => refusal.render(),
    }
}

/// The layout, the selection and the constraint document in a report.
pub(super) fn render_arrange(
    kind: crate::app::layout::layout_choice::LayoutKind,
    selected: &[crate::contracts::identity::NodeId],
    document: &crate::app::layout::arrange::Document,
) -> Vec<String> {
    let ids: Vec<String> = selected.iter().map(|node| node.get().to_string()).collect();
    let mut lines = vec![
        format!("layout: {}", kind.name()),
        format!("selected: {}", ids.join(", ")),
    ];
    lines.extend(document.render());
    lines
}

/// An `(axis, mode)` from `<axis>:<mode>`.
pub(super) fn parse_axis_mode(
    argument: &str,
) -> Result<
    (
        crate::app::layout::arrange::Axis,
        crate::app::layout::arrange::Align,
    ),
    String,
> {
    let (axis, mode) = argument
        .split_once(':')
        .ok_or_else(|| format!("{argument:?} is not <axis>:<mode>"))?;
    let axis = crate::app::layout::arrange::Axis::from_name(axis)
        .ok_or_else(|| format!("{axis:?} is not an axis"))?;
    let mode = crate::app::layout::arrange::Align::from_name(mode)
        .ok_or_else(|| format!("{mode:?} is not an alignment"))?;
    Ok((axis, mode))
}

/// A nudge operation from `<axis>:<direction>:<step>[:<spacing>]`, snapping the step to the grid.
pub(super) fn parse_nudge(
    argument: &str,
) -> Result<crate::app::layout::arrange::Operation, String> {
    let fields: Vec<&str> = argument.split(':').collect();
    let (axis, direction, step, spacing) = match fields.as_slice() {
        [axis, direction, step] => (*axis, *direction, *step, None),
        [axis, direction, step, spacing] => (*axis, *direction, *step, Some(*spacing)),
        _ => {
            return Err(format!(
                "{argument:?} is not <axis>:<direction>:<step>[:<spacing>]"
            ));
        }
    };
    let axis = crate::app::layout::arrange::Axis::from_name(axis)
        .ok_or_else(|| format!("{axis:?} is not an axis"))?;
    let direction = direction
        .parse::<i8>()
        .map_err(|_| format!("{direction:?} is not a direction"))?;
    let step = step
        .parse::<f64>()
        .map_err(|_| format!("{step:?} is not a step"))?;
    let step = match spacing {
        Some(spacing) => {
            let spacing = spacing
                .parse::<f64>()
                .map_err(|_| format!("{spacing:?} is not a spacing"))?;
            crate::app::layout::arrange::snapped_step(step, spacing)
        }
        None => step,
    };
    Ok(crate::app::layout::arrange::Operation::Nudge {
        axis,
        direction,
        step,
    })
}

/// Checks simulated frames against a target's support (task T10.05).
pub(super) fn frames(options: &crate::cli::FramesOptions) -> bool {
    use crate::app::layout::frames::{Frame, Insets, Orientation, Support, check};
    use crate::app::layout::layout_plan::Size;
    use std::collections::BTreeMap;

    let mut support = Support::new();
    let mut frames: BTreeMap<String, Frame> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "support-size" => match parse_floats(argument).as_deref() {
                Ok([width, height]) => {
                    support.sizes.push(Size::new(*width, *height));
                    Ok(vec![format!("support-size: {width},{height}")])
                }
                _ => Err(format!("{argument:?} is not <w>,<h>")),
            },
            "support-orientation" => match Orientation::from_name(argument) {
                Some(orientation) => {
                    support.orientations.push(orientation);
                    Ok(vec![format!("support-orientation: {}", orientation.name())])
                }
                None => Err(format!("{argument:?} is not an orientation")),
            },
            "support-safe-area" => match argument {
                "true" => {
                    support.honours_safe_area = true;
                    Ok(vec!["support-safe-area: true".to_owned()])
                }
                "false" => {
                    support.honours_safe_area = false;
                    Ok(vec!["support-safe-area: false".to_owned()])
                }
                other => Err(format!("{other:?} is not true or false")),
            },
            "support-max-scale" => match argument.parse::<f64>() {
                Ok(scale) => {
                    support.max_scale = scale;
                    Ok(vec![format!("support-max-scale: {scale}")])
                }
                Err(_) => Err(format!("{argument:?} is not a scale")),
            },
            "frame" => match parse_frame(argument) {
                Ok(frame) => {
                    if !order.contains(&frame.label) {
                        order.push(frame.label.clone());
                    }
                    frames.insert(frame.label.clone(), frame);
                    Ok(vec![format!("frame: {argument}")])
                }
                Err(reason) => Err(reason),
            },
            "frame-scale" => match argument.split_once(':') {
                Some((label, scale)) => match scale.parse::<f64>() {
                    Ok(scale) => match frames.get_mut(label) {
                        Some(frame) => {
                            frame.scale = scale;
                            Ok(vec![format!("frame-scale {label}: {scale}")])
                        }
                        None => Err(format!("frame {label:?} is not defined")),
                    },
                    Err(_) => Err(format!("{scale:?} is not a scale")),
                },
                None => Err(format!("{argument:?} is not <label>:<scale>")),
            },
            "frame-insets" => match argument.split_once(':') {
                Some((label, insets)) => match parse_floats(insets).as_deref() {
                    Ok([top, right, bottom, left]) => match frames.get_mut(label) {
                        Some(frame) => {
                            frame.insets = Insets::new(*top, *right, *bottom, *left);
                            Ok(vec![format!("frame-insets {label}: {insets}")])
                        }
                        None => Err(format!("frame {label:?} is not defined")),
                    },
                    _ => Err(format!("{insets:?} is not <t>,<r>,<b>,<l>")),
                },
                None => Err(format!("{argument:?} is not <label>:<insets>")),
            },
            "check" => {
                let list: Vec<Frame> = order
                    .iter()
                    .filter_map(|label| frames.get(label).cloned())
                    .collect();
                let found = check(&list, &support);
                let blockers = found.iter().filter(|d| d.blocks_generation()).count();
                let mut lines = vec![format!(
                    "check: {} difference(s), {blockers} blocker(s)",
                    found.len()
                )];
                lines.extend(
                    found
                        .iter()
                        .map(|difference| format!("  {}", difference.render())),
                );
                Ok(lines)
            }
            "state" => Ok(render_frames(&support, &frames, &order)),
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a frames operation")),
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
    let list: Vec<Frame> = order
        .iter()
        .filter_map(|label| frames.get(label).cloned())
        .collect();
    for line in render_frames(&support, &frames, &order) {
        println!("{line}");
    }
    for line in check(&list, &support) {
        println!("  {}", line.render());
    }
    true
}

/// The support and the frames in a report.
pub(super) fn render_frames(
    support: &crate::app::layout::frames::Support,
    frames: &std::collections::BTreeMap<String, crate::app::layout::frames::Frame>,
    order: &[String],
) -> Vec<String> {
    let mut lines = vec![format!(
        "support: {} size(s), {} orientation(s), safe area {}, max scale {}",
        support.sizes.len(),
        support.orientations.len(),
        support.honours_safe_area,
        support.max_scale
    )];
    for label in order {
        if let Some(frame) = frames.get(label) {
            lines.push(format!(
                "  frame {}: {}x{} {} scale {}",
                frame.label,
                frame.size.width,
                frame.size.height,
                frame.orientation.name(),
                frame.scale
            ));
        }
    }
    lines
}

/// A frame from `<label>:<w>,<h>:<orientation>`.
pub(super) fn parse_frame(argument: &str) -> Result<crate::app::layout::frames::Frame, String> {
    let fields: Vec<&str> = argument.splitn(3, ':').collect();
    let [label, size, orientation] = fields.as_slice() else {
        return Err(format!("{argument:?} is not <label>:<w>,<h>:<orientation>"));
    };
    let orientation = crate::app::layout::frames::Orientation::from_name(orientation)
        .ok_or_else(|| format!("{orientation:?} is not an orientation"))?;
    match parse_floats(size).as_deref() {
        Ok([width, height]) => Ok(crate::app::layout::frames::Frame::new(
            *label,
            crate::app::layout::layout_plan::Size::new(*width, *height),
            orientation,
        )),
        _ => Err(format!("{size:?} is not <w>,<h>")),
    }
}

/// Builds and inspects the focus order (task T10.06).
pub(super) fn focus_order(options: &crate::cli::FocusOrderOptions) -> bool {
    use crate::app::canvas::structure::Tree;
    use crate::app::layout::focus_order::build;
    use crate::contracts::identity::NodeId;
    use std::collections::BTreeMap;

    let mut tree = Tree::new();
    let mut focusable: Vec<NodeId> = Vec::new();
    let mut excluded: Vec<NodeId> = Vec::new();
    let mut explicit: BTreeMap<NodeId, i64> = BTreeMap::new();
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "root" => match argument.split_once(':') {
                Some((kind, name)) => match parse_kind(kind) {
                    Ok(kind) => match tree.insert(None, kind, name) {
                        Ok(node) => Ok(vec![format!("root {name}: node {}", node.get())]),
                        Err(error) => Err(error.message()),
                    },
                    Err(reason) => Err(reason),
                },
                None => Err(format!("{argument:?} is not <kind>:<name>")),
            },
            "insert" => match parse_insert(argument) {
                Ok((parent, kind, name)) => match tree.insert(Some(parent), kind, name.clone()) {
                    Ok(node) => Ok(vec![format!("insert {name}: node {}", node.get())]),
                    Err(error) => Err(error.message()),
                },
                Err(reason) => Err(reason),
            },
            "focusable" => match parse_scene_node(argument) {
                Ok(node) => {
                    if !focusable.contains(&node) {
                        focusable.push(node);
                    }
                    Ok(vec![format!("focusable: {}", node.get())])
                }
                Err(reason) => Err(reason),
            },
            "exclude" => match parse_scene_node(argument) {
                Ok(node) => {
                    if !excluded.contains(&node) {
                        excluded.push(node);
                    }
                    Ok(vec![format!("exclude: {}", node.get())])
                }
                Err(reason) => Err(reason),
            },
            "index" => match argument.split_once(':') {
                Some((node, value)) => match (parse_scene_node(node), value.parse::<i64>()) {
                    (Ok(node), Ok(value)) => {
                        explicit.insert(node, value);
                        Ok(vec![format!("index {}: {value}", node.get())])
                    }
                    _ => Err(format!("{argument:?} is not <node>:<n>")),
                },
                None => Err(format!("{argument:?} is not <node>:<n>")),
            },
            "build" => Ok(build(&tree, &focusable, &excluded, &explicit).render()),
            "step" => match argument.split_once(':') {
                Some((node, delta)) => match (parse_scene_node(node), delta.parse::<i64>()) {
                    (Ok(node), Ok(delta)) => {
                        let order = build(&tree, &focusable, &excluded, &explicit);
                        let next = if delta < 0 {
                            order.previous(node)
                        } else {
                            order.next(node)
                        };
                        Ok(vec![match next {
                            Some(next) => {
                                format!("step {} by {delta}: node {}", node.get(), next.get())
                            }
                            None => format!("step {} by {delta}: nowhere", node.get()),
                        }])
                    }
                    _ => Err(format!("{argument:?} is not <node>:<delta>")),
                },
                None => Err(format!("{argument:?} is not <node>:<delta>")),
            },
            "export" => {
                let ids: Vec<String> = build(&tree, &focusable, &excluded, &explicit)
                    .export()
                    .iter()
                    .map(|node| node.get().to_string())
                    .collect();
                Ok(vec![format!("export: {}", ids.join(" -> "))])
            }
            "state" => Ok(build(&tree, &focusable, &excluded, &explicit).render()),
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a focus-order operation")),
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
    for line in build(&tree, &focusable, &excluded, &explicit).render() {
        println!("{line}");
    }
    true
}

/// Declares a target's capability and builds layout variants (task T10.07).
pub(super) fn variants(options: &crate::cli::VariantsOptions) -> bool {
    use crate::app::layout::variants::{Capability, ConditionKind, Variant, add};

    let mut capability = Capability::new();
    let mut variants: Vec<Variant> = Vec::new();
    let mut current: Option<Variant> = None;
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "capability" => match ConditionKind::from_name(argument) {
                Some(kind) => {
                    capability = capability.supporting(kind);
                    Ok(vec![format!("capability: {}", kind.name())])
                }
                None => Err(format!("{argument:?} is not a condition kind")),
            },
            "variant" => {
                current = Some(Variant::new(argument));
                Ok(vec![format!("variant: {argument}")])
            }
            "when" => match parse_condition(argument) {
                Ok(condition) => match current.as_mut() {
                    Some(variant) => {
                        variant.conditions.push(condition);
                        Ok(vec![format!("when: {}", argument)])
                    }
                    None => Err("no variant is being built".to_owned()),
                },
                Err(reason) => Err(reason),
            },
            "value" => match argument.split_once('=') {
                Some((key, value)) => match current.as_mut() {
                    Some(variant) => {
                        variant.values.insert(key.to_owned(), value.to_owned());
                        Ok(vec![format!("value: {argument}")])
                    }
                    None => Err("no variant is being built".to_owned()),
                },
                None => Err(format!("{argument:?} is not <key>=<value>")),
            },
            "add" => match current.take() {
                Some(variant) => {
                    let name = variant.name.clone();
                    match add(&capability, &mut variants, variant) {
                        Ok(()) => Ok(vec![format!("add: variant {name}")]),
                        Err(block) => Ok(block.render()),
                    }
                }
                None => Err("no variant is being built".to_owned()),
            },
            "state" => Ok(render_variants(&capability, &variants)),
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a variants operation")),
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
    for line in render_variants(&capability, &variants) {
        println!("{line}");
    }
    true
}

/// The capability and the variants in a report.
pub(super) fn render_variants(
    capability: &crate::app::layout::variants::Capability,
    variants: &[crate::app::layout::variants::Variant],
) -> Vec<String> {
    let kinds: Vec<&str> = capability
        .conditions
        .iter()
        .map(|kind| kind.name())
        .collect();
    let mut lines = vec![format!("capability: {}", kinds.join(", "))];
    lines.push(format!("variants: {}", variants.len()));
    for variant in variants {
        lines.extend(variant.render());
    }
    lines
}

/// A condition from `<kind>=<value>`.
pub(super) fn parse_condition(
    argument: &str,
) -> Result<crate::app::layout::variants::Condition, String> {
    use crate::app::layout::variants::Condition;

    let (kind, value) = argument
        .split_once('=')
        .ok_or_else(|| format!("{argument:?} is not <kind>=<value>"))?;
    let kind = crate::app::layout::variants::ConditionKind::from_name(kind)
        .ok_or_else(|| format!("{kind:?} is not a condition kind"))?;
    match kind {
        crate::app::layout::variants::ConditionKind::Orientation => {
            let orientation = crate::app::layout::frames::Orientation::from_name(value)
                .ok_or_else(|| format!("{value:?} is not an orientation"))?;
            Ok(Condition::Orientation(orientation))
        }
        crate::app::layout::variants::ConditionKind::MinWidth => {
            Ok(Condition::MinWidth(parse_number(value)?))
        }
        crate::app::layout::variants::ConditionKind::MaxWidth => {
            Ok(Condition::MaxWidth(parse_number(value)?))
        }
        crate::app::layout::variants::ConditionKind::MinHeight => {
            Ok(Condition::MinHeight(parse_number(value)?))
        }
        crate::app::layout::variants::ConditionKind::MaxHeight => {
            Ok(Condition::MaxHeight(parse_number(value)?))
        }
    }
}

/// A number from an argument.
pub(super) fn parse_number(value: &str) -> Result<f64, String> {
    value
        .parse::<f64>()
        .map_err(|_| format!("{value:?} is not a number"))
}

/// Compares rendered boxes with the library's own solve on the same input (task T10.08).
pub(super) fn layout_probe(options: &crate::cli::LayoutProbeOptions) -> bool {
    use crate::app::layout::layout_choice::LayoutKind;
    use crate::app::layout::layout_plan::{Computed, LayoutInput, LibraryEngine, Role, Size};
    use crate::app::layout::layout_probe::{Run, compare, run};

    let mut scenario = "scenario".to_owned();
    let mut inputs: Vec<LayoutInput> = Vec::new();
    let mut rendered_boxes: Vec<Computed> = Vec::new();
    let mut kind = LayoutKind::Row;
    let mut container: Option<Size> = None;
    let mut gap = 0u32;
    let mut padding = 0u32;
    // The probe side is never supplied by the caller: it is the library layout selected for
    // the scenario, solved on the same inputs the rendered side was given.
    let engine = |kind, container: Option<Size>, gap, padding| {
        let mut engine = LibraryEngine::new(kind).with_gap(gap).with_padding(padding);
        if let Some(container) = container {
            engine = engine.with_container(container);
        }
        engine
    };
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "scenario" => {
                scenario = argument.to_owned();
                Ok(vec![format!("scenario: {argument}")])
            }
            "available" => match argument.split_once(':') {
                Some((node, size)) => match (parse_scene_node(node), parse_floats(size).as_deref())
                {
                    (Ok(node), Ok([width, height])) => {
                        inputs.retain(|input| input.node != node);
                        inputs.push(LayoutInput::new(node, Size::new(*width, *height)));
                        Ok(vec![format!("available {}: {width},{height}", node.get())])
                    }
                    _ => Err(format!("{argument:?} is not <node>:<w>,<h>")),
                },
                None => Err(format!("{argument:?} is not <node>:<w>,<h>")),
            },
            "layout" => match LayoutKind::from_name(argument) {
                Some(selected) => {
                    kind = selected;
                    Ok(vec![format!(
                        "layout: {} ({})",
                        selected.name(),
                        LibraryEngine::new(selected).selection().name()
                    )])
                }
                None => Err(format!("{argument:?} is not a layout kind")),
            },
            "container" => match parse_floats(argument).as_deref() {
                Ok([width, height]) => {
                    container = Some(Size::new(*width, *height));
                    Ok(vec![format!("container: {width},{height}")])
                }
                _ => Err(format!("{argument:?} is not <w>,<h>")),
            },
            "gap" | "padding" => match argument.parse::<u32>() {
                Ok(value) => {
                    if verb == "gap" {
                        gap = value;
                    } else {
                        padding = value;
                    }
                    Ok(vec![format!("{verb}: {value}")])
                }
                Err(_) => Err(format!("{argument:?} is not a non-negative pixel count")),
            },
            // What the host rendered. This is the side the library output is checked against.
            "box" => match parse_box(argument) {
                Ok(box_) => {
                    rendered_boxes.retain(|existing| existing.node != box_.node);
                    rendered_boxes.push(box_);
                    Ok(vec![format!("rendered box {}", box_.render())])
                }
                Err(reason) => Err(reason),
            },
            // Kept only so an old command still runs: the probe is the library's answer, so a
            // caller-supplied list would make the comparison two inputs instead of evidence.
            "probe-box" => Ok(vec![format!(
                "probe-box: {argument} ignored — the probe is produced by the library engine"
            )]),
            "compare" => {
                let probe = run(
                    &engine(kind, container, gap, padding),
                    Role::Generation,
                    "library",
                    &inputs,
                );
                let preview = Run {
                    target: "preview (rendered)".to_owned(),
                    boxes: rendered_boxes.clone(),
                    diagnostics: Vec::new(),
                };
                let report = compare(&scenario, &inputs, &preview, &probe);
                let mut lines = report.render();
                if report.agreed() {
                    lines.push("  the rendered boxes and the library probe agree".to_owned());
                }
                Ok(lines)
            }
            "state" => {
                let probe = run(
                    &engine(kind, container, gap, padding),
                    Role::Generation,
                    "library",
                    &inputs,
                );
                Ok(render_probe(&scenario, &inputs, &rendered_boxes, &probe))
            }
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a layout-probe operation")),
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
    let probe = run(
        &engine(kind, container, gap, padding),
        Role::Generation,
        "library",
        &inputs,
    );
    for line in render_probe(&scenario, &inputs, &rendered_boxes, &probe) {
        println!("{line}");
    }
    true
}

/// The scenario, the inputs, the rendered boxes and the library's run in a report.
pub(super) fn render_probe(
    scenario: &str,
    inputs: &[crate::app::layout::layout_plan::LayoutInput],
    rendered: &[crate::app::layout::layout_plan::Computed],
    library: &crate::app::layout::layout_probe::Run,
) -> Vec<String> {
    let mut lines = vec![format!("scenario: {scenario}")];
    lines.push(format!("inputs: {}", inputs.len()));
    for input in inputs {
        lines.push(format!(
            "  node {} available {},{}",
            input.node.get(),
            input.available.width,
            input.available.height
        ));
    }
    lines.push(format!("rendered boxes: {}", rendered.len()));
    lines.push(format!("library boxes: {}", library.boxes.len()));
    for box_ in &library.boxes {
        lines.push(format!("  {}", box_.render()));
    }
    for diagnostic in &library.diagnostics {
        lines.push(format!("  {}", diagnostic.render()));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::{parse_constraint, render_layout_state, run_arrange};
    use crate::app::layout::arrange::{Axis, Document, Operation};
    use crate::app::layout::layout_choice::LayoutKind;
    use crate::app::layout::layout_plan::{
        Computed, LayoutInput, LibraryEngine, Role, Size, Value,
    };
    use crate::app::layout::layout_probe::{Run, compare, run};
    use crate::contracts::identity::NodeId;

    fn node(value: u64) -> NodeId {
        NodeId::from_raw(value)
    }

    #[test]
    fn a_constraint_value_is_parsed_by_its_property() {
        let minimum = parse_constraint("2:min:explicit:10,20").expect("a size value");
        assert_eq!(
            minimum.value,
            Some(Value::Size {
                width: 10.0,
                height: 20.0
            })
        );
        let position = parse_constraint("2:x:explicit:24").expect("a scalar value");
        assert_eq!(position.value, Some(Value::Scalar(24.0)));
        let cell = parse_constraint("2:cell:explicit:1,2").expect("a cell value");
        assert_eq!(cell.value, Some(Value::Cell { row: 1, column: 2 }));
        let align = parse_constraint("2:align:explicit:centre").expect("a word value");
        assert_eq!(align.value, Some(Value::Word("centre".to_owned())));
    }

    #[test]
    fn a_constraint_without_a_value_keeps_none() {
        let constraint = parse_constraint("2:grow:explicit").expect("the legacy form");
        assert_eq!(constraint.value, None);
    }

    #[test]
    fn a_property_with_no_value_form_is_refused() {
        let error = parse_constraint("2:unknown:explicit:1").expect_err("no value form");
        assert!(error.contains("no value form"));
    }

    #[test]
    fn a_frame_lets_a_size_match_write_a_real_width() {
        let mut document = Document::new().with_bounds(Computed {
            node: node(1),
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        });
        let lines = run_arrange(
            LayoutKind::Row,
            &mut document,
            &[node(1), node(2)],
            Operation::MatchSize {
                axis: Axis::Horizontal,
            },
        );
        assert!(lines[0].contains("2 constraint(s)"));
        assert_eq!(document.value(node(2), "width"), Some(&Value::Scalar(10.0)));
    }

    #[test]
    fn the_probe_compares_library_output_with_rendered_boxes() {
        let inputs = [LayoutInput::new(node(1), Size::new(30.0, 20.0))];
        let probe = run(
            &LibraryEngine::new(LayoutKind::Row),
            Role::Generation,
            "library",
            &inputs,
        );
        assert_eq!(probe.boxes.len(), 1, "the library solved the input");
        let rendered = Run {
            target: "preview (rendered)".to_owned(),
            boxes: vec![Computed {
                node: node(1),
                x: 0.0,
                y: 0.0,
                width: 20.0,
                height: 20.0,
            }],
            diagnostics: Vec::new(),
        };

        let report = compare("mismatch", &inputs, &rendered, &probe);

        assert!(!report.agreed());
        let difference = &report.differences[0];
        assert_eq!(difference.preview.map(|box_| box_.width), Some(20.0));
        assert_eq!(difference.probe.map(|box_| box_.width), Some(30.0));
        assert!(difference.render().contains("preview"));
        assert!(difference.render().contains("but probe"));
    }

    #[test]
    fn the_state_names_the_selection_and_demotes_caller_boxes() {
        let inputs = [LayoutInput::new(node(1), Size::new(30.0, 20.0))];
        let caller = [Computed {
            node: node(1),
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        }];

        let lines = render_layout_state(&inputs, LayoutKind::Column, &caller);

        assert!(lines.iter().any(|line| line.contains("FlexLayout")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("not used by the library engine"))
        );
    }
}
