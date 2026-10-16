// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Replace candidates: a diff per candidate, checkboxes, and a protection filter (task T08.04).
//!
//! The acceptance line is *"a read-only or generated region is not modified, and the replacement
//! syntax is checked first"*. Two orderings, so the module fixes both:
//!
//! * **The replacement is parsed before any candidate exists.** [`parse_template`] runs once, up
//!   front, and answers [`TemplateError`] for a bad `$` reference — so a malformed replacement is a
//!   reported error, never a half-built candidate list that fails later.
//! * **A candidate that would change a protected or read-only region is blocked, not built.** Every
//!   candidate is offered to the [T06.06](crate::project::text::protection) guard through
//!   [`WriteSource::Replace`]; a refusal becomes [`Candidate::blocked`] with the reason, and a blocked
//!   candidate is never checked, so it cannot be applied.
//! * **A candidate shows its own change.** [`Candidate::before`]/[`after`] are the line as it is and
//!   as it would become, and [`Candidate::segments`] is their character diff, so "what would this do"
//!   is answerable without applying anything.
//! * **Only checked candidates are a replace.** [`Candidates::checked`] is the set a commit would use,
//!   so the checkbox is the consent, not a decoration.

#![allow(dead_code)]

use std::collections::BTreeMap;

use crate::app::search::diff::{Options, Segment, inline};
use crate::app::search::text::{Query, search_text};
use crate::project::text::protection::{Write, WriteGuard, WriteSource};

/// One part of a replacement template.
#[derive(Clone, PartialEq, Eq, Debug)]
enum Part {
    /// Literal text.
    Literal(String),
    /// `$0`: the whole match.
    WholeMatch,
}

/// A parsed replacement template.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Template {
    parts: Vec<Part>,
    source: String,
}

/// Why a replacement template is invalid.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TemplateError {
    /// The character offset of the problem.
    pub at: usize,
    /// What is wrong.
    pub message: String,
}

impl TemplateError {
    /// The error in one sentence.
    pub fn message(&self) -> String {
        format!(
            "the replacement is invalid at character {}: {}",
            self.at, self.message
        )
    }
}

/// Parses a replacement template: literal text and `$0`, with `$$` for a literal `$`.
pub fn parse_template(text: &str) -> Result<Template, TemplateError> {
    let characters: Vec<char> = text.chars().collect();
    let mut parts = Vec::new();
    let mut literal = String::new();
    let mut index = 0;
    while index < characters.len() {
        let character = characters[index];
        if character == '$' {
            match characters.get(index + 1) {
                Some('0') => {
                    if !literal.is_empty() {
                        parts.push(Part::Literal(std::mem::take(&mut literal)));
                    }
                    parts.push(Part::WholeMatch);
                    index += 2;
                }
                Some('$') => {
                    literal.push('$');
                    index += 2;
                }
                Some(other) => {
                    return Err(TemplateError {
                        at: index,
                        message: format!("${other} is not a reference; only $0 and $$ are known"),
                    });
                }
                None => {
                    return Err(TemplateError {
                        at: index,
                        message: "a $ at the end refers to nothing".to_owned(),
                    });
                }
            }
        } else {
            literal.push(character);
            index += 1;
        }
    }
    if !literal.is_empty() {
        parts.push(Part::Literal(literal));
    }
    Ok(Template {
        parts,
        source: text.to_owned(),
    })
}

impl Template {
    /// The template's source text.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Renders the template for a matched text.
    pub fn render(&self, matched: &str) -> String {
        let mut out = String::new();
        for part in &self.parts {
            match part {
                Part::Literal(text) => out.push_str(text),
                Part::WholeMatch => out.push_str(matched),
            }
        }
        out
    }
}

/// Why a candidate is blocked.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BlockReason {
    /// The reason, in words.
    pub reason: String,
}

/// One replace candidate.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Candidate {
    /// The file.
    pub path: String,
    /// The zero-based line.
    pub line: usize,
    /// The matched span, as a byte range in the file.
    pub span: (usize, usize),
    /// The line as it is.
    pub before: String,
    /// The line as it would become.
    pub after: String,
    /// Whether the candidate is checked for a replace.
    pub checked: bool,
    /// Why it cannot be applied, when it cannot.
    pub blocked: Option<BlockReason>,
}

