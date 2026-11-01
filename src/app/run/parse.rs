// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The argument parsers the run dispatcher hands to its domain modules.
//!
//! These are the `<field>:<value>` readers behind the CLI operations. They live here rather than in
//! the dispatcher so `mod.rs` stays the scheduler and the shared helper it says it is (principle
//! #63). A parse either produces the domain value or names what it could not read; none of them
//! invents a default.

use super::unescape;

/// A diagnostic from `<severity>:<source>:<start>:<end>:<message>`.
pub(super) fn parse_diagnostic(
    argument: &str,
) -> Result<crate::app::language::diagnostics::Diagnostic, String> {
    use crate::app::language::diagnostics::{Diagnostic, Severity, Source};
    use crate::app::language::navigation::Span;
    let fields: Vec<&str> = argument.splitn(5, ':').collect();
    let [severity, source, start, end, message] = fields.as_slice() else {
        return Err(format!(
            "{argument:?} is not <severity>:<source>:<s>:<e>:<message>"
        ));
    };
    let severity =
        Severity::from_name(severity).ok_or_else(|| format!("{severity:?} is not a severity"))?;
    let source = Source::from_name(source).ok_or_else(|| format!("{source:?} is not a source"))?;
    let number = |text: &str| -> Result<usize, String> {
        text.parse()
            .map_err(|_| format!("{text:?} is not an offset"))
    };
    Ok(Diagnostic::new(
        Span::new(number(start)?, number(end)?),
        severity,
        source,
        *message,
    ))
}

/// A token from `<kind>:<start>:<end>`.
pub(super) fn parse_token(
    argument: &str,
) -> Result<crate::app::language::diagnostics::SemanticToken, String> {
    use crate::app::language::diagnostics::{SemanticToken, TokenKind};
    use crate::app::language::navigation::Span;
    let fields: Vec<&str> = argument.splitn(3, ':').collect();
    let [kind, start, end] = fields.as_slice() else {
        return Err(format!("{argument:?} is not <kind>:<start>:<end>"));
    };
    let kind = TokenKind::from_name(kind).ok_or_else(|| format!("{kind:?} is not a token kind"))?;
    let number = |text: &str| -> Result<usize, String> {
        text.parse()
            .map_err(|_| format!("{text:?} is not an offset"))
    };
    Ok(SemanticToken {
        span: Span::new(number(start)?, number(end)?),
        kind,
    })
}

/// An action from `<title>:<at>:<remove>:<insert>`.
pub(super) fn parse_action(
    argument: &str,
) -> Result<crate::app::language::diagnostics::CodeAction, String> {
    use crate::app::language::diagnostics::{ActionKind, CodeAction, TextEdit};
    let fields: Vec<&str> = argument.rsplitn(4, ':').collect();
    let [insert, remove, at, title] = fields.as_slice() else {
        return Err(format!(
            "{argument:?} is not <title>:<at>:<remove>:<insert>"
        ));
    };
    let at: usize = at.parse().map_err(|_| format!("{at:?} is not an offset"))?;
    let remove: usize = remove
        .parse()
        .map_err(|_| format!("{remove:?} is not a length"))?;
    Ok(CodeAction::new(
        *title,
        ActionKind::QuickFix,
        vec![TextEdit::new(at, remove, unescape(insert))],
    ))
}

/// A `(uri, start, end)` from `<uri>:<start>:<end>` (the uri may contain colons).
pub(super) fn parse_uri_range(argument: &str) -> Result<(String, usize, usize), String> {
    let fields: Vec<&str> = argument.rsplitn(3, ':').collect();
    let [end, start, uri] = fields.as_slice() else {
        return Err(format!("{argument:?} is not <uri>:<start>:<end>"));
    };
    let number = |text: &str| -> Result<usize, String> {
        text.parse()
            .map_err(|_| format!("{text:?} is not an offset"))
    };
    Ok(((*uri).to_owned(), number(start)?, number(end)?))
}

