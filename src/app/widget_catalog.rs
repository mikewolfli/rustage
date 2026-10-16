// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The widget catalog: the design palette's control kinds, drawn from the contract (task T09.01).
//!
//! The acceptance line is *"the names come from the contract, and an unsupported item has a reason"*.
//! Both are about not inventing things, so the catalog invents nothing:
//!
//! * **Every name is a contract name.** A [`WidgetDescriptor`] carries the id and display name the
//!   contract gave it; the catalog indexes those and never synthesises one. A query for a kind the
//!   contract does not describe answers nothing rather than a plausible-looking entry.
//! * **An unsupported control says why.** [`Support`] is either [`Supported`](Support::Supported) or
//!   [`Unsupported`](Support::Unsupported) with a reason, and [`Catalog::reason`] answers it — the same
//!   shape [T05.04](crate::app::palette) uses for commands, so a greyed-out control is explainable.
//! * **What the contract cannot express is listed, not dropped.** [`WidgetDescriptor::limits`] holds
//!   the properties the contract has no schema for, with a note, so "not offered" is never mistaken for
//!   "does not exist".
//! * **Search is the one fuzzy rule.** [`Catalog::search`] reuses
//!   [`fuzzy_score`](crate::app::quick_open::fuzzy_score) over the id, name and category, so the
//!   palette and quick open rank the same way; favorites lift a kind but never hide the rest.
//!
//! The contract itself is [T00.01](crate::host)'s probe of the pinned library; a real build reads it
//! from that artifact, which is why this module takes descriptors as input rather than carrying a list.

#![allow(dead_code)]

use std::collections::BTreeSet;

use crate::app::quick_open::fuzzy_score;

/// The type a property's value has.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ValueType {
    /// A number.
    Number,
    /// Free text.
    Text,
    /// A boolean.
    Bool,
    /// A colour.
    Color,
    /// A length (points, percent, `auto`).
    Dimension,
    /// One of a fixed set.
    Enum(Vec<String>),
    /// A reference to a resource of a kind.
    Resource {
        /// The resource kind (icon, image, font, ...).
        kind: String,
    },
}

impl ValueType {
    /// The type's name.
    pub fn name(&self) -> String {
        match self {
            Self::Number => "number".to_owned(),
            Self::Text => "text".to_owned(),
            Self::Bool => "bool".to_owned(),
            Self::Color => "color".to_owned(),
            Self::Dimension => "dimension".to_owned(),
            Self::Enum(values) => format!("enum({})", values.join(",")),
            Self::Resource { kind } => format!("resource({kind})"),
        }
    }
}

/// One property a control exposes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PropertySchema {
    /// The property's name, from the contract.
    pub name: String,
    /// The value's type.
    pub value_type: ValueType,
    /// The default value, when the contract gives one.
    pub default: Option<String>,
}

impl PropertySchema {
    /// A property.
    pub fn new(name: impl Into<String>, value_type: ValueType) -> Self {
        Self {
            name: name.into(),
            value_type,
            default: None,
        }
    }

    /// The same property with a default.
    pub fn with_default(mut self, default: impl Into<String>) -> Self {
        self.default = Some(default.into());
        self
    }

    /// The property in one line.
    pub fn render(&self) -> String {
        match &self.default {
            Some(default) => format!("{}: {} = {default}", self.name, self.value_type.name()),
            None => format!("{}: {}", self.name, self.value_type.name()),
        }
    }
}

/// Whether the current target can place a control.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Support {
    /// It can be placed.
    Supported,
    /// It cannot, for a named reason.
    Unsupported {
        /// Why.
        reason: String,
    },
}

impl Support {
    /// Whether it is supported.
    pub fn is_supported(&self) -> bool {
        matches!(self, Self::Supported)
    }

    /// The reason, when there is one.
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Supported => None,
            Self::Unsupported { reason } => Some(reason),
        }
    }
}

