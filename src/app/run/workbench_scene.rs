// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The workbench's object tree and canvas selection (tasks T09.02–T09.04/T09.08).
//!
//! The scene owns hit testing, z-order and generation; the structural tree owns parent/child
//! relationships and names; this type owns the one selection set both views read. The window
//! mirrors [`CanvasState::rows`] into the Outline list and each node's rectangle into a real
//! button, so a tree click and a canvas click cannot disagree about what is selected.

use std::any::Any;
use std::collections::BTreeMap;

use rust_widgets::core::{ObjectId, Rect as WidgetRect};
use rust_widgets::layout::{ChildInfo, Layout, LayoutContext};

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

/// The library control and property one demo node is edited through.
///
/// This is stored next to the node rather than derived from its display name at lookup time: the
/// tree shows names a person chose, and a renamed "button" must keep editing `button.text` rather
/// than silently changing which contract the Inspector reads.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct NodeContract {
    /// The library's canonical control name.
    pub control: String,
    /// The property name that control publishes.
    pub property: String,
}

impl NodeContract {
    pub(super) fn new(control: impl Into<String>, property: impl Into<String>) -> Self {
        Self {
            control: control.into(),
            property: property.into(),
        }
    }

    /// The contract in one line, `control.property`.
    pub(super) fn render(&self) -> String {
        format!("{}.{}", self.control, self.property)
    }
}

/// One node in a copied subtree, detached from its ids and parent.
#[derive(Clone, PartialEq, Eq, Debug)]
struct ClipboardNode {
    /// Depth inside the copied subtree; the root is 0.
    depth: usize,
    kind: Kind,
    name: String,
    /// The design-space rectangle at copy time; paste offsets it so the copy is visible.
    rect: Rect,
    contract: Option<NodeContract>,
    hidden: bool,
    locked: bool,
}

/// A copied subtree, pre-order, ready to paste with fresh ids.
///
/// This is the designer's own scene clipboard, not the system clipboard: T05.06 owns the OS MIME
/// payloads and cross-project id reallocation. Keeping the two apart means a Copy here cannot
/// claim the OS clipboard now holds a node payload it cannot describe.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct ClipboardSubtree {
    nodes: Vec<ClipboardNode>,
}

impl ClipboardSubtree {
    /// How many nodes it holds.
    pub(super) fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether it holds nothing.
    pub(super) fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

/// The object tree, scene and selection.
pub(super) struct CanvasState {
    tree: Tree,
    scene: Scene,
    selected: Vec<NodeId>,
    /// Each node's design-space rectangle, keyed by id so [`Self::rect`] is a lookup rather than a
    /// scan and [`Self::nodes`] is one pass over the scene order.
    nodes: BTreeMap<NodeId, Rect>,
    contracts: BTreeMap<NodeId, NodeContract>,
    /// A counter that changes whenever the picture the window mirrors changes: names, selection or
    /// membership. The window uses it to skip rebuilding a canvas snapshot that cannot differ.
    revision: u64,
    /// A counter for the Outline's row content (names and membership), which selection changes do
    /// not alter. The window uses it to avoid rebuilding every row on a click.
    content_revision: u64,
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

        let mut nodes = BTreeMap::new();
        for (node, rect) in [
            (root, Rect::new(0, 0, 600, 400)),
            (panel, Rect::new(40, 60, 520, 280)),
            (button, Rect::new(80, 120, 180, 48)),
            (label, Rect::new(140, 150, 200, 60)),
            (note, Rect::new(320, 120, 140, 40)),
            (footer, Rect::new(40, 330, 200, 40)),
        ] {
            scene.insert(node, rect);
            nodes.insert(node, rect);
        }
        scene.set_hidden(note, true);
        scene.set_locked(footer, true);

        // The demo mapping is explicit: a container edits its title, a button/label edits its text.
        // `panel` is the library's plain-container spelling for the control the demo calls `Demo`.
        let contracts = BTreeMap::from([
            (root, NodeContract::new("window", "title")),
            (panel, NodeContract::new("panel", "title")),
            (button, NodeContract::new("button", "text")),
            (label, NodeContract::new("label", "text")),
            (note, NodeContract::new("label", "text")),
            (footer, NodeContract::new("button", "text")),
        ]);

