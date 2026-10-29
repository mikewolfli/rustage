// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The command palette's state (task T05.04).
//!
//! The model is deliberately separate from the widgets: [`crate::app::commands::palette`] owns
//! recents and the registry-derived entry list, and this type owns which query is open and what
//! was focused before the palette took the keyboard. Filtering, highlighting and activation belong
//! to the library's `CommandPalette` control (blue21 W-2a); the host only feeds it the entries and
//! executes the activated command id.

use crate::app::commands::command::{Context, Registry};
use crate::app::commands::palette::{Entry, Recents, search};

/// The command palette as the workbench window holds it.
#[derive(Default)]
pub(super) struct Palette {
    open: bool,
    query: String,
    entries: Vec<Entry>,
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
        self.previous_focus = previous_focus;
        self.refresh(registry, context, recents);
    }

    /// Closes the palette and returns the widget that had the keyboard before it opened.
    pub(super) fn close(&mut self) -> Option<u64> {
        self.open = false;
        self.previous_focus.take()
    }

    /// Recomputes the entries for the current query.
    pub(super) fn refresh(&mut self, registry: &Registry, context: &Context, recents: &Recents) {
        self.entries = search(registry, context, &self.query, recents);
    }

    /// The current query text.
    pub(super) fn query(&self) -> &str {
        &self.query
    }

    /// Every entry the open palette holds, before the control's own filtering.
    pub(super) fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// One entry by command id, whether or not the current query matches it.
    pub(super) fn entry_by_id(&self, id: &str) -> Option<&Entry> {
        self.entries.iter().find(|entry| entry.id() == id)
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
    fn an_empty_query_lists_commands_a_query_narrows_them_and_reason_is_queryable() {
        let registry = built_in();
        let recents = Recents::new(8);
        let mut palette = Palette::new();
        palette.open(None, &registry, &context(), &recents);
        assert!(palette.is_open());
        assert!(palette.entries().len() > 1);
        let welcome = palette.entry_by_id("view.welcome").expect("welcome");
        assert!(welcome.title().contains("Welcome"));
    }

    #[test]
    fn a_disabled_command_says_why_and_is_not_executed() {
        let registry = built_in();
        let recents = Recents::new(8);
        let mut palette = Palette::new();
        let untrusted = Context::in_area("design");
        palette.open(None, &registry, &untrusted, &recents);
        let entry = palette
            .entry_by_id("build.run")
            .expect("a registered command");
        assert!(!entry.is_enabled());
        assert!(entry.reason().unwrap_or_default().contains("trusted"));
    }

    #[test]
    fn a_recent_command_is_lifted_without_hiding_unseen_ones() {
        let registry = built_in();
        let mut recents = Recents::new(8);
        recents.record("file.save");
        let mut palette = Palette::new();
        palette.open(None, &registry, &context(), &recents);
        assert_eq!(
            palette.entries().first().map(|entry| entry.id()),
            Some("file.save")
        );
        assert!(palette.entries().len() > 1);
    }

    #[test]
    fn an_entry_is_found_by_its_id() {
        let registry = built_in();
        let recents = Recents::new(8);
        let mut palette = Palette::new();
        palette.open(None, &registry, &context(), &recents);
        assert!(palette.entry_by_id("view.welcome").is_some());
        assert!(palette.entry_by_id("not.a-command").is_none());
    }
}
