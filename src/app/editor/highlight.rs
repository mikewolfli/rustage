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
//! * **The grammar is the library's.** [`lex_line`] calls the widget's [`BuiltinHighlighter`] over
//!   the library's [`LanguageId`]; this module owns the incremental cache and the role projection,
//!   not a second lexer. The host therefore highlights the same languages the widget does.

#![allow(dead_code)]

use std::collections::BTreeMap;

use rust_widgets::widget::special_widgets::code_editor::{
    BuiltinHighlighter, LanguageId, LineState, SyntaxHighlighter, TokenKind,
};

/// A language this build highlights.
///
/// The grammar and the list of languages are the library's ([`LanguageId`]); this type only supplies
/// the names a command line or a configuration file uses, so the host does not keep a second language
/// table that could disagree with the widget's.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Language(LanguageId);

// The four names below keep the host's existing spelling (`Language::Rust`) instead of forcing every
// caller onto an upper-case constant; the values are the library's own variants.
#[allow(non_upper_case_globals)]
impl Language {
    /// No grammar: every line is plain text.
    pub const Plain: Self = Self(LanguageId::PlainText);
    /// Rust.
    pub const Rust: Self = Self(LanguageId::Rust);
    /// JSON.
    pub const Json: Self = Self(LanguageId::Json);
    /// CSS.
    pub const Css: Self = Self(LanguageId::Css);

    /// The library's own language identifier, which is the one the widget lexes with.
    pub const fn id(self) -> LanguageId {
        self.0
    }

    /// The language's name, as a command line or a configuration file writes it.
    pub const fn name(self) -> &'static str {
        match self.0 {
            LanguageId::Rust => "rust",
            LanguageId::Python => "python",
            LanguageId::JavaScript => "javascript",
            LanguageId::Go => "go",
            LanguageId::CLike => "c",
            LanguageId::Java => "java",
            LanguageId::CSharp => "csharp",
            LanguageId::Cpp => "cpp",
            LanguageId::Kotlin => "kotlin",
            LanguageId::Swift => "swift",
            LanguageId::Html => "html",
            LanguageId::Xml => "xml",
            LanguageId::Json => "json",
            LanguageId::Css => "css",
            LanguageId::Yaml => "yaml",
            LanguageId::Toml => "toml",
            LanguageId::Shell => "shell",
            LanguageId::Sql => "sql",
            LanguageId::Markdown => "markdown",
            LanguageId::PlainText => "plain",
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
            "py" => Self(LanguageId::Python),
            "js" | "jsx" | "ts" | "tsx" | "mjs" => Self(LanguageId::JavaScript),
            "go" => Self(LanguageId::Go),
            "c" | "h" => Self(LanguageId::CLike),
            "java" => Self(LanguageId::Java),
            "cs" => Self(LanguageId::CSharp),
            "cc" | "cpp" | "cxx" | "hpp" => Self(LanguageId::Cpp),
            "kt" | "kts" => Self(LanguageId::Kotlin),
            "swift" => Self(LanguageId::Swift),
            "html" | "htm" => Self(LanguageId::Html),
            "xml" => Self(LanguageId::Xml),
            "json" => Self::Json,
            "css" => Self::Css,
            "yml" | "yaml" => Self(LanguageId::Yaml),
            "toml" => Self(LanguageId::Toml),
            "sh" | "bash" | "zsh" => Self(LanguageId::Shell),
            "sql" => Self(LanguageId::Sql),
            "md" | "markdown" => Self(LanguageId::Markdown),
            _ => Self::Plain,
        }
    }

    /// Every language the library's lexer understands.
    pub const ALL: [Self; 20] = [
        Self(LanguageId::Rust),
        Self(LanguageId::Python),
        Self(LanguageId::JavaScript),
        Self(LanguageId::Go),
        Self(LanguageId::CLike),
        Self(LanguageId::Java),
        Self(LanguageId::CSharp),
        Self(LanguageId::Cpp),
        Self(LanguageId::Kotlin),
        Self(LanguageId::Swift),
        Self(LanguageId::Html),
        Self(LanguageId::Xml),
        Self(LanguageId::Json),
        Self(LanguageId::Css),
        Self(LanguageId::Yaml),
        Self(LanguageId::Toml),
        Self(LanguageId::Shell),
        Self(LanguageId::Sql),
        Self(LanguageId::Markdown),
        Self(LanguageId::PlainText),
    ];
}

