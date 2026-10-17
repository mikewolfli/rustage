// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Form validation: required fields, typed cross-field rules, submit/reset, and the rule that a
//! validation which arrived after the draft changed must not submit (task T13.09).
//!
//! The acceptance line is *"it uses typed library/business contracts rather than an invented
//! script; a stale validation does not submit, and preview and both generated modes agree"*. Three
//! design choices make that true:
//!
//! * **Rules are data, not a script.** [`Rule`] is a closed enum. There is no expression parser and
//!   no interpreter, so a rule the model cannot express is a rule the form cannot carry; the
//!   generated application never evaluates a string a designer typed (principle 12/17).
//! * **Staleness is a revision, not a timer.** A [`Draft`] carries the model revision its values
//!   were entered against, and every edit bumps it. An async [`Ticket`] records the revision it was
//!   started at; [`submit`] refuses a ticket whose revision is behind the draft, so a result that
//!   describes an older draft can never be committed as if it described the current one.
//! * **One reader for all three modes.** [`verdict_in`] is the single function that reads a form;
//!   the preview and the two generated modes pass the same form data through it, so they cannot
//!   disagree unless someone deliberately makes a mode skip a rule.

#![allow(dead_code)]

use std::collections::BTreeMap;

/// The kind of value a field holds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// Free text.
    Text,
    /// A whole number.
    Integer,
    /// A flag.
    Flag,
    /// A choice from a named set.
    Choice,
}

impl Kind {
    /// The kind's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Integer => "integer",
            Self::Flag => "flag",
            Self::Choice => "choice",
        }
    }

    /// Reads a kind's name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "text" => Some(Self::Text),
            "integer" => Some(Self::Integer),
            "flag" => Some(Self::Flag),
            "choice" => Some(Self::Choice),
            _ => None,
        }
    }
}

/// A field's value.
///
/// [`Value::Missing`] is distinct from [`Value::Text`] holding an empty string: the first is "no
/// value was entered at all", the second is "a value was entered and it is empty". A required field
/// rejects both, a length rule only applies to the second, and conflating them would lose that.
#[derive(Clone, PartialEq, Debug)]
pub enum Value {
    /// No value was entered.
    Missing,
    /// A text value.
    Text(String),
    /// A whole-number value.
    Integer(i64),
    /// A flag value.
    Flag(bool),
    /// A choice from a named set.
    Choice(String),
}

impl Value {
    /// The value's kind name.
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Text(_) => "text",
            Self::Integer(_) => "integer",
            Self::Flag(_) => "flag",
            Self::Choice(_) => "choice",
        }
    }

    /// Whether the field has no usable value: absent, or an empty text.
    pub fn is_blank(&self) -> bool {
        match self {
            Self::Missing => true,
            Self::Text(text) => text.is_empty(),
            _ => false,
        }
    }

    /// The value in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Missing => "missing".to_owned(),
            Self::Text(text) => text.clone(),
            Self::Integer(number) => number.to_string(),
            Self::Flag(flag) => flag.to_string(),
            Self::Choice(choice) => choice.clone(),
        }
    }
}

/// A typed validation rule.
///
/// This is the whole vocabulary: required-ness is a [`Field`] flag and the rest are these variants.
/// A rule that is not here is a rule the form does not have, which is why no interpreter exists.
#[derive(Clone, PartialEq, Debug)]
pub enum Rule {
    /// This field's value must equal another field's value.
    SameAs { other: String },
    /// This field's value must differ from another field's value.
    DifferentFrom { other: String },
    /// This field is required when another field equals a value.
    RequiredWhen { other: String, equals: Value },
    /// The text must be at least this many characters.
    MinLength(usize),
    /// The integer must be at least this.
    AtLeast(i64),
    /// The choice must be one of these.
    OneOf(Vec<String>),
}

