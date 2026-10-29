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
//! * **Key/path lookups are one derived snapshot.** [`SceneIndex`] mirrors the read-only contract of
//!   the library's `rust_widgets::view::TreeIndex` (`id_at`, `id_of_key`, `path_of`, `key_of`, `keys`,
//!   `duplicate_keys`, `generation`), but over host [`NodeId`]s. It is rebuilt lazily from the scene's
//!   instances and paint order after a generation bump; it is never written to independently, so it
//!   cannot become a second source of truth.
//!
//! Geometry here is a plain rectangle; a real backend's hit test is the library's, and this module is
//! what the object tree, the canvas and the checker share so they agree on which node a point means.
//!
//! # Why the library's `TreeIndex` is not embedded directly
//!
//! The pinned upstream revision exposes `TreeIndex` as a **read-only** snapshot: its `build`
//! constructor is `pub(crate)`, and the only public producer is `ViewEngine::tree_index`, which walks
//! a mounted declarative view tree and stores the engine's `ObjectId` (`u64`). The design canvas is not
//! mounted in a `ViewEngine`, and it owns durable `NodeId`s, not runtime `ObjectId`s — the library's
//! own contract says those two must not be conflated. There is therefore no public API by which this
//! module can hand a host `NodeId` tree to `TreeIndex`.
//!
//! [`SceneIndex`] is the documented adapter for that gap. It reproduces the published lookup rules
//! (sibling-scoped keys, duplicate keys answer `None` and are reported, paths are snapshot-addressed)
//! but substitutes `NodeId` for `ObjectId`. Replace it with the library type when upstream exposes a
//! public/generic constructor; until then this is a derived view of [`Scene`], not a second identity
//! store.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, OnceLock};

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

/// The node instances, their geometry, the paint order, and the derived key/path index.
///
/// The index is built on first lookup after a mutation and dropped on the next mutation, so a scene
/// that is edited many times and queried once pays one `O(n log n)` walk, not one per edit. That is
/// the same cost model as the library's `TreeIndex`, which is an on-demand snapshot rather than an
/// incrementally maintained map.
#[derive(Clone, Debug, Default)]
pub struct Scene {
    instances: BTreeMap<NodeId, Instance>,
    order: Vec<NodeId>,
    /// Identity generation: bumped whenever an instance, its hierarchy or the paint order changes.
    generation: u64,
    /// The lazily built key/path snapshot for [`Self::generation`].
    index: OnceLock<SceneIndex>,
}

impl PartialEq for Scene {
    fn eq(&self, other: &Self) -> bool {
        // The index is derived; comparing it would make equality depend on whether a lookup happened.
        self.instances == other.instances && self.order == other.order
    }
}

impl Eq for Scene {}

/// One node's live instance.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Instance {
    generation: u64,
    rect: Rect,
    parent: Option<NodeId>,
    hidden: bool,
    locked: bool,
    /// The sibling-scoped identity the host declared, if any.
    key: Option<String>,
}

/// A snapshot of the scene's key/path ↔ [`NodeId`] mapping.
///
/// This is the host-side adapter for `rust_widgets::view::TreeIndex` (see the module docs for the
/// exact upstream gap). Its rules are the library's:
///
/// * A **path** is the child-index sequence from a virtual root, so a top-level node is `[i]` and its
///   first child is `[i, 0]`. It is valid only for the generation it was taken from; an insertion or
///   removal moves the later paths.
/// * A **key** is scoped to its siblings. A key claimed twice under one parent maps to neither
///   claimant, and is named by [`Self::duplicate_keys`] instead of being resolved arbitrarily.
///
/// The snapshot is derived from [`Scene`]'s instances and paint order. It has no mutator and is only
/// replaced when the scene's generation changes.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct SceneIndex {
    generation: u64,
    /// The node at each path. The `Arc` is shared with `node_to_path`, mirroring the library's
    /// one-allocation-per-node snapshot (G-167).
    path_to_node: BTreeMap<Arc<[usize]>, NodeId>,
    /// The path of each node, in the same snapshot.
    node_to_path: BTreeMap<NodeId, Arc<[usize]>>,
    /// The unique keyed node under each parent.
    key_to_node: BTreeMap<(Option<NodeId>, String), NodeId>,
    /// The unique key of each node, so `key_of` is a lookup rather than a scan.
    node_to_key: BTreeMap<NodeId, String>,
    /// Keys claimed more than once under one parent, sorted.
    duplicated_keys: Vec<(Option<NodeId>, String)>,
}

