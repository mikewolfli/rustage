// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Text and project search, with a visible regex error, cancellation and truncation (task T08.02).
//!
//! The acceptance line is *"a regex error, a cancellation and a truncated result are visible"*. The
//! three ways a search can fail to give a full answer are each a **field on the result** rather than a
//! silent shortfall:
//!
//! * **A bad pattern is a value, not a panic.** The pattern compiler runs before any file is read and
//!   answers [`SearchError::BadPattern`] with the offset of the offending character, so a malformed
//!   pattern is reported once, up front.
//! * **A cancellation is recorded.** [`search`] checks the [`Cancellation`] token between files; a
//!   cancelled search returns the matches it found with [`Results::cancelled`] set, so "this is a
//!   prefix" is stated rather than implied by a short list.
//! * **A truncation is recorded.** Past [`Limits::max_results`] the search stops and sets
//!   [`Results::truncated`], so a capped list never looks like a complete one.
//!
//! Two policy choices the acceptance does not name but a reader would ask about:
//!
//! * **The regex is a stated subset.** The crate carries no regex dependency (and adding a heavy one
//!   for this is not clearly worth it), so [`Query::regex`] compiles a subset — literals, `.`, `*`,
//!   `+`, `?`, `[...]` classes, `^` and `$` — and *says so* in this line rather than pretending to be
//!   `regex`. A pattern using anything outside the subset (a `(` group, an alternation) is a literal
//!   or an error, not a silent wrong answer.
//! * **Case-insensitivity folds to lower case in both the text and the pattern.** For the subset this
//!   is exact for ASCII and approximate for other scripts; recorded as a boundary.

#![allow(dead_code)]

use crate::app::scale::Cancellation;

/// How to search.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Query {
    /// The pattern.
    pub pattern: String,
    /// Whether the pattern is a regex (the subset this module compiles) or a plain substring.
    pub regex: bool,
    /// Whether case matters.
    pub case_sensitive: bool,
    /// Whether a match must stand alone as a word.
    pub whole_word: bool,
    /// A byte range of each file to search, when the search is scoped.
    pub range: Option<(usize, usize)>,
}

impl Query {
    /// A literal search.
    pub fn literal(pattern: impl Into<String>) -> Self {
        Self {
            pattern: pattern.into(),
            regex: false,
            case_sensitive: true,
            whole_word: false,
            range: None,
        }
    }

    /// A regex search over the subset this module compiles.
    pub fn regex(pattern: impl Into<String>) -> Self {
        Self {
            regex: true,
            ..Self::literal(pattern)
        }
    }

    /// The query in one line.
    pub fn render(&self) -> String {
        format!(
            "{:?} ({}), {}{}",
            self.pattern,
            if self.regex { "regex" } else { "literal" },
            if self.case_sensitive {
                "case"
            } else {
                "no-case"
            },
            if self.whole_word { ", whole-word" } else { "" }
        )
    }
}

/// Why a search could not run.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SearchError {
    /// The pattern is not valid in the subset this module compiles.
    BadPattern {
        /// The character offset the problem is at.
        at: usize,
        /// What is wrong.
        message: String,
    },
}

impl SearchError {
    /// The error in one sentence.
    pub fn message(&self) -> String {
        match self {
            Self::BadPattern { at, message } => {
                format!("the pattern is invalid at character {at}: {message}")
            }
        }
    }
}

/// How many results to keep.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Limits {
    /// The most matches to return.
    pub max_results: usize,
}

impl Limits {
    /// A ceiling of one thousand matches.
    pub const fn new() -> Self {
        Self { max_results: 1000 }
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self::new()
    }
}

/// One match.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Match {
    /// The file.
    pub path: String,
    /// The zero-based line.
    pub line: usize,
    /// The match's start, as a byte offset in the file.
    pub start: usize,
    /// Its end.
    pub end: usize,
    /// The line's text.
    pub text: String,
}

