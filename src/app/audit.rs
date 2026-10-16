// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The command inventory audit (task T05.08): the registry, the menu tree and the keymap checked
//! against each other, so nothing dangling is reachable and a new area is re-verified in full.
//!
//! The acceptance line is *"no dangling enabled menu, and a new area is re-verified incrementally"*.
//! Both are properties of the **whole inventory**, which is why the check lives here rather than in
//! each table:
//!
//! * **[`audit`] walks every place a command is named and asks the registry.** A menu leaf, a
//!   keyboard binding and (through the same registry) a palette hit all name ids; an id the registry
//!   does not know is a *dangling* reference, and it is reported with the place that names it. A menu
//!   item that is drawn as enabled but runs nothing is exactly what this forbids.
//! * **It is whole-inventory, so a new area re-verifies by being added.** There is no per-area switch
//!   to remember: adding a command, a menu group or a shortcut adds rows to the same walk, and the
//!   next run of the audit covers them. That is what "incremental re-verification" means when the
//!   inventory is small enough to walk — the alternative, a checklist per area, is a list that drifts.
//! * **Reachability is reported, not enforced.** A registered command that no menu offers and no key
//!   binds is [`Report::unreachable`], a note rather than a failure: a command reachable only from a
//!   palette (T05.04) is legitimate, and the palette searches the registry itself rather than a table
//!   this audit could hold to account.
//!
//! The three sources have one owner each — [`crate::app::command`], [`crate::app::menu`],
//! [`crate::app::shortcuts`] — and this module holds no table of its own, so it cannot become the
//! second place a command is defined.

// The report's accessors are read by the audit command and the tests; the CLI reads `render` and
// `is_clean`, so the two accessors are for a report reader that arrives with T05.08's later half.
#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use crate::app::command::Registry;
use crate::app::menu::Menu;
use crate::app::shortcuts::Keymap;

/// Everything the audit found.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Report {
    problems: Vec<String>,
    unreachable: Vec<String>,
}

impl Report {
    /// The dangling references and broken invariants, each naming the place it was found.
    pub fn problems(&self) -> &[String] {
        &self.problems
    }

    /// Registered commands that no menu offers and no key binds.
    pub fn unreachable(&self) -> &[String] {
        &self.unreachable
    }

    /// Whether there is nothing wrong (unreachable commands are a note, not a problem).
    pub fn is_clean(&self) -> bool {
        self.problems.is_empty()
    }

    /// The report in one listing.
    pub fn render(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if self.problems.is_empty() {
            lines.push("no dangling references: every menu leaf and key binding names a registered command".to_owned());
        } else {
            for problem in &self.problems {
                lines.push(format!("problem: {problem}"));
            }
        }
        for command in &self.unreachable {
            lines.push(format!("note: {command} is in no menu and bound to no key (it is reachable from the palette)"));
        }
        lines
    }
}

/// Walks every table that names a command and checks it against the registry.
pub fn audit(registry: &Registry, menus: &[Menu], keymap: &Keymap) -> Report {
    let mut problems = Vec::new();

    for menu in menus {
        // The menu's own invariants (a registered command, no shared ids, no hidden working command).
        problems.extend(
            menu.problems(registry)
                .into_iter()
                .map(|problem| format!("menu {}: {problem}", menu.title())),
        );
    }

    for binding in keymap.bindings() {
        if let Some(reason) = registry.why_not_exists(binding.command()) {
            problems.push(format!(
                "key {} binds {}: {reason}",
                binding.chord(),
                binding.command()
            ));
        }
    }

    // Which registered commands are named anywhere a person can reach them without the palette.
    let mut named: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for menu in menus {
        for (_, command) in menu.commands() {
            named.insert(command);
        }
    }
    for binding in keymap.bindings() {
        named.insert(binding.command().to_owned());
    }
    let unreachable: Vec<String> = registry
        .commands()
        .map(|command| command.id().to_string())
        .filter(|id| !named.contains(id))
        .collect();

    Report {
        problems,
        unreachable,
    }
}

/// How a person can reach one command.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Reach {
    /// A menu or toolbar entry **and** a key binding.
    Both,
    /// A menu or toolbar entry but no key.
    VisibleOnly,
    /// A key binding but no menu or toolbar entry.
    KeyboardOnly,
    /// Neither: reachable only from the palette.
    Palette,
}

impl Reach {
    /// Whether both entries exist.
    pub const fn is_both(self) -> bool {
        matches!(self, Self::Both)
    }

    /// The reach's name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Both => "visible and keyboard",
            Self::VisibleOnly => "visible only",
            Self::KeyboardOnly => "keyboard only",
            Self::Palette => "palette only",
        }
    }
}

