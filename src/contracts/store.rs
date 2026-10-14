// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The single writable model: contract **C-STORE**.
//!
//! T00.05's acceptance line is "no second writable model across views and services", and this
//! module is where that is either true or not. It is true by construction rather than by
//! convention:
//!
//! * The document — pages, nodes, properties, revision — lives in one place with private fields.
//! * The only method that changes a node is [`DocumentStore::apply`], and the only thing it
//!   accepts is a [`ChangeSet`] built by [`crate::contracts::change`]. There is no `set_property`
//!   on the store, no public field, and `Node`'s mutators are crate-internal.
//! * Everything else gets read access: [`DocumentView`] borrows the store, and
//!   [`DocumentSnapshot`] is a detached read-only copy that remembers the revision it was taken
//!   at. A service that computes something from a snapshot can therefore *say* it is outdated
//!   ([`DocumentSnapshot::staleness`]) instead of presenting yesterday's answer as today's.
//!
//! The one other `&mut self` method is [`DocumentStore::allocate_id`]. It moves the id sequence,
//! not the document, which is why it does not touch the revision: ids are identity, not content,
//! and nothing about the document changed. Keeping that distinction explicit is what lets the
//! claim above stay honest instead of quietly having an exception.
//!
//! **Atomicity, and what it costs.** `apply` validates and stages every operation in an overlay
//! that holds only the nodes the change set touches, then commits. A rejected change set leaves
//! the document exactly as it was — the revision does not move, no property changes, no node
//! appears. The cost is one clone per touched node, which keeps the edit path proportional to the
//! change and not to the document (whitepaper §17.9 item 6); the actual cost is a measurement
//! question that belongs to T00.07 and T03.01, not a claim made here.

use std::collections::{BTreeMap, BTreeSet, btree_map::Entry};

use super::change::{ApplyOutcome, ChangeOp, ChangeRejection, ChangeSet};
use super::identity::{IdAllocator, NodeId, PageId};
use super::model::{Node, Page, PropertyName, PropertyValue};
use super::permission::{Decision, PermissionGate};
use super::revision::{Revision, Staleness, VersionStamp};

/// The design document: the one place a node or a property can change.
#[derive(Debug)]
pub struct DocumentStore {
    revision: Revision,
    allocator: IdAllocator,
    gate: PermissionGate,
    pages: BTreeMap<PageId, Page>,
    nodes: BTreeMap<NodeId, Node>,
}

impl DocumentStore {
    /// Builds a document holding one page and its root node.
    ///
    /// Returns `None` when the root does not belong to the page, because a store that started out
    /// inconsistent would make every later invariant meaningless. Further pages are added by a page
    /// change set; that operation does not exist yet and belongs to T02.04 (multi-page entries).
    pub fn new(gate: PermissionGate, page: Page, root: Node) -> Option<Self> {
        if root.page() != page.id() || root.id() != page.root() {
            return None;
        }

        let mut allocator = IdAllocator::new();
        allocator.adopt(root.id());

        let page_id = page.id();
        Some(Self {
            revision: Revision::INITIAL,
            allocator,
            gate,
            pages: BTreeMap::from([(page_id, page)]),
            nodes: BTreeMap::from([(root.id(), root)]),
        })
    }

    /// The revision the document is at.
    pub fn revision(&self) -> Revision {
        self.revision
    }

    /// The permission state. The gate is part of the store because "who may change this" and "what
    /// the document is" are decided at the same place.
    pub fn gate(&self) -> &PermissionGate {
        &self.gate
    }

    /// Mutable access to the permission state, for trust prompts and approvals.
    pub fn gate_mut(&mut self) -> &mut PermissionGate {
        &mut self.gate
    }

    /// One page.
    pub fn page(&self, id: PageId) -> Option<&Page> {
        self.pages.get(&id)
    }

    /// Every page.
    pub fn pages(&self) -> impl Iterator<Item = &Page> {
        self.pages.values()
    }

