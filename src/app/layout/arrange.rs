// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Aligning, distributing and matching size, and the keyboard nudge with its snap conversion
//! (task T10.04).
//!
//! The acceptance line is *"one operation changes the real constraints, not writing implicit
//! coordinates"*. An arrangement is therefore a set of **typed constraint writes applied to a
//! document**, and every operation is translated into a value the current layout can actually hold:
//!
//! * **Every change carries its value.** [`Change`] names a node, a property and a
//!   [`Value`] — an alignment is `align = centre`, a gap is
//!   `gap = 8`, a match is `width = 40` — so a change is a real edit, not a property name.
//! * **An edit is pure and reversible.** [`apply_change`] validates one change, produces a new
//!   [`Document`], and records the [`Inverse`] needed to restore the old value (or clear a value
//!   that did not exist). [`Transaction::undo`] applies that inverse to the new document.
//! * **One operation is one undo.** [`apply_to`] validates every change an operation makes against
//!   the same document, applies them together, and returns an [`Arrangement`] whose
//!   [`Arrangement::undo`] restores the whole before-state in one step.
//! * **The translation is the layout's, not the designer's.** [`expresses`] decides whether the
//!   chosen property has a word in the layout; a free layout aligns with a position, a flowing one
//!   with `align`, because that is the word each of them has.
//! * **A nudge is converted, or refused with the reason.** A nudge across a column has no word in a
//!   column, so [`apply`] refuses it rather than silently writing a coordinate the layout does not
//!   own — which is exactly the "implicit coordinate" the acceptance forbids.
//! * **A step is snapped before it is a nudge.** [`snapped_step`] reuses the viewport's grid rule
//!   (T09.05), so a nudge moves by whole grid cells and the arithmetic lives in one place.

#![allow(dead_code)]

use crate::app::layout::layout_choice::LayoutKind;
use crate::app::layout::layout_plan::{Computed, Size, Value};
use crate::contracts::identity::NodeId;
use crate::contracts::model::PropertyName;
use std::collections::{BTreeMap, BTreeSet};

/// A direction an arrangement works along.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Axis {
    /// Left to right.
    Horizontal,
    /// Top to bottom.
    Vertical,
}

impl Axis {
    /// The axis's name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Horizontal => "horizontal",
            Self::Vertical => "vertical",
        }
    }

    /// The axis a name picks.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "horizontal" | "x" => Some(Self::Horizontal),
            "vertical" | "y" => Some(Self::Vertical),
            _ => None,
        }
    }

    /// The free-layout position property for this axis.
    const fn position(self) -> &'static str {
        match self {
            Self::Horizontal => "x",
            Self::Vertical => "y",
        }
    }

    /// The exact-size property for this axis.
    const fn size_property(self) -> &'static str {
        match self {
            Self::Horizontal => "width",
            Self::Vertical => "height",
        }
    }

    /// A box's extent along this axis.
    fn extent(self, bounds: &Computed) -> f64 {
        match self {
            Self::Horizontal => bounds.width,
            Self::Vertical => bounds.height,
        }
    }

    /// A box's start along this axis.
    fn start(self, bounds: &Computed) -> f64 {
        match self {
            Self::Horizontal => bounds.x,
            Self::Vertical => bounds.y,
        }
    }

    /// A box's end along this axis.
    fn end(self, bounds: &Computed) -> f64 {
        match self {
            Self::Horizontal => bounds.x + bounds.width,
            Self::Vertical => bounds.y + bounds.height,
        }
    }
}

/// How an alignment places the selection.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Align {
    /// To the leading edge.
    Start,
    /// To the centre.
    Centre,
    /// To the trailing edge.
    End,
    /// Filled across the axis.
    Stretch,
}

impl Align {
    /// Every mode, in a stable order.
    pub const ALL: [Self; 4] = [Self::Start, Self::Centre, Self::End, Self::Stretch];

    /// The mode's name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Centre => "centre",
            Self::End => "end",
            Self::Stretch => "stretch",
        }
    }

    /// The mode a name picks.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|mode| mode.name() == name)
    }
}

/// What a person asked the selection to do.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Operation {
    /// Align the selection along an axis.
    Align {
        /// Which way.
        axis: Axis,
        /// How.
        mode: Align,
    },
    /// Spread the selection evenly along an axis.
    Distribute {
        /// Which way.
        axis: Axis,
    },
    /// Give every selected node the size of the first along an axis.
    MatchSize {
        /// Which way.
        axis: Axis,
    },
    /// Move the selection one step along an axis.
    Nudge {
        /// Which way.
        axis: Axis,
        /// Which direction along it.
        direction: i8,
        /// How far, already snapped to the grid.
        step: f64,
    },
}

/// One real constraint an operation writes: a node, a property, and its typed value.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Change {
    /// The node it is written on.
    pub node: NodeId,
    /// The property it writes.
    pub property: PropertyName,
    /// The value it writes.
    pub value: Value,
}

