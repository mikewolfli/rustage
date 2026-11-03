// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The rows of the `panel.actions` panel (roadmap §2.2, F26).
//!
//! The roadmap names four facts for this panel: shared actions, shortcuts, status, and event
//! targets and referrers. Only the first two are held by the two models this module is given, so
//! only those two are rendered:
//!
//! * **Shared actions** come from a [`crate::app::commands::command::Registry`]: one row per
//!   [`Command`] with its id, category, title and — when it declares any — its parameter names.
//! * **Shortcuts** come from a [`crate::app::commands::shortcuts::Keymap`]: one row per [`Binding`]
//!   with its rendered chord, its scope and the command it runs. The scope is rendered so a mode-
//!   or panel-specific chord is distinguishable from a global one.
//!
//! Two facts are deliberately absent rather than invented:
//!
//! * **Status** is not a field of [`Command`]. Availability is answered per call by
//!   [`crate::app::commands::command::Registry::why_not`], against a
//!   [`crate::app::commands::command::Context`] the panel does not hold; a row here would have to
//!   guess that context, so no status row is emitted.
//! * **Event targets and referrers** are in neither model. They belong to the design's event
//!   registry, not to the command registry or the keymap.
//!
//! Rows are deterministic: the summary first, then one row per command in the registry's id order,
//! then the binding rows sorted by rendered chord (with the scope and command as tie-breakers).

use crate::app::commands::command::Command;
use crate::app::commands::shortcuts::Binding;

/// The `panel.actions` rows: a summary, the shared commands, then the shortcuts.
///
/// A binding may name a command that is not registered — a user override is allowed to do that —
/// and is still shown: the row reports the keymap as it is. The summary's "bound" count is the
/// intersection, because it answers how many *registered* commands a key reaches.
pub(super) fn rows(
    registry: &crate::app::commands::command::Registry,
    keymap: &crate::app::commands::shortcuts::Keymap,
) -> Vec<String> {
    let bindings = keymap.bindings();
    let bound = registry
        .commands()
        .filter(|command| {
            let id = command.id().to_string();
            bindings
                .iter()
                .any(|binding| binding.command() == id.as_str())
        })
        .count();

    let mut rows = Vec::with_capacity(registry.len() + keymap.len() + 1);
    rows.push(format!(
        "summary: {} commands · {} bindings · {} commands bound",
        registry.len(),
        keymap.len(),
        bound
    ));
    for command in registry.commands() {
        rows.push(action_row(command));
    }
    // Every row starts with the same prefix, so sorting the rendered rows is sorting by chord and
    // then by scope and command: a stable order without depending on `bindings()`'s order.
    let mut shortcuts: Vec<String> = bindings.iter().map(shortcut_row).collect();
    shortcuts.sort();
    rows.extend(shortcuts);
    rows
}

/// One shared action: its id, category and title, plus its parameter names when it has any.
fn action_row(command: &Command) -> String {
    let params: Vec<&str> = command.params().iter().map(|param| param.name()).collect();
    if params.is_empty() {
        format!(
            "action: {} · {} · {}",
            command.id(),
            command.category(),
            command.title()
        )
    } else {
        format!(
            "action: {} · {} · {} · params {}",
            command.id(),
            command.category(),
            command.title(),
            params.join(", ")
        )
    }
}

/// One shortcut: the chord, the scope it applies in and the command it runs.
fn shortcut_row(binding: &Binding) -> String {
    format!(
        "shortcut: {} · {} · {}",
        binding.chord(),
        binding.scope().render(),
        binding.command()
    )
}

#[cfg(test)]
mod tests {
    use super::rows;
    use crate::app::commands::command::built_in;
    use crate::app::commands::shortcuts::{Binding, Chord, Keymap, Scope};

    fn keymap() -> Keymap {
        let mut keymap = Keymap::new();
        keymap
            .add(Binding::new(
                Chord::parse("Ctrl+S").expect("a valid chord"),
                "file.save",
                Scope::Global,
            ))
            .expect("a distinct global chord");
        keymap
            .add(Binding::new(
                Chord::parse("Ctrl+Alt+E").expect("a valid chord"),
                "tree.activate",
                Scope::Mode("design".to_owned()),
            ))
            .expect("a distinct mode chord");
        keymap
    }

    #[test]
    fn the_summary_names_the_command_and_binding_counts_and_how_many_are_bound() {
        let registry = built_in();
        let keymap = keymap();
        let rows = rows(&registry, &keymap);
        let summary = rows.first().expect("a summary row");
        assert!(summary.contains(&format!("{} commands", registry.len())));
        assert!(summary.contains(&format!("{} bindings", keymap.len())));
        let bound = registry
            .commands()
            .filter(|command| {
                let id = command.id().to_string();
                keymap
                    .bindings()
                    .iter()
                    .any(|binding| binding.command() == id.as_str())
            })
            .count();
        assert!(summary.contains(&format!("{bound} commands bound")));
    }

    #[test]
    fn a_known_command_appears_with_its_category_and_title_and_parameters() {
        let registry = built_in();
        let rows = rows(&registry, &Keymap::new());
        let close_row = rows
            .iter()
            .find(|row| row.starts_with("action: file.close · "))
            .expect("the close command");
        assert!(close_row.contains("File"));
        assert!(close_row.contains("Close"));
        let close = rows
            .iter()
            .find(|row| row.starts_with("action: file.close · "))
            .expect("the close command");
        assert!(close.contains("decision"));
    }

    #[test]
    fn a_shortcut_row_names_its_chord_scope_and_command() {
        let registry = built_in();
        let keymap = keymap();
        let rows = rows(&registry, &keymap);
        let global = rows
            .iter()
            .find(|row| row.starts_with("shortcut: Ctrl+S · "))
            .expect("the global binding");
        assert!(global.contains("file.save"));
        assert!(global.contains("global"));
        let mode = rows
            .iter()
            .find(|row| row.starts_with("shortcut: Ctrl+Alt+E · "))
            .expect("the mode binding");
        assert!(mode.contains("tree.activate"));
        assert!(mode.contains("mode:design"));
    }

    #[test]
    fn every_row_is_non_empty_and_the_shortcuts_are_sorted_by_chord() {
        let registry = built_in();
        let keymap = keymap();
        let rows = rows(&registry, &keymap);
        assert_eq!(rows.len(), 1 + registry.len() + keymap.len());
        assert!(rows.iter().all(|row| !row.trim().is_empty()));
        let shortcuts: Vec<&String> = rows
            .iter()
            .filter(|row| row.starts_with("shortcut: "))
            .collect();
        assert_eq!(shortcuts.len(), keymap.bindings().len());
        let mut sorted = shortcuts.clone();
        sorted.sort();
        assert_eq!(shortcuts, sorted);
    }
}
