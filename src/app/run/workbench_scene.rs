// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The workbench's object tree and canvas selection (tasks T09.02–T09.04/T09.08).
//!
//! The scene owns hit testing, z-order and generation; the structural tree owns parent/child
//! relationships and names; this type owns the one selection set both views read. The window
//! mirrors [`CanvasState::rows`] into the Outline list and each node's rectangle into a real
//! button, so a tree click and a canvas click cannot disagree about what is selected.

use std::any::Any;

use rust_widgets::core::{ObjectId, Rect as WidgetRect};
use rust_widgets::layout::Layout;

#[cfg(test)]
use crate::app::canvas::scene::{Handle, Stale};
use crate::app::canvas::scene::{Rect, Scene};
use crate::app::canvas::structure::{Kind, Tree};
use crate::contracts::identity::NodeId;

/// The design-space size the demo canvas shows.
pub(super) const DESIGN_WIDTH: u32 = 600;
pub(super) const DESIGN_HEIGHT: u32 = 400;

/// One node in the demo scene.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct Node {
    /// The stable id.
    pub node: NodeId,
    /// Its design-space rectangle.
    pub rect: Rect,
}

/// The object tree, scene and selection.
pub(super) struct CanvasState {
    tree: Tree,
    scene: Scene,
    selected: Vec<NodeId>,
    nodes: Vec<Node>,
}

impl CanvasState {
    /// The demo scene: a panel holding a button and a label that overlap, a hidden note and a
    /// locked footer. The overlap is what makes "the topmost node is the click" testable; the
    /// hidden and locked nodes are what keeps those two states from being confused with each other.
    pub(super) fn demo() -> Self {
        let mut tree = Tree::new();
        let mut scene = Scene::new();
        let root = tree
            .insert(None, Kind::Container, "window")
            .expect("a root is allowed");
        // The Inspector's demo property seeds itself with "Demo"; naming the panel that makes the
        // seed the node's real title rather than a second value the model has to remember.
        let panel = tree
            .insert(Some(root), Kind::Container, "Demo")
            .expect("a container is allowed in a container");
        let button = tree
            .insert(Some(panel), Kind::Widget, "button")
            .expect("a widget is allowed in a container");
        let label = tree
            .insert(Some(panel), Kind::Text, "label")
            .expect("text is allowed in a container");
        let note = tree
            .insert(Some(panel), Kind::Text, "hidden note")
            .expect("text is allowed in a container");
        let footer = tree
            .insert(Some(root), Kind::Widget, "footer")
            .expect("a widget is allowed in a container");

        let mut nodes = Vec::new();
        for (node, rect) in [
            (root, Rect::new(0, 0, 600, 400)),
            (panel, Rect::new(40, 60, 520, 280)),
            (button, Rect::new(80, 120, 180, 48)),
            (label, Rect::new(140, 150, 200, 60)),
            (note, Rect::new(320, 120, 140, 40)),
            (footer, Rect::new(40, 330, 200, 40)),
        ] {
            scene.insert(node, rect);
            nodes.push(Node { node, rect });
        }
        scene.set_hidden(note, true);
        scene.set_locked(footer, true);

        Self {
            tree,
            scene,
            selected: Vec::new(),
            nodes,
        }
    }

    /// The Outline rows, pre-order, with hidden/locked markers.
    pub(super) fn rows(&self) -> Vec<String> {
        let mut out = Vec::new();
        for root in self.tree.roots() {
            self.rows_into(*root, 0, &mut out);
        }
        out
    }

    fn rows_into(&self, node: NodeId, depth: usize, out: &mut Vec<String>) {
        let Some(entry) = self.tree.get(node) else {
            return;
        };
        let marker = if self.scene.is_hidden(node) {
            " [hidden]"
        } else if self.scene.is_locked(node) {
            " [locked]"
        } else {
            ""
        };
        out.push(format!("{}{}{}", "  ".repeat(depth), entry.name, marker));
        for child in &entry.children {
            self.rows_into(*child, depth + 1, out);
        }
    }

    /// The node each Outline row names, in the same order as [`Self::rows`].
    pub(super) fn row_nodes(&self) -> Vec<NodeId> {
        let mut out = Vec::new();
        for root in self.tree.roots() {
            self.row_nodes_into(*root, &mut out);
        }
        out
    }

