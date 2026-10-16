// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The design scene: a node's render instance, hit testing and selection mapping (task T09.02).
//!
//! The acceptance line is *"after a delete or a rebuild, nothing refers to the old instance"*. A
//! handle that outlives the instance it names is exactly the bug the line describes, so the module
//! makes a handle prove itself:
//!
//! * **A handle carries the generation it was made for.** [`Handle`] is a [`NodeId`] and a generation;
//!   [`Scene::resolve`] answers a [`Stale`] refusal for a handle whose node is gone (`Removed`) or was
//!   rebuilt since (`Rebuilt`), so a stale handle cannot read the new instance by accident.
//! * **A rebuild is a new generation, not an in-place edit.** [`Scene::rebuild`] bumps the node's
//!   generation, which is what makes the previous handles stale.
//! * **Hit testing is the paint order.** [`Scene::hit`] answers the topmost node whose rectangle
//!   contains the point, and [`Scene::raise`] changes that order, so what a click selects is what a
//!   person sees on top.
//! * **Selection is a set of node ids.** [`Scene::resolve_selection`] maps a selection to the handles
//!   that are currently valid, dropping (and reporting) the ones that are not, so a selection survives
//!   a rebuild honestly rather than pointing at an instance that no longer exists.
//!
//! Geometry here is a plain rectangle; a real backend's hit test is the library's, and this module is
//! what the object tree, the canvas and the checker share so they agree on which node a point means.

#![allow(dead_code)]

use std::collections::BTreeMap;

use crate::contracts::identity::NodeId;

/// A rectangle in the design space.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rect {
    /// Left.
    pub x: i32,
    /// Top.
    pub y: i32,
    /// Width.
    pub width: u32,
    /// Height.
    pub height: u32,
}

impl Rect {
    /// A rectangle.
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Whether a point is inside (the right and bottom edges are outside, half-open).
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x
            && x < self.x + self.width as i32
            && y >= self.y
            && y < self.y + self.height as i32
    }

    /// Whether this rectangle and another overlap at all.
    pub fn intersects(&self, other: Rect) -> bool {
        self.x < other.x + other.width as i32
            && other.x < self.x + self.width as i32
            && self.y < other.y + other.height as i32
            && other.y < self.y + self.height as i32
    }

    /// The rectangle in one line.
    pub fn render(&self) -> String {
        format!("{},{} {}x{}", self.x, self.y, self.width, self.height)
    }
}

/// A handle to a node's render instance, valid only for the generation it was made in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Handle {
    /// The node.
    pub node: NodeId,
    /// The generation of the instance.
    pub generation: u64,
}

impl Handle {
    /// The handle in one line.
    pub fn render(&self) -> String {
        format!("node {} gen {}", self.node.get(), self.generation)
    }
}

/// Why a handle no longer refers to a live instance.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stale {
    /// The node's instance was removed.
    Removed {
        /// The node.
        node: NodeId,
    },
    /// The node's instance was rebuilt since the handle was made.
    Rebuilt {
        /// The node.
        node: NodeId,
        /// The generation the handle names.
        handle: u64,
        /// The generation the instance is now at.
        current: u64,
    },
}

impl Stale {
    /// The refusal in one sentence.
    pub fn message(&self) -> String {
        match self {
            Self::Removed { node } => format!("node {} has no live instance", node.get()),
            Self::Rebuilt {
                node,
                handle,
                current,
            } => format!(
                "node {} was rebuilt: handle generation {handle}, instance generation {current}",
                node.get()
            ),
        }
    }
}

/// The node instances, their geometry, and the paint order.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Scene {
    instances: BTreeMap<NodeId, Instance>,
    order: Vec<NodeId>,
}

/// One node's live instance.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Instance {
    generation: u64,
    rect: Rect,
    parent: Option<NodeId>,
    hidden: bool,
    locked: bool,
}

impl Scene {
    /// An empty scene.
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts a node's instance, leaving it on top.
    pub fn insert(&mut self, node: NodeId, rect: Rect) -> Handle {
        let next = self
            .instances
            .get(&node)
            .map_or(0, |instance| instance.generation + 1);
        let (parent, hidden, locked) = self
            .instances
            .get(&node)
            .map_or((None, false, false), |instance| {
                (instance.parent, instance.hidden, instance.locked)
            });
        self.instances.insert(
            node,
            Instance {
                generation: next,
                rect,
                parent,
                hidden,
                locked,
            },
        );
        self.order.retain(|existing| *existing != node);
        self.order.push(node);
        Handle {
            node,
            generation: next,
        }
    }

    /// Sets a node's parent (T09.03's hierarchy), refusing a parent that is not live.
    pub fn set_parent(&mut self, node: NodeId, parent: Option<NodeId>) -> Result<(), String> {
        if let Some(parent) = parent
            && !self.instances.contains_key(&parent)
        {
            return Err(format!("parent node {} has no live instance", parent.get()));
        }
        let Some(instance) = self.instances.get_mut(&node) else {
            return Err(format!("node {} has no live instance", node.get()));
        };
        instance.parent = parent;
        Ok(())
    }

