// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Preserving the live UI state across a rebuild: input, focus, scroll and selection — but only onto
//! a node that is still the same node *and* the same kind (task T14.03).
//!
//! The acceptance line is *"only a NodeId- and type-compatible instance restores, and the old
//! instance's state is not applied"*. The rule is therefore a match on two things, never on one:
//!
//! * **A capture is keyed by node and kind.** [`Kept`] records both, so restoring can compare them
//!   instead of trusting the id alone.
//! * **A mismatch is a notice, not a restore.** [`Restore::Incompatible`] names what it was and what
//!   it is now, and no state is applied — so a kind change cannot carry an old scroll or an old input
//!   into a widget that means something else.
//! * **A gone node is a notice.** [`Restore::Gone`] says the node is no longer there, so a rebuild
//!   that dropped it is visible rather than silently losing the state.
//! * **What was not captured is left alone.** A present node with no capture is not in the answer at
//!   all, so restoring never invents state for a widget nobody touched.

#![allow(dead_code)]

use std::collections::BTreeMap;

use rust_widgets::core::ObjectId;
use rust_widgets::view::TreeIndex;
use rust_widgets::widget::capability::CapabilityValue;

use crate::contracts::identity::NodeId;

/// The scroll half of this model uses the library's own row-scroll channel.
///
/// RW-58 gave 31 row containers the `ScrollableRows` trait and a `scroll_row` property, and
/// `WidgetHandle::scroll_row`/`set_scroll_row` read and write it through that property. This module
/// uses the same property by id, so there is no second notion of "the first visible row".
///
/// What stays outside the channel is the **pixel** scroll pair (`ScrollArea`, `MdiArea`), which keeps
/// `ScrollableSurface`. A node that is one of those does not answer `scroll_row`, so it contributes
/// no scroll state rather than a guessed row. This constant is that remaining boundary, reported by
/// the preview instead of pretending the pixel offset was restored.
pub const SCROLL_GAP: &str = "rust_widgets exposes row scroll generically (`ScrollableRows` + the \
     `scroll_row` property on 31 containers), so this model captures and restores it by id. Pixel \
     containers (`ScrollArea`, `MdiArea`) keep `ScrollableSurface` and do not answer `scroll_row`; \
     they contribute no scroll state rather than a converted row index.";

/// Reads a row container's first visible row through the library's `scroll_row` property.
///
/// `None` covers both "not a row container" (the property does not exist) and "the control refused
/// the read"; row `0` is a real answer and stays distinct.
fn scroll_row_of(id: ObjectId) -> Option<u32> {
    match rust_widgets::widget::capability::access::read_widget_property_by_id(id, "scroll_row") {
        Ok(CapabilityValue::UInt(row)) => u32::try_from(row).ok(),
        _ => None,
    }
}

/// Moves a row container to `row` through the same property the control publishes.
///
/// The control clamps, so a row past the end lands at the last row rather than being rejected here.
pub fn set_scroll_row(id: ObjectId, row: u32) -> bool {
    rust_widgets::widget::capability::access::write_widget_property_by_id(
        id,
        "scroll_row",
        CapabilityValue::UInt(u64::from(row)),
    )
    .is_ok()
}

/// Reads a control's current text through the library's own accessor.
///
/// A control that publishes no text answers `None` rather than an empty string, so "no text channel"
/// and "a field the person emptied" stay different facts.
fn text_of(id: ObjectId) -> Option<String> {
    match rust_widgets::widget::capability::access::read_widget_property_by_id(id, "text") {
        Ok(CapabilityValue::String(text)) => Some(text),
        _ => None,
    }
}

/// Writes a control's text through the same property.
pub fn set_text(id: ObjectId, text: &str) -> bool {
    rust_widgets::widget::capability::access::write_widget_property_by_id(
        id,
        "text",
        CapabilityValue::String(text.to_owned()),
    )
    .is_ok()
}

