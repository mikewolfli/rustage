// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The workbench's Data and inspection surface (tasks T13.04/T14.05).
//!
//! Two facts are kept apart here, for the same reason the Events panel keeps three:
//!
//! * **what the design says** — recorded as the design and computed layers of an [`Inspection`];
//! * **what the running instance says** — only a real adapter with a started instance may record
//!   it, so a design session reports the running layer as unavailable instead of showing a copy.
//!
//! The Data panel edits the *sample* dataset through the one typed schema; a refused value keeps
//! the text and advances nothing. The inspection is read-only: [`InspectState::inspect`] never
//! calls [`Inspection::trigger`], and the effect counter stays where it was.

use crate::app::data::data_model::{Data, Dataset, Schema, active, validate};
use crate::app::preview::inspection::{Adapter, Inspection, Mode, Source};
use crate::app::style::design_tokens::{TokenKind, TokenSet};
use crate::contracts::identity::NodeId;
use crate::contracts::model::PropertyValue;

use std::collections::BTreeMap;

use super::data_events::{parse_data_value, render_data_value};
use super::workbench_scene::CanvasState;

/// One node's design facts, kept so a version bump can re-stamp them without guessing.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Fact {
    node: NodeId,
    parent: Option<NodeId>,
    kind: String,
    name: String,
    line: u32,
    rect: (i32, i32, u32, u32),
}

/// The Data panel and the read-only inspector.
pub(super) struct InspectState {
    schema: Schema,
    sample: Data,
    runtime: Data,
    version: u32,
    inspection: Inspection,
    facts: Vec<Fact>,
    selected: Vec<NodeId>,
    problem: Option<String>,
    /// The design tokens and the theme last applied to the selection. The model keeps the
    /// `var(--accent)` reference; only the computed layer carries the expanded literal.
    tokens: TokenSet,
    theme: Option<String>,
    backgrounds: BTreeMap<NodeId, String>,
}

impl InspectState {
    /// A schema with three typed fields and a sample dataset; the runtime set is empty because no
    /// instance has been started.
    pub(super) fn new() -> Self {
        let schema = Schema::new("form", 1)
            .with(
                "count",
                crate::app::data::data_model::FieldKind::Integer,
                true,
            )
            .with("text", crate::app::data::data_model::FieldKind::Text, false)
            .with(
                "enabled",
                crate::app::data::data_model::FieldKind::Bool,
                false,
            );
        let sample = Data::new()
            .with("count", PropertyValue::Integer(1))
            .with("text", PropertyValue::Text("hello".to_owned()))
            .with("enabled", PropertyValue::Bool(true));
        let mut tokens = TokenSet::new();
        tokens.define(TokenKind::Colour, "--accent", "#0a84ff");
        tokens.define_theme("dark", &[("--accent".to_owned(), "#ff9f0a".to_owned())]);
        let mut state = Self {
            schema,
            sample,
            runtime: Data::new(),
            version: 1,
            inspection: Inspection::new(),
            facts: Vec::new(),
            selected: Vec::new(),
            problem: None,
            tokens,
            theme: None,
            backgrounds: BTreeMap::new(),
        };
        state.rebuild();
        state
    }

    /// The document version the sample schema and the design layer share.
    pub(super) fn version(&self) -> u32 {
        self.version
    }

    /// The running effects the inspection has dispatched. A read-only inspection never moves it.
    pub(super) fn effects(&self) -> u32 {
        self.inspection.effects()
    }