    /// A node's parent.
    pub fn parent(&self, node: NodeId) -> Option<NodeId> {
        self.instances
            .get(&node)
            .and_then(|instance| instance.parent)
    }

    /// A node's children, in paint order.
    pub fn children(&self, node: NodeId) -> Vec<NodeId> {
        self.order
            .iter()
            .filter(|candidate| self.parent(**candidate) == Some(node))
            .copied()
            .collect()
    }

    /// Hides or shows a node. Hiding is **not** locking: a hidden node is not drawn, a locked one is.
    pub fn set_hidden(&mut self, node: NodeId, hidden: bool) -> bool {
        match self.instances.get_mut(&node) {
            Some(instance) => {
                instance.hidden = hidden;
                true
            }
            None => false,
        }
    }

    /// Locks or unlocks a node. Locking is **not** hiding: a locked node is drawn but not selectable.
    pub fn set_locked(&mut self, node: NodeId, locked: bool) -> bool {
        match self.instances.get_mut(&node) {
            Some(instance) => {
                instance.locked = locked;
                true
            }
            None => false,
        }
    }

    /// Whether a node is hidden.
    pub fn is_hidden(&self, node: NodeId) -> bool {
        self.instances
            .get(&node)
            .is_some_and(|instance| instance.hidden)
    }

    /// Whether a node is locked.
    pub fn is_locked(&self, node: NodeId) -> bool {
        self.instances
            .get(&node)
            .is_some_and(|instance| instance.locked)
    }

    /// Whether a node and every ancestor of it are visible (so it is drawn).
    pub fn is_visible(&self, node: NodeId) -> bool {
        let mut current = Some(node);
        while let Some(id) = current {
            match self.instances.get(&id) {
                Some(instance) if instance.hidden => return false,
                Some(instance) => current = instance.parent,
                None => return false,
            }
        }
        true
    }

    /// Whether a node may be selected: live, visible, and not locked.
    pub fn is_selectable(&self, node: NodeId) -> bool {
        self.instances.contains_key(&node) && self.is_visible(node) && !self.is_locked(node)
    }

    /// Rebuilds a node's instance, so every earlier handle is stale.
    pub fn rebuild(&mut self, node: NodeId, rect: Rect) -> Option<Handle> {
        if !self.instances.contains_key(&node) {
            return None;
        }
        Some(self.insert(node, rect))
    }

    /// Removes a node's instance, so every handle to it is stale.
    pub fn remove(&mut self, node: NodeId) -> bool {
        self.order.retain(|existing| *existing != node);
        self.instances.remove(&node).is_some()
    }

    /// Whether a node has a live instance.
    pub fn contains(&self, node: NodeId) -> bool {
        self.instances.contains_key(&node)
    }

    /// How many live instances there are.
    pub fn len(&self) -> usize {
        self.instances.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.instances.is_empty()
    }

    /// The current handle for a node.
    pub fn instance(&self, node: NodeId) -> Option<Handle> {
        self.instances.get(&node).map(|instance| Handle {
            node,
            generation: instance.generation,
        })
    }

    /// Reads a handle's rectangle, or says why it is stale.
    pub fn resolve(&self, handle: Handle) -> Result<Rect, Stale> {
        match self.instances.get(&handle.node) {
            None => Err(Stale::Removed { node: handle.node }),
            Some(instance) if instance.generation != handle.generation => Err(Stale::Rebuilt {
                node: handle.node,
                handle: handle.generation,
                current: instance.generation,
            }),
            Some(instance) => Ok(instance.rect),
        }
    }

    /// The topmost **visible, selectable** node whose rectangle contains a point.
    ///
    /// Hidden and locked nodes are skipped: a hidden node is not drawn, and a locked one is drawn but
    /// cannot be picked (the distinction T09.03's acceptance is about).
    pub fn hit(&self, x: i32, y: i32) -> Option<NodeId> {
        self.order
            .iter()
            .rev()
            .find(|node| {
                self.is_selectable(**node)
                    && self
                        .instances
                        .get(node)
                        .is_some_and(|instance| instance.rect.contains(x, y))
            })
            .copied()
    }

    /// Every selectable node whose rectangle intersects a marquee.
    pub fn within(&self, rect: Rect) -> Vec<NodeId> {
        self.order
            .iter()
            .filter(|node| {
                self.is_selectable(**node)
                    && self
                        .instances
                        .get(node)
                        .is_some_and(|instance| instance.rect.intersects(rect))
            })
            .copied()
            .collect()
    }

