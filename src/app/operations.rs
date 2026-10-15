// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Batch operations on a selection: the preview, the confirmation, the history and the permission
//! explanation (task T05.07).
//!
//! The acceptance line is *"an inapplicable item in a multi-selection is not silently skipped"*. The
//! word that matters is **silently**, and the shape here is chosen so that silence is impossible:
//!
//! * **[`BatchPreview`] classifies every selected item before anything happens.** Each item is either
//!   applicable or carries the *reason* it is not, so the thing a person sees is not "5 processed" but
//!   "4 applied, 1 not applicable because …". An operation that would skip an item cannot do so
//!   without the reason being in the preview.
//! * **A skip is a decision, not a default.** [`Choice::ApplyAll`] with any inapplicable item is
//!   **refused**, naming them: to proceed one has to say [`Choice::ApplyNone`], which is where the
//!   skip becomes deliberate. This is the whole of the acceptance line — the only way past an
//!   inapplicable item is to acknowledge it.
//! * **The history records the skips too.** [`History`] keeps applied *and* skipped items per
//!   operation, so a later "why is this one missing" is answerable from the record rather than from
//!   memory.
//! * **The permission is explained before it is checked.** [`Operation::explain`] says which external
//!   effect the operation has and whether it is authorized, and [`BatchPreview::apply`] refuses when
//!   it is not — with that explanation, not with "denied". A person who has to grant an effect needs
//!   to know which one and why.

// The preview's accessors, the history's queries and the outcome's detail are read by the operation
// history panel and the confirmation dialog (T04/T26) as those land; until then the tests exercise
// them, and the silence is stated once here with its reason, as the neighbouring trees do.
#![allow(dead_code)]

use std::collections::BTreeSet;

use crate::contracts::permission::EffectClass;

/// What a batch operation is: its name, the external effects it has, and a note for the person.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Operation {
    name: String,
    effects: Vec<EffectClass>,
    note: String,
}

impl Operation {
    /// An operation with no external effects.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            effects: Vec::new(),
            note: String::new(),
        }
    }

    /// The same operation, declaring an external effect it has.
    pub fn with_effect(mut self, effect: EffectClass) -> Self {
        if !self.effects.contains(&effect) {
            self.effects.push(effect);
        }
        self
    }

    /// The same operation, with a note for the person.
    pub fn noted(mut self, note: impl Into<String>) -> Self {
        self.note = note.into();
        self
    }

    /// The operation's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Its external effects.
    pub fn effects(&self) -> &[EffectClass] {
        &self.effects
    }

    /// Which of its effects are not authorized, in the order it declares them.
    pub fn unauthorized(&self, authorized: &BTreeSet<EffectClass>) -> Vec<EffectClass> {
        self.effects
            .iter()
            .copied()
            .filter(|effect| !authorized.contains(effect))
            .collect()
    }

    /// The permission explanation: what this needs, and whether it has it.
    pub fn explain(&self, authorized: &BTreeSet<EffectClass>) -> String {
        let missing = self.unauthorized(authorized);
        let note = if self.note.is_empty() {
            String::new()
        } else {
            format!(" ({})", self.note)
        };
        if self.effects.is_empty() {
            return format!("{} changes nothing outside the document{note}", self.name);
        }
        let effects: Vec<&str> = self.effects.iter().map(|effect| effect.name()).collect();
        if missing.is_empty() {
            format!(
                "{} has the {} effect, which is authorized{note}",
                self.name,
                effects.join(", ")
            )
        } else {
            format!(
                "{} has the {} effect, and {} is not authorized{note}",
                self.name,
                effects.join(", "),
                missing
                    .iter()
                    .map(|effect| effect.name())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    }
}

/// Why one selected item cannot take part.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Skipped {
    id: String,
    reason: String,
}

impl Skipped {
    /// The item's id.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Why it cannot take part.
    pub fn reason(&self) -> &str {
        &self.reason
    }
}

/// Whether one selected item can take part.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Verdict {
    /// It can.
    Applies,
    /// It cannot, for a reason.
    Inapplicable {
        /// Why, in the words of whatever decided it.
        reason: String,
    },
}

impl Verdict {
    /// An item that cannot take part, with the reason.
    pub fn inapplicable(reason: impl Into<String>) -> Self {
        Self::Inapplicable {
            reason: reason.into(),
        }
    }
}

/// A selection and an operation, classified before anything happens.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BatchPreview {
    operation: Operation,
    items: Vec<(String, Verdict)>,
}

impl BatchPreview {
    /// Classifies a selection for an operation.
    pub fn of(operation: Operation, items: impl IntoIterator<Item = (String, Verdict)>) -> Self {
        Self {
            operation,
            items: items.into_iter().collect(),
        }
    }

