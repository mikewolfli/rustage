// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The designer's own interface vocabulary, identified as keys (roadmap task T24.03).
//!
//! This is the half of i18n the principle calls "标记程序所有字符串": every label the designer's own
//! interface shows is given a stable key here, and the English pack is *derived* from it, so the
//! strings the interface shows and the strings the pack holds cannot drift apart — there is one
//! source, and the pack is a view of it.
//!
//! Where a key comes from matters, so each [`Entry`] names its [`Source`]:
//!
//! * [`Source::Command`] — the command registry ([`crate::app::commands::command`]): a command's
//!   title, category and description, and each parameter's help. The key is the command's own stable
//!   id (`cmd.file.save.title`), so it does not move when the English does.
//! * [`Source::Menu`] — the menu tree ([`crate::app::commands::menu`]): a menu's title, each leaf's
//!   title (keyed by the command the leaf binds) and each submenu's title (keyed by the submenu's id).
//! * [`Source::Diagnostic`] — the sentences a refusal is reported with ([`TEMPLATES`]); these are read
//!   back by [`crate::app::commands::command::Refusal::message`] and
//!   [`crate::app::commands::command::Registry::why_not_exists`], so a translated pack changes the
//!   diagnostic a person reads.
//! * [`Source::Shell`] — the process's own lines ([`SHELL`]): the version banner, the status summary
//!   and the `rustage:` prefix a refusal is printed with.
//!
//! Two boundaries are deliberate and are named rather than hidden. First, a *key* is derived from an
//! existing identifier (a command id, a menu id, a named submenu) and never from the English text, so
//! editing the English does not move a key. Second, the per-command diagnostic *detail* sentences a
//! handler builds from its own arguments (the `reason` carried by a refusal) are not catalogued here
//! yet; the refusal *templates* that wrap them are, and the detail vocabulary is T24.03's remaining
//! work.

#![allow(dead_code)]

use crate::app::commands::command::built_in as registry;
use crate::app::commands::menu::{Item, built_in as menus};
use crate::app::commands::shortcuts::built_in as keymap;
use crate::app::locale::catalog::Pack;
use crate::host::locale::Language;

/// Where a catalogued string comes from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    /// A command's own vocabulary.
    Command,
    /// A menu's labels.
    Menu,
    /// A sentence a diagnostic is reported with.
    Diagnostic,
    /// A line the process itself prints.
    Shell,
}

impl Source {
    /// The source's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Command => "command",
            Self::Menu => "menu",
            Self::Diagnostic => "diagnostic",
            Self::Shell => "shell",
        }
    }
}

/// One identified interface string.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Entry {
    /// The stable key.
    pub key: String,
    /// The English text the interface shows.
    pub english: String,
    /// Where it comes from.
    pub source: Source,
}

/// The diagnostic templates, keyed by what they say rather than by their English.
///
/// A `{name}` is a placeholder [`crate::app::locale::catalog::Catalog::fill`] replaces.
pub const TEMPLATES: [(&str, &str); 7] = [
    ("diag.command.unknown", "there is no command {id}"),
    ("diag.command.disabled", "{id} is not available: {reason}"),
    ("diag.command.missing", "{id} needs a {name} argument"),
    (
        "diag.command.bad-argument",
        "{id}: {name} is not valid: {reason}",
    ),
    (
        "diag.command.unauthorized",
        "{id} has the {effect} effect, which is not authorized",
    ),
    ("diag.command.no-such-command", "there is no command {id}"),
    ("diag.command.not-an-id", "{id} is not a command id"),
];

/// The process's own lines, keyed by where they are printed.
pub const SHELL: [(&str, &str); 5] = [
    ("shell.version", "rustage {version}"),
    (
        "shell.status.not-implemented",
        "The desktop designer is not implemented yet.",
    ),
    (
        "shell.status.icon-strip",
        "The icon strip is: run `rustage --render-icons <directory>` to render it.",
    ),
    (
        "shell.status.probes",
        "The dependency contract probes are: run `rustage --probe-apis <directory>`.",
    ),
    ("shell.error.prefix", "rustage: {message}"),
];

