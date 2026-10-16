// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The layout contract, its computed result, its diagnostics and where a constraint came from
//! (task T10.01).
//!
//! The acceptance line is *"the preview and the generation reuse the same semantics rather than a
//! copy of the algorithm"*. The one way to make that true is to keep the algorithm in exactly one
//! place and let nothing else own a copy:
//!
//! * **The algorithm is the engine's, and only the engine's.** [`Engine`] is the seam the library
//!   implements; this module computes no box of its own, so there is no second algorithm to drift.
//! * **Both callers come through one door.** [`layout`] is the only caller of [`Engine::layout`], and
//!   it takes a [`Role`] — preview or generation — that changes *who is asking*, never the answer.
//! * **A result carries its own explanation.** A layout either yields the [`Computed`] boxes or a list
//!   of [`Diagnostic`]s, each naming the node and, when it can, the [`Source`] — node, property and
//!   origin — so "why is this box like this" is answerable and can be jumped to.
//! * **A constraint remembers where it came from.** [`Origin`] tells an explicit value from an
//!   inherited one from a default, so the property panel shows a source rather than a value with no
//!   history.

#![allow(dead_code)]

use crate::contracts::identity::NodeId;
use crate::contracts::model::PropertyName;

/// Where a constraint's value came from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Origin {
    /// Written on this node.
    Explicit,
    /// Taken from an ancestor.
    Inherited,
    /// Supplied by the schema when nothing was written.
    Default,
}

impl Origin {
    /// The origin's name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Explicit => "explicit",
            Self::Inherited => "inherited",
            Self::Default => "default",
        }
    }

    /// The origin a name picks.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "explicit" => Some(Self::Explicit),
            "inherited" => Some(Self::Inherited),
            "default" => Some(Self::Default),
            _ => None,
        }
    }
}

/// The source of a value: which node, which property, and how it got there.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Source {
    /// The node the value belongs to.
    pub node: NodeId,
    /// The property's name.
    pub property: PropertyName,
    /// How the value got there.
    pub origin: Origin,
}

impl Source {
    /// The source in one line.
    pub fn render(&self) -> String {
        format!(
            "node {} {} ({})",
            self.node.get(),
            self.property.as_str(),
            self.origin.name()
        )
    }
}

/// A constraint on one node, and where it came from.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Constraint {
    /// The node it constrains.
    pub node: NodeId,
    /// The property it names.
    pub property: PropertyName,
    /// Where its value came from.
    pub origin: Origin,
}

impl Constraint {
    /// A constraint.
    pub fn new(node: NodeId, property: PropertyName, origin: Origin) -> Self {
        Self {
            node,
            property,
            origin,
        }
    }

    /// Where this constraint's value came from.
    pub fn source(&self) -> Source {
        Source {
            node: self.node,
            property: self.property.clone(),
            origin: self.origin,
        }
    }
}

/// A size, in device-independent pixels.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Size {
    /// Width.
    pub width: f64,
    /// Height.
    pub height: f64,
}

impl Size {
    /// A size.
    pub const fn new(width: f64, height: f64) -> Self {
        Self { width, height }
    }
}

/// The input one node's layout gets: the space it is offered and the constraints on it.
#[derive(Clone, PartialEq, Debug)]
pub struct LayoutInput {
    /// The node.
    pub node: NodeId,
    /// The space the parent offers.
    pub available: Size,
    /// The constraints on it, each carrying its source.
    pub constraints: Vec<Constraint>,
}

impl LayoutInput {
    /// An input with no constraints.
    pub fn new(node: NodeId, available: Size) -> Self {
        Self {
            node,
            available,
            constraints: Vec::new(),
        }
    }

    /// The same input with one more constraint.
    pub fn with(mut self, constraint: Constraint) -> Self {
        self.constraints.push(constraint);
        self
    }

    /// Where the constraint on a property came from, if there is one.
    pub fn source(&self, property: &PropertyName) -> Option<Source> {
        self.constraints
            .iter()
            .find(|constraint| &constraint.property == property)
            .map(Constraint::source)
    }
}

/// A computed box.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Computed {
    /// The node.
    pub node: NodeId,
    /// Left.
    pub x: f64,
    /// Top.
    pub y: f64,
    /// Width.
    pub width: f64,
    /// Height.
    pub height: f64,
}

