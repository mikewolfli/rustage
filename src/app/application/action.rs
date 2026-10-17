// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The shared action model (task T26.02).
//!
//! The acceptance line is *"every entry point that references the same action shows consistent state;
//! conflicts are diagnosable"*. Three decisions make it true:
//!
//! * **An action has stable identity, and it is the only place its state lives.** [`ActionId`] names
//!   an action; the menu, the tool bar and the context menu reference that id rather than carrying
//!   their own copy, so there is nothing to fall out of step. [`Catalog::state_via`] reads the one
//!   [`State`] for an id, which is why any two entry points agree by construction.
//! * **A radio group is exclusive by operation, not by convention.** [`Catalog::select`] checks one
//!   member and clears the others in the same group in a single call, so two members cannot both be
//!   checked after a selection.
//! * **A conflict is an explicit finding.** A reference to an id that was never defined and two
//!   actions that claim the same shortcut in the same scope are [`Conflict`] values a caller can
//!   inspect and render, never a silent last-writer-wins.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

/// An action's stable identity.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ActionId(String);

impl ActionId {
    /// Builds an id from its name, or `None` when the name is blank or contains whitespace.
    pub fn new(name: &str) -> Option<Self> {
        let trimmed = name.trim();
        if trimmed.is_empty() || trimmed.split_whitespace().count() != 1 {
            return None;
        }
        Some(Self(trimmed.to_owned()))
    }

    /// The id's name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A keyboard modifier.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Modifier {
    /// Control.
    Control,
    /// Shift.
    Shift,
    /// Alt or Option.
    Alt,
    /// Command or Meta.
    Command,
}

impl Modifier {
    /// Every modifier, in the order a shortcut renders them.
    pub const ALL: [Modifier; 4] = [
        Modifier::Control,
        Modifier::Command,
        Modifier::Alt,
        Modifier::Shift,
    ];

    /// The modifier's short name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Control => "Ctrl",
            Self::Shift => "Shift",
            Self::Alt => "Alt",
            Self::Command => "Cmd",
        }
    }

    /// Reads a modifier name, or `None` when it is not one.
    pub fn parse(text: &str) -> Option<Self> {
        match text.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => Some(Self::Control),
            "shift" => Some(Self::Shift),
            "alt" | "option" => Some(Self::Alt),
            "cmd" | "command" | "meta" | "super" => Some(Self::Command),
            _ => None,
        }
    }
}

/// A keyboard shortcut: a key plus the modifiers held with it.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Shortcut {
    /// The key, such as `S` or `F5`.
    pub key: String,
    /// The modifiers, a set so a repeated one is not written twice.
    pub modifiers: BTreeSet<Modifier>,
}

impl Shortcut {
    /// Parses a shortcut such as `Ctrl+Shift+S`, or `None` when a token is not a modifier or key.
    pub fn parse(text: &str) -> Option<Self> {
        let mut parts: Vec<&str> = text.split('+').collect();
        let key = parts.pop()?.trim();
        if key.is_empty() || key.split_whitespace().count() != 1 {
            return None;
        }
        let mut modifiers = BTreeSet::new();
        for part in parts {
            modifiers.insert(Modifier::parse(part.trim())?);
        }
        Some(Self {
            key: key.to_owned(),
            modifiers,
        })
    }

    /// The shortcut as a canonical, comparable string.
    pub fn render(&self) -> String {
        let mut parts: Vec<String> = Modifier::ALL
            .into_iter()
            .filter(|modifier| self.modifiers.contains(modifier))
            .map(|modifier| modifier.name().to_owned())
            .collect();
        parts.push(self.key.clone());
        parts.join("+")
    }
}

/// A built-in shell action the platform performs itself, rather than the application.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Builtin {
    /// Quit the application.
    Quit,
    /// Show the about box.
    About,
    /// Show preferences.
    Preferences,
    /// Close the window.
    CloseWindow,
}

impl Builtin {
    /// The built-in's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Quit => "quit",
            Self::About => "about",
            Self::Preferences => "preferences",
            Self::CloseWindow => "close-window",
        }
    }

    /// Reads a built-in's name, or `None` when it is not one.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "quit" => Some(Self::Quit),
            "about" => Some(Self::About),
            "preferences" => Some(Self::Preferences),
            "close-window" => Some(Self::CloseWindow),
            _ => None,
        }
    }
}

