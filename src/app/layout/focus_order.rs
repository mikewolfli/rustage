// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The focus order — the tab sequence — with keyboard navigation and unreachable-node warnings
//! (task T10.06).
//!
//! The acceptance line is *"exportable as the focus semantics the target supports"*. The tab
//! sequence is therefore one ordered list, and everything else is derived from the tree and stated
//! inputs:
//!
//! * **The order is one list, exported as it is.** [`FocusOrder::order`] is the tab sequence and
//!   [`FocusOrder::export`] hands it to the generator unchanged, so the preview and the generated app
//!   cannot disagree about where Tab goes.
//! * **Reachability is computed, not declared.** A candidate is unreachable when it or an ancestor is
//!   excluded, and [`build`] lists those separately — so an unreachable focus target is reported, not
//!   silently dropped from the sequence.
//! * **A node that cannot take focus is never in the order.** [`build`] starts from the focusable set,
//!   so a decoration cannot become a tab stop by sitting between two real ones.
//! * **An explicit index wins, the tree decides the rest.** A node with a `tab index` sorts by it
//!   first; everything else follows the tree's own order, which is what "natural order" means.

#![allow(dead_code)]

use std::collections::BTreeMap;

use crate::app::canvas::structure::Tree;
use crate::contracts::identity::NodeId;

/// The tab sequence, and what could not be reached.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct FocusOrder {
    order: Vec<NodeId>,
    unreachable: Vec<NodeId>,
}

impl FocusOrder {
    /// The tab sequence.
    pub fn order(&self) -> &[NodeId] {
        &self.order
    }

    /// The focusable nodes that cannot be reached from anywhere.
    pub fn unreachable(&self) -> &[NodeId] {
        &self.unreachable
    }

    /// The sequence, unchanged, for the generator — the focus semantics the target gets.
    pub fn export(&self) -> Vec<NodeId> {
        self.order.clone()
    }

    /// Where Tab goes from a node, wrapping at the end.
    pub fn next(&self, node: NodeId) -> Option<NodeId> {
        self.step(node, 1)
    }

    /// Where Shift-Tab goes from a node, wrapping at the start.
    pub fn previous(&self, node: NodeId) -> Option<NodeId> {
        self.step(node, -1)
    }

    /// Whether a node is in the sequence.
    pub fn contains(&self, node: NodeId) -> bool {
        self.order.contains(&node)
    }

    /// The order in a report — the navigation, drawn as a chain.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!("focus order: {} stop(s)", self.order.len())];
        let chain: Vec<String> = self
            .order
            .iter()
            .map(|node| node.get().to_string())
            .collect();
        lines.push(format!("  {}", chain.join(" -> ")));
        lines.push(format!("  unreachable: {}", self.unreachable.len()));
        for node in &self.unreachable {
            lines.push(format!("    node {} cannot be reached", node.get()));
        }
        lines
    }

    /// Moves `delta` stops through the sequence, wrapping.
    fn step(&self, node: NodeId, delta: isize) -> Option<NodeId> {
        let position = self.order.iter().position(|existing| *existing == node)?;
        let len = self.order.len() as isize;
        Some(self.order[(position as isize + delta).rem_euclid(len) as usize])
    }
}

/// Builds the tab sequence from a tree, the focusable nodes, the excluded ones and any explicit index.
///
/// `excluded` holds the nodes that cannot be reached *and everything under them*: a focus stop inside
/// a hidden or disabled container is unreachable, and this is what says so.
pub fn build(
    tree: &Tree,
    focusable: &[NodeId],
    excluded: &[NodeId],
    explicit: &BTreeMap<NodeId, i64>,
) -> FocusOrder {
    let mut reachable = Vec::new();
    let mut unreachable = Vec::new();
    for node in in_tree_order(tree) {
        if !focusable.contains(&node) {
            continue;
        }
        if is_excluded(tree, node, excluded) {
            unreachable.push(node);
        } else {
            reachable.push(node);
        }
    }
    // An explicit index sorts first, by its value; the rest keep the tree's order.
    reachable.sort_by_key(|node| explicit.get(node).map_or((1, 0), |index| (0, *index)));
    FocusOrder {
        order: reachable,
        unreachable,
    }
}

/// The tree's nodes in depth-first order, which is the natural tab order.
fn in_tree_order(tree: &Tree) -> Vec<NodeId> {
    let mut ordered = Vec::new();
    let mut stack: Vec<NodeId> = tree.roots().iter().rev().copied().collect();
    while let Some(node) = stack.pop() {
        ordered.push(node);
        if let Some(holder) = tree.get(node) {
            for child in holder.children.iter().rev() {
                stack.push(*child);
            }
        }
    }
    ordered
}