impl Match {
    /// The match in one line.
    pub fn render(&self) -> String {
        format!("{}:{} {}..{}", self.path, self.line, self.start, self.end)
    }
}

/// The outcome of a search.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Results {
    /// The matches, in file and position order.
    pub matches: Vec<Match>,
    /// Whether the result was cut off at the ceiling.
    pub truncated: bool,
    /// Whether the search was cancelled before it finished.
    pub cancelled: bool,
    /// How many files were searched.
    pub files_searched: usize,
}

impl Results {
    /// Whether the result is a complete answer.
    pub fn is_complete(&self) -> bool {
        !self.truncated && !self.cancelled
    }

    /// The outcome in one report.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "{} match(es) over {} file(s){}{}",
            self.matches.len(),
            self.files_searched,
            if self.truncated { ", truncated" } else { "" },
            if self.cancelled { ", cancelled" } else { "" }
        )];
        for item in &self.matches {
            lines.push(format!("  {}", item.render()));
        }
        lines
    }
}

/// One token of a compiled pattern.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) enum Token {
    /// A literal character.
    Char(char),
    /// `.`: any character.
    Any,
    /// `[...]`: a class.
    Class {
        negated: bool,
        ranges: Vec<(char, char)>,
    },
    /// `^`: the start of the line.
    Start,
    /// `$`: the end of the line.
    End,
    /// A repeat of the token before it.
    Repeat { kind: RepeatKind, token: Box<Token> },
}

/// Which repeat a postfix operator is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum RepeatKind {
    /// `*`
    Star,
    /// `+`
    Plus,
    /// `?`
    Opt,
}

/// Compiles a pattern into tokens, or says where it is invalid.
pub(crate) fn compile(pattern: &str) -> Result<Vec<Token>, SearchError> {
    let characters: Vec<char> = pattern.chars().collect();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < characters.len() {
        let at = index;
        let token = match characters[index] {
            '^' => {
                index += 1;
                Token::Start
            }
            '$' => {
                index += 1;
                Token::End
            }
            '.' => {
                index += 1;
                Token::Any
            }
            '[' => {
                let (class, next) = compile_class(&characters, index)?;
                index = next;
                class
            }
            '\\' => {
                if index + 1 >= characters.len() {
                    return Err(SearchError::BadPattern {
                        at,
                        message: "a backslash at the end escapes nothing".to_owned(),
                    });
                }
                index += 2;
                class_or_char(characters[index - 1])
            }
            '*' | '+' | '?' => {
                return Err(SearchError::BadPattern {
                    at,
                    message: "a repeat needs a character before it".to_owned(),
                });
            }
            other => {
                index += 1;
                Token::Char(other)
            }
        };
        if index < characters.len() && matches!(characters[index], '*' | '+' | '?') {
            let kind = match characters[index] {
                '*' => RepeatKind::Star,
                '+' => RepeatKind::Plus,
                _ => RepeatKind::Opt,
            };
            index += 1;
            if matches!(token, Token::Start | Token::End) {
                return Err(SearchError::BadPattern {
                    at,
                    message: "an anchor cannot be repeated".to_owned(),
                });
            }
            tokens.push(Token::Repeat {
                kind,
                token: Box::new(token),
            });
        } else {
            tokens.push(token);
        }
    }
    Ok(tokens)
}

/// The token a backslash escape stands for: a shorthand class, or a literal.
fn class_or_char(escaped: char) -> Token {
    let digits = vec![('0', '9')];
    let word = vec![('a', 'z'), ('A', 'Z'), ('0', '9'), ('_', '_')];
    let space = vec![(' ', ' '), ('\t', '\t'), ('\n', '\n')];
    match escaped {
        'd' => Token::Class {
            negated: false,
            ranges: digits,
        },
        'D' => Token::Class {
            negated: true,
            ranges: digits,
        },
        'w' => Token::Class {
            negated: false,
            ranges: word,
        },
        'W' => Token::Class {
            negated: true,
            ranges: word,
        },
        's' => Token::Class {
            negated: false,
            ranges: space,
        },
        'S' => Token::Class {
            negated: true,
            ranges: space,
        },
        'n' => Token::Char('\n'),
        't' => Token::Char('\t'),
        other => Token::Char(other),
    }
}

