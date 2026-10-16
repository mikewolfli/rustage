// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Aligning, distributing and matching size, and the keyboard nudge with its snap conversion
//! (task T10.04).
//!
//! The acceptance line is *"one operation changes the real constraints, not writing implicit
//! coordinates"*. An arrangement is therefore a set of **constraint writes**, and every operation is
//! translated into a word the current layout can actually hold:
//!
//! * **Every operation becomes a constraint on a property.** [`apply`] answers a [`Change`] per
//!   selected node, naming the property it writes — `align`, `gap`, `width`, `order`, `x` — so an
//!   alignment is an alignment and not a pile of coordinates.
//! * **The translation is the layout's, not the designer's.** [`LayoutKind::expresses`] decides
//!   whether the chosen property is expressible; a flowing layout aligns with `align`, a free one
//!   with a position, because that is the word each of them has.
//! * **A nudge is converted, or refused with the reason.** A nudge across a column has no word in a
//!   column, so [`apply`] refuses it rather than silently writing a coordinate the layout does not
//!   own — which is exactly the "implicit coordinate" the acceptance forbids.
//! * **A step is snapped before it is a nudge.** [`snapped_step`] reuses the viewport's grid rule
//!   (T09.05), so a nudge moves by whole grid cells and the arithmetic lives in one place.

#![allow(dead_code)]

use crate::app::layout::layout_choice::LayoutKind;
use crate::contracts::identity::NodeId;
use crate::contracts::model::PropertyName;

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

/// One real constraint an operation writes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Change {
    /// The node it is written on.
    pub node: NodeId,
    /// The property it writes.
    pub property: PropertyName,
}

/// An operation that can be made: the constraint writes it makes.
#[derive(Clone, PartialEq, Debug)]
pub struct Arrangement {
    /// What was asked.
    pub operation: Operation,
    /// The constraints it writes, one per selected node.
    pub changes: Vec<Change>,
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
            lines.push(format!(
                "  node {} {}",
                change.node.get(),
                change.property.as_str()
            ));
        }
        lines
    }
}

/// An operation that cannot be made, and why.
#[derive(Clone, PartialEq, Debug)]
pub struct Refusal {
    /// What was asked.
    pub operation: Operation,
    /// Why it cannot be done.
    pub reason: String,
}

impl Refusal {
    /// The refusal in a report.
    pub fn render(&self) -> Vec<String> {
        vec![format!(
            "{}: refused — {}",
            describe(self.operation),
            self.reason
        )]
    }
}

/// Applies an operation to a selection under a layout, as constraint writes.
pub fn apply(
    kind: LayoutKind,
    nodes: &[NodeId],
    operation: Operation,
) -> Result<Arrangement, Refusal> {
    let name = match property_for(kind, operation) {
        Ok(name) => name,
        Err(reason) => {
            return Err(Refusal { operation, reason });
        }
    };
    let property = PropertyName::new(name).expect("a fixed property name");
    if !kind.expresses(&property) {
        return Err(Refusal {
            operation,
            reason: format!(
                "a {} layout cannot express {}",
                kind.name(),
                property.as_str()
            ),
        });
    }
    let changes = nodes
        .iter()
        .map(|node| Change {
            node: *node,
            property: property.clone(),
        })
        .collect();
    Ok(Arrangement { operation, changes })
}

/// The step a nudge of `step` becomes once snapped to a grid spacing (reusing T09.05's rule).
pub fn snapped_step(step: f64, spacing: f64) -> f64 {
    use crate::app::canvas::viewport::{Grid, Point};
    Grid::new(spacing).snap(Point::new(step, 0.0)).x
}

/// The property an operation writes under a layout, before it is checked against the vocabulary.
fn property_for(kind: LayoutKind, operation: Operation) -> Result<&'static str, String> {
    match operation {
        // A free or stacked layout aligns with a position; a flowing one has an `align`.
        Operation::Align { axis, .. } => Ok(if kind == LayoutKind::Free {
            axis.position()
        } else {
            "align"
        }),
        Operation::Distribute { axis } => Ok(if kind == LayoutKind::Free {
            axis.position()
        } else {
            "gap"
        }),
        Operation::MatchSize { axis } => Ok(match axis {
            Axis::Horizontal => "width",
            Axis::Vertical => "height",
        }),
        Operation::Nudge { axis, .. } => match kind {
            LayoutKind::Free | LayoutKind::Stack => Ok(axis.position()),
            LayoutKind::Row if axis == Axis::Horizontal => Ok("order"),
            LayoutKind::Column if axis == Axis::Vertical => Ok("order"),
            LayoutKind::Grid => Ok("order"),
            LayoutKind::Row => Err("a row cannot move a child across the flow".to_owned()),
            LayoutKind::Column => Err("a column cannot move a child across the flow".to_owned()),
        },
    }
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
    use super::{Align, Axis, Operation, apply, snapped_step};
    use crate::app::layout::layout_choice::LayoutKind;
    use crate::contracts::identity::NodeId;

    fn node(value: u64) -> NodeId {
        NodeId::from_raw(value)
    }

    fn nodes() -> Vec<NodeId> {
        vec![node(1), node(2), node(3)]
    }

    /// The property names an arrangement writes, in order.
    fn properties(kind: LayoutKind, operation: Operation) -> Vec<String> {
        apply(kind, &nodes(), operation)
            .expect("an expressible operation")
            .changes
            .iter()
            .map(|change| change.property.as_str().to_owned())
            .collect()
    }

    #[test]
    fn aligning_in_a_flowing_layout_sets_the_align_constraint() {
        let operation = Operation::Align {
            axis: Axis::Vertical,
            mode: Align::Centre,
        };
        assert_eq!(
            properties(LayoutKind::Row, operation),
            vec!["align", "align", "align"]
        );
    }

    #[test]
    fn aligning_in_a_free_layout_sets_a_position() {
        let operation = Operation::Align {
            axis: Axis::Horizontal,
            mode: Align::Start,
        };
        assert_eq!(properties(LayoutKind::Free, operation), vec!["x", "x", "x"]);
    }

    #[test]
    fn distributing_in_a_flow_sets_the_gap() {
        let operation = Operation::Distribute {
            axis: Axis::Horizontal,
        };
        assert_eq!(
            properties(LayoutKind::Row, operation),
            vec!["gap", "gap", "gap"]
        );
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
    fn matching_the_size_sets_width_or_height() {
        let horizontal = Operation::MatchSize {
            axis: Axis::Horizontal,
        };
        let vertical = Operation::MatchSize {
            axis: Axis::Vertical,
        };
        assert_eq!(properties(LayoutKind::Grid, horizontal), vec!["width"; 3]);
        assert_eq!(properties(LayoutKind::Grid, vertical), vec!["height"; 3]);
    }

    #[test]
    fn a_nudge_down_a_column_becomes_an_order_change() {
        let operation = Operation::Nudge {
            axis: Axis::Vertical,
            direction: 1,
            step: 8.0,
        };
        assert_eq!(properties(LayoutKind::Column, operation), vec!["order"; 3]);
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
    fn a_nudge_in_a_free_layout_sets_the_position() {
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
        assert_eq!(properties(LayoutKind::Free, horizontal), vec!["x"; 3]);
        assert_eq!(properties(LayoutKind::Free, vertical), vec!["y"; 3]);
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
        for kind in LayoutKind::ALL {
            let operation = Operation::MatchSize {
                axis: Axis::Horizontal,
            };
            assert!(apply(kind, &nodes(), operation).is_ok(), "{kind:?}");
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
}
