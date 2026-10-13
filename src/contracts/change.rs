// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Change sets: contract **C-CHANGE**.
//!
//! Whitepaper §12.3 puts the change-set IR at the centre of the architecture: a document *state*
//! has to be diffed to be reviewed, undone or regenerated, while a document *change* already is
//! the diff. So the only way to modify the model is to hand the store a [`ChangeSet`], and this
//! module fixes what one may contain.
//!
//! What the shape guarantees:
//!
//! * **One unit of undo.** A change set is applied as a whole: either every operation lands and the
//!   revision moves once, or none lands and the revision does not move at all.
//! * **Self-inverting.** Every operation carries the state it replaces — the old value, the old
//!   position, the removed record — so [`ChangeSet::inverse`] is derived rather than stored. A
//!   stored inverse would be a second description of the same change, and the two would drift.
//! * **Self-checking.** Because an operation carries what it expects to find, the store can refuse
//!   it when that expectation no longer holds, instead of applying it to the wrong state. A
//!   proposal made against r7 and applied to r9 fails on the revision, not silently on a node.
//! * **Auditable.** A change set names who proposed it ([`ChangeOrigin`]) and what it is for
//!   (`intent`), which is what a review prompt, a log entry or an agent proposal has to show.
//!
//! Rejections are values, not panics and not silent no-ops: [`ChangeRejection`] names the
//! expectation that failed, so a caller can explain the failure instead of reporting an empty
//! success.

use std::fmt;

use super::identity::{NodeId, PageId};
use super::model::{Node, PropertyName, PropertyValue};
use super::permission::{Capability, DenialReason, Principal};
use super::revision::Revision;

/// Who proposed a change set. The store checks the proposal against the permission gate using this.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum ChangeOrigin {
    /// A person acting through the UI.
    Human,
    /// An agent's proposal, which reaches the model only through this path.
    Agent,
}

impl ChangeOrigin {
    /// The principal this origin acts as.
    pub const fn principal(self) -> Principal {
        match self {
            Self::Human => Principal::Human,
            Self::Agent => Principal::Agent,
        }
    }

    /// The machine name used in reports and tests.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Agent => "agent",
        }
    }
}

/// One operation on the document.
///
/// Positions are explicit (`index`, `from_index`, `to_index`) rather than implied: an operation
/// that just said "append" would not be invertible without asking the model what happened, and an
/// undo step that has to ask the model is not a description of the change.
#[derive(Clone, PartialEq, Debug)]
pub enum ChangeOp {
    /// Inserts a node under a parent at `index`. `index` must be at most the parent's current
    /// child count, so "append" is written as the child count at the moment the set was prepared.
    AddNode {
        parent: NodeId,
        index: usize,
        node: Box<Node>,
    },
    /// Removes a childless node, carrying the record and the position needed to put it back.
    RemoveNode {
        node: Box<Node>,
        parent: NodeId,
        index: usize,
    },
    /// Moves a node between positions, carrying both the origin and the destination.
    MoveNode {
        id: NodeId,
        from_parent: NodeId,
        from_index: usize,
        to_parent: NodeId,
        to_index: usize,
    },
    /// Sets a property, remembering what was there (`None` when it was unset).
    SetProperty {
        id: NodeId,
        name: PropertyName,
        previous: Option<PropertyValue>,
        value: PropertyValue,
    },
    /// Removes a property, carrying the value being removed. That value is what makes the
    /// operation checkable and invertible: the store refuses it if the property does not currently
    /// hold it, and undoing it is "set it back to exactly this".
    ClearProperty {
        id: NodeId,
        name: PropertyName,
        removed: PropertyValue,
    },
}

impl ChangeOp {
    /// Which capability this operation needs. Structure changes are reviewed; property changes are
    /// the reversible case an agent may do on its own (whitepaper §12.5).
    pub const fn capability(&self) -> Capability {
        match self {
            Self::AddNode { .. } | Self::RemoveNode { .. } | Self::MoveNode { .. } => {
                Capability::AddOrRemoveNode
            }
            Self::SetProperty { .. } | Self::ClearProperty { .. } => Capability::EditWidgetProperty,
        }
    }

