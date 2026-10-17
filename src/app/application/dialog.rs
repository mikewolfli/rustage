// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Dialogs, modality, results and focus restoration (task T26.06).
//!
//! The acceptance line is *"multi-window ownership, close cancels, nested modals do not deadlock"*.
//! The rules that make each part true:
//!
//! * **Every dialog is owned.** A [`Dialog`] names the window it belongs to, and [`Stack::open`]
//!   refuses one whose owner is not on the focus path, so a modal can never be orphaned from the
//!   window that would receive its focus back.
//! * **A close cancels.** [`Stack::close`] answers [`Outcome::Cancelled`]; it is not possible to
//!   close a dialog and have it look accepted. Accepting is a separate call, [`Stack::accept`], which
//!   needs the dialog's default action.
//! * **The stack cannot deadlock.** A modal may only be closed from the top, and closing it removes
//!   exactly one entry, so focus returns to the dialog (or the main window) beneath it. Owning nested
//!   modals and closing them in order therefore always unwinds back to the main window: opening N
//!   modals and closing N modals is a finite walk that ends where it started.

#![allow(dead_code)]

use std::collections::BTreeMap;

use super::action::ActionId;

/// A window or dialog's identity.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct WindowId(String);

impl WindowId {
    /// Builds a window id, or `None` when the name is blank.
    pub fn new(name: &str) -> Option<Self> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return None;
        }
        Some(Self(trimmed.to_owned()))
    }

    /// The id's name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What kind of dialog this is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// A message with a single acknowledgement.
    Message,
    /// A confirmation.
    Confirm,
    /// A prompt for a value.
    Prompt,
    /// A multi-step wizard.
    Wizard,
}

impl Kind {
    /// The kind's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Message => "message",
            Self::Confirm => "confirm",
            Self::Prompt => "prompt",
            Self::Wizard => "wizard",
        }
    }

    /// Reads a kind's name, or `None` when it is not one.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "message" => Some(Self::Message),
            "confirm" => Some(Self::Confirm),
            "prompt" => Some(Self::Prompt),
            "wizard" => Some(Self::Wizard),
            _ => None,
        }
    }
}

/// Whether a dialog blocks the window that owns it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Modality {
    /// It blocks its owner until it closes.
    Modal,
    /// It does not block its owner.
    Modeless,
}

impl Modality {
    /// The modality's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Modal => "modal",
            Self::Modeless => "modeless",
        }
    }

    /// Reads a modality's name, or `None` when it is not one.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "modal" => Some(Self::Modal),
            "modeless" => Some(Self::Modeless),
            _ => None,
        }
    }

    /// Whether it blocks its owner.
    pub fn is_modal(&self) -> bool {
        matches!(self, Self::Modal)
    }
}

/// A dialog or pop-up.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Dialog {
    /// Its identity.
    pub id: WindowId,
    /// The window it belongs to.
    pub owner: WindowId,
    /// What kind it is.
    pub kind: Kind,
    /// Whether it blocks its owner.
    pub modality: Modality,
    /// The action its confirm button runs.
    pub default: Option<ActionId>,
    /// The action its cancel button runs.
    pub cancel: Option<ActionId>,
}

impl Dialog {
    /// A modal dialog owned by a window, with no actions yet.
    pub fn new(id: WindowId, owner: WindowId, kind: Kind, modality: Modality) -> Self {
        Self {
            id,
            owner,
            kind,
            modality,
            default: None,
            cancel: None,
        }
    }
}

/// What a dialog settled to.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Outcome {
    /// It was accepted, running this action with these field values.
    Accepted {
        /// The action that was run.
        action: ActionId,
        /// The values the dialog carried.
        fields: BTreeMap<String, String>,
    },
    /// It was closed, which cancels it.
    Cancelled,
}

impl Outcome {
    /// The outcome's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Accepted { .. } => "accepted",
            Self::Cancelled => "cancelled",
        }
    }

    /// The outcome in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Accepted { action, fields } => {
                format!(
                    "accepted via {} ({} field(s))",
                    action.as_str(),
                    fields.len()
                )
            }
            Self::Cancelled => "cancelled".to_owned(),
        }
    }
}

/// The main window and the dialogs open over it, most recent last.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Stack {
    main: WindowId,
    open: Vec<Dialog>,
}

impl Stack {
    /// A stack around the main window, with nothing open.
    pub fn new(main: WindowId) -> Self {
        Self {
            main,
            open: Vec::new(),
        }
    }

    /// The main window.
    pub fn main(&self) -> &WindowId {
        &self.main
    }

    /// Whether a window id is the main window or an open dialog.
    pub fn contains(&self, id: &WindowId) -> bool {
        &self.main == id || self.open.iter().any(|dialog| &dialog.id == id)
    }