    /// The operation.
    pub fn operation(&self) -> &Operation {
        &self.operation
    }

    /// The selected items, in order.
    pub fn items(&self) -> &[(String, Verdict)] {
        &self.items
    }

    /// The items that would take part.
    pub fn applies(&self) -> Vec<String> {
        self.items
            .iter()
            .filter(|(_, verdict)| matches!(verdict, Verdict::Applies))
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// The items that would not, with their reasons.
    pub fn blocked(&self) -> Vec<Skipped> {
        self.items
            .iter()
            .filter_map(|(id, verdict)| match verdict {
                Verdict::Applies => None,
                Verdict::Inapplicable { reason } => Some(Skipped {
                    id: id.clone(),
                    reason: reason.clone(),
                }),
            })
            .collect()
    }

    /// Whether any selected item cannot take part.
    pub fn needs_a_decision(&self) -> bool {
        !self.blocked().is_empty()
    }

    /// Whether every selected item can take part.
    pub fn is_clean(&self) -> bool {
        !self.needs_a_decision()
    }

    /// The preview in one report, with the permission explanation.
    pub fn render(&self, authorized: &BTreeSet<EffectClass>) -> Vec<String> {
        let mut lines = vec![format!(
            "{} on {} item(s): {} would apply, {} would not",
            self.operation.name(),
            self.items.len(),
            self.applies().len(),
            self.blocked().len()
        )];
        lines.push(format!("  {}", self.operation.explain(authorized)));
        for (id, verdict) in &self.items {
            match verdict {
                Verdict::Applies => lines.push(format!("  apply   {id}")),
                Verdict::Inapplicable { reason } => lines.push(format!("  skip    {id}: {reason}")),
            }
        }
        if self.needs_a_decision() {
            lines.push(
                "  an item that does not apply is not skipped silently: choose apply-none to skip it \
                 on purpose, or cancel"
                    .to_owned(),
            );
        }
        lines
    }

    /// Carries the operation out.
    ///
    /// The two refusals come first, in the order a person would want them: the permission (nothing
    /// happens without it, whatever was chosen), then the unacknowledged skip. `ApplyAll` over a
    /// selection with an inapplicable item is refused, which is what makes the skip a decision.
    pub fn apply(
        &self,
        choice: Choice,
        authorized: &BTreeSet<EffectClass>,
    ) -> Result<Outcome, Refusal> {
        if matches!(choice, Choice::Cancel) {
            return Ok(Outcome::Cancelled);
        }
        let missing = self.operation.unauthorized(authorized);
        if !missing.is_empty() {
            return Err(Refusal {
                reason: self.operation.explain(authorized),
            });
        }
        let blocked = self.blocked();
        match choice {
            Choice::ApplyAll if !blocked.is_empty() => Err(Refusal {
                reason: format!(
                    "{} selected item(s) do not apply ({}); apply-all would skip them silently, so \
                     either choose apply-none to skip them on purpose or cancel",
                    blocked.len(),
                    blocked
                        .iter()
                        .map(|skipped| format!("{}: {}", skipped.id(), skipped.reason()))
                        .collect::<Vec<_>>()
                        .join("; ")
                ),
            }),
            Choice::ApplyAll => Ok(Outcome::Done {
                applied: self.applies(),
                skipped: Vec::new(),
            }),
            Choice::ApplyNone => Ok(Outcome::Done {
                applied: self.applies(),
                skipped: blocked,
            }),
            Choice::Cancel => Ok(Outcome::Cancelled),
        }
    }
}

/// What to do with a preview.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Choice {
    /// Apply to every item, refusing if any does not apply.
    ApplyAll,
    /// Apply to the items that do, and skip the rest on purpose.
    ApplyNone,
    /// Do nothing.
    Cancel,
}

impl Choice {
    /// The name a command line uses.
    pub const fn name(self) -> &'static str {
        match self {
            Self::ApplyAll => "apply-all",
            Self::ApplyNone => "apply-none",
            Self::Cancel => "cancel",
        }
    }

    /// The choice a name asks for, when it is one.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "apply-all" => Some(Self::ApplyAll),
            "apply-none" => Some(Self::ApplyNone),
            "cancel" => Some(Self::Cancel),
            _ => None,
        }
    }
}

/// Why an operation did not happen.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Refusal {
    reason: String,
}

impl Refusal {
    /// Why.
    pub fn reason(&self) -> &str {
        &self.reason
    }

    /// The refusal in one sentence.
    pub fn message(&self) -> String {
        format!("{}; nothing was changed", self.reason)
    }
}