    /// The node this operation acts on, when it acts on an existing one.
    pub fn target(&self) -> NodeId {
        match self {
            Self::AddNode { parent, .. } => *parent,
            Self::RemoveNode { node, .. } => node.id(),
            Self::MoveNode { id, .. } => *id,
            Self::SetProperty { id, .. } | Self::ClearProperty { id, .. } => *id,
        }
    }

    /// The operation that undoes this one, derived from what the operation carries.
    pub fn inverse(&self) -> Self {
        match self {
            Self::AddNode {
                parent,
                index,
                node,
            } => Self::RemoveNode {
                node: node.clone(),
                parent: *parent,
                index: *index,
            },
            Self::RemoveNode {
                node,
                parent,
                index,
            } => Self::AddNode {
                parent: *parent,
                index: *index,
                node: node.clone(),
            },
            Self::MoveNode {
                id,
                from_parent,
                from_index,
                to_parent,
                to_index,
            } => Self::MoveNode {
                id: *id,
                from_parent: *to_parent,
                from_index: *to_index,
                to_parent: *from_parent,
                to_index: *from_index,
            },
            // Undoing a set means putting back what was there — and the inverse's own `previous`
            // is the value the set left behind, which is what the store will find in place.
            Self::SetProperty {
                id,
                name,
                previous,
                value,
            } => match previous {
                Some(previous) => Self::SetProperty {
                    id: *id,
                    name: name.clone(),
                    previous: Some(value.clone()),
                    value: previous.clone(),
                },
                None => Self::ClearProperty {
                    id: *id,
                    name: name.clone(),
                    removed: value.clone(),
                },
            },
            // Undoing a clear means setting the removed value back onto a property that is now unset.
            Self::ClearProperty { id, name, removed } => Self::SetProperty {
                id: *id,
                name: name.clone(),
                previous: None,
                value: removed.clone(),
            },
        }
    }

    /// A one-line description for review prompts and logs.
    pub fn describe(&self) -> String {
        match self {
            Self::AddNode {
                node,
                parent,
                index,
            } => format!(
                "add {} {} under {} at {}",
                node.kind().as_str(),
                node.id().get(),
                parent.get(),
                index,
            ),
            Self::RemoveNode { node, index, .. } => {
                format!("remove node {} from position {}", node.id().get(), index)
            }
            Self::MoveNode {
                id,
                to_parent,
                to_index,
                ..
            } => {
                format!(
                    "move node {} under {} at {}",
                    id.get(),
                    to_parent.get(),
                    to_index
                )
            }
            Self::SetProperty { id, name, .. } => {
                format!("set {} on node {}", name.as_str(), id.get())
            }
            Self::ClearProperty { id, name, .. } => {
                format!("clear {} on node {}", name.as_str(), id.get())
            }
        }
    }
}

/// A set of operations proposed as one unit.
#[derive(Clone, PartialEq, Debug)]
pub struct ChangeSet {
    base: Revision,
    origin: ChangeOrigin,
    intent: String,
    ops: Vec<ChangeOp>,
}

impl ChangeSet {
    /// Builds a change set, or `None` when it has no operations or no stated intent.
    ///
    /// An empty change set is refused because applying it would move the revision without changing
    /// anything; an unexplained one because "what was this change for" then becomes unanswerable.
    pub fn new(
        base: Revision,
        origin: ChangeOrigin,
        intent: &str,
        ops: Vec<ChangeOp>,
    ) -> Option<Self> {
        let intent = intent.trim();
        if ops.is_empty() || intent.is_empty() {
            return None;
        }
        Some(Self {
            base,
            origin,
            intent: intent.to_owned(),
            ops,
        })
    }

    /// The revision this change set was prepared against. Applying it to any other revision is a
    /// conflict, not an implicit merge.
    pub fn base(&self) -> Revision {
        self.base
    }

    /// Who proposed it.
    pub fn origin(&self) -> ChangeOrigin {
        self.origin
    }

    /// What it is for.
    pub fn intent(&self) -> &str {
        &self.intent
    }

