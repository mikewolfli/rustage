// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! A node's box constraints — minimum, maximum, preferred, grow, margin, gap and overflow — and the
//! validation that locates an illegal or conflicting one (task T10.03).
//!
//! The acceptance line is *"an illegal or conflicting constraint is located at the specific node"*.
//! A constraint set is checked as a whole, and every problem is reported the same way the layout
//! diagnostics are — node, property and origin — so a problem is a *location*, not just a sentence:
//!
//! * **A violation carries the node and the property.** [`BoxConstraints::validate`] returns
//!   [`Diagnostic`]s whose [`Source`] names the node and the exact property at fault, so the panel
//!   can point at the field rather than at the node in general.
//! * **An unsatisfiable set is refused, never silently clamped.** This module *finds* the conflict;
//!   it does not pick a winner, because quietly narrowing a `max` would be an edit nobody asked for.
//! * **The bounds are half-open in the right direction.** A minimum above the maximum is a conflict,
//!   and a preferred size outside the range is one too, so the two ways a size can contradict itself
//!   are both caught.
//! * **Overflow is a setting with a consequence.** When content is larger than the box and the box
//!   does not scroll, the overflow setting is what says so, so the diagnostic can name it.

#![allow(dead_code)]

use crate::app::layout_plan::{Diagnostic, Origin, Size, Source};
use crate::contracts::identity::NodeId;
use crate::contracts::model::PropertyName;

/// What a box does when its content is larger than it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Overflow {
    /// Content spills out.
    Visible,
    /// Content is hidden.
    Hidden,
    /// Content can be scrolled.
    Scroll,
    /// Content is clipped.
    Clip,
}

impl Overflow {
    /// Every kind, in a stable order.
    pub const ALL: [Self; 4] = [Self::Visible, Self::Hidden, Self::Scroll, Self::Clip];

    /// The kind's name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Visible => "visible",
            Self::Hidden => "hidden",
            Self::Scroll => "scroll",
            Self::Clip => "clip",
        }
    }

    /// The kind a name picks.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|kind| kind.name() == name)
    }

    /// Whether the box keeps content reachable when it does not fit.
    pub const fn keeps_content_reachable(self) -> bool {
        matches!(self, Self::Scroll)
    }
}

/// A node's box constraints. Each is optional; an unset one constrains nothing.
#[derive(Clone, PartialEq, Debug)]
pub struct BoxConstraints {
    /// The node.
    pub node: NodeId,
    /// The smallest it may be.
    pub minimum: Option<Size>,
    /// The largest it may be.
    pub maximum: Option<Size>,
    /// The size it would like.
    pub preferred: Option<Size>,
    /// How much spare room it takes, relative to its siblings.
    pub grow: Option<f64>,
    /// The space left around it.
    pub margin: Option<f64>,
    /// The space between its children (for a container).
    pub gap: Option<f64>,
    /// What happens when content does not fit.
    pub overflow: Option<Overflow>,
}

impl BoxConstraints {
    /// No constraints on a node.
    pub fn new(node: NodeId) -> Self {
        Self {
            node,
            minimum: None,
            maximum: None,
            preferred: None,
            grow: None,
            margin: None,
            gap: None,
            overflow: None,
        }
    }

    /// Every problem with this set, each located at its node and property.
    pub fn validate(&self) -> Vec<Diagnostic> {
        let mut problems = Vec::new();
        self.check_range(&mut problems);
        self.check_non_negative(&mut problems);
        self.check_overflow(&mut problems);
        problems
    }

