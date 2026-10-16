// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The schema-driven property editor: one field per property type, and a refusal that never touches
//! the valid model (task T11.01).
//!
//! The acceptance line is *"type and range are validated, and an error does not change the valid
//! model"*. The editor therefore keeps the *valid values* and the *text being typed* apart, which is
//! what makes an error harmless:
//!
//! * **The valid value is what a refusal cannot reach.** A [`PropertyType`] parses the text; on
//!   success the [`Editor`] replaces the stored value, and on failure it stores the *text* and the
//!   reason and leaves the value where it was, so a bad keystroke cannot corrupt the model.
//! * **The text a person is typing is kept.** [`Editor::pending`] answers the text and
//!   [`Editor::problem`] the reason, so the field shows what was typed with the error beside it
//!   rather than snapping back to the old value.
//! * **The type decides, once.** [`PropertyType::parse`] is the only place a string becomes a value,
//!   so the panel, the file reader and the checker cannot disagree about what a property accepts.
//! * **A property the schema does not describe is not editable.** Attempting one is refused without
//!   touching anything, so the editor cannot invent a property the model has no room for.

#![allow(dead_code)]

use std::collections::BTreeMap;

use crate::contracts::identity::NodeId;
use crate::contracts::model::{PropertyName, PropertyValue};

/// What a property accepts, and the range it accepts it in.
#[derive(Clone, PartialEq, Debug)]
pub enum PropertyType {
    /// A fractional number, optionally bounded.
    Number {
        /// The smallest allowed, when bounded.
        min: Option<f64>,
        /// The largest allowed, when bounded.
        max: Option<f64>,
    },
    /// A whole number, optionally bounded.
    Integer {
        /// The smallest allowed, when bounded.
        min: Option<i64>,
        /// The largest allowed, when bounded.
        max: Option<i64>,
    },
    /// A choice from a named set.
    Enum {
        /// The variants, in order.
        variants: Vec<String>,
    },
    /// Free text, at most this many characters.
    Text {
        /// The longest allowed.
        max_len: usize,
    },
    /// A colour, written as the model writes colours.
    Colour,
    /// A size: a number that cannot be negative.
    Size,
    /// A reference to a project resource.
    Asset,
}

impl PropertyType {
    /// The type's name, for a report.
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Number { .. } => "number",
            Self::Integer { .. } => "integer",
            Self::Enum { .. } => "enum",
            Self::Text { .. } => "text",
            Self::Colour => "colour",
            Self::Size => "size",
            Self::Asset => "asset",
        }
    }

    /// Turns typed text into a value, or explains why it is not one.
    pub fn parse(&self, text: &str) -> Result<PropertyValue, String> {
        match self {
            Self::Number { min, max } => {
                let value = text
                    .trim()
                    .parse::<f64>()
                    .map_err(|_| format!("{text:?} is not a number"))?;
                if let Some(min) = min
                    && value < *min
                {
                    return Err(format!("{value} is below the minimum {min}"));
                }
                if let Some(max) = max
                    && value > *max
                {
                    return Err(format!("{value} is above the maximum {max}"));
                }
                Ok(PropertyValue::Number(value))
            }
            Self::Integer { min, max } => {
                let value = text
                    .trim()
                    .parse::<i64>()
                    .map_err(|_| format!("{text:?} is not a whole number"))?;
                if let Some(min) = min
                    && value < *min
                {
                    return Err(format!("{value} is below the minimum {min}"));
                }
                if let Some(max) = max
                    && value > *max
                {
                    return Err(format!("{value} is above the maximum {max}"));
                }
                Ok(PropertyValue::Integer(value))
            }
            Self::Enum { variants } => {
                let value = text.trim();
                if variants.iter().any(|variant| variant == value) {
                    Ok(PropertyValue::Variant(value.to_owned()))
                } else {
                    Err(format!("{value:?} is not one of {}", variants.join(", ")))
                }
            }
            Self::Text { max_len } => {
                if text.chars().count() > *max_len {
                    Err(format!("the text is longer than {max_len} characters"))
                } else {
                    Ok(PropertyValue::Text(text.to_owned()))
                }
            }
            Self::Colour => {
                let value = text.trim();
                if value.starts_with('#') && value.len() > 1 {
                    Ok(PropertyValue::Color(value.to_owned()))
                } else {
                    Err(format!("{value:?} is not a colour"))
                }
            }
            Self::Size => {
                let value = text
                    .trim()
                    .parse::<f64>()
                    .map_err(|_| format!("{text:?} is not a size"))?;
                if value < 0.0 {
                    Err(format!("{value} is below zero"))
                } else {
                    Ok(PropertyValue::Number(value))
                }
            }
            Self::Asset => {
                let value = text.trim();
                if value.is_empty() {
                    Err("an asset reference cannot be empty".to_owned())
                } else {
                    Ok(PropertyValue::Asset(value.to_owned()))
                }
            }
        }
    }

    /// How the type reads, for the panel's field.
    pub fn render(&self) -> String {
        match self {
            Self::Number { min, max } => format!(
                "number [{}..{}]",
                min.map_or("".to_owned(), |value| value.to_string()),
                max.map_or("".to_owned(), |value| value.to_string())
            ),
            Self::Integer { min, max } => format!(
                "integer [{}..{}]",
                min.map_or("".to_owned(), |value| value.to_string()),
                max.map_or("".to_owned(), |value| value.to_string())
            ),
            Self::Enum { variants } => format!("enum({})", variants.join(",")),
            Self::Text { max_len } => format!("text<={max_len}"),
            Self::Colour => "colour".to_owned(),
            Self::Size => "size".to_owned(),
            Self::Asset => "asset".to_owned(),
        }
    }
}

