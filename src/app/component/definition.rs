// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Component definitions, instances, parameters and slots, with the cycle check that keeps a
//! definition from containing itself (task T12.01).
//!
//! The acceptance line is *"expanding to generation keeps instance identity and source mapping"*.
//! Three decisions make that true:
//!
//! * **An instance owns a stable [`NodeId`].** [`Instance`] carries the id the document already
//!   gave it, and [`Registry::expand`] uses that same id for the instance's own produced node, so
//!   expanding never mints a fresh identity for a selection, a source map or an undo step to
//!   chase.
//! * **Expansion records who owns what.** Every produced node carries the id of the instance that
//!   owns it, and [`Expansion::owner_of`] answers the same question from the resolution record, so
//!   "which instance wrote this construction" survives expansion — that record *is* the source
//!   mapping the acceptance line asks for, expressed as identity until a generator assigns byte
//!   spans.
//! * **A definition cannot contain itself.** [`Registry::insert`] walks the nested references and
//!   refuses a definition that reaches back to itself, naming the cycle path (`a -> b -> a`),
//!   because an unbounded expansion is a stack overflow dressed as a feature.
//!
//! Child nodes come from the document's [`IdAllocator`] rather than from arithmetic on the
//! instance id, so expansion cannot hand out an id the document already issued. A slot fill is
//! substituted only into the definition the instance is *of*; a nested reference resolves with its
//! own empty fill table, which keeps substitution local and cheap to reason about.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use crate::contracts::identity::{IdAllocator, NodeId};
use crate::contracts::model::{NodeKind, PropertyName, PropertyValue};

/// One declared parameter of a definition.
#[derive(Clone, PartialEq, Debug)]
pub struct Parameter {
    /// The parameter's name; it becomes a property name on the produced node.
    pub name: String,
    /// The value used when an instance leaves the argument out, if any.
    pub default: Option<PropertyValue>,
}

impl Parameter {
    /// Builds a parameter, or `None` when the name is blank.
    pub fn new(name: &str, default: Option<PropertyValue>) -> Option<Self> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return None;
        }
        Some(Self {
            name: trimmed.to_owned(),
            default,
        })
    }
}

/// One declared slot of a definition: a hole a placed instance may fill.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Slot {
    /// The slot's name.
    pub name: String,
}

impl Slot {
    /// Builds a slot, or `None` when the name is blank.
    pub fn new(name: &str) -> Option<Self> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return None;
        }
        Some(Self {
            name: trimmed.to_owned(),
        })
    }
}

/// One piece of a definition's body.
#[derive(Clone, PartialEq, Debug)]
pub enum Part {
    /// A concrete control, with its properties.
    Control {
        /// The control kind.
        kind: NodeKind,
        /// The control's properties.
        properties: BTreeMap<PropertyName, PropertyValue>,
    },
    /// A reference to another definition, expanded in the owner's identity.
    Nested {
        /// The referenced definition's name.
        definition: String,
    },
    /// A hole filled by the placed instance.
    Slot {
        /// The slot's name.
        name: String,
    },
}

/// A reusable component definition.
#[derive(Clone, PartialEq, Debug)]
pub struct Definition {
    /// The definition's name; it doubles as the produced root's control kind.
    pub name: String,
    /// The definition's version, bumped by an edit.
    pub version: u32,
    /// The declared parameters, in declaration order.
    pub parameters: Vec<Parameter>,
    /// The declared slots, in declaration order.
    pub slots: Vec<Slot>,
    /// The body, in order.
    pub body: Vec<Part>,
}

