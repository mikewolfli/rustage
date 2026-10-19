// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The workbench window: the first real integration of the shell models (task F04/F21).
//!
//! The models already exist — [`Workbench`] (mode, document, target, status), [`Panels`] (which
//! panels a host has and where), [`Groups`]/[`Tabs`] (editor groups and their documents), and
//! [`Layout`] (captured/restored visibility and arrangement). This module is the missing
//! presentation: a real window with a toolbar, docked list panels, editor tab strips, a status bar
//! and a native menu bar, all driven by those models rather than by a second state.
//!
//! # What this first slice does, and does not
//!
//! * **Does:** open a real 1280x800 window; show the built-in panels plus `events` and `data`;
//!   toggle a panel from the toolbar or the menu; save the current panels/groups to a layout file
//!   and load it on the next start; split the editor into two groups; switch the active tab;
//!   update the status bar; leave a bounded session on `--exit-after-ms`.
//! * **Does not:** drag-dock panels, float them, restore scroll positions per tab, or paint a
//!   full code editor (the editor area shows the active document's text lines in a list). Those
//!   are the next slices of the same layer, not a claim made here.
//!
//! The window is a real one and the widgets are the library's own painted controls: the modules
//! above are the state, `rust_widgets::app::WindowHandle` is the presentation, and every action
//! updates the model first and the widgets second.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rust_widgets::app::{ButtonHandle, WidgetHandle, WindowHandle};

use crate::app::shell::groups::{Axis, Groups};
use crate::app::shell::layout::Layout;
use crate::app::shell::panels::{Dock, PanelSpec, Panels, built_in};
use crate::app::shell::workbench::{Mode, Workbench};
use crate::host::platform::NativePlatform;
use crate::host::startup::{StartupPlatform, StartupRequest};

use super::report;

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 800;
const MENU_HEIGHT: u32 = 24;
const TOOLBAR_Y: i32 = 30;
const TOOLBAR_HEIGHT: u32 = 32;
const CONTENT_Y: i32 = 70;
const STATUS_Y: i32 = 768;
const STATUS_HEIGHT: u32 = 28;

/// The panels this window adds to the built-in four.
const EXTRA_PANELS: [(&str, &str, Dock); 2] = [
    ("events", "Events", Dock::Left),
    ("data", "Data & State", Dock::Right),
];

/// One editor group's widgets.
struct GroupView {
    tabs: Vec<u64>,
    editor: u64,
    /// The group's ids, so hiding a group hides its tab strip and editor together.
    all: Vec<u64>,
}

/// Everything the callbacks need. Kept behind one mutex so a button and the menu poller cannot
/// update the model and the widgets in different orders.
struct Gui {
    workbench: Workbench,
    panels: Panels,
    groups: Groups,
    default_layout: Layout,
    layout_path: Option<PathBuf>,
    panel_widgets: BTreeMap<String, Vec<u64>>,
    group_views: Vec<GroupView>,
    status: u64,
    message: String,
}

impl Gui {
    fn status_line(&self) -> String {
        let document = self.workbench.document().unwrap_or("no document");
        let target = self
            .workbench
            .status()
            .into_iter()
            .find(|item| item.label() == "target")
            .map(|item| item.value().to_owned())
            .unwrap_or_else(|| "-".to_owned());
        format!(
            "{} | {} | {} | {}",
            self.workbench.mode().name(),
            document,
            self.workbench.title(),
            target
        )
    }

    fn shown_panels(&self) -> BTreeSet<String> {
        self.panels.layout().shown().into_iter().collect()
    }

    fn sync_panels(&mut self) {
        let shown = self.shown_panels();
        for (id, widgets) in &self.panel_widgets {
            let visible = shown.contains(id);
            for widget in widgets {
                rust_widgets::set_widget_visible(*widget, visible);
            }
        }
    }