/// The types this build knows, by property name.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Schema {
    entries: BTreeMap<PropertyName, PropertyType>,
}

impl Schema {
    /// A schema with nothing described.
    pub fn new() -> Self {
        Self::default()
    }

    /// The same schema with one more property described.
    pub fn with(mut self, name: PropertyName, kind: PropertyType) -> Self {
        self.entries.insert(name, kind);
        self
    }

    /// The type of a property, if the schema describes it.
    pub fn kind_of(&self, name: &PropertyName) -> Option<&PropertyType> {
        self.entries.get(name)
    }

    /// Every described property, in name order.
    pub fn entries(&self) -> impl Iterator<Item = (&PropertyName, &PropertyType)> {
        self.entries.iter()
    }
}

/// A key for one field: a node and a property.
type Field = (NodeId, PropertyName);

/// The editor: the valid values, the text being typed, and the reasons it is not accepted yet.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Editor {
    schema: Schema,
    values: BTreeMap<Field, PropertyValue>,
    pending: BTreeMap<Field, String>,
    problems: BTreeMap<Field, String>,
}

impl Editor {
    /// An editor over a schema.
    pub fn new(schema: Schema) -> Self {
        Self {
            schema,
            ..Self::default()
        }
    }

    /// The schema.
    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    /// Describes one more property, keeping every value already typed.
    pub fn describe(&mut self, name: PropertyName, kind: PropertyType) {
        self.schema.entries.insert(name, kind);
    }

    /// The valid value of a field, if it has one.
    pub fn value(&self, node: NodeId, name: &PropertyName) -> Option<&PropertyValue> {
        self.values.get(&(node, name.clone()))
    }

    /// The text being typed into a field, when it was not accepted.
    pub fn pending(&self, node: NodeId, name: &PropertyName) -> Option<&str> {
        self.pending.get(&(node, name.clone())).map(String::as_str)
    }

    /// Why the text in a field was not accepted.
    pub fn problem(&self, node: NodeId, name: &PropertyName) -> Option<&str> {
        self.problems.get(&(node, name.clone())).map(String::as_str)
    }

    /// Types text into a field.
    ///
    /// A valid value replaces the stored one and clears any problem; a refusal stores the text and
    /// the reason and leaves the valid value exactly as it was.
    pub fn set(&mut self, node: NodeId, name: PropertyName, text: &str) -> Result<(), String> {
        let Some(kind) = self.schema.kind_of(&name).cloned() else {
            return Err(format!("{} is not a described property", name.as_str()));
        };
        match kind.parse(text) {
            Ok(value) => {
                self.values.insert((node, name.clone()), value);
                self.pending.remove(&(node, name.clone()));
                self.problems.remove(&(node, name));
                Ok(())
            }
            Err(reason) => {
                self.pending.insert((node, name.clone()), text.to_owned());
                self.problems.insert((node, name), reason.clone());
                Err(reason)
            }
        }
    }

    /// The fields with an unaccepted edit, as `(node, property, reason)`.
    pub fn problems(&self) -> Vec<(NodeId, PropertyName, String)> {
        self.problems
            .iter()
            .map(|((node, name), reason)| (*node, name.clone(), reason.clone()))
            .collect()
    }

