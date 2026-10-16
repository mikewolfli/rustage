// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The design surface's complete keyboard, and the stress harness that shows the tree, the canvas
//! and the resources agree (task T09.08).
//!
//! The acceptance line is *"the tree and the canvas selections agree, and resources do not grow"*.
//! Both are properties of how the surface is built rather than of how carefully it is used:
//!
//! * **The canvas is derived from the tree.** [`Surface::scene`] walks the object tree and gives each
//!   node a rectangle from its position, so the two surfaces hold the same nodes by construction and
//!   cannot drift; [`Surface::agrees`] checks that fact at any moment. (Real geometry is F10's; here
//!   it is a deterministic function of the tree, which is exactly what makes disagreement impossible.)
//! * **One selection, both surfaces.** [`Surface::selection`] is the single set the object tree and
//!   the canvas both read (T09.03), so "the tree and the canvas agree about what is selected" is not
//!   a synchronisation but a fact.
//! * **The keyboard is the one keyboard contract.** [`keymap`] is a [`Keymap`] of design commands in
//!   the design scope, so the design surface does not grow a second key table; a typed character is
//!   suppressed by the same rule the editor's keys are.
//! * **The resources are bounded.** [`Surface::bounded`] caps the undo history, and every operation
//!   leaves the node count where it started, so a long session's memory does not grow without bound
//!   ([`Footprint`] is what the stress test watches).

#![allow(dead_code)]

use std::collections::BTreeSet;

use crate::app::canvas::drag::{Design, Edit};
use crate::app::canvas::scene::{Rect, Scene};
use crate::app::canvas::selection::Selection;
use crate::app::canvas::structure::Kind;
use crate::app::commands::shortcuts::{
    Binding, Chord, Key, Keymap, Resolution, Scene as Keys, Scope,
};
use crate::contracts::identity::NodeId;

/// The width of one derived canvas rectangle.
const NODE_WIDTH: i32 = 120;
/// The height of one derived canvas rectangle.
const NODE_HEIGHT: i32 = 30;
/// The horizontal step between derived canvas rectangles.
const NODE_STEP_X: i32 = 140;
/// The vertical step between one depth and the next.
const NODE_STEP_Y: i32 = 40;

/// What a design-surface keystroke asks for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    /// Select every node.
    SelectAll,
    /// Clear the selection.
    Deselect,
    /// Remove the selected nodes as one transaction.
    Delete,
    /// Take back the newest transaction.
    Undo,
    /// Put back the newest taken-back transaction.
    Redo,
    /// Move the keyboard to the next node.
    Next,
    /// Move the keyboard to the previous node.
    Previous,
    /// Move the keyboard to the focused node's parent.
    Parent,
    /// Move the keyboard to the focused node's first child.
    Child,
}

impl Action {
    /// The command string the keymap binds.
    pub fn command(self) -> &'static str {
        match self {
            Self::SelectAll => "design.select-all",
            Self::Deselect => "design.deselect",
            Self::Delete => "design.delete",
            Self::Undo => "design.undo",
            Self::Redo => "design.redo",
            Self::Next => "design.next",
            Self::Previous => "design.previous",
            Self::Parent => "design.parent",
            Self::Child => "design.child",
        }
    }

    /// The action a command string names.
    pub fn from_command(command: &str) -> Option<Self> {
        [
            Self::SelectAll,
            Self::Deselect,
            Self::Delete,
            Self::Undo,
            Self::Redo,
            Self::Next,
            Self::Previous,
            Self::Parent,
            Self::Child,
        ]
        .into_iter()
        .find(|action| action.command() == command)
    }
}

/// The design surface's keymap: the same one keyboard contract, in the design scope.
pub fn keymap() -> Keymap {
    let mut keymap = Keymap::new();
    let bindings = [
        ("Ctrl+A", Action::SelectAll),
        ("Escape", Action::Deselect),
        ("Delete", Action::Delete),
        ("Ctrl+Z", Action::Undo),
        ("Ctrl+Shift+Z", Action::Redo),
        ("Tab", Action::Next),
        ("Shift+Tab", Action::Previous),
        ("Up", Action::Parent),
        ("Down", Action::Child),
    ];
    for (chord, action) in bindings {
        keymap
            .add(Binding::new(
                Chord::parse(chord).expect("a valid design chord"),
                action.command(),
                Scope::Mode("design".to_owned()),
            ))
            .expect("the design chords do not conflict");
    }
    keymap
}