/// The live state kept for one node.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Kept {
    /// The node.
    pub node: NodeId,
    /// The kind it was at capture time.
    pub kind: String,
    /// Whether it held focus.
    pub focus: bool,
    /// What was typed into it.
    pub input: String,
    /// Where it was scrolled to.
    pub scroll: u32,
    /// The selection, as a start and an end.
    pub selection: Option<(usize, usize)>,
}

/// Everything captured before the rebuild.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Capture {
    /// The captured nodes.
    pub entries: Vec<Kept>,
}

impl Capture {
    /// An empty capture.
    pub fn new() -> Self {
        Self::default()
    }

    /// The same capture with one more node.
    pub fn with(mut self, kept: Kept) -> Self {
        self.entries.push(kept);
        self
    }

    /// The capture for a node, if it was captured.
    pub fn get(&self, node: NodeId) -> Option<&Kept> {
        self.entries.iter().find(|kept| kept.node == node)
    }

    /// The capture in one line per node.
    pub fn render(&self) -> Vec<String> {
        self.entries
            .iter()
            .map(|kept| {
                format!(
                    "node {} ({}) focus={} scroll={} input={:?}",
                    kept.node.get(),
                    kept.kind,
                    kept.focus,
                    kept.scroll,
                    kept.input
                )
            })
            .collect()
    }
}

/// A node present after the rebuild.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Present {
    /// The node.
    pub node: NodeId,
    /// The kind it is now.
    pub kind: String,
}

impl Present {
    /// A present node of a kind.
    pub fn new(node: NodeId, kind: impl Into<String>) -> Self {
        Self {
            node,
            kind: kind.into(),
        }
    }
}

/// What became of one captured node.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Restore {
    /// The node and its kind match, so the state is applied.
    Restored(Kept),
    /// The node is gone.
    Gone {
        /// The node that is gone.
        node: NodeId,
    },
    /// The node's kind changed, so the old state is not applied.
    Incompatible {
        /// The node.
        node: NodeId,
        /// The kind it was.
        was: String,
        /// The kind it is now.
        now: String,
    },
}

impl Restore {
    /// Whether a restore happened.
    pub fn is_restored(&self) -> bool {
        matches!(self, Self::Restored(_))
    }

    /// The restore in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Restored(kept) => format!(
                "node {}: restored (scroll {}, input {:?})",
                kept.node.get(),
                kept.scroll,
                kept.input
            ),
            Self::Gone { node } => format!("node {}: gone", node.get()),
            Self::Incompatible { node, was, now } => format!(
                "node {}: incompatible — was {was}, is now {now}; the old state is not applied",
                node.get()
            ),
        }
    }
}

/// One planned focus restore: what happened and, when it may be restored, the live control.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FocusRestore {
    /// What became of the captured node.
    pub outcome: Restore,
    /// The live control to focus, only for a [`Restore::Restored`] outcome.
    pub target: Option<ObjectId>,
}

impl FocusRestore {
    /// Whether the focus restore happened.
    pub fn is_restored(&self) -> bool {
        self.outcome.is_restored()
    }

    /// The restore in one line, naming focus only.
    ///
    /// The scroll field is never restored (see [`SCROLL_GAP`]), so this renderer does not repeat the
    /// capture's default scroll value as though it had been applied.
    pub fn render(&self) -> String {
        match &self.outcome {
            Restore::Restored(kept) if kept.focus => {
                format!("node {}: focus restored", kept.node.get())
            }
            other => other.render(),
        }
    }
}

/// Captures every node's live scroll and the focused node, keyed by the document's `NodeId`.
///
/// The node convention is the same one [`crate::app::preview::preview_service::Document`] uses: a
/// node's key is the decimal spelling of its `NodeId`, and a key that is not a number contributes
/// nothing rather than inventing an entry. A node whose control does not answer `scroll_row` (a
/// label, or a pixel container) keeps scroll `0`, which is also the state it was in.
pub fn capture_state(index: &TreeIndex, focused: Option<ObjectId>) -> Capture {
    capture_with_declarations(index, focused, &BTreeMap::new())
}

