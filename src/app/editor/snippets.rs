// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Language snippets, placeholder tab-stops and surrounding selection (task T06.09).
//!
//! The acceptance line is *"a snippet that reaches across a protected region is refused; cancel,
//! multi-cursor and IME stay consistent with undo; and no snippet script runs"*. Every clause is a
//! property of the module's shape rather than a promise:
//!
//! * **A snippet is data, never a script.** [`Snippet::parse`] reads a body of literal runs and
//!   placeholders from a closed grammar — `$n`, `${n}`, `${n:default}`, and the escapes `\$` and
//!   `$$`. There is no command, no expression, no variable from the environment and no evaluation of
//!   any kind: a body that would need one is only ever literal text. "No snippet script runs" is
//!   therefore structural — there is no place a script could run.
//! * **Expansion is one transaction.** [`Session::start`] replaces every selection with the rendered
//!   body through one [`TextStore::edit_many`], so a snippet expanded at ten carets is one undo step,
//!   exactly like every edit of [T06.04](crate::app::editor::editing). Undo therefore takes the whole
//!   expansion back at once.
//! * **The guard runs before the edit.** The very [`Write`] list the expansion will apply is offered
//!   to [`WriteGuard::check`] first, so a snippet whose body or placeholders reach into a region a
//!   generator owns is refused **whole** — nothing is applied, and the refusal names the region.
//! * **A placeholder is a stop, and equal indices are one stop.** [`Session::jump_next`] walks the
//!   stops in order; ranges that share a placeholder index travel together, so typing into one writes
//!   them all (the linkage) as one transaction.
//! * **Cancel is not an edit.** [`Session::cancel`] ends the session where it stands and records no
//!   undo step; the expansion itself remains the one step it always was.
//!
//! Surrounding a selection ([`surround_selection`]) is the same all-or-nothing shape and reuses the
//! single wrap rule of [`crate::app::editor::editing::surround_edits`] rather than a second one, which is what
//! keeps auto-pairing and surrounding selection one concept rather than two.
//!
//! **Boundaries.** A multi-line body's own indentation is inserted verbatim; re-indenting its lines to
//! the insertion column is a display concern belonging with [T06.03](crate::app::editor::adapter) and is
//! not done here. Tab-stops are session state, not document state, so closing the document ends them.

#![allow(dead_code)]

use crate::app::editor::editing::{Carets, Editor, Selection, surround_edits};
use crate::project::text::piece_table::{TextError, TextStore};
use crate::project::text::protection::{Write, WriteGuard, WriteRefusal, WriteSource};

/// One piece of a parsed snippet body: a literal run, or a placeholder.
#[derive(Clone, PartialEq, Eq, Debug)]
enum Segment {
    /// Bytes copied into the document unchanged.
    Literal(String),
    /// A numbered hole a person fills in.
    Placeholder(Placeholder),
}

/// A placeholder: its index (`0` is the exit) and the text it stands for until it is filled.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Placeholder {
    index: u32,
    default: String,
}

/// Why a snippet body, or an expansion of one, was refused.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SnippetError {
    /// A snippet needs a trigger to be expanded by.
    EmptyPrefix,
    /// A `${` was opened and never closed.
    UnclosedPlaceholder {
        /// Where the `${` was.
        at: usize,
    },
    /// A `${` appeared inside another placeholder.
    NestedPlaceholder {
        /// Where the inner `${` was.
        at: usize,
    },
    /// A placeholder's index was not a number.
    BadIndex {
        /// Where the placeholder was.
        at: usize,
    },
    /// The expansion would write into a region a generator owns.
    Refused(WriteRefusal),
    /// The store refused the edit (for example a range that is not a character boundary).
    Text(TextError),
    /// The resulting carets would overlap, which cannot happen for a well-formed expansion.
    Overlap(String),
}

