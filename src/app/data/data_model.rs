// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Data schema, the values a preview runs on, and the sample/empty/error distinction (task T13.01).
//!
//! The acceptance line is *"a sample does not stand in for runtime data, and it stores no secrets"*.
//! Both are about not letting a stand-in pass as the real thing, so the module makes the two hard to
//! confuse:
//!
//! * **A dataset says which kind it is.** [`Dataset`] is a [`Kind::Runtime`], a [`Kind::Sample`], an
//!   empty set or an [`Kind::Error`] with its reason; [`Kind::label`] spells the difference out
//!   ("sample (not runtime data)"), so a sample can never be read as the live values.
//! * **An error is surfaced, not papered over by a sample.** [`active`] answers the runtime data when
//!   it is there, an error when the fetch failed, and only then the sample — and the sample is always
//!   labelled. So "the fetch failed" cannot come out looking like "here are your values".
//! * **A sample refuses secrets.** [`Dataset::sample`] answers the offending field names instead of storing
//!   them, so a credential cannot be captured into the project by a preview run.
//! * **The values are the project's value vocabulary.** A field holds a
//!   [`PropertyValue`], so the data a preview binds is the same shape the rest of the designer
//!   already speaks.

#![allow(dead_code)]

use std::collections::BTreeMap;

use crate::contracts::model::PropertyValue;

/// The kinds of value a data field may hold.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum FieldKind {
    /// Free text.
    Text,
    /// A whole number.
    Integer,
    /// A fractional number.
    Number,
    /// A flag.
    Bool,
    /// One of a fixed set of names.
    Enum(Vec<String>),
}

impl FieldKind {
    /// The kind's name, as a caller writes it.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Integer => "integer",
            Self::Number => "number",
            Self::Bool => "bool",
            Self::Enum(_) => "enum",
        }
    }

    /// Reads a kind, or says what is not supported.
    pub fn parse(text: &str) -> Result<Self, String> {
        match text {
            "text" => Ok(Self::Text),
            "integer" => Ok(Self::Integer),
            "number" => Ok(Self::Number),
            "bool" => Ok(Self::Bool),
            _ => match text
                .strip_prefix("enum(")
                .and_then(|rest| rest.strip_suffix(')'))
            {
                Some(variants) => {
                    let names: Vec<String> = variants
                        .split(',')
                        .map(|name| name.trim().to_owned())
                        .collect();
                    if names.iter().any(String::is_empty) {
                        return Err(format!("{text:?} has an empty variant"));
                    }
                    Ok(Self::Enum(names))
                }
                None => Err(format!("{text:?} is not a data field kind")),
            },
        }
    }

    /// Whether a value is of this kind.
    pub fn accepts(&self, value: &PropertyValue) -> bool {
        match (self, value) {
            (Self::Text, PropertyValue::Text(_)) => true,
            (Self::Integer, PropertyValue::Integer(_)) => true,
            (Self::Number, PropertyValue::Number(_)) => true,
            (Self::Number, PropertyValue::Integer(_)) => true,
            (Self::Bool, PropertyValue::Bool(_)) => true,
            (Self::Enum(names), PropertyValue::Variant(name)) => names.contains(name),
            _ => false,
        }
    }

    /// The kind in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Enum(names) => format!("enum({})", names.join(",")),
            other => other.name().to_owned(),
        }
    }
}

/// One field of a data schema.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DataField {
    /// The field's name.
    pub name: String,
    /// What kind of value it holds.
    pub kind: FieldKind,
    /// Whether it must be present.
    pub required: bool,
}

/// A data schema: the fields a dataset is checked against, with its version.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Schema {
    /// The schema's name.
    pub name: String,
    /// The schema's version.
    pub version: u32,
    /// The fields, in declaration order.
    pub fields: Vec<DataField>,
}

impl Schema {
    /// A schema with no fields.
    pub fn new(name: impl Into<String>, version: u32) -> Self {
        Self {
            name: name.into(),
            version,
            fields: Vec::new(),
        }
    }