/// One control kind, as the contract describes it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct WidgetDescriptor {
    /// The contract's id (its stable name).
    pub id: String,
    /// The display name, from the contract.
    pub name: String,
    /// The category it is grouped under.
    pub category: String,
    /// The properties it exposes.
    pub properties: Vec<PropertySchema>,
    /// Whether the target can place it.
    pub support: Support,
    /// What the contract cannot express, with a note.
    pub limits: Vec<String>,
}

impl WidgetDescriptor {
    /// A supported control.
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        category: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            category: category.into(),
            properties: Vec::new(),
            support: Support::Supported,
            limits: Vec::new(),
        }
    }

    /// The same descriptor, unsupported for a reason.
    pub fn unsupported(mut self, reason: impl Into<String>) -> Self {
        self.support = Support::Unsupported {
            reason: reason.into(),
        };
        self
    }

    /// The same descriptor with a property.
    pub fn with_property(mut self, property: PropertySchema) -> Self {
        self.properties.push(property);
        self
    }

    /// The same descriptor with a limit note.
    pub fn with_limit(mut self, note: impl Into<String>) -> Self {
        self.limits.push(note.into());
        self
    }

    /// The descriptor in one line.
    pub fn render(&self) -> String {
        match self.support.reason() {
            Some(reason) => format!("{} [{}] — unsupported: {reason}", self.name, self.category),
            None => format!("{} [{}]", self.name, self.category),
        }
    }
}

/// The catalog of control kinds the contract describes.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Catalog {
    descriptors: Vec<WidgetDescriptor>,
    favorites: BTreeSet<String>,
}

impl Catalog {
    /// A catalog over contract descriptors.
    pub fn of(descriptors: Vec<WidgetDescriptor>) -> Self {
        Self {
            descriptors,
            favorites: BTreeSet::new(),
        }
    }

    /// How many kinds it describes.
    pub fn len(&self) -> usize {
        self.descriptors.len()
    }

    /// Whether it describes none.
    pub fn is_empty(&self) -> bool {
        self.descriptors.is_empty()
    }

    /// A descriptor by its contract id.
    pub fn get(&self, id: &str) -> Option<&WidgetDescriptor> {
        self.descriptors
            .iter()
            .find(|descriptor| descriptor.id == id)
    }

    /// The categories, in order.
    pub fn categories(&self) -> BTreeSet<&str> {
        self.descriptors
            .iter()
            .map(|descriptor| descriptor.category.as_str())
            .collect()
    }

    /// The kinds in a category.
    pub fn in_category(&self, category: &str) -> Vec<&WidgetDescriptor> {
        self.descriptors
            .iter()
            .filter(|descriptor| descriptor.category == category)
            .collect()
    }

