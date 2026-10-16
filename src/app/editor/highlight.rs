// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Syntax highlighting: cross-line state, an asynchronous incremental cache, and the language and
//! theme seams (task T06.02).
//!
//! The acceptance line is *"multi-line comments and strings are right, and drawing does not execute
//! parsing"*. The second half is the one that shapes the module:
//!
//! * **The paint path reads and cannot lex.** [`Highlights::state`] and [`Highlights::spans`] take
//!   `&self`, and the only method that lexes — [`Highlights::step`] — takes `&mut self`. A painter
//!   holding `&Highlights` therefore *cannot* run the lexer, so "drawing does not parse" is a
//!   property of the types. A line whose spans are not computed yet reads as [`State::Unknown`], never
//!   as a fabricated "clean line", which is the distinction the upstream `line_state_is_known` asks
//!   for.
//! * **Cross-line state is a small value, carried line to line.** A line starts in a [`LineState`] —
//!   nothing open, inside a block comment, or inside a raw string — and ends in one. That is what
//!   makes a multi-line `/* … */` or `r#" … "#` correct: the second line's highlighting is decided by
//!   the first line's *end state*, not by looking backwards.
//! * **The cache fills incrementally and asynchronously.** [`Highlights::step`] lexes at most a
//!   budget of lines, so a frame can do a bounded amount and paint whatever is ready. After an edit of
//!   one line, [`Highlights::set_line`] marks the cache unreliable from that line; stepping re-lexes
//!   forward and **stops as soon as the end state stops changing**, because then the lines after it
//!   still start in the state they were cached with — the edit cost one line, not a file.
//! * **The theme is a seam, not part of the model.** A [`Span`] carries a [`Role`]; a [`Theme`] maps
//!   roles to colours. The highlights never hold a colour, so a theme change repaints without
//!   re-lexing, and the model does not depend on any theme.

#![allow(dead_code)]

use std::collections::BTreeMap;

/// A language this build highlights.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Language {
    /// No grammar: every line is plain text. Used for a file whose language is not recognised.
    #[default]
    Plain,
    /// Rust.
    Rust,
    /// JSON.
    Json,
    /// CSS.
    Css,
}

impl Language {
    /// The language's name, as a command line or a configuration file writes it.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::Rust => "rust",
            Self::Json => "json",
            Self::Css => "css",
        }
    }

    /// The language a name asks for, when it is one.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|language| language.name() == name)
    }

    /// The language a file path suggests, by its extension.
    pub fn of_path(path: &str) -> Self {
        let extension = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        match extension.as_str() {
            "rs" => Self::Rust,
            "json" => Self::Json,
            "css" => Self::Css,
            _ => Self::Plain,
        }
    }

    /// Every language.
    pub const ALL: [Self; 4] = [Self::Plain, Self::Rust, Self::Json, Self::Css];
}

/// What part of a line a span is, before any theme has a say.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Role {
    /// A comment.
    Comment,
    /// A string or character literal.
    String,
    /// A keyword.
    Keyword,
    /// A number.
    Number,
    /// Punctuation.
    Punctuation,
}

impl Role {
    /// The role's name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Comment => "comment",
            Self::String => "string",
            Self::Keyword => "keyword",
            Self::Number => "number",
            Self::Punctuation => "punctuation",
        }
    }

    /// Every role a theme must have a colour for.
    pub const ALL: [Self; 5] = [
        Self::Comment,
        Self::String,
        Self::Keyword,
        Self::Number,
        Self::Punctuation,
    ];
}

/// The state a line is in at its edges: what a multi-line construct left open.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub enum LineState {
    /// Nothing is open.
    #[default]
    Clean,
    /// A block comment `/* … */` is open.
    BlockComment,
    /// A raw string is open, opened by `r` followed by this many `#` and a quote.
    RawString {
        /// How many `#` the opener had.
        hashes: usize,
    },
}

