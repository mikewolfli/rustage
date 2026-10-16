// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Choosing, switching and detaching a container's layout, and migrating its children's constraints
//! (task T10.02).
//!
//! The acceptance line is *"a conversion that cannot be expressed is refused with the loss
//! explained"*. A layout kind is a *vocabulary* — which constraints it can hold — so switching is a
//! question of whether every child's constraint has a word in the new vocabulary:
//!
//! * **Each kind declares what it can express, once.** [`LayoutKind::expresses`] is the single rule;
//!   the switch, the migration and the detach all ask it, so they cannot disagree about what a layout
//!   can hold.
//! * **A switch keeps what the new layout can say and reports what it cannot.** [`switch`] returns a
//!   [`Migration`] of the constraints that survive, or a [`Refusal`] naming every [`Loss`] — node and
//!   property — so "this would not be expressible" is a fact with a location, not a silent drop.
//! * **Detaching is switching to the free layout.** [`detach`] is [`switch`] to [`LayoutKind::Free`],
//!   so "take the layout off" has no separate rule of its own to drift.
//! * **Nothing is migrated implicitly.** A switch moves a constraint only when the new kind can hold
//!   the *same* property; there is no re-interpretation that quietly turns an `x` into something else.

#![allow(dead_code)]

use crate::app::layout::layout_plan::Constraint;
use crate::contracts::identity::NodeId;
use crate::contracts::model::PropertyName;

/// What a container's layout can express.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LayoutKind {
    /// Children in a row.
    Row,
    /// Children in a column.
    Column,
    /// Children layered on top of each other.
    Stack,
    /// Children in a grid.
    Grid,
    /// Children positioned freely.
    Free,
}

impl LayoutKind {
    /// Every kind, in a stable order.
    pub const ALL: [Self; 5] = [Self::Row, Self::Column, Self::Stack, Self::Grid, Self::Free];

    /// The kind's name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Row => "row",
            Self::Column => "column",
            Self::Stack => "stack",
            Self::Grid => "grid",
            Self::Free => "free",
        }
    }

    /// The kind a name picks.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|kind| kind.name() == name)
    }

    /// Whether this layout can hold a constraint on a property. The one vocabulary rule.
    pub fn expresses(self, property: &PropertyName) -> bool {
        let name = property.as_str();
        match self {
            // Free positioning is the only layout that can say *where*.
            Self::Free => matches!(name, "x" | "y" | "width" | "height" | "margin"),
            // A stack also places, but within the space it is given.
            Self::Stack => matches!(name, "x" | "y" | "width" | "height" | "margin" | "align"),
            // Flowing layouts say how much room to take, where to sit in the flow, and how to align
            // within the cross axis.
            Self::Row | Self::Column => {
                matches!(
                    name,
                    "width" | "height" | "grow" | "margin" | "gap" | "order" | "align"
                )
            }
            // A grid additionally has gaps and order.
            Self::Grid => matches!(
                name,
                "width" | "height" | "margin" | "gap" | "order" | "align"
            ),
        }
    }
}

/// Where a container is now, and what its children are constrained by.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Container {
    /// The container's node.
    pub node: NodeId,
    /// Its current layout, or `None` when it has none yet.
    pub current: Option<LayoutKind>,
    /// Its children's constraints.
    pub children: Vec<Constraint>,
}

impl Container {
    /// A container with no layout and no constrained children.
    pub fn new(node: NodeId) -> Self {
        Self {
            node,
            current: None,
            children: Vec::new(),
        }
    }

    /// The same container with one more child constraint.
    pub fn with(mut self, constraint: Constraint) -> Self {
        self.children.push(constraint);
        self
    }

    /// The same container with a current layout.
    pub fn laid_out(mut self, kind: LayoutKind) -> Self {
        self.current = Some(kind);
        self
    }
}

/// One constraint the new layout cannot hold.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Loss {
    /// The child whose constraint is lost.
    pub node: NodeId,
    /// The property that cannot be expressed.
    pub property: PropertyName,
    /// Why.
    pub reason: String,
}

impl Loss {
    /// The loss in one line.
    pub fn render(&self) -> String {
        format!(
            "node {} loses {} — {}",
            self.node.get(),
            self.property.as_str(),
            self.reason
        )
    }
}

/// A switch that cannot be made, and everything it would lose.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Refusal {
    /// The layout that was asked for.
    pub to: LayoutKind,
    /// Every constraint that would be lost.
    pub losses: Vec<Loss>,
}

impl Refusal {
    /// The refusal in a report.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "switch to {}: refused, {} constraint(s) cannot be expressed",
            self.to.name(),
            self.losses.len()
        )];
        lines.extend(
            self.losses
                .iter()
                .map(|loss| format!("  {}", loss.render())),
        );
        lines
    }
}

/// A switch that can be made: the constraints the new layout keeps.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Migration {
    /// The layout being moved to.
    pub to: LayoutKind,
    /// The constraints that survive, as `(node, property)`.
    pub kept: Vec<(NodeId, PropertyName)>,
}

impl Migration {
    /// The migration in a report.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "switch to {}: {} constraint(s) kept",
            self.to.name(),
            self.kept.len()
        )];
        for (node, property) in &self.kept {
            lines.push(format!("  node {} keeps {}", node.get(), property.as_str()));
        }
        lines
    }
}