impl Change {
    /// A constraint write.
    pub fn new(node: NodeId, property: PropertyName, value: Value) -> Self {
        Self {
            node,
            property,
            value,
        }
    }

    /// The change in one line.
    pub fn render(&self) -> String {
        format!(
            "node {} {} = {}",
            self.node.get(),
            self.property.as_str(),
            self.value.render()
        )
    }
}

/// How to undo one change: restore the old value, or clear a property that did not exist.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Inverse {
    /// Put the previous value back.
    Restore(Change),
    /// Remove a value that the change created.
    Clear {
        /// The node.
        node: NodeId,
        /// The property.
        property: PropertyName,
    },
}

/// The constraint values a document holds, plus the boxes a geometry-dependent operation reads.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Document {
    values: BTreeMap<(NodeId, PropertyName), Value>,
    bounds: BTreeMap<NodeId, Computed>,
    container: Option<Size>,
}

impl Document {
    /// An empty document.
    pub fn new() -> Self {
        Self::default()
    }

    /// The same document with one value.
    pub fn with(mut self, node: NodeId, property: &str, value: Value) -> Self {
        self.set(node, name(property), value);
        self
    }

    /// The same document with a node's box, for an operation that needs geometry.
    pub fn with_bounds(mut self, bounds: Computed) -> Self {
        self.bounds.insert(bounds.node, bounds);
        self
    }

    /// The same document with the container extent a free alignment measures against.
    pub fn with_container(mut self, container: Size) -> Self {
        self.container = Some(container);
        self
    }

    /// The value written for a property, if one is.
    pub fn value(&self, node: NodeId, property: &str) -> Option<&Value> {
        self.values.get(&(node, name(property)))
    }

    /// A node's box, if the operation was given one.
    pub fn bounds(&self, node: NodeId) -> Option<&Computed> {
        self.bounds.get(&node)
    }

    /// Writes a value, replacing any existing one.
    pub fn set(&mut self, node: NodeId, property: PropertyName, value: Value) {
        self.values.insert((node, property), value);
    }

    /// Removes a value, returning the one that was there.
    pub fn clear(&mut self, node: NodeId, property: &PropertyName) -> Option<Value> {
        self.values.remove(&(node, property.clone()))
    }

    /// The extent an alignment or distribution measures against.
    fn extent(&self, axis: Axis) -> Option<f64> {
        if let Some(container) = self.container {
            return Some(match axis {
                Axis::Horizontal => container.width,
                Axis::Vertical => container.height,
            });
        }
        self.bounds
            .values()
            .map(|bounds| axis.end(bounds))
            .reduce(f64::max)
    }

    /// The document in a report.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "document: {} value(s), {} box(es)",
            self.values.len(),
            self.bounds.len()
        )];
        for ((node, property), value) in &self.values {
            lines.push(format!(
                "  node {} {} = {}",
                node.get(),
                property.as_str(),
                value.render()
            ));
        }
        lines
    }
}

/// A constraint write that has been applied, with the inverse needed to undo it.
#[derive(Clone, PartialEq, Debug)]
pub struct Transaction {
    /// The write.
    pub change: Change,
    /// How to undo it.
    pub inverse: Inverse,
    /// The document after the write.
    pub document: Document,
}

impl Transaction {
    /// Restores the document this transaction started from.
    pub fn undo(&self) -> Document {
        undo(&self.document, std::slice::from_ref(&self.inverse))
    }
}

/// An operation that can be made: the constraint writes it makes and the document it produces.
#[derive(Clone, PartialEq, Debug)]
pub struct Arrangement {
    /// What was asked.
    pub operation: Operation,
    /// The constraints it writes, one per selected node (or one per computed placement).
    pub changes: Vec<Change>,
    /// How to restore the before-document.
    pub inverse: Vec<Inverse>,
    /// The document after every change was applied.
    pub document: Document,
}

impl Arrangement {
    /// The arrangement in a report.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "{}: {} constraint(s)",
            describe(self.operation),
            self.changes.len()
        )];
        for change in &self.changes {
            lines.push(format!("  {}", change.render()));
        }
        lines
    }

    /// Restores the before-document in one step. One operation is one undo.
    pub fn undo(&self) -> Document {
        undo(&self.document, &self.inverse)
    }
}

/// An operation or edit that cannot be made, and why.
#[derive(Clone, PartialEq, Debug)]
pub struct Refusal {
    /// The operation, when the refusal is about one.
    pub operation: Option<Operation>,
    /// Why it cannot be done.
    pub reason: String,
}

impl Refusal {
    /// A refused operation.
    fn for_operation(operation: Operation, reason: impl Into<String>) -> Self {
        Self {
            operation: Some(operation),
            reason: reason.into(),
        }
    }

    /// A refused constraint edit.
    fn for_change(reason: impl Into<String>) -> Self {
        Self {
            operation: None,
            reason: reason.into(),
        }
    }

