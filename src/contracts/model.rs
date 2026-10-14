// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The public objects of the design document: contract **C-MODEL**.
//!
//! These are the shapes every view, service and generator shares. They are deliberately small and
//! closed: a node has an identity, a kind, its own properties and its children, and nothing else.
//! Layout constraints, event wiring and style live in their own contracts with their own tasks
//! (T10.x layout, T13.x wiring, T11.x CSS) because folding them in here would give every one of
//! those features a way to grow a private copy of the document.
//!
//! Validation is at the edge, where values arrive: a [`PropertyName`] or [`NodeKind`] cannot be
//! built from an empty or whitespace-only string, so "a property whose name is blank" is not a
//! state the rest of the code has to defend against. Value *ranges* are not checked here — the
//! property schema owns those (T12.04), and inventing a second range table would be exactly the
//! drifting parallel path this project rejects.

use std::collections::BTreeMap;

use super::identity::{NodeId, PageId};

/// A control kind, as named by the capability registry.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct NodeKind(String);

impl NodeKind {
    /// Builds a kind, or `None` when the name is blank or contains whitespace.
    pub fn new(name: &str) -> Option<Self> {
        let trimmed = name.trim();
        if trimmed.is_empty() || trimmed.split_whitespace().count() != 1 {
            return None;
        }
        Some(Self(trimmed.to_owned()))
    }

    /// The kind's name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A property's name, as named by the property schema.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct PropertyName(String);

impl PropertyName {
    /// Builds a name, or `None` when it is blank.
    pub fn new(name: &str) -> Option<Self> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return None;
        }
        Some(Self(trimmed.to_owned()))
    }

    /// The name as written.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A property value. The variants are the value families the property panel edits (whitepaper
/// §17.5); which one a given property accepts comes from its schema, not from this enum.
#[derive(Clone, PartialEq, Debug)]
pub enum PropertyValue {
    /// A flag.
    Bool(bool),
    /// A whole number, such as a pixel dimension.
    Integer(i64),
    /// A fractional number, such as a weight or a scale.
    Number(f64),
    /// Free text, such as a label or a tooltip.
    Text(String),
    /// A choice from a named set, such as an alignment or an icon role.
    Variant(String),
    /// A colour, as written in the project file.
    Color(String),
    /// A reference to a project resource, by project-relative path.
    Asset(String),
}

impl PropertyValue {
    /// The value family's name, for diagnostics and for schema checks.
    pub const fn family(&self) -> &'static str {
        match self {
            Self::Bool(_) => "bool",
            Self::Integer(_) => "integer",
            Self::Number(_) => "number",
            Self::Text(_) => "text",
            Self::Variant(_) => "variant",
            Self::Color(_) => "color",
            Self::Asset(_) => "asset",
        }
    }
}

/// One node of the design document.
///
/// The node owns its properties and the *order* of its children; it does not own the children's
/// contents. Parent and children are kept consistent by the store's single apply path, so a node
/// never has to be fixed up in two places.
#[derive(Clone, PartialEq, Debug)]
pub struct Node {
    id: NodeId,
    page: PageId,
    kind: NodeKind,
    properties: BTreeMap<PropertyName, PropertyValue>,
    children: Vec<NodeId>,
    parent: Option<NodeId>,
}

impl Node {
    /// A node with no properties, no children and no parent. The store attaches it to the document.
    pub fn new(page: PageId, id: NodeId, kind: NodeKind) -> Self {
        Self {
            id,
            page,
            kind,
            properties: BTreeMap::new(),
            children: Vec::new(),
            parent: None,
        }
    }

    /// A record of a node as a document holds it: identity, kind and properties, with no parent and
    /// no children.
    ///
    /// What a change operation carries so that undoing it can put the *same* node back: a removal
    /// records the node it took out, and the inverse is an addition of that record. The store is
    /// still what attaches a node to a document — this only makes the record.
    pub fn record(
        page: PageId,
        id: NodeId,
        kind: NodeKind,
        properties: BTreeMap<PropertyName, PropertyValue>,
    ) -> Self {
        Self {
            id,
            page,
            kind,
            properties,
            children: Vec::new(),
            parent: None,
        }
    }

    /// The node's identity.
    pub fn id(&self) -> NodeId {
        self.id
    }

    /// The page this node belongs to.
    pub fn page(&self) -> PageId {
        self.page
    }