impl Definition {
    /// Builds a definition, or `None` when the name is blank or a parameter or slot name repeats.
    pub fn new(
        name: &str,
        version: u32,
        parameters: Vec<Parameter>,
        slots: Vec<Slot>,
        body: Vec<Part>,
    ) -> Option<Self> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return None;
        }
        let mut parameter_names = BTreeSet::new();
        for parameter in &parameters {
            if !parameter_names.insert(parameter.name.clone()) {
                return None;
            }
        }
        let mut slot_names = BTreeSet::new();
        for slot in &slots {
            if !slot_names.insert(slot.name.clone()) {
                return None;
            }
        }
        Some(Self {
            name: trimmed.to_owned(),
            version,
            parameters,
            slots,
            body,
        })
    }

    /// One declared parameter, if this definition declares it.
    pub fn parameter(&self, name: &str) -> Option<&Parameter> {
        self.parameters
            .iter()
            .find(|parameter| parameter.name == name)
    }

    /// One declared slot, if this definition declares it.
    pub fn slot(&self, name: &str) -> Option<&Slot> {
        self.slots.iter().find(|slot| slot.name == name)
    }

    /// The definitions this one references directly, in body order.
    pub fn nested(&self) -> Vec<String> {
        self.body
            .iter()
            .filter_map(|part| match part {
                Part::Nested { definition } => Some(definition.clone()),
                Part::Control { .. } | Part::Slot { .. } => None,
            })
            .collect()
    }
}

/// A placed use of a definition, with the document identity it keeps.
#[derive(Clone, PartialEq, Debug)]
pub struct Instance {
    /// The definition this instance is of.
    pub definition: String,
    /// The instance's stable identity in the document.
    pub id: NodeId,
    /// The argument bound to each parameter, by parameter name.
    pub arguments: BTreeMap<String, PropertyValue>,
    /// The body substituted into each slot, by slot name.
    pub fills: BTreeMap<String, Vec<Part>>,
}

impl Instance {
    /// An instance of a definition under an identity, with no arguments and no fills.
    pub fn new(definition: &str, id: NodeId) -> Self {
        Self {
            definition: definition.to_owned(),
            id,
            arguments: BTreeMap::new(),
            fills: BTreeMap::new(),
        }
    }
}

/// One concrete node produced by expansion.
#[derive(Clone, PartialEq, Debug)]
pub struct ExpandedNode {
    /// The produced node's identity.
    pub id: NodeId,
    /// The produced node's control kind.
    pub kind: NodeKind,
    /// The produced node's properties.
    pub properties: BTreeMap<PropertyName, PropertyValue>,
    /// The instance that owns this node.
    pub owner: NodeId,
}

/// Which instance owns each produced node: the identity record expansion leaves behind.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Mapping {
    entries: BTreeMap<NodeId, NodeId>,
}

impl Mapping {
    /// Records that `node` is owned by `owner`.
    fn insert(&mut self, node: NodeId, owner: NodeId) {
        self.entries.insert(node, owner);
    }

    /// The instance that owns a produced node, if it was produced here.
    pub fn owner_of(&self, node: NodeId) -> Option<NodeId> {
        self.entries.get(&node).copied()
    }

    /// How many nodes are mapped.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether nothing is mapped.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// The concrete nodes an instance expands to, and the identity record that accompanies them.
#[derive(Clone, PartialEq, Debug)]
pub struct Expansion {
    /// The produced nodes, the instance's own node first.
    pub nodes: Vec<ExpandedNode>,
    /// Which instance owns each produced node.
    pub mapping: Mapping,
}

impl Expansion {
    /// The instance's own produced node, which keeps the instance's id.
    pub fn root(&self) -> Option<&ExpandedNode> {
        self.nodes.first()
    }

    /// The instance that owns a produced node.
    pub fn owner_of(&self, node: NodeId) -> Option<NodeId> {
        self.mapping.owner_of(node)
    }

    fn push(&mut self, node: ExpandedNode) {
        self.mapping.insert(node.id, node.owner);
        self.nodes.push(node);
    }
}

/// A set of definitions and the instances placed from them.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Registry {
    definitions: BTreeMap<String, Definition>,
    instances: BTreeMap<NodeId, Instance>,
}

impl Registry {
    /// A registry with nothing in it.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds or replaces a definition, refusing one that reaches back to itself.
    ///
    /// The graph of registered definitions is acyclic by induction, so a new cycle can only run
    /// through the definition being inserted; a depth-first walk from its name finds it and names
    /// the path it took.
    pub fn insert(&mut self, definition: Definition) -> Result<(), String> {
        let name = definition.name.clone();
        let mut edges: BTreeMap<String, Vec<String>> = self
            .definitions
            .iter()
            .map(|(key, value)| (key.clone(), value.nested()))
            .collect();
        edges.insert(name.clone(), definition.nested());
        if let Some(path) = find_cycle(&name, &edges) {
            return Err(format!(
                "definition {name:?} contains itself: {}",
                path.join(" -> ")
            ));
        }
        self.definitions.insert(name, definition);
        Ok(())
    }