impl Computed {
    /// The box in one line.
    pub fn render(&self) -> String {
        format!(
            "node {} at {},{} {}x{}",
            self.node.get(),
            self.x,
            self.y,
            self.width,
            self.height
        )
    }
}

/// Why a layout could not be computed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Diagnostic {
    /// The node it is about.
    pub node: NodeId,
    /// The property it is about, when one property is at fault.
    pub property: Option<PropertyName>,
    /// What is wrong.
    pub message: String,
    /// The constraint it came from, when there is one.
    pub source: Option<Source>,
}

impl Diagnostic {
    /// A diagnostic about a node.
    pub fn new(node: NodeId, message: impl Into<String>) -> Self {
        Self {
            node,
            property: None,
            message: message.into(),
            source: None,
        }
    }

    /// The same diagnostic, pointed at a constraint's source.
    pub fn at(mut self, source: Source) -> Self {
        self.property = Some(source.property.clone());
        self.source = Some(source);
        self
    }

    /// Where to jump to explain this: the source, when there is one.
    pub fn jump(&self) -> Option<&Source> {
        self.source.as_ref()
    }

    /// The diagnostic in one line.
    pub fn render(&self) -> String {
        match &self.source {
            Some(source) => format!(
                "node {}: {} [{}]",
                self.node.get(),
                self.message,
                source.render()
            ),
            None => format!("node {}: {}", self.node.get(), self.message),
        }
    }
}

/// The one layout engine. The library implements it; the designer does not.
pub trait Engine {
    /// Computes a box for each input, or explains why it cannot.
    fn layout(&self, inputs: &[LayoutInput]) -> Result<Vec<Computed>, Vec<Diagnostic>>;
}

/// Who is asking for a layout.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    /// The development-time preview.
    Preview,
    /// The generated code.
    Generation,
}

impl Role {
    /// The role's name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Preview => "preview",
            Self::Generation => "generation",
        }
    }

    /// The role a name picks.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "preview" => Some(Self::Preview),
            "generation" => Some(Self::Generation),
            _ => None,
        }
    }
}

/// What one caller got back: the role, the boxes, and any diagnostics.
#[derive(Clone, PartialEq, Debug)]
pub struct Plan {
    /// Who asked.
    pub role: Role,
    /// The boxes, when the engine could compute them.
    pub computed: Vec<Computed>,
    /// Why it could not, when it could not.
    pub diagnostics: Vec<Diagnostic>,
}

impl Plan {
    /// Whether the plan carries boxes rather than refusals.
    pub fn is_computed(&self) -> bool {
        self.diagnostics.is_empty()
    }

    /// The plan in a report.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "plan ({}): {} box(es), {} diagnostic(s)",
            self.role.name(),
            self.computed.len(),
            self.diagnostics.len()
        )];
        for box_ in &self.computed {
            lines.push(format!("  {}", box_.render()));
        }
        for diagnostic in &self.diagnostics {
            lines.push(format!("  {}", diagnostic.render()));
        }
        lines
    }
}

