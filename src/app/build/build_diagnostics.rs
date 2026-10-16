// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Build problems, and where in the design they point (task T16.03).
//!
//! The acceptance line is *"diagnostics carry a source position, are linked back to a node, and are
//! marked stale rather than shown as current"*.
//!
//! * **The input is cargo's own JSON.** [`parse`] reads `cargo --message-format=json` lines; a line
//!   that is not a compiler message is kept as raw output rather than discarded, because a build that
//!   printed something a person needs to see must not be swallowed.
//! * **A position becomes a node.** [`locate`] turns a message's file/line/column into a byte offset
//!   and asks the source map ([`crate::contracts::source_map`]) which node's generated code that
//!   offset is — the link that turns "error at main.rs:12:9" into "the `save` button".
//! * **A diagnostic is stamped.** The batch carries the model revision it was produced against, and
//!   [`Batch::is_stale`] says whether it still describes the document in front of the user.

#![allow(dead_code)]

use crate::contracts::identity::NodeId;
use crate::contracts::revision::{Revision, Staleness};
use crate::contracts::source_map::SourceMap;
use crate::project::record::json::{self, Json};

/// Where a problem is, as a compiler reports it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Span {
    /// The file, as the compiler named it.
    pub file: String,
    /// The one-based line.
    pub line: u32,
    /// The one-based column.
    pub column: u32,
}

/// One problem cargo reported.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Message {
    /// `error`, `warning`, `note`, …
    pub level: String,
    /// The message text.
    pub text: String,
    /// The error code, when there is one.
    pub code: Option<String>,
    /// The primary position, when the message has one.
    pub span: Option<Span>,
}

impl Message {
    /// The message in one line.
    pub fn render(&self) -> String {
        let place = match &self.span {
            Some(span) => format!("{}:{}:{}", span.file, span.line, span.column),
            None => "(no position)".to_owned(),
        };
        match &self.code {
            Some(code) => format!("{place}: {} [{code}]: {}", self.level, self.text),
            None => format!("{place}: {}: {}", self.level, self.text),
        }
    }
}

/// Everything one build printed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Batch {
    /// The model revision the build was started against.
    pub revision: Revision,
    /// The problems it reported.
    pub messages: Vec<Message>,
    /// Lines that were not compiler messages.
    pub raw: Vec<String>,
}

impl Batch {
    /// Whether the batch still describes the document the user is looking at.
    pub fn staleness(&self, current: Revision) -> Staleness {
        Staleness::between(self.revision, current)
    }

    /// Whether the batch is stale for `current`.
    pub fn is_stale(&self, current: Revision) -> bool {
        !self.staleness(current).is_current()
    }

    /// How many messages are errors.
    pub fn errors(&self) -> usize {
        self.messages
            .iter()
            .filter(|message| message.level == "error")
            .count()
    }

    /// The batch in one line per message.
    pub fn render(&self) -> Vec<String> {
        let mut out: Vec<String> = self.messages.iter().map(Message::render).collect();
        for line in &self.raw {
            out.push(format!("output: {line}"));
        }
        out
    }
}

/// Parses cargo's JSON output into a batch stamped with `revision`.
pub fn parse(output: &str, revision: Revision) -> Batch {
    let mut messages = Vec::new();
    let mut raw = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match parse_line(line) {
            Some(message) => messages.push(message),
            None => raw.push(line.to_owned()),
        }
    }
    Batch {
        revision,
        messages,
        raw,
    }
}

/// Parses one line, answering `None` when it is not a compiler message.
pub fn parse_line(line: &str) -> Option<Message> {
    let value = json::parse(line).ok()?;
    let object = value.as_object()?;
    if object.get("reason")?.as_str()? != "compiler-message" {
        return None;
    }
    let message = object.get("message")?.as_object()?;
    let level = message.get("level")?.as_str()?.to_owned();
    let text = message.get("message")?.as_str()?.to_owned();
    let code = message
        .get("code")
        .and_then(Json::as_object)
        .and_then(|code| code.get("code"))
        .and_then(Json::as_str)
        .map(str::to_owned);
    let span = message
        .get("spans")
        .and_then(Json::as_array)
        .and_then(|spans| {
            spans
                .iter()
                .find(|span| {
                    span.as_object()
                        .and_then(|span| span.get("is_primary"))
                        .and_then(Json::as_bool)
                        .unwrap_or(false)
                })
                .or_else(|| spans.first())
        })
        .and_then(Json::as_object)
        .and_then(|span| {
            Some(Span {
                file: span.get("file_name")?.as_str()?.to_owned(),
                line: span.get("line_start")?.as_u64()? as u32,
                column: span.get("column_start")?.as_u64()? as u32,
            })
        });
    Some(Message {
        level,
        text,
        code,
        span,
    })
}