    /// The operations, in application order.
    pub fn ops(&self) -> &[ChangeOp] {
        &self.ops
    }

    /// The capabilities this change set needs, in first-use order. The store checks each of them.
    pub fn capabilities(&self) -> Vec<Capability> {
        let mut needed = Vec::new();
        for op in &self.ops {
            let capability = op.capability();
            if !needed.contains(&capability) {
                needed.push(capability);
            }
        }
        needed
    }

    /// The change set that undoes this one, to be applied at `applied_at` — the revision this one
    /// produced.
    pub fn inverse(&self, applied_at: Revision) -> Self {
        let ops = self.ops.iter().rev().map(ChangeOp::inverse).collect();
        Self {
            base: applied_at,
            origin: self.origin,
            intent: format!("undo: {}", self.intent),
            ops,
        }
    }
}

/// Why a change set was refused. Every variant names the expectation that failed.
#[derive(Clone, PartialEq, Debug)]
pub enum ChangeRejection {
    /// The change set was prepared against a different revision: the document moved on.
    StaleBase { base: Revision, current: Revision },
    /// The permission gate refused an operation the change set needs.
    Denied {
        capability: Capability,
        reason: DenialReason,
    },
    /// An operation names a node the document does not have.
    UnknownNode { id: NodeId },
    /// An operation names a node that was removed. Retired ids are not resurrected.
    RetiredNode { id: NodeId },
    /// An operation names a page the document does not have.
    UnknownPage { page: PageId },
    /// A node was placed under a parent on another page.
    CrossPagePlacement { node: NodeId, parent: NodeId },
    /// A node is added twice in one change set.
    DuplicateNode { id: NodeId },
    /// An operation's recorded position is not the position the document has.
    StaleStructure {
        id: NodeId,
        expected: String,
        found: String,
    },
    /// A page root was removed or moved. Pages own their root; replacing it is a page operation.
    PageRootChange { page: PageId },
    /// A subtree removal was requested without removing the children first.
    ImplicitSubtreeRemoval { id: NodeId, children: usize },
    /// A move would place a node inside its own subtree.
    CyclicMove { id: NodeId, parent: NodeId },
    /// A placement index is past the end of the target's child list.
    PlacementOutOfRange {
        id: NodeId,
        index: usize,
        children: usize,
    },
    /// A property operation expected a value the node does not currently have.
    ConflictingValue {
        id: NodeId,
        name: PropertyName,
        expected: Option<PropertyValue>,
        found: Option<PropertyValue>,
    },
}

impl ChangeRejection {
    /// The machine name used in the contract data and by tests.
    pub const fn name(&self) -> &'static str {
        match self {
            Self::StaleBase { .. } => "stale-base",
            Self::Denied { .. } => "denied",
            Self::UnknownNode { .. } => "unknown-node",
            Self::RetiredNode { .. } => "retired-node",
            Self::UnknownPage { .. } => "unknown-page",
            Self::CrossPagePlacement { .. } => "cross-page-placement",
            Self::DuplicateNode { .. } => "duplicate-node",
            Self::StaleStructure { .. } => "stale-structure",
            Self::PageRootChange { .. } => "page-root-change",
            Self::ImplicitSubtreeRemoval { .. } => "implicit-subtree-removal",
            Self::CyclicMove { .. } => "cyclic-move",
            Self::PlacementOutOfRange { .. } => "placement-out-of-range",
            Self::ConflictingValue { .. } => "conflicting-value",
        }
    }

    /// A message a problem panel can show: what failed, on what, and what to do about it.
    pub fn message(&self) -> String {
        match self {
            Self::StaleBase { base, current } => format!(
                "the change was prepared for {base} but the document is at {current}; reload or re-apply it",
            ),
            Self::Denied { capability, reason } => format!(
                "{} was refused ({}) : {}",
                capability.name(),
                reason.name(),
                reason.explanation(),
            ),
            Self::UnknownNode { id } => format!("node {} does not exist", id.get()),
            Self::RetiredNode { id } => {
                format!("node {} was removed; its identity is not reused", id.get())
            }
            Self::UnknownPage { page } => format!("page {} does not exist", page.get()),
            Self::CrossPagePlacement { node, parent } => format!(
                "node {} belongs to another page than parent {}",
                node.get(),
                parent.get(),
            ),
            Self::DuplicateNode { id } => format!("node {} is placed twice", id.get()),
            Self::StaleStructure {
                id,
                expected,
                found,
            } => format!("node {} was expected {expected} but is {found}", id.get(),),
            Self::PageRootChange { page } => format!(
                "page {} keeps its root node; replacing a page root is a page operation",
                page.get(),
            ),
            Self::ImplicitSubtreeRemoval { id, children } => format!(
                "node {} still has {children} child node(s); remove them first",
                id.get(),
            ),
            Self::CyclicMove { id, parent } => format!(
                "node {} cannot be moved inside parent {} because that parent is in its own subtree",
                id.get(),
                parent.get(),
            ),
            Self::PlacementOutOfRange {
                id,
                index,
                children,
            } => format!(
                "node {} cannot be placed at index {index} where the target has {children} child node(s)",
                id.get(),
            ),
            Self::ConflictingValue {
                id,
                name,
                expected,
                found,
            } => format!(
                "{} on node {} was expected to be {:?} but is {:?}",
                name.as_str(),
                id.get(),
                expected.as_ref().map(PropertyValue::family),
                found.as_ref().map(PropertyValue::family),
            ),
        }
    }
}