impl Rule {
    /// Checks the rule, answering the problem it found, or `None` when it holds.
    ///
    /// A rule whose operand has the wrong kind is skipped: the type mismatch is already reported by
    /// [`validate`], and a second, confusing message about the same field helps nobody.
    pub fn check(
        &self,
        field: &str,
        value: &Value,
        values: &BTreeMap<String, Value>,
    ) -> Option<String> {
        match self {
            Self::SameAs { other } => {
                if value != &values.get(other).cloned().unwrap_or(Value::Missing) {
                    Some(format!("{field} must equal {other}"))
                } else {
                    None
                }
            }
            Self::DifferentFrom { other } => {
                let theirs = values.get(other).cloned().unwrap_or(Value::Missing);
                if !value.is_blank() && !theirs.is_blank() && value == &theirs {
                    Some(format!("{field} must differ from {other}"))
                } else {
                    None
                }
            }
            Self::RequiredWhen { other, equals } => {
                let theirs = values.get(other).cloned().unwrap_or(Value::Missing);
                if &theirs == equals && value.is_blank() {
                    Some(format!(
                        "{field} is required when {other} is {}",
                        equals.render()
                    ))
                } else {
                    None
                }
            }
            Self::MinLength(minimum) => match value {
                Value::Text(text) if text.len() < *minimum => {
                    Some(format!("{field} must be at least {minimum} characters"))
                }
                _ => None,
            },
            Self::AtLeast(minimum) => match value {
                Value::Integer(number) if number < minimum => {
                    Some(format!("{field} must be at least {minimum}"))
                }
                _ => None,
            },
            Self::OneOf(allowed) => match value {
                Value::Choice(choice) if !allowed.contains(choice) => {
                    Some(format!("{field} must be one of {}", allowed.join(", ")))
                }
                _ => None,
            },
        }
    }

    /// The rule in one line, so a caller can show what the form requires.
    pub fn describe(&self) -> String {
        match self {
            Self::SameAs { other } => format!("must equal {other}"),
            Self::DifferentFrom { other } => format!("must differ from {other}"),
            Self::RequiredWhen { other, equals } => {
                format!("required when {other} is {}", equals.render())
            }
            Self::MinLength(minimum) => format!("at least {minimum} characters"),
            Self::AtLeast(minimum) => format!("at least {minimum}"),
            Self::OneOf(allowed) => format!("one of {}", allowed.join(", ")),
        }
    }
}

/// One field of a form.
#[derive(Clone, PartialEq, Debug)]
pub struct Field {
    /// The field's name, which is also its key in a [`Draft`].
    pub name: String,
    /// The kind of value it holds.
    pub kind: Kind,
    /// Whether it must have a non-blank value.
    pub required: bool,
    /// The typed rules that apply to it.
    pub rules: Vec<Rule>,
}

impl Field {
    /// A field of a kind, not required and with no rules.
    pub fn new(name: &str, kind: Kind) -> Self {
        Self {
            name: name.to_owned(),
            kind,
            required: false,
            rules: Vec::new(),
        }
    }

    /// The same field, required.
    pub fn requiring(mut self) -> Self {
        self.required = true;
        self
    }

    /// The same field, with one more rule.
    pub fn with(mut self, rule: Rule) -> Self {
        self.rules.push(rule);
        self
    }
}

/// Whether a value is acceptable for a field of this kind.
///
/// [`Value::Missing`] is accepted here: absence is a value the field may legitimately hold, and it
/// is required-ness, not the type, that forbids it.
pub fn accepts(kind: Kind, value: &Value) -> bool {
    matches!(
        (kind, value),
        (_, Value::Missing)
            | (Kind::Text, Value::Text(_))
            | (Kind::Integer, Value::Integer(_))
            | (Kind::Flag, Value::Flag(_))
            | (Kind::Choice, Value::Choice(_))
    )
}

/// A form: the fields and the rules that decide whether a submission is valid.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Form {
    /// The fields, in display order.
    pub fields: Vec<Field>,
}

impl Form {
    /// A form over these fields.
    pub fn new(fields: Vec<Field>) -> Self {
        Self { fields }
    }

    /// The field with this name, if any.
    pub fn field(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|field| field.name == name)
    }

    /// The field with this name, for editing.
    pub fn field_mut(&mut self, name: &str) -> Option<&mut Field> {
        self.fields.iter_mut().find(|field| field.name == name)
    }
}

/// A form being filled in.
///
/// The revision is the model revision the values were entered against. Every [`Draft::set`] and
/// [`reset`] bumps it, which is what makes an in-flight validation of an older revision detectable
/// rather than silently applied.
#[derive(Clone, PartialEq, Debug)]
pub struct Draft {
    /// The values, by field name.
    pub values: BTreeMap<String, Value>,
    revision: u64,
    saves: u64,
}

impl Draft {
    /// An empty draft at a revision.
    pub fn new(revision: u64) -> Self {
        Self {
            values: BTreeMap::new(),
            revision,
            saves: 0,
        }
    }

    /// The current revision.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// How many times a submission has been committed.
    pub fn saves(&self) -> u64 {
        self.saves
    }

    /// Enters a value, which advances the revision so an older validation is now stale.
    pub fn set(&mut self, field: &str, value: Value) {
        self.values.insert(field.to_owned(), value);
        self.revision += 1;
    }

    /// Records that a submission was accepted. Only [`submit`]'s caller does this, never a reset.
    pub fn commit(&mut self) {
        self.saves += 1;
    }
}

