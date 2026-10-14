// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! A JSON reader and writer that keeps what it does not understand: contract **C-PROJECT**.
//!
//! The project file is JSON, and decision `F1` asks for something a "parse into a struct and forget
//! the rest" reader cannot do: a member this build does not know is **preserved and reported, never
//! silently dropped**, so that an older build cannot damage a newer file's content. This module
//! therefore reads into [`Json`], a value that keeps every member exactly as it was found — order
//! included — and interpretation happens afterwards, on top of that value
//! ([`crate::project::format`]). Whatever interpretation leaves over is written back, which makes
//! "no member is lost" a property of the code path rather than a promise in a comment.
//!
//! Two further choices are deliberate:
//!
//! * **Numbers keep their literal text.** `1e2` and `100` are two spellings of one value, and a
//!   writer that reformats them turns a file a person wrote into a file a library wrote. Keeping
//!   the text means a round trip returns what was there; interpretation is what converts
//!   ([`Number::as_u64`]).
//! * **A duplicate member is refused.** Two values for one name is not a document with a
//!   preference, it is a document that means two things, and picking one of them quietly is exactly
//!   the kind of decision this project refuses to make.
//!
//! The escape function is shared with the probe report writer (`crate::probe::report::json_string`
//! calls [`escape_string`]), so both writers escape identically instead of each having its own idea
//! of what needs a backslash.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

/// How deep a document may nest before it is refused.
///
/// A recursive reader on input from outside is a stack overflow waiting for a file with ten
/// thousand `[`; the limit is a value rather than "whatever fits", so the refusal is a diagnostic.
pub const MAX_DEPTH: usize = 64;

/// A JSON value.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Json {
    /// `null`.
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// A number, kept as it was written.
    Number(Number),
    /// A string, with its escapes decoded.
    String(String),
    /// An array.
    Array(Vec<Json>),
    /// An object, with its members in the order they were written.
    Object(JsonObject),
}

impl Json {
    /// A string value.
    pub fn string(value: impl Into<String>) -> Self {
        Self::String(value.into())
    }

    /// A whole number, written the way a person writes one.
    pub fn number(value: u64) -> Self {
        Self::Number(Number::from_u64(value))
    }

    /// An array of values.
    pub fn array(values: impl IntoIterator<Item = Json>) -> Self {
        Self::Array(values.into_iter().collect())
    }

    /// Which kind of value this is.
    pub const fn kind(&self) -> JsonKind {
        match self {
            Self::Null => JsonKind::Null,
            Self::Bool(_) => JsonKind::Bool,
            Self::Number(_) => JsonKind::Number,
            Self::String(_) => JsonKind::String,
            Self::Array(_) => JsonKind::Array,
            Self::Object(_) => JsonKind::Object,
        }
    }

    /// The string, when this is one.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            _ => None,
        }
    }

    /// The array, when this is one.
    pub fn as_array(&self) -> Option<&[Json]> {
        match self {
            Self::Array(values) => Some(values),
            _ => None,
        }
    }

    /// The object, when this is one.
    pub fn as_object(&self) -> Option<&JsonObject> {
        match self {
            Self::Object(object) => Some(object),
            _ => None,
        }
    }

    /// The number as a whole number, when it is written as one.
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Self::Number(number) => number.as_u64(),
            _ => None,
        }
    }

    /// The number as a whole number that may be negative, when it is written as one.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Number(number) => number.as_i64(),
            _ => None,
        }
    }

    /// Writes the value the way a person reads it: one member per line, two-space indentation.
    ///
    /// The output ends with a newline, so a file this writer produced is a text file by every
    /// tool's definition.
    pub fn render(&self) -> String {
        let mut out = String::new();
        self.write_to(&mut out, 0);
        out.push('\n');
        out
    }

    /// Writes the value without any optional whitespace.
    pub fn render_compact(&self) -> String {
        let mut out = String::new();
        self.write_compact_to(&mut out);
        out
    }

    fn write_to(&self, out: &mut String, depth: usize) {
        if let Self::Object(object) = self
            && !object.is_empty()
        {
            out.push_str("{\n");
            for (index, (name, value)) in object.iter().enumerate() {
                if index > 0 {
                    out.push_str(",\n");
                }
                indent(out, depth + 1);
                out.push_str(&escape_string(name));
                out.push_str(": ");
                value.write_to(out, depth + 1);
            }
            out.push('\n');
            indent(out, depth);
            out.push('}');
            return;
        }
        if let Self::Array(values) = self
            && !values.is_empty()
        {
            out.push_str("[\n");
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    out.push_str(",\n");
                }
                indent(out, depth + 1);
                value.write_to(out, depth + 1);
            }
            out.push('\n');
            indent(out, depth);
            out.push(']');
            return;
        }
        self.write_compact_to(out);
    }

    fn write_compact_to(&self, out: &mut String) {
        match self {
            Self::Null => out.push_str("null"),
            Self::Bool(true) => out.push_str("true"),
            Self::Bool(false) => out.push_str("false"),
            Self::Number(number) => out.push_str(number.literal()),
            Self::String(value) => out.push_str(&escape_string(value)),
            Self::Array(values) => {
                out.push('[');
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    value.write_compact_to(out);
                }
                out.push(']');
            }
            Self::Object(object) => {
                out.push('{');
                for (index, (name, value)) in object.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    out.push_str(&escape_string(name));
                    out.push(':');
                    value.write_compact_to(out);
                }
                out.push('}');
            }
        }
    }
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

