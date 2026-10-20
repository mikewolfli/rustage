// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The Inspector's editable property (tasks T11.01/T11.02).
//!
//! The schema editor in [`crate::app::style::property_editor`] already separates the valid value
//! from the text being typed. This type gives it a selection of workbench nodes and a revision, so
//! a valid edit advances the revision exactly once and a refused edit advances it zero times.
//!
//! A multi-selection follows the one rule the multi-property model owns: the shared value is the
//! agreement of the selection or [`Common::Mixed`], never one node's value standing for the rest.
//! An edit is validated once and then applied to every selected node, so a refusal cannot leave
//! half the selection changed. Reset takes the explicit edit away and restores the structural name
//! each node had when the Inspector first saw it.

use std::collections::{BTreeMap, BTreeSet};

use crate::app::layout::layout_plan::Origin;
use crate::app::style::multi_property::{
    Common, NodeProperties, common, reset as reset_setting, value_of,
};
use crate::app::style::property_editor::{Editor, PropertyType, Schema};
use crate::contracts::identity::NodeId;
use crate::contracts::model::{PropertyName, PropertyValue};

/// The one property the workbench's demo Inspector edits.
pub(super) const TITLE: &str = "title";

/// The Inspector's editable property and the model revision it has reached.
pub(super) struct PropertyState {
    editor: Editor,
    nodes: Vec<NodeId>,
    name: PropertyName,
    /// The structural name each node had when the Inspector first saw it, which is what reset
    /// restores.
    baseline: BTreeMap<NodeId, String>,
    /// The nodes whose title has been explicitly edited.
    edited: BTreeSet<NodeId>,
    revision: u64,
}

impl PropertyState {
    /// An editor over one node's `title`, seeded with `initial`.
    ///
    /// The seed goes through the same [`Editor::set`] path the user's text does, so the initial
    /// value cannot be a value the schema would refuse.
    pub(super) fn new(initial: &str) -> Self {
        let name = PropertyName::new(TITLE).expect("`title` is a property name");
        let schema = Schema::new().with(name.clone(), PropertyType::Text { max_len: 40 });
        let mut editor = Editor::new(schema);
        let node = NodeId::from_raw(1);
        if let Err(reason) = editor.set(node, name.clone(), initial) {
            // A hard-coded seed that the schema refuses is a programming error, not user input;
            // keep the editor usable and make the reason visible on first commit.
            crate::host::log::emit(
                crate::host::log::Level::Warn,
                "app::run::workbench-property",
                &format!("the Inspector's seed was refused: {reason}"),
            );
        }
        let mut baseline = BTreeMap::new();
        baseline.insert(node, initial.to_owned());
        Self {
            editor,
            nodes: vec![node],
            name,
            baseline,
            edited: BTreeSet::new(),
            revision: 0,
        }
    }

    /// The node the property belongs to.
    pub(super) fn node(&self) -> NodeId {
        self.nodes
            .first()
            .copied()
            .unwrap_or_else(|| NodeId::from_raw(0))
    }

    /// How many nodes the Inspector is editing.
    pub(super) fn count(&self) -> usize {
        self.nodes.len()
    }

    /// The selected nodes, in selection order.
    pub(super) fn nodes(&self) -> &[NodeId] {
        &self.nodes
    }

    /// Points the Inspector at another single node, seeding its `title` from the tree's name.
    ///
    /// A node the Inspector has already edited keeps its accepted value, so switching the
    /// selection back and forth does not silently discard a person's work.
    pub(super) fn retarget(&mut self, node: NodeId, initial: &str) {
        self.retarget_many(&[(node, initial)]);
    }

    /// Points the Inspector at a selection, seeding each node it has not seen before.
    pub(super) fn retarget_many(&mut self, nodes: &[(NodeId, &str)]) {
        let mut selection = Vec::new();
        for (node, initial) in nodes {
            self.baseline
                .entry(*node)
                .or_insert_with(|| (*initial).to_owned());
            if self.editor.value(*node, &self.name).is_none()
                && let Err(reason) = self.editor.set(*node, self.name.clone(), initial)
            {
                crate::host::log::emit(
                    crate::host::log::Level::Warn,
                    "app::run::workbench-property",
                    &format!("the Inspector could not seed node {}: {reason}", node.get()),
                );
            }
            if !selection.contains(node) {
                selection.push(*node);
            }
        }
        self.nodes = selection;
    }

    /// The property name.
    pub(super) fn name(&self) -> &PropertyName {
        &self.name
    }

    /// The value the field shows: the agreed value, or blank when the selection is mixed.
    pub(super) fn value_text(&self) -> String {
        match self.common() {
            Common::Agreed(value) => render_value(&value),
            Common::Mixed => String::new(),
        }
    }