/// What an action dispatches to. Typed, so "a menu item that does nothing" is not a value.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum EventTarget {
    /// A named handler the generated application dispatches.
    Handler(String),
    /// A navigation route.
    Route(String),
    /// A built-in the platform performs.
    Builtin(Builtin),
}

impl EventTarget {
    /// Parses `<kind>:<value>`, or `None` when the kind or value is unknown.
    pub fn parse(kind: &str, value: &str) -> Option<Self> {
        if value.trim().is_empty() {
            return None;
        }
        match kind {
            "handler" => Some(Self::Handler(value.to_owned())),
            "route" => Some(Self::Route(value.to_owned())),
            "builtin" => Builtin::parse(value).map(Self::Builtin),
            _ => None,
        }
    }

    /// The target in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Handler(value) => format!("handler:{value}"),
            Self::Route(value) => format!("route:{value}"),
            Self::Builtin(builtin) => format!("builtin:{}", builtin.name()),
        }
    }
}

/// An action's live state, the part a menu check mark or a greyed item shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct State {
    /// Whether the action can run.
    pub enabled: bool,
    /// Whether the action shows as selected.
    pub checked: bool,
}

/// One shared action: identity plus everything an entry point needs to show and run it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Action {
    /// The stable identity every entry point references.
    pub id: ActionId,
    /// The title shown.
    pub title: String,
    /// The icon name, if the target has an icon theme for it.
    pub icon: Option<String>,
    /// The keyboard shortcut, if any.
    pub shortcut: Option<Shortcut>,
    /// Whether the action can run.
    pub enabled: bool,
    /// Whether the action shows as selected.
    pub checked: bool,
    /// The mutually-exclusive radio group it belongs to, if any.
    pub group: Option<String>,
    /// What it dispatches to.
    pub target: EventTarget,
}

impl Action {
    /// An action with a title and a target, enabled and unchecked.
    pub fn new(id: ActionId, title: &str, target: EventTarget) -> Result<Self, String> {
        let trimmed = title.trim();
        if trimmed.is_empty() {
            return Err(format!("action {} needs a title", id.as_str()));
        }
        Ok(Self {
            id,
            title: trimmed.to_owned(),
            icon: None,
            shortcut: None,
            enabled: true,
            checked: false,
            group: None,
            target,
        })
    }

    /// This action's state.
    pub fn state(&self) -> State {
        State {
            enabled: self.enabled,
            checked: self.checked,
        }
    }
}

/// A place an action is reachable from.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum EntryPoint {
    /// The menu bar.
    Menu,
    /// The tool bar.
    Toolbar,
    /// A context menu.
    Context,
}

impl EntryPoint {
    /// The entry point's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Menu => "menu",
            Self::Toolbar => "toolbar",
            Self::Context => "context",
        }
    }

    /// Reads an entry point's name, or `None` when it is not one.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "menu" => Some(Self::Menu),
            "toolbar" => Some(Self::Toolbar),
            "context" => Some(Self::Context),
            _ => None,
        }
    }
}

/// A scope in which a shortcut competes for the keyboard.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Scope {
    /// Everywhere in the application.
    Global,
    /// Inside a menu.
    Menu,
    /// Inside a tool bar.
    Toolbar,
    /// Inside a context menu.
    Context,
}

impl Scope {
    /// The scope's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Menu => "menu",
            Self::Toolbar => "toolbar",
            Self::Context => "context",
        }
    }

    /// Reads a scope's name, or `None` when it is not one.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "global" => Some(Self::Global),
            "menu" => Some(Self::Menu),
            "toolbar" => Some(Self::Toolbar),
            "context" => Some(Self::Context),
            _ => None,
        }
    }
}

/// An action claiming a shortcut in a scope.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Claim {
    /// The scope it competes in.
    pub scope: Scope,
    /// The action claiming it.
    pub action: ActionId,
}

/// Something that does not line up in a catalog.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Conflict {
    /// An entry point references an action that was never defined.
    UnknownReference {
        /// The entry point.
        entry: EntryPoint,
        /// The id it referenced.
        id: ActionId,
    },
    /// Two actions claim the same shortcut in one scope.
    DuplicateShortcut {
        /// The scope.
        scope: Scope,
        /// The shortcut both claim.
        shortcut: Shortcut,
        /// The action that claimed it first.
        first: ActionId,
        /// The action that claimed it second.
        second: ActionId,
    },
}