impl SnippetError {
    /// The one-line explanation a person is shown.
    pub fn message(&self) -> String {
        match self {
            Self::EmptyPrefix => "a snippet needs a non-empty trigger".to_owned(),
            Self::UnclosedPlaceholder { at } => {
                format!("the placeholder opening at {at} is never closed")
            }
            Self::NestedPlaceholder { at } => {
                format!("the placeholder opening at {at} is nested inside another")
            }
            Self::BadIndex { at } => format!("the placeholder at {at} has no index"),
            Self::Refused(refusal) => refusal.message(),
            Self::Text(error) => error.message(),
            Self::Overlap(reason) => reason.clone(),
        }
    }
}

/// A parsed snippet: the trigger that expands it, and its body as segments.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Snippet {
    prefix: String,
    segments: Vec<Segment>,
}

/// One stop of an expansion: the ranges a placeholder occupies, relative to the body start.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Stop {
    ranges: Vec<(usize, usize)>,
}

/// A rendered snippet body: the literal text, its stops in order, and the exit offset.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Expansion {
    text: String,
    stops: Vec<Vec<(usize, usize)>>,
    exit: usize,
}

impl Expansion {
    /// The literal text the snippet expands to.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The exit offset, where the caret ends up once every stop is filled.
    pub fn exit(&self) -> usize {
        self.exit
    }

    /// The stops, in order; each stop is the ranges that share a placeholder index.
    pub fn stops(&self) -> &[Vec<(usize, usize)>] {
        &self.stops
    }
}

impl Snippet {
    /// Reads a body of literals and placeholders. See the module docs for the grammar.
    pub fn parse(prefix: &str, body: &str) -> Result<Self, SnippetError> {
        if prefix.trim().is_empty() {
            return Err(SnippetError::EmptyPrefix);
        }
        let characters: Vec<(usize, char)> = body.char_indices().collect();
        let mut segments: Vec<Segment> = Vec::new();
        let mut literal = String::new();
        let mut index = 0usize;
        while index < characters.len() {
            let (at, character) = characters[index];
            match character {
                '\\' => match characters.get(index + 1).map(|(_, c)| *c) {
                    Some('$') => {
                        literal.push('$');
                        index += 2;
                    }
                    Some('\\') => {
                        literal.push('\\');
                        index += 2;
                    }
                    _ => {
                        literal.push('\\');
                        index += 1;
                    }
                },
                '$' if characters.get(index + 1).map(|(_, c)| *c) == Some('$') => {
                    literal.push('$');
                    index += 2;
                }
                '$' if characters.get(index + 1).map(|(_, c)| *c) == Some('{') => {
                    let open = at;
                    let mut close = index + 2;
                    let mut inner = String::new();
                    let mut closed = false;
                    while close < characters.len() {
                        let (inner_at, inner_char) = characters[close];
                        if inner_char == '}' {
                            closed = true;
                            break;
                        }
                        if inner_char == '$'
                            && characters.get(close + 1).map(|(_, c)| *c) == Some('{')
                        {
                            return Err(SnippetError::NestedPlaceholder { at: inner_at });
                        }
                        inner.push(inner_char);
                        close += 1;
                    }
                    if !closed {
                        return Err(SnippetError::UnclosedPlaceholder { at: open });
                    }
                    let placeholder = parse_placeholder(&inner, open)?;
                    push_placeholder(&mut segments, &mut literal, placeholder);
                    index = close + 1;
                }
                '$' => {
                    let mut next = index + 1;
                    let mut digits = String::new();
                    while next < characters.len() && characters[next].1.is_ascii_digit() {
                        digits.push(characters[next].1);
                        next += 1;
                    }
                    if digits.is_empty() {
                        literal.push('$');
                        index += 1;
                    } else {
                        let parsed: u32 =
                            digits.parse().map_err(|_| SnippetError::BadIndex { at })?;
                        push_placeholder(
                            &mut segments,
                            &mut literal,
                            Placeholder {
                                index: parsed,
                                default: String::new(),
                            },
                        );
                        index = next;
                    }
                }
                other => {
                    literal.push(other);
                    index += 1;
                }
            }
        }
        if !literal.is_empty() {
            segments.push(Segment::Literal(literal));
        }
        Ok(Self {
            prefix: prefix.to_owned(),
            segments,
        })
    }