/// What applying an action did.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    /// The selection now holds this many nodes.
    Selected(usize),
    /// The selection was cleared.
    Cleared,
    /// This many nodes were removed.
    Deleted(usize),
    /// A transaction was taken back (or there was none).
    Undone(bool),
    /// A transaction was put back (or there was none).
    Redone(bool),
    /// The keyboard moved to a node.
    Focused(NodeId),
    /// The action had nothing to do.
    Nothing,
}

impl Outcome {
    /// The outcome in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Selected(count) => format!("selected {count} node(s)"),
            Self::Cleared => "selection cleared".to_owned(),
            Self::Deleted(count) => format!("deleted {count} node(s)"),
            Self::Undone(done) => format!("undo: {done}"),
            Self::Redone(done) => format!("redo: {done}"),
            Self::Focused(node) => format!("focused node {}", node.get()),
            Self::Nothing => "nothing to do".to_owned(),
        }
    }
}

/// What a surface is holding, in counts — the shape the stress test watches for growth.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Footprint {
    /// Nodes in the tree.
    pub nodes: usize,
    /// Transactions that can be taken back.
    pub past: usize,
    /// Transactions that can be put forward.
    pub future: usize,
    /// Nodes in the selection.
    pub selected: usize,
}

/// The design surface: the tree, its derived canvas, one selection and the keyboard.
pub struct Surface {
    design: Design,
    selection: Selection,
    focus: Option<NodeId>,
    keymap: Keymap,
}

impl Surface {
    /// An empty surface with unbounded history.
    pub fn new() -> Self {
        Self::of(Design::new())
    }

    /// An empty surface that keeps at most `limit` transactions, so memory stays bounded.
    pub fn bounded(limit: usize) -> Self {
        Self::of(Design::with_limit(limit))
    }

    fn of(design: Design) -> Self {
        Self {
            design,
            selection: Selection::new(),
            focus: None,
            keymap: keymap(),
        }
    }

    /// The object tree.
    pub fn tree(&self) -> &crate::app::canvas::structure::Tree {
        self.design.tree()
    }

    /// The selection both surfaces share.
    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    /// The node the keyboard is on.
    pub fn focus(&self) -> Option<NodeId> {
        self.focus
    }

    /// The canvas, derived from the tree so the two cannot disagree.
    pub fn scene(&self) -> Scene {
        let tree = self.design.tree();
        let mut scene = Scene::new();
        let mut stack: Vec<(NodeId, usize, Option<NodeId>)> =
            tree.roots().iter().rev().map(|id| (*id, 0, None)).collect();
        let mut index = 0usize;
        while let Some((node, depth, parent)) = stack.pop() {
            scene.insert(node, rect_of(index, depth));
            if let Some(parent) = parent {
                let _ = scene.set_parent(node, Some(parent));
            }
            index += 1;
            if let Some(holder) = tree.get(node) {
                for child in holder.children.iter().rev() {
                    stack.push((*child, depth + 1, Some(node)));
                }
            }
        }
        scene
    }

    /// Whether the tree and the canvas hold the same nodes, and the selection is a subset of them.
    pub fn agrees(&self) -> bool {
        let tree: BTreeSet<NodeId> = self.design.tree().ids().into_iter().collect();
        let canvas: BTreeSet<NodeId> = self.scene().order().iter().copied().collect();
        tree == canvas && self.selection.ids().iter().all(|node| tree.contains(node))
    }

    /// Inserts a node, returning its fresh id.
    pub fn add(
        &mut self,
        parent: Option<NodeId>,
        kind: Kind,
        name: &str,
    ) -> Result<NodeId, String> {
        let edit = Edit::Insert {
            parent,
            kind,
            name: name.to_owned(),
        };
        let outcomes = self
            .design
            .apply(std::slice::from_ref(&edit))
            .map_err(|error| error.message())?;
        match outcomes.as_slice() {
            [crate::app::canvas::drag::Outcome::Inserted(id)] => Ok(*id),
            _ => Err("the insert returned no id".to_owned()),
        }
    }