    /// At `width`/`height`: does the minimum exceed the maximum, or the preferred size fall outside
    /// whichever bound is set?
    fn check_range(&self, problems: &mut Vec<Diagnostic>) {
        if let (Some(minimum), Some(maximum)) = (self.minimum, self.maximum) {
            if minimum.width > maximum.width {
                problems.push(
                    self.problem("min-width", "the minimum width is above the maximum width"),
                );
            }
            if minimum.height > maximum.height {
                problems.push(self.problem(
                    "min-height",
                    "the minimum height is above the maximum height",
                ));
            }
        }
        let Some(preferred) = self.preferred else {
            return;
        };
        if let Some(maximum) = self.maximum {
            if preferred.width > maximum.width {
                problems.push(self.problem(
                    "preferred-width",
                    "the preferred width is above the maximum width",
                ));
            }
            if preferred.height > maximum.height {
                problems.push(self.problem(
                    "preferred-height",
                    "the preferred height is above the maximum height",
                ));
            }
        }
        if let Some(minimum) = self.minimum {
            if preferred.width < minimum.width {
                problems.push(self.problem(
                    "preferred-width",
                    "the preferred width is below the minimum width",
                ));
            }
            if preferred.height < minimum.height {
                problems.push(self.problem(
                    "preferred-height",
                    "the preferred height is below the minimum height",
                ));
            }
        }
    }

    /// At `grow`, `margin` and `gap`: is any of them negative?
    fn check_non_negative(&self, problems: &mut Vec<Diagnostic>) {
        if let Some(grow) = self.grow
            && grow < 0.0
        {
            problems.push(self.problem("grow", "the grow factor is negative"));
        }
        if let Some(margin) = self.margin
            && margin < 0.0
        {
            problems.push(self.problem("margin", "the margin is negative"));
        }
        if let Some(gap) = self.gap
            && gap < 0.0
        {
            problems.push(self.problem("gap", "the gap is negative"));
        }
    }

    /// When the content wants more than the box allows, does the overflow say how it is reached?
    fn check_overflow(&self, problems: &mut Vec<Diagnostic>) {
        let (Some(preferred), Some(maximum), Some(overflow)) =
            (self.preferred, self.maximum, self.overflow)
        else {
            return;
        };
        let too_wide = preferred.width > maximum.width;
        let too_tall = preferred.height > maximum.height;
        if (too_wide || too_tall) && !overflow.keeps_content_reachable() {
            problems.push(self.problem(
                "overflow",
                "content is larger than the box and the overflow setting does not scroll it",
            ));
        }
    }

    /// A problem about a property of this node.
    fn problem(&self, property: &str, message: &str) -> Diagnostic {
        let source = Source {
            node: self.node,
            property: PropertyName::new(property).expect("a fixed property name"),
            origin: Origin::Explicit,
        };
        Diagnostic::new(self.node, message).at(source)
    }

