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

use std::any::Any;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rust_widgets::app::{ButtonHandle, WidgetHandle, WindowHandle};
use rust_widgets::control_backend::get_control_backend_for_widget;
use rust_widgets::core::Point;
use rust_widgets::layout::{BoxLayout, Layout as _, LayoutConstraints, Orientation, SizePolicy};
use rust_widgets::widget::capability::CapabilityValue;
use rust_widgets::widget::runtime::{clear_focus, focus_widget, focused_widget, with_widget_mut};

use crate::app::canvas::scene::Rect as SceneRect;
use crate::app::commands::command::{Context, Registry, built_in};
use crate::app::commands::palette::Recents;
use crate::app::shell::groups::{Axis, Groups};
use crate::app::shell::layout::Layout;
use crate::app::shell::panels::{Dock, PanelSpec, Panels, built_in as built_in_panels};
use crate::app::shell::workbench::{Mode, Workbench};
use crate::contracts::identity::NodeId;
use crate::contracts::permission::EffectClass;
use crate::host::platform::NativePlatform;
use crate::host::startup::{StartupPlatform, StartupRequest};

use super::report;
use super::workbench_debug::{Command as DebugCommand, DebugState, Update as DebugUpdate};
use super::workbench_events::EventsState;
use super::workbench_inspect::InspectState;
use super::workbench_layout::{Area, WorkbenchLayout};
use super::workbench_pages::PagesState;
use super::workbench_palette::Palette;
use super::workbench_property::PropertyState;
use super::workbench_scene::{CanvasLayout, CanvasState, CanvasTransform, span};

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 800;
const MENU_HEIGHT: u32 = 24;
const TOOLBAR_Y: i32 = 30;
const TOOLBAR_HEIGHT: u32 = 32;
const CONTENT_Y: i32 = 70;
const STATUS_Y: i32 = 768;
const STATUS_HEIGHT: u32 = 28;

/// The panels this window adds to the built-in four.
const EXTRA_PANELS: [(&str, &str, Dock); 4] = [
    ("pages", "Pages", Dock::Left),
    ("events", "Events", Dock::Left),
    ("data", "Data & State", Dock::Right),
    ("debug", "Debug", Dock::Right),
];

/// The Inspector's list, value field, Apply/Clear/Reset/Values buttons, theme field and theme
/// button, then the error line and note.
type InspectorControls = (u64, u64, u64, u64, u64, u64, u64, u64, u64, u64);

/// The Events panel's list, handler field, Bind/Unbind/Fire buttons, error line and note.
type EventControls = (u64, u64, u64, u64, u64, u64, u64);

/// The Data panel's list, field, Apply button, error line and note.
type DataControls = (u64, u64, u64, u64, u64);

/// The Pages panel's list, field, Add/Open/Back/Fwd/Close buttons, error line and note.
type PageControls = (u64, u64, u64, u64, u64, u64, u64, u64, u64);

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
    /// The window whose children this layout places.
    window: u64,
    /// Every widget's place in the responsive chrome.
    placements: Vec<(u64, Area)>,
    /// The command palette's state, and the controls that mirror it.
    palette: Palette,
    palette_widgets: Vec<u64>,
    palette_query: u64,
    palette_list: u64,
    palette_help: u64,
    registry: Registry,
    recents: Recents,
    /// The Inspector's one editable property and the controls that mirror it.
    property: PropertyState,
    property_field: u64,
    property_error: u64,
    inspector_list: u64,
    /// The object tree, the canvas scene and the one selection both views read (T09.02–T09.04).
    scene: CanvasState,
    outline_list: u64,
    canvas_panel: u64,
    canvas_surface: u64,
    /// The canvas pointer facts, in design space: whether a press is down, where it began and
    /// where the pointer last was. A drag wider/taller than [`DRAG_THRESHOLD`] is a marquee.
    drag_active: bool,
    drag_start: Option<(i32, i32)>,
    pointer: Option<(i32, i32)>,
    /// The debug panel's view of a real background DAP session.
    debug: DebugState,
    /// The fixed label per row; a label repaints reliably from a timer, a list box did not.
    debug_rows: Vec<u64>,
    /// The Events panel: declarations, wiring and triggers, with the event the rows select.
    events: EventsState,
    events_list: u64,
    events_field: u64,
    events_error: u64,
    events_note: u64,
    events_selected: usize,
    /// The Data panel and the read-only inspector over the same document version.
    inspect: InspectState,
    data_list: u64,
    data_field: u64,
    data_error: u64,
    data_note: u64,
    data_selected: usize,
    inspect_note: u64,
    /// The Pages panel: the declarative graph and the window's back/forward stacks.
    pages: PagesState,
    pages_list: u64,
    pages_field: u64,
    pages_error: u64,
    pages_note: u64,
    pages_selected: usize,
    /// The latest canvas picture, and whether a repaint is already queued.
    ///
    /// Pointer motion can arrive much faster than the painter can draw; the snapshot is replaced
    /// in place and one repaint is queued, so a drag cannot pile up an unbounded number of frames.
    canvas_snapshot: Arc<Mutex<CanvasSnapshot>>,
    canvas_scheduled: Arc<AtomicBool>,
}

/// One frame of the canvas: each visible node's rectangle, name, hidden/locked/selected state,
/// and the marquee if a drag is in progress.
type CanvasSnapshot = (
    Vec<(SceneRect, String, bool, bool, bool)>,
    Option<SceneRect>,
);

/// How far a press must move in design space before it is a marquee rather than a click.
const DRAG_THRESHOLD: u32 = 8;

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

    /// Re-applies the responsive chrome layout with the current editor-group count.
    ///
    /// The library stores the layout on the window and re-runs it on every `Resized` event, so
    /// this is what makes a resize scale the chrome instead of only the widgets present at
    /// startup. Splitting or closing a group changes the count and therefore the layout object.
    fn refresh_layout(&self) {
        let mut layout = WorkbenchLayout::new(self.groups.len() as u32);
        for (id, area) in &self.placements {
            layout.place(*id, *area);
        }
        WindowHandle::from_raw(self.window).set_layout(layout);
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
        // The model decides how many groups exist; the responsive layout follows it. Geometry is
        // no longer written here, so the two cannot disagree about which box a group owns.
        self.refresh_layout();
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
        // A split or a panel toggle replaces the layout object, which re-places the canvas
        // control; the stored draw commands are absolute, so they are rebuilt for the new rect.
        sync_canvas(self);
    }
}

fn palette_context() -> Context {
    // The workbench's demo project is trusted for the session, and the palette is a person's
    // explicit request to run a command; those are the two facts the registry's availability
    // check asks for. File-write and network effects stay unauthorized, so their commands keep
    // their real disabled reason rather than being greyed out by a second list.
    Context::in_area("design")
        .trusted()
        .authorizing(EffectClass::Command)
}

/// Runs `f` with the palette state while the registry and recents stay borrowed from the GUI.
///
/// Taking the state out for the call is what lets a method on [`Palette`] read the registry it
/// searches: a `MutexGuard<Gui>` does not expose disjoint field borrows, and cloning the registry
/// would be a second copy of the command list.
fn palette_with<R>(gui: &mut Gui, f: impl FnOnce(&mut Palette, &Registry, &Recents) -> R) -> R {
    let mut state = std::mem::replace(&mut gui.palette, Palette::new());
    let result = f(&mut state, &gui.registry, &gui.recents);
    gui.palette = state;
    result
}

/// Subscribes one unit signal on a control by name.
///
/// The widget layer owns the signal; a name that does not resolve to one is a no-op rather than a
/// panic, which is the same contract the rest of the dynamic wiring uses.
fn subscribe_unit(
    id: u64,
    event: &'static str,
    mut callback: impl FnMut() + Send + Sync + 'static,
) {
    let subscribed = with_widget_mut(id, move |widget| {
        widget
            .event_signal_dyn(event)
            .map(|signal| signal.subscribe(Box::new(move |_| callback())))
    })
    .flatten()
    .is_some();
    if !subscribed {
        crate::host::log::emit(
            crate::host::log::Level::Warn,
            "app::run::workbench-palette",
            &format!("control {id} publishes no {event} signal"),
        );
    }
}

/// Subscribes one payload-carrying signal whose payload is a row index.
fn subscribe_index(
    id: u64,
    event: &'static str,
    mut callback: impl FnMut(u64) + Send + Sync + 'static,
) {
    let subscribed = with_widget_mut(id, move |widget| {
        widget.event_signal_dyn(event).map(|signal| {
            signal.subscribe(Box::new(move |payload| {
                if let CapabilityValue::UInt(index) = payload {
                    callback(*index);
                }
            }))
        })
    })
    .flatten()
    .is_some();
    if !subscribed {
        crate::host::log::emit(
            crate::host::log::Level::Warn,
            "app::run::workbench-palette",
            &format!("control {id} publishes no {event} signal"),
        );
    }
}

/// Logs when a control gains keyboard focus.
///
/// Focus traversal is the library's: the platform consumes Tab and calls `focus_next` in the
/// registration order. This observer makes that traversal observable in the host log so a window
/// probe can assert that keyboard-only navigation reached a panel rather than only the toolbar.
fn watch_focus(id: u64, name: &'static str) {
    let watched = with_widget_mut(id, move |widget| {
        widget.base().focus_gained.connect(move || {
            crate::host::log::emit(
                crate::host::log::Level::Info,
                "app::run::workbench-focus",
                &format!("focus gained {name} ({id})"),
            );
        });
        true
    })
    .unwrap_or(false);
    if !watched {
        crate::host::log::emit(
            crate::host::log::Level::Warn,
            "app::run::workbench-focus",
            &format!("control {id} could not be watched for focus"),
        );
    }
}

fn sync_palette(gui: &mut Gui) {
    let rows = gui.palette.rows();
    let selected = gui.palette.selected();
    let has_selection = gui.palette.selected_entry().is_some();
    rust_widgets::set_widget_text(gui.palette_help, &gui.palette.help(&gui.registry));

    // Rebuilding a list can make the control report a selection. Its callback locks the same
    // `Gui` this function is called under, and a synchronous emit would deadlock the mutex.
    // The list is therefore updated on the next main-loop turn, after the caller releases it.
    let list = gui.palette_list;
    let _ = rust_widgets::schedule_on_main(Duration::from_millis(0), move || {
        rust_widgets::list_box_clear_items(list);
        for row in &rows {
            let _ = rust_widgets::list_box_add_item(list, row);
        }
        if has_selection {
            let _ = rust_widgets::list_box_set_current_index(list, selected);
        }
    });
}

/// Mirrors `gui.message` into the real status bar for callback paths that do not go through
/// [`apply_action`].
fn update_status(gui: &Gui) {
    rust_widgets::set_widget_text(
        gui.status,
        &format!("{} | {}", gui.status_line(), gui.message),
    );
}