/// What an operation did.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Outcome {
    /// It ran: what it applied to, and what it skipped on purpose.
    Done {
        /// The items it applied to.
        applied: Vec<String>,
        /// The items it skipped, with the reasons.
        skipped: Vec<Skipped>,
    },
    /// Nothing happened.
    Cancelled,
}

impl Outcome {
    /// The outcome in one sentence.
    pub fn message(&self) -> String {
        match self {
            Self::Done { applied, skipped } => format!(
                "applied to {} item(s), skipped {} on purpose",
                applied.len(),
                skipped.len()
            ),
            Self::Cancelled => "nothing was changed".to_owned(),
        }
    }
}

/// One operation that happened, for the history.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct HistoryEntry {
    operation: String,
    applied: Vec<String>,
    skipped: Vec<Skipped>,
    at: u64,
}

impl HistoryEntry {
    /// The operation's name.
    pub fn operation(&self) -> &str {
        &self.operation
    }

    /// What it applied to.
    pub fn applied(&self) -> &[String] {
        &self.applied
    }

    /// What it skipped, with the reasons.
    pub fn skipped(&self) -> &[Skipped] {
        &self.skipped
    }

    /// When it happened, as nanoseconds since the Unix epoch.
    pub const fn at(&self) -> u64 {
        self.at
    }

    /// The entry in one line.
    pub fn render(&self) -> String {
        let skipped = if self.skipped.is_empty() {
            String::new()
        } else {
            format!(
                ", skipped {} ({})",
                self.skipped.len(),
                self.skipped
                    .iter()
                    .map(|skipped| format!("{}: {}", skipped.id(), skipped.reason()))
                    .collect::<Vec<_>>()
                    .join("; ")
            )
        };
        format!(
            "{} applied to {} item(s){skipped}",
            self.operation,
            self.applied.len()
        )
    }
}

/// The operations that have happened, newest first.
#[derive(Clone, Debug, Default)]
pub struct History {
    entries: Vec<HistoryEntry>,
    capacity: usize,
}

