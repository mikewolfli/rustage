// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! What a preview inspection shows, and what it refuses to do (task T14.05).
//!
//! The acceptance line is *"it shows the design value, the computed value and the running value with
//! their versions; a read-only inspection does not run business logic, and a native external process
//! is only opened through a real adapter capability"*. Three rules carry it:
//!
//! * **One property, three layers, three versions.** [`Values`] keeps the design value, the computed
//!   value and the running value apart, each stamped with the version it was read at, so a stale
//!   running value cannot be shown as the design value and a missing layer says *not available*
//!   instead of borrowing its neighbour's text.
//! * **A read-only inspection has no way to have an effect.** [`Inspection::values`] and
//!   [`Inspection::locate`] cannot produce a side effect, and [`Inspection::trigger`] is refused in
//!   [`Mode::Design`], so "inspecting the selection" can never dispatch anything.
//! * **The running layer needs a real process.** A running value can only be recorded while
//!   [`Adapter::Real`] and a started instance are both there; a sample double or an absent adapter
//!   answers with the reason instead of letting a stand-in pretend to be live.

#![allow(dead_code)]

use std::collections::BTreeMap;

use crate::contracts::identity::NodeId;

/// The mode the designer is in.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mode {
    /// Design selection: the selection is inspected and nothing is dispatched.
    #[default]
    Design,
    /// Interaction: an action may be dispatched.
    Interact,
}

impl Mode {
    /// The mode's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Design => "design",
            Self::Interact => "interact",
        }
    }

    /// Reads a mode's name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "design" => Some(Self::Design),
            "interact" => Some(Self::Interact),
            _ => None,
        }
    }
}

/// How real the native side of a capability is.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Adapter {
    /// A real external process answers.
    Real,
    /// A sample double answers; no process is opened.
    Sample,
    /// Nothing native is open at all.
    #[default]
    Absent,
}

impl Adapter {
    /// The adapter's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Real => "real",
            Self::Sample => "sample",
            Self::Absent => "absent",
        }
    }

    /// Reads an adapter's name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "real" => Some(Self::Real),
            "sample" => Some(Self::Sample),
            "absent" => Some(Self::Absent),
            _ => None,
        }
    }

    /// Whether a real external process is behind this capability.
    pub fn is_real(&self) -> bool {
        matches!(self, Self::Real)
    }
}

/// Which of the three layers a value belongs to.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Layer {
    /// What the document says.
    Design,
    /// What the layout and style pass resolved.
    Computed,
    /// What the running instance reports.
    Runtime,
}

impl Layer {
    /// The layer's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Design => "design",
            Self::Computed => "computed",
            Self::Runtime => "runtime",
        }
    }
}

/// One value and the version it was read at.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Stamped {
    /// The value, as it is shown.
    pub text: String,
    /// The version it was read at.
    pub version: u32,
}

/// The design, computed and running values of one property.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Values {
    layers: BTreeMap<Layer, Stamped>,
}

impl Values {
    /// An empty answer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records one layer's value.
    pub fn set(&mut self, layer: Layer, text: impl Into<String>, version: u32) {
        self.layers.insert(
            layer,
            Stamped {
                text: text.into(),
                version,
            },
        );
    }

    /// One layer's value, if it was recorded.
    pub fn get(&self, layer: Layer) -> Option<&Stamped> {
        self.layers.get(&layer)
    }

    /// Whether the answer is empty.
    pub fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }

    /// One layer's line, against the version that layer is read at.
    fn line(&self, layer: Layer, current: Option<u32>) -> String {
        let Some(value) = self.layers.get(&layer) else {
            return format!("{}: not available", layer.name());
        };
        match current {
            None => format!(
                "{}: {} (v {}, no running instance)",
                layer.name(),
                value.text,
                value.version
            ),
            Some(current) if current == value.version => {
                format!("{}: {} (v {})", layer.name(), value.text, value.version)
            }
            Some(current) => format!(
                "{}: {} (v {}, stale: now at v {current})",
                layer.name(),
                value.text,
                value.version
            ),
        }
    }

    /// The three layers as lines, each against the version it is read at.
    ///
    /// The design and computed layers are read against the document's edit version and the running
    /// layer against the instance's version, because those are the versions that can move each one.
    pub fn render(&self, edit_version: u32, instance: Option<u32>) -> Vec<String> {
        vec![
            self.line(Layer::Design, Some(edit_version)),
            self.line(Layer::Computed, Some(edit_version)),
            self.line(Layer::Runtime, instance),
        ]
    }

    /// The three layers on one line.
    pub fn summary(&self, edit_version: u32, instance: Option<u32>) -> String {
        self.render(edit_version, instance).join("; ")
    }
}

