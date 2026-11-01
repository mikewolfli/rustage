// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The workbench window: the first real integration of the shell models (task F04/F21).
//!
//! The models already exist — [`Workbench`] (mode, document, target, status), [`Panels`] (which
//! panels a host has and where), [`Groups`]/[`Tabs`](crate::app::shell::tabs::Tabs) (editor groups and their documents), and
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

use rust_widgets::app::{ButtonHandle, MenuHandle, WidgetHandle, WindowHandle};
use rust_widgets::control_backend::get_control_backend_for_widget;
use rust_widgets::core::{Point, Rect};
use rust_widgets::layout::{BoxLayout, Layout as _, LayoutConstraints, Orientation, SizePolicy};
use rust_widgets::widget::PropertyGrid;
use rust_widgets::widget::capability::CapabilityValue;
use rust_widgets::widget::runtime::{clear_focus, focus_widget, focused_widget, with_widget_mut};
use rust_widgets::widget::special_widgets::code_editor::CodeEditor;
use rust_widgets::widget::{GroupBox, Menu, ToolBar};

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

/// The Inspector's property grid, the Clear/Reset/Values buttons, the theme field and button,
/// then the error line and note.
type InspectorControls = (u64, u64, u64, u64, u64, u64, u64, u64);

/// The Events panel's list, handler field, Bind/Unbind/Fire buttons, error line and note.
type EventControls = (u64, u64, u64, u64, u64, u64, u64);

/// The Data panel's property grid, error line and note.
type DataControls = (u64, u64, u64);

/// The Pages panel's list, field, Add/Open/Back/Fwd/Close buttons, error line and note.
type PageControls = (u64, u64, u64, u64, u64, u64, u64, u64, u64);

/// One editor group's widgets.
struct GroupView {
    /// The library's tab strip for this group.
    tabs: u64,
    editor: u64,
    /// The group's ids, so hiding a group hides its tab strip and editor together.
    all: Vec<u64>,
}

/// One interface label the window can re-read after a translation reload.
#[derive(Clone, Copy)]
enum LabelKind {
    /// A control whose `text` property is the label.
    Text,
    /// A `Menu` widget whose `title` is the label.
    MenuTitle,
    /// A `GroupBox` panel whose `title` is the label.
    PanelTitle,
}

struct Translatable {
    id: u64,
    key: String,
    kind: LabelKind,
}

/// A translated label, through the library's global manager (loaded by `main` from the same JSON
/// packs the CLI reads).
fn tr(key: &str) -> String {
    rust_widgets::i18n::translate(key)
}

/// A translated label with its `{name}` placeholders filled.
fn tr_args(key: &str, arguments: &[(&str, &str)]) -> String {
    rust_widgets::i18n::translate_with_args(key, None, 0, arguments)
}

/// The catalog key for a panel id.
fn panel_key(id: &str) -> &'static str {
    match id {
        "pages" => "ui.panel.pages",
        "outline" => "ui.panel.outline",
        "events" => "ui.panel.events",
        "inspector" => "ui.panel.inspector",
        "data" => "ui.panel.data",
        "messages" => "ui.panel.messages",
        "problems" => "ui.panel.problems",
        "debug" => "ui.panel.debug",
        _ => "ui.panel.outline",
    }
}

/// Adds one menu leaf and remembers its label key.
///
/// A registry menu leaf's key is `menu.<menu>.item.<command>`; the menu model also names planned
/// leaves, whose key exists for the same reason (the label is visible even when the command is not
/// implemented yet). The model's `title` is the fallback when a pack does not carry the key.
fn add_menu_leaf(
    handle: &WindowHandle,
    menu: &MenuHandle,
    key: &str,
    command: &str,
    title: &str,
    actions: &mut BTreeMap<u64, String>,
    gui: &Arc<Mutex<Gui>>,
) {
    let translated = tr(key);
    let label = if translated == key {
        title.to_owned()
    } else {
        translated
    };
    let item = handle.new_menu_item(menu, &label, None);
    actions.insert(item.raw_id(), format!("command:{command}"));
    if let Ok(mut gui) = gui.lock() {
        gui.translatable.push(Translatable {
            id: item.raw_id(),
            key: key.to_owned(),
            kind: LabelKind::Text,
        });
    }
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
    /// The command palette's state and the one control that shows it.
    palette: Palette,
    palette_surface: u64,
    registry: Registry,
    recents: Recents,
    /// The Inspector's one editable property and the controls that mirror it.
    property: PropertyState,
    /// The library's property grid that shows and edits the Inspector's rows.
    property_grid: u64,
    /// The grid row of the editable property, so a committed cell is the one the model validates.
    property_row: usize,
    /// Rows the Values button read from the inspection, shown under the property rows until the
    /// selection changes.
    inspect_extra: Vec<String>,
    property_error: u64,
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
    /// The Data panel: a property grid over the sample schema, and the read-only inspection.
    inspect: InspectState,
    data_grid: u64,
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
    canvas_snapshot: Arc<Mutex<Arc<CanvasSnapshot>>>,
    canvas_scheduled: Arc<AtomicBool>,
    /// The scene revision the last canvas snapshot was built from; a pointer event that changes
    /// nothing in the scene reuses the node list instead of allocating it again.
    canvas_revision: u64,
    /// The Outline rows and their node ids, rebuilt only when names or membership change.
    outline_rows: Vec<String>,
    outline_index: BTreeMap<NodeId, usize>,
    outline_content_revision: u64,
    /// Labels re-read after a translation hot reload.
    translatable: Vec<Translatable>,
    /// The toolbar and the catalog keys of its items, so a reload can rebuild its labels.
    toolbar: u64,
    toolbar_items: Vec<(&'static str, &'static str)>,
    /// The Inspector's right-click menu.
    context_menu: u64,
    /// The scene node's right-click menu.
    scene_menu: u64,
}

/// One visible node as the canvas paints it: rectangle, label, and hidden/locked/selected flags.
type CanvasNode = (SceneRect, String, bool, bool, bool);

/// One frame of the canvas: each visible node's rectangle, label, hidden/locked/selected state,
/// and the marquee if a drag is in progress.
///
/// The node list is shared behind an `Arc` so a scheduled frame clones the pointer, not every
/// name. Only the marquee changes between pointer events.
struct CanvasSnapshot {
    nodes: Arc<[CanvasNode]>,
    marquee: Option<SceneRect>,
}

/// How far a press must move in design space before it is a marquee rather than a click.
const DRAG_THRESHOLD: u32 = 8;

/// Reports a widget or model operation that failed during window construction or a callback.
///
/// These used to be `let _ =`: a refused registration or layout quietly removed a control with no
/// record of why. The window keeps running, but the log says what did not happen (principle #22).
fn warn_failure(what: &str, reason: impl std::fmt::Display) {
    crate::host::log::emit(
        crate::host::log::Level::Warn,
        "app::run::workbench-gui",
        &format!("{what}: {reason}"),
    );
}

/// Shows a panel only when its backing service exists, and says why when it does not.
fn show_panel_if_available(panels: &mut Panels, id: &str, services: &BTreeSet<String>) {
    let Some(spec) = panels.spec(id) else {
        warn_failure(
            "a panel was not shown",
            format!("there is no panel {id}; a panel is registered before it is shown"),
        );
        return;
    };
    let service = spec.service().map(str::to_owned);
    if !spec.is_available(services) {
        if let Err(error) = panels.hide(id) {
            warn_failure("an unavailable panel could not be hidden", error.message());
        }
        crate::host::log::emit(
            crate::host::log::Level::Info,
            "app::run::workbench-gui",
            &format!(
                "panel {id} is registered but {} is unavailable, so it is not shown",
                service.as_deref().unwrap_or("its service")
            ),
        );
        return;
    }
    if let Err(error) = panels.show(id) {
        warn_failure("a panel could not be shown", error.message());
    }
}

impl Gui {
    fn status_line(&self) -> String {
        let document = self
            .workbench
            .document()
            .map(str::to_owned)
            .unwrap_or_else(|| tr("ui.status.no-document"));
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
        let groups: Vec<(u64, Vec<String>, String, Option<usize>)> = self
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
                let active = tabs
                    .tabs()
                    .iter()
                    .position(|tab| tabs.active().is_some_and(|active| active == tab));
                (group.get(), documents, text, active)
            })
            .collect();
        for (index, view) in self.group_views.iter_mut().enumerate() {
            let Some((_, documents, text, active)) = groups.get(index) else {
                for widget in &view.all {
                    rust_widgets::set_widget_visible(*widget, false);
                }
                continue;
            };
            for widget in &view.all {
                rust_widgets::set_widget_visible(*widget, true);
            }
            // The tab strip is the library's `TabWidget`; the model still owns which documents
            // exist and which one is active. Titles and count are mirrored here, so a document
            // that appears or disappears cannot leave a stale tab behind.
            let tabs = rust_widgets::app::TabWidgetHandle::from_raw(view.tabs);
            let mut count = tabs.tab_count();
            for (tab, document) in documents.iter().enumerate() {
                let label = document.rsplit('/').next().unwrap_or(document);
                if tab < count {
                    tabs.set_tab_text(tab, label);
                } else if tabs.add_tab(label, None).is_some() {
                    count += 1;
                }
            }
            while count > documents.len() {
                if tabs.remove_tab(count - 1) {
                    count -= 1;
                } else {
                    break;
                }
            }
            if let Some(active) = active {
                tabs.set_current_index(*active);
            }
            let applied = with_widget_mut(view.editor, |widget| {
                let any: &mut dyn std::any::Any = widget;
                match any.downcast_mut::<CodeEditor>() {
                    Some(editor) => {
                        if editor.text() != *text {
                            editor.set_text(text);
                        }
                        true
                    }
                    None => false,
                }
            });
            if applied != Some(true) {
                self.message = "the editor surface did not accept the document".to_owned();
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
            "app::run::workbench",
            &format!("control {id} publishes no {event} signal"),
        );
    }
}

