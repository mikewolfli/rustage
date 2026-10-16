// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Structural edits on the object tree: insert, delete, copy, rename, reorder (task T09.04).
//!
//! The acceptance line is *"no cycle and no duplicate id, and deleting a referenced node warns"*.
//! Three invariants the tree must hold, so the module enforces each at the one place it could break:
//!
//! * **Ids are unique because they come from the allocator.** [`Tree::insert`] takes its id from
//!   [`IdAllocator`], which never reissues one, so a duplicate id is not a state the tree can reach —
//!   it is not checked after the fact but made impossible.
//! * **A cycle is refused before the link is made.** [`Tree::reparent`] refuses a parent that is
//!   already inside the node's own subtree, so the tree stays a tree.
//! * **A delete reports what referred to the node.** [`Tree::delete`] returns a [`DeleteReport`] whose
//!   `references` list every node that pointed at something removed, so "this will break a binding"
//!   is a fact returned to the caller rather than discovered later.
//! * **A child must be a kind its parent allows.** [`Kind::allows_child`] is the one rule, checked on
//!   insert and on reparent, so an invalid nesting is refused with the two kinds named.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use crate::contracts::identity::{IdAllocator, NodeId};

/// What a node is, and therefore what it may contain.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// A container, which may hold anything.
    Container,
    /// A leaf control, which may hold text or an image.
    Widget,
    /// A run of text.
    Text,
    /// An image.
    Image,
}

impl Kind {
    /// Every kind, in a stable order.
    pub const ALL: [Self; 4] = [Self::Container, Self::Widget, Self::Text, Self::Image];

    /// The kind's name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Container => "container",
            Self::Widget => "widget",
            Self::Text => "text",
            Self::Image => "image",
        }
    }

    /// The kind a name picks.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|kind| kind.name() == name)
    }

    /// Whether a kind may contain another. The one nesting rule.
    pub fn allows_child(self, child: Kind) -> bool {
        match self {
            Self::Container => true,
            Self::Widget => matches!(child, Self::Text | Self::Image),
            Self::Text | Self::Image => false,
        }
    }
}

/// One node of the tree.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Node {
    /// The stable id.
    pub id: NodeId,
    /// What it is.
    pub kind: Kind,
    /// Its name.
    pub name: String,
    /// Its parent, or `None` for a root.
    pub parent: Option<NodeId>,
    /// Its children, in order.
    pub children: Vec<NodeId>,
}

/// Why a structural edit was refused.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum StructError {
    /// The node does not exist.
    Missing {
        /// The node.
        node: NodeId,
    },
    /// The child's kind is not allowed inside the parent's kind.
    ChildNotAllowed {
        /// The parent's kind.
        parent: Kind,
        /// The child's kind.
        child: Kind,
    },
    /// The move would make a node its own ancestor.
    Cycle {
        /// The node being moved.
        node: NodeId,
        /// The parent it was moving under.
        parent: NodeId,
    },
    /// The index is past the end of the sibling list.
    IndexOutOfRange {
        /// The index asked for.
        index: usize,
        /// How many siblings there are.
        len: usize,
    },
}

impl StructError {
    /// The refusal in one sentence.
    pub fn message(&self) -> String {
        match self {
            Self::Missing { node } => format!("node {} does not exist", node.get()),
            Self::ChildNotAllowed { parent, child } => {
                format!("a {} cannot contain a {}", parent.name(), child.name())
            }
            Self::Cycle { node, parent } => format!(
                "moving node {} under node {} would make a cycle",
                node.get(),
                parent.get()
            ),
            Self::IndexOutOfRange { index, len } => {
                format!("there is no position {index} among {len} sibling(s)")
            }
        }
    }
}

/// What a delete removed, and what referred to it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DeleteReport {
    /// Every node removed (the subject and its descendants).
    pub removed: Vec<NodeId>,
    /// The references that pointed at a removed node, as `(referrer, removed)`.
    pub references: Vec<(NodeId, NodeId)>,
}

impl DeleteReport {
    /// The report in one line.
    pub fn render(&self) -> String {
        format!(
            "deleted {} node(s), {} reference(s) to fix",
            self.removed.len(),
            self.references.len()
        )
    }
}

/// The object tree.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Tree {
    nodes: BTreeMap<NodeId, Node>,
    roots: Vec<NodeId>,
    allocator: IdAllocator,
    references: BTreeMap<NodeId, BTreeSet<NodeId>>,
}