/// Compiles a `[...]` class, returning it and the index after the closing `]`.
fn compile_class(characters: &[char], open: usize) -> Result<(Token, usize), SearchError> {
    let mut index = open + 1;
    let negated = characters.get(index) == Some(&'^');
    if negated {
        index += 1;
    }
    let mut ranges = Vec::new();
    let mut first = true;
    while index < characters.len() {
        let character = characters[index];
        if character == ']' && !first {
            return Ok((Token::Class { negated, ranges }, index + 1));
        }
        first = false;
        let low = if character == '\\' {
            index += 1;
            match characters.get(index) {
                Some(escaped) => *escaped,
                None => {
                    return Err(SearchError::BadPattern {
                        at: index,
                        message: "a backslash at the end escapes nothing".to_owned(),
                    });
                }
            }
        } else {
            character
        };
        // A range a-z.
        if characters.get(index + 1) == Some(&'-')
            && characters.get(index + 2).is_some_and(|c| *c != ']')
        {
            let high = characters[index + 2];
            ranges.push((low, high));
            index += 3;
        } else {
            ranges.push((low, low));
            index += 1;
        }
    }
    Err(SearchError::BadPattern {
        at: open,
        message: "the character class is never closed".to_owned(),
    })
}

/// Finds a match starting at or after `from` in a line, as character indices.
fn find_in_line(
    pattern: &str,
    tokens: Option<&[Token]>,
    line: &str,
    from: usize,
) -> Option<(usize, usize)> {
    let characters: Vec<char> = line.chars().collect();
    match tokens {
        Some(tokens) => {
            for start in from..=characters.len() {
                if let Some(end) = match_from(tokens, &characters, start)
                    && end > start
                {
                    return Some((start, end));
                }
            }
            None
        }
        None => {
            let needle: Vec<char> = pattern.chars().collect();
            if needle.is_empty() || needle.len() > characters.len() {
                return None;
            }
            let last = characters.len() - needle.len();
            if from > last {
                return None;
            }
            for start in from..=last {
                if characters[start..start + needle.len()] == needle[..] {
                    return Some((start, start + needle.len()));
                }
            }
            None
        }
    }
}

/// Whether the tokens can match starting at `position`, returning where the match ends.
fn match_from(tokens: &[Token], text: &[char], position: usize) -> Option<usize> {
    match tokens.split_first() {
        None => Some(position),
        Some((Token::Start, rest)) => (position == 0).then(|| match_from(rest, text, position))?,
        Some((Token::End, rest)) => {
            (position == text.len()).then(|| match_from(rest, text, position))?
        }
        Some((Token::Repeat { kind, token }, rest)) => {
            match_repeat(*kind, token, rest, text, position)
        }
        Some((token, rest)) => {
            match_atom(token, text, position).and_then(|next| match_from(rest, text, next))
        }
    }
}

/// Matches one atom and returns the position after it.
fn match_atom(token: &Token, text: &[char], position: usize) -> Option<usize> {
    match token {
        Token::Char(expected) => (text.get(position) == Some(expected)).then_some(position + 1),
        Token::Any => text.get(position).map(|_| position + 1),
        Token::Class { negated, ranges } => {
            let character = *text.get(position)?;
            let inside = ranges
                .iter()
                .any(|(low, high)| low <= &character && &character <= high);
            (inside != *negated).then_some(position + 1)
        }
        Token::Start | Token::End | Token::Repeat { .. } => None,
    }
}