        Self {
            tree,
            scene,
            selected: Vec::new(),
            nodes,
            contracts,
            revision: 0,
            content_revision: 0,
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

    /// The contract this node was mapped to, or why it has none.
    ///
    /// The mapping is stored per node when the scene is built; a missing entry is reported rather
    /// than guessed from the node's name or kind.
    pub(super) fn contract(&self, node: NodeId) -> Result<NodeContract, String> {
        self.contracts.get(&node).cloned().ok_or_else(|| {
            format!(
                "node {} has no contract mapping; the Inspector will not guess one",
                node.get()
            )
        })
    }

    /// A node's parent in the structural tree, or `None` for a root.
    pub(super) fn parent(&self, node: NodeId) -> Option<NodeId> {
        self.tree.get(node).and_then(|entry| entry.parent)
    }

    /// Duplicates a node's subtree under the same parent and selects the copy.
    ///
    /// The structural copy comes from [`Tree::copy`], which deep-copies in pre-order with fresh ids;
    /// this method pairs the old and new pre-order lists so each copy keeps its source's rectangle
    /// (offset so it is visible), contract, hidden and locked state. The copy's root gets a unique
    /// display name (`Demo` becomes `Demo copy`), so the Outline can tell the two apart.
    ///
    /// The copy is selected unless its source was hidden or locked; a hidden duplicate stays hidden,
    /// because the duplicate is of the node a person sees, states included.
    pub(super) fn duplicate(&mut self, node: NodeId) -> Result<NodeId, String> {
        if !self.tree.ids().contains(&node) {
            return Err(format!("node {} does not exist", node.get()));
        }
        let sources = self.subtree_preorder(node);
        let copy = self.tree.copy(node).map_err(|error| error.message())?;
        let copies = self.subtree_preorder(copy);
        debug_assert_eq!(
            sources.len(),
            copies.len(),
            "a pre-order copy has one new node per source node"
        );

        for (source, copied) in sources.iter().zip(copies.iter()) {
            let rect = self
                .rect(*source)
                .map_or(Rect::new(60, 90, 160, 40), |rect| {
                    Rect::new(rect.x + 16, rect.y + 16, rect.width, rect.height)
                });
            self.scene.insert(*copied, rect);
            self.nodes.insert(*copied, rect);
            if let Some(contract) = self.contracts.get(source).cloned() {
                self.contracts.insert(*copied, contract);
            }
            if self.scene.is_hidden(*source) {
                self.scene.set_hidden(*copied, true);
            }
            if self.scene.is_locked(*source) {
                self.scene.set_locked(*copied, true);
            }
        }

        if let Some(base) = self.name(node).map(str::to_owned) {
            let unique = self.unique_name(&format!("{base} copy"));
            self.tree
                .rename(copy, unique)
                .map_err(|error| error.message())?;
        }
        if self.scene.is_selectable(copy) {
            self.selected = vec![copy];
        }
        self.revision += 1;
        self.content_revision += 1;
        Ok(copy)
    }

    /// Copies a node's subtree into the scene clipboard.
    ///
    /// Ids, parent links and scene instances do not travel: a clipboard value is structure, names,
    /// rectangles, contracts and hidden/locked state. Paste gives every node a fresh id.
    pub(super) fn copy_subtree(&self, node: NodeId) -> Result<ClipboardSubtree, String> {
        if self.kind(node).is_none() {
            return Err(format!("node {} does not exist", node.get()));
        }
        let mut nodes = Vec::new();
        self.copy_into(node, 0, &mut nodes);
        Ok(ClipboardSubtree { nodes })
    }

    /// Pastes a copied subtree under the nearest container at or above the selection.
    pub(super) fn paste_clipboard(&mut self, subtree: &ClipboardSubtree) -> Result<NodeId, String> {
        if subtree.is_empty() {
            return Err("the scene clipboard is empty".to_owned());
        }
        let parent = self.container_for_insertion().ok_or_else(|| {
            "the scene has no container that can hold a pasted node; add a container first"
                .to_owned()
        })?;
        self.paste_into(subtree, parent)
    }

    fn copy_into(&self, node: NodeId, depth: usize, out: &mut Vec<ClipboardNode>) {
        let Some(entry) = self.tree.get(node) else {
            return;
        };
        out.push(ClipboardNode {
            depth,
            kind: entry.kind,
            name: entry.name.clone(),
            rect: self.rect(node).unwrap_or(Rect::new(0, 0, 160, 40)),
            contract: self.contracts.get(&node).cloned(),
            hidden: self.scene.is_hidden(node),
            locked: self.scene.is_locked(node),
        });
        for child in &entry.children {
            self.copy_into(*child, depth + 1, out);
        }
    }

    fn paste_into(&mut self, subtree: &ClipboardSubtree, parent: NodeId) -> Result<NodeId, String> {
        if self.kind(parent) != Some(Kind::Container) {
            return Err(format!(
                "node {} is not a container; a pasted subtree needs a container parent",
                parent.get()
            ));
        }
        // Validate the shape before any node is inserted: a half-pasted tree is a state nobody
        // asked for, and the clipboard is data from outside this call.
        let mut previous: Option<usize> = None;
        for entry in &subtree.nodes {
            match previous {
                None if entry.depth != 0 => {
                    return Err("the clipboard does not start at a root".to_owned());
                }
                Some(0) if entry.depth == 0 => {
                    return Err("the clipboard holds more than one root".to_owned());
                }
                Some(depth) if entry.depth > depth + 1 => {
                    return Err("the clipboard is not a tree: a depth is skipped".to_owned());
                }
                _ => {}
            }
            previous = Some(entry.depth);
        }

        let mut stack: Vec<NodeId> = Vec::new();
        let mut root: Option<NodeId> = None;
        for entry in &subtree.nodes {
            let host = if entry.depth == 0 {
                parent
            } else {
                *stack
                    .get(entry.depth - 1)
                    .ok_or_else(|| "the clipboard is not a tree".to_owned())?
            };
            let name = self.unique_name(&entry.name);
            let node = self
                .tree
                .insert(Some(host), entry.kind, name)
                .map_err(|error| error.message())?;
            stack.truncate(entry.depth);
            stack.push(node);
            let rect = Rect::new(
                entry.rect.x + 16,
                entry.rect.y + 16,
                entry.rect.width,
                entry.rect.height,
            );
            self.scene.insert(node, rect);
            self.nodes.insert(node, rect);
            if let Some(contract) = &entry.contract {
                self.contracts.insert(node, contract.clone());
            }
            if entry.hidden {
                self.scene.set_hidden(node, true);
            }
            if entry.locked {
                self.scene.set_locked(node, true);
            }
            if root.is_none() {
                root = Some(node);
            }
        }
        self.revision += 1;
        self.content_revision += 1;
        root.ok_or_else(|| "the scene clipboard is empty".to_owned())
    }

    /// Moves a node out of its container into the container's own parent.
    ///
    /// This is the inverse of [`Self::wrap_in_container`] without deleting anything: the container
    /// stays where it is (it may hold other nodes), and the node's rectangle does not move. A node
    /// already at the root is refused with that fact rather than treated as a success. The old
    /// parent is returned so the caller can say what the node left.
    pub(super) fn move_out_of_container(&mut self, node: NodeId) -> Result<NodeId, String> {
        if self.kind(node).is_none() {
            return Err(format!("node {} does not exist", node.get()));
        }
        let Some(parent) = self.parent(node) else {
            return Err(format!("node {} is already at the root", node.get()));
        };
        let grandparent = self.parent(parent);
        self.tree
            .reparent(node, grandparent)
            .map_err(|error| error.message())?;
        self.revision += 1;
        self.content_revision += 1;
        Ok(parent)
    }

    /// Moves a node into the container immediately above it among its siblings.
    ///
    /// The target is deterministic — the previous sibling, which must be a `Container` — because the
    /// canvas is not a drop target yet (upstream feedback C-11). The first sibling has nothing above
    /// it, and a non-container previous sibling is refused with its name rather than skipping to
    /// some other container the person did not point at.
    pub(super) fn move_into_previous_container(&mut self, node: NodeId) -> Result<NodeId, String> {
        let Some(entry) = self.tree.get(node) else {
            return Err(format!("node {} does not exist", node.get()));
        };
        let siblings: Vec<NodeId> = match entry.parent {
            Some(parent) => self
                .tree
                .get(parent)
                .map(|holder| holder.children.clone())
                .unwrap_or_default(),
            None => self.tree.roots().to_vec(),
        };
        let position = siblings
            .iter()
            .position(|sibling| *sibling == node)
            .ok_or_else(|| format!("node {} is not among its siblings", node.get()))?;
        if position == 0 {
            return Err(format!("node {} has no sibling above it", node.get()));
        }
        let target = siblings[position - 1];
        if self.kind(target) != Some(Kind::Container) {
            let name = self.name(target).unwrap_or("?");
            return Err(format!("the node above ({name}) is not a container"));
        }
        self.tree
            .reparent(node, Some(target))
            .map_err(|error| error.message())?;
        self.revision += 1;
        self.content_revision += 1;
        Ok(target)
    }

    /// Wraps a node in a new container under the node's current parent.
    ///
    /// The wrapper is a real structural node: it becomes the parent in the tree, carries the demo's
    /// container contract (`panel.title`, the same mapping the demo's own containers use), and is
    /// painted directly behind the wrapped node so the node remains the hit. The container's
    /// rectangle is the node's, grown by 8 design units on each side.
    ///
    /// A failure to place the wrapper behind the node would leave a half-made structure, so the
    /// tree is restored before the error is returned rather than returning a wrapper nobody can
    /// select.
    pub(super) fn wrap_in_container(&mut self, node: NodeId) -> Result<NodeId, String> {
        if self.kind(node).is_none() {
            return Err(format!("node {} does not exist", node.get()));
        }
        let parent = self.parent(node);
        let name = self.unique_name("Group");
        let container = self
            .tree
            .insert(parent, Kind::Container, name)
            .map_err(|error| error.message())?;
        if let Err(error) = self.tree.reparent(node, Some(container)) {
            let _ = self.tree.delete(container);
            return Err(error.message());
        }
        let rect = self.rect(node).map_or(Rect::new(40, 60, 200, 120), |rect| {
            Rect::new(
                (rect.x - 8).max(0),
                (rect.y - 8).max(0),
                rect.width + 16,
                rect.height + 16,
            )
        });
        self.scene.insert(container, rect);
        self.nodes.insert(container, rect);
        self.contracts
            .insert(container, NodeContract::new("panel", "title"));
        if !self.scene.place_below(container, node) {
            let _ = self.tree.reparent(node, parent);
            let _ = self.tree.delete(container);
            self.scene.remove(container);
            self.nodes.remove(&container);
            self.contracts.remove(&container);
            return Err("the wrapper could not be placed behind the node".to_owned());
        }
        self.revision += 1;
        self.content_revision += 1;
        Ok(container)
    }

    /// A node and its descendants in the Outline's order.
    fn subtree_preorder(&self, node: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        self.subtree_preorder_into(node, &mut out);
        out
    }

    fn subtree_preorder_into(&self, node: NodeId, out: &mut Vec<NodeId>) {
        out.push(node);
        if let Some(entry) = self.tree.get(node) {
            for child in &entry.children {
                self.subtree_preorder_into(*child, out);
            }
        }
    }

    /// A node's first child in tree order, or `None` for a leaf.
    ///
    /// "First" is the Outline's order, so the row under the parent is the node this answers with.
    pub(super) fn first_child(&self, node: NodeId) -> Option<NodeId> {
        self.tree
            .get(node)
            .and_then(|entry| entry.children.first().copied())
    }

    /// Renames a node after the Inspector accepted a new title.
    pub(super) fn rename(&mut self, node: NodeId, name: &str) -> Result<(), String> {
        self.tree
            .rename(node, name)
            .map_err(|error| error.message())?;
        self.revision += 1;
        self.content_revision += 1;
        Ok(())
    }

    /// The revision of the picture the window mirrors (names, selection, membership).
    pub(super) fn revision(&self) -> u64 {
        self.revision
    }

    /// The revision of the Outline's row content: names and membership, not selection.
    pub(super) fn content_revision(&self) -> u64 {
        self.content_revision
    }

    /// Clears the selection, which is what a click on empty canvas does.
    pub(super) fn clear_selection(&mut self) -> usize {
        let had = self.selected.len();
        if had > 0 {
            self.selected.clear();
            self.revision += 1;
        }
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
        self.revision += 1;
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
        if selected != self.selected {
            self.selected = selected;
            self.revision += 1;
        }
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
        self.nodes.get(&node).copied()
    }

    /// Every visible node with its rectangle, in paint order (bottom first).
    ///
    /// One pass over the scene order and one map lookup per node, so the cost is linear in the
    /// number of nodes rather than the old scan-within-scan.
    pub(super) fn nodes(&self) -> Vec<Node> {
        self.scene
            .order()
            .iter()
            .filter(|node| !self.scene.is_hidden(**node))
            .filter_map(|node| {
                self.nodes.get(node).map(|rect| Node {
                    node: *node,
                    rect: *rect,
                })
            })
            .collect()
    }

    /// Inserts one control from the widget library as a leaf node.
    ///
    /// The palette entry supplies the contract (`control` and the property the Inspector edits);
    /// this method decides only *where* the node goes. The parent is the nearest container at or
    /// above the selection, or the first root container when nothing selectable is under a
    /// container. The tree's one nesting rule is why the walk starts at the selection's parent for a
    /// leaf: a `Widget` cannot contain a `Widget`.
    ///
    /// Every entry is inserted as [`Kind::Widget`]. Turning it into a container is an explicit
    /// design action, not something a palette click guesses from a name; the catalog publishes no
    /// container/leaf flag.
    ///
    /// The place is a cascade derived from the document size, so two insertions never land exactly
    /// on top of each other and the same document produces the same position. The node is *not*
    /// selected here; the caller decides whether inserting also selects.
    pub(super) fn insert_widget(
        &mut self,
        control: &str,
        name: &str,
        property: &str,
    ) -> Result<NodeId, String> {
        if control.trim().is_empty() {
            return Err("a widget needs a contract name".to_owned());
        }
        if name.trim().is_empty() {
            return Err("a widget needs a display name".to_owned());
        }
        if property.trim().is_empty() {
            return Err(format!(
                "the contract of {control} publishes no property the Inspector could edit"
            ));
        }
        let parent = self.container_for_insertion().ok_or_else(|| {
            "the scene has no container that can hold a widget; add a container first".to_owned()
        })?;
        let name = self.unique_name(name);
        let node = self
            .tree
            .insert(Some(parent), Kind::Widget, name)
            .map_err(|error| error.message())?;
        let ordinal = self.tree.len() as i32;
        let rect = Rect::new(
            60 + (ordinal * 16) % 240,
            90 + (ordinal * 16) % 200,
            160,
            40,
        );
        self.scene.insert(node, rect);
        self.nodes.insert(node, rect);
        self.contracts
            .insert(node, NodeContract::new(control, property));
        self.revision += 1;
        self.content_revision += 1;
        Ok(node)
    }

    /// The nearest container at or above the selection, or the first root container.
    fn container_for_insertion(&self) -> Option<NodeId> {
        let mut candidate = self.selected.first().copied();
        while let Some(node) = candidate {
            if self.kind(node) == Some(Kind::Container) {
                return Some(node);
            }
            candidate = self.parent(node);
        }
        self.tree
            .roots()
            .iter()
            .copied()
            .find(|node| self.kind(*node) == Some(Kind::Container))
    }

    /// `base`, or the first `base N` the tree does not already use.
    ///
    /// A base that already ends in a number continues that series (`button 2` becomes `button 3`),
    /// rather than stacking suffixes (`button 2 2`).
    fn unique_name(&self, base: &str) -> String {
        if !self.has_name(base) {
            return base.to_owned();
        }
        let (stem, first) = match base.rsplit_once(' ') {
            Some((stem, tail))
                if !stem.is_empty()
                    && !tail.is_empty()
                    && tail.chars().all(|character| character.is_ascii_digit()) =>
            {
                (
                    stem,
                    tail.parse::<u64>().unwrap_or(1).saturating_add(1).max(2),
                )
            }
            _ => (base, 2),
        };
        let mut suffix = first;
        loop {
            let candidate = format!("{stem} {suffix}");
            if !self.has_name(&candidate) {
                return candidate;
            }
            suffix += 1;
        }
    }

    fn has_name(&self, name: &str) -> bool {
        self.tree
            .ids()
            .iter()
            .any(|node| self.name(*node) == Some(name))
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
    /// The scene context menu's Delete calls this; the model was written and tested before the menu
    /// existed so the action could not need a second structural tree. The whole subtree goes with
    /// the node, and every removed instance and rectangle is dropped so no descendant is left behind
    /// as a hit-test target. The selection keeps only nodes that survived.
    pub(super) fn delete(&mut self, node: NodeId) -> Result<String, String> {
        let report = self.tree.delete(node).map_err(|error| error.message())?;
        // The structural delete removes the whole subtree; every removed instance and its
        // rectangle go with it, so no descendant is left behind as a hit-test target.
        for removed in &report.removed {
            self.scene.remove(*removed);
            self.nodes.remove(removed);
            self.contracts.remove(removed);
            self.selected.retain(|selected| selected != removed);
        }
        self.revision += 1;
        self.content_revision += 1;
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

    /// Hides or shows a node. Hiding a node removes it from the selection, so the window never
    /// shows an inspector for something the canvas no longer draws.
    pub(super) fn set_hidden(&mut self, node: NodeId, hidden: bool) {
        self.scene.set_hidden(node, hidden);
        if hidden {
            self.selected.retain(|selected| *selected != node);
        }
        self.revision += 1;
        self.content_revision += 1;
    }

    /// Locks or unlocks a node for design editing.
    pub(super) fn set_locked(&mut self, node: NodeId, locked: bool) {
        self.scene.set_locked(node, locked);
        self.revision += 1;
    }

    /// Moves a node forward in paint and hit-test order.
    pub(super) fn raise(&mut self, node: NodeId) -> bool {
        let raised = self.scene.raise(node);
        if raised {
            self.revision += 1;
            self.content_revision += 1;
        }
        raised
    }

    /// Moves a node one step up the paint order.
    pub(super) fn raise_one(&mut self, node: NodeId) -> bool {
        let moved = self.scene.raise_one(node);
        if moved {
            self.revision += 1;
        }
        moved
    }

    /// Moves a node one step down the paint order.
    pub(super) fn lower_one(&mut self, node: NodeId) -> bool {
        let moved = self.scene.lower_one(node);
        if moved {
            self.revision += 1;
        }
        moved
    }

    /// Sends a node to the back of the paint order.
    pub(super) fn send_to_back(&mut self, node: NodeId) -> bool {
        let moved = self.scene.lower(node);
        if moved {
            self.revision += 1;
        }
        moved
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

    // Canvas node rectangles are design-space facts owned by `CanvasState`, and the canvas
    // transform is the one place that maps them into widget space. Child hints from the library
    // describe preferred sizes, which an absolutely-positioned design canvas does not use; feeding
    // them in would turn an explicit design position into a layout guess. The context is likewise
    // not applied here because the viewport transform owns scaling.
    fn arrange(
        &self,
        rect: WidgetRect,
        _children: &[ChildInfo],
        out: &mut dyn FnMut(ObjectId, WidgetRect),
    ) {
        self.update(rect, out);
    }

    fn arrange_with_context(
        &self,
        rect: WidgetRect,
        _children: &[ChildInfo],
        _context: &LayoutContext,
        out: &mut dyn FnMut(ObjectId, WidgetRect),
    ) {
        self.update(rect, out);
    }

    fn update_with_context(
        &self,
        rect: WidgetRect,
        _context: &LayoutContext,
        widgets: &mut dyn FnMut(ObjectId, WidgetRect),
    ) {
        self.update(rect, widgets);
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
    use crate::contracts::identity::NodeId;

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
    fn the_demo_mapping_is_stored_per_node_not_guessed_from_the_display_name() {
        let mut state = CanvasState::demo();
        let rows = state.row_nodes();
        let contracts: Vec<String> = rows
            .iter()
            .map(|node| state.contract(*node).expect("a mapped node").render())
            .collect();
        assert_eq!(
            contracts,
            vec![
                "window.title",
                "panel.title",
                "button.text",
                "label.text",
                "label.text",
                "button.text",
            ]
        );

        // Renaming the node changes what the tree shows, never which contract the node edits.
        let button = rows[2];
        state.rename(button, "primary action").expect("rename");
        assert_eq!(
            state.contract(button).expect("still mapped").render(),
            "button.text"
        );

        let unknown = state
            .contract(NodeId::from_raw(999))
            .expect_err("no mapping");
        assert!(unknown.contains("no contract mapping"), "{unknown}");
    }

    #[test]
    fn deleting_a_node_removes_its_contract_mapping() {
        let mut state = CanvasState::demo();
        let panel = state.row_nodes()[1];
        state.delete(panel).expect("delete");
        let reason = state.contract(panel).expect_err("the mapping is gone");
        assert!(reason.contains("no contract mapping"), "{reason}");
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

    #[test]
    fn the_scene_menu_mutates_hide_lock_and_order_without_a_second_state() {
        let mut state = CanvasState::demo();
        let rows = state.row_nodes();
        let button = rows[2];
        let label = rows[3];
        state.select_only(button).expect("selectable");

        state.set_locked(button, true);
        assert!(state.is_locked(button));
        assert_eq!(state.selected(), [button], "locking keeps the selection");
        state.set_locked(button, false);

        // Raising the button puts it above the overlapping label.
        assert_eq!(state.hit(150, 155), Some(label));
        assert!(state.raise(button));
        assert_eq!(state.hit(150, 155), Some(button));

        // Hiding the selected node removes it from the selection; showing it again does not
        // restore a selection the designer did not make.
        state.set_hidden(button, true);
        assert!(state.is_hidden(button));
        assert!(state.selected().is_empty());
        state.set_hidden(button, false);
        assert!(!state.is_hidden(button));
    }

    #[test]
    fn a_library_widget_lands_in_the_nearest_container_with_a_unique_name() {
        let mut state = CanvasState::demo();
        let rows = state.row_nodes();
        let panel = rows[1];
        let button = rows[2];
        // A leaf is selected; a Widget cannot contain a Widget, so the walk goes up to the panel.
        state.select_only(button).expect("selectable");

        let node = state
            .insert_widget("slider", "slider", "value")
            .expect("inserted");

        assert_eq!(state.parent(node), Some(panel));
        assert_eq!(
            state.kind(node),
            Some(crate::app::canvas::structure::Kind::Widget)
        );
        assert_eq!(state.name(node), Some("slider"));
        assert_eq!(state.contract(node).expect("a contract").control, "slider");
        assert!(state.rect(node).is_some(), "the scene can hit-test it");

        // The same display name is not reused; the second node is still a distinct place and id.
        let second = state
            .insert_widget("slider", "slider", "value")
            .expect("inserted");
        assert_eq!(state.name(second), Some("slider 2"));
        assert_ne!(state.rect(node), state.rect(second));
        assert_ne!(node, second);
    }

    #[test]
    fn a_widget_with_no_editable_property_is_refused_before_anything_is_inserted() {
        let mut state = CanvasState::demo();
        let before = state.row_nodes();

        let refusal = state
            .insert_widget("panel", "panel", " ")
            .expect_err("no property means no Inspector contract");

        assert!(refusal.contains("no property"), "{refusal}");
        assert_eq!(state.row_nodes(), before, "the scene is untouched");
    }

    #[test]
    fn the_first_child_is_the_row_under_the_parent_and_a_leaf_has_none() {
        let mut state = CanvasState::demo();
        let rows = state.row_nodes();
        assert_eq!(
            state.first_child(rows[1]),
            Some(rows[2]),
            "Demo holds button"
        );
        assert_eq!(state.first_child(rows[2]), None, "a leaf has no child");
        assert_eq!(state.parent(rows[2]), Some(rows[1]));
    }

    #[test]
    fn duplicating_a_subtree_keeps_rects_contracts_and_a_unique_name() {
        let mut state = CanvasState::demo();
        let rows = state.row_nodes();
        let panel = rows[1];
        let before = state.row_nodes().len();

        let copy = state.duplicate(panel).expect("duplicated");

        // Demo holds the button, the label and the hidden note.
        assert_eq!(state.row_nodes().len(), before + 4);
        assert_eq!(state.name(copy), Some("Demo copy"));
        assert_eq!(state.contract(copy).expect("a contract").control, "panel");
        assert_eq!(
            state
                .first_child(copy)
                .and_then(|child| state.contract(child).ok())
                .map(|contract| contract.control),
            Some("button".to_owned()),
            "the copy keeps the source's child contracts"
        );
        assert!(state.rect(copy).is_some(), "the copy is hit-testable");
        assert_eq!(state.selected(), [copy], "the copy is selected");
    }

    #[test]
    fn sending_the_top_node_back_makes_the_lower_one_the_hit() {
        let mut state = CanvasState::demo();
        let rows = state.row_nodes();
        let button = rows[2];
        let label = rows[3];
        // The label was inserted after the button and overlaps it, so it is the hit.
        assert_eq!(state.hit(150, 155), Some(label));
        assert!(state.send_to_back(label));
        assert_eq!(state.hit(150, 155), Some(button));
    }

    #[test]
    fn wrapping_a_node_makes_a_container_behind_it_with_the_same_contract_convention() {
        let mut state = CanvasState::demo();
        let rows = state.row_nodes();
        let panel = rows[1];
        let button = rows[2];

        let container = state.wrap_in_container(button).expect("wrapped");

        assert_eq!(state.parent(container), Some(panel));
        assert_eq!(state.parent(button), Some(container));
        assert_eq!(state.name(container), Some("Group"));
        assert_eq!(
            state.contract(container).expect("a contract").control,
            "panel"
        );
        let container_rect = state.rect(container).expect("a rectangle");
        let button_rect = state.rect(button).expect("a rectangle");
        assert!(container_rect.x <= button_rect.x);
        assert!(container_rect.y <= button_rect.y);
        // The wrapper is behind the node, so the node is still the hit inside it.
        assert_eq!(state.hit(90, 125), Some(button));
        // And the next palette insertion can land inside the new container.
        state.select_only(container).expect("selectable");
        let inserted = state
            .insert_widget("button", "inside", "text")
            .expect("inserted");
        assert_eq!(state.parent(inserted), Some(container));
    }

    #[test]
    fn moving_a_node_out_of_its_wrapper_restores_the_previous_parent() {
        let mut state = CanvasState::demo();
        let rows = state.row_nodes();
        let panel = rows[1];
        let button = rows[2];
        let container = state.wrap_in_container(button).expect("wrapped");
        assert_eq!(state.parent(button), Some(container));

        let left = state.move_out_of_container(button).expect("moved out");

        assert_eq!(left, container, "the old parent is reported");
        assert_eq!(state.parent(button), Some(panel));
        assert!(state.rect(button).is_some(), "the node keeps its rectangle");
        // A root node has no container to leave.
        let root = state.row_nodes()[0];
        let refusal = state
            .move_out_of_container(root)
            .expect_err("the root has no parent");
        assert!(refusal.contains("already at the root"), "{refusal}");
    }

    #[test]
    fn a_copied_subtree_pastes_with_fresh_ids_and_the_same_contracts() {
        let mut state = CanvasState::demo();
        let rows = state.row_nodes();
        let panel = rows[1];
        let button = rows[2];
        let before = state.row_nodes().len();

        let copy = state.copy_subtree(button).expect("copied");
        assert_eq!(copy.len(), 1);
        state.select_only(button).expect("selectable");
        let root = state.paste_clipboard(&copy).expect("pasted");

        assert_eq!(state.row_nodes().len(), before + 1);
        assert_ne!(root, button, "a paste has its own id");
        // The selection is the button, whose parent is the Demo panel.
        assert_eq!(state.parent(root), Some(panel));
        assert_eq!(state.contract(root).expect("a contract").control, "button");
        assert_eq!(state.name(root), Some("button 2"));

        // A whole subtree keeps its shape and lands inside the selected container. The panel now
        // holds the pasted button copy as well, so its subtree is five nodes.
        let panel_copy = state.copy_subtree(panel).expect("copied panel");
        assert_eq!(panel_copy.len(), 5);
        state.select_only(panel).expect("selectable");
        let pasted_panel = state.paste_clipboard(&panel_copy).expect("pasted panel");
        assert_eq!(state.parent(pasted_panel), Some(panel));
        assert_eq!(state.row_nodes().len(), before + 1 + 5);
    }

    #[test]
    fn a_pasted_name_continues_the_number_series_instead_of_stacking() {
        let mut state = CanvasState::demo();
        let rows = state.row_nodes();
        let button = rows[2];
        state.select_only(button).expect("selectable");

        let copy = state.copy_subtree(button).expect("copied");
        let first = state.paste_clipboard(&copy).expect("pasted");
        assert_eq!(state.name(first), Some("button 2"));

        let again = state.copy_subtree(first).expect("copied the copy");
        let second = state.paste_clipboard(&again).expect("pasted again");
        assert_eq!(state.name(second), Some("button 3"));
    }

    #[test]
    fn a_node_can_move_into_the_container_above_it() {
        let mut state = CanvasState::demo();
        let rows = state.row_nodes();
        let button = rows[2];
        let label = rows[3];
        let group = state.wrap_in_container(label).expect("wrapped");
        // The wrapper is appended last; put it above the button so it is the previous sibling.
        state.tree.reorder(group, 0).expect("reordered");

        let target = state
            .move_into_previous_container(button)
            .expect("moved into the container above");

        assert_eq!(target, group);
        assert_eq!(state.parent(button), Some(group));

        let refusal = state
            .move_into_previous_container(group)
            .expect_err("nothing is above the first sibling");
        assert!(refusal.contains("no sibling above"), "{refusal}");
    }
}
