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
//! * **The schema comes from the library contract when there is one.** [`Schema::from_property_schema`]
//!   and [`Schema::from_manifest`] map the library's declared kind, accepted tokens, constraints and
//!   measured default into this module's [`PropertyType`]; a kind or token list the contract does not
//!   publish is not filled in from a guess. [`Schema::new`] plus [`Schema::with`] stays the explicit
//!   constructor for hand-built schemas (the CLI tests).

#![allow(dead_code)]

use std::collections::BTreeMap;

use rust_widgets::widget::capability::designer_manifest::DesignerConstraint;
use rust_widgets::widget::capability::{
    CapabilityValue, Constraint as ContractConstraint, DesignerManifest,
    PropertySchema as ContractPropertySchema, PropertyValueKind, value_kind_token,
};

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
            Self::Text { max_len } if *max_len == usize::MAX => "text".to_owned(),
            Self::Text { max_len } => format!("text<={max_len}"),
            Self::Colour => "colour".to_owned(),
            Self::Size => "size".to_owned(),
            Self::Asset => "asset".to_owned(),
        }
    }
}

/// The types this build knows, by property name, with the contract's measured defaults and the
/// parts of a contract this editor could not represent.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Schema {
    entries: BTreeMap<PropertyName, PropertyType>,
    defaults: BTreeMap<PropertyName, PropertyValue>,
    gaps: Vec<String>,
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

    /// Records the value a freshly created control holds, when the contract published one.
    ///
    /// A default for a property the schema does not describe is ignored: a value nothing can edit
    /// is not a default, it is a stray value.
    pub fn with_default(mut self, name: PropertyName, value: PropertyValue) -> Self {
        if self.entries.contains_key(&name) {
            self.defaults.insert(name, value);
        }
        self
    }

    /// The type of a property, if the schema describes it.
    pub fn kind_of(&self, name: &PropertyName) -> Option<&PropertyType> {
        self.entries.get(name)
    }

    /// The contract's default for a property, when it published one.
    pub fn default_of(&self, name: &PropertyName) -> Option<&PropertyValue> {
        self.defaults.get(name)
    }

    /// Every described property, in name order.
    pub fn entries(&self) -> impl Iterator<Item = (&PropertyName, &PropertyType)> {
        self.entries.iter()
    }

    /// The parts of the contract that were not turned into a field, with a reason.
    ///
    /// A gap is reported rather than dropped: "not offered" must not read as "not declared".
    pub fn gaps(&self) -> &[String] {
        &self.gaps
    }

    /// A schema for one property, from the library's own [`ContractPropertySchema`].
    ///
    /// The kind, its accepted tokens and its numeric range come from the contract; nothing is
    /// invented. A kind this editor cannot present (a rectangle, an icon source, a list) is an
    /// error rather than a field that would accept values the control does not.
    pub fn from_property_schema(
        control: &str,
        property: &ContractPropertySchema,
    ) -> Result<Self, SchemaError> {
        let name = PropertyName::new(property.name).ok_or_else(|| SchemaError {
            control: control.to_owned(),
            property: property.name.to_owned(),
            reason: "the property name is blank".to_owned(),
        })?;
        let tokens: Vec<String> = property
            .accepted_tokens
            .iter()
            .map(|token| (*token).to_owned())
            .collect();
        let kind = property_type(
            property.value_kind,
            &tokens,
            Bounds::from_contract(property.constraints),
        )
        .map_err(|reason| SchemaError {
            control: control.to_owned(),
            property: property.name.to_owned(),
            reason,
        })?;
        Ok(Self::new().with(name, kind))
    }

    /// A schema for every property a control's designer manifest describes.
    ///
    /// Defaults are the manifest's *measured* values, not a guess at what a control starts with.
    /// A property whose kind or token list this editor cannot present is left out of `entries` and
    /// recorded in [`Schema::gaps`], so editing the rest of the contract stays possible while the
    /// omission is still visible.
    pub fn from_manifest(control: &str, manifest: &DesignerManifest) -> Self {
        let mut schema = Self::new();
        for property in &manifest.properties {
            let Some(name) = PropertyName::new(&property.name) else {
                schema.gaps.push(format!(
                    "{control}: the contract property name {:?} is blank",
                    property.name
                ));
                continue;
            };
            match property_type(
                property.kind,
                &property.tokens,
                Bounds::from_designer(&property.constraints),
            ) {
                Ok(kind) => {
                    if let Some(value) = contract_value(property.kind, &property.default_value) {
                        schema.defaults.insert(name.clone(), value);
                    }
                    schema.entries.insert(name, kind);
                }
                Err(reason) => schema
                    .gaps
                    .push(format!("{control}.{}: {reason}", property.name)),
            }
        }
        schema
    }
}

