// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Reading a generated file back into a model change (task T15.05).
//!
//! The acceptance line is *"an unrecognised change keeps the text and can be cancelled"*. The
//! reverse path is deliberately small, because arbitrary Rust cannot be turned into a design: it
//! reads only the constructs a generated file spells in a fixed way — a node's `.key`, its kind, a
//! `.prop("name", value)` assignment, a `.child(…)` edge — and treats everything else as a person's.
//!
//! * **The support table is explicit.** [`SUPPORT`] names each construct and whether it is readable
//!   back; a reader that meets something outside it does not guess (principle 29: constraints at the
//!   type layer, not in a comment).
//! * **A change that is not understood keeps its text.** [`Candidate::unrecognised`] holds the lines
//!   a person edited that the reader could not map, alongside the full [`Candidate::text`], so the
//!   caller can show the diff and refuse — nothing is dropped (principle 19).
//! * **Three-way is a view, not a decision.** [`conflicts`] compares base, mine and theirs and names
//!   the three values for each property they disagree on; it never picks a side.

#![allow(dead_code)]

use std::collections::BTreeMap;

/// One construct the reverse reader knows about, and whether it reads it back.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Support {
    /// The spelling in the generated file.
    pub construct: &'static str,
    /// Whether a change to it can become a model change.
    pub readable: bool,
    /// What it means, for the support table a reader is shown.
    pub meaning: &'static str,
}

/// The constructs a generated file uses, and which of them read back.
pub const SUPPORT: &[Support] = &[
    Support {
        construct: ".key(\"…\")",
        readable: true,
        meaning: "a node's identity; a change here is a change of which node",
    },
    Support {
        construct: "Node::new(\"kind\") / create_*",
        readable: true,
        meaning: "a node's kind",
    },
    Support {
        construct: ".prop(\"name\", value)",
        readable: true,
        meaning: "a property value",
    },
    Support {
        construct: ".child(…)",
        readable: true,
        meaning: "a parent-child edge (reparenting)",
    },
    Support {
        construct: "fn body statements",
        readable: false,
        meaning: "business logic: hand-written, kept as text",
    },
    Support {
        construct: "#[cfg(…)] / attributes",
        readable: false,
        meaning: "build configuration: hand-written, kept as text",
    },
];

/// The support entry for a construct key, if the table names one.
pub fn support_for(construct: &str) -> Option<&'static Support> {
    SUPPORT.iter().find(|entry| entry.construct == construct)
}

/// One property the reader mapped back to a node.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Change {
    /// The node's key.
    pub node: String,
    /// The property name.
    pub property: String,
    /// The value the edited text now holds.
    pub value: String,
}

impl Change {
    /// The change in one line.
    pub fn render(&self) -> String {
        format!("{} .prop({:?}, {:?})", self.node, self.property, self.value)
    }
}

/// What reading a generated file back produced.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Candidate {
    /// The property changes the reader understood.
    pub recognized: Vec<Change>,
    /// The lines the reader could not map back to the model.
    pub unrecognised: Vec<String>,
    /// The whole edited text, kept so a cancelled candidate loses nothing.
    pub text: String,
}

impl Candidate {
    /// Cancels: the caller keeps the edited text and applies no model change.
    pub fn cancel(self) -> String {
        self.text
    }

    /// Whether the reader understood everything that changed.
    pub fn is_clean(&self) -> bool {
        self.unrecognised.is_empty()
    }

    /// The candidate in one line per fact.
    pub fn render(&self) -> Vec<String> {
        let mut out: Vec<String> = self.recognized.iter().map(Change::render).collect();
        for line in &self.unrecognised {
            out.push(format!("kept (unrecognised): {line}"));
        }
        if out.is_empty() {
            out.push("no readable change".to_owned());
        }
        out
    }
}

/// One property base, mine and theirs disagree on.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Conflict {
    /// The node's key.
    pub node: String,
    /// The property name.
    pub property: String,
    /// The value the base held.
    pub base: String,
    /// The value this side holds.
    pub mine: String,
    /// The value the other side holds.
    pub theirs: String,
}

impl Conflict {
    /// The conflict in one line.
    pub fn render(&self) -> String {
        format!(
            "{} .prop({:?}): base {:?}, mine {:?}, theirs {:?}",
            self.node, self.property, self.base, self.mine, self.theirs
        )
    }
}

/// Reads `edited` against `previous`, mapping the constructs it understands.
///
/// `previous` is the generated text the model last agreed with; `edited` is what is on disk now.
pub fn read(previous: &str, edited: &str) -> Candidate {
    let old = properties(previous);
    let new = properties(edited);
    let mut recognized = Vec::new();
    for (key, value) in &new {
        match old.get(key) {
            Some(previous_value) if previous_value == value => {}
            _ => recognized.push(Change {
                node: key.0.clone(),
                property: key.1.clone(),
                value: value.clone(),
            }),
        }
    }
    let unrecognised = unrecognised_lines(previous, edited);
    Candidate {
        recognized,
        unrecognised,
        text: edited.to_owned(),
    }
}

/// The three-way view: every property base, mine and theirs do not all agree on.
pub fn conflicts(base: &str, mine: &str, theirs: &str) -> Vec<Conflict> {
    let base = properties(base);
    let mine = properties(mine);
    let theirs = properties(theirs);
    let mut out = Vec::new();
    for key in mine.keys() {
        let b = base.get(key).cloned().unwrap_or_default();
        let m = mine.get(key).cloned().unwrap_or_default();
        let t = theirs.get(key).cloned().unwrap_or_default();
        if m != t {
            out.push(Conflict {
                node: key.0.clone(),
                property: key.1.clone(),
                base: b,
                mine: m,
                theirs: t,
            });
        }
    }
    out
}

