// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Registering a custom widget: its typed schema, its events and its per-target implementations
//! (task T12.04).
//!
//! The acceptance line is *"a missing target implementation must not appear publishable"*. The
//! design keeps that true by making publishability a value with two shapes:
//!
//! * **[`Publishable`] is `Yes` or `No`, never a bare boolean.** A widget whose implementations do
//!   not include the selected target answers [`Publishable::No`] and names the target it is missing,
//!   so a missing implementation is a fact with a name rather than a green check.
//! * **An unknown target is a different failure from a missing implementation.**
//!   [`Registry::publishable`] refuses a target the project does not know outright (`Err`) before it
//!   ever asks about implementations, so "this profile does not exist" and "this profile has no
//!   implementation" cannot be confused.
//! * **The schema is validated against real values.** [`validate`] refuses a value of the wrong
//!   family and a required field that is absent, so a registered schema is a check, not a comment.
//!
//! Registration refuses a duplicate widget name and duplicate field or event names, because a
//! second declaration of one field is a typo whose two halves would otherwise both be accepted.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use crate::contracts::model::{PropertyName, PropertyValue};

/// The output profiles a custom widget may have an implementation for.
pub const TARGETS: [&str; 5] = ["desktop", "tablet", "mobile", "mini", "embedded"];

/// Whether `target` is one of the five output profiles.
pub fn is_target(target: &str) -> bool {
    TARGETS.contains(&target)
}

/// The value family a field or an event payload declares.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ValueType {
    /// A flag.
    Bool,
    /// A whole number.
    Integer,
    /// A fractional number.
    Number,
    /// Free text.
    Text,
    /// A colour.
    Color,
    /// A resource reference.
    Asset,
}

impl ValueType {
    /// The family's name, as written in the schema.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Bool => "bool",
            Self::Integer => "integer",
            Self::Number => "number",
            Self::Text => "text",
            Self::Color => "color",
            Self::Asset => "asset",
        }
    }

    /// Reads a family name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "bool" => Some(Self::Bool),
            "integer" => Some(Self::Integer),
            "number" => Some(Self::Number),
            "text" => Some(Self::Text),
            "color" => Some(Self::Color),
            "asset" => Some(Self::Asset),
            _ => None,
        }
    }

    /// Whether a value belongs to this family.
    pub fn accepts(&self, value: &PropertyValue) -> bool {
        matches!(
            (self, value),
            (Self::Bool, PropertyValue::Bool(_))
                | (Self::Integer, PropertyValue::Integer(_))
                | (Self::Number, PropertyValue::Number(_))
                | (Self::Text, PropertyValue::Text(_))
                | (Self::Color, PropertyValue::Color(_))
                | (Self::Asset, PropertyValue::Asset(_))
        )
    }
}

/// One field of a custom widget's schema.
#[derive(Clone, PartialEq, Debug)]
pub struct Field {
    /// The field's property name.
    pub name: PropertyName,
    /// The value family the field accepts.
    pub value_type: ValueType,
    /// Whether a value must be present.
    pub required: bool,
}

/// One event a custom widget emits.
#[derive(Clone, PartialEq, Debug)]
pub struct Event {
    /// The event's name.
    pub name: String,
    /// The payload's value family.
    pub payload: ValueType,
}

/// A registered custom widget: schema, events and per-target implementations.
#[derive(Clone, PartialEq, Debug)]
pub struct CustomWidget {
    /// The widget's control kind name.
    pub name: String,
    /// The fields of its schema.
    pub fields: Vec<Field>,
    /// The events it emits.
    pub events: Vec<Event>,
    /// The implementation note for each target that has one, keyed by target.
    pub implementations: BTreeMap<String, String>,
}

impl CustomWidget {
    /// Builds a widget with no fields, events or implementations, or `None` when the name is blank.
    pub fn new(name: &str) -> Option<Self> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return None;
        }
        Some(Self {
            name: trimmed.to_owned(),
            fields: Vec::new(),
            events: Vec::new(),
            implementations: BTreeMap::new(),
        })
    }

    /// Whether this widget has an implementation for a target.
    pub fn implements(&self, target: &str) -> bool {
        self.implementations.contains_key(target)
    }
}

/// Whether a widget can be published for a target.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Publishable {
    /// It can, for this target.
    Yes {
        /// The target.
        target: String,
    },
    /// It cannot, because this target has no implementation.
    No {
        /// The target whose implementation is missing.
        missing_target: String,
    },
}

impl Publishable {
    /// Whether the widget can be published.
    pub fn is_publishable(&self) -> bool {
        matches!(self, Self::Yes { .. })
    }

    /// The verdict in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Yes { target } => format!("publishable for {target}"),
            Self::No { missing_target } => {
                format!("not publishable: no implementation for {missing_target}")
            }
        }
    }
}

/// The registered custom widgets.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Registry {
    widgets: BTreeMap<String, CustomWidget>,
}

impl Registry {
    /// A registry with nothing in it.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a widget, refusing a duplicate name and a duplicate field or event name.
    pub fn register(&mut self, widget: CustomWidget) -> Result<(), String> {
        if self.widgets.contains_key(&widget.name) {
            return Err(format!(
                "custom widget {:?} is already registered",
                widget.name
            ));
        }
        let mut fields = BTreeSet::new();
        for field in &widget.fields {
            if !fields.insert(field.name.clone()) {
                return Err(format!(
                    "custom widget {:?} declares field {:?} twice",
                    widget.name,
                    field.name.as_str()
                ));
            }
        }
        let mut events = BTreeSet::new();
        for event in &widget.events {
            if !events.insert(event.name.clone()) {
                return Err(format!(
                    "custom widget {:?} declares event {:?} twice",
                    widget.name, event.name
                ));
            }
        }
        self.widgets.insert(widget.name.clone(), widget);
        Ok(())
    }

