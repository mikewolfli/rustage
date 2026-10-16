// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Dragging a node in the object tree or on the canvas: the drop candidates a drag offers, the
//! insertion preview, cancellation, and a batch transaction with one-step undo (task T09.06).
//!
//! The acceptance line is *"an invalid drop leaves no half-move, and a single undo restores"*. The
//! module makes both true by keeping the *deciding* separate from the *writing*:
//!
//! * **A drag decides; it does not write.** [`Drag`] holds the node being dragged and where it is
//!   aimed. [`Drag::begin`] and [`Drag::aim`] change no model, so a cancelled drag leaves nothing
//!   behind *by construction* rather than by clean-up.
//! * **Releasing names one of three outcomes.** [`Drag::release`] answers [`Drop::Cancelled`],
//!   [`Drop::Refused`] (with the reason) or [`Drop::Move`]. Only the last is a write, so "invalid
//!   drop leaves no half-move" is the fact that a refusal is not a move.
//! * **The candidates and the move share one rule.** [`Drag::candidates`] and [`Drag::release`] both
//!   ask [`Tree::can_reparent_at`], so a container the drag offers is a container the move accepts.
//! * **A batch is applied to a copy and adopted whole.** [`Design::apply`] applies every edit to a
//!   clone and replaces the tree only if all of them succeeded, so a batch that is refused halfway
//!   changes nothing.
//! * **One undo is one transaction.** The tree before a successful batch is what [`Design::undo`]
//!   restores, so a person takes back a whole drag — or a whole paste — with one step.

#![allow(dead_code)]

use crate::app::structure::{Kind, StructError, Tree};
use crate::contracts::identity::NodeId;

/// Where a drag would put the node: a parent (or the root) and a position among its children.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Move {
    /// The node being moved.
    pub node: NodeId,
    /// Its new parent, or `None` for the root.
    pub parent: Option<NodeId>,
    /// The position among the parent's other children.
    pub index: usize,
}

impl Move {
    /// A move.
    pub fn new(node: NodeId, parent: Option<NodeId>, index: usize) -> Self {
        Self {
            node,
            parent,
            index,
        }
    }

    /// Whether this drop would be accepted, without applying it (the container preview's decision).
    pub fn check(&self, tree: &Tree) -> Result<(), StructError> {
        tree.can_reparent_at(self.node, self.parent, self.index)
    }

    /// The move in one line.
    pub fn render(&self) -> String {
        let parent = match self.parent {
            Some(parent) => format!("under {}", parent.get()),
            None => "at the root".to_owned(),
        };
        format!("move node {} {parent} at {}", self.node.get(), self.index)
    }
}

/// A place a drag may land: a container (or the root) and the positions it offers.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Target {
    /// The container, or `None` for the root.
    pub container: Option<NodeId>,
    /// How many positions the drop may name (`0..=slots`).
    pub slots: usize,
}

impl Target {
    /// The target in one line.
    pub fn render(&self) -> String {
        match self.container {
            Some(container) => format!("node {} ({} slot(s))", container.get(), self.slots),
            None => format!("the root ({} slot(s))", self.slots),
        }
    }
}

/// The drop candidates a drag of one node offers.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Candidates {
    targets: Vec<Target>,
}

impl Candidates {
    /// The root and every container that would accept `node` and is not inside its own subtree.
    pub fn of(tree: &Tree, node: NodeId) -> Result<Self, StructError> {
        let dragged = tree.get(node).ok_or(StructError::Missing { node })?;
        let kind = dragged.kind;
        let mut targets = vec![Target {
            container: None,
            slots: tree.insert_slots(node, None),
        }];
        for id in tree.ids() {
            if id == node {
                continue;
            }
            let Some(holder) = tree.get(id) else {
                continue;
            };
            if !holder.kind.allows_child(kind) || tree.is_descendant(id, node) {
                continue;
            }
            targets.push(Target {
                container: Some(id),
                slots: tree.insert_slots(node, Some(id)),
            });
        }
        Ok(Self { targets })
    }

    /// The targets.
    pub fn targets(&self) -> &[Target] {
        &self.targets
    }

    /// Whether a container is one of the candidates.
    pub fn accepts(&self, container: Option<NodeId>) -> bool {
        self.targets
            .iter()
            .any(|target| target.container == container)
    }

    /// The candidates in a report.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!("candidates: {} target(s)", self.targets.len())];
        for target in &self.targets {
            lines.push(format!("  {}", target.render()));
        }
        lines
    }
}

/// What releasing a drag does.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Drop {
    /// No target is aimed at, so nothing happens.
    Cancelled,
    /// The aim is refused; nothing happens, and the reason is named.
    Refused(String),
    /// The move to apply.
    Move(Move),
}