impl History {
    /// A history holding at most `capacity` entries (0 means unbounded).
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: Vec::new(),
            capacity,
        }
    }

    /// Records what an operation did, and returns whether it recorded anything.
    ///
    /// A cancelled operation is not an operation: recording it would make "what happened" include
    /// things that did not. A done operation is recorded even when it skipped something, because the
    /// skips are exactly what a later question is about.
    pub fn record(&mut self, operation: &str, outcome: &Outcome, at: u64) -> bool {
        let Outcome::Done { applied, skipped } = outcome else {
            return false;
        };
        self.entries.insert(
            0,
            HistoryEntry {
                operation: operation.to_owned(),
                applied: applied.clone(),
                skipped: skipped.clone(),
                at,
            },
        );
        if self.capacity > 0 {
            self.entries.truncate(self.capacity);
        }
        true
    }

    /// The entries, newest first.
    pub fn entries(&self) -> &[HistoryEntry] {
        &self.entries
    }

    /// How many entries there are.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Every item that was skipped for an operation, across the history.
    pub fn skips_of(&self, operation: &str) -> Vec<Skipped> {
        self.entries
            .iter()
            .filter(|entry| entry.operation == operation)
            .flat_map(|entry| entry.skipped.iter().cloned())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn effects(names: &[EffectClass]) -> BTreeSet<EffectClass> {
        names.iter().copied().collect()
    }

    fn operation() -> Operation {
        Operation::new("Delete")
            .with_effect(EffectClass::FileWrite)
            .noted("removes the file from disk")
    }

    fn preview() -> BatchPreview {
        BatchPreview::of(
            operation(),
            [
                ("pages/home.json".to_owned(), Verdict::Applies),
                (
                    "pages/root.json".to_owned(),
                    Verdict::inapplicable("the page root cannot be deleted"),
                ),
                ("styles/app.css".to_owned(), Verdict::Applies),
            ],
        )
    }

    #[test]
    fn the_preview_names_every_item_and_why_one_does_not_apply() {
        let preview = preview();
        assert_eq!(preview.applies(), ["pages/home.json", "styles/app.css"]);
        assert_eq!(preview.blocked().len(), 1);
        assert_eq!(preview.blocked()[0].id(), "pages/root.json");
        assert_eq!(
            preview.blocked()[0].reason(),
            "the page root cannot be deleted"
        );
        assert!(preview.needs_a_decision());
        assert!(!preview.is_clean());

        let report = preview.render(&effects(&[])).join("\n");
        assert!(report.contains("1 would not"), "{report}");
        assert!(report.contains("skip    pages/root.json: the page root cannot be deleted"));
        assert!(report.contains("not skipped silently"), "{report}");
    }

    #[test]
    fn applying_to_everything_with_an_inapplicable_item_is_refused_rather_than_skipping() {
        let preview = preview();
        let refusal = preview
            .apply(Choice::ApplyAll, &effects(&[EffectClass::FileWrite]))
            .expect_err("apply-all must not skip");
        assert!(
            refusal.reason().contains("pages/root.json"),
            "{}",
            refusal.reason()
        );
        assert!(
            refusal.reason().contains("skip them silently"),
            "{}",
            refusal.reason()
        );
        assert!(refusal.message().contains("nothing was changed"));
    }

    #[test]
    fn skipping_on_purpose_is_a_choice_and_the_outcome_records_it() {
        let preview = preview();
        let outcome = preview
            .apply(Choice::ApplyNone, &effects(&[EffectClass::FileWrite]))
            .expect("apply-none proceeds");
        match &outcome {
            Outcome::Done { applied, skipped } => {
                assert_eq!(applied.len(), 2);
                assert_eq!(skipped.len(), 1);
                assert_eq!(skipped[0].id(), "pages/root.json");
            }
            Outcome::Cancelled => panic!("expected it to run"),
        }
        assert!(outcome.message().contains("skipped 1 on purpose"));
    }

    #[test]
    fn a_clean_selection_applies_to_everything_without_a_second_question() {
        let clean = BatchPreview::of(
            Operation::new("Rename"),
            [
                ("a".to_owned(), Verdict::Applies),
                ("b".to_owned(), Verdict::Applies),
            ],
        );
        assert!(clean.is_clean());
        let outcome = clean.apply(Choice::ApplyAll, &effects(&[])).expect("runs");
        assert_eq!(
            outcome.message(),
            "applied to 2 item(s), skipped 0 on purpose"
        );
    }

    #[test]
    fn an_unauthorized_effect_is_refused_with_its_explanation() {
        let preview = preview();
        let refusal = preview
            .apply(Choice::ApplyNone, &effects(&[]))
            .expect_err("not authorized");
        assert!(
            refusal.reason().contains("file-write"),
            "{}",
            refusal.reason()
        );
        assert!(
            refusal.reason().contains("not authorized"),
            "{}",
            refusal.reason()
        );
        // The same explanation appears in the preview, so a person can see it before choosing.
        let report = preview.render(&effects(&[])).join("\n");
        assert!(report.contains("file-write"), "{report}");
        assert!(report.contains("not authorized"), "{report}");
    }

    #[test]
    fn cancelling_changes_nothing_and_is_not_recorded() {
        let preview = preview();
        let outcome = preview
            .apply(Choice::Cancel, &effects(&[EffectClass::FileWrite]))
            .expect("cancel is allowed without the effect");
        assert_eq!(outcome, Outcome::Cancelled);

        let mut history = History::new(10);
        assert!(
            !history.record("Delete", &outcome, 1),
            "a cancel is not an operation"
        );
        assert!(history.is_empty());
    }

    #[test]
    fn the_history_keeps_the_skips_so_a_later_question_is_answerable() {
        let preview = preview();
        let outcome = preview
            .apply(Choice::ApplyNone, &effects(&[EffectClass::FileWrite]))
            .expect("runs");
        let mut history = History::new(10);
        assert!(history.record("Delete", &outcome, 1_000));

        assert_eq!(history.len(), 1);
        let entry = &history.entries()[0];
        assert_eq!(entry.operation(), "Delete");
        assert_eq!(entry.applied().len(), 2);
        assert_eq!(entry.at(), 1_000);
        let skips = history.skips_of("Delete");
        assert_eq!(skips.len(), 1);
        assert_eq!(skips[0].id(), "pages/root.json");
        assert!(entry.render().contains("skipped 1"));
        assert!(history.skips_of("Rename").is_empty());
    }

    #[test]
    fn the_history_is_capped_and_newest_first() {
        let mut history = History::new(2);
        let outcome = Outcome::Done {
            applied: vec!["a".to_owned()],
            skipped: Vec::new(),
        };
        history.record("one", &outcome, 1);
        history.record("two", &outcome, 2);
        history.record("three", &outcome, 3);
        assert_eq!(history.len(), 2);
        assert_eq!(history.entries()[0].operation(), "three");
        assert_eq!(history.entries()[1].operation(), "two");
    }

    #[test]
    fn an_operation_with_no_effects_says_so_rather_than_looking_authorized() {
        let operation = Operation::new("Select All").noted("changes only the selection");
        let explanation = operation.explain(&effects(&[]));
        assert!(
            explanation.contains("changes nothing outside the document"),
            "{explanation}"
        );
        assert!(
            explanation.contains("changes only the selection"),
            "{explanation}"
        );
        assert!(operation.unauthorized(&effects(&[])).is_empty());
    }
}
