// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The application-structure panel's rows (roadmap F26, `panel.application`).
//!
//! The panel describes the workbench's own chrome as it is built: the menu tree from the command
//! model, the toolbar items the window created, the status line's current text, and the four
//! context-menu groups. Each input is the real structure the window uses, not a second copy:
//!
//! * **Menus** come from [`crate::app::commands::menu::built_in`] and each menu's own `render`,
//!   so a leaf that runs a command and a leaf that only names a future task stay distinguishable.
//! * **Toolbar** entries are the `(label, action)` pairs the window mounted.
//! * **Status** is the line the status bar shows right now.
//! * **Context menus** are the key lists the window's four right-click menus are built from.
//!
//! The panel is read-only: it is a description of the chrome, and every action still goes through
//! the menu, toolbar or context menu that owns it.

use crate::app::commands::menu::Menu;

/// The application-structure rows.
pub(super) fn rows(
    menus: &[Menu],
    toolbar: &[(String, String)],
    status: &str,
    context_menus: &[(String, Vec<String>)],
) -> Vec<String> {
    let mut rows = vec![format!("menus: {}", menus.len())];
    for menu in menus {
        rows.push(format!("menu {} ({})", menu.title(), menu.id()));
        rows.extend(menu.render().into_iter().map(|line| format!("  {line}")));
    }
    rows.push(format!("toolbar: {} item(s)", toolbar.len()));
    for (label, action) in toolbar {
        rows.push(format!("  {label} -> {action}"));
    }
    rows.push(format!("status: {status}"));
    rows.push(format!("context menus: {}", context_menus.len()));
    for (group, labels) in context_menus {
        rows.push(format!("  {group} ({} item(s))", labels.len()));
        rows.extend(labels.iter().map(|label| format!("    {label}")));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::rows;
    use crate::app::commands::menu::{Item, Menu};

    #[test]
    fn every_chrome_section_is_reported() {
        let menus = vec![Menu::new(
            "file",
            "File",
            vec![
                Item::command("Save", "file.save"),
                Item::submenu(
                    "edit",
                    "Edit",
                    vec![Item::planned("Undo", "edit.undo", "T05.02")],
                ),
            ],
        )];
        let toolbar = vec![("Commands".to_owned(), "palette:open".to_owned())];
        let context_menus = vec![(
            "inspector".to_owned(),
            vec!["Clear field".to_owned(), "Reset".to_owned()],
        )];
        let lines = rows(&menus, &toolbar, "ready", &context_menus);

        assert!(lines[0].contains("menus: 1"), "{lines:?}");
        assert!(
            lines.iter().any(|line| line.contains("file.save")),
            "{lines:?}"
        );
        assert!(
            lines.iter().any(|line| line.contains("planned: T05.02")),
            "{lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Commands -> palette:open")),
            "{lines:?}"
        );
        assert!(
            lines.iter().any(|line| line == "status: ready"),
            "{lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("inspector (2 item(s))")),
            "{lines:?}"
        );
        assert!(lines.iter().all(|line| !line.is_empty()), "{lines:?}");
    }
}