    /// One node's accepted value, for a caller applying the same edit to the tree.
    pub(super) fn value_for(&self, node: NodeId) -> Option<String> {
        self.editor.value(node, &self.name).map(render_value)
    }

    /// Whether the selection agrees on the value.
    pub(super) fn is_mixed(&self) -> bool {
        matches!(self.common(), Common::Mixed)
    }

    /// The selection's answer in one line, for the Inspector's rows.
    pub(super) fn summary(&self) -> String {
        match self.common() {
            Common::Agreed(value) => format!("agreed: {}", render_value(&value)),
            Common::Mixed => {
                let mut parts = Vec::new();
                for node in &self.nodes {
                    let text = self
                        .editor
                        .value(*node, &self.name)
                        .map(render_value)
                        .unwrap_or_else(|| "(unset)".to_owned());
                    parts.push(format!("{}={text}", node.get()));
                }
                format!("mixed ({}): {}", self.nodes.len(), parts.join(", "))
            }
        }
    }

    /// Why the last edit was refused, while the valid value stayed where it was.
    pub(super) fn problem(&self) -> Option<&str> {
        self.nodes
            .first()
            .and_then(|node| self.editor.problem(*node, &self.name))
    }

    /// The model revision: one per accepted edit or reset.
    pub(super) fn revision(&self) -> u64 {
        self.revision
    }

    /// Types `text` into the field.
    ///
    /// On success the valid value is replaced for every selected node and the revision advances by
    /// one. The text is validated once before any node is written, so a refusal cannot leave a
    /// partly edited selection: the editor keeps the text as pending, the reason is returned, and
    /// both the values and the revision are unchanged.
    pub(super) fn apply(&mut self, text: &str) -> Result<u64, String> {
        let Some(kind) = self.editor.schema().kind_of(&self.name).cloned() else {
            return Err(format!(
                "the schema does not describe {:?}",
                self.name.as_str()
            ));
        };
        if self.nodes.is_empty() {
            return Err("nothing is selected".to_owned());
        }
        if let Err(reason) = kind.parse(text) {
            // Record the refused text on the primary node so the panel can show what is pending
            // beside the reason; a refusal changes no valid value and no revision.
            let _ = self.editor.set(self.nodes[0], self.name.clone(), text);
            return Err(reason);
        }
        for node in &self.nodes {
            self.editor.set(*node, self.name.clone(), text)?;
            self.edited.insert(*node);
        }
        self.revision += 1;
        Ok(self.revision)
    }

    /// Takes the explicit edits away and restores each node's structural name.
    ///
    /// This uses the shared [`reset_setting`] answer for the origin that is left, so the panel can
    /// say what it reverted to rather than only that it cleared something.
    pub(super) fn reset(&mut self) -> (usize, Vec<String>) {
        let mut count = 0;
        let mut lines = Vec::new();
        for node in &self.nodes {
            if !self.edited.contains(node) {
                let outcome = reset_setting(Origin::Default, None);
                lines.push(format!("node {}: {}", node.get(), outcome.render()));
                continue;
            }
            let Some(baseline) = self.baseline.get(node).cloned() else {
                continue;
            };
            match self.editor.set(*node, self.name.clone(), &baseline) {
                Ok(()) => {
                    self.edited.remove(node);
                    count += 1;
                    let value = self.editor.value(*node, &self.name).cloned();
                    let outcome = reset_setting(Origin::Explicit, value.as_ref());
                    lines.push(format!("node {}: {}", node.get(), outcome.render()));
                }
                Err(reason) => {
                    lines.push(format!("node {}: not reset — {reason}", node.get()));
                }
            }
        }
        if count > 0 {
            self.revision += 1;
        }
        (count, lines)
    }

    /// The shared-property answer built through the one multi-property model.
    fn common(&self) -> Common {
        let nodes: Vec<NodeProperties> = self
            .nodes
            .iter()
            .map(|node| {
                let value = self
                    .editor
                    .value(*node, &self.name)
                    .cloned()
                    .unwrap_or(PropertyValue::Text(String::new()));
                let origin = if self.edited.contains(node) {
                    Origin::Explicit
                } else {
                    Origin::Default
                };
                NodeProperties::new(*node).with(self.name.clone(), value, origin)
            })
            .collect();
        if nodes.is_empty() {
            return Common::Mixed;
        }
        // The intersection is answered by the shared model; the Inspector shows the one property
        // every selected node can hold, so the name is always common.
        let shared = common(&nodes);
        if !shared.contains(&self.name) {
            return Common::Mixed;
        }
        value_of(&nodes, &self.name)
    }
}

