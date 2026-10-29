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
//!
//! When a control name is known the schema comes from the library's own `PropertySchema`
//! ([`crate::app::style::property_editor::Schema::from_property_schema`]), including its accepted
//! tokens, range and measured default. When the contract does not describe the property, the
//! Inspector reports "schema unavailable" and refuses edits instead of falling back to a guessed
//! type.

use std::collections::{BTreeMap, BTreeSet};

use crate::app::layout::layout_plan::Origin;
use crate::app::style::multi_property::{
    Common, NodeProperties, common, reset as reset_setting, value_of,
};
use crate::app::style::property_editor::{
    Editor, PropertyType, Schema, contract_value, render_value,
};
use crate::contracts::identity::NodeId;
use crate::contracts::model::{PropertyName, PropertyValue};

use super::workbench_scene::NodeContract;

/// The property that carries a demo node's structural name, and the test-only hand-built schema.
pub(super) const TITLE: &str = "title";

/// The Inspector's editable property and the model revision it has reached.
pub(super) struct PropertyState {
    editor: Editor,
    nodes: Vec<NodeId>,
    name: PropertyName,
    /// The structural name each node had when the Inspector first saw it, which is what reset
    /// restores.
    baseline: BTreeMap<NodeId, String>,
    /// The nodes whose property has been explicitly edited.
    edited: BTreeSet<NodeId>,
    revision: u64,
    /// The contract control the Inspector is showing, when one is known.
    control: Option<String>,
    /// Why the contract could not describe the property, when it could not.
    unavailable: Option<String>,
    /// The contract each selected node was explicitly mapped to.
    contracts: BTreeMap<NodeId, NodeContract>,
    /// What differs between the selected nodes' controls, when anything does.
    contract_note: Option<String>,
    /// Why the selected nodes cannot share one editable property.
    incompatible: Option<String>,
}

impl PropertyState {
    /// An editor over one node's `title`, seeded with `initial`.
    ///
    /// This is the hand-built schema the CLI tests have always used; it claims nothing about a
    /// control. The GUI uses [`Self::for_control`] so the Inspector reads the library's contract.
    #[cfg(test)]
    pub(super) fn new(initial: &str) -> Self {
        let name = PropertyName::new(TITLE).expect("`title` is a property name");
        let schema = Schema::new().with(name.clone(), PropertyType::Text { max_len: 40 });
        let mut state = Self {
            editor: Editor::new(schema),
            nodes: Vec::new(),
            name,
            baseline: BTreeMap::new(),
            edited: BTreeSet::new(),
            revision: 0,
            control: None,
            unavailable: None,
            contracts: BTreeMap::new(),
            contract_note: None,
            incompatible: None,
        };
        state.retarget(NodeId::from_raw(1), initial);
        state
    }