    /// The editor in a report.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "editor: {} value(s), {} pending, {} problem(s)",
            self.values.len(),
            self.pending.len(),
            self.problems.len()
        )];
        for ((node, name), value) in &self.values {
            lines.push(format!(
                "  node {} {} = {}",
                node.get(),
                name.as_str(),
                value.family()
            ));
        }
        for ((node, name), reason) in &self.problems {
            lines.push(format!("  node {} {}: {reason}", node.get(), name.as_str()));
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{Editor, PropertyType, Schema};
    use crate::contracts::identity::NodeId;
    use crate::contracts::model::{PropertyName, PropertyValue};

    fn node(value: u64) -> NodeId {
        NodeId::from_raw(value)
    }

    fn name(text: &str) -> PropertyName {
        PropertyName::new(text).expect("a name")
    }

    fn editor() -> Editor {
        Editor::new(
            Schema::new()
                .with(
                    name("width"),
                    PropertyType::Number {
                        min: Some(0.0),
                        max: Some(1000.0),
                    },
                )
                .with(
                    name("variant"),
                    PropertyType::Enum {
                        variants: vec!["primary".to_owned(), "ghost".to_owned()],
                    },
                )
                .with(name("label"), PropertyType::Text { max_len: 8 })
                .with(name("tint"), PropertyType::Colour)
                .with(name("icon"), PropertyType::Asset),
        )
    }

    #[test]
    fn a_valid_number_is_accepted_and_stored() {
        let mut editor = editor();
        assert_eq!(editor.set(node(1), name("width"), "120"), Ok(()));
        assert_eq!(
            editor.value(node(1), &name("width")),
            Some(&PropertyValue::Number(120.0))
        );
        assert!(editor.problems().is_empty());
    }

    #[test]
    fn a_number_out_of_range_is_refused_and_the_valid_value_is_unchanged() {
        let mut editor = editor();
        editor.set(node(1), name("width"), "120").expect("valid");
        let reason = editor
            .set(node(1), name("width"), "5000")
            .expect_err("out of range");
        assert!(reason.contains("above the maximum"));
        assert_eq!(
            editor.value(node(1), &name("width")),
            Some(&PropertyValue::Number(120.0)),
            "the valid model did not change"
        );
        assert_eq!(editor.pending(node(1), &name("width")), Some("5000"));
        assert_eq!(
            editor.problem(node(1), &name("width")),
            Some(reason.as_str())
        );
    }

    #[test]
    fn a_property_the_schema_does_not_describe_is_refused_without_touching_anything() {
        let mut editor = editor();
        assert!(editor.set(node(1), name("colour"), "red").is_err());
        assert!(editor.value(node(1), &name("colour")).is_none());
        assert!(editor.pending(node(1), &name("colour")).is_none());
    }

    #[test]
    fn an_enum_accepts_a_variant_and_refuses_anything_else() {
        let mut editor = editor();
        editor
            .set(node(1), name("variant"), "primary")
            .expect("a variant");
        assert!(editor.set(node(1), name("variant"), "loud").is_err());
        assert_eq!(
            editor.value(node(1), &name("variant")),
            Some(&PropertyValue::Variant("primary".to_owned()))
        );
    }

    #[test]
    fn text_over_the_limit_is_refused() {
        let mut editor = editor();
        assert!(editor.set(node(1), name("label"), "ok").is_ok());
        assert!(editor.set(node(1), name("label"), "far too long").is_err());
        assert_eq!(
            editor.value(node(1), &name("label")),
            Some(&PropertyValue::Text("ok".to_owned()))
        );
    }

    #[test]
    fn a_colour_and_an_asset_are_parsed_as_their_own_families() {
        let mut editor = editor();
        editor
            .set(node(1), name("tint"), "#112233")
            .expect("a colour");
        editor
            .set(node(1), name("icon"), "assets/a.svg")
            .expect("an asset");
        assert_eq!(
            editor
                .value(node(1), &name("tint"))
                .map(PropertyValue::family),
            Some("color")
        );
        assert_eq!(
            editor
                .value(node(1), &name("icon"))
                .map(PropertyValue::family),
            Some("asset")
        );
        assert!(editor.set(node(1), name("tint"), "red").is_err());
        assert!(editor.set(node(1), name("icon"), "").is_err());
    }

    #[test]
    fn a_later_valid_edit_clears_the_problem() {
        let mut editor = editor();
        assert!(editor.set(node(1), name("width"), "oops").is_err());
        assert!(editor.problem(node(1), &name("width")).is_some());
        editor.set(node(1), name("width"), "10").expect("valid");
        assert!(editor.problem(node(1), &name("width")).is_none());
        assert!(editor.pending(node(1), &name("width")).is_none());
    }

    #[test]
    fn a_size_cannot_be_negative() {
        let size = PropertyType::Size;
        assert!(size.parse("-1").is_err());
        assert_eq!(size.parse("12"), Ok(PropertyValue::Number(12.0)));
    }

    #[test]
    fn every_type_names_itself() {
        let kinds = [
            PropertyType::Number {
                min: None,
                max: None,
            },
            PropertyType::Integer {
                min: None,
                max: None,
            },
            PropertyType::Enum {
                variants: Vec::new(),
            },
            PropertyType::Text { max_len: 1 },
            PropertyType::Colour,
            PropertyType::Size,
            PropertyType::Asset,
        ];
        for kind in kinds {
            assert!(!kind.name().is_empty());
            assert!(!kind.render().is_empty());
        }
    }
}