    /// The same schema with one more field.
    pub fn with(mut self, name: impl Into<String>, kind: FieldKind, required: bool) -> Self {
        self.fields.push(DataField {
            name: name.into(),
            kind,
            required,
        });
        self
    }

    /// The field with this name, if it is declared.
    pub fn field(&self, name: &str) -> Option<&DataField> {
        self.fields.iter().find(|field| field.name == name)
    }

    /// The schema in one line per field.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!("schema {} v{}", self.name, self.version)];
        for field in &self.fields {
            lines.push(format!(
                "  {}{}: {}",
                field.name,
                if field.required { " (required)" } else { "" },
                field.kind.render()
            ));
        }
        lines
    }
}

/// The values of one dataset.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Data {
    /// The values, by field name.
    pub values: BTreeMap<String, PropertyValue>,
}

impl Data {
    /// An empty set of values.
    pub fn new() -> Self {
        Self::default()
    }

    /// The same data with one more value.
    pub fn with(mut self, name: impl Into<String>, value: PropertyValue) -> Self {
        self.values.insert(name.into(), value);
        self
    }
}

/// What a dataset is.
#[derive(Clone, PartialEq, Debug)]
pub enum Kind {
    /// The live values the preview is running on.
    Runtime(Data),
    /// A stand-in, for designing before there is live data.
    Sample(Data),
    /// No values at all.
    Empty,
    /// The values could not be fetched; this is why.
    Error {
        /// Why the fetch failed.
        reason: String,
    },
}

impl Kind {
    /// The kind's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Runtime(_) => "runtime",
            Self::Sample(_) => "sample",
            Self::Empty => "empty",
            Self::Error { .. } => "error",
        }
    }

    /// The kind's values, if it has any.
    pub fn data(&self) -> Option<&Data> {
        match self {
            Self::Runtime(data) | Self::Sample(data) => Some(data),
            Self::Empty | Self::Error { .. } => None,
        }
    }

    /// A label that never lets a stand-in pass for the real thing.
    pub fn label(&self) -> String {
        match self {
            Self::Runtime(_) => "runtime data".to_owned(),
            Self::Sample(_) => "sample (not runtime data)".to_owned(),
            Self::Empty => "empty (no data)".to_owned(),
            Self::Error { reason } => format!("error: {reason}"),
        }
    }
}

/// A dataset: what kind it is, the schema version it was read against, and the values.
#[derive(Clone, PartialEq, Debug)]
pub struct Dataset {
    /// The schema version the data was read against.
    pub schema_version: u32,
    /// The kind, and the values when it has any.
    pub kind: Kind,
}

impl Dataset {
    /// A runtime dataset.
    pub fn runtime(schema_version: u32, data: Data) -> Self {
        Self {
            schema_version,
            kind: Kind::Runtime(data),
        }
    }

    /// A sample dataset, refusing to store a credential.
    pub fn sample(schema_version: u32, data: Data) -> Result<Self, Vec<String>> {
        let found = secrets(&data);
        if found.is_empty() {
            Ok(Self {
                schema_version,
                kind: Kind::Sample(data),
            })
        } else {
            Err(found)
        }
    }

    /// An empty dataset.
    pub fn empty(schema_version: u32) -> Self {
        Self {
            schema_version,
            kind: Kind::Empty,
        }
    }

    /// A dataset whose fetch failed, with the reason.
    pub fn error(schema_version: u32, reason: impl Into<String>) -> Self {
        Self {
            schema_version,
            kind: Kind::Error {
                reason: reason.into(),
            },
        }
    }

    /// Whether this is the live data.
    pub fn is_runtime(&self) -> bool {
        matches!(self.kind, Kind::Runtime(_))
    }

    /// The label that says what this dataset is.
    pub fn label(&self) -> String {
        self.kind.label()
    }
}