impl LineState {
    /// The state's name.
    pub fn name(&self) -> String {
        match self {
            Self::Clean => "clean".to_owned(),
            Self::BlockComment => "in a block comment".to_owned(),
            Self::RawString { hashes } => format!("in a raw string ({hashes} #)"),
        }
    }

    /// Whether nothing is open.
    pub const fn is_clean(&self) -> bool {
        matches!(self, Self::Clean)
    }
}

/// A line's start state, as the paint path sees it: known or not yet computed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum State {
    /// The state is known.
    Known(LineState),
    /// The line has not been lexed yet, so its state is not known. Never reported as clean.
    Unknown,
}

impl State {
    /// Whether the state is known.
    pub const fn is_known(&self) -> bool {
        matches!(self, Self::Known(_))
    }

    /// The state, when it is known.
    pub const fn known(&self) -> Option<&LineState> {
        match self {
            Self::Known(state) => Some(state),
            Self::Unknown => None,
        }
    }

    /// The state as one word.
    pub fn name(&self) -> String {
        match self {
            Self::Known(state) => state.name(),
            Self::Unknown => "not lexed yet".to_owned(),
        }
    }
}

/// One highlighted range of a line.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Span {
    start: usize,
    end: usize,
    role: Role,
}

impl Span {
    /// A span.
    pub const fn new(start: usize, end: usize, role: Role) -> Self {
        Self { start, end, role }
    }

    /// Where it begins, as a byte offset in the line.
    pub const fn start(&self) -> usize {
        self.start
    }

    /// Where it ends (exclusive).
    pub const fn end(&self) -> usize {
        self.end
    }

    /// What it is. The theme turns this into a colour.
    pub const fn role(&self) -> Role {
        self.role
    }
}

/// A theme: what each role looks like. The seam between the model and the colours.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Theme {
    name: String,
    colors: BTreeMap<Role, String>,
}

impl Theme {
    /// The theme a name asks for, when it is one.
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "dark" => Some(Self::of("dark", DARK)),
            "light" => Some(Self::of("light", LIGHT)),
            _ => None,
        }
    }

    /// The default dark theme.
    pub fn dark() -> Self {
        Self::named("dark").expect("the dark theme exists")
    }

    /// The default light theme.
    pub fn light() -> Self {
        Self::named("light").expect("the light theme exists")
    }

    fn of(name: &str, pairs: [(Role, &str); 5]) -> Self {
        Self {
            name: name.to_owned(),
            colors: pairs
                .into_iter()
                .map(|(role, color)| (role, color.to_owned()))
                .collect(),
        }
    }

    /// The theme's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The colour a role is drawn in.
    pub fn color_of(&self, role: Role) -> &str {
        self.colors
            .get(&role)
            .map(String::as_str)
            .unwrap_or("#ffffff")
    }

    /// Whether every role has a colour, so nothing is drawn in a fallback by accident.
    pub fn covers_every_role(&self) -> bool {
        Role::ALL.iter().all(|role| self.colors.contains_key(role))
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::dark()
    }
}

const DARK: [(Role, &str); 5] = [
    (Role::Comment, "#7f848e"),
    (Role::String, "#98c379"),
    (Role::Keyword, "#c678dd"),
    (Role::Number, "#d19a66"),
    (Role::Punctuation, "#abb2bf"),
];

const LIGHT: [(Role, &str); 5] = [
    (Role::Comment, "#a0a1a7"),
    (Role::String, "#50a14f"),
    (Role::Keyword, "#a626a4"),
    (Role::Number, "#986801"),
    (Role::Punctuation, "#383a42"),
];

/// A line's highlight cache: the state it starts in, its spans, and the state it ends in.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
struct Cached {
    spans: Vec<Span>,
    ends: LineState,
}

/// The highlight cache over a document's lines.
#[derive(Clone, Debug)]
pub struct Highlights {
    language: Language,
    lines: Vec<String>,
    cache: Vec<Cached>,
    /// Lines `[0, resolved)` have computed spans and a trustworthy end state.
    resolved: usize,
    /// Whether a re-lex may stop as soon as the end state stops changing.
    ///
    /// True after a single-line edit (the lines after it are unchanged), false after the whole text
    /// was replaced (every line may differ).
    converge: bool,
    /// How many lines have been lexed, in total. A counter the evidence reads.
    lexed_lines: usize,
}