impl Tree {
    /// An empty tree.
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts a node under a parent (or at the root), taking a fresh id from the allocator.
    pub fn insert(
        &mut self,
        parent: Option<NodeId>,
        kind: Kind,
        name: impl Into<String>,
    ) -> Result<NodeId, StructError> {
        if let Some(parent) = parent {
            let holder = self
                .nodes
                .get(&parent)
                .ok_or(StructError::Missing { node: parent })?;
            if !holder.kind.allows_child(kind) {
                return Err(StructError::ChildNotAllowed {
                    parent: holder.kind,
                    child: kind,
                });
            }
        }
        let id = self.allocator.allocate();
        self.nodes.insert(
            id,
            Node {
                id,
                kind,
                name: name.into(),
                parent,
                children: Vec::new(),
            },
        );
        match parent {
            Some(parent) => self
                .nodes
                .get_mut(&parent)
                .expect("a checked parent")
                .children
                .push(id),
            None => self.roots.push(id),
        }
        Ok(id)
    }

    /// A node, when it exists.
    pub fn get(&self, node: NodeId) -> Option<&Node> {
        self.nodes.get(&node)
    }

    /// Every id, in id order.
    pub fn ids(&self) -> Vec<NodeId> {
        self.nodes.keys().copied().collect()
    }

    /// How many nodes there are.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// The root ids, in order.
    pub fn roots(&self) -> &[NodeId] {
        &self.roots
    }

    /// Records that one node refers to another (a binding, a style link, ...).
    pub fn reference(&mut self, from: NodeId, to: NodeId) {
        self.references.entry(from).or_default().insert(to);
    }

    /// Deletes a node and its subtree, reporting what referred to it.
    pub fn delete(&mut self, node: NodeId) -> Result<DeleteReport, StructError> {
        let target = self.nodes.get(&node).ok_or(StructError::Missing { node })?;
        let parent = target.parent;
        let mut removed = Vec::new();
        let mut stack = vec![node];
        while let Some(current) = stack.pop() {
            if removed.contains(&current) {
                continue;
            }
            removed.push(current);
            if let Some(holder) = self.nodes.get(&current) {
                stack.extend(holder.children.iter().copied());
            }
        }
        // Every reference that points at a removed node.
        let mut references = Vec::new();
        for (from, targets) in &self.references {
            for target in targets {
                if removed.contains(target) && !removed.contains(from) {
                    references.push((*from, *target));
                }
            }
        }
        // Unlink from the parent (or the roots), then drop the nodes and retire the ids.
        match parent {
            Some(parent) => {
                if let Some(holder) = self.nodes.get_mut(&parent) {
                    holder.children.retain(|child| *child != node);
                }
            }
            None => self.roots.retain(|root| *root != node),
        }
        for id in &removed {
            self.nodes.remove(id);
            self.allocator.retire(*id);
            self.references.remove(id);
            for targets in self.references.values_mut() {
                targets.remove(id);
            }
        }
        Ok(DeleteReport {
            removed,
            references,
        })
    }

    /// Deep-copies a node's subtree with fresh ids, under the same parent.
    pub fn copy(&mut self, node: NodeId) -> Result<NodeId, StructError> {
        let target = self.nodes.get(&node).ok_or(StructError::Missing { node })?;
        let parent = target.parent;
        let clone = self.copy_subtree(node, parent)?;
        Ok(clone)
    }

    /// Renames a node.
    pub fn rename(&mut self, node: NodeId, name: impl Into<String>) -> Result<(), StructError> {
        let target = self
            .nodes
            .get_mut(&node)
            .ok_or(StructError::Missing { node })?;
        target.name = name.into();
        Ok(())
    }

    /// Moves a node within its sibling list.
    pub fn reorder(&mut self, node: NodeId, index: usize) -> Result<(), StructError> {
        let target = self.nodes.get(&node).ok_or(StructError::Missing { node })?;
        let parent = target.parent;
        let siblings = match parent {
            Some(parent) => {
                &mut self
                    .nodes
                    .get_mut(&parent)
                    .expect("a checked parent")
                    .children
            }
            None => &mut self.roots,
        };
        let Some(position) = siblings.iter().position(|sibling| *sibling == node) else {
            return Err(StructError::Missing { node });
        };
        if index >= siblings.len() {
            return Err(StructError::IndexOutOfRange {
                index,
                len: siblings.len(),
            });
        }
        let moved = siblings.remove(position);
        siblings.insert(index, moved);
        Ok(())
    }

    /// Moves a node under a new parent (or to the root), refusing a cycle.
    pub fn reparent(&mut self, node: NodeId, parent: Option<NodeId>) -> Result<(), StructError> {
        let index = self.insert_slots(node, parent);
        self.reparent_at(node, parent, index)
    }

