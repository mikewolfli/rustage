// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! F13: form validation — typed rules, submit, reset and async staleness (task T13.09).
//!
//! One command handler for the form domain; the dispatcher lives in [`super`](self). The handler
//! builds a form and a draft from the operations, then runs the requested action. A refused submit
//! or an invalid form is reported as a real finding (a `false` return), never as a success shape.

use super::*;
use crate::app::data::validation::{
    Draft, Field, Form, Kind, Mode, Rule, Ticket, Value, reset, submit,
};

/// Splits `name:value`.
fn split_pair<'a>(rest: &'a str, what: &str) -> Result<(&'a str, &'a str), String> {
    rest.split_once(':')
        .ok_or_else(|| format!("{what} needs <name>:<value>, got {rest:?}"))
}

/// Splits `name=value`.
fn split_eq<'a>(rest: &'a str, what: &str) -> Result<(&'a str, &'a str), String> {
    rest.split_once('=')
        .ok_or_else(|| format!("{what} needs <name>=<value>, got {rest:?}"))
}

/// The declared field with this name.
fn field_mut<'a>(fields: &'a mut [Field], name: &str) -> Option<&'a mut Field> {
    fields.iter_mut().find(|field| field.name == name)
}

/// Attaches a rule to a declared field.
fn add_rule(fields: &mut [Field], name: &str, rule: Rule) -> Result<(), String> {
    match field_mut(fields, name) {
        Some(field) => {
            field.rules.push(rule);
            Ok(())
        }
        None => Err(format!("a rule names an undeclared field {name:?}")),
    }
}