    fn row_nodes_into(&self, node: NodeId, out: &mut Vec<NodeId>) {
        out.push(node);
        if let Some(entry) = self.tree.get(node) {
            for child in &entry.children {
                self.row_nodes_into(*child, out);
            }
        }
    }

    /// The selected nodes, in selection order.
    pub(super) fn selected(&self) -> &[NodeId] {
        &self.selected
    }

    /// The node shown as selected in the tree, if exactly one is selected.
    pub(super) fn primary(&self) -> Option<NodeId> {
        self.selected.first().copied()
    }

    /// A node's name, when the tree still holds it.
    pub(super) fn name(&self, node: NodeId) -> Option<&str> {
        self.tree.get(node).map(|entry| entry.name.as_str())
    }

    /// A node's kind, when the tree still holds it.
    pub(super) fn kind(&self, node: NodeId) -> Option<Kind> {
        self.tree.get(node).map(|entry| entry.kind)
    }

    /// A node's parent in the structural tree, or `None` for a root.
    pub(super) fn parent(&self, node: NodeId) -> Option<NodeId> {
        self.tree.get(node).and_then(|entry| entry.parent)
    }

    /// Renames a node after the Inspector accepted a new title.
    pub(super) fn rename(&mut self, node: NodeId, name: &str) -> Result<(), String> {
        self.tree
            .rename(node, name)
            .map_err(|error| error.message())
    }

    /// Clears the selection, which is what a click on empty canvas does.
    pub(super) fn clear_selection(&mut self) -> usize {
        let had = self.selected.len();
        self.selected.clear();
        had
    }

    /// Selects exactly `node`, refusing a hidden or locked one with the reason that applies.
    pub(super) fn select_only(&mut self, node: NodeId) -> Result<(), String> {
        if !self.scene.contains(node) {
            return Err(format!("node {} is not in the scene", node.get()));
        }
        if self.scene.is_hidden(node) {
            return Err(format!(
                "node {} is hidden; hidden nodes are not selectable",
                node.get()
            ));
        }
        if self.scene.is_locked(node) {
            return Err(format!(
                "node {} is locked; locked nodes are not selectable",
                node.get()
            ));
        }
        if self.selected == [node] {
            return Ok(());
        }
        self.selected = vec![node];
        Ok(())
    }

    /// Selects every selectable node in `nodes`, skipping hidden/locked ones.
    pub(super) fn select_many(&mut self, nodes: impl IntoIterator<Item = NodeId>) -> usize {
        let mut selected = Vec::new();
        for node in nodes {
            if self.scene.is_selectable(node) && !selected.contains(&node) {
                selected.push(node);
            }
        }
        self.selected = selected;
        self.selected.len()
    }

    /// The topmost selectable node at a point in design space.
    pub(super) fn hit(&self, x: i32, y: i32) -> Option<NodeId> {
        self.scene.hit(x, y)
    }

    /// Every selectable node intersecting a design-space rectangle.
    pub(super) fn within(&self, rect: Rect) -> Vec<NodeId> {
        self.scene.within(rect)
    }

    /// A node's design-space rectangle.
    pub(super) fn rect(&self, node: NodeId) -> Option<Rect> {
        self.nodes
            .iter()
            .find(|entry| entry.node == node)
            .map(|entry| entry.rect)
    }

    /// Every node with its rectangle, in paint order (bottom first).
    pub(super) fn nodes(&self) -> Vec<Node> {
        self.scene
            .order()
            .iter()
            .filter_map(|node| self.rect(*node).map(|rect| Node { node: *node, rect }))
            .filter(|entry| !self.scene.is_hidden(entry.node))
            .collect()
    }

    /// Rebuilds a node's instance and reports both the new handle and the old handle's refusal.
    ///
    /// This is the model-level evidence for the generation rule; the canvas button is the widget
    /// instance and its own layout pass is what a repaint follows, so the window does not call it.
    #[cfg(test)]
    pub(super) fn rebuild(&mut self, node: NodeId) -> Result<(Handle, Stale), String> {
        let rect = self
            .rect(node)
            .ok_or_else(|| format!("node {} is not in the scene", node.get()))?;
        let old = self
            .scene
            .instance(node)
            .ok_or_else(|| format!("node {} has no instance", node.get()))?;
        let new = self
            .scene
            .rebuild(node, rect)
            .ok_or_else(|| format!("node {} could not be rebuilt", node.get()))?;
        let stale = self
            .scene
            .resolve(old)
            .expect_err("the old handle is stale");
        Ok((new, stale))
    }