/// A file edit from `<uri>:<baseline>:<at>:<remove>:<insert>` (the uri may contain colons).
pub(super) fn parse_file_edit(
    argument: &str,
) -> Result<crate::app::language::workspace_edit::FileEdit, String> {
    use crate::app::language::diagnostics::TextEdit;
    use crate::app::language::workspace_edit::FileEdit;
    let fields: Vec<&str> = argument.rsplitn(5, ':').collect();
    let [insert, remove, at, baseline, uri] = fields.as_slice() else {
        return Err(format!(
            "{argument:?} is not <uri>:<baseline>:<at>:<remove>:<insert>"
        ));
    };
    let baseline: u64 = baseline
        .parse()
        .map_err(|_| format!("{baseline:?} is not a version"))?;
    let at: usize = at.parse().map_err(|_| format!("{at:?} is not an offset"))?;
    let remove: usize = remove
        .parse()
        .map_err(|_| format!("{remove:?} is not a length"))?;
    Ok(FileEdit::new(
        *uri,
        baseline,
        vec![TextEdit::new(at, remove, unescape(insert))],
    ))
}

/// An entry from `<key>:<requires|->:<detail>`.
pub(super) fn parse_entry(
    argument: &str,
) -> Result<crate::app::language::schema_docs::Entry, String> {
    use crate::app::language::schema_docs::Entry;
    let (key, rest) = argument
        .split_once(':')
        .ok_or_else(|| format!("{argument:?} has no requirement"))?;
    let (requires, detail) = rest
        .split_once(':')
        .ok_or_else(|| format!("{rest:?} has no detail"))?;
    let entry = Entry::new(key, "schema", unescape(detail));
    if requires == "-" || requires.is_empty() {
        Ok(entry)
    } else {
        Ok(entry.requiring(requires))
    }
}

/// An offset from an argument.
pub(super) fn number(text: &str) -> Result<usize, String> {
    text.parse()
        .map_err(|_| format!("{text:?} is not an offset"))
}

/// A widget from `<id>:<name>:<category>`.
pub(super) fn parse_widget(argument: &str) -> Option<(String, String, String)> {
    let fields: Vec<&str> = argument.splitn(3, ':').collect();
    let [id, name, category] = fields.as_slice() else {
        return None;
    };
    Some(((*id).to_owned(), (*name).to_owned(), (*category).to_owned()))
}

/// A property from `<id>:<name>:<type>[:<default>]`.
pub(super) fn parse_property(
    argument: &str,
) -> Result<(String, crate::app::canvas::widget_catalog::PropertySchema), String> {
    use crate::app::canvas::widget_catalog::{PropertySchema, ValueType};
    let fields: Vec<&str> = argument.splitn(4, ':').collect();
    let [id, name, type_name, rest @ ..] = fields.as_slice() else {
        return Err(format!(
            "{argument:?} is not <id>:<name>:<type>[:<default>]"
        ));
    };
    let value_type = match type_name.split_once('(') {
        Some((base, rest)) => {
            let inner = rest.trim_end_matches(')');
            match base {
                "enum" => ValueType::Enum(inner.split(',').map(str::to_owned).collect()),
                "resource" => ValueType::Resource {
                    kind: inner.to_owned(),
                },
                other => return Err(format!("{other:?} is not a value type")),
            }
        }
        None => match *type_name {
            "number" => ValueType::Number,
            "text" => ValueType::Text,
            "bool" => ValueType::Bool,
            "color" => ValueType::Color,
            "dimension" => ValueType::Dimension,
            other => return Err(format!("{other:?} is not a value type")),
        },
    };
    let mut property = PropertySchema::new(*name, value_type);
    if let Some(default) = rest.first() {
        property = property.with_default(unescape(default));
    }
    Ok(((*id).to_owned(), property))
}

/// A node from a decimal argument.
pub(super) fn parse_scene_node(text: &str) -> Result<crate::contracts::identity::NodeId, String> {
    text.parse::<u64>()
        .map(crate::contracts::identity::NodeId::from_raw)
        .map_err(|_| format!("{text:?} is not a node id"))
}