/// Which kind of value something is, for a diagnostic that names what was found.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum JsonKind {
    /// `null`.
    Null,
    /// `true` or `false`.
    Bool,
    /// A number.
    Number,
    /// A string.
    String,
    /// An array.
    Array,
    /// An object.
    Object,
}

impl JsonKind {
    /// The words a diagnostic uses.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Bool => "a boolean",
            Self::Number => "a number",
            Self::String => "a string",
            Self::Array => "an array",
            Self::Object => "an object",
        }
    }
}

/// A number, kept as the text it was written as.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Number(String);

impl Number {
    /// A number from its JSON text.
    ///
    /// The text is not validated here: [`parse`] is the only place that accepts text from outside
    /// and it validates. This constructor exists for values this build writes itself.
    pub fn new(literal: impl Into<String>) -> Self {
        Self(literal.into())
    }

    /// A whole number written in the plain style.
    pub fn from_u64(value: u64) -> Self {
        Self(value.to_string())
    }

    /// A whole number that may be negative, written in the plain style.
    pub fn from_i64(value: i64) -> Self {
        Self(value.to_string())
    }

    /// The text this number is written as.
    pub fn literal(&self) -> &str {
        &self.0
    }

    /// The value as a whole number, when it is written as one.
    pub fn as_u64(&self) -> Option<u64> {
        self.0.parse::<u64>().ok()
    }

    /// The value as a whole number that may be negative, when it is written as one.
    pub fn as_i64(&self) -> Option<i64> {
        self.0.parse::<i64>().ok()
    }
}

/// An object: members in the order they were written.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct JsonObject {
    members: Vec<(String, Json)>,
}

impl JsonObject {
    /// An object with no members.
    pub fn new() -> Self {
        Self::default()
    }

    /// How many members it has.
    pub fn len(&self) -> usize {
        self.members.len()
    }

    /// Whether it has no members.
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// The members, in order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Json)> {
        self.members
            .iter()
            .map(|(name, value)| (name.as_str(), value))
    }

    /// The member names, in order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.members.iter().map(|(name, _)| name.as_str())
    }

    /// One member.
    pub fn get(&self, name: &str) -> Option<&Json> {
        self.members
            .iter()
            .find(|(member, _)| member == name)
            .map(|(_, value)| value)
    }

    /// Whether a member exists.
    pub fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// Adds or replaces a member, keeping the position of one that was already there.
    ///
    /// Keeping the position is what makes a round trip of a file a person wrote come back in the
    /// order they wrote it instead of in this build's preferred order.
    pub fn insert(&mut self, name: impl Into<String>, value: Json) {
        let name = name.into();
        if let Some((_, existing)) = self.members.iter_mut().find(|(member, _)| *member == name) {
            *existing = value;
            return;
        }
        self.members.push((name, value));
    }

    /// Removes a member.
    pub fn remove(&mut self, name: &str) -> Option<Json> {
        let index = self.members.iter().position(|(member, _)| member == name)?;
        Some(self.members.remove(index).1)
    }
}

