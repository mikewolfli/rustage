// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Keeping a position honest after the text moves (task T15.06).
//!
//! The acceptance line is *"after continuous formatting and editing, no wrong node is located"*. Four
//! pieces answer it, and each is a plain value rather than a rule to remember:
//!
//! * **Positions are recomputed, not carried by an offset alone.** [`Position`] is a line and a
//!   column (in characters), and [`offset_of`] / [`position_of`] are the one pair that converts
//!   between it and a byte offset, so a multi-byte character cannot shift a column by counting bytes.
//! * **An edit shifts what is after it.** [`Edit`] and [`Edit::apply`] move a byte offset the way an
//!   insertion or deletion of a byte range actually does, and answer `None` for an offset the edit
//!   deleted — a position inside deleted text has no answer, which the caller must handle instead of
//!   receiving a stale offset that happens to be in range.
//! * **A regeneration loop is refused, not survived.** [`LoopGuard`] bounds how many times one edit
//!   may trigger another pass, so "the generator generates, the watcher regenerates" stops at a
//!   counted limit with a reason rather than spinning.
//! * **A mapping that no longer fits is rebuilt or reported.** [`recover`] relocates against the new
//!   text and, when it can map strictly fewer nodes than it did before, reports the loss instead of
//!   handing back a map that quietly forgot a node.

#![allow(dead_code)]

use crate::contracts::identity::NodeId;
use crate::contracts::source_map::SourceMap;

use super::mapping;

/// A line and a column, both one-based, the column counted in characters.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Position {
    /// The one-based line.
    pub line: u32,
    /// The one-based column, in characters.
    pub column: u32,
}

impl Position {
    /// The position in one line.
    pub fn render(&self) -> String {
        format!("{}:{}", self.line, self.column)
    }
}

/// The position of a byte offset, clamping to the end of the text.
pub fn position_of(text: &str, byte: u32) -> Position {
    let byte = byte.min(text.len() as u32) as usize;
    let mut line = 1u32;
    let mut column = 1u32;
    for (at, character) in text.char_indices() {
        if at >= byte {
            break;
        }
        if character == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    Position { line, column }
}

/// The byte offset of a position, or `None` when the line or column is past the text.
pub fn offset_of(text: &str, position: Position) -> Option<u32> {
    if position.line == 0 || position.column == 0 {
        return None;
    }
    let mut line = 1u32;
    let mut column = 1u32;
    if line == position.line && column == position.column {
        return Some(0);
    }
    for (at, character) in text.char_indices() {
        if character == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
        if line == position.line && column == position.column {
            return Some(at as u32 + character.len_utf8() as u32);
        }
        if line > position.line {
            return None;
        }
    }
    None
}

/// One edit to a source: a byte range replaced by text of a different length.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Edit {
    /// The first byte removed.
    pub start: u32,
    /// One past the last byte removed.
    pub end: u32,
    /// How many bytes are inserted in its place.
    pub inserted: u32,
}

impl Edit {
    /// How far an offset at or after the edit's end moves.
    pub fn delta(&self) -> i64 {
        i64::from(self.inserted) - i64::from(self.end - self.start)
    }

    /// Where `offset` ends up after this edit, or `None` when the edit deletes it.
    pub fn apply(&self, offset: u32) -> Option<u32> {
        if offset < self.start {
            return Some(offset);
        }
        if offset < self.end {
            return None;
        }
        Some((i64::from(offset) + self.delta()).max(0) as u32)
    }
}

/// Refuses a regeneration that would run away.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LoopGuard {
    limit: u32,
    depth: u32,
}

impl LoopGuard {
    /// A guard that allows `limit` nested passes.
    pub fn new(limit: u32) -> Self {
        Self { limit, depth: 0 }
    }

    /// Enters one pass, or explains that the limit is reached.
    pub fn enter(&mut self) -> Result<u32, String> {
        if self.depth >= self.limit {
            return Err(format!(
                "a change triggered {} further passes, which is the limit; the loop is stopped",
                self.limit
            ));
        }
        self.depth += 1;
        Ok(self.depth)
    }

    /// Leaves one pass.
    pub fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    /// How deep the current chain is.
    pub fn depth(&self) -> u32 {
        self.depth
    }
}

/// One step of the generate domain's history, so an undo can cross views.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Step {
    /// The view the edit came from (`tree`, `canvas`, `code`, …).
    pub view: String,
    /// What the step did, for the history list.
    pub label: String,
    /// The mapping's node set before the step, so undoing can restore it.
    pub before: Vec<NodeId>,
    /// The mapping's node set after the step.
    pub after: Vec<NodeId>,
}

/// A history the generator's views share, so one undo crosses every view of one change.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Journal {
    steps: Vec<Step>,
    undone: Vec<Step>,
}

impl Journal {
    /// An empty journal.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a step, clearing the redo stack.
    pub fn record(&mut self, step: Step) {
        self.undone.clear();
        self.steps.push(step);
    }

    /// Takes back the last step, whichever view recorded it.
    pub fn undo(&mut self) -> Option<Step> {
        let step = self.steps.pop()?;
        self.undone.push(step.clone());
        Some(step)
    }

    /// Puts back the last undone step.
    pub fn redo(&mut self) -> Option<Step> {
        let step = self.undone.pop()?;
        self.steps.push(step.clone());
        Some(step)
    }

    /// How many steps can be taken back.
    pub fn undoable(&self) -> usize {
        self.steps.len()
    }
}

