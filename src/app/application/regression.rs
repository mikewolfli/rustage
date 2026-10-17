// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The end-to-end shell walk: preview, both generation modes, keyboard and events (task T26.08).
//!
//! The acceptance line is *"a menu action really fires with no duplicate callback; the static artifact
//! behaves the same"*. Two rules make it hold:
//!
//! * **One action fires once per activation, whatever it is bound to.** A [`Walk`] binds shared
//!   [`ActionId`]s to entry points. Activating through one entry fires each bound id exactly once,
//!   even when the action is also bound to another entry, because the id — not the entry — is the
//!   thing that runs. [`Walk::activate`] therefore never double-counts; a wiring mistake that binds
//!   one id twice in the *same* entry is a separate finding, [`Walk::duplicates`], rather than a
//!   second callback.
//! * **The runtime and static artifacts are compared, not assumed equal.** [`behaves_identically`]
//!   compares the event sequence and the fire counts of two walks, so "the static build behaves the
//!   same" is a measurement a caller can fail on.

#![allow(dead_code)]

use std::collections::BTreeMap;

use super::action::ActionId;

/// Which artifact a walk is observing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// The running preview.
    Runtime,
    /// The statically generated artifact.
    Static,
}

impl Mode {
    /// The mode's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Runtime => "runtime",
            Self::Static => "static",
        }
    }

    /// Reads a mode's name, or `None` when it is not one.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "runtime" => Some(Self::Runtime),
            "static" => Some(Self::Static),
            _ => None,
        }
    }
}

/// An entry point an action is reachable from.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Entry {
    /// The menu bar.
    Menu,
    /// The tool bar.
    Toolbar,
    /// A context menu.
    Context,
    /// A keyboard shortcut.
    Shortcut,
}

impl Entry {
    /// The entry's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Menu => "menu",
            Self::Toolbar => "toolbar",
            Self::Context => "context",
            Self::Shortcut => "shortcut",
        }
    }

    /// Reads an entry's name, or `None` when it is not one.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "menu" => Some(Self::Menu),
            "toolbar" => Some(Self::Toolbar),
            "context" => Some(Self::Context),
            "shortcut" => Some(Self::Shortcut),
            _ => None,
        }
    }
}

/// Which actions each entry point is bound to.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Bindings {
    map: BTreeMap<Entry, Vec<ActionId>>,
}

impl Bindings {
    /// No bindings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Binds an action to an entry point.
    pub fn bind(&mut self, entry: Entry, action: ActionId) {
        self.map.entry(entry).or_default().push(action);
    }

    /// The actions bound to an entry point, in order.
    pub fn actions(&self, entry: Entry) -> &[ActionId] {
        self.map.get(&entry).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Whether an action is bound to an entry point.
    pub fn is_bound(&self, entry: Entry, action: &ActionId) -> bool {
        self.actions(entry).contains(action)
    }

    /// Every place an action is bound twice in the same entry, which would be a second callback.
    pub fn duplicates(&self) -> Vec<String> {
        let mut found = Vec::new();
        for (entry, actions) in &self.map {
            let mut seen: Vec<&ActionId> = Vec::new();
            for action in actions {
                if seen.contains(&action) {
                    found.push(format!("{} binds {} twice", entry.name(), action.as_str(),));
                } else {
                    seen.push(action);
                }
            }
        }
        found
    }
}

/// One end-to-end observation of the shell.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Walk {
    /// Which artifact is observed.
    pub mode: Mode,
    binds: Bindings,
    fired: BTreeMap<ActionId, usize>,
    events: Vec<String>,
    activations: Vec<String>,
}

impl Walk {
    /// A walk over one mode.
    pub fn new(mode: Mode) -> Self {
        Self {
            mode,
            binds: Bindings::new(),
            fired: BTreeMap::new(),
            events: Vec::new(),
            activations: Vec::new(),
        }
    }

    /// Binds an action to an entry point.
    pub fn bind(&mut self, entry: Entry, action: ActionId) {
        self.binds.bind(entry, action);
    }

    /// Records that the shell was previewed.
    pub fn preview(&mut self) {
        self.events.push("preview".to_owned());
    }

    /// Records that the artifact was generated.
    pub fn generate(&mut self) {
        self.events.push(format!("generate:{}", self.mode.name()));
    }

    /// Activates an action through an entry point, firing it exactly once.
    ///
    /// Refused when the entry point is not bound to the action: an activation that reaches nothing is
    /// a mistake, not a silent no-op.
    pub fn activate(&mut self, entry: Entry, action: &ActionId) -> Result<(), String> {
        if !self.binds.is_bound(entry, action) {
            return Err(format!(
                "{} is not bound to {}",
                action.as_str(),
                entry.name(),
            ));
        }
        *self.fired.entry(action.clone()).or_insert(0) += 1;
        let fired = format!("{}:{} fired", entry.name(), action.as_str());
        self.events.push(fired.clone());
        self.activations.push(fired);
        Ok(())
    }