    /// The last refused data edit, if any.
    pub(super) fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }

    /// The field names in declaration order.
    pub(super) fn fields(&self) -> Vec<String> {
        self.schema
            .fields
            .iter()
            .map(|field| field.name.clone())
            .collect()
    }

    /// The data rows: the active dataset, one row per field and the schema diagnostics.
    pub(super) fn rows(&self) -> Vec<String> {
        let sample = Dataset::sample(self.version, self.sample.clone());
        let sample = match sample {
            Ok(dataset) => Some(dataset),
            Err(found) => {
                return vec![format!(
                    "sample refused: credential field(s) {}",
                    found.join(", ")
                )];
            }
        };
        let runtime = if self.runtime.values.is_empty() {
            None
        } else {
            Some(Dataset::runtime(self.version, self.runtime.clone()))
        };
        let selection = active(runtime.as_ref(), sample.as_ref());
        let mut rows = vec![format!(
            "data v{} — active: {}",
            self.version,
            selection.label()
        )];
        rows.push(format!(
            "theme: {}",
            self.theme.as_deref().unwrap_or("(none)")
        ));
        for field in &self.schema.fields {
            let value = selection
                .data()
                .and_then(|data| data.values.get(&field.name))
                .map(render_data_value)
                .unwrap_or_else(|| "(unset)".to_owned());
            rows.push(format!("{}: {} = {value}", field.name, field.kind.render()));
        }
        if let Some(dataset) = &sample {
            for diagnostic in validate(&self.schema, dataset) {
                rows.push(format!("problem: {}", diagnostic.render()));
            }
        }
        rows
    }

    /// The sample value of a field, as the panel's edit field shows it.
    pub(super) fn value_of(&self, field: &str) -> Option<String> {
        self.sample.values.get(field).map(render_data_value)
    }

    /// Points the inspector at the current selection and records every node's design facts.
    pub(super) fn retarget(&mut self, scene: &CanvasState, selected: &[NodeId]) {
        self.facts = scene
            .row_nodes()
            .into_iter()
            .enumerate()
            .map(|(index, node)| Fact {
                node,
                parent: scene.parent(node),
                kind: scene
                    .kind(node)
                    .map(|kind| kind.name().to_owned())
                    .unwrap_or_else(|| "?".to_owned()),
                name: scene.name(node).unwrap_or("").to_owned(),
                // The demo project's generated file has one construct per node; the line is the
                // fact both the inspector and the debug mapping read, not a guessed offset.
                line: 10 + index as u32 * 4,
                rect: scene
                    .rect(node)
                    .map(|rect| (rect.x, rect.y, rect.width, rect.height))
                    .unwrap_or((0, 0, 0, 0)),
            })
            .collect();
        self.selected = selected.to_vec();
        self.rebuild();
    }

    /// The design/computed/running values of the selected node, plus where it came from.
    ///
    /// This is the read-only half of T14.05: it reads and never dispatches.
    pub(super) fn inspect(&mut self) -> Result<Vec<String>, String> {
        let node = self
            .selected
            .first()
            .copied()
            .ok_or_else(|| "nothing is selected".to_owned())?;
        let mut lines = vec![format!(
            "design session v{} · effects {}",
            self.version,
            self.inspection.effects()
        )];
        lines.extend(self.inspection.values(node, "title")?);
        lines.extend(self.inspection.values(node, "bounds")?);
        if self.backgrounds.contains_key(&node) {
            lines.extend(self.inspection.values(node, "background")?);
        }
        lines.extend(self.inspection.locate(node)?);
        lines.push("design layout tree:".to_owned());
        lines.extend(self.tree());
        Ok(lines)
    }

    /// The design-time layout tree.
    pub(super) fn tree(&self) -> Vec<String> {
        self.inspection.tree()
    }

    /// Applies `text` to one sample field through the typed schema.
    ///
    /// On success the sample and the document version advance together; on refusal the text stays
    /// pending, the version is unchanged, and the reason is returned.
    pub(super) fn edit(&mut self, field: &str, text: &str) -> Result<u32, String> {
        let declared = self
            .schema
            .field(field)
            .ok_or_else(|| format!("no field {field:?} is declared"))?;
        let kind = declared.kind.clone();
        let value = parse_data_value(Some(&kind), text)?;
        self.sample.values.insert(field.to_owned(), value);
        self.version += 1;
        self.schema.version = self.version;
        self.problem = None;
        self.rebuild();
        Ok(self.version)
    }

    /// Refuses an edit without touching the model, keeping the reason for the panel.
    pub(super) fn refuse(&mut self, reason: impl Into<String>) -> String {
        let reason = reason.into();
        self.problem = Some(reason.clone());
        reason
    }

    /// The theme last applied to the selection.
    pub(super) fn theme(&self) -> Option<&str> {
        self.theme.as_deref()
    }

    /// The computed background of a node after a theme application.
    pub(super) fn background(&self, node: NodeId) -> Option<&str> {
        self.backgrounds.get(&node).map(String::as_str)
    }

    /// Applies a theme's `accent` token to every selected node.
    ///
    /// The design layer keeps `var(--accent)`; the computed layer stores the literal the theme
    /// expands to. An unknown theme is refused and changes nothing. This is the multi-selection
    /// review path J04 asks for: one action, every selected node's computed value shown against
    /// the same document version.
    pub(super) fn apply_theme(
        &mut self,
        name: &str,
        selected: &[NodeId],
    ) -> Result<Vec<String>, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("a theme name is required".to_owned());
        }
        if selected.is_empty() {
            return Err("nothing is selected".to_owned());
        }
        let expansion = self
            .tokens
            .expand("var(--accent)", Some(name))
            .map_err(|error| error.message())?;
        for node in selected {
            self.backgrounds.insert(*node, expansion.value.clone());
        }
        self.theme = Some(name.to_owned());
        self.version += 1;
        self.schema.version = self.version;
        self.problem = None;
        self.rebuild();
        let mut lines = vec![format!(
            "theme {name}: var(--accent) {} on {} node(s)",
            expansion.render(),
            selected.len()
        )];
        for node in selected {
            lines.push(format!(
                "  node {} background {}",
                node.get(),
                expansion.value
            ));
        }
        Ok(lines)
    }

    /// Rebuilds the inspection from the recorded facts at the current version.
    fn rebuild(&mut self) {
        let mut inspection = Inspection::new();
        inspection.set_mode(Mode::Design);
        inspection.set_adapter(Adapter::Absent);
        inspection.set_edit_version(self.version);
        for fact in &self.facts {
            inspection.kind(fact.node, fact.kind.clone());
            inspection.source(fact.node, Source::new("src/generated/ui.rs", fact.line, 1));
            inspection.declare_design(fact.node, "title", fact.name.clone());
            inspection.declare_computed(fact.node, "title", fact.name.clone());
            let bounds = format!(
                "{}x{} at {},{}",
                fact.rect.2, fact.rect.3, fact.rect.0, fact.rect.1
            );
            inspection.declare_design(fact.node, "bounds", bounds.clone());
            inspection.declare_computed(fact.node, "bounds", bounds);
            if let Some(background) = self.backgrounds.get(&fact.node) {
                inspection.declare_design(fact.node, "background", "var(--accent)");
                inspection.declare_computed(fact.node, "background", background.clone());
            }
            if let Some(parent) = fact.parent {
                // A refused child would break the tree; the demo scene is already a tree, so a
                // refusal here is a programming error and is reported rather than hidden.
                if let Err(reason) = inspection.child(fact.node, parent) {
                    crate::host::log::emit(
                        crate::host::log::Level::Warn,
                        "app::run::workbench-inspect",
                        &format!("inspection tree refused a child: {reason}"),
                    );
                }
            }
        }
        self.inspection = inspection;
    }
}