/// What rebuilding a mapping against changed text found.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Recovery {
    /// Whether the map was rebuilt from the new text.
    pub rebuilt: bool,
    /// Nodes the previous map held that the new one does not.
    pub lost: Vec<NodeId>,
}

impl Recovery {
    /// Whether the mapping still holds every node it did.
    pub fn is_complete(&self) -> bool {
        self.lost.is_empty()
    }

    /// The recovery in one line.
    pub fn render(&self) -> String {
        if self.lost.is_empty() {
            format!(
                "mapping rebuilt against {} nodes",
                if self.rebuilt { "new" } else { "old" }
            )
        } else {
            format!("mapping lost {} node(s)", self.lost.len())
        }
    }
}

/// Rebuilds a mapping against changed text, reporting any node it can no longer place.
pub fn recover(
    file: &str,
    source: &str,
    previous: &SourceMap,
    keys: &[(NodeId, String)],
    all_nodes: &[NodeId],
) -> (SourceMap, Recovery) {
    let rebuilt = mapping::relocate(file, source, previous, keys);
    let lost: Vec<NodeId> = all_nodes
        .iter()
        .copied()
        .filter(|node| previous.span_of(*node).is_some() && rebuilt.span_of(*node).is_none())
        .collect();
    (
        rebuilt,
        Recovery {
            rebuilt: true,
            lost,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::{Edit, Journal, LoopGuard, Position, Step, offset_of, position_of, recover};
    use crate::contracts::identity::NodeId;
    use crate::contracts::revision::{GeneratorVersion, Revision};
    use crate::contracts::source_map::SourceMap;

    #[test]
    fn a_position_counts_characters_not_bytes() {
        // The first line holds two multi-byte characters before the newline.
        let text = "αβ\ngamma\n";
        assert_eq!(position_of(text, 0), Position { line: 1, column: 1 });
        assert_eq!(position_of(text, 2), Position { line: 1, column: 2 });
        // Byte 4 is the newline itself, which ends line 1; line 2 starts at byte 5.
        assert_eq!(position_of(text, 4), Position { line: 1, column: 3 });
        assert_eq!(position_of(text, 5), Position { line: 2, column: 1 });
        assert_eq!(offset_of(text, Position { line: 2, column: 1 }), Some(5));
        assert_eq!(offset_of(text, Position { line: 2, column: 3 }), Some(7));
        assert_eq!(offset_of(text, Position { line: 9, column: 1 }), None);
    }

    #[test]
    fn an_edit_moves_what_follows_and_deletes_what_it_covers() {
        let edit = Edit {
            start: 10,
            end: 12,
            inserted: 5,
        };
        assert_eq!(edit.delta(), 3);
        assert_eq!(edit.apply(4), Some(4));
        assert_eq!(
            edit.apply(11),
            None,
            "an offset inside the deleted range is gone"
        );
        assert_eq!(edit.apply(20), Some(23));
    }

    #[test]
    fn a_runaway_regeneration_is_stopped_at_the_limit() {
        let mut guard = LoopGuard::new(2);
        assert_eq!(guard.enter(), Ok(1));
        assert_eq!(guard.enter(), Ok(2));
        assert!(guard.enter().is_err());
        guard.leave();
        assert_eq!(guard.enter(), Ok(2));
        assert_eq!(guard.depth(), 2);
    }

    #[test]
    fn one_undo_crosses_every_view() {
        let mut journal = Journal::new();
        journal.record(Step {
            view: "tree".to_owned(),
            label: "add button".to_owned(),
            before: vec![],
            after: vec![NodeId::from_raw(2)],
        });
        journal.record(Step {
            view: "code".to_owned(),
            label: "rename".to_owned(),
            before: vec![NodeId::from_raw(2)],
            after: vec![NodeId::from_raw(2)],
        });
        assert_eq!(journal.undoable(), 2);
        assert_eq!(journal.undo().unwrap().view, "code");
        assert_eq!(journal.redo().unwrap().view, "code");
    }

    #[test]
    fn a_recovery_that_cannot_place_a_node_says_so() {
        let source = "let t = Node::new(\"window\").key(\"root\");\n\
                      let b = Node::new(\"button\").key(\"n_0\");\n";
        let keys = vec![
            (NodeId::from_raw(1), "root".to_owned()),
            (NodeId::from_raw(2), "n_0".to_owned()),
        ];
        let mut previous = SourceMap::new(GeneratorVersion::CURRENT, Revision::INITIAL);
        previous.insert(
            NodeId::from_raw(1),
            crate::contracts::source_map::SourceSpan::new("ui.rs", 0, 10).unwrap(),
        );
        previous.insert(
            NodeId::from_raw(2),
            crate::contracts::source_map::SourceSpan::new("ui.rs", 10, 20).unwrap(),
        );

        // The button was deleted by a hand edit: the rebuilt map can no longer place it.
        let edited = "let t = Node::new(\"window\").key(\"root\");\n";
        let (_, recovery) = recover(
            "ui.rs",
            edited,
            &previous,
            &keys,
            &[NodeId::from_raw(1), NodeId::from_raw(2)],
        );
        assert!(!recovery.is_complete());
        assert!(recovery.lost.contains(&NodeId::from_raw(2)));
        assert!(recovery.render().contains("lost 1"));

        // With the button present, nothing is lost.
        let (_, recovery) = recover(
            "ui.rs",
            source,
            &previous,
            &keys,
            &[NodeId::from_raw(1), NodeId::from_raw(2)],
        );
        assert!(recovery.is_complete());
    }
}