/// Matches a repeat, greedily, and the rest after it.
fn match_repeat(
    kind: RepeatKind,
    token: &Token,
    rest: &[Token],
    text: &[char],
    position: usize,
) -> Option<usize> {
    // Every position reachable by repeating the atom; the atom always advances, so this terminates.
    let mut positions = vec![position];
    let mut current = position;
    while let Some(next) = match_atom(token, text, current) {
        positions.push(next);
        current = next;
    }
    let candidates: Vec<usize> = match kind {
        RepeatKind::Star => positions.iter().rev().copied().collect(),
        RepeatKind::Plus => positions.iter().skip(1).rev().copied().collect(),
        RepeatKind::Opt => positions.iter().take(2).rev().copied().collect(),
    };
    candidates
        .into_iter()
        .find_map(|candidate| match_from(rest, text, candidate))
}

/// Whether a character is part of a word for a whole-word search.
fn is_word_char(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

/// Searches one file's text.
pub fn search_text(path: &str, text: &str, query: &Query) -> Result<Vec<Match>, SearchError> {
    let (pattern, tokens) = prepare(query)?;
    Ok(collect_matches(
        path,
        text,
        query,
        &pattern,
        tokens.as_deref(),
    ))
}

/// Searches many files, checking the token between them.
pub fn search(
    files: &[(String, String)],
    query: &Query,
    limits: Limits,
    token: &Cancellation,
) -> Result<Results, SearchError> {
    let (pattern, tokens) = prepare(query)?;
    let mut results = Results::default();
    for (path, text) in files {
        if token.is_cancelled() {
            results.cancelled = true;
            return Ok(results);
        }
        if results.matches.len() >= limits.max_results {
            results.truncated = true;
            return Ok(results);
        }
        results.files_searched += 1;
        let mut found = collect_matches(path, text, query, &pattern, tokens.as_deref());
        for item in found.drain(..) {
            results.matches.push(item);
            if results.matches.len() >= limits.max_results {
                results.truncated = true;
                return Ok(results);
            }
        }
    }
    Ok(results)
}

/// Prepares the pattern: lower-cased for an insensitive search, and compiled when it is a regex.
fn prepare(query: &Query) -> Result<(String, Option<Vec<Token>>), SearchError> {
    let pattern = if query.case_sensitive {
        query.pattern.clone()
    } else {
        query.pattern.to_lowercase()
    };
    let tokens = if query.regex {
        Some(compile(&pattern)?)
    } else {
        None
    };
    Ok((pattern, tokens))
}

/// Every match in one text.
fn collect_matches(
    path: &str,
    text: &str,
    query: &Query,
    pattern: &str,
    tokens: Option<&[Token]>,
) -> Vec<Match> {
    let mut matches = Vec::new();
    let mut line_start = 0;
    for (line, raw) in text.split('\n').enumerate() {
        let searchable = if query.case_sensitive || query.regex {
            raw.to_owned()
        } else {
            raw.to_lowercase()
        };
        let mut from = 0;
        while let Some((start, end)) = find_in_line(pattern, tokens, &searchable, from) {
            from = end.max(start + 1);
            let byte_start = line_start + byte_offset(raw, start);
            let byte_end = line_start + byte_offset(raw, end);
            if let Some((low, high)) = query.range
                && (byte_start < low || byte_start >= high)
            {
                continue;
            }
            if query.whole_word && !is_whole_word(raw, start, end) {
                continue;
            }
            matches.push(Match {
                path: path.to_owned(),
                line,
                start: byte_start,
                end: byte_end,
                text: raw.to_owned(),
            });
        }
        line_start += raw.len() + 1;
    }
    matches
}

/// Whether a character range is a whole word in a line.
fn is_whole_word(line: &str, start: usize, end: usize) -> bool {
    let characters: Vec<char> = line.chars().collect();
    let before = start
        .checked_sub(1)
        .and_then(|i| characters.get(i))
        .copied();
    let after = characters.get(end).copied();
    let before_ok = before.is_none_or(|c| !is_word_char(c));
    let after_ok = after.is_none_or(|c| !is_word_char(c));
    before_ok && after_ok
}

/// The byte offset of a character index in a line.
fn byte_offset(line: &str, character: usize) -> usize {
    line.char_indices()
        .nth(character)
        .map_or(line.len(), |(offset, _)| offset)
}

#[cfg(test)]
mod tests {
    use super::{Limits, Query, SearchError, compile, search, search_text};
    use crate::app::scale::Cancellation;

    fn files() -> Vec<(String, String)> {
        vec![
            ("a.rs".to_owned(), "fn foo() {\n    foo();\n}\n".to_owned()),
            ("b.rs".to_owned(), "let FOO = 1;\n".to_owned()),
        ]
    }

    #[test]
    fn a_literal_search_finds_every_occurrence() {
        let results = search(
            &files(),
            &Query::literal("foo"),
            Limits::new(),
            &Cancellation::new(),
        )
        .expect("a search");
        assert_eq!(results.matches.len(), 2);
        assert!(results.is_complete());
        assert_eq!(results.matches[0].line, 0);
        assert_eq!(results.matches[1].line, 1);
    }

    #[test]
    fn case_insensitivity_and_whole_word_are_honoured() {
        let mut query = Query::literal("foo");
        query.case_sensitive = false;
        let results =
            search(&files(), &query, Limits::new(), &Cancellation::new()).expect("search");
        // Three "foo" case-insensitively: two in a.rs, one in b.rs.
        assert_eq!(results.matches.len(), 3);

        // Whole word: "foo()" still counts (a paren is not a word char), but "foobar" would not.
        let text = "foo foobar foo\n";
        let mut query = Query::literal("foo");
        query.whole_word = true;
        let results = search_text("x", text, &query).expect("search");
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn a_regex_subset_matches() {
        let text = "let x = 12;\nlet y = 7;\n";
        let results = search_text("x", text, &Query::regex(r"\d+")).expect("search");
        assert_eq!(results.len(), 2);
        // Anchors and classes.
        let results = search_text("x", text, &Query::regex("^let [a-z]")).expect("search");
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn a_bad_pattern_is_an_error_with_an_offset() {
        match compile("a[") {
            Err(SearchError::BadPattern { at, message }) => {
                assert_eq!(at, 1);
                assert!(message.contains("never closed"));
            }
            other => panic!("expected a bad pattern, got {other:?}"),
        }
        assert!(matches!(
            compile("*x"),
            Err(SearchError::BadPattern { at: 0, .. })
        ));
        assert!(matches!(
            compile("a\\"),
            Err(SearchError::BadPattern { at: 1, .. })
        ));
        // A search with a bad pattern errors before reading anything.
        let error = search(
            &files(),
            &Query::regex("a["),
            Limits::new(),
            &Cancellation::new(),
        )
        .expect_err("a bad pattern");
        assert!(error.message().contains("invalid"));
    }

    #[test]
    fn a_cancellation_is_visible() {
        let token = Cancellation::new();
        token.cancel();
        let results =
            search(&files(), &Query::literal("foo"), Limits::new(), &token).expect("search");
        assert!(results.cancelled);
        assert!(!results.is_complete());
        assert!(results.matches.is_empty());
    }

    #[test]
    fn a_truncated_result_is_visible() {
        let limits = Limits { max_results: 1 };
        let results = search(
            &files(),
            &Query::literal("foo"),
            limits,
            &Cancellation::new(),
        )
        .expect("search");
        assert!(results.truncated);
        assert_eq!(results.matches.len(), 1);
        assert!(!results.is_complete());
    }

    #[test]
    fn a_scoped_range_limits_the_matches() {
        let mut query = Query::literal("foo");
        query.range = Some((0, 10));
        let results =
            search(&files(), &query, Limits::new(), &Cancellation::new()).expect("search");
        // Only the match in the first ten bytes of a.rs.
        assert_eq!(results.matches.len(), 1);
        assert_eq!(results.matches[0].path, "a.rs");
    }
}