impl Conflict {
    /// The conflict in one line.
    pub fn render(&self) -> String {
        match self {
            Self::UnknownReference { entry, id } => format!(
                "the {} references action {} which is not defined",
                entry.name(),
                id.as_str(),
            ),
            Self::DuplicateShortcut {
                scope,
                shortcut,
                first,
                second,
            } => format!(
                "actions {} and {} both claim {} in the {} scope",
                first.as_str(),
                second.as_str(),
                shortcut.render(),
                scope.name(),
            ),
        }
    }
}

/// The set of actions plus the entry points and shortcut scopes that reference them.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Catalog {
    actions: BTreeMap<ActionId, Action>,
    references: Vec<(EntryPoint, ActionId)>,
    claims: Vec<Claim>,
}

impl Catalog {
    /// An empty catalog.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an action, refusing an id that is already defined: one identity, one action.
    pub fn add(&mut self, action: Action) -> Result<(), String> {
        if self.actions.contains_key(&action.id) {
            return Err(format!(
                "action {} is defined twice; an id has one action",
                action.id.as_str(),
            ));
        }
        self.actions.insert(action.id.clone(), action);
        Ok(())
    }

    /// One action, if it is defined.
    pub fn action(&self, id: &ActionId) -> Option<&Action> {
        self.actions.get(id)
    }

    /// Records that an entry point references an action.
    pub fn reference(&mut self, entry: EntryPoint, id: ActionId) {
        self.references.push((entry, id));
    }

    /// Records that an action claims a shortcut in a scope.
    pub fn claim(&mut self, scope: Scope, id: ActionId) {
        self.claims.push(Claim { scope, action: id });
    }

    /// The one state for an id, whatever entry point reads it.
    pub fn state(&self, id: &ActionId) -> Option<State> {
        self.actions.get(id).map(Action::state)
    }

    /// The state an entry point sees, when that entry point references the action.
    ///
    /// The state is read from the single definition, so two entry points that reference the same id
    /// cannot disagree.
    pub fn state_via(&self, entry: EntryPoint, id: &ActionId) -> Option<State> {
        let referenced = self
            .references
            .iter()
            .any(|(candidate, other)| *candidate == entry && other == id);
        if !referenced {
            return None;
        }
        self.state(id)
    }

    /// Turns an action on or off, refusing an unknown id.
    pub fn set_enabled(&mut self, id: &ActionId, enabled: bool) -> Result<(), String> {
        match self.actions.get_mut(id) {
            Some(action) => {
                action.enabled = enabled;
                Ok(())
            }
            None => Err(format!("action {} is not defined", id.as_str())),
        }
    }

    /// Turns an action's check mark on or off, refusing an unknown id.
    pub fn set_checked(&mut self, id: &ActionId, checked: bool) -> Result<(), String> {
        match self.actions.get_mut(id) {
            Some(action) => {
                action.checked = checked;
                Ok(())
            }
            None => Err(format!("action {} is not defined", id.as_str())),
        }
    }

    /// The members of a radio group, in id order.
    pub fn group_members(&self, group: &str) -> Vec<ActionId> {
        self.actions
            .values()
            .filter(|action| action.group.as_deref() == Some(group))
            .map(|action| action.id.clone())
            .collect()
    }

    /// Selects an action: checks it and clears the other members of its radio group, refusing an
    /// unknown id. A call is the whole operation, so two members cannot both end up checked.
    pub fn select(&mut self, id: &ActionId) -> Result<(), String> {
        let Some(group) = self.actions.get(id).map(|action| action.group.clone()) else {
            return Err(format!("action {} is not defined", id.as_str()));
        };
        if let Some(group) = group {
            for action in self.actions.values_mut() {
                if action.group.as_deref() == Some(group.as_str()) {
                    action.checked = false;
                }
            }
        }
        if let Some(action) = self.actions.get_mut(id) {
            action.checked = true;
        }
        Ok(())
    }