/// Why a contract property could not become an editor field.
///
/// The control and property are carried so a refusal names what it is about instead of pointing at
/// "the schema" in general.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SchemaError {
    control: String,
    property: String,
    reason: String,
}

impl SchemaError {
    /// The refusal in one line.
    pub fn message(&self) -> String {
        format!(
            "{}.{} has no editor schema: {}",
            self.control, self.property, self.reason
        )
    }
}

impl std::fmt::Display for SchemaError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message())
    }
}

/// The numeric and emptiness facts a contract declares about one property.
#[derive(Clone, Copy, Default)]
struct Bounds {
    min: Option<f64>,
    max: Option<f64>,
    required: bool,
}

impl Bounds {
    /// The bounds the library's runtime constraints declare.
    fn from_contract(constraints: &[ContractConstraint]) -> Self {
        let mut bounds = Self::default();
        for constraint in constraints {
            match constraint {
                ContractConstraint::Range(range) => {
                    bounds.min = Some(range.minimum);
                    bounds.max = Some(range.maximum);
                }
                ContractConstraint::Required => bounds.required = true,
                // A precision rounds a value rather than refusing one, so it does not make any
                // input illegal; encoding it as a refusal would refuse values the control takes.
                ContractConstraint::Precision { .. } => {}
            }
        }
        bounds
    }

    /// The bounds a designer manifest's flat constraint rows declare.
    fn from_designer(constraints: &[DesignerConstraint]) -> Self {
        let mut bounds = Self::default();
        for constraint in constraints {
            match constraint.kind {
                "range" => {
                    bounds.min = constraint.minimum.or(bounds.min);
                    bounds.max = constraint.maximum.or(bounds.max);
                }
                "required" => bounds.required = true,
                _ => {}
            }
        }
        bounds
    }
}

/// Maps a declared library kind, its tokens and its bounds into an editor type.
///
/// A kind with no honest field in this editor is an error: the alternative is a field that accepts
/// something the control would refuse, or refuses something it accepts.
fn property_type(
    kind: PropertyValueKind,
    tokens: &[String],
    bounds: Bounds,
) -> Result<PropertyType, String> {
    Ok(match kind {
        PropertyValueKind::Bool => {
            // The editor has no field that carries a flag as a boolean value, and spelling it as
            // an enum would put the wrong value family in the model.
            return Err("a `bool` property has no field type in this editor".to_owned());
        }
        PropertyValueKind::Int => PropertyType::Integer {
            // An integer inside a float range is the integers the range contains.
            min: bounds.min.map(|value| value.ceil() as i64),
            max: bounds.max.map(|value| value.floor() as i64),
        },
        PropertyValueKind::UInt => PropertyType::Integer {
            // An unsigned property cannot hold a negative value whether or not it declares a range.
            min: Some(bounds.min.unwrap_or(0.0).max(0.0).ceil() as i64),
            max: bounds.max.map(|value| value.floor() as i64),
        },
        PropertyValueKind::Float | PropertyValueKind::Number => PropertyType::Number {
            min: bounds.min,
            max: bounds.max,
        },
        PropertyValueKind::String if bounds.required => {
            // `Required` is a refusal the text field cannot express today; accepting an empty
            // string would accept what the control refuses.
            return Err("a required string has no field type in this editor".to_owned());
        }
        PropertyValueKind::String => PropertyType::Text {
            max_len: usize::MAX,
        },
        PropertyValueKind::Enum if tokens.is_empty() => {
            return Err("an enum property publishes no accepted tokens".to_owned());
        }
        PropertyValueKind::Enum => PropertyType::Enum {
            variants: tokens.to_vec(),
        },
        PropertyValueKind::Color => PropertyType::Colour,
        other => {
            return Err(format!(
                "a `{}` property has no field type in this editor",
                value_kind_token(other)
            ));
        }
    })
}