    /// Resolves the current selection, reporting nodes whose instance vanished.
    #[cfg(test)]
    pub(super) fn resolve_selection(&self) -> (Vec<Handle>, Vec<NodeId>) {
        self.scene.resolve_selection(&self.selected)
    }

    /// Removes a node from both the structural tree and the scene.
    ///
    /// The delete action is the next T09 slice; the model is written and tested here so the
    /// action cannot need a second structural tree.
    #[cfg(test)]
    pub(super) fn delete(&mut self, node: NodeId) -> Result<String, String> {
        let report = self.tree.delete(node).map_err(|error| error.message())?;
        // The structural delete removes the whole subtree; every removed instance and its
        // rectangle go with it, so no descendant is left behind as a hit-test target.
        for removed in &report.removed {
            self.scene.remove(*removed);
            self.nodes.retain(|entry| entry.node != *removed);
            self.selected.retain(|selected| selected != removed);
        }
        Ok(report.render())
    }

    /// Whether a node is hidden.
    pub(super) fn is_hidden(&self, node: NodeId) -> bool {
        self.scene.is_hidden(node)
    }

    /// Whether a node is locked.
    pub(super) fn is_locked(&self, node: NodeId) -> bool {
        self.scene.is_locked(node)
    }

    /// The row index that names `node`, when the tree still holds it.
    pub(super) fn row_of(&self, node: NodeId) -> Option<usize> {
        self.row_nodes().iter().position(|row| *row == node)
    }
}

impl Default for CanvasState {
    fn default() -> Self {
        Self::demo()
    }
}

/// The uniform scale and centring offset that maps design space into a control rect.
///
/// Both the layout pass and the pointer callbacks use this one transform, so a click and the
/// rectangle drawn under it cannot disagree about where a node is.
#[derive(Clone, Copy, Debug)]
pub(super) struct CanvasTransform {
    /// Widget pixels per design pixel.
    pub scale: f32,
    /// The widget-space x of design x = 0.
    pub offset_x: i32,
    /// The widget-space y of design y = 0.
    pub offset_y: i32,
}

impl CanvasTransform {
    /// The transform for a control rect (design space is centred in it, never stretched).
    pub(super) fn new(rect: WidgetRect) -> Self {
        let scale = (rect.width as f32 / DESIGN_WIDTH as f32)
            .min(rect.height as f32 / DESIGN_HEIGHT as f32)
            .max(0.01);
        Self {
            scale,
            offset_x: rect.x + ((rect.width as f32 - DESIGN_WIDTH as f32 * scale) / 2.0) as i32,
            offset_y: rect.y + ((rect.height as f32 - DESIGN_HEIGHT as f32 * scale) / 2.0) as i32,
        }
    }

    /// A design-space rectangle in widget space.
    pub(super) fn place(&self, node: Rect) -> WidgetRect {
        WidgetRect::new(
            self.offset_x + (node.x as f32 * self.scale) as i32,
            self.offset_y + (node.y as f32 * self.scale) as i32,
            ((node.width as f32 * self.scale) as u32).max(1),
            ((node.height as f32 * self.scale) as u32).max(1),
        )
    }

    /// A widget-space point in design space (used by the pointer callbacks).
    pub(super) fn to_design(self, x: i32, y: i32) -> (i32, i32) {
        let scale = f64::from(self.scale).max(0.01);
        (
            ((x - self.offset_x) as f64 / scale).round() as i32,
            ((y - self.offset_y) as f64 / scale).round() as i32,
        )
    }
}

/// A rectangle spanned by two design-space points, in any drag direction.
pub(super) fn span(a: (i32, i32), b: (i32, i32)) -> Rect {
    let x = a.0.min(b.0);
    let y = a.1.min(b.1);
    let width = a.0.abs_diff(b.0).max(1);
    let height = a.1.abs_diff(b.1).max(1);
    Rect::new(x, y, width, height)
}

