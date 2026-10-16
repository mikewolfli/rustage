// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Typed bindings between data fields and node properties, the conversion contract, and the refusals
//! for an incompatible pair or a cycle (task T13.02).
//!
//! The acceptance line is *"an incompatible or cyclic binding is refused, and two-way updates do not
//! oscillate"*. All three are about a binding that cannot settle, so the module refuses one before it
//! is added:
//!
//! * **A conversion says what it can carry.** [`Conversion`] is declared, and
//!   [`Conversion::accepts`] answers whether that conversion carries a source to a target — an
//!   identity between two texty carriers, a text bridge when exactly one side is texty, a scale
//!   between two numeric carriers. Anything else is refused, so a pair cannot be joined by a
//!   conversion that would not actually convert.
//! * **A lossy conversion may not be two-way.** [`Direction::TwoWay`] through a conversion that loses
//!   information is refused as *oscillating*, because the write-back would not reproduce what it read
//!   and the two sides would keep correcting each other.
//! * **A cycle is refused by the binding that would close it.** [`Graph::admit`] tries the binding on
//!   a copy and answers the cycle it would close, naming the path, so a loop cannot be half-built.
//! * **A missing field or slot is named.** [`Environment`] is where the field kinds and slot types
//!   come from, and [`Graph::admit`] answers which one is absent rather than binding to nothing.

#![allow(dead_code)]

use crate::app::data::data_model::{FieldKind, Schema};
use crate::app::style::property_editor::PropertyType;
use crate::contracts::identity::NodeId;
use crate::contracts::model::PropertyName;

/// A place a value can be read from or written to.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Origin {
    /// A data field.
    Field(String),
    /// A node's property.
    Slot {
        /// The node.
        node: NodeId,
        /// The property.
        property: PropertyName,
    },
}

impl Origin {
    /// The origin in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Field(name) => format!("field {name}"),
            Self::Slot { node, property } => {
                format!("node {}.{}", node.get(), property.as_str())
            }
        }
    }
}

/// Which way a binding carries values.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Direction {
    /// From the source to the target.
    OneWay,
    /// Both ways.
    TwoWay,
}

impl Direction {
    /// The direction's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::OneWay => "one-way",
            Self::TwoWay => "two-way",
        }
    }

    /// Reads a direction's name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "one-way" | "one" => Some(Self::OneWay),
            "two-way" | "two" => Some(Self::TwoWay),
            _ => None,
        }
    }
}

/// A declared value conversion between a source and a target.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Conversion {
    /// The two sides are already the same family.
    Identity,
    /// The value is carried as text.
    Text,
    /// A number is multiplied by this factor on the way to the target.
    Scale(f64),
}

impl Conversion {
    /// The conversion's name.
    pub fn name(&self) -> String {
        match self {
            Self::Identity => "identity".to_owned(),
            Self::Text => "text".to_owned(),
            Self::Scale(factor) => format!("scale:{factor}"),
        }
    }

    /// Reads a conversion, or `None` if it is not one.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "identity" => Some(Self::Identity),
            "text" => Some(Self::Text),
            _ => text
                .strip_prefix("scale:")
                .and_then(|factor| factor.parse::<f64>().ok())
                .map(Self::Scale)
                .filter(|conversion| {
                    // A zero or non-finite factor cannot carry a value.
                    matches!(conversion, Self::Scale(factor) if factor.is_finite() && *factor != 0.0)
                }),
        }
    }

    /// Whether the conversion loses information on the way back.
    pub fn is_lossy(&self) -> bool {
        !matches!(self, Self::Identity)
    }

    /// Whether the conversion carries a source to a target.
    pub fn accepts(&self, source: &Carrier<'_>, target: &Carrier<'_>) -> bool {
        match self {
            Self::Identity => compatible(source, target),
            Self::Text => is_texty(source) || is_texty(target),
            Self::Scale(_) => is_numeric(source) && is_numeric(target),
        }
    }
}