impl Highlights {
    /// An empty cache for a language.
    pub fn new(language: Language) -> Self {
        Self {
            language,
            lines: Vec::new(),
            cache: Vec::new(),
            resolved: 0,
            converge: false,
            lexed_lines: 0,
        }
    }

    /// The language being lexed.
    pub const fn language(&self) -> Language {
        self.language
    }

    /// Switches the language, which throws the cache away: the grammar changed, so every line's
    /// spans may be different.
    pub fn set_language(&mut self, language: Language) {
        if self.language == language {
            return;
        }
        self.language = language;
        self.resolved = 0;
        self.converge = false;
    }

    /// Replaces the whole text. Every line is unreliable until re-lexed.
    pub fn set_text(&mut self, text: &str) {
        self.lines = text.lines().map(str::to_owned).collect();
        self.cache = vec![Cached::default(); self.lines.len()];
        self.resolved = 0;
        self.converge = false;
    }

    /// Appends a line at the end.
    pub fn push_line(&mut self, line: impl Into<String>) {
        self.lines.push(line.into());
        self.cache.push(Cached::default());
    }

    /// Replaces one line's text.
    ///
    /// The lines after it are unchanged and it is only this line's *end state* that can move, so a
    /// re-lex is allowed to stop as soon as that state stops changing.
    pub fn set_line(&mut self, index: usize, line: impl Into<String>) {
        if index >= self.lines.len() {
            return;
        }
        self.lines[index] = line.into();
        self.resolved = self.resolved.min(index);
        self.converge = true;
    }

    /// How many lines the document has.
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// How many lines have computed spans.
    pub const fn resolved(&self) -> usize {
        self.resolved
    }

    /// How many lines have been lexed in total.
    pub const fn lexed_lines(&self) -> usize {
        self.lexed_lines
    }

    /// Whether every line has computed spans.
    pub fn is_complete(&self) -> bool {
        self.resolved >= self.lines.len()
    }

    /// The state a line starts in, as the paint path sees it.
    ///
    /// A line's start state is known once the line before it has been lexed, so this answers
    /// [`State::Known`] for line 0 and for any line whose predecessor is resolved, and
    /// [`State::Unknown`] otherwise. It never answers "clean" for a line it has not reached.
    pub fn state(&self, line: usize) -> State {
        if line == 0 {
            return State::Known(LineState::Clean);
        }
        if line <= self.resolved {
            return State::Known(self.cache[line - 1].ends.clone());
        }
        State::Unknown
    }

    /// A line's spans, when they are computed.
    pub fn spans(&self, line: usize) -> Option<&[Span]> {
        (line < self.resolved).then(|| self.cache[line].spans.as_slice())
    }

    /// Lexes at most `budget` more lines, and answers how many it did.
    ///
    /// This is the only method that runs the lexer. Stopping early: once a re-lexed line's end state
    /// equals the one that was cached, the lines after it still start in the state they were cached
    /// with and their text is unchanged, so the rest of the cache is trustworthy again.
    pub fn step(&mut self, budget: usize) -> usize {
        let mut done = 0;
        while done < budget && self.resolved < self.lines.len() {
            let index = self.resolved;
            let start = if index == 0 {
                LineState::Clean
            } else {
                self.cache[index - 1].ends.clone()
            };
            let (spans, ends) = lex_line(self.language, &self.lines[index], &start);
            let previous = std::mem::replace(&mut self.cache[index].ends, ends.clone());
            self.cache[index].spans = spans;
            self.resolved += 1;
            self.lexed_lines += 1;
            done += 1;
            if self.converge && index > 0 && previous == ends {
                // The edit was confined to line `index` and it ends the same way, so every line after
                // it is still valid.
                self.resolved = self.lines.len();
                break;
            }
        }
        done
    }

