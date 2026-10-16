// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Selection: single, multi, marquee, parent-child navigation and tree/canvas linkage (task T09.03).
//!
//! The acceptance line is *"overlapping nodes can be selected, and hidden is not confused with
//! locked"*. The second half is the one that is easy to get wrong, so the module works on the
//! [`Scene`]'s own distinction rather than inventing a second one:
//!
//! * **The selection is one set, shared by the tree and the canvas.** [`Selection`] holds node ids;
//!   both surfaces read the same set, so "the tree and the canvas agree" is structural.
//! * **Hidden and locked are different, and both are the scene's.** A hidden node is not drawn (so
//!   [`Scene::hit`] misses it) and a locked node is drawn but not selectable; [`Selection::click`]
//!   refuses a locked node and [`Selection::marquee`] never picks one. Clearing one flag never clears
//!   the other.
//! * **Overlapping nodes can be selected.** [`Selection::click`] picks the topmost; [`Selection::cycle`]
//!   walks down through the nodes under a point, so a node under a bigger one is still reachable.
//! * **Navigation is the scene's hierarchy.** [`Selection::parent_of`]/[`children_of`] read the scene's
//!   parent links, so the tree and the canvas cannot disagree about who is whose child.

#![allow(dead_code)]

use std::collections::BTreeSet;

use crate::app::canvas::scene::{Rect, Scene};
use crate::contracts::identity::NodeId;

/// The set of selected nodes, one set for the whole workbench.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Selection {
    set: BTreeSet<NodeId>,
    anchor: Option<NodeId>,
}

impl Selection {
    /// An empty selection.
    pub fn new() -> Self {
        Self::default()
    }

    /// The selected ids, in id order.
    pub fn ids(&self) -> Vec<NodeId> {
        self.set.iter().copied().collect()
    }

    /// How many nodes are selected.
    pub fn len(&self) -> usize {
        self.set.len()
    }

    /// Whether nothing is selected.
    pub fn is_empty(&self) -> bool {
        self.set.is_empty()
    }

    /// Whether a node is selected.
    pub fn contains(&self, node: NodeId) -> bool {
        self.set.contains(&node)
    }

    /// The anchor: the node a shift-extend ranges from.
    pub fn anchor(&self) -> Option<NodeId> {
        self.anchor
    }

    /// Clears the selection.
    pub fn clear(&mut self) {
        self.set.clear();
        self.anchor = None;
    }

    /// Selects one node, replacing the selection. Refuses a node that is not selectable.
    pub fn click(&mut self, scene: &Scene, node: NodeId) -> bool {
        if !scene.is_selectable(node) {
            return false;
        }
        self.set.clear();
        self.set.insert(node);
        self.anchor = Some(node);
        true
    }

    /// Adds or removes a node (a modifier click), leaving the rest of the selection.
    pub fn toggle(&mut self, scene: &Scene, node: NodeId) -> bool {
        if !scene.is_selectable(node) {
            return false;
        }
        if self.set.contains(&node) {
            self.set.remove(&node);
        } else {
            self.set.insert(node);
        }
        self.anchor = Some(node);
        true
    }

    /// Replaces the selection with every selectable node overlapping a marquee.
    pub fn marquee(&mut self, scene: &Scene, rect: Rect) -> Vec<NodeId> {
        self.set = scene.within(rect).into_iter().collect();
        self.anchor = self.set.iter().next().copied();
        self.ids()
    }

    /// Selects every selectable node.
    pub fn select_all(&mut self, scene: &Scene) {
        self.set = scene
            .order()
            .iter()
            .copied()
            .filter(|node| scene.is_selectable(*node))
            .collect();
        self.anchor = self.set.iter().next().copied();
    }

    /// Cycles the pick at a point: the topmost first, then the ones under it, so overlapping nodes are
    /// all reachable.
    pub fn cycle(&mut self, scene: &Scene, x: i32, y: i32) -> Option<NodeId> {
        let stack: Vec<NodeId> = scene
            .order()
            .iter()
            .rev()
            .copied()
            .filter(|node| {
                scene.is_selectable(*node)
                    && scene
                        .instance(*node)
                        .and_then(|handle| scene.resolve(handle).ok())
                        .is_some_and(|rect| rect.contains(x, y))
            })
            .collect();
        if stack.is_empty() {
            return None;
        }
        // The next pick after whatever is currently selected, wrapping.
        let next = match self
            .set
            .iter()
            .find_map(|node| stack.iter().position(|c| c == node))
        {
            Some(position) => stack[(position + 1) % stack.len()],
            None => stack[0],
        };
        self.set.clear();
        self.set.insert(next);
        self.anchor = Some(next);
        Some(next)
    }

    /// A node's parent (the tree's parent, the canvas's container).
    pub fn parent_of(&self, scene: &Scene, node: NodeId) -> Option<NodeId> {
        scene.parent(node)
    }

    /// A node's children, in paint order.
    pub fn children_of(&self, scene: &Scene, node: NodeId) -> Vec<NodeId> {
        scene.children(node)
    }