/// Repaints the Inspector rows from the one property model and the selected node.
fn refresh_inspector(gui: &mut Gui) {
    let kind = gui
        .scene
        .kind(gui.property.node())
        .map(|kind| kind.name())
        .unwrap_or("?");
    let contract = gui
        .property
        .control()
        .map(|control| format!("{control}.{}", gui.property.name().as_str()))
        .unwrap_or_else(|| "(no contract)".to_owned());
    let nodes = if gui.property.count() > 1 {
        format!("{} nodes", gui.property.count())
    } else {
        gui.property.node().get().to_string()
    };
    let items = [
        format!("node: {nodes}"),
        format!("kind: {kind} · {contract}"),
        format!(
            "{}: {}",
            gui.property.name().as_str(),
            gui.property.summary()
        ),
        format!(
            "theme: {} · computed background {}",
            gui.inspect.theme().unwrap_or("(none)"),
            gui.inspect
                .background(gui.property.node())
                .unwrap_or("(not themed)")
        ),
    ];
    rust_widgets::list_box_clear_items(gui.inspector_list);
    for item in &items {
        let _ = rust_widgets::list_box_add_item(gui.inspector_list, item);
    }
    // Contract facts are reported instead of hidden behind one node's schema: an unavailable
    // schema refuses edits, and a selection whose properties disagree is not agreement.
    if gui.property_error != 0 {
        let note = gui
            .property
            .schema_unavailable()
            .map(|reason| format!("schema unavailable: {reason}"))
            .or_else(|| {
                gui.property
                    .incompatible()
                    .map(|reason| format!("contracts differ: {reason}"))
            })
            .or_else(|| {
                gui.property
                    .contract_note()
                    .map(|note| format!("contracts: {note}"))
            });
        if let Some(note) = note {
            rust_widgets::set_widget_text(gui.property_error, &note);
        }
    }
    rust_widgets::set_widget_text(gui.property_field, &gui.property.value_text());
    rust_widgets::set_widget_placeholder(
        gui.property_field,
        if gui.property.is_mixed() {
            "mixed — typing applies to every selected node"
        } else {
            ""
        },
    );
    if gui.inspect_note != 0 {
        rust_widgets::set_widget_text(
            gui.inspect_note,
            &format!(
                "design v{} · effects {} · read-only",
                gui.inspect.version(),
                gui.inspect.effects()
            ),
        );
    }
    crate::host::log::emit(
        crate::host::log::Level::Info,
        "app::run::workbench-property",
        &format!("inspector rows: {} [contract {contract}]", items[2]),
    );
    if let Some(note) = gui.property.contract_note() {
        crate::host::log::emit(
            crate::host::log::Level::Info,
            "app::run::workbench-property",
            &format!("inspector contracts: {note}"),
        );
    }
    if let Some(reason) = gui.property.incompatible() {
        crate::host::log::emit(
            crate::host::log::Level::Warn,
            "app::run::workbench-property",
            &format!("inspector refuses one shared property: {reason}"),
        );
    }
}

/// The event the Events rows currently select.
fn selected_event(gui: &Gui) -> Option<String> {
    gui.events.events().get(gui.events_selected).cloned()
}

/// Mirrors the Events model into its list and its two labels.
fn sync_events(gui: &mut Gui) {
    let rows = gui.events.rows();
    let selected = gui.events_selected;
    let status = selected_event(gui)
        .map(|event| gui.events.status_of(&event).render(&event))
        .unwrap_or_else(|| "no event is declared".to_owned());
    let last = gui.events.last_firing().join(" | ");
    let handlers = gui.events.handlers().join(", ");
    let wiring = gui.events.wiring_rows().join("; ");
    rust_widgets::set_widget_text(
        gui.events_note,
        &format!(
            "{status} | rev {} | handlers: {handlers} | {wiring} | {last}",
            gui.events.revision()
        ),
    );
    let list = gui.events_list;
    if list == 0 {
        return;
    }
    let _ = rust_widgets::schedule_on_main(Duration::from_millis(0), move || {
        rust_widgets::list_box_clear_items(list);
        for row in &rows {
            let _ = rust_widgets::list_box_add_item(list, row);
        }
        let _ = rust_widgets::list_box_set_current_index(list, selected);
    });
}

/// Mirrors the Data model into its list and its labels.
fn sync_data(gui: &mut Gui) {
    let rows = gui.inspect.rows();
    let fields = gui.inspect.fields();
    let selected = gui.data_selected.min(fields.len().saturating_sub(1));
    gui.data_selected = selected;
    let value = fields
        .get(selected)
        .and_then(|field| gui.inspect.value_of(field))
        .unwrap_or_default();
    rust_widgets::set_widget_text(gui.data_field, &value);
    if let Some(problem) = gui.inspect.problem() {
        rust_widgets::set_widget_text(gui.data_error, &format!("not applied: {problem}"));
    }
    rust_widgets::set_widget_text(
        gui.data_note,
        &format!(
            "data v{} · effects {} · runtime: none",
            gui.inspect.version(),
            gui.inspect.effects()
        ),
    );
    let list = gui.data_list;
    if list == 0 {
        return;
    }
    let _ = rust_widgets::schedule_on_main(Duration::from_millis(0), move || {
        rust_widgets::list_box_clear_items(list);
        for row in &rows {
            let _ = rust_widgets::list_box_add_item(list, row);
        }
    });
}

/// Mirrors the Pages model into its list and its labels.
fn sync_pages(gui: &mut Gui) {
    let rows = gui.pages.rows();
    let selected = gui.pages_selected;
    rust_widgets::set_widget_text(
        gui.pages_note,
        &format!(
            "{} page(s) · current {} · revision {}",
            gui.pages.pages().len(),
            gui.pages.current().unwrap_or("(none)"),
            gui.pages.revision()
        ),
    );
    let list = gui.pages_list;
    if list == 0 {
        return;
    }
    let _ = rust_widgets::schedule_on_main(Duration::from_millis(0), move || {
        rust_widgets::list_box_clear_items(list);
        for row in &rows {
            let _ = rust_widgets::list_box_add_item(list, row);
        }
        let _ = rust_widgets::list_box_set_current_index(list, selected);
    });
}

/// Points the Events and Data panels at the same selection the tree and canvas share.
fn retarget_panels(gui: &mut Gui) {
    if let Some(node) = gui.scene.primary() {
        gui.events.retarget(node);
    }
    let selected = gui.scene.selected().to_vec();
    {
        let Gui { inspect, scene, .. } = gui;
        inspect.retarget(scene, &selected);
    }
    sync_events(gui);
    sync_data(gui);
}

/// Commits the field's text through the schema editor (task T11.01).
fn apply_property(gui: &mut Gui) {
    let text = rust_widgets::get_widget_text(gui.property_field);
    crate::host::log::emit(
        crate::host::log::Level::Info,
        "app::run::workbench-property",
        &format!("inspector Apply text {} character(s)", text.chars().count()),
    );
    match gui.property.apply(&text) {
        Ok(revision) => {
            let nodes = gui.property.nodes().to_vec();
            // Only a `title` contract is the tree's structural name. A button/label's `text` is
            // stored by the schema editor and must not rename the node in the Outline or canvas.
            let renames = gui.property.edits_node_name();
            let mut applied = Vec::new();
            for node in &nodes {
                if renames {
                    match gui.scene.rename(*node, &text) {
                        Ok(()) => applied.push(node.get().to_string()),
                        Err(reason) => applied.push(format!("{} refused: {reason}", node.get())),
                    }
                } else {
                    applied.push(node.get().to_string());
                }
            }
            let nodes_text = applied.join(", ");
            rust_widgets::set_widget_text(gui.property_error, "");
            refresh_inspector(gui);
            sync_outline(gui);
            sync_canvas(gui);
            gui.message = format!(
                "{} = {text:?} on {nodes_text} (revision {revision})",
                gui.property.name().as_str()
            );
            crate::host::log::emit(
                crate::host::log::Level::Info,
                "app::run::workbench-property",
                &format!(
                    "inspector applied {} = {text:?} on {nodes_text}; revision {revision}",
                    gui.property.name().as_str()
                ),
            );
            if renames {
                crate::host::log::emit(
                    crate::host::log::Level::Info,
                    "app::run::workbench-scene",
                    &format!("renamed node(s) {nodes_text} to {text:?}"),
                );
            } else {
                crate::host::log::emit(
                    crate::host::log::Level::Info,
                    "app::run::workbench-property",
                    &format!(
                        "stored {} = {text:?} on node(s) {nodes_text}; the tree name is unchanged",
                        gui.property.name().as_str()
                    ),
                );
            }
        }
        Err(reason) => {
            let reason = gui.property.problem().map(str::to_owned).unwrap_or(reason);
            rust_widgets::set_widget_text(gui.property_error, &format!("not applied: {reason}"));
            gui.message = format!(
                "{} not applied at revision {}: {reason}",
                gui.property.name().as_str(),
                gui.property.revision()
            );
            crate::host::log::emit(
                crate::host::log::Level::Info,
                "app::run::workbench-property",
                &format!(
                    "inspector refused {} = {text:?}; revision {} unchanged: {reason}",
                    gui.property.name().as_str(),
                    gui.property.revision()
                ),
            );
        }
    }
    update_status(gui);
}

/// The design-space point under an absolute window position.
///
/// The panel's rectangle is what the canvas control is laid out against, so the transform is the
/// same one [`CanvasLayout`] used. Reading it from the panel (not the canvas control) is what keeps
/// this callable from inside a canvas signal: the control being dispatched is already lent out.
fn canvas_point(canvas_panel: u64, x: i32, y: i32) -> Option<(i32, i32)> {
    if canvas_panel == 0 {
        return None;
    }
    with_widget_mut(canvas_panel, |widget| {
        CanvasTransform::new(widget.geometry()).to_design(x, y)
    })
}

/// Rebuilds the Outline list from the structural tree, keeping the selected row current.
///
/// The list is rebuilt on the next main-loop turn: an emit from `list_box_clear_items` or
/// `list_box_set_current_index` would re-enter the `Gui` mutex this function is called under.
fn sync_outline(gui: &mut Gui) {
    let rows = gui.scene.rows();
    let selected = gui.scene.primary().and_then(|node| gui.scene.row_of(node));
    let list = gui.outline_list;
    if list == 0 {
        return;
    }
    let _ = rust_widgets::schedule_on_main(Duration::from_millis(0), move || {
        rust_widgets::list_box_clear_items(list);
        for row in &rows {
            let _ = rust_widgets::list_box_add_item(list, row);
        }
        if let Some(index) = selected {
            let _ = rust_widgets::list_box_set_current_index(list, index);
        }
    });
}

/// Mirrors the one selection into every canvas button's label.
/// Repaints the canvas from the scene and the one selection.
///
/// The draw is deferred: the model is read while the `Gui` mutex is held, and the widget is
/// repainted on the next main-loop turn so a signal callback can call this without re-entering the
/// runtime while its control is the event target.
fn sync_canvas(gui: &Gui) {
    let nodes: Vec<(SceneRect, String, bool, bool, bool)> = gui
        .scene
        .nodes()
        .into_iter()
        .map(|node| {
            (
                node.rect,
                gui.scene.name(node.node).unwrap_or("?").to_owned(),
                gui.scene.is_hidden(node.node),
                gui.scene.is_locked(node.node),
                gui.scene.selected().contains(&node.node),
            )
        })
        .collect();
    let marquee = if gui.drag_active {
        match (gui.drag_start, gui.pointer) {
            (Some(start), Some(end)) => Some(span(start, end)),
            _ => None,
        }
    } else {
        None
    };
    if let Ok(mut snapshot) = gui.canvas_snapshot.lock() {
        *snapshot = (nodes, marquee);
    }
    // One queued frame at a time, and it reads the newest snapshot when it runs. A drag that
    // emits a pointer event per pixel therefore repaints at the loop's pace instead of stacking
    // frames it cannot show.
    if gui.canvas_scheduled.swap(true, Ordering::SeqCst) {
        return;
    }
    let snapshot = Arc::clone(&gui.canvas_snapshot);
    let scheduled = Arc::clone(&gui.canvas_scheduled);
    let panel = gui.canvas_panel;
    let surface = gui.canvas_surface;
    let _ = rust_widgets::schedule_on_main(Duration::from_millis(8), move || {
        if let Ok((nodes, marquee)) = snapshot.lock().map(|snapshot| snapshot.clone()) {
            draw_canvas(panel, surface, &nodes, marquee);
        }
        scheduled.store(false, Ordering::SeqCst);
    });
}