/// Writes text through a temporary file beside the target and a rename.
///
/// One definition for every writer of this format: a partial file is never the file a reader sees,
/// and a failure in the middle leaves the target as it was. The temporary file is hidden and named
/// after the target, so an interrupted write is recognisable rather than mysterious.
pub(crate) fn write_atomically(path: &Path, text: &str) -> Result<PathBuf, std::io::Error> {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let directory = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_owned());
    let temporary = directory.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        COUNTER.fetch_add(1, AtomicOrdering::Relaxed)
    ));
    std::fs::write(&temporary, text)?;
    if let Err(error) = std::fs::rename(&temporary, path) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    Ok(path.to_path_buf())
}

/// Escapes a string the way JSON requires.
///
/// Kept next to the writer because it is the writer's rule; the probe report writer calls it too,
/// so the two cannot drift.
pub fn escape_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            control if (control as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", control as u32));
            }
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// Why a document was refused.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum JsonErrorKind {
    /// The document ends in the middle of a value.
    UnexpectedEnd,
    /// Something else was there than what the grammar needs.
    Expected {
        /// What had to be there.
        what: &'static str,
        /// What was there, as a person would name it.
        found: String,
    },
    /// A number is not written the way JSON allows.
    InvalidNumber {
        /// The text as far as it was read.
        literal: String,
    },
    /// A backslash is followed by something that is not an escape.
    InvalidEscape {
        /// The character after the backslash.
        escape: char,
    },
    /// `\u` is not followed by four hexadecimal digits.
    InvalidUnicodeEscape,
    /// A `\u` escape names half of a surrogate pair, which is not a character.
    UnpairedSurrogate,
    /// A raw control character appears in a string.
    ControlCharacter {
        /// The character.
        control: char,
    },
    /// One member name appears twice.
    DuplicateMember {
        /// The name.
        name: String,
    },
    /// There is more than one value.
    TrailingData,
    /// The document nests deeper than [`MAX_DEPTH`].
    TooDeep,
}

impl JsonErrorKind {
    /// A sentence naming what is wrong, for a diagnostic.
    pub fn message(&self) -> String {
        match self {
            Self::UnexpectedEnd => "the document ends in the middle of a value".to_owned(),
            Self::Expected { what, found } => format!("expected {what}, found {found}"),
            Self::InvalidNumber { literal } => format!("`{literal}` is not a number JSON allows"),
            Self::InvalidEscape { escape } => format!("`\\{escape}` is not an escape JSON allows"),
            Self::InvalidUnicodeEscape => {
                "`\\u` must be followed by four hexadecimal digits".to_owned()
            }
            Self::UnpairedSurrogate => {
                "a `\\u` escape names half of a surrogate pair, which is not a character".to_owned()
            }
            Self::ControlCharacter { control } => format!(
                "a raw control character (U+{:04X}) in a string must be escaped",
                *control as u32
            ),
            Self::DuplicateMember { name } => format!("the member `{name}` appears twice"),
            Self::TrailingData => "the document holds more than one value".to_owned(),
            Self::TooDeep => format!("the document nests deeper than {MAX_DEPTH} levels"),
        }
    }
}

/// Where a document was refused, and why.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct JsonError {
    kind: JsonErrorKind,
    line: usize,
    column: usize,
}

impl JsonError {
    /// Why it was refused.
    pub fn kind(&self) -> &JsonErrorKind {
        &self.kind
    }

    /// The line, counting from one.
    pub const fn line(&self) -> usize {
        self.line
    }

    /// The column, counting from one in characters.
    pub const fn column(&self) -> usize {
        self.column
    }
}

impl fmt::Display for JsonError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "line {}, column {}: {}",
            self.line,
            self.column,
            self.kind.message()
        )
    }
}

impl std::error::Error for JsonError {}

/// Reads one JSON value.
///
/// Trailing whitespace is allowed; anything else after the value is
/// [`JsonErrorKind::TrailingData`], because a file with two documents in it is not a file this
/// build can be sure it understood.
pub fn parse(text: &str) -> Result<Json, JsonError> {
    let mut parser = Parser {
        characters: text.chars().collect(),
        at: 0,
    };
    parser.skip_whitespace();
    let value = parser.value(0)?;
    parser.skip_whitespace();
    if parser.at < parser.characters.len() {
        return Err(parser.error(JsonErrorKind::TrailingData));
    }
    Ok(value)
}