    /// The window that has focus: the top dialog, or the main window.
    pub fn focus(&self) -> WindowId {
        self.open
            .last()
            .map(|dialog| dialog.id.clone())
            .unwrap_or_else(|| self.main.clone())
    }

    /// The open dialog ids, oldest first.
    pub fn open_ids(&self) -> Vec<WindowId> {
        self.open.iter().map(|dialog| dialog.id.clone()).collect()
    }

    /// One open dialog by id.
    pub fn dialog(&self, id: &WindowId) -> Option<&Dialog> {
        self.open.iter().find(|dialog| &dialog.id == id)
    }

    /// Opens a dialog, refusing a duplicate id or an owner that is not open.
    pub fn open(&mut self, dialog: Dialog) -> Result<(), String> {
        if self.contains(&dialog.id) {
            return Err(format!("{} is already open", dialog.id.as_str()));
        }
        if !self.contains(&dialog.owner) {
            return Err(format!(
                "{} is owned by {}, which is not open",
                dialog.id.as_str(),
                dialog.owner.as_str(),
            ));
        }
        self.open.push(dialog);
        Ok(())
    }

    /// The index of a dialog, or an error when it is not open.
    fn index_of(&self, id: &WindowId) -> Result<usize, String> {
        self.open
            .iter()
            .position(|dialog| &dialog.id == id)
            .ok_or_else(|| format!("{} is not open", id.as_str()))
    }

    /// Refuses closing a modal that another modal sits above, which is the deadlock this guards.
    fn check_closable(&self, index: usize) -> Result<(), String> {
        let dialog = &self.open[index];
        if dialog.modality.is_modal() && index + 1 != self.open.len() {
            return Err(format!(
                "{} is a modal that {} sits above; close the top dialog first",
                dialog.id.as_str(),
                self.open[index + 1].id.as_str(),
            ));
        }
        Ok(())
    }

    /// Closes a dialog, which cancels it. Focus moves back to its owner.
    pub fn close(&mut self, id: &WindowId) -> Result<Outcome, String> {
        let index = self.index_of(id)?;
        self.check_closable(index)?;
        self.open.remove(index);
        Ok(Outcome::Cancelled)
    }

    /// Accepts a top dialog, running its default action. A dialog with no default action cannot be
    /// accepted, so a wizard that is not finished cannot look finished.
    pub fn accept(
        &mut self,
        id: &WindowId,
        fields: BTreeMap<String, String>,
    ) -> Result<Outcome, String> {
        let index = self.index_of(id)?;
        self.check_closable(index)?;
        let Some(action) = self.open[index].default.clone() else {
            return Err(format!("{} has no default action to accept", id.as_str()));
        };
        self.open.remove(index);
        Ok(Outcome::Accepted { action, fields })
    }

    /// The action a dialog's confirm button runs, if any.
    pub fn default_action(&self, id: &WindowId) -> Option<&ActionId> {
        self.dialog(id).and_then(|dialog| dialog.default.as_ref())
    }