    /// Selects one node and moves the keyboard to it.
    pub fn select(&mut self, node: NodeId) -> bool {
        let scene = self.scene();
        let selected = self.selection.click(&scene, node);
        if selected {
            self.focus = Some(node);
        }
        selected
    }

    /// Resolves pressed keys to a design action through the one keyboard contract.
    pub fn press(&self, pressed: &[Key], context: &Keys) -> Result<Action, String> {
        match self.keymap.resolve(pressed, context) {
            Resolution::Triggered { command } => Action::from_command(&command)
                .ok_or_else(|| format!("{command:?} is not a design action")),
            Resolution::Pending { pressed, .. } => {
                let keys = pressed
                    .iter()
                    .map(|key| key.to_string())
                    .collect::<Vec<_>>()
                    .join(" ");
                Err(format!("{keys} starts a chord; press the rest"))
            }
            Resolution::Suppressed { reason } => Err(reason),
            Resolution::Nothing => Err("nothing is bound to those keys".to_owned()),
        }
    }

    /// Resolves pressed keys and applies what they ask for.
    pub fn press_and_apply(&mut self, pressed: &[Key], context: &Keys) -> Result<Outcome, String> {
        let action = self.press(pressed, context)?;
        self.apply(action)
    }

    /// Applies a design action.
    pub fn apply(&mut self, action: Action) -> Result<Outcome, String> {
        match action {
            Action::SelectAll => {
                let scene = self.scene();
                self.selection.select_all(&scene);
                Ok(Outcome::Selected(self.selection.len()))
            }
            Action::Deselect => {
                self.selection.clear();
                Ok(Outcome::Cleared)
            }
            Action::Delete => self.delete_selection(),
            Action::Undo => {
                let undone = self.design.undo();
                self.prune();
                Ok(Outcome::Undone(undone))
            }
            Action::Redo => {
                let redone = self.design.redo();
                self.prune();
                Ok(Outcome::Redone(redone))
            }
            Action::Next => self.step(1),
            Action::Previous => self.step(-1),
            Action::Parent => self.focus_parent(),
            Action::Child => self.focus_child(),
        }
    }

    /// The surface as a report.
    pub fn render(&self) -> Vec<String> {
        let mut lines = self.design.render();
        lines.extend(self.selection.render());
        let footprint = self.footprint();
        lines.push(format!(
            "footprint: {} node(s), {} past, {} future, {} selected",
            footprint.nodes, footprint.past, footprint.future, footprint.selected
        ));
        lines
    }

    /// What the surface is holding.
    pub fn footprint(&self) -> Footprint {
        let (past, future) = self.design.history();
        Footprint {
            nodes: self.design.tree().len(),
            past,
            future,
            selected: self.selection.len(),
        }
    }

    /// Removes the selected nodes as one transaction, then drops them from the selection.
    fn delete_selection(&mut self) -> Result<Outcome, String> {
        let tree = self.design.tree();
        let selected: Vec<NodeId> = self.selection.ids();
        // Only the topmost selected nodes: deleting a parent already removes a selected child.
        let roots: Vec<NodeId> = selected
            .iter()
            .copied()
            .filter(|node| !has_selected_ancestor(tree, *node, &selected))
            .collect();
        if roots.is_empty() {
            return Ok(Outcome::Nothing);
        }
        let edits: Vec<Edit> = roots
            .iter()
            .map(|node| Edit::Delete { node: *node })
            .collect();
        let removed = self
            .design
            .apply(&edits)
            .map_err(|error| error.message())?
            .iter()
            .map(|outcome| match outcome {
                crate::app::canvas::drag::Outcome::Deleted { removed, .. } => removed.len(),
                _ => 0,
            })
            .sum();
        self.prune();
        Ok(Outcome::Deleted(removed))
    }