/// Which dataset a preview should use.
#[derive(Clone, PartialEq, Debug)]
pub enum Selection<'a> {
    /// The live data.
    Runtime(&'a Data),
    /// A stand-in, which is labelled as one.
    Sample(&'a Data),
    /// Nothing to show.
    Empty,
    /// The fetch failed; the reason is shown instead of a stand-in.
    Error(&'a str),
}

impl Selection<'_> {
    /// The selection as a label.
    pub fn label(&self) -> String {
        match self {
            Self::Runtime(_) => "runtime data".to_owned(),
            Self::Sample(_) => "sample (not runtime data)".to_owned(),
            Self::Empty => "empty (no data)".to_owned(),
            Self::Error(reason) => format!("error: {reason}"),
        }
    }

    /// Whether the selection is a stand-in.
    pub fn is_sample(&self) -> bool {
        matches!(self, Self::Sample(_))
    }

    /// The values, if the selection has any.
    pub fn data(&self) -> Option<&Data> {
        match self {
            Self::Runtime(data) | Self::Sample(data) => Some(data),
            Self::Empty | Self::Error(_) => None,
        }
    }
}

/// One thing wrong with a dataset against its schema.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Diagnostic {
    /// The field the problem is in, or `(data)` for a whole-set problem.
    pub field: String,
    /// What is wrong.
    pub reason: String,
}

impl Diagnostic {
    /// The diagnostic in one line.
    pub fn render(&self) -> String {
        format!("{}: {}", self.field, self.reason)
    }
}

/// Checks a dataset against its schema.
///
/// The version is checked first: data read against another schema version is reported rather than
/// checked field by field, because a version mismatch is the real problem.
pub fn validate(schema: &Schema, dataset: &Dataset) -> Vec<Diagnostic> {
    if dataset.schema_version != schema.version {
        return vec![Diagnostic {
            field: "(data)".to_owned(),
            reason: format!(
                "read against schema v{} but the schema is v{}",
                dataset.schema_version, schema.version
            ),
        }];
    }
    let Some(data) = dataset.kind.data() else {
        return Vec::new();
    };
    let mut diagnostics = Vec::new();
    for (name, value) in &data.values {
        match schema.field(name) {
            Some(field) => {
                if !field.kind.accepts(value) {
                    diagnostics.push(Diagnostic {
                        field: name.clone(),
                        reason: format!(
                            "{} is not a {} ({})",
                            value.family(),
                            field.kind.name(),
                            field.kind.render()
                        ),
                    });
                }
            }
            None => diagnostics.push(Diagnostic {
                field: name.clone(),
                reason: "the schema has no such field".to_owned(),
            }),
        }
    }
    for field in &schema.fields {
        if field.required && !data.values.contains_key(&field.name) {
            diagnostics.push(Diagnostic {
                field: field.name.clone(),
                reason: "a required field is missing".to_owned(),
            });
        }
    }
    diagnostics
}

/// Chooses the dataset to run on: runtime, else the error, else the sample, else empty.
///
/// The sample is the last resort and is always labelled, so it cannot quietly take the runtime data's
/// place.
pub fn active<'a>(runtime: Option<&'a Dataset>, sample: Option<&'a Dataset>) -> Selection<'a> {
    if let Some(dataset) = runtime {
        match &dataset.kind {
            Kind::Runtime(data) => return Selection::Runtime(data),
            Kind::Error { reason } => return Selection::Error(reason),
            _ => {}
        }
    }
    if let Some(dataset) = sample
        && let Kind::Sample(data) = &dataset.kind
    {
        return Selection::Sample(data);
    }
    Selection::Empty
}

/// The field names in a dataset that look like credential material.
pub fn secrets(data: &Data) -> Vec<String> {
    let mut found = Vec::new();
    for (name, value) in &data.values {
        if is_secret_name(name) {
            found.push(name.clone());
        } else if let PropertyValue::Text(text) = value
            && looks_like_a_credential(text)
        {
            found.push(name.clone());
        }
    }
    found
}