    /// The trigger that expands this snippet.
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// Renders the body: the literal text, the ordered stops, and the exit offset.
    ///
    /// Offsets are relative to the body start; [`Session::start`] places them in the document.
    pub fn expand(&self) -> Expansion {
        let mut text = String::new();
        let mut order: Vec<u32> = Vec::new();
        let mut placements: Vec<(u32, (usize, usize))> = Vec::new();
        let mut exit: Option<usize> = None;
        for segment in &self.segments {
            match segment {
                Segment::Literal(run) => text.push_str(run),
                Segment::Placeholder(placeholder) => {
                    let start = text.len();
                    text.push_str(&placeholder.default);
                    let end = text.len();
                    if placeholder.index == 0 {
                        // `$0` is the exit: the caret ends up here once every stop is filled.
                        exit = Some(start);
                    } else {
                        if !order.contains(&placeholder.index) {
                            order.push(placeholder.index);
                        }
                        placements.push((placeholder.index, (start, end)));
                    }
                }
            }
        }
        let stops = order
            .iter()
            .map(|index| {
                placements
                    .iter()
                    .filter(|(placed, _)| placed == index)
                    .map(|(_, range)| *range)
                    .collect()
            })
            .collect();
        let exit = exit.unwrap_or(text.len());
        Expansion { text, stops, exit }
    }
}

/// Sends a placeholder into the segment list, flushing the literal run first.
fn push_placeholder(segments: &mut Vec<Segment>, literal: &mut String, placeholder: Placeholder) {
    if !literal.is_empty() {
        segments.push(Segment::Literal(std::mem::take(literal)));
    }
    segments.push(Segment::Placeholder(placeholder));
}

/// Reads the inside of a `${...}`: an index, or an index and a default after `:`.
fn parse_placeholder(inner: &str, at: usize) -> Result<Placeholder, SnippetError> {
    let (number, default) = match inner.split_once(':') {
        Some((number, default)) => (number, default.to_owned()),
        None => (inner, String::new()),
    };
    if number.is_empty() {
        return Err(SnippetError::BadIndex { at });
    }
    let index: u32 = number.parse().map_err(|_| SnippetError::BadIndex { at })?;
    Ok(Placeholder { index, default })
}

/// One expansion placed in the document: its stops and exit as absolute offsets.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Placed {
    stops: Vec<Vec<(usize, usize)>>,
    exit: usize,
}

/// A live snippet session: the expansions at their absolute offsets, and the current stop.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Session {
    placed: Vec<Placed>,
    step: usize,
    finished: bool,
}

impl Session {
    /// Expands a snippet at every caret as one transaction, and returns the live session.
    ///
    /// The expansion's own [`Write`]s are offered to the guard before anything is applied, so a
    /// snippet that would reach a protected region is refused whole.
    pub fn start(
        store: &mut TextStore,
        carets: &Carets,
        snippet: &Snippet,
        guard: &WriteGuard,
    ) -> Result<Self, SnippetError> {
        let expansion = snippet.expand();
        let mut edits = Vec::new();
        let mut writes = Vec::new();
        let mut bases = Vec::new();
        let mut delta: isize = 0;
        for selection in carets.selections() {
            let start = selection.start();
            let base = (start as isize + delta) as usize;
            edits.push((start, selection.len(), expansion.text.clone()));
            writes.push(Write::replace(start, selection.len(), expansion.text.len()));
            bases.push(base);
            delta += expansion.text.len() as isize - selection.len() as isize;
        }
        guard
            .check(WriteSource::Input, &writes)
            .map_err(SnippetError::Refused)?;
        store.edit_many(edits).map_err(SnippetError::Text)?;
        let placed = bases
            .iter()
            .map(|base| Placed {
                stops: expansion
                    .stops
                    .iter()
                    .map(|stop| {
                        stop.iter()
                            .map(|(start, end)| (base + start, base + end))
                            .collect()
                    })
                    .collect(),
                exit: base + expansion.exit,
            })
            .collect();
        Ok(Self {
            placed,
            step: 0,
            finished: false,
        })
    }