    fn sync_editors(&mut self) {
        layout_group_widgets(self);
        let groups: Vec<(u64, Vec<String>, String)> = self
            .groups
            .group_ids()
            .into_iter()
            .map(|group| {
                let tabs = self.groups.tabs(group).expect("a registered group");
                let documents: Vec<String> = tabs
                    .tabs()
                    .iter()
                    .map(|tab| tab.document().to_owned())
                    .collect();
                let text = tabs
                    .active()
                    .and_then(|tab| self.groups.text(tab.document()))
                    .unwrap_or("")
                    .to_owned();
                (group.get(), documents, text)
            })
            .collect();
        for (index, view) in self.group_views.iter_mut().enumerate() {
            let Some((_, documents, text)) = groups.get(index) else {
                for widget in &view.all {
                    rust_widgets::set_widget_visible(*widget, false);
                }
                continue;
            };
            for widget in &view.all {
                rust_widgets::set_widget_visible(*widget, true);
            }
            for (button, document) in view.tabs.iter().zip(documents.iter()) {
                let label = document.rsplit('/').next().unwrap_or(document);
                rust_widgets::set_widget_text(*button, label);
                rust_widgets::set_widget_visible(*button, true);
            }
            for button in view.tabs.iter().skip(documents.len()) {
                rust_widgets::set_widget_visible(*button, false);
            }
            rust_widgets::list_box_clear_items(view.editor);
            if text.is_empty() {
                let _ = rust_widgets::list_box_add_item(view.editor, "(the document is empty)");
            } else {
                for line in text.lines() {
                    let _ = rust_widgets::list_box_add_item(view.editor, line);
                }
            }
        }
        rust_widgets::set_widget_text(
            self.status,
            &format!("{} | {}", self.status_line(), self.message),
        );
    }
}

fn apply_action(gui: &mut Gui, action: &str) {
    if let Some(id) = action.strip_prefix("toggle:") {
        if gui.panels.layout().shown().iter().any(|shown| shown == id) {
            let _ = gui.panels.hide(id);
            gui.message = format!("hidden {id}");
        } else if gui.panels.show(id).is_ok() {
            gui.message = format!("shown {id}");
        } else {
            gui.message = format!("no panel {id}");
        }
        gui.sync_panels();
        gui.sync_editors();
        return;
    }
    match action {
        "layout:save" => match &gui.layout_path {
            Some(path) => {
                let layout = Layout::captured("workbench", &gui.groups, &gui.panels, false, false);
                match std::fs::write(path, layout.to_json()) {
                    Ok(()) => gui.message = format!("saved layout to {}", path.display()),
                    Err(error) => gui.message = format!("layout not saved: {error}"),
                }
            }
            None => gui.message = "no --layout path was given".to_owned(),
        },
        "layout:reset" => {
            let _ = gui.default_layout.apply(&mut gui.groups, &mut gui.panels);
            gui.message = "layout reset".to_owned();
            gui.sync_panels();
            gui.sync_editors();
        }
        "group:split" => {
            let document = gui
                .groups
                .tabs(gui.groups.focus())
                .and_then(|tabs| tabs.active().map(|tab| tab.document().to_owned()));
            gui.groups.split(Axis::LeftRight);
            if let Some(document) = document {
                gui.groups.another_view(document);
            }
            gui.message = "editor split".to_owned();
            gui.sync_editors();
        }
        "group:next" => {
            gui.groups.focus_next();
            gui.message = format!("focus group {}", gui.groups.focus().get());
        }
        "quit" => {
            gui.message = "closing".to_owned();
            rust_widgets::quit();
        }
        other => gui.message = format!("unknown action {other}"),
    }
    rust_widgets::set_widget_text(
        gui.status,
        &format!("{} | {}", gui.status_line(), gui.message),
    );
}

fn wire_button(button: &ButtonHandle, action: &str, gui: &Arc<Mutex<Gui>>) {
    let gui = Arc::clone(gui);
    let action = action.to_owned();
    button.on_click(move || {
        if let Ok(mut gui) = gui.lock() {
            apply_action(&mut gui, &action);
        }
    });
}

