// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Stable identity: node and page ids that survive edits, contract **C-IDENTITY**.
//!
//! A visual designer rewrites the document constantly, so anything that points into the document
//! — a selection, a source-map entry, a diagnostic, an undo step — has to point at something that
//! does not move when the document does. That is what a stable id is for, and the designer's
//! mapping between the UI and the generated source depends on it.
//!
//! Two rules make the ids usable as identity rather than as indices:
//!
//! 1. **Allocation is monotonic.** An id is never handed out twice inside one document, so an old
//!    reference can never silently resolve to a different node.
//! 2. **Retirement is recorded.** When a node is removed its id becomes retired, and a later
//!    change set that still refers to it is refused as [`crate::contracts::change::ChangeRejection::RetiredNode`]
//!    instead of resurrecting the node under the same name.
//!
//! Ids are opaque on purpose: nothing in the designer may derive meaning, ordering or grouping from
//! the number. The public constructors exist because ids are persisted in the project file and
//! appear in source maps, both of which need a value-form round trip.

use std::collections::BTreeSet;

/// A node's stable identity inside one document.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct NodeId(u64);

impl NodeId {
    /// Rebuilds an id from the number it was written as.
    ///
    /// Used by persistence and by source maps; it does not allocate and does not check whether the
    /// number was ever handed out, so a document reader that accepts ids from disk must validate
    /// them against the document's allocator ([`IdAllocator::is_known`]).
    pub const fn from_raw(value: u64) -> Self {
        Self(value)
    }

    /// The number this id is written as.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// A page's stable identity inside one project.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct PageId(u64);

impl PageId {
    /// Rebuilds an id from the number it was written as.
    pub const fn from_raw(value: u64) -> Self {
        Self(value)
    }

    /// The number this id is written as.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Hands out ids and remembers which ones have been retired.
///
/// The allocator belongs to the document, not to a view or a service: one document, one sequence.
/// It deliberately holds no back-reference to the model, so it cannot be used to mutate anything.
#[derive(Debug, Default)]
pub struct IdAllocator {
    next: u64,
    issued: BTreeSet<u64>,
    retired: BTreeSet<u64>,
}

impl IdAllocator {
    /// A fresh allocator that has issued nothing.
    pub fn new() -> Self {
        Self::default()
    }

    /// The next unused id. Monotonic: a value that was issued once is never issued again.
    pub fn allocate(&mut self) -> NodeId {
        let value = self.next;
        self.next += 1;
        self.issued.insert(value);
        NodeId(value)
    }

    /// Marks an id that came from outside — a document read from disk, or a root node the store is
    /// constructed around — as issued, and moves the sequence past it.
    ///
    /// Adopting an already-issued id is a no-op, so it is safe to adopt ids in any order.
    pub fn adopt(&mut self, id: NodeId) {
        self.issued.insert(id.get());
        self.next = self.next.max(id.get() + 1);
    }

    /// Marks an id as no longer present in the document.
    ///
    /// Retiring is idempotent and never un-retires: the id stays retired for the lifetime of the
    /// document even if a node with the same kind is added later under a fresh id.
    pub fn retire(&mut self, id: NodeId) {
        self.retired.insert(id.get());
    }

    /// Whether this allocator ever issued the id.
    pub fn is_known(&self, id: NodeId) -> bool {
        self.issued.contains(&id.get())
    }

    /// Whether the id was issued and then retired.
    pub fn is_retired(&self, id: NodeId) -> bool {
        self.retired.contains(&id.get())
    }

    /// How many ids have been issued. Never decreases.
    pub fn issued(&self) -> usize {
        self.issued.len()
    }
}

#[cfg(test)]
mod tests {
    use super::{IdAllocator, NodeId, PageId};

    #[test]
    fn allocation_is_monotonic_and_never_repeats() {
        let mut allocator = IdAllocator::new();
        let first = allocator.allocate();
        let second = allocator.allocate();
        allocator.retire(first);
        let third = allocator.allocate();

        assert_ne!(first, second);
        assert_ne!(second, third);
        assert_ne!(first, third);
        assert!(first.get() < second.get() && second.get() < third.get());
        assert_eq!(allocator.issued(), 3);
    }

    #[test]
    fn a_retired_id_is_still_known_but_not_live() {
        let mut allocator = IdAllocator::new();
        let id = allocator.allocate();

        assert!(allocator.is_known(id));
        assert!(!allocator.is_retired(id));

        allocator.retire(id);
        allocator.retire(id);

        assert!(allocator.is_known(id));
        assert!(allocator.is_retired(id));
    }

    #[test]
    fn an_unknown_id_is_neither_known_nor_retired() {
        let allocator = IdAllocator::new();

        assert!(!allocator.is_known(NodeId::from_raw(7)));
        assert!(!allocator.is_retired(NodeId::from_raw(7)));
    }

    #[test]
    fn adopted_ids_are_known_and_never_handed_out_again() {
        let mut allocator = IdAllocator::new();
        let from_disk = NodeId::from_raw(12);

        allocator.adopt(from_disk);
        allocator.adopt(from_disk);

        assert!(allocator.is_known(from_disk));
        assert_eq!(allocator.issued(), 1);
        assert_eq!(allocator.allocate(), NodeId::from_raw(13));
    }

    #[test]
    fn ids_round_trip_through_their_written_form() {
        let mut allocator = IdAllocator::new();
        let id = allocator.allocate();

        assert_eq!(NodeId::from_raw(id.get()), id);
        assert_eq!(
            PageId::from_raw(PageId::from_raw(3).get()),
            PageId::from_raw(3)
        );
    }
}
