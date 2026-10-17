// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! A tree or list control's items: hierarchy, expansion, selection, lazy loading and icon roles
//! (task T27.03).
//!
//! The acceptance line is *"a cycle / duplicate id is refused; an async-load error can be displayed"*.
//! The two halves are the two ways a tree can lie about itself:
//!
//! * **The structure is a tree, or it is refused.** [`Tree::add`] refuses a duplicate [`ItemId`]; a
//!   network of items where one is its own ancestor is not a tree, so [`Tree::reparent`] refuses the
//!   move that would close a cycle and [`Tree::validate`] refuses a structure that already contains
//!   one. Nothing is silently accepted and then drawn wrong.
//! * **A failed load is shown, not hidden.** A lazily loaded item holds a [`Lazy`] state. When the
//!   load fails, or answers an unknown result, the item becomes [`Lazy::Failed`] with the reason, and
//!   [`Tree::render`] prints it. A cancelled load settles as [`Lazy::Pending`] and is never mistaken
//!   for loaded content — the [`Outcome`]/[`Cancel`] shape from the adapters is reused so that "still
//!   not loaded" and "loaded" cannot be confused.

#![allow(dead_code)]

use std::collections::BTreeSet;

use crate::app::data::adapters::{Adapter, Cancel, Kind, Outcome, Request};

/// An item's stable identity inside one tree.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ItemId(u64);

impl ItemId {
    /// Rebuilds an item id from the number it was written as.
    pub const fn from_raw(value: u64) -> Self {
        Self(value)
    }

    /// The number this id is written as.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// What an item's icon says it is.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum IconRole {
    /// No icon.
    None,
    /// A container that can hold children.
    Container,
    /// A leaf that cannot.
    Leaf,
    /// An icon drawn from a named set.
    Status(String),
}

impl IconRole {
    /// The role's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Container => "container",
            Self::Leaf => "leaf",
            Self::Status(_) => "status",
        }
    }

    /// Reads a role's name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "none" => Some(Self::None),
            "container" => Some(Self::Container),
            "leaf" => Some(Self::Leaf),
            _ => None,
        }
    }
}

/// A lazily loaded item's state.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Lazy {
    /// Not a lazily loaded item; its children are already in the tree.
    Eager,
    /// Lazily loaded and not yet asked for.
    Pending,
    /// The load is in flight.
    Loading,
    /// The load returned content.
    Loaded,
    /// The load failed or returned an unknown result; the reason is shown.
    Failed(String),
}

impl Lazy {
    /// The state's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Eager => "eager",
            Self::Pending => "pending",
            Self::Loading => "loading",
            Self::Loaded => "loaded",
            Self::Failed(_) => "failed",
        }
    }

    /// The failure reason, when the load failed.
    pub fn error(&self) -> Option<&str> {
        match self {
            Self::Failed(reason) => Some(reason),
            _ => None,
        }
    }
}

/// One item in a tree.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Node {
    /// The item's stable identity.
    pub id: ItemId,
    /// The item's parent, or `None` for a root.
    pub parent: Option<ItemId>,
    /// The item's label.
    pub label: String,
    /// What the item's icon says it is.
    pub icon: IconRole,
    /// Whether the item's children are shown.
    pub expanded: bool,
    /// How the item's children are loaded.
    pub lazy: Lazy,
}

/// A tree of items.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Tree {
    /// The items.
    pub nodes: Vec<Node>,
    /// The selected item, by identity.
    pub selected: Option<ItemId>,
}

impl Tree {
    /// An empty tree.
    pub fn new() -> Self {
        Self::default()
    }

    /// The item with this identity.
    pub fn node(&self, id: ItemId) -> Option<&Node> {
        self.nodes.iter().find(|node| node.id == id)
    }

    /// The item with this identity, mutably.
    pub fn node_mut(&mut self, id: ItemId) -> Option<&mut Node> {
        self.nodes.iter_mut().find(|node| node.id == id)
    }

