// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The menu tree (task T05.02): the §3 menu, with every leaf that exists bound to a registered command.
//!
//! The acceptance line is *"every implemented leaf binds a unique command id"*. A menu is the place a
//! person meets a command, and the two ways it can lie are both checked here against the registry
//! rather than against a second list:
//!
//! * **A leaf that claims a command must name one that exists.** [`Item::Command`] holds an id, and
//!   [`Menu::problems`] reports any id the registry does not know. The menu cannot show an item that
//!   would do nothing.
//! * **Two leaves cannot share one command id.** A command that appears twice in a menu is either a
//!   mistake or a deliberate alias, and an alias is a *different* thing (a second `Command` with its
//!   own id); sharing the id would make "which item is this" ambiguous and break a menu's own
//!   invariant that an item is a place.
//! * **A leaf for a command this build does not implement carries no id.** [`Item::Planned`] is a
//!   title and the roadmap task that will give it a command; the check reports a `Planned` leaf whose
//!   id *is* registered, because that is the same mismatch in the other direction — the menu would be
//!   hiding a working command behind a "coming soon".
//!
//! What is deliberately *not* here: platform menu adaptation. Turning this tree into a native
//! `NSMenu`, a GTK menu or a `Win32` menu is the platform backend's job (T04.01's window and the
//! library's platform layer), and the tree is the platform-independent thing they all render.

// The tree's own accessors and the check's scratch collector are exercised by the tests until a
// settings surface (T26) and the platform menu builders (T04/T04.01) read them; the silence is
// stated once here with its reason, as the neighbouring trees do.
#![allow(dead_code)]

use std::collections::BTreeMap;

use crate::app::command::Registry;

/// One item in a menu.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Item {
    /// An item that runs a registered command.
    Command {
        /// The title the menu shows.
        title: String,
        /// The command it runs; the registry is the authority on whether it exists.
        command: String,
    },
    /// An item for a command this build does not implement yet, naming the task that will.
    Planned {
        /// The title the menu will show.
        title: String,
        /// The command id the roadmap names, for the record.
        command: String,
        /// The task that will implement it.
        task: String,
    },
    /// A separator line.
    Separator,
    /// A submenu.
    Submenu {
        /// The title the menu shows.
        title: String,
        /// Its items.
        items: Vec<Item>,
    },
}

impl Item {
    /// An item that runs a command.
    pub fn command(title: impl Into<String>, command: impl Into<String>) -> Self {
        Self::Command {
            title: title.into(),
            command: command.into(),
        }
    }

    /// An item for a command that is not implemented yet.
    pub fn planned(
        title: impl Into<String>,
        command: impl Into<String>,
        task: impl Into<String>,
    ) -> Self {
        Self::Planned {
            title: title.into(),
            command: command.into(),
            task: task.into(),
        }
    }

    /// A submenu.
    pub fn submenu(title: impl Into<String>, items: Vec<Item>) -> Self {
        Self::Submenu {
            title: title.into(),
            items,
        }
    }
}

/// A menu: a title and its items.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Menu {
    title: String,
    items: Vec<Item>,
}

impl Menu {
    /// A menu.
    pub fn new(title: impl Into<String>, items: Vec<Item>) -> Self {
        Self {
            title: title.into(),
            items,
        }
    }

    /// The menu's title.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Its items.
    pub fn items(&self) -> &[Item] {
        &self.items
    }

    /// Every leaf that runs a command, as `(title, command)`, in menu order.
    pub fn commands(&self) -> Vec<(String, String)> {
        let mut found = Vec::new();
        collect_commands(&self.items, &mut found);
        found
    }

    /// Every leaf, in menu order, with its path so a report can point at it.
    pub fn leaves(&self) -> Vec<(String, &Item)> {
        let mut found = Vec::new();
        collect_leaves(&self.title, &self.items, &mut found);
        found
    }

    /// Checks the menu against the registry, and reports what does not line up.
    ///
    /// The three checks are the acceptance line plus the two mismatches around it: an implemented
    /// leaf must name a registered command, no two leaves may name the same command, and a planned
    /// leaf must not name one that is registered.
    pub fn problems(&self, registry: &Registry) -> Vec<String> {
        let mut problems = Vec::new();
        let mut seen: BTreeMap<String, String> = BTreeMap::new();
        for (path, item) in self.leaves() {
            match item {
                Item::Command { command, .. } => match registry.why_not_exists(command) {
                    None => {
                        if let Some(previous) = seen.insert(command.clone(), path.clone()) {
                            problems.push(format!(
                                "{path} and {previous} both bind {command}; a menu item is a place, so an \
                                 alias is a second command with its own id"
                            ));
                        }
                    }
                    Some(reason) => problems.push(format!("{path} binds {command}: {reason}")),
                },
                Item::Planned { command, task, .. } => {
                    if registry.why_not_exists(command).is_none() {
                        problems.push(format!(
                            "{path} is marked planned for {task} but {command} is registered; the menu \
                             would be hiding a command that works"
                        ));
                    }
                }
                Item::Separator | Item::Submenu { .. } => {}
            }
        }
        problems
    }

    /// The menu in a report: one line per leaf, with its command or the task it waits for.
    pub fn render(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for (path, item) in self.leaves() {
            match item {
                Item::Command { command, .. } => lines.push(format!("{path} -> {command}")),
                Item::Planned { command, task, .. } => {
                    lines.push(format!("{path} -> {command} (planned: {task})"))
                }
                Item::Separator | Item::Submenu { .. } => {}
            }
        }
        lines
    }
}