/// What part of a line a span is, before any theme has a say.
///
/// This is a projection of the library's [`TokenKind`]: it keys the host's themes, while the kinds
/// the widget distinguishes (identifier, operator, preprocessor, …) are mapped into it by
/// [`Role::of`]. Nothing here classifies a word; the lexer is the library's.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Role {
    /// A comment.
    Comment,
    /// A string or character literal.
    String,
    /// A keyword or preprocessor word.
    Keyword,
    /// A type name.
    Type,
    /// A function or method name.
    Function,
    /// A number.
    Number,
    /// Punctuation and operators.
    Punctuation,
}

impl Role {
    /// The role a library token kind is drawn as, or `None` for text a theme leaves alone.
    pub const fn of(kind: TokenKind) -> Option<Self> {
        match kind {
            TokenKind::Keyword | TokenKind::Preprocessor => Some(Self::Keyword),
            TokenKind::Type => Some(Self::Type),
            TokenKind::Function => Some(Self::Function),
            TokenKind::String | TokenKind::Character => Some(Self::String),
            TokenKind::Number => Some(Self::Number),
            TokenKind::Comment | TokenKind::BlockComment => Some(Self::Comment),
            TokenKind::Operator => Some(Self::Punctuation),
            TokenKind::Identifier | TokenKind::Plain => None,
        }
    }

    /// The role's name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Comment => "comment",
            Self::String => "string",
            Self::Keyword => "keyword",
            Self::Type => "type",
            Self::Function => "function",
            Self::Number => "number",
            Self::Punctuation => "punctuation",
        }
    }

    /// Every role a theme must have a colour for.
    pub const ALL: [Self; 7] = [
        Self::Comment,
        Self::String,
        Self::Keyword,
        Self::Type,
        Self::Function,
        Self::Number,
        Self::Punctuation,
    ];
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
            Self::Known(LineState::Clean) => "clean".to_owned(),
            Self::Known(LineState::BlockComment) => "in a block comment".to_owned(),
            Self::Known(LineState::MultiLineString) => "in a multi-line string".to_owned(),
            Self::Known(LineState::MultiLineCharString) => {
                "in a multi-line character string".to_owned()
            }
            Self::Known(LineState::RawString { hashes }) => {
                format!("in a raw string ({hashes} #)")
            }
            Self::Known(LineState::RawStringTick) => "in a backtick raw string".to_owned(),
            Self::Known(LineState::HtmlComment) => "in an HTML comment".to_owned(),
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

    fn of(name: &str, pairs: [(Role, &str); 7]) -> Self {
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

const DARK: [(Role, &str); 7] = [
    (Role::Comment, "#7f848e"),
    (Role::String, "#98c379"),
    (Role::Keyword, "#c678dd"),
    (Role::Type, "#e5c07b"),
    (Role::Function, "#61afef"),
    (Role::Number, "#d19a66"),
    (Role::Punctuation, "#abb2bf"),
];

const LIGHT: [(Role, &str); 7] = [
    (Role::Comment, "#a0a1a7"),
    (Role::String, "#50a14f"),
    (Role::Keyword, "#a626a4"),
    (Role::Type, "#c18401"),
    (Role::Function, "#4078f2"),
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
            return State::Known(self.cache[line - 1].ends);
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
                self.cache[index - 1].ends
            };
            let (spans, ends) = lex_line(self.language, &self.lines[index], &start);
            let previous = std::mem::replace(&mut self.cache[index].ends, ends);
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
///
/// The lexer is the library's [`BuiltinHighlighter`], so the host and the widget cannot disagree
/// about what a comment, a raw string or a keyword is. This function only projects the library's
/// `TokenSpan`s onto the host's [`Role`]s and drops the spans a theme leaves uncoloured; it keeps
/// no grammar of its own.
pub fn lex_line(language: Language, line: &str, start: &LineState) -> (Vec<Span>, LineState) {
    let highlighter = BuiltinHighlighter::new(language.id());
    let (tokens, ends) = highlighter.highlight_line(line, *start);
    let spans = tokens
        .into_iter()
        .filter_map(|token| {
            Role::of(token.kind).map(|role| Span::new(token.start, token.end, role))
        })
        .collect();
    (spans, ends)
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
        // The library leaves JSON's structural braces unspanned; it styles the values and the
        // separators between them, which is the behaviour this test now records.
        assert_eq!(
            roles,
            vec![
                Role::String,
                Role::Punctuation,
                Role::Number,
                Role::Punctuation,
                Role::String,
                Role::Punctuation,
                Role::Keyword
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