    /// One definition, if it is registered.
    pub fn definition(&self, name: &str) -> Option<&Definition> {
        self.definitions.get(name)
    }

    /// The registered definition names, in order.
    pub fn names(&self) -> Vec<&str> {
        self.definitions.keys().map(String::as_str).collect()
    }

    /// Places an instance, refusing an unknown definition, an argument no parameter accepts, a
    /// parameter with no default and no argument, and a fill for a slot that is not declared.
    pub fn place(&mut self, instance: Instance) -> Result<(), String> {
        let name = instance.definition.clone();
        let definition = self
            .definitions
            .get(&name)
            .ok_or_else(|| format!("no definition {name:?}"))?;
        for argument in instance.arguments.keys() {
            if definition.parameter(argument).is_none() {
                return Err(format!("definition {name:?} has no parameter {argument:?}"));
            }
        }
        for parameter in &definition.parameters {
            if parameter.default.is_none() && !instance.arguments.contains_key(&parameter.name) {
                return Err(format!(
                    "parameter {:?} of {name:?} needs an argument",
                    parameter.name
                ));
            }
        }
        for slot in instance.fills.keys() {
            if definition.slot(slot).is_none() {
                return Err(format!("definition {name:?} has no slot {slot:?}"));
            }
        }
        self.instances.insert(instance.id, instance);
        Ok(())
    }

    /// One placed instance, if it exists.
    pub fn instance(&self, id: NodeId) -> Option<&Instance> {
        self.instances.get(&id)
    }

    /// The identities of instances of a definition, in identity order.
    pub fn instances_of(&self, definition: &str) -> Vec<NodeId> {
        self.instances
            .values()
            .filter(|instance| instance.definition == definition)
            .map(|instance| instance.id)
            .collect()
    }

    /// Expands an instance into concrete nodes, keeping its identity in a resolution record.
    pub fn expand(&self, id: NodeId, allocator: &mut IdAllocator) -> Result<Expansion, String> {
        let instance = self
            .instances
            .get(&id)
            .ok_or_else(|| format!("no instance {}", id.get()))?
            .clone();
        let definition = self
            .definitions
            .get(&instance.definition)
            .ok_or_else(|| format!("no definition {:?}", instance.definition))?
            .clone();
        let kind = NodeKind::new(&definition.name).ok_or_else(|| {
            format!(
                "definition name {:?} is not a single control kind",
                definition.name
            )
        })?;
        allocator.adopt(id);
        let mut properties = BTreeMap::new();
        for parameter in &definition.parameters {
            let value = instance
                .arguments
                .get(&parameter.name)
                .cloned()
                .or_else(|| parameter.default.clone());
            if let Some(value) = value
                && let Some(name) = PropertyName::new(&parameter.name)
            {
                properties.insert(name, value);
            }
        }
        let mut expansion = Expansion {
            nodes: Vec::new(),
            mapping: Mapping::default(),
        };
        expansion.push(ExpandedNode {
            id,
            kind,
            properties,
            owner: id,
        });
        self.expand_parts(
            &definition.body,
            &instance.fills,
            id,
            allocator,
            &mut expansion,
        )?;
        Ok(expansion)
    }

    fn expand_parts(
        &self,
        parts: &[Part],
        fills: &BTreeMap<String, Vec<Part>>,
        owner: NodeId,
        allocator: &mut IdAllocator,
        expansion: &mut Expansion,
    ) -> Result<(), String> {
        for part in parts {
            match part {
                Part::Control { kind, properties } => {
                    let node = allocator.allocate();
                    expansion.push(ExpandedNode {
                        id: node,
                        kind: kind.clone(),
                        properties: properties.clone(),
                        owner,
                    });
                }
                Part::Nested { definition } => {
                    let nested = self
                        .definitions
                        .get(definition)
                        .ok_or_else(|| format!("no definition {definition:?}"))?
                        .clone();
                    self.expand_parts(&nested.body, &BTreeMap::new(), owner, allocator, expansion)?;
                }
                Part::Slot { name } => {
                    if let Some(filled) = fills.get(name) {
                        self.expand_parts(filled, &BTreeMap::new(), owner, allocator, expansion)?;
                    }
                }
            }
        }
        Ok(())
    }
}