    /// Moves the keyboard `delta` places through the nodes and selects where it lands.
    fn step(&mut self, delta: isize) -> Result<Outcome, String> {
        let ids = self.design.tree().ids();
        if ids.is_empty() {
            return Ok(Outcome::Nothing);
        }
        let next = match self
            .focus
            .and_then(|focus| ids.iter().position(|id| *id == focus))
        {
            Some(position) => {
                let len = ids.len() as isize;
                (position as isize + delta).rem_euclid(len) as usize
            }
            None => {
                if delta < 0 {
                    ids.len() - 1
                } else {
                    0
                }
            }
        };
        self.select(ids[next]);
        Ok(Outcome::Focused(ids[next]))
    }

    /// Moves the keyboard to the focused node's parent and selects it.
    fn focus_parent(&mut self) -> Result<Outcome, String> {
        let node = self.focus.ok_or_else(|| "nothing is focused".to_owned())?;
        let parent = self
            .design
            .tree()
            .get(node)
            .and_then(|holder| holder.parent)
            .ok_or_else(|| format!("node {} has no parent", node.get()))?;
        self.select(parent);
        Ok(Outcome::Focused(parent))
    }

    /// Moves the keyboard to the focused node's first child and selects it.
    fn focus_child(&mut self) -> Result<Outcome, String> {
        let node = self.focus.ok_or_else(|| "nothing is focused".to_owned())?;
        let child = self
            .design
            .tree()
            .get(node)
            .and_then(|holder| holder.children.first().copied())
            .ok_or_else(|| format!("node {} has no child", node.get()))?;
        self.select(child);
        Ok(Outcome::Focused(child))
    }

    /// Drops from the selection anything the surface no longer holds.
    fn prune(&mut self) {
        let scene = self.scene();
        self.selection.prune(&scene);
    }
}

impl Default for Surface {
    fn default() -> Self {
        Self::new()
    }
}

/// The rectangle a node gets from its position and depth on the canvas.
fn rect_of(index: usize, depth: usize) -> Rect {
    Rect::new(
        index as i32 * NODE_STEP_X,
        depth as i32 * NODE_STEP_Y,
        NODE_WIDTH as u32,
        NODE_HEIGHT as u32,
    )
}