    /// One node.
    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(&id)
    }

    /// Every node.
    pub fn nodes(&self) -> impl Iterator<Item = &Node> {
        self.nodes.values()
    }

    /// Issues an id for a node a change set is about to add.
    ///
    /// The id sequence is not part of the revision: issuing an id changes no node and no property,
    /// and a change set that is never applied leaves the sequence advanced but the document
    /// untouched. Ids are never reused, so an undone addition does not come back under the same
    /// identity ([`IdAllocator::retire`]).
    pub fn allocate_id(&mut self) -> NodeId {
        self.allocator.allocate()
    }

    /// Adopts an id that came from outside the store — a page document read from a file.
    ///
    /// The store refuses a change set that adds a node whose id it never issued
    /// (`ChangeRejection::UnknownNode`), which is what keeps one document from being written with
    /// another's identities. A file is the third place ids come from besides the allocator and the
    /// root the store is built around, so it has to be able to say "this one is mine now": adopting
    /// changes no node and no property and does not move the revision, exactly like
    /// [`Self::allocate_id`] — ids are identity, not content. Adopting an id the store already knows
    /// is a no-op, and adopting a retired one does not make it live again: a retired identity stays
    /// refused for the lifetime of the document.
    pub fn adopt_id(&mut self, id: NodeId) {
        self.allocator.adopt(id);
    }

    /// Read-only access to the document.
    pub fn view(&self) -> DocumentView<'_> {
        DocumentView { store: self }
    }

    /// Applies a change set, or refuses it without changing anything.
    pub fn apply(&mut self, set: ChangeSet) -> ApplyOutcome {
        if set.base() != self.revision {
            return self.reject(ChangeRejection::StaleBase {
                base: set.base(),
                current: self.revision,
            });
        }

        let principal = set.origin().principal();
        for capability in set.capabilities() {
            if let Decision::Denied { capability, reason } = self.gate.decide(principal, capability)
            {
                return self.reject(ChangeRejection::Denied { capability, reason });
            }
        }

        let mut working = WorkingSet::new();
        for op in set.ops() {
            if let Err(rejection) = working.apply_op(self, op) {
                return self.reject(rejection);
            }
        }

        let from = self.revision;
        for (id, node) in working.touched {
            self.nodes.insert(id, node);
        }
        for id in working.removed {
            self.nodes.remove(&id);
            self.allocator.retire(id);
        }
        self.revision = self.revision.next();

        ApplyOutcome::Applied {
            from,
            to: self.revision,
        }
    }

    fn reject(&self, rejection: ChangeRejection) -> ApplyOutcome {
        ApplyOutcome::Rejected {
            rejection,
            revision: self.revision,
        }
    }

    /// Whether this document ever issued the id. Used to refuse change sets that invent ids, and
    /// readable by a caller that is preparing a change set and would rather find out first.
    pub fn knows_id(&self, id: NodeId) -> bool {
        self.allocator.is_known(id)
    }

    /// Whether the id was issued and then removed. A retired identity is never live again, so a
    /// change set that adds it is refused; a caller preparing one can ask here.
    pub fn is_retired(&self, id: NodeId) -> bool {
        self.allocator.is_retired(id)
    }
}

/// A read-only view of the store.
///
/// It borrows the model and exposes no mutation, so a panel or a service holding one cannot edit
/// the document and cannot outlive the change it is reading. What a view deliberately does **not**
/// carry is a pinned revision: while it borrows the store it always sees the current state, and
/// claiming otherwise would be a promise the borrow checker does not need to keep. Anything that
/// has to remember *when* it read the document takes [`DocumentView::snapshot`] instead, which is
/// stamped and can be compared for staleness.
#[derive(Clone, Copy, Debug)]
pub struct DocumentView<'a> {
    store: &'a DocumentStore,
}

impl<'a> DocumentView<'a> {
    /// One node.
    pub fn node(&self, id: NodeId) -> Option<&'a Node> {
        self.store.node(id)
    }

    /// Every node.
    pub fn nodes(&self) -> impl Iterator<Item = &'a Node> {
        self.store.nodes()
    }

    /// One page.
    pub fn page(&self, id: PageId) -> Option<&'a Page> {
        self.store.page(id)
    }

    /// Every page.
    pub fn pages(&self) -> impl Iterator<Item = &'a Page> {
        self.store.pages()
    }

    /// A detached read-only copy, stamped with the revision it was taken at.
    pub fn snapshot(&self) -> DocumentSnapshot {
        DocumentSnapshot {
            stamp: VersionStamp::current(self.store.revision),
            pages: self.store.pages.clone(),
            nodes: self.store.nodes.clone(),
        }
    }
}

/// A detached read-only copy of the document, stamped with the revision it came from.
///
/// This is what a background task, a preview or a build gets: not the model, and not something it
/// could write back. Its fields are private and it has no `&mut self` method, so "the service kept
/// its own copy of the document and wrote to it" is not a mistake the type system allows — and
/// because the copy carries its revision, a result computed from it can be labelled stale instead
/// of being shown as current.
#[derive(Clone, Debug)]
pub struct DocumentSnapshot {
    stamp: VersionStamp,
    pages: BTreeMap<PageId, Page>,
    nodes: BTreeMap<NodeId, Node>,
}

impl DocumentSnapshot {
    /// The revision the document was at when this snapshot was taken.
    pub fn revision(&self) -> Revision {
        self.stamp.model
    }