    /// The control kind, as named by the capability registry.
    pub fn kind(&self) -> &NodeKind {
        &self.kind
    }

    /// The node's properties.
    pub fn properties(&self) -> &BTreeMap<PropertyName, PropertyValue> {
        &self.properties
    }

    /// One property, if it is set on this node.
    pub fn property(&self, name: &PropertyName) -> Option<&PropertyValue> {
        self.properties.get(name)
    }

    /// The node's children, in layout order.
    pub fn children(&self) -> &[NodeId] {
        &self.children
    }

    /// The node's parent, or `None` for a page's root node.
    pub fn parent(&self) -> Option<NodeId> {
        self.parent
    }

    /// Sets a property. Crate-internal: the store's apply path is the only caller, which is what
    /// keeps "one writable model" true rather than merely intended.
    pub(crate) fn set_property(
        &mut self,
        name: PropertyName,
        value: PropertyValue,
    ) -> Option<PropertyValue> {
        self.properties.insert(name, value)
    }

    /// Removes a property, returning what was there.
    pub(crate) fn remove_property(&mut self, name: &PropertyName) -> Option<PropertyValue> {
        self.properties.remove(name)
    }

    /// The parent link, written only by the store.
    pub(crate) fn set_parent(&mut self, parent: Option<NodeId>) {
        self.parent = parent;
    }

    /// The children list, written only by the store.
    pub(crate) fn set_children(&mut self, children: Vec<NodeId>) {
        self.children = children;
    }
}

/// A page: one window's or screen's worth of controls, with a root node.
#[derive(Clone, PartialEq, Debug)]
pub struct Page {
    id: PageId,
    name: String,
    root: NodeId,
}

impl Page {
    /// A page whose root node is already known.
    pub fn new(id: PageId, name: &str, root: NodeId) -> Option<Self> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return None;
        }
        Some(Self {
            id,
            name: trimmed.to_owned(),
            root,
        })
    }

    /// The page's identity.
    pub fn id(&self) -> PageId {
        self.id
    }

    /// The page's display name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The page's root node.
    pub fn root(&self) -> NodeId {
        self.root
    }
}

#[cfg(test)]
mod tests {
    use super::{Node, NodeKind, Page, PropertyName, PropertyValue};
    use crate::contracts::identity::{IdAllocator, PageId};

    #[test]
    fn a_blank_kind_or_property_name_is_not_a_value() {
        assert!(NodeKind::new("").is_none());
        assert!(NodeKind::new("   ").is_none());
        assert!(NodeKind::new("two words").is_none());
        assert!(PropertyName::new("").is_none());
        assert!(PropertyName::new("\t").is_none());
    }

    #[test]
    fn kinds_and_property_names_are_trimmed_on_the_way_in() {
        assert_eq!(NodeKind::new("  button  ").unwrap().as_str(), "button");
        assert_eq!(PropertyName::new(" text ").unwrap().as_str(), "text");
    }

    #[test]
    fn a_blank_page_name_is_not_a_page() {
        let root = IdAllocator::new().allocate();

        assert!(Page::new(PageId::from_raw(0), "  ", root).is_none());
        let page = Page::new(PageId::from_raw(0), "main", root).unwrap();
        assert_eq!(page.name(), "main");
        assert_eq!(page.root(), root);
    }

    #[test]
    fn a_fresh_node_has_nothing_but_its_identity() {
        let mut allocator = IdAllocator::new();
        let page = PageId::from_raw(0);
        let node = Node::new(page, allocator.allocate(), NodeKind::new("button").unwrap());

        assert_eq!(node.page(), page);
        assert_eq!(node.kind().as_str(), "button");
        assert!(node.properties().is_empty());
        assert!(node.children().is_empty());
        assert_eq!(node.parent(), None);
    }

    #[test]
    fn value_families_are_named_for_diagnostics() {
        let values = [
            (PropertyValue::Bool(true), "bool"),
            (PropertyValue::Integer(3), "integer"),
            (PropertyValue::Number(1.5), "number"),
            (PropertyValue::Text("ok".into()), "text"),
            (PropertyValue::Variant("center".into()), "variant"),
            (PropertyValue::Color("#000000".into()), "color"),
            (PropertyValue::Asset("assets/logo.svg".into()), "asset"),
        ];

        for (value, family) in values {
            assert_eq!(value.family(), family);
        }
    }
}