/// A kind from a name.
pub(super) fn parse_kind(text: &str) -> Result<crate::app::canvas::structure::Kind, String> {
    crate::app::canvas::structure::Kind::from_name(text)
        .ok_or_else(|| format!("{text:?} is not a node kind"))
}

/// A `(parent, kind, name)` from `<parent>:<kind>:<name>`.
pub(super) fn parse_insert(
    argument: &str,
) -> Result<
    (
        crate::contracts::identity::NodeId,
        crate::app::canvas::structure::Kind,
        String,
    ),
    String,
> {
    let fields: Vec<&str> = argument.splitn(3, ':').collect();
    let [parent, kind, name] = fields.as_slice() else {
        return Err(format!("{argument:?} is not <parent>:<kind>:<name>"));
    };
    Ok((
        parse_scene_node(parent)?,
        parse_kind(kind)?,
        (*name).to_owned(),
    ))
}

/// The comma-separated numbers in an argument.
pub(super) fn parse_floats(text: &str) -> Result<Vec<f64>, String> {
    text.split(',')
        .map(|part| {
            part.trim()
                .parse::<f64>()
                .map_err(|_| format!("{part:?} is not a number"))
        })
        .collect()
}

/// A property type from `<kind>[:<args>]`.
pub(super) fn parse_property_type(
    text: &str,
) -> Result<crate::app::style::property_editor::PropertyType, String> {
    use crate::app::style::property_editor::PropertyType;

    let (kind, args) = text.split_once(':').unwrap_or((text, ""));
    match kind {
        "number" => {
            let (min, max) = parse_bounds(args)?;
            Ok(PropertyType::Number { min, max })
        }
        "integer" => {
            let (min, max) = parse_bounds(args)?;
            Ok(PropertyType::Integer {
                min: min.map(|value| value as i64),
                max: max.map(|value| value as i64),
            })
        }
        "enum" => Ok(PropertyType::Enum {
            variants: args.split(',').map(str::to_owned).collect(),
        }),
        "text" => match args.parse::<usize>() {
            Ok(max_len) => Ok(PropertyType::Text { max_len }),
            Err(_) => Err(format!("{args:?} is not a length")),
        },
        "colour" | "color" => Ok(PropertyType::Colour),
        "size" => Ok(PropertyType::Size),
        "asset" => Ok(PropertyType::Asset),
        other => Err(format!("{other:?} is not a property type")),
    }
}

/// A `(min, max)` from `<min>:<max>`, either side possibly empty.
pub(super) fn parse_bounds(args: &str) -> Result<(Option<f64>, Option<f64>), String> {
    let (min, max) = match args.split_once(':') {
        Some((min, max)) => (min, max),
        None => (args, ""),
    };
    let number = |value: &str| -> Result<Option<f64>, String> {
        if value.is_empty() {
            Ok(None)
        } else {
            value
                .parse::<f64>()
                .map(Some)
                .map_err(|_| format!("{value:?} is not a bound"))
        }
    };
    Ok((number(min)?, number(max)?))
}

pub(super) fn parse_origin(text: &str) -> Result<crate::app::data::binding::Origin, String> {
    use crate::app::data::binding::Origin;
    use crate::contracts::identity::NodeId;
    use crate::contracts::model::PropertyName;
    if let Some(name) = text.strip_prefix("f:") {
        return Ok(Origin::Field(name.to_owned()));
    }
    match text.split_once('.') {
        Some((node, property)) => {
            let node = node
                .parse::<u64>()
                .map_err(|_| format!("{node:?} is not a node id"))?;
            let property = PropertyName::new(property)
                .ok_or_else(|| format!("{property:?} is not a property name"))?;
            Ok(Origin::Slot {
                node: NodeId::from_raw(node),
                property,
            })
        }
        None => Err(format!(
            "{text:?} is neither f:<field> nor <node>.<property>"
        )),
    }
}