/// Every `.prop("name", value)` in a text, keyed by the node's most recent `.key` and the name.
fn properties(text: &str) -> BTreeMap<(String, String), String> {
    let mut key = String::from("root");
    let mut out = BTreeMap::new();
    for line in text.lines() {
        // A line may carry both a `.key` and a `.prop` (the generator writes them chained), so the
        // key is updated first and the property scan then runs over the whole line rather than being
        // skipped by a `continue`.
        if let Some(found) = literal(line, ".key(") {
            key = found;
        }
        let mut rest = line;
        while let Some(at) = rest.find(".prop(") {
            let after = &rest[at + ".prop(".len()..];
            if let Some((name, value)) = split_prop(after) {
                out.insert((key.clone(), name), value);
            }
            rest = after;
        }
    }
    out
}

/// Lines that are neither a readable construct nor present in the previous text.
fn unrecognised_lines(previous: &str, edited: &str) -> Vec<String> {
    let known: Vec<&str> = previous.lines().collect();
    edited
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return false;
            }
            if trimmed.contains(".key(") || trimmed.contains(".prop(") {
                return false;
            }
            !known.contains(line)
        })
        .map(|line| line.to_owned())
        .collect()
}

/// The first quoted literal after `opener` on a line.
fn literal(line: &str, opener: &str) -> Option<String> {
    let rest = after(line, opener)?;
    first_string(rest)
}

/// The text after `needle` on the line, if it is there.
fn after<'a>(line: &'a str, needle: &str) -> Option<&'a str> {
    line.find(needle).map(|at| &line[at + needle.len()..])
}

/// The first `"…"` in `text`.
fn first_string(text: &str) -> Option<String> {
    let start = text.find('"')? + 1;
    let end = text[start..].find('"')? + start;
    Some(text[start..end].to_owned())
}

/// Splits `"name", value)` into its name and value.
fn split_prop(rest: &str) -> Option<(String, String)> {
    let name = first_string(rest)?;
    // The comma after the name's closing quote.
    let quote = rest.find('"')?;
    let after_name = rest[quote + 1..].find('"')? + quote + 1;
    let comma = rest[after_name..].find(',')? + after_name;
    Some((name, value_of(&rest[comma + 1..])))
}

/// The value between a comma and the call's closing paren, unwrapping string quotes.
///
/// The scan tracks whether it is inside a string so a `)` inside a quoted value does not end the
/// value early; the paren that closes the call is the first one outside quotes.
fn value_of(tail: &str) -> String {
    let mut in_string = false;
    for (index, character) in tail.char_indices() {
        match character {
            '"' => in_string = !in_string,
            ')' if !in_string => {
                return tail[..index].trim().trim_matches('"').to_owned();
            }
            _ => {}
        }
    }
    tail.trim()
        .trim_end_matches(')')
        .trim()
        .trim_matches('"')
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::{SUPPORT, conflicts, read, support_for};

    const BASE: &str = "let t = Node::new(\"window\").key(\"root\")\n\
        \x20   .child(Node::new(\"button\").key(\"n_0\").prop(\"text\", \"Go\"));\n";

    #[test]
    fn the_support_table_says_what_reads_back() {
        assert!(support_for(".prop(\"name\", value)").unwrap().readable);
        assert!(!support_for("fn body statements").unwrap().readable);
        assert!(support_for("definitely_not_a_construct").is_none());
        assert!(SUPPORT.iter().any(|entry| entry.construct == ".child(…)"));
    }

    #[test]
    fn a_changed_property_value_reads_back_as_one_change() {
        let edited = BASE.replace("\"Go\"", "\"Stop\"");
        let candidate = read(BASE, &edited);
        assert_eq!(candidate.recognized.len(), 1);
        assert_eq!(candidate.recognized[0].node, "n_0");
        assert_eq!(candidate.recognized[0].property, "text");
        assert_eq!(candidate.recognized[0].value, "Stop");
        assert!(candidate.is_clean(), "{:?}", candidate.render());
    }

    #[test]
    fn an_unrecognised_change_keeps_its_text_and_can_be_cancelled() {
        let edited = format!("{BASE}fn mine() {{ /* hand-written */ }}\n");
        let candidate = read(BASE, &edited);
        assert!(candidate.recognized.is_empty());
        assert!(
            candidate
                .unrecognised
                .iter()
                .any(|line| line.contains("mine"))
        );
        let kept = candidate.cancel();
        assert!(kept.contains("fn mine()"));
    }

    #[test]
    fn a_three_way_view_names_all_three_values_and_picks_none() {
        let mine = BASE.replace("\"Go\"", "\"Mine\"");
        let theirs = BASE.replace("\"Go\"", "\"Theirs\"");
        let view = conflicts(BASE, &mine, &theirs);
        assert_eq!(view.len(), 1);
        assert_eq!(view[0].node, "n_0");
        assert_eq!(view[0].base, "Go");
        assert_eq!(view[0].mine, "Mine");
        assert_eq!(view[0].theirs, "Theirs");
        assert!(view[0].render().contains("mine \"Mine\""));
    }

    #[test]
    fn an_unchanged_property_is_not_a_conflict() {
        assert!(conflicts(BASE, BASE, BASE).is_empty());
    }
}