/// Subscribes one payload-carrying signal whose payload is a string.
fn subscribe_string(
    id: u64,
    event: &'static str,
    mut callback: impl FnMut(&str) + Send + Sync + 'static,
) {
    let subscribed = with_widget_mut(id, move |widget| {
        widget.event_signal_dyn(event).map(|signal| {
            signal.subscribe(Box::new(move |payload| {
                if let CapabilityValue::String(text) = payload {
                    callback(text);
                }
            }))
        })
    })
    .flatten()
    .is_some();
    if !subscribed {
        crate::host::log::emit(
            crate::host::log::Level::Warn,
            "app::run::workbench",
            &format!("control {id} publishes no {event} signal"),
        );
    }
}

/// Subscribes one payload-carrying signal shaped `(row index, text)`.
///
/// `PropertyGrid::cell_changed` carries the row and the committed text as a tuple; the generic
/// `subscribe_index` helper only unwraps a bare `UInt`, so this is the one place that knows the
/// shape.
fn subscribe_cell(
    id: u64,
    event: &'static str,
    mut callback: impl FnMut(u64, &str) + Send + Sync + 'static,
) {
    let subscribed = with_widget_mut(id, move |widget| {
        widget.event_signal_dyn(event).map(|signal| {
            signal.subscribe(Box::new(move |payload| {
                if let CapabilityValue::Tuple(values) = payload
                    && let (Some(CapabilityValue::UInt(row)), Some(CapabilityValue::String(text))) =
                        (values.first(), values.get(1))
                {
                    callback(*row, text);
                }
            }))
        })
    })
    .flatten()
    .is_some();
    if !subscribed {
        crate::host::log::emit(
            crate::host::log::Level::Warn,
            "app::run::workbench",
            &format!("control {id} publishes no {event} signal"),
        );
    }
}

/// A row as the property grid shows it: name, value, and whether a person may edit it.
type GridRow = (String, String, bool);

/// Replaces every row of a `PropertyGrid`.
///
/// The grid owns the rows; the host only hands it the current model. `clear` + `add_property` is
/// the widget's own structure-change path, so its cached display rows and hit test follow.
fn set_property_rows(id: u64, rows: &[GridRow], selected: Option<usize>) {
    if id == 0 {
        return;
    }
    let rows = rows.to_vec();
    let changed = with_widget_mut(id, move |widget| {
        let Some(grid) = (widget as &mut dyn Any).downcast_mut::<PropertyGrid>() else {
            return false;
        };
        grid.clear();
        for (name, value, editable) in &rows {
            grid.add_property(name.clone(), value.clone(), *editable);
        }
        grid.set_selected_index(selected);
        true
    })
    .unwrap_or(false);
    if !changed {
        warn_failure(
            "the property grid could not be filled",
            "the mounted control was not a PropertyGrid",
        );
    }
}