/// Finds a cycle reachable from `start`, returning the path with `start` at both ends.
fn find_cycle(start: &str, edges: &BTreeMap<String, Vec<String>>) -> Option<Vec<String>> {
    fn walk(
        node: &str,
        edges: &BTreeMap<String, Vec<String>>,
        path: &mut Vec<String>,
    ) -> Option<Vec<String>> {
        if let Some(index) = path.iter().position(|seen| seen == node) {
            let mut cycle = path[index..].to_vec();
            cycle.push(node.to_owned());
            return Some(cycle);
        }
        path.push(node.to_owned());
        if let Some(children) = edges.get(node) {
            for child in children {
                if let Some(cycle) = walk(child, edges, path) {
                    return Some(cycle);
                }
            }
        }
        path.pop();
        None
    }
    walk(start, edges, &mut Vec::new())
}

#[cfg(test)]
mod tests {
    use super::{Definition, Instance, Parameter, Part, Registry, Slot};
    use crate::contracts::identity::{IdAllocator, NodeId};
    use crate::contracts::model::{NodeKind, PropertyName, PropertyValue};
    use std::collections::BTreeMap;

    fn control(kind: &str) -> Part {
        Part::Control {
            kind: NodeKind::new(kind).expect("a kind"),
            properties: BTreeMap::new(),
        }
    }

    fn nested(definition: &str) -> Part {
        Part::Nested {
            definition: definition.to_owned(),
        }
    }

    fn parameter(name: &str) -> Parameter {
        Parameter::new(name, None).expect("a parameter")
    }

    #[test]
    fn a_blank_name_or_a_duplicate_member_is_not_a_definition() {
        assert!(Definition::new("  ", 1, Vec::new(), Vec::new(), Vec::new()).is_none());
        assert!(
            Definition::new(
                "button",
                1,
                vec![parameter("text"), parameter("text")],
                Vec::new(),
                Vec::new()
            )
            .is_none()
        );
        let slot = Slot::new("body").expect("a slot");
        assert!(
            Definition::new("row", 1, Vec::new(), vec![slot.clone(), slot], Vec::new()).is_none()
        );
    }

    #[test]
    fn a_definition_that_contains_itself_is_refused() {
        let mut registry = Registry::new();
        let definition =
            Definition::new("a", 1, Vec::new(), Vec::new(), vec![nested("a")]).expect("a");
        let reason = registry.insert(definition).expect_err("a self cycle");
        assert!(reason.contains("a -> a"), "{reason}");
    }

    #[test]
    fn a_transitive_cycle_names_the_path() {
        let mut registry = Registry::new();
        let a = Definition::new("a", 1, Vec::new(), Vec::new(), vec![nested("b")]).expect("a");
        registry.insert(a).expect("no cycle yet");
        let b = Definition::new("b", 1, Vec::new(), Vec::new(), vec![nested("a")]).expect("b");
        let reason = registry.insert(b).expect_err("a cycle");
        assert!(reason.contains("b -> a -> b"), "{reason}");
    }

    #[test]
    fn an_unknown_argument_is_refused() {
        let mut registry = Registry::new();
        let definition = Definition::new(
            "button",
            1,
            vec![Parameter::new("text", Some(PropertyValue::Text("Ok".to_owned()))).expect("a")],
            Vec::new(),
            Vec::new(),
        )
        .expect("a definition");
        registry.insert(definition).expect("no cycle");

        let mut instance = Instance::new("button", NodeId::from_raw(1));
        instance
            .arguments
            .insert("size".to_owned(), PropertyValue::Integer(3));
        let reason = registry.place(instance).expect_err("an unknown argument");
        assert!(reason.contains("no parameter"), "{reason}");
    }

    #[test]
    fn a_parameter_without_a_default_needs_an_argument() {
        let mut registry = Registry::new();
        let definition =
            Definition::new("button", 1, vec![parameter("text")], Vec::new(), Vec::new())
                .expect("a definition");
        registry.insert(definition).expect("no cycle");
        let reason = registry
            .place(Instance::new("button", NodeId::from_raw(1)))
            .expect_err("a missing argument");
        assert!(reason.contains("needs an argument"), "{reason}");
    }