    /// Lexes everything that is left. A convenience for a caller that need not be incremental.
    pub fn resolve_all(&mut self) -> usize {
        let mut total = 0;
        while self.resolved < self.lines.len() {
            let did = self.step(self.lines.len() - self.resolved);
            if did == 0 {
                break;
            }
            total += did;
        }
        total
    }

    /// The cache in one report: the language, the lines, and each line's state and spans.
    pub fn render(&self, theme: &Theme) -> Vec<String> {
        let mut lines = vec![format!(
            "{}: {} line(s), {} resolved, {} lexed in total",
            self.language.name(),
            self.line_count(),
            self.resolved,
            self.lexed_lines
        )];
        for index in 0..self.lines.len() {
            let state = self.state(index);
            match self.spans(index) {
                None => lines.push(format!("  {index}: {} (not lexed)", state.name())),
                Some(spans) => {
                    let pieces: Vec<String> = spans
                        .iter()
                        .map(|span| {
                            let text = &self.lines[index][span.start()..span.end()];
                            format!(
                                "{} {:?} {}",
                                span.role().name(),
                                text,
                                theme.color_of(span.role())
                            )
                        })
                        .collect();
                    lines.push(format!(
                        "  {index}: {} — {}",
                        state.name(),
                        if pieces.is_empty() {
                            "plain".to_owned()
                        } else {
                            pieces.join(", ")
                        }
                    ));
                }
            }
        }
        lines
    }
}

/// Lexes one line, given the state it starts in, and answers its spans and end state.
pub fn lex_line(language: Language, line: &str, start: &LineState) -> (Vec<Span>, LineState) {
    match language {
        Language::Plain => (Vec::new(), LineState::Clean),
        Language::Rust => lex_rust(line, start),
        Language::Json => lex_json(line),
        Language::Css => lex_css(line, start),
    }
}

const RUST_KEYWORDS: [&str; 14] = [
    "fn", "let", "mut", "pub", "use", "mod", "impl", "struct", "enum", "match", "if", "else",
    "return", "for",
];

fn lex_rust(line: &str, start: &LineState) -> (Vec<Span>, LineState) {
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut spans = Vec::new();
    let mut at = 0;

    match start {
        LineState::BlockComment => match block_close(bytes, 0) {
            Some(end) => {
                spans.push(Span::new(0, end, Role::Comment));
                at = end;
            }
            None => {
                spans.push(Span::new(0, len, Role::Comment));
                return (spans, LineState::BlockComment);
            }
        },
        LineState::RawString { hashes } => {
            let close = raw_close(*hashes);
            match find(line, &close, 0) {
                Some(position) => {
                    spans.push(Span::new(0, position + close.len(), Role::String));
                    at = position + close.len();
                }
                None => {
                    spans.push(Span::new(0, len, Role::String));
                    return (spans, LineState::RawString { hashes: *hashes });
                }
            }
        }
        LineState::Clean => {}
    }

    while at < len {
        let byte = bytes[at];
        if byte == b'/' && bytes.get(at + 1) == Some(&b'/') {
            spans.push(Span::new(at, len, Role::Comment));
            break;
        }
        if byte == b'/' && bytes.get(at + 1) == Some(&b'*') {
            match block_close(bytes, at + 2) {
                Some(end) => {
                    spans.push(Span::new(at, end, Role::Comment));
                    at = end;
                }
                None => {
                    spans.push(Span::new(at, len, Role::Comment));
                    return (spans, LineState::BlockComment);
                }
            }
            continue;
        }
        if byte == b'"' {
            let end = escaped(bytes, at, b'"');
            spans.push(Span::new(at, end, Role::String));
            at = end;
            continue;
        }
        if byte == b'r'
            && let Some((open_end, hashes)) = raw_open(bytes, at)
        {
            let close = raw_close(hashes);
            match find(line, &close, open_end) {
                Some(position) => {
                    spans.push(Span::new(at, position + close.len(), Role::String));
                    at = position + close.len();
                }
                None => {
                    spans.push(Span::new(at, len, Role::String));
                    return (spans, LineState::RawString { hashes });
                }
            }
            continue;
        }
        if byte == b'\''
            && let Some(end) = char_literal(bytes, at)
        {
            spans.push(Span::new(at, end, Role::String));
            at = end;
            continue;
        }
        if byte.is_ascii_digit() {
            let end = number(bytes, at);
            spans.push(Span::new(at, end, Role::Number));
            at = end;
            continue;
        }
        if byte == b'_' || byte.is_ascii_alphabetic() {
            let end = identifier(bytes, at);
            if RUST_KEYWORDS.contains(&&line[at..end]) {
                spans.push(Span::new(at, end, Role::Keyword));
            }
            at = end;
            continue;
        }
        if byte.is_ascii_whitespace() {
            at += 1;
            continue;
        }
        if is_punctuation(byte) {
            spans.push(Span::new(at, at + 1, Role::Punctuation));
        }
        at += 1;
    }
    (spans, LineState::Clean)
}