    /// Searches by a query, best first, lifting favorites.
    ///
    /// Only what the contract describes can appear: there is no entry to fall back to.
    pub fn search(&self, query: &str) -> Vec<&WidgetDescriptor> {
        let mut hits: Vec<(i64, bool, &WidgetDescriptor)> = self
            .descriptors
            .iter()
            .filter_map(|descriptor| {
                let haystack = format!(
                    "{} {} {}",
                    descriptor.id, descriptor.name, descriptor.category
                );
                fuzzy_score(query, &haystack).map(|score| {
                    let favorite = self.favorites.contains(&descriptor.id);
                    (score, favorite, descriptor)
                })
            })
            .collect();
        hits.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)).then(a.2.id.cmp(&b.2.id)));
        hits.into_iter()
            .map(|(_, _, descriptor)| descriptor)
            .collect()
    }

    /// Marks a kind a favorite, or not.
    pub fn favorite(&mut self, id: &str, on: bool) {
        if on {
            self.favorites.insert(id.to_owned());
        } else {
            self.favorites.remove(id);
        }
    }

    /// Whether a kind is a favorite.
    pub fn is_favorite(&self, id: &str) -> bool {
        self.favorites.contains(id)
    }

    /// The unsupported kinds, each with its reason.
    pub fn unsupported(&self) -> Vec<&WidgetDescriptor> {
        self.descriptors
            .iter()
            .filter(|descriptor| !descriptor.support.is_supported())
            .collect()
    }

    /// Why a kind cannot be placed, when it cannot.
    pub fn reason(&self, id: &str) -> Option<&str> {
        self.get(id)
            .and_then(|descriptor| descriptor.support.reason())
    }

    /// A kind's property schema.
    pub fn schema(&self, id: &str) -> Option<&[PropertySchema]> {
        self.get(id)
            .map(|descriptor| descriptor.properties.as_slice())
    }

    /// The catalog in one report.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "catalog: {} kind(s), {} unsupported, {} favorite(s)",
            self.descriptors.len(),
            self.unsupported().len(),
            self.favorites.len()
        )];
        for descriptor in &self.descriptors {
            lines.push(format!("  {}", descriptor.render()));
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{Catalog, PropertySchema, Support, ValueType, WidgetDescriptor};

    fn catalog() -> Catalog {
        Catalog::of(vec![
            WidgetDescriptor::new("button", "Button", "controls")
                .with_property(PropertySchema::new("label", ValueType::Text).with_default("Button"))
                .with_property(PropertySchema::new(
                    "variant",
                    ValueType::Enum(vec!["primary".to_owned(), "ghost".to_owned()]),
                )),
            WidgetDescriptor::new("image", "Image", "media")
                .with_property(PropertySchema::new(
                    "source",
                    ValueType::Resource {
                        kind: "image".to_owned(),
                    },
                ))
                .unsupported("images need a device profile this target does not have")
                .with_limit("animated formats are not expressible in the contract"),
        ])
    }

    #[test]
    fn names_come_from_the_contract() {
        let catalog = catalog();
        assert_eq!(catalog.len(), 2);
        // A name the contract gave is found; one it did not gives nothing.
        assert_eq!(catalog.get("button").expect("button").name, "Button");
        assert!(catalog.get("invented").is_none());
        // A search for a name not in the contract returns nothing, not a plausible entry.
        assert!(catalog.search("slider").is_empty());
    }

    #[test]
    fn an_unsupported_control_has_a_reason() {
        let catalog = catalog();
        assert_eq!(catalog.unsupported().len(), 1);
        let reason = catalog.reason("image").expect("a reason");
        assert!(reason.contains("device profile"));
        // A supported control has no reason.
        assert_eq!(catalog.reason("button"), None);
        assert!(
            catalog
                .get("button")
                .expect("button")
                .support
                .is_supported()
        );
    }

    #[test]
    fn the_schema_is_the_contracts_properties() {
        let catalog = catalog();
        let schema = catalog.schema("button").expect("a schema");
        assert_eq!(schema.len(), 2);
        assert_eq!(schema[0].name, "label");
        assert_eq!(schema[0].default.as_deref(), Some("Button"));
        assert_eq!(
            catalog.schema("button").expect("schema")[1].value_type,
            ValueType::Enum(vec!["primary".to_owned(), "ghost".to_owned()])
        );
    }

    #[test]
    fn limits_are_listed_not_dropped() {
        let catalog = catalog();
        let limits = &catalog.get("image").expect("image").limits;
        assert_eq!(limits.len(), 1);
        assert!(limits[0].contains("animated formats"));
    }

    #[test]
    fn search_lifts_favorites_but_keeps_the_rest() {
        let mut catalog = catalog();
        catalog.favorite("image", true);
        let hits: Vec<&str> = catalog
            .search("")
            .iter()
            .map(|descriptor| descriptor.id.as_str())
            .collect();
        // The favorite is first, but the other kind is still findable.
        assert_eq!(hits, vec!["image", "button"]);
        assert!(catalog.is_favorite("image"));
    }

    #[test]
    fn categories_group_the_kinds() {
        let catalog = catalog();
        let categories: Vec<&str> = catalog.categories().into_iter().collect();
        assert_eq!(categories, vec!["controls", "media"]);
        assert_eq!(catalog.in_category("controls").len(), 1);
    }

    #[test]
    fn a_support_reason_is_queryable_from_the_type() {
        let support = Support::Unsupported {
            reason: "not here".to_owned(),
        };
        assert_eq!(support.reason(), Some("not here"));
        assert!(!support.is_supported());
        assert_eq!(Support::Supported.reason(), None);
    }
}