#[cfg(test)]
mod tests {
    use super::InspectState;
    use crate::app::run::workbench_scene::CanvasState;
    use crate::contracts::identity::NodeId;

    fn state() -> (InspectState, CanvasState) {
        let scene = CanvasState::demo();
        let mut state = InspectState::new();
        let selected = scene
            .row_nodes()
            .get(1)
            .copied()
            .into_iter()
            .collect::<Vec<_>>();
        state.retarget(&scene, &selected);
        (state, scene)
    }

    #[test]
    fn the_inspector_shows_design_and_computed_values_with_versions() {
        let (mut state, _) = state();
        let lines = state.inspect().expect("a selection");
        assert!(lines[0].contains("design session v1"), "{lines:?}");
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("design: Demo (v 1)")),
            "{lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("computed: Demo (v 1)")),
            "{lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("runtime: not available")),
            "{lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("src/generated/ui.rs:14:1")),
            "{lines:?}"
        );
    }

    #[test]
    fn a_read_only_inspection_never_dispatches() {
        let (mut state, _) = state();
        assert_eq!(state.effects(), 0);
        let _ = state.inspect().expect("a selection");
        assert_eq!(state.effects(), 0);
    }

    #[test]
    fn editing_a_sample_field_advances_the_version_and_the_rows() {
        let (mut state, _) = state();
        let version = state.edit("count", "7").expect("a valid integer");
        assert_eq!(version, 2);
        assert_eq!(state.version(), 2);
        assert!(
            state.rows().iter().any(|row| row == "count: integer = 7"),
            "{:?}",
            state.rows()
        );
        assert!(state.rows()[0].contains("data v2"), "{:?}", state.rows());
    }

    #[test]
    fn a_refused_value_keeps_the_model_and_the_version() {
        let (mut state, _) = state();
        let reason = state.edit("count", "seven").expect_err("not an integer");
        assert!(reason.contains("whole number"), "{reason}");
        assert_eq!(state.version(), 1);
        assert!(state.rows().iter().any(|row| row == "count: integer = 1"));
    }

    #[test]
    fn an_undeclared_field_is_refused() {
        let (mut state, _) = state();
        assert!(state.edit("missing", "1").is_err());
        assert_eq!(state.version(), 1);
    }

    #[test]
    fn applying_a_theme_expands_one_token_across_the_selection() {
        let scene = CanvasState::demo();
        let mut state = InspectState::new();
        let selected: Vec<crate::contracts::identity::NodeId> = scene
            .row_nodes()
            .into_iter()
            .filter(|node| node.get() == 1 || node.get() == 2)
            .collect();
        state.retarget(&scene, &selected);
        let lines = state
            .apply_theme("dark", &selected)
            .expect("a defined theme");
        assert!(lines[0].contains("theme dark"), "{lines:?}");
        assert_eq!(state.theme(), Some("dark"));
        assert_eq!(state.background(selected[0]), Some("#ff9f0a"));
        assert_eq!(state.background(selected[1]), Some("#ff9f0a"));
        let values = state.inspect().expect("a selection");
        assert!(
            values
                .iter()
                .any(|line| line.starts_with("design: var(--accent) (v 2)")),
            "{values:?}"
        );
        assert!(
            values
                .iter()
                .any(|line| line.starts_with("computed: #ff9f0a (v 2)")),
            "{values:?}"
        );
        assert_eq!(state.effects(), 0);
    }

    #[test]
    fn an_unknown_theme_is_refused_and_changes_nothing() {
        let (mut state, _) = state();
        let selected = vec![crate::contracts::identity::NodeId::from_raw(1)];
        let reason = state
            .apply_theme("missing", &selected)
            .expect_err("no such theme");
        assert!(reason.contains("missing"), "{reason}");
        assert_eq!(state.theme(), None);
        assert_eq!(state.version(), 1);
    }

    #[test]
    fn the_layout_tree_names_the_selected_node() {
        let (state, _) = state();
        let tree = state.tree();
        assert!(
            tree.iter().any(|line| line.contains("node 0 (container)")),
            "{tree:?}"
        );
        assert!(
            tree.iter().any(|line| line.contains("node 1 (container)")),
            "{tree:?}"
        );
        assert!(
            tree.iter()
                .any(|line| line.trim_start().starts_with("node 2 (widget)")),
            "{tree:?}"
        );
        let _ = NodeId::from_raw(0);
    }
}
