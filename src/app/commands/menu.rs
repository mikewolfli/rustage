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

use crate::app::commands::command::Registry;

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
        /// The submenu's stable id, which its labels are keyed by.
        id: String,
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
    pub fn submenu(id: impl Into<String>, title: impl Into<String>, items: Vec<Item>) -> Self {
        Self::Submenu {
            id: id.into(),
            title: title.into(),
            items,
        }
    }

    /// Whether this item is a place a person can run something.
    ///
    /// A planned leaf is ledger material, not a surface: the native menu has no disabled switch, so
    /// painting it would produce an enabled entry with no handler. A submenu is paintable only when
    /// it contains at least one command leaf; an empty branch would open an empty popup. This is the
    /// rule the window applies before it creates a menu handle (principle #22/#26).
    pub fn is_paintable(&self) -> bool {
        match self {
            Self::Command { .. } => true,
            Self::Planned { .. } | Self::Separator => false,
            Self::Submenu { items, .. } => items.iter().any(Item::is_paintable),
        }
    }
}

/// A menu: a stable id, a title and its items.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Menu {
    id: String,
    title: String,
    items: Vec<Item>,
}

impl Menu {
    /// A menu with an id.
    pub fn new(id: impl Into<String>, title: impl Into<String>, items: Vec<Item>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            items,
        }
    }

    /// The menu's stable id.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The menu's title.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Its items.
    pub fn items(&self) -> &[Item] {
        &self.items
    }

    /// Whether the menu has at least one leaf that runs a command.
    ///
    /// A menu made only of planned leaves is a roadmap entry, not something the window should
    /// paint. The window checks this before creating the menu handle; the tree can therefore hold
    /// the full ledger without producing empty menus (principle #26/#61).
    pub fn is_paintable(&self) -> bool {
        self.items.iter().any(Item::is_paintable)
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
            Item::Submenu { title, items, .. } => {
                let child = format!("{path} > {title}");
                collect_leaves(&child, items, found);
            }
            other => found.push((path.to_owned(), other)),
        }
    }
}

/// The menus this build declares: the §3 groups whose commands exist, with the rest marked planned so
/// the tree is honest about what a person would see rather than only about what works.
/// The roadmap task that owns a planned command, when the menu ledger names one.
///
/// The runtime uses this so a command a key still binds (Ctrl+S, Ctrl+Z) answers with the task that
/// will implement it instead of "no handler": the menu is the ledger, and a second table here would
/// be the drift the module note warns about.
pub fn planned_task(command: &str) -> Option<String> {
    fn find(items: &[Item], command: &str) -> Option<String> {
        for item in items {
            match item {
                Item::Planned {
                    command: id, task, ..
                } if id == command => {
                    return Some(task.clone());
                }
                Item::Submenu { items, .. } => {
                    if let Some(task) = find(items, command) {
                        return Some(task);
                    }
                }
                _ => {}
            }
        }
        None
    }
    built_in()
        .iter()
        .find_map(|menu| find(menu.items(), command))
}