    /// Adds an item, refusing a duplicate identity or an unknown parent.
    pub fn add(
        &mut self,
        id: ItemId,
        parent: Option<ItemId>,
        label: &str,
        icon: IconRole,
    ) -> Result<(), String> {
        if self.node(id).is_some() {
            return Err(format!("item id {} already exists", id.get()));
        }
        if let Some(parent) = parent
            && self.node(parent).is_none()
        {
            return Err(format!("parent {} does not exist", parent.get()));
        }
        self.nodes.push(Node {
            id,
            parent,
            label: label.to_owned(),
            icon,
            expanded: false,
            lazy: Lazy::Eager,
        });
        Ok(())
    }

    /// The items whose parent is `parent`, in insertion order.
    pub fn children(&self, parent: Option<ItemId>) -> Vec<&Node> {
        self.nodes
            .iter()
            .filter(|node| node.parent == parent)
            .collect()
    }

    /// Moves an item under a new parent.
    ///
    /// Refuses the move that would make the item an ancestor of itself: walking up from the new
    /// parent must never reach the item. A cycle is not a tree, so it is refused rather than stored.
    pub fn reparent(&mut self, id: ItemId, parent: Option<ItemId>) -> Result<(), String> {
        if self.node(id).is_none() {
            return Err(format!("item {} does not exist", id.get()));
        }
        if let Some(parent) = parent {
            if self.node(parent).is_none() {
                return Err(format!("parent {} does not exist", parent.get()));
            }
            let mut walk = Some(parent);
            while let Some(current) = walk {
                if current == id {
                    return Err(format!(
                        "moving {} under {} would make a cycle",
                        id.get(),
                        parent.get()
                    ));
                }
                walk = self.node(current).and_then(|node| node.parent);
            }
        }
        if let Some(node) = self.node_mut(id) {
            node.parent = parent;
        }
        Ok(())
    }

    /// Marks an item as lazily loaded and not yet asked for.
    pub fn make_lazy(&mut self, id: ItemId) -> Result<(), String> {
        let node = self
            .node_mut(id)
            .ok_or_else(|| format!("item {} does not exist", id.get()))?;
        node.lazy = Lazy::Pending;
        Ok(())
    }

    /// Loads a lazily loaded item through an adapter.
    ///
    /// The outcome is stored on the item so it can be displayed: a failure, or an unknown result,
    /// becomes [`Lazy::Failed`] with the reason; a cancellation settles as [`Lazy::Pending`] so an
    /// abandoned load is never mistaken for content.
    pub fn load(
        &mut self,
        id: ItemId,
        target: &str,
        adapter: &mut Adapter,
        cancel: &Cancel,
    ) -> Result<Outcome, String> {
        if self.node(id).is_none() {
            return Err(format!("item {} does not exist", id.get()));
        }
        if let Some(node) = self.node_mut(id) {
            node.lazy = Lazy::Loading;
        }
        let request = Request::new(Kind::Network, target);
        let outcome = adapter.perform(&request, cancel);
        let state = match &outcome {
            Outcome::Ok(_) => Lazy::Loaded,
            Outcome::Failed(reason) => Lazy::Failed(reason.clone()),
            Outcome::Unknown(reason) => Lazy::Failed(format!("unknown: {reason}")),
            Outcome::Cancelled => Lazy::Pending,
        };
        if let Some(node) = self.node_mut(id) {
            node.lazy = state;
        }
        Ok(outcome)
    }

    /// The depth of an item, refusing a structure that is not a tree.
    pub fn depth(&self, id: ItemId) -> Result<usize, String> {
        let mut depth = 0;
        let mut walk = self
            .node(id)
            .ok_or_else(|| format!("item {} does not exist", id.get()))?
            .parent;
        while let Some(current) = walk {
            depth += 1;
            walk = match self.node(current) {
                Some(parent) => {
                    if depth > self.nodes.len() {
                        return Err(format!("item {} is inside a cycle", id.get()));
                    }
                    parent.parent
                }
                None => return Err(format!("item {} names a missing parent", id.get())),
            };
        }
        Ok(depth)
    }