    /// The refusal in a report.
    pub fn render(&self) -> Vec<String> {
        match self.operation {
            Some(operation) => vec![format!(
                "{}: refused — {}",
                describe(operation),
                self.reason
            )],
            None => vec![format!("edit refused — {}", self.reason)],
        }
    }
}

/// Applies one constraint write to a document, purely, with its inverse.
///
/// This is the document-transaction seam: the returned [`Transaction`] owns a new document, never a
/// mutated one, and its inverse restores the exact previous state.
pub fn apply_change(
    kind: LayoutKind,
    document: &Document,
    change: Change,
) -> Result<Transaction, Refusal> {
    validate_change(kind, &change)?;
    let inverse = inverse_for(document, &change);
    let mut next = document.clone();
    next.set(change.node, change.property.clone(), change.value.clone());
    validate_document(&next)?;
    Ok(Transaction {
        change,
        inverse,
        document: next,
    })
}

/// Applies an operation to a selection under a layout, as constraint writes.
///
/// Without a document this can still write alignments, gaps and positions, but an operation whose
/// value depends on the selection's boxes is refused rather than guessed. Use [`apply_to`] when the
/// boxes are known.
pub fn apply(
    kind: LayoutKind,
    nodes: &[NodeId],
    operation: Operation,
) -> Result<Arrangement, Refusal> {
    apply_to(kind, &Document::new(), nodes, operation)
}

/// Applies an operation to a document and a selection, as one transaction.
pub fn apply_to(
    kind: LayoutKind,
    document: &Document,
    nodes: &[NodeId],
    operation: Operation,
) -> Result<Arrangement, Refusal> {
    let changes = changes_for(kind, document, nodes, operation)?;
    let mut next = document.clone();
    let mut inverse = Vec::with_capacity(changes.len());
    for change in &changes {
        validate_change(kind, change)?;
        inverse.push(inverse_for(&next, change));
        next.set(change.node, change.property.clone(), change.value.clone());
    }
    validate_document(&next)?;
    Ok(Arrangement {
        operation,
        changes,
        inverse,
        document: next,
    })
}

/// The step a nudge of `step` becomes once snapped to a grid spacing (reusing T09.05's rule).
pub fn snapped_step(step: f64, spacing: f64) -> f64 {
    use crate::app::canvas::viewport::{Grid, Point};
    Grid::new(spacing).snap(Point::new(step, 0.0)).x
}

/// Why an operation cannot be turned into constraint writes.
fn changes_for(
    kind: LayoutKind,
    document: &Document,
    nodes: &[NodeId],
    operation: Operation,
) -> Result<Vec<Change>, Refusal> {
    if nodes.is_empty() {
        return Ok(Vec::new());
    }
    match operation {
        Operation::Align { axis, mode } => Ok(align_changes(kind, document, nodes, axis, mode)?),
        Operation::Distribute { axis } => distribute_changes(kind, document, nodes, axis),
        Operation::MatchSize { axis } => match_size_changes(kind, document, nodes, axis),
        Operation::Nudge {
            axis,
            direction,
            step,
        } => nudge_changes(kind, document, nodes, axis, direction, step),
    }
}

/// An alignment: a named constraint in a flow, a real position (or size) in a free layout.
fn align_changes(
    kind: LayoutKind,
    document: &Document,
    nodes: &[NodeId],
    axis: Axis,
    mode: Align,
) -> Result<Vec<Change>, Refusal> {
    if kind != LayoutKind::Free {
        return Ok(nodes
            .iter()
            .map(|node| change(*node, "align", Value::Word(mode.name().to_owned())))
            .collect());
    }
    let extent = document.extent(axis).ok_or_else(|| {
        Refusal::for_change("a free alignment needs the container extent or the selected boxes")
    })?;
    let mut changes = Vec::with_capacity(nodes.len());
    for node in nodes {
        if mode == Align::Stretch {
            changes.push(change(*node, axis.size_property(), Value::Scalar(extent)));
            continue;
        }
        let bounds = document.bounds(*node).ok_or_else(|| {
            Refusal::for_change(format!(
                "node {} has no box; a free alignment needs a coordinate to write",
                node.get()
            ))
        })?;
        let size = axis.extent(bounds);
        let position = match mode {
            Align::Start => 0.0,
            Align::Centre => (extent - size) / 2.0,
            Align::End => extent - size,
            Align::Stretch => unreachable!("handled above"),
        };
        changes.push(change(*node, axis.position(), Value::Scalar(position)));
    }
    Ok(changes)
}