/// Places one real canvas/background control and one button per visible node.
pub(super) struct CanvasLayout {
    canvas: ObjectId,
    nodes: Vec<(ObjectId, Rect)>,
}

impl CanvasLayout {
    /// A layout over the canvas and its node buttons.
    pub(super) fn new(canvas: ObjectId, nodes: Vec<(ObjectId, Rect)>) -> Self {
        Self { canvas, nodes }
    }
}

impl Layout for CanvasLayout {
    fn add_widget(&mut self, _widget_id: ObjectId, _stretch: u32) {}

    fn remove_widget(&mut self, widget_id: ObjectId) {
        self.nodes.retain(|(id, _)| *id != widget_id);
    }

    fn update(&self, rect: WidgetRect, widgets: &mut dyn FnMut(ObjectId, WidgetRect)) {
        widgets(self.canvas, rect);
        let transform = CanvasTransform::new(rect);
        for (id, node) in &self.nodes {
            widgets(*id, transform.place(*node));
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::CanvasState;
    use crate::app::canvas::scene::Rect;

    #[test]
    fn the_topmost_overlapping_node_is_the_hit_and_the_tree_still_reaches_the_one_below() {
        let mut state = CanvasState::demo();
        let rows = state.row_nodes();
        let button = rows[2];
        let label = rows[3];
        // The label is inserted after the button and overlaps it.
        assert_eq!(state.hit(150, 155), Some(label));
        state
            .select_only(button)
            .expect("the tree can select the lower node");
        assert_eq!(state.selected(), [button]);
        assert_eq!(state.hit(150, 155), Some(label));
    }

    #[test]
    fn hidden_and_locked_nodes_are_not_selectable_and_say_which_they_are() {
        let mut state = CanvasState::demo();
        let rows = state.row_nodes();
        let note = rows[4];
        let footer = rows[5];
        let hidden = state.select_only(note).expect_err("hidden");
        assert!(hidden.contains("hidden"), "{hidden}");
        let locked = state.select_only(footer).expect_err("locked");
        assert!(locked.contains("locked"), "{locked}");
        // The point is inside the locked footer but also inside the window behind it: the hit is
        // the window, never the locked node.
        assert_eq!(state.hit(40, 340), Some(rows[0]));
        assert_ne!(state.hit(40, 340), Some(footer));
    }

    #[test]
    fn a_marquee_returns_only_selectable_intersections() {
        let state = CanvasState::demo();
        let rows = state.row_nodes();
        let note = rows[4];
        let panel = rows[1];
        let nodes = state.within(Rect::new(70, 110, 300, 100));
        assert!(!nodes.is_empty());
        assert!(nodes.contains(&panel));
        let over_note = state.within(Rect::new(320, 120, 10, 10));
        assert!(over_note.contains(&panel), "the panel really does overlap");
        assert!(
            !over_note.contains(&note),
            "a hidden node is never in a marquee"
        );
    }

    #[test]
    fn deleting_a_node_removes_its_subtree_and_clears_the_selection() {
        let mut state = CanvasState::demo();
        let rows = state.row_nodes();
        let panel = rows[1];
        state.select_only(panel).expect("selectable");
        let report = state.delete(panel).expect("delete");
        assert!(report.contains("4 node(s)"), "{report}");
        assert!(state.selected().is_empty());
        assert!(!state.row_nodes().contains(&panel));
        assert_eq!(state.hit(150, 155), Some(rows[0]), "the window remains");
    }

    #[test]
    fn a_rebuild_stales_the_old_handle_while_the_node_selection_survives() {
        let mut state = CanvasState::demo();
        let rows = state.row_nodes();
        let button = rows[2];
        state.select_only(button).expect("selectable");
        let (new, stale) = state.rebuild(button).expect("rebuild");
        assert!(matches!(
            stale,
            crate::app::canvas::scene::Stale::Rebuilt { .. }
        ));
        let (handles, missing) = state.resolve_selection();
        assert_eq!(handles, vec![new]);
        assert!(missing.is_empty());
        assert_eq!(state.selected(), [button]);
    }
}