/// One side of a binding, as the kind it holds.
#[derive(Clone, Copy, Debug)]
pub enum Carrier<'a> {
    /// A data field's kind.
    Field(&'a FieldKind),
    /// A node property's type.
    Property(&'a PropertyType),
}

/// The declared type of a slot a binding reads or writes.
#[derive(Clone, PartialEq, Debug)]
pub struct SlotKind {
    /// The node.
    pub node: NodeId,
    /// The property.
    pub property: PropertyName,
    /// Its type.
    pub kind: PropertyType,
}

/// Where the field kinds and slot types come from.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Environment {
    /// The data schema.
    pub schema: Schema,
    /// The slots the bindings may name.
    pub slots: Vec<SlotKind>,
}

impl Environment {
    /// An environment over a schema.
    pub fn new(schema: Schema) -> Self {
        Self {
            schema,
            slots: Vec::new(),
        }
    }

    /// The same environment with one more slot.
    pub fn with_slot(mut self, node: NodeId, property: PropertyName, kind: PropertyType) -> Self {
        self.slots.push(SlotKind {
            node,
            property,
            kind,
        });
        self
    }

    /// The kind of a data field, if it is declared.
    pub fn field_kind(&self, name: &str) -> Option<&FieldKind> {
        self.schema.field(name).map(|field| &field.kind)
    }

    /// The type of a slot, if it is declared.
    pub fn slot_kind(&self, node: NodeId, property: &PropertyName) -> Option<&PropertyType> {
        self.slots
            .iter()
            .find(|slot| slot.node == node && &slot.property == property)
            .map(|slot| &slot.kind)
    }
}

/// One binding: a source, a target, a direction and a conversion.
#[derive(Clone, PartialEq, Debug)]
pub struct Binding {
    /// Where the value comes from.
    pub source: Origin,
    /// Where it goes.
    pub target: Origin,
    /// Which way it flows.
    pub direction: Direction,
    /// How it is carried.
    pub conversion: Conversion,
}

/// Whether a binding may be added.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Verdict {
    /// It may.
    Accepted,
    /// It may not, and this is why.
    Refused {
        /// Why it may not.
        reason: String,
    },
}

impl Verdict {
    /// Whether the binding was accepted.
    pub fn is_accepted(&self) -> bool {
        matches!(self, Self::Accepted)
    }

    /// The refusal's reason, if it was refused.
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Accepted => None,
            Self::Refused { reason } => Some(reason),
        }
    }

    /// The verdict in one line.
    pub fn render(&self, label: &str) -> String {
        match self {
            Self::Accepted => format!("{label}: accepted"),
            Self::Refused { reason } => format!("{label}: REFUSED — {reason}"),
        }
    }
}

/// One binding that could not be admitted.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Diagnostic {
    /// The binding's index in the graph.
    pub binding: usize,
    /// Why it could not be admitted.
    pub reason: String,
}

impl Diagnostic {
    /// The diagnostic in one line.
    pub fn render(&self) -> String {
        format!("binding {}: {}", self.binding, self.reason)
    }
}

/// The bindings of a document.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Graph {
    /// The bindings, in declaration order.
    pub bindings: Vec<Binding>,
}

impl Graph {
    /// A graph with no bindings.
    pub fn new() -> Self {
        Self::default()
    }

    /// The same graph with one more binding.
    pub fn with(mut self, binding: Binding) -> Self {
        self.bindings.push(binding);
        self
    }

    /// Admits a binding against the environment, including whether it would close a cycle.
    pub fn admit(&self, binding: &Binding, environment: &Environment) -> Verdict {
        let source = match carrier(&binding.source, environment) {
            Ok(carrier) => carrier,
            Err(reason) => return Verdict::Refused { reason },
        };
        let target = match carrier(&binding.target, environment) {
            Ok(carrier) => carrier,
            Err(reason) => return Verdict::Refused { reason },
        };
        if binding.direction == Direction::TwoWay && binding.conversion.is_lossy() {
            return Verdict::Refused {
                reason: format!(
                    "a two-way binding through the lossy {} conversion would oscillate",
                    binding.conversion.name()
                ),
            };
        }
        if !binding.conversion.accepts(&source, &target) {
            return Verdict::Refused {
                reason: format!(
                    "the {} conversion does not carry {} to {}",
                    binding.conversion.name(),
                    binding.source.render(),
                    binding.target.render()
                ),
            };
        }
        let mut trial = self.clone();
        trial.bindings.push(binding.clone());
        if let Some(cycle) = trial.cycle() {
            return Verdict::Refused {
                reason: format!("the binding would close a cycle: {}", render_path(&cycle)),
            };
        }
        Verdict::Accepted
    }

