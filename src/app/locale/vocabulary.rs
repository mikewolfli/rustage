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

use std::sync::OnceLock;

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
    /// A label the workbench window paints (menus, toolbar, panel titles, buttons).
    Window,
}

impl Source {
    /// The source's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Command => "command",
            Self::Menu => "menu",
            Self::Diagnostic => "diagnostic",
            Self::Shell => "shell",
            Self::Window => "window",
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
/// The keys of the diagnostic templates, in the order the report lists them.
pub const TEMPLATE_KEYS: [&str; 7] = [
    "diag.command.unknown",
    "diag.command.disabled",
    "diag.command.missing",
    "diag.command.bad-argument",
    "diag.command.unauthorized",
    "diag.command.no-such-command",
    "diag.command.not-an-id",
];

/// The workbench window's own labels, keyed by what they name.
///
/// The keys of the workbench window's labels.
pub const WINDOW_KEYS: [&str; 115] = [
    "ui.panel.pages",
    "ui.panel.outline",
    "ui.panel.events",
    "ui.panel.inspector",
    "ui.panel.data",
    "ui.panel.messages",
    "ui.panel.problems",
    "ui.panel.debug",
    "ui.menu.quit",
    "ui.menu.layout",
    "ui.menu.editor",
    "ui.menu.save-layout",
    "ui.menu.reset-layout",
    "ui.menu.split-right",
    "ui.menu.next-group",
    "ui.toolbar.commands",
    "ui.toolbar.split",
    "ui.toolbar.next-group",
    "ui.toolbar.save-layout",
    "ui.toolbar.reset-layout",
    "ui.button.add",
    "ui.button.open",
    "ui.button.back",
    "ui.button.forward",
    "ui.button.close",
    "ui.button.clear",
    "ui.button.reset",
    "ui.button.values",
    "ui.button.theme",
    "ui.button.bind",
    "ui.button.unbind",
    "ui.button.fire",
    "ui.button.start",
    "ui.button.run",
    "ui.button.step",
    "ui.button.stop",
    "ui.placeholder.page-name",
    "ui.placeholder.handler-name",
    "ui.placeholder.theme-name",
    "ui.placeholder.mixed",
    "ui.inspector.node",
    "ui.inspector.kind",
    "ui.inspector.theme",
    "ui.inspector.default",
    "ui.inspector.accepted",
    "ui.inspector.inspect",
    "ui.inspector.selection",
    "ui.data.dataset",
    "ui.data.diagnostic",
    "ui.context.clear",
    "ui.context.reset",
    "ui.context.inspect",
    "ui.context.select-parent",
    "ui.context.toggle-hide",
    "ui.context.toggle-lock",
    "ui.context.raise",
    "ui.context.editor.close",
    "ui.context.editor.close-others",
    "ui.context.editor.close-right",
    "ui.context.editor.split",
    "ui.context.editor.next-group",
    "ui.context.diagnostic.copy",
    "ui.context.diagnostic.copy-all",
    "ui.context.diagnostic.clear",
    "ui.status.ready",
    "ui.status.no-document",
    "ui.status.no-problems",
    "ui.status.command-palette",
    "ui.status.palette-closed",
    "ui.status.palette-could-not-focus",
    "ui.status.hidden",
    "ui.status.shown",
    "ui.status.no-panel",
    "ui.status.layout-reset",
    "ui.status.layout-saved",
    "ui.status.layout-not-saved",
    "ui.status.no-layout-path",
    "ui.status.editor-split",
    "ui.status.focus-group",
    "ui.status.closing",
    "ui.status.unknown-action",
    "ui.status.no-parent",
    "ui.status.locked",
    "ui.status.unlocked",
    "ui.status.hidden-node",
    "ui.status.shown-node",
    "ui.status.raised",
    "ui.status.already-top",
    "ui.status.inspector-cleared",
    "ui.status.reset-nodes",
    "ui.status.inspected",
    "ui.status.theme-applied",
    "ui.status.data-applied",
    "ui.status.page-declared",
    "ui.status.navigation-declared",
    "ui.status.now-on",
    "ui.status.bound",
    "ui.status.unbound",
    "ui.error.schema-unavailable",
    "ui.error.contracts-differ",
    "ui.error.contracts",
    "ui.error.not-applied",
    "ui.error.inspect-refused",
    "ui.error.theme-refused",
    "ui.error.not-added",
    "ui.error.not-opened",
    "ui.error.not-moved",
    "ui.error.not-closed",
    "ui.error.not-bound",
    "ui.error.not-unbound",
    "ui.error.not-fired",
    "ui.note.inspector",
    "ui.note.data",
    "ui.note.events",
    "ui.note.pages",
];

/// The keys of the process's own lines, in the order the report lists them.
pub const SHELL_KEYS: [&str; 5] = [
    "shell.version",
    "shell.status.not-implemented",
    "shell.status.icon-strip",
    "shell.status.probes",
    "shell.error.prefix",
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

    for (keys, source) in [
        (TEMPLATE_KEYS.as_slice(), Source::Diagnostic),
        (SHELL_KEYS.as_slice(), Source::Shell),
        (WINDOW_KEYS.as_slice(), Source::Window),
    ] {
        for key in keys {
            push(&mut found, (*key).to_owned(), english_of(key), source);
        }
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

/// The directory the JSON language packs live in.
///
/// The packs are runtime data, not compiled into the binary: a translator edits
/// `language/<tag>.json` and a hot reload picks it up, which is the upstream convention the
/// project follows (blue21 L-1/L-4). The lookup order is:
///
/// 1. `RUSTAGE_LANGUAGE_DIR`, for a packaged build or a test with its own fixtures;
/// 2. `language/` under the working directory, which is how `cargo run` finds the checkout;
/// 3. `language/` next to the executable, which is how an installed bundle finds its data.
///
/// None of these existing is not a silent fallback: [`pack`] logs the missing file and answers an
/// empty pack, which the completeness tests fail on and the interface shows as its keys.
pub fn language_dir() -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("RUSTAGE_LANGUAGE_DIR") {
        return std::path::PathBuf::from(dir);
    }
    let local = std::path::PathBuf::from("language");
    if local.is_dir() {
        return local;
    }
    if let Ok(executable) = std::env::current_exe()
        && let Some(parent) = executable.parent()
    {
        let beside = parent.join("language");
        if beside.is_dir() {
            return beside;
        }
    }
    local
}

/// The path of one language's JSON pack.
pub fn pack_path(language: Language) -> std::path::PathBuf {
    language_dir().join(format!("{}.json", language.tag()))
}

/// The decoded pack for a language, read from its JSON file once.
fn loaded(language: Language) -> &'static Pack {
    static ENGLISH: OnceLock<Pack> = OnceLock::new();
    static CHINESE: OnceLock<Pack> = OnceLock::new();
    let slot: &'static OnceLock<Pack> = match language {
        Language::English => &ENGLISH,
        Language::SimplifiedChinese => &CHINESE,
    };
    slot.get_or_init(|| {
        let path = pack_path(language);
        match std::fs::read_to_string(&path) {
            Ok(text) => match Pack::decode(&text) {
                Ok(pack) => pack,
                Err(fault) => {
                    crate::host::log::emit(
                        crate::host::log::Level::Warn,
                        "app::locale::vocabulary",
                        &format!(
                            "the {} pack at {} is malformed: {}",
                            language.tag(),
                            path.display(),
                            fault.message()
                        ),
                    );
                    Pack::new(language)
                }
            },
            Err(error) => {
                crate::host::log::emit(
                    crate::host::log::Level::Warn,
                    "app::locale::vocabulary",
                    &format!(
                        "the {} pack could not be read from {}: {error}",
                        language.tag(),
                        path.display()
                    ),
                );
                Pack::new(language)
            }
        }
    })
}