fn wire_tab(button: &ButtonHandle, group: usize, index: usize, gui: &Arc<Mutex<Gui>>) {
    let gui = Arc::clone(gui);
    button.on_click(move || {
        if let Ok(mut gui) = gui.lock() {
            let Some(id) = gui.groups.group_ids().get(group).copied() else {
                return;
            };
            if let Some(tabs) = gui.groups.tabs_mut(id) {
                let _ = tabs.activate(index);
            }
            gui.message = format!("tab {index} in group {group}");
            gui.sync_editors();
        }
    });
}

/// Sizes the two editor groups from the model: one group uses the full centre, two split it.
fn layout_group_widgets(gui: &mut Gui) {
    let split = gui.groups.len() > 1;
    if let Some(view) = gui.group_views.first() {
        let width = if split { 330 } else { 680 };
        for (index, id) in view.tabs.iter().enumerate() {
            rust_widgets::set_widget_geometry(*id, 270 + index as i32 * 110, CONTENT_Y, 108, 26);
        }
        rust_widgets::set_widget_geometry(view.editor, 270, CONTENT_Y + 30, width, 535);
    }
    if let Some(view) = gui.group_views.get(1) {
        for widget in &view.all {
            rust_widgets::set_widget_visible(*widget, split);
        }
    }
}