    /// How many positions a parent offers for `node`, counting the slots the node itself would leave.
    ///
    /// The dragged node is not counted among its new siblings, so index `0..=slots` is the range a
    /// drop may name.
    pub fn insert_slots(&self, node: NodeId, parent: Option<NodeId>) -> usize {
        let old = self.nodes.get(&node).and_then(|holder| holder.parent);
        match parent {
            Some(parent) => {
                let count = self
                    .nodes
                    .get(&parent)
                    .map_or(0, |holder| holder.children.len());
                if old == Some(parent) {
                    count.saturating_sub(1)
                } else {
                    count
                }
            }
            None => {
                let count = self.roots.len();
                if old.is_none() {
                    count.saturating_sub(1)
                } else {
                    count
                }
            }
        }
    }

    /// Whether `node` may be moved under `parent` at `index`, without moving it.
    ///
    /// The one place the move rules live, so a preview and the move itself cannot disagree.
    pub fn can_reparent_at(
        &self,
        node: NodeId,
        parent: Option<NodeId>,
        index: usize,
    ) -> Result<(), StructError> {
        let target = self.nodes.get(&node).ok_or(StructError::Missing { node })?;
        let kind = target.kind;
        if let Some(parent) = parent {
            let holder = self
                .nodes
                .get(&parent)
                .ok_or(StructError::Missing { node: parent })?;
            if !holder.kind.allows_child(kind) {
                return Err(StructError::ChildNotAllowed {
                    parent: holder.kind,
                    child: kind,
                });
            }
            if parent == node || self.is_descendant(parent, node) {
                return Err(StructError::Cycle { node, parent });
            }
        }
        let slots = self.insert_slots(node, parent);
        if index > slots {
            return Err(StructError::IndexOutOfRange { index, len: slots });
        }
        Ok(())
    }

    /// Moves `node` under `parent` at `index`, refusing anything [`Self::can_reparent_at`] refuses.
    pub fn reparent_at(
        &mut self,
        node: NodeId,
        parent: Option<NodeId>,
        index: usize,
    ) -> Result<(), StructError> {
        self.can_reparent_at(node, parent, index)?;
        let old_parent = self.nodes.get(&node).expect("a checked node").parent;
        match old_parent {
            Some(old) => {
                if let Some(holder) = self.nodes.get_mut(&old) {
                    holder.children.retain(|child| *child != node);
                }
            }
            None => self.roots.retain(|root| *root != node),
        }
        match parent {
            Some(parent) => {
                let children = &mut self
                    .nodes
                    .get_mut(&parent)
                    .expect("a checked parent")
                    .children;
                let at = index.min(children.len());
                children.insert(at, node);
            }
            None => {
                let at = index.min(self.roots.len());
                self.roots.insert(at, node);
            }
        }
        self.nodes.get_mut(&node).expect("a checked node").parent = parent;
        Ok(())
    }

    /// Whether every node is reachable from a root exactly once (a tree, not a graph with a cycle).
    pub fn is_acyclic(&self) -> bool {
        let mut seen = BTreeSet::new();
        let mut stack: Vec<NodeId> = self.roots.clone();
        while let Some(current) = stack.pop() {
            if !seen.insert(current) {
                return false;
            }
            if let Some(node) = self.nodes.get(&current) {
                stack.extend(node.children.iter().copied());
            }
        }
        seen.len() == self.nodes.len()
    }

    /// Whether `candidate` is inside `node`'s subtree.
    pub fn is_descendant(&self, candidate: NodeId, node: NodeId) -> bool {
        let mut stack = vec![node];
        while let Some(current) = stack.pop() {
            if let Some(holder) = self.nodes.get(&current) {
                for child in &holder.children {
                    if *child == candidate {
                        return true;
                    }
                    stack.push(*child);
                }
            }
        }
        false
    }

    /// Copies a subtree, returning the new root's id.
    fn copy_subtree(
        &mut self,
        node: NodeId,
        parent: Option<NodeId>,
    ) -> Result<NodeId, StructError> {
        let source = self.nodes.get(&node).ok_or(StructError::Missing { node })?;
        let kind = source.kind;
        let name = source.name.clone();
        let children = source.children.clone();
        let clone = self.insert(parent, kind, name)?;
        for child in children {
            self.copy_subtree(child, Some(clone))?;
        }
        Ok(clone)
    }