/// Rewrites one cell of a `PropertyGrid` without rebuilding the row set.
fn set_property_cell(id: u64, row: usize, value: &str) {
    if id == 0 {
        return;
    }
    let value = value.to_owned();
    let changed = with_widget_mut(id, move |widget| {
        let Some(grid) = (widget as &mut dyn Any).downcast_mut::<PropertyGrid>() else {
            return false;
        };
        grid.set_value(row, value)
    })
    .unwrap_or(false);
    if !changed {
        warn_failure(
            "a property cell could not be rewritten",
            format!("row {row} is not in the grid"),
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
    let surface = gui.palette_surface;
    if surface == 0 {
        return;
    }
    let entries: Vec<rust_widgets::widget::special_widgets::CommandEntry> = gui
        .palette
        .entries()
        .iter()
        .map(|entry| {
            let title = match entry.reason() {
                Some(reason) => format!("{} — disabled: {reason}", entry.title()),
                None => entry.title().to_owned(),
            };
            rust_widgets::widget::special_widgets::CommandEntry::new(entry.id(), title)
                .with_category(entry.category())
        })
        .collect();
    let query = gui.palette.query().to_owned();
    // `CommandPalette` emits `query_changed` from `set_query`; the callback locks this same `Gui`,
    // so the write happens on the next main-loop turn, after the caller releases the mutex.
    let _ = rust_widgets::schedule_on_main(Duration::from_millis(0), move || {
        let _ = with_widget_mut(surface, move |widget| {
            let Some(palette) = (widget as &mut dyn Any)
                .downcast_mut::<rust_widgets::widget::special_widgets::CommandPalette>(
            ) else {
                return false;
            };
            palette.set_entries(entries);
            palette.set_query(query);
            true
        });
    });
}

/// Re-reads every remembered label and the toolbar from the global catalogue.
///
/// This is what a hot reload paints: the catalogue changed on disk, so the controls are handed the
/// new text without rebuilding the window. A control that no longer exists is skipped by
/// `with_widget_mut`/`set_widget_text` returning false, not treated as a reason to stop.
fn retranslate(gui: &mut Gui) {
    for label in &gui.translatable {
        let text = tr(&label.key);
        match label.kind {
            LabelKind::Text => rust_widgets::set_widget_text(label.id, &text),
            LabelKind::MenuTitle => {
                let _ = with_widget_mut(label.id, move |widget| {
                    let Some(menu) = (widget as &mut dyn Any).downcast_mut::<Menu>() else {
                        return false;
                    };
                    menu.set_title(text);
                    true
                });
            }
            LabelKind::PanelTitle => {
                let _ = with_widget_mut(label.id, move |widget| {
                    let Some(panel) = (widget as &mut dyn Any).downcast_mut::<GroupBox>() else {
                        return false;
                    };
                    panel.set_title(text);
                    true
                });
            }
        }
    }
    if gui.toolbar != 0 {
        let items: Vec<(String, String)> = gui
            .toolbar_items
            .iter()
            .map(|(key, action)| (tr(key), (*action).to_owned()))
            .collect();
        let _ = with_widget_mut(gui.toolbar, move |widget| {
            let Some(bar) = (widget as &mut dyn Any).downcast_mut::<ToolBar>() else {
                return false;
            };
            bar.clear();
            for (index, (label, action)) in items.iter().enumerate() {
                bar.insert_item(
                    index,
                    rust_widgets::widget::menu_toolbar::tool_bar::ToolBarItem::new(
                        action.as_str(),
                        label.as_str(),
                    ),
                );
            }
            true
        });
    }
    update_status(gui);
}

/// Drives the library's translation hot reload from the main thread.
///
/// The watcher is enabled over the `language/` directory; this poll drains its events and repaints
/// the remembered labels. It is deliberately a timer rather than a per-pixel hook: translation
/// files change at human speed, and a 250 ms poll costs one mutex check per tick.
fn schedule_i18n(gui: Arc<Mutex<Gui>>) {
    let next = Arc::clone(&gui);
    let _ = rust_widgets::schedule_on_main(Duration::from_millis(250), move || {
        let applied = rust_widgets::i18n::pump_hot_reload();
        if applied > 0
            && let Ok(mut gui) = next.lock()
        {
            retranslate(&mut gui);
            gui.message = format!("language reloaded: {applied} file event(s)");
            update_status(&gui);
            crate::host::log::emit(
                crate::host::log::Level::Info,
                "app::run::workbench-i18n",
                &format!("translation hot reload applied {applied} event(s)"),
            );
        }
        schedule_i18n(next);
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
    // The grid's own rows carry the Inspector's contract facts; the editable row is the one the
    // model validates, and every other row is a read-only answer beside it (blue21 W-2d).
    let mut rows: Vec<GridRow> = vec![
        (tr("ui.inspector.node"), nodes, false),
        (
            tr("ui.inspector.kind"),
            format!("{kind} · {contract}"),
            false,
        ),
        (
            gui.property.name().as_str().to_owned(),
            gui.property.summary(),
            true,
        ),
        (
            tr("ui.inspector.theme"),
            format!(
                "{} · computed background {}",
                gui.inspect.theme().unwrap_or("(none)"),
                gui.inspect
                    .background(gui.property.node())
                    .unwrap_or("(not themed)")
            ),
            false,
        ),
    ];
    // The contract's default and accepted tokens are shown beside the value, so an enum property
    // does not ask a person to type a token the schema already enumerates.
    if let Some(default) = gui.property.contract_default() {
        rows.push((tr("ui.inspector.default"), default, false));
    }
    let tokens = gui.property.accepted_tokens();
    if !tokens.is_empty() {
        rows.push((tr("ui.inspector.accepted"), tokens.join(" | "), false));
    }
    for line in &gui.inspect_extra {
        rows.push((tr("ui.inspector.inspect"), line.clone(), false));
    }
    if gui.property.is_mixed() {
        rows.push((
            tr("ui.inspector.selection"),
            tr("ui.placeholder.mixed"),
            false,
        ));
    }
    set_property_rows(gui.property_grid, &rows, Some(gui.property_row));
    // Contract facts are reported instead of hidden behind one node's schema: an unavailable
    // schema refuses edits, and a selection whose properties disagree is not agreement.
    if gui.property_error != 0 {
        let note = gui
            .property
            .schema_unavailable()
            .map(|reason| tr_args("ui.error.schema-unavailable", &[("reason", reason)]))
            .or_else(|| {
                gui.property
                    .incompatible()
                    .map(|reason| tr_args("ui.error.contracts-differ", &[("reason", reason)]))
            })
            .or_else(|| {
                gui.property
                    .contract_note()
                    .map(|note| tr_args("ui.error.contracts", &[("reason", note)]))
            });
        if let Some(note) = note {
            rust_widgets::set_widget_text(gui.property_error, &note);
        }
    }
    if gui.inspect_note != 0 {
        rust_widgets::set_widget_text(
            gui.inspect_note,
            &tr_args(
                "ui.note.inspector",
                &[
                    ("version", &gui.inspect.version().to_string()),
                    ("effects", &gui.inspect.effects().to_string()),
                ],
            ),
        );
    }
    crate::host::log::emit(
        crate::host::log::Level::Info,
        "app::run::workbench-property",
        &format!("inspector rows: {} [contract {contract}]", rows[2].1),
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
        &tr_args(
            "ui.note.events",
            &[
                ("status", &status),
                ("revision", &gui.events.revision().to_string()),
                ("handlers", &handlers),
                ("wiring", &wiring),
                ("last", &last),
            ],
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

/// Mirrors the Data model into its property grid and its labels.
///
/// The grid's first row names the dataset and is read-only; every field after it is editable, so
/// committing a cell edits exactly the field whose row it was.
fn sync_data(gui: &mut Gui) {
    let fields = gui.inspect.fields();
    let mut rows: Vec<GridRow> = vec![(
        tr("ui.data.dataset"),
        format!(
            "form v{} · effects {} · runtime none",
            gui.inspect.version(),
            gui.inspect.effects()
        ),
        false,
    )];
    for field in &fields {
        let value = gui.inspect.value_of(field).unwrap_or_default();
        rows.push((field.clone(), value, true));
    }
    // Keep the schema's diagnostics visible instead of hiding them behind the editable rows.
    for diagnostic in gui.inspect.rows() {
        if diagnostic.starts_with("problem:") || diagnostic.starts_with("sample refused:") {
            rows.push((tr("ui.data.diagnostic"), diagnostic, false));
        }
    }
    let selected = gui.data_selected.min(fields.len().saturating_sub(1));
    gui.data_selected = selected;
    set_property_rows(gui.data_grid, &rows, Some(selected + 1));
    if let Some(problem) = gui.inspect.problem() {
        rust_widgets::set_widget_text(
            gui.data_error,
            &tr_args("ui.error.not-applied", &[("reason", problem)]),
        );
    }
    rust_widgets::set_widget_text(
        gui.data_note,
        &tr_args(
            "ui.note.data",
            &[
                ("version", &gui.inspect.version().to_string()),
                ("effects", &gui.inspect.effects().to_string()),
            ],
        ),
    );
}

/// Mirrors the Pages model into its list and its labels.
fn sync_pages(gui: &mut Gui) {
    let rows = gui.pages.rows();
    let selected = gui.pages_selected;
    rust_widgets::set_widget_text(
        gui.pages_note,
        &tr_args(
            "ui.note.pages",
            &[
                ("count", &gui.pages.pages().len().to_string()),
                ("current", gui.pages.current().unwrap_or("(none)")),
                ("revision", &gui.pages.revision().to_string()),
            ],
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

/// Commits a typed text through the schema editor (task T11.01).
fn apply_property(gui: &mut Gui, text: &str) {
    crate::host::log::emit(
        crate::host::log::Level::Info,
        "app::run::workbench-property",
        &format!("inspector Apply text {} character(s)", text.chars().count()),
    );
    match gui.property.apply(text) {
        Ok(revision) => {
            let nodes = gui.property.nodes().to_vec();
            // Only a `title` contract is the tree's structural name. A button/label's `text` is
            // stored by the schema editor and must not rename the node in the Outline or canvas.
            let renames = gui.property.edits_node_name();
            let mut applied = Vec::new();
            for node in &nodes {
                if renames {
                    match gui.scene.rename(*node, text) {
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
            rust_widgets::set_widget_text(
                gui.property_error,
                &tr_args("ui.error.not-applied", &[("reason", &reason)]),
            );
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

/// Empties the Inspector's editable cell without changing the valid model value.
fn clear_property(gui: &mut Gui) {
    set_property_cell(gui.property_grid, gui.property_row, "");
    rust_widgets::set_widget_text(gui.property_error, "");
    gui.message = tr_args(
        "ui.status.inspector-cleared",
        &[
            ("name", gui.property.name().as_str()),
            ("value", &format!("{:?}", gui.property.value_text())),
        ],
    );
    update_status(gui);
    crate::host::log::emit(
        crate::host::log::Level::Info,
        "app::run::workbench-property",
        &format!(
            "inspector field cleared; valid value unchanged: {:?}",
            gui.property.value_text()
        ),
    );
}

/// Takes the explicit edit away and restores each node's structural name.
fn reset_property(gui: &mut Gui) {
    let nodes = gui.property.nodes().to_vec();
    let renames = gui.property.edits_node_name();
    let (count, lines) = gui.property.reset();
    gui.inspect_extra.clear();
    if renames {
        for node in &nodes {
            if let Some(value) = gui.property.value_for(*node)
                && let Err(reason) = gui.scene.rename(*node, &value)
            {
                warn_failure("a node could not be renamed on reset", reason);
            }
        }
    }
    rust_widgets::set_widget_text(gui.property_error, &lines.join(" | "));
    refresh_inspector(gui);
    sync_outline(gui);
    sync_canvas(gui);
    gui.message = tr_args("ui.status.reset-nodes", &[("count", &count.to_string())]);
    update_status(gui);
    crate::host::log::emit(
        crate::host::log::Level::Info,
        "app::run::workbench-property",
        &format!("inspector reset {count} node(s): {}", lines.join("; ")),
    );
}

/// Reads design/computed/runtime and the source location without dispatching a trigger.
fn inspect_values(gui: &mut Gui) {
    let selected = gui.scene.selected().to_vec();
    {
        let Gui { inspect, scene, .. } = &mut *gui;
        inspect.retarget(scene, &selected);
    }
    match gui.inspect.inspect() {
        Ok(lines) => {
            gui.inspect_extra = lines.clone();
            refresh_inspector(gui);
            gui.message = tr_args(
                "ui.status.inspected",
                &[
                    ("version", &gui.inspect.version().to_string()),
                    ("effects", &gui.inspect.effects().to_string()),
                ],
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
            let text = tr_args("ui.error.inspect-refused", &[("reason", &reason)]);
            rust_widgets::set_widget_text(gui.property_error, &text);
            gui.message = text;
        }
    }
    if gui.inspect_note != 0 {
        rust_widgets::set_widget_text(
            gui.inspect_note,
            &tr_args(
                "ui.note.inspector",
                &[
                    ("version", &gui.inspect.version().to_string()),
                    ("effects", &gui.inspect.effects().to_string()),
                ],
            ),
        );
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
/// Rows are rebuilt only when a name or the membership changed; a click or a marquee only moves
/// the current row, so it does not pay for every row's indentation string again.
fn sync_outline(gui: &mut Gui) {
    let list = gui.outline_list;
    if list == 0 {
        return;
    }
    let revision = gui.scene.content_revision();
    let rows_changed = revision != gui.outline_content_revision;
    if rows_changed {
        gui.outline_rows = gui.scene.rows();
        gui.outline_index = gui
            .scene
            .row_nodes()
            .into_iter()
            .enumerate()
            .map(|(index, node)| (node, index))
            .collect();
        gui.outline_content_revision = revision;
    }
    let selected = gui
        .scene
        .primary()
        .and_then(|node| gui.outline_index.get(&node).copied());
    let rows = rows_changed.then(|| gui.outline_rows.clone());
    let _ = rust_widgets::schedule_on_main(Duration::from_millis(0), move || {
        if let Some(rows) = rows {
            rust_widgets::list_box_clear_items(list);
            for row in &rows {
                let _ = rust_widgets::list_box_add_item(list, row);
            }
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
fn sync_canvas(gui: &mut Gui) {
    let marquee = if gui.drag_active {
        match (gui.drag_start, gui.pointer) {
            (Some(start), Some(end)) => Some(span(start, end)),
            _ => None,
        }
    } else {
        None
    };
    let revision = gui.scene.revision();
    // Building the node list allocates one name per visible node, so it happens only when the
    // scene actually changed. A drag emits a pointer event per pixel; those events only move the
    // marquee and reuse the shared node list.
    let nodes = if revision == gui.canvas_revision {
        match gui.canvas_snapshot.lock() {
            Ok(snapshot) => Arc::clone(&snapshot.nodes),
            Err(_) => return,
        }
    } else {
        let selected = gui.scene.selected();
        let nodes: Arc<[CanvasNode]> = gui
            .scene
            .nodes()
            .into_iter()
            .map(|node| {
                let name = gui.scene.name(node.node).unwrap_or("?");
                let label = if gui.scene.is_locked(node.node) {
                    format!("{name} [locked]")
                } else {
                    name.to_owned()
                };
                (
                    node.rect,
                    label,
                    gui.scene.is_hidden(node.node),
                    gui.scene.is_locked(node.node),
                    selected.contains(&node.node),
                )
            })
            .collect::<Vec<_>>()
            .into();
        gui.canvas_revision = revision;
        nodes
    };
    if let Ok(mut snapshot) = gui.canvas_snapshot.lock() {
        // Replacing the `Arc` is cheap and keeps the old list alive in any frame already reading
        // it.
        *snapshot = Arc::new(CanvasSnapshot { nodes, marquee });
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
        if let Ok(snapshot) = snapshot.lock() {
            let frame = Arc::clone(&snapshot);
            drop(snapshot);
            draw_canvas(panel, surface, &frame);
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
fn draw_canvas(panel: u64, surface: u64, frame: &CanvasSnapshot) {
    let Some(rect) = with_widget_mut(panel, |widget| widget.geometry()) else {
        return;
    };
    let transform = CanvasTransform::new(rect);
    static FONT: std::sync::OnceLock<rust_widgets::core::Font> = std::sync::OnceLock::new();
    let font = FONT.get_or_init(|| rust_widgets::core::Font::new("Inter", 13.0, false, false));
    let _ = with_widget_mut(surface, |widget| {
        let Some(canvas) = (widget as &mut dyn Any)
            .downcast_mut::<rust_widgets::widget::special_widgets::Canvas>()
        else {
            return;
        };
        canvas.clear();
        canvas.fill_rect(rect, CANVAS_BACKGROUND);
        for (design, name, hidden, locked, selected) in frame.nodes.iter() {
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
            canvas.draw_text(
                Point::new(node.x + 8, node.y + 8),
                name,
                font,
                if *locked { MUTED_TEXT } else { TEXT },
            );
        }
        if let Some(marquee) = frame.marquee {
            canvas.draw_rect(transform.place(marquee), ACCENT);
        }
    });
}

/// Applies one scene context-menu action to the primary selection.
///
/// The actions mutate the same `CanvasState` the Outline and canvas clicks use, so a menu entry and
/// a pointer gesture cannot disagree. A hidden node leaves the selection (the canvas no longer
/// draws it); a locked node stays selected but is refused by the design gestures it would block.
fn scene_context_action(gui: &mut Gui, action: &str) {
    let Some(node) = gui.scene.primary() else {
        gui.message = tr("ui.status.unknown-action");
        update_status(gui);
        return;
    };
    let name = gui.scene.name(node).unwrap_or("?").to_owned();
    match action {
        "select-parent" => match gui.scene.parent(node) {
            Some(parent) => select_node(gui, parent, "context"),
            None => {
                let node_text = node.get().to_string();
                gui.message = tr_args("ui.status.no-parent", &[("node", &node_text)]);
                update_status(gui);
            }
        },
        "toggle-lock" => {
            let locked = !gui.scene.is_locked(node);
            gui.scene.set_locked(node, locked);
            gui.message = if locked {
                tr_args("ui.status.locked", &[("name", &name)])
            } else {
                tr_args("ui.status.unlocked", &[("name", &name)])
            };
            retarget_panels(gui);
            sync_canvas(gui);
            sync_outline(gui);
            update_status(gui);
        }
        "toggle-hide" => {
            let hidden = !gui.scene.is_hidden(node);
            gui.scene.set_hidden(node, hidden);
            gui.message = if hidden {
                tr_args("ui.status.hidden-node", &[("name", &name)])
            } else {
                tr_args("ui.status.shown-node", &[("name", &name)])
            };
            retarget_panels(gui);
            sync_canvas(gui);
            sync_outline(gui);
            update_status(gui);
        }
        "raise" => {
            if gui.scene.raise(node) {
                gui.message = tr_args("ui.status.raised", &[("name", &name)]);
            } else {
                gui.message = tr_args("ui.status.already-top", &[("name", &name)]);
            }
            sync_canvas(gui);
            sync_outline(gui);
            update_status(gui);
        }
        _ => {}
    }
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
            gui.inspect_extra.clear();
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
        gui.inspect_extra.clear();
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
    if gui.palette_surface != 0 {
        rust_widgets::set_widget_visible(gui.palette_surface, true);
    }
    // The palette is placed by the same responsive layout as the chrome, so the overlay follows
    // a resize instead of keeping the geometry it was created with.
    gui.refresh_layout();
    sync_palette(gui);
    if gui.palette_surface != 0 && focus_widget(gui.palette_surface) {
        gui.message = tr("ui.status.command-palette");
        crate::host::log::emit(
            crate::host::log::Level::Info,
            "app::run::workbench-palette",
            &format!("palette opened; previous focus {previous:?}"),
        );
    } else {
        gui.message = tr("ui.status.palette-could-not-focus");
        crate::host::log::emit(
            crate::host::log::Level::Info,
            "app::run::workbench-palette",
            "palette could not focus its control",
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
    if gui.palette_surface != 0 {
        rust_widgets::set_widget_visible(gui.palette_surface, false);
    }
    match previous {
        Some(id) if id != gui.palette_surface => {
            if !focus_widget(id) {
                clear_focus();
            }
        }
        Some(_) | None => {
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

fn run_palette_id(gui: &mut Gui, id: &str) {
    let Some(entry) = gui.palette.entry_by_id(id).cloned() else {
        gui.message = format!("{id}: no such command is registered");
        update_status(gui);
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
            open_onboarding(gui);
            gui.message = format!("{id}: welcome opened");
            update_status(gui);
            return;
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

fn run_command(gui: &mut Gui, command: &str) {
    let outcome = match command {
        "view.palette" => {
            open_palette(gui);
            return;
        }
        "view.welcome" => {
            open_onboarding(gui);
            return;
        }
        "panel.outline" => {
            apply_action(gui, "toggle:outline");
            return;
        }
        "panel.inspector" => {
            apply_action(gui, "toggle:inspector");
            return;
        }
        "panel.close" => "no focused docked panel is tracked in this window yet".to_owned(),
        "keys.show" => shortcut_summary(),
        other => format!("{other}: resolved; the workbench has no handler for it yet"),
    };
    gui.message = outcome;
    update_status(gui);
    crate::host::log::emit(
        crate::host::log::Level::Info,
        "app::run::workbench-menu",
        &format!("menu command {command}: {}", gui.message),
    );
}

/// Opens the onboarding document in the focused editor group.
///
/// The text is built from the live command registry, so the “what can I do?” page cannot list a
/// command the build does not have (task T24.07). Opening it is the `view.welcome` command's work:
/// the mode changes and a real document appears, rather than a mode flag with no surface.
fn open_onboarding(gui: &mut Gui) {
    let text = onboarding_text(&gui.registry);
    gui.workbench.set_mode(Mode::Welcome);
    gui.groups.pin("WELCOME.md");
    if gui.groups.text("WELCOME.md") != Some(text.as_str())
        && let Err(reason) = gui.groups.type_into(text)
    {
        warn_failure("the onboarding document could not be filled", reason);
    }
    gui.sync_editors();
    gui.message = tr("ui.status.ready");
    update_status(gui);
}

/// The onboarding page: a short guided flow plus every command this build registers.
fn onboarding_text(registry: &Registry) -> String {
    let mut text = String::from(
        "# Welcome to rustage\n\n\
         rustage designs a rust-widgets interface as a document, previews it without a build, and\n\
         generates Rust source for the selected target.\n\n\
         ## Try this first\n\n\
         1. Open the command palette: toolbar “Commands”, View → Command Palette.\n\
         2. Select a node in the Outline or on the canvas; edit it in the Inspector.\n\
         3. Run a live edit and watch the preview update without cargo.\n\
         4. Generate source for the current target and check it builds.\n\n\
         `--help` lists the non-window workflows; `--headless` runs validate/generate/check in CI.\n\n\
         ## Commands in this build\n\n",
    );
    let mut by_category: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for command in registry.commands() {
        by_category
            .entry(command.category())
            .or_default()
            .push(format!(
                "- `{}` — {}: {}",
                command.id(),
                command.title(),
                command.help()
            ));
    }
    for (category, lines) in by_category {
        text.push_str(&format!("### {category}\n\n"));
        for line in lines {
            text.push_str(&line);
            text.push('\n');
        }
        text.push('\n');
    }
    text
}

fn apply_action(gui: &mut Gui, action: &str) {
    if let Some(command) = action.strip_prefix("command:") {
        run_command(gui, command);
        return;
    }
    if let Some(id) = action.strip_prefix("toggle:") {
        if gui.panels.layout().shown().iter().any(|shown| shown == id) {
            if let Err(error) = gui.panels.hide(id) {
                warn_failure("a panel could not be hidden", error.message());
            }
            gui.message = tr_args("ui.status.hidden", &[("id", id)]);
        } else if gui.panels.show(id).is_ok() {
            gui.message = tr_args("ui.status.shown", &[("id", id)]);
        } else {
            gui.message = tr_args("ui.status.no-panel", &[("id", id)]);
        }
        gui.sync_panels();
        gui.sync_editors();
        return;
    }
    match action {
        "palette:open" => open_palette(gui),
        "palette:close" => {
            close_palette(gui);
            gui.message = tr("ui.status.palette-closed");
        }
        "layout:save" => match &gui.layout_path {
            Some(path) => {
                let layout = Layout::captured("workbench", &gui.groups, &gui.panels, false, false);
                match std::fs::write(path, layout.to_json()) {
                    Ok(()) => {
                        gui.message = tr_args(
                            "ui.status.layout-saved",
                            &[("path", &path.display().to_string())],
                        );
                    }
                    Err(error) => {
                        gui.message = tr_args(
                            "ui.status.layout-not-saved",
                            &[("reason", &error.to_string())],
                        );
                    }
                }
            }
            None => gui.message = tr("ui.status.no-layout-path"),
        },
        "layout:reset" => {
            if let Err(error) = gui.default_layout.apply(&mut gui.groups, &mut gui.panels) {
                warn_failure("the default layout could not be applied", error.message());
            }
            gui.message = tr("ui.status.layout-reset");
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
            gui.message = tr("ui.status.editor-split");
            gui.sync_editors();
        }
        "group:next" => {
            gui.groups.focus_next();
            gui.message = tr_args(
                "ui.status.focus-group",
                &[("group", &gui.groups.focus().get().to_string())],
            );
        }
        "quit" => {
            gui.message = tr("ui.status.closing");
            rust_widgets::quit();
        }
        other => gui.message = tr_args("ui.status.unknown-action", &[("action", other)]),
    }
    rust_widgets::set_widget_text(
        gui.status,
        &format!("{} | {}", gui.status_line(), gui.message),
    );
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

    // The services the built-in panels need are the ones this slice actually has: a property
    // schema and diagnostics come from models that exist. `document-tree` does not exist yet, so
    // the Outline is registered but hidden instead of being shown without a backend (I-11). A
    // panel whose service is missing stays registered but is not shown (T04.02's gate), and the
    // log names it.
    let services: BTreeSet<String> = ["property-schema", "diagnostics"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    let mut panels = built_in_panels();
    for (id, _title, dock) in EXTRA_PANELS {
        if let Err(error) = panels.register(PanelSpec::new(id, tr(panel_key(id)), dock, 300)) {
            warn_failure("an extra panel could not be registered", error.message());
        }
    }
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
        show_panel_if_available(&mut panels, id, &services);
    }
    // §2.2's ledger: every required panel is accounted for, and the ones this slice does not have
    // are recorded as missing rather than drawn as empty panels.
    if let Some(summary) = panels.accounting_lines(&services).first() {
        crate::host::log::emit(
            crate::host::log::Level::Info,
            "app::run::workbench-gui",
            summary,
        );
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
        if let Err(reason) = groups.type_into(text) {
            warn_failure("the demo document text could not be set", reason);
        }
    }
    let focus = groups.focus();
    if let Some(tabs) = groups.tabs_mut(focus)
        && let Err(reason) = tabs.activate(0)
    {
        warn_failure("the first tab could not be activated", reason);
    }
    let default_layout = Layout::captured("default", &groups, &panels, false, false);
    if let Some(path) = &layout_path
        && let Ok(text) = std::fs::read_to_string(path)
        && let Ok(layout) = Layout::from_json(&text)
        && let Err(error) = layout.apply(&mut groups, &mut panels)
    {
        warn_failure("the saved layout could not be applied", error.message());
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
        if let Err(reason) = scene.select_only(initial) {
            warn_failure("the initial scene selection was refused", reason);
        }
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
        message: tr("ui.status.ready"),
        window,
        placements: Vec::new(),
        palette: Palette::new(),
        palette_surface: 0,
        registry: built_in(),
        recents: Recents::new(8),
        property,
        property_grid: 0,
        property_row: 2,
        inspect_extra: Vec::new(),
        property_error: 0,
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
        data_grid: 0,
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
        canvas_snapshot: Arc::new(Mutex::new(Arc::new(CanvasSnapshot {
            nodes: Arc::from(Vec::new()),
            marquee: None,
        }))),
        canvas_scheduled: Arc::new(AtomicBool::new(false)),
        canvas_revision: u64::MAX,
        outline_rows: Vec::new(),
        outline_index: BTreeMap::new(),
        outline_content_revision: u64::MAX,
        translatable: Vec::new(),
        toolbar: 0,
        toolbar_items: Vec::new(),
        context_menu: 0,
        scene_menu: 0,
    }));

    // The interface catalogue is runtime data under `language/`; load it into the library's manager
    // and watch the directory so an edit appears without restarting (blue21 L-4 items 3/5).
    if let Err(reason) = crate::app::locale::catalog::install_global() {
        warn_failure("the interface language could not be loaded", reason);
    }
    let language_dir = crate::app::locale::vocabulary::language_dir();
    match rust_widgets::i18n::enable_global_hot_reload(&language_dir) {
        Ok(loaded) => crate::host::log::emit(
            crate::host::log::Level::Info,
            "app::run::workbench-i18n",
            &format!(
                "watching {} for language packs ({loaded} loaded)",
                language_dir.display()
            ),
        ),
        Err(reason) => crate::host::log::emit(
            crate::host::log::Level::Warn,
            "app::run::workbench-i18n",
            &format!("language hot reload is unavailable: {reason}"),
        ),
    }
    schedule_i18n(Arc::clone(&gui));

    // Menu bar. The toolbar is the portable path (macOS paints no native menus in this revision);
    // the menu bar is still created because on the other hosts it is the native one.
    let bar = handle.new_menu_bar(0, 0, WIDTH, MENU_HEIGHT);
    if !handle.attach_menu_bar(&bar) {
        warn_failure(
            "the menu bar could not be attached",
            "the window refused it; the menu actions are still reachable from the toolbar",
        );
    }
    if let Ok(mut gui) = gui.lock() {
        gui.placements.push((bar.raw_id(), Area::Menu));
    }
    let mut menu_actions: BTreeMap<u64, String> = BTreeMap::new();
    // The menu tree is the registry's, not a second hand-spelled copy: every leaf names the command
    // it binds, and the label is the identified vocabulary key (`menu.<id>.item.<command>`). This is
    // what blue21 L-2/T05.02 asked for — the same menu the catalog covers is the one the window paints.
    for menu in crate::app::commands::menu::built_in() {
        let title_key = format!("menu.{}.title", menu.id());
        let handle_menu = handle.new_menu(&bar, &tr(&title_key), 0, 0, 0, 0);
        if let Ok(mut gui) = gui.lock() {
            gui.translatable.push(Translatable {
                id: handle_menu.raw_id(),
                key: title_key,
                kind: LabelKind::MenuTitle,
            });
        }
        for item in menu.items() {
            match item {
                crate::app::commands::menu::Item::Command { title, command }
                | crate::app::commands::menu::Item::Planned { title, command, .. } => {
                    add_menu_leaf(
                        &handle,
                        &handle_menu,
                        &format!("menu.{}.item.{command}", menu.id()),
                        command,
                        title,
                        &mut menu_actions,
                        &gui,
                    );
                }
                crate::app::commands::menu::Item::Submenu { id, items, .. } => {
                    // The window's API has no nested menu, so a submenu's leaves are flattened under
                    // its parent but keep the submenu-qualified key.
                    for child in items {
                        match child {
                            crate::app::commands::menu::Item::Command { title, command }
                            | crate::app::commands::menu::Item::Planned {
                                title, command, ..
                            } => {
                                add_menu_leaf(
                                    &handle,
                                    &handle_menu,
                                    &format!("menu.{}.submenu.{id}.item.{command}", menu.id()),
                                    command,
                                    title,
                                    &mut menu_actions,
                                    &gui,
                                );
                            }
                            _ => {}
                        }
                    }
                }
                crate::app::commands::menu::Item::Separator => {}
            }
        }
        // The window's own leaves live in the registry menu that names their subject: Quit in File,
        // panel toggles in View. They are workbench actions, not registry commands, so their action
        // strings stay in this window's vocabulary.
        match menu.id() {
            "file" => {
                let item = handle.new_menu_item(&handle_menu, &tr("ui.menu.quit"), None);
                menu_actions.insert(item.raw_id(), "quit".to_owned());
                if let Ok(mut gui) = gui.lock() {
                    gui.translatable.push(Translatable {
                        id: item.raw_id(),
                        key: "ui.menu.quit".to_owned(),
                        kind: LabelKind::Text,
                    });
                }
            }
            "view" => {
                // Outline and Inspector are already leaves of the registry's View menu
                // (`panel.outline`/`panel.inspector`, which `run_command` maps to the toggles); the
                // window-only panels are appended here so View stays one menu, not two.
                for (key, action) in [
                    ("ui.panel.pages", "toggle:pages"),
                    ("ui.panel.messages", "toggle:messages"),
                    ("ui.panel.problems", "toggle:problems"),
                    ("ui.panel.events", "toggle:events"),
                    ("ui.panel.data", "toggle:data"),
                    ("ui.panel.debug", "toggle:debug"),
                ] {
                    let item = handle.new_menu_item(&handle_menu, &tr(key), None);
                    menu_actions.insert(item.raw_id(), action.to_owned());
                    if let Ok(mut gui) = gui.lock() {
                        gui.translatable.push(Translatable {
                            id: item.raw_id(),
                            key: key.to_owned(),
                            kind: LabelKind::Text,
                        });
                    }
                }
            }
            _ => {}
        }
    }
    // The window's own groups are not registry commands: layout persistence and editor splitting
    // are workbench actions, so they stay spelled here.
    for (menu_key, entries) in [
        (
            "ui.menu.layout",
            vec![
                ("ui.menu.save-layout", "layout:save"),
                ("ui.menu.reset-layout", "layout:reset"),
            ],
        ),
        (
            "ui.menu.editor",
            vec![
                ("ui.menu.split-right", "group:split"),
                ("ui.menu.next-group", "group:next"),
            ],
        ),
    ] {
        let menu = handle.new_menu(&bar, &tr(menu_key), 0, 0, 0, 0);
        if let Ok(mut gui) = gui.lock() {
            gui.translatable.push(Translatable {
                id: menu.raw_id(),
                key: menu_key.to_owned(),
                kind: LabelKind::MenuTitle,
            });
        }
        for (key, action) in entries {
            let item = handle.new_menu_item(&menu, &tr(key), None);
            menu_actions.insert(item.raw_id(), action.to_owned());
            if let Ok(mut gui) = gui.lock() {
                gui.translatable.push(Translatable {
                    id: item.raw_id(),
                    key: key.to_owned(),
                    kind: LabelKind::Text,
                });
            }
        }
    }

    // Toolbar. The strip is the library's `ToolBar`, and each action is one of its items; the
    // model still decides what an action does, so a click and a menu entry share `apply_action`.
    let toolbar_items: Vec<(&'static str, &'static str)> = vec![
        ("ui.toolbar.commands", "palette:open"),
        ("ui.panel.outline", "toggle:outline"),
        ("ui.panel.inspector", "toggle:inspector"),
        ("ui.panel.messages", "toggle:messages"),
        ("ui.panel.problems", "toggle:problems"),
        ("ui.panel.events", "toggle:events"),
        ("ui.panel.data", "toggle:data"),
        ("ui.toolbar.split", "group:split"),
        ("ui.toolbar.next-group", "group:next"),
        ("ui.toolbar.save-layout", "layout:save"),
        ("ui.toolbar.reset-layout", "layout:reset"),
    ];
    let tool_bar = handle.new_tool_bar(8, TOOLBAR_Y, WIDTH - 16, TOOLBAR_HEIGHT);
    let inserted = with_widget_mut(tool_bar.raw_id(), |widget| {
        let Some(bar) = (widget as &mut dyn Any).downcast_mut::<ToolBar>() else {
            return false;
        };
        for (index, (key, action)) in toolbar_items.iter().enumerate() {
            bar.insert_item(
                index,
                rust_widgets::widget::menu_toolbar::tool_bar::ToolBarItem::new(*action, tr(key)),
            );
        }
        true
    })
    .unwrap_or(false);
    if !inserted {
        warn_failure(
            "the tool bar could not be filled",
            "the mounted control was not a ToolBar",
        );
    }
    {
        let gui = Arc::clone(&gui);
        subscribe_string(tool_bar.raw_id(), "action_triggered", move |action| {
            if let Ok(mut gui) = gui.lock() {
                apply_action(&mut gui, action);
            }
        });
    }
    if let Ok(mut gui) = gui.lock() {
        gui.toolbar = tool_bar.raw_id();
        gui.toolbar_items = toolbar_items.clone();
        gui.placements
            .push((tool_bar.raw_id(), Area::Toolbar { index: 0, count: 1 }));
    }

    // The Inspector's context menu (blue21 §3.3). The popup is the library's own `Menu`; the
    // PropertyGrid emits `context_menu_requested`, and this window opens the menu at that point.
    // Labels are the catalog's, and the action map is keyed by the same translated label, so a
    // language reload cannot make a right-click run the wrong command.
    let context_menu = handle.new_menu(&bar, "", 0, 0, 0, 0);
    let mut context_actions: BTreeMap<String, String> = BTreeMap::new();
    for (key, action) in [
        ("ui.context.clear", "clear"),
        ("ui.context.reset", "reset"),
        ("ui.context.inspect", "inspect"),
    ] {
        let label = tr(key);
        handle.new_menu_item(&context_menu, &label, None);
        context_actions.insert(label, action.to_owned());
    }
    {
        let gui = Arc::clone(&gui);
        let actions = context_actions.clone();
        context_menu.on_triggered(move |label| {
            if let Ok(mut gui) = gui.lock() {
                let Some(action) = actions.get(&label).cloned() else {
                    return;
                };
                match action.as_str() {
                    "clear" => clear_property(&mut gui),
                    "reset" => reset_property(&mut gui),
                    "inspect" => inspect_values(&mut gui),
                    _ => {}
                }
            }
        });
    }
    if let Ok(mut gui) = gui.lock() {
        gui.context_menu = context_menu.raw_id();
    }

    // The scene's context menu (blue21 §3.3, object-tree/canvas node). The canvas publishes
    // `context_menu_requested`; the menu's actions mutate the scene the same way the Outline and
    // the canvas clicks do. The actions are the ones this slice can carry honestly: selecting the
    // parent, design lock, design hide and raising. Extract-component, events and source are not
    // wired because their models are not on this surface yet, and a menu entry that does nothing
    // would be worse than its absence.
    let scene_menu = handle.new_menu(&bar, "", 0, 0, 0, 0);
    let mut scene_actions: BTreeMap<String, String> = BTreeMap::new();
    for (key, action) in [
        ("ui.context.select-parent", "select-parent"),
        ("ui.context.toggle-lock", "toggle-lock"),
        ("ui.context.toggle-hide", "toggle-hide"),
        ("ui.context.raise", "raise"),
    ] {
        let label = tr(key);
        handle.new_menu_item(&scene_menu, &label, None);
        scene_actions.insert(label, action.to_owned());
    }
    {
        let gui = Arc::clone(&gui);
        let actions = scene_actions.clone();
        scene_menu.on_triggered(move |label| {
            if let Ok(mut gui) = gui.lock() {
                let Some(action) = actions.get(&label).cloned() else {
                    return;
                };
                scene_context_action(&mut gui, &action);
            }
        });
    }
    if let Ok(mut gui) = gui.lock() {
        gui.scene_menu = scene_menu.raw_id();
    }

    // Docked panels. The model owns which panels exist and whether they are shown; the responsive
    // layout owns where they sit, and each panel's own `BoxLayout` owns its title/list split. A
    // panel therefore keeps its internal proportions when the window scales.
    let outline_items = {
        let gui = gui.lock().expect("the workbench state is not poisoned");
        gui.scene.rows()
    };
    let mut inspector_controls: Option<InspectorControls> = None;
    let mut events_controls: Option<EventControls> = None;
    let mut data_controls: Option<DataControls> = None;
    let mut pages_controls: Option<PageControls> = None;
    let mut outline_list: Option<u64> = None;
    for (id, _title, rect, area) in [
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
        let title_key = panel_key(id);
        panel.set_title(&tr(title_key));
        if let Ok(mut gui) = gui.lock() {
            gui.translatable.push(Translatable {
                id: panel.raw_id(),
                key: title_key.to_owned(),
                kind: LabelKind::PanelTitle,
            });
        }
        // The panel is the library's `GroupBox`: `set_title` draws the title band itself, so the
        // list starts below that band. A second same-named label used to duplicate the title
        // (blue21 W-2e).
        // Child factories parent the controls to the panel, so the panel's layout, clipping and
        // teardown own them; asking the window for them (2.8.10 and earlier) left the parent link
        // as an implicit contract (feedback-20261020 §3.1, closed by 2.8.11).
        // The Inspector and Data panels edit through the library's `PropertyGrid` rather than a
        // list plus a hand-built field row (blue21 W-2d), so they create no list at all.
        let list_id = if id == "inspector" || id == "data" {
            0
        } else {
            let list = panel.new_list_box(rect.0 + 8, rect.1 + 30, rect.2 - 16, rect.3 - 38);
            let focus_name = match id {
                "pages" => "pages list",
                "outline" => "outline list",
                "events" => "events list",
                "messages" => "messages list",
                "problems" => "problems list",
                _ => "panel list",
            };
            watch_focus(list.raw_id(), focus_name);
            list.raw_id()
        };
        let mut inner = BoxLayout::new(Orientation::Vertical, 4, 8);
        if list_id != 0 {
            inner.add_widget(list_id, 1);
            inner.set_size_policy(list_id, SizePolicy::Expanding);
        }
        let mut extra_widgets: Vec<u64> = Vec::new();
        if id == "pages" {
            let field =
                panel.new_line_edit("", rect.0 + 8, rect.1 + rect.3 as i32 - 96, rect.2 - 16, 26);
            field.set_placeholder(&tr("ui.placeholder.page-name"));
            watch_focus(field.raw_id(), "pages field");
            let actions = panel.new_panel(rect.0 + 8, rect.1 + rect.3 as i32 - 66, rect.2 - 16, 26);
            let add = actions.new_button(&tr("ui.button.add"), 0, 0, 44, 26);
            let open = actions.new_button(&tr("ui.button.open"), 0, 0, 44, 26);
            let back = actions.new_button(&tr("ui.button.back"), 0, 0, 44, 26);
            let forward = actions.new_button(&tr("ui.button.forward"), 0, 0, 44, 26);
            let close = actions.new_button(&tr("ui.button.close"), 0, 0, 44, 26);
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
                list_id,
                field.raw_id(),
                add.raw_id(),
                open.raw_id(),
                back.raw_id(),
                forward.raw_id(),
                close.raw_id(),
                error.raw_id(),
                note.raw_id(),
            ));
            if let Ok(mut gui) = gui.lock() {
                for (id, key) in [
                    (add.raw_id(), "ui.button.add"),
                    (open.raw_id(), "ui.button.open"),
                    (back.raw_id(), "ui.button.back"),
                    (forward.raw_id(), "ui.button.forward"),
                    (close.raw_id(), "ui.button.close"),
                ] {
                    gui.translatable.push(Translatable {
                        id,
                        key: key.to_owned(),
                        kind: LabelKind::Text,
                    });
                }
            }
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
            // The property rows are the library's `PropertyGrid` (blue21 W-2d): the widget paints the
            // name/value columns, owns row selection and emits `cell_changed`, while the contract
            // schema still validates whatever text it commits. The panel's own layout owns the
            // grid's geometry, like every other child control.
            let grid = handle
                .mount_widget_by_name(
                    "property_grid",
                    Rect::new(rect.0 + 8, rect.1 + 30, rect.2 - 16, 104),
                    "",
                )
                .expect("the library registers the property grid widget");
            watch_focus(grid.raw_id(), "inspector property grid");
            let theme_y = rect.1 + rect.3 as i32 - 80;
            let actions_y = rect.1 + rect.3 as i32 - 116;
            let actions = panel.new_panel(rect.0 + 8, actions_y, rect.2 - 16, 26);
            let clear = actions.new_button(&tr("ui.button.clear"), 0, 0, 72, 26);
            let reset = actions.new_button(&tr("ui.button.reset"), 0, 0, 72, 26);
            let inspect = actions.new_button(&tr("ui.button.values"), 0, 0, 72, 26);
            let mut action_layout = BoxLayout::new(Orientation::Horizontal, 4, 0);
            for widget in [clear.raw_id(), reset.raw_id(), inspect.raw_id()] {
                action_layout.add_widget(widget, 1);
                action_layout.set_size_policy(widget, SizePolicy::Expanding);
            }
            actions.set_layout(Box::new(action_layout));
            let theme_row = panel.new_panel(rect.0 + 8, theme_y, rect.2 - 16, 26);
            let theme_field = theme_row.new_line_edit("", rect.0 + 8, theme_y, rect.2 - 96, 26);
            theme_field.set_placeholder(&tr("ui.placeholder.theme-name"));
            watch_focus(theme_field.raw_id(), "inspector theme field");
            let theme_button = theme_row.new_button(
                &tr("ui.button.theme"),
                rect.0 + rect.2 as i32 - 80,
                theme_y,
                72,
                26,
            );
            let error =
                panel.new_label("", rect.0 + 8, rect.1 + rect.3 as i32 - 50, rect.2 - 16, 16);
            let note =
                panel.new_label("", rect.0 + 8, rect.1 + rect.3 as i32 - 30, rect.2 - 16, 16);
            let mut theme_layout = BoxLayout::new(Orientation::Horizontal, 4, 0);
            theme_layout.add_widget(theme_field.raw_id(), 1);
            theme_layout.set_size_policy(theme_field.raw_id(), SizePolicy::Expanding);
            theme_layout.add_widget(theme_button.raw_id(), 0);
            theme_layout
                .set_constraints(theme_button.raw_id(), LayoutConstraints::new(72, Some(72)));
            theme_layout.set_size_policy(theme_button.raw_id(), SizePolicy::Fixed);
            theme_row.set_layout(Box::new(theme_layout));
            for (widget, height) in [
                (grid.raw_id(), 104),
                (actions.raw_id(), 26),
                (theme_row.raw_id(), 26),
                (error.raw_id(), 16),
                (note.raw_id(), 16),
            ] {
                inner.add_widget(widget, 0);
                inner.set_constraints(widget, LayoutConstraints::new(height, Some(height)));
                inner.set_size_policy(widget, SizePolicy::Fixed);
            }
            inspector_controls = Some((
                grid.raw_id(),
                clear.raw_id(),
                reset.raw_id(),
                inspect.raw_id(),
                theme_field.raw_id(),
                theme_button.raw_id(),
                error.raw_id(),
                note.raw_id(),
            ));
            if let Ok(mut gui) = gui.lock() {
                for (id, key) in [
                    (clear.raw_id(), "ui.button.clear"),
                    (reset.raw_id(), "ui.button.reset"),
                    (inspect.raw_id(), "ui.button.values"),
                    (theme_button.raw_id(), "ui.button.theme"),
                ] {
                    gui.translatable.push(Translatable {
                        id,
                        key: key.to_owned(),
                        kind: LabelKind::Text,
                    });
                }
            }
            extra_widgets.extend([
                grid.raw_id(),
                actions.raw_id(),
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
            field.set_placeholder(&tr("ui.placeholder.handler-name"));
            watch_focus(field.raw_id(), "events handler field");
            field.on_value_changed(|text| {
                crate::host::log::emit(
                    crate::host::log::Level::Info,
                    "app::run::workbench-focus",
                    &format!("events field text {} character(s)", text.chars().count()),
                );
            });
            let actions = panel.new_panel(rect.0 + 8, rect.1 + rect.3 as i32 - 66, rect.2 - 16, 26);
            let bind = actions.new_button(&tr("ui.button.bind"), 0, 0, 72, 26);
            let unbind = actions.new_button(&tr("ui.button.unbind"), 0, 0, 72, 26);
            let fire = actions.new_button(&tr("ui.button.fire"), 0, 0, 72, 26);
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
                list_id,
                field.raw_id(),
                bind.raw_id(),
                unbind.raw_id(),
                fire.raw_id(),
                error.raw_id(),
                note.raw_id(),
            ));
            if let Ok(mut gui) = gui.lock() {
                for (id, key) in [
                    (bind.raw_id(), "ui.button.bind"),
                    (unbind.raw_id(), "ui.button.unbind"),
                    (fire.raw_id(), "ui.button.fire"),
                ] {
                    gui.translatable.push(Translatable {
                        id,
                        key: key.to_owned(),
                        kind: LabelKind::Text,
                    });
                }
            }
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
            // The sample schema's fields are the library's `PropertyGrid` as well (blue21 W-2d):
            // one editable row per field, with the dataset row as read-only context.
            let grid = handle
                .mount_widget_by_name(
                    "property_grid",
                    Rect::new(rect.0 + 8, rect.1 + 30, rect.2 - 16, 176),
                    "",
                )
                .expect("the library registers the property grid widget");
            watch_focus(grid.raw_id(), "data property grid");
            let error =
                panel.new_label("", rect.0 + 8, rect.1 + rect.3 as i32 - 44, rect.2 - 16, 16);
            let note =
                panel.new_label("", rect.0 + 8, rect.1 + rect.3 as i32 - 24, rect.2 - 16, 16);
            for (widget, height) in [
                (grid.raw_id(), 176),
                (error.raw_id(), 16),
                (note.raw_id(), 16),
            ] {
                inner.add_widget(widget, 0);
                inner.set_constraints(widget, LayoutConstraints::new(height, Some(height)));
                inner.set_size_policy(widget, SizePolicy::Fixed);
            }
            data_controls = Some((grid.raw_id(), error.raw_id(), note.raw_id()));
            extra_widgets.extend([grid.raw_id(), error.raw_id(), note.raw_id()]);
        }
        panel.set_layout(Box::new(inner));
        let items: Vec<String> = match id {
            "outline" => outline_items.clone(),
            "messages" => vec![tr("ui.status.ready")],
            // The Events, Data and Pages lists are filled from their models once the controls exist;
            // the Inspector and Data panels own a property grid instead of a list.
            "events" | "data" | "pages" | "inspector" => Vec::new(),
            _ => vec![tr("ui.status.no-problems")],
        };
        for item in items {
            if list_id != 0 {
                let _ = rust_widgets::list_box_add_item(list_id, &item);
            }
        }
        if id == "outline" {
            outline_list = Some(list_id);
        }
        if let Ok(mut gui) = gui.lock() {
            gui.placements.push((panel.raw_id(), area));
            let entry = gui.panel_widgets.entry(id.to_owned()).or_default();
            entry.push(panel.raw_id());
            entry.push(list_id);
            entry.extend(extra_widgets);
        }
    }

    // The Inspector's editor: the property grid commits a cell through `cell_changed`, and the
    // schema editor validates the text before any node changes.
    if let Some((
        grid_id,
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
            gui.property_grid = grid_id;
            gui.property_error = error_id;
            gui.inspect_note = note_id;
        }
        {
            let gui = Arc::clone(&gui);
            subscribe_cell(grid_id, "cell_changed", move |row, text| {
                if let Ok(mut gui) = gui.lock()
                    && row as usize == gui.property_row
                {
                    apply_property(&mut gui, text);
                }
            });
        }
        {
            let gui = Arc::clone(&gui);
            subscribe_string(grid_id, "context_menu_requested", move |payload| {
                let Some((x, y)) = payload
                    .split_once(',')
                    .and_then(|(x, y)| Some((x.trim().parse().ok()?, y.trim().parse().ok()?)))
                else {
                    return;
                };
                if let Ok(gui) = gui.lock()
                    && gui.context_menu != 0
                {
                    let opened = MenuHandle::from_raw(gui.context_menu).open_at(
                        x,
                        y,
                        Rect::new(0, 0, WIDTH, HEIGHT),
                    );
                    if !opened {
                        warn_failure("the Inspector context menu could not open", payload);
                    }
                }
            });
        }
        {
            let gui = Arc::clone(&gui);
            ButtonHandle::from_raw(clear_id).on_click(move || {
                if let Ok(mut gui) = gui.lock() {
                    clear_property(&mut gui);
                }
            });
        }
        {
            let gui = Arc::clone(&gui);
            ButtonHandle::from_raw(reset_id).on_click(move || {
                if let Ok(mut gui) = gui.lock() {
                    reset_property(&mut gui);
                }
            });
        }
        {
            let gui = Arc::clone(&gui);
            ButtonHandle::from_raw(inspect_id).on_click(move || {
                if let Ok(mut gui) = gui.lock() {
                    inspect_values(&mut gui);
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
                            gui.message =
                                tr_args("ui.status.theme-applied", &[("message", &lines[0])]);
                            refresh_inspector(&mut gui);
                            crate::host::log::emit(
                                crate::host::log::Level::Info,
                                "app::run::workbench-inspect",
                                &format!("theme applied: {}", lines.join("; ")),
                            );
                        }
                        Err(reason) => {
                            let reason = gui.inspect.refuse(reason);
                            let text = tr_args("ui.error.theme-refused", &[("reason", &reason)]);
                            rust_widgets::set_widget_text(gui.property_error, &text);
                            gui.message = text;
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
                                tr_args(
                                    "ui.status.navigation-declared",
                                    &[
                                        ("from", &format!("{from:?}")),
                                        ("to", &format!("{to:?}")),
                                        ("revision", &revision.to_string()),
                                    ],
                                )
                            })
                        }
                        None => gui.pages.add_page(&name).map(|revision| {
                            tr_args(
                                "ui.status.page-declared",
                                &[
                                    ("name", &format!("{name:?}")),
                                    ("revision", &revision.to_string()),
                                ],
                            )
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
                            let text = tr_args("ui.error.not-added", &[("reason", &reason)]);
                            rust_widgets::set_widget_text(gui.pages_error, &text);
                            gui.message = text;
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
                            let text = tr_args("ui.error.not-opened", &[("reason", &reason)]);
                            rust_widgets::set_widget_text(gui.pages_error, &text);
                            gui.message = text;
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
                            gui.message =
                                tr_args("ui.status.now-on", &[("page", &format!("{page:?}"))]);
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
                            let text = tr_args("ui.error.not-moved", &[("reason", &reason)]);
                            rust_widgets::set_widget_text(gui.pages_error, &text);
                            gui.message = text;
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
                            let text = tr_args("ui.error.not-closed", &[("reason", &reason)]);
                            rust_widgets::set_widget_text(gui.pages_error, &text);
                            gui.message = text;
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
                            gui.message = tr_args(
                                "ui.status.bound",
                                &[
                                    ("event", &event),
                                    ("handler", &handler),
                                    ("node", &node.to_string()),
                                    ("revision", &revision.to_string()),
                                ],
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
                            let text = tr_args("ui.error.not-bound", &[("reason", &reason)]);
                            rust_widgets::set_widget_text(gui.events_error, &text);
                            gui.message = text;
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
                            gui.message = tr_args(
                                "ui.status.unbound",
                                &[
                                    ("event", &event),
                                    ("node", &gui.events.node().get().to_string()),
                                    ("revision", &revision.to_string()),
                                ],
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
                            let text = tr_args("ui.error.not-unbound", &[("reason", &reason)]);
                            rust_widgets::set_widget_text(gui.events_error, &text);
                            gui.message = text;
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
                            let text = tr_args("ui.error.not-fired", &[("reason", &reason)]);
                            rust_widgets::set_widget_text(gui.events_error, &text);
                            gui.message = text;
                        }
                    }
                    sync_events(&mut gui);
                    update_status(&gui);
                }
            });
        }
    }

    // The Data panel (T13.04/T14.05). The sample dataset is edited through the typed schema;
    // the running layer stays unavailable because no instance is started in a design session. The
    // grid's cell commit is the Apply action: row 0 is the dataset label, so field rows start at 1.
    if let Some((grid_id, error_id, note_id)) = data_controls {
        if let Ok(mut gui) = gui.lock() {
            gui.data_grid = grid_id;
            gui.data_error = error_id;
            gui.data_note = note_id;
            sync_data(&mut gui);
        }
        {
            let gui = Arc::clone(&gui);
            subscribe_cell(grid_id, "cell_changed", move |row, text| {
                if let Ok(mut gui) = gui.lock() {
                    // Row 0 is the dataset label; field rows start at 1.
                    if row == 0 {
                        return;
                    }
                    let Some(field) = gui
                        .inspect
                        .fields()
                        .get((row as usize).saturating_sub(1))
                        .cloned()
                    else {
                        return;
                    };
                    gui.data_selected = (row as usize).saturating_sub(1);
                    match gui.inspect.edit(&field, text) {
                        Ok(version) => {
                            rust_widgets::set_widget_text(gui.data_error, "");
                            gui.message = tr_args(
                                "ui.status.data-applied",
                                &[
                                    ("field", &field),
                                    ("value", &format!("{text:?}")),
                                    ("version", &version.to_string()),
                                ],
                            );
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
                            let message = tr_args("ui.error.not-applied", &[("reason", &reason)]);
                            rust_widgets::set_widget_text(gui.data_error, &message);
                            gui.message = message;
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
    let debug_title =
        debug_panel.new_label(&tr(panel_key("debug")), WIDTH as i32 - 312, 466, 304, 20);
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
    let debug_start = debug_actions.new_button(&tr("ui.button.start"), 0, 0, 73, 28);
    let debug_continue = debug_actions.new_button(&tr("ui.button.run"), 0, 0, 73, 28);
    let debug_step = debug_actions.new_button(&tr("ui.button.step"), 0, 0, 73, 28);
    let debug_stop = debug_actions.new_button(&tr("ui.button.stop"), 0, 0, 73, 28);
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
        gui.translatable.push(Translatable {
            id: debug_title.raw_id(),
            key: "ui.panel.debug".to_owned(),
            kind: LabelKind::Text,
        });
        for (id, key) in [
            (debug_start.raw_id(), "ui.button.start"),
            (debug_continue.raw_id(), "ui.button.run"),
            (debug_step.raw_id(), "ui.button.step"),
            (debug_stop.raw_id(), "ui.button.stop"),
        ] {
            gui.translatable.push(Translatable {
                id,
                key: key.to_owned(),
                kind: LabelKind::Text,
            });
        }
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

    // Two editor groups: the second stays hidden until "Split" is used. The strip is the
    // library's `TabWidget`; the model still owns the documents and the active index, and the
    // control's `current_changed` writes that choice back into the model.
    for group in 0..2 {
        let (tx, _ex, ew) = if group == 0 {
            (270, 270, 680)
        } else {
            (610, 610, 340)
        };
        let documents: Vec<String> = gui
            .lock()
            .ok()
            .and_then(|gui| {
                let id = *gui.groups.group_ids().get(group)?;
                let tabs = gui.groups.tabs(id)?;
                Some(
                    tabs.tabs()
                        .iter()
                        .map(|tab| tab.document().to_owned())
                        .collect(),
                )
            })
            .unwrap_or_default();
        let tab_strip = handle.new_tab_widget(tx, CONTENT_Y, ew, 26);
        for document in &documents {
            let label = document.rsplit('/').next().unwrap_or(document);
            if tab_strip.add_tab(label, None).is_none() {
                warn_failure("a tab could not be added to the strip", label);
            }
        }
        {
            let gui = Arc::clone(&gui);
            subscribe_index(tab_strip.raw_id(), "current_changed", move |index| {
                if let Ok(mut gui) = gui.lock() {
                    let Some(id) = gui.groups.group_ids().get(group).copied() else {
                        return;
                    };
                    let activated = gui
                        .groups
                        .tabs_mut(id)
                        .map(|tabs| tabs.activate(index as usize));
                    match activated {
                        Some(Ok(())) => {}
                        Some(Err(reason)) => warn_failure("a tab could not be activated", reason),
                        None => warn_failure("a tab strip has no group", group),
                    }
                    gui.message = format!("tab {index} in group {group}");
                    gui.sync_editors();
                }
            });
        }
        watch_focus(tab_strip.raw_id(), "editor tabs");
        let mut all = vec![tab_strip.raw_id()];
        if let Ok(mut gui) = gui.lock() {
            gui.placements.push((
                tab_strip.raw_id(),
                Area::TabStrip {
                    group: group as u32,
                },
            ));
        }
        // The editor area is the library's own code editor, not a list of lines. The surface is
        // mounted by its registered name; `sync_editors` reaches it through the widget runtime and
        // downcasts to `CodeEditor` (the widget trait is `Any`), which is the only public path from
        // a mounted surface back to its typed API.
        let editor = handle
            .mount_widget_by_name("code_editor", Rect::new(tx, CONTENT_Y + 30, ew, 535), "")
            .expect("the library registers the code editor widget");
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
                tabs: tab_strip.raw_id(),
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
                                sync_canvas(&mut gui);
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
    {
        let gui = Arc::clone(&gui);
        subscribe_string(canvas_surface, "context_menu_requested", move |payload| {
            let Some((x, y)) = payload.split_once(',').and_then(|(x, y)| {
                Some((x.trim().parse::<i32>().ok()?, y.trim().parse::<i32>().ok()?))
            }) else {
                return;
            };
            if let Ok(mut gui) = gui.lock() {
                let Some(design) = canvas_point(gui.canvas_panel, x, y) else {
                    return;
                };
                let Some(node) = gui.scene.hit(design.0, design.1) else {
                    // A right-click on empty canvas has no node menu; the selection is left as it
                    // was rather than cleared as a side effect of opening nothing.
                    return;
                };
                if gui.scene.primary() != Some(node) {
                    select_node(&mut gui, node, "context");
                }
                if gui.scene_menu != 0
                    && !MenuHandle::from_raw(gui.scene_menu).open_at(
                        x,
                        y,
                        Rect::new(0, 0, WIDTH, HEIGHT),
                    )
                {
                    warn_failure("the scene context menu could not open", payload);
                }
            }
        });
    }

    // Command palette (T05.04/T05.05). It is the library's `CommandPalette`, mounted after the
    // docked panels so the painter's creation order keeps it above them; its geometry comes from
    // the same responsive layout as the rest of the chrome, not from a second placement. Filtering,
    // highlighting and activation belong to the control; the host only feeds it entries and runs
    // the activated command id through the registry model.
    match handle.mount_widget_by_name("command_palette", Rect::new(0, 0, 640, 320), "") {
        Ok(palette) => {
            let id = palette.raw_id();
            rust_widgets::set_widget_visible(id, false);
            watch_focus(id, "command palette");
            {
                let gui = Arc::clone(&gui);
                subscribe_string(id, "command_activated", move |command| {
                    if let Ok(mut gui) = gui.lock()
                        && gui.palette.is_open()
                    {
                        run_palette_id(&mut gui, command);
                    }
                });
            }
            {
                let gui = Arc::clone(&gui);
                subscribe_string(id, "query_changed", move |text| {
                    if let Ok(gui) = gui.lock()
                        && gui.palette.is_open()
                    {
                        crate::host::log::emit(
                            crate::host::log::Level::Info,
                            "app::run::workbench-palette",
                            &format!("palette query {text:?}"),
                        );
                    }
                });
            }
            {
                // Upstream 3.2.0 publishes `dismissed`: Escape now closes the palette again, the way
                // the hand-built field's `editing_finished` did (feedback-20261030 F-12).
                let gui = Arc::clone(&gui);
                subscribe_unit(id, "dismissed", move || {
                    if let Ok(mut gui) = gui.lock()
                        && gui.palette.is_open()
                    {
                        close_palette(&mut gui);
                        gui.message = tr("ui.status.palette-closed");
                        update_status(&gui);
                        crate::host::log::emit(
                            crate::host::log::Level::Info,
                            "app::run::workbench-palette",
                            "palette dismissed by Escape",
                        );
                    }
                });
            }
            {
                let gui = Arc::clone(&gui);
                let _ = with_widget_mut(id, move |widget| {
                    widget.base().focus_lost.connect(move || {
                        if let Ok(mut gui) = gui.lock()
                            && gui.palette.is_open()
                        {
                            close_palette(&mut gui);
                            gui.message = tr("ui.status.palette-closed");
                            update_status(&gui);
                        }
                    });
                    true
                });
            }
            if let Ok(mut gui) = gui.lock() {
                gui.palette_surface = id;
                gui.placements.push((id, Area::Palette));
            }
        }
        Err(error) => warn_failure(
            "the command palette could not be mounted",
            format!("{error:?}"),
        ),
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

#[cfg(test)]
mod tests {
    use super::onboarding_text;
    use crate::app::commands::command::built_in;

    #[test]
    fn the_onboarding_page_lists_every_registered_command() {
        let registry = built_in();
        let text = onboarding_text(&registry);
        assert!(text.contains("# Welcome to rustage"));
        for command in registry.commands() {
            assert!(
                text.contains(&command.id().to_string()),
                "{} is missing from the onboarding page",
                command.id()
            );
        }
    }
}