    /// What this snapshot was produced from.
    pub fn stamp(&self) -> VersionStamp {
        self.stamp
    }

    /// Whether the document has moved on since this snapshot was taken.
    pub fn staleness(&self, current: Revision) -> Staleness {
        self.stamp.staleness(current)
    }

    /// One node.
    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(&id)
    }

    /// Every node.
    pub fn nodes(&self) -> impl Iterator<Item = &Node> {
        self.nodes.values()
    }

    /// One page.
    pub fn page(&self, id: PageId) -> Option<&Page> {
        self.pages.get(&id)
    }

    /// Every page.
    pub fn pages(&self) -> impl Iterator<Item = &Page> {
        self.pages.values()
    }
}

/// The staged state of one change set: only the nodes it touches are copied out of the store.
struct WorkingSet {
    touched: BTreeMap<NodeId, Node>,
    added: BTreeSet<NodeId>,
    removed: BTreeSet<NodeId>,
}

impl WorkingSet {
    fn new() -> Self {
        Self {
            touched: BTreeMap::new(),
            added: BTreeSet::new(),
            removed: BTreeSet::new(),
        }
    }

    fn get<'a>(&'a self, store: &'a DocumentStore, id: NodeId) -> Option<&'a Node> {
        if self.removed.contains(&id) {
            return None;
        }
        self.touched.get(&id).or_else(|| store.node(id))
    }

    fn get_mut(&mut self, store: &DocumentStore, id: NodeId) -> Option<&mut Node> {
        if self.removed.contains(&id) {
            return None;
        }
        if let Entry::Vacant(slot) = self.touched.entry(id) {
            let staged = store.node(id)?.clone();
            slot.insert(staged);
        }
        self.touched.get_mut(&id)
    }

    fn children_of(&self, store: &DocumentStore, id: NodeId) -> Option<Vec<NodeId>> {
        self.get(store, id).map(|node| node.children().to_vec())
    }

    /// Where `id` sits in `parent`'s child list, as the working set currently has it.
    fn child_index(&self, store: &DocumentStore, parent: NodeId, id: NodeId) -> Option<usize> {
        self.children_of(store, parent)?
            .iter()
            .position(|child| *child == id)
    }

    fn is_in_subtree(&self, store: &DocumentStore, root: NodeId, candidate: NodeId) -> bool {
        let mut cursor = Some(candidate);
        while let Some(id) = cursor {
            if id == root {
                return true;
            }
            cursor = self.get(store, id).and_then(Node::parent);
        }
        false
    }

    fn apply_op(&mut self, store: &DocumentStore, op: &ChangeOp) -> Result<(), ChangeRejection> {
        match op {
            ChangeOp::AddNode {
                parent,
                index,
                node,
            } => self.add_node(store, *parent, *index, node),
            ChangeOp::RemoveNode {
                node,
                parent,
                index,
            } => self.remove_node(store, node.id(), *parent, *index),
            ChangeOp::MoveNode {
                id,
                from_parent,
                from_index,
                to_parent,
                to_index,
            } => self.move_node(store, *id, *from_parent, *from_index, *to_parent, *to_index),
            ChangeOp::SetProperty {
                id,
                name,
                previous,
                value,
            } => self.set_property(store, *id, name, previous, value),
            ChangeOp::ClearProperty { id, name, removed } => {
                self.clear_property(store, *id, name, removed)
            }
        }
    }

    fn add_node(
        &mut self,
        store: &DocumentStore,
        parent: NodeId,
        index: usize,
        node: &Node,
    ) -> Result<(), ChangeRejection> {
        let id = node.id();
        let parent_page = self
            .get(store, parent)
            .ok_or(ChangeRejection::UnknownNode { id: parent })?
            .page();
        if node.page() != parent_page {
            return Err(ChangeRejection::CrossPagePlacement { node: id, parent });
        }
        if store.node(id).is_some() || self.added.contains(&id) {
            return Err(ChangeRejection::DuplicateNode { id });
        }
        if !store.knows_id(id) {
            return Err(ChangeRejection::UnknownNode { id });
        }
        if store.is_retired(id) {
            return Err(ChangeRejection::RetiredNode { id });
        }

        let children = self.children_of(store, parent).unwrap_or_default();
        if index > children.len() {
            return Err(ChangeRejection::PlacementOutOfRange {
                id,
                index,
                children: children.len(),
            });
        }

        let mut staged = node.clone();
        staged.set_parent(Some(parent));
        self.added.insert(id);
        self.touched.insert(id, staged);

        let mut list = children;
        list.insert(index, id);
        self.get_mut(store, parent)
            .expect("parent was found above")
            .set_children(list);
        Ok(())
    }