    /// The first cycle in the graph, if there is one.
    pub fn cycle(&self) -> Option<Vec<Origin>> {
        find_cycle(&self.edges())
    }

    /// Every binding that could not be admitted, in order.
    ///
    /// The bindings are admitted one at a time against those already accepted, so a cycle is blamed
    /// on the binding that closed it rather than on all of them.
    pub fn check(&self, environment: &Environment) -> Vec<Diagnostic> {
        let mut accepted = Graph::new();
        let mut diagnostics = Vec::new();
        for (index, binding) in self.bindings.iter().enumerate() {
            match accepted.admit(binding, environment) {
                Verdict::Accepted => accepted.bindings.push(binding.clone()),
                Verdict::Refused { reason } => diagnostics.push(Diagnostic {
                    binding: index,
                    reason,
                }),
            }
        }
        diagnostics
    }

    /// The graph in one line per binding.
    pub fn render(&self) -> Vec<String> {
        self.bindings
            .iter()
            .enumerate()
            .map(|(index, binding)| {
                format!(
                    "binding {index}: {} -> {} ({}, {})",
                    binding.source.render(),
                    binding.target.render(),
                    binding.direction.name(),
                    binding.conversion.name()
                )
            })
            .collect()
    }

    fn edges(&self) -> Vec<(Origin, Origin, usize)> {
        let mut edges = Vec::new();
        for (index, binding) in self.bindings.iter().enumerate() {
            edges.push((binding.source.clone(), binding.target.clone(), index));
            if binding.direction == Direction::TwoWay {
                edges.push((binding.target.clone(), binding.source.clone(), index));
            }
        }
        edges
    }
}

/// The kind a source or target holds, or why it cannot be found.
fn carrier<'a>(origin: &Origin, environment: &'a Environment) -> Result<Carrier<'a>, String> {
    match origin {
        Origin::Field(name) => environment
            .field_kind(name)
            .map(Carrier::Field)
            .ok_or_else(|| format!("no data field {name:?} is declared")),
        Origin::Slot { node, property } => environment
            .slot_kind(*node, property)
            .map(Carrier::Property)
            .ok_or_else(|| {
                format!(
                    "node {} declares no property {:?}",
                    node.get(),
                    property.as_str()
                )
            }),
    }
}

/// Whether a value can pass between two carriers with no conversion.
fn compatible(source: &Carrier<'_>, target: &Carrier<'_>) -> bool {
    (is_texty(source) && is_texty(target))
        || (is_numeric(source) && is_numeric(target))
        || (is_variant(source) && is_variant(target))
}

/// Whether a carrier holds text-shaped values.
fn is_texty(carrier: &Carrier<'_>) -> bool {
    matches!(
        carrier,
        Carrier::Field(FieldKind::Text)
            | Carrier::Property(
                PropertyType::Text { .. } | PropertyType::Colour | PropertyType::Asset
            )
    )
}

/// Whether a carrier holds numeric values.
fn is_numeric(carrier: &Carrier<'_>) -> bool {
    matches!(
        carrier,
        Carrier::Field(FieldKind::Integer | FieldKind::Number)
            | Carrier::Property(
                PropertyType::Number { .. } | PropertyType::Integer { .. } | PropertyType::Size
            )
    )
}

/// Whether a carrier holds a choice from a named set.
fn is_variant(carrier: &Carrier<'_>) -> bool {
    matches!(
        carrier,
        Carrier::Field(FieldKind::Enum(_)) | Carrier::Property(PropertyType::Enum { .. })
    )
}

