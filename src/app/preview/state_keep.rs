// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Preserving the live UI state across a rebuild: input, focus, scroll and selection — but only onto
//! a node that is still the same node *and* the same kind (task T14.03).
//!
//! The acceptance line is *"only a NodeId- and type-compatible instance restores, and the old
//! instance's state is not applied"*. The rule is therefore a match on two things, never on one:
//!
//! * **A capture is keyed by node and kind.** [`Kept`] records both, so restoring can compare them
//!   instead of trusting the id alone.
//! * **A mismatch is a notice, not a restore.** [`Restore::Incompatible`] names what it was and what
//!   it is now, and no state is applied — so a kind change cannot carry an old scroll or an old input
//!   into a widget that means something else.
//! * **A gone node is a notice.** [`Restore::Gone`] says the node is no longer there, so a rebuild
//!   that dropped it is visible rather than silently losing the state.
//! * **What was not captured is left alone.** A present node with no capture is not in the answer at
//!   all, so restoring never invents state for a widget nobody touched.

#![allow(dead_code)]

use crate::contracts::identity::NodeId;

/// The live state kept for one node.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Kept {
    /// The node.
    pub node: NodeId,
    /// The kind it was at capture time.
    pub kind: String,
    /// Whether it held focus.
    pub focus: bool,
    /// What was typed into it.
    pub input: String,
    /// Where it was scrolled to.
    pub scroll: u32,
    /// The selection, as a start and an end.
    pub selection: Option<(usize, usize)>,
}

/// Everything captured before the rebuild.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Capture {
    /// The captured nodes.
    pub entries: Vec<Kept>,
}

impl Capture {
    /// An empty capture.
    pub fn new() -> Self {
        Self::default()
    }

    /// The same capture with one more node.
    pub fn with(mut self, kept: Kept) -> Self {
        self.entries.push(kept);
        self
    }

    /// The capture for a node, if it was captured.
    pub fn get(&self, node: NodeId) -> Option<&Kept> {
        self.entries.iter().find(|kept| kept.node == node)
    }

    /// The capture in one line per node.
    pub fn render(&self) -> Vec<String> {
        self.entries
            .iter()
            .map(|kept| {
                format!(
                    "node {} ({}) focus={} scroll={} input={:?}",
                    kept.node.get(),
                    kept.kind,
                    kept.focus,
                    kept.scroll,
                    kept.input
                )
            })
            .collect()
    }
}

/// A node present after the rebuild.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Present {
    /// The node.
    pub node: NodeId,
    /// The kind it is now.
    pub kind: String,
}

impl Present {
    /// A present node of a kind.
    pub fn new(node: NodeId, kind: impl Into<String>) -> Self {
        Self {
            node,
            kind: kind.into(),
        }
    }
}

/// What became of one captured node.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Restore {
    /// The node and its kind match, so the state is applied.
    Restored(Kept),
    /// The node is gone.
    Gone {
        /// The node that is gone.
        node: NodeId,
    },
    /// The node's kind changed, so the old state is not applied.
    Incompatible {
        /// The node.
        node: NodeId,
        /// The kind it was.
        was: String,
        /// The kind it is now.
        now: String,
    },
}

impl Restore {
    /// Whether a restore happened.
    pub fn is_restored(&self) -> bool {
        matches!(self, Self::Restored(_))
    }

    /// The restore in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Restored(kept) => format!(
                "node {}: restored (scroll {}, input {:?})",
                kept.node.get(),
                kept.scroll,
                kept.input
            ),
            Self::Gone { node } => format!("node {}: gone", node.get()),
            Self::Incompatible { node, was, now } => format!(
                "node {}: incompatible — was {was}, is now {now}; the old state is not applied",
                node.get()
            ),
        }
    }
}