    fn remove_node(
        &mut self,
        store: &DocumentStore,
        id: NodeId,
        parent: NodeId,
        index: usize,
    ) -> Result<(), ChangeRejection> {
        let stored = self
            .get(store, id)
            .ok_or(ChangeRejection::UnknownNode { id })?;
        let page = stored.page();
        let stored_parent = stored
            .parent()
            .ok_or(ChangeRejection::PageRootChange { page })?;
        let children = stored.children().len();
        if stored_parent != parent {
            return Err(ChangeRejection::StaleStructure {
                id,
                expected: format!("under node {}", parent.get()),
                found: format!("under node {}", stored_parent.get()),
            });
        }
        let stored_index =
            self.child_index(store, parent, id)
                .ok_or(ChangeRejection::StaleStructure {
                    id,
                    expected: format!("at index {index}"),
                    found: "not among its parent's children".to_owned(),
                })?;
        if stored_index != index {
            return Err(ChangeRejection::StaleStructure {
                id,
                expected: format!("at index {index}"),
                found: format!("at index {stored_index}"),
            });
        }
        if children != 0 {
            return Err(ChangeRejection::ImplicitSubtreeRemoval { id, children });
        }

        let mut list = self.children_of(store, parent).expect("parent exists");
        list.remove(index);
        self.get_mut(store, parent)
            .expect("parent exists")
            .set_children(list);
        self.touched.remove(&id);
        self.added.remove(&id);
        self.removed.insert(id);
        Ok(())
    }

    fn move_node(
        &mut self,
        store: &DocumentStore,
        id: NodeId,
        from_parent: NodeId,
        from_index: usize,
        to_parent: NodeId,
        to_index: usize,
    ) -> Result<(), ChangeRejection> {
        let stored = self
            .get(store, id)
            .ok_or(ChangeRejection::UnknownNode { id })?;
        let page = stored.page();
        let stored_parent = stored
            .parent()
            .ok_or(ChangeRejection::PageRootChange { page })?;
        if stored_parent != from_parent {
            return Err(ChangeRejection::StaleStructure {
                id,
                expected: format!("under node {}", from_parent.get()),
                found: format!("under node {}", stored_parent.get()),
            });
        }
        let stored_index =
            self.child_index(store, from_parent, id)
                .ok_or(ChangeRejection::StaleStructure {
                    id,
                    expected: format!("at index {from_index}"),
                    found: "not among its parent's children".to_owned(),
                })?;
        if stored_index != from_index {
            return Err(ChangeRejection::StaleStructure {
                id,
                expected: format!("at index {from_index}"),
                found: format!("at index {stored_index}"),
            });
        }

        let target = self
            .get(store, to_parent)
            .ok_or(ChangeRejection::UnknownNode { id: to_parent })?;
        if target.page() != page {
            return Err(ChangeRejection::CrossPagePlacement {
                node: id,
                parent: to_parent,
            });
        }
        if self.is_in_subtree(store, id, to_parent) {
            return Err(ChangeRejection::CyclicMove {
                id,
                parent: to_parent,
            });
        }

        let mut destination = self.children_of(store, to_parent).expect("target exists");
        if to_parent == from_parent {
            destination.remove(from_index);
        }
        if to_index > destination.len() {
            return Err(ChangeRejection::PlacementOutOfRange {
                id,
                index: to_index,
                children: destination.len(),
            });
        }

        let mut origin = self.children_of(store, from_parent).expect("origin exists");
        origin.remove(stored_index);
        self.get_mut(store, from_parent)
            .expect("origin exists")
            .set_children(origin);

        destination.insert(to_index, id);
        self.get_mut(store, to_parent)
            .expect("target exists")
            .set_children(destination);
        self.get_mut(store, id)
            .expect("node exists")
            .set_parent(Some(to_parent));
        Ok(())
    }

    fn set_property(
        &mut self,
        store: &DocumentStore,
        id: NodeId,
        name: &PropertyName,
        previous: &Option<PropertyValue>,
        value: &PropertyValue,
    ) -> Result<(), ChangeRejection> {
        let found = self
            .get(store, id)
            .ok_or(ChangeRejection::UnknownNode { id })?
            .property(name)
            .cloned();
        if found != *previous {
            return Err(ChangeRejection::ConflictingValue {
                id,
                name: name.clone(),
                expected: previous.clone(),
                found,
            });
        }

        self.get_mut(store, id)
            .expect("node exists")
            .set_property(name.clone(), value.clone());
        Ok(())
    }