    #[test]
    fn a_fill_for_an_undeclared_slot_is_refused() {
        let mut registry = Registry::new();
        let definition =
            Definition::new("button", 1, Vec::new(), Vec::new(), Vec::new()).expect("a definition");
        registry.insert(definition).expect("no cycle");
        let mut instance = Instance::new("button", NodeId::from_raw(1));
        instance
            .fills
            .insert("header".to_owned(), vec![control("label")]);
        let reason = registry.place(instance).expect_err("an unknown slot");
        assert!(reason.contains("no slot"), "{reason}");
    }

    #[test]
    fn expanding_keeps_the_instance_identity_and_the_source_mapping() {
        let mut registry = Registry::new();
        let definition = Definition::new(
            "button",
            2,
            vec![Parameter::new("text", Some(PropertyValue::Text("Ok".to_owned()))).expect("a")],
            Vec::new(),
            vec![control("label")],
        )
        .expect("a definition");
        registry.insert(definition).expect("no cycle");
        registry
            .place(Instance::new("button", NodeId::from_raw(7)))
            .expect("a placement");

        let mut allocator = IdAllocator::new();
        let expansion = registry
            .expand(NodeId::from_raw(7), &mut allocator)
            .expect("an expansion");

        let root = expansion.root().expect("a root");
        assert_eq!(root.id, NodeId::from_raw(7));
        assert_eq!(root.kind.as_str(), "button");
        assert_eq!(expansion.nodes.len(), 2);
        for node in &expansion.nodes {
            assert_eq!(expansion.owner_of(node.id), Some(NodeId::from_raw(7)));
        }
        assert_eq!(
            root.properties
                .get(&PropertyName::new("text").expect("a name")),
            Some(&PropertyValue::Text("Ok".to_owned())),
        );
    }

    #[test]
    fn a_slot_fill_is_substituted_into_the_instance() {
        let mut registry = Registry::new();
        let definition = Definition::new(
            "row",
            1,
            Vec::new(),
            vec![Slot::new("body").expect("a slot")],
            vec![
                control("label"),
                Part::Slot {
                    name: "body".to_owned(),
                },
            ],
        )
        .expect("a definition");
        registry.insert(definition).expect("no cycle");
        let mut instance = Instance::new("row", NodeId::from_raw(1));
        instance
            .fills
            .insert("body".to_owned(), vec![control("icon")]);
        registry.place(instance).expect("a placement");

        let mut allocator = IdAllocator::new();
        let expansion = registry
            .expand(NodeId::from_raw(1), &mut allocator)
            .expect("an expansion");
        let kinds: Vec<&str> = expansion
            .nodes
            .iter()
            .map(|node| node.kind.as_str())
            .collect();
        assert_eq!(kinds, vec!["row", "label", "icon"]);
    }

    #[test]
    fn a_nested_definition_expands_in_the_owners_identity() {
        let mut registry = Registry::new();
        let cell = Definition::new("cell", 1, Vec::new(), Vec::new(), vec![control("label")])
            .expect("cell");
        registry.insert(cell).expect("no cycle");
        let row = Definition::new(
            "row",
            1,
            Vec::new(),
            Vec::new(),
            vec![nested("cell"), control("spacer")],
        )
        .expect("row");
        registry.insert(row).expect("no cycle");
        registry
            .place(Instance::new("row", NodeId::from_raw(5)))
            .expect("a placement");

        let mut allocator = IdAllocator::new();
        let expansion = registry
            .expand(NodeId::from_raw(5), &mut allocator)
            .expect("an expansion");
        let kinds: Vec<&str> = expansion
            .nodes
            .iter()
            .map(|node| node.kind.as_str())
            .collect();
        assert_eq!(kinds, vec!["row", "label", "spacer"]);
        assert!(
            expansion
                .nodes
                .iter()
                .all(|node| node.owner == NodeId::from_raw(5))
        );
    }

    #[test]
    fn an_unknown_instance_is_refused_on_expansion() {
        let registry = Registry::new();
        let mut allocator = IdAllocator::new();
        assert!(
            registry
                .expand(NodeId::from_raw(9), &mut allocator)
                .is_err()
        );
    }
}