    /// Every conflict the catalog has: an undefined reference, or two claims of one shortcut.
    pub fn conflicts(&self) -> Vec<Conflict> {
        let mut conflicts = Vec::new();
        for (entry, id) in &self.references {
            if !self.actions.contains_key(id) {
                conflicts.push(Conflict::UnknownReference {
                    entry: *entry,
                    id: id.clone(),
                });
            }
        }
        let mut by_scope: BTreeMap<Scope, BTreeMap<Shortcut, ActionId>> = BTreeMap::new();
        for claim in &self.claims {
            let Some(action) = self.actions.get(&claim.action) else {
                continue;
            };
            let Some(shortcut) = &action.shortcut else {
                continue;
            };
            let table = by_scope.entry(claim.scope).or_default();
            match table.get(shortcut) {
                Some(first) if first != &claim.action => {
                    conflicts.push(Conflict::DuplicateShortcut {
                        scope: claim.scope,
                        shortcut: shortcut.clone(),
                        first: first.clone(),
                        second: claim.action.clone(),
                    });
                }
                Some(_) => {}
                None => {
                    table.insert(shortcut.clone(), claim.action.clone());
                }
            }
        }
        conflicts
    }

    /// The catalog in a report: one line per action with its state and target.
    pub fn render(&self) -> Vec<String> {
        self.actions
            .values()
            .map(|action| {
                let mut line = format!(
                    "{} {:?} [{}] -> {}",
                    action.id.as_str(),
                    action.title,
                    if action.enabled {
                        "enabled"
                    } else {
                        "disabled"
                    },
                    action.target.render(),
                );
                if let Some(shortcut) = &action.shortcut {
                    line.push_str(&format!(" ({})", shortcut.render()));
                }
                if action.checked {
                    line.push_str(" *");
                }
                line
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Action, ActionId, Catalog, Conflict, EntryPoint, EventTarget, Modifier, Scope, Shortcut,
        State,
    };

    fn id(name: &str) -> ActionId {
        ActionId::new(name).expect("an id")
    }

    fn action(name: &str, title: &str) -> Action {
        Action::new(
            id(name),
            title,
            EventTarget::parse("handler", name).expect("a target"),
        )
        .expect("an action")
    }

    #[test]
    fn an_id_refuses_a_blank_or_multiword_name() {
        assert!(ActionId::new("").is_none());
        assert!(ActionId::new("  ").is_none());
        assert!(ActionId::new("two words").is_none());
        assert_eq!(id("file.save").as_str(), "file.save");
    }

    #[test]
    fn entry_points_and_scopes_are_closed_names() {
        for entry in [EntryPoint::Menu, EntryPoint::Toolbar, EntryPoint::Context] {
            assert_eq!(EntryPoint::parse(entry.name()), Some(entry));
        }
        assert_eq!(EntryPoint::parse("palette"), None);
        for scope in [Scope::Global, Scope::Menu, Scope::Toolbar, Scope::Context] {
            assert_eq!(Scope::parse(scope.name()), Some(scope));
        }
        assert_eq!(Scope::parse("dialog"), None);
    }

    #[test]
    fn a_shortcut_round_trips_through_its_canonical_form() {
        let shortcut = Shortcut::parse("Shift+Ctrl+S").expect("a shortcut");
        assert_eq!(shortcut.render(), "Ctrl+Shift+S");
        assert!(shortcut.modifiers.contains(&Modifier::Control));
        assert!(shortcut.modifiers.contains(&Modifier::Shift));
        // An unknown modifier is refused; a bare key is a shortcut of its own.
        assert!(Shortcut::parse("Hyper+S").is_none());
        assert!(Shortcut::parse("+S").is_none());
        assert_eq!(
            Shortcut::parse("F5").map(|s| s.render()),
            Some("F5".to_owned())
        );
    }

    #[test]
    fn an_event_target_is_typed_and_an_unknown_kind_is_not_a_target() {
        assert_eq!(
            EventTarget::parse("handler", "save").map(|t| t.render()),
            Some("handler:save".to_owned()),
        );
        assert_eq!(
            EventTarget::parse("builtin", "quit").map(|t| t.render()),
            Some("builtin:quit".to_owned()),
        );
        assert_eq!(EventTarget::parse("builtin", "explode"), None);
        assert_eq!(EventTarget::parse("wish", "anything"), None);
    }

    #[test]
    fn every_entry_point_that_references_an_action_shows_the_same_state() {
        let mut catalog = Catalog::new();
        catalog.add(action("save", "Save")).expect("an action");
        for entry in [EntryPoint::Menu, EntryPoint::Toolbar, EntryPoint::Context] {
            catalog.reference(entry, id("save"));
        }

        let via_menu = catalog.state_via(EntryPoint::Menu, &id("save"));
        let via_toolbar = catalog.state_via(EntryPoint::Toolbar, &id("save"));
        let via_context = catalog.state_via(EntryPoint::Context, &id("save"));
        assert_eq!(
            via_menu,
            Some(State {
                enabled: true,
                checked: false
            })
        );
        assert_eq!(via_menu, via_toolbar);
        assert_eq!(via_toolbar, via_context);

        // Disabling once is visible from every entry point, because there is one definition.
        catalog.set_enabled(&id("save"), false).expect("a known id");
        assert_eq!(
            catalog.state_via(EntryPoint::Menu, &id("save")),
            Some(State {
                enabled: false,
                checked: false
            })
        );
        assert_eq!(
            catalog.state_via(EntryPoint::Toolbar, &id("save")),
            catalog.state_via(EntryPoint::Menu, &id("save")),
        );
    }

    #[test]
    fn a_reference_to_an_undefined_action_is_a_diagnosable_conflict() {
        let mut catalog = Catalog::new();
        catalog.reference(EntryPoint::Menu, id("ghost"));
        let conflicts = catalog.conflicts();
        assert_eq!(conflicts.len(), 1);
        assert!(matches!(conflicts[0], Conflict::UnknownReference { .. }));
        assert!(conflicts[0].render().contains("ghost"), "{conflicts:?}");
    }

    #[test]
    fn two_actions_claiming_one_shortcut_in_one_scope_is_a_conflict() {
        let mut catalog = Catalog::new();
        let mut first = action("save", "Save");
        first.shortcut = Shortcut::parse("Ctrl+S");
        let mut second = action("save-as", "Save As");
        second.shortcut = Shortcut::parse("Ctrl+S");
        catalog.add(first).expect("an action");
        catalog.add(second).expect("an action");
        catalog.claim(Scope::Global, id("save"));
        catalog.claim(Scope::Global, id("save-as"));

        let conflicts = catalog.conflicts();
        assert_eq!(conflicts.len(), 1);
        assert!(matches!(conflicts[0], Conflict::DuplicateShortcut { .. }));
        assert!(conflicts[0].render().contains("Ctrl+S"), "{conflicts:?}");
    }

    #[test]
    fn the_same_shortcut_in_two_different_scopes_is_not_a_conflict() {
        let mut catalog = Catalog::new();
        let mut action = action("save", "Save");
        action.shortcut = Shortcut::parse("Ctrl+S");
        catalog.add(action).expect("an action");
        catalog.claim(Scope::Menu, id("save"));
        catalog.claim(Scope::Toolbar, id("save"));
        assert!(catalog.conflicts().is_empty());
    }

    #[test]
    fn selecting_a_radio_member_clears_the_others() {
        let mut catalog = Catalog::new();
        let mut left = action("align.left", "Left");
        left.group = Some("align".to_owned());
        let mut right = action("align.right", "Right");
        right.group = Some("align".to_owned());
        catalog.add(left).expect("an action");
        catalog.add(right).expect("an action");
        assert_eq!(catalog.group_members("align").len(), 2);

        catalog.select(&id("align.left")).expect("a known id");
        assert_eq!(
            catalog.state(&id("align.left")).map(|s| s.checked),
            Some(true)
        );
        assert_eq!(
            catalog.state(&id("align.right")).map(|s| s.checked),
            Some(false)
        );

        catalog.select(&id("align.right")).expect("a known id");
        assert_eq!(
            catalog.state(&id("align.left")).map(|s| s.checked),
            Some(false)
        );
        assert_eq!(
            catalog.state(&id("align.right")).map(|s| s.checked),
            Some(true)
        );
    }

    #[test]
    fn an_action_must_have_a_title_and_one_id_is_one_action() {
        assert!(Action::new(id("x"), "  ", EventTarget::parse("route", "home").unwrap()).is_err());
        let mut catalog = Catalog::new();
        catalog.add(action("save", "Save")).expect("an action");
        assert!(catalog.add(action("save", "Save again")).is_err());
    }

    #[test]
    fn the_catalog_renders_each_action_with_its_state() {
        let mut catalog = Catalog::new();
        catalog.add(action("save", "Save")).expect("an action");
        assert!(catalog.action(&id("save")).is_some());
        catalog.set_enabled(&id("save"), false).expect("a known id");
        catalog.set_checked(&id("save"), true).expect("a known id");
        assert_eq!(
            catalog.state(&id("save")).map(|state| state.checked),
            Some(true)
        );
        let report = catalog.render().join("\n");
        assert!(report.contains("save"), "{report}");
        assert!(report.contains("disabled"), "{report}");
        assert!(report.contains("-> handler:save"), "{report}");
    }
}