/// What happened when a change set reached the store.
#[derive(Clone, PartialEq, Debug)]
pub enum ApplyOutcome {
    /// Every operation landed. The document moved from `from` to `to` in one step.
    Applied { from: Revision, to: Revision },
    /// Nothing landed. `revision` is the document's revision, unchanged by the attempt.
    Rejected {
        rejection: ChangeRejection,
        revision: Revision,
    },
}

impl ApplyOutcome {
    /// Whether the change set was applied.
    pub const fn is_applied(&self) -> bool {
        matches!(self, Self::Applied { .. })
    }

    /// The document's revision after the attempt, whether or not it was applied.
    pub const fn revision(&self) -> Revision {
        match self {
            Self::Applied { to, .. } => *to,
            Self::Rejected { revision, .. } => *revision,
        }
    }

    /// The rejection, when there was one.
    pub const fn rejection(&self) -> Option<&ChangeRejection> {
        match self {
            Self::Applied { .. } => None,
            Self::Rejected { rejection, .. } => Some(rejection),
        }
    }
}

impl fmt::Display for ApplyOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Applied { from, to } => write!(formatter, "applied {from} -> {to}"),
            Self::Rejected {
                rejection,
                revision,
            } => {
                write!(formatter, "rejected at {revision}: {}", rejection.message())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ApplyOutcome, ChangeOp, ChangeOrigin, ChangeRejection, ChangeSet};
    use crate::contracts::identity::{IdAllocator, NodeId, PageId};
    use crate::contracts::model::{Node, NodeKind, PropertyName, PropertyValue};
    use crate::contracts::permission::{Capability, DenialReason, Principal};
    use crate::contracts::revision::Revision;

    fn node(id: NodeId, page: PageId) -> Box<Node> {
        Box::new(Node::new(page, id, NodeKind::new("button").unwrap()))
    }

    #[test]
    fn a_change_set_needs_operations_and_an_intent() {
        assert!(ChangeSet::new(Revision::INITIAL, ChangeOrigin::Human, "  ", Vec::new()).is_none());

        let page = PageId::from_raw(0);
        let parent = NodeId::from_raw(9);
        let op = ChangeOp::AddNode {
            parent,
            index: 0,
            node: node(NodeId::from_raw(0), page),
        };
        assert!(
            ChangeSet::new(
                Revision::INITIAL,
                ChangeOrigin::Human,
                "\t",
                vec![op.clone()]
            )
            .is_none()
        );

        let set = ChangeSet::new(
            Revision::INITIAL,
            ChangeOrigin::Human,
            " tidy up ",
            vec![op],
        )
        .unwrap();
        assert_eq!(set.intent(), "tidy up");
        assert_eq!(set.base(), Revision::INITIAL);
        assert_eq!(set.ops().len(), 1);
    }

    #[test]
    fn structure_changes_are_reviewed_and_property_changes_are_not() {
        let mut allocator = IdAllocator::new();
        let page = PageId::from_raw(0);
        let id = allocator.allocate();
        let name = PropertyName::new("text").unwrap();

        let structural = ChangeOp::AddNode {
            parent: NodeId::from_raw(9),
            index: 0,
            node: node(id, page),
        };
        let property = ChangeOp::SetProperty {
            id,
            name,
            previous: None,
            value: PropertyValue::Text("ok".into()),
        };

        assert_eq!(structural.capability(), Capability::AddOrRemoveNode);
        assert_eq!(property.capability(), Capability::EditWidgetProperty);
        assert_eq!(structural.target(), NodeId::from_raw(9));
        assert_eq!(
            structural.describe(),
            format!("add button {} under 9 at 0", id.get())
        );

        let set = ChangeSet::new(
            Revision::INITIAL,
            ChangeOrigin::Agent,
            "label the button",
            vec![property, structural],
        )
        .unwrap();

        assert_eq!(
            set.capabilities(),
            vec![Capability::EditWidgetProperty, Capability::AddOrRemoveNode],
        );
        assert_eq!(set.origin().principal(), Principal::Agent);
        assert_eq!(set.origin().name(), "agent");
    }

    #[test]
    fn setting_a_property_inverts_to_restoring_or_clearing_it() {
        let name = PropertyName::new("text").unwrap();
        let id = NodeId::from_raw(2);

        let over_an_old_value = ChangeOp::SetProperty {
            id,
            name: name.clone(),
            previous: Some(PropertyValue::Text("old".into())),
            value: PropertyValue::Text("new".into()),
        };
        assert_eq!(
            over_an_old_value.inverse(),
            ChangeOp::SetProperty {
                id,
                name: name.clone(),
                previous: Some(PropertyValue::Text("new".into())),
                value: PropertyValue::Text("old".into()),
            },
        );

        let over_nothing = ChangeOp::SetProperty {
            id,
            name: name.clone(),
            previous: None,
            value: PropertyValue::Text("new".into()),
        };
        assert_eq!(
            over_nothing.inverse(),
            ChangeOp::ClearProperty {
                id,
                name,
                removed: PropertyValue::Text("new".into())
            },
        );
    }

    #[test]
    fn clearing_a_property_inverts_to_setting_the_removed_value_back() {
        let name = PropertyName::new("text").unwrap();
        let id = NodeId::from_raw(2);
        let cleared = ChangeOp::ClearProperty {
            id,
            name: name.clone(),
            removed: PropertyValue::Text("gone".into()),
        };

        assert_eq!(
            cleared.inverse(),
            ChangeOp::SetProperty {
                id,
                name,
                previous: None,
                value: PropertyValue::Text("gone".into()),
            },
        );
    }

    #[test]
    fn structural_operations_invert_to_the_exact_opposite_position() {
        let mut allocator = IdAllocator::new();
        let page = PageId::from_raw(0);
        let parent = allocator.allocate();
        let child = allocator.allocate();

        let add = ChangeOp::AddNode {
            parent,
            index: 1,
            node: node(child, page),
        };
        assert_eq!(
            add.inverse(),
            ChangeOp::RemoveNode {
                node: node(child, page),
                parent,
                index: 1
            },
        );

        let grandparent = allocator.allocate();
        let remove = ChangeOp::RemoveNode {
            node: node(child, page),
            parent,
            index: 3,
        };
        assert_eq!(
            remove.inverse(),
            ChangeOp::AddNode {
                parent,
                index: 3,
                node: node(child, page)
            },
        );

        let moves = ChangeOp::MoveNode {
            id: child,
            from_parent: parent,
            from_index: 2,
            to_parent: grandparent,
            to_index: 0,
        };
        assert_eq!(
            moves.inverse(),
            ChangeOp::MoveNode {
                id: child,
                from_parent: grandparent,
                from_index: 0,
                to_parent: parent,
                to_index: 2,
            },
        );
    }

    #[test]
    fn an_inverse_runs_backwards_and_carries_the_revision_it_undoes() {
        let mut allocator = IdAllocator::new();
        let page = PageId::from_raw(0);
        let parent = allocator.allocate();
        let first = allocator.allocate();
        let second = allocator.allocate();
        let set = ChangeSet::new(
            Revision::from_raw(4),
            ChangeOrigin::Human,
            "add two nodes",
            vec![
                ChangeOp::AddNode {
                    parent,
                    index: 0,
                    node: node(first, page),
                },
                ChangeOp::AddNode {
                    parent,
                    index: 1,
                    node: node(second, page),
                },
            ],
        )
        .unwrap();

        let inverse = set.inverse(Revision::from_raw(5));

        assert_eq!(inverse.base(), Revision::from_raw(5));
        assert_eq!(inverse.intent(), "undo: add two nodes");
        assert_eq!(
            inverse.ops(),
            [
                ChangeOp::RemoveNode {
                    node: node(second, page),
                    parent,
                    index: 1
                },
                ChangeOp::RemoveNode {
                    node: node(first, page),
                    parent,
                    index: 0
                },
            ],
        );
    }

    #[test]
    fn an_outcome_always_names_a_revision() {
        let applied = ApplyOutcome::Applied {
            from: Revision::INITIAL,
            to: Revision::from_raw(1),
        };
        assert!(applied.is_applied());
        assert_eq!(applied.revision(), Revision::from_raw(1));
        assert_eq!(applied.rejection(), None);
        assert_eq!(applied.to_string(), "applied r0 -> r1");

        let rejected = ApplyOutcome::Rejected {
            rejection: ChangeRejection::UnknownNode {
                id: NodeId::from_raw(9),
            },
            revision: Revision::from_raw(3),
        };
        assert!(!rejected.is_applied());
        assert_eq!(rejected.revision(), Revision::from_raw(3));
        assert_eq!(
            rejected.rejection().map(ChangeRejection::name),
            Some("unknown-node")
        );
        assert!(rejected.to_string().contains("node 9 does not exist"));
    }

    #[test]
    fn every_rejection_names_itself_uniquely_and_explains_itself() {
        let name = PropertyName::new("text").unwrap();
        let rejections = [
            ChangeRejection::StaleBase {
                base: Revision::INITIAL,
                current: Revision::from_raw(2),
            },
            ChangeRejection::Denied {
                capability: Capability::WriteFile,
                reason: DenialReason::HumanOnly,
            },
            ChangeRejection::UnknownNode {
                id: NodeId::from_raw(1),
            },
            ChangeRejection::RetiredNode {
                id: NodeId::from_raw(1),
            },
            ChangeRejection::UnknownPage {
                page: PageId::from_raw(1),
            },
            ChangeRejection::CrossPagePlacement {
                node: NodeId::from_raw(1),
                parent: NodeId::from_raw(2),
            },
            ChangeRejection::DuplicateNode {
                id: NodeId::from_raw(1),
            },
            ChangeRejection::StaleStructure {
                id: NodeId::from_raw(1),
                expected: "at index 0".into(),
                found: "at index 2".into(),
            },
            ChangeRejection::PageRootChange {
                page: PageId::from_raw(0),
            },
            ChangeRejection::ImplicitSubtreeRemoval {
                id: NodeId::from_raw(1),
                children: 2,
            },
            ChangeRejection::CyclicMove {
                id: NodeId::from_raw(1),
                parent: NodeId::from_raw(2),
            },
            ChangeRejection::PlacementOutOfRange {
                id: NodeId::from_raw(1),
                index: 5,
                children: 1,
            },
            ChangeRejection::ConflictingValue {
                id: NodeId::from_raw(1),
                name,
                expected: None,
                found: Some(PropertyValue::Text("x".into())),
            },
        ];

        let mut names = Vec::new();
        for rejection in rejections {
            assert!(
                rejection.message().len() > rejection.name().len(),
                "{}",
                rejection.name()
            );
            names.push(rejection.name());
        }
        let unique = names
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        assert_eq!(unique, names.len(), "two rejections share a machine name");
    }
}