    fn clear_property(
        &mut self,
        store: &DocumentStore,
        id: NodeId,
        name: &PropertyName,
        removed: &PropertyValue,
    ) -> Result<(), ChangeRejection> {
        let found = self
            .get(store, id)
            .ok_or(ChangeRejection::UnknownNode { id })?
            .property(name)
            .cloned();
        if found.as_ref() != Some(removed) {
            return Err(ChangeRejection::ConflictingValue {
                id,
                name: name.clone(),
                expected: Some(removed.clone()),
                found,
            });
        }

        self.get_mut(store, id)
            .expect("node exists")
            .remove_property(name);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{DocumentSnapshot, DocumentStore};
    use crate::contracts::change::{ChangeOp, ChangeOrigin, ChangeRejection, ChangeSet};
    use crate::contracts::identity::{NodeId, PageId};
    use crate::contracts::model::{Node, NodeKind, Page, PropertyName, PropertyValue};
    use crate::contracts::permission::{Capability, DenialReason, PermissionGate};
    use crate::contracts::revision::{Revision, Staleness};

    fn node_kind() -> NodeKind {
        NodeKind::new("button").unwrap()
    }

    /// A store with one page and one root node, plus the ids the tests build change sets with.
    struct Fixture {
        store: DocumentStore,
        page: PageId,
        root: NodeId,
        text: PropertyName,
    }

    fn fixture() -> Fixture {
        let mut gate = PermissionGate::new();
        gate.trust_project();

        let page = PageId::from_raw(0);
        let root = NodeId::from_raw(0);
        let root_node = Node::new(page, root, NodeKind::new("window").unwrap());
        let page_value = Page::new(page, "main", root).unwrap();
        let store = DocumentStore::new(gate, page_value, root_node).unwrap();

        Fixture {
            store,
            page,
            root,
            text: PropertyName::new("text").unwrap(),
        }
    }

    /// Adds one child. Takes the store and the ids rather than the whole fixture so a test can
    /// still read `fixture.root` in the same expression.
    fn add_child(
        store: &mut DocumentStore,
        page: PageId,
        parent: NodeId,
        index: usize,
        origin: ChangeOrigin,
    ) -> NodeId {
        let id = store.allocate_id();
        let node = Node::new(page, id, node_kind());
        let set = ChangeSet::new(
            store.revision(),
            origin,
            "add a button",
            vec![ChangeOp::AddNode {
                parent,
                index,
                node: Box::new(node),
            }],
        )
        .unwrap();
        let outcome = store.apply(set);
        assert!(outcome.is_applied(), "{}", outcome);
        id
    }

    #[test]
    fn a_document_starts_at_the_initial_revision_with_its_root() {
        let fixture = fixture();

        assert_eq!(fixture.store.revision(), Revision::INITIAL);
        assert_eq!(fixture.store.page(fixture.page).unwrap().name(), "main");
        assert_eq!(
            fixture.store.node(fixture.root).unwrap().kind().as_str(),
            "window"
        );
        assert!(fixture.store.view().node(fixture.root).is_some());
    }

    #[test]
    fn a_store_refuses_a_root_node_that_is_not_its_page_root() {
        let page = PageId::from_raw(0);
        let page_value = Page::new(page, "main", NodeId::from_raw(0)).unwrap();
        let stranger = Node::new(page, NodeId::from_raw(1), node_kind());

        assert!(DocumentStore::new(PermissionGate::new(), page_value, stranger).is_none());
    }

    #[test]
    fn one_change_set_moves_the_revision_once_however_many_operations_it_has() {
        let mut fixture = fixture();
        let first = fixture.store.allocate_id();
        let second = fixture.store.allocate_id();
        let set = ChangeSet::new(
            fixture.store.revision(),
            ChangeOrigin::Human,
            "add two controls",
            vec![
                ChangeOp::AddNode {
                    parent: fixture.root,
                    index: 0,
                    node: Box::new(Node::new(fixture.page, first, node_kind())),
                },
                ChangeOp::AddNode {
                    parent: fixture.root,
                    index: 1,
                    node: Box::new(Node::new(fixture.page, second, node_kind())),
                },
                ChangeOp::SetProperty {
                    id: second,
                    name: fixture.text.clone(),
                    previous: None,
                    value: PropertyValue::Text("save".into()),
                },
            ],
        )
        .unwrap();

        let outcome = fixture.store.apply(set);

        assert_eq!(
            outcome,
            super::ApplyOutcome::Applied {
                from: Revision::INITIAL,
                to: Revision::from_raw(1)
            }
        );
        assert_eq!(fixture.store.revision(), Revision::from_raw(1));
        assert_eq!(
            fixture.store.node(fixture.root).unwrap().children(),
            [first, second]
        );
        assert_eq!(
            fixture.store.node(second).unwrap().property(&fixture.text),
            Some(&PropertyValue::Text("save".into())),
        );
    }

    #[test]
    fn a_change_set_against_an_older_revision_is_refused_and_changes_nothing() {
        let mut fixture = fixture();
        let child = add_child(
            &mut fixture.store,
            fixture.page,
            fixture.root,
            0,
            ChangeOrigin::Human,
        );
        let stale = ChangeSet::new(
            Revision::INITIAL,
            ChangeOrigin::Human,
            "set a label from an old revision",
            vec![ChangeOp::SetProperty {
                id: child,
                name: fixture.text.clone(),
                previous: None,
                value: PropertyValue::Text("late".into()),
            }],
        )
        .unwrap();

        let outcome = fixture.store.apply(stale);

        assert_eq!(
            outcome.rejection(),
            Some(&ChangeRejection::StaleBase {
                base: Revision::INITIAL,
                current: Revision::from_raw(1)
            }),
        );
        assert_eq!(fixture.store.revision(), Revision::from_raw(1));
        assert_eq!(
            fixture.store.node(child).unwrap().property(&fixture.text),
            None
        );
    }

    #[test]
    fn a_rejected_change_set_leaves_the_operations_before_it_unapplied() {
        let mut fixture = fixture();
        let child = add_child(
            &mut fixture.store,
            fixture.page,
            fixture.root,
            0,
            ChangeOrigin::Human,
        );
        let set = ChangeSet::new(
            fixture.store.revision(),
            ChangeOrigin::Human,
            "label the button and then edit a node that is gone",
            vec![
                ChangeOp::SetProperty {
                    id: child,
                    name: fixture.text.clone(),
                    previous: None,
                    value: PropertyValue::Text("undo me".into()),
                },
                ChangeOp::SetProperty {
                    id: NodeId::from_raw(999),
                    name: fixture.text.clone(),
                    previous: None,
                    value: PropertyValue::Text("nowhere".into()),
                },
            ],
        )
        .unwrap();

        let outcome = fixture.store.apply(set);

        assert_eq!(
            outcome.rejection(),
            Some(&ChangeRejection::UnknownNode {
                id: NodeId::from_raw(999)
            })
        );
        assert_eq!(fixture.store.revision(), Revision::from_raw(1));
        assert_eq!(
            fixture.store.node(child).unwrap().property(&fixture.text),
            None
        );
    }

    #[test]
    fn an_agent_may_relabel_a_control_but_not_add_one_until_a_human_approves() {
        let mut fixture = fixture();
        let child = add_child(
            &mut fixture.store,
            fixture.page,
            fixture.root,
            0,
            ChangeOrigin::Human,
        );

        let relabel = ChangeSet::new(
            fixture.store.revision(),
            ChangeOrigin::Agent,
            "make the button say Save",
            vec![ChangeOp::SetProperty {
                id: child,
                name: fixture.text.clone(),
                previous: None,
                value: PropertyValue::Text("Save".into()),
            }],
        )
        .unwrap();
        assert!(fixture.store.apply(relabel).is_applied());

        let id = fixture.store.allocate_id();
        let structural = ChangeSet::new(
            fixture.store.revision(),
            ChangeOrigin::Agent,
            "add a label under the window",
            vec![ChangeOp::AddNode {
                parent: fixture.root,
                index: 1,
                node: Box::new(Node::new(fixture.page, id, NodeKind::new("label").unwrap())),
            }],
        )
        .unwrap();

        let outcome = fixture.store.apply(structural.clone());

        assert_eq!(
            outcome.rejection(),
            Some(&ChangeRejection::Denied {
                capability: Capability::AddOrRemoveNode,
                reason: DenialReason::NeedsReview,
            }),
        );
        assert_eq!(
            fixture.store.node(fixture.root).unwrap().children(),
            [child]
        );

        fixture
            .store
            .gate_mut()
            .approve(Capability::AddOrRemoveNode);
        assert!(fixture.store.apply(structural).is_applied());
        assert_eq!(
            fixture.store.node(fixture.root).unwrap().children(),
            [child, id]
        );
    }

    #[test]
    fn an_agent_cannot_edit_a_property_by_inventing_its_own_previous_value() {
        let mut fixture = fixture();
        let child = add_child(
            &mut fixture.store,
            fixture.page,
            fixture.root,
            0,
            ChangeOrigin::Human,
        );
        let first = ChangeSet::new(
            fixture.store.revision(),
            ChangeOrigin::Human,
            "label it",
            vec![ChangeOp::SetProperty {
                id: child,
                name: fixture.text.clone(),
                previous: None,
                value: PropertyValue::Text("Save".into()),
            }],
        )
        .unwrap();
        assert!(fixture.store.apply(first).is_applied());

        let based_on_nothing = ChangeSet::new(
            fixture.store.revision(),
            ChangeOrigin::Agent,
            "change the label as if it were unset",
            vec![ChangeOp::SetProperty {
                id: child,
                name: fixture.text.clone(),
                previous: None,
                value: PropertyValue::Text("Discard".into()),
            }],
        )
        .unwrap();

        let outcome = fixture.store.apply(based_on_nothing);

        assert!(matches!(
            outcome.rejection(),
            Some(ChangeRejection::ConflictingValue { found: Some(_), .. }),
        ));
        assert_eq!(
            fixture.store.node(child).unwrap().property(&fixture.text),
            Some(&PropertyValue::Text("Save".into())),
        );
    }

    #[test]
    fn illegal_structures_are_refused_by_name() {
        let mut fixture = fixture();
        let child = add_child(
            &mut fixture.store,
            fixture.page,
            fixture.root,
            0,
            ChangeOrigin::Human,
        );
        let grandchild = add_child(
            &mut fixture.store,
            fixture.page,
            child,
            0,
            ChangeOrigin::Human,
        );

        let duplicated = fixture.store.allocate_id();
        let duplicate = ChangeSet::new(
            fixture.store.revision(),
            ChangeOrigin::Human,
            "add the same node twice",
            vec![
                ChangeOp::AddNode {
                    parent: fixture.root,
                    index: 1,
                    node: Box::new(Node::new(fixture.page, duplicated, node_kind())),
                },
                ChangeOp::AddNode {
                    parent: child,
                    index: 1,
                    node: Box::new(Node::new(fixture.page, duplicated, node_kind())),
                },
            ],
        )
        .unwrap();
        assert_eq!(
            fixture.store.apply(duplicate).rejection(),
            Some(&ChangeRejection::DuplicateNode { id: duplicated }),
        );

        let out_of_range = ChangeSet::new(
            fixture.store.revision(),
            ChangeOrigin::Human,
            "place a node past the end",
            vec![ChangeOp::AddNode {
                parent: fixture.root,
                index: 9,
                node: Box::new(Node::new(
                    fixture.page,
                    fixture.store.allocate_id(),
                    node_kind(),
                )),
            }],
        )
        .unwrap();
        assert!(matches!(
            fixture.store.apply(out_of_range).rejection(),
            Some(ChangeRejection::PlacementOutOfRange { index: 9, .. }),
        ));

        let subtree = ChangeSet::new(
            fixture.store.revision(),
            ChangeOrigin::Human,
            "remove a node that still has children",
            vec![ChangeOp::RemoveNode {
                node: Box::new(fixture.store.node(child).unwrap().clone()),
                parent: fixture.root,
                index: 0,
            }],
        )
        .unwrap();
        assert_eq!(
            fixture.store.apply(subtree).rejection(),
            Some(&ChangeRejection::ImplicitSubtreeRemoval {
                id: child,
                children: 1
            }),
        );

        let cycle = ChangeSet::new(
            fixture.store.revision(),
            ChangeOrigin::Human,
            "move the parent under its own child",
            vec![ChangeOp::MoveNode {
                id: child,
                from_parent: fixture.root,
                from_index: 0,
                to_parent: grandchild,
                to_index: 0,
            }],
        )
        .unwrap();
        assert_eq!(
            fixture.store.apply(cycle).rejection(),
            Some(&ChangeRejection::CyclicMove {
                id: child,
                parent: grandchild
            }),
        );

        let root_change = ChangeSet::new(
            fixture.store.revision(),
            ChangeOrigin::Human,
            "remove the page root",
            vec![ChangeOp::RemoveNode {
                node: Box::new(fixture.store.node(fixture.root).unwrap().clone()),
                parent: child,
                index: 0,
            }],
        )
        .unwrap();
        assert_eq!(
            fixture.store.apply(root_change).rejection(),
            Some(&ChangeRejection::PageRootChange { page: fixture.page }),
        );

        let stale_structure = ChangeSet::new(
            fixture.store.revision(),
            ChangeOrigin::Human,
            "remove a node from the position it used to hold",
            vec![ChangeOp::RemoveNode {
                node: Box::new(fixture.store.node(grandchild).unwrap().clone()),
                parent: child,
                index: 4,
            }],
        )
        .unwrap();
        assert!(matches!(
            fixture.store.apply(stale_structure).rejection(),
            Some(ChangeRejection::StaleStructure { found, .. }) if found == "at index 0",
        ));
    }

    #[test]
    fn a_removed_identity_is_never_reused() {
        let mut fixture = fixture();
        let child = add_child(
            &mut fixture.store,
            fixture.page,
            fixture.root,
            0,
            ChangeOrigin::Human,
        );
        let removal = ChangeSet::new(
            fixture.store.revision(),
            ChangeOrigin::Human,
            "remove the button",
            vec![ChangeOp::RemoveNode {
                node: Box::new(fixture.store.node(child).unwrap().clone()),
                parent: fixture.root,
                index: 0,
            }],
        )
        .unwrap();
        assert!(fixture.store.apply(removal).is_applied());
        assert_eq!(fixture.store.node(child), None);
        assert!(
            fixture
                .store
                .node(fixture.root)
                .unwrap()
                .children()
                .is_empty()
        );

        let resurrect = ChangeSet::new(
            fixture.store.revision(),
            ChangeOrigin::Human,
            "put a node back under the identity that was removed",
            vec![ChangeOp::AddNode {
                parent: fixture.root,
                index: 0,
                node: Box::new(Node::new(fixture.page, child, node_kind())),
            }],
        )
        .unwrap();

        assert_eq!(
            fixture.store.apply(resurrect).rejection(),
            Some(&ChangeRejection::RetiredNode { id: child }),
        );
    }

    #[test]
    fn a_node_added_by_an_invented_id_is_refused() {
        let mut fixture = fixture();
        let invented = NodeId::from_raw(4242);
        let set = ChangeSet::new(
            fixture.store.revision(),
            ChangeOrigin::Human,
            "add a node without asking the document for an id",
            vec![ChangeOp::AddNode {
                parent: fixture.root,
                index: 0,
                node: Box::new(Node::new(fixture.page, invented, node_kind())),
            }],
        )
        .unwrap();

        assert_eq!(
            fixture.store.apply(set).rejection(),
            Some(&ChangeRejection::UnknownNode { id: invented }),
        );
    }

    #[test]
    fn undo_restores_the_previous_state_including_positions() {
        let mut fixture = fixture();
        let first = add_child(
            &mut fixture.store,
            fixture.page,
            fixture.root,
            0,
            ChangeOrigin::Human,
        );
        let second = add_child(
            &mut fixture.store,
            fixture.page,
            fixture.root,
            1,
            ChangeOrigin::Human,
        );

        let edit = ChangeSet::new(
            fixture.store.revision(),
            ChangeOrigin::Human,
            "swap the two controls and label one",
            vec![
                ChangeOp::MoveNode {
                    id: first,
                    from_parent: fixture.root,
                    from_index: 0,
                    to_parent: fixture.root,
                    to_index: 1,
                },
                ChangeOp::SetProperty {
                    id: first,
                    name: fixture.text.clone(),
                    previous: None,
                    value: PropertyValue::Text("moved".into()),
                },
            ],
        )
        .unwrap();
        let applied = fixture.store.apply(edit.clone());
        assert!(applied.is_applied());
        assert_eq!(
            fixture.store.node(fixture.root).unwrap().children(),
            [second, first]
        );

        let undo = edit.inverse(applied.revision());
        let undone = fixture.store.apply(undo);

        assert!(undone.is_applied(), "{}", undone);
        assert_eq!(
            fixture.store.node(fixture.root).unwrap().children(),
            [first, second]
        );
        assert_eq!(
            fixture.store.node(first).unwrap().property(&fixture.text),
            None
        );
        assert_eq!(fixture.store.revision(), Revision::from_raw(4));
    }

    #[test]
    fn a_snapshot_is_a_read_only_copy_that_says_when_it_is_stale() {
        let mut fixture = fixture();
        let snapshot: DocumentSnapshot = fixture.store.view().snapshot();
        let child = add_child(
            &mut fixture.store,
            fixture.page,
            fixture.root,
            0,
            ChangeOrigin::Human,
        );

        assert_eq!(snapshot.revision(), Revision::INITIAL);
        assert_eq!(
            snapshot.node(child),
            None,
            "the snapshot cannot see a later change"
        );
        assert_eq!(
            snapshot.staleness(fixture.store.revision()),
            Staleness::Stale {
                computed_at: Revision::INITIAL,
                current: Revision::from_raw(1)
            },
        );
        assert_eq!(snapshot.stamp().model, Revision::INITIAL);
        assert!(
            fixture.store.view().node(child).is_some(),
            "a live view sees the change"
        );
    }

    #[test]
    fn issuing_an_id_moves_no_revision_because_it_changes_no_document() {
        let mut fixture = fixture();
        let before = fixture.store.revision();

        let id = fixture.store.allocate_id();

        assert_eq!(fixture.store.revision(), before);
        assert_eq!(fixture.store.node(id), None);
    }
}