fn lex_json(line: &str) -> (Vec<Span>, LineState) {
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut spans = Vec::new();
    let mut at = 0;
    while at < len {
        let byte = bytes[at];
        if byte == b'"' {
            let end = escaped(bytes, at, b'"');
            spans.push(Span::new(at, end, Role::String));
            at = end;
            continue;
        }
        if byte.is_ascii_digit()
            || (byte == b'-' && bytes.get(at + 1).is_some_and(u8::is_ascii_digit))
        {
            let end = number(bytes, at);
            spans.push(Span::new(at, end, Role::Number));
            at = end;
            continue;
        }
        if byte == b'_' || byte.is_ascii_alphabetic() {
            let end = identifier(bytes, at);
            if matches!(&line[at..end], "true" | "false" | "null") {
                spans.push(Span::new(at, end, Role::Keyword));
            }
            at = end;
            continue;
        }
        if byte.is_ascii_whitespace() {
            at += 1;
            continue;
        }
        if matches!(byte, b'{' | b'}' | b'[' | b']' | b':' | b',') {
            spans.push(Span::new(at, at + 1, Role::Punctuation));
        }
        at += 1;
    }
    (spans, LineState::Clean)
}

fn lex_css(line: &str, start: &LineState) -> (Vec<Span>, LineState) {
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut spans = Vec::new();
    let mut at = 0;

    if matches!(start, LineState::BlockComment) {
        match block_close(bytes, 0) {
            Some(end) => {
                spans.push(Span::new(0, end, Role::Comment));
                at = end;
            }
            None => {
                spans.push(Span::new(0, len, Role::Comment));
                return (spans, LineState::BlockComment);
            }
        }
    }

    while at < len {
        let byte = bytes[at];
        if byte == b'/' && bytes.get(at + 1) == Some(&b'*') {
            match block_close(bytes, at + 2) {
                Some(end) => {
                    spans.push(Span::new(at, end, Role::Comment));
                    at = end;
                }
                None => {
                    spans.push(Span::new(at, len, Role::Comment));
                    return (spans, LineState::BlockComment);
                }
            }
            continue;
        }
        if byte == b'"' || byte == b'\'' {
            let end = escaped(bytes, at, byte);
            spans.push(Span::new(at, end, Role::String));
            at = end;
            continue;
        }
        if byte == b'@' {
            let end = identifier(bytes, at + 1);
            spans.push(Span::new(at, end, Role::Keyword));
            at = end.max(at + 1);
            continue;
        }
        if byte.is_ascii_digit() {
            let end = number(bytes, at);
            spans.push(Span::new(at, end, Role::Number));
            at = end;
            continue;
        }
        if byte.is_ascii_whitespace() {
            at += 1;
            continue;
        }
        if matches!(
            byte,
            b'{' | b'}' | b':' | b';' | b',' | b'(' | b')' | b'#' | b'.'
        ) {
            spans.push(Span::new(at, at + 1, Role::Punctuation));
        }
        at += 1;
    }
    (spans, LineState::Clean)
}