fn collect_commands(items: &[Item], found: &mut Vec<(String, String)>) {
    for item in items {
        match item {
            Item::Command { title, command } => found.push((title.clone(), command.clone())),
            Item::Submenu { items, .. } => collect_commands(items, found),
            Item::Planned { .. } | Item::Separator => {}
        }
    }
}

fn collect_leaves<'a>(path: &str, items: &'a [Item], found: &mut Vec<(String, &'a Item)>) {
    for item in items {
        match item {
            Item::Submenu { title, items } => {
                let child = format!("{path} > {title}");
                collect_leaves(&child, items, found);
            }
            other => found.push((path.to_owned(), other)),
        }
    }
}

/// The menus this build declares: the §3 groups whose commands exist, with the rest marked planned so
/// the tree is honest about what a person would see rather than only about what works.
pub fn built_in() -> Vec<Menu> {
    vec![
        Menu::new(
            "File",
            vec![
                Item::command("Save", "file.save"),
                Item::command("Save All", "file.save-all"),
                Item::command("Save As", "file.save-as"),
                Item::separator(),
                Item::command("Close", "file.close"),
                Item::planned("New Project", "file.new-project", "T02.02"),
                Item::planned("Open Project", "file.open-project", "T02.03"),
            ],
        ),
        Menu::new(
            "Edit",
            vec![
                Item::command("Undo", "edit.undo"),
                Item::command("Redo", "edit.redo"),
                Item::planned("Cut", "edit.cut", "T05.06"),
                Item::planned("Copy", "edit.copy", "T05.06"),
                Item::planned("Paste", "edit.paste", "T05.06"),
            ],
        ),
        Menu::new(
            "View",
            vec![
                Item::command("Welcome", "view.welcome"),
                Item::command("Command Palette", "view.palette"),
                Item::submenu(
                    "Panels",
                    vec![
                        Item::planned("Outline", "panel.outline", "T04.02"),
                        Item::planned("Inspector", "panel.inspector", "T04.02"),
                    ],
                ),
            ],
        ),
        Menu::new("Build", vec![Item::command("Run", "build.run")]),
    ]
}

impl Item {
    /// A separator line.
    pub fn separator() -> Self {
        Self::Separator
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::command::{CommandId, built_in as registry};

    #[test]
    fn every_implemented_leaf_binds_a_registered_command_and_the_check_says_so() {
        let registry = registry();
        let menus = built_in();
        for menu in &menus {
            assert!(
                menu.problems(&registry).is_empty(),
                "{:?}",
                menu.problems(&registry)
            );
        }
        // And the leaves really are bound: the menu is not empty of commands.
        let total: usize = menus.iter().map(|menu| menu.commands().len()).sum();
        assert!(
            total >= 7,
            "expected the implemented commands to appear, got {total}"
        );
    }

    #[test]
    fn a_leaf_that_names_a_command_the_registry_does_not_know_is_a_problem() {
        let registry = registry();
        let menu = Menu::new("File", vec![Item::command("Explode", "file.explode")]);
        let problems = menu.problems(&registry);
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("file.explode"), "{problems:?}");
        assert!(problems[0].contains("no command"), "{problems:?}");
    }

    #[test]
    fn two_leaves_may_not_share_one_command_id() {
        let registry = registry();
        let menu = Menu::new(
            "File",
            vec![
                Item::command("Save", "file.save"),
                Item::command("Save (again)", "file.save"),
            ],
        );
        let problems = menu.problems(&registry);
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("both bind file.save"), "{problems:?}");
        assert!(
            problems[0].contains("alias is a second command"),
            "{problems:?}"
        );
    }

    #[test]
    fn a_planned_leaf_that_names_a_registered_command_is_the_same_mismatch_the_other_way() {
        let registry = registry();
        let menu = Menu::new("File", vec![Item::planned("Save", "file.save", "T99.99")]);
        let problems = menu.problems(&registry);
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("would be hiding"), "{problems:?}");
    }

    #[test]
    fn the_tree_reports_its_leaves_with_their_paths() {
        let menus = built_in();
        let view = menus
            .iter()
            .find(|menu| menu.title() == "View")
            .expect("a View menu");
        let render = view.render().join("\n");
        assert!(
            render.contains("View > Panels -> panel.outline (planned: T04.02)"),
            "{render}"
        );
        assert!(render.contains("View -> view.welcome"), "{render}");
        assert!(
            view.leaves()
                .iter()
                .any(|(path, _)| path == "View > Panels"),
            "a nested leaf keeps its path"
        );
    }

    #[test]
    fn every_menu_title_and_leaf_title_is_non_empty() {
        for menu in built_in() {
            assert!(!menu.title().is_empty());
            for (_, item) in menu.leaves() {
                match item {
                    Item::Command { title, .. } | Item::Planned { title, .. } => {
                        assert!(!title.is_empty())
                    }
                    Item::Separator | Item::Submenu { .. } => {}
                }
            }
        }
        // A planned leaf carries the task that will give it a command.
        for menu in built_in() {
            for (_, item) in menu.leaves() {
                if let Item::Planned { task, .. } = item {
                    assert!(!task.is_empty());
                }
            }
        }
    }

    #[test]
    fn a_command_id_in_the_menu_is_the_same_shape_the_registry_uses() {
        for menu in built_in() {
            for (title, command) in menu.commands() {
                assert!(
                    CommandId::parse(&command).is_some(),
                    "{title} names {command}, which is not a command id"
                );
            }
        }
    }
}