    /// Adds every descendant of a node to the selection (a recursive pick).
    pub fn select_subtree(&mut self, scene: &Scene, node: NodeId) {
        let mut stack = vec![node];
        while let Some(current) = stack.pop() {
            if scene.is_selectable(current) {
                self.set.insert(current);
            }
            stack.extend(scene.children(current));
        }
        self.anchor = Some(node);
    }

    /// Drops the nodes that are no longer selectable, so a selection survives a hide or a lock.
    pub fn prune(&mut self, scene: &Scene) -> Vec<NodeId> {
        let dropped: Vec<NodeId> = self
            .set
            .iter()
            .copied()
            .filter(|node| !scene.is_selectable(*node))
            .collect();
        for node in &dropped {
            self.set.remove(node);
        }
        dropped
    }

    /// The selection in one report.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!("selection: {} node(s)", self.set.len())];
        for node in &self.set {
            lines.push(format!("  node {}", node.get()));
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::Selection;
    use crate::app::canvas::scene::{Rect, Scene};
    use crate::contracts::identity::NodeId;

    fn node(value: u64) -> NodeId {
        NodeId::from_raw(value)
    }

    fn scene() -> Scene {
        let mut scene = Scene::new();
        scene.insert(node(1), Rect::new(0, 0, 100, 100));
        scene.insert(node(2), Rect::new(0, 0, 100, 100));
        scene.set_parent(node(2), Some(node(1))).expect("parent");
        scene.insert(node(3), Rect::new(200, 200, 10, 10));
        scene
    }

    #[test]
    fn overlapping_nodes_are_all_selectable_by_cycling() {
        let scene = scene();
        let mut selection = Selection::new();
        // The first pick is the topmost.
        assert_eq!(selection.cycle(&scene, 50, 50), Some(node(2)));
        // The next pick is the one under it.
        assert_eq!(selection.cycle(&scene, 50, 50), Some(node(1)));
        // And it wraps back.
        assert_eq!(selection.cycle(&scene, 50, 50), Some(node(2)));
    }

    #[test]
    fn hidden_and_locked_are_not_confused() {
        let mut scene = scene();
        let mut selection = Selection::new();
        // Locking node 2 makes it unselectable but still **drawn** (hit reaches node 1 under it).
        scene.set_locked(node(2), true);
        assert!(scene.is_locked(node(2)) && !scene.is_hidden(node(2)));
        assert!(!selection.click(&scene, node(2)));
        // Hiding node 1 takes it out of the hit test entirely, without locking it.
        scene.set_hidden(node(1), true);
        assert!(scene.is_hidden(node(1)) && !scene.is_locked(node(1)));
        assert_eq!(scene.hit(50, 50), None);
        // Unlocking node 2 does not unhide node 1, and node 2 sits inside the hidden node 1, so it
        // is invisible through its ancestor and still not selectable.
        scene.set_locked(node(2), false);
        assert!(scene.is_hidden(node(1)));
        assert!(!selection.click(&scene, node(2)));
        // Showing node 1 again makes node 2 selectable, without touching its lock.
        scene.set_hidden(node(1), false);
        assert!(selection.click(&scene, node(2)));
    }

    #[test]
    fn a_marquee_selects_everything_it_overlaps() {
        let scene = scene();
        let mut selection = Selection::new();
        let picked = selection.marquee(&scene, Rect::new(-10, -10, 60, 60));
        // Nodes 1 and 2 overlap; node 3 does not.
        assert_eq!(picked.len(), 2);
        assert!(selection.contains(node(1)) && selection.contains(node(2)));
        assert!(!selection.contains(node(3)));
    }

    #[test]
    fn a_locked_node_is_never_marquee_selected() {
        let mut scene = scene();
        scene.set_locked(node(2), true);
        let mut selection = Selection::new();
        let picked = selection.marquee(&scene, Rect::new(-10, -10, 200, 200));
        // Node 2 is locked, node 1 is under the marquee; node 3 is far away.
        assert_eq!(picked, vec![node(1)]);
    }

    #[test]
    fn navigation_reads_the_scenes_hierarchy() {
        let scene = scene();
        let selection = Selection::new();
        assert_eq!(selection.parent_of(&scene, node(2)), Some(node(1)));
        assert_eq!(selection.children_of(&scene, node(1)), vec![node(2)]);
        assert!(selection.children_of(&scene, node(2)).is_empty());
    }

    #[test]
    fn a_subtree_pick_takes_every_descendant() {
        let scene = scene();
        let mut selection = Selection::new();
        selection.select_subtree(&scene, node(1));
        assert!(selection.contains(node(1)) && selection.contains(node(2)));
        assert!(!selection.contains(node(3)));
    }

    #[test]
    fn pruning_drops_a_node_that_becomes_unselectable() {
        let mut scene = scene();
        let mut selection = Selection::new();
        selection.click(&scene, node(2));
        scene.set_locked(node(2), true);
        assert_eq!(selection.prune(&scene), vec![node(2)]);
        assert!(selection.is_empty());
    }
}