/// Where a node came from in the project's source.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Source {
    /// The file.
    pub file: String,
    /// The line.
    pub line: u32,
    /// The column.
    pub column: u32,
}

impl Source {
    /// A source location.
    pub fn new(file: impl Into<String>, line: u32, column: u32) -> Self {
        Self {
            file: file.into(),
            line,
            column,
        }
    }

    /// The location as `<file>:<line>:<column>`.
    pub fn render(&self) -> String {
        format!("{}:{}:{}", self.file, self.line, self.column)
    }
}

/// One inspection session: the mode, the running instance and what the inspectors have.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Inspection {
    mode: Mode,
    adapter: Adapter,
    instance: Option<(String, u32)>,
    edit_version: u32,
    values: BTreeMap<(NodeId, String), Values>,
    sources: BTreeMap<NodeId, Source>,
    kinds: BTreeMap<NodeId, String>,
    parents: BTreeMap<NodeId, NodeId>,
    events: BTreeMap<String, String>,
    data: BTreeMap<String, String>,
    effects: u32,
}

impl Inspection {
    /// An inspection with nothing in it: design mode, no adapter and no instance.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the mode.
    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
    }

    /// The mode.
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Sets how real the native side is.
    pub fn set_adapter(&mut self, adapter: Adapter) {
        self.adapter = adapter;
    }

    /// How real the native side is.
    pub fn adapter(&self) -> Adapter {
        self.adapter
    }

    /// Sets the document's edit version.
    pub fn set_edit_version(&mut self, version: u32) {
        self.edit_version = version;
    }

    /// The document's edit version.
    pub fn edit_version(&self) -> u32 {
        self.edit_version
    }

    /// Starts (or replaces) the running instance.
    pub fn start_instance(&mut self, name: impl Into<String>, version: u32) {
        self.instance = Some((name.into(), version));
    }

    /// Stops the running instance.
    pub fn stop_instance(&mut self) {
        self.instance = None;
    }

    /// The running instance and the version it runs.
    pub fn instance(&self) -> Option<(&str, u32)> {
        self.instance
            .as_ref()
            .map(|(name, version)| (name.as_str(), *version))
    }

    /// The version the running layer is read against.
    pub fn instance_version(&self) -> Option<u32> {
        self.instance.as_ref().map(|(_, version)| *version)
    }

    /// The side effects dispatched so far.
    pub fn effects(&self) -> u32 {
        self.effects
    }

    /// Names what a node is.
    pub fn kind(&mut self, node: NodeId, kind: impl Into<String>) {
        self.kinds.insert(node, kind.into());
    }

    /// Records where a node came from.
    pub fn source(&mut self, node: NodeId, source: Source) {
        self.sources.insert(node, source);
        self.kinds.entry(node).or_default();
    }

    /// Records a node's place in the running layout tree.
    ///
    /// A node that is its own parent, or a chain that would close a loop, is refused: a tree that is
    /// not a tree could not be rendered or walked.
    pub fn child(&mut self, node: NodeId, parent: NodeId) -> Result<(), String> {
        if node == parent {
            return Err(format!("node {} cannot be its own parent", node.get()));
        }
        let mut walk = Some(parent);
        while let Some(current) = walk {
            if current == node {
                return Err(format!(
                    "node {} is an ancestor of {} already, so this would make a loop",
                    node.get(),
                    parent.get()
                ));
            }
            walk = self.parents.get(&current).copied();
        }
        self.parents.insert(node, parent);
        self.kinds.entry(node).or_default();
        self.kinds.entry(parent).or_default();
        Ok(())
    }

    /// Records an event and its latest payload.
    pub fn event(&mut self, name: impl Into<String>, payload: impl Into<String>) {
        self.events.insert(name.into(), payload.into());
    }

    /// Records a field the data inspector sees.
    pub fn data(&mut self, field: impl Into<String>, value: impl Into<String>) {
        self.data.insert(field.into(), value.into());
    }

    /// The answer for one property.
    fn entry(&mut self, node: NodeId, property: &str) -> &mut Values {
        self.values.entry((node, property.to_owned())).or_default()
    }

    /// Records what the document says a property is.
    pub fn declare_design(&mut self, node: NodeId, property: &str, text: impl Into<String>) {
        let version = self.edit_version;
        self.entry(node, property).set(Layer::Design, text, version);
    }

    /// Records what the layout and style pass resolved a property to.
    pub fn declare_computed(&mut self, node: NodeId, property: &str, text: impl Into<String>) {
        let version = self.edit_version;
        self.entry(node, property)
            .set(Layer::Computed, text, version);
    }

    /// Records what the running instance reports.
    ///
    /// Refused unless a real adapter and a started instance are both there: the running layer is the
    /// native external process, and nothing else may speak for it.
    pub fn declare_runtime(
        &mut self,
        node: NodeId,
        property: &str,
        text: impl Into<String>,
        version: u32,
    ) -> Result<(), String> {
        if !self.adapter.is_real() {
            return Err(format!(
                "the running layer needs a real adapter capability, and this session is on {:?}",
                self.adapter.name()
            ));
        }
        if self.instance.is_none() {
            return Err("there is no running instance to report a running value".to_owned());
        }
        self.entry(node, property)
            .set(Layer::Runtime, text, version);
        Ok(())
    }

    /// Shows one property's three layers and their versions.
    ///
    /// This is the read-only inspection: it reads and never dispatches.
    pub fn values(&self, node: NodeId, property: &str) -> Result<Vec<String>, String> {
        let Some(values) = self.values.get(&(node, property.to_owned())) else {
            return Err(format!(
                "nothing is known about {property:?} on node {}",
                node.get()
            ));
        };
        Ok(values.render(self.edit_version, self.instance_version()))
    }

    /// Shows a node: what it is, where it came from and where it sits.
    ///
    /// Read-only, like [`Inspection::values`].
    pub fn locate(&self, node: NodeId) -> Result<Vec<String>, String> {
        if !self.kinds.contains_key(&node) {
            return Err(format!("there is no node {}", node.get()));
        }
        Ok(vec![
            format!(
                "node {} ({})",
                node.get(),
                self.kinds.get(&node).expect("just checked")
            ),
            match self.sources.get(&node) {
                Some(source) => format!("source: {}", source.render()),
                None => "source: unknown".to_owned(),
            },
            format!("path: {}", self.path(node)),
        ])
    }

    /// The route from the tree's root down to a node.
    fn path(&self, node: NodeId) -> String {
        let mut route = vec![node];
        let mut walk = self.parents.get(&node).copied();
        while let Some(current) = walk {
            route.push(current);
            walk = self.parents.get(&current).copied();
        }
        route.reverse();
        route
            .iter()
            .map(|id| id.get().to_string())
            .collect::<Vec<_>>()
            .join(" > ")
    }

    /// Dispatches an action.
    ///
    /// Refused in [`Mode::Design`], where a selection is only inspected: nothing about an inspection
    /// may start business logic, and a refusal counts no effect.
    pub fn trigger(&mut self, action: &str) -> Result<String, String> {
        if self.mode != Mode::Interact {
            return Err(format!(
                "the session is in {} mode, so {action:?} is not dispatched; a read-only inspection \
                 does not run business logic",
                self.mode.name()
            ));
        }
        self.effects += 1;
        Ok(format!("dispatched {action}"))
    }

    /// The running layout tree, one line per node with its children indented.
    pub fn tree(&self) -> Vec<String> {
        let mut children: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::new();
        for (node, parent) in &self.parents {
            children.entry(*parent).or_default().push(*node);
        }
        let mut lines = Vec::new();
        for root in self
            .kinds
            .keys()
            .filter(|node| !self.parents.contains_key(node))
        {
            self.walk(*root, 0, &children, &mut lines);
        }
        lines
    }

    /// One node and its subtree, indented.
    fn walk(
        &self,
        node: NodeId,
        depth: usize,
        children: &BTreeMap<NodeId, Vec<NodeId>>,
        lines: &mut Vec<String>,
    ) {
        lines.push(format!(
            "{:indent$}node {} ({})",
            "",
            node.get(),
            self.kinds
                .get(&node)
                .map(String::as_str)
                .unwrap_or("(unnamed)"),
            indent = depth * 2
        ));
        for child in children.get(&node).into_iter().flatten() {
            self.walk(*child, depth + 1, children, lines);
        }
    }

    /// The whole inspection in one line per fact.
    pub fn render(&self) -> Vec<String> {
        let instance = match self.instance() {
            Some((name, version)) => format!("{name} (v {version})"),
            None => "none".to_owned(),
        };
        let mut lines = vec![format!(
            "mode: {}; adapter: {}; instance: {instance}; edit version: {}; effects: {}",
            self.mode.name(),
            self.adapter.name(),
            self.edit_version,
            self.effects
        )];
        lines.push(format!("values: {}", self.values.len()));
        for ((node, property), values) in &self.values {
            lines.push(format!(
                "  node {} {property}: {}",
                node.get(),
                values.summary(self.edit_version, self.instance_version())
            ));
        }
        lines.push(format!("events: {}", self.events.len()));
        for (name, payload) in &self.events {
            lines.push(format!("  {name}: {payload}"));
        }
        lines.push(format!("data: {}", self.data.len()));
        for (field, value) in &self.data {
            lines.push(format!("  {field}={value}"));
        }
        lines.push("layout tree:".to_owned());
        lines.extend(self.tree());
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{Adapter, Inspection, Layer, Mode, Source};

    fn node(id: u64) -> crate::contracts::identity::NodeId {
        crate::contracts::identity::NodeId::from_raw(id)
    }

    /// An inspection with a real adapter and a running instance at version 4.
    fn running() -> Inspection {
        let mut inspection = Inspection::new();
        inspection.set_adapter(Adapter::Real);
        inspection.set_edit_version(4);
        inspection.start_instance("phone", 4);
        inspection
    }

    #[test]
    fn the_three_layers_keep_their_own_text_and_version() {
        let mut inspection = running();
        inspection.declare_design(node(1), "width", "12px");
        inspection.declare_computed(node(1), "width", "16px");
        inspection
            .declare_runtime(node(1), "width", "18px", 4)
            .expect("a running value");
        let lines = inspection.values(node(1), "width").expect("an answer");
        assert_eq!(
            lines,
            vec![
                "design: 12px (v 4)".to_owned(),
                "computed: 16px (v 4)".to_owned(),
                "runtime: 18px (v 4)".to_owned(),
            ]
        );
    }

    #[test]
    fn a_missing_layer_says_not_available_rather_than_borrowing_a_neighbour() {
        let mut inspection = running();
        inspection.declare_design(node(1), "width", "12px");
        let lines = inspection.values(node(1), "width").expect("an answer");
        assert_eq!(lines[0], "design: 12px (v 4)");
        assert_eq!(lines[1], "computed: not available");
        assert_eq!(lines[2], "runtime: not available");
    }

    #[test]
    fn a_running_value_needs_a_real_adapter() {
        for adapter in [Adapter::Sample, Adapter::Absent] {
            let mut inspection = Inspection::new();
            inspection.set_adapter(adapter);
            inspection.start_instance("phone", 4);
            let refused = inspection.declare_runtime(node(1), "width", "18px", 4);
            assert!(refused.is_err());
            assert!(refused.unwrap_err().contains(adapter.name()));
        }
    }

    #[test]
    fn a_running_value_needs_a_running_instance() {
        let mut inspection = Inspection::new();
        inspection.set_adapter(Adapter::Real);
        let refused = inspection.declare_runtime(node(1), "width", "18px", 4);
        assert!(refused.is_err());
    }

    #[test]
    fn a_running_value_older_than_the_instance_is_marked_stale() {
        let mut inspection = running();
        inspection
            .declare_runtime(node(1), "width", "18px", 3)
            .expect("a running value");
        let lines = inspection.values(node(1), "width").expect("an answer");
        assert_eq!(lines[2], "runtime: 18px (v 3, stale: now at v 4)");
    }

    #[test]
    fn a_computed_value_from_an_older_edit_is_marked_stale() {
        let mut inspection = running();
        inspection.set_edit_version(3);
        inspection.declare_computed(node(1), "width", "16px");
        inspection.set_edit_version(4);
        let lines = inspection.values(node(1), "width").expect("an answer");
        assert_eq!(lines[1], "computed: 16px (v 3, stale: now at v 4)");
    }

    #[test]
    fn a_running_value_after_the_instance_stopped_says_so() {
        let mut inspection = running();
        inspection
            .declare_runtime(node(1), "width", "18px", 4)
            .expect("a running value");
        inspection.stop_instance();
        let lines = inspection.values(node(1), "width").expect("an answer");
        assert_eq!(lines[2], "runtime: 18px (v 4, no running instance)");
    }

    #[test]
    fn inspecting_does_not_run_business_logic() {
        let mut inspection = running();
        inspection.declare_design(node(1), "width", "12px");
        inspection.declare_computed(node(1), "width", "16px");
        inspection.kind(node(1), "panel");
        inspection.values(node(1), "width").expect("an answer");
        inspection.locate(node(1)).expect("a node");
        inspection.render();
        inspection.tree();
        assert_eq!(inspection.effects(), 0);
    }

    #[test]
    fn a_trigger_in_design_mode_is_refused_and_counts_no_effect() {
        let mut inspection = Inspection::new();
        let refused = inspection.trigger("send-order");
        assert!(refused.is_err());
        assert!(refused.unwrap_err().contains("design"));
        assert_eq!(inspection.effects(), 0);
    }

    #[test]
    fn a_trigger_in_interact_mode_is_dispatched_once() {
        let mut inspection = Inspection::new();
        inspection.set_mode(Mode::Interact);
        assert_eq!(
            inspection.trigger("send-order").expect("a dispatch"),
            "dispatched send-order"
        );
        assert_eq!(inspection.effects(), 1);
    }

    #[test]
    fn a_trigger_in_design_mode_is_dispatched_again_after_switching_to_interact() {
        let mut inspection = Inspection::new();
        assert!(inspection.trigger("send-order").is_err());
        inspection.set_mode(Mode::Interact);
        assert!(inspection.trigger("send-order").is_ok());
        inspection.set_mode(Mode::Design);
        assert!(inspection.trigger("send-order").is_err());
        assert_eq!(inspection.effects(), 1);
    }

    #[test]
    fn locating_a_node_answers_its_source_and_its_path() {
        let mut inspection = running();
        inspection.kind(node(1), "panel");
        inspection.kind(node(2), "list");
        inspection.kind(node(3), "label");
        inspection.child(node(2), node(1)).expect("a child");
        inspection.child(node(3), node(2)).expect("a child");
        inspection.source(node(3), Source::new("src/page.rs", 42, 8));
        let lines = inspection.locate(node(3)).expect("a node");
        assert_eq!(lines[0], "node 3 (label)");
        assert_eq!(lines[1], "source: src/page.rs:42:8");
        assert_eq!(lines[2], "path: 1 > 2 > 3");
    }

    #[test]
    fn a_node_without_a_source_says_unknown() {
        let mut inspection = running();
        inspection.kind(node(1), "panel");
        let lines = inspection.locate(node(1)).expect("a node");
        assert_eq!(lines[1], "source: unknown");
        assert_eq!(lines[2], "path: 1");
    }

    #[test]
    fn a_node_cannot_be_its_own_parent_and_a_loop_is_refused() {
        let mut inspection = running();
        assert!(inspection.child(node(1), node(1)).is_err());
        inspection.child(node(2), node(1)).expect("a child");
        inspection.child(node(3), node(2)).expect("a child");
        let refused = inspection.child(node(1), node(3));
        assert!(refused.is_err());
    }

    #[test]
    fn the_tree_indents_a_child_under_its_parent() {
        let mut inspection = running();
        inspection.kind(node(1), "panel");
        inspection.kind(node(2), "label");
        inspection.child(node(2), node(1)).expect("a child");
        assert_eq!(
            inspection.tree(),
            vec!["node 1 (panel)".to_owned(), "  node 2 (label)".to_owned()]
        );
    }

    #[test]
    fn the_inspection_renders_the_mode_the_adapter_the_effects_and_every_inspector() {
        let mut inspection = running();
        inspection.declare_design(node(1), "width", "12px");
        inspection.event("tapped", "{\"id\":3}");
        inspection.data("total", "12");
        inspection.kind(node(1), "panel");
        let lines = inspection.render();
        assert_eq!(
            lines[0],
            "mode: design; adapter: real; instance: phone (v 4); edit version: 4; effects: 0"
        );
        assert!(lines.contains(&"events: 1".to_owned()));
        assert!(lines.contains(&"  tapped: {\"id\":3}".to_owned()));
        assert!(lines.contains(&"data: 1".to_owned()));
        assert!(lines.contains(&"  total=12".to_owned()));
        assert!(lines.contains(&"node 1 (panel)".to_owned()));
    }

    #[test]
    fn the_mode_and_the_adapter_parse_and_render() {
        assert_eq!(Mode::parse("design"), Some(Mode::Design));
        assert_eq!(Mode::parse("interact"), Some(Mode::Interact));
        assert_eq!(Mode::parse("canvas"), None);
        assert_eq!(Adapter::parse("real"), Some(Adapter::Real));
        assert_eq!(Adapter::parse("absent"), Some(Adapter::Absent));
        assert_eq!(Adapter::parse("live"), None);
        assert_eq!(Adapter::default(), Adapter::Absent);
        assert!(!Adapter::Sample.is_real());
        assert_eq!(Layer::Runtime.name(), "runtime");
    }
}