impl SceneIndex {
    /// The scene generation this snapshot describes.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// How many nodes the snapshot holds.
    pub fn len(&self) -> usize {
        self.path_to_node.len()
    }

    /// Whether the snapshot holds no nodes.
    pub fn is_empty(&self) -> bool {
        self.path_to_node.is_empty()
    }

    /// The node at `path`, or `None` when the path addresses nothing in this snapshot.
    pub fn id_at(&self, path: &[usize]) -> Option<NodeId> {
        self.path_to_node.get(path).copied()
    }

    /// The node keyed `key` under `parent`, or `None`.
    ///
    /// `parent` is `None` for top-level nodes. A duplicated key answers `None`; ask
    /// [`Self::duplicate_keys`] to tell that condition from "no such key".
    pub fn id_of_key(&self, parent: Option<NodeId>, key: &str) -> Option<NodeId> {
        self.key_to_node.get(&(parent, key.to_owned())).copied()
    }

    /// Every key claimed more than once under one parent, sorted.
    pub fn duplicate_keys(&self) -> &[(Option<NodeId>, String)] {
        &self.duplicated_keys
    }

    /// The path of `node` in this snapshot, or `None` when the node is not part of it.
    pub fn path_of(&self, node: NodeId) -> Option<&[usize]> {
        self.node_to_path.get(&node).map(|path| path.as_ref())
    }

    /// The key of `node`, when it declared a unique one and is part of this snapshot.
    pub fn key_of(&self, node: NodeId) -> Option<&str> {
        self.node_to_key.get(&node).map(String::as_str)
    }

    /// Every uniquely keyed node as `(parent, key, node)`, sorted by `(parent, key)`.
    ///
    /// A duplicated key is absent: it maps to no single node.
    pub fn keys(&self) -> Vec<(Option<NodeId>, &str, NodeId)> {
        self.key_to_node
            .iter()
            .map(|((parent, key), node)| (*parent, key.as_str(), *node))
            .collect()
    }

    /// Walks the scene once, assigning paths and key scopes from the live instances.
    fn build(scene: &Scene) -> Self {
        // Group children in paint order once, so the walk does not filter the whole order per node.
        let mut children_of: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::new();
        let mut roots = Vec::new();
        for node in &scene.order {
            let parent = scene
                .instances
                .get(node)
                .and_then(|instance| instance.parent);
            match parent {
                Some(parent) if scene.instances.contains_key(&parent) => {
                    children_of.entry(parent).or_default().push(*node);
                }
                // A missing parent is treated as a virtual-root child: a removed ancestor must not
                // make a still-live descendant unaddressable.
                _ => roots.push(*node),
            }
        }

        let mut index = Self {
            generation: scene.generation,
            ..Self::default()
        };
        let mut visited = BTreeSet::new();
        let mut path = Vec::new();
        let mut next_root = 0usize;
        for root in roots {
            path.clear();
            path.push(next_root);
            next_root += 1;
            index.walk(root, &mut path, &mut visited, &children_of);
        }
        // A malformed parent cycle has no root; anchor each remaining component at its first node in
        // paint order so every live instance is still addressable and the walk always terminates.
        for node in &scene.order {
            if !visited.contains(node) {
                path.clear();
                path.push(next_root);
                next_root += 1;
                index.walk(*node, &mut path, &mut visited, &children_of);
            }
        }

        // The key map is derived last, from the same instances the path map describes. Building it
        // from `path_to_node` values (rather than a separate declaration) keeps the two in step.
        for node in index.path_to_node.values() {
            let Some(instance) = scene.instances.get(node) else {
                continue;
            };
            let Some(key) = &instance.key else {
                continue;
            };
            let claimed = (instance.parent, key.clone());
            if index.duplicated_keys.contains(&claimed) {
                continue;
            }
            if let Some(previous) = index.key_to_node.remove(&claimed) {
                // A second claimant makes the key identify neither, exactly as the library does.
                index.node_to_key.remove(&previous);
                index.duplicated_keys.push(claimed);
            } else {
                index.key_to_node.insert(claimed, *node);
                index.node_to_key.insert(*node, key.clone());
            }
        }
        index.duplicated_keys.sort();
        index
    }

