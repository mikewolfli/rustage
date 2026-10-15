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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::command::{CommandId, built_in as registry};
    use crate::app::menu::{Item, Menu};
    use crate::app::shortcuts::{Binding, Chord, Keymap, Scope};

    fn menus() -> Vec<Menu> {
        crate::app::menu::built_in()
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
}