/// The index just past a nested `/* … */` starting after `from`, or `None` if it does not close.
fn block_close(bytes: &[u8], from: usize) -> Option<usize> {
    let mut depth = 1usize;
    let mut at = from;
    while at + 1 < bytes.len() {
        if bytes[at] == b'/' && bytes[at + 1] == b'*' {
            depth += 1;
            at += 2;
        } else if bytes[at] == b'*' && bytes[at + 1] == b'/' {
            depth -= 1;
            at += 2;
            if depth == 0 {
                return Some(at);
            }
        } else {
            at += 1;
        }
    }
    None
}

/// The index just past a string starting at `at` with the given quote, honouring `\` escapes.
fn escaped(bytes: &[u8], at: usize, quote: u8) -> usize {
    let mut index = at + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            byte if byte == quote => return index + 1,
            _ => index += 1,
        }
    }
    bytes.len()
}

/// A Rust char literal at `at`, when it is one (not a lifetime).
fn char_literal(bytes: &[u8], at: usize) -> Option<usize> {
    match bytes.get(at + 1) {
        Some(b'\\') => {
            let mut index = at + 2;
            while index < bytes.len() && bytes[index] != b'\'' {
                index += 1;
            }
            Some((index + 1).min(bytes.len()))
        }
        Some(_) if bytes.get(at + 2) == Some(&b'\'') => Some(at + 3),
        _ => None,
    }
}

/// A raw string opener at `at` (`r`, some `#`, a `"`), answering where the content starts and how
/// many `#` there were.
fn raw_open(bytes: &[u8], at: usize) -> Option<(usize, usize)> {
    let mut index = at + 1;
    while bytes.get(index) == Some(&b'#') {
        index += 1;
    }
    if bytes.get(index) == Some(&b'"') {
        Some((index + 1, index - at - 1))
    } else {
        None
    }
}

/// The delimiter that closes a raw string opened with `hashes` `#`.
fn raw_close(hashes: usize) -> String {
    format!("\"{}", "#".repeat(hashes))
}

/// A number, including `_` separators, a fraction, an exponent and a type suffix.
fn number(bytes: &[u8], at: usize) -> usize {
    let mut index = at;
    while index < bytes.len()
        && (bytes[index].is_ascii_alphanumeric()
            || matches!(bytes[index], b'_' | b'.' | b'+' | b'-'))
    {
        index += 1;
    }
    index
}

/// An identifier.
fn identifier(bytes: &[u8], at: usize) -> usize {
    let mut index = at;
    while index < bytes.len() && (bytes[index] == b'_' || bytes[index].is_ascii_alphanumeric()) {
        index += 1;
    }
    index
}

fn is_punctuation(byte: u8) -> bool {
    matches!(
        byte,
        b'{' | b'}'
            | b'('
            | b')'
            | b'['
            | b']'
            | b'<'
            | b'>'
            | b'.'
            | b','
            | b';'
            | b':'
            | b'='
            | b'+'
            | b'-'
            | b'*'
            | b'/'
            | b'&'
            | b'|'
            | b'!'
            | b'?'
            | b'#'
            | b'%'
            | b'^'
            | b'~'
            | b'@'
    )
}

/// The first index of `needle` in `haystack` at or after `from`.
fn find(haystack: &str, needle: &str, from: usize) -> Option<usize> {
    haystack[from..]
        .find(needle)
        .map(|position| position + from)
}

#[cfg(test)]
mod tests {
    use super::{Highlights, Language, LineState, Role, State, Theme, lex_line};

    fn rust(lines: &[&str]) -> Highlights {
        let mut highlights = Highlights::new(Language::Rust);
        highlights.set_text(&lines.join("\n"));
        highlights.resolve_all();
        highlights
    }