/// Captures scroll, focus **and the text the person typed**, given what the declaration says.
///
/// Input is recorded only where the live text differs from the text the declaration gives that
/// node. That is the one honest marker available: a control showing what its declaration says holds
/// no person's edit, so a rebuild must not "restore" it (and must not freeze a value the document
/// just changed). A node whose declaration says nothing about text is skipped rather than guessed at
/// — a control with no text channel is not an input, and the library publishes no "this text is
/// user input" flag for the host to ask.
pub fn capture_with_declarations(
    index: &TreeIndex,
    focused: Option<ObjectId>,
    declared: &BTreeMap<NodeId, String>,
) -> Capture {
    let mut capture = Capture::new();
    for (_, key, id) in index.keys() {
        let Some(node) = key.parse::<u64>().ok().map(NodeId::from_raw) else {
            continue;
        };
        let Some(kind) = index.widget_of(id) else {
            continue;
        };
        let input = match declared.get(&node) {
            Some(declared) => match text_of(id) {
                Some(live) if live != *declared => live,
                _ => String::new(),
            },
            None => String::new(),
        };
        capture.entries.push(Kept {
            node,
            kind: kind.to_owned(),
            focus: focused == Some(id),
            input,
            scroll: scroll_row_of(id).unwrap_or(0),
            selection: None,
        });
    }
    capture
}

/// Captures the focused control from a mounted view, keyed by the document's `NodeId`.
///
/// Kept for callers that only need the focus half; [`capture_state`] also records scroll for every
/// row container. A focused control that the caller cannot map to a node (the root, an anonymous
/// control, or a key that is not a number) contributes nothing, rather than inventing an entry.
pub fn capture_focus(index: &TreeIndex, focused: Option<ObjectId>) -> Capture {
    let Some(id) = focused else {
        return Capture::new();
    };
    let node = index
        .key_of(id)
        .and_then(|key| key.parse::<u64>().ok())
        .map(NodeId::from_raw);
    let (Some(node), Some(kind)) = (node, index.widget_of(id)) else {
        return Capture::new();
    };
    Capture::new().with(Kept {
        node,
        kind: kind.to_owned(),
        focus: true,
        input: String::new(),
        scroll: 0,
        selection: None,
    })
}

/// Plans which focus restores are compatible with the tree currently mounted.
///
/// The decision is the ordinary [`restore`] one — node and kind must both match — and the answer is
/// annotated with the live control to focus so a caller can carry it out without repeating the key
/// lookup. Only entries that were focused are planned: [`capture_state`] records every node for its
/// scroll, and focusing all of them would leave the last one holding the keyboard. Scroll is planned
/// by [`restore_scroll`].
pub fn restore_focus(capture: &Capture, index: &TreeIndex) -> Vec<FocusRestore> {
    let (present, by_key) = present_and_targets(index);
    let focused = Capture {
        entries: capture
            .entries
            .iter()
            .filter(|kept| kept.focus)
            .cloned()
            .collect(),
    };
    restore(&focused, &present)
        .into_iter()
        .map(|outcome| {
            let target = match &outcome {
                Restore::Restored(kept) => by_key.get(&kept.node.get().to_string()).copied(),
                Restore::Gone { .. } | Restore::Incompatible { .. } => None,
            };
            FocusRestore { outcome, target }
        })
        .collect()
}

/// One captured node's scroll, with the live control to move.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ScrollRestore {
    /// The ordinary node-and-kind decision.
    pub outcome: Restore,
    /// The control to move, when the node is present at the same kind.
    pub target: Option<ObjectId>,
    /// The row to move it to (only meaningful for a restored outcome).
    pub row: u32,
}

impl ScrollRestore {
    /// Whether the state may be applied.
    pub fn is_restored(&self) -> bool {
        matches!(self.outcome, Restore::Restored(_))
    }

    /// One line naming the node, the row and the decision.
    pub fn render(&self) -> String {
        format!("{} (scroll row {})", self.outcome.render(), self.row)
    }
}