/// How a value reads in a single-line field.
fn render_value(value: &PropertyValue) -> String {
    match value {
        PropertyValue::Bool(value) => value.to_string(),
        PropertyValue::Integer(value) => value.to_string(),
        PropertyValue::Number(value) => value.to_string(),
        PropertyValue::Text(value)
        | PropertyValue::Variant(value)
        | PropertyValue::Color(value)
        | PropertyValue::Asset(value) => value.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::PropertyState;
    use crate::contracts::identity::NodeId;

    #[test]
    fn a_valid_edit_replaces_the_value_and_advances_the_revision_once() {
        let mut state = PropertyState::new("Demo");
        assert_eq!(state.value_text(), "Demo");
        assert_eq!(state.revision(), 0);

        assert_eq!(state.apply("Hello").expect("valid text"), 1);
        assert_eq!(state.value_text(), "Hello");
        assert_eq!(state.problem(), None);
        assert_eq!(state.revision(), 1);
    }

    #[test]
    fn retargeting_keeps_each_node_s_accepted_value() {
        let mut state = PropertyState::new("Demo");
        let second = NodeId::from_raw(2);
        assert_eq!(state.apply("First").expect("valid"), 1);
        state.retarget(second, "Second");
        assert_eq!(state.node(), second);
        assert_eq!(state.value_text(), "Second");
        assert_eq!(state.apply("Renamed").expect("valid"), 2);
        state.retarget(NodeId::from_raw(1), "ignored");
        assert_eq!(state.value_text(), "First");
        assert_eq!(state.revision(), 2);
    }

    #[test]
    fn a_refused_edit_keeps_the_text_and_leaves_the_value_and_revision_alone() {
        let mut state = PropertyState::new("Demo");
        let overlong = "x".repeat(41);
        let reason = state.apply(&overlong).expect_err("over the schema limit");
        assert!(reason.contains("40"), "{reason}");
        assert_eq!(state.value_text(), "Demo");
        assert_eq!(state.revision(), 0);
        assert_eq!(state.problem(), Some(reason.as_str()));

        // The refused text stays pending, which is what the field shows beside the reason.
        assert_eq!(
            state
                .editor
                .pending(state.node(), &state.name)
                .map(str::to_owned),
            Some(overlong)
        );
    }

    #[test]
    fn a_multi_selection_shows_agreement_or_mixed_never_one_node_s_value() {
        let mut state = PropertyState::new("Demo");
        state.retarget_many(&[
            (NodeId::from_raw(1), "Demo"),
            (NodeId::from_raw(2), "button"),
        ]);
        assert_eq!(state.count(), 2);
        assert!(state.is_mixed());
        assert_eq!(state.value_text(), "");
        assert!(state.summary().contains("mixed (2)"), "{}", state.summary());
    }

    #[test]
    fn a_multi_selection_edit_is_validated_once_and_applied_to_every_node() {
        let mut state = PropertyState::new("Demo");
        state.retarget_many(&[
            (NodeId::from_raw(1), "Demo"),
            (NodeId::from_raw(2), "button"),
        ]);
        let overlong = "x".repeat(41);
        assert!(state.apply(&overlong).is_err());
        assert_eq!(state.revision(), 0);
        // Neither node moved, so the refusal was not a partial write.
        state.retarget(NodeId::from_raw(1), "Demo");
        assert_eq!(state.value_text(), "Demo");
        state.retarget(NodeId::from_raw(2), "button");
        assert_eq!(state.value_text(), "button");

        state.retarget_many(&[
            (NodeId::from_raw(1), "Demo"),
            (NodeId::from_raw(2), "button"),
        ]);
        assert_eq!(state.apply("Shared").expect("valid"), 1);
        assert!(!state.is_mixed());
        assert_eq!(state.value_text(), "Shared");
    }

    #[test]
    fn reset_restores_the_structural_name_and_reports_the_source() {
        let mut state = PropertyState::new("Demo");
        let second = NodeId::from_raw(2);
        state.retarget_many(&[(NodeId::from_raw(1), "Demo"), (second, "button")]);
        state.apply("Shared").expect("valid");
        let (count, lines) = state.reset();
        assert_eq!(count, 2);
        assert!(
            lines.iter().all(|line| line.contains("reverted to")),
            "{lines:?}"
        );
        assert_eq!(state.revision(), 2);
        state.retarget(NodeId::from_raw(1), "ignored");
        assert_eq!(state.value_text(), "Demo");
        state.retarget(second, "ignored");
        assert_eq!(state.value_text(), "button");
    }
}