    /// The action a dialog's cancel button runs, if any.
    pub fn cancel_action(&self, id: &WindowId) -> Option<&ActionId> {
        self.dialog(id).and_then(|dialog| dialog.cancel.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::{Dialog, Kind, Modality, Outcome, Stack, WindowId};
    use crate::app::application::action::ActionId;
    use std::collections::BTreeMap;

    fn window(name: &str) -> WindowId {
        WindowId::new(name).expect("a window id")
    }

    fn action(name: &str) -> ActionId {
        ActionId::new(name).expect("an action id")
    }

    #[test]
    fn a_window_id_refuses_a_blank_name_and_a_kind_is_parsed_at_the_edge() {
        assert!(WindowId::new("  ").is_none());
        assert_eq!(
            WindowId::new(" main ").map(|id| id.as_str().to_owned()),
            Some("main".to_owned())
        );
        assert_eq!(Kind::parse("wizard"), Some(Kind::Wizard));
        assert_eq!(Kind::parse("balloon"), None);
        assert_eq!(Kind::Wizard.name(), "wizard");
        assert_eq!(Modality::parse("modal"), Some(Modality::Modal));
        assert_eq!(Modality::parse("floating"), None);
        assert_eq!(Modality::Modal.name(), "modal");
        assert!(Modality::Modal.is_modal());
        assert!(!Modality::Modeless.is_modal());
    }

    #[test]
    fn nested_modals_unwind_back_to_the_main_window_without_deadlocking() {
        let mut stack = Stack::new(window("main"));
        let first = window("first");
        let second = window("second");
        let third = window("third");
        stack
            .open(Dialog::new(
                first.clone(),
                window("main"),
                Kind::Confirm,
                Modality::Modal,
            ))
            .expect("a dialog");
        stack
            .open(Dialog::new(
                second.clone(),
                first.clone(),
                Kind::Prompt,
                Modality::Modal,
            ))
            .expect("a dialog");
        stack
            .open(Dialog::new(
                third.clone(),
                second.clone(),
                Kind::Wizard,
                Modality::Modal,
            ))
            .expect("a dialog");
        assert_eq!(stack.focus(), third);
        assert_eq!(stack.main(), &window("main"));
        assert!(stack.contains(&third));
        assert!(stack.dialog(&third).is_some());

        assert_eq!(stack.close(&third).expect("a close"), Outcome::Cancelled);
        assert_eq!(stack.focus(), second, "focus returns to the owner");
        assert_eq!(stack.close(&second).expect("a close"), Outcome::Cancelled);
        assert_eq!(stack.focus(), first);
        assert_eq!(stack.close(&first).expect("a close"), Outcome::Cancelled);
        assert_eq!(stack.focus(), window("main"));
        assert!(stack.open_ids().is_empty());
    }

    #[test]
    fn closing_a_modal_another_modal_sits_on_is_refused() {
        let mut stack = Stack::new(window("main"));
        stack
            .open(Dialog::new(
                window("first"),
                window("main"),
                Kind::Confirm,
                Modality::Modal,
            ))
            .expect("a dialog");
        stack
            .open(Dialog::new(
                window("second"),
                window("first"),
                Kind::Confirm,
                Modality::Modal,
            ))
            .expect("a dialog");

        let refusal = stack.close(&window("first")).expect_err("a refusal");
        assert!(refusal.contains("close the top dialog first"), "{refusal}");
        // The stack is unchanged, so the refusal did not deadlock or half-close anything.
        assert_eq!(stack.open_ids(), vec![window("first"), window("second")]);
    }

    #[test]
    fn a_close_cancels_rather_than_accepting() {
        let mut stack = Stack::new(window("main"));
        stack
            .open(Dialog::new(
                window("ask"),
                window("main"),
                Kind::Confirm,
                Modality::Modal,
            ))
            .expect("a dialog");
        assert_eq!(
            stack.close(&window("ask")).expect("a close"),
            Outcome::Cancelled
        );
        assert_eq!(Outcome::Cancelled.render(), "cancelled");
    }

    #[test]
    fn accepting_a_dialog_runs_its_default_action_and_needs_one_to_exist() {
        let mut stack = Stack::new(window("main"));
        let mut dialog = Dialog::new(
            window("ask"),
            window("main"),
            Kind::Confirm,
            Modality::Modal,
        );
        stack.open(dialog.clone()).expect("a dialog");
        assert!(
            stack.accept(&window("ask"), BTreeMap::new()).is_err(),
            "a dialog without a default action cannot be accepted"
        );

        dialog.default = Some(action("dialog.ok"));
        dialog.cancel = Some(action("dialog.cancel"));
        let mut stack = Stack::new(window("main"));
        stack.open(dialog).expect("a dialog");
        assert_eq!(
            stack.default_action(&window("ask")),
            Some(&action("dialog.ok"))
        );
        assert_eq!(
            stack.cancel_action(&window("ask")),
            Some(&action("dialog.cancel"))
        );

        let mut fields = BTreeMap::new();
        fields.insert("name".to_owned(), "Note".to_owned());
        let outcome = stack.accept(&window("ask"), fields).expect("an acceptance");
        assert_eq!(outcome.name(), "accepted");
        assert!(matches!(outcome, Outcome::Accepted { .. }));
        assert_eq!(stack.focus(), window("main"));
    }

    #[test]
    fn a_dialog_owned_by_a_window_that_is_not_open_is_refused() {
        let mut stack = Stack::new(window("main"));
        let refusal = stack
            .open(Dialog::new(
                window("ask"),
                window("ghost"),
                Kind::Message,
                Modality::Modal,
            ))
            .expect_err("a refusal");
        assert!(refusal.contains("which is not open"), "{refusal}");
    }

    #[test]
    fn the_same_dialog_id_cannot_be_opened_twice() {
        let mut stack = Stack::new(window("main"));
        let dialog = Dialog::new(
            window("ask"),
            window("main"),
            Kind::Message,
            Modality::Modal,
        );
        stack.open(dialog.clone()).expect("a dialog");
        assert!(stack.open(dialog).is_err());
    }

    #[test]
    fn a_modeless_dialog_may_be_closed_beneath_a_modal() {
        let mut stack = Stack::new(window("main"));
        stack
            .open(Dialog::new(
                window("panel"),
                window("main"),
                Kind::Message,
                Modality::Modeless,
            ))
            .expect("a dialog");
        stack
            .open(Dialog::new(
                window("ask"),
                window("main"),
                Kind::Confirm,
                Modality::Modal,
            ))
            .expect("a dialog");
        // The modeless dialog is not the top, but not being modal it may still be closed.
        assert_eq!(
            stack.close(&window("panel")).expect("a close"),
            Outcome::Cancelled
        );
        assert_eq!(stack.open_ids(), vec![window("ask")]);
    }
}