/// The first cycle in a set of directed arcs, as the path that closes it.
///
/// Each arc carries the index of the binding it came from, and a walk may not use the same binding
/// twice. That is what keeps one two-way binding from looking like a cycle: its two arcs are one
/// logical relationship, so `A -> B -> A` through a *single* two-way binding is not a loop, while a
/// two-way binding plus a further one-way binding back is.
fn find_cycle(arcs: &[(Origin, Origin, usize)]) -> Option<Vec<Origin>> {
    fn walk(
        node: &Origin,
        arcs: &[(Origin, Origin, usize)],
        path: &mut Vec<Origin>,
        used: &mut Vec<usize>,
    ) -> Option<Vec<Origin>> {
        for (from, to, index) in arcs {
            if from != node || used.contains(index) {
                continue;
            }
            if let Some(start) = path.iter().position(|seen| seen == to) {
                let mut cycle = path[start..].to_vec();
                cycle.push(to.clone());
                return Some(cycle);
            }
            used.push(*index);
            path.push(to.clone());
            if let Some(cycle) = walk(to, arcs, path, used) {
                return Some(cycle);
            }
            path.pop();
            used.pop();
        }
        None
    }
    let mut starts: Vec<Origin> = Vec::new();
    for (from, _, _) in arcs {
        if !starts.contains(from) {
            starts.push(from.clone());
        }
    }
    for start in &starts {
        if let Some(cycle) = walk(start, arcs, &mut vec![start.clone()], &mut Vec::new()) {
            return Some(cycle);
        }
    }
    None
}

/// A cycle path in one line.
fn render_path(path: &[Origin]) -> String {
    path.iter()
        .map(Origin::render)
        .collect::<Vec<_>>()
        .join(" -> ")
}

#[cfg(test)]
mod tests {
    use super::{
        Binding, Conversion, Direction, Environment, Graph, Origin, Verdict, carrier, find_cycle,
    };
    use crate::app::data::data_model::{FieldKind, Schema};
    use crate::app::style::property_editor::PropertyType;
    use crate::contracts::identity::NodeId;
    use crate::contracts::model::PropertyName;

    fn node(value: u64) -> NodeId {
        NodeId::from_raw(value)
    }

    fn name(text: &str) -> PropertyName {
        PropertyName::new(text).expect("a name")
    }

    fn slot(value: u64, property: &str) -> Origin {
        Origin::Slot {
            node: node(value),
            property: name(property),
        }
    }

    fn field(text: &str) -> Origin {
        Origin::Field(text.to_owned())
    }

    fn binding(
        source: Origin,
        target: Origin,
        direction: Direction,
        conversion: Conversion,
    ) -> Binding {
        Binding {
            source,
            target,
            direction,
            conversion,
        }
    }

    fn text_type() -> PropertyType {
        PropertyType::Text { max_len: 100 }
    }

    fn number_type() -> PropertyType {
        PropertyType::Number {
            min: None,
            max: None,
        }
    }

    fn enum_type() -> PropertyType {
        PropertyType::Enum {
            variants: vec!["a".to_owned(), "b".to_owned()],
        }
    }

    fn environment() -> Environment {
        Environment::new(
            Schema::new("user", 1)
                .with("label", FieldKind::Text, false)
                .with("count", FieldKind::Integer, false)
                .with("ratio", FieldKind::Number, false)
                .with("tone", FieldKind::Enum(vec!["calm".to_owned()]), false)
                .with("busy", FieldKind::Bool, false),
        )
        .with_slot(node(1), name("text"), text_type())
        .with_slot(node(1), name("width"), number_type())
        .with_slot(node(1), name("tint"), PropertyType::Colour)
        .with_slot(node(2), name("state"), enum_type())
        .with_slot(node(2), name("mode"), enum_type())
        .with_slot(node(2), name("label"), text_type())
        .with_slot(node(3), name("kind"), enum_type())
    }

    #[test]
    fn a_compatible_one_way_binding_is_accepted() {
        let graph = Graph::new();
        let one = binding(
            field("count"),
            slot(1, "width"),
            Direction::OneWay,
            Conversion::Identity,
        );
        assert_eq!(graph.admit(&one, &environment()), Verdict::Accepted);
        assert!(graph.admit(&one, &environment()).is_accepted());
    }