    /// An editor over one property of a contract control.
    ///
    /// The schema is the library's declaration. When the contract does not describe
    /// `control.property`, every edit is refused with the "schema unavailable" reason rather than
    /// the Inspector inventing a text field.
    pub(super) fn for_control(control: &str, property: &str, initial: &str) -> Self {
        let mut state = Self {
            editor: Editor::new(Schema::new()),
            nodes: Vec::new(),
            name: PropertyName::new(TITLE).expect("`title` is a property name"),
            baseline: BTreeMap::new(),
            edited: BTreeSet::new(),
            revision: 0,
            control: Some(control.to_owned()),
            unavailable: None,
            contracts: BTreeMap::new(),
            contract_note: None,
            incompatible: None,
        };
        if let Err(reason) = state.bind_property(control, property) {
            crate::host::log::emit(
                crate::host::log::Level::Warn,
                "app::run::workbench-property",
                &reason,
            );
        }
        state.retarget(NodeId::from_raw(1), initial);
        state
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

    /// Points the Inspector at another single node, seeding its value from the tree's name.
    ///
    /// A node the Inspector has already edited keeps its accepted value, so switching the
    /// selection back and forth does not silently discard a person's work.
    pub(super) fn retarget(&mut self, node: NodeId, initial: &str) {
        self.seed(&[(node, initial)]);
    }

    /// Points the Inspector at another contract control and property, then at `node`.
    ///
    /// `property` is part of the contract: `window.title` and `button.text` are different schemas,
    /// so the name changes with the control. A control/property the contract does not describe
    /// leaves the Inspector reporting the refusal instead of keeping the old type.
    pub(super) fn retarget_control(
        &mut self,
        node: NodeId,
        control: &str,
        property: &str,
        initial: &str,
    ) -> Result<(), String> {
        let unchanged = self.control.as_deref() == Some(control)
            && self.name.as_str() == property
            && self.unavailable.is_none()
            && self.editor.schema().kind_of(&self.name).is_some();
        let outcome = if unchanged {
            Ok(())
        } else {
            self.bind_property(control, property)
        };
        self.contracts.clear();
        self.contracts
            .insert(node, NodeContract::new(control, property));
        self.contract_note = None;
        self.incompatible = None;
        self.seed(&[(node, initial)]);
        outcome
    }

    /// Points the Inspector at a selection whose nodes each carry an explicit contract.
    ///
    /// Nodes that publish the same property with the same type share one field, even when the
    /// control differs; that difference is reported through [`Self::contract_note`]. A selection
    /// whose properties or types differ is reported through [`Self::incompatible`], shows
    /// [`Common::Mixed`], and refuses an edit rather than letting one node's schema stand for the
    /// rest. `problems` carries nodes the scene could not map to a contract at all.
    pub(super) fn retarget_many_contracts(
        &mut self,
        nodes: &[(NodeId, NodeContract, &str)],
        problems: &[String],
    ) {
        self.contracts.clear();
        self.contract_note = None;
        self.incompatible = None;
        for (node, contract, _) in nodes {
            self.contracts.insert(*node, contract.clone());
        }

        let Some((primary_node, primary, _)) = nodes.first() else {
            self.nodes.clear();
            if !problems.is_empty() {
                self.incompatible = Some(format!(
                    "no selected node has a contract mapping: {}",
                    problems.join("; ")
                ));
            }
            return;
        };
        let primary_node = *primary_node;
        let primary = primary.clone();
        let primary_kind = contract_kind(&primary);
        let mut differing = problems.to_vec();
        let mut controls = Vec::new();
        let mut compatible = Vec::new();
        for (node, contract, initial) in nodes {
            if contract.control != primary.control {
                controls.push(format!("node {} is {}", node.get(), contract.render()));
            }
            if contract.property != primary.property {
                differing.push(format!("node {} is {}", node.get(), contract.render()));
                continue;
            }
            match (contract_kind(contract), &primary_kind) {
                (Ok((_, kind)), Ok((_, primary_kind))) if kind == *primary_kind => {
                    compatible.push((*node, *initial));
                }
                (Err(reason), _) => {
                    differing.push(format!(
                        "node {} is {}: {reason}",
                        node.get(),
                        contract.render()
                    ));
                }
                _ => differing.push(format!(
                    "node {} is {} with a different property type",
                    node.get(),
                    contract.render()
                )),
            }
        }

        let outcome = self.bind_property(&primary.control, &primary.property);
        if outcome.is_ok() {
            self.seed(&compatible);
        }
        // The Inspector names the whole selection, including nodes whose contract kept them out of
        // the shared field; `incompatible` is what stops those nodes from claiming agreement.
        self.nodes = nodes.iter().map(|(node, _, _)| *node).collect();
        if !differing.is_empty() {
            self.incompatible = Some(format!(
                "the selection has different contract properties: showing {} for node {}; {}",
                primary.render(),
                primary_node.get(),
                differing.join("; ")
            ));
        }
        if !controls.is_empty() {
            self.contract_note = Some(format!(
                "controls differ: showing {} for node {}; {}",
                primary.render(),
                primary_node.get(),
                controls.join(", ")
            ));
        }
    }

    /// Points the Inspector at a selection without contract information.
    ///
    /// This is the test-only path over the hand-built schema; the GUI always supplies contracts
    /// through [`Self::retarget_many_contracts`].
    #[cfg(test)]
    pub(super) fn retarget_many(&mut self, nodes: &[(NodeId, &str)]) {
        self.contracts.clear();
        self.contract_note = None;
        self.incompatible = None;
        self.seed(nodes);
    }

    /// Reports a node the scene could not map to a contract.
    ///
    /// The editor is emptied so nothing editable remains; [`Self::schema_unavailable`] carries the
    /// scene's reason and [`Self::apply`] refuses every edit.
    pub(super) fn report_unavailable(&mut self, node: NodeId, initial: &str, reason: String) {
        self.contracts.clear();
        self.contract_note = None;
        self.incompatible = None;
        self.control = None;
        self.editor = Editor::new(Schema::new());
        self.unavailable = Some(reason);
        self.nodes = vec![node];
        self.baseline
            .entry(node)
            .or_insert_with(|| initial.to_owned());
    }

    /// Seeds each node the Inspector has not seen before.
    fn seed(&mut self, nodes: &[(NodeId, &str)]) {
        let available = match self.ensure_schema() {
            Ok(()) => true,
            Err(reason) => {
                // An unavailable schema is a fact the panel shows, not a reason to seed a value the
                // contract never declared.
                crate::host::log::emit(
                    crate::host::log::Level::Warn,
                    "app::run::workbench-property",
                    &reason,
                );
                false
            }
        };
        let mut selection = Vec::new();
        for (node, initial) in nodes {
            self.baseline
                .entry(*node)
                .or_insert_with(|| (*initial).to_owned());
            if available && self.editor.value(*node, &self.name).is_none() {
                let seed = self.seed_text(initial);
                if let Err(reason) = self.editor.set(*node, self.name.clone(), &seed) {
                    crate::host::log::emit(
                        crate::host::log::Level::Warn,
                        "app::run::workbench-property",
                        &format!("the Inspector could not seed node {}: {reason}", node.get()),
                    );
                }
            }
            if !selection.contains(node) {
                selection.push(*node);
            }
        }
        self.nodes = selection;
    }

    /// Resolves the contract schema on first use, when a control name is known.
    fn ensure_schema(&mut self) -> Result<(), String> {
        if let Some(reason) = self.unavailable.clone() {
            return Err(reason);
        }
        let Some(control) = self.control.clone() else {
            return Ok(());
        };
        if self.editor.schema().kind_of(&self.name).is_some() {
            return Ok(());
        }
        let property = self.name.as_str().to_owned();
        self.bind_property(&control, &property)
    }

    /// Reads `control`'s contract schema for `property`.
    ///
    /// A successful bind takes the contract's canonical property name and rebinds the editor, which
    /// keeps accepted values whose type did not change. A refused bind empties the schema and records
    /// the reason, so [`Self::apply`] cannot accept anything.
    fn bind_property(&mut self, control: &str, property: &str) -> Result<(), String> {
        self.control = Some(control.to_owned());
        let Some(name) = PropertyName::new(property) else {
            let message = format!("schema unavailable for {control}: the property name is blank");
            self.editor = Editor::new(Schema::new());
            self.name = PropertyName::new(TITLE).expect("`title` is a property name");
            self.unavailable = Some(message.clone());
            return Err(message);
        };
        match resolve_contract(control, &name) {
            Ok(schema) => {
                let canonical = schema
                    .entries()
                    .next()
                    .map(|(name, _)| name.clone())
                    .unwrap_or(name);
                self.editor.rebind(schema);
                self.name = canonical;
                self.unavailable = None;
                Ok(())
            }
            Err(reason) => {
                let message = format!(
                    "schema unavailable for {control}.{}: {reason}",
                    name.as_str()
                );
                self.editor = Editor::new(Schema::new());
                self.name = name;
                self.unavailable = Some(message.clone());
                Err(message)
            }
        }
    }

    /// The text a new node starts with: its own value, or the contract's default when it has none.
    fn seed_text(&self, initial: &str) -> String {
        if !initial.is_empty() {
            return initial.to_owned();
        }
        self.editor
            .schema()
            .default_of(&self.name)
            .map(render_value)
            .unwrap_or_default()
    }

    /// The property name.
    pub(super) fn name(&self) -> &PropertyName {
        &self.name
    }

    /// The contract control the Inspector reads, when one is known.
    pub(super) fn control(&self) -> Option<&str> {
        self.control.as_deref()
    }

    /// Whether the edited property is the structural name the tree shows.
    ///
    /// The demo maps a window/panel's name to its `title` contract; a button/label's `text` is not
    /// the tree's name, so accepting an edit to it must not rename the node.
    pub(super) fn edits_node_name(&self) -> bool {
        self.name.as_str() == TITLE
    }

    /// What differs between the selected nodes' controls, when anything does.
    ///
    /// Controls can differ while the property and its type agree (`button.text` and `label.text`);
    /// that is compatible, but still worth saying instead of showing one control as if it were the
    /// selection's only one.
    pub(super) fn contract_note(&self) -> Option<&str> {
        self.contract_note.as_deref()
    }

    /// Why the selected nodes cannot share one editable property, when they cannot.
    pub(super) fn incompatible(&self) -> Option<&str> {
        self.incompatible.as_deref()
    }

    /// The accepted tokens of the property, from the contract.
    ///
    /// Empty for every non-enum property, and for an unavailable schema: a choice list nobody
    /// declared is not offered.
    pub(super) fn accepted_tokens(&self) -> Vec<String> {
        match self.editor.schema().kind_of(&self.name) {
            Some(PropertyType::Enum { variants }) => variants.clone(),
            _ => Vec::new(),
        }
    }

    /// The contract's default for the property, rendered for the field.
    pub(super) fn contract_default(&self) -> Option<String> {
        self.editor
            .schema()
            .default_of(&self.name)
            .map(render_value)
    }

    /// Why the contract could not describe the property, when it could not.
    ///
    /// This is the Inspector's "schema unavailable" line: while it is set, [`Self::apply`] refuses
    /// every edit, and no value is seeded from a guessed type.
    pub(super) fn schema_unavailable(&self) -> Option<&str> {
        self.unavailable.as_deref()
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
        if let Some(reason) = &self.incompatible {
            // Different contract properties cannot agree, even if the values happen to match.
            return format!("mixed ({}): {reason}", self.nodes.len());
        }
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
        self.ensure_schema()?;
        if let Some(reason) = &self.incompatible {
            return Err(format!(
                "{reason}; the Inspector cannot apply one property to this selection"
            ));
        }
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
        if self.incompatible.is_some() {
            return Common::Mixed;
        }
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

/// The canonical property name and type one node contract resolves to.
///
/// The name is the library's, not the requested spelling: the same check that binds the Inspector
/// decides whether two nodes' contracts can share a field.
fn contract_kind(contract: &NodeContract) -> Result<(PropertyName, PropertyType), String> {
    let requested = PropertyName::new(&contract.property)
        .ok_or_else(|| format!("{}: the property name is blank", contract.render()))?;
    let schema = resolve_contract(&contract.control, &requested)?;
    schema
        .entries()
        .next()
        .map(|(name, kind)| (name.clone(), kind.clone()))
        .ok_or_else(|| format!("{}: the contract describes no property", contract.render()))
}

/// Reads one property's schema, tokens, bounds and measured default from the library contract.
///
/// The default is read separately from the schema because the contract answers "what type" and
/// "what does a fresh control hold" through different entry points: a property without a measured
/// default is still editable, it just has no starting value to show.
fn resolve_contract(control: &str, property: &PropertyName) -> Result<Schema, String> {
    // The host factory, not `WidgetFactory::shared()`: the parent crate's table is immutable and
    // does not own suite controls such as `form`/`form_set`, so the Inspector could not read them.
    let factory = crate::app::contract::factory();
    let contract = factory
        .property_schema(control, property.as_str())
        .map_err(|error| format!("the contract has no such property ({error:?})"))?;
    let kind = contract.value_kind;
    let mut schema =
        Schema::from_property_schema(control, &contract).map_err(|error| error.message())?;
    let canonical = schema.entries().next().map(|(name, _)| name.clone());
    if let Ok(default) = factory.default_property_value(control, property.as_str())
        && let Some(value) = contract_value(kind, &default)
        && let Some(name) = canonical
    {
        schema = schema.with_default(name, value);
    }
    Ok(schema)
}

#[cfg(test)]
mod tests {
    use super::PropertyState;
    use crate::app::run::workbench_scene::CanvasState;
    use crate::contracts::identity::NodeId;

    fn node_named(scene: &CanvasState, name: &str) -> NodeId {
        scene
            .row_nodes()
            .into_iter()
            .find(|node| scene.name(*node) == Some(name))
            .expect("the demo names the node")
    }

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

    #[test]
    fn a_contract_control_shows_its_default_and_accepted_tokens() {
        let state = PropertyState::for_control("button", "elide", "");
        assert_eq!(state.control(), Some("button"));
        assert_eq!(state.schema_unavailable(), None);
        // The empty seed takes the contract's measured default rather than an invented value.
        assert_eq!(state.value_text(), "right");
        assert_eq!(state.contract_default().as_deref(), Some("right"));
        assert_eq!(
            state.accepted_tokens(),
            vec![
                "none".to_owned(),
                "left".to_owned(),
                "middle".to_owned(),
                "right".to_owned(),
            ]
        );
    }

    #[test]
    fn a_suite_control_resolves_through_the_host_factory() {
        // `form` is installed by `rust-widgets-suites` and is absent from the parent crate's
        // immutable `WidgetFactory::shared()` table. The Inspector must read the host table, or
        // a form node would be reported as "schema unavailable" instead of editable.
        let state = PropertyState::for_control("form", "title", "");
        assert_eq!(state.control(), Some("form"));
        assert_eq!(state.schema_unavailable(), None);
        assert_eq!(state.value_text(), "");
        assert_eq!(state.contract_default().as_deref(), Some(""));

        // A form-set property resolves too, including through its alias.
        let state = PropertyState::for_control("formset", "current_index", "");
        assert_eq!(state.schema_unavailable(), None);
        assert_eq!(state.value_text(), "0");
    }

    #[test]
    fn a_token_the_contract_does_not_publish_is_refused() {
        let mut state = PropertyState::for_control("button", "elide", "");
        let reason = state.apply("loud").expect_err("not a declared token");
        assert!(reason.contains("not one of"), "{reason}");
        // The valid value and the revision are where they were.
        assert_eq!(state.value_text(), "right");
        assert_eq!(state.revision(), 0);
    }

    #[test]
    fn an_unavailable_schema_is_reported_rather_than_guessed() {
        let mut state = PropertyState::for_control("not_a_widget", "elide", "right");
        let unavailable = state.schema_unavailable().expect("a reason");
        assert!(unavailable.contains("schema unavailable"), "{unavailable}");
        // Nothing was seeded through a guessed text schema.
        assert_eq!(state.value_text(), "");
        assert!(state.accepted_tokens().is_empty());
        assert_eq!(state.contract_default(), None);

        let reason = state.apply("none").expect_err("no contract schema");
        assert!(reason.contains("schema unavailable"), "{reason}");
        assert_eq!(state.revision(), 0);
    }

    #[test]
    fn a_control_without_the_property_kind_this_editor_has_is_reported() {
        // `geometry` is a string property the button declares read-only; `icon_source` is an icon
        // source, which has no field in this editor.
        let mut state = PropertyState::for_control("button", "icon_source", "");
        let unavailable = state.schema_unavailable().expect("a reason");
        assert!(unavailable.contains("icon-spec"), "{unavailable}");
        assert!(state.apply("x").is_err());
    }

    #[test]
    fn retargeting_reads_the_new_controls_schema() {
        let mut state = PropertyState::for_control("button", "elide", "");
        assert_eq!(state.value_text(), "right");
        let second = NodeId::from_raw(2);
        state
            .retarget_control(second, "label", "elide", "")
            .expect("label publishes elide");
        assert_eq!(state.control(), Some("label"));
        // The label's own measured default, not the button's — the schema was re-read.
        assert_eq!(state.contract_default().as_deref(), Some("none"));
        assert_eq!(state.value_text(), "none");
    }

    #[test]
    fn a_contract_multi_selection_still_shows_agreement_or_mixed() {
        let mut state = PropertyState::for_control("button", "elide", "none");
        assert_eq!(state.value_text(), "none");
        state.retarget_many(&[
            (NodeId::from_raw(1), "none"),
            (NodeId::from_raw(2), "right"),
        ]);
        assert_eq!(state.count(), 2);
        assert!(state.is_mixed());
        assert_eq!(state.value_text(), "");

        // One validation, applied to both nodes; the contract tokens still decide.
        assert!(state.apply("loud").is_err());
        assert!(state.is_mixed());
        assert_eq!(state.apply("left").expect("a declared token"), 1);
        assert!(!state.is_mixed());
        assert_eq!(state.value_text(), "left");
    }

    #[test]
    fn the_root_selects_the_window_title_contract_schema() {
        let scene = CanvasState::demo();
        let root = node_named(&scene, "window");
        let contract = scene.contract(root).expect("the root is mapped");
        assert_eq!(contract.render(), "window.title");

        let mut state = PropertyState::for_control("panel", "title", "Demo");
        state
            .retarget_control(root, &contract.control, &contract.property, "window")
            .expect("window publishes title");
        assert_eq!(state.control(), Some("window"));
        assert_eq!(state.name().as_str(), "title");
        assert_eq!(state.schema_unavailable(), None);
        assert_eq!(state.value_text(), "window");
        assert!(state.edits_node_name());
    }

    #[test]
    fn the_button_switches_the_inspector_from_title_to_text() {
        let scene = CanvasState::demo();
        let button = node_named(&scene, "button");
        let contract = scene.contract(button).expect("the button is mapped");
        assert_eq!(contract.render(), "button.text");

        let mut state = PropertyState::for_control("window", "title", "window");
        state
            .retarget_control(button, &contract.control, &contract.property, "button")
            .expect("button publishes text");
        assert_eq!(state.control(), Some("button"));
        assert_eq!(state.name().as_str(), "text");
        assert_eq!(state.schema_unavailable(), None);
        assert_eq!(state.value_text(), "button");
        // The node's text is not the tree's structural name, so an accepted edit must not rename it.
        assert!(!state.edits_node_name());
    }

    #[test]
    fn a_value_the_contract_refuses_is_refused_with_its_reason() {
        // `icon_size` is an unsigned contract property; a negative value is refused by the contract's
        // own range rather than by a guessed text rule.
        let mut state = PropertyState::for_control("button", "icon_size", "16");
        let reason = state.apply("-1").expect_err("below the unsigned minimum");
        assert!(reason.contains("below the minimum 0"), "{reason}");
        assert_eq!(state.value_text(), "16");
        assert_eq!(state.revision(), 0);
    }

    #[test]
    fn an_unknown_node_contract_is_reported_rather_than_guessed() {
        let scene = CanvasState::demo();
        let reason = scene
            .contract(NodeId::from_raw(999))
            .expect_err("no mapping");
        assert!(reason.contains("no contract mapping"), "{reason}");

        let mut state = PropertyState::for_control("window", "title", "window");
        state.report_unavailable(NodeId::from_raw(999), "unknown", reason);
        let unavailable = state.schema_unavailable().expect("a reason");
        assert!(unavailable.contains("no contract mapping"), "{unavailable}");
        assert_eq!(state.value_text(), "");
        assert!(state.apply("anything").is_err());
    }

    #[test]
    fn a_multi_selection_with_incompatible_contract_properties_is_not_agreement() {
        let scene = CanvasState::demo();
        let root = node_named(&scene, "window");
        let button = node_named(&scene, "button");
        let entries = [
            (root, scene.contract(root).expect("window"), "window"),
            (button, scene.contract(button).expect("button"), "button"),
        ];
        let refs: Vec<_> = entries
            .iter()
            .map(|(node, contract, seed)| (*node, contract.clone(), *seed))
            .collect();

        let mut state = PropertyState::for_control("window", "title", "window");
        state.retarget_many_contracts(&refs, &[]);
        assert_eq!(state.count(), 2);
        assert!(state.is_mixed());
        assert_eq!(state.value_text(), "");
        let reason = state.incompatible().expect("an explicit reason");
        assert!(reason.contains("window.title"), "{reason}");
        assert!(reason.contains("button.text"), "{reason}");
        assert!(state.summary().contains("mixed (2)"), "{}", state.summary());

        // One node's schema cannot stand for the other: the edit is refused, not half-applied.
        let refusal = state
            .apply("shared")
            .expect_err("different contract properties");
        assert!(
            refusal.contains("different contract properties"),
            "{refusal}"
        );
        assert_eq!(state.revision(), 0);
    }

    #[test]
    fn a_multi_selection_with_the_same_property_on_different_controls_reports_the_controls() {
        let scene = CanvasState::demo();
        let button = node_named(&scene, "button");
        let label = node_named(&scene, "label");
        let entries = [
            (button, scene.contract(button).expect("button"), "shared"),
            (label, scene.contract(label).expect("label"), "shared"),
        ];
        let refs: Vec<_> = entries
            .iter()
            .map(|(node, contract, seed)| (*node, contract.clone(), *seed))
            .collect();

        let mut state = PropertyState::for_control("button", "text", "shared");
        state.retarget_many_contracts(&refs, &[]);
        // `button.text` and `label.text` declare the same type, so agreement is real — but the
        // differing controls are named instead of one control standing for the selection.
        assert!(!state.is_mixed());
        assert_eq!(state.value_text(), "shared");
        let note = state.contract_note().expect("controls differ");
        assert!(note.contains("button.text"), "{note}");
        assert!(note.contains("label.text"), "{note}");
        assert_eq!(state.apply("both").expect("both controls accept text"), 1);
    }
}