/// A distribution: the flow's gap, or a position per node in a free layout.
fn distribute_changes(
    kind: LayoutKind,
    document: &Document,
    nodes: &[NodeId],
    axis: Axis,
) -> Result<Vec<Change>, Refusal> {
    if kind == LayoutKind::Free {
        let mut boxes: Vec<(NodeId, &Computed)> = nodes
            .iter()
            .map(|node| {
                document
                    .bounds(*node)
                    .map(|bounds| (*node, bounds))
                    .ok_or_else(|| {
                        Refusal::for_change(format!(
                            "node {} has no box; a free distribution needs its current position",
                            node.get()
                        ))
                    })
            })
            .collect::<Result<_, _>>()?;
        boxes.sort_by(|(_, a), (_, b)| axis.start(a).total_cmp(&axis.start(b)));
        let origin = axis.start(boxes[0].1);
        let span = axis.end(boxes[boxes.len() - 1].1) - origin;
        let used: f64 = boxes.iter().map(|(_, bounds)| axis.extent(bounds)).sum();
        let gap = if boxes.len() > 1 {
            ((span - used) / (boxes.len() - 1) as f64).max(0.0)
        } else {
            0.0
        };
        let mut cursor = origin;
        let mut changes = Vec::with_capacity(boxes.len());
        for (node, bounds) in boxes {
            changes.push(change(node, axis.position(), Value::Scalar(cursor)));
            cursor += axis.extent(bounds) + gap;
        }
        return Ok(changes);
    }

    if !expresses(kind, "gap") {
        return Err(Refusal::for_change(format!(
            "a {} layout cannot express gap",
            kind.name()
        )));
    }
    let gap = even_gap(document, nodes, axis)?;
    Ok(nodes
        .iter()
        .map(|node| change(*node, "gap", Value::Scalar(gap)))
        .collect())
}

/// The gap that spaces the selected boxes evenly, from the boxes the document holds.
fn even_gap(document: &Document, nodes: &[NodeId], axis: Axis) -> Result<f64, Refusal> {
    let mut boxes = Vec::with_capacity(nodes.len());
    for node in nodes {
        let bounds = document.bounds(*node).ok_or_else(|| {
            Refusal::for_change(format!(
                "node {} has no box; distributing needs the selected boxes",
                node.get()
            ))
        })?;
        boxes.push(bounds);
    }
    boxes.sort_by(|a, b| axis.start(a).total_cmp(&axis.start(b)));
    if boxes.len() < 2 {
        return Ok(0.0);
    }
    let span = axis.end(boxes[boxes.len() - 1]) - axis.start(boxes[0]);
    let used: f64 = boxes.iter().map(|bounds| axis.extent(bounds)).sum();
    Ok(((span - used) / (boxes.len() - 1) as f64).max(0.0))
}

/// A match: every node gets the first node's size as a real width/height value.
fn match_size_changes(
    kind: LayoutKind,
    document: &Document,
    nodes: &[NodeId],
    axis: Axis,
) -> Result<Vec<Change>, Refusal> {
    let property = axis.size_property();
    if !expresses(kind, property) {
        return Err(Refusal::for_change(format!(
            "a {} layout cannot express {property}",
            kind.name()
        )));
    }
    let first = nodes[0];
    let size = document
        .bounds(first)
        .map(|bounds| axis.extent(bounds))
        .or_else(|| document.value(first, property).and_then(Value::scalar))
        .ok_or_else(|| {
            Refusal::for_change(format!(
                "node {} has no size to match; give it a box or a {property} value",
                first.get()
            ))
        })?;
    Ok(nodes
        .iter()
        .map(|node| change(*node, property, Value::Scalar(size)))
        .collect())
}

/// A nudge: a position, or the order within a flow, changed by the snapped step.
fn nudge_changes(
    kind: LayoutKind,
    document: &Document,
    nodes: &[NodeId],
    axis: Axis,
    direction: i8,
    step: f64,
) -> Result<Vec<Change>, Refusal> {
    let property = match nudge_property(kind, axis) {
        Ok(property) => property,
        Err(reason) => return Err(Refusal::for_change(reason)),
    };
    let delta = if property == "order" {
        f64::from(direction)
    } else {
        f64::from(direction) * step
    };
    Ok(nodes
        .iter()
        .enumerate()
        .map(|(index, node)| {
            let current = document
                .value(*node, property)
                .and_then(Value::scalar)
                .unwrap_or_else(|| {
                    if property == "order" {
                        index as f64
                    } else {
                        0.0
                    }
                });
            change(*node, property, Value::Scalar(current + delta))
        })
        .collect())
}

/// The property a nudge writes under a layout, or why it has no word there.
fn nudge_property(kind: LayoutKind, axis: Axis) -> Result<&'static str, String> {
    match kind {
        LayoutKind::Free | LayoutKind::Stack => Ok(axis.position()),
        LayoutKind::Row if axis == Axis::Horizontal => Ok("order"),
        LayoutKind::Column if axis == Axis::Vertical => Ok("order"),
        LayoutKind::Grid => Ok("order"),
        LayoutKind::Row => Err("a row cannot move a child across the flow".to_owned()),
        LayoutKind::Column => Err("a column cannot move a child across the flow".to_owned()),
    }
}