    /// The constraints in a report.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!("constraints node {}:", self.node.get())];
        if let Some(minimum) = self.minimum {
            lines.push(format!("  min {},{}", minimum.width, minimum.height));
        }
        if let Some(maximum) = self.maximum {
            lines.push(format!("  max {},{}", maximum.width, maximum.height));
        }
        if let Some(preferred) = self.preferred {
            lines.push(format!(
                "  preferred {},{}",
                preferred.width, preferred.height
            ));
        }
        if let Some(grow) = self.grow {
            lines.push(format!("  grow {grow}"));
        }
        if let Some(margin) = self.margin {
            lines.push(format!("  margin {margin}"));
        }
        if let Some(gap) = self.gap {
            lines.push(format!("  gap {gap}"));
        }
        if let Some(overflow) = self.overflow {
            lines.push(format!("  overflow {}", overflow.name()));
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{BoxConstraints, Overflow};
    use crate::app::layout_plan::{Diagnostic, Size};
    use crate::contracts::identity::NodeId;

    fn node(value: u64) -> NodeId {
        NodeId::from_raw(value)
    }

    fn problems(constraints: &BoxConstraints) -> Vec<Diagnostic> {
        constraints.validate()
    }

    /// The property names the diagnostics point at, in order.
    fn properties(diagnostics: &[Diagnostic]) -> Vec<String> {
        diagnostics
            .iter()
            .filter_map(|diagnostic| diagnostic.source.as_ref())
            .map(|source| source.property.as_str().to_owned())
            .collect()
    }

    #[test]
    fn a_valid_set_has_no_diagnostics() {
        let constraints = BoxConstraints {
            minimum: Some(Size::new(10.0, 10.0)),
            maximum: Some(Size::new(100.0, 100.0)),
            preferred: Some(Size::new(50.0, 40.0)),
            grow: Some(1.0),
            margin: Some(4.0),
            gap: Some(8.0),
            overflow: Some(Overflow::Scroll),
            ..BoxConstraints::new(node(1))
        };
        assert!(problems(&constraints).is_empty());
    }

    #[test]
    fn a_minimum_above_the_maximum_is_located_at_the_minimum() {
        let constraints = BoxConstraints {
            minimum: Some(Size::new(100.0, 10.0)),
            maximum: Some(Size::new(50.0, 100.0)),
            ..BoxConstraints::new(node(2))
        };
        let found = problems(&constraints);
        assert_eq!(found.len(), 1);
        assert_eq!(properties(&found), vec!["min-width"]);
        assert_eq!(found[0].node, node(2), "the node is named");
        assert_eq!(
            found[0].jump().map(|source| source.node),
            Some(node(2)),
            "the source is the node"
        );
    }

    #[test]
    fn a_preferred_outside_the_range_is_located_at_the_preferred_size() {
        let constraints = BoxConstraints {
            minimum: Some(Size::new(10.0, 10.0)),
            maximum: Some(Size::new(50.0, 50.0)),
            preferred: Some(Size::new(80.0, 40.0)),
            ..BoxConstraints::new(node(3))
        };
        assert_eq!(properties(&problems(&constraints)), vec!["preferred-width"]);
    }

    #[test]
    fn a_negative_grow_margin_and_gap_are_each_located() {
        let constraints = BoxConstraints {
            grow: Some(-1.0),
            margin: Some(-2.0),
            gap: Some(-3.0),
            ..BoxConstraints::new(node(4))
        };
        assert_eq!(
            properties(&problems(&constraints)),
            vec!["grow", "margin", "gap"]
        );
    }

    #[test]
    fn content_larger_than_a_non_scrolling_box_is_located_at_the_overflow() {
        let constraints = BoxConstraints {
            maximum: Some(Size::new(50.0, 50.0)),
            preferred: Some(Size::new(80.0, 50.0)),
            overflow: Some(Overflow::Clip),
            ..BoxConstraints::new(node(5))
        };
        // The preferred size is over the maximum *and* the clip would not keep the content reachable,
        // so both are true and both are located.
        assert_eq!(
            properties(&problems(&constraints)),
            vec!["preferred-width", "overflow"]
        );
    }

    #[test]
    fn a_scrolling_box_that_fits_is_not_a_problem() {
        let constraints = BoxConstraints {
            maximum: Some(Size::new(50.0, 50.0)),
            preferred: Some(Size::new(40.0, 40.0)),
            overflow: Some(Overflow::Scroll),
            ..BoxConstraints::new(node(5))
        };
        assert!(problems(&constraints).is_empty());
    }

    #[test]
    fn a_preferred_size_with_only_a_maximum_is_still_checked() {
        let constraints = BoxConstraints {
            maximum: Some(Size::new(50.0, 50.0)),
            preferred: Some(Size::new(80.0, 40.0)),
            ..BoxConstraints::new(node(6))
        };
        assert_eq!(properties(&problems(&constraints)), vec!["preferred-width"]);
    }

    #[test]
    fn a_preferred_size_with_only_a_minimum_is_still_checked() {
        let constraints = BoxConstraints {
            minimum: Some(Size::new(100.0, 100.0)),
            preferred: Some(Size::new(80.0, 40.0)),
            ..BoxConstraints::new(node(7))
        };
        assert_eq!(
            properties(&problems(&constraints)),
            vec!["preferred-width", "preferred-height"]
        );
    }

    #[test]
    fn every_overflow_name_round_trips() {
        for kind in Overflow::ALL {
            assert_eq!(Overflow::from_name(kind.name()), Some(kind));
        }
        assert!(Overflow::from_name("cover").is_none());
    }

    #[test]
    fn only_scroll_keeps_content_reachable() {
        assert!(Overflow::Scroll.keeps_content_reachable());
        assert!(!Overflow::Visible.keeps_content_reachable());
        assert!(!Overflow::Hidden.keeps_content_reachable());
        assert!(!Overflow::Clip.keeps_content_reachable());
    }
}
