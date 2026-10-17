// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Edit delegates: how a cell is displayed, how it is edited, and what happens on commit or cancel
//! (task T27.04).
//!
//! The acceptance line is *"a validation failure does not commit; cancelling triggers no save side
//! effect"*. An edit is a short-lived thing with exactly four ends, and the design fixes what each one
//! is allowed to do:
//!
//! * **Display and editor are different.** [`DelegateKind::display`] is the read-only cell;
//!   [`DelegateKind::editor`] is what the cell shows while it is being edited. A cell is never edited
//!   in place of its own display by accident, because the two are separate calls.
//! * **A refusal does not commit.** [`Edit::commit`] validates first. A value that fails leaves the
//!   edit [`EditState::Rejected`] with the reason, changes nothing, and records no [`Save`].
//! * **A cancel has no side effect.** [`Edit::cancel`] settles the edit as [`EditState::Cancelled`]
//!   and touches the [`Saves`] log not at all, so abandoning an edit cannot quietly persist it.
//! * **A save is the only side effect.** Only a validated commit records one [`Save`], which is the
//!   single place a caller can point at and say "this is what the edit did".

#![allow(dead_code)]

/// The kind of editor a column's cells use.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum DelegateKind {
    /// Free text.
    Text,
    /// A number.
    Number,
    /// A flag.
    Check,
    /// A choice from a named set; an empty set accepts any text.
    Choice(Vec<String>),
}

impl DelegateKind {
    /// The delegate's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Number => "number",
            Self::Check => "check",
            Self::Choice(_) => "choice",
        }
    }

    /// Reads a delegate's name. A `choice` starts with no allowed values.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "text" => Some(Self::Text),
            "number" => Some(Self::Number),
            "check" => Some(Self::Check),
            "choice" => Some(Self::Choice(Vec::new())),
            _ => None,
        }
    }

    /// How the value is shown when the cell is not being edited.
    pub fn display(&self, value: &str) -> String {
        match self {
            Self::Text | Self::Number | Self::Choice(_) => value.to_owned(),
            Self::Check => match value {
                "true" => "☑".to_owned(),
                _ => "☐".to_owned(),
            },
        }
    }

    /// What the cell shows while it is being edited.
    pub fn editor(&self, value: &str) -> String {
        match self {
            Self::Text => format!("[text: {value}]"),
            Self::Number => format!("[number: {value}]"),
            Self::Check => format!("[check: {value}]"),
            Self::Choice(allowed) => {
                let set = if allowed.is_empty() {
                    "any".to_owned()
                } else {
                    allowed.join(", ")
                };
                format!("[choice of {set}: {value}]")
            }
        }
    }

    /// Checks a candidate, answering the value to commit or why it is refused.
    pub fn validate(&self, candidate: &str) -> Result<String, String> {
        match self {
            Self::Text => Ok(candidate.trim().to_owned()),
            Self::Number => candidate
                .trim()
                .parse::<f64>()
                .map(|_| candidate.trim().to_owned())
                .map_err(|_| format!("{candidate:?} is not a number")),
            Self::Check => match candidate.trim() {
                "true" | "yes" | "1" => Ok("true".to_owned()),
                "false" | "no" | "0" => Ok("false".to_owned()),
                other => Err(format!("{other:?} is not a flag")),
            },
            Self::Choice(allowed) => {
                let candidate = candidate.trim();
                if allowed.is_empty() || allowed.iter().any(|choice| choice == candidate) {
                    Ok(candidate.to_owned())
                } else {
                    Err(format!(
                        "{candidate:?} is not one of {}",
                        allowed.join(", ")
                    ))
                }
            }
        }
    }
}

/// One recorded save: the side effect a committed edit had.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Save {
    /// The row the edit was on.
    pub row: usize,
    /// The column the edit was on.
    pub column: String,
    /// The value that was committed.
    pub value: String,
}

/// The log of saves a session has performed.
///
/// A cancel records nothing here, which is how "cancelling triggers no save side effect" is made an
/// observable fact rather than a promise.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Saves {
    /// The saves, in the order they happened.
    pub log: Vec<Save>,
}

impl Saves {
    /// An empty log.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a save.
    pub fn record(&mut self, row: usize, column: &str, value: &str) {
        self.log.push(Save {
            row,
            column: column.to_owned(),
            value: value.to_owned(),
        });
    }

    /// How many saves were recorded.
    pub fn len(&self) -> usize {
        self.log.len()
    }

    /// Whether nothing was saved.
    pub fn is_empty(&self) -> bool {
        self.log.is_empty()
    }
}

/// Where an edit is.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum EditState {
    /// The edit is open and nothing has been decided.
    Editing,
    /// The candidate validated and was committed, with the value that was saved.
    Committed(String),
    /// The candidate failed validation, with the reason.
    Rejected(String),
    /// The edit was abandoned; nothing was saved.
    Cancelled,
}

impl EditState {
    /// The state's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Editing => "editing",
            Self::Committed(_) => "committed",
            Self::Rejected(_) => "rejected",
            Self::Cancelled => "cancelled",
        }
    }

    /// Whether the edit committed.
    pub fn is_committed(&self) -> bool {
        matches!(self, Self::Committed(_))
    }
}

/// One cell edit: the value it started from, the candidate being typed, and where it ended.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Edit {
    /// The row the edit is on.
    pub row: usize,
    /// The column the edit is on.
    pub column: String,
    /// The value the cell held before the edit.
    pub original: String,
    /// The value being typed.
    pub candidate: String,
    /// Where the edit is.
    pub state: EditState,
}