    /// How many stops the session has, not counting the exit.
    pub fn stops(&self) -> usize {
        self.placed.first().map_or(0, |placed| placed.stops.len())
    }

    /// Which stop is current; equal to [`Session::stops`] when the caret is at the exit.
    pub fn step(&self) -> usize {
        self.step
    }

    /// Whether the session has been cancelled or has run past its last stop.
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// The carets for the current stop: every linked range, or the exit caret at the end.
    pub fn carets(&self) -> Carets {
        let mut selections = Vec::new();
        for placed in &self.placed {
            match placed.stops.get(self.step) {
                Some(ranges) => {
                    for (start, end) in ranges {
                        selections.push(Selection::range(*start, *end));
                    }
                }
                None => selections.push(Selection::caret(placed.exit)),
            }
        }
        Carets::of(selections).expect("the stops of one snippet are disjoint and ordered")
    }

    /// Moves to the next stop, or to the exit; says whether it moved.
    pub fn jump_next(&mut self) -> bool {
        if self.step < self.stops() {
            self.step += 1;
            true
        } else {
            false
        }
    }

    /// Moves to the previous stop; says whether it moved.
    pub fn jump_previous(&mut self) -> bool {
        if self.step > 0 {
            self.step -= 1;
            true
        } else {
            false
        }
    }

    /// Ends the session where it stands. This is not an edit, so it records no undo step.
    pub fn cancel(&mut self) {
        self.finished = true;
    }

    /// Types `text` into the current stop, writing every linked range as one transaction.
    ///
    /// Returns whether the session handled it; at the exit there is nothing to mirror and the caller
    /// types normally. An IME commit is delivered through this same path, so it takes one undo step
    /// just like a keystroke.
    pub fn type_text(
        &mut self,
        store: &mut TextStore,
        guard: &WriteGuard,
        text: &str,
    ) -> Result<bool, SnippetError> {
        if self.finished || self.step >= self.stops() {
            return Ok(false);
        }
        let mut ranges = Vec::new();
        for placed in &self.placed {
            ranges.extend(placed.stops[self.step].iter().copied());
        }
        if ranges.is_empty() {
            return Ok(false);
        }
        let mut edits = Vec::new();
        let mut writes = Vec::new();
        for (start, end) in &ranges {
            edits.push((*start, end - start, text.to_owned()));
            writes.push(Write::replace(*start, end - start, text.len()));
        }
        guard
            .check(WriteSource::Input, &writes)
            .map_err(SnippetError::Refused)?;
        store.edit_many(edits.clone()).map_err(SnippetError::Text)?;

        // Rebuild the current stop from the edits, and shift every other range by the edits before
        // it. The edits are ascending because the stops are laid out in document order.
        let deltas: Vec<(usize, usize, usize)> = edits
            .iter()
            .map(|(at, remove, insert)| (*at, *remove, insert.len()))
            .collect();
        let new_current = place_edits(&deltas);
        let mut cursor = 0usize;
        for placed in &mut self.placed {
            let count = placed.stops[self.step].len();
            placed.stops[self.step] = new_current[cursor..cursor + count].to_vec();
            cursor += count;
            for (index, stop) in placed.stops.iter_mut().enumerate() {
                if index == self.step {
                    continue;
                }
                for range in stop.iter_mut() {
                    let (start, end) = (range.0, range.1);
                    range.0 = translate(start, &deltas);
                    range.1 = translate(end, &deltas);
                }
            }
            placed.exit = translate(placed.exit, &deltas);
        }
        Ok(true)
    }