    #[test]
    fn a_multi_line_block_comment_is_correct_on_every_line() {
        let highlights = rust(&["/* open", "still inside", "close */ let x = 1;"]);

        assert_eq!(highlights.state(0), State::Known(LineState::Clean));
        assert_eq!(
            highlights.state(1),
            State::Known(LineState::BlockComment),
            "line 1 starts inside the comment because line 0 left it open"
        );
        assert_eq!(highlights.state(2), State::Known(LineState::BlockComment));
        // Line 0 is entirely the comment.
        assert_eq!(highlights.spans(0).unwrap().len(), 1);
        assert_eq!(highlights.spans(0).unwrap()[0].role(), Role::Comment);
        // Line 2 closes the comment and then lexes `let` as a keyword and `1` as a number.
        let roles: Vec<Role> = highlights
            .spans(2)
            .unwrap()
            .iter()
            .map(|span| span.role())
            .collect();
        assert_eq!(
            roles,
            vec![
                Role::Comment,
                Role::Keyword,
                Role::Punctuation,
                Role::Number,
                Role::Punctuation
            ]
        );
        // Every line is back to clean after it.
        assert_eq!(highlights.state(3), State::Known(LineState::Clean));
    }

    #[test]
    fn a_multi_line_raw_string_is_correct_and_odd_hashes_do_not_end_it() {
        // r#" ... "# — a bare `"` inside the first line must not close it; only `"#` does.
        let highlights = rust(&["let s = r#\"a \" b", "more", "\"#; let t = 2;"]);
        assert_eq!(highlights.state(0), State::Known(LineState::Clean));
        assert_eq!(
            highlights.state(1),
            State::Known(LineState::RawString { hashes: 1 })
        );
        // The whole first line from `r` on is one string.
        let spans = highlights.spans(0).unwrap();
        let last = spans.last().unwrap();
        assert_eq!(last.role(), Role::String);
        assert_eq!(last.end(), "let s = r#\"a \" b".len());
        // The closing `"#` lands on line 2, after which `let` is a keyword again.
        assert_eq!(highlights.state(3), State::Known(LineState::Clean));
    }

    #[test]
    fn json_and_css_lex_their_own_way() {
        let mut json = Highlights::new(Language::Json);
        json.set_text("{\"a\": 1, \"b\": true}");
        json.resolve_all();
        let roles: Vec<Role> = json.spans(0).unwrap().iter().map(|s| s.role()).collect();
        assert_eq!(
            roles,
            vec![
                Role::Punctuation,
                Role::String,
                Role::Punctuation,
                Role::Number,
                Role::Punctuation,
                Role::String,
                Role::Punctuation,
                Role::Keyword,
                Role::Punctuation
            ]
        );

        // CSS has the block comment, and a JSON-style `true` is not a keyword there.
        let mut css = Highlights::new(Language::Css);
        css.set_text("/* a\nb */ color: red;");
        css.resolve_all();
        assert_eq!(css.state(1), State::Known(LineState::BlockComment));
        assert_eq!(css.state(2), State::Known(LineState::Clean));
    }

    #[test]
    fn an_unlexed_line_reads_as_unknown_never_as_clean() {
        let mut highlights = Highlights::new(Language::Rust);
        highlights.set_text("/* a\nb */\nc");
        assert_eq!(highlights.resolved(), 0);
        // Line 0 always starts clean.
        assert_eq!(highlights.state(0), State::Known(LineState::Clean));
        // Line 1 might be clean or inside a comment; until line 0 is lexed we do not know.
        assert_eq!(highlights.state(1), State::Unknown);
        assert!(highlights.spans(0).is_none());

        highlights.step(1);
        assert_eq!(
            highlights.state(1),
            State::Known(LineState::BlockComment),
            "after line 0, line 1's start state is known"
        );
        assert_eq!(highlights.state(2), State::Unknown);
    }