/// Whether any of `selected` is an ancestor of `node`.
fn has_selected_ancestor(
    tree: &crate::app::canvas::structure::Tree,
    node: NodeId,
    selected: &[NodeId],
) -> bool {
    let mut current = tree.get(node).and_then(|holder| holder.parent);
    while let Some(ancestor) = current {
        if selected.contains(&ancestor) {
            return true;
        }
        current = tree.get(ancestor).and_then(|holder| holder.parent);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::{Action, Surface, keymap};
    use crate::app::canvas::structure::Kind;
    use crate::app::commands::shortcuts::{Key, Scene as Keys};

    fn design_context() -> Keys {
        Keys::in_mode("design", None)
    }

    fn key(name: &str) -> Key {
        Key::parse(name).expect("a valid key")
    }

    /// A flat tree of `count` widget roots under one container.
    fn surface_with(count: usize) -> (Surface, Vec<crate::contracts::identity::NodeId>) {
        let mut surface = Surface::new();
        let panel = surface.add(None, Kind::Container, "panel").expect("a root");
        let mut ids = Vec::new();
        for index in 0..count {
            ids.push(
                surface
                    .add(Some(panel), Kind::Widget, &format!("n{index}"))
                    .expect("a child"),
            );
        }
        (surface, ids)
    }

    #[test]
    fn the_design_keymap_resolves_a_chord_to_an_action() {
        let surface = Surface::new();
        let action = surface
            .press(&[key("Ctrl+A")], &design_context())
            .expect("a bound chord");
        assert_eq!(action, Action::SelectAll);
    }

    #[test]
    fn a_typed_character_is_never_a_design_command() {
        let surface = Surface::new();
        assert!(surface.press(&[key("a")], &design_context()).is_err());
    }

    #[test]
    fn an_unbound_chord_runs_nothing() {
        let surface = Surface::new();
        assert!(surface.press(&[key("Ctrl+Q")], &design_context()).is_err());
    }

    #[test]
    fn select_all_selects_every_node_on_both_surfaces() {
        let (mut surface, _ids) = surface_with(5);
        let outcome = surface.apply(Action::SelectAll).expect("select all");
        assert_eq!(outcome, super::Outcome::Selected(6));
        assert!(surface.agrees());
        assert_eq!(surface.selection().len(), 6);
    }

    #[test]
    fn deleting_the_selection_is_one_undo() {
        let (mut surface, ids) = surface_with(3);
        let before = surface.footprint();
        let _ = surface.select(ids[0]);
        surface.apply(Action::SelectAll).expect("select all");
        surface.apply(Action::Delete).expect("delete");
        assert_eq!(surface.footprint().nodes, 0);
        assert!(surface.agrees());
        surface.apply(Action::Undo).expect("undo");
        assert_eq!(surface.footprint().nodes, before.nodes);
        assert!(surface.agrees());
    }

    #[test]
    fn a_selected_parent_takes_its_selected_child_with_it() {
        let mut surface = Surface::new();
        let panel = surface.add(None, Kind::Container, "panel").expect("root");
        let child = surface
            .add(Some(panel), Kind::Widget, "button")
            .expect("child");
        surface.select(panel);
        surface
            .press_and_apply(&[key("Ctrl+A")], &design_context())
            .expect("select all");
        surface.apply(Action::Delete).expect("delete");
        assert_eq!(surface.footprint().nodes, 0);
        assert!(!surface.selection().contains(child));
        assert!(surface.agrees());
    }

    #[test]
    fn the_keyboard_walks_the_hierarchy() {
        let mut surface = Surface::new();
        let panel = surface.add(None, Kind::Container, "panel").expect("root");
        let child = surface
            .add(Some(panel), Kind::Widget, "button")
            .expect("child");
        surface.select(panel);
        surface
            .press_and_apply(&[key("Down")], &design_context())
            .expect("to the child");
        assert_eq!(surface.focus(), Some(child));
        surface
            .press_and_apply(&[key("Up")], &design_context())
            .expect("to the parent");
        assert_eq!(surface.focus(), Some(panel));
    }

    #[test]
    fn the_tree_and_the_canvas_agree_through_a_thousand_operations() {
        let (mut surface, _ids) = surface_with(1000);
        assert!(surface.agrees());
        assert_eq!(surface.footprint().nodes, 1001);
        let baseline = surface.footprint();

        for round in 0..100 {
            surface.apply(Action::SelectAll).expect("select all");
            surface.apply(Action::Delete).expect("delete");
            assert_eq!(surface.footprint().nodes, 0, "round {round} cleared");
            assert!(surface.agrees(), "round {round} agreed after delete");
            surface.apply(Action::Undo).expect("undo");
            assert_eq!(
                surface.footprint().nodes,
                baseline.nodes,
                "round {round} restored"
            );
            assert!(surface.agrees(), "round {round} agreed after undo");
        }
        assert_eq!(surface.footprint().nodes, baseline.nodes);
    }

    #[test]
    fn a_bounded_surface_does_not_grow_its_history() {
        let mut surface = Surface::bounded(16);
        for index in 0..200 {
            surface
                .add(None, Kind::Widget, &format!("n{index}"))
                .expect("a node");
        }
        let footprint = surface.footprint();
        assert_eq!(footprint.nodes, 200);
        assert!(footprint.past <= 16, "history stayed within the limit");
        assert_eq!(footprint.future, 0);
    }

    #[test]
    fn the_selection_is_never_larger_than_the_surface() {
        let (mut surface, _ids) = surface_with(50);
        for _ in 0..50 {
            surface.apply(Action::SelectAll).expect("select all");
            let footprint = surface.footprint();
            assert!(footprint.selected <= footprint.nodes);
        }
    }

    #[test]
    fn undoing_with_nothing_to_take_back_says_so() {
        let mut surface = Surface::new();
        assert_eq!(
            surface.apply(Action::Undo).expect("undo"),
            super::Outcome::Undone(false)
        );
    }

    #[test]
    fn the_keymap_has_one_binding_per_design_command() {
        assert_eq!(keymap().len(), 9);
    }
}