/// Whether a layout can hold a constraint on a property. The arrangement's vocabulary rule.
fn expresses(kind: LayoutKind, property: &str) -> bool {
    match kind {
        LayoutKind::Free => matches!(
            property,
            "x" | "y" | "width" | "height" | "margin" | "min" | "max" | "preferred"
        ),
        LayoutKind::Stack => matches!(
            property,
            "x" | "y" | "width" | "height" | "margin" | "align" | "min" | "max" | "preferred"
        ),
        LayoutKind::Row | LayoutKind::Column => matches!(
            property,
            "width"
                | "height"
                | "grow"
                | "margin"
                | "gap"
                | "order"
                | "align"
                | "min"
                | "max"
                | "preferred"
                | "padding"
        ),
        LayoutKind::Grid => matches!(
            property,
            "width"
                | "height"
                | "margin"
                | "gap"
                | "order"
                | "align"
                | "min"
                | "max"
                | "preferred"
                | "padding"
                | "cell"
        ),
    }
}

/// Validates one change against the layout and the value family its property holds.
fn validate_change(kind: LayoutKind, change: &Change) -> Result<(), Refusal> {
    let property = change.property.as_str();
    if !expresses(kind, property) {
        return Err(Refusal::for_change(format!(
            "a {} layout cannot express {property}",
            kind.name()
        )));
    }
    check_value(change).map_err(Refusal::for_change)?;
    Ok(())
}

/// Validates that a change's value has the family its property needs.
fn check_value(change: &Change) -> Result<(), String> {
    let property = change.property.as_str();
    match property {
        "min" | "max" | "preferred" => {
            let size = change
                .value
                .size()
                .ok_or_else(|| format!("{property} needs a width,height value"))?;
            if !finite_non_negative(size) {
                return Err(format!(
                    "{property} {},{} is not a finite non-negative size",
                    size.width, size.height
                ));
            }
        }
        "width" | "height" | "gap" | "margin" | "padding" | "grow" => {
            let number = change
                .value
                .scalar()
                .ok_or_else(|| format!("{property} needs a number"))?;
            if !number.is_finite() || number < 0.0 {
                return Err(format!(
                    "{property} {number} is not a finite non-negative value"
                ));
            }
        }
        "x" | "y" | "order" => {
            let number = change
                .value
                .scalar()
                .ok_or_else(|| format!("{property} needs a number"))?;
            if !number.is_finite() {
                return Err(format!("{property} {number} is not a finite number"));
            }
        }
        "align" => {
            let word = change
                .value
                .word()
                .ok_or_else(|| "align needs a named mode".to_owned())?;
            if Align::from_name(word).is_none() {
                return Err(format!("{word:?} is not an alignment"));
            }
        }
        "cell" => {
            if change.value.cell().is_none() {
                return Err("cell needs a row,column value".to_owned());
            }
        }
        _ => {
            return Err(format!(
                "{property} is not a constraint the arrangement can write"
            ));
        }
    }
    Ok(())
}

/// Validates every node's values and size bounds in a document after an edit.
fn validate_document(document: &Document) -> Result<(), Refusal> {
    for ((node, property), value) in &document.values {
        let change = Change::new(*node, property.clone(), value.clone());
        check_value(&change).map_err(Refusal::for_change)?;
    }
    let nodes: BTreeSet<NodeId> = document.values.keys().map(|(node, _)| *node).collect();
    for node in nodes {
        let minimum = document.value(node, "min").and_then(Value::size);
        let maximum = document.value(node, "max").and_then(Value::size);
        let preferred = document.value(node, "preferred").and_then(Value::size);
        if let (Some(minimum), Some(maximum)) = (minimum, maximum) {
            if minimum.width > maximum.width {
                return Err(Refusal::for_change(format!(
                    "node {}: the minimum width {} is above the maximum width {}",
                    node.get(),
                    minimum.width,
                    maximum.width
                )));
            }
            if minimum.height > maximum.height {
                return Err(Refusal::for_change(format!(
                    "node {}: the minimum height {} is above the maximum height {}",
                    node.get(),
                    minimum.height,
                    maximum.height
                )));
            }
        }
        if let Some(preferred) = preferred {
            if let Some(maximum) = maximum {
                if preferred.width > maximum.width {
                    return Err(Refusal::for_change(format!(
                        "node {}: the preferred width {} is above the maximum width {}",
                        node.get(),
                        preferred.width,
                        maximum.width
                    )));
                }
                if preferred.height > maximum.height {
                    return Err(Refusal::for_change(format!(
                        "node {}: the preferred height {} is above the maximum height {}",
                        node.get(),
                        preferred.height,
                        maximum.height
                    )));
                }
            }
            if let Some(minimum) = minimum {
                if preferred.width < minimum.width {
                    return Err(Refusal::for_change(format!(
                        "node {}: the preferred width {} is below the minimum width {}",
                        node.get(),
                        preferred.width,
                        minimum.width
                    )));
                }
                if preferred.height < minimum.height {
                    return Err(Refusal::for_change(format!(
                        "node {}: the preferred height {} is below the minimum height {}",
                        node.get(),
                        preferred.height,
                        minimum.height
                    )));
                }
            }
        }
    }
    Ok(())
}