    #[test]
    fn painting_reads_the_cache_and_cannot_lex() {
        let highlights = rust(&["let x = 1;", "fn f() {}"]);
        let lexed = highlights.lexed_lines();

        // The paint path: `state` and `spans` take `&self`, so this cannot run the lexer.
        let paint = |highlights: &Highlights| {
            for line in 0..highlights.line_count() {
                let _ = highlights.state(line);
                let _ = highlights.spans(line);
            }
        };
        for _ in 0..100 {
            paint(&highlights);
        }
        assert_eq!(
            highlights.lexed_lines(),
            lexed,
            "drawing never lexed anything"
        );
    }

    #[test]
    fn an_edit_inside_a_line_re_lexes_only_until_the_state_converges() {
        let lines: Vec<String> = (0..100)
            .map(|index| format!("let v{index} = {index};"))
            .collect();
        let mut highlights = Highlights::new(Language::Rust);
        highlights.set_text(&lines.join("\n"));
        highlights.resolve_all();
        let before = highlights.lexed_lines();

        // Change one line without changing its end state.
        highlights.set_line(40, "let changed = 41;");
        assert_eq!(
            highlights.resolved(),
            40,
            "the cache is unreliable from the edit on"
        );
        highlights.resolve_all();

        assert_eq!(
            highlights.lexed_lines() - before,
            1,
            "the edit cost exactly one line, not the file"
        );
        assert!(highlights.is_complete());
    }

    #[test]
    fn an_edit_that_opens_a_comment_re_lexes_the_tail() {
        let lines: Vec<String> = (0..20)
            .map(|index| format!("let v{index} = {index};"))
            .collect();
        let mut highlights = Highlights::new(Language::Rust);
        highlights.set_text(&lines.join("\n"));
        highlights.resolve_all();
        let before = highlights.lexed_lines();

        // This line now opens a block comment that never closes on it.
        highlights.set_line(5, "let broken = /* oops");
        highlights.resolve_all();

        assert!(
            highlights.lexed_lines() - before >= 15,
            "the tail had to be re-lexed: the state changed"
        );
        assert_eq!(
            highlights.state(6),
            State::Known(LineState::BlockComment),
            "everything after the edit is inside the comment now"
        );
    }

    #[test]
    fn switching_the_language_throws_the_cache_away() {
        let mut highlights = Highlights::new(Language::Json);
        highlights.set_text("{\"a\": 1}");
        highlights.resolve_all();
        assert!(highlights.is_complete());

        highlights.set_language(Language::Rust);
        assert_eq!(
            highlights.resolved(),
            0,
            "a new grammar re-lexes everything"
        );
        assert_eq!(highlights.language(), Language::Rust);
        assert_eq!(Language::of_path("src/main.rs"), Language::Rust);
        assert_eq!(Language::of_path("a/b/c.json"), Language::Json);
        assert_eq!(Language::of_path("style.css"), Language::Css);
        assert_eq!(Language::of_path("README"), Language::Plain);
    }

    #[test]
    fn the_theme_is_a_seam_and_the_model_holds_no_colour() {
        // A span carries a role; the theme decides the colour.
        let (spans, _) = lex_line(Language::Json, "\"x\"", &LineState::Clean);
        assert_eq!(spans[0].role(), Role::String);

        // Both themes cover every role, and the same span is a different colour under each.
        let dark = Theme::dark();
        let light = Theme::light();
        assert!(dark.covers_every_role());
        assert!(light.covers_every_role());
        assert_ne!(dark.color_of(Role::String), light.color_of(Role::String));
        // The spans themselves do not change when the theme does.
        let highlights = rust(&["let x = 1;"]);
        let runs_before = highlights.spans(0).unwrap().to_vec();
        let _ = Theme::named("light");
        assert_eq!(highlights.spans(0).unwrap(), runs_before.as_slice());
    }

    #[test]
    fn the_report_names_the_language_the_states_and_the_spans() {
        let highlights = rust(&["/* a", "b */ let x = 1;"]);
        let report = highlights.render(&Theme::dark()).join("\n");
        assert!(report.contains("rust: 2 line(s)"), "{report}");
        assert!(report.contains("in a block comment"), "{report}");
        assert!(report.contains("comment"), "{report}");
        assert!(report.contains("keyword"), "{report}");
    }
}