impl Drop {
    /// The outcome in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Cancelled => "drop: cancelled".to_owned(),
            Self::Refused(reason) => format!("drop: refused — {reason}"),
            Self::Move(moving) => format!("drop: {}", moving.render()),
        }
    }
}

/// Where a drop would put the node, said without moving it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Preview {
    /// The node being dragged.
    pub node: NodeId,
    /// Where it would land.
    pub target: Target,
    /// The position it would take.
    pub index: usize,
    /// Whether the drop is accepted.
    pub valid: bool,
    /// Why it is not, when it is not.
    pub reason: Option<String>,
}

impl Preview {
    /// The preview in one line.
    pub fn render(&self) -> String {
        let at = match self.target.container {
            Some(container) => format!("node {} at {}", container.get(), self.index),
            None => format!("the root at {}", self.index),
        };
        if self.valid {
            format!("preview: node {} into {at}", self.node.get())
        } else {
            format!(
                "preview: node {} into {at} refused — {}",
                self.node.get(),
                self.reason.as_deref().unwrap_or("no reason")
            )
        }
    }
}

/// A drag in progress: the node, its candidates, and the aim it currently has.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Drag {
    node: NodeId,
    candidates: Candidates,
    aim: Option<Target>,
    index: usize,
}

impl Drag {
    /// Starts dragging `node`, refusing a node that does not exist. Writes nothing.
    pub fn begin(tree: &Tree, node: NodeId) -> Result<Self, StructError> {
        Ok(Self {
            node,
            candidates: Candidates::of(tree, node)?,
            aim: None,
            index: 0,
        })
    }

    /// The node being dragged.
    pub fn node(&self) -> NodeId {
        self.node
    }

    /// The candidates this drag offers.
    pub fn candidates(&self) -> &Candidates {
        &self.candidates
    }

    /// Aims the drag at a container. Writes nothing; the aim is checked when it is released.
    pub fn aim(&mut self, container: Option<NodeId>, index: usize) {
        let slots = self
            .candidates
            .targets()
            .iter()
            .find(|target| target.container == container)
            .map_or(0, |target| target.slots);
        self.aim = Some(Target { container, slots });
        self.index = index;
    }

    /// The aim, if there is one.
    pub fn aim_target(&self) -> Option<&Target> {
        self.aim.as_ref()
    }

    /// The position the drag is aimed at.
    pub fn index(&self) -> usize {
        self.index
    }

    /// Where the drop would put the node, described without moving it.
    pub fn preview(&self, tree: &Tree) -> Preview {
        let attempt = self.attempt();
        let target = Target {
            container: self.aim.as_ref().and_then(|aim| aim.container),
            slots: match &attempt {
                Some(moving) => tree.insert_slots(moving.node, moving.parent),
                None => 0,
            },
        };
        let result = attempt.map(|moving| moving.check(tree));
        match result {
            Some(Ok(())) => Preview {
                node: self.node,
                target,
                index: self.index,
                valid: true,
                reason: None,
            },
            Some(Err(error)) => Preview {
                node: self.node,
                target,
                index: self.index,
                valid: false,
                reason: Some(error.message()),
            },
            None => Preview {
                node: self.node,
                target,
                index: self.index,
                valid: false,
                reason: Some("no target is aimed at".to_owned()),
            },
        }
    }

    /// Releases the drag: cancelled with no aim, refused if the aim is invalid, else the move.
    pub fn release(&self, tree: &Tree) -> Drop {
        let Some(moving) = self.attempt() else {
            return Drop::Cancelled;
        };
        match moving.check(tree) {
            Ok(()) => Drop::Move(moving),
            Err(error) => Drop::Refused(error.message()),
        }
    }

    /// The move the current aim would make, before it is checked.
    fn attempt(&self) -> Option<Move> {
        let aim = self.aim.as_ref()?;
        Some(Move::new(self.node, aim.container, self.index))
    }
}

/// One edit in a batch.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Edit {
    /// Inserts a new node.
    Insert {
        /// Where.
        parent: Option<NodeId>,
        /// What kind.
        kind: Kind,
        /// Its name.
        name: String,
    },
    /// Moves an existing node.
    Move(Move),
    /// Removes a node and the subtree under it.
    Delete {
        /// The node.
        node: NodeId,
    },
}

/// What one edit did.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Outcome {
    /// A node was inserted, with its fresh id.
    Inserted(NodeId),
    /// A node was moved.
    Moved(NodeId),
    /// A node and its subtree were removed, with the references left dangling.
    Deleted {
        /// Every id removed.
        removed: Vec<NodeId>,
        /// Every reference that pointed at a removed node.
        references: Vec<(NodeId, NodeId)>,
    },
}