/// Every identified interface string, in key order.
///
/// The order is by key because the result is drilled into a [`Pack`], which is ordered anyway; a
/// caller that wants source order can group by [`Entry::source`].
pub fn entries() -> Vec<Entry> {
    let mut found = Vec::new();

    for command in registry().commands() {
        let id = command.id().to_string();
        push(
            &mut found,
            format!("cmd.{id}.title"),
            command.title(),
            Source::Command,
        );
        push(
            &mut found,
            format!("cmd.{id}.category"),
            command.category(),
            Source::Command,
        );
        push(
            &mut found,
            format!("cmd.{id}.description"),
            command.help(),
            Source::Command,
        );
        for param in command.params() {
            push(
                &mut found,
                format!("cmd.{id}.param.{}.help", param.name()),
                param.help(),
                Source::Command,
            );
        }
    }

    for menu in menus() {
        push(
            &mut found,
            format!("menu.{}.title", menu.id()),
            menu.title(),
            Source::Menu,
        );
        walk(&mut found, &format!("menu.{}", menu.id()), menu.items());
    }

    for (key, english) in TEMPLATES {
        push(&mut found, key.to_owned(), english, Source::Diagnostic);
    }
    for (key, english) in SHELL {
        push(&mut found, key.to_owned(), english, Source::Shell);
    }

    found.sort_by(|left, right| left.key.cmp(&right.key));
    found
}

/// Records one entry.
fn push(entries: &mut Vec<Entry>, key: String, english: &str, source: Source) {
    entries.push(Entry {
        key,
        english: english.to_owned(),
        source,
    });
}

/// Walks a menu's items, keying each label.
fn walk(entries: &mut Vec<Entry>, prefix: &str, items: &[Item]) {
    for item in items {
        match item {
            Item::Command { title, command } | Item::Planned { title, command, .. } => {
                push(
                    entries,
                    format!("{prefix}.item.{command}"),
                    title,
                    Source::Menu,
                );
            }
            Item::Submenu { id, title, items } => {
                let child = format!("{prefix}.submenu.{id}");
                push(entries, format!("{child}.title"), title, Source::Menu);
                walk(entries, &child, items);
            }
            Item::Separator => {}
        }
    }
}

/// The pack for a language.
///
/// Only a bundled language has messages; an unbundled one is refused by
/// [`crate::app::locale::catalog::Catalog::for_language`] before it reaches here, so this answers an
/// empty pack rather than inventing text.
///
/// The pack is derived on demand, which is right for the command line and for the reports that are
/// the vocabulary's only consumers today. A windowed consumer that looks text up per frame would hold
/// one [`Catalog`] instead of deriving one per lookup — that is T24.06's concern, and it is stated
/// here so the choice is not mistaken for an oversight.
pub fn pack(language: Language) -> Pack {
    let mut pack = Pack::new(language);
    if language.bundled() {
        for entry in entries() {
            pack.insert(entry.key, entry.english);
        }
    }
    pack
}

/// What the vocabulary covers, and where a label is missing.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Coverage {
    /// How many strings are identified.
    pub entries: usize,
    /// How many commands are registered.
    pub commands: usize,
    /// How many menus are declared.
    pub menus: usize,
    /// How many key bindings are declared.
    pub bindings: usize,
    /// Interface elements with no key, which must be empty.
    pub uncovered: Vec<String>,
}

impl Coverage {
    /// Whether every interface element has a key.
    pub fn is_complete(&self) -> bool {
        self.uncovered.is_empty()
    }

    /// The coverage in one line.
    pub fn render(&self) -> String {
        format!(
            "{} string(s): {} command(s), {} menu(s), {} binding(s); {} uncovered",
            self.entries,
            self.commands,
            self.menus,
            self.bindings,
            self.uncovered.len()
        )
    }
}