impl Edit {
    /// Opens an edit on a cell that currently holds `current`.
    pub fn begin(row: usize, column: &str, current: &str) -> Self {
        Self {
            row,
            column: column.to_owned(),
            original: current.to_owned(),
            candidate: current.to_owned(),
            state: EditState::Editing,
        }
    }

    /// Replaces the candidate.
    pub fn set(&mut self, candidate: &str) {
        self.candidate = candidate.to_owned();
    }

    /// Commits the candidate.
    ///
    /// The candidate is validated first: a value that does not fit leaves the edit
    /// [`EditState::Rejected`] and records no save, so a refused edit cannot look like one that
    /// happened. Only a validated value becomes [`EditState::Committed`] and records one [`Save`].
    pub fn commit(&mut self, kind: &DelegateKind, saves: &mut Saves) -> bool {
        match kind.validate(&self.candidate) {
            Ok(value) => {
                self.state = EditState::Committed(value.clone());
                saves.record(self.row, &self.column, &value);
                true
            }
            Err(reason) => {
                self.state = EditState::Rejected(reason);
                false
            }
        }
    }

    /// Cancels the edit. This performs no save side effect at all.
    pub fn cancel(&mut self) {
        self.state = EditState::Cancelled;
    }

    /// The committed value, when the edit committed.
    pub fn value(&self) -> Option<&str> {
        match &self.state {
            EditState::Committed(value) => Some(value),
            _ => None,
        }
    }

    /// Why the edit was refused, when it was.
    pub fn error(&self) -> Option<&str> {
        match &self.state {
            EditState::Rejected(reason) => Some(reason),
            _ => None,
        }
    }

    /// The edit in one line, including its error state.
    pub fn render(&self) -> String {
        match &self.state {
            EditState::Editing => {
                format!("{}:{} editing {:?}", self.row, self.column, self.candidate)
            }
            EditState::Committed(value) => {
                format!("{}:{} committed {value:?}", self.row, self.column)
            }
            EditState::Rejected(reason) => {
                format!("{}:{} refused: {reason}", self.row, self.column)
            }
            EditState::Cancelled => {
                format!("{}:{} cancelled (nothing saved)", self.row, self.column)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DelegateKind, Edit, EditState, Save, Saves};

    #[test]
    fn a_failed_validation_does_not_commit_and_records_no_save() {
        let mut edit = Edit::begin(2, "count", "1");
        edit.set("not a number");
        let mut saves = Saves::new();
        assert!(!edit.commit(&DelegateKind::Number, &mut saves));
        assert_eq!(edit.state.name(), "rejected");
        assert!(edit.error().is_some());
        assert!(saves.is_empty());
        assert_eq!(edit.value(), None);
    }

    #[test]
    fn a_valid_candidate_commits_and_records_exactly_one_save() {
        let mut edit = Edit::begin(2, "count", "1");
        edit.set("42");
        let mut saves = Saves::new();
        assert!(edit.commit(&DelegateKind::Number, &mut saves));
        assert!(edit.state.is_committed());
        assert_eq!(edit.value(), Some("42"));
        assert_eq!(
            saves.log,
            vec![Save {
                row: 2,
                column: "count".to_owned(),
                value: "42".to_owned(),
            }]
        );
    }

    #[test]
    fn cancelling_records_no_save() {
        let mut edit = Edit::begin(0, "name", "old");
        edit.set("new");
        let saves = Saves::new();
        edit.cancel();
        assert_eq!(edit.state, EditState::Cancelled);
        assert!(saves.is_empty());
        assert_eq!(edit.value(), None);
    }

    #[test]
    fn a_choice_delegate_refuses_a_value_outside_its_set() {
        let mut edit = Edit::begin(0, "status", "one");
        edit.set("three");
        let mut saves = Saves::new();
        let kind = DelegateKind::Choice(vec!["one".to_owned(), "two".to_owned()]);
        assert!(!edit.commit(&kind, &mut saves));
        assert!(
            edit.error()
                .is_some_and(|reason| reason.contains("one, two"))
        );
        edit.set("two");
        assert!(edit.commit(&kind, &mut saves));
        assert_eq!(saves.len(), 1);
    }

    #[test]
    fn a_check_delegate_normalises_its_flag() {
        let mut edit = Edit::begin(0, "done", "false");
        edit.set("yes");
        let mut saves = Saves::new();
        assert!(edit.commit(&DelegateKind::Check, &mut saves));
        assert_eq!(edit.value(), Some("true"));
        assert_eq!(saves.log[0].value, "true");
    }

    #[test]
    fn the_display_and_the_editor_differ() {
        let kind = DelegateKind::Text;
        assert_eq!(kind.display("hello"), "hello");
        assert_eq!(kind.editor("hello"), "[text: hello]");
        assert_eq!(DelegateKind::Check.display("true"), "☑");
    }

    #[test]
    fn an_edit_renders_its_state_including_the_refusal() {
        let mut edit = Edit::begin(1, "age", "0");
        edit.set("young");
        let mut saves = Saves::new();
        edit.commit(&DelegateKind::Number, &mut saves);
        assert!(edit.render().contains("refused"));
        edit.cancel();
        assert!(edit.render().contains("nothing saved"));
    }
}