/// The English text for a key, read from the English pack.
fn english_of(key: &'static str) -> &'static str {
    loaded(Language::English).get(key).unwrap_or(key)
}

/// The pack for a language, read from its JSON language file under [`language_dir`].
///
/// Only a bundled language has messages; an unbundled one is refused by
/// [`crate::app::locale::catalog::Catalog::for_language`] before it reaches here, so this answers an
/// empty pack rather than inventing text.
pub fn pack(language: Language) -> Pack {
    if language.bundled() {
        loaded(language).clone()
    } else {
        Pack::new(language)
    }
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
    use super::{SHELL_KEYS, Source, TEMPLATE_KEYS, WINDOW_KEYS, coverage, entries, pack};
    use crate::app::locale::catalog::{Catalog, Text};
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
        for key in TEMPLATE_KEYS {
            assert!(keys.contains(&key.to_owned()), "{key}");
        }
        for key in SHELL_KEYS {
            assert!(keys.contains(&key.to_owned()), "{key}");
        }
        for key in WINDOW_KEYS {
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
            Source::Window,
        ] {
            assert!(
                sources.contains(&source),
                "{} is not represented",
                source.name()
            );
        }
    }

    #[test]
    fn the_english_pack_is_the_identified_vocabulary_not_a_copy() {
        let catalog = Catalog::from_pack(pack(Language::English));
        let identified = entries();
        for entry in &identified {
            assert_eq!(
                catalog.text(&entry.key),
                Text::Found(entry.english.as_str()),
                "{} differs from the registry-derived English",
                entry.key
            );
        }
        assert_eq!(catalog.pack().len(), identified.len());
    }

    #[test]
    fn the_chinese_pack_translates_every_identified_key() {
        let catalog = Catalog::from_pack(pack(Language::SimplifiedChinese));
        for entry in entries() {
            assert!(
                catalog.text(&entry.key).is_found(),
                "{} has no Chinese text",
                entry.key
            );
        }
        // No extra key: the two tables are the same set, not merely a covering pair.
        assert_eq!(catalog.pack().len(), entries().len());
    }

    #[test]
    fn the_chinese_pack_keeps_every_placeholder() {
        fn placeholders(text: &str) -> std::collections::BTreeSet<String> {
            let mut found = std::collections::BTreeSet::new();
            let mut rest = text;
            while let Some(start) = rest.find('{') {
                let after = &rest[start + 1..];
                let Some(end) = after.find('}') else { break };
                found.insert(after[..end].to_owned());
                rest = &after[end + 1..];
            }
            found
        }

        let english = Catalog::from_pack(pack(Language::English));
        let chinese = Catalog::from_pack(pack(Language::SimplifiedChinese));
        for (key, text) in chinese.pack().iter() {
            assert!(!text.trim().is_empty(), "{key} has no text");
            let Some(source) = english.pack().get(key) else {
                panic!("{key} is translated but not identified");
            };
            assert_eq!(
                placeholders(text),
                placeholders(source),
                "{key} changes its placeholders"
            );
        }
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