/// Plans which scroll restores are compatible with the tree currently mounted.
///
/// Only a capture with a non-zero row is planned: restoring row `0` would scroll a control the person
/// never moved, and a node that is gone or retyped is reported and left alone exactly as in
/// [`restore`]. A pixel container never enters the plan because [`capture_state`] records no row for
/// it (see [`SCROLL_GAP`]).
pub fn restore_scroll(capture: &Capture, index: &TreeIndex) -> Vec<ScrollRestore> {
    let (present, by_key) = present_and_targets(index);
    let scrolled = Capture {
        entries: capture
            .entries
            .iter()
            .filter(|kept| kept.scroll > 0)
            .cloned()
            .collect(),
    };
    restore(&scrolled, &present)
        .into_iter()
        .map(|outcome| {
            let (target, row) = match &outcome {
                Restore::Restored(kept) => (
                    by_key.get(&kept.node.get().to_string()).copied(),
                    kept.scroll,
                ),
                Restore::Gone { .. } | Restore::Incompatible { .. } => (None, 0),
            };
            ScrollRestore {
                outcome,
                target,
                row,
            }
        })
        .collect()
}

/// One captured node's typed text, with the live control to write it into.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct InputRestore {
    /// The ordinary node-and-kind decision.
    pub outcome: Restore,
    /// The control to write into, when the node is present at the same kind.
    pub target: Option<ObjectId>,
    /// The text the person typed (only meaningful for a restored outcome).
    pub input: String,
}

impl InputRestore {
    /// Whether the state may be applied.
    pub fn is_restored(&self) -> bool {
        matches!(self.outcome, Restore::Restored(_))
    }

    /// One line naming the node, the text and the decision.
    pub fn render(&self) -> String {
        format!("{} (input {:?})", self.outcome.render(), self.input)
    }
}

/// Plans which typed-text restores are compatible with the tree currently mounted.
///
/// Only a capture with input is planned: a node whose live text matched its declaration holds no
/// person's edit, and a node that is gone or retyped is reported and left alone exactly as in
/// [`restore`]. Selection is not part of this plan — the pinned library publishes no generic
/// selection read/write pair, so nothing is approximated.
pub fn restore_input(capture: &Capture, index: &TreeIndex) -> Vec<InputRestore> {
    let (present, by_key) = present_and_targets(index);
    let typed = Capture {
        entries: capture
            .entries
            .iter()
            .filter(|kept| !kept.input.is_empty())
            .cloned()
            .collect(),
    };
    restore(&typed, &present)
        .into_iter()
        .map(|outcome| {
            let (target, input) = match &outcome {
                Restore::Restored(kept) => (
                    by_key.get(&kept.node.get().to_string()).copied(),
                    kept.input.clone(),
                ),
                Restore::Gone { .. } | Restore::Incompatible { .. } => (None, String::new()),
            };
            InputRestore {
                outcome,
                target,
                input,
            }
        })
        .collect()
}

/// One pass over the tree's keys builds both the present list and the key→control map the target
/// lookups need.
fn present_and_targets(
    index: &TreeIndex,
) -> (Vec<Present>, std::collections::HashMap<String, ObjectId>) {
    let keys = index.keys();
    let mut by_key = std::collections::HashMap::with_capacity(keys.len());
    let mut present = Vec::with_capacity(keys.len());
    for (_, key, id) in keys {
        let Some(node) = key.parse::<u64>().ok().map(NodeId::from_raw) else {
            continue;
        };
        by_key.insert(key.to_owned(), id);
        present.push(Present::new(node, index.widget_of(id).unwrap_or_default()));
    }
    (present, by_key)
}