/// Decides, per captured node, whether its state may be restored.
pub fn restore(capture: &Capture, present: &[Present]) -> Vec<Restore> {
    capture
        .entries
        .iter()
        .map(
            |kept| match present.iter().find(|node| node.node == kept.node) {
                None => Restore::Gone { node: kept.node },
                Some(node) if node.kind == kept.kind => Restore::Restored(kept.clone()),
                Some(node) => Restore::Incompatible {
                    node: kept.node,
                    was: kept.kind.clone(),
                    now: node.kind.clone(),
                },
            },
        )
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Capture, Kept, Present, Restore, restore};
    use crate::contracts::identity::NodeId;

    fn node(value: u64) -> NodeId {
        NodeId::from_raw(value)
    }

    fn kept(value: u64, kind: &str) -> Kept {
        Kept {
            node: node(value),
            kind: kind.to_owned(),
            focus: true,
            input: "typed".to_owned(),
            scroll: 40,
            selection: Some((1, 4)),
        }
    }

    #[test]
    fn a_matching_node_and_kind_restores() {
        let capture = Capture::new().with(kept(1, "text-field"));
        let restores = restore(&capture, &[Present::new(node(1), "text-field")]);
        assert_eq!(restores.len(), 1);
        assert!(restores[0].is_restored());
        assert!(restores[0].render().contains("scroll 40"));
    }

    #[test]
    fn a_node_that_is_gone_is_reported() {
        let capture = Capture::new().with(kept(1, "text-field"));
        let restores = restore(&capture, &[]);
        assert_eq!(restores, vec![Restore::Gone { node: node(1) }]);
        assert!(restores[0].render().contains("gone"));
    }

    #[test]
    fn a_kind_change_is_incompatible_and_applies_nothing() {
        let capture = Capture::new().with(kept(1, "text-field"));
        let restores = restore(&capture, &[Present::new(node(1), "button")]);
        assert_eq!(
            restores,
            vec![Restore::Incompatible {
                node: node(1),
                was: "text-field".to_owned(),
                now: "button".to_owned(),
            }]
        );
        assert!(!restores[0].is_restored());
        assert!(restores[0].render().contains("not applied"));
    }

    #[test]
    fn an_empty_capture_restores_nothing() {
        assert!(restore(&Capture::new(), &[Present::new(node(1), "button")]).is_empty());
    }

    #[test]
    fn a_node_nobody_captured_is_not_in_the_answer() {
        let capture = Capture::new().with(kept(1, "text-field"));
        let restores = restore(
            &capture,
            &[
                Present::new(node(1), "text-field"),
                Present::new(node(2), "button"),
            ],
        );
        assert_eq!(restores.len(), 1);
    }

    #[test]
    fn focus_input_scroll_and_selection_are_carried() {
        let capture = Capture::new().with(kept(1, "text-field"));
        let Restore::Restored(state) =
            &restore(&capture, &[Present::new(node(1), "text-field")])[0]
        else {
            panic!("a restored state");
        };
        assert!(state.focus);
        assert_eq!(state.input, "typed");
        assert_eq!(state.scroll, 40);
        assert_eq!(state.selection, Some((1, 4)));
    }

    #[test]
    fn two_nodes_restore_independently() {
        let capture = Capture::new()
            .with(kept(1, "text-field"))
            .with(kept(2, "list"));
        let restores = restore(
            &capture,
            &[
                Present::new(node(2), "list"),
                Present::new(node(1), "switch"),
            ],
        );
        assert!(matches!(restores[0], Restore::Incompatible { .. }));
        assert!(restores[1].is_restored());
    }

    #[test]
    fn a_capture_can_be_looked_up_by_node() {
        let capture = Capture::new().with(kept(1, "text-field"));
        assert_eq!(
            capture.get(node(1)).map(|kept| kept.kind.as_str()),
            Some("text-field")
        );
        assert!(capture.get(node(9)).is_none());
    }

    #[test]
    fn the_capture_renders_what_it_holds() {
        let lines = Capture::new().with(kept(1, "text-field")).render();
        assert_eq!(
            lines,
            vec!["node 1 (text-field) focus=true scroll=40 input=\"typed\"".to_owned()]
        );
    }

    #[test]
    fn the_restores_render_each_outcome_differently() {
        let capture = Capture::new()
            .with(kept(1, "a"))
            .with(kept(2, "b"))
            .with(kept(3, "c"));
        let restores = restore(
            &capture,
            &[Present::new(node(1), "a"), Present::new(node(3), "z")],
        );
        assert!(restores[0].render().starts_with("node 1: restored"));
        assert_eq!(restores[1].render(), "node 2: gone");
        assert!(restores[2].render().contains("was c, is now z"));
    }
}