impl Outcome {
    /// The outcome in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Inserted(node) => format!("inserted node {}", node.get()),
            Self::Moved(node) => format!("moved node {}", node.get()),
            Self::Deleted {
                removed,
                references,
            } => format!(
                "deleted {} node(s), {} reference(s) to fix",
                removed.len(),
                references.len()
            ),
        }
    }
}

/// A design session: the object tree, a batch transaction, and one-step undo.
#[derive(Clone, Debug, Default)]
pub struct Design {
    tree: Tree,
    past: Vec<Tree>,
    future: Vec<Tree>,
    limit: Option<usize>,
}

impl Design {
    /// An empty design with unbounded history.
    pub fn new() -> Self {
        Self::default()
    }

    /// An empty design whose undo history keeps at most `limit` transactions, so a long session’s
    /// memory does not grow without bound (task T09.08).
    pub fn with_limit(limit: usize) -> Self {
        Self {
            limit: Some(limit),
            ..Self::default()
        }
    }

    /// How many transactions can be taken back, and how many put forward again.
    pub fn history(&self) -> (usize, usize) {
        (self.past.len(), self.future.len())
    }

    /// The tree.
    pub fn tree(&self) -> &Tree {
        &self.tree
    }

    /// Whether there is anything to take back.
    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }

    /// Whether anything was taken back that can be put forward again.
    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }

    /// Applies every edit as **one transaction**, and returns what landed.
    ///
    /// The edits are applied to a copy; the copy is adopted only when all of them succeeded, so a
    /// batch refused halfway changes nothing. An empty batch is not a step, so it is not undoable.
    pub fn apply(&mut self, edits: &[Edit]) -> Result<Vec<Outcome>, StructError> {
        if edits.is_empty() {
            return Ok(Vec::new());
        }
        let mut working = self.tree.clone();
        let mut outcomes = Vec::with_capacity(edits.len());
        for edit in edits {
            let outcome = match edit {
                Edit::Insert { parent, kind, name } => {
                    Outcome::Inserted(working.insert(*parent, *kind, name.clone())?)
                }
                Edit::Move(moving) => {
                    working.reparent_at(moving.node, moving.parent, moving.index)?;
                    Outcome::Moved(moving.node)
                }
                Edit::Delete { node } => {
                    let report = working.delete(*node)?;
                    Outcome::Deleted {
                        removed: report.removed,
                        references: report.references,
                    }
                }
            };
            outcomes.push(outcome);
        }
        let previous = std::mem::replace(&mut self.tree, working);
        self.past.push(previous);
        if let Some(limit) = self.limit
            && self.past.len() > limit
        {
            let drop = self.past.len() - limit;
            self.past.drain(0..drop);
        }
        self.future.clear();
        Ok(outcomes)
    }

    /// Takes back the newest transaction, and says whether there was one.
    pub fn undo(&mut self) -> bool {
        let Some(previous) = self.past.pop() else {
            return false;
        };
        let current = std::mem::replace(&mut self.tree, previous);
        self.future.push(current);
        true
    }

    /// Puts back the newest taken-back transaction, and says whether there was one.
    pub fn redo(&mut self) -> bool {
        let Some(next) = self.future.pop() else {
            return false;
        };
        let current = std::mem::replace(&mut self.tree, next);
        self.past.push(current);
        true
    }

    /// The design in a report.
    pub fn render(&self) -> Vec<String> {
        let mut lines = self.tree.render();
        lines.push(format!(
            "history: {} past, {} future",
            self.past.len(),
            self.future.len()
        ));
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{Candidates, Design, Drag, Drop, Edit, Move, Outcome};
    use crate::app::structure::{Kind, Tree};
    use crate::contracts::identity::NodeId;

    fn node(value: u64) -> NodeId {
        NodeId::from_raw(value)
    }

    /// A design with three widgets in a row under one container (root 0, children 1, 2, 3).
    fn seeded() -> Design {
        let mut design = Design::new();
        design
            .apply(&[Edit::Insert {
                parent: None,
                kind: Kind::Container,
                name: "panel".to_owned(),
            }])
            .expect("a root");
        for name in ["a", "b", "c"] {
            design
                .apply(&[Edit::Insert {
                    parent: Some(node(0)),
                    kind: Kind::Widget,
                    name: name.to_owned(),
                }])
                .expect("a child");
        }
        design
    }

    #[test]
    fn a_batch_is_adopted_whole_or_not_at_all() {
        let mut design = seeded();
        let before = design.tree().clone();
        // The second move is refused (no position 9), so the first must not stick either.
        let result = design.apply(&[
            Edit::Move(Move::new(node(1), None, 0)),
            Edit::Move(Move::new(node(2), Some(node(0)), 9)),
        ]);
        assert!(result.is_err());
        assert_eq!(
            design.tree(),
            &before,
            "a refused batch leaves no half-move"
        );
    }

    #[test]
    fn one_undo_takes_back_a_whole_batch() {
        let mut design = seeded();
        let before = design.tree().clone();
        design
            .apply(&[
                Edit::Move(Move::new(node(1), None, 0)),
                Edit::Move(Move::new(node(2), None, 1)),
            ])
            .expect("a valid batch");
        assert_ne!(design.tree(), &before);
        assert!(design.undo());
        assert_eq!(design.tree(), &before, "one undo restores the whole batch");
    }

    #[test]
    fn redo_puts_the_batch_back() {
        let mut design = seeded();
        design
            .apply(&[Edit::Move(Move::new(node(1), None, 0))])
            .expect("a move");
        let moved = design.tree().clone();
        assert!(design.undo());
        assert!(design.redo());
        assert_eq!(design.tree(), &moved);
    }

    #[test]
    fn an_empty_batch_is_not_a_step() {
        let mut design = seeded();
        let history = |design: &Design| design.render().last().cloned().unwrap_or_default();
        let before = history(&design);
        assert_eq!(design.apply(&[]), Ok(Vec::new()));
        assert_eq!(history(&design), before, "an empty batch adds no history");
    }

    #[test]
    fn a_cancelled_drag_writes_nothing() {
        let design = seeded();
        let before = design.tree().clone();
        let drag = Drag::begin(design.tree(), node(1)).expect("a live node");
        assert_eq!(drag.release(design.tree()), Drop::Cancelled);
        assert_eq!(design.tree(), &before);
    }

    #[test]
    fn an_invalid_drop_is_refused_and_changes_nothing() {
        let design = seeded();
        let before = design.tree().clone();
        let mut drag = Drag::begin(design.tree(), node(0)).expect("a live node");
        // Node 0 cannot go under its own child node 1: that would make a cycle.
        drag.aim(Some(node(1)), 0);
        let outcome = drag.release(design.tree());
        assert!(matches!(outcome, Drop::Refused(_)));
        assert_eq!(design.tree(), &before, "a refused drop writes nothing");
    }

    #[test]
    fn a_valid_drop_yields_the_move_but_still_writes_nothing() {
        let design = seeded();
        let before = design.tree().clone();
        let mut drag = Drag::begin(design.tree(), node(2)).expect("a live node");
        drag.aim(Some(node(0)), 0);
        let Drop::Move(moving) = drag.release(design.tree()) else {
            panic!("a valid drop should be a move");
        };
        assert_eq!(moving, Move::new(node(2), Some(node(0)), 0));
        assert_eq!(
            design.tree(),
            &before,
            "releasing decides, it does not write"
        );
    }

    #[test]
    fn the_candidates_offer_every_container_that_accepts_the_kind() {
        let mut tree = Tree::new();
        let panel = tree.insert(None, Kind::Container, "panel").expect("root");
        let _leaf = tree.insert(Some(panel), Kind::Widget, "a").expect("child");
        let candidates = Candidates::of(&tree, node(1)).expect("a live node");
        assert!(candidates.accepts(None));
        assert!(candidates.accepts(Some(panel)));
    }

    #[test]
    fn a_containers_subtree_is_not_offered_for_its_own_ancestor() {
        let mut tree = Tree::new();
        let outer = tree.insert(None, Kind::Container, "outer").expect("root");
        let inner = tree
            .insert(Some(outer), Kind::Container, "inner")
            .expect("child");
        let candidates = Candidates::of(&tree, outer).expect("a live node");
        assert!(
            !candidates.accepts(Some(inner)),
            "no drop into its own subtree"
        );
        assert!(candidates.accepts(None));
    }

    #[test]
    fn the_preview_agrees_with_the_move_it_would_make() {
        let design = seeded();
        let mut drag = Drag::begin(design.tree(), node(1)).expect("a live node");
        drag.aim(Some(node(0)), 1);
        let preview = drag.preview(design.tree());
        assert!(preview.valid);
        assert!(preview.render().contains("preview"));
        assert!(matches!(drag.release(design.tree()), Drop::Move(_)));
    }

    #[test]
    fn deleting_reports_the_references_it_leaves() {
        let mut design = Design::new();
        design
            .apply(&[Edit::Insert {
                parent: None,
                kind: Kind::Container,
                name: "a".to_owned(),
            }])
            .expect("a root");
        design
            .apply(&[Edit::Insert {
                parent: Some(node(0)),
                kind: Kind::Widget,
                name: "b".to_owned(),
            }])
            .expect("a child");
        let outcomes = design
            .apply(&[Edit::Delete { node: node(0) }])
            .expect("a delete");
        assert!(matches!(outcomes.as_slice(), [Outcome::Deleted { .. }]));
        assert!(!design.can_redo());
    }
}