/// The flat, macOS-like palette the canvas paints.
const CANVAS_BACKGROUND: rust_widgets::core::Color =
    rust_widgets::core::Color::rgb(0xF5, 0xF5, 0xF7);
const NODE_FILL: rust_widgets::core::Color = rust_widgets::core::Color::rgb(0xFF, 0xFF, 0xFF);
const NODE_BORDER: rust_widgets::core::Color = rust_widgets::core::Color::rgb(0xC7, 0xC7, 0xCC);
const LOCKED_FILL: rust_widgets::core::Color = rust_widgets::core::Color::rgb(0xE8, 0xE8, 0xED);
const SELECTED_FILL: rust_widgets::core::Color = rust_widgets::core::Color::rgb(0xE3, 0xF0, 0xFF);
const ACCENT: rust_widgets::core::Color = rust_widgets::core::Color::rgb(0x0A, 0x84, 0xFF);
const TEXT: rust_widgets::core::Color = rust_widgets::core::Color::rgb(0x1D, 0x1D, 0x1F);
const MUTED_TEXT: rust_widgets::core::Color = rust_widgets::core::Color::rgb(0x86, 0x86, 0x8B);

/// Paints the design surface and every visible node in one pass.
fn draw_canvas(
    panel: u64,
    surface: u64,
    nodes: &[(SceneRect, String, bool, bool, bool)],
    marquee: Option<SceneRect>,
) {
    let Some(rect) = with_widget_mut(panel, |widget| widget.geometry()) else {
        return;
    };
    let transform = CanvasTransform::new(rect);
    let font = rust_widgets::core::Font::new("Inter", 13.0, false, false);
    let _ = with_widget_mut(surface, |widget| {
        let Some(canvas) = (widget as &mut dyn Any)
            .downcast_mut::<rust_widgets::widget::special_widgets::Canvas>()
        else {
            return;
        };
        canvas.clear();
        canvas.fill_rect(rect, CANVAS_BACKGROUND);
        for (design, name, hidden, locked, selected) in nodes {
            if *hidden {
                continue;
            }
            let node = transform.place(*design);
            let (fill, border) = if *selected {
                (SELECTED_FILL, ACCENT)
            } else if *locked {
                (LOCKED_FILL, NODE_BORDER)
            } else {
                (NODE_FILL, NODE_BORDER)
            };
            canvas.fill_rect(node, fill);
            canvas.draw_rect(node, border);
            let label = if *locked {
                format!("{name} [locked]")
            } else {
                name.clone()
            };
            canvas.draw_text(
                Point::new(node.x + 8, node.y + 8),
                &label,
                &font,
                if *locked { MUTED_TEXT } else { TEXT },
            );
        }
        if let Some(marquee) = marquee {
            canvas.draw_rect(transform.place(marquee), ACCENT);
        }
    });
}

/// Selects one node from a tree row, a canvas button or a canvas click.
///
/// A hidden or locked node is refused with the reason the scene gives; the Outline then snaps back
/// to the selection that actually holds, so a refused row never looks selected.
fn select_node(gui: &mut Gui, node: NodeId, source: &str) {
    match gui.scene.select_only(node) {
        Ok(()) => {
            let name = gui.scene.name(node).unwrap_or("?").to_owned();
            // The contract is the scene's stored mapping, never a lookup by display name. A node
            // with no mapping is reported and the Inspector is left unable to guess a schema.
            let outcome = match gui.scene.contract(node) {
                Ok(contract) => gui.property.retarget_control(
                    node,
                    &contract.control,
                    &contract.property,
                    &name,
                ),
                Err(reason) => {
                    gui.property.report_unavailable(node, &name, reason.clone());
                    Err(reason)
                }
            };
            gui.message = format!("selected node {} ({name})", node.get());
            rust_widgets::set_widget_text(gui.property_field, &gui.property.value_text());
            rust_widgets::set_widget_text(gui.property_error, "");
            refresh_inspector(gui);
            retarget_panels(gui);
            sync_canvas(gui);
            sync_outline(gui);
            if let Err(reason) = outcome {
                crate::host::log::emit(
                    crate::host::log::Level::Warn,
                    "app::run::workbench-property",
                    &format!(
                        "inspector contract refused for node {}: {reason}",
                        node.get()
                    ),
                );
            }
            crate::host::log::emit(
                crate::host::log::Level::Info,
                "app::run::workbench-scene",
                &format!("selected node {} ({name}) from {source}", node.get()),
            );
        }
        Err(reason) => {
            gui.message = format!("not selected: {reason}");
            sync_outline(gui);
            crate::host::log::emit(
                crate::host::log::Level::Info,
                "app::run::workbench-scene",
                &format!(
                    "refused {source} selection of node {}: {reason}",
                    node.get()
                ),
            );
        }
    }
    update_status(gui);
}

/// Selects every selectable node a marquee crosses and retargets the Inspector at the first.
fn marquee_select(gui: &mut Gui, rect: SceneRect) {
    let nodes = gui.scene.within(rect);
    let count = gui.scene.select_many(nodes.iter().copied());
    let names: Vec<String> = nodes
        .iter()
        .filter_map(|node| {
            gui.scene
                .name(*node)
                .map(|name| format!("{} {name}", node.get()))
        })
        .collect();
    gui.message = format!("marquee selected {count} node(s)");
    if !gui.scene.selected().is_empty() {
        let selected = gui.scene.selected().to_vec();
        let mut entries = Vec::new();
        let mut problems = Vec::new();
        for node in &selected {
            let Some(name) = gui.scene.name(*node) else {
                continue;
            };
            match gui.scene.contract(*node) {
                Ok(contract) => entries.push((*node, contract, name.to_owned())),
                Err(reason) => problems.push(reason),
            }
        }
        let refs: Vec<_> = entries
            .iter()
            .map(|(node, contract, name)| (*node, contract.clone(), name.as_str()))
            .collect();
        gui.property.retarget_many_contracts(&refs, &problems);
        rust_widgets::set_widget_text(gui.property_field, &gui.property.value_text());
        rust_widgets::set_widget_text(gui.property_error, "");
        refresh_inspector(gui);
        if let Some(reason) = gui.property.incompatible() {
            crate::host::log::emit(
                crate::host::log::Level::Warn,
                "app::run::workbench-property",
                &format!("inspector cannot share one property across the marquee: {reason}"),
            );
        }
    }
    retarget_panels(gui);
    sync_canvas(gui);
    sync_outline(gui);
    crate::host::log::emit(
        crate::host::log::Level::Info,
        "app::run::workbench-scene",
        &format!(
            "marquee {}: {count} node(s): {}",
            rect.render(),
            names.join(", ")
        ),
    );
    update_status(gui);
}

/// Ends a canvas press: a short one is a click hit test, a long one is a marquee.
fn finish_canvas_drag(gui: &mut Gui) {
    if !gui.drag_active {
        return;
    }
    gui.drag_active = false;
    let start = gui.drag_start.take();
    let Some(end) = gui.pointer else {
        crate::host::log::emit(
            crate::host::log::Level::Info,
            "app::run::workbench-scene",
            "canvas released without a known pointer position",
        );
        return;
    };
    let start = start.unwrap_or(end);
    let moved = start.0.abs_diff(end.0).max(start.1.abs_diff(end.1));
    if moved >= DRAG_THRESHOLD {
        marquee_select(gui, span(start, end));
        return;
    }
    if let Some(node) = gui.scene.hit(end.0, end.1) {
        select_node(gui, node, "canvas");
        return;
    }
    let had = gui.scene.clear_selection();
    gui.message = if had == 0 {
        "no selectable node".to_owned()
    } else {
        "selection cleared".to_owned()
    };
    retarget_panels(gui);
    sync_canvas(gui);
    sync_outline(gui);
    update_status(gui);
    crate::host::log::emit(
        crate::host::log::Level::Info,
        "app::run::workbench-scene",
        &format!("canvas click at {:?}: no selectable node", end),
    );
}

fn open_palette(gui: &mut Gui) {
    if gui.palette.is_open() {
        return;
    }
    crate::host::log::emit(
        crate::host::log::Level::Info,
        "app::run::workbench-palette",
        "palette open requested",
    );
    let previous = focused_widget();
    let context = palette_context();
    palette_with(gui, |palette, registry, recents| {
        palette.open(previous, registry, &context, recents);
    });
    for id in &gui.palette_widgets {
        rust_widgets::set_widget_visible(*id, true);
    }
    // The palette is placed by the same responsive layout as the chrome, so the overlay follows
    // a resize instead of keeping the geometry it was created with.
    gui.refresh_layout();
    // The palette starts empty. The model was reset synchronously; the field is cleared on the
    // next main-loop turn so its value callback cannot re-enter the `Gui` mutex this action holds.
    let query_field = gui.palette_query;
    let _ = rust_widgets::schedule_on_main(Duration::from_millis(0), move || {
        rust_widgets::set_widget_text(query_field, "");
    });
    sync_palette(gui);
    if focus_widget(gui.palette_query) {
        gui.message = "command palette".to_owned();
        crate::host::log::emit(
            crate::host::log::Level::Info,
            "app::run::workbench-palette",
            &format!("palette opened; previous focus {previous:?}"),
        );
    } else {
        gui.message = "the command palette could not take the keyboard".to_owned();
        crate::host::log::emit(
            crate::host::log::Level::Info,
            "app::run::workbench-palette",
            "palette could not focus its query field",
        );
        close_palette(gui);
    }
    update_status(gui);
}

fn close_palette(gui: &mut Gui) {
    if !gui.palette.is_open() {
        return;
    }
    let previous = gui.palette.close();
    for id in &gui.palette_widgets {
        rust_widgets::set_widget_visible(*id, false);
    }
    match previous {
        Some(id) if !gui.palette_widgets.contains(&id) => {
            if !focus_widget(id) {
                clear_focus();
            }
        }
        Some(_) => {
            // The keyboard was already on a palette control; there is nowhere useful to restore.
            clear_focus();
        }
        None => {
            clear_focus();
        }
    }
    crate::host::log::emit(
        crate::host::log::Level::Info,
        "app::run::workbench-palette",
        &format!("palette closed; focus {:?}", focused_widget()),
    );
}

fn shortcut_summary() -> String {
    let keymap = crate::app::commands::shortcuts::built_in();
    let bindings = keymap.bindings();
    let mut parts: Vec<String> = bindings
        .iter()
        .take(8)
        .map(|binding| format!("{} {}", binding.chord(), binding.command()))
        .collect();
    if bindings.len() > parts.len() {
        parts.push("…".to_owned());
    }
    format!("shortcuts: {}", parts.join(" · "))
}