    /// Brings a node to the front of the paint order.
    pub fn raise(&mut self, node: NodeId) -> bool {
        if !self.instances.contains_key(&node) {
            return false;
        }
        self.order.retain(|existing| *existing != node);
        self.order.push(node);
        true
    }

    /// The paint order, bottom first.
    pub fn order(&self) -> &[NodeId] {
        &self.order
    }

    /// Maps a selection to the handles that are currently valid.
    ///
    /// A node whose instance is gone is reported in the second list rather than silently dropped.
    pub fn resolve_selection(&self, selection: &[NodeId]) -> (Vec<Handle>, Vec<NodeId>) {
        let mut handles = Vec::new();
        let mut missing = Vec::new();
        for node in selection {
            match self.instance(*node) {
                Some(handle) => handles.push(handle),
                None => missing.push(*node),
            }
        }
        (handles, missing)
    }

    /// The scene in one report.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!("scene: {} instance(s)", self.instances.len())];
        for node in &self.order {
            if let Some(instance) = self.instances.get(node) {
                let flags = match (instance.hidden, instance.locked) {
                    (true, true) => " hidden locked",
                    (true, false) => " hidden",
                    (false, true) => " locked",
                    (false, false) => "",
                };
                let parent = match instance.parent {
                    Some(parent) => format!(" parent {}", parent.get()),
                    None => String::new(),
                };
                lines.push(format!(
                    "  node {} gen {} {}{flags}{parent}",
                    node.get(),
                    instance.generation,
                    instance.rect.render()
                ));
            }
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{Handle, Rect, Scene, Stale};
    use crate::contracts::identity::NodeId;

    fn node(value: u64) -> NodeId {
        NodeId::from_raw(value)
    }

    #[test]
    fn a_rebuilt_instance_stales_every_earlier_handle() {
        let mut scene = Scene::new();
        let first = scene.insert(node(1), Rect::new(0, 0, 10, 10));
        assert_eq!(first.generation, 0);
        let second = scene
            .rebuild(node(1), Rect::new(5, 5, 10, 10))
            .expect("rebuilt");
        assert_eq!(second.generation, 1);
        // The old handle is refused, not read as the new instance.
        assert_eq!(
            scene.resolve(first),
            Err(Stale::Rebuilt {
                node: node(1),
                handle: 0,
                current: 1
            })
        );
        assert_eq!(scene.resolve(second), Ok(Rect::new(5, 5, 10, 10)));
    }

    #[test]
    fn a_removed_instance_stales_every_handle() {
        let mut scene = Scene::new();
        let handle = scene.insert(node(2), Rect::new(0, 0, 4, 4));
        assert!(scene.remove(node(2)));
        assert_eq!(scene.resolve(handle), Err(Stale::Removed { node: node(2) }));
        assert!(!scene.contains(node(2)));
        assert!(!scene.remove(node(2)));
    }

    #[test]
    fn hit_testing_is_the_paint_order() {
        let mut scene = Scene::new();
        scene.insert(node(1), Rect::new(0, 0, 100, 100));
        scene.insert(node(2), Rect::new(10, 10, 20, 20));
        // node 2 is on top and contains the point.
        assert_eq!(scene.hit(15, 15), Some(node(2)));
        // A point only node 1 covers returns node 1.
        assert_eq!(scene.hit(50, 50), Some(node(1)));
        // Raising node 1 changes the answer.
        assert!(scene.raise(node(1)));
        assert_eq!(scene.hit(15, 15), Some(node(1)));
        // A point nothing covers returns nothing.
        assert_eq!(scene.hit(500, 500), None);
    }

    #[test]
    fn a_selection_maps_to_valid_handles_and_reports_the_missing() {
        let mut scene = Scene::new();
        scene.insert(node(1), Rect::new(0, 0, 10, 10));
        scene.insert(node(2), Rect::new(0, 0, 10, 10));
        let (handles, missing) = scene.resolve_selection(&[node(1), node(2), node(3)]);
        assert_eq!(handles.len(), 2);
        assert_eq!(missing, vec![node(3)]);
        // After a rebuild, the fresh handle is the one a selection resolves to.
        scene.rebuild(node(1), Rect::new(1, 1, 2, 2));
        let (handles, _) = scene.resolve_selection(&[node(1)]);
        assert_eq!(handles, vec![scene.instance(node(1)).expect("live")]);
        assert_eq!(handles[0].generation, 1);
    }

    #[test]
    fn a_rectangle_is_half_open() {
        let rect = Rect::new(0, 0, 10, 10);
        assert!(rect.contains(0, 0));
        assert!(rect.contains(9, 9));
        assert!(!rect.contains(10, 10));
        assert!(!rect.contains(-1, 0));
    }

    #[test]
    fn a_handle_reads_its_own_generation() {
        let handle = Handle {
            node: node(7),
            generation: 3,
        };
        assert!(handle.render().contains("gen 3"));
    }
}