/// Which commands have a visible entry and a keyboard entry (task T04.08).
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Discoverability {
    classified: BTreeMap<String, Reach>,
    problems: Vec<String>,
}

impl Discoverability {
    /// Each command's reach, in id order.
    pub fn classified(&self) -> &BTreeMap<String, Reach> {
        &self.classified
    }

    /// Every command that has both entries, in id order.
    pub fn both(&self) -> Vec<String> {
        self.classified
            .iter()
            .filter(|(_, reach)| reach.is_both())
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// The findings: a toolbar command with no key, and a keyed command with no visible entry.
    pub fn problems(&self) -> &[String] {
        &self.problems
    }

    /// Whether every key operation is reachable both ways.
    pub fn is_clean(&self) -> bool {
        self.problems.is_empty()
    }

    /// The findings in one listing.
    pub fn render(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if self.problems.is_empty() {
            lines.push("every key operation has a visible entry and a keyboard entry".to_owned());
        } else {
            for problem in &self.problems {
                lines.push(format!("problem: {problem}"));
            }
        }
        let both = self.both();
        if !both.is_empty() {
            lines.push(format!(
                "{} command(s) reachable both ways: {}",
                both.len(),
                both.join(", ")
            ));
        }
        lines
    }
}

/// Checks that every **key operation** is reachable both by eye and by key (task T04.08).
///
/// A key operation is one a person is offered at a glance (a toolbar button) or by key (a binding).
/// Each must have **both** entries, and the two ways they could fail are named differently because
/// they mean different things:
///
/// * a toolbar command with no key is something a person can see but cannot reach from the keyboard;
/// * a keyed command with no menu or toolbar entry is something only someone who already knows the
///   shortcut can find — the state this task exists to forbid.
///
/// `toolbars` is each area's command set (from [`crate::app::chrome::toolbar_for`]); a command in any
/// area's toolbar counts as visible. The check holds no list of its own, so it cannot become a second
/// definition of what is reachable.
pub fn discoverability(
    registry: &Registry,
    menus: &[Menu],
    toolbars: &[&[String]],
    keymap: &Keymap,
) -> Discoverability {
    let mut visible: BTreeSet<String> = BTreeSet::new();
    for menu in menus {
        for (_, command) in menu.commands() {
            visible.insert(command);
        }
    }
    let mut toolbar: BTreeSet<String> = BTreeSet::new();
    for set in toolbars {
        for id in *set {
            toolbar.insert(id.clone());
        }
    }
    visible.extend(toolbar.iter().cloned());
    let keyboard: BTreeSet<String> = keymap
        .bindings()
        .iter()
        .map(|binding| binding.command().to_owned())
        .collect();

    let mut classified = BTreeMap::new();
    for command in registry.commands() {
        let id = command.id().to_string();
        let seen = visible.contains(&id);
        let keyed = keyboard.contains(&id);
        let reach = match (seen, keyed) {
            (true, true) => Reach::Both,
            (true, false) => Reach::VisibleOnly,
            (false, true) => Reach::KeyboardOnly,
            (false, false) => Reach::Palette,
        };
        classified.insert(id, reach);
    }

    let mut problems = Vec::new();
    for id in &toolbar {
        if !keyboard.contains(id) {
            problems.push(format!("{id} is on a toolbar but bound to no key"));
        }
    }
    for id in &keyboard {
        if !visible.contains(id) {
            problems.push(format!(
                "{id} is bound to a key but is in no menu and on no toolbar"
            ));
        }
    }

    Discoverability {
        classified,
        problems,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::command::{CommandId, built_in as registry};
    use crate::app::menu::{Item, Menu};
    use crate::app::shortcuts::{Binding, Chord, Keymap, Scope};

    fn menus() -> Vec<Menu> {
        crate::app::menu::built_in()
    }

    fn toolbars() -> Vec<Vec<String>> {
        ["design", "code", "debug", "welcome"]
            .into_iter()
            .map(crate::app::chrome::toolbar_for)
            .collect()
    }

    #[test]
    fn the_built_in_inventory_has_nothing_dangling() {
        let report = audit(&registry(), &menus(), &crate::app::shortcuts::built_in());
        assert!(report.is_clean(), "{:?}", report.problems());
        assert!(
            report.problems().is_empty(),
            "the shipped menu and keymap must name only registered commands: {:?}",
            report.problems()
        );
        // The command palette finds its way to view.palette, so nothing need be flagged unreachable
        // as a failure; any note that is present names a real command.
        for note in report.unreachable() {
            assert!(registry().why_not_exists(note).is_none(), "{note}");
        }
    }

    #[test]
    fn a_menu_leaf_naming_an_unregistered_command_is_reported() {
        let menu = Menu::new("File", vec![Item::command("Explode", "file.explode")]);
        let report = audit(&registry(), &[menu], &Keymap::new());
        assert!(!report.is_clean());
        assert!(
            report.problems()[0].contains("menu File"),
            "{:?}",
            report.problems()
        );
        assert!(report.problems()[0].contains("file.explode"));
    }

    #[test]
    fn a_key_binding_naming_an_unregistered_command_is_reported() {
        let mut keymap = Keymap::new();
        keymap
            .add(Binding::new(
                Chord::parse("Ctrl+Q").unwrap(),
                "nothing.here",
                Scope::Global,
            ))
            .expect("no conflict");
        let report = audit(&registry(), &[], &keymap);
        assert!(!report.is_clean());
        assert!(
            report.problems()[0].contains("Ctrl+Q"),
            "{:?}",
            report.problems()
        );
        assert!(report.problems()[0].contains("nothing.here"));
    }

    #[test]
    fn a_registered_command_nobody_can_reach_is_a_note_not_a_problem() {
        let mut registry = registry();
        registry
            .register(crate::app::command::Command::new(
                CommandId::parse("hidden.thing").unwrap(),
                "Hidden",
                "Hidden",
            ))
            .expect("registers");
        let report = audit(&registry, &menus(), &crate::app::shortcuts::built_in());
        assert!(
            report.is_clean(),
            "unreachable is not a problem: {:?}",
            report.problems()
        );
        assert!(
            report
                .unreachable()
                .iter()
                .any(|note| note == "hidden.thing")
        );
        assert!(report.render().join("\n").contains("hidden.thing"));
    }

    #[test]
    fn adding_an_area_re_verifies_through_the_same_walk() {
        // A new menu group whose leaf names a command that does not exist yet is caught without any
        // per-area setup: the walk is over whatever tables it is given.
        let mut new_menus = menus();
        new_menus.push(Menu::new(
            "Publish",
            vec![
                Item::command("Publish", "publish.run"),
                Item::planned("Sign", "publish.sign", "T23.01"),
            ],
        ));
        let report = audit(&registry(), &new_menus, &crate::app::shortcuts::built_in());
        assert!(
            report
                .problems()
                .iter()
                .any(|problem| problem.contains("publish.run"))
        );
        // The planned leaf is fine: it names no registered command.
        assert!(
            !report
                .problems()
                .iter()
                .any(|problem| problem.contains("publish.sign"))
        );
    }

    #[test]
    fn every_key_operation_is_reachable_both_ways_in_the_built_in_inventory() {
        let sets = toolbars();
        let sets: Vec<&[String]> = sets.iter().map(Vec::as_slice).collect();
        let report = discoverability(
            &registry(),
            &menus(),
            sets.as_slice(),
            &crate::app::shortcuts::built_in(),
        );
        assert!(report.is_clean(), "{:?}", report.problems());
        let both = report.both();
        assert!(both.contains(&"file.save".to_owned()), "{both:?}");
        assert!(both.contains(&"view.palette".to_owned()), "{both:?}");
        // A command reachable only from the palette is classified as such, not as a failure.
        assert_eq!(
            report.classified().get("file.save-as"),
            Some(&Reach::VisibleOnly)
        );
    }

    #[test]
    fn a_toolbar_command_with_no_key_is_reported() {
        // file.save-as is in a menu but bound to no key; putting it on a toolbar makes the missing
        // keyboard entry a problem, because a toolbar button is an advertised operation.
        let set = vec!["file.save-as".to_owned()];
        let sets: [&[String]; 1] = [set.as_slice()];
        let report = discoverability(
            &registry(),
            &menus(),
            &sets,
            &crate::app::shortcuts::built_in(),
        );
        assert!(
            report
                .problems()
                .iter()
                .any(|problem| problem.contains("file.save-as") && problem.contains("no key")),
            "{:?}",
            report.problems()
        );
    }

    #[test]
    fn a_keyed_command_with_no_visible_entry_is_reported() {
        let mut keymap = Keymap::new();
        keymap
            .add(Binding::new(
                Chord::parse("Ctrl+Q").unwrap(),
                "file.save-as",
                Scope::Global,
            ))
            .expect("no conflict");
        // No menus and no toolbars: the keyed command has nowhere visible to point at.
        let no_toolbars: [&[String]; 0] = [];
        let report = discoverability(&registry(), &[], &no_toolbars, &keymap);
        assert!(
            report
                .problems()
                .iter()
                .any(|problem| problem.contains("file.save-as") && problem.contains("no menu")),
            "{:?}",
            report.problems()
        );
        assert_eq!(
            report.classified().get("file.save-as"),
            Some(&Reach::KeyboardOnly)
        );
    }
}