    /// One registered widget, if it exists.
    pub fn widget(&self, name: &str) -> Option<&CustomWidget> {
        self.widgets.get(name)
    }

    /// The registered widget names, in order.
    pub fn names(&self) -> Vec<&str> {
        self.widgets.keys().map(String::as_str).collect()
    }

    /// Whether a widget is publishable for a target.
    ///
    /// An unknown target is refused outright; a known target with no implementation answers
    /// [`Publishable::No`] naming it. The two are never the same outcome.
    pub fn publishable(&self, name: &str, target: &str) -> Result<Publishable, String> {
        if !is_target(target) {
            return Err(format!("{target:?} is not a target"));
        }
        let widget = self
            .widget(name)
            .ok_or_else(|| format!("no custom widget {name:?}"))?;
        if widget.implements(target) {
            Ok(Publishable::Yes {
                target: target.to_owned(),
            })
        } else {
            Ok(Publishable::No {
                missing_target: target.to_owned(),
            })
        }
    }
}

/// Checks a set of values against a widget's schema.
pub fn validate(
    widget: &CustomWidget,
    values: &BTreeMap<PropertyName, PropertyValue>,
) -> Result<(), String> {
    for field in &widget.fields {
        match values.get(&field.name) {
            Some(value) => {
                if !field.value_type.accepts(value) {
                    return Err(format!(
                        "field {:?} needs a {} value, got {}",
                        field.name.as_str(),
                        field.value_type.name(),
                        value.family()
                    ));
                }
            }
            None => {
                if field.required {
                    return Err(format!("field {:?} is required", field.name.as_str()));
                }
            }
        }
    }
    for name in values.keys() {
        if widget
            .fields
            .iter()
            .all(|field| field.name.as_str() != name.as_str())
        {
            return Err(format!(
                "{:?} is not a field of {:?}",
                name.as_str(),
                widget.name
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{CustomWidget, Event, Field, Registry, ValueType, validate};
    use crate::contracts::model::{PropertyName, PropertyValue};
    use std::collections::BTreeMap;

    fn name(text: &str) -> PropertyName {
        PropertyName::new(text).expect("a name")
    }

    fn field(name_text: &str, value_type: ValueType, required: bool) -> Field {
        Field {
            name: name(name_text),
            value_type,
            required,
        }
    }

    fn gauge() -> CustomWidget {
        let mut widget = CustomWidget::new("gauge").expect("a widget");
        widget.fields.push(field("value", ValueType::Number, true));
        widget.events.push(Event {
            name: "changed".to_owned(),
            payload: ValueType::Number,
        });
        widget
    }

    #[test]
    fn a_widget_without_an_implementation_for_the_target_is_not_publishable() {
        let mut registry = Registry::new();
        registry.register(gauge()).expect("a registration");
        assert!(
            registry
                .widget("gauge")
                .expect("a widget")
                .implementations
                .is_empty()
        );
        let verdict = registry.publishable("gauge", "desktop").expect("a verdict");
        assert!(!verdict.is_publishable());
        assert_eq!(
            verdict.render(),
            "not publishable: no implementation for desktop"
        );
        assert_eq!(
            verdict,
            super::Publishable::No {
                missing_target: "desktop".to_owned()
            }
        );
    }

    #[test]
    fn a_widget_with_an_implementation_for_the_target_is_publishable() {
        let mut registry = Registry::new();
        let mut widget = gauge();
        widget
            .implementations
            .insert("desktop".to_owned(), "uses the native gauge".to_owned());
        registry.register(widget).expect("a registration");
        let verdict = registry.publishable("gauge", "desktop").expect("a verdict");
        assert!(verdict.is_publishable());
        assert_eq!(verdict.render(), "publishable for desktop");
    }

    #[test]
    fn an_unknown_target_is_refused_as_unsupported() {
        let mut registry = Registry::new();
        registry.register(gauge()).expect("a registration");
        assert!(registry.publishable("gauge", "watch").is_err());
    }

    #[test]
    fn an_unknown_widget_is_refused() {
        let registry = Registry::new();
        assert!(registry.publishable("missing", "desktop").is_err());
    }

    #[test]
    fn a_missing_required_field_is_refused() {
        let widget = gauge();
        let reason = validate(&widget, &BTreeMap::new()).expect_err("a missing field");
        assert!(reason.contains("required"), "{reason}");
    }

    #[test]
    fn a_value_of_the_wrong_family_is_refused() {
        let widget = gauge();
        let mut values = BTreeMap::new();
        values.insert(name("value"), PropertyValue::Text("high".to_owned()));
        let reason = validate(&widget, &values).expect_err("a wrong family");
        assert!(reason.contains("needs a number"), "{reason}");
    }

    #[test]
    fn a_value_accepted_by_the_schema_passes() {
        let widget = gauge();
        let mut values = BTreeMap::new();
        values.insert(name("value"), PropertyValue::Number(0.5));
        assert!(validate(&widget, &values).is_ok());
    }

    #[test]
    fn registering_a_duplicate_field_name_is_refused() {
        let mut registry = Registry::new();
        let mut widget = gauge();
        widget.fields.push(field("value", ValueType::Text, false));
        let reason = registry.register(widget).expect_err("a duplicate field");
        assert!(reason.contains("twice"), "{reason}");
    }

    #[test]
    fn a_blank_widget_name_is_not_a_widget() {
        assert!(CustomWidget::new("  ").is_none());
        assert_eq!(ValueType::parse("number"), Some(ValueType::Number));
        assert_eq!(ValueType::parse("blob"), None);
    }
}