impl Candidate {
    /// The candidate in one line.
    pub fn render(&self) -> String {
        let state = match &self.blocked {
            Some(block) => format!("blocked: {}", block.reason),
            None => format!("checked={}", self.checked),
        };
        format!(
            "{}:{} {}..{} ({state})",
            self.path, self.line, self.span.0, self.span.1
        )
    }

    /// The character diff of the line before and after.
    pub fn segments(&self) -> Vec<Segment> {
        inline(&self.before, &self.after)
    }
}

/// The candidates of a replace.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Candidates {
    candidates: Vec<Candidate>,
}

impl Candidates {
    /// Builds candidates for a search and a template, blocking those the guards refuse.
    pub fn build(
        files: &[(String, String)],
        query: &Query,
        template: &Template,
        guards: &BTreeMap<String, WriteGuard>,
    ) -> Result<Self, crate::app::search::text::SearchError> {
        let mut candidates = Vec::new();
        for (path, text) in files {
            if query.range.is_some() {
                // A scoped search still works; the range is honoured by `search_text`.
            }
            let matches = search_text(path, text, query)?;
            for found in matches {
                let matched = &text[found.start..found.end];
                let replacement = template.render(matched);
                let before_line = found.text.clone();
                let after_line =
                    replace_in_line(&before_line, text, found.start, found.end, &replacement);
                let blocked = guards.get(path).and_then(|guard| {
                    guard
                        .check(
                            WriteSource::Replace,
                            &[Write::replace(
                                found.start,
                                found.end - found.start,
                                replacement.len(),
                            )],
                        )
                        .err()
                        .map(|refusal| BlockReason {
                            reason: refusal.message(),
                        })
                });
                candidates.push(Candidate {
                    path: path.clone(),
                    line: found.line,
                    span: (found.start, found.end),
                    before: before_line,
                    after: after_line,
                    checked: blocked.is_none(),
                    blocked,
                });
            }
        }
        Ok(Self { candidates })
    }

    /// The candidates.
    pub fn all(&self) -> &[Candidate] {
        &self.candidates
    }

    /// How many there are.
    pub fn len(&self) -> usize {
        self.candidates.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.candidates.is_empty()
    }

    /// The candidates that would be applied.
    pub fn checked(&self) -> Vec<&Candidate> {
        self.candidates.iter().filter(|c| c.checked).collect()
    }

    /// The blocked candidates.
    pub fn blocked(&self) -> Vec<&Candidate> {
        self.candidates
            .iter()
            .filter(|c| c.blocked.is_some())
            .collect()
    }

    /// Checks or unchecks one candidate. A blocked candidate cannot be checked.
    pub fn check(&mut self, index: usize, on: bool) -> Result<(), String> {
        let Some(candidate) = self.candidates.get_mut(index) else {
            return Err(format!("there is no candidate {index}"));
        };
        if on && candidate.blocked.is_some() {
            return Err("a blocked candidate cannot be checked".to_owned());
        }
        candidate.checked = on;
        Ok(())
    }

    /// Checks or unchecks the unblocked candidates.
    pub fn check_all(&mut self, on: bool) {
        for candidate in &mut self.candidates {
            if candidate.blocked.is_none() {
                candidate.checked = on;
            }
        }
    }

    /// The candidates in one report.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "{} candidate(s), {} checked, {} blocked",
            self.candidates.len(),
            self.checked().len(),
            self.blocked().len()
        )];
        for candidate in &self.candidates {
            lines.push(format!("  {}", candidate.render()));
            if candidate.blocked.is_none() {
                for segment in candidate.segments() {
                    lines.push(format!("    {}", segment.render()));
                }
            }
        }
        lines
    }
}

/// The line the candidate is on, with the matched span replaced.
fn replace_in_line(line: &str, file: &str, start: usize, end: usize, replacement: &str) -> String {
    // The match's byte offsets are absolute; shift them into the line.
    let line_start = line_offset(file, start);
    let local_start = (start - line_start).min(line.len());
    let local_end = (end - line_start).min(line.len());
    let mut out = String::with_capacity(line.len() + replacement.len());
    out.push_str(&line[..local_start]);
    out.push_str(replacement);
    out.push_str(&line[local_end..]);
    out
}