    /// Depth-first walk that records one shared path allocation for both directions.
    fn walk(
        &mut self,
        node: NodeId,
        path: &mut Vec<usize>,
        visited: &mut BTreeSet<NodeId>,
        children_of: &BTreeMap<NodeId, Vec<NodeId>>,
    ) {
        if !visited.insert(node) {
            return;
        }
        let shared: Arc<[usize]> = Arc::from(path.as_slice());
        self.path_to_node.insert(Arc::clone(&shared), node);
        self.node_to_path.insert(node, shared);
        if let Some(children) = children_of.get(&node) {
            for (position, child) in children.iter().enumerate() {
                path.push(position);
                self.walk(*child, path, visited, children_of);
                path.pop();
            }
        }
    }
}

impl Scene {
    /// An empty scene.
    pub fn new() -> Self {
        Self::default()
    }

    /// The identity generation: it changes whenever an instance, its hierarchy or the paint order
    /// changes, and the key/path index is rebuilt when a lookup sees a newer value.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The key/path snapshot for the current generation.
    ///
    /// The first lookup after a mutation builds it; later lookups reuse the same snapshot. A path
    /// or key is valid for exactly the generation it was read at, which is why
    /// [`Self::generation`] is exposed alongside it.
    pub fn index(&self) -> &SceneIndex {
        self.index.get_or_init(|| SceneIndex::build(self))
    }

    /// Inserts a node's instance, leaving it on top.
    ///
    /// Any key already declared for the node is preserved; [`Self::insert_keyed`] declares or
    /// replaces one.
    pub fn insert(&mut self, node: NodeId, rect: Rect) -> Handle {
        self.insert_with_key(node, None, rect)
    }

    /// Inserts a node's instance with a sibling-scoped key, leaving it on top.
    pub fn insert_keyed(&mut self, node: NodeId, key: impl Into<String>, rect: Rect) -> Handle {
        self.insert_with_key(node, Some(key.into()), rect)
    }

    fn insert_with_key(&mut self, node: NodeId, key: Option<String>, rect: Rect) -> Handle {
        let existing = self.instances.get(&node);
        let next = existing.map_or(0, |instance| instance.generation + 1);
        let (parent, hidden, locked, existing_key) =
            existing.map_or((None, false, false, None), |instance| {
                (
                    instance.parent,
                    instance.hidden,
                    instance.locked,
                    instance.key.clone(),
                )
            });
        self.instances.insert(
            node,
            Instance {
                generation: next,
                rect,
                parent,
                hidden,
                locked,
                key: key.or(existing_key),
            },
        );
        self.order.retain(|existing| *existing != node);
        self.order.push(node);
        self.bump_generation();
        Handle {
            node,
            generation: next,
        }
    }

    /// Assigns or replaces a node's key, invalidating the key/path snapshot.
    pub fn set_key(&mut self, node: NodeId, key: impl Into<String>) -> bool {
        let key = key.into();
        let changed = match self.instances.get_mut(&node) {
            Some(instance) => {
                instance.key = Some(key);
                true
            }
            None => false,
        };
        if changed {
            self.bump_generation();
        }
        changed
    }

    /// Marks every derived lookup as belonging to a previous generation.
    fn bump_generation(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.index = OnceLock::new();
    }