fn run_palette_selected(gui: &mut Gui) {
    let Some(entry) = gui.palette.selected_entry().cloned() else {
        gui.message = "no command is selected".to_owned();
        return;
    };
    if !entry.is_enabled() {
        gui.message = format!("{}: {}", entry.id(), entry.reason().unwrap_or("disabled"));
        crate::host::log::emit(
            crate::host::log::Level::Info,
            "app::run::workbench-palette",
            &format!("palette refused {}", entry.id()),
        );
        update_status(gui);
        return;
    }

    let id = entry.id().to_owned();
    gui.recents.record(id.clone());
    let outcome = match id.as_str() {
        "view.palette" => {
            gui.message = "the command palette is already open".to_owned();
            update_status(gui);
            return;
        }
        "view.welcome" => {
            gui.workbench.set_mode(Mode::Welcome);
            format!("{id}: welcome mode")
        }
        "panel.close" => {
            // Focus tracking for docked panels is a later T04.07 slice; hiding an arbitrary panel
            // would be a different action from the one the user asked for.
            format!("{id}: no focused docked panel is tracked in this window yet")
        }
        "keys.show" => shortcut_summary(),
        other => format!("{other}: resolved; the workbench has no handler for it yet"),
    };
    close_palette(gui);
    gui.message = outcome;
    update_status(gui);
    crate::host::log::emit(
        crate::host::log::Level::Info,
        "app::run::workbench-palette",
        &format!("palette ran {id}"),
    );
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
        "palette:open" => open_palette(gui),
        "palette:close" => {
            close_palette(gui);
            gui.message = "command palette closed".to_owned();
        }
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

/// Wires one debug-panel control to the worker command it names.
fn wire_debug_button(button: &ButtonHandle, command: DebugCommand, gui: &Arc<Mutex<Gui>>) {
    let gui_for_click = Arc::clone(gui);
    button.on_click(move || {
        let next = Arc::clone(&gui_for_click);
        if let Ok(mut gui) = next.lock() {
            let accepted = match command {
                DebugCommand::Start => {
                    // The debuggee is generated from the same scene the tree and canvas show, so
                    // the source map entries name real nodes. The selected node is the breakpoint
                    // anchor; with no selection the second row (the demo panel) is used.
                    let nodes: Vec<(NodeId, String)> = gui
                        .scene
                        .row_nodes()
                        .into_iter()
                        .filter_map(|node| gui.scene.name(node).map(|name| (node, name.to_owned())))
                        .collect();
                    let break_node = gui
                        .scene
                        .primary()
                        .or_else(|| nodes.get(1).map(|(node, _)| *node))
                        .or_else(|| nodes.first().map(|(node, _)| *node))
                        .unwrap_or_else(|| NodeId::from_raw(0));
                    gui.debug.start(nodes, break_node)
                }
                other => gui.debug.send(other),
            };
            gui.message = if accepted {
                format!("debug: {}", command.name())
            } else {
                format!("debug: {} unavailable", command.name())
            };
            update_status(&gui);
            crate::host::log::emit(
                crate::host::log::Level::Info,
                "app::run::workbench-debug",
                &format!("debug {} requested (accepted {accepted})", command.name()),
            );
        }
        if command == DebugCommand::Start {
            schedule_debug_poll(&next);
        }
    });
}

/// Polls the worker's snapshot channel on the main loop while a session is live.
fn schedule_debug_poll(gui: &Arc<Mutex<Gui>>) {
    let next = Arc::clone(gui);
    let _ = rust_widgets::schedule_on_main(Duration::from_millis(200), move || {
        let mut live = false;
        let mut updates = Vec::new();
        if let Ok(mut gui) = next.lock() {
            while let Some(update) = gui.debug.poll() {
                updates.push(update);
            }
            live = gui.debug.is_live();
            for update in updates {
                apply_debug_update(&mut gui, update);
            }
        }
        if live {
            schedule_debug_poll(&next);
        }
    });
}

/// Mirrors one worker report into the debug list and the status bar.
fn apply_debug_update(gui: &mut Gui, update: DebugUpdate) {
    gui.message = update.status.clone();
    let rows = update.rows.clone();
    let labels = gui.debug_rows.clone();
    if !labels.is_empty() {
        let shown = rows.len().min(labels.len());
        for (index, id) in labels.iter().enumerate() {
            if let Some(row) = rows.get(index) {
                rust_widgets::set_widget_text(*id, row);
                rust_widgets::set_widget_visible(*id, true);
            } else {
                rust_widgets::set_widget_text(*id, "");
                rust_widgets::set_widget_visible(*id, false);
            }
        }
        crate::host::log::emit(
            crate::host::log::Level::Info,
            "app::run::workbench-debug",
            &format!("debug rows shown: {shown}/{} label(s)", labels.len()),
        );
    }
    update_status(gui);
    crate::host::log::emit(
        crate::host::log::Level::Info,
        "app::run::workbench-debug",
        &format!("debug: {}", update.status),
    );
    for row in &rows {
        crate::host::log::emit(
            crate::host::log::Level::Info,
            "app::run::workbench-debug",
            &format!("debug row: {row}"),
        );
    }
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

    let mut panels = built_in_panels();
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
        "pages",
        "outline",
        "inspector",
        "messages",
        "problems",
        "events",
        "data",
        "debug",
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

    // The design scene starts with its panel selected, which is the node the Inspector's seed is
    // about. The Inspector reads the panel's contract (`panel.title`); selecting through the model
    // (not by writing the list) is what makes the first paint and the first click obey one rule.
    let mut scene = CanvasState::demo();
    let mut property = PropertyState::for_control("window", "title", "Demo");
    if let Some(initial) = scene.row_nodes().get(1).copied() {
        let name = scene.name(initial).unwrap_or("Demo").to_owned();
        match scene.contract(initial) {
            Ok(contract) => {
                if let Err(reason) =
                    property.retarget_control(initial, &contract.control, &contract.property, &name)
                {
                    crate::host::log::emit(
                        crate::host::log::Level::Warn,
                        "app::run::workbench-property",
                        &reason,
                    );
                }
            }
            Err(reason) => property.report_unavailable(initial, &name, reason),
        }
        let _ = scene.select_only(initial);
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
        window,
        placements: Vec::new(),
        palette: Palette::new(),
        palette_widgets: Vec::new(),
        palette_query: 0,
        palette_list: 0,
        palette_help: 0,
        registry: built_in(),
        recents: Recents::new(8),
        property,
        property_field: 0,
        property_error: 0,
        inspector_list: 0,
        scene,
        outline_list: 0,
        canvas_panel: 0,
        canvas_surface: 0,
        drag_active: false,
        drag_start: None,
        pointer: None,
        debug: DebugState::new(),
        debug_rows: Vec::new(),
        events: EventsState::new(),
        events_list: 0,
        events_field: 0,
        events_error: 0,
        events_note: 0,
        events_selected: 0,
        inspect: InspectState::new(),
        data_list: 0,
        data_field: 0,
        data_error: 0,
        data_note: 0,
        data_selected: 0,
        inspect_note: 0,
        pages: PagesState::new(),
        pages_list: 0,
        pages_field: 0,
        pages_error: 0,
        pages_note: 0,
        pages_selected: 0,
        canvas_snapshot: Arc::new(Mutex::new((Vec::new(), None))),
        canvas_scheduled: Arc::new(AtomicBool::new(false)),
    }));

    // Menu bar. The toolbar is the portable path (macOS paints no native menus in this revision);
    // the menu bar is still created because on the other hosts it is the native one.
    let bar = handle.new_menu_bar(0, 0, WIDTH, MENU_HEIGHT);
    let _ = handle.attach_menu_bar(&bar);
    if let Ok(mut gui) = gui.lock() {
        gui.placements.push((bar.raw_id(), Area::Menu));
    }
    let mut menu_actions: BTreeMap<u64, String> = BTreeMap::new();
    for (menu_name, entries) in [
        ("File", vec![("Quit", "quit")]),
        (
            "View",
            vec![
                ("Command Palette", "palette:open"),
                ("Pages", "toggle:pages"),
                ("Outline", "toggle:outline"),
                ("Inspector", "toggle:inspector"),
                ("Messages", "toggle:messages"),
                ("Problems", "toggle:problems"),
                ("Events", "toggle:events"),
                ("Data & State", "toggle:data"),
                ("Debug", "toggle:debug"),
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

    // Toolbar. Buttons are the actions a person can reach without a menu. Their widths are
    // shared evenly by the responsive layout, so a wider window widens them rather than leaving
    // a strip of unused chrome at the right edge.
    let toolbar: Vec<(&str, &str)> = vec![
        ("Commands", "palette:open"),
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
    ];
    let toolbar_count = toolbar.len() as u32;
    for (index, (label, action)) in toolbar.iter().enumerate() {
        let button = handle.new_button(label, 0, TOOLBAR_Y, 120, TOOLBAR_HEIGHT);
        wire_button(&button, action, &gui);
        if let Ok(mut gui) = gui.lock() {
            gui.placements.push((
                button.raw_id(),
                Area::Toolbar {
                    index: index as u32,
                    count: toolbar_count,
                },
            ));
        }
    }

    // Docked panels. The model owns which panels exist and whether they are shown; the responsive
    // layout owns where they sit, and each panel's own `BoxLayout` owns its title/list split. A
    // panel therefore keeps its internal proportions when the window scales.
    let (inspector_node, inspector_name, inspector_value, inspector_kind, outline_items) = {
        let gui = gui.lock().expect("the workbench state is not poisoned");
        (
            gui.property.node().get(),
            gui.property.name().as_str().to_owned(),
            gui.property.value_text(),
            gui.scene
                .kind(gui.property.node())
                .map(|kind| kind.name())
                .unwrap_or("?")
                .to_owned(),
            gui.scene.rows(),
        )
    };
    let mut inspector_controls: Option<InspectorControls> = None;
    let mut events_controls: Option<EventControls> = None;
    let mut data_controls: Option<DataControls> = None;
    let mut pages_controls: Option<PageControls> = None;
    let mut outline_list: Option<u64> = None;
    for (id, title, rect, area) in [
        (
            "pages",
            "Pages",
            (0, CONTENT_Y, 260, 180),
            Area::LeftPanel { index: 0, count: 3 },
        ),
        (
            "outline",
            "Outline",
            (0, 254, 260, 180),
            Area::LeftPanel { index: 1, count: 3 },
        ),
        (
            "events",
            "Events",
            (0, 442, 260, 200),
            Area::LeftPanel { index: 2, count: 3 },
        ),
        (
            "inspector",
            "Inspector",
            (WIDTH as i32 - 320, CONTENT_Y, 320, 280),
            Area::RightPanel { index: 0, count: 2 },
        ),
        (
            "data",
            "Data & State",
            (WIDTH as i32 - 320, 358, 320, 280),
            Area::RightPanel { index: 1, count: 2 },
        ),
        (
            "messages",
            "Messages",
            (260, 645, 340, 120),
            Area::BottomPanel { index: 0, count: 2 },
        ),
        (
            "problems",
            "Problems",
            (610, 645, 340, 120),
            Area::BottomPanel { index: 1, count: 2 },
        ),
    ] {
        let panel = handle.new_panel(rect.0, rect.1, rect.2, rect.3);
        panel.set_title(title);
        // Child factories parent the controls to the panel, so the panel's layout, clipping and
        // teardown own them; asking the window for them (2.8.10 and earlier) left the parent link
        // as an implicit contract (feedback-20261020 §3.1, closed by 2.8.11).
        let title_widget = panel.new_label(title, rect.0 + 8, rect.1 + 6, rect.2 - 16, 20);
        let list = panel.new_list_box(rect.0 + 8, rect.1 + 30, rect.2 - 16, rect.3 - 38);
        let focus_name = match id {
            "pages" => "pages list",
            "outline" => "outline list",
            "events" => "events list",
            "inspector" => "inspector list",
            "data" => "data list",
            "messages" => "messages list",
            "problems" => "problems list",
            _ => "panel list",
        };
        watch_focus(list.raw_id(), focus_name);
        let mut inner = BoxLayout::new(Orientation::Vertical, 4, 8);
        inner.add_widget(title_widget.raw_id(), 0);
        inner.set_constraints(title_widget.raw_id(), LayoutConstraints::new(20, Some(20)));
        inner.set_size_policy(title_widget.raw_id(), SizePolicy::Fixed);
        inner.add_widget(list.raw_id(), 1);
        inner.set_size_policy(list.raw_id(), SizePolicy::Expanding);
        let mut extra_widgets: Vec<u64> = Vec::new();
        if id == "pages" {
            let field =
                panel.new_line_edit("", rect.0 + 8, rect.1 + rect.3 as i32 - 96, rect.2 - 16, 26);
            field.set_placeholder("page name");
            watch_focus(field.raw_id(), "pages field");
            let actions = panel.new_panel(rect.0 + 8, rect.1 + rect.3 as i32 - 66, rect.2 - 16, 26);
            let add = actions.new_button("Add", 0, 0, 44, 26);
            let open = actions.new_button("Open", 0, 0, 44, 26);
            let back = actions.new_button("Back", 0, 0, 44, 26);
            let forward = actions.new_button("Fwd", 0, 0, 44, 26);
            let close = actions.new_button("Close", 0, 0, 44, 26);
            let mut action_layout = BoxLayout::new(Orientation::Horizontal, 4, 0);
            for widget in [
                add.raw_id(),
                open.raw_id(),
                back.raw_id(),
                forward.raw_id(),
                close.raw_id(),
            ] {
                action_layout.add_widget(widget, 1);
                action_layout.set_size_policy(widget, SizePolicy::Expanding);
            }
            actions.set_layout(Box::new(action_layout));
            let error =
                panel.new_label("", rect.0 + 8, rect.1 + rect.3 as i32 - 36, rect.2 - 16, 16);
            let note =
                panel.new_label("", rect.0 + 8, rect.1 + rect.3 as i32 - 18, rect.2 - 16, 16);
            for (widget, height) in [
                (field.raw_id(), 26),
                (actions.raw_id(), 26),
                (error.raw_id(), 16),
                (note.raw_id(), 16),
            ] {
                inner.add_widget(widget, 0);
                inner.set_constraints(widget, LayoutConstraints::new(height, Some(height)));
                inner.set_size_policy(widget, SizePolicy::Fixed);
            }
            pages_controls = Some((
                list.raw_id(),
                field.raw_id(),
                add.raw_id(),
                open.raw_id(),
                back.raw_id(),
                forward.raw_id(),
                close.raw_id(),
                error.raw_id(),
                note.raw_id(),
            ));
            extra_widgets.extend([
                field.raw_id(),
                actions.raw_id(),
                add.raw_id(),
                open.raw_id(),
                back.raw_id(),
                forward.raw_id(),
                close.raw_id(),
                error.raw_id(),
                note.raw_id(),
            ]);
        }
        if id == "inspector" {
            // The value row and its error line are real controls inside the Inspector panel, not
            // a second floating form: the panel's own layout owns their geometry.
            let theme_y = rect.1 + rect.3 as i32 - 80;
            let value_y = rect.1 + rect.3 as i32 - 110;
            let row = panel.new_panel(rect.0 + 8, value_y, rect.2 - 16, 26);
            let field = row.new_line_edit(&inspector_value, rect.0 + 8, value_y, rect.2 - 96, 26);
            let apply = row.new_button("Apply", rect.0 + rect.2 as i32 - 250, value_y, 54, 26);
            let clear = row.new_button("Clear", rect.0 + rect.2 as i32 - 192, value_y, 48, 26);
            let reset = row.new_button("Reset", rect.0 + rect.2 as i32 - 140, value_y, 54, 26);
            let inspect = row.new_button("Values", rect.0 + rect.2 as i32 - 82, value_y, 58, 26);
            let theme_row = panel.new_panel(rect.0 + 8, theme_y, rect.2 - 16, 26);
            let theme_field = theme_row.new_line_edit("", rect.0 + 8, theme_y, rect.2 - 96, 26);
            theme_field.set_placeholder("theme name (dark)");
            watch_focus(field.raw_id(), "inspector value field");
            watch_focus(theme_field.raw_id(), "inspector theme field");
            let theme_button =
                theme_row.new_button("Theme", rect.0 + rect.2 as i32 - 80, theme_y, 72, 26);
            let error =
                panel.new_label("", rect.0 + 8, rect.1 + rect.3 as i32 - 50, rect.2 - 16, 16);
            let note =
                panel.new_label("", rect.0 + 8, rect.1 + rect.3 as i32 - 30, rect.2 - 16, 16);
            let mut row_layout = BoxLayout::new(Orientation::Horizontal, 4, 0);
            row_layout.add_widget(field.raw_id(), 1);
            row_layout.set_size_policy(field.raw_id(), SizePolicy::Expanding);
            for (widget, width) in [
                (apply.raw_id(), 54),
                (clear.raw_id(), 48),
                (reset.raw_id(), 54),
                (inspect.raw_id(), 58),
            ] {
                row_layout.add_widget(widget, 0);
                row_layout.set_constraints(widget, LayoutConstraints::new(width, Some(width)));
                row_layout.set_size_policy(widget, SizePolicy::Fixed);
            }
            row.set_layout(Box::new(row_layout));
            let mut theme_layout = BoxLayout::new(Orientation::Horizontal, 4, 0);
            theme_layout.add_widget(theme_field.raw_id(), 1);
            theme_layout.set_size_policy(theme_field.raw_id(), SizePolicy::Expanding);
            theme_layout.add_widget(theme_button.raw_id(), 0);
            theme_layout
                .set_constraints(theme_button.raw_id(), LayoutConstraints::new(72, Some(72)));
            theme_layout.set_size_policy(theme_button.raw_id(), SizePolicy::Fixed);
            theme_row.set_layout(Box::new(theme_layout));
            for (widget, height) in [
                (row.raw_id(), 26),
                (theme_row.raw_id(), 26),
                (error.raw_id(), 16),
                (note.raw_id(), 16),
            ] {
                inner.add_widget(widget, 0);
                inner.set_constraints(widget, LayoutConstraints::new(height, Some(height)));
                inner.set_size_policy(widget, SizePolicy::Fixed);
            }
            inspector_controls = Some((
                list.raw_id(),
                field.raw_id(),
                apply.raw_id(),
                clear.raw_id(),
                reset.raw_id(),
                inspect.raw_id(),
                theme_field.raw_id(),
                theme_button.raw_id(),
                error.raw_id(),
                note.raw_id(),
            ));
            extra_widgets.extend([
                row.raw_id(),
                field.raw_id(),
                apply.raw_id(),
                clear.raw_id(),
                reset.raw_id(),
                inspect.raw_id(),
                theme_row.raw_id(),
                theme_field.raw_id(),
                theme_button.raw_id(),
                error.raw_id(),
                note.raw_id(),
            ]);
        }
        if id == "events" {
            // The handler field and the Bind/Unbind/Fire row are the editing half of the panel;
            // the status line above keeps *declared*, *wired* and *fired* apart.
            let field =
                panel.new_line_edit("", rect.0 + 8, rect.1 + rect.3 as i32 - 96, rect.2 - 16, 26);
            field.set_placeholder("handler name");
            watch_focus(field.raw_id(), "events handler field");
            field.on_value_changed(|text| {
                crate::host::log::emit(
                    crate::host::log::Level::Info,
                    "app::run::workbench-focus",
                    &format!("events field text {} character(s)", text.chars().count()),
                );
            });
            let actions = panel.new_panel(rect.0 + 8, rect.1 + rect.3 as i32 - 66, rect.2 - 16, 26);
            let bind = actions.new_button("Bind", 0, 0, 72, 26);
            let unbind = actions.new_button("Unbind", 0, 0, 72, 26);
            let fire = actions.new_button("Fire", 0, 0, 72, 26);
            let mut action_layout = BoxLayout::new(Orientation::Horizontal, 4, 0);
            for widget in [bind.raw_id(), unbind.raw_id(), fire.raw_id()] {
                action_layout.add_widget(widget, 1);
                action_layout.set_size_policy(widget, SizePolicy::Expanding);
            }
            actions.set_layout(Box::new(action_layout));
            let error =
                panel.new_label("", rect.0 + 8, rect.1 + rect.3 as i32 - 36, rect.2 - 16, 16);
            let note =
                panel.new_label("", rect.0 + 8, rect.1 + rect.3 as i32 - 18, rect.2 - 16, 16);
            for (widget, height) in [
                (field.raw_id(), 26),
                (actions.raw_id(), 26),
                (error.raw_id(), 16),
                (note.raw_id(), 16),
            ] {
                inner.add_widget(widget, 0);
                inner.set_constraints(widget, LayoutConstraints::new(height, Some(height)));
                inner.set_size_policy(widget, SizePolicy::Fixed);
            }
            events_controls = Some((
                list.raw_id(),
                field.raw_id(),
                bind.raw_id(),
                unbind.raw_id(),
                fire.raw_id(),
                error.raw_id(),
                note.raw_id(),
            ));
            extra_widgets.extend([
                field.raw_id(),
                actions.raw_id(),
                bind.raw_id(),
                unbind.raw_id(),
                fire.raw_id(),
                error.raw_id(),
                note.raw_id(),
            ]);
        }
        if id == "data" {
            let row_y = rect.1 + rect.3 as i32 - 74;
            let row = panel.new_panel(rect.0 + 8, row_y, rect.2 - 16, 26);
            let field = row.new_line_edit("", rect.0 + 8, row_y, rect.2 - 96, 26);
            watch_focus(field.raw_id(), "data field");
            let apply = row.new_button("Apply", rect.0 + rect.2 as i32 - 80, row_y, 72, 26);
            let mut row_layout = BoxLayout::new(Orientation::Horizontal, 4, 0);
            row_layout.add_widget(field.raw_id(), 1);
            row_layout.set_size_policy(field.raw_id(), SizePolicy::Expanding);
            row_layout.add_widget(apply.raw_id(), 0);
            row_layout.set_constraints(apply.raw_id(), LayoutConstraints::new(72, Some(72)));
            row_layout.set_size_policy(apply.raw_id(), SizePolicy::Fixed);
            row.set_layout(Box::new(row_layout));
            let error =
                panel.new_label("", rect.0 + 8, rect.1 + rect.3 as i32 - 44, rect.2 - 16, 16);
            let note =
                panel.new_label("", rect.0 + 8, rect.1 + rect.3 as i32 - 24, rect.2 - 16, 16);
            for (widget, height) in [
                (row.raw_id(), 26),
                (error.raw_id(), 16),
                (note.raw_id(), 16),
            ] {
                inner.add_widget(widget, 0);
                inner.set_constraints(widget, LayoutConstraints::new(height, Some(height)));
                inner.set_size_policy(widget, SizePolicy::Fixed);
            }
            data_controls = Some((
                list.raw_id(),
                field.raw_id(),
                apply.raw_id(),
                error.raw_id(),
                note.raw_id(),
            ));
            extra_widgets.extend([
                row.raw_id(),
                field.raw_id(),
                apply.raw_id(),
                error.raw_id(),
                note.raw_id(),
            ]);
        }
        panel.set_layout(Box::new(inner));
        let items: Vec<String> = match id {
            "outline" => outline_items.clone(),
            "inspector" => vec![
                format!("node: {inspector_node}"),
                format!("kind: {inspector_kind}"),
                format!("{inspector_name}: {inspector_value}"),
            ],
            "messages" => vec!["ready".to_owned()],
            // The Events and Data lists are filled from their models once the controls exist.
            "events" | "data" => Vec::new(),
            // Pages is filled from its model too; the empty list is replaced on the first sync.
            "pages" => Vec::new(),
            _ => vec!["no problems".to_owned()],
        };
        for item in items {
            let _ = rust_widgets::list_box_add_item(list.raw_id(), &item);
        }
        if id == "outline" {
            outline_list = Some(list.raw_id());
        }
        if let Ok(mut gui) = gui.lock() {
            gui.placements.push((panel.raw_id(), area));
            let entry = gui.panel_widgets.entry(id.to_owned()).or_default();
            entry.push(panel.raw_id());
            entry.push(title_widget.raw_id());
            entry.push(list.raw_id());
            entry.extend(extra_widgets);
        }
    }

    // The Inspector's editor: Apply commits the typed text through the schema editor, and a
    // double-click on the property row moves the keyboard to the field.
    if let Some((
        list_id,
        field_id,
        apply_id,
        clear_id,
        reset_id,
        inspect_id,
        theme_field_id,
        theme_button_id,
        error_id,
        note_id,
    )) = inspector_controls
    {
        if let Ok(mut gui) = gui.lock() {
            gui.inspector_list = list_id;
            gui.property_field = field_id;
            gui.property_error = error_id;
            gui.inspect_note = note_id;
        }
        {
            let gui = Arc::clone(&gui);
            ButtonHandle::from_raw(apply_id).on_click(move || {
                if let Ok(mut gui) = gui.lock() {
                    crate::host::log::emit(
                        crate::host::log::Level::Info,
                        "app::run::workbench-property",
                        "inspector Apply pressed",
                    );
                    apply_property(&mut gui);
                }
            });
        }
        {
            let gui = Arc::clone(&gui);
            ButtonHandle::from_raw(clear_id).on_click(move || {
                if let Ok(mut gui) = gui.lock() {
                    rust_widgets::set_widget_text(gui.property_field, "");
                    rust_widgets::set_widget_text(gui.property_error, "");
                    gui.message = format!(
                        "field cleared; the valid {} is {:?}",
                        gui.property.name().as_str(),
                        gui.property.value_text()
                    );
                    update_status(&gui);
                    crate::host::log::emit(
                        crate::host::log::Level::Info,
                        "app::run::workbench-property",
                        &format!(
                            "inspector field cleared; valid value unchanged: {:?}",
                            gui.property.value_text()
                        ),
                    );
                }
            });
        }
        {
            let gui = Arc::clone(&gui);
            ButtonHandle::from_raw(reset_id).on_click(move || {
                if let Ok(mut gui) = gui.lock() {
                    // Reset is the T11.02 half the Clear button does not do: the explicit edit is
                    // taken away and each node returns to the structural name it started from.
                    let nodes = gui.property.nodes().to_vec();
                    let renames = gui.property.edits_node_name();
                    let (count, lines) = gui.property.reset();
                    if renames {
                        for node in &nodes {
                            if let Some(value) = gui.property.value_for(*node) {
                                let _ = gui.scene.rename(*node, &value);
                            }
                        }
                    }
                    rust_widgets::set_widget_text(gui.property_error, &lines.join(" | "));
                    refresh_inspector(&mut gui);
                    sync_outline(&mut gui);
                    sync_canvas(&gui);
                    gui.message = format!("reset {count} node(s) to the structural title");
                    update_status(&gui);
                    crate::host::log::emit(
                        crate::host::log::Level::Info,
                        "app::run::workbench-property",
                        &format!("inspector reset {count} node(s): {}", lines.join("; ")),
                    );
                }
            });
        }
        {
            let gui = Arc::clone(&gui);
            ButtonHandle::from_raw(inspect_id).on_click(move || {
                if let Ok(mut gui) = gui.lock() {
                    // The read-only half of T14.05: re-record the current selection, then read
                    // design/computed/runtime and the source location. No trigger is dispatched.
                    let selected = gui.scene.selected().to_vec();
                    {
                        let Gui { inspect, scene, .. } = &mut *gui;
                        inspect.retarget(scene, &selected);
                    }
                    match gui.inspect.inspect() {
                        Ok(lines) => {
                            rust_widgets::list_box_clear_items(gui.inspector_list);
                            for line in &lines {
                                let _ = rust_widgets::list_box_add_item(gui.inspector_list, line);
                            }
                            gui.message = format!(
                                "inspected node; design v{}, effects {}",
                                gui.inspect.version(),
                                gui.inspect.effects()
                            );
                            rust_widgets::set_widget_text(gui.property_error, "");
                            crate::host::log::emit(
                                crate::host::log::Level::Info,
                                "app::run::workbench-inspect",
                                &format!(
                                    "inspect rows: {} (effects {})",
                                    lines.len(),
                                    gui.inspect.effects()
                                ),
                            );
                        }
                        Err(reason) => {
                            rust_widgets::set_widget_text(
                                gui.property_error,
                                &format!("inspect refused: {reason}"),
                            );
                            gui.message = format!("inspect refused: {reason}");
                        }
                    }
                    if gui.inspect_note != 0 {
                        rust_widgets::set_widget_text(
                            gui.inspect_note,
                            &format!(
                                "design v{} · effects {} · read-only",
                                gui.inspect.version(),
                                gui.inspect.effects()
                            ),
                        );
                    }
                    update_status(&gui);
                }
            });
        }
        {
            let gui = Arc::clone(&gui);
            ButtonHandle::from_raw(theme_button_id).on_click(move || {
                if let Ok(mut gui) = gui.lock() {
                    let name = rust_widgets::get_widget_text(theme_field_id);
                    let selected = gui.scene.selected().to_vec();
                    {
                        let Gui { inspect, scene, .. } = &mut *gui;
                        inspect.retarget(scene, &selected);
                    }
                    match gui.inspect.apply_theme(&name, &selected) {
                        Ok(lines) => {
                            rust_widgets::set_widget_text(gui.property_error, "");
                            gui.message = lines[0].clone();
                            refresh_inspector(&mut gui);
                            crate::host::log::emit(
                                crate::host::log::Level::Info,
                                "app::run::workbench-inspect",
                                &format!("theme applied: {}", lines.join("; ")),
                            );
                        }
                        Err(reason) => {
                            let reason = gui.inspect.refuse(reason);
                            rust_widgets::set_widget_text(
                                gui.property_error,
                                &format!("theme refused: {reason}"),
                            );
                            gui.message = format!("theme refused: {reason}");
                            crate::host::log::emit(
                                crate::host::log::Level::Info,
                                "app::run::workbench-inspect",
                                &format!("theme refused: {reason}"),
                            );
                        }
                    }
                    update_status(&gui);
                }
            });
        }
        {
            let gui = Arc::clone(&gui);
            subscribe_index(list_id, "item_activated", move |index| {
                if index == 2
                    && let Ok(gui) = gui.lock()
                {
                    let focused = focus_widget(gui.property_field);
                    crate::host::log::emit(
                        crate::host::log::Level::Info,
                        "app::run::workbench-property",
                        &format!("inspector title row activated; field focused {focused}"),
                    );
                }
            });
        }
    }

    // The Outline list is the tree view of the same scene the canvas draws. A row click selects
    // the node that row names; the scene decides whether that is allowed.
    if let Some(list_id) = outline_list {
        if let Ok(mut gui) = gui.lock() {
            gui.outline_list = list_id;
        }
        // The scene starts with the Demo panel selected, so the list's first paint highlights the
        // same row the canvas does. The rebuild is deferred and the selection guard makes the
        // programmatic current-row change a no-op.
        if let Ok(mut gui) = gui.lock() {
            sync_outline(&mut gui);
        }
        let gui = Arc::clone(&gui);
        subscribe_index(list_id, "item_selected", move |index| {
            if let Ok(mut gui) = gui.lock()
                && let Some(node) = gui.scene.row_nodes().get(index as usize).copied()
            {
                // Rebuilding the list sets the current row to the selection; that programmatic
                // change emits this signal too. Answering it would select, rebuild and emit
                // forever, so a row that already is the primary selection is a no-op.
                if gui.scene.primary() == Some(node) {
                    return;
                }
                crate::host::log::emit(
                    crate::host::log::Level::Info,
                    "app::run::workbench-scene",
                    &format!("outline item_selected {index} -> node {}", node.get()),
                );
                select_node(&mut gui, node, "tree");
            }
        });
    }

    // The Pages panel (T13.05): the same Navigator the CLI drives, with the window's own
    // back/forward stacks. A route only moves along a declared edge, and a closing window refuses.
    if let Some((
        list_id,
        field_id,
        add_id,
        open_id,
        back_id,
        forward_id,
        close_id,
        error_id,
        note_id,
    )) = pages_controls
    {
        if let Ok(mut gui) = gui.lock() {
            gui.pages_list = list_id;
            gui.pages_field = field_id;
            gui.pages_error = error_id;
            gui.pages_note = note_id;
            sync_pages(&mut gui);
        }
        {
            let gui = Arc::clone(&gui);
            subscribe_index(list_id, "item_selected", move |index| {
                if let Ok(mut gui) = gui.lock() {
                    if gui.pages_selected == index as usize {
                        return;
                    }
                    gui.pages_selected = index as usize;
                    if let Some(page) = gui.pages.pages().get(gui.pages_selected).cloned() {
                        rust_widgets::set_widget_text(gui.pages_field, &page);
                    }
                    sync_pages(&mut gui);
                }
            });
        }
        {
            let gui = Arc::clone(&gui);
            ButtonHandle::from_raw(add_id).on_click(move || {
                if let Ok(mut gui) = gui.lock() {
                    let name = rust_widgets::get_widget_text(gui.pages_field);
                    // "home -> detail" declares a navigation; a bare name declares a page.
                    let outcome = match name.split_once("->") {
                        Some((from, to)) => {
                            let from = from.trim();
                            let to = to.trim();
                            gui.pages.add_edge(from, to).map(|revision| {
                                format!(
                                    "declared navigation {from:?} -> {to:?} (revision {revision})"
                                )
                            })
                        }
                        None => gui.pages.add_page(&name).map(|revision| {
                            format!("declared page {name:?} (revision {revision})")
                        }),
                    };
                    match outcome {
                        Ok(message) => {
                            rust_widgets::set_widget_text(gui.pages_error, "");
                            gui.message = message.clone();
                            crate::host::log::emit(
                                crate::host::log::Level::Info,
                                "app::run::workbench-pages",
                                &message,
                            );
                        }
                        Err(reason) => {
                            rust_widgets::set_widget_text(
                                gui.pages_error,
                                &format!("not added: {reason}"),
                            );
                            gui.message = format!("not added: {reason}");
                        }
                    }
                    sync_pages(&mut gui);
                    update_status(&gui);
                }
            });
        }
        {
            let gui = Arc::clone(&gui);
            ButtonHandle::from_raw(open_id).on_click(move || {
                if let Ok(mut gui) = gui.lock() {
                    let page = rust_widgets::get_widget_text(gui.pages_field);
                    match gui.pages.open_page(&page) {
                        Ok(revision) => {
                            rust_widgets::set_widget_text(gui.pages_error, "");
                            gui.message = format!(
                                "opened {page:?}; current {:?} (revision {revision})",
                                gui.pages.current()
                            );
                            crate::host::log::emit(
                                crate::host::log::Level::Info,
                                "app::run::workbench-pages",
                                &format!("open {page:?}; revision {revision}"),
                            );
                        }
                        Err(reason) => {
                            rust_widgets::set_widget_text(
                                gui.pages_error,
                                &format!("not opened: {reason}"),
                            );
                            gui.message = format!("not opened: {reason}");
                        }
                    }
                    sync_pages(&mut gui);
                    update_status(&gui);
                }
            });
        }
        for (button, forward) in [(back_id, false), (forward_id, true)] {
            let gui = Arc::clone(&gui);
            ButtonHandle::from_raw(button).on_click(move || {
                if let Ok(mut gui) = gui.lock() {
                    let outcome = if forward {
                        gui.pages.forward()
                    } else {
                        gui.pages.back()
                    };
                    match outcome {
                        Ok(page) => {
                            rust_widgets::set_widget_text(gui.pages_error, "");
                            gui.message = format!("now on {page:?}");
                            crate::host::log::emit(
                                crate::host::log::Level::Info,
                                "app::run::workbench-pages",
                                &format!(
                                    "{} -> {page:?}",
                                    if forward { "forward" } else { "back" }
                                ),
                            );
                        }
                        Err(reason) => {
                            rust_widgets::set_widget_text(
                                gui.pages_error,
                                &format!("not moved: {reason}"),
                            );
                            gui.message = format!("not moved: {reason}");
                        }
                    }
                    sync_pages(&mut gui);
                    update_status(&gui);
                }
            });
        }
        {
            let gui = Arc::clone(&gui);
            ButtonHandle::from_raw(close_id).on_click(move || {
                if let Ok(mut gui) = gui.lock() {
                    match gui.pages.close() {
                        Ok(state) => {
                            rust_widgets::set_widget_text(gui.pages_error, "");
                            gui.message = state.clone();
                            crate::host::log::emit(
                                crate::host::log::Level::Info,
                                "app::run::workbench-pages",
                                &format!("window {state}"),
                            );
                        }
                        Err(reason) => {
                            rust_widgets::set_widget_text(
                                gui.pages_error,
                                &format!("not closed: {reason}"),
                            );
                            gui.message = format!("not closed: {reason}");
                        }
                    }
                    sync_pages(&mut gui);
                    update_status(&gui);
                }
            });
        }
    }

    // The Events panel (T13.03/T13.04). A row selects a declared event; Bind/Unbind edit the
    // one attachment the selected node has, and Fire checks a trigger against the declaration.
    if let Some((list_id, field_id, bind_id, unbind_id, fire_id, error_id, note_id)) =
        events_controls
    {
        if let Ok(mut gui) = gui.lock() {
            gui.events_list = list_id;
            gui.events_field = field_id;
            gui.events_error = error_id;
            gui.events_note = note_id;
            sync_events(&mut gui);
        }
        {
            let gui = Arc::clone(&gui);
            subscribe_index(list_id, "item_selected", move |index| {
                if let Ok(mut gui) = gui.lock() {
                    // Rebuilding the list sets the current row to the selection; that programmatic
                    // change emits this signal too, so a row that already is selected is a no-op.
                    if gui.events_selected == index as usize {
                        return;
                    }
                    gui.events_selected = index as usize;
                    let handler = selected_event(&gui)
                        .and_then(|event| gui.events.handler_of(&event).map(str::to_owned))
                        .unwrap_or_default();
                    rust_widgets::set_widget_text(gui.events_field, &handler);
                    sync_events(&mut gui);
                    crate::host::log::emit(
                        crate::host::log::Level::Info,
                        "app::run::workbench-events",
                        &format!(
                            "events row {index} -> {} on node {}",
                            selected_event(&gui).unwrap_or_default(),
                            gui.events.node().get()
                        ),
                    );
                }
            });
        }
        {
            let gui = Arc::clone(&gui);
            ButtonHandle::from_raw(bind_id).on_click(move || {
                if let Ok(mut gui) = gui.lock() {
                    let Some(event) = selected_event(&gui) else {
                        return;
                    };
                    let handler = rust_widgets::get_widget_text(gui.events_field);
                    match gui.events.bind(&event, &handler) {
                        Ok(revision) => {
                            rust_widgets::set_widget_text(gui.events_error, "");
                            let node = gui.events.node().get();
                            gui.message = format!(
                                "bound {event} -> {handler} on node {node} (revision {revision})"
                            );
                            crate::host::log::emit(
                                crate::host::log::Level::Info,
                                "app::run::workbench-events",
                                &format!(
                                    "bind {event} -> {handler} on node {node}: revision {revision}"
                                ),
                            );
                        }
                        Err(reason) => {
                            rust_widgets::set_widget_text(
                                gui.events_error,
                                &format!("not bound: {reason}"),
                            );
                            gui.message = format!("not bound: {reason}");
                            crate::host::log::emit(
                                crate::host::log::Level::Info,
                                "app::run::workbench-events",
                                &format!("bind refused: {reason}"),
                            );
                        }
                    }
                    sync_events(&mut gui);
                    update_status(&gui);
                }
            });
        }
        {
            let gui = Arc::clone(&gui);
            ButtonHandle::from_raw(unbind_id).on_click(move || {
                if let Ok(mut gui) = gui.lock() {
                    let Some(event) = selected_event(&gui) else {
                        return;
                    };
                    match gui.events.unbind(&event) {
                        Ok(revision) => {
                            rust_widgets::set_widget_text(gui.events_error, "");
                            gui.message = format!(
                                "unbound {event} on node {} (revision {revision})",
                                gui.events.node().get()
                            );
                            crate::host::log::emit(
                                crate::host::log::Level::Info,
                                "app::run::workbench-events",
                                &format!(
                                    "unbind {event} on node {}: revision {revision}",
                                    gui.events.node().get()
                                ),
                            );
                        }
                        Err(reason) => {
                            rust_widgets::set_widget_text(
                                gui.events_error,
                                &format!("not unbound: {reason}"),
                            );
                            gui.message = format!("not unbound: {reason}");
                        }
                    }
                    sync_events(&mut gui);
                    update_status(&gui);
                }
            });
        }
        {
            let gui = Arc::clone(&gui);
            ButtonHandle::from_raw(fire_id).on_click(move || {
                if let Ok(mut gui) = gui.lock() {
                    let Some(event) = selected_event(&gui) else {
                        return;
                    };
                    match gui.events.fire(&event) {
                        Ok(lines) => {
                            rust_widgets::set_widget_text(gui.events_error, &lines.join(" | "));
                            gui.message = lines.join(" | ");
                            crate::host::log::emit(
                                crate::host::log::Level::Info,
                                "app::run::workbench-events",
                                &format!("fired {event}: {}", lines.join("; ")),
                            );
                        }
                        Err(reason) => {
                            rust_widgets::set_widget_text(
                                gui.events_error,
                                &format!("not fired: {reason}"),
                            );
                            gui.message = format!("not fired: {reason}");
                        }
                    }
                    sync_events(&mut gui);
                    update_status(&gui);
                }
            });
        }
    }

    // The Data panel (T13.04/T14.05). The sample dataset is edited through the typed schema;
    // the running layer stays unavailable because no instance is started in a design session.
    if let Some((list_id, field_id, apply_id, error_id, note_id)) = data_controls {
        if let Ok(mut gui) = gui.lock() {
            gui.data_list = list_id;
            gui.data_field = field_id;
            gui.data_error = error_id;
            gui.data_note = note_id;
            sync_data(&mut gui);
        }
        {
            let gui = Arc::clone(&gui);
            subscribe_index(list_id, "item_selected", move |index| {
                if let Ok(mut gui) = gui.lock() {
                    // Row 0 is the dataset label, so field rows start at 1.
                    gui.data_selected = (index as usize).saturating_sub(1);
                    sync_data(&mut gui);
                    crate::host::log::emit(
                        crate::host::log::Level::Info,
                        "app::run::workbench-data",
                        &format!(
                            "data row {index} -> field {:?}",
                            gui.inspect.fields().get(gui.data_selected)
                        ),
                    );
                }
            });
        }
        {
            let gui = Arc::clone(&gui);
            ButtonHandle::from_raw(apply_id).on_click(move || {
                if let Ok(mut gui) = gui.lock() {
                    let fields = gui.inspect.fields();
                    let Some(field) = fields.get(gui.data_selected).cloned() else {
                        return;
                    };
                    let text = rust_widgets::get_widget_text(gui.data_field);
                    match gui.inspect.edit(&field, &text) {
                        Ok(version) => {
                            rust_widgets::set_widget_text(gui.data_error, "");
                            gui.message = format!("{field} = {text:?} (data v{version})");
                            crate::host::log::emit(
                                crate::host::log::Level::Info,
                                "app::run::workbench-data",
                                &format!(
                                    "data applied {field} = {text:?}; version {version}; effects {}",
                                    gui.inspect.effects()
                                ),
                            );
                        }
                        Err(reason) => {
                            let reason = gui.inspect.refuse(reason);
                            rust_widgets::set_widget_text(
                                gui.data_error,
                                &format!("not applied: {reason}"),
                            );
                            gui.message = format!(
                                "{field} not applied at version {}: {reason}",
                                gui.inspect.version()
                            );
                            crate::host::log::emit(
                                crate::host::log::Level::Info,
                                "app::run::workbench-data",
                                &format!("data refused {field} = {text:?}: {reason}"),
                            );
                        }
                    }
                    sync_data(&mut gui);
                    update_status(&gui);
                }
            });
        }
    }

    // The debug panel (T19.03–T19.06). Its controls drive a real DAP session on a worker
    // thread; the panel only ever shows a snapshot the worker sent, so a slow adapter cannot
    // freeze the window.
    let debug_panel = handle.new_panel(WIDTH as i32 - 320, 458, 320, 184);
    let debug_title = debug_panel.new_label("Debug", WIDTH as i32 - 312, 466, 304, 20);
    let debug_rows_panel = debug_panel.new_panel(WIDTH as i32 - 312, 490, 304, 100);
    let mut debug_rows_layout = BoxLayout::new(Orientation::Vertical, 2, 0);
    let mut debug_row_labels = Vec::new();
    // Seven rows fit the 112px body at 16px each: adapter, breakpoint, thread, frame, scope,
    // one variable and the raw source position.
    for _ in 0..7 {
        let label = debug_rows_panel.new_label("", 0, 0, 0, 0);
        debug_rows_layout.add_widget(label.raw_id(), 0);
        debug_rows_layout.set_constraints(label.raw_id(), LayoutConstraints::new(16, Some(16)));
        debug_rows_layout.set_size_policy(label.raw_id(), SizePolicy::Fixed);
        debug_row_labels.push(label.raw_id());
    }
    debug_rows_panel.set_layout(Box::new(debug_rows_layout));
    let debug_actions = debug_panel.new_panel(WIDTH as i32 - 312, 606, 304, 28);
    let debug_start = debug_actions.new_button("Start", 0, 0, 73, 28);
    let debug_continue = debug_actions.new_button("Run", 0, 0, 73, 28);
    let debug_step = debug_actions.new_button("Step", 0, 0, 73, 28);
    let debug_stop = debug_actions.new_button("Stop", 0, 0, 73, 28);
    let mut debug_action_layout = BoxLayout::new(Orientation::Horizontal, 4, 0);
    for id in [
        debug_start.raw_id(),
        debug_continue.raw_id(),
        debug_step.raw_id(),
        debug_stop.raw_id(),
    ] {
        debug_action_layout.add_widget(id, 1);
        debug_action_layout.set_size_policy(id, SizePolicy::Expanding);
    }
    debug_actions.set_layout(Box::new(debug_action_layout));
    let mut debug_layout = BoxLayout::new(Orientation::Vertical, 4, 8);
    debug_layout.add_widget(debug_title.raw_id(), 0);
    debug_layout.set_constraints(debug_title.raw_id(), LayoutConstraints::new(20, Some(20)));
    debug_layout.set_size_policy(debug_title.raw_id(), SizePolicy::Fixed);
    debug_layout.add_widget(debug_rows_panel.raw_id(), 1);
    debug_layout.set_size_policy(debug_rows_panel.raw_id(), SizePolicy::Expanding);
    debug_layout.add_widget(debug_actions.raw_id(), 0);
    debug_layout.set_constraints(debug_actions.raw_id(), LayoutConstraints::new(28, Some(28)));
    debug_layout.set_size_policy(debug_actions.raw_id(), SizePolicy::Fixed);
    debug_panel.set_layout(Box::new(debug_layout));
    if let Ok(mut gui) = gui.lock() {
        gui.debug_rows = debug_row_labels.clone();
        gui.placements.push((
            debug_panel.raw_id(),
            Area::RightPanel { index: 2, count: 3 },
        ));
        let entry = gui.panel_widgets.entry("debug".to_owned()).or_default();
        entry.extend([
            debug_panel.raw_id(),
            debug_title.raw_id(),
            debug_rows_panel.raw_id(),
            debug_actions.raw_id(),
            debug_start.raw_id(),
            debug_continue.raw_id(),
            debug_step.raw_id(),
            debug_stop.raw_id(),
        ]);
        entry.extend(debug_row_labels.iter().copied());
    }
    wire_debug_button(&debug_start, DebugCommand::Start, &gui);
    wire_debug_button(&debug_continue, DebugCommand::Continue, &gui);
    wire_debug_button(&debug_step, DebugCommand::Step, &gui);
    wire_debug_button(&debug_stop, DebugCommand::Stop, &gui);
    if let Ok(gui) = gui.lock() {
        let rows = gui.debug.rows();
        for (index, id) in gui.debug_rows.iter().enumerate() {
            let row = rows.get(index).cloned().unwrap_or_default();
            rust_widgets::set_widget_text(*id, &row);
            rust_widgets::set_widget_visible(*id, !row.is_empty());
        }
    }

    // Status bar.
    let status = handle.new_status_bar("", 0, STATUS_Y, WIDTH, STATUS_HEIGHT);
    if let Ok(mut gui) = gui.lock() {
        gui.status = status.raw_id();
        gui.placements.push((status.raw_id(), Area::Status));
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
            if let Ok(mut gui) = gui.lock() {
                gui.placements.push((
                    button.raw_id(),
                    Area::Tab {
                        group: group as u32,
                        index: index as u32,
                        count: 3,
                    },
                ));
            }
        }
        let editor = handle.new_list_box(tx, CONTENT_Y + 30, ew, 535);
        all.push(editor.raw_id());
        if let Ok(mut gui) = gui.lock() {
            gui.placements.push((
                editor.raw_id(),
                Area::Editor {
                    group: group as u32,
                },
            ));
        }
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

    // The design canvas (T09.02). It sits over the first editor group's text area: the tab strip
    // stays visible, and the canvas is created after the editors so the painter's creation order
    // keeps it above them.
    //
    // The control is created through the control backend, not `create_widget_of_kind`: that entry
    // mounts a per-widget platform surface, and the native desktop backend does not provide one,
    // so it answers 0. The custom backend's `create_canvas` is the route the library's own
    // `create_*` functions use on this host.
    let canvas_panel = handle.new_panel(264, 110, 688, 532);
    let canvas_surface = get_control_backend_for_widget(rust_widgets::widget::WidgetKind::Canvas)
        .create_canvas(canvas_panel.raw_id(), 264, 110, 688, 532);
    // The nodes are painted by the canvas itself, not by child controls: a child button would
    // capture the pointer and a marquee could neither start on a container nor cross one.
    canvas_panel.set_layout(Box::new(CanvasLayout::new(canvas_surface, Vec::new())));
    if let Ok(mut gui) = gui.lock() {
        gui.canvas_panel = canvas_panel.raw_id();
        gui.canvas_surface = canvas_surface;
        gui.placements.push((canvas_panel.raw_id(), Area::Canvas));
    }
    watch_focus(canvas_surface, "canvas");

    // Pointer facts for click selection and drag marquee. `mouse_pressed`/`mouse_released` carry
    // no position, so the `mouse_moved` payload keeps the current design-space point; the press
    // then fixes the anchor. Reading the transform from the panel (not this control) is what
    // allows the read while the control is the event's target.
    {
        let gui = Arc::clone(&gui);
        subscribe_unit(canvas_surface, "mouse_pressed", move || {
            if let Ok(mut gui) = gui.lock() {
                gui.drag_active = true;
                gui.drag_start = gui.pointer;
                crate::host::log::emit(
                    crate::host::log::Level::Info,
                    "app::run::workbench-scene",
                    &format!("canvas press at {:?}", gui.drag_start),
                );
            }
        });
    }
    {
        let gui = Arc::clone(&gui);
        let _ = with_widget_mut(canvas_surface, move |widget| {
            (widget as &mut dyn Any)
                .downcast_mut::<rust_widgets::widget::special_widgets::Canvas>()
                .map(|canvas| {
                    let gui = Arc::clone(&gui);
                    canvas.mouse_moved.connect(move |point: Arc<Point>| {
                        if let Ok(mut gui) = gui.lock()
                            && let Some(design) = canvas_point(gui.canvas_panel, point.x, point.y)
                        {
                            let previous = gui.pointer;
                            gui.pointer = Some(design);
                            if gui.drag_active && gui.drag_start.is_none() {
                                gui.drag_start = Some(design);
                            }
                            if gui.drag_active && previous != Some(design) {
                                sync_canvas(&gui);
                            }
                        }
                    });
                })
        });
    }
    {
        let gui = Arc::clone(&gui);
        subscribe_unit(canvas_surface, "mouse_released", move || {
            if let Ok(mut gui) = gui.lock() {
                finish_canvas_drag(&mut gui);
            }
        });
    }

    // Command palette (T05.04/T05.05). It is an overlay created after the docked panels so the
    // painter's creation order keeps it above them; its geometry comes from the same responsive
    // layout as the rest of the chrome, not from a second placement.
    let palette_panel = handle.new_panel(0, 0, 640, 320);
    let palette_title = palette_panel.new_label("Command Palette", 0, 0, 0, 0);
    let palette_query = palette_panel.new_line_edit("", 0, 0, 0, 0);
    palette_query.set_placeholder("Type a command…");
    watch_focus(palette_query.raw_id(), "palette query");
    let palette_list = palette_panel.new_list_box(0, 0, 0, 0);
    let palette_help = palette_panel.new_label("", 0, 0, 0, 0);
    let palette_hint =
        palette_panel.new_label("Type to filter · Run executes · Close hides", 0, 0, 0, 0);
    let palette_actions = palette_panel.new_panel(0, 0, 0, 0);
    let palette_run = palette_actions.new_button("Run", 0, 0, 0, 0);
    let palette_close = palette_actions.new_button("Close", 0, 0, 0, 0);
    let mut action_layout = BoxLayout::new(Orientation::Horizontal, 4, 0);
    for id in [palette_run.raw_id(), palette_close.raw_id()] {
        action_layout.add_widget(id, 1);
        action_layout.set_size_policy(id, SizePolicy::Expanding);
    }
    palette_actions.set_layout(Box::new(action_layout));
    let mut palette_layout = BoxLayout::new(Orientation::Vertical, 4, 8);
    for (id, height, policy) in [
        (palette_title.raw_id(), 20, SizePolicy::Fixed),
        (palette_query.raw_id(), 28, SizePolicy::Fixed),
        (palette_list.raw_id(), 0, SizePolicy::Expanding),
        (palette_help.raw_id(), 20, SizePolicy::Fixed),
        (palette_hint.raw_id(), 18, SizePolicy::Fixed),
        (palette_actions.raw_id(), 28, SizePolicy::Fixed),
    ] {
        palette_layout.add_widget(id, 0);
        palette_layout.set_size_policy(id, policy);
        if height > 0 {
            palette_layout.set_constraints(id, LayoutConstraints::new(height, Some(height)));
        }
    }
    palette_panel.set_layout(Box::new(palette_layout));
    let palette_widgets = vec![
        palette_panel.raw_id(),
        palette_title.raw_id(),
        palette_query.raw_id(),
        palette_list.raw_id(),
        palette_help.raw_id(),
        palette_hint.raw_id(),
        palette_actions.raw_id(),
        palette_run.raw_id(),
        palette_close.raw_id(),
    ];
    for id in &palette_widgets {
        rust_widgets::set_widget_visible(*id, false);
    }

    // A query edit filters through the same palette model the command line uses. The callback
    // runs on the UI thread; the registry only has to be read, so no effect is authorized here.
    {
        let gui = Arc::clone(&gui);
        palette_query.on_value_changed(move |text| {
            if let Ok(mut gui) = gui.lock()
                && gui.palette.is_open()
            {
                let context = palette_context();
                palette_with(&mut gui, |palette, registry, recents| {
                    palette.set_query(text, registry, &context, recents);
                });
                sync_palette(&mut gui);
                crate::host::log::emit(
                    crate::host::log::Level::Info,
                    "app::run::workbench-palette",
                    &format!(
                        "palette query {:?}: {} result(s)",
                        text,
                        gui.palette.result_count()
                    ),
                );
            }
        });
    }
    {
        let gui = Arc::clone(&gui);
        subscribe_unit(palette_query.raw_id(), "return_pressed", move || {
            if let Ok(mut gui) = gui.lock()
                && gui.palette.is_open()
            {
                run_palette_selected(&mut gui);
            }
        });
    }
    {
        // Escape ends editing. A focus move also emits `editing_finished`, so the panel only
        // closes when the query still holds the keyboard — otherwise clicking a row would
        // close the palette before its selection arrived.
        let gui = Arc::clone(&gui);
        subscribe_unit(palette_query.raw_id(), "editing_finished", move || {
            if let Ok(mut gui) = gui.lock()
                && gui.palette.is_open()
                && focused_widget() == Some(gui.palette_query)
            {
                close_palette(&mut gui);
                gui.message = "command palette closed".to_owned();
                update_status(&gui);
            }
        });
    }
    {
        let gui = Arc::clone(&gui);
        subscribe_index(palette_list.raw_id(), "item_selected", move |index| {
            if let Ok(mut gui) = gui.lock()
                && gui.palette.is_open()
                && gui.palette.select(index as usize)
            {
                rust_widgets::set_widget_text(gui.palette_help, &gui.palette.help(&gui.registry));
                crate::host::log::emit(
                    crate::host::log::Level::Info,
                    "app::run::workbench-palette",
                    &format!("palette selected {index}"),
                );
            }
        });
    }
    {
        let gui = Arc::clone(&gui);
        subscribe_index(palette_list.raw_id(), "item_activated", move |index| {
            if let Ok(mut gui) = gui.lock()
                && gui.palette.is_open()
            {
                gui.palette.select(index as usize);
                run_palette_selected(&mut gui);
            }
        });
    }
    {
        let gui = Arc::clone(&gui);
        palette_run.on_click(move || {
            if let Ok(mut gui) = gui.lock()
                && gui.palette.is_open()
            {
                run_palette_selected(&mut gui);
            }
        });
    }
    {
        let gui = Arc::clone(&gui);
        palette_close.on_click(move || {
            if let Ok(mut gui) = gui.lock()
                && gui.palette.is_open()
            {
                close_palette(&mut gui);
                gui.message = "command palette closed".to_owned();
                update_status(&gui);
            }
        });
    }
    if let Ok(mut gui) = gui.lock() {
        gui.palette_widgets = palette_widgets;
        gui.palette_query = palette_query.raw_id();
        gui.palette_list = palette_list.raw_id();
        gui.palette_help = palette_help.raw_id();
        gui.placements.push((palette_panel.raw_id(), Area::Palette));
    }

    if let Ok(mut gui) = gui.lock() {
        gui.sync_panels();
        gui.sync_editors();
        // The initial selection is the same one the canvas paints, so the Inspector, Events and
        // Data panels start from it rather than from a second demo value.
        retarget_panels(&mut gui);
        refresh_inspector(&mut gui);
        sync_pages(&mut gui);
    }
    if let Ok(gui) = gui.lock() {
        let selection: Vec<u64> = gui.scene.selected().iter().map(|node| node.get()).collect();
        crate::host::log::emit(
            crate::host::log::Level::Info,
            "app::run::workbench-scene",
            &format!(
                "scene ready: {} node(s), selection {selection:?}",
                gui.scene.row_nodes().len()
            ),
        );
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