/// The inverse of writing `change` into `document`.
fn inverse_for(document: &Document, change: &Change) -> Inverse {
    match document.value(change.node, change.property.as_str()) {
        Some(value) => Inverse::Restore(Change::new(
            change.node,
            change.property.clone(),
            value.clone(),
        )),
        None => Inverse::Clear {
            node: change.node,
            property: change.property.clone(),
        },
    }
}

/// Applies inverse steps to a document, in reverse order.
fn undo(document: &Document, inverse: &[Inverse]) -> Document {
    let mut next = document.clone();
    for step in inverse.iter().rev() {
        match step {
            Inverse::Restore(change) => {
                next.set(change.node, change.property.clone(), change.value.clone());
            }
            Inverse::Clear { node, property } => {
                next.clear(*node, property);
            }
        }
    }
    next
}

/// A change with a fixed property name.
fn change(node: NodeId, property: &'static str, value: Value) -> Change {
    Change::new(node, name(property), value)
}

/// A fixed property name.
fn name(property: &str) -> PropertyName {
    PropertyName::new(property).expect("a fixed property name")
}

/// Whether a size is finite and non-negative on both axes.
fn finite_non_negative(size: Size) -> bool {
    size.width.is_finite() && size.height.is_finite() && size.width >= 0.0 && size.height >= 0.0
}