    /// Sets a node's parent (T09.03's hierarchy), refusing a parent that is not live.
    pub fn set_parent(&mut self, node: NodeId, parent: Option<NodeId>) -> Result<(), String> {
        if let Some(parent) = parent
            && !self.instances.contains_key(&parent)
        {
            return Err(format!("parent node {} has no live instance", parent.get()));
        }
        let changed = match self.instances.get_mut(&node) {
            Some(instance) if instance.parent != parent => {
                instance.parent = parent;
                true
            }
            Some(_) => false,
            None => return Err(format!("node {} has no live instance", node.get())),
        };
        if changed {
            self.bump_generation();
        }
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
        if self.instances.remove(&node).is_none() {
            return false;
        }
        self.order.retain(|existing| *existing != node);
        self.bump_generation();
        true
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
        if self.order.last() == Some(&node) {
            return true;
        }
        self.order.retain(|existing| *existing != node);
        self.order.push(node);
        self.bump_generation();
        true
    }

    /// The paint order, bottom first.
    pub fn order(&self) -> &[NodeId] {
        &self.order
    }

    /// The `NodeId` at a path in the current key/path snapshot.
    pub fn id_at(&self, path: &[usize]) -> Option<NodeId> {
        self.index().id_at(path)
    }

    /// The node keyed `key` under `parent` in the current snapshot.
    pub fn id_of_key(&self, parent: Option<NodeId>, key: &str) -> Option<NodeId> {
        self.index().id_of_key(parent, key)
    }

    /// The path of a node in the current snapshot.
    pub fn path_of(&self, node: NodeId) -> Option<&[usize]> {
        self.index().path_of(node)
    }

    /// The key of a node in the current snapshot.
    pub fn key_of(&self, node: NodeId) -> Option<&str> {
        self.index().key_of(node)
    }

    /// Every key claimed more than once under one parent in the current snapshot.
    pub fn duplicate_keys(&self) -> &[(Option<NodeId>, String)] {
        self.index().duplicate_keys()
    }