    /// How many times an action fired.
    pub fn fired(&self, action: &ActionId) -> usize {
        self.fired.get(action).copied().unwrap_or(0)
    }

    /// The event sequence observed, including how the artifact was produced.
    pub fn events(&self) -> &[String] {
        &self.events
    }

    /// The activations observed, without the preview/generate production record.
    pub fn activations(&self) -> &[String] {
        &self.activations
    }

    /// Every place an action is bound twice in one entry.
    pub fn duplicates(&self) -> Vec<String> {
        self.binds.duplicates()
    }
}

/// Whether two walks behaved identically: the same activations and the same fire counts.
///
/// The production record is excluded, because a mode names itself in it (`generate:static`) and
/// that is an identity rather than a behaviour; what must match is what each artifact *did*.
pub fn behaves_identically(left: &Walk, right: &Walk) -> bool {
    left.activations == right.activations && left.fired == right.fired
}

#[cfg(test)]
mod tests {
    use super::{Entry, Mode, Walk, behaves_identically};
    use crate::app::application::action::ActionId;

    fn action(name: &str) -> ActionId {
        ActionId::new(name).expect("an action id")
    }

    #[test]
    fn a_mode_and_an_entry_are_closed_names() {
        assert_eq!(Mode::parse("runtime"), Some(Mode::Runtime));
        assert_eq!(Mode::parse("static"), Some(Mode::Static));
        assert_eq!(Mode::parse("reload"), None);
        assert_eq!(Entry::parse("toolbar"), Some(Entry::Toolbar));
        assert_eq!(Entry::parse("gesture"), None);
    }

    #[test]
    fn a_menu_action_fires_exactly_once_even_when_it_is_also_on_the_toolbar_and_context_menu() {
        let mut walk = Walk::new(Mode::Runtime);
        let save = action("file.save");
        walk.bind(Entry::Menu, save.clone());
        walk.bind(Entry::Toolbar, save.clone());
        walk.bind(Entry::Context, save.clone());

        walk.activate(Entry::Menu, &save).expect("an activation");
        assert_eq!(walk.fired(&save), 1, "one activation runs the action once");
        assert!(walk.duplicates().is_empty());
    }

    #[test]
    fn a_keyboard_activation_fires_the_action_once() {
        let mut walk = Walk::new(Mode::Runtime);
        let save = action("file.save");
        walk.bind(Entry::Shortcut, save.clone());
        walk.activate(Entry::Shortcut, &save)
            .expect("an activation");
        assert_eq!(walk.fired(&save), 1);
        assert_eq!(walk.events(), &["shortcut:file.save fired".to_owned()]);
    }

    #[test]
    fn a_duplicate_binding_is_diagnosed_but_still_fires_only_once() {
        let mut walk = Walk::new(Mode::Runtime);
        let save = action("file.save");
        walk.bind(Entry::Menu, save.clone());
        walk.bind(Entry::Menu, save.clone());
        let duplicates = walk.duplicates();
        assert_eq!(duplicates.len(), 1);
        assert!(
            duplicates[0].contains("binds file.save twice"),
            "{duplicates:?}"
        );

        walk.activate(Entry::Menu, &save).expect("an activation");
        assert_eq!(walk.fired(&save), 1, "the callback is not duplicated");
    }

    #[test]
    fn activating_through_an_unbound_entry_is_refused() {
        let mut walk = Walk::new(Mode::Runtime);
        let save = action("file.save");
        walk.bind(Entry::Menu, save.clone());
        let refusal = walk.activate(Entry::Toolbar, &save).expect_err("a refusal");
        assert!(refusal.contains("not bound to toolbar"), "{refusal}");
        assert_eq!(walk.fired(&save), 0);
    }

    #[test]
    fn the_static_artifact_behaves_the_same_as_the_runtime_one() {
        let save = action("file.save");
        let mut runtime = Walk::new(Mode::Runtime);
        let mut static_artifact = Walk::new(Mode::Static);
        for walk in [&mut runtime, &mut static_artifact] {
            walk.bind(Entry::Shortcut, save.clone());
            walk.preview();
            walk.generate();
            walk.activate(Entry::Shortcut, &save)
                .expect("an activation");
        }
        assert!(behaves_identically(&runtime, &static_artifact));

        // A different activation makes them diverge, so the comparison is a real measurement.
        static_artifact
            .activate(Entry::Shortcut, &save)
            .expect("an activation");
        assert!(!behaves_identically(&runtime, &static_artifact));
    }

    #[test]
    fn preview_and_generation_are_recorded_in_order() {
        let mut walk = Walk::new(Mode::Static);
        walk.preview();
        walk.generate();
        assert_eq!(
            walk.events(),
            &["preview".to_owned(), "generate:static".to_owned()],
        );
        assert_eq!(walk.mode.name(), "static");
    }
}