/// Whether a node or any of its ancestors is excluded.
fn is_excluded(tree: &Tree, node: NodeId, excluded: &[NodeId]) -> bool {
    let mut current = Some(node);
    while let Some(node) = current {
        if excluded.contains(&node) {
            return true;
        }
        current = tree.get(node).and_then(|holder| holder.parent);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::{FocusOrder, build};
    use crate::app::canvas::structure::{Kind, Tree};
    use crate::contracts::identity::NodeId;
    use std::collections::BTreeMap;

    fn node(value: u64) -> NodeId {
        NodeId::from_raw(value)
    }

    /// A container with three widgets under it, in order.
    fn tree() -> Tree {
        let mut tree = Tree::new();
        let panel = tree.insert(None, Kind::Container, "panel").expect("root");
        for name in ["a", "b", "c"] {
            tree.insert(Some(panel), Kind::Widget, name).expect("child");
        }
        tree
    }

    fn order_of(order: &FocusOrder) -> Vec<u64> {
        order.order().iter().map(|node| node.get()).collect()
    }

    #[test]
    fn focusable_nodes_follow_the_tree_order() {
        let tree = tree();
        let order = build(&tree, &[node(1), node(2), node(3)], &[], &BTreeMap::new());
        assert_eq!(order_of(&order), vec![1, 2, 3]);
        assert!(order.unreachable().is_empty());
    }

    #[test]
    fn a_node_under_an_excluded_ancestor_is_unreachable() {
        let tree = tree();
        let order = build(
            &tree,
            &[node(1), node(2), node(3)],
            &[node(0)],
            &BTreeMap::new(),
        );
        assert!(
            order_of(&order).is_empty(),
            "nothing under the panel is reachable"
        );
        assert_eq!(order.unreachable(), &[node(1), node(2), node(3)]);
    }

    #[test]
    fn an_excluded_node_itself_is_unreachable() {
        let tree = tree();
        let order = build(&tree, &[node(1), node(2)], &[node(1)], &BTreeMap::new());
        assert_eq!(order_of(&order), vec![2]);
        assert_eq!(order.unreachable(), &[node(1)]);
    }

    #[test]
    fn a_non_focusable_node_is_never_in_the_order() {
        let tree = tree();
        let order = build(&tree, &[node(1), node(3)], &[], &BTreeMap::new());
        assert_eq!(order_of(&order), vec![1, 3]);
        assert!(!order.contains(node(2)));
        assert!(!order.contains(node(0)), "the panel is not a stop");
    }

    #[test]
    fn an_explicit_index_sorts_a_node_first() {
        let tree = tree();
        let mut explicit = BTreeMap::new();
        explicit.insert(node(3), 1);
        let order = build(&tree, &[node(1), node(2), node(3)], &[], &explicit);
        assert_eq!(order_of(&order), vec![3, 1, 2]);
    }

    #[test]
    fn two_explicit_indices_sort_by_their_values() {
        let tree = tree();
        let mut explicit = BTreeMap::new();
        explicit.insert(node(2), 5);
        explicit.insert(node(3), -1);
        let order = build(&tree, &[node(1), node(2), node(3)], &[], &explicit);
        assert_eq!(order_of(&order), vec![3, 2, 1]);
    }

    #[test]
    fn tab_wraps_at_the_end_and_shift_tab_at_the_start() {
        let tree = tree();
        let order = build(&tree, &[node(1), node(2), node(3)], &[], &BTreeMap::new());
        assert_eq!(order.next(node(1)), Some(node(2)));
        assert_eq!(order.next(node(3)), Some(node(1)), "wraps");
        assert_eq!(order.previous(node(1)), Some(node(3)), "wraps back");
        assert_eq!(
            order.next(node(9)),
            None,
            "a node not in the order has no next"
        );
    }

    #[test]
    fn the_exported_sequence_is_the_order() {
        let tree = tree();
        let order = build(&tree, &[node(1), node(2), node(3)], &[], &BTreeMap::new());
        assert_eq!(order.export(), order.order().to_vec());
    }

    #[test]
    fn the_report_draws_the_navigation_as_a_chain() {
        let tree = tree();
        let order = build(&tree, &[node(1), node(2)], &[node(0)], &BTreeMap::new());
        let rendered = order.render();
        assert!(rendered[0].contains("0 stop(s)"));
        assert!(rendered.iter().any(|line| line.contains("unreachable: 2")));
    }
}