    /// Expands, collapses or selects an item, refusing an unknown identity.
    pub fn expand(&mut self, id: ItemId) -> Result<(), String> {
        self.mark_expanded(id, true)
    }

    /// Collapses an item.
    pub fn collapse(&mut self, id: ItemId) -> Result<(), String> {
        self.mark_expanded(id, false)
    }

    fn mark_expanded(&mut self, id: ItemId, expanded: bool) -> Result<(), String> {
        let node = self
            .node_mut(id)
            .ok_or_else(|| format!("item {} does not exist", id.get()))?;
        node.expanded = expanded;
        Ok(())
    }

    /// Selects an item, refusing an unknown identity.
    pub fn select(&mut self, id: ItemId) -> Result<(), String> {
        if self.node(id).is_none() {
            return Err(format!("item {} does not exist", id.get()));
        }
        self.selected = Some(id);
        Ok(())
    }

    /// Refuses duplicate identities and any cycle.
    pub fn validate(&self) -> Result<(), String> {
        let mut seen = BTreeSet::new();
        for node in &self.nodes {
            if !seen.insert(node.id) {
                return Err(format!("item id {} appears twice", node.id.get()));
            }
        }
        for node in &self.nodes {
            let mut walk = node.parent;
            let mut steps = 0;
            while let Some(current) = walk {
                if current == node.id {
                    return Err(format!("item {} is its own ancestor", node.id.get()));
                }
                steps += 1;
                if steps > self.nodes.len() {
                    return Err(format!("item {} is inside a cycle", node.id.get()));
                }
                match self.node(current) {
                    Some(parent) => walk = parent.parent,
                    None => {
                        return Err(format!(
                            "item {} names a missing parent {}",
                            node.id.get(),
                            current.get()
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    /// The tree as lines, indented by depth, showing a failed load where it happened.
    pub fn render(&self) -> Vec<String> {
        let mut lines = Vec::new();
        let roots: Vec<ItemId> = self.children(None).iter().map(|node| node.id).collect();
        for root in roots {
            self.render_node(root, 0, &mut lines);
        }
        if lines.is_empty() {
            lines.push("(no items)".to_owned());
        }
        lines
    }

    fn render_node(&self, id: ItemId, depth: usize, lines: &mut Vec<String>) {
        let Some(node) = self.node(id) else {
            return;
        };
        let mark = if node.expanded { "v" } else { ">" };
        let mut line = format!(
            "{}{} {} [{}]",
            "  ".repeat(depth),
            mark,
            node.label,
            node.icon.name()
        );
        if let Some(error) = node.lazy.error() {
            line.push_str(&format!(" — load failed: {error}"));
        }
        if self.selected == Some(id) {
            line.push_str(" *selected");
        }
        lines.push(line);
        if node.expanded {
            let children: Vec<ItemId> =
                self.children(Some(id)).iter().map(|node| node.id).collect();
            for child in children {
                self.render_node(child, depth + 1, lines);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{IconRole, ItemId, Lazy, Tree};
    use crate::app::data::adapters::{Adapter, Cancel, Outcome};

    fn item(value: u64) -> ItemId {
        ItemId::from_raw(value)
    }

    #[test]
    fn a_duplicate_id_is_refused() {
        let mut tree = Tree::new();
        tree.add(item(1), None, "root", IconRole::Container)
            .expect("a root");
        assert!(
            tree.add(item(1), None, "again", IconRole::Leaf)
                .unwrap_err()
                .contains("already exists")
        );
    }

    #[test]
    fn adding_an_item_with_no_parent_is_a_root() {
        let mut tree = Tree::new();
        tree.add(item(1), None, "root", IconRole::Container)
            .expect("a root");
        tree.add(item(2), Some(item(1)), "child", IconRole::Leaf)
            .expect("a child");
        assert_eq!(tree.children(None).len(), 1);
        assert_eq!(tree.children(Some(item(1))).len(), 1);
        assert_eq!(tree.depth(item(2)).expect("a depth"), 1);
    }

    #[test]
    fn a_reparent_that_would_make_a_cycle_is_refused() {
        let mut tree = Tree::new();
        tree.add(item(1), None, "root", IconRole::Container)
            .expect("a root");
        tree.add(item(2), Some(item(1)), "child", IconRole::Container)
            .expect("a child");
        tree.add(item(3), Some(item(2)), "grandchild", IconRole::Leaf)
            .expect("a grandchild");

        // Moving the root under its own grandchild would close a cycle.
        assert!(
            tree.reparent(item(1), Some(item(3)))
                .unwrap_err()
                .contains("cycle")
        );
        // Moving an item under itself is the same refusal.
        assert!(tree.reparent(item(2), Some(item(2))).is_err());
        assert!(tree.validate().is_ok());
    }

    #[test]
    fn a_load_failure_is_displayed_on_the_item() {
        let mut tree = Tree::new();
        tree.add(item(1), None, "orders", IconRole::Container)
            .expect("a root");
        tree.make_lazy(item(1)).expect("a lazy item");

        let mut adapter = Adapter::sample(&[Outcome::Failed("the service is down".to_owned())]);
        let outcome = tree
            .load(item(1), "orders", &mut adapter, &Cancel::new())
            .expect("an outcome");
        assert_eq!(outcome.name(), "failed");
        assert_eq!(
            tree.node(item(1)).and_then(|node| node.lazy.error()),
            Some("the service is down")
        );
        assert!(
            tree.render()
                .iter()
                .any(|line| line.contains("load failed: the service is down"))
        );
    }

    #[test]
    fn an_unknown_load_result_is_shown_as_a_failure() {
        let mut tree = Tree::new();
        tree.add(item(1), None, "orders", IconRole::Container)
            .expect("a root");
        let mut adapter = Adapter::sample(&[Outcome::Unknown("the write timed out".to_owned())]);
        tree.load(item(1), "orders", &mut adapter, &Cancel::new())
            .expect("an outcome");
        assert!(
            tree.node(item(1))
                .and_then(|node| node.lazy.error())
                .is_some_and(|reason| reason.contains("unknown"))
        );
    }

    #[test]
    fn a_cancelled_load_settles_as_pending_not_loaded() {
        let mut tree = Tree::new();
        tree.add(item(1), None, "orders", IconRole::Container)
            .expect("a root");
        let mut adapter = Adapter::sample(&[Outcome::Ok("content".to_owned())]);
        let outcome = tree
            .load(item(1), "orders", &mut adapter, &Cancel::set())
            .expect("an outcome");
        assert_eq!(outcome, Outcome::Cancelled);
        assert_eq!(
            tree.node(item(1)).map(|node| &node.lazy),
            Some(&Lazy::Pending)
        );
        assert!(
            tree.node(item(1))
                .and_then(|node| node.lazy.error())
                .is_none()
        );
    }

    #[test]
    fn a_successful_load_marks_the_item_loaded() {
        let mut tree = Tree::new();
        tree.add(item(1), None, "orders", IconRole::Container)
            .expect("a root");
        let mut adapter = Adapter::sample(&[Outcome::Ok("two rows".to_owned())]);
        tree.load(item(1), "orders", &mut adapter, &Cancel::new())
            .expect("an outcome");
        assert_eq!(
            tree.node(item(1)).map(|node| node.lazy.name()),
            Some("loaded")
        );
    }

    #[test]
    fn an_expanded_parent_shows_its_children_indented() {
        let mut tree = Tree::new();
        tree.add(item(1), None, "root", IconRole::Container)
            .expect("a root");
        tree.add(item(2), Some(item(1)), "child", IconRole::Leaf)
            .expect("a child");
        tree.expand(item(1)).expect("an expansion");
        let lines = tree.render();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("v root"));
        assert!(lines[1].starts_with("  > child"));
    }

    #[test]
    fn an_unknown_item_cannot_be_manipulated() {
        let mut tree = Tree::new();
        assert!(tree.expand(item(9)).is_err());
        assert!(tree.select(item(9)).is_err());
        assert!(tree.reparent(item(9), None).is_err());
        assert!(tree.depth(item(9)).is_err());
    }
}
