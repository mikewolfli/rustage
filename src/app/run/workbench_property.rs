// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The Inspector's editable property (task T11.01).
//!
//! The schema editor in [`crate::app::style::property_editor`] already separates the valid value
//! from the text being typed. This type gives it a workbench node and a revision, so a valid edit
//! advances the revision exactly once and a refused edit advances it zero times. The window only
//! mirrors [`PropertyState::value_text`] and [`PropertyState::problem`] into real controls.

use crate::app::style::property_editor::{Editor, PropertyType, Schema};
use crate::contracts::identity::NodeId;
use crate::contracts::model::{PropertyName, PropertyValue};

/// The one property the workbench's demo Inspector edits.
pub(super) const TITLE: &str = "title";

/// The Inspector's editable property and the model revision it has reached.
pub(super) struct PropertyState {
    editor: Editor,
    node: NodeId,
    name: PropertyName,
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
        Self {
            editor,
            node,
            name,
            revision: 0,
        }
    }

    /// The node the property belongs to.
    pub(super) fn node(&self) -> NodeId {
        self.node
    }

    /// Points the Inspector at another node, seeding its `title` from the tree's current name.
    ///
    /// A node the Inspector has already edited keeps its accepted value, so switching the
    /// selection back and forth does not silently discard a person's work.
    pub(super) fn retarget(&mut self, node: NodeId, initial: &str) {
        self.node = node;
        if self.editor.value(node, &self.name).is_none()
            && let Err(reason) = self.editor.set(node, self.name.clone(), initial)
        {
            crate::host::log::emit(
                crate::host::log::Level::Warn,
                "app::run::workbench-property",
                &format!("the Inspector could not seed node {}: {reason}", node.get()),
            );
        }
    }

    /// The property name.
    pub(super) fn name(&self) -> &PropertyName {
        &self.name
    }

    /// The last accepted value, as the field shows it.
    pub(super) fn value_text(&self) -> String {
        self.editor
            .value(self.node, &self.name)
            .map(render_value)
            .unwrap_or_default()
    }

    /// Why the last edit was refused, while the valid value stayed where it was.
    pub(super) fn problem(&self) -> Option<&str> {
        self.editor.problem(self.node, &self.name)
    }

    /// The model revision: one per accepted edit.
    pub(super) fn revision(&self) -> u64 {
        self.revision
    }

    /// Types `text` into the field.
    ///
    /// On success the valid value is replaced and the revision advances by one. On refusal the
    /// editor keeps the text as pending, the reason is returned, and both the valid value and the
    /// revision are unchanged.
    pub(super) fn apply(&mut self, text: &str) -> Result<u64, String> {
        match self.editor.set(self.node, self.name.clone(), text) {
            Ok(()) => {
                self.revision += 1;
                Ok(self.revision)
            }
            Err(reason) => Err(reason),
        }
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
                .pending(state.node, &state.name)
                .map(str::to_owned),
            Some(overlong)
        );
    }
}