    /// Every uniquely keyed node in the current snapshot, sorted by `(parent, key)`.
    pub fn keys(&self) -> Vec<(Option<NodeId>, &str, NodeId)> {
        self.index().keys()
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

    #[test]
    fn a_key_and_a_path_resolve_to_the_same_node() {
        let mut scene = Scene::new();
        scene.insert_keyed(node(1), "window", Rect::new(0, 0, 100, 100));
        scene.insert_keyed(node(2), "panel", Rect::new(10, 10, 50, 50));
        scene
            .set_parent(node(2), Some(node(1)))
            .expect("a live parent");
        scene.insert_keyed(node(3), "button", Rect::new(20, 20, 10, 10));
        scene
            .set_parent(node(3), Some(node(2)))
            .expect("a live parent");

        assert_eq!(scene.id_at(&[0]), Some(node(1)));
        assert_eq!(scene.id_at(&[0, 0]), Some(node(2)));
        assert_eq!(scene.id_at(&[0, 0, 0]), Some(node(3)));
        assert_eq!(scene.path_of(node(3)), Some(&[0, 0, 0][..]));
        assert_eq!(scene.id_of_key(None, "window"), Some(node(1)));
        assert_eq!(scene.id_of_key(Some(node(1)), "panel"), Some(node(2)));
        assert_eq!(scene.id_of_key(Some(node(2)), "button"), Some(node(3)));
        assert_eq!(scene.key_of(node(3)), Some("button"));
        // A key is sibling-scoped, not global.
        assert_eq!(scene.id_of_key(Some(node(1)), "button"), None);
        assert_eq!(
            scene.keys(),
            vec![
                (None, "window", node(1)),
                (Some(node(1)), "panel", node(2)),
                (Some(node(2)), "button", node(3)),
            ]
        );
    }

    #[test]
    fn a_duplicated_sibling_key_maps_to_neither_claimant_and_is_reported() {
        let mut scene = Scene::new();
        scene.insert_keyed(node(1), "button", Rect::new(0, 0, 10, 10));
        scene.insert_keyed(node(2), "button", Rect::new(10, 0, 10, 10));

        assert_eq!(scene.id_of_key(None, "button"), None);
        assert_eq!(scene.duplicate_keys(), &[(None, "button".to_owned())]);
        assert!(scene.keys().is_empty());
        // A duplicate key hides neither node: both are still addressable by path.
        assert_eq!(scene.id_at(&[0]), Some(node(1)));
        assert_eq!(scene.id_at(&[1]), Some(node(2)));

        // The same key under different parents is not a duplicate.
        let mut scoped = Scene::new();
        scoped.insert_keyed(node(1), "first", Rect::new(0, 0, 10, 10));
        scoped.insert_keyed(node(2), "second", Rect::new(0, 10, 10, 10));
        scoped.insert_keyed(node(3), "title", Rect::new(1, 1, 1, 1));
        scoped.insert_keyed(node(4), "title", Rect::new(2, 2, 1, 1));
        scoped.set_parent(node(3), Some(node(1))).expect("parent");
        scoped.set_parent(node(4), Some(node(2))).expect("parent");
        assert_eq!(scoped.id_of_key(Some(node(1)), "title"), Some(node(3)));
        assert_eq!(scoped.id_of_key(Some(node(2)), "title"), Some(node(4)));
        assert!(scoped.duplicate_keys().is_empty());
    }

    #[test]
    fn a_rebuild_or_delete_invalidates_the_previous_snapshot() {
        let mut scene = Scene::new();
        scene.insert_keyed(node(1), "panel", Rect::new(0, 0, 100, 100));
        let first = scene.insert_keyed(node(2), "button", Rect::new(10, 10, 20, 20));
        scene
            .set_parent(node(2), Some(node(1)))
            .expect("a live parent");
        let path = scene.path_of(node(2)).expect("indexed").to_vec();
        let generation = scene.generation();
        assert_eq!(scene.index().generation(), generation);

        // A rebuild is a new instance generation, so the old handle is refused.
        let rebuilt = scene
            .rebuild(node(2), Rect::new(20, 20, 20, 20))
            .expect("rebuilt");
        assert_eq!(
            scene.resolve(first),
            Err(Stale::Rebuilt {
                node: node(2),
                handle: first.generation,
                current: rebuilt.generation,
            })
        );
        assert!(scene.generation() > generation);
        assert_eq!(scene.index().generation(), scene.generation());
        // The node still exists, so its path and key still address it.
        assert_eq!(scene.id_at(&path), Some(node(2)));
        assert_eq!(scene.id_of_key(Some(node(1)), "button"), Some(node(2)));

        // A delete drops the node from the snapshot; the old path no longer names it.
        let after_rebuild = scene.generation();
        assert!(scene.remove(node(2)));
        assert!(scene.generation() > after_rebuild);
        assert_eq!(scene.id_at(&path), None);
        assert_eq!(scene.path_of(node(2)), None);
        assert_eq!(scene.key_of(node(2)), None);
        assert_eq!(scene.id_of_key(Some(node(1)), "button"), None);
    }

    #[test]
    fn raising_a_node_moves_its_path_with_the_paint_order() {
        let mut scene = Scene::new();
        scene.insert_keyed(node(1), "first", Rect::new(0, 0, 10, 10));
        scene.insert_keyed(node(2), "second", Rect::new(5, 5, 10, 10));
        assert_eq!(scene.id_at(&[0]), Some(node(1)));
        assert_eq!(scene.id_at(&[1]), Some(node(2)));

        assert!(scene.raise(node(1)));
        assert_eq!(scene.id_at(&[0]), Some(node(2)));
        assert_eq!(scene.id_at(&[1]), Some(node(1)));
        // Hit testing still reads the paint order, so it names the same node the index does.
        let top = scene.hit(7, 7).expect("a node");
        assert_eq!(top, node(1));
        assert_eq!(scene.id_at(scene.path_of(top).expect("indexed")), Some(top));
    }
}