/// Clears every entered value and advances the revision.
///
/// A reset has no side effect beyond the draft: it composes no save, and it makes any in-flight
/// validation stale.
pub fn reset(draft: &mut Draft) {
    draft.values.clear();
    draft.revision += 1;
}

/// One validation problem, attributed to the field that carries it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Issue {
    /// The field the problem belongs to.
    pub field: String,
    /// What is wrong.
    pub problem: String,
}

impl Issue {
    /// A problem on a field.
    pub fn new(field: &str, problem: String) -> Self {
        Self {
            field: field.to_owned(),
            problem,
        }
    }
}

/// The result of validating a form.
#[derive(Clone, PartialEq, Debug)]
pub enum Verdict {
    /// Every field and rule holds.
    Valid,
    /// The problems that were found, in field order.
    Invalid(Vec<Issue>),
}

impl Verdict {
    /// Whether the form is valid.
    pub fn is_valid(&self) -> bool {
        matches!(self, Self::Valid)
    }

    /// The problems, empty when valid.
    pub fn issues(&self) -> &[Issue] {
        match self {
            Self::Valid => &[],
            Self::Invalid(issues) => issues,
        }
    }

    /// The verdict in one line per problem, or one line saying it is valid.
    pub fn render(&self) -> Vec<String> {
        match self {
            Self::Valid => vec!["valid".to_owned()],
            Self::Invalid(issues) => issues
                .iter()
                .map(|issue| format!("invalid: {}: {}", issue.field, issue.problem))
                .collect(),
        }
    }
}

/// Validates a set of values against a form.
///
/// Every field is checked for type and required-ness and then against each rule; a field never
/// reported as valid by omission, which is what "a type mismatch or a missing field is diagnosed"
/// means here.
pub fn validate(form: &Form, values: &BTreeMap<String, Value>) -> Verdict {
    let mut issues = Vec::new();
    for field in &form.fields {
        let value = values.get(&field.name).cloned().unwrap_or(Value::Missing);
        if field.required && value.is_blank() {
            issues.push(Issue::new(
                &field.name,
                format!("{} is required", field.name),
            ));
        }
        if !accepts(field.kind, &value) {
            issues.push(Issue::new(
                &field.name,
                format!(
                    "{} expects a {}, not {}",
                    field.name,
                    field.kind.name(),
                    value.kind_name()
                ),
            ));
        }
        for rule in &field.rules {
            if let Some(problem) = rule.check(&field.name, &value, values) {
                issues.push(Issue::new(&field.name, problem));
            }
        }
    }
    if issues.is_empty() {
        Verdict::Valid
    } else {
        Verdict::Invalid(issues)
    }
}

/// Where a validation is being read: the preview, or one of the two generated modes.
///
/// The three exist because the artifacts differ; the *validation* must not, which is why they are
/// an argument to [`verdict_in`] rather than a branch inside [`validate`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// The development preview.
    Preview,
    /// Mode 1: the JSON + CSS runtime load.
    Runtime,
    /// Mode 2: generated Rust source.
    Static,
}

/// The verdict a mode reaches for a form. All three use the same reader, deliberately.
pub fn verdict_in(mode: Mode, form: &Form, values: &BTreeMap<String, Value>) -> Verdict {
    // `mode` is a parameter so a test can pin the three modes to the same verdict; the body does not
    // branch on it, because a mode that skipped a rule would be exactly the drift this forbids.
    let _ = mode;
    validate(form, values)
}

/// An async validation that is still running, started against a draft revision.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Ticket {
    /// The field being validated.
    pub field: String,
    /// The draft revision it was started at.
    pub revision: u64,
}

impl Ticket {
    /// A ticket for a field at a revision.
    pub fn new(field: &str, revision: u64) -> Self {
        Self {
            field: field.to_owned(),
            revision,
        }
    }
}

/// What an async validation's return means for the draft it was started for.
#[derive(Clone, PartialEq, Debug)]
pub enum Resolution {
    /// The result describes the current draft and stands.
    Current(Result<(), String>),
    /// The draft changed while it ran, so the result is discarded and must not submit.
    Stale,
    /// The validation was cancelled, so it did not complete and must not submit.
    Cancelled,
}

impl Resolution {
    /// The problem to record, if this resolution blocks the field; `None` when it is a pass or stale.
    pub fn problem(&self) -> Option<String> {
        match self {
            Self::Current(Err(problem)) => Some(problem.clone()),
            _ => None,
        }
    }
}