/// Whether a field name looks like credential material.
///
/// The name is split at separators and camelCase boundaries, and a word matches when a whole
/// segment is one of the words, or when the segments *joined* are — so `api_key`, `apiKey` and
/// `authToken` all match, while `author` does not (a substring match would have flagged it).
pub fn is_secret_name(name: &str) -> bool {
    const WORDS: [&str; 8] = [
        "password",
        "passwd",
        "secret",
        "token",
        "apikey",
        "privatekey",
        "credential",
        "auth",
    ];
    let segments = segments(name);
    if segments
        .iter()
        .any(|segment| WORDS.contains(&segment.as_str()))
    {
        return true;
    }
    WORDS.contains(&segments.concat().as_str())
}

/// Splits a name at separators and at lowercase-to-uppercase boundaries, lowercased.
fn segments(name: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut previous: Option<char> = None;
    for character in name.chars() {
        if !character.is_alphanumeric() {
            if !current.is_empty() {
                segments.push(std::mem::take(&mut current));
            }
            previous = None;
            continue;
        }
        if let Some(previous) = previous
            && previous.is_lowercase()
            && character.is_uppercase()
            && !current.is_empty()
        {
            segments.push(std::mem::take(&mut current));
        }
        current.extend(character.to_lowercase());
        previous = Some(character);
    }
    if !current.is_empty() {
        segments.push(current);
    }
    segments
}

/// Whether a text value looks like a credential by its prefix.
fn looks_like_a_credential(text: &str) -> bool {
    text.starts_with("sk-") || text.starts_with("ghp_") || text.starts_with("AKIA")
}

#[cfg(test)]
mod tests {
    use super::{
        Data, Dataset, FieldKind, Kind, Schema, active, is_secret_name, secrets, validate,
    };
    use crate::contracts::model::PropertyValue;

    fn schema() -> Schema {
        Schema::new("user", 3)
            .with("name", FieldKind::Text, true)
            .with("age", FieldKind::Integer, false)
            .with(
                "role",
                FieldKind::Enum(vec!["admin".to_owned(), "user".to_owned()]),
                false,
            )
    }

    fn text(value: &str) -> PropertyValue {
        PropertyValue::Text(value.to_owned())
    }

    #[test]
    fn a_sample_is_labelled_as_not_runtime_data() {
        let data = Data::new().with("name", text("Ada"));
        let dataset = Dataset::sample(3, data).expect("a sample");
        assert!(!dataset.is_runtime());
        assert_eq!(dataset.label(), "sample (not runtime data)");
        assert_eq!(dataset.kind.name(), "sample");
    }

    #[test]
    fn a_runtime_dataset_is_preferred_over_a_sample() {
        let sample = Dataset::sample(3, Data::new().with("name", text("Sam"))).expect("a sample");
        let runtime = Dataset::runtime(3, Data::new().with("name", text("Ada")));
        let selection = active(Some(&runtime), Some(&sample));
        assert_eq!(selection.label(), "runtime data");
        assert_eq!(
            selection.data().and_then(|data| data.values.get("name")),
            Some(&text("Ada"))
        );
    }

    #[test]
    fn an_error_is_surfaced_instead_of_falling_back_to_a_sample() {
        let sample = Dataset::sample(3, Data::new().with("name", text("Sam"))).expect("a sample");
        let failed = Dataset::error(3, "the endpoint refused");
        let selection = active(Some(&failed), Some(&sample));
        assert_eq!(selection.label(), "error: the endpoint refused");
        assert_eq!(selection.data(), None);
    }

    #[test]
    fn the_sample_is_used_only_when_there_is_no_runtime_data() {
        let sample = Dataset::sample(3, Data::new().with("name", text("Sam"))).expect("a sample");
        let selection = active(None, Some(&sample));
        assert!(selection.is_sample());
        assert_eq!(selection.label(), "sample (not runtime data)");
    }

    #[test]
    fn nothing_at_all_selects_empty() {
        let selection = active(None, None);
        assert_eq!(selection.label(), "empty (no data)");
        assert!(selection.data().is_none());
    }