/// Opens the workbench window and runs it until the window closes or the deadline passes.
pub(super) fn workbench_gui(
    layout_path: Option<PathBuf>,
    exit_after: Duration,
    smoke: bool,
) -> bool {
    let mut platform = NativePlatform::new();
    if let Err(error) = platform.init() {
        return report(error.message());
    }
    let request = StartupRequest {
        title: "rustage — design workbench".to_owned(),
        width: WIDTH,
        height: HEIGHT,
        exit_after,
        ..StartupRequest::default()
    };
    let window = match platform.create_window(&request) {
        Ok(window) => window,
        Err(error) => return report(error.message()),
    };
    let handle = WindowHandle::from_raw(window);
    crate::host::log::emit(
        crate::host::log::Level::Info,
        "app::run::workbench",
        &format!("workbench window {window} created {WIDTH}x{HEIGHT}"),
    );

    // The models.
    let mut workbench = Workbench::new();
    workbench.set_mode(Mode::Design);
    workbench.open_project("demo-project");
    workbench.open_document("src/main.rs");
    workbench.set_target("desktop");

    let mut panels = built_in();
    for (id, title, dock) in EXTRA_PANELS {
        let _ = panels.register(PanelSpec::new(id, title, dock, 300));
        let _ = panels.show(id);
    }
    // The services the built-in panels need are the ones this slice actually has: the document
    // tree, a property schema and diagnostics all come from models that exist.
    let services: BTreeSet<String> = ["document-tree", "property-schema", "diagnostics"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    let _ = &services;
    for id in [
        "outline",
        "inspector",
        "messages",
        "problems",
        "events",
        "data",
    ] {
        let _ = panels.show(id);
    }

    let mut groups = Groups::new(8);
    for (document, text) in [
        (
            "src/main.rs",
            "fn main() {\n    println!(\"hello from the workbench\");\n}\n",
        ),
        (
            "src/ui.rs",
            "pub fn build_ui() {\n    // generated tree\n}\n",
        ),
        ("styles.css", "window {\n  background: #1e1e1e;\n}\n"),
    ] {
        groups.pin(document);
        let _ = groups.type_into(text);
    }
    let focus = groups.focus();
    if let Some(tabs) = groups.tabs_mut(focus) {
        let _ = tabs.activate(0);
    }
    let default_layout = Layout::captured("default", &groups, &panels, false, false);
    if let Some(path) = &layout_path
        && let Ok(text) = std::fs::read_to_string(path)
        && let Ok(layout) = Layout::from_json(&text)
    {
        let _ = layout.apply(&mut groups, &mut panels);
    }

    let gui = Arc::new(Mutex::new(Gui {
        workbench,
        panels,
        groups,
        default_layout,
        layout_path: layout_path.clone(),
        panel_widgets: BTreeMap::new(),
        group_views: Vec::new(),
        status: 0,
        message: "ready".to_owned(),
    }));

    // Menu bar. The toolbar is the portable path (macOS paints no native menus in this revision);
    // the menu bar is still created because on the other hosts it is the native one.
    let bar = handle.new_menu_bar(0, 0, WIDTH, MENU_HEIGHT);
    let _ = handle.attach_menu_bar(&bar);
    let mut menu_actions: BTreeMap<u64, String> = BTreeMap::new();
    for (menu_name, entries) in [
        ("File", vec![("Quit", "quit")]),
        (
            "View",
            vec![
                ("Outline", "toggle:outline"),
                ("Inspector", "toggle:inspector"),
                ("Messages", "toggle:messages"),
                ("Problems", "toggle:problems"),
                ("Events", "toggle:events"),
                ("Data & State", "toggle:data"),
            ],
        ),
        (
            "Layout",
            vec![
                ("Save layout", "layout:save"),
                ("Reset layout", "layout:reset"),
            ],
        ),
        (
            "Editor",
            vec![("Split right", "group:split"), ("Next group", "group:next")],
        ),
    ] {
        let menu = handle.new_menu(&bar, menu_name, 0, 0, 0, 0);
        for (label, action) in entries {
            let item = handle.new_menu_item(&menu, label, None);
            menu_actions.insert(item.raw_id(), action.to_owned());
        }
    }

    // Toolbar. Buttons are the actions a person can reach without a menu.
    let mut x = 8;
    for (label, action) in [
        ("Outline", "toggle:outline"),
        ("Inspector", "toggle:inspector"),
        ("Messages", "toggle:messages"),
        ("Problems", "toggle:problems"),
        ("Events", "toggle:events"),
        ("Data", "toggle:data"),
        ("Split", "group:split"),
        ("Next group", "group:next"),
        ("Save layout", "layout:save"),
        ("Reset layout", "layout:reset"),
    ] {
        let width = (label.chars().count() as u32 * 9 + 24).max(96);
        let button = handle.new_button(label, x, TOOLBAR_Y, width, TOOLBAR_HEIGHT);
        wire_button(&button, action, &gui);
        x += width as i32 + 6;
    }

    // Docked panels. The rects are the first slice's fixed docks; a later slice moves them by the
    // layout's groups and sizes, which the model already carries.
    for (id, title, rect) in [
        ("outline", "Outline", (0, CONTENT_Y, 260, 280)),
        ("events", "Events", (0, 358, 260, 280)),
        (
            "inspector",
            "Inspector",
            (WIDTH as i32 - 320, CONTENT_Y, 320, 280),
        ),
        ("data", "Data & State", (WIDTH as i32 - 320, 358, 320, 280)),
        ("messages", "Messages", (260, 645, 340, 120)),
        ("problems", "Problems", (610, 645, 340, 120)),
    ] {
        let panel = handle.new_panel(rect.0, rect.1, rect.2, rect.3);
        panel.set_title(title);
        let title = handle.new_label(title, rect.0 + 8, rect.1 + 6, rect.2 - 16, 20);
        let list = handle.new_list_box(rect.0 + 8, rect.1 + 30, rect.2 - 16, rect.3 - 38);
        let items: Vec<String> = match id {
            "outline" => vec![
                "src/main.rs".to_owned(),
                "src/ui.rs".to_owned(),
                "styles.css".to_owned(),
            ],
            "events" => vec![
                "click -> count (command)".to_owned(),
                "change -> edit (call)".to_owned(),
                "submit -> submit (call)".to_owned(),
            ],
            "data" => vec![
                "state.count: number".to_owned(),
                "state.text: text".to_owned(),
                "binding: field.text <-> state.text".to_owned(),
            ],
            "inspector" => vec![
                "node: root".to_owned(),
                "kind: window".to_owned(),
                "title: Demo".to_owned(),
            ],
            "messages" => vec!["ready".to_owned()],
            _ => vec!["no problems".to_owned()],
        };
        for item in items {
            let _ = rust_widgets::list_box_add_item(list.raw_id(), &item);
        }
        if let Ok(mut gui) = gui.lock() {
            let entry = gui.panel_widgets.entry(id.to_owned()).or_default();
            entry.push(panel.raw_id());
            entry.push(title.raw_id());
            entry.push(list.raw_id());
        }
    }

    // Status bar.
    let status = handle.new_status_bar("", 0, STATUS_Y, WIDTH, STATUS_HEIGHT);
    if let Ok(mut gui) = gui.lock() {
        gui.status = status.raw_id();
    }

    // Two editor groups: the second stays hidden until "Split" is used.
    for group in 0..2 {
        let (tx, _ex, ew) = if group == 0 {
            (270, 270, 680)
        } else {
            (610, 610, 340)
        };
        let mut tabs = Vec::new();
        let mut all = Vec::new();
        for index in 0..3 {
            let button = handle.new_button("", tx + index as i32 * 110, CONTENT_Y, 108, 26);
            wire_tab(&button, group, index, &gui);
            tabs.push(button.raw_id());
            all.push(button.raw_id());
        }
        let editor = handle.new_list_box(tx, CONTENT_Y + 30, ew, 535);
        all.push(editor.raw_id());
        if group == 1 {
            for widget in &all {
                rust_widgets::set_widget_visible(*widget, false);
            }
        }
        if let Ok(mut gui) = gui.lock() {
            gui.group_views.push(GroupView {
                tabs,
                editor: editor.raw_id(),
                all,
            });
        }
    }

    if let Ok(mut gui) = gui.lock() {
        gui.sync_panels();
        gui.sync_editors();
    }

    // Menu activation is polled: the platform reports an item id, not a callback.
    if !menu_actions.is_empty() {
        schedule_menu(Arc::clone(&gui), Arc::new(menu_actions));
    }

    if smoke {
        crate::host::log::emit(
            crate::host::log::Level::Info,
            "app::run::workbench",
            "workbench smoke: toggling panels, saving layout and splitting",
        );
        let smoke_gui = Arc::clone(&gui);
        let _ = rust_widgets::schedule_on_main(Duration::from_millis(300), move || {
            if let Ok(mut gui) = smoke_gui.lock() {
                apply_action(&mut gui, "toggle:outline");
                apply_action(&mut gui, "toggle:events");
                apply_action(&mut gui, "group:split");
                apply_action(&mut gui, "layout:save");
                apply_action(&mut gui, "group:next");
            }
        });
        let quit_gui = Arc::clone(&gui);
        let _ = rust_widgets::schedule_on_main(Duration::from_millis(900), move || {
            if let Ok(mut gui) = quit_gui.lock() {
                apply_action(&mut gui, "quit");
            }
        });
    }

    // Menu-driven panel toggles must be visible in the status line even when no button was
    // pressed, so the poller runs on the library's own main-thread timer.
    match platform.run_loop(&request) {
        Ok(_) => {
            crate::host::log::emit(
                crate::host::log::Level::Info,
                "app::run::workbench",
                "workbench session ended",
            );
            true
        }
        Err(error) => report(error.message()),
    }
}

fn schedule_menu(gui: Arc<Mutex<Gui>>, actions: Arc<BTreeMap<u64, String>>) {
    let next = Arc::clone(&gui);
    let next_actions = Arc::clone(&actions);
    let _ = rust_widgets::schedule_on_main(Duration::from_millis(50), move || {
        while let Some(id) = rust_widgets::poll_menu_triggered() {
            if let Some(action) = actions.get(&id)
                && let Ok(mut gui) = gui.lock()
            {
                apply_action(&mut gui, action);
            }
        }
        schedule_menu(next, next_actions);
    });
}