    #[test]
    fn a_missing_data_field_is_refused_and_named() {
        let one = binding(
            field("gone"),
            slot(1, "width"),
            Direction::OneWay,
            Conversion::Identity,
        );
        let verdict = Graph::new().admit(&one, &environment());
        assert_eq!(verdict.reason(), Some("no data field \"gone\" is declared"));
    }

    #[test]
    fn an_unknown_target_slot_is_refused() {
        let one = binding(
            field("count"),
            slot(9, "width"),
            Direction::OneWay,
            Conversion::Identity,
        );
        let verdict = Graph::new().admit(&one, &environment());
        assert!(verdict.reason().expect("a reason").contains("node 9"));
    }

    #[test]
    fn an_incompatible_pair_is_refused_and_named() {
        let one = binding(
            field("busy"),
            slot(1, "text"),
            Direction::OneWay,
            Conversion::Identity,
        );
        let verdict = Graph::new().admit(&one, &environment());
        assert!(
            verdict
                .reason()
                .expect("a reason")
                .contains("identity conversion does not carry")
        );
    }

    #[test]
    fn a_declared_text_conversion_bridges_text_and_colour() {
        let one = binding(
            field("label"),
            slot(1, "tint"),
            Direction::OneWay,
            Conversion::Text,
        );
        assert_eq!(Graph::new().admit(&one, &environment()), Verdict::Accepted);
    }

    #[test]
    fn a_scale_conversion_bridges_two_numeric_carriers() {
        let one = binding(
            field("count"),
            slot(1, "width"),
            Direction::OneWay,
            Conversion::Scale(2.0),
        );
        assert_eq!(Graph::new().admit(&one, &environment()), Verdict::Accepted);
    }

    #[test]
    fn a_scale_conversion_between_two_texts_is_refused() {
        let one = binding(
            field("label"),
            slot(1, "text"),
            Direction::OneWay,
            Conversion::Scale(2.0),
        );
        let verdict = Graph::new().admit(&one, &environment());
        assert!(verdict.reason().expect("a reason").contains("scale:2"));
    }

    #[test]
    fn a_two_way_binding_through_a_lossy_conversion_is_refused_as_oscillating() {
        let one = binding(
            field("label"),
            slot(1, "text"),
            Direction::TwoWay,
            Conversion::Text,
        );
        let verdict = Graph::new().admit(&one, &environment());
        assert!(
            verdict
                .reason()
                .expect("a reason")
                .contains("would oscillate")
        );
    }

    #[test]
    fn a_two_way_identity_binding_is_accepted() {
        let one = binding(
            field("label"),
            slot(1, "text"),
            Direction::TwoWay,
            Conversion::Identity,
        );
        assert_eq!(Graph::new().admit(&one, &environment()), Verdict::Accepted);
    }

    #[test]
    fn a_cycle_among_slots_is_refused_and_the_cycle_is_named() {
        let environment = environment();
        let first = binding(
            slot(1, "text"),
            slot(2, "state"),
            Direction::OneWay,
            Conversion::Text,
        );
        let mut graph = Graph::new();
        assert_eq!(graph.admit(&first, &environment), Verdict::Accepted);
        graph.bindings.push(first);
        let back = binding(
            slot(2, "state"),
            slot(1, "text"),
            Direction::OneWay,
            Conversion::Text,
        );
        let verdict = graph.admit(&back, &environment);
        let reason = verdict.reason().expect("a reason");
        assert!(reason.contains("cycle"), "reason: {reason}");
        assert!(reason.contains("node 1.text"));
    }

    #[test]
    fn a_two_way_self_loop_is_refused_as_a_cycle() {
        let one = binding(
            slot(1, "text"),
            slot(1, "text"),
            Direction::TwoWay,
            Conversion::Identity,
        );
        let verdict = Graph::new().admit(&one, &environment());
        assert!(verdict.reason().expect("a reason").contains("cycle"));
    }