struct Parser {
    characters: Vec<char>,
    at: usize,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.characters.get(self.at).copied()
    }

    fn next(&mut self) -> Option<char> {
        let character = self.peek()?;
        self.at += 1;
        Some(character)
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t' | '\n' | '\r')) {
            self.at += 1;
        }
    }

    fn error(&self, kind: JsonErrorKind) -> JsonError {
        let mut line = 1;
        let mut column = 1;
        for character in self.characters.iter().take(self.at) {
            if *character == '\n' {
                line += 1;
                column = 1;
            } else {
                column += 1;
            }
        }
        JsonError { kind, line, column }
    }

    fn found(&self) -> String {
        match self.peek() {
            None => "the end of the document".to_owned(),
            Some(character) if (character as u32) < 0x20 => {
                format!("a control character (U+{:04X})", character as u32)
            }
            Some(character) => format!("`{character}`"),
        }
    }

    fn expect(&mut self, expected: char, what: &'static str) -> Result<(), JsonError> {
        if self.peek() == Some(expected) {
            self.at += 1;
            return Ok(());
        }
        Err(self.error(JsonErrorKind::Expected {
            what,
            found: self.found(),
        }))
    }

    fn value(&mut self, depth: usize) -> Result<Json, JsonError> {
        if depth > MAX_DEPTH {
            return Err(self.error(JsonErrorKind::TooDeep));
        }
        match self.peek() {
            None => Err(self.error(JsonErrorKind::UnexpectedEnd)),
            Some('{') => self.object(depth),
            Some('[') => self.array(depth),
            Some('"') => Ok(Json::String(self.string()?)),
            Some('t') => self.literal("true", Json::Bool(true)),
            Some('f') => self.literal("false", Json::Bool(false)),
            Some('n') => self.literal("null", Json::Null),
            Some('-' | '0'..='9') => Ok(Json::Number(self.number()?)),
            Some(_) => Err(self.error(JsonErrorKind::Expected {
                what: "a value",
                found: self.found(),
            })),
        }
    }

    fn literal(&mut self, text: &'static str, value: Json) -> Result<Json, JsonError> {
        for expected in text.chars() {
            if self.next() != Some(expected) {
                return Err(self.error(JsonErrorKind::Expected {
                    what: text,
                    found: self.found(),
                }));
            }
        }
        Ok(value)
    }

    fn object(&mut self, depth: usize) -> Result<Json, JsonError> {
        self.expect('{', "an object")?;
        let mut object = JsonObject::new();
        self.skip_whitespace();
        if self.peek() == Some('}') {
            self.at += 1;
            return Ok(Json::Object(object));
        }
        loop {
            self.skip_whitespace();
            let name = self.string()?;
            if object.contains(&name) {
                return Err(self.error(JsonErrorKind::DuplicateMember { name }));
            }
            self.skip_whitespace();
            self.expect(':', "`:` after a member name")?;
            self.skip_whitespace();
            let value = self.value(depth + 1)?;
            object.insert(name, value);
            self.skip_whitespace();
            match self.peek() {
                Some(',') => self.at += 1,
                Some('}') => {
                    self.at += 1;
                    return Ok(Json::Object(object));
                }
                _ => {
                    return Err(self.error(JsonErrorKind::Expected {
                        what: "`,` or `}`",
                        found: self.found(),
                    }));
                }
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Json, JsonError> {
        self.expect('[', "an array")?;
        let mut values = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(']') {
            self.at += 1;
            return Ok(Json::Array(values));
        }
        loop {
            self.skip_whitespace();
            values.push(self.value(depth + 1)?);
            self.skip_whitespace();
            match self.peek() {
                Some(',') => self.at += 1,
                Some(']') => {
                    self.at += 1;
                    return Ok(Json::Array(values));
                }
                _ => {
                    return Err(self.error(JsonErrorKind::Expected {
                        what: "`,` or `]`",
                        found: self.found(),
                    }));
                }
            }
        }
    }

    fn string(&mut self) -> Result<String, JsonError> {
        self.expect('"', "`\"` to start a string")?;
        let mut out = String::new();
        loop {
            let Some(character) = self.next() else {
                return Err(self.error(JsonErrorKind::UnexpectedEnd));
            };
            match character {
                '"' => return Ok(out),
                '\\' => out.push(self.escape()?),
                control if (control as u32) < 0x20 => {
                    return Err(self.error(JsonErrorKind::ControlCharacter { control }));
                }
                other => out.push(other),
            }
        }
    }

    fn escape(&mut self) -> Result<char, JsonError> {
        let Some(character) = self.next() else {
            return Err(self.error(JsonErrorKind::UnexpectedEnd));
        };
        match character {
            '"' => Ok('"'),
            '\\' => Ok('\\'),
            '/' => Ok('/'),
            'b' => Ok('\u{08}'),
            'f' => Ok('\u{0c}'),
            'n' => Ok('\n'),
            'r' => Ok('\r'),
            't' => Ok('\t'),
            'u' => self.code_unit(),
            other => Err(self.error(JsonErrorKind::InvalidEscape { escape: other })),
        }
    }

    fn hex4(&mut self) -> Result<u32, JsonError> {
        let mut value = 0u32;
        for _ in 0..4 {
            let Some(character) = self.next() else {
                return Err(self.error(JsonErrorKind::UnexpectedEnd));
            };
            let Some(digit) = character.to_digit(16) else {
                return Err(self.error(JsonErrorKind::InvalidUnicodeEscape));
            };
            value = value * 16 + digit;
        }
        Ok(value)
    }

    fn code_unit(&mut self) -> Result<char, JsonError> {
        let first = self.hex4()?;
        // A character outside the basic plane is written as a surrogate pair; half of a pair is not
        // a character, so it is refused rather than turned into a replacement character nobody
        // wrote.
        if (0xd800..0xdc00).contains(&first) {
            if self.next() != Some('\\') || self.next() != Some('u') {
                return Err(self.error(JsonErrorKind::UnpairedSurrogate));
            }
            let second = self.hex4()?;
            if !(0xdc00..0xe000).contains(&second) {
                return Err(self.error(JsonErrorKind::UnpairedSurrogate));
            }
            let combined = 0x10000 + ((first - 0xd800) << 10) + (second - 0xdc00);
            return char::from_u32(combined)
                .ok_or_else(|| self.error(JsonErrorKind::UnpairedSurrogate));
        }
        if (0xdc00..0xe000).contains(&first) {
            return Err(self.error(JsonErrorKind::UnpairedSurrogate));
        }
        char::from_u32(first).ok_or_else(|| self.error(JsonErrorKind::InvalidUnicodeEscape))
    }

    fn number(&mut self) -> Result<Number, JsonError> {
        let start = self.at;
        if self.peek() == Some('-') {
            self.at += 1;
        }
        let integer_start = self.at;
        while matches!(self.peek(), Some('0'..='9')) {
            self.at += 1;
        }
        let integer = &self.characters[integer_start..self.at];
        let integer_is_valid = match integer {
            [] => false,
            // A leading zero is allowed only when it is the whole integer part.
            [only] => *only != '0' || !matches!(self.peek(), Some('0'..='9')),
            [first, ..] => *first != '0',
        };
        if !integer_is_valid {
            return Err(self.invalid_number(start));
        }
        if self.peek() == Some('.') {
            self.at += 1;
            let fraction_start = self.at;
            while matches!(self.peek(), Some('0'..='9')) {
                self.at += 1;
            }
            if fraction_start == self.at {
                return Err(self.invalid_number(start));
            }
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            self.at += 1;
            if matches!(self.peek(), Some('+' | '-')) {
                self.at += 1;
            }
            let exponent_start = self.at;
            while matches!(self.peek(), Some('0'..='9')) {
                self.at += 1;
            }
            if exponent_start == self.at {
                return Err(self.invalid_number(start));
            }
        }
        let literal: String = self.characters[start..self.at].iter().collect();
        Ok(Number(literal))
    }

    fn invalid_number(&self, start: usize) -> JsonError {
        let mut end = self.at;
        while end < self.characters.len()
            && matches!(
                self.characters[end],
                '0'..='9' | '.' | '-' | '+' | 'e' | 'E'
            )
        {
            end += 1;
        }
        self.error(JsonErrorKind::InvalidNumber {
            literal: self.characters[start..end].iter().collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Json, JsonErrorKind, JsonObject, MAX_DEPTH, Number, escape_string, parse};

    fn kind_of(text: &str) -> JsonErrorKind {
        parse(text)
            .expect_err("this text is not JSON")
            .kind()
            .clone()
    }

    #[test]
    fn a_document_keeps_the_order_and_the_spelling_it_was_written_with() {
        let text = "{\"b\": 1e2, \"a\": \"x\", \"c\": [true, null, 0.50]}";
        let value = parse(text).expect("valid JSON");
        assert_eq!(
            value.render_compact(),
            "{\"b\":1e2,\"a\":\"x\",\"c\":[true,null,0.50]}"
        );
        let object = value.as_object().expect("an object");
        assert_eq!(object.names().collect::<Vec<_>>(), ["b", "a", "c"]);
        let number = match object.get("b") {
            Some(Json::Number(number)) => number.clone(),
            other => panic!("b should be a number, found {other:?}"),
        };
        assert_eq!(number.literal(), "1e2");
        assert_eq!(number.as_u64(), None, "an exponent is not a whole number");
        assert_eq!(
            object.get("c").and_then(Json::as_array).map(<[Json]>::len),
            Some(3)
        );
    }

    #[test]
    fn a_number_is_read_the_way_json_writes_one() {
        assert_eq!(parse("123").expect("valid").as_u64(), Some(123));
        assert_eq!(parse("-1").expect("valid").as_u64(), None);
        assert_eq!(parse("0").expect("valid").as_u64(), Some(0));
        assert_eq!(parse("1.5").expect("valid").as_u64(), None);
        assert_eq!(
            parse("[1, 20, 0]").expect("valid").render_compact(),
            "[1,20,0]"
        );
    }

    #[test]
    fn a_number_that_is_not_a_number_is_refused_by_name() {
        for text in ["01", "1.", "1e", "1e+", "-"] {
            let kind = kind_of(text);
            assert!(
                matches!(kind, JsonErrorKind::InvalidNumber { .. }),
                "{text} was refused as {kind:?}"
            );
        }
        match kind_of("01") {
            JsonErrorKind::InvalidNumber { literal } => assert_eq!(literal, "01"),
            other => panic!("expected an invalid number, found {other:?}"),
        }
        // A value that does not start like a number is refused as a value, which names the problem
        // better than "not a number" would.
        for text in [".5", "+1"] {
            assert!(matches!(kind_of(text), JsonErrorKind::Expected { .. }));
        }
    }

    #[test]
    fn strings_are_decoded_and_written_back_identically() {
        let text = r#"{"s": "quote \" backslash \\ slash \/ newline \n tab \t code \u00e9 emoji \ud83d\ude00"}"#;
        let value = parse(text).expect("valid JSON");
        let decoded = value
            .as_object()
            .and_then(|object| object.get("s"))
            .and_then(Json::as_str)
            .expect("a string");
        assert!(decoded.contains('\u{e9}'));
        assert!(decoded.contains('\u{1f600}'));
        assert!(decoded.contains('\n'));
        assert_eq!(parse(&value.render()).expect("valid"), value);
    }

    #[test]
    fn a_string_with_an_escape_this_grammar_does_not_have_is_refused() {
        match kind_of(r#""\x41""#) {
            JsonErrorKind::InvalidEscape { escape } => assert_eq!(escape, 'x'),
            other => panic!("expected an invalid escape, found {other:?}"),
        }
        assert_eq!(kind_of(r#""\u12""#), JsonErrorKind::InvalidUnicodeEscape);
        assert_eq!(kind_of(r#""\u12zz""#), JsonErrorKind::InvalidUnicodeEscape);
        assert_eq!(kind_of(r#""\ud83d""#), JsonErrorKind::UnpairedSurrogate);
        assert_eq!(kind_of(r#""\udc00""#), JsonErrorKind::UnpairedSurrogate);
        assert_eq!(
            kind_of(r#""\ud83d\u0041""#),
            JsonErrorKind::UnpairedSurrogate
        );
        assert_eq!(
            kind_of("\"raw\nnewline\""),
            JsonErrorKind::ControlCharacter { control: '\n' }
        );
    }

    #[test]
    fn an_object_with_two_values_for_one_name_is_refused() {
        assert_eq!(
            kind_of(r#"{"a": 1, "a": 2}"#),
            JsonErrorKind::DuplicateMember {
                name: "a".to_owned()
            }
        );
    }

    #[test]
    fn nesting_deeper_than_the_limit_is_refused_instead_of_overflowing_the_stack() {
        let text = "[".repeat(MAX_DEPTH + 2) + &"]".repeat(MAX_DEPTH + 2);
        assert_eq!(kind_of(&text), JsonErrorKind::TooDeep);
        let allowed = "[".repeat(MAX_DEPTH) + &"]".repeat(MAX_DEPTH);
        assert!(parse(&allowed).is_ok());
    }

    #[test]
    fn a_document_that_stops_in_the_middle_says_so() {
        for text in ["", "{", "[1,", "{\"a\":", "\"unterminated", "tru"] {
            let error = parse(text).expect_err("this text is not JSON");
            assert!(
                matches!(
                    error.kind(),
                    JsonErrorKind::UnexpectedEnd | JsonErrorKind::Expected { .. }
                ),
                "{text} was refused as {:?}",
                error.kind()
            );
        }
    }

    #[test]
    fn two_values_in_one_document_are_refused() {
        assert_eq!(kind_of("{} {}"), JsonErrorKind::TrailingData);
        assert_eq!(kind_of("1 2"), JsonErrorKind::TrailingData);
        assert!(parse("  {}\n").is_ok());
    }

    #[test]
    fn the_position_of_a_refusal_counts_lines_and_characters() {
        let error = parse("{\n  \"a\": 1,\n  \"b\" 2\n}").expect_err("not JSON");
        assert_eq!(error.line(), 3);
        assert!(error.column() > 1);
        assert!(error.to_string().starts_with("line 3, column"));
        let unicode = parse("\"\u{4e2d}\u{6587}\" \u{4e2d}").expect_err("not JSON");
        assert_eq!(unicode.kind(), &JsonErrorKind::TrailingData);
        assert_eq!(unicode.column(), 6);
    }

    #[test]
    fn render_is_idempotent_and_shows_structure() {
        let text = "{\"a\":{\"b\":[]},\"c\":[{},[1]],\"d\":{}}";
        let value = parse(text).expect("valid JSON");
        let pretty = value.render();
        assert_eq!(
            pretty,
            "{\n  \"a\": {\n    \"b\": []\n  },\n  \"c\": [\n    {},\n    [\n      1\n    ]\n  ],\n  \"d\": {}\n}\n"
        );
        assert_eq!(parse(&pretty).expect("valid JSON"), value);
        assert_eq!(parse(&pretty).expect("valid").render(), pretty);
    }

    #[test]
    fn an_object_inserts_in_place_and_removes_by_name() {
        let mut object = JsonObject::new();
        object.insert("a", Json::number(1));
        object.insert("b", Json::number(2));
        object.insert("a", Json::number(3));
        assert_eq!(object.names().collect::<Vec<_>>(), ["a", "b"]);
        assert!(object.contains("a"));
        assert_eq!(object.get("a").and_then(Json::as_u64), Some(3));
        assert_eq!(object.remove("a"), Some(Json::number(3)));
        assert_eq!(object.len(), 1);
        assert!(object.remove("a").is_none());
        assert!(!object.is_empty());
        assert!(JsonObject::new().is_empty());
    }

    #[test]
    fn escaping_covers_what_json_requires_and_leaves_the_rest_alone() {
        assert_eq!(escape_string("plain"), "\"plain\"");
        assert_eq!(escape_string("a\"b"), "\"a\\\"b\"");
        assert_eq!(escape_string("a\\b"), "\"a\\\\b\"");
        assert_eq!(escape_string("a\nb"), "\"a\\nb\"");
        assert_eq!(escape_string("\u{1}"), "\"\\u0001\"");
        assert_eq!(escape_string("\u{4e2d}"), "\"\u{4e2d}\"");
        assert_eq!(escape_string("\u{1f600}"), "\"\u{1f600}\"");
        for code in 0u32..0x2000 {
            if let Some(character) = char::from_u32(code) {
                let escaped = escape_string(&character.to_string());
                let reparsed = parse(&escaped).expect("escaped text is JSON");
                assert_eq!(
                    reparsed.as_str(),
                    Some(character.to_string().as_str()),
                    "U+{code:04X}"
                );
            }
        }
    }

    #[test]
    fn kinds_are_named_for_a_diagnostic() {
        assert_eq!(Json::Null.kind().name(), "null");
        assert_eq!(Json::Bool(true).kind().name(), "a boolean");
        assert_eq!(Json::number(1).kind().name(), "a number");
        assert_eq!(Json::string("a").kind().name(), "a string");
        assert_eq!(Json::Array(vec![]).kind().name(), "an array");
        assert_eq!(Json::Object(JsonObject::new()).kind().name(), "an object");
        assert_eq!(Number::from_u64(7).literal(), "7");
        assert_eq!(Number::new("7").as_u64(), Some(7));
        assert_eq!(Json::array([]).render_compact(), "[]");
    }
}