/// Chooses `to` for a container, keeping every child constraint it can express.
///
/// Refuses when any child constraint has no word in the new layout, naming each loss.
pub fn switch(to: LayoutKind, children: &[Constraint]) -> Result<Migration, Refusal> {
    let mut kept = Vec::new();
    let mut losses = Vec::new();
    for constraint in children {
        if to.expresses(&constraint.property) {
            kept.push((constraint.node, constraint.property.clone()));
        } else {
            losses.push(Loss {
                node: constraint.node,
                property: constraint.property.clone(),
                reason: format!(
                    "a {} layout cannot express {}",
                    to.name(),
                    constraint.property.as_str()
                ),
            });
        }
    }
    if losses.is_empty() {
        Ok(Migration { to, kept })
    } else {
        Err(Refusal { to, losses })
    }
}

/// Takes the layout off a container: a switch to the free layout.
pub fn detach(children: &[Constraint]) -> Result<Migration, Refusal> {
    switch(LayoutKind::Free, children)
}

#[cfg(test)]
mod tests {
    use super::{LayoutKind, Loss, detach, switch};
    use crate::app::layout::layout_plan::{Constraint, Origin};
    use crate::contracts::identity::NodeId;
    use crate::contracts::model::PropertyName;

    fn node(value: u64) -> NodeId {
        NodeId::from_raw(value)
    }

    fn name(text: &str) -> PropertyName {
        PropertyName::new(text).expect("a name")
    }

    fn constraint(node: NodeId, property: &str) -> Constraint {
        Constraint::new(node, name(property), Origin::Explicit)
    }

    #[test]
    fn a_flowing_switch_keeps_the_constraints_both_flows_can_say() {
        let children = [constraint(node(1), "grow"), constraint(node(2), "width")];
        let migration = switch(LayoutKind::Column, &children).expect("both are expressible");
        assert_eq!(migration.kept.len(), 2);
        assert_eq!(migration.to, LayoutKind::Column);
    }

    #[test]
    fn a_switch_that_would_lose_a_position_is_refused_and_names_it() {
        let children = [constraint(node(1), "x"), constraint(node(2), "width")];
        let refusal = switch(LayoutKind::Row, &children).expect_err("x has no word in a row");
        assert_eq!(refusal.to, LayoutKind::Row);
        assert_eq!(refusal.losses.len(), 1);
        assert_eq!(refusal.losses[0].node, node(1));
        assert_eq!(refusal.losses[0].property.as_str(), "x");
        assert!(
            refusal
                .render()
                .first()
                .is_some_and(|line| line.contains("refused"))
        );
    }

    #[test]
    fn detaching_to_free_refuses_a_grow_because_free_cannot_say_it() {
        let children = [constraint(node(1), "grow")];
        let refusal = detach(&children).expect_err("free has no grow");
        assert_eq!(refusal.losses.len(), 1);
        assert!(refusal.losses[0].render().contains("grow"));
    }

    #[test]
    fn detaching_keeps_positions_that_free_expresses() {
        let children = [constraint(node(1), "x"), constraint(node(2), "width")];
        let migration = detach(&children).expect("free expresses both");
        assert_eq!(migration.to, LayoutKind::Free);
        assert_eq!(migration.kept.len(), 2);
    }

    #[test]
    fn free_expresses_a_position_and_a_flow_does_not() {
        assert!(LayoutKind::Free.expresses(&name("x")));
        assert!(!LayoutKind::Row.expresses(&name("x")));
        assert!(LayoutKind::Row.expresses(&name("grow")));
        assert!(!LayoutKind::Free.expresses(&name("grow")));
        assert!(LayoutKind::Grid.expresses(&name("gap")));
        assert!(!LayoutKind::Free.expresses(&name("gap")));
    }

    #[test]
    fn every_kind_name_round_trips() {
        for kind in LayoutKind::ALL {
            assert_eq!(LayoutKind::from_name(kind.name()), Some(kind));
        }
        assert!(LayoutKind::from_name("absolute").is_none());
    }

    #[test]
    fn a_switch_with_nothing_to_lose_keeps_everything() {
        let children = [constraint(node(1), "width"), constraint(node(2), "height")];
        let migration = switch(LayoutKind::Stack, &children).expect("a stack expresses both");
        assert_eq!(migration.kept.len(), 2);
    }

    #[test]
    fn a_refusal_reports_every_loss_not_just_the_first() {
        let children = [
            constraint(node(1), "x"),
            constraint(node(2), "y"),
            constraint(node(3), "width"),
        ];
        let refusal = switch(LayoutKind::Column, &children).expect_err("two positions are lost");
        assert_eq!(refusal.losses.len(), 2);
        let rendered = refusal.render();
        assert_eq!(rendered.len(), 3, "a header and two losses");
    }

    #[test]
    fn the_loss_type_names_the_property_and_the_reason() {
        let loss = Loss {
            node: node(4),
            property: name("gap"),
            reason: "free cannot express gap".to_owned(),
        };
        assert!(loss.render().contains("gap"));
        assert!(loss.render().contains("node 4"));
    }
}