/// What an operation is, for a report.
fn describe(operation: Operation) -> String {
    match operation {
        Operation::Align { axis, mode } => format!("align {} {}", mode.name(), axis.name()),
        Operation::Distribute { axis } => format!("distribute {}", axis.name()),
        Operation::MatchSize { axis } => format!("match size {}", axis.name()),
        Operation::Nudge {
            axis,
            direction,
            step,
        } => format!("nudge {} by {direction} x{step}", axis.name()),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Align, Axis, Document, Inverse, Operation, apply, apply_change, apply_to, snapped_step,
    };
    use crate::app::layout::layout_choice::LayoutKind;
    use crate::app::layout::layout_plan::{Computed, Size, Value};
    use crate::contracts::identity::NodeId;

    fn node(value: u64) -> NodeId {
        NodeId::from_raw(value)
    }

    fn nodes() -> Vec<NodeId> {
        vec![node(1), node(2), node(3)]
    }

    fn box_(node: NodeId, x: f64, y: f64, width: f64, height: f64) -> Computed {
        Computed {
            node,
            x,
            y,
            width,
            height,
        }
    }

    /// The values an arrangement writes, in order.
    fn values(kind: LayoutKind, operation: Operation) -> Vec<Value> {
        apply(kind, &nodes(), operation)
            .expect("an expressible operation")
            .changes
            .iter()
            .map(|change| change.value.clone())
            .collect()
    }

    /// A document with three 10-wide boxes laid out left to right.
    fn row_document() -> Document {
        Document::new()
            .with_bounds(box_(node(1), 0.0, 0.0, 10.0, 10.0))
            .with_bounds(box_(node(2), 20.0, 0.0, 10.0, 10.0))
            .with_bounds(box_(node(3), 40.0, 0.0, 10.0, 10.0))
    }

    #[test]
    fn aligning_in_a_flowing_layout_writes_the_align_value() {
        let operation = Operation::Align {
            axis: Axis::Vertical,
            mode: Align::Centre,
        };
        assert_eq!(
            values(LayoutKind::Row, operation),
            vec![Value::Word("centre".to_owned()); 3]
        );
    }

    #[test]
    fn aligning_in_a_free_layout_writes_real_positions() {
        let document = Document::new()
            .with_container(Size::new(100.0, 40.0))
            .with_bounds(box_(node(1), 0.0, 0.0, 20.0, 10.0));
        let operation = Operation::Align {
            axis: Axis::Horizontal,
            mode: Align::Centre,
        };
        let arrangement = apply_to(LayoutKind::Free, &document, &[node(1)], operation)
            .expect("the container and the box are known");
        assert_eq!(arrangement.changes[0].property.as_str(), "x");
        assert_eq!(
            arrangement.changes[0].value,
            Value::Scalar(40.0),
            "(100 - 20) / 2"
        );
    }

    #[test]
    fn distributing_in_a_flow_writes_the_computed_gap() {
        let operation = Operation::Distribute {
            axis: Axis::Horizontal,
        };
        let arrangement = apply_to(LayoutKind::Row, &row_document(), &nodes(), operation)
            .expect("the boxes are known");
        assert_eq!(
            arrangement.changes[0].value,
            Value::Scalar(10.0),
            "the 20 px gaps become one 10 px gap"
        );
        assert_eq!(
            arrangement
                .changes
                .iter()
                .map(|change| change.property.as_str())
                .collect::<Vec<_>>(),
            vec!["gap"; 3]
        );
    }

    #[test]
    fn distributing_without_boxes_is_refused() {
        let operation = Operation::Distribute {
            axis: Axis::Horizontal,
        };
        let refusal = apply(LayoutKind::Row, &nodes(), operation).expect_err("no boxes");
        assert!(refusal.reason.contains("no box"));
        assert!(refusal.render()[0].contains("refused"));
    }

    #[test]
    fn distributing_in_a_stack_is_refused() {
        let operation = Operation::Distribute {
            axis: Axis::Horizontal,
        };
        let refusal =
            apply(LayoutKind::Stack, &nodes(), operation).expect_err("a stack has no gap");
        assert!(refusal.reason.contains("cannot express gap"));
    }

    #[test]
    fn matching_the_size_writes_the_first_size() {
        let document = row_document();
        let horizontal = Operation::MatchSize {
            axis: Axis::Horizontal,
        };
        let vertical = Operation::MatchSize {
            axis: Axis::Vertical,
        };
        let widths = apply_to(LayoutKind::Grid, &document, &nodes(), horizontal)
            .expect("the first box has a width");
        assert_eq!(widths.changes[0].property.as_str(), "width");
        assert!(
            widths
                .changes
                .iter()
                .all(|change| change.value == Value::Scalar(10.0))
        );
        let heights = apply_to(LayoutKind::Grid, &document, &nodes(), vertical)
            .expect("the first box has a height");
        assert_eq!(heights.changes[0].property.as_str(), "height");
    }

    #[test]
    fn matching_a_size_without_a_size_is_refused() {
        let operation = Operation::MatchSize {
            axis: Axis::Horizontal,
        };
        let refusal = apply(LayoutKind::Row, &nodes(), operation).expect_err("no size to match");
        assert!(refusal.reason.contains("no size to match"));
    }

    #[test]
    fn a_nudge_down_a_column_writes_order_values() {
        let document = Document::new()
            .with(node(1), "order", Value::Scalar(0.0))
            .with(node(2), "order", Value::Scalar(1.0))
            .with(node(3), "order", Value::Scalar(2.0));
        let operation = Operation::Nudge {
            axis: Axis::Vertical,
            direction: 1,
            step: 8.0,
        };
        let arrangement = apply_to(LayoutKind::Column, &document, &nodes(), operation)
            .expect("a column can reorder");
        assert_eq!(
            arrangement
                .changes
                .iter()
                .map(|change| change.value.clone())
                .collect::<Vec<_>>(),
            vec![Value::Scalar(1.0), Value::Scalar(2.0), Value::Scalar(3.0)]
        );
    }

    #[test]
    fn a_nudge_across_a_column_is_refused() {
        let operation = Operation::Nudge {
            axis: Axis::Horizontal,
            direction: 1,
            step: 8.0,
        };
        let refusal = apply(LayoutKind::Column, &nodes(), operation).expect_err("no word across");
        assert!(refusal.reason.contains("across the flow"));
        assert!(refusal.render()[0].contains("refused"));
    }

    #[test]
    fn a_nudge_in_a_free_layout_writes_a_position() {
        let document = Document::new()
            .with(node(1), "x", Value::Scalar(10.0))
            .with(node(1), "y", Value::Scalar(10.0));
        let horizontal = Operation::Nudge {
            axis: Axis::Horizontal,
            direction: -1,
            step: 8.0,
        };
        let vertical = Operation::Nudge {
            axis: Axis::Vertical,
            direction: 1,
            step: 8.0,
        };
        let moved = apply_to(LayoutKind::Free, &document, &[node(1)], horizontal)
            .expect("a free layout moves x");
        assert_eq!(moved.changes[0].property.as_str(), "x");
        assert_eq!(moved.changes[0].value, Value::Scalar(2.0));
        let moved = apply_to(LayoutKind::Free, &document, &[node(1)], vertical)
            .expect("a free layout moves y");
        assert_eq!(moved.changes[0].property.as_str(), "y");
        assert_eq!(moved.changes[0].value, Value::Scalar(18.0));
    }

    #[test]
    fn a_snapped_step_is_a_grid_multiple() {
        assert_eq!(snapped_step(13.0, 8.0), 16.0);
        assert_eq!(snapped_step(3.0, 8.0), 0.0);
        assert_eq!(snapped_step(8.0, 8.0), 8.0);
    }

    #[test]
    fn an_operation_on_no_nodes_writes_nothing() {
        let operation = Operation::MatchSize {
            axis: Axis::Horizontal,
        };
        let arrangement = apply(LayoutKind::Row, &[], operation).expect("a valid operation");
        assert!(arrangement.changes.is_empty());
    }

    #[test]
    fn every_layout_can_match_a_width() {
        let document = row_document();
        for kind in LayoutKind::ALL {
            let operation = Operation::MatchSize {
                axis: Axis::Horizontal,
            };
            assert!(
                apply_to(kind, &document, &nodes(), operation).is_ok(),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn axis_and_align_names_round_trip() {
        assert_eq!(Axis::from_name("x"), Some(Axis::Horizontal));
        assert_eq!(Axis::from_name("vertical"), Some(Axis::Vertical));
        for mode in Align::ALL {
            assert_eq!(Align::from_name(mode.name()), Some(mode));
        }
        assert!(Axis::from_name("z").is_none());
    }

    #[test]
    fn a_width_edit_produces_a_new_document_and_its_inverse() {
        let document = Document::new().with(node(1), "width", Value::Scalar(10.0));
        let transaction = apply_change(
            LayoutKind::Row,
            &document,
            super::change(node(1), "width", Value::Scalar(24.0)),
        )
        .expect("a valid width");

        assert_eq!(
            transaction
                .document
                .value(node(1), "width")
                .and_then(Value::scalar),
            Some(24.0)
        );
        assert_eq!(transaction.undo(), document, "one edit is one undo");
    }

    #[test]
    fn a_new_value_is_cleared_by_its_inverse() {
        let document = Document::new();
        let transaction = apply_change(
            LayoutKind::Row,
            &document,
            super::change(node(1), "gap", Value::Scalar(8.0)),
        )
        .expect("a valid gap");
        assert!(matches!(transaction.inverse, Inverse::Clear { .. }));
        assert_eq!(transaction.undo(), document);
    }

    #[test]
    fn min_max_gap_margin_align_and_width_are_all_typed_edits() {
        let document = Document::new();
        let edits = [
            super::change(
                node(1),
                "min",
                Value::Size {
                    width: 10.0,
                    height: 10.0,
                },
            ),
            super::change(
                node(1),
                "max",
                Value::Size {
                    width: 100.0,
                    height: 100.0,
                },
            ),
            super::change(node(1), "gap", Value::Scalar(8.0)),
            super::change(node(1), "margin", Value::Scalar(4.0)),
            super::change(node(1), "align", Value::Word("centre".to_owned())),
            super::change(node(1), "width", Value::Scalar(40.0)),
        ];
        let mut current = document;
        for edit in edits {
            let transaction = apply_change(LayoutKind::Row, &current, edit)
                .expect("each typed edit is valid on its own");
            current = transaction.document;
        }
        assert_eq!(
            current.value(node(1), "min"),
            Some(&Value::Size {
                width: 10.0,
                height: 10.0
            })
        );
        assert_eq!(
            current.value(node(1), "max"),
            Some(&Value::Size {
                width: 100.0,
                height: 100.0
            })
        );
        assert_eq!(current.value(node(1), "gap"), Some(&Value::Scalar(8.0)));
        assert_eq!(current.value(node(1), "margin"), Some(&Value::Scalar(4.0)));
        assert_eq!(
            current.value(node(1), "align"),
            Some(&Value::Word("centre".to_owned()))
        );
        assert_eq!(current.value(node(1), "width"), Some(&Value::Scalar(40.0)));
    }

    #[test]
    fn a_min_above_the_max_is_refused_at_the_node() {
        let document = Document::new().with(
            node(2),
            "max",
            Value::Size {
                width: 50.0,
                height: 50.0,
            },
        );
        let refusal = apply_change(
            LayoutKind::Row,
            &document,
            super::change(
                node(2),
                "min",
                Value::Size {
                    width: 100.0,
                    height: 10.0,
                },
            ),
        )
        .expect_err("the minimum is above the maximum");
        assert!(refusal.reason.contains("node 2"));
        assert!(refusal.reason.contains("minimum width"));
    }

    #[test]
    fn a_negative_width_or_margin_is_refused() {
        for property in ["width", "margin", "gap"] {
            let refusal = apply_change(
                LayoutKind::Row,
                &Document::new(),
                super::change(node(3), property, Value::Scalar(-1.0)),
            )
            .expect_err("a negative length is illegal");
            assert!(refusal.reason.contains(property));
        }
    }

    #[test]
    fn an_align_word_the_layout_does_not_know_is_refused() {
        let refusal = apply_change(
            LayoutKind::Row,
            &Document::new(),
            super::change(node(4), "align", Value::Word("diagonal".to_owned())),
        )
        .expect_err("diagonal is not an alignment");
        assert!(refusal.reason.contains("not an alignment"));
    }

    #[test]
    fn one_operation_is_one_undo() {
        let document = row_document()
            .with(node(1), "align", Value::Word("start".to_owned()))
            .with(node(2), "align", Value::Word("start".to_owned()))
            .with(node(3), "align", Value::Word("start".to_owned()));
        let operation = Operation::Align {
            axis: Axis::Vertical,
            mode: Align::Centre,
        };
        let arrangement =
            apply_to(LayoutKind::Row, &document, &nodes(), operation).expect("three alignments");
        assert_eq!(arrangement.changes.len(), 3);
        assert_eq!(arrangement.inverse.len(), 3);
        assert_eq!(
            arrangement.undo(),
            document,
            "one undo restores all three constraints together"
        );
    }
}