    /// The session in one report: the current stop and where each expansion's stops are.
    pub fn render(&self) -> Vec<String> {
        let state = if self.finished { "finished" } else { "open" };
        let mut lines = vec![format!(
            "session: stop {} of {}, {state}",
            self.step,
            self.stops()
        )];
        for (index, placed) in self.placed.iter().enumerate() {
            lines.push(format!("  expansion {index}:"));
            for (stop, ranges) in placed.stops.iter().enumerate() {
                let marker = if stop == self.step { ">" } else { " " };
                let ranges = ranges
                    .iter()
                    .map(|(start, end)| format!("{start}..{end}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                lines.push(format!("   {marker} stop {stop}: {ranges}"));
            }
            lines.push(format!("     exit: {}", placed.exit));
        }
        lines
    }
}

/// Wraps every selection with `open` and `close`, refusing the whole write across a protected region.
pub fn surround_selection(
    store: &mut TextStore,
    editor: &mut Editor,
    open: char,
    close: char,
    guard: &WriteGuard,
) -> Result<(), SnippetError> {
    let text = store.text();
    let (edits, carets) = surround_edits(&text, editor.carets().selections(), open, close);
    let writes: Vec<Write> = edits
        .iter()
        .map(|(at, remove, insert)| Write::replace(*at, *remove, insert.len()))
        .collect();
    guard
        .check(WriteSource::Input, &writes)
        .map_err(SnippetError::Refused)?;
    store.edit_many(edits).map_err(SnippetError::Text)?;
    let text = store.text();
    editor
        .set_carets(&text, carets)
        .map_err(SnippetError::Overlap)
}

/// The new ranges for a set of ascending edits: each edit's inserted text, at its final offset.
fn place_edits(edits: &[(usize, usize, usize)]) -> Vec<(usize, usize)> {
    let mut delta: isize = 0;
    let mut out = Vec::with_capacity(edits.len());
    for (at, remove, insert) in edits {
        let start = (*at as isize + delta) as usize;
        out.push((start, start + insert));
        delta += *insert as isize - *remove as isize;
    }
    out
}

/// Where `position` lands once a set of ascending edits is applied.
fn translate(position: usize, edits: &[(usize, usize, usize)]) -> usize {
    let mut delta: isize = 0;
    for (at, remove, insert) in edits {
        if position <= *at {
            break;
        }
        if position >= at + remove {
            delta += *insert as isize - *remove as isize;
        }
    }
    (position as isize + delta) as usize
}

#[cfg(test)]
mod tests {
    use super::{Session, Snippet, SnippetError, surround_selection};
    use crate::app::editor::editing::{Carets, Editor, Selection};
    use crate::project::document::conflict::Writability;
    use crate::project::text::piece_table::TextStore;
    use crate::project::text::protection::{ProtectedRegion, WriteGuard};

    fn open() -> WriteGuard {
        WriteGuard::open()
    }

    fn guarded() -> WriteGuard {
        WriteGuard::new(vec![ProtectedRegion::new(4, 10)], Writability::Writable)
    }

    #[test]
    fn a_body_parses_into_literals_and_placeholders() {
        let snippet = Snippet::parse("fn", "fn ${1:name}(${2}) {$0}").expect("parses");
        assert_eq!(snippet.prefix(), "fn");
        let expansion = snippet.expand();
        assert_eq!(expansion.text, "fn name() {}");
        assert_eq!(expansion.stops.len(), 2);
        // Stop 0 is the first `$1`, stop 1 the `$2`; the `$0` is the exit before the final `}`.
        assert_eq!(expansion.stops[0], vec![(3, 7)]);
        assert_eq!(expansion.stops[1], vec![(8, 8)]);
        assert_eq!(expansion.exit, 11);
    }

    #[test]
    fn equal_indices_share_one_stop_and_link() {
        let snippet = Snippet::parse("eq", "$1 = $1; $2").expect("parses");
        let expansion = snippet.expand();
        assert_eq!(expansion.text, " = ; ");
        assert_eq!(expansion.stops.len(), 2);
        assert_eq!(expansion.stops[0], vec![(0, 0), (3, 3)]);
        assert_eq!(expansion.stops[1], vec![(5, 5)]);
        assert_eq!(expansion.exit, 5);
    }

    #[test]
    fn the_escapes_are_literal_and_a_dollar_survives() {
        let snippet = Snippet::parse("lit", "a\\$b $$ c").expect("parses");
        let expansion = snippet.expand();
        assert_eq!(expansion.text, "a$b $ c");
        assert!(expansion.stops.is_empty());
    }

    #[test]
    fn a_nested_or_unclosed_placeholder_is_refused() {
        assert_eq!(
            Snippet::parse("bad", "${1:${2}}"),
            Err(SnippetError::NestedPlaceholder { at: 4 })
        );
        assert_eq!(
            Snippet::parse("bad", "${1"),
            Err(SnippetError::UnclosedPlaceholder { at: 0 })
        );
        assert_eq!(
            Snippet::parse("bad", "${}"),
            Err(SnippetError::BadIndex { at: 0 })
        );
        assert_eq!(Snippet::parse("  ", "x"), Err(SnippetError::EmptyPrefix));
    }

    #[test]
    fn expanding_replaces_every_selection_as_one_undo_step() {
        let mut store = TextStore::new("one\ntwo\n");
        let snippet = Snippet::parse("x", "[$1]").expect("parses");
        // A caret at the start of each line.
        let carets = Carets::of(vec![Selection::caret(0), Selection::caret(4)]).expect("disjoint");
        let session = Session::start(&mut store, &carets, &snippet, &open()).expect("expands");
        assert_eq!(store.text(), "[]one\n[]two\n");
        assert_eq!(session.stops(), 1);
        // One undo takes the whole expansion back.
        assert!(store.undo().expect("undo"));
        assert_eq!(store.text(), "one\ntwo\n");
    }

    #[test]
    fn tab_jumps_the_stops_in_order_and_back() {
        let mut store = TextStore::new("");
        let snippet = Snippet::parse("fn", "fn ${1:name}(${2}) {$0}").expect("parses");
        let carets = Carets::caret(0);
        let mut session = Session::start(&mut store, &carets, &snippet, &open()).expect("expands");
        assert_eq!(session.step(), 0);
        assert_eq!(session.carets().primary(), Selection::range(3, 7));
        assert!(session.jump_next());
        assert_eq!(session.carets().primary(), Selection::range(8, 8));
        assert!(session.jump_next());
        // Past the last stop: the caret is at the exit, before the final `}`.
        assert_eq!(session.carets().primary(), Selection::caret(11));
        assert!(!session.jump_next());
        assert!(session.jump_previous());
        assert_eq!(session.carets().primary(), Selection::range(8, 8));
    }

    #[test]
    fn typing_into_a_placeholder_writes_every_linked_copy_in_one_step() {
        let mut store = TextStore::new("");
        let snippet = Snippet::parse("eq", "$1 = $1; $2").expect("parses");
        let carets = Carets::caret(0);
        let mut session = Session::start(&mut store, &carets, &snippet, &open()).expect("expands");
        assert_eq!(store.text(), " = ; ");
        assert!(session.type_text(&mut store, &open(), "x").expect("types"));
        assert_eq!(store.text(), "x = x; ");
        // The step boundary shifts with the edit: the `$2` stop moved past both `x`s.
        assert!(session.jump_next());
        assert_eq!(session.carets().primary(), Selection::caret(7));
        // The whole typed value is one undo step.
        assert!(store.undo().expect("undo"));
        assert_eq!(store.text(), " = ; ");
    }

    #[test]
    fn an_ime_composition_fills_every_linked_placeholder_in_one_step() {
        let mut store = TextStore::new("");
        let snippet = Snippet::parse("eq", "$1=$1").expect("parses");
        let mut session =
            Session::start(&mut store, &Carets::caret(0), &snippet, &open()).expect("expands");
        assert_eq!(store.text(), "=");
        // A multi-byte composition goes into both linked copies whole, as one transaction.
        assert!(
            session
                .type_text(&mut store, &open(), "你好")
                .expect("types")
        );
        assert_eq!(store.text(), "你好=你好");
        assert!(store.undo().expect("undo"));
        assert_eq!(store.text(), "=");
    }

    #[test]
    fn cancel_ends_the_session_without_recording_an_undo_step() {
        let mut store = TextStore::new("x");
        let snippet = Snippet::parse("s", "a${1}b").expect("parses");
        let carets = Carets::caret(1);
        let mut session = Session::start(&mut store, &carets, &snippet, &open()).expect("expands");
        assert_eq!(store.text(), "xab");
        session.cancel();
        assert!(session.is_finished());
        // Typing after a cancel is refused by the session and never reaches the store.
        assert!(!session.type_text(&mut store, &open(), "z").expect("types"));
        assert_eq!(store.text(), "xab");
        // One undo takes the expansion back; the cancel added nothing to take back.
        assert!(store.undo().expect("undo"));
        assert_eq!(store.text(), "x");
    }

    #[test]
    fn a_snippet_that_reaches_a_protected_region_is_refused_whole() {
        let mut store = TextStore::new("0123456789");
        let guard = guarded();
        let snippet = Snippet::parse("p", "P").expect("parses");
        // A caret inside the protected region 4..10 is refused.
        let carets = Carets::caret(6);
        match Session::start(&mut store, &carets, &snippet, &guard) {
            Err(SnippetError::Refused(refusal)) => {
                assert!(refusal.message().contains("protected region 4..10"));
            }
            other => panic!("the snippet was not refused: {other:?}"),
        }
        assert_eq!(store.text(), "0123456789");
        // A caret outside the region expands as normal.
        let carets = Carets::caret(0);
        assert!(Session::start(&mut store, &carets, &snippet, &guard).is_ok());
        assert_eq!(store.text(), "P0123456789");
    }

    #[test]
    fn one_refused_caret_refuses_the_whole_multi_cursor_expansion() {
        let mut store = TextStore::new("0123456789");
        let guard = guarded();
        let snippet = Snippet::parse("p", "P").expect("parses");
        // One caret outside the region, one inside: nothing is applied.
        let carets = Carets::of(vec![Selection::caret(0), Selection::caret(6)]).expect("disjoint");
        assert!(Session::start(&mut store, &carets, &snippet, &guard).is_err());
        assert_eq!(store.text(), "0123456789");
    }

    #[test]
    fn surrounding_a_selection_wraps_it_and_refuses_across_a_protected_region() {
        let mut store = TextStore::new("0123456789");
        let guard = guarded();
        let mut editor = Editor::new();
        // Wrapping 0..2 writes before the region: allowed.
        editor
            .set_carets(
                &store.text(),
                Carets::of(vec![Selection::range(0, 2)]).expect("disjoint"),
            )
            .expect("snaps");
        surround_selection(&mut store, &mut editor, '(', ')', &guard).expect("wraps");
        assert_eq!(store.text(), "(01)23456789");
        // The caret sits between the wrapped text and the closer.
        assert_eq!(editor.carets().primary(), Selection::caret(3));

        // Wrapping 3..8 reaches into the region 4..10: refused whole.
        let before = store.text();
        editor
            .set_carets(
                &before,
                Carets::of(vec![Selection::range(3, 8)]).expect("disjoint"),
            )
            .expect("snaps");
        match surround_selection(&mut store, &mut editor, '(', ')', &guard) {
            Err(SnippetError::Refused(refusal)) => {
                assert!(refusal.message().contains("protected region"));
            }
            other => panic!("the surround was not refused: {other:?}"),
        }
        assert_eq!(store.text(), before);
    }
}