    /// The tree in one report: an indented outline.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!("tree: {} node(s)", self.nodes.len())];
        for root in &self.roots {
            self.render_node(*root, 0, &mut lines);
        }
        lines
    }

    /// One node and its descendants, indented.
    fn render_node(&self, node: NodeId, depth: usize, lines: &mut Vec<String>) {
        if let Some(holder) = self.nodes.get(&node) {
            lines.push(format!(
                "{}{} [{}] #{}",
                "  ".repeat(depth),
                holder.name,
                holder.kind.name(),
                node.get()
            ));
            for child in &holder.children {
                self.render_node(*child, depth + 1, lines);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Kind, StructError, Tree};
    use crate::contracts::identity::NodeId;

    #[test]
    fn a_child_must_be_a_kind_its_parent_allows() {
        let mut tree = Tree::new();
        let text = tree.insert(None, Kind::Text, "t").expect("a text root");
        // A text cannot contain anything.
        assert_eq!(
            tree.insert(Some(text), Kind::Image, "i"),
            Err(StructError::ChildNotAllowed {
                parent: Kind::Text,
                child: Kind::Image,
            })
        );
        // A widget may hold text.
        let widget = tree.insert(None, Kind::Widget, "w").expect("a widget");
        assert!(tree.insert(Some(widget), Kind::Text, "label").is_ok());
    }

    #[test]
    fn ids_are_unique_because_the_allocator_never_reissues() {
        let mut tree = Tree::new();
        let mut ids = Vec::new();
        for index in 0..50 {
            ids.push(
                tree.insert(None, Kind::Container, format!("c{index}"))
                    .expect("insert"),
            );
        }
        let unique: std::collections::BTreeSet<NodeId> = ids.iter().copied().collect();
        assert_eq!(unique.len(), ids.len());
        // Deleting and inserting again does not reuse the id.
        let removed = ids[0];
        tree.delete(removed).expect("delete");
        let fresh = tree.insert(None, Kind::Container, "new").expect("insert");
        assert!(fresh != removed && !tree.ids().contains(&removed));
    }

    #[test]
    fn a_copied_subtree_gets_fresh_ids_and_keeps_the_structure() {
        let mut tree = Tree::new();
        let parent = tree.insert(None, Kind::Container, "panel").expect("panel");
        let child = tree
            .insert(Some(parent), Kind::Widget, "button")
            .expect("button");
        tree.insert(Some(child), Kind::Text, "label")
            .expect("label");
        let copy = tree.copy(parent).expect("copy");
        assert!(copy != parent);
        assert_eq!(tree.get(copy).expect("copy").children.len(), 1);
        assert!(tree.is_acyclic());
        // Both subtrees exist, so the count grew by the three copied nodes.
        assert_eq!(tree.len(), 6);
    }

    #[test]
    fn a_reparent_that_would_make_a_cycle_is_refused() {
        let mut tree = Tree::new();
        let outer = tree.insert(None, Kind::Container, "outer").expect("outer");
        let inner = tree
            .insert(Some(outer), Kind::Container, "inner")
            .expect("inner");
        // Moving outer under inner (its own descendant) would make a cycle.
        assert_eq!(
            tree.reparent(outer, Some(inner)),
            Err(StructError::Cycle {
                node: outer,
                parent: inner
            })
        );
        assert!(tree.is_acyclic());
    }

    #[test]
    fn deleting_reports_every_reference_to_a_removed_node() {
        let mut tree = Tree::new();
        let a = tree.insert(None, Kind::Widget, "a").expect("a");
        let b = tree.insert(None, Kind::Widget, "b").expect("b");
        tree.reference(b, a);
        let report = tree.delete(a).expect("delete");
        assert_eq!(report.removed, vec![a]);
        assert_eq!(report.references, vec![(b, a)]);
        assert!(report.render().contains("1 reference(s)"));
    }

    #[test]
    fn reorder_moves_a_node_within_its_siblings() {
        let mut tree = Tree::new();
        let parent = tree.insert(None, Kind::Container, "p").expect("p");
        let one = tree.insert(Some(parent), Kind::Widget, "one").expect("one");
        let two = tree.insert(Some(parent), Kind::Widget, "two").expect("two");
        tree.reorder(two, 0).expect("reorder");
        assert_eq!(tree.get(parent).expect("p").children, vec![two, one]);
        assert!(tree.reorder(one, 9).is_err());
    }

    #[test]
    fn rename_changes_only_the_name() {
        let mut tree = Tree::new();
        let node = tree.insert(None, Kind::Widget, "old").expect("node");
        tree.rename(node, "new").expect("rename");
        assert_eq!(tree.get(node).expect("node").name, "new");
        assert_eq!(
            tree.rename(NodeId::from_raw(999), "x"),
            Err(StructError::Missing {
                node: NodeId::from_raw(999)
            })
        );
    }

    #[test]
    fn a_kind_name_round_trips() {
        for kind in Kind::ALL {
            assert_eq!(Kind::from_name(kind.name()), Some(kind));
        }
        assert_eq!(Kind::from_name("nowhere"), None);
    }
}