/// Lays out by asking the one engine. This is the only caller of [`Engine::layout`].
///
/// The role is recorded, not applied: a preview and a generation differ in what they *do* with the
/// boxes, never in how the boxes are found, which is what keeps the two from diverging.
pub fn layout(engine: &dyn Engine, role: Role, inputs: &[LayoutInput]) -> Plan {
    match engine.layout(inputs) {
        Ok(computed) => Plan {
            role,
            computed,
            diagnostics: Vec::new(),
        },
        Err(diagnostics) => Plan {
            role,
            computed: Vec::new(),
            diagnostics,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Computed, Constraint, Diagnostic, Engine, LayoutInput, Origin, Role, Size, Source, layout,
    };
    use crate::contracts::identity::NodeId;
    use crate::contracts::model::PropertyName;
    use std::cell::Cell;

    fn node(value: u64) -> NodeId {
        NodeId::from_raw(value)
    }

    fn name(text: &str) -> PropertyName {
        PropertyName::new(text).expect("a name")
    }

    /// A stand-in engine: it answers the boxes it was told, and diagnoses a node it was not told.
    struct Supplied {
        boxes: Vec<Computed>,
        calls: Cell<usize>,
    }

    impl Supplied {
        fn new(boxes: Vec<Computed>) -> Self {
            Self {
                boxes,
                calls: Cell::new(0),
            }
        }
    }

    impl Engine for Supplied {
        fn layout(&self, inputs: &[LayoutInput]) -> Result<Vec<Computed>, Vec<Diagnostic>> {
            self.calls.set(self.calls.get() + 1);
            let mut computed = Vec::new();
            let mut diagnostics = Vec::new();
            for input in inputs {
                match self.boxes.iter().find(|box_| box_.node == input.node) {
                    Some(box_) => computed.push(*box_),
                    None => diagnostics.push(Diagnostic::new(
                        input.node,
                        "the engine was given no box for this node",
                    )),
                }
            }
            if diagnostics.is_empty() {
                Ok(computed)
            } else {
                Err(diagnostics)
            }
        }
    }

    #[test]
    fn the_preview_and_the_generation_get_the_same_boxes_from_one_engine() {
        let box_ = Computed {
            node: node(1),
            x: 10.0,
            y: 20.0,
            width: 100.0,
            height: 40.0,
        };
        let engine = Supplied::new(vec![box_]);
        let inputs = [LayoutInput::new(node(1), Size::new(400.0, 300.0))];

        let preview = layout(&engine, Role::Preview, &inputs);
        let generation = layout(&engine, Role::Generation, &inputs);

        assert_eq!(preview.computed, generation.computed);
        assert_eq!(preview.diagnostics, generation.diagnostics);
        assert_ne!(preview.role, generation.role);
        assert_eq!(engine.calls.get(), 2, "both callers asked the one engine");
    }

    #[test]
    fn a_refusal_is_the_same_for_both_callers() {
        let engine = Supplied::new(Vec::new());
        let inputs = [LayoutInput::new(node(7), Size::new(10.0, 10.0))];
        let preview = layout(&engine, Role::Preview, &inputs);
        let generation = layout(&engine, Role::Generation, &inputs);
        assert!(!preview.is_computed());
        assert_eq!(preview.diagnostics, generation.diagnostics);
    }

    #[test]
    fn a_constraint_carries_its_source() {
        let constraint = Constraint::new(node(2), name("width"), Origin::Inherited);
        let source = constraint.source();
        assert_eq!(source.node, node(2));
        assert_eq!(source.property.as_str(), "width");
        assert_eq!(source.origin, Origin::Inherited);
        assert!(source.render().contains("inherited"));
    }

    #[test]
    fn an_input_answers_where_a_propertys_value_came_from() {
        let input = LayoutInput::new(node(1), Size::new(80.0, 24.0))
            .with(Constraint::new(node(1), name("width"), Origin::Explicit))
            .with(Constraint::new(node(1), name("height"), Origin::Default));
        let width = input.source(&name("width")).expect("a source");
        assert_eq!(width.origin, Origin::Explicit);
        assert_eq!(
            input.source(&name("height")).map(|s| s.origin),
            Some(Origin::Default)
        );
        assert!(input.source(&name("gap")).is_none());
    }

    #[test]
    fn a_diagnostic_can_be_jumped_to_its_constraint() {
        let source = Source {
            node: node(3),
            property: name("min-width"),
            origin: Origin::Explicit,
        };
        let diagnostic = Diagnostic::new(node(3), "min-width exceeds max-width").at(source.clone());
        assert_eq!(diagnostic.jump(), Some(&source));
        assert!(diagnostic.render().contains("min-width"));
        assert_eq!(
            diagnostic.property.as_ref().map(PropertyName::as_str),
            Some("min-width")
        );
    }

    #[test]
    fn a_diagnostic_without_a_source_has_no_jump_target() {
        let diagnostic = Diagnostic::new(node(4), "the engine refused");
        assert!(diagnostic.jump().is_none());
        assert_eq!(diagnostic.render(), "node 4: the engine refused");
    }

    #[test]
    fn the_origin_names_round_trip() {
        for origin in [Origin::Explicit, Origin::Inherited, Origin::Default] {
            assert_eq!(Origin::from_name(origin.name()), Some(origin));
        }
        assert!(Origin::from_name("guessed").is_none());
    }

    #[test]
    fn the_role_names_round_trip() {
        for role in [Role::Preview, Role::Generation] {
            assert_eq!(Role::from_name(role.name()), Some(role));
        }
        assert!(Role::from_name("other").is_none());
    }
}
