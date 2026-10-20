// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The command palette's state (task T05.04).
//!
//! The model is deliberately separate from the widgets: [`crate::app::commands::palette`] owns
//! fuzzy search, recents and help, and this type owns which query is open, which row is selected
//! and what was focused before the palette took the keyboard. The workbench window only mirrors
//! this state into real controls, so a palette row and the command it names cannot drift while a
//! user types.

use crate::app::commands::command::{Context, Registry};
use crate::app::commands::palette::{Entry, Recents, help_for, search};

/// The command palette as the workbench window holds it.
#[derive(Default)]
pub(super) struct Palette {
    open: bool,
    query: String,
    entries: Vec<Entry>,
    selected: usize,
    previous_focus: Option<u64>,
}

impl Palette {
    /// A closed palette with no query.
    pub(super) fn new() -> Self {
        Self::default()
    }

    /// Whether the palette currently owns the keyboard.
    pub(super) fn is_open(&self) -> bool {
        self.open
    }

    /// Opens the palette with an empty query and remembers where the keyboard was.
    ///
    /// The previous focus is kept even when it names a palette control, and the caller decides
    /// whether restoring it is meaningful; the palette itself must not guess another widget.
    pub(super) fn open(
        &mut self,
        previous_focus: Option<u64>,
        registry: &Registry,
        context: &Context,
        recents: &Recents,
    ) {
        self.open = true;
        self.query.clear();
        self.selected = 0;
        self.previous_focus = previous_focus;
        self.refresh(registry, context, recents);
    }

    /// Closes the palette and returns the widget that had the keyboard before it opened.
    pub(super) fn close(&mut self) -> Option<u64> {
        self.open = false;
        self.previous_focus.take()
    }

    /// Replaces the query and recomputes the rows.
    pub(super) fn set_query(
        &mut self,
        query: &str,
        registry: &Registry,
        context: &Context,
        recents: &Recents,
    ) {
        self.query = query.to_owned();
        self.selected = 0;
        self.refresh(registry, context, recents);
    }

    /// Recomputes the rows for the current query.
    pub(super) fn refresh(&mut self, registry: &Registry, context: &Context, recents: &Recents) {
        self.entries = search(registry, context, &self.query, recents);
        if self.selected >= self.entries.len() {
            self.selected = 0;
        }
    }

    /// The rows as the list control shows them, bounded so a large registry cannot build an
    /// unbounded widget list in one keystroke.
    pub(super) fn rows(&self) -> Vec<String> {
        const LIMIT: usize = 50;
        if self.entries.is_empty() {
            return vec!["(no matching command)".to_owned()];
        }
        self.entries.iter().take(LIMIT).map(Entry::render).collect()
    }

    /// How many commands matched the current query.
    pub(super) fn result_count(&self) -> usize {
        self.entries.len()
    }

    /// The selected entry, when there is one.
    pub(super) fn selected_entry(&self) -> Option<&Entry> {
        self.entries.get(self.selected)
    }

    /// Selects `index`, within bounds. Returns whether the selection moved.
    pub(super) fn select(&mut self, index: usize) -> bool {
        if index >= self.entries.len() || index == self.selected {
            return false;
        }
        self.selected = index;
        true
    }

    /// The index the list shows as current.
    pub(super) fn selected(&self) -> usize {
        self.selected
    }

    /// One line of help for the selected row: the command's own parameters when it can run,
    /// or the registry's own disabled reason when it cannot.
    pub(super) fn help(&self, registry: &Registry) -> String {
        let Some(entry) = self.selected_entry() else {
            return "No command matches this query.".to_owned();
        };
        if let Some(reason) = entry.reason() {
            return format!("{} — disabled: {reason}", entry.id());
        }
        match help_for(registry, entry.id()) {
            Ok(help) if help.params().is_empty() => {
                format!("{} — {}", help.id(), help.title())
            }
            Ok(help) => {
                let params = help
                    .params()
                    .iter()
                    .map(|param| format!("{}:{}", param.name(), param.kind()))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{} — {} · {params}", help.id(), help.title())
            }
            Err(reason) => format!("{} — {reason}", entry.id()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Palette;
    use crate::app::commands::command::{Context, built_in};
    use crate::app::commands::palette::Recents;
    use crate::contracts::permission::EffectClass;

    fn context() -> Context {
        Context::in_area("design")
            .trusted()
            .authorizing(EffectClass::Command)
    }

    #[test]
    fn an_empty_query_lists_commands_and_a_query_narrows_them() {
        let registry = built_in();
        let recents = Recents::new(8);
        let mut palette = Palette::new();
        palette.open(None, &registry, &context(), &recents);
        assert!(palette.open && palette.rows().len() > 1);

        palette.set_query("welcome", &registry, &context(), &recents);
        assert_eq!(
            palette.selected_entry().map(|entry| entry.id()),
            Some("view.welcome")
        );
        assert!(palette.help(&registry).contains("Welcome"));
    }

    #[test]
    fn a_disabled_command_says_why_and_is_not_executed() {
        let registry = built_in();
        let recents = Recents::new(8);
        let mut palette = Palette::new();
        let untrusted = Context::in_area("design");
        palette.open(None, &registry, &untrusted, &recents);
        palette.set_query("build", &registry, &untrusted, &recents);
        let entry = palette.selected_entry().expect("a match");
        assert_eq!(entry.id(), "build.run");
        assert!(!entry.is_enabled());
        assert!(palette.help(&registry).contains("disabled"));
        assert!(palette.help(&registry).contains("trusted"));
    }

    #[test]
    fn a_recent_command_is_lifted_without_hiding_unseen_ones() {
        let registry = built_in();
        let mut recents = Recents::new(8);
        recents.record("file.save");
        let mut palette = Palette::new();
        palette.open(None, &registry, &context(), &recents);
        assert_eq!(
            palette.selected_entry().map(|entry| entry.id()),
            Some("file.save")
        );
        assert!(palette.rows().len() > 1);
    }

    #[test]
    fn selection_stays_inside_the_rows() {
        let registry = built_in();
        let recents = Recents::new(8);
        let mut palette = Palette::new();
        palette.open(None, &registry, &context(), &recents);
        assert!(palette.select(1));
        assert_eq!(palette.selected(), 1);
        assert!(!palette.select(usize::MAX));
        assert_eq!(palette.selected(), 1);
    }
}