/// A contract value in this module's value family, when the two agree.
///
/// `None` is the honest answer for a value whose kind the editor does not present (a rectangle, an
/// icon source, a list) and for the contract's `Null`, which means "not set" rather than a value.
pub fn contract_value(kind: PropertyValueKind, value: &CapabilityValue) -> Option<PropertyValue> {
    match (kind, value) {
        (_, CapabilityValue::Null) => None,
        (PropertyValueKind::Bool, CapabilityValue::Bool(value)) => {
            Some(PropertyValue::Bool(*value))
        }
        (PropertyValueKind::Int, CapabilityValue::Int(value)) => {
            Some(PropertyValue::Integer(*value))
        }
        (PropertyValueKind::UInt, CapabilityValue::UInt(value)) => {
            i64::try_from(*value).ok().map(PropertyValue::Integer)
        }
        (PropertyValueKind::Float | PropertyValueKind::Number, CapabilityValue::Float(value)) => {
            Some(PropertyValue::Number(*value))
        }
        (PropertyValueKind::Float | PropertyValueKind::Number, CapabilityValue::Int(value)) => {
            Some(PropertyValue::Number(*value as f64))
        }
        (PropertyValueKind::Float | PropertyValueKind::Number, CapabilityValue::UInt(value)) => {
            Some(PropertyValue::Number(*value as f64))
        }
        (PropertyValueKind::String, CapabilityValue::String(value)) => {
            Some(PropertyValue::Text(value.clone()))
        }
        (PropertyValueKind::Enum, CapabilityValue::String(value)) => {
            Some(PropertyValue::Variant(value.clone()))
        }
        (PropertyValueKind::Color, CapabilityValue::Color(color)) => {
            Some(PropertyValue::Color(format!(
                "#{:02x}{:02x}{:02x}{:02x}",
                color.r, color.g, color.b, color.a
            )))
        }
        _ => None,
    }
}