/// Builds a form, validates a draft and decides whether it may be submitted (task T13.09).
pub(super) fn form(options: &crate::cli::FormOptions) -> bool {
    let mut fields: Vec<Field> = Vec::new();
    let mut base_revision: u64 = 0;
    let mut entered: Vec<(String, Value)> = Vec::new();
    let mut pending: Option<Ticket> = None;
    let mut mode = Mode::Preview;
    let mut action = String::new();
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<(), String> = match verb {
            "field" => match split_pair(argument, "field") {
                Ok((name, kind)) => match Kind::parse(kind) {
                    Some(kind) => {
                        fields.push(Field::new(name, kind));
                        Ok(())
                    }
                    None => Err(format!(
                        "{kind:?} is not a field kind (text, integer, flag, choice)"
                    )),
                },
                Err(reason) => Err(reason),
            },
            "required" => match field_mut(&mut fields, argument) {
                Some(field) => {
                    field.required = true;
                    Ok(())
                }
                None => Err(format!("required names an undeclared field {argument:?}")),
            },
            "same-as" => match split_pair(argument, "same-as") {
                Ok((name, other)) => add_rule(
                    &mut fields,
                    name,
                    Rule::SameAs {
                        other: other.to_owned(),
                    },
                ),
                Err(reason) => Err(reason),
            },
            "different-from" => match split_pair(argument, "different-from") {
                Ok((name, other)) => add_rule(
                    &mut fields,
                    name,
                    Rule::DifferentFrom {
                        other: other.to_owned(),
                    },
                ),
                Err(reason) => Err(reason),
            },
            "required-when" => match split_pair(argument, "required-when")
                .and_then(|(name, rest)| split_eq(rest, "required-when").map(|(o, v)| (name, o, v)))
            {
                Ok((name, other, value)) => add_rule(
                    &mut fields,
                    name,
                    Rule::RequiredWhen {
                        other: other.to_owned(),
                        equals: Value::Text(value.to_owned()),
                    },
                ),
                Err(reason) => Err(reason),
            },
            "min-length" => match split_pair(argument, "min-length").and_then(|(name, n)| {
                n.parse::<usize>()
                    .map(|n| (name, n))
                    .map_err(|_| format!("{n:?} is not a number"))
            }) {
                Ok((name, n)) => add_rule(&mut fields, name, Rule::MinLength(n)),
                Err(reason) => Err(reason),
            },
            "at-least" => match split_pair(argument, "at-least").and_then(|(name, n)| {
                n.parse::<i64>()
                    .map(|n| (name, n))
                    .map_err(|_| format!("{n:?} is not a number"))
            }) {
                Ok((name, n)) => add_rule(&mut fields, name, Rule::AtLeast(n)),
                Err(reason) => Err(reason),
            },
            "one-of" => match split_pair(argument, "one-of") {
                Ok((name, rest)) => add_rule(
                    &mut fields,
                    name,
                    Rule::OneOf(rest.split('|').map(str::to_owned).collect()),
                ),
                Err(reason) => Err(reason),
            },
            "value" => match split_eq(argument, "value") {
                Ok((name, text)) => {
                    if field_mut(&mut fields, name).is_none() {
                        Err(format!("value names an undeclared field {name:?}"))
                    } else {
                        entered.push((name.to_owned(), Value::Text(text.to_owned())));
                        Ok(())
                    }
                }
                Err(reason) => Err(reason),
            },
            "revision" => match argument.parse::<u64>() {
                Ok(n) => {
                    base_revision = n;
                    Ok(())
                }
                Err(_) => Err(format!("{argument:?} is not a revision")),
            },
            "pending" => match split_pair(argument, "pending").and_then(|(field, n)| {
                n.parse::<u64>()
                    .map(|n| (field, n))
                    .map_err(|_| format!("{n:?} is not a revision"))
            }) {
                Ok((field, n)) => {
                    pending = Some(Ticket::new(field, n));
                    Ok(())
                }
                Err(reason) => Err(reason),
            },
            "mode" => match argument {
                "preview" => {
                    mode = Mode::Preview;
                    Ok(())
                }
                "runtime" => {
                    mode = Mode::Runtime;
                    Ok(())
                }
                "static" => {
                    mode = Mode::Static;
                    Ok(())
                }
                other => Err(format!("{other:?} is not a mode")),
            },
            "validate" | "submit" | "reset" | "show" => {
                action = verb.to_owned();
                Ok(())
            }
            other => Err(format!("{other:?} is not a form operation")),
        };
        if let Err(reason) = step {
            return report(reason);
        }
    }

    let form = Form::new(fields);
    let mut draft = Draft::new(base_revision);
    for (name, value) in entered {
        draft.set(&name, value);
    }

    if action == "reset" {
        reset(&mut draft);
        println!("reset: draft at revision {}", draft.revision());
        return true;
    }

    println!(
        "form: {} field(s), draft revision {}, saves {}",
        form.fields.len(),
        draft.revision(),
        draft.saves()
    );
    for field in &form.fields {
        let mut line = format!(
            "  {}: {}{}",
            field.name,
            field.kind.name(),
            if field.required { " required" } else { "" }
        );
        for rule in &field.rules {
            line.push_str(&format!(", {}", rule.describe()));
        }
        println!("{line}");
    }

    match action.as_str() {
        "submit" => {
            let result = submit(&form, &draft, pending.as_ref());
            println!("{}", result.render());
            if result.is_accepted() {
                draft.commit();
                println!("saves: {}", draft.saves());
                true
            } else {
                false
            }
        }
        _ => {
            let verdict = crate::app::data::validation::verdict_in(mode, &form, &draft.values);
            for line in verdict.render() {
                println!("{line}");
            }
            verdict.is_valid()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::data::validation::{Kind, Rule, Value};

    #[test]
    fn splits_name_value_pairs() {
        assert_eq!(split_pair("email:text", "field"), Ok(("email", "text")));
        assert_eq!(split_eq("name=Ada", "value"), Ok(("name", "Ada")));
        assert!(split_pair("email", "field").is_err());
        assert!(split_eq("name", "value").is_err());
    }

    #[test]
    fn a_rule_on_an_undeclared_field_is_refused() {
        let mut fields = vec![Field::new("email", Kind::Text)];
        assert!(
            add_rule(&mut fields, "missing", Rule::MinLength(3))
                .unwrap_err()
                .contains("undeclared")
        );
        assert!(add_rule(&mut fields, "email", Rule::MinLength(3)).is_ok());
    }

    #[test]
    fn a_value_is_recorded_as_text() {
        let mut fields = vec![Field::new("name", Kind::Text)];
        field_mut(&mut fields, "name").expect("a field");
        assert!(field_mut(&mut fields, "name").is_some());
        assert!(field_mut(&mut fields, "absent").is_none());
        let value = Value::Text("Ada".to_owned());
        assert_eq!(value.render(), "Ada");
    }

    #[test]
    fn the_form_handler_accepts_a_valid_form_and_refuses_an_invalid_one() {
        let mut valid = crate::cli::FormOptions::new();
        valid.ops = vec![
            "field:name:text".to_owned(),
            "required:name".to_owned(),
            "value:name=Ada".to_owned(),
            "submit".to_owned(),
        ];
        assert!(form(&valid));

        let mut invalid = crate::cli::FormOptions::new();
        invalid.ops = vec![
            "field:name:text".to_owned(),
            "required:name".to_owned(),
            "submit".to_owned(),
        ];
        assert!(!form(&invalid));
    }

    #[test]
    fn an_unknown_verb_is_refused() {
        let mut options = crate::cli::FormOptions::new();
        options.ops = vec!["explode:now".to_owned()];
        assert!(!form(&options));
    }
}