/// Decides, per captured node, whether its state may be restored.
pub fn restore(capture: &Capture, present: &[Present]) -> Vec<Restore> {
    capture
        .entries
        .iter()
        .map(
            |kept| match present.iter().find(|node| node.node == kept.node) {
                None => Restore::Gone { node: kept.node },
                Some(node) if node.kind == kept.kind => Restore::Restored(kept.clone()),
                Some(node) => Restore::Incompatible {
                    node: kept.node,
                    was: kept.kind.clone(),
                    now: node.kind.clone(),
                },
            },
        )
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Capture, Kept, Present, Restore, capture_state, restore, restore_scroll};
    use crate::contracts::identity::NodeId;
    use rust_widgets::core::ObjectId;
    use rust_widgets::view::{TreeIndex, TreeIndexBuilder};

    fn node(value: u64) -> NodeId {
        NodeId::from_raw(value)
    }

    /// A root and one keyed child (key `"1"` → id `3`), the shape the preview mounts.
    fn index() -> TreeIndex {
        TreeIndexBuilder::<ObjectId>::new()
            .node(&[], 1, "group_box", None)
            .node(&[0], 3, "list_box", Some("1"))
            .build()
    }

    fn kept(value: u64, kind: &str) -> Kept {
        Kept {
            node: node(value),
            kind: kind.to_owned(),
            focus: true,
            input: "typed".to_owned(),
            scroll: 40,
            selection: Some((1, 4)),
        }
    }

    #[test]
    fn the_capture_records_every_node_and_only_the_focused_one_as_focused() {
        let capture = capture_state(&index(), Some(3));
        let child = capture.get(node(1)).expect("the keyed node is captured");
        assert_eq!(child.kind, "list_box");
        assert!(child.focus);
        // The id is not mounted here, so no control answers `scroll_row`.
        assert_eq!(child.scroll, 0);
        assert!(
            capture.get(node(3)).is_none(),
            "the key is a node id, not the control id"
        );
        let unfocused = capture_state(&index(), None);
        assert!(!unfocused.get(node(1)).expect("captured").focus);
    }

    #[test]
    fn only_a_matching_node_with_a_row_is_planned_for_scroll() {
        let index = index();
        let capture = Capture::new().with(kept(1, "list_box"));
        let plan = restore_scroll(&capture, &index);
        assert_eq!(plan.len(), 1);
        assert!(plan[0].is_restored());
        assert_eq!(plan[0].target, Some(3));
        assert_eq!(plan[0].row, 40);
        assert!(plan[0].render().contains("scroll row 40"));

        // A capture at row 0 is not a restore: it would move a control nobody touched.
        let mut untouched = Capture::new().with(kept(1, "list_box"));
        untouched.entries[0].scroll = 0;
        assert!(restore_scroll(&untouched, &index).is_empty());

        // A kind change is reported and nothing is applied.
        let retyped = Capture::new().with(kept(1, "group_box"));
        let changed = restore_scroll(&retyped, &index);
        assert_eq!(changed.len(), 1);
        assert!(!changed[0].is_restored());
        assert_eq!(changed[0].row, 0);
    }

    #[test]
    fn only_a_matching_node_with_typed_text_is_planned_for_input() {
        let index = index();
        let capture = Capture::new().with(kept(1, "list_box"));
        let plan = super::restore_input(&capture, &index);
        assert_eq!(plan.len(), 1);
        assert!(plan[0].is_restored());
        assert_eq!(plan[0].target, Some(3));
        assert_eq!(plan[0].input, "typed");
        assert!(plan[0].render().contains("input \"typed\""));

        // A capture with no input is not a restore: the control already shows its declaration.
        let mut untouched = Capture::new().with(kept(1, "list_box"));
        untouched.entries[0].input.clear();
        assert!(super::restore_input(&untouched, &index).is_empty());

        // A kind change is reported and nothing is applied.
        let retyped = Capture::new().with(kept(1, "group_box"));
        let changed = super::restore_input(&retyped, &index);
        assert_eq!(changed.len(), 1);
        assert!(!changed[0].is_restored());
        assert!(changed[0].input.is_empty());
    }

    #[test]
    fn focus_restores_only_plan_the_focused_entry() {
        let index = index();
        let mut capture = Capture::new().with(kept(1, "list_box"));
        capture.entries[0].focus = false;
        assert!(super::restore_focus(&capture, &index).is_empty());
        capture.entries[0].focus = true;
        let plan = super::restore_focus(&capture, &index);
        assert_eq!(plan.len(), 1);
        assert!(plan[0].is_restored());
        assert_eq!(plan[0].target, Some(3));
    }

    #[test]
    fn a_matching_node_and_kind_restores() {
        let capture = Capture::new().with(kept(1, "text-field"));
        let restores = restore(&capture, &[Present::new(node(1), "text-field")]);
        assert_eq!(restores.len(), 1);
        assert!(restores[0].is_restored());
        assert!(restores[0].render().contains("scroll 40"));
    }

    #[test]
    fn a_node_that_is_gone_is_reported() {
        let capture = Capture::new().with(kept(1, "text-field"));
        let restores = restore(&capture, &[]);
        assert_eq!(restores, vec![Restore::Gone { node: node(1) }]);
        assert!(restores[0].render().contains("gone"));
    }

    #[test]
    fn a_kind_change_is_incompatible_and_applies_nothing() {
        let capture = Capture::new().with(kept(1, "text-field"));
        let restores = restore(&capture, &[Present::new(node(1), "button")]);
        assert_eq!(
            restores,
            vec![Restore::Incompatible {
                node: node(1),
                was: "text-field".to_owned(),
                now: "button".to_owned(),
            }]
        );
        assert!(!restores[0].is_restored());
        assert!(restores[0].render().contains("not applied"));
    }

    #[test]
    fn an_empty_capture_restores_nothing() {
        assert!(restore(&Capture::new(), &[Present::new(node(1), "button")]).is_empty());
    }

    #[test]
    fn a_node_nobody_captured_is_not_in_the_answer() {
        let capture = Capture::new().with(kept(1, "text-field"));
        let restores = restore(
            &capture,
            &[
                Present::new(node(1), "text-field"),
                Present::new(node(2), "button"),
            ],
        );
        assert_eq!(restores.len(), 1);
    }

    #[test]
    fn focus_input_scroll_and_selection_are_carried() {
        let capture = Capture::new().with(kept(1, "text-field"));
        let Restore::Restored(state) =
            &restore(&capture, &[Present::new(node(1), "text-field")])[0]
        else {
            panic!("a restored state");
        };
        assert!(state.focus);
        assert_eq!(state.input, "typed");
        assert_eq!(state.scroll, 40);
        assert_eq!(state.selection, Some((1, 4)));
    }

    #[test]
    fn two_nodes_restore_independently() {
        let capture = Capture::new()
            .with(kept(1, "text-field"))
            .with(kept(2, "list"));
        let restores = restore(
            &capture,
            &[
                Present::new(node(2), "list"),
                Present::new(node(1), "switch"),
            ],
        );
        assert!(matches!(restores[0], Restore::Incompatible { .. }));
        assert!(restores[1].is_restored());
    }

    #[test]
    fn a_capture_can_be_looked_up_by_node() {
        let capture = Capture::new().with(kept(1, "text-field"));
        assert_eq!(
            capture.get(node(1)).map(|kept| kept.kind.as_str()),
            Some("text-field")
        );
        assert!(capture.get(node(9)).is_none());
    }

    #[test]
    fn the_capture_renders_what_it_holds() {
        let lines = Capture::new().with(kept(1, "text-field")).render();
        assert_eq!(
            lines,
            vec!["node 1 (text-field) focus=true scroll=40 input=\"typed\"".to_owned()]
        );
    }

    #[test]
    fn the_restores_render_each_outcome_differently() {
        let capture = Capture::new()
            .with(kept(1, "a"))
            .with(kept(2, "b"))
            .with(kept(3, "c"));
        let restores = restore(
            &capture,
            &[Present::new(node(1), "a"), Present::new(node(3), "z")],
        );
        assert!(restores[0].render().starts_with("node 1: restored"));
        assert_eq!(restores[1].render(), "node 2: gone");
        assert!(restores[2].render().contains("was c, is now z"));
    }
}