/// How a value reads in a single-line field.
pub fn render_value(value: &PropertyValue) -> String {
    match value {
        PropertyValue::Bool(value) => value.to_string(),
        PropertyValue::Integer(value) => value.to_string(),
        PropertyValue::Number(value) => value.to_string(),
        PropertyValue::Text(value)
        | PropertyValue::Variant(value)
        | PropertyValue::Color(value)
        | PropertyValue::Asset(value) => value.clone(),
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
    /// The type each accepted value was last validated against.
    ///
    /// A value can outlive a schema that no longer describes its property (so switching back keeps
    /// it). Keeping the type with the value is what lets a later rebind drop it when the property
    /// comes back with a different family, instead of presenting a stale value as the new type.
    kinds: BTreeMap<PropertyName, PropertyType>,
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

    /// Replaces the schema, keeping accepted values whose property keeps the same type.
    ///
    /// A value for a property the new schema does not describe is kept: another control may
    /// describe it again when the Inspector moves back to that node, and only the current property
    /// name is ever read. A property whose declared type changed drops its value, because the old
    /// value belongs to a family the control no longer accepts.
    pub fn rebind(&mut self, schema: Schema) {
        let previous = std::mem::replace(&mut self.schema, schema);
        let next = &self.schema;
        let kinds = &self.kinds;
        let survives = |name: &PropertyName| {
            let before = previous.kind_of(name).or_else(|| kinds.get(name));
            match (before, next.kind_of(name)) {
                (Some(before), Some(after)) => before == after,
                _ => true,
            }
        };
        self.values.retain(|(_, name), _| survives(name));
        self.pending.retain(|(_, name), _| survives(name));
        self.problems.retain(|(_, name), _| survives(name));
        let described: Vec<(PropertyName, PropertyType)> = self
            .schema
            .entries()
            .map(|(name, kind)| (name.clone(), kind.clone()))
            .collect();
        for (name, kind) in described {
            self.kinds.insert(name, kind);
        }
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
                self.kinds.insert(name.clone(), kind);
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
    use rust_widgets::widget::capability::{
        CapabilityValue, Constraint, DesignerManifest, DesignerProperty,
        PropertySchema as ContractPropertySchema, PropertyValueKind, RangeSpec,
    };

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

    #[test]
    fn a_contract_schema_maps_its_tokens_and_refuses_an_unknown_token() {
        let contract = ContractPropertySchema::enumerated(
            "state",
            true,
            true,
            &["unchecked", "partially_checked", "checked"],
        );
        let schema = Schema::from_property_schema("check_box", &contract).expect("a mapped enum");
        assert_eq!(
            schema.kind_of(&name("state")),
            Some(&PropertyType::Enum {
                variants: vec![
                    "unchecked".to_owned(),
                    "partially_checked".to_owned(),
                    "checked".to_owned(),
                ],
            })
        );

        let mut editor = Editor::new(schema);
        editor
            .set(node(1), name("state"), "checked")
            .expect("a published token");
        let reason = editor
            .set(node(1), name("state"), "half")
            .expect_err("not a published token");
        assert!(reason.contains("not one of"), "{reason}");
        assert_eq!(
            editor.value(node(1), &name("state")),
            Some(&PropertyValue::Variant("checked".to_owned()))
        );
    }

    #[test]
    fn a_contract_range_becomes_the_number_bounds() {
        let contract = ContractPropertySchema::new("value", PropertyValueKind::Float, true, true)
            .constrained(&[Constraint::Range(RangeSpec {
                minimum: 0.0,
                maximum: 1.0,
                step: None,
            })]);
        let schema = Schema::from_property_schema("progress_circle", &contract).expect("mapped");
        assert_eq!(
            schema.kind_of(&name("value")),
            Some(&PropertyType::Number {
                min: Some(0.0),
                max: Some(1.0),
            })
        );
        let kind = schema.kind_of(&name("value")).expect("a kind");
        assert!(kind.parse("1.5").is_err(), "the contract bound is enforced");
        assert_eq!(kind.parse("0.5"), Ok(PropertyValue::Number(0.5)));
    }

    #[test]
    fn a_contract_unsigned_kind_cannot_be_negative() {
        let contract =
            ContractPropertySchema::new("icon_size", PropertyValueKind::UInt, true, true);
        let schema = Schema::from_property_schema("button", &contract).expect("mapped");
        let kind = schema.kind_of(&name("icon_size")).expect("a kind");
        assert_eq!(
            kind,
            &PropertyType::Integer {
                min: Some(0),
                max: None,
            }
        );
        assert!(kind.parse("-1").is_err());
        assert_eq!(kind.parse("16"), Ok(PropertyValue::Integer(16)));
    }

    #[test]
    fn a_contract_bool_is_reported_rather_than_turned_into_a_wrong_family() {
        let contract = ContractPropertySchema::new("visible", PropertyValueKind::Bool, true, true);
        let error = Schema::from_property_schema("button", &contract).expect_err("no field");
        assert!(error.message().contains("bool"), "{error}");
    }

    #[test]
    fn a_contract_manifest_makes_defaults_visible_and_names_what_it_cannot_show() {
        let manifest = DesignerManifest {
            protocol: 4,
            library: "test".to_owned(),
            profile: "desktop".to_owned(),
            triple: "x86_64".to_owned(),
            control: "button".to_owned(),
            aliases: Vec::new(),
            alias_of: String::new(),
            properties: vec![
                DesignerProperty {
                    name: "elide".to_owned(),
                    kind: PropertyValueKind::Enum,
                    readable: true,
                    writable: true,
                    tokens: vec!["none".to_owned(), "right".to_owned()],
                    default_value: CapabilityValue::String("right".to_owned()),
                    constraints: Vec::new(),
                },
                DesignerProperty {
                    name: "text".to_owned(),
                    kind: PropertyValueKind::String,
                    readable: true,
                    writable: true,
                    tokens: Vec::new(),
                    default_value: CapabilityValue::String("Demo".to_owned()),
                    constraints: Vec::new(),
                },
                DesignerProperty {
                    name: "icon_source".to_owned(),
                    kind: PropertyValueKind::IconSpec,
                    readable: true,
                    writable: true,
                    tokens: Vec::new(),
                    default_value: CapabilityValue::Null,
                    constraints: Vec::new(),
                },
            ],
            events: Vec::new(),
            commands: Vec::new(),
        };
        let schema = Schema::from_manifest("button", &manifest);

        // The enum's accepted tokens and measured default are both visible.
        assert_eq!(
            schema.kind_of(&name("elide")),
            Some(&PropertyType::Enum {
                variants: vec!["none".to_owned(), "right".to_owned()],
            })
        );
        assert_eq!(
            schema.default_of(&name("elide")),
            Some(&PropertyValue::Variant("right".to_owned()))
        );
        assert_eq!(
            schema.default_of(&name("text")),
            Some(&PropertyValue::Text("Demo".to_owned()))
        );

        // The icon source has no field here, so it is a named gap rather than an invented field.
        assert!(schema.kind_of(&name("icon_source")).is_none());
        assert_eq!(schema.gaps().len(), 1);
        assert!(
            schema.gaps()[0].contains("icon-spec"),
            "{:?}",
            schema.gaps()
        );
    }

    #[test]
    fn a_contract_kind_with_no_field_is_refused_rather_than_guessed() {
        let contract = ContractPropertySchema::new("bounds", PropertyValueKind::Rect, true, true);
        let error = Schema::from_property_schema("chart", &contract).expect_err("no field");
        assert!(error.message().contains("rect"), "{error}");
        assert!(error.message().contains("no editor schema"), "{error}");
    }

    #[test]
    fn rebinding_keeps_values_whose_type_is_unchanged_and_drops_one_whose_type_moved() {
        let text = name("text");
        let mut editor = Editor::new(Schema::new().with(
            text.clone(),
            PropertyType::Text {
                max_len: usize::MAX,
            },
        ));
        editor
            .set(node(1), text.clone(), "kept")
            .expect("text is accepted");

        // Another control declares the same property with the same type: the edit survives.
        editor.rebind(Schema::new().with(
            text.clone(),
            PropertyType::Text {
                max_len: usize::MAX,
            },
        ));
        assert_eq!(
            editor.value(node(1), &text),
            Some(&PropertyValue::Text("kept".to_owned()))
        );

        // The property is no longer described at all: the value is still kept for a later rebind.
        editor.rebind(Schema::new());
        assert_eq!(
            editor.value(node(1), &text),
            Some(&PropertyValue::Text("kept".to_owned()))
        );

        // The same property with a different family is not the same value any more.
        editor.rebind(Schema::new().with(
            text.clone(),
            PropertyType::Integer {
                min: None,
                max: None,
            },
        ));
        assert_eq!(editor.value(node(1), &text), None);
    }
}