/// The node a message points at, by turning its position into a byte offset and asking the map.
pub fn locate(message: &Message, file_text: &str, map: &SourceMap) -> Option<NodeId> {
    let span = message.span.as_ref()?;
    let offset = offset_of(file_text, span.line, span.column)?;
    map.node_at(&span.file, offset)
}

/// The byte offset of a one-based line and column counted in characters.
fn offset_of(text: &str, line: u32, column: u32) -> Option<u32> {
    if line == 0 || column == 0 {
        return None;
    }
    let mut current_line = 1u32;
    let mut current_column = 1u32;
    if current_line == line && current_column == column {
        return Some(0);
    }
    for (at, character) in text.char_indices() {
        if character == '\n' {
            current_line += 1;
            current_column = 1;
        } else {
            current_column += 1;
        }
        if current_line == line && current_column == column {
            return Some(at as u32 + character.len_utf8() as u32);
        }
        if current_line > line {
            return None;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{Span, locate, parse, parse_line};
    use crate::contracts::identity::NodeId;
    use crate::contracts::revision::{GeneratorVersion, Revision};
    use crate::contracts::source_map::{SourceMap, SourceSpan};

    const ERROR_LINE: &str = r#"{"reason":"compiler-message","message":{"message":"mismatched types","level":"error","code":{"code":"E0308"},"spans":[{"file_name":"src/main.rs","line_start":3,"line_end":3,"column_start":5,"column_end":9,"is_primary":true}]}}"#;

    #[test]
    fn a_compiler_message_parses_into_a_level_a_code_and_a_position() {
        let message = parse_line(ERROR_LINE).expect("a message");
        assert_eq!(message.level, "error");
        assert_eq!(message.text, "mismatched types");
        assert_eq!(message.code.as_deref(), Some("E0308"));
        assert_eq!(
            message.span,
            Some(Span {
                file: "src/main.rs".to_owned(),
                line: 3,
                column: 5
            }),
        );
        assert!(message.render().contains("src/main.rs:3:5"));
    }

    #[test]
    fn a_line_that_is_not_a_compiler_message_is_raw_output() {
        assert!(parse_line("Compiling demo v0.1.0").is_none());
        let batch = parse("Compiling demo v0.1.0\n\n", Revision::INITIAL);
        assert!(batch.messages.is_empty());
        assert_eq!(batch.raw.len(), 1);
    }

    #[test]
    fn a_batch_is_stamped_and_reports_its_errors() {
        let batch = parse(ERROR_LINE, Revision::from_raw(4));
        assert_eq!(batch.errors(), 1);
        assert!(!batch.is_stale(Revision::from_raw(4)));
        assert!(batch.is_stale(Revision::from_raw(5)));
        assert!(
            batch
                .staleness(Revision::from_raw(5))
                .to_string()
                .contains("computed at r4")
        );
    }

    #[test]
    fn a_message_located_inside_generated_code_names_the_node() {
        let source = "line one\nline two\npub fn x() { let a = 1; }\n";
        let mut map = SourceMap::new(GeneratorVersion::CURRENT, Revision::INITIAL);
        map.insert(
            NodeId::from_raw(2),
            SourceSpan::new("src/main.rs", 18, source.len() as u32).unwrap(),
        );
        let message = parse_line(ERROR_LINE).expect("a message");
        // Column 5 of line 3 is inside the recorded span.
        assert_eq!(locate(&message, source, &map), Some(NodeId::from_raw(2)));
    }

    #[test]
    fn a_message_outside_any_span_has_no_node() {
        let source = "line one\nline two\npub fn x() {}\n";
        let map = SourceMap::new(GeneratorVersion::CURRENT, Revision::INITIAL);
        let message = parse_line(ERROR_LINE).expect("a message");
        assert_eq!(locate(&message, source, &map), None);
    }
}