/// Measures what the vocabulary covers.
///
/// The check is a cross-check rather than a count: an entry that is *derived* from a table cannot be
/// missing from itself, so what is measured is whether the three tables agree — every menu leaf and
/// every key binding names a command the registry knows, and every command has the keys its own
/// vocabulary is built from. That is the "no omissions" T24.03 asks for, over the surfaces that exist.
pub fn coverage() -> Coverage {
    let identified = entries();
    let has = |key: String| identified.iter().any(|entry| entry.key == key);
    let registry = registry();
    let mut uncovered = Vec::new();

    for command in registry.commands() {
        let id = command.id().to_string();
        for suffix in ["title", "description", "category"] {
            let key = format!("cmd.{id}.{suffix}");
            if !has(key.clone()) {
                uncovered.push(key);
            }
        }
        for param in command.params() {
            let key = format!("cmd.{id}.param.{}.help", param.name());
            if !has(key.clone()) {
                uncovered.push(key);
            }
        }
    }

    let known = |id: &str| registry.why_not_exists(id).is_none();
    for menu in menus() {
        for (_, item) in menu.leaves() {
            // A leaf that claims a command must name one that exists; a *planned* leaf is the honest
            // opposite — it names the command a later task will register — so it is not required to.
            if let Item::Command { command, .. } = item
                && !known(command)
            {
                uncovered.push(format!("menu {} names {command}", menu.id()));
            }
        }
    }

    let bindings = keymap();
    for binding in bindings.bindings() {
        let command = binding.command();
        if !known(command) {
            uncovered.push(format!("a key binding names {command}"));
        }
        let key = format!("cmd.{command}.title");
        if !has(key.clone()) {
            uncovered.push(key);
        }
    }

    Coverage {
        entries: identified.len(),
        commands: registry.len(),
        menus: menus().len(),
        bindings: bindings.bindings().len(),
        uncovered,
    }
}

#[cfg(test)]
mod tests {
    use super::{SHELL, Source, TEMPLATES, coverage, entries, pack};
    use crate::app::locale::catalog::{Catalog, Pack};
    use crate::host::locale::Language;

    #[test]
    fn every_identified_string_has_a_unique_key_and_non_empty_text() {
        let mut seen = std::collections::BTreeSet::new();
        for entry in entries() {
            assert!(
                seen.insert(entry.key.clone()),
                "duplicate key {}",
                entry.key
            );
            assert!(
                !entry.english.trim().is_empty(),
                "{} has no text",
                entry.key
            );
        }
    }

    #[test]
    fn the_command_registry_is_identified_by_its_own_ids() {
        let keys: Vec<String> = entries().into_iter().map(|entry| entry.key).collect();
        assert!(keys.contains(&"cmd.file.save.title".to_owned()));
        assert!(keys.contains(&"cmd.file.save.category".to_owned()));
        assert!(keys.contains(&"cmd.file.save.description".to_owned()));
        assert!(keys.contains(&"cmd.file.close.param.decision.help".to_owned()));
    }

    #[test]
    fn the_menu_tree_is_identified_by_command_and_menu_ids() {
        let keys: Vec<String> = entries().into_iter().map(|entry| entry.key).collect();
        assert!(keys.contains(&"menu.file.title".to_owned()));
        assert!(keys.contains(&"menu.file.item.file.new-project".to_owned()));
        assert!(keys.contains(&"menu.view.submenu.panels.title".to_owned()));
        assert!(keys.contains(&"menu.view.submenu.panels.item.panel.outline".to_owned()));
    }

    #[test]
    fn the_diagnostics_and_shell_lines_are_identified() {
        let keys: Vec<String> = entries().into_iter().map(|entry| entry.key).collect();
        for (key, _) in TEMPLATES {
            assert!(keys.contains(&key.to_owned()), "{key}");
        }
        for (key, _) in SHELL {
            assert!(keys.contains(&key.to_owned()), "{key}");
        }
    }

    #[test]
    fn every_source_appears() {
        let sources: Vec<Source> = entries().into_iter().map(|entry| entry.source).collect();
        for source in [
            Source::Command,
            Source::Menu,
            Source::Diagnostic,
            Source::Shell,
        ] {
            assert!(
                sources.contains(&source),
                "{} is not represented",
                source.name()
            );
        }
    }

    #[test]
    fn the_english_pack_answers_every_identified_key() {
        let catalog = Catalog::from_pack(pack(Language::English));
        for entry in entries() {
            assert!(
                catalog.text(&entry.key).is_found(),
                "{} is not in the pack",
                entry.key
            );
        }
        assert!(catalog.pack().len() >= entries().len());
    }

    #[test]
    fn an_unbundled_language_gets_no_invented_text() {
        assert_eq!(
            pack(Language::SimplifiedChinese),
            Pack::new(Language::SimplifiedChinese)
        );
        assert!(pack(Language::SimplifiedChinese).is_empty());
    }

    #[test]
    fn the_coverage_over_the_real_tables_has_no_omission() {
        let coverage = coverage();
        assert!(coverage.is_complete(), "{:?}", coverage.uncovered);
        assert!(coverage.commands > 0);
        assert!(coverage.menus > 0);
        assert!(coverage.bindings > 0);
        assert!(coverage.entries >= coverage.commands * 3);
        assert!(coverage.render().contains("uncovered"));
    }
}