    #[test]
    fn a_sample_refuses_to_store_a_credential() {
        let data = Data::new()
            .with("name", text("Ada"))
            .with("api_token", text("abc"));
        let refusal = Dataset::sample(3, data).expect_err("a refusal");
        assert_eq!(refusal, vec!["api_token"]);
    }

    #[test]
    fn a_secret_name_is_recognised_whatever_the_separators() {
        assert!(is_secret_name("api_key"));
        assert!(is_secret_name("APIKey"));
        assert!(is_secret_name("apiKey"));
        assert!(is_secret_name("client-secret"));
        assert!(is_secret_name("authToken"));
        assert!(!is_secret_name("name"));
        assert!(!is_secret_name("author"));
    }

    #[test]
    fn a_credential_shaped_value_is_caught_too() {
        let data = Data::new().with("key", text("sk-1234567890"));
        assert_eq!(secrets(&data), vec!["key"]);
    }

    #[test]
    fn a_valid_dataset_has_no_diagnostics() {
        let data = Data::new()
            .with("name", text("Ada"))
            .with("age", PropertyValue::Integer(36))
            .with("role", PropertyValue::Variant("admin".to_owned()));
        let dataset = Dataset::runtime(3, data);
        assert!(validate(&schema(), &dataset).is_empty());
    }

    #[test]
    fn an_unknown_field_is_reported() {
        let dataset = Dataset::runtime(
            3,
            Data::new()
                .with("name", text("Ada"))
                .with("nick", text("x")),
        );
        let diagnostics = validate(&schema(), &dataset);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].field, "nick");
        assert!(diagnostics[0].render().contains("no such field"));
    }

    #[test]
    fn a_missing_required_field_is_reported() {
        let dataset = Dataset::runtime(3, Data::new().with("age", PropertyValue::Integer(1)));
        let diagnostics = validate(&schema(), &dataset);
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.field == "name"
                    && diagnostic.reason.contains("required"))
        );
    }

    #[test]
    fn a_wrong_kind_is_reported() {
        let dataset = Dataset::runtime(3, Data::new().with("name", PropertyValue::Integer(1)));
        let diagnostics = validate(&schema(), &dataset);
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].render().contains("not a text"));
    }

    #[test]
    fn an_enum_value_outside_the_set_is_reported() {
        let dataset = Dataset::runtime(
            3,
            Data::new()
                .with("name", text("Ada"))
                .with("role", PropertyValue::Variant("root".to_owned())),
        );
        let diagnostics = validate(&schema(), &dataset);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].field, "role");
    }

    #[test]
    fn a_version_mismatch_is_reported_before_the_fields() {
        let old = Dataset::runtime(2, Data::new().with("name", text("Ada")));
        let diagnostics = validate(&schema(), &old);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].field, "(data)");
        assert!(diagnostics[0].render().contains("v2"));
    }

    #[test]
    fn the_schema_renders_its_fields_and_requiredness() {
        let lines = schema().render();
        assert_eq!(lines[0], "schema user v3");
        assert!(lines.iter().any(|line| line == "  name (required): text"));
        assert!(lines.iter().any(|line| line == "  role: enum(admin,user)"));
    }

    #[test]
    fn the_field_kinds_parse_and_render() {
        assert_eq!(FieldKind::parse("text").expect("a kind"), FieldKind::Text);
        assert_eq!(
            FieldKind::parse("enum(a,b)").expect("a kind"),
            FieldKind::Enum(vec!["a".to_owned(), "b".to_owned()])
        );
        assert!(FieldKind::parse("enum()").is_err());
        assert!(FieldKind::parse("blob").is_err());
    }

    #[test]
    fn the_errors_kind_carries_its_reason() {
        let dataset = Dataset::error(3, "offline");
        assert_eq!(dataset.kind.name(), "error");
        assert_eq!(dataset.kind.data(), None);
        assert_eq!(Kind::Empty.name(), "empty");
    }
}