/// The byte offset a line containing an offset starts at.
fn line_offset(file: &str, offset: usize) -> usize {
    file[..offset.min(file.len())]
        .rfind('\n')
        .map_or(0, |index| index + 1)
}

/// The whole-line diff options a preview uses.
pub fn preview_options() -> Options {
    Options::strict()
}

#[cfg(test)]
mod tests {
    use super::{Candidates, parse_template};
    use crate::app::search::text::Query;
    use crate::project::document::conflict::Writability;
    use crate::project::text::protection::{ProtectedRegion, WriteGuard};
    use std::collections::BTreeMap;

    fn files() -> Vec<(String, String)> {
        vec![("a.rs".to_owned(), "let x = 1;\nlet y = 2;\n".to_owned())]
    }

    #[test]
    fn the_replacement_syntax_is_checked_first() {
        assert!(parse_template("replaced").is_ok());
        assert!(parse_template("[$0]").is_ok());
        assert!(parse_template("a$$b").is_ok());
        let error = parse_template("$1").expect_err("a bad reference");
        assert_eq!(error.at, 0);
        assert!(error.message().contains("not a reference"));
        assert!(parse_template("trailing$").is_err());
    }

    #[test]
    fn a_candidate_is_built_with_a_before_and_after() {
        let template = parse_template("N").expect("a template");
        let candidates =
            Candidates::build(&files(), &Query::literal("x"), &template, &BTreeMap::new())
                .expect("candidates");
        assert_eq!(candidates.len(), 1);
        let candidate = &candidates.all()[0];
        assert_eq!(candidate.before, "let x = 1;");
        assert_eq!(candidate.after, "let N = 1;");
        assert!(candidate.checked);
        // The diff shows what changed.
        assert!(
            candidate
                .segments()
                .iter()
                .any(|segment| matches!(segment, super::Segment::Remove(t) if t == "x"))
        );
    }

    #[test]
    fn a_protected_region_blocks_a_candidate() {
        let mut guards = BTreeMap::new();
        guards.insert(
            "a.rs".to_owned(),
            WriteGuard::new(vec![ProtectedRegion::new(4, 5)], Writability::Writable),
        );
        let template = parse_template("N").expect("a template");
        let mut candidates = Candidates::build(&files(), &Query::literal("x"), &template, &guards)
            .expect("candidates");
        // The match at offset 4 is inside the protected region.
        assert_eq!(candidates.blocked().len(), 1);
        assert!(!candidates.all()[0].checked);
        // A blocked candidate cannot be checked.
        assert!(candidates.check(0, true).is_err());
        assert!(candidates.checked().is_empty());
    }

    #[test]
    fn a_read_only_file_blocks_every_candidate() {
        let mut guards = BTreeMap::new();
        guards.insert(
            "a.rs".to_owned(),
            WriteGuard::new(
                Vec::new(),
                Writability::ReadOnly {
                    reason: "declared read-only".to_owned(),
                },
            ),
        );
        let template = parse_template("N").expect("a template");
        let candidates = Candidates::build(&files(), &Query::literal("l"), &template, &guards)
            .expect("candidates");
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates.blocked().len(), 2);
        assert!(candidates.checked().is_empty());
    }

    #[test]
    fn only_checked_candidates_are_a_replace() {
        let template = parse_template("N").expect("a template");
        let mut candidates =
            Candidates::build(&files(), &Query::literal("l"), &template, &BTreeMap::new())
                .expect("candidates");
        assert_eq!(candidates.checked().len(), 2);
        candidates.check_all(false);
        assert!(candidates.checked().is_empty());
        candidates.check(1, true).expect("not blocked");
        assert_eq!(candidates.checked().len(), 1);
        assert_eq!(candidates.checked()[0].line, 1);
    }

    #[test]
    fn the_whole_match_reference_reproduces_the_match() {
        let template = parse_template("<$0>").expect("a template");
        assert_eq!(template.render("x"), "<x>");
        let template = parse_template("$$").expect("a template");
        assert_eq!(template.render("x"), "$");
    }
}