    #[test]
    fn a_chain_of_three_bindings_is_accepted() {
        let environment = environment();
        let mut graph = Graph::new();
        for (source, target, conversion) in [
            (slot(1, "text"), slot(2, "state"), Conversion::Text),
            (slot(2, "state"), slot(3, "kind"), Conversion::Identity),
        ] {
            let one = binding(source, target, Direction::OneWay, conversion);
            assert_eq!(graph.admit(&one, &environment), Verdict::Accepted);
            graph.bindings.push(one);
        }
        assert!(graph.cycle().is_none());
    }

    #[test]
    fn check_reports_every_binding_that_cannot_be_admitted() {
        let graph = Graph::new()
            .with(binding(
                field("count"),
                slot(1, "width"),
                Direction::OneWay,
                Conversion::Identity,
            ))
            .with(binding(
                field("gone"),
                slot(1, "width"),
                Direction::OneWay,
                Conversion::Identity,
            ))
            .with(binding(
                field("busy"),
                slot(1, "text"),
                Direction::OneWay,
                Conversion::Identity,
            ));
        let diagnostics = graph.check(&environment());
        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].binding, 1);
        assert_eq!(diagnostics[1].binding, 2);
        assert!(diagnostics[0].render().starts_with("binding 1:"));
    }

    #[test]
    fn an_empty_graph_has_no_cycle() {
        assert!(Graph::new().cycle().is_none());
        assert!(Graph::new().check(&environment()).is_empty());
    }

    #[test]
    fn the_graph_renders_its_bindings() {
        let graph = Graph::new().with(binding(
            field("count"),
            slot(1, "width"),
            Direction::TwoWay,
            Conversion::Identity,
        ));
        assert_eq!(
            graph.render(),
            vec!["binding 0: field count -> node 1.width (two-way, identity)".to_owned()]
        );
    }

    #[test]
    fn a_carrier_is_resolved_from_the_environment() {
        let environment = environment();
        assert!(carrier(&field("count"), &environment).is_ok());
        assert!(carrier(&field("gone"), &environment).is_err());
        assert!(carrier(&slot(1, "width"), &environment).is_ok());
        assert!(carrier(&slot(1, "gone"), &environment).is_err());
    }

    #[test]
    fn conversions_parse_and_reject_a_zero_factor() {
        assert_eq!(Conversion::parse("identity"), Some(Conversion::Identity));
        assert_eq!(Conversion::parse("text"), Some(Conversion::Text));
        assert_eq!(Conversion::parse("scale:2.5"), Some(Conversion::Scale(2.5)));
        assert_eq!(Conversion::parse("scale:0"), None);
        assert_eq!(Conversion::parse("scale:abc"), None);
        assert_eq!(Conversion::parse("blob"), None);
        assert!(!Conversion::Identity.is_lossy());
        assert!(Conversion::Text.is_lossy());
    }

    #[test]
    fn find_cycle_answers_none_for_an_acyclic_graph() {
        let arcs = vec![
            (field("a"), slot(1, "width"), 0),
            (field("b"), slot(1, "text"), 1),
        ];
        assert!(find_cycle(&arcs).is_none());
    }

    #[test]
    fn a_single_two_way_binding_is_not_a_cycle() {
        let environment = environment();
        let identity = binding(
            slot(1, "text"),
            slot(2, "label"),
            Direction::TwoWay,
            Conversion::Identity,
        );
        let mut graph = Graph::new();
        assert_eq!(graph.admit(&identity, &environment), Verdict::Accepted);
        graph.bindings.push(identity);
        assert!(graph.cycle().is_none());
        // The same pair one-way plus a binding back *is* a cycle.
        let there = binding(
            slot(1, "text"),
            slot(2, "label"),
            Direction::OneWay,
            Conversion::Identity,
        );
        let back = binding(
            slot(2, "label"),
            slot(1, "text"),
            Direction::OneWay,
            Conversion::Identity,
        );
        assert!(
            Graph::new()
                .with(there)
                .admit(&back, &environment)
                .reason()
                .is_some()
        );
    }
}