/// Resolves an async validation's return against the draft's current revision.
pub fn resolve(
    ticket: &Ticket,
    current_revision: u64,
    cancelled: bool,
    result: Result<(), String>,
) -> Resolution {
    if cancelled {
        return Resolution::Cancelled;
    }
    if ticket.revision != current_revision {
        return Resolution::Stale;
    }
    Resolution::Current(result)
}

/// The result of asking to submit a form.
#[derive(Clone, PartialEq, Debug)]
pub enum Submit {
    /// The form is valid and no validation is outstanding; a save may follow.
    Accepted,
    /// The submission is refused, with the reason.
    Refused(String),
}

impl Submit {
    /// Whether the submission was accepted.
    pub fn is_accepted(&self) -> bool {
        matches!(self, Self::Accepted)
    }

    /// The outcome in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Accepted => "submitted".to_owned(),
            Self::Refused(reason) => format!("refused: {reason}"),
        }
    }
}

/// Decides whether a draft may be submitted.
///
/// The order matters and encodes the acceptance line: a validation started against an older
/// revision is refused as stale *before* the form is even read, so a result that describes the wrong
/// draft can never slip through as a pass.
pub fn submit(form: &Form, draft: &Draft, pending: Option<&Ticket>) -> Submit {
    if let Some(ticket) = pending {
        if ticket.revision != draft.revision {
            return Submit::Refused(format!(
                "a validation of {} started at revision {} is stale; the draft is at revision {}",
                ticket.field, ticket.revision, draft.revision
            ));
        }
        return Submit::Refused(format!("{} is still being validated", ticket.field));
    }
    match validate(form, &draft.values) {
        Verdict::Valid => Submit::Accepted,
        Verdict::Invalid(issues) => Submit::Refused(format!(
            "the form has {} problem{}",
            issues.len(),
            if issues.len() == 1 { "" } else { "s" }
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Draft, Field, Form, Kind, Mode, Resolution, Rule, Submit, Ticket, Value, reset, resolve,
        submit, validate, verdict_in,
    };
    use std::collections::BTreeMap;

    fn values(entries: &[(&str, Value)]) -> BTreeMap<String, Value> {
        entries
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect()
    }

    fn signup() -> Form {
        Form::new(vec![
            Field::new("email", Kind::Text)
                .requiring()
                .with(Rule::MinLength(3)),
            Field::new("password", Kind::Text).requiring(),
            Field::new("confirm", Kind::Text).with(Rule::SameAs {
                other: "password".to_owned(),
            }),
            Field::new("age", Kind::Integer).with(Rule::AtLeast(18)),
            Field::new("plan", Kind::Choice)
                .with(Rule::OneOf(vec!["free".to_owned(), "pro".to_owned()])),
        ])
    }

    #[test]
    fn a_required_field_rejects_a_missing_and_a_blank_value() {
        let form = Form::new(vec![Field::new("name", Kind::Text).requiring()]);
        assert!(!validate(&form, &values(&[])).is_valid());
        assert!(!validate(&form, &values(&[("name", Value::Text(String::new()))])).is_valid());
        assert!(validate(&form, &values(&[("name", Value::Text("Ada".to_owned()))])).is_valid());
    }

    #[test]
    fn a_type_mismatch_is_diagnosed_rather_than_ignored() {
        let form = Form::new(vec![Field::new("age", Kind::Integer)]);
        let verdict = validate(
            &form,
            &values(&[("age", Value::Text("eighteen".to_owned()))]),
        );
        assert!(verdict.issues()[0].problem.contains("expects a integer"));
        assert_eq!(verdict.issues()[0].field, "age");
    }

    #[test]
    fn a_cross_field_rule_compares_two_fields() {
        let form = signup();
        let mismatched = validate(
            &form,
            &values(&[
                ("email", Value::Text("a@b".to_owned())),
                ("password", Value::Text("s3cret".to_owned())),
                ("confirm", Value::Text("other".to_owned())),
            ]),
        );
        assert!(
            mismatched
                .issues()
                .iter()
                .any(|issue| issue.problem.contains("must equal password"))
        );
        let matched = validate(
            &form,
            &values(&[
                ("email", Value::Text("a@b".to_owned())),
                ("password", Value::Text("s3cret".to_owned())),
                ("confirm", Value::Text("s3cret".to_owned())),
                ("age", Value::Integer(30)),
                ("plan", Value::Choice("pro".to_owned())),
            ]),
        );
        assert!(matched.is_valid(), "{:?}", matched.issues());
    }

    #[test]
    fn a_conditional_rule_only_applies_when_its_condition_holds() {
        let form = Form::new(vec![
            Field::new("kind", Kind::Choice),
            Field::new("other", Kind::Choice).with(Rule::RequiredWhen {
                other: "kind".to_owned(),
                equals: Value::Choice("custom".to_owned()),
            }),
        ]);
        let when_absent = validate(
            &form,
            &values(&[("kind", Value::Choice("builtin".to_owned()))]),
        );
        assert!(when_absent.is_valid());
        let when_present = validate(
            &form,
            &values(&[("kind", Value::Choice("custom".to_owned()))]),
        );
        assert!(!when_present.is_valid());
    }

    #[test]
    fn an_invalid_form_is_not_submitted() {
        let form = signup();
        let draft = Draft::new(1);
        let result = submit(&form, &draft, None);
        assert!(!result.is_accepted());
        assert!(result.render().starts_with("refused:"));
    }

    #[test]
    fn a_valid_form_is_submitted_exactly_once_per_commit() {
        let form = Form::new(vec![Field::new("name", Kind::Text).requiring()]);
        let mut draft = Draft::new(1);
        draft.set("name", Value::Text("Ada".to_owned()));
        let result = submit(&form, &draft, None);
        assert_eq!(result, Submit::Accepted);
        assert_eq!(draft.saves(), 0);
        draft.commit();
        assert_eq!(draft.saves(), 1);
    }

    #[test]
    fn a_stale_ticket_cannot_submit() {
        let form = Form::new(vec![Field::new("name", Kind::Text).requiring()]);
        let mut draft = Draft::new(1);
        draft.set("name", Value::Text("Ada".to_owned()));
        let started = draft.revision();
        // The draft changes while the async validation is still running.
        draft.set("name", Value::Text("Grace".to_owned()));
        let ticket = Ticket::new("name", started);
        let result = submit(&form, &draft, Some(&ticket));
        assert!(!result.is_accepted());
        assert!(result.render().contains("stale"));
    }

    #[test]
    fn a_fresh_ticket_still_blocks_until_it_returns() {
        let form = Form::new(vec![Field::new("name", Kind::Text).requiring()]);
        let mut draft = Draft::new(1);
        draft.set("name", Value::Text("Ada".to_owned()));
        let ticket = Ticket::new("name", draft.revision());
        let result = submit(&form, &draft, Some(&ticket));
        assert!(result.render().contains("still being validated"));
    }

    #[test]
    fn a_stale_async_result_is_discarded() {
        let ticket = Ticket::new("name", 4);
        assert_eq!(
            resolve(&ticket, 5, false, Err("taken".to_owned())),
            Resolution::Stale
        );
    }

    #[test]
    fn a_current_async_failure_is_kept_and_a_current_pass_stands() {
        let ticket = Ticket::new("name", 5);
        let failed = resolve(&ticket, 5, false, Err("taken".to_owned()));
        assert_eq!(failed.problem(), Some("taken".to_owned()));
        let passed = resolve(&ticket, 5, false, Ok(()));
        assert_eq!(passed.problem(), None);
    }

    #[test]
    fn a_cancelled_async_validation_is_not_a_pass() {
        let ticket = Ticket::new("name", 5);
        assert_eq!(resolve(&ticket, 5, true, Ok(())), Resolution::Cancelled);
    }

    #[test]
    fn resetting_discards_values_and_advances_the_revision_without_saving() {
        let mut draft = Draft::new(1);
        draft.set("name", Value::Text("Ada".to_owned()));
        let before = draft.revision();
        reset(&mut draft);
        assert!(draft.values.is_empty());
        assert!(draft.revision() > before);
        assert_eq!(draft.saves(), 0);
    }

    #[test]
    fn the_three_modes_reach_the_same_verdict() {
        let form = signup();
        let entered = values(&[
            ("email", Value::Text("a@b".to_owned())),
            ("password", Value::Text("s3cret".to_owned())),
            ("confirm", Value::Text("other".to_owned())),
        ]);
        let preview = verdict_in(Mode::Preview, &form, &entered);
        let runtime = verdict_in(Mode::Runtime, &form, &entered);
        let statically = verdict_in(Mode::Static, &form, &entered);
        assert_eq!(preview, runtime);
        assert_eq!(runtime, statically);
        assert!(!preview.is_valid());
    }

    #[test]
    fn rules_are_data_that_describe_themselves() {
        assert_eq!(Rule::MinLength(3).describe(), "at least 3 characters");
        assert_eq!(
            Rule::OneOf(vec!["a".to_owned(), "b".to_owned()]).describe(),
            "one of a, b"
        );
        assert_eq!(Kind::parse("integer"), Some(Kind::Integer));
        assert_eq!(Kind::parse("script"), None);
    }
}