pub fn built_in() -> Vec<Menu> {
    vec![
        Menu::new(
            "file",
            "File",
            vec![
                Item::command("Save", "file.save"),
                Item::command("Save All", "file.save-all"),
                Item::command("Save As", "file.save-as"),
                Item::command("Save Copy", "file.save-copy"),
                Item::separator(),
                Item::command("Close", "file.close"),
                Item::command("Close Others", "file.close-others"),
                Item::command("Close Saved", "file.close-saved"),
                Item::command("Close Project", "file.close-project"),
                Item::command("Reopen Closed Document", "file.open-reopen"),
                Item::command("Copy Project Path", "file.copy-path"),
                Item::command("Reveal Project", "file.reveal"),
                Item::planned("New Project", "file.new-project", "T02.02"),
                Item::planned("Open Project", "file.open-project", "T02.03"),
            ],
        ),
        Menu::new(
            "edit",
            "Edit",
            vec![
                Item::command("Undo", "edit.undo"),
                Item::command("Redo", "edit.redo"),
                Item::command("Operation History", "edit.history-view"),
                Item::command("Cut", "edit.cut"),
                Item::command("Copy", "edit.copy"),
                Item::command("Copy Node ID", "edit.copy-node-id"),
                Item::command("Paste", "edit.paste"),
                Item::command("Select All", "edit.select-all"),
                Item::command("Clear Selection", "edit.select-clear"),
                Item::command("Expand Selection", "edit.select-expand"),
                Item::command("Shrink Selection", "edit.select-shrink"),
                Item::command("Select All Occurrences", "edit.select-occurrences"),
                Item::command("Select Next Occurrence", "edit.select-next-occurrence"),
                Item::command("Add Cursor Above", "edit.select-cursor-above"),
                Item::command("Add Cursor Below", "edit.select-cursor-below"),
                Item::separator(),
                Item::submenu(
                    "text",
                    "Text",
                    vec![
                        Item::command("Toggle Comment", "edit.text-comment"),
                        Item::command("Indent", "edit.text-indent"),
                        Item::command("Outdent", "edit.text-outdent"),
                        Item::command("Move Lines Up", "edit.text-move-up"),
                        Item::command("Move Lines Down", "edit.text-move-down"),
                        Item::command("Duplicate Lines", "edit.text-duplicate"),
                        Item::command("Delete Lines", "edit.text-delete"),
                        Item::command("Join Lines", "edit.text-join"),
                        Item::command("Sort Lines", "edit.text-sort"),
                    ],
                ),
                Item::separator(),
                Item::command("Find", "edit.find"),
                Item::command("Replace", "edit.replace"),
                Item::command("Find Next", "edit.find-next"),
                Item::command("Find Previous", "edit.find-previous"),
                Item::command("Find in Selection", "edit.find-selection"),
            ],
        ),
        Menu::new(
            "view",
            "View",
            vec![
                Item::command("Welcome", "view.welcome"),
                Item::command("Command Palette", "view.palette"),
                // The keyboard commands have a visible home too: T04.08's rule is that a key
                // operation is reachable by eye as well as by key, and these three were bound
                // with nothing in a menu to point at them.
                Item::command("Keyboard Shortcuts", "keys.show"),
                Item::command("Hide Current Panel", "view.panel-hide-current"),
                Item::command("Restore Closed Panel", "view.panel-restore"),
                Item::command("Toolbar", "view.display-toolbar"),
                Item::command("Status Bar", "view.display-status-bar"),
                Item::command("Zoom In", "view.display-zoom-in"),
                Item::command("Zoom Out", "view.display-zoom-out"),
                Item::command("Reset Zoom", "view.display-zoom-reset"),
                Item::submenu(
                    "editor",
                    "Editor",
                    vec![
                        Item::command("Split Editor", "view.split"),
                        Item::command("Split Editor Down", "view.split-down"),
                        Item::command("Merge Editor Group", "view.merge-group"),
                        Item::command("Next Editor Group", "view.next-group"),
                        Item::command("Previous Editor Group", "view.previous-group"),
                        Item::command("Move Tab to Next Group", "view.move-tab"),
                    ],
                ),
                Item::submenu(
                    "modes",
                    "Modes",
                    vec![
                        Item::command("Welcome", "view.mode-welcome"),
                        Item::command("Project", "view.mode-project"),
                        Item::command("Design", "view.mode-design"),
                        Item::command("Code", "view.mode-code"),
                        Item::command("Debug", "view.mode-debug"),
                        Item::command("Test", "view.mode-test"),
                        Item::command("Release", "view.mode-release"),
                    ],
                ),
                Item::planned("Close Panel", "panel.close", "T04.02"),
                Item::planned("Activate Tree Item", "tree.activate", "T04.02"),
                Item::submenu(
                    "panels",
                    "Panels",
                    vec![
                        Item::planned("Outline", "panel.outline", "T04.02"),
                        Item::planned("Inspector", "panel.inspector", "T04.02"),
                    ],
                ),
            ],
        ),
        Menu::new(
            "navigate",
            "Navigate",
            vec![
                Item::command("Select Parent", "nav.design-parent"),
                Item::command("Select First Child", "nav.design-child"),
                Item::command("Select Next Sibling", "nav.design-next-sibling"),
                Item::command("Select Previous Sibling", "nav.design-previous-sibling"),
                Item::command("Go to Line", "nav.goto-line"),
                Item::command("Open File", "nav.open-file"),
                Item::command("Go to Symbol in File", "nav.file-symbol"),
                Item::command("Go to Symbol in Workspace", "nav.workspace-symbol"),
                Item::command("Reveal Current File", "nav.reveal-current-file"),
                Item::submenu(
                    "code",
                    "Code",
                    vec![
                        Item::command("Go to Definition", "nav.code-definition"),
                        Item::command("Go to Declaration", "nav.code-declaration"),
                        Item::command("Go to Type Definition", "nav.code-type-definition"),
                        Item::command("Go to Implementation", "nav.code-implementation"),
                        Item::command("Find References", "nav.code-references"),
                        Item::command("Go Back", "nav.code-back"),
                        Item::command("Go Forward", "nav.code-forward"),
                        Item::separator(),
                        Item::command("Fold Block", "code.fold"),
                        Item::command("Unfold Block", "code.unfold"),
                        Item::command("Fold All", "code.fold-all"),
                        Item::command("Unfold All", "code.unfold-all"),
                        Item::command("Show Outline", "code.outline"),
                        Item::command("Show Breadcrumbs", "code.breadcrumbs"),
                        Item::command("Toggle Inlay Hints", "code.inlay-hints"),
                        Item::command("Toggle Bookmark", "code.bookmark-toggle"),
                    ],
                ),
            ],
        ),
        Menu::new(
            "debug",
            "Debug",
            vec![
                Item::command("Start Debugging", "debug.start"),
                Item::command("Continue", "debug.continue"),
                Item::command("Pause", "debug.pause"),
                Item::command("Step Over", "debug.step"),
                Item::command("Step Into", "debug.step-in"),
                Item::command("Step Out", "debug.step-out"),
                Item::command("Stop Debugging", "debug.stop"),
            ],
        ),
        Menu::new(
            "project",
            "Project",
            vec![
                Item::command("Check Project", "project.check"),
                Item::command("Trust Project", "project.trust"),
                Item::command("Clean Unused Resources", "project.clean-resources"),
                Item::command("Rescan Project", "project.rescan"),
                Item::command("Check References", "project.check-references"),
                Item::command("Migration Preview", "project.migration"),
            ],
        ),
        Menu::new(
            "design",
            "Design",
            vec![
                Item::command("Fit Canvas", "design.fit"),
                Item::command("Actual Size", "design.actual-size"),
                Item::command("Show Grid", "design.grid"),
                Item::command("Show Rulers", "design.rulers"),
            ],
        ),
        Menu::new(
            "build",
            "Build",
            vec![
                Item::command("Run", "build.run"),
                Item::command("Check", "build.check"),
                Item::command("Rebuild", "build.rebuild"),
                Item::command("Clean", "build.clean"),
                Item::command("Cancel", "build.cancel"),
                Item::command("Copy Output", "build.copy-output"),
                Item::command("Copy Build Command", "build.copy-command"),
                Item::command("Go to First Error", "build.locate-first-error"),
            ],
        ),
        Menu::new(
            "run",
            "Run",
            vec![
                Item::command("Run Application", "run.start"),
                Item::command("Stop Application", "run.stop"),
                Item::command("Clear Output", "run.clear"),
                Item::command("Copy Output", "run.copy-output"),
            ],
        ),
        Menu::new(
            "preview",
            "Preview",
            vec![
                Item::command("Start Preview", "preview.start"),
                Item::command("Stop Preview", "preview.stop"),
                Item::command("Rebuild Preview UI", "preview.rebuild"),
            ],
        ),
        Menu::new(
            "vcs",
            "Version Control",
            vec![Item::command("Refresh Version Control", "vcs.refresh")],
        ),
        Menu::new(
            "terminal",
            "Terminal",
            vec![
                Item::command("Start Terminal", "terminal.start"),
                Item::command("Stop Terminal", "terminal.stop"),
            ],
        ),
        Menu::new(
            "test",
            "Test",
            vec![
                Item::command("Run Tests", "test.run"),
                Item::command("Rerun Failed Tests", "test.run-failed"),
                Item::command("Run Tests in Current File", "test.run-current-file"),
                Item::command("Stop Tests", "test.run-stop"),
            ],
        ),
        Menu::new(
            "help",
            "Help",
            vec![
                Item::command("Keyboard Shortcuts", "help.shortcuts"),
                Item::command("Command Reference", "help.commands"),
                Item::command("Widget Reference", "help.controls"),
            ],
        ),
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
    use crate::app::commands::command::{CommandId, built_in as registry};

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
            total >= 5,
            "expected the implemented commands to appear, got {total}"
        );
    }

    #[test]
    fn a_leaf_that_names_a_command_the_registry_does_not_know_is_a_problem() {
        let registry = registry();
        let menu = Menu::new(
            "file",
            "File",
            vec![Item::command("Explode", "file.explode")],
        );
        let problems = menu.problems(&registry);
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("file.explode"), "{problems:?}");
        assert!(problems[0].contains("no command"), "{problems:?}");
    }

    #[test]
    fn two_leaves_may_not_share_one_command_id() {
        let registry = registry();
        let menu = Menu::new(
            "file",
            "File",
            vec![
                Item::command("Close", "file.close"),
                Item::command("Close (again)", "file.close"),
            ],
        );
        let problems = menu.problems(&registry);
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("both bind file.close"), "{problems:?}");
        assert!(
            problems[0].contains("alias is a second command"),
            "{problems:?}"
        );
    }

    #[test]
    fn a_planned_leaf_that_names_a_registered_command_is_the_same_mismatch_the_other_way() {
        let registry = registry();
        let menu = Menu::new(
            "file",
            "File",
            vec![Item::planned("Close", "file.close", "T99.99")],
        );
        let problems = menu.problems(&registry);
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("would be hiding"), "{problems:?}");
    }

    #[test]
    fn a_menu_or_submenu_of_only_planned_leaves_is_not_paintable() {
        let planned = Menu::new(
            "planned",
            "Planned",
            vec![Item::planned("Later", "file.later", "T99.99")],
        );
        assert!(!planned.is_paintable());
        assert!(!Item::planned("Later", "file.later", "T99.99").is_paintable());
        assert!(
            !Item::submenu(
                "planned",
                "Planned",
                vec![Item::planned("Later", "file.later", "T99.99")]
            )
            .is_paintable()
        );

        let implemented = Menu::new(
            "implemented",
            "Implemented",
            vec![Item::command("Close", "file.close")],
        );
        assert!(implemented.is_paintable());
        assert!(Item::command("Close", "file.close").is_paintable());
        assert!(
            Item::submenu(
                "implemented",
                "Implemented",
                vec![Item::command("Close", "file.close")]
            )
            .is_paintable()
        );
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
