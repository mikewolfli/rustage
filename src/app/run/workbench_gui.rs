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
//! * **Does:** open a real 1280x800 window; show the built-in panels plus `pages`, `events`,
//!   `widgets` and `data`; insert a control from the contract-backed widget library; toggle a panel
//!   from the toolbar or the menu; save the current panels/groups to a layout file and load it on the
//!   next start; split the editor into two groups; switch the active tab; update the status bar; leave
//!   a bounded session on `--exit-after-ms`.
//! * **Does not:** drag-dock panels, float them, restore scroll positions per tab, or paint a
//!   full code editor (the editor area shows the active document's text lines in a list). Those
//!   are the next slices of the same layer, not a claim made here.
//!
//! The window is a real one and the widgets are the library's own painted controls: the modules
//! above are the state, `rust_widgets::app::WindowHandle` is the presentation, and every action
//! updates the model first and the widgets second.

use std::any::Any;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use rust_widgets::app::{ButtonHandle, MenuHandle, WidgetHandle, WindowHandle};
use rust_widgets::control_backend::get_control_backend_for_widget;
use rust_widgets::core::{Color, Point, Rect};
use rust_widgets::layout::{BoxLayout, Layout as _, LayoutConstraints, Orientation, SizePolicy};
use rust_widgets::widget::PropertyGrid;
use rust_widgets::widget::PropertyItem;
use rust_widgets::widget::capability::CapabilityValue;
use rust_widgets::widget::dialog::alert_dialogs::ConfirmDialog;
use rust_widgets::widget::dialog::input_dialog::InputDialog;
use rust_widgets::widget::runtime::{
    self, clear_focus, focus_widget, focused_widget, with_widget_mut,
};
use rust_widgets::widget::special_widgets::canvas::CanvasTransform;
use rust_widgets::widget::special_widgets::code_editor::{CodeEditor, TextChange};
use rust_widgets::widget::view_widgets::PropertyItemKind;
use rust_widgets::widget::{GroupBox, ListBox, Menu, TabWidget, ToolBar};

use crate::app::canvas::scene::Rect as SceneRect;
use crate::app::canvas::widget_catalog::{Catalog, WidgetDescriptor};
use crate::app::canvas::widget_favorites::Favorites;
use crate::app::commands::command::{Call, Context, Registry, built_in};
use crate::app::commands::palette::Recents;
use crate::app::shell::groups::{Axis, Groups};
use crate::app::shell::layout::Layout;
use crate::app::shell::panels::{Dock, PanelSpec, Panels, built_in as built_in_panels};
use crate::app::shell::workbench::{Mode, Workbench};
use crate::contracts::identity::NodeId;
use crate::contracts::permission::EffectClass;
use crate::host::platform::NativePlatform;
use crate::host::startup::{StartupPlatform, StartupRequest};
use crate::project::lifecycle::session::ProjectSession;
use crate::project::record::entries::ProjectTree;
use crate::project::record::format::{PROJECT_FILE_NAME, ProjectContent, migration_path};

use super::report;
use super::workbench_debug::{Command as DebugCommand, DebugState, Update as DebugUpdate};
use super::workbench_events::EventsState;
use super::workbench_inspect::InspectState;
use super::workbench_layout::{Area, WorkbenchLayout};
use super::workbench_pages::PagesState;
use super::workbench_palette::Palette;
use super::workbench_property::PropertyState;
use super::workbench_scene::{
    CanvasLayout, CanvasState, CanvasView, ClipboardSubtree, DESIGN_HEIGHT, DESIGN_WIDTH,
    canvas_rect, span,
};

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 800;
const MENU_HEIGHT: u32 = 24;
const TOOLBAR_Y: i32 = 30;
const TOOLBAR_HEIGHT: u32 = 32;
const CONTENT_Y: i32 = 70;
const STATUS_Y: i32 = 768;
const STATUS_HEIGHT: u32 = 28;

/// The panels this window adds to the built-in set.
const EXTRA_PANELS: [(&str, &str, Dock); 37] = [
    ("widgets", "Widgets", Dock::Left),
    ("pages", "Pages", Dock::Left),
    ("events", "Events", Dock::Left),
    ("assets", "Assets", Dock::Left),
    ("project", "Project", Dock::Left),
    ("data", "Data & State", Dock::Right),
    ("states", "States", Dock::Right),
    ("items", "Collection contents", Dock::Right),
    ("components", "Components", Dock::Right),
    ("timeline", "Timeline", Dock::Right),
    ("ai", "AI session", Dock::Right),
    ("source", "Source outline", Dock::Left),
    ("layout", "Layout inspector", Dock::Right),
    ("theme", "Theme", Dock::Right),
    ("debug", "Debug", Dock::Right),
    ("breakpoints", "Breakpoints", Dock::Right),
    ("stack", "Call stack", Dock::Right),
    ("variables", "Variables", Dock::Right),
    ("targets", "Targets", Dock::Right),
    ("actions", "Actions", Dock::Right),
    ("help", "Help", Dock::Bottom),
    ("search", "Search", Dock::Bottom),
    ("environments", "Environments", Dock::Bottom),
    ("inspection", "Inspection", Dock::Bottom),
    ("vcs", "Version control", Dock::Bottom),
    ("dependencies", "Dependencies", Dock::Bottom),
    ("notifications", "Notifications", Dock::Bottom),
    ("application", "Application structure", Dock::Bottom),
    ("history", "History", Dock::Bottom),
    ("extensions", "Extensions", Dock::Bottom),
    ("tests", "Tests", Dock::Bottom),
    ("tasks", "Background tasks", Dock::Bottom),
    ("build", "Build output", Dock::Bottom),
    ("run", "Application output", Dock::Bottom),
    ("terminal", "Terminal", Dock::Bottom),
    ("preview", "Preview", Dock::Bottom),
    ("hot", "Hot update", Dock::Bottom),
];

/// The Inspector's right-click menu: label key and action.
const INSPECTOR_MENU_KEYS: [(&str, &str); 3] = [
    ("ui.context.clear", "clear"),
    ("ui.context.reset", "reset"),
    ("ui.context.inspect", "inspect"),
];

/// The scene node's right-click menu.
const SCENE_MENU_KEYS: [(&str, &str); 16] = [
    ("ui.context.select-parent", "select-parent"),
    ("ui.context.select-child", "select-child"),
    ("ui.context.rename", "rename"),
    ("ui.context.wrap-container", "wrap-container"),
    ("ui.context.move-out", "move-out"),
    ("ui.context.move-into", "move-into"),
    ("ui.context.copy-node", "copy-node"),
    ("ui.context.paste-node", "paste-node"),
    ("ui.context.toggle-lock", "toggle-lock"),
    ("ui.context.toggle-hide", "toggle-hide"),
    ("ui.context.raise", "raise"),
    ("ui.context.raise-one", "raise-one"),
    ("ui.context.lower-one", "lower-one"),
    ("ui.context.send-to-back", "send-to-back"),
    ("ui.context.duplicate", "duplicate"),
    ("ui.context.delete", "delete"),
];

/// The editor tab's right-click menu.
const EDITOR_MENU_KEYS: [(&str, &str); 5] = [
    ("ui.context.editor.close", "close"),
    ("ui.context.editor.close-others", "close-others"),
    ("ui.context.editor.close-right", "close-right"),
    ("ui.context.editor.split", "split"),
    ("ui.context.editor.next-group", "next-group"),
];

/// The Messages list's right-click menu.
const DIAGNOSTIC_MENU_KEYS: [(&str, &str); 3] = [
    ("ui.context.diagnostic.copy", "copy"),
    ("ui.context.diagnostic.copy-all", "copy-all"),
    ("ui.context.diagnostic.clear", "clear"),
];

/// One widget-library row's label: the library's designer category and the control name, with a
/// `*` when the person marked the control.
///
/// The category is a token the contract publishes (`basic`, `advanced`, ...); it is data about the
/// control, not an interface sentence, so it is not translated here.
fn widget_row(catalog: &Catalog, descriptor: &WidgetDescriptor) -> String {
    let marker = if catalog.is_favorite(&descriptor.id) {
        "* "
    } else {
        ""
    };
    format!("{marker}[{}] {}", descriptor.category, descriptor.name)
}

/// The rows for `query`, as `(contract id, label)` pairs.
///
/// An empty query lists every control grouped by the library's designer category; a query goes
/// through the catalog's own fuzzy search, so the window does not keep a second filter. In both
/// cases the id is what the activation path looks up: a row is a place, not the descriptor itself.
fn widget_rows(catalog: &Catalog, query: &str) -> Vec<(String, String)> {
    let query = query.trim();
    if query.is_empty() {
        let mut rows = Vec::new();
        for category in catalog.categories() {
            for descriptor in catalog.in_category(category) {
                rows.push((descriptor.id.clone(), widget_row(catalog, descriptor)));
            }
        }
        rows
    } else {
        catalog
            .search(query)
            .into_iter()
            .map(|descriptor| (descriptor.id.clone(), widget_row(catalog, descriptor)))
            .collect()
    }
}

/// The local toolbox a desktop target has on this machine, probed once per session.
///
/// `Probe::from_path` reads every directory on `PATH`; two panels ask for the toolbox, so the
/// result is kept rather than read twice. `host` decides the signing program; the mobile platform
/// is unused by a desktop profile.
fn local_toolbox() -> &'static crate::app::targets::local::Toolbox {
    static TOOLBOX: std::sync::OnceLock<crate::app::targets::local::Toolbox> =
        std::sync::OnceLock::new();
    TOOLBOX.get_or_init(|| {
        crate::app::targets::local::Toolbox::for_target(
            "desktop",
            crate::app::targets::configuration::Os::host(),
            crate::app::targets::local::MobilePlatform::Android,
            &crate::app::targets::local::Probe::from_path(),
        )
    })
}

/// The build-environment probe (cargo, rustc, installed targets, the host SDK), read once.
fn build_environment() -> &'static crate::app::build::toolchain::Environment {
    static ENVIRONMENT: std::sync::OnceLock<crate::app::build::toolchain::Environment> =
        std::sync::OnceLock::new();
    ENVIRONMENT.get_or_init(crate::app::build::toolchain::Environment::probe)
}

/// Opens the project document in `directory`.
///
/// The directory is the unit a person recognises; the document inside it has one name. Nothing is
/// written, trusted or run here — [`ProjectSession::open`] only reads.
fn open_project(directory: &Path) -> Result<ProjectSession, String> {
    let file = directory.join(PROJECT_FILE_NAME);
    ProjectSession::open(&file).map_err(|error| error.message())
}

/// The Project panel's rows: the document's own tree, grouped the way the project model groups it.
///
/// The tree is the model's `ProjectTree`, not a second scan of the directory: pages, project files,
/// generated outputs, stylesheets, target profiles and the network services the document declares.
fn project_rows(project: &Option<ProjectSession>) -> Vec<String> {
    let Some(session) = project else {
        return vec![tr("ui.project.none")];
    };
    let Some(document) = session.file().document() else {
        return vec![tr("ui.project.unreadable")];
    };
    let root = session.root().as_path().display().to_string();
    ProjectTree::of(document, Some(&root)).render()
}

/// The Dependencies panel's rows: the lockfile and the resolution a caller would have to authorize.
fn dependency_rows(project: &Option<ProjectSession>) -> Vec<String> {
    let Some(session) = project else {
        return vec![tr("ui.project.none")];
    };
    crate::app::run::workbench_dependencies::rows(session.root().as_path())
}

/// The Environments panel's rows: the local toolkit and the facts every capability claim rests on.
///
/// The roadmap's panel is "the local kit/toolchain and the capabilities"; a connection or container
/// is a future extension and is deliberately not shown. Every row is a fact this process can read
/// now: the backend the window is drawn with, the host, the pinned dependency, the active interface
/// language, and the local tools a desktop target needs on this machine.
fn environment_rows() -> Vec<String> {
    let revision = crate::probe::DEPENDENCY_REVISION;
    let language = rust_widgets::i18n::get_manager()
        .as_ref()
        .map(|manager| manager.current_language().clone())
        .unwrap_or_else(|| "en".to_owned());
    let mut rows = vec![
        format!(
            "{}: {}",
            tr("ui.environment.backend"),
            rust_widgets::platform::backend_name()
        ),
        format!(
            "{}: {}/{}",
            tr("ui.environment.platform"),
            std::env::consts::OS,
            std::env::consts::ARCH
        ),
        format!(
            "{}: {} {} @ {}",
            tr("ui.environment.dependency"),
            crate::probe::DEPENDENCY_CRATE,
            crate::probe::DEPENDENCY_VERSION,
            &revision[..12.min(revision.len())]
        ),
        format!("{}: {language}", tr("ui.environment.language")),
    ];
    rows.extend(local_toolbox().render());
    rows
}

/// The Targets panel's rows: the profile set, the installed targets, the toolchain and SDK, the
/// device seam and the certification level.
///
/// The designer has no target selected in this session, so nothing here is a chosen value: the rows
/// are what `Environment::probe` and `Toolbox` measured, plus the certification report's own state.
/// A cell without evidence stays unverified, so `0/25` is a result rather than a blank.
fn target_rows() -> Vec<String> {
    use crate::app::targets::certification::{Report, Stage};
    use crate::app::targets::configuration::PROFILES;
    use crate::app::targets::local::{ToolKind, ToolState};

    let environment = build_environment();
    let none = tr("ui.target.none");
    let targets = if environment.targets.is_empty() {
        none.clone()
    } else {
        environment.targets.join(", ")
    };
    let toolchain = match (&environment.cargo, &environment.rustc) {
        (Some(cargo), Some(rustc)) => format!("{cargo}; {rustc}"),
        (Some(line), None) | (None, Some(line)) => line.clone(),
        (None, None) => none.clone(),
    };
    let devices = match local_toolbox().get(ToolKind::Devices) {
        Some(tool) => match (&tool.state, &tool.program) {
            (ToolState::Present, Some(program)) => program.clone(),
            (ToolState::Missing { remedy }, Some(program)) => format!("{program} ({remedy})"),
            (ToolState::NotApplicable { reason }, _) => reason.clone(),
            (_, None) => none.clone(),
        },
        None => none.clone(),
    };
    let report = Report::new();
    let total = report.rows().len() * Stage::all().len();
    let verified = total.saturating_sub(report.unverified().len());
    vec![
        format!("{}: {}", tr("ui.target.profiles"), PROFILES.join(", ")),
        format!("{}: {targets}", tr("ui.target.target")),
        format!("{}: {toolchain}", tr("ui.target.toolchain")),
        format!(
            "{}: {}",
            tr("ui.target.sdk"),
            environment.sdk.clone().unwrap_or_else(|| none.clone())
        ),
        format!("{}: {devices}", tr("ui.target.devices")),
        format!(
            "{}: {verified}/{total} {}",
            tr("ui.target.certification"),
            tr("ui.target.verified")
        ),
    ]
}

/// The property the Inspector edits for one control.
///
/// `text` and `title` come first because they are the two properties the scene's own contracts use.
/// Otherwise the control's first published property. `None` means the Inspector would have nothing
/// to edit, and the palette refuses that row rather than inventing a property name.
fn inspector_property(descriptor: &WidgetDescriptor) -> Option<String> {
    ["text", "title"]
        .iter()
        .find_map(|name| {
            descriptor
                .properties
                .iter()
                .find(|property| property.name == *name)
        })
        .or_else(|| descriptor.properties.first())
        .map(|property| property.name.clone())
}

/// The Inspector's property grid, the Clear/Reset/Values buttons, the theme field and button,
/// then the error line and note.
type InspectorControls = (u64, u64, u64, u64, u64, u64, u64, u64);

/// The Events panel's list, handler field, Bind/Unbind/Fire buttons, error line and note.
type EventControls = (u64, u64, u64, u64, u64, u64, u64);

/// The Data panel's property grid, error line and note.
type DataControls = (u64, u64, u64);

/// The Pages panel's list, field, Add/Open/Back/Fwd/Close buttons, error line and note.
type PageControls = (u64, u64, u64, u64, u64, u64, u64, u64, u64);

/// The panels and chrome zen mode hid, held only while zen is on.
struct ZenSnapshot {
    shown: Vec<String>,
    toolbar: bool,
    status: bool,
}

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

/// What the rename dialog answered.
enum RenameAnswer {
    /// The field's text, as the dialog reported it.
    Accepted(String),
    /// Escape or the cancel path.
    Cancelled,
}

/// One answer from the rename dialog, waiting for the main loop to apply it.
///
/// The dialog's signals run inside the widget runtime's event dispatch, so they must not lock the
/// `Gui` (the smoke path already holds it while dispatching). They write this slot instead, and the
/// poller applies the answer outside every widget borrow.
struct RenameRequest {
    node: NodeId,
    answer: RenameAnswer,
}

/// The answer to the unsaved-changes question a window close asked.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum CloseAnswer {
    /// Quit and discard the unsaved buffers (the person chose the confirming action).
    Discard,
    /// Keep the window open; nothing is discarded.
    KeepEditing,
}

/// Which one-field prompt is open.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PromptKind {
    /// Go to line (or line:column).
    Goto,
    /// Quick-open a project file by path or fuzzy query.
    OpenFile,
    /// Jump to a symbol in the focused editor's document.
    FileSymbol,
    /// Jump to a symbol the language server reports for the workspace.
    WorkspaceSymbol,
    /// Write the focused document to a new project path and keep working there.
    SaveAs,
    /// Write a copy of the focused document without changing the active document.
    SaveCopy,
    /// Confirm removing the declaration of unreferenced resources.
    CleanResources,
}

/// One answer from a one-field prompt, waiting for the main loop to apply it.
enum Prompt {
    /// The go-to-line field's text.
    Goto(String),
    /// The quick-open field's text.
    OpenFile(String),
    /// The file-symbol field's text.
    FileSymbol(String),
    /// The workspace-symbol field's text.
    WorkspaceSymbol(String),
    /// The save-as path field's text.
    SaveAs(String),
    /// The save-copy path field's text.
    SaveCopy(String),
    /// The resource-cleanup confirmation field's text.
    CleanResources(String),
    /// Escape or the cancel path, with the prompt it came from.
    Cancelled(PromptKind),
}

/// One location the editor can return to: a document and a caret position in it.
#[derive(Clone, PartialEq, Eq, Debug)]
struct JumpLocation {
    /// The document the caret was in.
    document: String,
    /// The zero-based line.
    line: usize,
    /// The zero-based character column.
    column: usize,
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
        "widgets" => "ui.panel.widgets",
        "pages" => "ui.panel.pages",
        "assets" => "ui.panel.assets",
        "outline" => "ui.panel.outline",
        "events" => "ui.panel.events",
        "theme" => "ui.panel.theme",
        "help" => "ui.panel.help",
        "search" => "ui.panel.search",
        "environments" => "ui.panel.environments",
        "targets" => "ui.panel.targets",
        "inspection" => "ui.panel.inspection",
        "vcs" => "ui.panel.vcs",
        "project" => "ui.panel.project",
        "dependencies" => "ui.panel.dependencies",
        "breakpoints" => "ui.panel.breakpoints",
        "stack" => "ui.panel.stack",
        "variables" => "ui.panel.variables",
        "actions" => "ui.panel.actions",
        "notifications" => "ui.panel.notifications",
        "application" => "ui.panel.application",
        "history" => "ui.panel.history",
        "extensions" => "ui.panel.extensions",
        "tests" => "ui.panel.tests",
        "tasks" => "ui.panel.tasks",
        "build" => "ui.panel.build",
        "run" => "ui.panel.run",
        "terminal" => "ui.panel.terminal",
        "preview" => "ui.panel.preview",
        "hot" => "ui.panel.hot",
        "inspector" => "ui.panel.inspector",
        "data" => "ui.panel.data",
        "states" => "ui.panel.states",
        "items" => "ui.panel.items",
        "components" => "ui.panel.components",
        "timeline" => "ui.panel.timeline",
        "ai" => "ui.panel.ai",
        "source" => "ui.panel.source-outline",
        "layout" => "ui.panel.layout",
        "messages" => "ui.panel.messages",
        "problems" => "ui.panel.problems",
        "debug" => "ui.panel.debug",
        _ => "ui.panel.outline",
    }
}

/// The action a View → Panels entry runs.
///
/// The ledger id is what `view.panel-show` accepts; a panel outside §2.2 (the built-in Messages
/// list) keeps the window's own toggle, because the command's argument list is the ledger.
fn panel_action(host: &str) -> String {
    match crate::app::shell::panels::REQUIRED_PANELS
        .iter()
        .find(|panel| panel.host_id() == Some(host))
    {
        Some(panel) => format!("command:view.panel-show?panel={}", panel.id()),
        None => format!("toggle:{host}"),
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
    shortcuts: &BTreeMap<String, String>,
    actions: &mut BTreeMap<u64, String>,
    gui: &Arc<Mutex<Gui>>,
) {
    let translated = tr(key);
    let label = if translated == key {
        title.to_owned()
    } else {
        translated
    };
    // The new leaf lands at the row the menu currently ends on; a separator added later would
    // shift only rows after it, and those are not history commands.
    let index = menu.item_count();
    // The chord is display text (the library documents it as such), and this window really routes
    // the single-key ones, so the menu and the palette name the same key.
    let shortcut = shortcuts
        .get(crate::app::commands::menu::Menu::command_id(command))
        .map(String::as_str);
    let item = handle.new_menu_item(menu, &label, shortcut);
    actions.insert(item.raw_id(), format!("command:{command}"));
    if let Ok(mut gui) = gui.lock() {
        // Undo and Redo follow the editor's own history state; record the row so
        // `refresh_history_commands` can enable or disable it without rebuilding the menu.
        if command == "edit.undo" || command == "edit.redo" {
            gui.history_items
                .push((menu.raw_id(), index, command.to_owned()));
        }
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
    /// The dock slot each panel was placed in. A slot hosts one visible panel at a time, so the
    /// toggle action hides the other panels sharing it.
    panel_areas: BTreeMap<String, Area>,
    /// The inspection and version-control lists, refreshed when their panels are shown.
    inspection_list: u64,
    vcs_list: u64,
    vcs_menu: u64,
    /// The project opened for this session, when one was named on the command line.
    project: Option<ProjectSession>,
    /// The text each document had on disk when it was read or last saved.
    saved_texts: BTreeMap<String, String>,
    /// The menu rows that must follow the editor's history state.
    ///
    /// `(menu id, row index, command id)`. The library enables a row by index, and the rows are
    /// created once, so the index recorded at build time stays valid across relabelling.
    history_items: Vec<(u64, usize, String)>,
    /// The session's permission record; trusting a project is the person's decision.
    permissions: crate::host::permission::PermissionService,
    project_list: u64,
    dependencies_list: u64,
    /// The debug session's separate panels and the actions list.
    breakpoints_list: u64,
    stack_list: u64,
    variables_list: u64,
    actions_list: u64,
    /// The host notification centre and the panel that shows it.
    notifications: crate::app::run::workbench_notifications::Notifications,
    notifications_list: u64,
    notifications_menu: u64,
    notification_row: Option<usize>,
    application_list: u64,
    /// The local extension registry the panel lists (empty until an install surface lands).
    extensions: crate::app::extension::Registry,
    extensions_usage: crate::app::extension::manager::Usage,
    extensions_list: u64,
    history_list: u64,
    /// The page state machine the States panel reads, and where it was read from.
    states_list: u64,
    states_source: Option<String>,
    states: Option<crate::project::record::page::PageStates>,
    /// The collection-contents panel's list.
    items_list: u64,
    /// The component-library panel's list.
    components_list: u64,
    /// The transition-preview panel's list.
    timeline_list: u64,
    /// The AI-session journal path and the list that shows it.
    ai_journal: Option<PathBuf>,
    ai_list: u64,
    /// The source-outline panel: the language server it may start and the rows it shows.
    source: crate::app::run::workbench_outline::SourceOutline,
    source_list: u64,
    source_field: u64,
    source_filter: String,
    /// Where code navigation has been, newest last; [`Gui::jump_forward`] is the reverse.
    jump_back: Vec<JumpLocation>,
    jump_forward: Vec<JumpLocation>,
    /// Documents closed by a command, newest last, with the text they had.
    closed_documents: Vec<(String, String)>,
    /// The resource paths a cleanup preview listed, waiting for confirmation.
    pending_resource_cleanup: Vec<String>,
    /// Commands the window executed, newest last, with the millisecond they ran.
    command_history: Vec<(u64, String)>,
    /// The layout-inspector panel's list.
    layout_list: u64,
    /// The test runner's state and the controls that show it.
    tests: crate::app::run::workbench_tests::TestsState,
    tests_list: u64,
    tests_menu: u64,
    /// The process executor's records.
    tasks: crate::app::run::workbench_tasks::Tasks,
    tasks_list: u64,
    /// The last build submitted to the same executor, and the controls that show it.
    build: crate::app::run::workbench_build::BuildState,
    build_owner: crate::host::tasks::OwnerId,
    build_list: u64,
    build_menu: u64,
    /// The last started program, the field that names it, and the panel that shows it.
    run: crate::app::run::workbench_run::RunState,
    run_list: u64,
    run_field: u64,
    run_program: String,
    run_menu: u64,
    /// The terminal session, its command-line field and its output list.
    terminal: crate::app::run::workbench_terminal::TerminalState,
    terminal_list: u64,
    terminal_field: u64,
    terminal_input: String,
    terminal_menu: u64,
    /// The preview service's dock slot and menu; the service itself lives on the main thread.
    preview_list: u64,
    preview_menu: u64,
    /// Whether the preview has a mounted library document (mount first, then update).
    preview_json: bool,
    hot: crate::app::preview::hot_update::Updater,
    hot_list: u64,
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
    /// The design surface's zoom and helpers; the scene itself is in design space (roadmap
    /// `design.tool.*`).
    canvas_view: CanvasView,
    scene: CanvasState,
    outline_list: u64,
    /// The in-app scene clipboard (not the OS clipboard; T05.06 owns that payload).
    scene_clipboard: Option<ClipboardSubtree>,
    /// The widget library: the captured contract, the persisted favorites, the ids of the rows
    /// currently shown, the search query those rows came from, the row a context menu is open on,
    /// the menu itself, and the two controls that show them.
    widget_catalog: Option<Catalog>,
    widget_favorites: Favorites,
    widget_favorites_path: Option<PathBuf>,
    widget_visible: Vec<String>,
    widget_query: String,
    widget_row: Option<usize>,
    widget_menu: u64,
    widgets_search: u64,
    widgets_list: u64,
    /// The Search panel's query field, result list and current hits.
    search_field: u64,
    search_list: u64,
    search_hits: Vec<SearchHit>,
    /// The Messages panel's log lines, newest last, and the list that shows them.
    messages: Vec<String>,
    messages_list: u64,
    /// The row the Messages context menu was opened on.
    diagnostic_row: Option<usize>,
    /// The Messages panel's right-click menu.
    diagnostic_menu: u64,
    canvas_panel: u64,
    canvas_surface: u64,
    /// The library's one tooltip pipeline; the runtime forwards hover to it (blue22 H-20).
    tooltip: u64,
    /// The colour editor popup: a panel and the library's picker, well, history and swatch.
    /// The row is the grid row the picker was opened for (blue22 H-18).
    color_panel: u64,
    color_picker: u64,
    color_well: u64,
    color_history: u64,
    color_swatch: u64,
    color_row: usize,
    /// The canvas pointer facts, in design space: whether a press is down, where it began and
    /// where the pointer last was. A drag wider/taller than [`DRAG_THRESHOLD`] is a marquee.
    drag_active: bool,
    drag_start: Option<(i32, i32)>,
    pointer: Option<(i32, i32)>,
    /// The node drag the press started, and the ghost overlay while it is past the threshold.
    drag: Option<crate::app::canvas::drag::Drag>,
    ghost: u64,
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
    outline_index: BTreeMap<NodeId, usize>,
    outline_content_revision: u64,
    /// Set while the Outline's current row is being written from the model.
    ///
    /// `list_box_set_current_index` emits `item_selected` synchronously. That emission is the model
    /// following itself, not a person clicking: without the guard, a queued rebuild can re-select a
    /// node the selection already dropped (deleting the selected node used to end with another node
    /// selected, from a rebuild scheduled before the delete).
    outline_syncing: Arc<AtomicBool>,
    /// Set while `sync_editors` writes the tab strips from the model.
    ///
    /// `TabWidget` emits `current_changed` synchronously when the active tab is set. The subscriber
    /// locks this same `Gui`, so a rebuild that changes the active tab while the caller already
    /// holds the lock would deadlock the main loop; the guard makes that emission the model
    /// following itself instead of a person's click.
    syncing_editors: Arc<AtomicBool>,
    /// The document each editor surface is showing.
    ///
    /// A text control reports a change with its new text but not with the document it was showing.
    /// This map is written by [`Gui::sync_editors`] in the same step that loads a document into the
    /// surface, so a change reported later is attributed to the text the surface actually held. It
    /// lives outside the `Gui` mutex because the report arrives while a caller may already hold that
    /// mutex (see the guard note above).
    editor_documents: Arc<Mutex<BTreeMap<u64, String>>>,
    /// The editor surfaces whose text changed since the model last read them.
    ///
    /// The library's `text_change` signal describes one edit and deliberately does not clone the
    /// document, so the report only records *which* surface moved. The model reads that surface's
    /// text on the next main-loop turn (or before the next load), which keeps the cost proportional
    /// to the edit rather than to the document.
    editor_changed: Arc<Mutex<BTreeSet<u64>>>,
    /// Labels re-read after a translation hot reload.
    translatable: Vec<Translatable>,
    /// The toolbar and the catalog keys of its items, so a reload can rebuild its labels.
    toolbar: u64,
    toolbar_items: Vec<(&'static str, &'static str)>,
    /// Whether the toolbar and the status bar take their row in the layout (roadmap view/display).
    show_toolbar: bool,
    show_status: bool,
    /// Whether the layout is locked against save, load and reset (roadmap view/workspace).
    layout_locked: bool,
    /// Panels hidden most recently, newest last, so restore can put one back (bounded).
    closed_panels: Vec<String>,
    /// Whether the platform window is in full screen.
    fullscreen: bool,
    /// The panels and chrome zen mode hid, held only while zen is on.
    zen: Option<ZenSnapshot>,
    /// A weak handle to the shared state, so a command that starts a background poller can hand it
    /// the same handle the menu closures hold without a second `Arc` cycle.
    self_weak: Weak<Mutex<Gui>>,
    /// The Inspector's right-click menu.
    context_menu: u64,
    /// The scene node's right-click menu.
    scene_menu: u64,
    /// The editor tab's right-click menu.
    editor_menu: u64,
    /// The mounted rename dialog (0 when none) and the slot its signals write.
    rename_dialog: u64,
    /// The one-field prompt dialog, when one is open.
    prompt_dialog: u64,
    /// One prompt answer waiting for the main loop.
    prompt_slot: Arc<Mutex<Option<Prompt>>>,
    /// The mounted unsaved-changes confirmation (0 when none) and the slot its answer arrives in.
    ///
    /// The dialog is the library's `ConfirmDialog`; its signals only write the slot, because they
    /// run inside the runtime's dispatch while the close handler may already hold this `Gui`.
    close_dialog: u64,
    close_slot: Arc<Mutex<Option<CloseAnswer>>>,
    rename_slot: Arc<Mutex<Option<RenameRequest>>>,
}

/// One hit in the Search panel, with the range a click selects.
#[derive(Clone, PartialEq, Eq, Debug)]
struct SearchHit {
    document: String,
    offset: u32,
    length: u32,
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
    view: CanvasView,
}

/// How far a press must move in design space before it is a marquee rather than a click.
const DRAG_THRESHOLD: u32 = 8;

/// One group's strip as [`Gui::sync_editors`] reads it.
///
/// The documents it shows, which one is active, that document's model text, and the index the
/// strip should show. A named record keeps the load, the tab labels and the "which document is this
/// surface showing" map from sharing one unread tuple.
struct StripState {
    documents: Vec<String>,
    active_document: Option<String>,
    active_index: Option<usize>,
}

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
        let mut layout = WorkbenchLayout::new(self.groups.len() as u32)
            .showing_toolbar(self.show_toolbar)
            .showing_status(self.show_status);
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
        // A text control's edit reaches the model before the model is loaded back into the control:
        // otherwise this load would overwrite the very text that has not been read yet.
        self.apply_pending_editor_text();
        // `TabWidget::set_current_index` emits `current_changed` synchronously, and that signal's
        // subscriber locks this same `Gui`; a rebuild the caller asked for is not a click. The flag
        // is cleared before the function returns, and nothing below returns early.
        self.syncing_editors.store(true, Ordering::SeqCst);
        // The model decides how many groups exist; the responsive layout follows it. Geometry is
        // no longer written here, so the two cannot disagree about which box a group owns.
        self.refresh_layout();
        let groups: Vec<StripState> = self
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
                let active_document = tabs.active().map(|tab| tab.document().to_owned());
                let active_index = tabs
                    .tabs()
                    .iter()
                    .position(|tab| tabs.active().is_some_and(|active| active == tab));
                StripState {
                    documents,
                    active_document,
                    active_index,
                }
            })
            .collect();
        for (index, view) in self.group_views.iter_mut().enumerate() {
            let Some(StripState {
                documents,
                active_document,
                active_index,
            }) = groups.get(index)
            else {
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
            let dirty = self.groups.dirty_documents();
            for (tab, document) in documents.iter().enumerate() {
                let label = tab_label(document, dirty.contains(document));
                if tab < count {
                    tabs.set_tab_text(tab, &label);
                } else if tabs.add_tab(&label, None).is_some() {
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
            if let Some(active) = active_index {
                tabs.set_current_index(*active);
            }
            // What the surface is showing, recorded in the same step that loads it: a change the
            // control reports later is attributed to the document it actually held.
            if let Ok(mut showing) = self.editor_documents.lock() {
                match active_document {
                    Some(document) => {
                        showing.insert(view.editor, document.clone());
                    }
                    None => {
                        showing.remove(&view.editor);
                    }
                }
            }
            let applied = with_widget_mut(view.editor, |widget| {
                let any: &mut dyn std::any::Any = widget;
                match any.downcast_mut::<CodeEditor>() {
                    Some(editor) => {
                        let texts: BTreeMap<String, String> = documents
                            .iter()
                            .filter_map(|document| {
                                self.groups
                                    .text(document)
                                    .map(|text| (document.clone(), text.to_owned()))
                            })
                            .collect();
                        sync_editor_buffers(editor, documents, active_document.as_deref(), &texts);
                        true
                    }
                    None => false,
                }
            });
            if applied != Some(true) {
                self.message = "the editor surface did not accept the document".to_owned();
                crate::host::log::emit(
                    crate::host::log::Level::Warn,
                    "app::run::workbench-editor",
                    &format!(
                        "surface {} did not accept {active_document:?} ({applied:?})",
                        view.editor
                    ),
                );
            }
        }
        // The title and status line name the buffer the editor area is actually showing, and its
        // saved state; both are read from the tabs rather than kept in a second record.
        sync_workbench_document(self);
        rust_widgets::set_widget_text(
            self.status,
            &format!("{} | {}", self.status_line(), self.message),
        );
        // A split or a panel toggle replaces the layout object, which re-places the canvas
        // control; the stored draw commands are absolute, so they are rebuilt for the new rect.
        sync_canvas(self);
        self.syncing_editors.store(false, Ordering::SeqCst);
    }

    /// Reads the editors whose text changed into the one model buffer.
    ///
    /// The library reports an edit as a line-range description rather than as a document clone —
    /// that is what keeps a keystroke proportional to the edit — so the text is read here, from the
    /// control that holds it. The read runs on the main loop, outside the widget borrow the signal
    /// was delivered in.
    ///
    /// A read that cannot happen yet (the surface is lent out to the caller) leaves the report in
    /// the pending set, so a later turn still sees it. A surface that is gone, or one showing no
    /// document, has nothing to write and is dropped.
    fn apply_pending_editor_text(&mut self) {
        let changed: Vec<u64> = match self.editor_changed.lock() {
            Ok(mut set) => std::mem::take(&mut *set).into_iter().collect(),
            Err(poisoned) => std::mem::take(&mut *poisoned.into_inner())
                .into_iter()
                .collect(),
        };
        if changed.is_empty() {
            return;
        }
        let mut still_pending = Vec::new();
        for editor in changed {
            let document = match self.editor_documents.lock() {
                Ok(showing) => showing.get(&editor).cloned(),
                Err(poisoned) => poisoned.into_inner().get(&editor).cloned(),
            };
            // A surface that is showing nothing (a closed group, an empty strip) names no buffer.
            let Some(document) = document else {
                continue;
            };
            let text = with_widget_mut(editor, |widget| {
                let any: &mut dyn std::any::Any = widget;
                any.downcast_mut::<CodeEditor>()
                    .map(|editor| editor.text().to_owned())
            });
            let Some(text) = text else {
                // The surface is lent out right now; the edit is still unread.
                still_pending.push(editor);
                continue;
            };
            // The surface is gone: a report about a control that no longer exists must not be
            // written into whichever buffer now happens to be active.
            let Some(text) = text else {
                continue;
            };
            self.set_document_text(&document, &text);
        }
        if !still_pending.is_empty()
            && let Ok(mut pending) = self.editor_changed.lock()
        {
            pending.extend(still_pending);
        }
    }

    /// Writes one document's text into the model, with its dirty flags, title and status.
    ///
    /// The editor already shows this text, so nothing is loaded back into it: a load would be a
    /// second writer of the same fact and would re-enter the control's own change signal.
    fn set_document_text(&mut self, document: &str, text: &str) {
        if self.groups.text(document) == Some(text) {
            return;
        }
        let was_dirty = self.groups.dirty_documents().contains(document);
        if let Err(reason) = self.groups.set_text(document, text) {
            warn_failure("an editor change could not reach the model", reason);
            return;
        }
        // "Dirty" is the buffer against the text it had on disk, not a counter of keystrokes, so
        // undoing back to the saved text leaves the document clean.
        //
        // A buffer with no disk baseline is never dirty: the synthetic demo documents were never
        // read from a file, `file.save` already refuses them for that reason, and a close that told
        // the person to save them would be advice nothing can carry out.
        let dirty = self
            .saved_texts
            .get(document)
            .is_some_and(|saved| saved != text);
        if dirty != was_dirty {
            crate::host::log::emit(
                crate::host::log::Level::Info,
                "app::run::workbench-editor",
                &format!(
                    "{document}: {}",
                    if dirty {
                        "unsaved changes"
                    } else {
                        "matches the file on disk"
                    }
                ),
            );
        }
        for group in self.groups.group_ids() {
            if let Some(tabs) = self.groups.tabs_mut(group) {
                tabs.mark_dirty(document, dirty);
            }
        }
        sync_workbench_document(self);
        // The strip shows which buffers are unsaved; an edit changed that, so the labels follow it.
        // The editor surfaces are not reloaded: that is `sync_editors`' job, and doing it here would
        // move the caret of the view being typed in.
        refresh_tab_labels(self);
        // Typing is the one event that gives Undo something to do; the menu follows the control.
        refresh_history_commands(self);
        rust_widgets::set_widget_text(
            self.status,
            &format!("{} | {}", self.status_line(), self.message),
        );
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

/// The two places history state shows, for a log line: the menu row and the palette's answer.
///
/// `(command, menu row enabled, palette refusal)`; a `None` refusal means the palette would run it.
fn history_state(gui: &Gui) -> Vec<(String, Option<bool>, Option<String>)> {
    let context = command_context(gui);
    gui.history_items
        .iter()
        .map(|(menu, index, command)| {
            let enabled = with_widget_mut(*menu, |widget| {
                let any: &mut dyn std::any::Any = widget;
                any.downcast_mut::<Menu>()
                    .and_then(|menu| menu.item_enabled(*index))
            })
            .flatten();
            let refusal = gui.registry.why_not(&Call::to(command.clone()), &context);
            (command.clone(), enabled, refusal)
        })
        .collect()
}

/// The one context the palette is filtered with, including what the editor's own history can do.
///
/// `can_undo`/`can_redo` come from the focused `CodeEditor`, so "Undo" is greyed out with its
/// reason exactly when the control would refuse — not from a host counter that could drift.
fn command_context(gui: &Gui) -> Context {
    let (undo, redo) = match edit_with_editor(gui, |editor| (editor.can_undo(), editor.can_redo()))
    {
        Some((undo, redo)) => (Some(undo), Some(redo)),
        // No editor surface: the fact is unknown, so the commands stay available and answer for
        // themselves when they run.
        None => (None, None),
    };
    palette_context().with_history(undo, redo)
}

/// Points the menu's Undo/Redo items at the same history the palette reads.
///
/// The library's `MenuHandle::set_item_enabled` is by row index, so the index of each leaf is
/// recorded when the menu is built. Nothing here is a second answer to "can this run": the
/// decision is the control's own `can_undo`/`can_redo`.
fn refresh_history_commands(gui: &Gui) {
    if gui.history_items.is_empty() {
        return;
    }
    let Some((undo, redo)) = edit_with_editor(gui, |editor| (editor.can_undo(), editor.can_redo()))
    else {
        return;
    };
    for (menu, index, command) in &gui.history_items {
        let enabled = if *command == "edit.undo" { undo } else { redo };
        // The menu enables a row by index; the handle has no such call (the tool bar's does), so
        // this goes through the widget, the same way the host reaches other typed setters.
        let _ = with_widget_mut(*menu, |widget| {
            let any: &mut dyn std::any::Any = widget;
            any.downcast_mut::<Menu>().map(|menu| {
                menu.set_item_enabled(*index, enabled);
                true
            })
        });
    }
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

/// A row as the property grid shows it.
///
/// `mixed` is the library's own "the selected objects differ" state: the grid paints the
/// placeholder and an empty commit keeps the row mixed, rather than the host concatenating the
/// differing values into read-only text (blue22 H-4).
#[derive(Clone, PartialEq, Debug)]
struct GridRow {
    name: String,
    value: String,
    editable: bool,
    mixed: bool,
    /// The editor kind the grid opens: text by default, colour for a colour property.
    kind: PropertyItemKind,
    /// `(min, max, step)` for a numeric row, straight from the control's schema (blue22 H-12).
    bounds: Option<(Option<f64>, Option<f64>, Option<f64>)>,
}

impl GridRow {
    fn new(name: impl Into<String>, value: impl Into<String>, editable: bool) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
            editable,
            mixed: false,
            kind: PropertyItemKind::Text,
            bounds: None,
        }
    }

    fn color(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
            editable: true,
            mixed: false,
            kind: PropertyItemKind::Color,
            bounds: None,
        }
    }

    fn mixed(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: String::new(),
            editable: true,
            mixed: true,
            kind: PropertyItemKind::Text,
            bounds: None,
        }
    }

    fn with_bounds(mut self, min: Option<f64>, max: Option<f64>, step: Option<f64>) -> Self {
        self.bounds = Some((min, max, step));
        self
    }
}

/// Stages a control-level tooltip.
///
/// Entry-level hints exist upstream since RW-62 (`ToolBar::entry_tooltip_at`,
/// `TabWidget::entry_tooltip_at`, `TooltipManager::note_entry_hover`), and the manager below reads
/// them from the hovered control. No host item sets one yet: a toolbar label has no longer text
/// than the label itself, and a hint that repeats the visible text is noise rather than help.
fn set_control_tooltip(id: u64, text: String) {
    if id == 0 {
        return;
    }
    let _ = with_widget_mut(id, move |widget| {
        widget.set_tooltip(text);
        true
    });
}

/// Drives the library's tooltip pipeline from the main thread.
fn schedule_tooltips(gui: Arc<Mutex<Gui>>) {
    let next = Arc::clone(&gui);
    let _ = rust_widgets::schedule_on_main(Duration::from_millis(100), move || {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis() as u64)
            .unwrap_or(0);
        if let Ok(gui) = next.lock()
            && gui.tooltip != 0
        {
            let hovered = runtime::hovered_widget().filter(|id| *id != gui.tooltip);
            let anchor = hovered.and_then(|id| with_widget_mut(id, |widget| widget.geometry()));
            let _ = with_widget_mut(gui.tooltip, move |widget| {
                let Some(manager) = (widget as &mut dyn Any)
                    .downcast_mut::<rust_widgets::widget::companions::TooltipManager>(
                ) else {
                    return false;
                };
                manager.tick(100, now);
                true
            });
            let visible = with_widget_mut(gui.tooltip, |widget| {
                (widget as &mut dyn Any)
                    .downcast_mut::<rust_widgets::widget::companions::TooltipManager>()
                    .is_some_and(|manager| manager.tooltip().is_some())
            })
            .unwrap_or(false);
            match (visible, anchor) {
                (true, Some(rect)) => {
                    rust_widgets::set_widget_geometry(
                        gui.tooltip,
                        rect.x + 12,
                        rect.y + 20,
                        280,
                        60,
                    );
                    rust_widgets::show_widget(gui.tooltip);
                }
                _ => rust_widgets::hide_widget(gui.tooltip),
            }
        }
        schedule_tooltips(next);
    });
}

/// Fills the design-inspection list from the current scene.
///
/// The check returns no row when the scene is clean; the panel shows the same “no problems” line
/// the Problems panel uses rather than an empty list that reads as "not run".
fn refresh_inspection(gui: &mut Gui) {
    let mut rows = crate::app::run::workbench_inspection::rows(&gui.scene);
    if rows.is_empty() {
        rows.push(tr("ui.status.no-problems"));
    }
    set_list_rows(gui.inspection_list, rows);
}

/// Fills the version-control list from the repository containing the working directory.
///
/// The module reads Git on every call, so showing the panel is the moment the status is taken; a
/// missing `git` or a directory that is not a repository is a row, never an empty list.
fn refresh_vcs(gui: &mut Gui) {
    let rows = crate::app::run::workbench_vcs::rows(std::path::Path::new("."));
    set_list_rows(gui.vcs_list, rows);
}

/// Fills the Project panel from the session that was opened at startup.
fn refresh_project(gui: &mut Gui) {
    set_list_rows(gui.project_list, project_rows(&gui.project));
}

/// Fills the Dependencies panel from the open project's root.
fn refresh_dependencies(gui: &mut Gui) {
    set_list_rows(gui.dependencies_list, dependency_rows(&gui.project));
}

/// One debug section, with an honest empty state before a session reports anything.
fn debug_section_rows(rows: Vec<String>) -> Vec<String> {
    if rows.is_empty() {
        vec![tr("ui.debug.no-session")]
    } else {
        rows
    }
}

/// Fills the Breakpoints panel from the debug session's own breakpoint rows.
fn refresh_breakpoints(gui: &mut Gui) {
    let list = gui.breakpoints_list;
    set_list_rows(list, debug_section_rows(gui.debug.breakpoints()));
}

/// Fills the Call-stack panel with the session's threads followed by its frames.
fn refresh_stack(gui: &mut Gui) {
    let list = gui.stack_list;
    let mut rows = gui.debug.threads();
    rows.extend(gui.debug.frames());
    set_list_rows(list, debug_section_rows(rows));
}

/// Fills the Variables panel with the session's scope and variable rows.
fn refresh_variables(gui: &mut Gui) {
    let list = gui.variables_list;
    set_list_rows(list, debug_section_rows(gui.debug.variables()));
}

/// Fills the Actions panel from the command registry and the real keymap.
fn refresh_actions(gui: &mut Gui) {
    let list = gui.actions_list;
    let rows = crate::app::run::workbench_actions::rows(&gui.registry, &window_keymap());
    set_list_rows(list, rows);
}

/// Fills the Notifications panel from the host's notification centre.
fn refresh_notifications(gui: &mut Gui) {
    let list = gui.notifications_list;
    let counts = gui.notifications.counts();
    let errors = counts.errors.to_string();
    let warnings = counts.warnings.to_string();
    let information = counts.information.to_string();
    let mut rows = vec![tr_args(
        "ui.notifications.counts",
        &[
            ("errors", &errors),
            ("warnings", &warnings),
            ("information", &information),
        ],
    )];
    let open = gui.notifications.rows();
    if open.is_empty() {
        rows.push(tr("ui.notifications.none"));
    } else {
        rows.extend(open);
    }
    set_list_rows(list, rows);
}

/// Fills the Application-structure panel from the chrome the window built.
fn refresh_application(gui: &mut Gui) {
    let list = gui.application_list;
    let rows = application_rows(gui);
    set_list_rows(list, rows);
}

/// The active interface language's date and number rules.
fn active_rules() -> crate::app::locale::format::Rules {
    let language = rust_widgets::i18n::get_manager()
        .as_ref()
        .and_then(|manager| crate::host::locale::Language::parse(manager.current_language()))
        .unwrap_or(crate::host::locale::DEFAULT);
    crate::app::locale::format::Rules::for_language(language)
}

/// Fills the History panel from the open project's repository (or the working directory).
fn refresh_history(gui: &mut Gui) {
    let list = gui.history_list;
    let directory = gui
        .project
        .as_ref()
        .map(|project| project.root().as_path())
        .unwrap_or_else(|| std::path::Path::new("."));
    let rows = crate::app::run::workbench_history::rows(directory, 20, active_rules());
    set_list_rows(list, rows);
}

/// Fills the Extensions panel from the local registry the window holds.
fn refresh_extensions(gui: &mut Gui) {
    let list = gui.extensions_list;
    let mut rows =
        crate::app::run::workbench_extensions::rows(&gui.extensions, &gui.extensions_usage);
    if rows.is_empty() {
        rows.push(tr("ui.extensions.none"));
    }
    set_list_rows(list, rows);
}

/// Fills the Tests panel with the last report the worker sent.
fn refresh_tests(gui: &mut Gui) {
    let list = gui.tests_list;
    set_list_rows(list, gui.tests.rows());
}

/// Fills the Background-tasks panel from the executor's own records.
fn refresh_tasks(gui: &mut Gui) {
    let list = gui.tasks_list;
    let mut rows = gui.tasks.rows();
    if rows.is_empty() {
        rows.push(tr("ui.tasks.none"));
    }
    set_list_rows(list, rows);
}

/// Fills the Build-output panel with the last report.
fn refresh_build(gui: &mut Gui) {
    let list = gui.build_list;
    set_list_rows(list, gui.build.rows());
}

/// Mirrors one build report into the list and the status bar.
fn apply_build_update(gui: &mut Gui, update: crate::app::run::workbench_build::Update) {
    gui.message = update.status.clone();
    let rows = update.rows.clone();
    let list = gui.build_list;
    set_list_rows(list, rows.clone());
    update_status(gui);
    crate::host::log::emit(
        crate::host::log::Level::Info,
        "app::run::workbench-build",
        &format!("build: {}", update.status),
    );
    for row in &rows {
        crate::host::log::emit(
            crate::host::log::Level::Info,
            "app::run::workbench-build",
            &format!("build row: {row}"),
        );
    }
    report_build_cause(gui);
}

/// Reports a finished build that did not succeed to the notification centre.
///
/// The cause is built from the executor's own task record ([`crate::host::notify::Cause::for_task`]),
/// so a failed or abandoned build becomes an actionable notification and a repeat increments the
/// existing one. A successful build is not news and is not reported.
fn report_build_cause(gui: &mut Gui) {
    let Some(task) = gui.build.task() else {
        return;
    };
    let record = gui
        .tasks
        .executor()
        .records()
        .into_iter()
        .find(|record| record.id == task);
    let Some(record) = record else {
        return;
    };
    let Some(cause) = crate::host::notify::Cause::for_task(&record) else {
        return;
    };
    if matches!(cause.severity(), crate::host::notify::Severity::Info) {
        return;
    }
    let reported = gui.notifications.report(cause);
    if reported.first_time {
        refresh_notifications(gui);
    }
}

/// Polls the submitted build until it reports.
fn schedule_build_poll(gui: &Arc<Mutex<Gui>>) {
    let next = Arc::clone(gui);
    let _ = rust_widgets::schedule_on_main(Duration::from_millis(300), move || {
        let mut live = false;
        let mut updates = Vec::new();
        if let Ok(mut gui) = next.lock() {
            while let Some(update) = gui.build.poll() {
                updates.push(update);
            }
            live = gui.build.is_live();
            for update in updates {
                apply_build_update(&mut gui, update);
            }
        }
        if live {
            schedule_build_poll(&next);
        }
    });
}

/// Fills the Application-output panel with the last report.
fn refresh_run(gui: &mut Gui) {
    let list = gui.run_list;
    set_list_rows(list, gui.run.rows());
}

/// Mirrors one application-output report into the list and the status bar.
fn apply_run_update(gui: &mut Gui, update: crate::app::run::workbench_run::Update) {
    gui.message = update.status.clone();
    let rows = update.rows.clone();
    let list = gui.run_list;
    set_list_rows(list, rows.clone());
    update_status(gui);
    crate::host::log::emit(
        crate::host::log::Level::Info,
        "app::run::workbench-run",
        &format!("run: {}", update.status),
    );
    for row in &rows {
        crate::host::log::emit(
            crate::host::log::Level::Info,
            "app::run::workbench-run",
            &format!("run row: {row}"),
        );
    }
}

/// Polls the running program until it ends or the panel asks it to stop.
fn schedule_run_poll(gui: &Arc<Mutex<Gui>>) {
    let next = Arc::clone(gui);
    let _ = rust_widgets::schedule_on_main(Duration::from_millis(200), move || {
        let mut live = false;
        let mut updates = Vec::new();
        if let Ok(mut gui) = next.lock() {
            while let Some(update) = gui.run.poll() {
                updates.push(update);
            }
            live = gui.run.is_live();
            for update in updates {
                apply_run_update(&mut gui, update);
            }
        }
        if live {
            schedule_run_poll(&next);
        }
    });
}

/// Fills the Terminal panel with the last report.
fn refresh_terminal(gui: &mut Gui) {
    let list = gui.terminal_list;
    set_list_rows(list, gui.terminal.rows());
}

/// Mirrors one terminal report into the list and the status bar.
fn apply_terminal_update(gui: &mut Gui, update: crate::app::run::workbench_terminal::Update) {
    gui.message = update.status.clone();
    let rows = update.rows.clone();
    let list = gui.terminal_list;
    set_list_rows(list, rows.clone());
    update_status(gui);
    crate::host::log::emit(
        crate::host::log::Level::Info,
        "app::run::workbench-terminal",
        &format!("terminal: {}", update.status),
    );
    for row in &rows {
        crate::host::log::emit(
            crate::host::log::Level::Info,
            "app::run::workbench-terminal",
            &format!("terminal row: {row}"),
        );
    }
}

/// Polls the terminal's worker while a session is connected.
fn schedule_terminal_poll(gui: &Arc<Mutex<Gui>>) {
    let next = Arc::clone(gui);
    let _ = rust_widgets::schedule_on_main(Duration::from_millis(200), move || {
        let mut live = false;
        let mut updates = Vec::new();
        if let Ok(mut gui) = next.lock() {
            while let Some(update) = gui.terminal.poll() {
                updates.push(update);
            }
            live = gui.terminal.is_live();
            for update in updates {
                apply_terminal_update(&mut gui, update);
            }
        }
        if live {
            schedule_terminal_poll(&next);
        }
    });
}

// The preview service is not `Send` — it owns a `ViewEngine` whose mounted document is `Rc` — so it
// lives on the main thread. The window's shared state holds only its dock slot and menu.
thread_local! {
    static PREVIEW: std::cell::RefCell<crate::app::preview::preview_service::Preview> =
        std::cell::RefCell::new(crate::app::preview::preview_service::Preview::new());
}

/// Runs `f` with the main thread's preview service.
fn with_preview<R>(f: impl FnOnce(&mut crate::app::preview::preview_service::Preview) -> R) -> R {
    PREVIEW.with(|preview| f(&mut preview.borrow_mut()))
}

/// Fills the Preview panel from the service the window holds.
fn refresh_preview(gui: &mut Gui) {
    let list = gui.preview_list;
    let rows = with_preview(|preview| crate::app::run::workbench_preview::rows(preview));
    set_list_rows(list, rows);
}

/// Fills the Hot-update panel from the updater's queue and history.
fn refresh_hot(gui: &mut Gui) {
    let list = gui.hot_list;
    set_list_rows(list, crate::app::run::workbench_hot::rows(&gui.hot));
}

/// Fills the States panel from the opened project's first page document.
///
/// The page's `states` member is the only source; a project without a page, or a page without a
/// machine, is reported as itself rather than replaced by an invented graph.
/// Why the startup page could not be read, so each panel can name the reason with its own key.
enum PageRead {
    NoProject,
    Unreadable,
    NoPage,
    Failed(String),
}

/// The page the States and Collection-contents panels read.
///
/// The startup page is the one the project says the application starts with; without one, the first
/// declared page is the only honest choice, and the returned source names it.
fn read_startup_page(
    gui: &Gui,
) -> Result<(String, crate::project::record::page::PageDocument), PageRead> {
    let Some(project) = gui.project.as_ref() else {
        return Err(PageRead::NoProject);
    };
    let Some(document) = project.file().document() else {
        return Err(PageRead::Unreadable);
    };
    let relative = match document.startup_written() {
        Some(written) => written.to_owned(),
        None => match document.pages().first() {
            Some(page) => page.document().written().to_owned(),
            None => return Err(PageRead::NoPage),
        },
    };
    match crate::project::record::page::PageDocument::read(project.root(), &relative) {
        Ok(page) => Ok((relative, page)),
        Err(error) => Err(PageRead::Failed(format!("{relative}: {}", error.message()))),
    }
}

/// Fills the States panel from the opened project's startup page.
fn refresh_states(gui: &mut Gui) {
    let list = gui.states_list;
    match read_startup_page(gui) {
        Ok((source, page)) => {
            let states = page.states().cloned();
            gui.states_source = Some(source.clone());
            gui.states = states.clone();
            let rows = crate::app::run::workbench_states::rows(&source, states.as_ref());
            set_list_rows(list, rows);
        }
        Err(PageRead::NoProject) => {
            gui.states_source = None;
            gui.states = None;
            set_list_rows(list, vec![tr("ui.states.no-project")]);
        }
        Err(PageRead::Unreadable) => {
            gui.states_source = None;
            gui.states = None;
            set_list_rows(list, vec![tr("ui.states.unreadable")]);
        }
        Err(PageRead::NoPage) => {
            gui.states_source = None;
            gui.states = None;
            set_list_rows(list, vec![tr("ui.states.no-page")]);
        }
        Err(PageRead::Failed(reason)) => {
            gui.states_source = None;
            gui.states = None;
            set_list_rows(
                list,
                vec![tr_args("ui.states.unreadable-at", &[("reason", &reason)])],
            );
        }
    }
}

/// Fills the Collection-contents panel from the opened project's startup page.
fn refresh_items(gui: &mut Gui) {
    let list = gui.items_list;
    match read_startup_page(gui) {
        Ok((source, page)) => {
            let rows = crate::app::run::workbench_items::rows(&source, page.collections());
            set_list_rows(list, rows);
        }
        Err(PageRead::NoProject) => set_list_rows(list, vec![tr("ui.items.no-project")]),
        Err(PageRead::Unreadable) => set_list_rows(list, vec![tr("ui.items.unreadable")]),
        Err(PageRead::NoPage) => set_list_rows(list, vec![tr("ui.items.no-page")]),
        Err(PageRead::Failed(reason)) => set_list_rows(
            list,
            vec![tr_args(
                "ui.components.unreadable-at",
                &[("reason", &reason)],
            )],
        ),
    }
}

/// Fills the Timeline panel from the opened project's startup page.
fn refresh_timeline(gui: &mut Gui) {
    let list = gui.timeline_list;
    match read_startup_page(gui) {
        Ok((source, page)) => {
            let rows = crate::app::run::workbench_timeline::rows(&source, page.motions());
            set_list_rows(list, rows);
        }
        Err(PageRead::NoProject) => set_list_rows(list, vec![tr("ui.timeline.no-project")]),
        Err(PageRead::Unreadable) => set_list_rows(list, vec![tr("ui.timeline.unreadable")]),
        Err(PageRead::NoPage) => set_list_rows(list, vec![tr("ui.timeline.no-page")]),
        Err(PageRead::Failed(reason)) => set_list_rows(
            list,
            vec![tr_args("ui.timeline.unreadable-at", &[("reason", &reason)])],
        ),
    }
}

/// Fills the AI-session panel from the journal named on the command line.
fn refresh_ai(gui: &mut Gui) {
    let list = gui.ai_list;
    let rows = match gui.ai_journal.as_deref() {
        Some(path) => crate::app::run::workbench_ai::rows(path),
        None => vec![tr("ui.ai.no-journal")],
    };
    set_list_rows(list, rows);
}

/// The document the focused editor group shows, with the buffer text it shows.
fn active_document(gui: &Gui) -> Option<(String, String)> {
    let tabs = gui.groups.tabs(gui.groups.focus())?;
    let document = tabs.active()?.document().to_owned();
    let text = gui.groups.text(&document)?.to_owned();
    Some((document, text))
}

/// Fills the source-outline panel.
///
/// Without `--lsp` no server is started and no document is claimed: the panel's own rows say that a
/// server has to be configured.
///
/// With one, the subject is the project's **generated artifact** when the startup page generates:
/// that is the text the per-node anchors describe and the text whose generated/handwritten regions
/// are interesting. When no generation is available (no project, a page the generator refuses, or a
/// blocked precheck) the panel falls back to the focused editor's buffer and reports that the
/// association is not available.
fn refresh_source(gui: &mut Gui) {
    let list = gui.source_list;
    if !gui.source.is_configured() {
        // `render` says so itself; no server is started and no document is claimed.
        let filter = gui.source_filter.clone();
        let rows = gui.source.render(&filter);
        set_list_rows(list, rows);
        return;
    }
    let root = gui
        .project
        .as_ref()
        .map(|project| project.root().as_path().to_path_buf())
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    let generated = generated_source(gui);
    match &generated {
        Some((page, outcome)) => {
            let document = crate::app::generate::artifact::dual_outputs()[0].clone();
            gui.source.refresh(
                &root,
                &document,
                &outcome.source,
                Some(crate::app::run::workbench_outline::Generated {
                    page,
                    source: &outcome.source,
                    anchors: &outcome.report.node_anchors,
                }),
            );
        }
        None => {
            let Some((buffer_document, buffer_text)) = active_document(gui) else {
                set_list_rows(list, vec![tr("ui.source.no-document")]);
                return;
            };
            gui.source
                .refresh(&root, &buffer_document, &buffer_text, None);
        }
    }
    let filter = gui.source_filter.clone();
    let rows = gui.source.render(&filter);
    set_list_rows(list, rows);
}

/// Fills the layout-inspector panel from the opened project's startup page.
///
/// The declaration is read from the page document itself (format v6's root `layout` object); the
/// diagnostics and the solved rectangles come from the generation run's own report, so a parameter
/// the library does not honour is named by the library and the canvas reads the same geometry as
/// the generated source (RW-111/RW-116). Without a run the panel says "not checked" rather than
/// "none".
fn refresh_layout_panel(gui: &mut Gui) {
    let list = gui.layout_list;
    let (source, page) = match read_startup_page(gui) {
        Ok(found) => found,
        Err(PageRead::NoProject) => {
            set_list_rows(list, vec![tr("ui.layout.no-project")]);
            return;
        }
        Err(PageRead::Unreadable) => {
            set_list_rows(list, vec![tr("ui.layout.unreadable")]);
            return;
        }
        Err(PageRead::NoPage) => {
            set_list_rows(list, vec![tr("ui.layout.no-page")]);
            return;
        }
        Err(PageRead::Failed(reason)) => {
            set_list_rows(
                list,
                vec![tr_args("ui.layout.unreadable-at", &[("reason", &reason)])],
            );
            return;
        }
    };
    let Some(root) = page.root_node() else {
        set_list_rows(list, vec![tr("ui.layout.no-page")]);
        return;
    };
    let declaration = crate::app::run::workbench_layout_panel::Declaration {
        node: root.id().get(),
        kind: root.kind().as_str(),
        layout: root.layout(),
        children: root.children().len(),
        constraints: page
            .nodes()
            .iter()
            .filter_map(|node| {
                node.constraints()
                    .map(|constraints| (node.id().get(), constraints))
            })
            .collect(),
    };
    // The issues and the solved rectangles belong to a generation run; without one the panel must
    // not claim a clean document, and it must not present a geometry it did not receive.
    let generated = generated_source(gui);
    let issues = generated
        .as_ref()
        .map(|(_, outcome)| outcome.report.layout_issues.as_slice());
    let geometry =
        generated.as_ref().map(
            |(_, outcome)| crate::app::run::workbench_layout_panel::Geometry {
                width: WIDTH,
                height: HEIGHT,
                rects: &outcome.report.node_rects,
                gaps: &outcome.report.layout_geometry_gaps,
            },
        );
    let rows =
        crate::app::run::workbench_layout_panel::rows(&source, &declaration, issues, geometry);
    set_list_rows(list, rows);
}

/// The startup page's fresh generation, for the source outline's node mapping.
///
/// A missing project, a page the generator refuses (the v2–v5 members it does not consume), or a
/// blocked precheck all answer `None`; the panel then says the association is not available rather
/// than presenting a map that belongs to some other document. Generation is in memory only — this
/// starts no build and writes no file.
fn generated_source(gui: &Gui) -> Option<(String, crate::app::generate::pipeline::Outcome)> {
    use crate::app::generate::pipeline::{self, Input};
    use rust_widgets::designer::TargetProfile;

    let (page, document) = read_startup_page(gui).ok()?;
    let root = gui.project.as_ref()?.root().as_path().to_path_buf();
    let path = root.join(&page);
    let json = crate::app::run::generate::read_document(&path.to_string_lossy()).ok()?;
    let nodes = document
        .nodes()
        .iter()
        .map(|node| pipeline::Node {
            kind: node.kind().as_str().to_owned(),
            children: node.children().len(),
        })
        .collect();
    let input = Input {
        json,
        target: TargetProfile::Default,
        width: WIDTH,
        height: HEIGHT,
        function_name: "build_ui".to_owned(),
        nodes,
        style: Vec::new(),
        layout: Vec::new(),
    };
    pipeline::run(&input).ok().map(|outcome| (page, outcome))
}

/// Moves the editor's caret to the symbol the activated source-outline row names.
fn jump_to_source(gui: &mut Gui, index: usize) {
    let Some((line, column)) = gui.source.target(index) else {
        return;
    };
    jump_to_source_position(gui, line, column);
}

/// Moves the caret to `line`/`column` in the document the source outline last read.
///
/// The outline was read from the focused group's buffer or from the generated artifact, so the
/// caret goes into that group. A document without a tab there is opened first; the generated
/// artifact has no file on disk in this session, so its text is the one the panel read. An already
/// open document is never overwritten, because its buffer may hold unsaved edits.
fn jump_to_source_position(gui: &mut Gui, line: usize, column: usize) -> bool {
    let Some(document) = gui.source.document().map(str::to_owned) else {
        return false;
    };
    let text = gui.source.document_text().map(str::to_owned);
    if !show_document_at(gui, &document, text.as_deref(), line, column, true) {
        gui.message = tr("ui.source.no-editor");
        update_status(gui);
        return false;
    }
    gui.message = tr_args("ui.source.jumped", &[("line", &(line + 1).to_string())]);
    update_status(gui);
    true
}

/// Shows `document` in the focused group and moves its caret to `line`/`column`.
///
/// `text` is written only when the document is not open yet: a jump must not replace a buffer that
/// may hold unsaved edits. The caret call is the editor control's own, so the outline and the editor
/// share one notion of a position.
fn show_document_at(
    gui: &mut Gui,
    document: &str,
    text: Option<&str>,
    line: usize,
    column: usize,
    record: bool,
) -> bool {
    if record {
        record_jump(gui);
    }
    let group = gui.groups.focus();
    let already_open = gui
        .groups
        .tabs(group)
        .is_some_and(|tabs| tabs.is_open(document));
    gui.groups.pin(document);
    if !already_open
        && let Some(text) = text
        && let Err(reason) = gui.groups.type_into(text)
    {
        warn_failure("the symbol's document could not be opened", reason);
        return false;
    }
    gui.sync_editors();
    let editor = gui
        .groups
        .group_ids()
        .iter()
        .position(|id| *id == group)
        .and_then(|position| gui.group_views.get(position))
        .map(|view| view.editor);
    let Some(editor) = editor else {
        return false;
    };
    with_widget_mut(editor, move |widget| {
        let Some(editor) = (widget as &mut dyn Any).downcast_mut::<CodeEditor>() else {
            return false;
        };
        editor.set_cursor(line, column, false);
        true
    }) == Some(true)
}

/// The caret's current document and position, when the focused group has an editor.
fn current_location(gui: &Gui) -> Option<JumpLocation> {
    let (document, _) = active_document(gui)?;
    let (line, column) = edit_with_editor(gui, |editor| editor.cursor())?;
    Some(JumpLocation {
        document,
        line,
        column,
    })
}

/// Records the caret's position before a jump, and drops the forward stack a new jump invalidates.
fn record_jump(gui: &mut Gui) {
    if let Some(location) = current_location(gui) {
        gui.jump_back.push(location);
        gui.jump_forward.clear();
    }
}

/// Shows a remembered location, reading its file when the tab is not open.
///
/// History replay does not record itself: the caller moves the opposite stack so back/forward walk
/// one list rather than growing it.
fn restore_location(gui: &mut Gui, location: &JumpLocation) -> Result<(), String> {
    let already_open = gui
        .groups
        .tabs(gui.groups.focus())
        .is_some_and(|tabs| tabs.is_open(&location.document));
    let text = if already_open {
        None
    } else {
        let root = project_root(gui).ok_or_else(|| tr("ui.project.none"))?;
        let path = if Path::new(&location.document).is_absolute() {
            PathBuf::from(&location.document)
        } else {
            root.join(&location.document)
        };
        Some(std::fs::read_to_string(&path).map_err(|error| error.to_string())?)
    };
    if let Some(text) = &text {
        remember_saved_text(gui, &location.document, text);
    }
    if !show_document_at(
        gui,
        &location.document,
        text.as_deref(),
        location.line,
        location.column,
        false,
    ) {
        return Err(tr("ui.source.no-editor"));
    }
    Ok(())
}

/// Runs one code-navigation query and moves to the first location inside the project.
///
/// Every target the server reported is written to the Messages panel, so a multi-target answer is
/// not reduced to the one this build jumped to. The first target whose URI is under the project
/// root is the one opened; a result entirely outside the project is refused with that reason.
fn code_navigation(gui: &mut Gui, query: crate::app::run::workbench_outline::CodeQuery) -> String {
    use crate::app::run::workbench_outline::SourceOutline;

    if !gui.source.is_configured() {
        return tr("ui.error.navigation-no-server");
    }
    let Some(root) = project_root(gui) else {
        return tr("ui.project.none");
    };
    let Some((document, text)) = active_document(gui) else {
        return tr("ui.status.no-editor");
    };
    let Some((line, column)) = edit_with_editor(gui, |editor| editor.cursor()) else {
        return tr("ui.source.no-editor");
    };
    let locations = match gui
        .source
        .code_locations(&root, &document, &text, line, column, query)
    {
        Ok(locations) => locations,
        Err(reason) => return tr_args("ui.error.navigation-failed", &[("reason", &reason)]),
    };
    if locations.is_empty() {
        return tr("ui.error.navigation-not-found");
    }
    let count = locations.len();
    for location in &locations {
        push_message(
            gui,
            format!(
                "{}:{}:{}",
                location.uri,
                location.line + 1,
                location.character + 1
            ),
        );
    }
    let mut opened = None;
    for location in &locations {
        let Some(path) = crate::host::lsp::path_from_uri(&location.uri) else {
            continue;
        };
        let Ok(relative) = path.strip_prefix(&root) else {
            continue;
        };
        let Ok(file_text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let document = relative.to_string_lossy().into_owned();
        let (line, column) = SourceOutline::position(&file_text, location.line, location.character)
            .unwrap_or((location.line as usize, location.character as usize));
        opened = Some((document, file_text, line, column));
        break;
    }
    let Some((document, file_text, line, column)) = opened else {
        return tr_args(
            "ui.error.navigation-failed",
            &[("reason", "no reported location is inside the project root")],
        );
    };
    if !show_document_at(gui, &document, Some(&file_text), line, column, true) {
        return tr("ui.source.no-editor");
    }
    if count > 1 {
        tr_args(
            "ui.status.navigation-many",
            &[
                ("count", &count.to_string()),
                ("path", &document),
                ("line", &(line + 1).to_string()),
            ],
        )
    } else {
        tr_args(
            "ui.status.navigation-done",
            &[("path", &document), ("line", &(line + 1).to_string())],
        )
    }
}

/// Walks the jump history in one direction.
fn code_history(gui: &mut Gui, forward: bool) -> String {
    let target = if forward {
        gui.jump_forward.pop()
    } else {
        gui.jump_back.pop()
    };
    let Some(target) = target else {
        return tr(if forward {
            "ui.status.no-next-location"
        } else {
            "ui.status.no-previous-location"
        });
    };
    let current = current_location(gui);
    if let Err(reason) = restore_location(gui, &target) {
        // Put the target back so a failed restore does not silently consume the history entry.
        if forward {
            gui.jump_forward.push(target);
        } else {
            gui.jump_back.push(target);
        }
        return tr_args("ui.error.navigation-failed", &[("reason", &reason)]);
    }
    if let Some(current) = current {
        if forward {
            gui.jump_back.push(current);
        } else {
            gui.jump_forward.push(current);
        }
    }
    tr_args(
        if forward {
            "ui.status.moved-forward"
        } else {
            "ui.status.moved-back"
        },
        &[
            ("path", &target.document),
            ("line", &(target.line + 1).to_string()),
        ],
    )
}

/// Fills the Components panel from the opened project's startup page.
fn refresh_components(gui: &mut Gui) {
    let list = gui.components_list;
    match read_startup_page(gui) {
        Ok((source, page)) => {
            let rows = crate::app::run::workbench_components::rows(&source, page.components());
            set_list_rows(list, rows);
        }
        Err(PageRead::NoProject) => set_list_rows(list, vec![tr("ui.components.no-project")]),
        Err(PageRead::Unreadable) => set_list_rows(list, vec![tr("ui.components.unreadable")]),
        Err(PageRead::NoPage) => set_list_rows(list, vec![tr("ui.components.no-page")]),
        Err(PageRead::Failed(reason)) => set_list_rows(
            list,
            vec![tr_args(
                "ui.components.unreadable-at",
                &[("reason", &reason)],
            )],
        ),
    }
}

/// Records an Inspector commit as a hot-update edit and flushes the batch.
///
/// The source is the inspector, not a generated file, so this is not an echo of the process's own
/// write. The updater classifies the edit (`Live` for a property) itself; the panel reads that
/// history, so the rule is not repeated here.
fn record_hot_edit(gui: &mut Gui, nodes: &str) {
    let subject = format!("{} on {nodes}", gui.property.name().as_str());
    record_hot(
        gui,
        "inspector",
        crate::app::preview::hot_update::Edit::Property(subject),
    );
}

/// Records one document edit and flushes the batch that is now complete.
///
/// The source names where the edit came from (the scene, the Inspector, a paste); it is never the
/// updater's own source, so the edit is not mistaken for an echo of this process's write. The
/// updater itself decides whether the batch is `live` or `rebuild` — the caller does not classify.
fn record_hot(gui: &mut Gui, source: &str, edit: crate::app::preview::hot_update::Edit) {
    let _ = gui.hot.push(source, edit);
    if gui.hot.flush().is_some() {
        refresh_hot(gui);
    }
}

/// Mirrors one test-run report into the list and the status bar.
fn apply_tests_update(gui: &mut Gui, update: crate::app::run::workbench_tests::Update) {
    gui.message = update.status.clone();
    let rows = update.rows.clone();
    let list = gui.tests_list;
    set_list_rows(list, rows.clone());
    update_status(gui);
    crate::host::log::emit(
        crate::host::log::Level::Info,
        "app::run::workbench-tests",
        &format!("tests: {}", update.status),
    );
    for row in &rows {
        crate::host::log::emit(
            crate::host::log::Level::Info,
            "app::run::workbench-tests",
            &format!("test row: {row}"),
        );
    }
}

/// Polls the test worker until it reports that it has finished.
fn schedule_tests_poll(gui: &Arc<Mutex<Gui>>) {
    let next = Arc::clone(gui);
    let _ = rust_widgets::schedule_on_main(Duration::from_millis(300), move || {
        let mut live = false;
        let mut updates = Vec::new();
        if let Ok(mut gui) = next.lock() {
            while let Some(update) = gui.tests.poll() {
                updates.push(update);
            }
            live = gui.tests.is_live();
            for update in updates {
                apply_tests_update(&mut gui, update);
            }
        }
        if live {
            schedule_tests_poll(&next);
        }
    });
}

/// The chrome the window built: menus, toolbar, status line and the four context-menu groups.
fn application_rows(gui: &Gui) -> Vec<String> {
    let menus = crate::app::commands::menu::built_in();
    let toolbar: Vec<(String, String)> = gui
        .toolbar_items
        .iter()
        .map(|(key, action)| (tr(key), (*action).to_owned()))
        .collect();
    let status = format!("{} | {}", gui.status_line(), gui.message);
    let context_menus: Vec<(String, Vec<String>)> = [
        ("inspector", INSPECTOR_MENU_KEYS.as_slice()),
        ("scene", SCENE_MENU_KEYS.as_slice()),
        ("editor", EDITOR_MENU_KEYS.as_slice()),
        ("diagnostic", DIAGNOSTIC_MENU_KEYS.as_slice()),
    ]
    .into_iter()
    .map(|(group, keys)| {
        (
            group.to_owned(),
            keys.iter().map(|(key, _)| tr(key)).collect::<Vec<_>>(),
        )
    })
    .collect();
    crate::app::run::workbench_application::rows(&menus, &toolbar, &status, &context_menus)
}

/// Replaces a `ListBox`'s rows without clearing and rebuilding an unchanged list.
///
/// The library publishes no per-row text setter, so a changed row is a remove-and-insert at its
/// index; an equal list is left untouched, which is the part that stops a repaint from rebuilding
/// five panels every turn (blue22 H-5, and the limitation is stated rather than hidden).
fn set_list_rows(list: u64, rows: Vec<String>) {
    if list == 0 {
        return;
    }
    let _ = with_widget_mut(list, move |widget| {
        let Some(list) = (widget as &mut dyn Any).downcast_mut::<ListBox>() else {
            return false;
        };
        // Compare the widget's own slice first: an equal list needs no allocation at all, and a
        // changed one only needs the common prefix. The old code cloned every row before the
        // comparison, which is exactly the per-repaint cost this path exists to avoid.
        let (common, previous_len) = {
            let current = list.items();
            if current == rows.as_slice() {
                return true;
            }
            let common = current
                .iter()
                .zip(&rows)
                .take_while(|(before, after)| before == after)
                .count();
            (common, current.len())
        };
        for index in (common..previous_len).rev() {
            list.remove_item(index);
        }
        for (offset, row) in rows.into_iter().enumerate().skip(common) {
            list.insert_item(offset, row);
        }
        true
    });
}

/// Builds the library row a [`GridRow`] describes.
fn property_item(row: &GridRow) -> PropertyItem {
    let item = if row.mixed {
        PropertyItem::mixed(row.name.clone()).with_placeholder(tr("ui.placeholder.mixed"))
    } else {
        PropertyItem::new(row.name.clone(), row.value.clone(), row.editable)
            .with_kind(row.kind.clone())
    };
    match row.bounds {
        Some((min, max, step)) => item.with_bounds(min, max, step),
        None => item,
    }
}

/// Compares two grid rows by every field the host sets.
///
/// `PropertyItem` is not `PartialEq`, so the comparison is spelled out here rather than derived at
/// the call site; a field added to the host row must be added here too.
fn same_property_item(before: &PropertyItem, after: &PropertyItem) -> bool {
    before.name == after.name
        && before.value == after.value
        && before.editable == after.editable
        && before.mixed == after.mixed
        && before.placeholder == after.placeholder
        && before.kind == after.kind
        && before.category == after.category
        && before.number_bounds == after.number_bounds
}

/// Brings the rows of a `PropertyGrid` up to date, editing only what changed.
///
/// RW-11's incremental row API is what keeps the grid's row-indexed state — selection, hover, an
/// open editor, a custom-editor binding — alive across a refresh. `clear` + re-adding every row
/// dropped all of it whenever the model was re-read, even when only one value had changed.
///
/// `row_error` paints a message on the row it belongs to (RW-12); it is cleared on every refresh
/// so a stale error cannot outlive the state that produced it.
fn set_property_rows(
    id: u64,
    rows: &[GridRow],
    selected: Option<usize>,
    row_error: Option<(usize, String)>,
) {
    if id == 0 {
        return;
    }
    let rows = rows.to_vec();
    let changed = with_widget_mut(id, move |widget| {
        let Some(grid) = (widget as &mut dyn Any).downcast_mut::<PropertyGrid>() else {
            return false;
        };
        let wanted: Vec<PropertyItem> = rows.iter().map(property_item).collect();
        let current: Vec<PropertyItem> = grid.properties().to_vec();
        let shared = current.len().min(wanted.len());
        for index in 0..shared {
            if !same_property_item(&current[index], &wanted[index]) {
                grid.update_property(index, wanted[index].clone());
            }
        }
        if current.len() > wanted.len() {
            for index in (wanted.len()..current.len()).rev() {
                grid.remove_property(index);
            }
        } else {
            for (index, item) in wanted.iter().enumerate().skip(shared) {
                grid.insert_property(index, item.clone());
            }
        }
        grid.clear_row_errors();
        if let Some((index, message)) = row_error {
            grid.set_row_error(index, Some(message));
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

/// Sets or clears one row's error message without touching the rest of the grid (RW-12).
fn set_property_row_error(id: u64, row: usize, message: Option<&str>) {
    if id == 0 {
        return;
    }
    let message = message.map(str::to_owned);
    let _ = with_widget_mut(id, move |widget| {
        let Some(grid) = (widget as &mut dyn Any).downcast_mut::<PropertyGrid>() else {
            return false;
        };
        grid.set_row_error(row, message)
    });
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
    // The host holds raw ids of mixed kinds, and `WidgetHandle` is implemented per kind
    // (`TabWidgetHandle`, `WindowHandle`, …) rather than by one generic handle, so the control's
    // own `focus_gained` signal is reached through the mounted widget here. RW-63's handle-level
    // `on_focus_gained` is the same signal; a typed handle is used where the host already has one.
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

/// The library row for one palette entry (RW-15).
///
/// The enabled state and its reason belong to the entry, not to the title: the control paints a
/// disabled row dimmed and shows the reason where a shortcut would be. Folding the reason into the
/// title made a disabled command look like a normal row whose name happened to end in
/// `— disabled: …`.
fn palette_command_entry(
    entry: &crate::app::commands::palette::Entry,
    shortcut: Option<&str>,
) -> rust_widgets::widget::special_widgets::CommandEntry {
    let row = rust_widgets::widget::special_widgets::CommandEntry::new(entry.id(), entry.title())
        .with_category(entry.category())
        .with_enabled(entry.is_enabled(), entry.reason().map(str::to_owned));
    // RW-16: the control paints the shortcut at the row's trailing edge (and the disabled reason
    // in its place). Only single-key bindings are shown: the window key handler routes one key at
    // a time, so a sequence would name a chord it cannot complete.
    match shortcut {
        Some(chord) => row.with_shortcut(chord),
        None => row,
    }
}

/// The desktop platform this window resolves keys for.
fn window_platform() -> crate::app::commands::shortcuts::Platform {
    if cfg!(target_os = "macos") {
        crate::app::commands::shortcuts::Platform::MacOs
    } else if cfg!(target_os = "windows") {
        crate::app::commands::shortcuts::Platform::Windows
    } else {
        crate::app::commands::shortcuts::Platform::Linux
    }
}

/// The keymap this window resolves keys with, in the platform's own notation.
///
/// One table feeds the key handler, the menu chords, the palette column and the help text, so
/// "what key runs this" cannot answer differently on two surfaces.
fn window_keymap() -> crate::app::commands::shortcuts::Keymap {
    window_platform().keymap()
}

/// The single-key chords this window actually routes, by command id, in the platform's notation.
fn routed_shortcuts() -> std::collections::BTreeMap<String, String> {
    let keymap = window_keymap();
    let mut shortcuts = std::collections::BTreeMap::new();
    for binding in keymap.bindings() {
        // A sequence needs two keypresses and a scoped binding only applies in its own panel; the
        // menu and the palette are global surfaces, so neither belongs in this column.
        if binding.chord().keys().len() != 1
            || !matches!(
                binding.scope(),
                crate::app::commands::shortcuts::Scope::Global
            )
        {
            continue;
        }
        shortcuts
            .entry(binding.command().to_owned())
            .or_insert_with(|| binding.chord().to_string());
    }
    shortcuts
}

/// The command a window keypress should run, or `None` when the key continues to the control.
///
/// The library's route decides *whether* the window sees a key (a bare character, a composing IME
/// and a declined handler never reach the window); this decides what the host keymap does with it.
/// Pure so the decision can be tested without a window: a text field takes a bare key but not a
/// command modifier, IME composition suppresses everything, and a panel-scoped chord matches only
/// while that panel owns the keyboard.
fn command_for_key(
    keymap: &crate::app::commands::shortcuts::Keymap,
    key: crate::app::commands::shortcuts::Key,
    mode: &str,
    panel: Option<String>,
    text_input: bool,
    composing: bool,
) -> Option<String> {
    let mut scene = if text_input {
        crate::app::commands::shortcuts::Scene::text_field(mode)
    } else {
        crate::app::commands::shortcuts::Scene::in_mode(mode, panel)
    };
    if composing {
        scene = scene.composing();
    }
    match keymap.resolve(&[key], &scene) {
        crate::app::commands::shortcuts::Resolution::Triggered { command } => Some(command),
        // A two-key sequence needs the first chord held across keypresses; this handler sees one
        // canonical key at a time and deliberately does not keep a buffer that a text key (which
        // never reaches it) could leave stale.
        crate::app::commands::shortcuts::Resolution::Pending { .. }
        | crate::app::commands::shortcuts::Resolution::Suppressed { .. }
        | crate::app::commands::shortcuts::Resolution::Nothing => None,
    }
}

fn sync_palette(gui: &mut Gui) {
    let surface = gui.palette_surface;
    if surface == 0 {
        return;
    }
    let shortcuts = routed_shortcuts();
    let entries: Vec<rust_widgets::widget::special_widgets::CommandEntry> = gui
        .palette
        .entries()
        .iter()
        .map(|entry| palette_command_entry(entry, shortcuts.get(entry.id()).map(String::as_str)))
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
            // RW-16: `update_entries` keeps the highlighted command highlighted (by id, not by
            // filtered position), which `set_entries` resets. A refresh — a language reload, a
            // trust change, a completed command — must not move the keyboard to another row.
            palette.update_entries(entries);
            if palette.query() != query.as_str() {
                palette.set_query(query);
            }
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
        let items: Vec<String> = gui
            .toolbar_items
            .iter()
            .map(|(key, _action)| tr(key))
            .collect();
        let relabelled = with_widget_mut(gui.toolbar, move |widget| {
            let Some(bar) = (widget as &mut dyn Any).downcast_mut::<ToolBar>() else {
                return false;
            };
            for (index, label) in items.iter().enumerate() {
                if !bar.set_item_text(index, label.as_str()) {
                    return false;
                }
            }
            true
        })
        .unwrap_or(false);
        if !relabelled {
            warn_failure(
                "a toolbar label could not be replaced in place",
                "the mounted control was not a ToolBar with the remembered item count",
            );
        }
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
    if gui.property.count() == 0 {
        // Nothing is selected: one explicit row rather than the last node's rows. The editable row
        // is gone, so the grid has no cell to commit and `apply` refuses through the same model.
        crate::host::log::emit(
            crate::host::log::Level::Info,
            "app::run::workbench-property",
            "inspector rows: no selection",
        );
        let rows: Vec<GridRow> = vec![GridRow::new(
            tr("ui.inspector.node"),
            tr("ui.placeholder.no-selection"),
            false,
        )];
        set_property_rows(gui.property_grid, &rows, None, None);
        if gui.property_error != 0 {
            rust_widgets::set_widget_text(gui.property_error, &tr("ui.placeholder.no-selection"));
        }
        if gui.inspect_note != 0 {
            rust_widgets::set_widget_text(gui.inspect_note, "");
        }
        return;
    }
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
        GridRow::new(tr("ui.inspector.node"), nodes, false),
        GridRow::new(
            tr("ui.inspector.kind"),
            format!("{kind} · {contract}"),
            false,
        ),
        {
            let colour = gui.property.is_colour();
            let mut row = if colour {
                GridRow::color(gui.property.name().as_str(), gui.property.summary())
            } else {
                GridRow::new(gui.property.name().as_str(), gui.property.summary(), true)
            };
            if gui.property.is_mixed() {
                row = GridRow::mixed(gui.property.name().as_str());
                if colour {
                    row.kind = PropertyItemKind::Color;
                }
            }
            if let Some((min, max, step)) = gui.property.number_bounds() {
                row = row.with_bounds(min, max, step);
            }
            row
        },
        GridRow::new(
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
        rows.push(GridRow::new(tr("ui.inspector.default"), default, false));
    }
    let tokens = gui.property.accepted_tokens();
    if !tokens.is_empty() {
        rows.push(GridRow::new(
            tr("ui.inspector.accepted"),
            tokens.join(" | "),
            false,
        ));
    }
    for line in &gui.inspect_extra {
        rows.push(GridRow::new(
            tr("ui.inspector.inspect"),
            line.clone(),
            false,
        ));
    }
    // The row-level error sits on the value row (RW-12) as well as the contract note below: the
    // grid paints it in the value column, where the person is looking.
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
    set_property_rows(
        gui.property_grid,
        &rows,
        Some(gui.property_row),
        note.as_ref().map(|note| (gui.property_row, note.clone())),
    );
    // Contract facts are reported instead of hidden behind one node's schema: an unavailable
    // schema refuses edits, and a selection whose properties disagree is not agreement.
    if gui.property_error != 0
        && let Some(note) = note
    {
        rust_widgets::set_widget_text(gui.property_error, &note);
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
        &format!("inspector rows: {} [contract {contract}]", rows[2].value),
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
        set_list_rows(list, rows);
        let _ = rust_widgets::list_box_set_current_index(list, selected);
    });
}

/// Mirrors the Data model into its property grid and its labels.
///
/// The grid's first row names the dataset and is read-only; every field after it is editable, so
/// committing a cell edits exactly the field whose row it was.
fn sync_data(gui: &mut Gui) {
    let fields = gui.inspect.fields();
    let mut rows: Vec<GridRow> = vec![GridRow::new(
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
        rows.push(GridRow::new(field.clone(), value, true));
    }
    // Keep the schema's diagnostics visible instead of hiding them behind the editable rows.
    for diagnostic in gui.inspect.rows() {
        if diagnostic.starts_with("problem:") || diagnostic.starts_with("sample refused:") {
            rows.push(GridRow::new(tr("ui.data.diagnostic"), diagnostic, false));
        }
    }
    let selected = gui.data_selected.min(fields.len().saturating_sub(1));
    gui.data_selected = selected;
    set_property_rows(gui.data_grid, &rows, Some(selected + 1), None);
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
        set_list_rows(list, rows);
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
/// Wires the colour popup's four controls to the Inspector row it edits.
///
/// The picker's commit is the model write; the well shows the current value; the history records
/// each commit and selecting an entry commits it; the swatch's activated cell commits its colour.
/// A typed hex value still reaches the model through the grid's own text editor (blue22 H-18).
fn wire_colour_editor(gui: &Arc<Mutex<Gui>>) {
    let (picker, history, swatch) = match gui.lock() {
        Ok(gui) => (gui.color_picker, gui.color_history, gui.color_swatch),
        Err(_) => return,
    };
    if picker != 0 {
        let state = Arc::clone(gui);
        let _ = with_widget_mut(picker, move |widget| {
            let Some(picker) = (widget as &mut dyn Any).downcast_mut::<rust_widgets::ColorPicker>()
            else {
                return false;
            };
            let committed = Arc::clone(&state);
            picker.color_committed.connect(move |colour: Arc<Color>| {
                if let Ok(mut gui) = committed.lock() {
                    commit_colour(&mut gui, *colour);
                }
            });
            let canceled = Arc::clone(&state);
            picker.canceled.connect(move || {
                if let Ok(mut gui) = canceled.lock() {
                    close_colour(&mut gui);
                }
            });
            true
        });
    }
    if history != 0 {
        let state = Arc::clone(gui);
        let _ = with_widget_mut(history, move |widget| {
            let Some(history) =
                (widget as &mut dyn Any).downcast_mut::<rust_widgets::ColorHistory>()
            else {
                return false;
            };
            history.color_selected.connect(move |colour: Arc<Color>| {
                if let Ok(mut gui) = state.lock() {
                    commit_colour(&mut gui, *colour);
                }
            });
            true
        });
    }
    if swatch != 0 {
        let state = Arc::clone(gui);
        let _ = with_widget_mut(swatch, move |widget| {
            let Some(swatch) =
                (widget as &mut dyn Any).downcast_mut::<rust_widgets::widget::companions::Swatch>()
            else {
                return false;
            };
            swatch.swatch_activated.connect(move |colour: Arc<Color>| {
                if let Ok(mut gui) = state.lock() {
                    commit_colour(&mut gui, *colour);
                }
            });
            true
        });
    }
}

/// Opens the colour popup for the grid row, seeded with the row's current hex value.
fn open_colour(gui: &mut Gui, row: usize, seed: &str) {
    gui.color_row = row;
    let colour = Color::parse_hex(seed).unwrap_or(Color::WHITE);
    if gui.color_picker != 0 {
        let _ = with_widget_mut(gui.color_picker, move |widget| {
            let Some(picker) = (widget as &mut dyn Any).downcast_mut::<rust_widgets::ColorPicker>()
            else {
                return false;
            };
            picker.set_initial(colour);
            true
        });
    }
    if gui.color_well != 0 {
        let _ = with_widget_mut(gui.color_well, move |widget| {
            let Some(well) = (widget as &mut dyn Any).downcast_mut::<rust_widgets::ColorWell>()
            else {
                return false;
            };
            well.set_color(colour);
            true
        });
    }
    for id in [
        gui.color_panel,
        gui.color_picker,
        gui.color_well,
        gui.color_history,
        gui.color_swatch,
    ] {
        if id != 0 {
            rust_widgets::show_widget(id);
        }
    }
}

/// Applies one committed colour to the Inspector property and records it in the popup.
fn commit_colour(gui: &mut Gui, colour: Color) {
    apply_property(gui, &colour.to_hex_rgb());
    if gui.color_well != 0 {
        let _ = with_widget_mut(gui.color_well, move |widget| {
            let Some(well) = (widget as &mut dyn Any).downcast_mut::<rust_widgets::ColorWell>()
            else {
                return false;
            };
            well.set_color(colour);
            true
        });
    }
    if gui.color_history != 0 {
        let _ = with_widget_mut(gui.color_history, move |widget| {
            let Some(history) =
                (widget as &mut dyn Any).downcast_mut::<rust_widgets::ColorHistory>()
            else {
                return false;
            };
            history.add_color(colour);
            true
        });
    }
    close_colour(gui);
}

/// Hides the colour popup without changing the model.
fn close_colour(gui: &mut Gui) {
    for id in [
        gui.color_panel,
        gui.color_picker,
        gui.color_well,
        gui.color_history,
        gui.color_swatch,
    ] {
        if id != 0 {
            rust_widgets::hide_widget(id);
        }
    }
}

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
            record_hot_edit(gui, &nodes_text);
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
            let text = tr_args("ui.error.not-applied", &[("reason", &reason)]);
            rust_widgets::set_widget_text(gui.property_error, &text);
            // RW-12: the refused value stays in the field and the reason is painted on the row
            // that owns it, rather than only in the note under the grid.
            set_property_row_error(gui.property_grid, gui.property_row, Some(&text));
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
    set_property_row_error(gui.property_grid, gui.property_row, None);
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
fn canvas_point(gui: &Gui, x: i32, y: i32) -> Option<(i32, i32)> {
    if gui.canvas_panel == 0 {
        return None;
    }
    with_widget_mut(gui.canvas_panel, |widget| {
        let transform = gui.canvas_view.transform(widget.geometry());
        let point = transform.inverse_point(Point::new(x, y));
        (point.x, point.y)
    })
}

/// Rebuilds the Messages list from the log lines, keeping the current row when it still exists.
fn sync_messages(gui: &mut Gui) {
    let list = gui.messages_list;
    if list == 0 {
        return;
    }
    let rows = gui.messages.clone();
    let selected = rust_widgets::list_box_current_index(list);
    let _ = rust_widgets::schedule_on_main(Duration::from_millis(0), move || {
        let last = rows.len().saturating_sub(1);
        set_list_rows(list, rows);
        let _ = rust_widgets::list_box_set_current_index(list, selected.unwrap_or(last).min(last));
    });
}

/// Appends one line to the Messages log, keeping the newest 200.
fn push_message(gui: &mut Gui, line: impl Into<String>) {
    gui.messages.push(line.into());
    const LIMIT: usize = 200;
    if gui.messages.len() > LIMIT {
        let excess = gui.messages.len() - LIMIT;
        gui.messages.drain(..excess);
    }
    sync_messages(gui);
}

/// Applies one Messages context-menu action.
fn diagnostic_context_action(gui: &mut Gui, action: &str, label: &str) {
    use crate::host::clipboard::{ClipboardHost, NativeClipboard};
    let text = match action {
        "copy" => gui
            .diagnostic_row
            .and_then(|index| gui.messages.get(index))
            .cloned(),
        "copy-all" => Some(gui.messages.join("\n")),
        "clear" => {
            gui.messages.clear();
            gui.diagnostic_row = None;
            sync_messages(gui);
            gui.message = label.to_owned();
            update_status(gui);
            return;
        }
        _ => None,
    };
    let Some(text) = text else {
        gui.message = tr("ui.status.no-document");
        update_status(gui);
        return;
    };
    match NativeClipboard::new().write_text(&text) {
        crate::host::clipboard::ClipboardWrite::Written => gui.message = label.to_owned(),
        crate::host::clipboard::ClipboardWrite::Refused { reason } => {
            warn_failure("the diagnostics could not be copied", reason);
        }
    }
    update_status(gui);
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
    // The rows are handed to the scheduled rebuild by value: the library has no per-row text
    // setter, so a content change re-adds every row anyway and a cached copy would only cost one
    // extra clone of the whole strip.
    let rows = rows_changed.then(|| gui.scene.rows());
    let syncing = Arc::clone(&gui.outline_syncing);
    let last = rows.as_ref().map_or(0, Vec::len).saturating_sub(1);
    let _ = rust_widgets::schedule_on_main(Duration::from_millis(0), move || {
        if let Some(rows) = rows {
            set_list_rows(list, rows);
        }
        syncing.store(true, Ordering::SeqCst);
        match selected {
            Some(index) => {
                let _ = rust_widgets::list_box_set_current_index(list, index.min(last));
            }
            None => {
                // No selection: clear the list's current row rather than pointing it at the last
                // row, which would make an unselected outline look selected.
                let _ = with_widget_mut(list, |widget| {
                    if let Some(box_) = (widget as &mut dyn Any).downcast_mut::<ListBox>() {
                        box_.clear_selection();
                    }
                    true
                });
            }
        }
        syncing.store(false, Ordering::SeqCst);
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
        *snapshot = Arc::new(CanvasSnapshot {
            nodes,
            marquee,
            view: gui.canvas_view,
        });
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
    let transform = frame.view.transform(rect);
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
        if frame.view.grid {
            draw_grid(canvas, rect, transform);
        }
        if frame.view.rulers {
            draw_rulers(canvas, rect, transform, font);
        }
        for (design, name, hidden, locked, selected) in frame.nodes.iter() {
            if *hidden {
                continue;
            }
            let node = transform.rect(canvas_rect(*design));
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
            canvas.draw_rect(transform.rect(canvas_rect(marquee)), ACCENT);
        }
    });
}

/// Paints the design grid over the control.
fn draw_grid(
    canvas: &mut rust_widgets::widget::special_widgets::Canvas,
    rect: Rect,
    transform: CanvasTransform,
) {
    const STEP: f32 = 16.0;
    const GRID: Color = Color::rgb(0xE6, 0xE6, 0xEA);
    let step = STEP * transform.scale;
    if !(4.0..=200.0).contains(&step) {
        return;
    }
    let right = rect.x + rect.width as i32;
    let bottom = rect.y + rect.height as i32;
    let mut x = transform.offset.x as f32;
    while x < right as f32 {
        if x >= rect.x as f32 {
            canvas.fill_rect(Rect::new(x.round() as i32, rect.y, 1, rect.height), GRID);
        }
        x += step;
    }
    let mut y = transform.offset.y as f32;
    while y < bottom as f32 {
        if y >= rect.y as f32 {
            canvas.fill_rect(Rect::new(rect.x, y.round() as i32, rect.width, 1), GRID);
        }
        y += step;
    }
}

/// Paints rulers along the top and left edges, labelled in design pixels.
fn draw_rulers(
    canvas: &mut rust_widgets::widget::special_widgets::Canvas,
    rect: Rect,
    transform: CanvasTransform,
    font: &rust_widgets::core::Font,
) {
    const RULER: u32 = 18;
    const BACKGROUND: Color = Color::rgb(0xF0, 0xF0, 0xF4);
    canvas.fill_rect(Rect::new(rect.x, rect.y, rect.width, RULER), BACKGROUND);
    canvas.fill_rect(Rect::new(rect.x, rect.y, RULER, rect.height), BACKGROUND);
    let step = 100.0 * transform.scale;
    if step < 12.0 {
        return;
    }
    let mut design = 0u32;
    while design <= DESIGN_WIDTH {
        let x = transform.offset.x + (design as f32 * transform.scale) as i32;
        if x >= rect.x && x < rect.x + rect.width as i32 {
            canvas.fill_rect(Rect::new(x, rect.y, 1, RULER), MUTED_TEXT);
            canvas.draw_text(
                Point::new(x + 2, rect.y + 2),
                &design.to_string(),
                font,
                MUTED_TEXT,
            );
        }
        design += 100;
    }
    let mut design = 0u32;
    while design <= DESIGN_HEIGHT {
        let y = transform.offset.y + (design as f32 * transform.scale) as i32;
        if y >= rect.y && y < rect.y + rect.height as i32 {
            canvas.fill_rect(Rect::new(rect.x, y, RULER, 1), MUTED_TEXT);
        }
        design += 100;
    }
}

/// Applies one scene context-menu action to the primary selection.
///
/// The actions mutate the same `CanvasState` the Outline and canvas clicks use, so a menu entry and
/// a pointer gesture cannot disagree. A hidden node leaves the selection (the canvas no longer
/// draws it); a locked node stays selected but is refused by the design gestures it would block.
fn scene_context_action(gui: &mut Gui, action: &str) {
    // Paste is the one scene action that does not need a node: its target is the nearest container
    // at or above the selection, and with no selection that is the first root container.
    let node = gui.scene.primary();
    if node.is_none() && action != "paste-node" {
        gui.message = tr("ui.status.unknown-action");
        update_status(gui);
        return;
    }
    let node = node.unwrap_or_else(|| NodeId::from_raw(0));
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
        "select-child" => match gui.scene.first_child(node) {
            Some(child) => select_node(gui, child, "context"),
            None => {
                gui.message = tr_args("ui.status.no-child", &[("name", &name)]);
                update_status(gui);
            }
        },
        "select-next-sibling" => match gui.scene.sibling(node, 1) {
            Some(sibling) => select_node(gui, sibling, "context"),
            None => {
                gui.message = tr_args("ui.status.no-sibling", &[("name", &name)]);
                update_status(gui);
            }
        },
        "select-previous-sibling" => match gui.scene.sibling(node, -1) {
            Some(sibling) => select_node(gui, sibling, "context"),
            None => {
                gui.message = tr_args("ui.status.no-sibling", &[("name", &name)]);
                update_status(gui);
            }
        },
        "rename" => open_rename(gui),
        "move-out" => match gui.scene.move_out_of_container(node) {
            Ok(parent) => {
                let parent_name = gui.scene.name(parent).unwrap_or("?").to_owned();
                sync_outline(gui);
                sync_canvas(gui);
                gui.message = tr_args(
                    "ui.status.moved-out",
                    &[("name", &name), ("parent", &parent_name)],
                );
                update_status(gui);
                crate::host::log::emit(
                    crate::host::log::Level::Info,
                    "app::run::workbench-scene",
                    &format!("moved {name} out of {parent_name}"),
                );
            }
            Err(reason) => {
                gui.message = tr_args("ui.error.not-moved-out", &[("reason", &reason)]);
                update_status(gui);
                crate::host::log::emit(
                    crate::host::log::Level::Warn,
                    "app::run::workbench-scene",
                    &format!("node {} was not moved out: {reason}", node.get()),
                );
            }
        },
        "move-into" => match gui.scene.move_into_previous_container(node) {
            Ok(target) => {
                let target_name = gui.scene.name(target).unwrap_or("?").to_owned();
                sync_outline(gui);
                sync_canvas(gui);
                gui.message = tr_args(
                    "ui.status.moved-into",
                    &[("name", &name), ("parent", &target_name)],
                );
                update_status(gui);
                crate::host::log::emit(
                    crate::host::log::Level::Info,
                    "app::run::workbench-scene",
                    &format!("moved {name} into {target_name}"),
                );
            }
            Err(reason) => {
                gui.message = tr_args("ui.error.not-moved-into", &[("reason", &reason)]);
                update_status(gui);
                crate::host::log::emit(
                    crate::host::log::Level::Warn,
                    "app::run::workbench-scene",
                    &format!(
                        "node {} was not moved into a container: {reason}",
                        node.get()
                    ),
                );
            }
        },
        "copy-node" => match gui.scene.copy_subtree(node) {
            Ok(subtree) => {
                let count = subtree.len();
                gui.scene_clipboard = Some(subtree);
                gui.message = tr_args(
                    "ui.status.node-copied",
                    &[("name", &name), ("count", &count.to_string())],
                );
                update_status(gui);
                crate::host::log::emit(
                    crate::host::log::Level::Info,
                    "app::run::workbench-scene",
                    &format!("copied {name} ({count} node(s)) to the scene clipboard"),
                );
            }
            Err(reason) => {
                gui.message = tr_args("ui.error.copy-refused", &[("reason", &reason)]);
                update_status(gui);
            }
        },
        "paste-node" => {
            let Some(subtree) = gui.scene_clipboard.clone() else {
                gui.message = tr("ui.error.nothing-to-paste");
                update_status(gui);
                return;
            };
            let count = subtree.len();
            match gui.scene.paste_clipboard(&subtree) {
                Ok(root) => {
                    let pasted = gui
                        .scene
                        .name(root)
                        .map(str::to_owned)
                        .unwrap_or_else(|| format!("node {}", root.get()));
                    record_hot(
                        gui,
                        "scene",
                        crate::app::preview::hot_update::Edit::AddNode(pasted),
                    );
                    select_node(gui, root, "paste");
                    gui.message = tr_args(
                        "ui.status.node-pasted",
                        &[
                            ("count", &count.to_string()),
                            ("node", &root.get().to_string()),
                        ],
                    );
                    update_status(gui);
                    crate::host::log::emit(
                        crate::host::log::Level::Info,
                        "app::run::workbench-scene",
                        &format!("pasted {count} node(s) as node {}", root.get()),
                    );
                }
                Err(reason) => {
                    gui.message = tr_args("ui.error.paste-refused", &[("reason", &reason)]);
                    update_status(gui);
                    crate::host::log::emit(
                        crate::host::log::Level::Warn,
                        "app::run::workbench-scene",
                        &format!("paste was refused: {reason}"),
                    );
                }
            }
        }
        "wrap-container" => match gui.scene.wrap_in_container(node) {
            Ok(container) => {
                record_hot(
                    gui,
                    "scene",
                    crate::app::preview::hot_update::Edit::AddNode(format!("container for {name}")),
                );
                select_node(gui, container, "context");
                gui.message = tr_args(
                    "ui.status.wrapped",
                    &[("name", &name), ("node", &container.get().to_string())],
                );
                update_status(gui);
                crate::host::log::emit(
                    crate::host::log::Level::Info,
                    "app::run::workbench-scene",
                    &format!("wrapped {name} in container {}", container.get()),
                );
            }
            Err(reason) => {
                gui.message = tr_args("ui.error.not-wrapped", &[("reason", &reason)]);
                update_status(gui);
                crate::host::log::emit(
                    crate::host::log::Level::Warn,
                    "app::run::workbench-scene",
                    &format!("node {} was not wrapped: {reason}", node.get()),
                );
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
        "raise-one" => {
            if gui.scene.raise_one(node) {
                gui.message = tr_args("ui.status.moved-up", &[("name", &name)]);
            } else {
                gui.message = tr_args("ui.status.already-top", &[("name", &name)]);
            }
            sync_canvas(gui);
            update_status(gui);
        }
        "lower-one" => {
            if gui.scene.lower_one(node) {
                gui.message = tr_args("ui.status.moved-down", &[("name", &name)]);
            } else {
                gui.message = tr_args("ui.status.already-bottom", &[("name", &name)]);
            }
            sync_canvas(gui);
            update_status(gui);
        }
        "send-to-back" => {
            if gui.scene.send_to_back(node) {
                gui.message = tr_args("ui.status.sent-to-back", &[("name", &name)]);
            } else {
                gui.message = tr_args("ui.status.already-bottom", &[("name", &name)]);
            }
            sync_canvas(gui);
            update_status(gui);
        }
        "duplicate" => match gui.scene.duplicate(node) {
            Ok(copy) => {
                let copied = gui
                    .scene
                    .name(copy)
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("node {}", copy.get()));
                record_hot(
                    gui,
                    "scene",
                    crate::app::preview::hot_update::Edit::AddNode(copied),
                );
                select_node(gui, copy, "context");
                gui.message = tr_args(
                    "ui.status.duplicated",
                    &[("name", &name), ("node", &copy.get().to_string())],
                );
                update_status(gui);
                crate::host::log::emit(
                    crate::host::log::Level::Info,
                    "app::run::workbench-scene",
                    &format!("duplicated {name} as node {}", copy.get()),
                );
            }
            Err(reason) => {
                gui.message = tr_args("ui.error.not-duplicated", &[("reason", &reason)]);
                update_status(gui);
                crate::host::log::emit(
                    crate::host::log::Level::Warn,
                    "app::run::workbench-scene",
                    &format!("node {} was not duplicated: {reason}", node.get()),
                );
            }
        },
        "delete" => match gui.scene.delete(node) {
            Ok(report) => {
                record_hot(
                    gui,
                    "scene",
                    crate::app::preview::hot_update::Edit::RemoveNode(name.clone()),
                );
                // The deleted node is no longer selected, so the Inspector must stop showing its
                // contract; without this it would keep editing a node the tree no longer holds.
                gui.property.clear();
                retarget_panels(gui);
                refresh_inspector(gui);
                sync_canvas(gui);
                sync_outline(gui);
                gui.message = tr_args("ui.status.deleted", &[("name", &name), ("report", &report)]);
                update_status(gui);
                crate::host::log::emit(
                    crate::host::log::Level::Info,
                    "app::run::workbench-scene",
                    &format!("deleted {name}: {report}"),
                );
            }
            Err(reason) => {
                gui.message = tr_args("ui.error.not-deleted", &[("reason", &reason)]);
                update_status(gui);
                crate::host::log::emit(
                    crate::host::log::Level::Warn,
                    "app::run::workbench-scene",
                    &format!("node {} was not deleted: {reason}", node.get()),
                );
            }
        },
        _ => {}
    }
}

/// Opens the library's input dialog to rename the selected node.
///
/// The dialog is mounted on the modal stack and its signals write [`Gui::rename_slot`]; they never
/// lock the `Gui`, because they run inside the runtime's event dispatch (the smoke path dispatches
/// the accept key while already holding the lock). The poller applies the answer afterwards.
fn open_rename(gui: &mut Gui) {
    let Some(node) = gui.scene.primary() else {
        gui.message = tr("ui.placeholder.no-selection");
        update_status(gui);
        return;
    };
    if gui.rename_dialog != 0 {
        let old = gui.rename_dialog;
        runtime::exit_modal(old);
        runtime::unregister(old);
        gui.rename_dialog = 0;
    }
    let current = gui.scene.name(node).unwrap_or("").to_owned();
    let mut dialog = InputDialog::new(Rect::new(
        (WIDTH as i32 - 360) / 2,
        (HEIGHT as i32 - 160) / 2,
        360,
        160,
    ));
    dialog.set_title(tr("ui.dialog.rename-title"));
    dialog.set_label_text(tr("ui.dialog.rename-label"));
    dialog.set_text_value(current.clone());

    let typed = Arc::new(Mutex::new(current.clone()));
    {
        let typed = Arc::clone(&typed);
        dialog.text_value_changed.connect(move |text| {
            if let Ok(mut slot) = typed.lock() {
                *slot = (*text).clone();
            }
        });
    }
    let slot = Arc::clone(&gui.rename_slot);
    {
        let slot = Arc::clone(&slot);
        let typed = Arc::clone(&typed);
        dialog.accepted.connect(move || {
            let text = typed.lock().map(|text| text.clone()).unwrap_or_default();
            if let Ok(mut pending) = slot.lock() {
                *pending = Some(RenameRequest {
                    node,
                    answer: RenameAnswer::Accepted(text),
                });
            }
        });
    }
    dialog.rejected.connect(move || {
        if let Ok(mut pending) = slot.lock() {
            *pending = Some(RenameRequest {
                node,
                answer: RenameAnswer::Cancelled,
            });
        }
    });

    let Some(id) = runtime::register(Box::new(dialog)) else {
        gui.message = tr_args(
            "ui.error.rename-refused",
            &[("reason", "the rename dialog could not be mounted")],
        );
        update_status(gui);
        return;
    };
    runtime::set_host_window(id, gui.window);
    if !runtime::enter_modal(id) {
        runtime::unregister(id);
        gui.message = tr_args(
            "ui.error.rename-refused",
            &[("reason", "the rename dialog could not take the modal stack")],
        );
        update_status(gui);
        return;
    }
    gui.rename_dialog = id;
    crate::host::log::emit(
        crate::host::log::Level::Info,
        "app::run::workbench-scene",
        &format!("rename dialog opened for node {} ({current:?})", node.get()),
    );
}

/// Applies one rename answer on the main loop, outside any widget borrow.
fn apply_rename_answer(gui: &mut Gui, request: RenameRequest) {
    if gui.rename_dialog != 0 {
        let id = gui.rename_dialog;
        runtime::exit_modal(id);
        runtime::unregister(id);
        gui.rename_dialog = 0;
    }
    let RenameAnswer::Accepted(text) = request.answer else {
        gui.message = tr("ui.status.rename-cancelled");
        update_status(gui);
        return;
    };
    let name = text.trim();
    if name.is_empty() {
        gui.message = tr_args(
            "ui.error.rename-refused",
            &[("reason", "the name cannot be empty")],
        );
        update_status(gui);
        return;
    }
    match gui.scene.rename(request.node, name) {
        Ok(()) => {
            // The Inspector's editor still holds the old name; clearing and reselecting rebuilds it
            // from the new structural name instead of leaving a Reset that restores the old one.
            gui.property.clear();
            select_node(gui, request.node, "rename");
            gui.message = tr_args("ui.status.renamed", &[("name", name)]);
            update_status(gui);
            crate::host::log::emit(
                crate::host::log::Level::Info,
                "app::run::workbench-scene",
                &format!("renamed node {} to {name:?}", request.node.get()),
            );
        }
        Err(reason) => {
            gui.message = tr_args("ui.error.rename-refused", &[("reason", &reason)]);
            update_status(gui);
        }
    }
}

/// Polls the rename slot on the main loop.
fn schedule_rename(gui: Arc<Mutex<Gui>>, slot: Arc<Mutex<Option<RenameRequest>>>) {
    let _ = rust_widgets::schedule_on_main(Duration::from_millis(50), move || {
        // Take the request first and release the slot's lock: the dialog's callbacks may be
        // running with the `Gui` lock held, and locking both in opposite orders would deadlock.
        let request = match slot.lock() {
            Ok(mut pending) => pending.take(),
            Err(_) => None,
        };
        if let Some(request) = request
            && let Ok(mut gui) = gui.lock()
        {
            apply_rename_answer(&mut gui, request);
        }
        schedule_rename(Arc::clone(&gui), Arc::clone(&slot));
    });
}

/// Opens the go-to-line dialog for the focused editor.
fn open_prompt(
    gui: &mut Gui,
    kind: PromptKind,
    title_key: &str,
    label_key: &str,
    require_editor: bool,
) {
    if require_editor && edit_with_editor(gui, |_| ()).is_none() {
        gui.message = tr("ui.status.no-editor");
        update_status(gui);
        return;
    }
    if gui.prompt_dialog != 0 {
        let old = gui.prompt_dialog;
        runtime::exit_modal(old);
        runtime::unregister(old);
        gui.prompt_dialog = 0;
    }
    let mut dialog = InputDialog::new(Rect::new(
        (WIDTH as i32 - 360) / 2,
        (HEIGHT as i32 - 160) / 2,
        360,
        160,
    ));
    dialog.set_title(tr(title_key));
    dialog.set_label_text(tr(label_key));
    dialog.set_text_value(String::new());

    let typed = Arc::new(Mutex::new(String::new()));
    {
        let typed = Arc::clone(&typed);
        dialog.text_value_changed.connect(move |text| {
            if let Ok(mut slot) = typed.lock() {
                *slot = (*text).clone();
            }
        });
    }
    let slot = Arc::clone(&gui.prompt_slot);
    {
        let slot = Arc::clone(&slot);
        let typed = Arc::clone(&typed);
        dialog.accepted.connect(move || {
            let text = typed.lock().map(|text| text.clone()).unwrap_or_default();
            if let Ok(mut pending) = slot.lock() {
                *pending = Some(match kind {
                    PromptKind::Goto => Prompt::Goto(text),
                    PromptKind::OpenFile => Prompt::OpenFile(text),
                    PromptKind::FileSymbol => Prompt::FileSymbol(text),
                    PromptKind::WorkspaceSymbol => Prompt::WorkspaceSymbol(text),
                    PromptKind::SaveAs => Prompt::SaveAs(text),
                    PromptKind::SaveCopy => Prompt::SaveCopy(text),
                    PromptKind::CleanResources => Prompt::CleanResources(text),
                });
            }
        });
    }
    dialog.rejected.connect(move || {
        if let Ok(mut pending) = slot.lock() {
            *pending = Some(Prompt::Cancelled(kind));
        }
    });

    let Some(id) = runtime::register(Box::new(dialog)) else {
        gui.message = tr_args(
            "ui.error.goto-refused",
            &[("reason", "the dialog could not be mounted")],
        );
        update_status(gui);
        return;
    };
    runtime::set_host_window(id, gui.window);
    if !runtime::enter_modal(id) {
        runtime::unregister(id);
        gui.message = tr_args(
            "ui.error.goto-refused",
            &[(
                "reason",
                "the go-to-line dialog could not take the modal stack",
            )],
        );
        update_status(gui);
        return;
    }
    gui.prompt_dialog = id;
}

/// Polls the prompt slot on the main loop.
fn schedule_prompt(gui: Arc<Mutex<Gui>>, slot: Arc<Mutex<Option<Prompt>>>) {
    let _ = rust_widgets::schedule_on_main(Duration::from_millis(50), move || {
        // Take the answer before locking the `Gui`, the same order the rename poll uses: a dialog
        // callback may hold the slot while the main loop holds the window state.
        let answer = match slot.lock() {
            Ok(mut pending) => pending.take(),
            Err(_) => None,
        };
        if let Some(answer) = answer
            && let Ok(mut state) = gui.lock()
        {
            apply_prompt(&mut state, answer);
        }
        schedule_prompt(gui, slot);
    });
}

/// Applies one go-to-line answer on the main loop, outside any widget borrow.
fn apply_prompt(gui: &mut Gui, answer: Prompt) {
    if gui.prompt_dialog != 0 {
        let id = gui.prompt_dialog;
        runtime::exit_modal(id);
        runtime::unregister(id);
        gui.prompt_dialog = 0;
    }
    match answer {
        Prompt::Goto(text) => apply_goto(gui, &text),
        Prompt::OpenFile(text) => apply_open_file(gui, &text),
        Prompt::FileSymbol(text) => apply_file_symbol(gui, &text),
        Prompt::WorkspaceSymbol(text) => apply_workspace_symbol(gui, &text),
        Prompt::SaveAs(text) => apply_save_as(gui, &text),
        Prompt::SaveCopy(text) => apply_save_copy(gui, &text),
        Prompt::CleanResources(text) => apply_clean_resources(gui, &text),
        Prompt::Cancelled(kind) => {
            gui.message = tr(match kind {
                PromptKind::Goto => "ui.status.goto-cancelled",
                PromptKind::OpenFile => "ui.status.open-cancelled",
                PromptKind::SaveAs | PromptKind::SaveCopy => "ui.status.save-cancelled",
                PromptKind::CleanResources => "ui.status.resources-clean-cancelled",
                PromptKind::FileSymbol | PromptKind::WorkspaceSymbol => {
                    "ui.status.symbol-cancelled"
                }
            });
            update_status(gui);
        }
    }
}

/// Moves the caret for a go-to-line answer.
fn apply_goto(gui: &mut Gui, text: &str) {
    let (line, column) = match parse_goto(text) {
        Ok(parsed) => parsed,
        Err(reason) => {
            gui.message = tr_args("ui.error.goto-refused", &[("reason", reason)]);
            update_status(gui);
            return;
        }
    };
    let applied = edit_with_editor(gui, |editor| {
        match column {
            Some(column) => editor.set_cursor(line - 1, column - 1, false),
            None => editor.goto_line(line - 1),
        }
        true
    });
    if applied.is_none() {
        gui.message = tr("ui.status.no-editor");
        update_status(gui);
        return;
    }
    gui.message = tr_args(
        "ui.status.goto-done",
        &[
            ("line", &line.to_string()),
            ("column", &column.unwrap_or(1).to_string()),
        ],
    );
    update_status(gui);
}

/// Records the text a document had on disk, for later external-change checks.
fn remember_saved_text(gui: &mut Gui, document: &str, text: &str) {
    gui.saved_texts.insert(document.to_owned(), text.to_owned());
}

/// Opens the project file a quick-open answer names, or the best fuzzy match.
fn apply_open_file(gui: &mut Gui, text: &str) {
    let Some(session) = gui.project.as_ref() else {
        gui.message = tr("ui.project.none");
        update_status(gui);
        return;
    };
    let entries: Vec<String> = session
        .file()
        .document()
        .map(|document| {
            document
                .references()
                .into_iter()
                .map(|reference| reference.written().to_owned())
                .collect()
        })
        .unwrap_or_default();
    let Some(relative) = resolve_open_path(&entries, text) else {
        gui.message = tr_args(
            "ui.error.file-not-opened",
            &[("reason", "no project file matches that input")],
        );
        update_status(gui);
        return;
    };
    let root = session.root().as_path().to_path_buf();
    let path = root.join(&relative);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) => {
            gui.message = tr_args(
                "ui.error.file-not-opened",
                &[("reason", &error.to_string())],
            );
            update_status(gui);
            return;
        }
    };
    remember_saved_text(gui, &relative, &text);
    gui.groups.pin(relative.clone());
    if let Err(reason) = gui.groups.type_into(text) {
        gui.message = tr_args("ui.error.file-not-opened", &[("reason", &reason)]);
        update_status(gui);
        return;
    }
    gui.sync_editors();
    gui.message = tr_args("ui.status.file-opened", &[("path", &relative)]);
    update_status(gui);
}

/// Jumps to a symbol in the focused editor's document, read through the language server.
///
/// The source panel normally outlines the generated artifact; a *file* symbol is about the buffer
/// the person is editing, so this reads that buffer explicitly rather than the panel's subject.
fn apply_file_symbol(gui: &mut Gui, text: &str) {
    let query = text.trim();
    if !gui.source.is_configured() {
        gui.message = tr("ui.error.symbol-no-server");
        update_status(gui);
        return;
    }
    if let Err(reason) = refresh_active_source(gui) {
        gui.message = tr_args("ui.error.symbol-not-jumped", &[("reason", &reason)]);
        update_status(gui);
        return;
    }
    match gui.source.file_symbol(query) {
        Some((name, line, column)) => {
            if jump_to_source_position(gui, line, column) {
                gui.message = tr_args(
                    "ui.status.file-symbol-jumped",
                    &[("name", &name), ("line", &(line + 1).to_string())],
                );
                update_status(gui);
            }
        }
        None => {
            gui.message = match gui.source.failure() {
                Some(reason) => tr_args("ui.error.symbol-not-jumped", &[("reason", reason)]),
                None => tr_args("ui.error.symbol-not-found", &[("query", query)]),
            };
            update_status(gui);
        }
    }
}

/// Reads the focused editor's buffer into the source outline for a file-symbol query.
fn refresh_active_source(gui: &mut Gui) -> Result<(), String> {
    let Some((document, text)) = active_document(gui) else {
        return Err(tr("ui.status.no-editor"));
    };
    let root = project_root(gui).unwrap_or_else(|| PathBuf::from("."));
    gui.source.refresh(&root, &document, &text, None);
    Ok(())
}

/// Jumps to the best workspace symbol the language server reports for `text`.
fn apply_workspace_symbol(gui: &mut Gui, text: &str) {
    let query = text.trim();
    if !gui.source.is_configured() {
        gui.message = tr("ui.error.symbol-no-server");
        update_status(gui);
        return;
    }
    let Some(root) = project_root(gui) else {
        gui.message = tr("ui.project.none");
        update_status(gui);
        return;
    };
    let symbols = match gui.source.workspace_symbols(&root, query) {
        Ok(symbols) => symbols,
        Err(reason) => {
            gui.message = tr_args("ui.error.symbol-not-jumped", &[("reason", &reason)]);
            update_status(gui);
            return;
        }
    };
    let Some(symbol) = best_workspace_symbol(&symbols, query) else {
        gui.message = tr_args("ui.error.symbol-not-found", &[("query", query)]);
        update_status(gui);
        return;
    };
    let Some(path) = crate::host::lsp::path_from_uri(&symbol.uri) else {
        gui.message = tr_args(
            "ui.error.symbol-not-jumped",
            &[("reason", "the symbol is not in a local file")],
        );
        update_status(gui);
        return;
    };
    let Ok(relative) = path.strip_prefix(&root) else {
        gui.message = tr_args(
            "ui.error.symbol-not-jumped",
            &[("reason", "the symbol is outside the project root")],
        );
        update_status(gui);
        return;
    };
    let document = relative.to_string_lossy().into_owned();
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) => {
            gui.message = tr_args(
                "ui.error.symbol-not-jumped",
                &[("reason", &error.to_string())],
            );
            update_status(gui);
            return;
        }
    };
    let (line, column) = crate::app::run::workbench_outline::SourceOutline::position(
        &text,
        symbol.line,
        symbol.character,
    )
    .unwrap_or((symbol.line as usize, symbol.character as usize));
    remember_saved_text(gui, &document, &text);
    if !show_document_at(gui, &document, Some(&text), line, column, true) {
        gui.message = tr("ui.source.no-editor");
        update_status(gui);
        return;
    }
    gui.message = tr_args(
        "ui.status.workspace-symbol-jumped",
        &[
            ("name", &symbol.name),
            ("path", &document),
            ("line", &(line + 1).to_string()),
        ],
    );
    update_status(gui);
}

/// The exact workspace symbol `query` names, or the best fuzzy match among the server's answer.
fn best_workspace_symbol<'a>(
    symbols: &'a [crate::app::run::workbench_outline::WorkspaceSymbol],
    query: &str,
) -> Option<&'a crate::app::run::workbench_outline::WorkspaceSymbol> {
    use crate::app::search::quick_open::fuzzy_score;

    symbols
        .iter()
        .find(|symbol| symbol.name == query)
        .or_else(|| {
            symbols
                .iter()
                .filter_map(|symbol| fuzzy_score(query, &symbol.name).map(|score| (score, symbol)))
                .max_by_key(|(score, _)| *score)
                .map(|(_, symbol)| symbol)
        })
}

/// The open project's root, or `None` when no project is open.
fn project_root(gui: &Gui) -> Option<PathBuf> {
    gui.project
        .as_ref()
        .map(|project| project.root().as_path().to_path_buf())
}

/// The exact project file a query names, or the best fuzzy match among the declared files.
///
/// The candidates come from the project document's own references, so a query cannot open a path
/// the project does not declare.
fn resolve_open_path(entries: &[String], query: &str) -> Option<String> {
    use crate::app::search::quick_open::{Candidate, QuickOpen};

    let query = query.trim();
    if query.is_empty() {
        return None;
    }
    if let Some(exact) = entries.iter().find(|entry| entry.as_str() == query) {
        return Some(exact.clone());
    }
    let mut open = QuickOpen::new();
    for entry in entries {
        open.add(Candidate::new(entry.as_str()));
    }
    open.open(query, 1)
        .into_iter()
        .next()
        .and_then(|hit| hit.path)
}

/// Parses `line` or `line:column`, both 1-based, from the go-to-line field.
fn parse_goto(text: &str) -> Result<(usize, Option<usize>), &'static str> {
    let text = text.trim();
    let (line, column) = match text.split_once(':') {
        Some((line, column)) => (line.trim(), Some(column.trim())),
        None => (text, None),
    };
    let line = line
        .parse::<usize>()
        .map_err(|_| "use line or line:column, for example 42 or 42:7")?;
    let column = match column {
        Some(raw) => Some(
            raw.parse::<usize>()
                .map_err(|_| "the column must be a whole number")?,
        ),
        None => None,
    };
    if line == 0 || column == Some(0) {
        return Err("line and column are 1-based");
    }
    Ok((line, column))
}

/// Applies one editor-tab context-menu action to the active tab.
fn editor_context_action(gui: &mut Gui, action: &str, label: &str) {
    let group = gui.groups.focus();
    let active = gui.groups.tabs(group).and_then(|tabs| {
        let current = tabs.active()?;
        tabs.tabs().iter().position(|tab| tab == current)
    });
    match action {
        "close" | "close-others" | "close-right" => {
            let Some(index) = active else {
                gui.message = tr("ui.status.no-document");
                update_status(gui);
                return;
            };
            let Some(tabs) = gui.groups.tabs_mut(group) else {
                return;
            };
            let result = match action {
                "close" => tabs.close_view(index).map(|_| 1),
                "close-others" => tabs.close_others(index),
                _ => tabs.close_to_the_right(index),
            };
            match result {
                Ok(count) => gui.message = format!("{label} ({count})"),
                Err(reason) => warn_failure("an editor tab could not be closed", reason),
            }
            gui.sync_editors();
        }
        "split" => {
            let document = gui
                .groups
                .tabs(group)
                .and_then(|tabs| tabs.active().map(|tab| tab.document().to_owned()));
            gui.groups.split(Axis::LeftRight);
            if let Some(document) = document {
                gui.groups.another_view(document);
            }
            gui.message = label.to_owned();
            gui.sync_editors();
        }
        "next-group" => {
            gui.groups.focus_next();
            gui.message = label.to_owned();
            gui.sync_editors();
        }
        _ => {}
    }
    update_status(gui);
}

/// The row a pointer is over in a list, using the control's own hit test.
fn list_row_at(id: u64, x: i32, y: i32) -> Option<usize> {
    with_widget_mut(id, |widget| {
        (widget as &mut dyn Any)
            .downcast_mut::<ListBox>()
            .and_then(|list| list.item_index_at(Point::new(x, y)))
    })
    .flatten()
}

/// The tab a pointer is over in a tab strip, using the control's own hit test.
fn tab_at(id: u64, x: i32, y: i32) -> Option<usize> {
    with_widget_mut(id, |widget| {
        (widget as &mut dyn Any)
            .downcast_mut::<TabWidget>()
            .and_then(|tabs| tabs.tab_index_at(Point::new(x, y)))
    })
    .flatten()
}

/// Reads an `x,y` signal payload.
fn point_of(payload: &str) -> Option<(i32, i32)> {
    let (x, y) = payload.split_once(',')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
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

/// Inserts the widget-library row at `index` into the scene under the nearest container.
///
/// Activation is the list's `item_activated` (Enter or a double click), not `item_selected`: moving
/// the highlight with the arrow keys is browsing, and browsing must not add a control per keypress.
fn insert_widget_from_palette(gui: &mut Gui, index: usize) {
    let Some(control) = gui.widget_visible.get(index).cloned() else {
        return;
    };
    let (name, property) = {
        let Some(catalog) = gui.widget_catalog.as_ref() else {
            gui.message = tr_args(
                "ui.error.widget-not-inserted",
                &[("reason", "no catalog was captured")],
            );
            update_status(gui);
            return;
        };
        let Some(descriptor) = catalog.get(&control) else {
            gui.message = tr_args(
                "ui.error.widget-not-inserted",
                &[("reason", "the row is not in the captured catalog")],
            );
            update_status(gui);
            return;
        };
        (descriptor.name.clone(), inspector_property(descriptor))
    };
    let Some(property) = property else {
        let text = tr_args("ui.error.widget-no-property", &[("name", &name)]);
        gui.message = text.clone();
        push_message(gui, format!("{control}: {text}"));
        update_status(gui);
        return;
    };
    match gui.scene.insert_widget(&control, &name, &property) {
        Ok(node) => {
            record_hot(
                gui,
                "scene",
                crate::app::preview::hot_update::Edit::AddNode(name.clone()),
            );
            select_node(gui, node, "widgets");
            gui.message = tr_args(
                "ui.status.widget-inserted",
                &[("name", &name), ("node", &node.get().to_string())],
            );
            update_status(gui);
            crate::host::log::emit(
                crate::host::log::Level::Info,
                "app::run::workbench-scene",
                &format!(
                    "inserted {} as node {} from the widget library",
                    control,
                    node.get()
                ),
            );
        }
        Err(reason) => {
            let text = tr_args("ui.error.widget-not-inserted", &[("reason", &reason)]);
            gui.message = text.clone();
            push_message(gui, format!("{control}: {text}"));
            update_status(gui);
            crate::host::log::emit(
                crate::host::log::Level::Warn,
                "app::run::workbench-scene",
                &format!("widget {control} was not inserted: {reason}"),
            );
        }
    }
}

/// Rebuilds the widget list for `query` through the catalog's own search.
///
/// The rows are written on the next main-loop turn: `list_box_clear_items` emits from inside the
/// change, and this function is called from the search field's callback while the `Gui` lock is
/// held.
fn filter_widgets(gui: &mut Gui, query: &str) {
    rebuild_widget_rows(gui, query);
}

/// Recomputes the visible rows and schedules the list write for `query`.
fn rebuild_widget_rows(gui: &mut Gui, query: &str) {
    let Some(catalog) = gui.widget_catalog.as_ref() else {
        return;
    };
    let rows = widget_rows(catalog, query);
    gui.widget_visible = rows.iter().map(|(id, _)| id.clone()).collect();
    gui.widget_query = query.to_owned();
    let labels: Vec<String> = rows.into_iter().map(|(_, label)| label).collect();
    let list = gui.widgets_list;
    if list == 0 {
        return;
    }
    let _ = rust_widgets::schedule_on_main(Duration::from_millis(0), move || {
        set_list_rows(list, labels);
    });
}

/// Finds every occurrence of `query` in one document, in text order.
///
/// The search is case-sensitive and literal: the Search panel is a document viewer's find, not a
/// regex engine, and a byte offset it reports must map back to the same text a click selects.
fn find_hits(
    document: &str,
    text: &str,
    query: &str,
    limit: usize,
    hits: &mut Vec<SearchHit>,
    labels: &mut Vec<String>,
) {
    if query.is_empty() || limit == 0 {
        return;
    }
    for (offset, _) in text.match_indices(query) {
        if hits.len() >= limit {
            break;
        }
        let line_start = text[..offset].rfind('\n').map_or(0, |index| index + 1);
        let line_end = text[offset..]
            .find('\n')
            .map_or(text.len(), |index| offset + index);
        let line_number = text[..offset].matches('\n').count() + 1;
        let preview: String = text[line_start..line_end]
            .trim()
            .chars()
            .take(120)
            .collect();
        hits.push(SearchHit {
            document: document.to_owned(),
            offset: offset as u32,
            length: query.len() as u32,
        });
        labels.push(format!("{document}:{line_number}: {preview}"));
    }
}

/// Recomputes the Search panel's hits for `query` over the documents the editor has open.
fn run_search(gui: &mut Gui, query: &str) {
    let query = query.trim();
    let documents: Vec<String> = gui
        .groups
        .documents()
        .into_iter()
        .map(str::to_owned)
        .collect();
    let mut hits = Vec::new();
    let mut labels = Vec::new();
    if !query.is_empty() {
        for document in &documents {
            if let Some(text) = gui.groups.text(document) {
                find_hits(document, text, query, 200, &mut hits, &mut labels);
            }
            if hits.len() >= 200 {
                break;
            }
        }
    }
    gui.search_hits = hits;
    if gui.search_list != 0 {
        // The field's `text_changed` is emitted synchronously, so the list is written on the next
        // main-loop turn rather than inside this callback (the same rule the widget search follows).
        let list = gui.search_list;
        let _ = rust_widgets::schedule_on_main(Duration::from_millis(0), move || {
            set_list_rows(list, labels);
        });
    }
    gui.message = if query.is_empty() {
        tr("ui.status.ready")
    } else if gui.search_hits.is_empty() {
        tr("ui.search.no-match")
    } else {
        format!("{} match(es)", gui.search_hits.len())
    };
    update_status(gui);
}

/// Opens the document and selects the range a Search result names.
fn activate_search_hit(gui: &mut Gui, index: usize) {
    let Some(hit) = gui.search_hits.get(index).cloned() else {
        return;
    };
    gui.groups.open(&hit.document);
    if let Err(reason) = gui
        .groups
        .select(hit.offset, hit.offset.saturating_add(hit.length))
    {
        warn_failure("a search result could not be selected", reason);
    }
    gui.sync_editors();
    gui.message = tr_args("ui.status.now-on", &[("page", &hit.document)]);
    update_status(gui);
}

/// Toggles the favorite state of the widget row at `index` and returns the new state.
///
/// `persist` is false only for the smoke run: a smoke test must not write the person's favorites
/// file. The catalog is the one source of the row marker; the `Favorites` value is the one source
/// of the file, and both are updated together.
fn toggle_widget_favorite(gui: &mut Gui, index: usize, persist: bool) -> bool {
    let Some(id) = gui.widget_visible.get(index).cloned() else {
        return false;
    };
    let Some(catalog) = gui.widget_catalog.as_mut() else {
        return false;
    };
    let on = !catalog.is_favorite(&id);
    catalog.favorite(&id, on);
    let name = catalog
        .get(&id)
        .map(|descriptor| descriptor.name.clone())
        .unwrap_or_else(|| id.clone());
    gui.widget_favorites.set(&id, on);
    if persist
        && let Some(path) = gui.widget_favorites_path.clone()
        && let Err(reason) = gui.widget_favorites.save(&path)
    {
        warn_failure("the widget favorites could not be saved", reason);
    }
    let query = gui.widget_query.clone();
    rebuild_widget_rows(gui, &query);
    gui.message = tr_args(
        if on {
            "ui.status.favorite-added"
        } else {
            "ui.status.favorite-removed"
        },
        &[("name", &name)],
    );
    update_status(gui);
    crate::host::log::emit(
        crate::host::log::Level::Info,
        "app::run::workbench-widgets",
        &format!("favorite {id}: {on}"),
    );
    on
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
    if gui.ghost != 0 {
        // The ghost layer is above the canvas and owns the release; its signal applies the drop.
        gui.drag_active = false;
        return;
    }
    if !gui.drag_active {
        return;
    }
    gui.drag_active = false;
    gui.drag = None;
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
    gui.property.clear();
    gui.message = if had == 0 {
        "no selectable node".to_owned()
    } else {
        "selection cleared".to_owned()
    };
    retarget_panels(gui);
    refresh_inspector(gui);
    sync_canvas(gui);
    sync_outline(gui);
    update_status(gui);
    crate::host::log::emit(
        crate::host::log::Level::Info,
        "app::run::workbench-scene",
        &format!("canvas click at {:?}: no selectable node", end),
    );
}

/// Starts the library's drag ghost for the node the press picked up (blue22 H-19).
///
/// The layer is a full-window overlay on purpose: it must paint outside the canvas's own clip.
/// It is created once the drag passes the threshold, so an ordinary click never mounts it, and it
/// is unregistered after the drop/cancel so it cannot keep the canvas from receiving pointers.
fn create_drag_ghost(state: &Arc<Mutex<Gui>>, gui: &mut Gui) {
    let Some(node) = gui.drag.as_ref().map(|drag| drag.node()) else {
        return;
    };
    let Some(node_rect) = gui.scene.rect(node) else {
        return;
    };
    let view = gui.canvas_view;
    let Some(origin) = with_widget_mut(gui.canvas_panel, move |widget| {
        view.transform(widget.geometry())
            .rect(canvas_rect(node_rect))
    }) else {
        warn_failure(
            "the drag ghost could not be placed",
            "the canvas panel is not mounted",
        );
        return;
    };
    let bounds = with_widget_mut(gui.canvas_panel, |widget| widget.geometry());
    let name = gui.scene.name(node).unwrap_or("node").to_owned();
    let mut layer =
        rust_widgets::widget::companions::DragGhostLayer::new(Rect::new(0, 0, WIDTH, HEIGHT));
    if let Some(bounds) = bounds {
        layer.set_drop_bounds(Rect::new(bounds.x, bounds.y, bounds.width, bounds.height));
    }
    layer.begin(
        Rect::new(origin.x, origin.y, origin.width, origin.height),
        name,
    );
    let ghost = rust_widgets::control_backend::get_control_backend().mount_widget_box(
        Box::new(layer),
        Some(gui.window),
        0,
    );
    if ghost == 0 {
        warn_failure(
            "the drag ghost could not be mounted",
            "the control backend refused the overlay widget",
        );
        return;
    }
    gui.ghost = ghost;

    let dropped_state = Arc::clone(state);
    let cancelled_state = Arc::clone(state);
    let _ = with_widget_mut(ghost, move |widget| {
        let Some(layer) = (widget as &mut dyn Any)
            .downcast_mut::<rust_widgets::widget::companions::DragGhostLayer>()
        else {
            return false;
        };
        layer
            .ghost_dropped
            .connect(move |value: Arc<(Point, bool)>| {
                let (point, floated) = *value;
                if let Ok(mut gui) = dropped_state.lock() {
                    finish_ghost_drag(&mut gui, point, floated);
                }
            });
        layer.ghost_cancelled.connect(move || {
            if let Ok(mut gui) = cancelled_state.lock() {
                cancel_ghost_drag(&mut gui);
            }
        });
        true
    });
}

/// Applies the ghost's reported drop point to the structural tree.
fn finish_ghost_drag(gui: &mut Gui, point: Point, floated: bool) {
    let ghost = gui.ghost;
    gui.ghost = 0;
    let Some(mut drag) = gui.drag.take() else {
        unregister_later(ghost);
        return;
    };
    if floated {
        gui.message = "drop cancelled: outside the canvas".to_owned();
    } else if let Some((x, y)) = canvas_point(gui, point.x, point.y) {
        let hit = gui.scene.hit(x, y);
        let target = if hit.is_some_and(|node| drag.candidates().accepts(Some(node))) {
            hit
        } else {
            hit.and_then(|node| gui.scene.parent(node))
                .filter(|parent| drag.candidates().accepts(Some(*parent)))
        };
        let index = gui.scene.child_count(target);
        drag.aim(target, index);
        match drag.release(gui.scene.tree()) {
            crate::app::canvas::drag::Drop::Move(moving) => match gui.scene.move_node(moving) {
                Ok(()) => {
                    gui.message = moving.render();
                    sync_outline(gui);
                    sync_canvas(gui);
                }
                Err(reason) => {
                    gui.message = format!("drop refused: {reason}");
                }
            },
            crate::app::canvas::drag::Drop::Refused(reason) => {
                gui.message = format!("drop refused: {reason}");
            }
            crate::app::canvas::drag::Drop::Cancelled => {
                gui.message = "drop cancelled".to_owned();
            }
        }
    } else {
        gui.message = "drop cancelled: no canvas point".to_owned();
    }
    update_status(gui);
    unregister_later(ghost);
}

/// Cancels the drag when the ghost reports Escape.
fn cancel_ghost_drag(gui: &mut Gui) {
    let ghost = gui.ghost;
    gui.ghost = 0;
    gui.drag = None;
    gui.message = "drop cancelled".to_owned();
    update_status(gui);
    unregister_later(ghost);
}

/// Unregisters an overlay after the signal that reported its end has returned.
fn unregister_later(ghost: u64) {
    if ghost == 0 {
        return;
    }
    let _ = rust_widgets::schedule_on_main(Duration::ZERO, move || {
        runtime::unregister(ghost);
    });
}

/// One generated sample asset: a label and the two gradient ends.
const SAMPLE_ASSETS: [(&str, [u8; 3], [u8; 3]); 6] = [
    ("accent", [33, 150, 243], [156, 39, 176]),
    ("surface", [238, 238, 238], [250, 250, 250]),
    ("ink", [33, 33, 33], [97, 97, 97]),
    ("success", [76, 175, 80], [0, 150, 136]),
    ("warning", [255, 193, 7], [255, 112, 67]),
    ("brand", [244, 67, 54], [255, 152, 0]),
];

/// RGBA sample images for the asset gallery.
///
/// These are generated, not decoded files, so the gallery's thumbnail strip and its owned
/// `ThumbnailCache` are exercised on a fresh checkout with no raster fixtures (H-21).
fn sample_asset_images() -> Vec<rust_widgets::GalleryImage> {
    const WIDTH: u32 = 96;
    const HEIGHT: u32 = 72;
    SAMPLE_ASSETS
        .iter()
        .map(|(label, from, to)| {
            let mut pixels = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
            for y in 0..HEIGHT {
                for x in 0..WIDTH {
                    let tx = x as f32 / (WIDTH - 1) as f32;
                    let ty = y as f32 / (HEIGHT - 1) as f32;
                    let t = tx.max(ty);
                    let channel =
                        |a: u8, b: u8| (a as f32 * (1.0 - t) + b as f32 * t).round() as u8;
                    pixels.extend_from_slice(&[
                        channel(from[0], to[0]),
                        channel(from[1], to[1]),
                        channel(from[2], to[2]),
                        255,
                    ]);
                }
            }
            let image = rust_widgets::Image::from_raw(
                rust_widgets::image::ImageData::Rgba8(pixels),
                WIDTH,
                HEIGHT,
            );
            let mut item =
                rust_widgets::GalleryImage::with_label(label, Some(label), WIDTH, HEIGHT);
            item.image = Some(image);
            item
        })
        .collect()
}

/// The registered theme names, in registration order.
fn theme_names() -> Vec<String> {
    rust_widgets::theme::global_theme_manager()
        .theme_names()
        .into_iter()
        .map(str::to_owned)
        .collect()
}

/// The active theme's name, when one is selected.
fn current_theme_name() -> Option<String> {
    let name = rust_widgets::theme::global_theme_manager()
        .current_theme_name()
        .to_owned();
    (!name.is_empty()).then_some(name)
}

/// The combo index of the active theme's metrics profile.
fn current_metrics_index() -> Option<usize> {
    let manager = rust_widgets::theme::global_theme_manager();
    let theme = manager.current_theme()?;
    Some(match theme.metrics_profile {
        rust_widgets::metrics::ControlMetricsProfile::Material => 0,
        rust_widgets::metrics::ControlMetricsProfile::Macos => 1,
        rust_widgets::metrics::ControlMetricsProfile::Cupertino => 2,
    })
}

/// Switches the active theme through the library's reduced-motion-aware crossfade (H-24).
fn switch_theme_faded(gui: &mut Gui, name: &str) {
    let switched = {
        let mut manager = rust_widgets::theme::global_theme_manager();
        manager.set_theme_faded_default(name)
    };
    gui.message = if switched {
        format!("theme {name}: switching")
    } else {
        format!("theme {name}: not registered")
    };
    update_status(gui);
}

/// Drives the theme crossfade until the manager reports it finished.
fn schedule_theme_crossfade(gui: Arc<Mutex<Gui>>) {
    let next = Arc::clone(&gui);
    let _ = rust_widgets::schedule_on_main(Duration::from_millis(16), move || {
        let running = {
            let mut manager = rust_widgets::theme::global_theme_manager();
            let running = manager.is_crossfading();
            if running {
                manager.tick_theme_crossfade(16);
            }
            running
        };
        let window = next.lock().map(|gui| gui.window).unwrap_or(0);
        if window != 0 {
            rust_widgets::invalidate_surface(window);
        }
        if running {
            schedule_theme_crossfade(next);
        }
    });
}

/// Writes the selected metrics profile into the active theme (H-23).
fn apply_metrics_profile(gui: &mut Gui, index: usize) {
    let profile = match index {
        1 => rust_widgets::metrics::ControlMetricsProfile::Macos,
        2 => rust_widgets::metrics::ControlMetricsProfile::Cupertino,
        _ => rust_widgets::metrics::ControlMetricsProfile::Material,
    };
    let mut manager = rust_widgets::theme::global_theme_manager();
    let Some(mut theme) = manager.current_theme().cloned() else {
        gui.message = "metrics: no active theme".to_owned();
        update_status(gui);
        return;
    };
    let name = theme.name.clone();
    theme.metrics_profile = profile;
    manager.register_theme(theme);
    let changed = manager.set_theme(&name);
    drop(manager);
    gui.message = format!(
        "metrics {index}: {}",
        if changed { "applied" } else { "refused" }
    );
    update_status(gui);
}

/// Writes the curve editor's control points into the active theme's motion easing (H-22).
fn apply_motion_curve(gui: &mut Gui, x1: f32, y1: f32, x2: f32, y2: f32) {
    let mut manager = rust_widgets::theme::global_theme_manager();
    let Some(mut theme) = manager.current_theme().cloned() else {
        gui.message = "motion: no active theme".to_owned();
        update_status(gui);
        return;
    };
    let name = theme.name.clone();
    theme.motion.easing = rust_widgets::style::EasingFunction::cubic_bezier(x1, y1, x2, y2);
    theme.motion.curve = rust_widgets::style::MotionCurve::Easing;
    manager.register_theme(theme);
    let changed = manager.set_theme(&name);
    drop(manager);
    gui.message = format!(
        "motion easing: cubic-bezier({x1:.2}, {y1:.2}, {x2:.2}, {y2:.2}){}",
        if changed { "" } else { " (refused)" }
    );
    update_status(gui);
}

/// Seeds the curve editor from the active theme's easing.
fn seed_curve_editor(id: u64) {
    let points = {
        let manager = rust_widgets::theme::global_theme_manager();
        manager
            .current_theme()
            .map_or(((0.42, 0.0), (0.58, 1.0)), |theme| {
                match theme.motion.easing {
                    rust_widgets::style::EasingFunction::CubicBezier { x1, y1, x2, y2 } => {
                        ((x1, y1), (x2, y2))
                    }
                    _ => ((0.42, 0.0), (0.58, 1.0)),
                }
            })
    };
    let _ = with_widget_mut(id, move |widget| {
        let Some(curve) =
            (widget as &mut dyn Any).downcast_mut::<rust_widgets::BezierCurveEditor>()
        else {
            return false;
        };
        curve.set_control_point1(points.0.0, points.0.1);
        curve.set_control_point2(points.1.0, points.1.1);
        true
    });
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
    let context = command_context(gui);
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
    let keymap = window_keymap();
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
    match id.as_str() {
        "view.palette" => {
            gui.message = "the command palette is already open".to_owned();
            update_status(gui);
            return;
        }
        // Every other palette entry runs through the same dispatcher as the menu, so a command
        // implemented for one entry is never "no handler" from the other. Closing the focused
        // docked panel is `view.panel-hide-current` there, not a second id this function would
        // have to keep in step.
        _ => {
            close_palette(gui);
            run_command(gui, &id);
            return;
        }
    }
}

/// The commands this window has a handler for.
///
/// A menu leaf and a shortcut both end in [`run_command`]; a command whose name is not here has no
/// effect in this build, and saying so is the only honest answer. The list is deliberately short:
/// registering a command claims it exists, and the window can only claim what it can do.
const HANDLED_COMMANDS: [&str; 141] = [
    "view.palette",
    "view.welcome",
    "view.split",
    "view.split-down",
    "view.next-group",
    "view.previous-group",
    "view.merge-group",
    "view.move-tab",
    "view.panel-show",
    "view.panel-hide-current",
    "view.panel-restore",
    "view.display-toolbar",
    "view.display-status-bar",
    "view.display-zoom-in",
    "view.display-zoom-out",
    "view.display-zoom-reset",
    "view.mode-welcome",
    "view.mode-project",
    "view.mode-design",
    "view.mode-code",
    "view.mode-debug",
    "view.mode-test",
    "view.mode-release",
    "view.save-layout",
    "view.reset-layout",
    "view.layout-load",
    "view.layout-lock",
    "view.layout-zen",
    "view.layout-fullscreen",
    "file.close",
    "file.close-others",
    "file.close-saved",
    "file.close-project",
    "file.open-reopen",
    "file.exit",
    "file.copy-path",
    "file.reveal",
    "file.save",
    "file.save-all",
    "file.save-as",
    "file.save-copy",
    "project.trust",
    "project.clean-resources",
    "keys.show",
    "build.run",
    "build.check",
    "build.rebuild",
    "build.clean",
    "build.cancel",
    "build.copy-output",
    "build.copy-command",
    "build.locate-first-error",
    "preview.start",
    "preview.stop",
    "preview.rebuild",
    "vcs.refresh",
    "terminal.start",
    "terminal.stop",
    "test.run",
    "test.run-failed",
    "test.run-current-file",
    "test.run-stop",
    "help.shortcuts",
    "help.commands",
    "help.controls",
    "project.check",
    "project.rescan",
    "project.migration",
    "project.check-references",
    "debug.start",
    "debug.continue",
    "debug.step",
    "debug.step-in",
    "debug.step-out",
    "debug.pause",
    "debug.stop",
    "run.start",
    "run.stop",
    "run.clear",
    "run.copy-output",
    "edit.undo",
    "edit.redo",
    "edit.cut",
    "edit.copy",
    "edit.copy-node-id",
    "edit.history-view",
    "edit.paste",
    "edit.select-all",
    "edit.select-clear",
    "edit.select-occurrences",
    "edit.select-next-occurrence",
    "edit.select-cursor-above",
    "edit.select-cursor-below",
    "edit.text-comment",
    "edit.text-indent",
    "edit.text-outdent",
    "edit.text-move-up",
    "edit.text-move-down",
    "edit.text-duplicate",
    "edit.text-delete",
    "edit.text-join",
    "edit.text-sort",
    "edit.find",
    "edit.replace",
    "edit.find-next",
    "edit.find-previous",
    "edit.find-selection",
    "nav.design-parent",
    "nav.design-child",
    "nav.design-next-sibling",
    "nav.design-previous-sibling",
    "nav.goto-line",
    "nav.open-file",
    "nav.file-symbol",
    "nav.workspace-symbol",
    "nav.reveal-current-file",
    "nav.code-definition",
    "nav.code-declaration",
    "nav.code-type-definition",
    "nav.code-implementation",
    "nav.code-references",
    "nav.code-back",
    "nav.code-forward",
    "edit.select-expand",
    "edit.select-shrink",
    "code.fold",
    "code.unfold",
    "code.fold-all",
    "code.unfold-all",
    "code.outline",
    "code.breadcrumbs",
    "code.inlay-hints",
    "code.line-numbers",
    "code.minimap",
    "code.word-wrap",
    "code.whitespace",
    "code.bookmark-toggle",
    "design.fit",
    "design.actual-size",
    "design.grid",
    "design.rulers",
];

fn run_command(gui: &mut Gui, command: &str) {
    run_call(gui, &Call::to(command));
}

/// Runs one registry call, with its arguments.
///
/// A menu leaf carries its argument in the action string (`command:view.panel-show?panel=…`); this
/// is where the call reaches the same dispatcher a key or the palette uses, so an implemented
/// command is never "no handler" depending on which surface asked for it.
fn run_call(gui: &mut Gui, call: &Call) {
    // A command may read the model (save, close, a tab switch), so an edit the control reported
    // but the model has not read yet is applied first. The poller does the same between commands;
    // this is what makes "type, then press the shortcut" independent of the poll's timing.
    gui.apply_pending_editor_text();
    let command = call.id();
    if !HANDLED_COMMANDS.contains(&command) {
        // A key still binds some of these (Ctrl+S, Ctrl+Z). Answering "resolved" was a lie: nothing
        // ran. The menu ledger names the task that will implement it, so the refusal can too.
        gui.message = match crate::app::commands::menu::planned_task(command) {
            Some(task) => tr_args(
                "ui.status.command-not-implemented",
                &[("command", command), ("task", &task)],
            ),
            None => tr_args("ui.status.command-no-handler", &[("command", command)]),
        };
        update_status(gui);
        crate::host::log::emit(
            crate::host::log::Level::Info,
            "app::run::workbench-menu",
            &format!("menu command {command}: not implemented"),
        );
        push_message(gui, gui.message.clone());
        return;
    }
    let outcome = match command {
        "view.palette" => {
            open_palette(gui);
            return;
        }
        "view.welcome" => {
            open_onboarding(gui);
            return;
        }
        "file.close" => {
            let group = gui.groups.focus();
            match gui.groups.tabs(group).and_then(|tabs| tabs.active_index()) {
                None => tr("ui.status.nothing-to-close"),
                Some(index) => {
                    let closed = gui
                        .groups
                        .tabs_mut(group)
                        .and_then(|tabs| tabs.close_view(index).ok())
                        .map(|tab| tab.document().to_owned());
                    match closed {
                        Some(document) => {
                            if let Some(text) = gui.groups.text(&document).map(str::to_owned) {
                                remember_closed_document(gui, &document, text);
                            }
                            gui.sync_editors();
                            tr_args("ui.status.document-closed", &[("document", &document)])
                        }
                        None => tr("ui.error.document-not-closed"),
                    }
                }
            }
        }
        "file.close-others" => close_others(gui),
        "file.close-saved" => close_saved(gui),
        "file.close-project" => close_project(gui),
        "file.open-reopen" => reopen_closed_document(gui),
        "file.exit" => {
            apply_action(gui, "quit");
            gui.message.clone()
        }
        "file.copy-path" => copy_project_path(gui),
        "file.reveal" => reveal_project(gui),
        "file.save" => file_save(gui),
        "file.save-all" => file_save_all(gui),
        "file.save-as" => {
            open_prompt(
                gui,
                PromptKind::SaveAs,
                "ui.dialog.save-as-title",
                "ui.dialog.save-as-label",
                true,
            );
            gui.message.clone()
        }
        "file.save-copy" => {
            open_prompt(
                gui,
                PromptKind::SaveCopy,
                "ui.dialog.save-copy-title",
                "ui.dialog.save-copy-label",
                true,
            );
            gui.message.clone()
        }
        "project.trust" => project_trust(gui),
        "project.clean-resources" => start_resource_cleanup(gui),
        "keys.show" => shortcut_summary(),
        "build.run" => start_build(gui, crate::app::build::compile::Action::Build),
        "build.check" => start_build(gui, crate::app::build::compile::Action::Check),
        "build.rebuild" => start_build(gui, crate::app::build::compile::Action::Rebuild),
        "build.clean" => start_build(gui, crate::app::build::compile::Action::Clean),
        "build.cancel" => build_cancel(gui),
        "build.copy-output" => build_copy_output(gui),
        "build.copy-command" => build_copy_command(gui),
        "build.locate-first-error" => build_locate_first_error(gui),
        "preview.start" => preview_start(gui),
        "preview.stop" => preview_stop(gui),
        "preview.rebuild" => preview_rebuild(gui),
        "vcs.refresh" => vcs_refresh(gui),
        "terminal.start" => terminal_start(gui),
        "terminal.stop" => terminal_stop(gui),
        "test.run" => tests_run(gui),
        "test.run-failed" => tests_run_failed(gui),
        "test.run-current-file" => tests_run_current_file(gui),
        "test.run-stop" => tests_stop(gui),
        "help.shortcuts" => {
            apply_action(gui, "help:shortcuts");
            gui.message.clone()
        }
        "help.commands" => {
            apply_action(gui, "help:commands");
            gui.message.clone()
        }
        "help.controls" => {
            apply_action(gui, "help:controls");
            gui.message.clone()
        }
        "project.check" => project_check(gui),
        "project.rescan" => project_rescan(gui),
        "project.migration" => project_migration(gui),
        "project.check-references" => project_check_references(gui),
        "debug.start" => debug_command(gui, DebugCommand::Start),
        "debug.continue" => debug_command(gui, DebugCommand::Continue),
        "debug.step" => debug_command(gui, DebugCommand::Step),
        "debug.step-in" => debug_command(gui, DebugCommand::StepIn),
        "debug.step-out" => debug_command(gui, DebugCommand::StepOut),
        "debug.pause" => debug_command(gui, DebugCommand::Pause),
        "debug.stop" => debug_command(gui, DebugCommand::Stop),
        "run.start" => run_panel_start(gui),
        "run.stop" => run_panel_stop(gui),
        "run.clear" => run_panel_clear(gui),
        "run.copy-output" => run_panel_copy(gui),
        "edit.undo" => edit_history(gui, true),
        "edit.redo" => edit_history(gui, false),
        "edit.cut" => edit_clipboard(gui, "cut"),
        "edit.copy" => edit_clipboard(gui, "copy"),
        "edit.copy-node-id" => copy_node_id(gui),
        "edit.history-view" => edit_history_view(gui),
        "edit.paste" => edit_clipboard(gui, "paste"),
        "edit.select-all" => edit_clipboard(gui, "select-all"),
        "edit.select-clear" => edit_select_clear(gui),
        "edit.select-expand" => edit_select_expand(gui),
        "edit.select-shrink" => edit_select_shrink(gui),
        "code.fold" => code_fold(gui, "fold"),
        "code.unfold" => code_fold(gui, "unfold"),
        "code.fold-all" => code_fold(gui, "fold-all"),
        "code.unfold-all" => code_fold(gui, "unfold-all"),
        "code.outline" => {
            if !gui
                .panels
                .layout()
                .shown()
                .iter()
                .any(|shown| shown == "source")
            {
                toggle_panel(gui, "source");
            }
            refresh_source(gui);
            tr_args("ui.status.shown", &[("id", "source")])
        }
        "code.breadcrumbs" => code_breadcrumbs(gui),
        "code.inlay-hints" => code_inlay_hints(gui),
        "code.line-numbers" => code_display_toggle(gui, DisplayToggle::LineNumbers),
        "code.minimap" => code_display_toggle(gui, DisplayToggle::Minimap),
        "code.word-wrap" => code_display_toggle(gui, DisplayToggle::WordWrap),
        "code.whitespace" => code_display_toggle(gui, DisplayToggle::Whitespace),
        "code.bookmark-toggle" => code_bookmark_toggle(gui),
        "edit.select-occurrences" => edit_select_occurrences(gui),
        "edit.select-next-occurrence" => edit_select_more(gui, "next-occurrence"),
        "edit.select-cursor-above" => edit_select_more(gui, "cursor-above"),
        "edit.select-cursor-below" => edit_select_more(gui, "cursor-below"),
        "edit.text-comment" => edit_text_operation(gui, "comment"),
        "edit.text-indent" => edit_text_operation(gui, "indent"),
        "edit.text-outdent" => edit_text_operation(gui, "outdent"),
        "edit.text-move-up" => edit_text_operation(gui, "move-up"),
        "edit.text-move-down" => edit_text_operation(gui, "move-down"),
        "edit.text-duplicate" => edit_text_operation(gui, "duplicate"),
        "edit.text-delete" => edit_text_operation(gui, "delete"),
        "edit.text-join" => edit_text_operation(gui, "join"),
        "edit.text-sort" => edit_text_operation(gui, "sort"),
        "edit.find" => edit_search(gui, "find"),
        "edit.replace" => edit_search(gui, "replace"),
        "edit.find-next" => edit_search(gui, "next"),
        "edit.find-previous" => edit_search(gui, "previous"),
        "edit.find-selection" => edit_find_selection(gui),
        "nav.design-parent" => scene_command(gui, "select-parent"),
        "nav.design-child" => scene_command(gui, "select-child"),
        "nav.design-next-sibling" => scene_command(gui, "select-next-sibling"),
        "nav.design-previous-sibling" => scene_command(gui, "select-previous-sibling"),
        "nav.goto-line" => {
            open_prompt(
                gui,
                PromptKind::Goto,
                "ui.dialog.goto-title",
                "ui.dialog.goto-label",
                true,
            );
            gui.message.clone()
        }
        "nav.open-file" => {
            open_prompt(
                gui,
                PromptKind::OpenFile,
                "ui.dialog.open-title",
                "ui.dialog.open-label",
                false,
            );
            gui.message.clone()
        }
        "nav.file-symbol" => {
            open_prompt(
                gui,
                PromptKind::FileSymbol,
                "ui.dialog.file-symbol-title",
                "ui.dialog.file-symbol-label",
                true,
            );
            gui.message.clone()
        }
        "nav.workspace-symbol" => {
            open_prompt(
                gui,
                PromptKind::WorkspaceSymbol,
                "ui.dialog.workspace-symbol-title",
                "ui.dialog.workspace-symbol-label",
                false,
            );
            gui.message.clone()
        }
        "nav.reveal-current-file" => reveal_current_file(gui),
        "nav.code-definition" => code_navigation(
            gui,
            crate::app::run::workbench_outline::CodeQuery::Definition,
        ),
        "nav.code-declaration" => code_navigation(
            gui,
            crate::app::run::workbench_outline::CodeQuery::Declaration,
        ),
        "nav.code-type-definition" => code_navigation(
            gui,
            crate::app::run::workbench_outline::CodeQuery::TypeDefinition,
        ),
        "nav.code-implementation" => code_navigation(
            gui,
            crate::app::run::workbench_outline::CodeQuery::Implementation,
        ),
        "nav.code-references" => code_navigation(
            gui,
            crate::app::run::workbench_outline::CodeQuery::References,
        ),
        "nav.code-back" => code_history(gui, false),
        "nav.code-forward" => code_history(gui, true),
        "design.fit" => design_zoom(gui, None),
        "design.actual-size" => design_zoom(gui, Some(1.0)),
        "design.grid" => design_aid(gui, "grid"),
        "design.rulers" => design_aid(gui, "rulers"),
        // The layout and editor actions the window already had, now reached through the registry.
        "view.split" => {
            apply_action(gui, "group:split");
            gui.message.clone()
        }
        "view.next-group" => {
            apply_action(gui, "group:next");
            gui.message.clone()
        }
        "view.previous-group" => {
            apply_action(gui, "group:previous");
            gui.message.clone()
        }
        "view.split-down" => {
            apply_action(gui, "group:split-down");
            gui.message.clone()
        }
        "view.merge-group" => {
            apply_action(gui, "group:merge");
            gui.message.clone()
        }
        "view.move-tab" => move_tab_to_next_group(gui),
        "view.save-layout" => {
            if gui.layout_locked {
                tr("ui.status.layout-locked")
            } else {
                apply_action(gui, "layout:save");
                gui.message.clone()
            }
        }
        "view.reset-layout" => {
            if gui.layout_locked {
                tr("ui.status.layout-locked")
            } else {
                apply_action(gui, "layout:reset");
                gui.message.clone()
            }
        }
        "view.layout-load" => load_layout(gui),
        "view.layout-lock" => toggle_layout_lock(gui),
        "view.layout-zen" => toggle_zen(gui),
        "view.layout-fullscreen" => toggle_fullscreen(gui),
        "view.panel-show" => show_panel(gui, call.arguments().get("panel").map(String::as_str)),
        "view.panel-hide-current" => hide_current_panel(gui),
        "view.panel-restore" => restore_panel(gui),
        "view.display-toolbar" => {
            apply_action(gui, "display:toolbar");
            gui.message.clone()
        }
        "view.display-status-bar" => {
            apply_action(gui, "display:status-bar");
            gui.message.clone()
        }
        "view.display-zoom-in" => editor_zoom(gui, "in"),
        "view.display-zoom-out" => editor_zoom(gui, "out"),
        "view.display-zoom-reset" => editor_zoom(gui, "reset"),
        "view.mode-welcome" => view_mode(gui, Mode::Welcome),
        "view.mode-project" => view_mode(gui, Mode::Project),
        "view.mode-design" => view_mode(gui, Mode::Design),
        "view.mode-code" => view_mode(gui, Mode::Code),
        "view.mode-debug" => view_mode(gui, Mode::Debug),
        "view.mode-test" => view_mode(gui, Mode::Test),
        "view.mode-release" => view_mode(gui, Mode::Release),
        other => format!("{other}: not implemented in this build"),
    };
    if command != "edit.history-view" {
        record_command(gui, command);
    }
    gui.message = outcome;
    refresh_history_commands(gui);
    update_status(gui);
    crate::host::log::emit(
        crate::host::log::Level::Info,
        "app::run::workbench-menu",
        &format!("menu command {command}: {}", gui.message),
    );
    push_message(gui, format!("{command}: {}", gui.message));
}

/// The panel whose control currently owns the keyboard, if any.
fn focused_panel(gui: &Gui) -> Option<String> {
    let focused = focused_widget()?;
    gui.panel_widgets
        .iter()
        .find(|(_, widgets)| widgets.contains(&focused))
        .map(|(id, _)| id.clone())
}

/// Remembers a hidden panel for `view.panel-restore`, keeping the stack bounded.
fn remember_closed(gui: &mut Gui, id: &str) {
    const LIMIT: usize = 16;
    gui.closed_panels.retain(|other| other != id);
    gui.closed_panels.push(id.to_owned());
    if gui.closed_panels.len() > LIMIT {
        gui.closed_panels.remove(0);
    }
}

/// Hides the panel that holds the keyboard focus.
///
/// The focused control is mapped to its panel through the window's own `panel_widgets`; a focus on
/// the editor, the canvas or the chrome belongs to no panel and is refused rather than guessed.
fn hide_current_panel(gui: &mut Gui) -> String {
    let Some(id) = focused_panel(gui) else {
        return tr("ui.status.no-focused-panel");
    };
    if let Err(error) = gui.panels.hide(&id) {
        return error.message().to_owned();
    }
    remember_closed(gui, &id);
    gui.sync_panels();
    gui.sync_editors();
    tr_args("ui.status.hidden", &[("id", &id)])
}

/// Shows the panel hidden most recently.
fn restore_panel(gui: &mut Gui) -> String {
    let Some(id) = gui.closed_panels.pop() else {
        return tr("ui.status.no-closed-panels");
    };
    if gui.panels.show(&id).is_err() {
        return tr_args("ui.status.no-panel", &[("id", &id)]);
    }
    gui.sync_panels();
    gui.sync_editors();
    tr_args("ui.status.shown", &[("id", &id)])
}

/// Shows one panel named by its §2.2 ledger id.
///
/// The ledger is the only list: a deferred panel answers with the task that will provide it, and an
/// id the ledger does not name is refused rather than passed to the docking model, which knows the
/// window's host ids and would be a second vocabulary.
fn show_panel(gui: &mut Gui, panel: Option<&str>) -> String {
    let Some(id) = panel else {
        return tr("ui.status.panel-not-named");
    };
    let Some(required) = crate::app::shell::panels::REQUIRED_PANELS
        .iter()
        .find(|required| required.id() == id)
    else {
        return tr_args("ui.status.no-panel", &[("id", id)]);
    };
    let Some(host) = required.host_id() else {
        return tr_args(
            "ui.status.command-not-implemented",
            &[("command", "view.panel-show"), ("task", required.domain())],
        );
    };
    toggle_panel(gui, host);
    gui.message.clone()
}

/// Zooms the focused editor's text with the control's own zoom state.
fn editor_zoom(gui: &mut Gui, direction: &str) -> String {
    let applied = edit_with_editor(gui, |editor| {
        match direction {
            "in" => editor.zoom_in(),
            "out" => editor.zoom_out(),
            _ => editor.reset_zoom(),
        }
        true
    });
    if applied.is_none() {
        return tr("ui.status.no-editor");
    }
    tr(match direction {
        "in" => "ui.status.zoomed-in",
        "out" => "ui.status.zoomed-out",
        _ => "ui.status.zoom-reset",
    })
}

/// Sets the design surface's zoom and repaints it.
///
/// `None` is “fit the control”, the behaviour before any explicit zoom; the pointer mapping uses
/// the same view state, so a click lands on the node the person sees.
fn design_zoom(gui: &mut Gui, zoom: Option<f32>) -> String {
    gui.canvas_view.zoom = zoom;
    sync_canvas(gui);
    tr(match zoom {
        Some(_) => "ui.status.zoom-actual",
        None => "ui.status.zoom-fit",
    })
}

/// Shows or hides one design aid and repaints the surface.
fn design_aid(gui: &mut Gui, aid: &str) -> String {
    let showing = if aid == "grid" {
        gui.canvas_view.grid = !gui.canvas_view.grid;
        gui.canvas_view.grid
    } else {
        gui.canvas_view.rulers = !gui.canvas_view.rulers;
        gui.canvas_view.rulers
    };
    sync_canvas(gui);
    tr_args(
        if showing {
            "ui.status.shown"
        } else {
            "ui.status.hidden"
        },
        &[("id", aid)],
    )
}

/// Runs one scene action as a command and answers with the message it produced.
fn scene_command(gui: &mut Gui, action: &str) -> String {
    scene_context_action(gui, action);
    gui.message.clone()
}

/// Closes every tab in the focused group except the active one.
/// Closes every tab whose document has no unsaved changes, in every editor group.
///
/// A document is skipped when any view of it is dirty: "saved" means no view would lose an edit,
/// not that this particular tab happened to be clean.
/// Records the buffer the editor area is showing in the title's one record, with its saved state.
///
/// The active tab is a fact of the editor area; the `Workbench` is the title's record of it. Copying
/// it here — after a tab switch, an edit or a save — is what keeps the status line from naming a
/// document the window stopped showing, and the unsaved marker from outliving the edit it marked.
fn sync_workbench_document(gui: &mut Gui) {
    let focus = gui.groups.focus();
    let active = gui.groups.tabs(focus).and_then(|tabs| {
        tabs.active()
            .map(|tab| (tab.document().to_owned(), tab.is_dirty()))
    });
    match active {
        Some((document, dirty)) => {
            // `open_document` clears the marker, so the tab's own state is applied after it.
            gui.workbench.open_document(document);
            if dirty {
                gui.workbench.mark_dirty();
            }
        }
        None => gui.workbench.close_document(),
    }
}

/// The label a tab strip shows for one document.
///
/// The library's `TabWidget` has no "modified" flag, so the marker the title uses is written into
/// the tab text here: one derivation for both places, and no screenful of space spent on a second
/// indicator. The marker is read from the buffers (`Groups::dirty_documents`), never tracked
/// separately.
fn tab_label(document: &str, dirty: bool) -> String {
    let name = document.rsplit('/').next().unwrap_or(document);
    if dirty {
        format!("• {name}")
    } else {
        name.to_owned()
    }
}

/// Rewrites the tab labels from the buffers, without touching the editors.
///
/// An edit changes which buffers are dirty; the tab strip is the place a person sees that for a
/// document that is not active. Only labels are written — loading a document back into an editor
/// here would move that view's caret on every keystroke.
fn refresh_tab_labels(gui: &mut Gui) {
    let dirty = gui.groups.dirty_documents();
    let groups = gui.groups.group_ids();
    for (index, view) in gui.group_views.iter().enumerate() {
        let Some(group) = groups.get(index).copied() else {
            continue;
        };
        let Some(tabs) = gui.groups.tabs(group) else {
            continue;
        };
        let strip = rust_widgets::app::TabWidgetHandle::from_raw(view.tabs);
        for (tab, entry) in tabs.tabs().iter().enumerate() {
            let label = tab_label(entry.document(), dirty.contains(entry.document()));
            strip.set_tab_text(tab, &label);
        }
    }
}

/// Reconciles one editor surface with its strip: one library buffer per open document.
///
/// The library's editor is multi-buffer by design — `open_buffer`/`activate_buffer_id`/
/// `close_buffer_id`, and `buffer_summaries` is documented as what a host's tab strip needs — so the
/// host keeps **one buffer per document** instead of reloading the whole text on every tab switch.
/// That is what leaves each document's undo history and caret where the person left them: switching
/// tabs activates a buffer, it does not replace a buffer's text.
///
/// The active buffer's text is brought in line with the model only when the two disagree — a command
/// that wrote the model, not a person's typing (which the model reads from this very control). A
/// wholesale replacement is not an undoable edit, so the history is cleared just for that case.
fn sync_editor_buffers(
    editor: &mut CodeEditor,
    documents: &[String],
    active: Option<&str>,
    texts: &BTreeMap<String, String>,
) {
    let buffers = |editor: &CodeEditor| -> BTreeMap<String, u64> {
        editor
            .buffer_summaries()
            .into_iter()
            .zip(editor.buffer_ids())
            .map(|((title, _, _), id)| (title, id))
            .collect()
    };

    // A document the surface does not hold yet gets its own buffer, with the model's text.
    let present = buffers(editor);
    for document in documents {
        if present.contains_key(document) {
            continue;
        }
        let text = texts.get(document).cloned().unwrap_or_default();
        editor.open_buffer(document.clone(), text);
    }

    // A buffer whose document left the strip goes away. Closing the last buffer empties it instead
    // of removing it, which is the library's documented rule and leaves nothing stale on screen.
    let open: Vec<(String, u64)> = buffers(editor).into_iter().collect();
    for (title, id) in open {
        if !documents.iter().any(|document| *document == title) {
            editor.close_buffer_id(id);
        }
    }

    let Some(active) = active else {
        return;
    };
    // Activation is by identity: an earlier close shifts positions but never an id.
    if let Some(id) = buffers(editor).get(active).copied() {
        editor.activate_buffer_id(id);
    }
    // The document's name owns its language (the library's lexer knows 20 of them). Setting it on
    // the active buffer is what gives the window real highlighting instead of the plain default.
    let language = crate::app::editor::highlight::Language::of_path(active).id();
    if editor.active_language() != language {
        editor.set_language(language);
    }
    let expected = texts.get(active).map(String::as_str).unwrap_or("");
    if editor.text() != expected {
        editor.set_text(expected);
        editor.clear_history();
    }
}

/// Decides whether the window may close, answering `false` while a buffer has unsaved changes.
///
/// The library asks this before the window disappears (`WindowHandle::set_close_handler`), so the
/// close button cannot discard an edit silently. The refusal names how many buffers are holding it
/// and leaves the window open; the menu's Quit is the explicit path that does discard, which is why
/// the message points at it instead of offering a second, host-built confirmation.
///
/// Any edit the control reported but the model has not read yet is applied first: a keystroke from a
/// few milliseconds ago is still an unsaved change.
fn window_close_allowed(gui: &mut Gui) -> bool {
    gui.apply_pending_editor_text();
    let dirty = gui.groups.dirty_documents();
    if dirty.is_empty() {
        return true;
    }
    let count = dirty.len();
    let count_text = count.to_string();
    let documents = dirty.into_iter().collect::<Vec<_>>().join(", ");
    gui.message = tr_args("ui.error.window-close-dirty", &[("count", &count_text)]);
    crate::host::log::emit(
        crate::host::log::Level::Warn,
        "app::run::workbench",
        &format!("close refused: {count_text} document(s) with unsaved changes: {documents}"),
    );
    update_status(gui);
    // The refusal is followed by the library's own question, so the answer can be "quit and
    // discard" without the person having to find the menu. A second click while the question is up
    // does not stack another one: the open dialog is the question.
    open_close_dialog(gui, count);
    false
}

/// Asks the unsaved-changes question with the library's `ConfirmDialog`.
///
/// The dialog is mounted on the modal stack and its two signals only write [`Gui::close_slot`]: they
/// run inside the runtime's dispatch, where asking for the `Gui` would deadlock the close handler
/// that opened them. [`schedule_close`] applies the answer on the main loop.
fn open_close_dialog(gui: &mut Gui, count: usize) {
    if gui.close_dialog != 0 {
        return;
    }
    let count = count.to_string();
    let mut dialog = ConfirmDialog::new(
        &tr("ui.dialog.close-dirty-title"),
        &tr_args("ui.dialog.close-dirty-message", &[("count", &count)]),
        Rect::new(
            (WIDTH as i32 - 440) / 2,
            (HEIGHT as i32 - 180) / 2,
            440,
            180,
        ),
    );
    dialog.set_accept_text(tr("ui.dialog.close-dirty-accept"));
    dialog.set_reject_text(tr("ui.dialog.close-dirty-reject"));
    let slot = Arc::clone(&gui.close_slot);
    dialog.accepted.connect(move || {
        if let Ok(mut pending) = slot.lock() {
            *pending = Some(CloseAnswer::Discard);
        }
    });
    let slot = Arc::clone(&gui.close_slot);
    dialog.rejected.connect(move || {
        if let Ok(mut pending) = slot.lock() {
            *pending = Some(CloseAnswer::KeepEditing);
        }
    });
    let Some(id) = runtime::register(Box::new(dialog)) else {
        warn_failure(
            "the unsaved-changes question could not be mounted",
            "the runtime refused to register the dialog",
        );
        return;
    };
    runtime::set_host_window(id, gui.window);
    if !runtime::enter_modal(id) {
        runtime::unregister(id);
        warn_failure(
            "the unsaved-changes question could not be mounted",
            "the modal stack refused the dialog",
        );
        return;
    }
    gui.close_dialog = id;
    crate::host::log::emit(
        crate::host::log::Level::Info,
        "app::run::workbench",
        &format!("asked about {count} unsaved document(s); dialog {id} is open"),
    );
}

/// Applies the answer to the unsaved-changes question on the main loop.
fn apply_close_answer(gui: &mut Gui, answer: CloseAnswer) {
    if gui.close_dialog != 0 {
        let id = gui.close_dialog;
        runtime::exit_modal(id);
        runtime::unregister(id);
        gui.close_dialog = 0;
    }
    match answer {
        CloseAnswer::Discard => {
            gui.message = tr("ui.status.closing");
            update_status(gui);
            crate::host::log::emit(
                crate::host::log::Level::Info,
                "app::run::workbench",
                "the person chose to discard the unsaved changes and quit",
            );
            // The library records the request; the loop performs the stop on its next frame, so no
            // toolkit call crosses this callback.
            rust_widgets::quit();
        }
        CloseAnswer::KeepEditing => {
            gui.message = tr("ui.status.close-cancelled");
            update_status(gui);
            crate::host::log::emit(
                crate::host::log::Level::Info,
                "app::run::workbench",
                "the person kept editing; the window stays open and nothing was discarded",
            );
        }
    }
}

/// Polls the unsaved-changes answer, the same way the rename and prompt dialogs are polled.
fn schedule_close(gui: Arc<Mutex<Gui>>, slot: Arc<Mutex<Option<CloseAnswer>>>) {
    let _ = rust_widgets::schedule_on_main(Duration::from_millis(50), move || {
        // Take the answer before locking the `Gui`: the dialog's callback may hold the slot while
        // the main loop holds the window state, and locking both in the other order would deadlock.
        let answer = match slot.lock() {
            Ok(mut pending) => pending.take(),
            Err(poisoned) => poisoned.into_inner().take(),
        };
        if let Some(answer) = answer
            && let Ok(mut gui) = gui.lock()
        {
            apply_close_answer(&mut gui, answer);
        }
        schedule_close(gui, slot);
    });
}

fn close_saved(gui: &mut Gui) -> String {
    let mut documents = BTreeSet::new();
    for group in gui.groups.group_ids() {
        if let Some(tabs) = gui.groups.tabs(group) {
            for tab in tabs.tabs() {
                documents.insert(tab.document().to_owned());
            }
        }
    }
    // One derivation of "which buffers are dirty" for every close path, so closing the window and
    // closing saved tabs cannot disagree about a document's state.
    let dirty = gui.groups.dirty_documents();
    let saved: Vec<(String, String)> = documents
        .difference(&dirty)
        .filter_map(|document| {
            gui.groups
                .text(document)
                .map(|text| (document.clone(), text.to_owned()))
        })
        .collect();
    let mut closed = 0;
    let mut closed_now = Vec::new();
    for group in gui.groups.group_ids() {
        if let Some(tabs) = gui.groups.tabs_mut(group) {
            for (document, text) in &saved {
                let count = tabs.close_document(document);
                if count > 0 {
                    closed += count;
                    closed_now.push((document.clone(), text.clone()));
                }
            }
        }
    }
    for (document, text) in closed_now {
        remember_closed_document(gui, &document, text);
    }
    gui.sync_editors();
    tr_args("ui.status.saved-closed", &[("count", &closed.to_string())])
}

fn close_others(gui: &mut Gui) -> String {
    let group = gui.groups.focus();
    let Some(index) = gui.groups.tabs(group).and_then(|tabs| tabs.active_index()) else {
        return tr("ui.status.nothing-to-close");
    };
    let closed_documents: Vec<(String, String)> = gui
        .groups
        .tabs(group)
        .map(|tabs| {
            tabs.tabs()
                .iter()
                .enumerate()
                .filter(|(position, _)| *position != index)
                .filter_map(|(_, tab)| {
                    let document = tab.document();
                    gui.groups
                        .text(document)
                        .map(|text| (document.to_owned(), text.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default();
    match gui
        .groups
        .tabs_mut(group)
        .map(|tabs| tabs.close_others(index))
    {
        Some(Ok(closed)) => {
            for (document, text) in closed_documents {
                remember_closed_document(gui, &document, text);
            }
            gui.sync_editors();
            tr_args("ui.status.others-closed", &[("count", &closed.to_string())])
        }
        Some(Err(reason)) => reason,
        None => tr("ui.status.nothing-to-close"),
    }
}

/// Remembers a document a command closed, with the text it had.
///
/// The stack is bounded and deduplicated: reopening is a convenience, not a session log, and a
/// document closed twice has only its latest text.
fn remember_closed_document(gui: &mut Gui, document: &str, text: String) {
    const LIMIT: usize = 16;
    gui.closed_documents.retain(|(other, _)| other != document);
    gui.closed_documents.push((document.to_owned(), text));
    if gui.closed_documents.len() > LIMIT {
        gui.closed_documents.remove(0);
    }
}

/// Reopens the most recently closed document, with its text.
fn reopen_closed_document(gui: &mut Gui) -> String {
    let Some((document, text)) = gui.closed_documents.pop() else {
        return tr("ui.error.nothing-to-reopen");
    };
    gui.groups.pin(document.clone());
    if let Err(reason) = gui.groups.type_into(text) {
        return reason;
    }
    gui.sync_editors();
    tr_args("ui.status.reopened", &[("document", &document)])
}

/// Moves the focused group's active tab to the next editor group.
///
/// Groups are ordered, and the last wraps to the first: with one group there is nowhere to move,
/// which is refused by name rather than silently reordering the strip.
fn move_tab_to_next_group(gui: &mut Gui) -> String {
    let from = gui.groups.focus();
    let Some(index) = gui.groups.tabs(from).and_then(|tabs| tabs.active_index()) else {
        return tr("ui.status.nothing-to-move");
    };
    let groups = gui.groups.group_ids();
    let Some(position) = groups.iter().position(|group| *group == from) else {
        return tr("ui.error.no-other-group");
    };
    let to = groups[(position + 1) % groups.len()];
    if to == from {
        return tr("ui.error.no-other-group");
    }
    let to_index = gui.groups.tabs(to).map_or(0, |tabs| tabs.len());
    match gui.groups.move_tab(from, index, to, to_index) {
        Ok(()) => {
            gui.sync_editors();
            tr("ui.status.tab-moved")
        }
        Err(reason) => tr_args("ui.error.tab-not-moved", &[("reason", &reason)]),
    }
}

/// Whether a document name resolves inside the open project root.
fn document_is_under_root(root: &crate::host::paths::ProjectRoot, document: &str) -> bool {
    if Path::new(document).is_absolute() {
        Path::new(document).starts_with(root.as_path())
    } else {
        root.resolve(document).is_ok()
    }
}

/// Closes the open project, refusing while any of its documents is dirty.
///
/// Closing a project is not a way to discard edits silently: a dirty buffer under the root is
/// named by count and the project stays open. With no dirty buffer, every tab under the root is
/// closed, the project is cleared, and the project panels say there is none.
fn close_project(gui: &mut Gui) -> String {
    let Some(session) = gui.project.as_ref() else {
        return tr("ui.project.none");
    };
    let root = session.root().clone();
    let mut dirty = 0usize;
    let mut documents = BTreeSet::new();
    for group in gui.groups.group_ids() {
        if let Some(tabs) = gui.groups.tabs(group) {
            for tab in tabs.tabs() {
                if !document_is_under_root(&root, tab.document()) {
                    continue;
                }
                documents.insert(tab.document().to_owned());
                if tab.is_dirty() {
                    dirty += 1;
                }
            }
        }
    }
    if dirty > 0 {
        return tr_args(
            "ui.error.project-close-dirty",
            &[("count", &dirty.to_string())],
        );
    }
    for group in gui.groups.group_ids() {
        if let Some(tabs) = gui.groups.tabs_mut(group) {
            for document in &documents {
                tabs.close_document(document);
            }
        }
    }
    gui.project = None;
    gui.saved_texts
        .retain(|document, _| !document_is_under_root(&root, document));
    gui.sync_editors();
    refresh_project(gui);
    refresh_dependencies(gui);
    refresh_components(gui);
    tr("ui.status.project-closed")
}

/// Copies the selected design node's stable id to the clipboard.
fn copy_node_id(gui: &mut Gui) -> String {
    use crate::host::clipboard::{ClipboardHost, ClipboardWrite, NativeClipboard};

    let Some(node) = gui.scene.primary() else {
        return tr("ui.error.no-node-selected");
    };
    let id = node.get().to_string();
    match NativeClipboard::new().write_text(&id) {
        ClipboardWrite::Written => tr_args("ui.status.node-id-copied", &[("id", &id)]),
        ClipboardWrite::Refused { reason } => {
            warn_failure("the node id could not be copied", &reason);
            tr_args("ui.error.copy-refused", &[("reason", &reason)])
        }
    }
}

/// Runs one operation on the focused editor's control, answering `None` when there is no editor.
fn edit_with_editor<T>(gui: &Gui, action: impl FnOnce(&mut CodeEditor) -> T) -> Option<T> {
    let group = gui.groups.focus();
    let editor = gui
        .groups
        .group_ids()
        .iter()
        .position(|id| *id == group)
        .and_then(|position| gui.group_views.get(position))
        .map(|view| view.editor)?;
    with_widget_mut(editor, |widget| {
        let editor = (widget as &mut dyn Any).downcast_mut::<CodeEditor>()?;
        Some(action(editor))
    })
    .flatten()
}

/// Applies the layout file this session was started with, without restarting.
///
/// A file that does not check out changes nothing: `Layout::from_json` and `Layout::apply` are the
/// same pair the startup path uses, so a bad layout is reported rather than half-applied.
fn load_layout(gui: &mut Gui) -> String {
    let Some(path) = gui.layout_path.clone() else {
        return tr("ui.status.no-layout-path");
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) => {
            return tr_args(
                "ui.status.layout-not-loaded",
                &[("reason", &error.to_string())],
            );
        }
    };
    let layout = match Layout::from_json(&text) {
        Ok(layout) => layout,
        Err(error) => {
            return tr_args(
                "ui.status.layout-not-loaded",
                &[("reason", &error.message())],
            );
        }
    };
    if let Err(error) = layout.apply(&mut gui.groups, &mut gui.panels) {
        return tr_args(
            "ui.status.layout-not-loaded",
            &[("reason", &error.message())],
        );
    }
    gui.sync_editors();
    gui.sync_panels();
    tr_args(
        "ui.status.layout-loaded",
        &[("path", &path.display().to_string())],
    )
}

/// Locks or unlocks the layout.
///
/// Locking is enforced where a layout would change: save, load and reset answer with the lock
/// message instead of writing over what the person pinned.
fn toggle_layout_lock(gui: &mut Gui) -> String {
    gui.layout_locked = !gui.layout_locked;
    tr(if gui.layout_locked {
        "ui.status.layout-locked"
    } else {
        "ui.status.layout-unlocked"
    })
}

/// Hides every docked panel and the chrome, and puts back exactly what was shown on exit.
fn toggle_zen(gui: &mut Gui) -> String {
    if let Some(snapshot) = gui.zen.take() {
        for id in snapshot.shown {
            let _ = gui.panels.show(&id);
        }
        gui.show_toolbar = snapshot.toolbar;
        gui.show_status = snapshot.status;
    } else {
        let shown: Vec<String> = gui.panels.layout().shown().to_vec();
        for id in &shown {
            let _ = gui.panels.hide(id);
        }
        gui.zen = Some(ZenSnapshot {
            shown,
            toolbar: gui.show_toolbar,
            status: gui.show_status,
        });
        gui.show_toolbar = false;
        gui.show_status = false;
    }
    if gui.toolbar != 0 {
        rust_widgets::set_widget_visible(gui.toolbar, gui.show_toolbar);
    }
    if gui.status != 0 {
        rust_widgets::set_widget_visible(gui.status, gui.show_status);
    }
    gui.sync_panels();
    gui.sync_editors();
    tr(if gui.zen.is_some() {
        "ui.status.zen-on"
    } else {
        "ui.status.zen-off"
    })
}

/// Asks the platform window to enter or leave full screen.
fn toggle_fullscreen(gui: &mut Gui) -> String {
    gui.fullscreen = !gui.fullscreen;
    if gui.window != 0 {
        WindowHandle::from_raw(gui.window).set_fullscreen(gui.fullscreen);
    }
    tr_args(
        if gui.fullscreen {
            "ui.status.shown"
        } else {
            "ui.status.hidden"
        },
        &[("id", "fullscreen")],
    )
}

/// Shows or hides one piece of the window chrome and re-flows the areas it used to occupy.
///
/// The widget's visibility alone would leave its row reserved, so the layout owns the flag too
/// (`WorkbenchLayout::showing_toolbar`/`showing_status`); the two cannot disagree.
fn toggle_chrome(gui: &mut Gui, toolbar: bool) {
    let (widget, id, showing) = if toolbar {
        (gui.toolbar, "toolbar", gui.show_toolbar)
    } else {
        (gui.status, "status-bar", gui.show_status)
    };
    let showing = !showing;
    if toolbar {
        gui.show_toolbar = showing;
    } else {
        gui.show_status = showing;
    }
    if widget != 0 {
        rust_widgets::set_widget_visible(widget, showing);
    }
    gui.refresh_layout();
    gui.message = tr_args(
        if showing {
            "ui.status.shown"
        } else {
            "ui.status.hidden"
        },
        &[("id", id)],
    );
}

/// Reports the open project's format, identity, members and references into Messages.
///
/// The questions are the same ones `--project-check` prints; reading the session's own
/// [`ProjectFile`] means the GUI and the CLI answer from one parse, not two.
fn project_check(gui: &mut Gui) -> String {
    let Some(session) = gui.project.as_ref() else {
        return tr("ui.project.none");
    };
    let file = session.file();
    let mut rows = vec![
        format!("project: {}", file.path().display()),
        format!("root: {}", file.root().as_path().display()),
        format!("format: {}", file.verdict().message()),
    ];
    match file.content() {
        ProjectContent::Interpreted(document) => {
            rows.push(format!(
                "identity: {} ({})",
                document.identity().name(),
                document.identity().id()
            ));
            rows.push(format!(
                "pages: {}, resources: {}, targets: {}, generated: {}, services: {}",
                document.pages().len(),
                document.resources().len(),
                document.targets().len(),
                document.generated().len(),
                document.services().len()
            ));
            rows.push(format!(
                "members this build keeps without reading: {}",
                document.unknown_fields().len()
            ));
            rows.push(format!("references: {}", document.references().len()));
        }
        ProjectContent::Foreign(_) => {
            rows.push(
                "the project file is not in a format this build reads; nothing was guessed"
                    .to_owned(),
            );
        }
    }
    let summary = rows.len().to_string();
    for row in rows {
        push_message(gui, row);
    }
    tr_args("ui.status.project-checked", &[("rows", &summary)])
}

/// Re-reads the project structure into the Project panel.
fn project_rescan(gui: &mut Gui) -> String {
    if gui.project.is_none() {
        return tr("ui.project.none");
    }
    refresh_project(gui);
    tr("ui.status.project-rescanned")
}

/// Reports the format version and the migration steps writing the project would take.
fn project_migration(gui: &mut Gui) -> String {
    let Some(session) = gui.project.as_ref() else {
        return tr("ui.project.none");
    };
    let file = session.file();
    let verdict = file.verdict();
    let mut rows = vec![format!("format: {}", verdict.message())];
    match migration_path(verdict.version()) {
        Ok(steps) if steps.is_empty() => {
            rows.push("nothing to migrate: this is the version the build writes".to_owned());
        }
        Ok(steps) => {
            for step in steps {
                rows.push(format!("{} -> {}: {}", step.from(), step.to(), step.what()));
            }
        }
        Err(version) => {
            rows.push(format!(
                "no migration step out of {version}; this build will not write it"
            ));
        }
    }
    for row in rows {
        push_message(gui, row);
    }
    tr("ui.status.migration-previewed")
}

/// Lists every project reference with its verdict, without touching any file.
fn project_check_references(gui: &mut Gui) -> String {
    let Some(session) = gui.project.as_ref() else {
        return tr("ui.project.none");
    };
    let file = session.file();
    let rendered: Vec<String> = {
        let ProjectContent::Interpreted(document) = file.content() else {
            return tr("ui.status.project-not-readable");
        };
        document
            .references()
            .into_iter()
            .map(|reference| reference.render())
            .collect()
    };
    let count = rendered.len().to_string();
    for row in rendered {
        push_message(gui, row);
    }
    tr_args("ui.status.references-checked", &[("count", &count)])
}

/// The declared resources no page document references.
///
/// A resource is referenced when any page's `Resource` reference names it, either as written or as
/// resolved (the two spellings one path can have). An unreadable page refuses the whole answer:
/// guessing that it references nothing would turn a read error into a deleted declaration.
fn unused_resources(session: &ProjectSession) -> Result<Vec<String>, String> {
    use crate::project::record::format::ReferenceKind;
    use crate::project::record::page::PageDocument;

    let file = session.file();
    let ProjectContent::Interpreted(document) = file.content() else {
        return Err(tr("ui.status.project-not-readable"));
    };
    let root = file.root();
    let mut referenced: BTreeSet<String> = BTreeSet::new();
    for page in document.pages() {
        let page_document =
            PageDocument::read(root, page.document().written()).map_err(|error| error.message())?;
        for reference in page_document.references() {
            if reference.kind() == ReferenceKind::Resource {
                referenced.insert(reference.written().to_owned());
                if let Some(path) = reference.resolved() {
                    referenced.insert(path.to_string_lossy().into_owned());
                }
            }
        }
    }
    Ok(document
        .resources()
        .iter()
        .map(|resource| resource.path().written().to_owned())
        .filter(|written| {
            if referenced.contains(written) {
                return false;
            }
            root.resolve(written)
                .ok()
                .map(|resolved| resolved.to_string_lossy().into_owned())
                .is_none_or(|resolved| !referenced.contains(&resolved))
        })
        .collect())
}

/// Lists unreferenced resources and asks for the confirmation that removes their declarations.
fn start_resource_cleanup(gui: &mut Gui) -> String {
    let Some(session) = gui.project.as_ref() else {
        return tr("ui.project.none");
    };
    let candidates = match unused_resources(session) {
        Ok(candidates) => candidates,
        Err(reason) => {
            return tr_args("ui.error.resources-clean-refused", &[("reason", &reason)]);
        }
    };
    if candidates.is_empty() {
        return tr("ui.status.resources-clean-none");
    }
    let count = candidates.len();
    for candidate in &candidates {
        push_message(gui, candidate.clone());
    }
    gui.pending_resource_cleanup = candidates;
    open_prompt(
        gui,
        PromptKind::CleanResources,
        "ui.dialog.clean-resources-title",
        "ui.dialog.clean-resources-label",
        false,
    );
    tr_args(
        "ui.status.resources-clean-candidates",
        &[("count", &count.to_string())],
    )
}

/// Applies a resource-cleanup answer: only the exact word `delete` removes declarations.
fn apply_clean_resources(gui: &mut Gui, answer: &str) {
    let candidates = std::mem::take(&mut gui.pending_resource_cleanup);
    if candidates.is_empty() || !answer.trim().eq_ignore_ascii_case("delete") {
        gui.message = tr("ui.status.resources-clean-cancelled");
        update_status(gui);
        return;
    }
    let outcome = {
        let Some(session) = gui.project.as_ref() else {
            gui.message = tr("ui.project.none");
            update_status(gui);
            return;
        };
        if !session.may_write(&gui.permissions) {
            gui.message = tr("ui.error.project-not-trusted");
            update_status(gui);
            return;
        }
        let file = session.file();
        let ProjectContent::Interpreted(document) = file.content() else {
            gui.message = tr("ui.status.project-not-readable");
            update_status(gui);
            return;
        };
        let mut updated = document.clone();
        for written in &candidates {
            let (next, _removed) = updated.without_resource(written);
            updated = next;
        }
        file.write_document(&updated)
            .map(|_| file.path().to_path_buf())
            .map_err(|error| error.message())
    };
    match outcome {
        Ok(path) => match ProjectSession::open(&path) {
            Ok(session) => {
                gui.project = Some(session);
                refresh_project(gui);
                refresh_dependencies(gui);
                gui.message = tr_args(
                    "ui.status.resources-cleaned",
                    &[("count", &candidates.len().to_string())],
                );
                update_status(gui);
            }
            Err(error) => {
                gui.message = tr_args(
                    "ui.error.resources-clean-refused",
                    &[("reason", &error.message())],
                );
                update_status(gui);
            }
        },
        Err(reason) => {
            gui.message = tr_args("ui.error.resources-clean-refused", &[("reason", &reason)]);
            update_status(gui);
        }
    }
}

/// Runs one debug command, the same call the panel's four buttons make.
fn debug_command(gui: &mut Gui, command: DebugCommand) -> String {
    let accepted = match command {
        DebugCommand::Start => {
            // The debuggee is generated from the same scene the tree and canvas show, so the source
            // map entries name real nodes. The selected node is the breakpoint anchor; with no
            // selection the second row (the demo panel) is used.
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
    if command == DebugCommand::Start
        && let Some(state) = gui.self_weak.upgrade()
    {
        schedule_debug_poll(&state);
    }
    crate::host::log::emit(
        crate::host::log::Level::Info,
        "app::run::workbench-debug",
        &format!("debug {} requested (accepted {accepted})", command.name()),
    );
    tr_args(
        if accepted {
            "ui.status.debug-ran"
        } else {
            "ui.status.debug-unavailable"
        },
        &[("command", command.name())],
    )
}

/// Switches the workbench's mode; nothing else about the session changes.
fn view_mode(gui: &mut Gui, mode: Mode) -> String {
    gui.workbench.set_mode(mode);
    update_status(gui);
    tr_args("ui.status.mode-changed", &[("mode", mode.name())])
}

/// Expands the editor selection to the next structural unit.
///
/// The editor's own `expand_selection` is progressive within one structural kind, so the command
/// tries the kinds in the order a person widens a selection: word, line, brackets, block. The
/// control records the replaced selection itself (RW-114), which is what
/// [`edit_select_shrink`] walks back through — one history, not a host copy that can drift from it.
fn edit_select_expand(gui: &mut Gui) -> String {
    use rust_widgets::widget::special_widgets::code_editor::StructuralKind;

    if active_document(gui).is_none() {
        return tr("ui.status.no-editor");
    }
    let expanded = edit_with_editor(gui, |editor| {
        for kind in [
            StructuralKind::Word,
            StructuralKind::Line,
            StructuralKind::Bracket,
            StructuralKind::Block,
        ] {
            if editor.expand_selection(kind) {
                return true;
            }
        }
        false
    });
    match expanded {
        Some(true) => tr("ui.status.selection-expanded"),
        Some(false) => tr("ui.status.nothing-to-expand"),
        None => tr("ui.source.no-editor"),
    }
}

/// Restores the selection the last `edit.select-expand` replaced (RW-114).
///
/// The editor owns the history (`CodeEditor::shrink_selection`) and clears it on every document
/// change, so a stale position is never restored.
fn edit_select_shrink(gui: &mut Gui) -> String {
    if active_document(gui).is_none() {
        return tr("ui.status.no-editor");
    }
    match edit_with_editor(gui, |editor| editor.shrink_selection()) {
        Some(true) => tr("ui.status.selection-shrunk"),
        Some(false) => tr("ui.status.nothing-to-shrink"),
        None => tr("ui.source.no-editor"),
    }
}

/// Toggles inlay-hint painting on the focused editor.
///
/// The control owns the flag (`config.show_inlay_hints`), so the command only flips it and reads
/// the result back; the hints themselves are not discarded.
fn code_inlay_hints(gui: &mut Gui) -> String {
    let enabled = edit_with_editor(gui, |editor| {
        let next = !editor.inlay_hints_enabled();
        editor.set_inlay_hints_enabled(next);
        next
    });
    match enabled {
        Some(true) => tr("ui.status.inlay-hints-shown"),
        Some(false) => tr("ui.status.inlay-hints-hidden"),
        None => tr("ui.source.no-editor"),
    }
}

/// A display setting that can be switched at runtime (RW-112).
#[derive(Clone, Copy, PartialEq, Eq)]
enum DisplayToggle {
    LineNumbers,
    Minimap,
    WordWrap,
    Whitespace,
}

/// Switches one of the editor's display settings without rebuilding the control.
///
/// The control owns each flag and records a full-surface repaint itself
/// (`CodeEditor::set_show_line_numbers` and friends, RW-112), so the command only flips it and reads
/// the result back. Rebuilding the editor would lose its undo history and selection, which is what
/// this path exists to avoid.
fn code_display_toggle(gui: &mut Gui, toggle: DisplayToggle) -> String {
    let state = edit_with_editor(gui, |editor| match toggle {
        DisplayToggle::LineNumbers => {
            let next = !editor.config().show_line_numbers;
            editor.set_show_line_numbers(next);
            next
        }
        DisplayToggle::Minimap => {
            let next = !editor.config().show_minimap;
            editor.set_show_minimap(next);
            next
        }
        DisplayToggle::WordWrap => {
            let next = !editor.config().word_wrap;
            editor.set_word_wrap(next);
            next
        }
        DisplayToggle::Whitespace => {
            let next = !editor.config().show_whitespace;
            editor.set_show_whitespace(next);
            next
        }
    });
    match (state, toggle) {
        (Some(true), DisplayToggle::LineNumbers) => tr("ui.status.line-numbers-on"),
        (Some(false), DisplayToggle::LineNumbers) => tr("ui.status.line-numbers-off"),
        (Some(true), DisplayToggle::Minimap) => tr("ui.status.minimap-on"),
        (Some(false), DisplayToggle::Minimap) => tr("ui.status.minimap-off"),
        (Some(true), DisplayToggle::WordWrap) => tr("ui.status.word-wrap-on"),
        (Some(false), DisplayToggle::WordWrap) => tr("ui.status.word-wrap-off"),
        (Some(true), DisplayToggle::Whitespace) => tr("ui.status.whitespace-on"),
        (Some(false), DisplayToggle::Whitespace) => tr("ui.status.whitespace-off"),
        (None, _) => tr("ui.source.no-editor"),
    }
}

/// Toggles a bookmark on the caret line of the focused editor.
fn code_bookmark_toggle(gui: &mut Gui) -> String {
    let bookmarked = edit_with_editor(gui, |editor| {
        let line = editor.cursor().0;
        editor.toggle_bookmark(line)
    });
    match bookmarked {
        Some(true) => tr("ui.status.bookmark-added"),
        Some(false) => tr("ui.status.bookmark-removed"),
        None => tr("ui.source.no-editor"),
    }
}

/// Folds or unfolds the editor's blocks (`code.structure.*`).
fn code_fold(gui: &mut Gui, command: &str) -> String {
    match command {
        "fold" => {
            let folded = edit_with_editor(gui, |editor| {
                let line = editor.cursor().0;
                editor.fold_block_at(line)
            });
            match folded {
                Some(true) => tr("ui.status.folded"),
                Some(false) => tr("ui.status.nothing-to-fold"),
                None => tr("ui.source.no-editor"),
            }
        }
        "unfold" => {
            let unfolded = edit_with_editor(gui, |editor| {
                let line = editor.cursor().0;
                let start = editor
                    .fold_regions()
                    .into_iter()
                    .find(|region| {
                        region.folded && region.start_line <= line && line <= region.end_line
                    })
                    .map(|region| region.start_line);
                match start {
                    Some(start) => editor.unfold(start),
                    None => editor.unfold(line),
                }
            });
            match unfolded {
                Some(true) => tr("ui.status.unfolded"),
                Some(false) => tr("ui.status.nothing-to-unfold"),
                None => tr("ui.source.no-editor"),
            }
        }
        "fold-all" => {
            let count = edit_with_editor(gui, |editor| editor.fold_all()).unwrap_or(0);
            tr_args("ui.status.folded-many", &[("count", &count.to_string())])
        }
        "unfold-all" => {
            let count = edit_with_editor(gui, |editor| {
                let folded = editor
                    .fold_regions()
                    .iter()
                    .filter(|region| region.folded)
                    .count();
                editor.unfold_all();
                folded
            })
            .unwrap_or(0);
            tr_args("ui.status.unfolded-many", &[("count", &count.to_string())])
        }
        _ => tr("ui.status.command-no-handler"),
    }
}

/// Shows the symbols containing the caret, joined the way a breadcrumb bar reads.
fn code_breadcrumbs(gui: &mut Gui) -> String {
    if !gui.source.is_configured() {
        return tr("ui.error.navigation-no-server");
    }
    let Some((line, column)) = edit_with_editor(gui, |editor| editor.cursor()) else {
        return tr("ui.status.no-editor");
    };
    if let Err(reason) = refresh_active_source(gui) {
        return tr_args("ui.error.navigation-failed", &[("reason", &reason)]);
    }
    let Some(text) = gui.source.document_text().map(str::to_owned) else {
        return tr("ui.error.navigation-not-found");
    };
    let Some(offset) = crate::app::run::workbench_outline::offset_of(&text, line, column) else {
        return tr("ui.error.navigation-not-found");
    };
    let crumbs = gui.source.breadcrumbs(offset);
    if crumbs.is_empty() {
        return tr("ui.error.navigation-not-found");
    }
    let path = crumbs.join(" > ");
    push_message(gui, path.clone());
    tr_args("ui.status.breadcrumbs", &[("path", &path)])
}

/// Collapses the editor selection to the caret, keeping the caret where it is.
fn edit_select_clear(gui: &mut Gui) -> String {
    let cleared = edit_with_editor(gui, |editor| {
        let (line, column) = editor.cursor();
        editor.set_cursor(line, column, false);
        true
    });
    if cleared.is_none() {
        return tr("ui.status.no-editor");
    }
    tr("ui.status.selection-cleared")
}

/// Applies one of the editor's line operations and writes the text back into the model.
///
/// The operation itself is the control's own (`toggle_line_comment`, `indent_selection`,
/// `move_line`, ...), so the host has no second implementation of indentation or line movement.
fn edit_text_operation(gui: &mut Gui, operation: &str) -> String {
    let changed = edit_with_editor(gui, |editor| {
        match operation {
            "comment" => editor.toggle_line_comment(),
            "indent" => editor.indent_selection(),
            "outdent" => editor.outdent_selection(),
            "move-up" => editor.move_line(-1),
            "move-down" => editor.move_line(1),
            "duplicate" => editor.duplicate_line(),
            "delete" => editor.delete_line(),
            "join" => editor.join_lines(),
            _ => editor.sort_lines(),
        }
        true
    });
    if changed.is_none() {
        return tr("ui.status.no-editor");
    }
    if let Some(text) = edit_with_editor(gui, |editor| editor.text())
        && let Err(reason) = gui.groups.type_into(text)
    {
        return reason;
    }
    gui.sync_editors();
    let id = match operation {
        "comment" => "edit.text-comment",
        "indent" => "edit.text-indent",
        "outdent" => "edit.text-outdent",
        "move-up" => "edit.text-move-up",
        "move-down" => "edit.text-move-down",
        "duplicate" => "edit.text-duplicate",
        "delete" => "edit.text-delete",
        "join" => "edit.text-join",
        _ => "edit.text-sort",
    };
    let title = tr(&format!("cmd.{id}.title"));
    tr_args("ui.status.text-changed", &[("command", &title)])
}

/// Extends the selection or adds a cursor with the editor's own multi-cursor state.
fn edit_select_more(gui: &mut Gui, operation: &str) -> String {
    let outcome = edit_with_editor(gui, |editor| match operation {
        "next-occurrence" => editor.select_next_occurrence(),
        "cursor-above" => editor.add_cursor_above(),
        _ => editor.add_cursor_below(),
    });
    match outcome {
        None => tr("ui.status.no-editor"),
        Some(true) => tr("ui.status.cursor-added"),
        Some(false) => tr("ui.status.no-cursor-line"),
    }
}

/// Adds a cursor at every occurrence of the selected text, the editor's own multi-cursor.
fn edit_select_occurrences(gui: &mut Gui) -> String {
    let count = edit_with_editor(gui, |editor| editor.select_all_occurrences());
    match count {
        None => tr("ui.status.no-editor"),
        Some(0) => tr("ui.status.nothing-selected"),
        Some(count) => tr_args(
            "ui.status.occurrences-selected",
            &[("count", &count.to_string())],
        ),
    }
}

/// Searches for the text the focused editor has selected.
fn edit_find_selection(gui: &mut Gui) -> String {
    let opened = edit_with_editor(gui, |editor| {
        let Some(selected) = editor.selected_text() else {
            return false;
        };
        if selected.is_empty() {
            return false;
        }
        editor.set_search_query(&selected);
        editor.open_find(false);
        true
    });
    match opened {
        None => tr("ui.status.no-editor"),
        Some(false) => tr("ui.status.nothing-selected"),
        Some(true) => tr("ui.status.find-open"),
    }
}

/// Opens the editor's find bar, or moves to the next/previous match.
///
/// The find state and the matches are the control's own: the host only asks it to open or to step,
/// so a second search model here cannot disagree with the one the bar shows.
fn edit_search(gui: &mut Gui, operation: &str) -> String {
    let stepped = edit_with_editor(gui, |editor| match operation {
        "find" => {
            editor.open_find(false);
            None
        }
        "replace" => {
            editor.open_find(true);
            None
        }
        "next" => Some(editor.find_next(false)),
        _ => Some(editor.find_next(true)),
    });
    match stepped {
        None => tr("ui.status.no-editor"),
        Some(None) => tr(match operation {
            "replace" => "ui.status.replace-open",
            _ => "ui.status.find-open",
        }),
        Some(Some(true)) => tr("ui.status.found"),
        Some(Some(false)) => tr("ui.status.no-match"),
    }
}

/// Cuts, copies, pastes or selects everything in the focused editor.
///
/// The clipboard is the editor control's own (it owns the system clipboard call); a cut or a paste
/// changes its text, so the text is written back into the model the same way `edit_history` does,
/// and a later `sync_editors` cannot silently revert it.
fn edit_clipboard(gui: &mut Gui, operation: &str) -> String {
    let done = edit_with_editor(gui, |editor| match operation {
        "cut" => editor.cut(),
        "copy" => editor.copy(),
        "paste" => editor.paste(),
        "select-all" => {
            editor.select_all();
            true
        }
        _ => false,
    });
    match done {
        None => tr("ui.status.no-editor"),
        Some(false) => tr(match operation {
            "paste" => "ui.status.clipboard-empty",
            _ => "ui.status.nothing-selected",
        }),
        Some(true) => {
            if operation == "cut" || operation == "paste" {
                if let Some(text) = edit_with_editor(gui, |editor| editor.text())
                    && let Err(reason) = gui.groups.type_into(text)
                {
                    return reason;
                }
                gui.sync_editors();
            }
            tr(match operation {
                "cut" => "ui.status.cut",
                "copy" => "ui.status.copied",
                "paste" => "ui.status.pasted",
                _ => "ui.status.selected-all",
            })
        }
    }
}

/// Takes back (or puts back) the focused editor's newest change.
///
/// The undo history is the editor control's own — the library's `CodeEditor` owns it, and a second
/// history here would be a second answer to "what was the last change". The text the control ends
/// with is written back into the model, so a later `sync_editors` cannot silently revert the undo.
fn edit_history(gui: &mut Gui, undo: bool) -> String {
    let group = gui.groups.focus();
    let Some(editor) = gui
        .groups
        .group_ids()
        .iter()
        .position(|id| *id == group)
        .and_then(|position| gui.group_views.get(position))
        .map(|view| view.editor)
    else {
        return tr("ui.status.no-editor");
    };
    let changed = with_widget_mut(editor, move |widget| {
        let editor = (widget as &mut dyn Any).downcast_mut::<CodeEditor>()?;
        Some(if undo { editor.undo() } else { editor.redo() })
    });
    match changed {
        Some(Some(true)) => {}
        Some(Some(false)) => {
            return tr(if undo {
                "ui.status.nothing-to-undo"
            } else {
                "ui.status.nothing-to-redo"
            });
        }
        _ => return tr("ui.status.no-editor"),
    }
    let text = with_widget_mut(editor, |widget| {
        let editor = (widget as &mut dyn Any).downcast_mut::<CodeEditor>()?;
        Some(editor.text())
    });
    if let Some(Some(text)) = text
        && let Err(reason) = gui.groups.type_into(text)
    {
        return reason;
    }
    gui.sync_editors();
    tr(if undo {
        "ui.status.undone"
    } else {
        "ui.status.redone"
    })
}

/// How many command-history rows the window keeps.
const COMMAND_HISTORY_LIMIT: usize = 200;

/// Records one executed command, newest last.
fn record_command(gui: &mut Gui, command: &str) {
    let at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0);
    gui.command_history.push((at, command.to_owned()));
    if gui.command_history.len() > COMMAND_HISTORY_LIMIT {
        gui.command_history.remove(0);
    }
}

/// One command-history row: the wall-clock time and the command id.
fn history_row(at: u64, command: &str) -> String {
    let seconds = (at / 1000) % 86_400;
    format!(
        "{:02}:{:02}:{:02} {command}",
        seconds / 3600,
        (seconds % 3600) / 60,
        seconds % 60
    )
}

/// Shows the commands this window executed, newest first.
fn edit_history_view(gui: &mut Gui) -> String {
    if gui.command_history.is_empty() {
        return tr("ui.status.history-empty");
    }
    let rows: Vec<String> = gui
        .command_history
        .iter()
        .rev()
        .map(|(at, command)| history_row(*at, command))
        .collect();
    let count = rows.len();
    for row in rows {
        push_message(gui, row);
    }
    tr_args("ui.status.history-shown", &[("count", &count.to_string())])
}

/// Starts the Terminal panel's shell session and keeps its poller alive.
fn terminal_start(gui: &mut Gui) -> String {
    let root = gui
        .project
        .as_ref()
        .map(|project| project.root().as_path().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let message = match gui.terminal.start(root) {
        Ok(()) => tr("ui.status.terminal-started"),
        Err(reason) => reason,
    };
    let rows = gui.terminal.rows();
    if gui.terminal_list != 0 {
        set_list_rows(gui.terminal_list, rows);
    }
    if let Some(state) = gui.self_weak.upgrade() {
        schedule_terminal_poll(&state);
    }
    message
}

/// Asks the Terminal panel's shell session to stop.
fn terminal_stop(gui: &mut Gui) -> String {
    if gui.terminal.stop() {
        tr("ui.status.stop-requested")
    } else {
        tr("ui.status.not-running")
    }
}

/// Starts the Tests panel's run and keeps its poller alive.
fn tests_run(gui: &mut Gui) -> String {
    tests_start(gui, crate::app::quality::Selection::all())
}

/// Starts one test selection on the worker and schedules its poll.
fn tests_start(gui: &mut Gui, selection: crate::app::quality::Selection) -> String {
    let root = gui
        .project
        .as_ref()
        .map(|project| project.root().as_path().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let started = gui.tests.start(root, selection);
    refresh_tests(gui);
    if started && let Some(state) = gui.self_weak.upgrade() {
        schedule_tests_poll(&state);
    }
    tr(if started {
        "ui.status.tests-started"
    } else {
        "ui.status.tests-busy"
    })
}

/// The cargo test filter that names the module a source file belongs to.
///
/// A test's harness name is its module path (`app::run::workbench_gui::tests::…`), not its file
/// path. `src/lib.rs`, `src/main.rs` and files outside `src/` have no distinguishing module path,
/// so they are refused rather than run against a filter that would select the whole project.
fn test_module_filter(document: &str) -> Option<String> {
    let parts: Vec<&str> = document
        .split(['/', '\\'])
        .filter(|part| !part.is_empty())
        .collect();
    let source = parts.iter().rposition(|part| *part == "src")?;
    let file = *parts.last()?;
    let stem = file.strip_suffix(".rs")?;
    let mut module: Vec<&str> = parts[source + 1..parts.len() - 1].to_vec();
    if !matches!(stem, "lib" | "main" | "mod") {
        module.push(stem);
    }
    if module.is_empty() {
        return None;
    }
    Some(module.join("::"))
}

/// Runs the tests whose module path belongs to the focused document.
fn tests_run_current_file(gui: &mut Gui) -> String {
    let Some((document, _)) = active_document(gui) else {
        return tr("ui.status.no-editor");
    };
    let Some(filter) = test_module_filter(&document) else {
        return tr("ui.error.tests-current-unavailable");
    };
    tests_start(gui, crate::app::quality::Selection::filter(filter))
}

/// Reruns exactly the tests that failed in the last finished run.
fn tests_run_failed(gui: &mut Gui) -> String {
    let failed = gui.tests.failed_names();
    if failed.is_empty() {
        return tr("ui.error.no-failed-tests");
    }
    tests_start(gui, crate::app::quality::Selection::exact(failed))
}

/// Asks the live test run to stop.
fn tests_stop(gui: &mut Gui) -> String {
    if gui.tests.stop() {
        tr("ui.status.tests-stopped")
    } else {
        tr("ui.status.tests-not-running")
    }
}

/// Starts the preview service and reports whether it reached ready.
fn preview_start(gui: &mut Gui) -> String {
    let outcome = with_preview(|preview| preview.start("workbench").and_then(|_| preview.ready()));
    refresh_preview(gui);
    match outcome {
        Ok(()) => tr("ui.status.preview-started"),
        Err(reason) => reason,
    }
}

/// Stops the preview service.
fn preview_stop(gui: &mut Gui) -> String {
    let stopped = with_preview(|preview| {
        let stopped = preview.stop();
        let _ = preview.finish_stop();
        stopped
    });
    refresh_preview(gui);
    match stopped {
        Ok(stopped) => format!("{}: {}", tr("ui.status.preview-stopped"), stopped.render()),
        Err(reason) => reason,
    }
}

/// Mounts or updates the preview from the startup page's library document.
///
/// The JSON is the one `--generate` consumes (`generate::read_document`), so the preview and the
/// generated source interpret the same declaration; the first rebuild mounts, later ones update so
/// the controls keep their ids.
fn preview_rebuild(gui: &mut Gui) -> String {
    use rust_widgets::widget::WidgetFactory;

    let (relative, _page) = match read_startup_page(gui) {
        Ok(found) => found,
        Err(PageRead::NoProject) => return tr("ui.project.none"),
        Err(PageRead::Unreadable) => {
            return tr_args(
                "ui.status.preview-rebuild-failed",
                &[("reason", "the project file is unreadable")],
            );
        }
        Err(PageRead::NoPage) => {
            return tr_args(
                "ui.status.preview-rebuild-failed",
                &[("reason", "the project names no page")],
            );
        }
        Err(PageRead::Failed(reason)) => {
            return tr_args("ui.status.preview-rebuild-failed", &[("reason", &reason)]);
        }
    };
    let Some(root) = gui
        .project
        .as_ref()
        .map(|p| p.root().as_path().to_path_buf())
    else {
        return tr("ui.project.none");
    };
    let path = root.join(&relative);
    let json = match crate::app::run::generate::read_document(&path.to_string_lossy()) {
        Ok(json) => json,
        Err(reason) => {
            return tr_args("ui.status.preview-rebuild-failed", &[("reason", &reason)]);
        }
    };
    let mounted = gui.preview_json;
    // State that could not be put back is the one thing about a rebuild a person needs to see: the
    // log keeps every outcome, but these lines also go to the Messages panel (principle #22).
    let mut unrestored: Vec<String> = Vec::new();
    let outcome: Result<String, String> = with_preview(|preview| {
        let factory = WidgetFactory::new_with_defaults();
        let make = crate::app::run::preview::control_creator(&factory);
        // T14.03: capture what the live session would lose across a rebuild and put it back on the
        // controls the update kept. Focus, row scroll and typed text are read from the library's own
        // channels (`focused_widget`, the `scroll_row` property, the `text` property), and a text is
        // only called "typed" when it differs from the page declaration, so a rebuilt control keeps
        // the value the document gives it.
        let capture = preview.capture_state();
        let mut restore =
            |preview: &crate::app::preview::preview_service::Preview,
             capture: &crate::app::preview::state_keep::Capture| {
                match preview.restore_state(capture) {
                    Ok(outcomes) if !outcomes.is_empty() => {
                        let lines: Vec<String> =
                            outcomes.iter().map(|outcome| outcome.render()).collect();
                        unrestored.extend(
                            outcomes
                                .iter()
                                .filter(|outcome| !outcome.is_restored())
                                .map(|outcome| outcome.render()),
                        );
                        crate::host::log::emit(
                            crate::host::log::Level::Info,
                            "app::run::workbench-preview-state",
                            &format!(
                                "restored {} live state(s): {}",
                                outcomes.len(),
                                lines.join("; ")
                            ),
                        );
                    }
                    Ok(_) => {}
                    Err(reason) => crate::host::log::emit(
                        crate::host::log::Level::Warn,
                        "app::run::workbench-preview-state",
                        &format!("live state could not be restored: {reason}"),
                    ),
                }
                match preview.restore_input(capture) {
                    Ok(outcomes) if !outcomes.is_empty() => {
                        let lines: Vec<String> =
                            outcomes.iter().map(|outcome| outcome.render()).collect();
                        unrestored.extend(
                            outcomes
                                .iter()
                                .filter(|outcome| !outcome.is_restored())
                                .map(|outcome| outcome.render()),
                        );
                        crate::host::log::emit(
                            crate::host::log::Level::Info,
                            "app::run::workbench-preview-state",
                            &format!(
                                "restored {} typed text(s): {}",
                                outcomes.len(),
                                lines.join("; ")
                            ),
                        );
                    }
                    Ok(_) => {}
                    Err(reason) => crate::host::log::emit(
                        crate::host::log::Level::Warn,
                        "app::run::workbench-preview-state",
                        &format!("typed text could not be restored: {reason}"),
                    ),
                }
            };
        if mounted {
            let result = preview.update_json(&json, &make);
            restore(preview, &capture);
            result.map(|report| {
                tr_args(
                    "ui.status.preview-updated",
                    &[
                        ("patches", &report.patches.to_string()),
                        ("updates", &report.updates.to_string()),
                    ],
                )
            })
        } else {
            let result = preview.mount_json(&json, &make);
            restore(preview, &capture);
            result.map(|report| {
                tr_args(
                    "ui.status.preview-mounted",
                    &[
                        ("widgets", &report.widgets_created.to_string()),
                        ("issues", &report.issues.to_string()),
                    ],
                )
            })
        }
    });
    for line in unrestored {
        push_message(
            gui,
            format!("{} {line}", tr("ui.status.preview-state-not-restored")),
        );
    }
    match outcome {
        Ok(message) => {
            gui.preview_json = true;
            refresh_preview(gui);
            message
        }
        Err(reason) => tr_args("ui.status.preview-rebuild-failed", &[("reason", &reason)]),
    }
}

/// Re-reads the repository and refreshes the Version-control panel.
fn vcs_refresh(gui: &mut Gui) -> String {
    refresh_vcs(gui);
    tr("ui.status.vcs-refreshed")
}

/// Clears the Application-output panel, the same call its context menu makes.
fn run_panel_clear(gui: &mut Gui) -> String {
    let message = if gui.run.clear() {
        tr("ui.status.output-cleared")
    } else {
        tr("ui.status.clear-running")
    };
    refresh_run(gui);
    message
}

/// Copies the Application-output text, the same call its context menu makes.
fn run_panel_copy(gui: &mut Gui) -> String {
    use crate::host::clipboard::{ClipboardHost, ClipboardWrite, NativeClipboard};

    let text = gui.run.text();
    match NativeClipboard::new().write_text(&text) {
        ClipboardWrite::Written => tr("ui.status.output-copied"),
        ClipboardWrite::Refused { reason } => {
            warn_failure("the run output could not be copied", &reason);
            tr_args("ui.error.copy-refused", &[("reason", &reason)])
        }
    }
}

/// Asks the running build to stop, the same call the Build panel's menu makes.
fn build_cancel(gui: &mut Gui) -> String {
    let result = {
        let Gui { tasks, build, .. } = &mut *gui;
        build.cancel(tasks.executor())
    };
    match result {
        crate::host::tasks::CancelResult::Requested => tr("ui.status.build-cancelling"),
        other => format!("cancel: {other:?}"),
    }
}

/// Copies the open project's root path, the one fact a path command can name without a selection.
fn copy_project_path(gui: &mut Gui) -> String {
    use crate::host::clipboard::{ClipboardHost, ClipboardWrite, NativeClipboard};

    let Some(session) = gui.project.as_ref() else {
        return tr("ui.project.none");
    };
    let path = session.root().as_path().to_string_lossy().into_owned();
    match NativeClipboard::new().write_text(&path) {
        ClipboardWrite::Written => tr_args("ui.status.path-copied", &[("path", &path)]),
        ClipboardWrite::Refused { reason } => {
            warn_failure("the project path could not be copied", &reason);
            tr_args("ui.error.copy-refused", &[("reason", &reason)])
        }
    }
}

/// Records a person's decision to trust the open project for this session.
///
/// Trust is what makes a write possible; it is never granted by opening a directory. The command
/// is the consent surface, so choosing it is the decision the permission record stores.
fn project_trust(gui: &mut Gui) -> String {
    use crate::project::lifecycle::session::TrustDecision;

    let Some(session) = gui.project.as_ref() else {
        return tr("ui.project.none");
    };
    let outcome = session.decide(&gui.permissions, TrustDecision::Trust);
    if outcome.trusted() {
        tr("ui.status.project-trusted")
    } else {
        tr_args(
            "ui.error.project-trust-refused",
            &[("reason", outcome.message())],
        )
    }
}

/// Saves the focused document back to the file it was opened from.
fn file_save(gui: &mut Gui) -> String {
    let Some((document, text)) = active_document(gui) else {
        return tr("ui.status.no-editor");
    };
    match save_document(gui, &document, &text) {
        Ok(path) => tr_args("ui.status.saved", &[("path", &path)]),
        Err(reason) => tr_args("ui.error.not-saved", &[("reason", &reason)]),
    }
}

/// Saves every open document that has a disk baseline.
fn file_save_all(gui: &mut Gui) -> String {
    let documents: Vec<(String, String)> = gui
        .groups
        .documents()
        .into_iter()
        .filter(|document| gui.saved_texts.contains_key(*document))
        .filter_map(|document| {
            gui.groups
                .text(document)
                .map(|text| (document.to_owned(), text.to_owned()))
        })
        .collect();
    let mut saved = 0usize;
    let mut failures = Vec::new();
    for (document, text) in documents {
        match save_document(gui, &document, &text) {
            Ok(_) => saved += 1,
            Err(reason) => failures.push(format!("{document}: {reason}")),
        }
    }
    if failures.is_empty() {
        tr_args("ui.status.saved-all", &[("count", &saved.to_string())])
    } else {
        tr_args("ui.error.save-failed", &[("reason", &failures.join("; "))])
    }
}

/// Writes one document back to disk after checking trust, scope and external changes.
///
/// The baseline is the text read when the document was opened (or the text last saved). A disk
/// that no longer matches it is refused rather than overwritten: another program's edit is not
/// this window's to discard. The write itself is the same atomic replace the project writer uses.
fn save_document(gui: &mut Gui, document: &str, text: &str) -> Result<String, String> {
    let Some(session) = gui.project.as_ref() else {
        return Err(tr("ui.project.none"));
    };
    if !session.may_write(&gui.permissions) {
        return Err(tr("ui.error.project-not-trusted"));
    }
    let root = session.root();
    let path = if Path::new(document).is_absolute() {
        PathBuf::from(document)
    } else {
        root.resolve(document).map_err(|error| error.message())?
    };
    let relative = path
        .strip_prefix(root.as_path())
        .map_err(|_| tr("ui.error.save-outside-root"))?
        .to_string_lossy()
        .into_owned();
    let Some(baseline) = gui.saved_texts.get(document) else {
        return Err(tr("ui.error.save-no-baseline"));
    };
    let disk = std::fs::read_to_string(&path).map_err(|error| error.to_string())?;
    if &disk != baseline {
        return Err(tr("ui.error.save-conflict"));
    }
    crate::project::record::json::write_atomically(&path, text)
        .map_err(|error| error.to_string())?;
    gui.saved_texts.insert(document.to_owned(), text.to_owned());
    for group in gui.groups.group_ids() {
        if let Some(tabs) = gui.groups.tabs_mut(group) {
            tabs.mark_dirty(document, false);
        }
    }
    // The title's marker follows the buffer that was just written; without this the window would
    // say "unsaved" about a document whose file it just replaced.
    if gui.workbench.document() == Some(document) {
        gui.workbench.mark_clean();
    }
    Ok(relative)
}

/// The project file a new name may be written to, after trust and root checks.
fn save_target(gui: &Gui, requested: &str) -> Result<(PathBuf, String), String> {
    let Some(session) = gui.project.as_ref() else {
        return Err(tr("ui.project.none"));
    };
    if !session.may_write(&gui.permissions) {
        return Err(tr("ui.error.project-not-trusted"));
    }
    let requested = requested.trim();
    if requested.is_empty() {
        return Err(tr("ui.error.save-target-empty"));
    }
    let root = session.root();
    let path = root
        .resolve_for_creation(requested)
        .map_err(|error| error.message())?;
    let relative = path
        .strip_prefix(root.as_path())
        .map_err(|_| tr("ui.error.save-outside-root"))?
        .to_string_lossy()
        .into_owned();
    Ok((path, relative))
}

/// Writes the focused document under a new name and continues there.
fn apply_save_as(gui: &mut Gui, requested: &str) {
    let Some((document, text)) = active_document(gui) else {
        gui.message = tr("ui.status.no-editor");
        update_status(gui);
        return;
    };
    let (path, relative) = match save_target(gui, requested) {
        Ok(found) => found,
        Err(reason) => {
            gui.message = tr_args("ui.error.not-saved", &[("reason", &reason)]);
            update_status(gui);
            return;
        }
    };
    if path.exists() {
        gui.message = tr_args(
            "ui.error.not-saved",
            &[("reason", &tr("ui.error.save-target-exists"))],
        );
        update_status(gui);
        return;
    }
    if let Err(error) = crate::project::record::json::write_atomically(&path, &text) {
        gui.message = tr_args("ui.error.not-saved", &[("reason", &error.to_string())]);
        update_status(gui);
        return;
    }
    remember_saved_text(gui, &relative, &text);
    // Save As changes the document's identity: the old tab is closed and the new file is shown.
    for group in gui.groups.group_ids() {
        if let Some(tabs) = gui.groups.tabs_mut(group) {
            tabs.close_document(&document);
        }
    }
    gui.groups.pin(relative.clone());
    if let Err(reason) = gui.groups.type_into(text) {
        gui.message = tr_args("ui.error.not-saved", &[("reason", &reason)]);
        update_status(gui);
        return;
    }
    gui.sync_editors();
    gui.message = tr_args("ui.status.saved-as", &[("path", &relative)]);
    update_status(gui);
}

/// Writes a copy of the focused document without changing the active document.
fn apply_save_copy(gui: &mut Gui, requested: &str) {
    let Some((_document, text)) = active_document(gui) else {
        gui.message = tr("ui.status.no-editor");
        update_status(gui);
        return;
    };
    let (path, relative) = match save_target(gui, requested) {
        Ok(found) => found,
        Err(reason) => {
            gui.message = tr_args("ui.error.not-saved", &[("reason", &reason)]);
            update_status(gui);
            return;
        }
    };
    if path.exists() {
        gui.message = tr_args(
            "ui.error.not-saved",
            &[("reason", &tr("ui.error.save-target-exists"))],
        );
        update_status(gui);
        return;
    }
    if let Err(error) = crate::project::record::json::write_atomically(&path, &text) {
        gui.message = tr_args("ui.error.not-saved", &[("reason", &error.to_string())]);
        update_status(gui);
        return;
    }
    // The copy is a file on disk; it does not become the active document and no tab opens for it.
    remember_saved_text(gui, &relative, &text);
    gui.message = tr_args("ui.status.saved-copy", &[("path", &relative)]);
    update_status(gui);
}

/// Hands the project root to the system's file manager through the host opener.
fn reveal_project(gui: &mut Gui) -> String {
    use crate::host::open_url::{OpenTarget, PlatformOpener, UrlOpener};

    let Some(session) = gui.project.as_ref() else {
        return tr("ui.project.none");
    };
    let path = session.root().as_path().to_path_buf();
    let opener = match PlatformOpener::for_this_platform() {
        Ok(opener) => opener,
        Err(refusal) => {
            return tr_args("ui.error.reveal-refused", &[("reason", &refusal.message())]);
        }
    };
    match opener.open(&OpenTarget::Path(path.clone())) {
        Ok(outcome) if outcome.started_the_handler() => tr_args(
            "ui.status.revealed",
            &[("path", &path.display().to_string())],
        ),
        Ok(outcome) => tr_args("ui.error.reveal-refused", &[("reason", &outcome.message())]),
        Err(refusal) => tr_args("ui.error.reveal-refused", &[("reason", &refusal.message())]),
    }
}

/// Hands the focused editor's document to the system's file manager.
///
/// A file manager shows a directory rather than selecting a file portably, so the document's own
/// directory is what is handed off. A document with no file on disk (a generated buffer) is
/// refused rather than revealing a path that does not exist.
fn reveal_current_file(gui: &mut Gui) -> String {
    use crate::host::open_url::{OpenTarget, PlatformOpener, UrlOpener};

    let Some(session) = gui.project.as_ref() else {
        return tr("ui.project.none");
    };
    let Some(document) = active_document(gui).map(|(document, _)| document) else {
        return tr("ui.error.no-current-document");
    };
    let path = if Path::new(&document).is_absolute() {
        PathBuf::from(&document)
    } else {
        session.root().as_path().join(&document)
    };
    if !path.is_file() {
        return tr_args(
            "ui.error.symbol-not-jumped",
            &[(
                "reason",
                "the focused document has no file on disk to reveal",
            )],
        );
    }
    let directory = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| path.clone());
    let opener = match PlatformOpener::for_this_platform() {
        Ok(opener) => opener,
        Err(refusal) => {
            return tr_args(
                "ui.error.file-reveal-refused",
                &[("reason", &refusal.message())],
            );
        }
    };
    match opener.open(&OpenTarget::Path(directory.clone())) {
        Ok(outcome) if outcome.started_the_handler() => tr_args(
            "ui.status.revealed",
            &[("path", &directory.display().to_string())],
        ),
        Ok(outcome) => tr_args(
            "ui.error.file-reveal-refused",
            &[("reason", &outcome.message())],
        ),
        Err(refusal) => tr_args(
            "ui.error.file-reveal-refused",
            &[("reason", &refusal.message())],
        ),
    }
}

/// Copies the build log to the clipboard, the same call the Build panel's menu makes.
fn build_copy_output(gui: &mut Gui) -> String {
    use crate::host::clipboard::{ClipboardHost, ClipboardWrite, NativeClipboard};

    let text = gui.build.rows().join("\n");
    match NativeClipboard::new().write_text(&text) {
        ClipboardWrite::Written => tr("ui.status.output-copied"),
        ClipboardWrite::Refused { reason } => {
            warn_failure("the build log could not be copied", &reason);
            tr_args("ui.error.copy-refused", &[("reason", &reason)])
        }
    }
}

/// Starts the Application-output panel's program, the same call its context menu makes.
fn run_panel_start(gui: &mut Gui) -> String {
    let program = gui.run_program.clone();
    let root = gui
        .project
        .as_ref()
        .map(|project| project.root().as_path().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let message = match gui.run.start(program, root) {
        Ok(()) => tr("ui.status.running"),
        Err(reason) => reason,
    };
    refresh_run(gui);
    message
}

/// Asks the running program to stop, the same call the panel's context menu makes.
fn run_panel_stop(gui: &mut Gui) -> String {
    if gui.run.stop() {
        tr("ui.status.stop-requested")
    } else {
        tr("ui.status.not-running")
    }
}

/// Submits a build of the open project, the same submission the Build panel's menu makes.
///
/// Without a project there is no manifest to name, and building the directory the designer happens
/// to run in would be a different project than the one on screen — so it is refused by name.
fn start_build(gui: &mut Gui, action: crate::app::build::compile::Action) -> String {
    let Some(root) = gui
        .project
        .as_ref()
        .map(|project| project.root().as_path().to_path_buf())
    else {
        return tr("ui.status.build-no-project");
    };
    let started = {
        let Gui {
            tasks,
            build,
            build_owner,
            ..
        } = &mut *gui;
        build.start(tasks.executor(), *build_owner, &root, action)
    };
    match started {
        Ok(()) => {
            let rows = gui.build.rows();
            set_list_rows(gui.build_list, rows);
            tr("ui.status.building")
        }
        Err(reason) => reason,
    }
}

/// Copies the cargo command the last build actually ran.
fn build_copy_command(gui: &mut Gui) -> String {
    use crate::host::clipboard::{ClipboardHost, ClipboardWrite, NativeClipboard};

    let Some(command) = gui
        .build
        .rows()
        .into_iter()
        .find_map(|row| row.strip_prefix("command: ").map(str::to_owned))
    else {
        return tr("ui.error.build-command-unavailable");
    };
    match NativeClipboard::new().write_text(&command) {
        ClipboardWrite::Written => {
            tr_args("ui.status.build-command-copied", &[("command", &command)])
        }
        ClipboardWrite::Refused { reason } => {
            warn_failure("the build command could not be copied", &reason);
            tr_args("ui.error.copy-refused", &[("reason", &reason)])
        }
    }
}

/// Opens the first source location the build rows recorded.
fn build_locate_first_error(gui: &mut Gui) -> String {
    let Some(root) = project_root(gui) else {
        return tr("ui.project.none");
    };
    let Some((path, line, column)) = gui.build.first_error_location() else {
        return tr("ui.error.first-error-not-found");
    };
    let path = if Path::new(&path).is_absolute() {
        PathBuf::from(&path)
    } else {
        root.join(&path)
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) => {
            return tr_args(
                "ui.error.navigation-failed",
                &[("reason", &error.to_string())],
            );
        }
    };
    let document = path
        .strip_prefix(&root)
        .map(|relative| relative.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_string_lossy().into_owned());
    let (line, column) = (line.saturating_sub(1), column.saturating_sub(1));
    remember_saved_text(gui, &document, &text);
    if !show_document_at(gui, &document, Some(&text), line, column, true) {
        return tr("ui.source.no-editor");
    }
    tr_args(
        "ui.status.first-error-opened",
        &[("path", &document), ("line", &(line + 1).to_string())],
    )
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

/// Opens a generated help document in the focused editor group.
fn open_help_document(gui: &mut Gui, name: &str, text: String) {
    gui.workbench.set_mode(Mode::Code);
    gui.groups.pin(name);
    if gui.groups.text(name) != Some(text.as_str())
        && let Err(reason) = gui.groups.type_into(text)
    {
        warn_failure("a help document could not be written", reason);
    }
    gui.sync_editors();
    gui.message = tr("ui.status.ready");
    update_status(gui);
}

/// The keyboard reference, built from the same keymap the window and audit use.
fn shortcut_reference_text() -> String {
    let keymap = window_keymap();
    let mut text = format!("# {}\n\n", tr("ui.help.shortcuts"));
    for binding in keymap.bindings() {
        text.push_str(&format!(
            "- `{}` — `{}`\n",
            binding.chord(),
            binding.command()
        ));
    }
    text
}

/// The control catalogue reference, built from the captured contract.
fn control_reference_text(catalog: Option<&Catalog>) -> String {
    let mut text = format!("# {}\n\n", tr("ui.help.controls"));
    let Some(catalog) = catalog else {
        text.push_str(&tr("ui.help.controls-unavailable"));
        text.push('\n');
        return text;
    };
    text.push_str(&format!("{} kind(s)\n\n", catalog.len()));
    for category in catalog.categories() {
        text.push_str(&format!("## {category}\n\n"));
        for descriptor in catalog.in_category(category) {
            text.push_str(&format!(
                "- `{}` — properties {}, events {}, commands {}, children {}\n",
                descriptor.id,
                descriptor.properties.len(),
                descriptor.events.len(),
                descriptor.commands.len(),
                descriptor.children.len(),
            ));
        }
        text.push('\n');
    }
    text
}

/// `name=value&name=value` after the `?` in a menu action.
///
/// The menu's action string is the one surface that carries an argument; the registry still validates
/// it, so a value the command does not accept is refused there rather than reaching a handler.
fn parse_action_arguments(query: &str) -> BTreeMap<String, String> {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect()
}

/// Shows or hides one window panel.
///
/// A dock slot shows one panel at a time: the panels sharing this slot are hidden before the new one
/// appears, so two panels cannot draw over each other.
fn toggle_panel(gui: &mut Gui, id: &str) {
    if gui.panels.layout().shown().iter().any(|shown| shown == id) {
        if let Err(error) = gui.panels.hide(id) {
            warn_failure("a panel could not be hidden", error.message());
        } else {
            remember_closed(gui, id);
        }
        gui.message = tr_args("ui.status.hidden", &[("id", id)]);
    } else {
        if let Some(area) = gui.panel_areas.get(id).copied() {
            let siblings: Vec<String> = gui
                .panel_areas
                .iter()
                .filter(|(other, other_area)| other.as_str() != id && **other_area == area)
                .map(|(other, _)| other.clone())
                .collect();
            for sibling in siblings {
                if gui
                    .panels
                    .layout()
                    .shown()
                    .iter()
                    .any(|shown| shown == &sibling)
                    && let Err(error) = gui.panels.hide(&sibling)
                {
                    warn_failure("a panel could not be hidden", error.message());
                }
            }
        }
        if gui.panels.show(id).is_ok() {
            gui.closed_panels.retain(|other| other != id);
            gui.message = tr_args("ui.status.shown", &[("id", id)]);
        } else {
            gui.message = tr_args("ui.status.no-panel", &[("id", id)]);
        }
        match id {
            "inspection" => refresh_inspection(gui),
            "vcs" => refresh_vcs(gui),
            "project" => refresh_project(gui),
            "dependencies" => refresh_dependencies(gui),
            "breakpoints" => refresh_breakpoints(gui),
            "stack" => refresh_stack(gui),
            "variables" => refresh_variables(gui),
            "actions" => refresh_actions(gui),
            "notifications" => refresh_notifications(gui),
            "application" => refresh_application(gui),
            "history" => refresh_history(gui),
            "extensions" => refresh_extensions(gui),
            "tests" => refresh_tests(gui),
            "tasks" => refresh_tasks(gui),
            "build" => refresh_build(gui),
            "run" => refresh_run(gui),
            "terminal" => refresh_terminal(gui),
            "preview" => refresh_preview(gui),
            "hot" => refresh_hot(gui),
            "states" => refresh_states(gui),
            "items" => refresh_items(gui),
            "components" => refresh_components(gui),
            "timeline" => refresh_timeline(gui),
            "ai" => refresh_ai(gui),
            "source" => refresh_source(gui),
            "layout" => refresh_layout_panel(gui),
            _ => {}
        }
    }
    gui.sync_panels();
    gui.sync_editors();
}

fn apply_action(gui: &mut Gui, action: &str) {
    gui.apply_pending_editor_text();
    if let Some(raw) = action.strip_prefix("command:") {
        let (id, arguments) = match raw.split_once('?') {
            Some((id, query)) => (id, parse_action_arguments(query)),
            None => (raw, BTreeMap::new()),
        };
        let mut call = Call::to(id);
        for (name, value) in arguments {
            call = call.with(name, value);
        }
        run_call(gui, &call);
        return;
    }
    if let Some(id) = action.strip_prefix("toggle:") {
        toggle_panel(gui, id);
        return;
    }
    match action {
        "palette:open" => open_palette(gui),
        "display:toolbar" => toggle_chrome(gui, true),
        "display:status-bar" => toggle_chrome(gui, false),
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
        "group:split-down" => {
            let document = gui
                .groups
                .tabs(gui.groups.focus())
                .and_then(|tabs| tabs.active().map(|tab| tab.document().to_owned()));
            gui.groups.split(Axis::TopBottom);
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
        "group:previous" => {
            gui.groups.focus_previous();
            gui.message = tr_args(
                "ui.status.focus-group",
                &[("group", &gui.groups.focus().get().to_string())],
            );
        }
        "group:merge" => match gui.groups.close_focus_group() {
            Ok(moved) => {
                gui.message = tr_args("ui.status.group-merged", &[("moved", &moved.to_string())]);
                gui.sync_editors();
            }
            Err(reason) => {
                gui.message = tr_args("ui.status.group-not-merged", &[("reason", &reason)]);
            }
        },
        "help:welcome" => open_onboarding(gui),
        "help:shortcuts" => {
            let text = shortcut_reference_text();
            open_help_document(gui, "SHORTCUTS.md", text);
        }
        "help:commands" => {
            let text = onboarding_text(&gui.registry);
            open_help_document(gui, "COMMANDS.md", text);
        }
        "help:controls" => {
            let text = control_reference_text(gui.widget_catalog.as_ref());
            open_help_document(gui, "CONTROLS.md", text);
        }
        "quit" => {
            // Quitting is the same question as closing: an explicit "quit" is still a request that
            // would discard buffers, so it goes through the one guard rather than a second, silently
            // different path. A clean window quits at once.
            if window_close_allowed(gui) {
                gui.message = tr("ui.status.closing");
                rust_widgets::quit();
            }
        }
        other => gui.message = tr_args("ui.status.unknown-action", &[("action", other)]),
    }
    refresh_history_commands(gui);
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
            gui.message = debug_command(&mut gui, command);
            update_status(&gui);
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
    // The three split panels read the same session the console does; they are filled whether or
    // not they are visible so switching to one shows the last report, not a stale list.
    let breakpoints = debug_section_rows(gui.debug.breakpoints());
    let mut stack = gui.debug.threads();
    stack.extend(gui.debug.frames());
    let stack = debug_section_rows(stack);
    let variables = debug_section_rows(gui.debug.variables());
    let lists = (gui.breakpoints_list, gui.stack_list, gui.variables_list);
    set_list_rows(lists.0, breakpoints);
    set_list_rows(lists.1, stack);
    set_list_rows(lists.2, variables);
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
    project_path: Option<PathBuf>,
    ai_journal: Option<PathBuf>,
    lsp_program: Option<String>,
    lsp_arguments: Vec<String>,
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
    // The open project is read before the panels are registered: whether `panel.project` is shown
    // follows from the session, not from a second configuration value.
    let project = match project_path {
        Some(directory) => match open_project(&directory) {
            Ok(session) => {
                if let Some(name) = session.name() {
                    workbench.open_project(name);
                }
                Some(session)
            }
            Err(reason) => {
                warn_failure("the project could not be opened", reason);
                None
            }
        },
        None => None,
    };
    if project.is_none() {
        workbench.open_project("demo-project");
    }
    workbench.open_document("src/main.rs");
    workbench.set_target("desktop");

    // The widget library (blue21 W-4): rows come from the pinned contract. A refusal withholds the
    // panel's service, so the palette is hidden rather than shown empty (T04.02's gate).
    let mut widget_catalog_available = false;
    let mut widget_catalog = match Catalog::from_host_contract("desktop") {
        Ok(catalog) => {
            widget_catalog_available = true;
            crate::host::log::emit(
                crate::host::log::Level::Info,
                "app::run::workbench-widgets",
                &format!("widget library: {} control(s)", catalog.len()),
            );
            Some(catalog)
        }
        Err(refusal) => {
            warn_failure(
                "the widget library could not be captured",
                refusal.message(),
            );
            None
        }
    };
    // The favorites are the person's, not the build's: they live in the host data directory and a
    // missing file is "none yet". A file that cannot be read is reported and the library still
    // opens without favorites, rather than losing the list on the next save.
    let widget_favorites_path = crate::host::paths::HostDirectories::from_environment()
        .ok()
        .map(|directories| {
            directories
                .path(crate::host::paths::HostDirectory::Data)
                .join("widget-favorites.json")
        });
    let widget_favorites = match widget_favorites_path.as_deref() {
        Some(path) => match Favorites::load(path) {
            Ok(favorites) => favorites,
            Err(reason) => {
                warn_failure("the widget favorites could not be read", reason);
                Favorites::new()
            }
        },
        None => Favorites::new(),
    };
    if let Some(catalog) = widget_catalog.as_mut() {
        for id in widget_favorites.ids() {
            catalog.favorite(id, true);
        }
    }
    let (widget_rows, widget_visible) = match widget_catalog.as_ref() {
        Some(catalog) => {
            let rows = widget_rows(catalog, "");
            let visible = rows.iter().map(|(id, _)| id.clone()).collect();
            (
                rows.into_iter().map(|(_, label)| label).collect::<Vec<_>>(),
                visible,
            )
        }
        None => (Vec::new(), Vec::new()),
    };

    // The services the built-in panels need are the ones this slice actually has: a property
    // schema and diagnostics come from models that exist. The Outline is the scene's object tree
    // (F09), so it needs no service; the source outline (F07) is the separate `source` panel, whose
    // document comes from the editor buffer and whose symbols come from `--lsp` (when given). A
    // panel whose service is missing stays registered but is not shown (T04.02's gate), and the log
    // names it.
    let mut services: BTreeSet<String> = ["property-schema", "diagnostics"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    if widget_catalog_available {
        services.insert("widget-catalog".to_owned());
    }
    let mut panels = built_in_panels();
    for (id, _title, dock) in EXTRA_PANELS {
        let mut spec = PanelSpec::new(id, tr(panel_key(id)), dock, 300);
        if id == "widgets" {
            spec = spec.needing("widget-catalog");
        }
        if let Err(error) = panels.register(spec) {
            warn_failure("an extra panel could not be registered", error.message());
        }
    }
    let mut show_ids = vec![
        "widgets",
        "pages",
        "assets",
        "outline",
        "inspector",
        "messages",
        "problems",
        "events",
        "data",
        "theme",
        "debug",
        "help",
        "search",
        "environments",
        "targets",
    ];
    // The Project panel is shown when a project was opened; the Dependencies panel shares its dock
    // slot with Environments/Version control and stays closed until asked for.
    if project.is_some() {
        show_ids.push("project");
    }
    for id in show_ids {
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

    let rename_slot: Arc<Mutex<Option<RenameRequest>>> = Arc::new(Mutex::new(None));
    let prompt_slot: Arc<Mutex<Option<Prompt>>> = Arc::new(Mutex::new(None));
    let close_slot: Arc<Mutex<Option<CloseAnswer>>> = Arc::new(Mutex::new(None));
    let gui = Arc::new(Mutex::new(Gui {
        workbench,
        panels,
        groups,
        default_layout,
        layout_path: layout_path.clone(),
        panel_widgets: BTreeMap::new(),
        panel_areas: BTreeMap::new(),
        inspection_list: 0,
        vcs_list: 0,
        vcs_menu: 0,
        project,
        saved_texts: BTreeMap::new(),
        history_items: Vec::new(),
        permissions: crate::host::permission::PermissionService::new(),
        project_list: 0,
        dependencies_list: 0,
        breakpoints_list: 0,
        stack_list: 0,
        variables_list: 0,
        actions_list: 0,
        notifications: crate::app::run::workbench_notifications::Notifications::new(),
        notifications_list: 0,
        notifications_menu: 0,
        notification_row: None,
        application_list: 0,
        extensions: crate::app::extension::Registry::new(),
        extensions_usage: crate::app::extension::manager::Usage::new(),
        extensions_list: 0,
        history_list: 0,
        states_list: 0,
        states_source: None,
        states: None,
        items_list: 0,
        components_list: 0,
        timeline_list: 0,
        ai_journal,
        ai_list: 0,
        source: crate::app::run::workbench_outline::SourceOutline::new(lsp_program, lsp_arguments),
        source_list: 0,
        source_field: 0,
        source_filter: String::new(),
        jump_back: Vec::new(),
        jump_forward: Vec::new(),
        closed_documents: Vec::new(),
        pending_resource_cleanup: Vec::new(),
        command_history: Vec::new(),
        layout_list: 0,
        tests: crate::app::run::workbench_tests::TestsState::new(),
        tests_list: 0,
        tests_menu: 0,
        tasks: crate::app::run::workbench_tasks::Tasks::new(),
        tasks_list: 0,
        build: crate::app::run::workbench_build::BuildState::new(),
        build_owner: crate::host::tasks::OwnerId::new(),
        build_list: 0,
        build_menu: 0,
        run: crate::app::run::workbench_run::RunState::new(),
        run_list: 0,
        run_field: 0,
        run_program: String::new(),
        run_menu: 0,
        terminal: crate::app::run::workbench_terminal::TerminalState::new(),
        terminal_list: 0,
        terminal_field: 0,
        terminal_input: String::new(),
        terminal_menu: 0,
        preview_list: 0,
        preview_menu: 0,
        preview_json: false,
        hot: crate::app::preview::hot_update::Updater::new("workbench"),
        hot_list: 0,
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
        scene_clipboard: None,
        widget_catalog,
        widget_favorites,
        widget_favorites_path,
        widget_visible,
        widget_query: String::new(),
        widget_row: None,
        widget_menu: 0,
        widgets_search: 0,
        widgets_list: 0,
        search_field: 0,
        search_list: 0,
        search_hits: Vec::new(),
        messages: Vec::new(),
        messages_list: 0,
        diagnostic_row: None,
        diagnostic_menu: 0,
        canvas_panel: 0,
        canvas_surface: 0,
        tooltip: 0,
        color_panel: 0,
        color_picker: 0,
        color_well: 0,
        color_history: 0,
        color_swatch: 0,
        color_row: 0,
        drag_active: false,
        drag_start: None,
        pointer: None,
        drag: None,
        ghost: 0,
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
            view: CanvasView::default(),
        }))),
        canvas_view: CanvasView::default(),
        canvas_scheduled: Arc::new(AtomicBool::new(false)),
        canvas_revision: u64::MAX,
        outline_index: BTreeMap::new(),
        outline_content_revision: u64::MAX,
        outline_syncing: Arc::new(AtomicBool::new(false)),
        syncing_editors: Arc::new(AtomicBool::new(false)),
        editor_documents: Arc::new(Mutex::new(BTreeMap::new())),
        editor_changed: Arc::new(Mutex::new(BTreeSet::new())),
        translatable: Vec::new(),
        toolbar: 0,
        toolbar_items: Vec::new(),
        show_toolbar: true,
        show_status: true,
        layout_locked: false,
        closed_panels: Vec::new(),
        fullscreen: false,
        zen: None,
        self_weak: Weak::new(),
        context_menu: 0,
        scene_menu: 0,
        editor_menu: 0,
        rename_dialog: 0,
        prompt_dialog: 0,
        prompt_slot: Arc::clone(&prompt_slot),
        close_dialog: 0,
        close_slot: Arc::clone(&close_slot),
        rename_slot: Arc::clone(&rename_slot),
    }));
    if let Ok(mut state) = gui.lock() {
        state.self_weak = Arc::downgrade(&gui);
    }
    schedule_rename(Arc::clone(&gui), rename_slot);
    schedule_prompt(Arc::clone(&gui), prompt_slot);
    schedule_close(Arc::clone(&gui), close_slot);

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
    // The keys this window routes, so a menu leaf shows the same chord the palette does.
    let menu_shortcuts = routed_shortcuts();
    // The menu tree is the registry's, not a second hand-spelled copy: every leaf names the command
    // it binds, and the label is the identified vocabulary key (`menu.<id>.item.<command>`). This is
    // what blue21 L-2/T05.02 asked for — the same menu the catalog covers is the one the window paints.
    for menu in crate::app::commands::menu::built_in() {
        // A menu of planned leaves is a ledger entry, not a surface: the native menu has no
        // disabled switch, so painting it would show enabled entries with no handler.
        if !menu.is_paintable() {
            continue;
        }
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
                crate::app::commands::menu::Item::Command { title, command } => {
                    add_menu_leaf(
                        &handle,
                        &handle_menu,
                        &format!("menu.{}.item.{command}", menu.id()),
                        command,
                        title,
                        &menu_shortcuts,
                        &mut menu_actions,
                        &gui,
                    );
                }
                // A planned leaf is ledger material (T05.02's task column), not a menu item. The
                // native menu has no enabled/disabled switch in this revision, so drawing it would
                // produce an enabled entry whose click only says "no handler" — exactly the
                // dangling enabled menu T05.08 forbids. `--app-menu` still lists it with its task.
                crate::app::commands::menu::Item::Planned { .. } => {}
                crate::app::commands::menu::Item::Submenu { id, title, items } => {
                    // A branch with no command leaf is not painted, for the same reason as a menu
                    // of planned leaves. The Panels branch is the one exception: its window-only
                    // toggles are appended below and are not registry commands.
                    let window_panels = menu.id() == "view" && id == "panels";
                    if !window_panels
                        && !items
                            .iter()
                            .any(crate::app::commands::menu::Item::is_paintable)
                    {
                        continue;
                    }
                    // A real nested menu, not flattened leaves: the library creates the branch row
                    // and the child popup, and the host opens the child on `submenu_requested`
                    // (feedback C-4 / blue22 H-3).
                    let title_key = format!("menu.{}.submenu.{id}.title", menu.id());
                    let translated = tr(&title_key);
                    let label = if translated == title_key {
                        title.clone()
                    } else {
                        translated
                    };
                    let branch = handle_menu.item_count();
                    let child_menu = handle_menu.new_submenu(&label, 0, 0, 0, 0);
                    if let Ok(mut gui) = gui.lock() {
                        gui.translatable.push(Translatable {
                            id: child_menu.raw_id(),
                            key: title_key,
                            kind: LabelKind::MenuTitle,
                        });
                    }
                    for child in items {
                        // A planned submenu leaf is not painted, for the same reason as above.
                        if let crate::app::commands::menu::Item::Command {
                            title: leaf,
                            command,
                        } = child
                        {
                            add_menu_leaf(
                                &handle,
                                &child_menu,
                                &format!("menu.{}.submenu.{id}.item.{command}", menu.id()),
                                command,
                                leaf,
                                &menu_shortcuts,
                                &mut menu_actions,
                                &gui,
                            );
                        }
                    }
                    // The window-only panels belong in the registry's own Panels submenu, not
                    // flattened beside its branch row.
                    if menu.id() == "view" && id == "panels" {
                        for (key, host) in [
                            ("ui.panel.pages", "pages"),
                            ("ui.panel.widgets", "widgets"),
                            ("ui.panel.assets", "assets"),
                            ("ui.panel.theme", "theme"),
                            ("ui.panel.outline", "outline"),
                            ("ui.panel.inspector", "inspector"),
                            ("ui.panel.messages", "messages"),
                            ("ui.panel.problems", "problems"),
                            ("ui.panel.events", "events"),
                            ("ui.panel.data", "data"),
                            ("ui.panel.states", "states"),
                            ("ui.panel.items", "items"),
                            ("ui.panel.components", "components"),
                            ("ui.panel.timeline", "timeline"),
                            ("ui.panel.ai", "ai"),
                            ("ui.panel.source-outline", "source"),
                            ("ui.panel.layout", "layout"),
                            ("ui.panel.debug", "debug"),
                            ("ui.panel.environments", "environments"),
                            ("ui.panel.targets", "targets"),
                            ("ui.panel.inspection", "inspection"),
                            ("ui.panel.vcs", "vcs"),
                            ("ui.panel.project", "project"),
                            ("ui.panel.dependencies", "dependencies"),
                            ("ui.panel.breakpoints", "breakpoints"),
                            ("ui.panel.stack", "stack"),
                            ("ui.panel.variables", "variables"),
                            ("ui.panel.actions", "actions"),
                            ("ui.panel.notifications", "notifications"),
                            ("ui.panel.application", "application"),
                            ("ui.panel.history", "history"),
                            ("ui.panel.extensions", "extensions"),
                            ("ui.panel.tests", "tests"),
                            ("ui.panel.tasks", "tasks"),
                            ("ui.panel.build", "build"),
                            ("ui.panel.run", "run"),
                            ("ui.panel.terminal", "terminal"),
                            ("ui.panel.preview", "preview"),
                            ("ui.panel.hot", "hot"),
                            ("ui.panel.help", "help"),
                            ("ui.panel.search", "search"),
                        ] {
                            let action = panel_action(host);
                            let item = handle.new_menu_item(&child_menu, &tr(key), None);
                            menu_actions.insert(item.raw_id(), action);
                            if let Ok(mut gui) = gui.lock() {
                                gui.translatable.push(Translatable {
                                    id: item.raw_id(),
                                    key: key.to_owned(),
                                    kind: LabelKind::Text,
                                });
                            }
                        }
                    }
                    let parent_id = handle_menu.raw_id();
                    let child_id = child_menu.raw_id();
                    subscribe_string(parent_id, "submenu_requested", move |requested| {
                        if requested != label {
                            return;
                        }
                        let Some(row) = MenuHandle::from_raw(parent_id).row_rect(branch) else {
                            crate::host::log::emit(
                                crate::host::log::Level::Warn,
                                "app::run::workbench",
                                "a submenu branch row has no painted rectangle",
                            );
                            return;
                        };
                        let opened = MenuHandle::from_raw(child_id).open_at(
                            row.x,
                            row.y.saturating_add(row.height as i32),
                            Rect::new(0, 0, WIDTH, HEIGHT),
                        );
                        if !opened {
                            warn_failure(
                                "a submenu could not open",
                                format!("{requested} was activated but its popup refused to open"),
                            );
                        }
                    });
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
                menu_actions.insert(item.raw_id(), "command:file.exit".to_owned());
                if let Ok(mut gui) = gui.lock() {
                    gui.translatable.push(Translatable {
                        id: item.raw_id(),
                        key: "ui.menu.quit".to_owned(),
                        kind: LabelKind::Text,
                    });
                }
            }
            "view" => {
                // The panel toggles live in the registry's View > Panels branch, which the
                // `Submenu` arm above fills; nothing is appended flat to View.
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
                ("ui.menu.save-layout", "command:view.save-layout"),
                ("ui.menu.load-layout", "command:view.layout-load"),
                ("ui.menu.reset-layout", "command:view.reset-layout"),
                ("ui.menu.lock-layout", "command:view.layout-lock"),
                ("ui.menu.zen", "command:view.layout-zen"),
                ("ui.menu.fullscreen", "command:view.layout-fullscreen"),
            ],
        ),
        (
            "ui.menu.editor",
            vec![
                ("ui.menu.split-right", "command:view.split"),
                ("ui.menu.split-down", "command:view.split-down"),
                ("ui.menu.next-group", "command:view.next-group"),
                ("ui.menu.previous-group", "command:view.previous-group"),
                ("ui.menu.merge-group", "command:view.merge-group"),
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
        ("ui.toolbar.split", "command:view.split"),
        ("ui.toolbar.next-group", "command:view.next-group"),
        ("ui.toolbar.save-layout", "command:view.save-layout"),
        ("ui.toolbar.reset-layout", "command:view.reset-layout"),
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
    for (key, action) in INSPECTOR_MENU_KEYS {
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
    for (key, action) in SCENE_MENU_KEYS {
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

    // The widget library's context menu: one toggle, applied to the row under the pointer. The
    // favorites are persisted by the host (not the catalog), so a restart keeps them.
    let widget_menu = handle.new_menu(&bar, "", 0, 0, 0, 0);
    let favorite_item = handle.new_menu_item(&widget_menu, &tr("ui.context.favorite"), None);
    {
        let gui = Arc::clone(&gui);
        let label = tr("ui.context.favorite");
        widget_menu.on_triggered(move |triggered| {
            if triggered != label {
                return;
            }
            if let Ok(mut gui) = gui.lock()
                && let Some(row) = gui.widget_row
            {
                toggle_widget_favorite(&mut gui, row, true);
            }
        });
    }
    if let Ok(mut gui) = gui.lock() {
        gui.widget_menu = widget_menu.raw_id();
        gui.translatable.push(Translatable {
            id: favorite_item.raw_id(),
            key: "ui.context.favorite".to_owned(),
            kind: LabelKind::Text,
        });
    }

    // The editor-tab context menu (blue21 §3.3). The tab strip is the library's `TabWidget`, which
    // publishes `context_menu_requested` and `tab_index_at`; the menu therefore acts on the tab under
    // the pointer, not on whichever tab happened to be active.
    let editor_menu = handle.new_menu(&bar, "", 0, 0, 0, 0);
    let mut editor_actions: BTreeMap<String, String> = BTreeMap::new();
    for (key, action) in EDITOR_MENU_KEYS {
        let label = tr(key);
        handle.new_menu_item(&editor_menu, &label, None);
        editor_actions.insert(label, action.to_owned());
    }
    {
        let gui = Arc::clone(&gui);
        let actions = editor_actions.clone();
        editor_menu.on_triggered(move |label| {
            if let Ok(mut gui) = gui.lock() {
                let Some(action) = actions.get(&label).cloned() else {
                    return;
                };
                editor_context_action(&mut gui, &action, &label);
            }
        });
    }
    if let Ok(mut gui) = gui.lock() {
        gui.editor_menu = editor_menu.raw_id();
    }

    // The Messages (diagnostics/log) context menu. It is row-scoped: the ListBox reports the row
    // under the pointer, and Copy works on that row rather than on whichever row was last selected.
    let diagnostic_menu = handle.new_menu(&bar, "", 0, 0, 0, 0);
    let mut diagnostic_actions: BTreeMap<String, String> = BTreeMap::new();
    for (key, action) in DIAGNOSTIC_MENU_KEYS {
        let label = tr(key);
        handle.new_menu_item(&diagnostic_menu, &label, None);
        diagnostic_actions.insert(label, action.to_owned());
    }
    {
        let gui = Arc::clone(&gui);
        let actions = diagnostic_actions.clone();
        diagnostic_menu.on_triggered(move |label| {
            if let Ok(mut gui) = gui.lock() {
                let Some(action) = actions.get(&label).cloned() else {
                    return;
                };
                diagnostic_context_action(&mut gui, &action, &label);
            }
        });
    }
    if let Ok(mut gui) = gui.lock() {
        gui.diagnostic_menu = diagnostic_menu.raw_id();
    }

    // Docked panels. The model owns which panels exist and whether they are shown; the responsive
    // layout owns where they sit, and each panel's own `BoxLayout` owns its title/list split. A
    // panel therefore keeps its internal proportions when the window scales.
    let outline_items = {
        let gui = gui.lock().expect("the workbench state is not poisoned");
        gui.scene.rows()
    };
    let (project_items, dependency_items) = {
        let gui = gui.lock().expect("the workbench state is not poisoned");
        (project_rows(&gui.project), dependency_rows(&gui.project))
    };
    let action_items = {
        let gui = gui.lock().expect("the workbench state is not poisoned");
        crate::app::run::workbench_actions::rows(
            &gui.registry,
            &crate::app::commands::shortcuts::built_in(),
        )
    };
    let application_items = {
        let gui = gui.lock().expect("the workbench state is not poisoned");
        application_rows(&gui)
    };
    // The inspection and version-control panels are filled when they are shown, not at startup:
    // one runs the design checks and the other runs Git, and neither is in the first-run layout.
    let mut inspector_controls: Option<InspectorControls> = None;
    let mut events_controls: Option<EventControls> = None;
    let mut data_controls: Option<DataControls> = None;
    let mut pages_controls: Option<PageControls> = None;
    let mut outline_list: Option<u64> = None;
    let mut source_list: Option<u64> = None;
    let mut source_field: Option<u64> = None;
    let mut widgets_list: Option<u64> = None;
    let mut widgets_search: Option<u64> = None;
    let mut help_list: Option<u64> = None;
    let mut search_field: Option<u64> = None;
    let mut search_list: Option<u64> = None;
    let mut run_field: Option<u64> = None;
    let mut terminal_field: Option<u64> = None;
    for (id, _title, rect, area) in [
        (
            "pages",
            "Pages",
            (0, CONTENT_Y, 260, 180),
            Area::LeftPanel { index: 0, count: 5 },
        ),
        (
            // The project tree shares the Pages dock: both are project navigation, one at a time.
            "project",
            "Project",
            (0, CONTENT_Y, 260, 180),
            Area::LeftPanel { index: 0, count: 5 },
        ),
        (
            "outline",
            "Outline",
            (0, 254, 260, 180),
            Area::LeftPanel { index: 1, count: 5 },
        ),
        (
            // The source outline is the same idea over a document than over the scene: it shares
            // the object tree's dock slot, and one of the two is shown at a time.
            "source",
            "Source outline",
            (0, 254, 260, 180),
            Area::LeftPanel { index: 1, count: 5 },
        ),
        (
            "events",
            "Events",
            (0, 442, 260, 200),
            Area::LeftPanel { index: 2, count: 5 },
        ),
        (
            "widgets",
            "Widgets",
            (0, 650, 260, 140),
            Area::LeftPanel { index: 3, count: 5 },
        ),
        (
            "assets",
            "Assets",
            (0, 790, 260, 150),
            Area::LeftPanel { index: 4, count: 5 },
        ),
        (
            "inspector",
            "Inspector",
            (WIDTH as i32 - 320, CONTENT_Y, 320, 210),
            Area::RightPanel { index: 0, count: 4 },
        ),
        (
            "data",
            "Data & State",
            (WIDTH as i32 - 320, 288, 320, 200),
            Area::RightPanel { index: 1, count: 4 },
        ),
        (
            // The state machine shares the Data dock: the declaration belongs to the same page.
            "states",
            "States",
            (WIDTH as i32 - 320, 288, 320, 200),
            Area::RightPanel { index: 1, count: 4 },
        ),
        (
            // Collection contents belong to the same page as its state machine.
            "items",
            "Collection contents",
            (WIDTH as i32 - 320, 288, 320, 200),
            Area::RightPanel { index: 1, count: 4 },
        ),
        (
            // Components belong to the same page as its state machine and collections.
            "components",
            "Components",
            (WIDTH as i32 - 320, 288, 320, 200),
            Area::RightPanel { index: 1, count: 4 },
        ),
        (
            // Transitions belong to the same page as its state machine and collections.
            "timeline",
            "Timeline",
            (WIDTH as i32 - 320, 288, 320, 200),
            Area::RightPanel { index: 1, count: 4 },
        ),
        (
            // The AI session is a service view, not page content; it shares the service dock.
            "ai",
            "AI session",
            (WIDTH as i32 - 320, 288, 320, 200),
            Area::RightPanel { index: 1, count: 4 },
        ),
        (
            // The layout inspector reads the same page as the state and collection views.
            "layout",
            "Layout inspector",
            (WIDTH as i32 - 320, 288, 320, 200),
            Area::RightPanel { index: 1, count: 4 },
        ),
        (
            "theme",
            "Theme",
            (WIDTH as i32 - 320, 496, 320, 110),
            Area::RightPanel { index: 2, count: 4 },
        ),
        (
            // The debug console and its breakpoints/stack/variables views share one dock slot:
            // one is shown at a time, and the View/Panels toggle switches between them.
            "debug",
            "Debug",
            (WIDTH as i32 - 320, 614, 320, 146),
            Area::RightPanel { index: 3, count: 4 },
        ),
        (
            "breakpoints",
            "Breakpoints",
            (WIDTH as i32 - 320, 614, 320, 146),
            Area::RightPanel { index: 3, count: 4 },
        ),
        (
            "stack",
            "Call stack",
            (WIDTH as i32 - 320, 614, 320, 146),
            Area::RightPanel { index: 3, count: 4 },
        ),
        (
            "variables",
            "Variables",
            (WIDTH as i32 - 320, 614, 320, 146),
            Area::RightPanel { index: 3, count: 4 },
        ),
        (
            // Targets moved to the project dock so the debug group has its own sidebar slot.
            "targets",
            "Targets",
            (683, 645, 133, 120),
            Area::BottomPanel { index: 3, count: 5 },
        ),
        (
            "messages",
            "Messages",
            (260, 645, 133, 120),
            Area::BottomPanel { index: 0, count: 5 },
        ),
        (
            // Actions shares the output dock.
            "actions",
            "Actions",
            (260, 645, 133, 120),
            Area::BottomPanel { index: 0, count: 5 },
        ),
        (
            // Notifications shares the output dock too.
            "notifications",
            "Notifications",
            (260, 645, 133, 120),
            Area::BottomPanel { index: 0, count: 5 },
        ),
        (
            // Background tasks shares the output dock.
            "tasks",
            "Background tasks",
            (260, 645, 133, 120),
            Area::BottomPanel { index: 0, count: 5 },
        ),
        (
            "problems",
            "Problems",
            (401, 645, 133, 120),
            Area::BottomPanel { index: 1, count: 5 },
        ),
        (
            // The inspection panel shares the quality dock with Problems: one is shown at a
            // time, and the View/Panels toggle switches between them.
            "inspection",
            "Inspection",
            (401, 645, 133, 120),
            Area::BottomPanel { index: 1, count: 5 },
        ),
        (
            // Tests shares the quality dock too.
            "tests",
            "Tests",
            (401, 645, 133, 120),
            Area::BottomPanel { index: 1, count: 5 },
        ),
        (
            "search",
            "Search",
            (542, 645, 133, 120),
            Area::BottomPanel { index: 2, count: 5 },
        ),
        (
            // Application output shares the editor-output dock with Search.
            "run",
            "Application output",
            (542, 645, 133, 120),
            Area::BottomPanel { index: 2, count: 5 },
        ),
        (
            // The terminal shares the same output dock.
            "terminal",
            "Terminal",
            (542, 645, 133, 120),
            Area::BottomPanel { index: 2, count: 5 },
        ),
        (
            // Preview and hot update share the project dock with the other service views.
            "preview",
            "Preview",
            (683, 645, 133, 120),
            Area::BottomPanel { index: 3, count: 5 },
        ),
        (
            "hot",
            "Hot update",
            (683, 645, 133, 120),
            Area::BottomPanel { index: 3, count: 5 },
        ),
        (
            "environments",
            "Environments",
            (683, 645, 133, 120),
            Area::BottomPanel { index: 3, count: 5 },
        ),
        (
            // Version control shares the project dock with Environments.
            "vcs",
            "Version control",
            (683, 645, 133, 120),
            Area::BottomPanel { index: 3, count: 5 },
        ),
        (
            // Dependencies shares the same project dock.
            "dependencies",
            "Dependencies",
            (683, 645, 133, 120),
            Area::BottomPanel { index: 3, count: 5 },
        ),
        (
            "help",
            "Help",
            (824, 645, 136, 120),
            Area::BottomPanel { index: 4, count: 5 },
        ),
        (
            // Application structure shares the Help dock.
            "application",
            "Application structure",
            (824, 645, 136, 120),
            Area::BottomPanel { index: 4, count: 5 },
        ),
        (
            // Extensions shares the Help dock too.
            "extensions",
            "Extensions",
            (824, 645, 136, 120),
            Area::BottomPanel { index: 4, count: 5 },
        ),
        (
            // Build output shares the Help dock.
            "build",
            "Build output",
            (824, 645, 136, 120),
            Area::BottomPanel { index: 4, count: 5 },
        ),
        (
            // History shares the project dock with the other repository views.
            "history",
            "History",
            (683, 645, 133, 120),
            Area::BottomPanel { index: 3, count: 5 },
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
        let list_id = if id == "inspector" || id == "data" || id == "assets" || id == "theme" {
            0
        } else {
            // The widget library and the source outline put a field above their list, so their rows
            // start lower.
            let top = if id == "widgets"
                || id == "search"
                || id == "run"
                || id == "terminal"
                || id == "source"
            {
                rect.1 + 60
            } else {
                rect.1 + 30
            };
            let height = rect.3
                - if id == "widgets"
                    || id == "search"
                    || id == "run"
                    || id == "terminal"
                    || id == "source"
                {
                    68
                } else {
                    38
                };
            let list = panel.new_list_box(rect.0 + 8, top, rect.2 - 16, height);
            let focus_name = match id {
                "pages" => "pages list",
                "outline" => "outline list",
                "source" => "source outline list",
                "layout" => "layout inspector list",
                "widgets" => "widget library",
                "events" => "events list",
                "messages" => "messages list",
                "problems" => "problems list",
                "help" => "help topics",
                "search" => "search results",
                "environments" => "environments list",
                "targets" => "targets list",
                "inspection" => "inspection list",
                "vcs" => "version-control list",
                "project" => "project tree",
                "dependencies" => "dependencies list",
                "breakpoints" => "breakpoints list",
                "stack" => "call stack list",
                "variables" => "variables list",
                "actions" => "actions list",
                "notifications" => "notifications list",
                "application" => "application structure list",
                "history" => "history list",
                "extensions" => "extensions list",
                "tests" => "tests list",
                "tasks" => "background tasks list",
                "build" => "build output list",
                "run" => "application output list",
                "terminal" => "terminal output",
                "preview" => "preview list",
                "hot" => "hot update list",
                "states" => "states list",
                "items" => "collection contents list",
                "components" => "components list",
                "timeline" => "timeline list",
                "ai" => "ai session list",
                _ => "panel list",
            };
            watch_focus(list.raw_id(), focus_name);
            list.raw_id()
        };
        let mut inner = BoxLayout::new(Orientation::Vertical, 4, 8);
        let mut extra_widgets: Vec<u64> = Vec::new();
        if id == "help" {
            // The Help panel is a list of real help documents, not a static text block: each row
            // opens the same content the commands and catalog produce.
            let topics = vec![
                tr("ui.help.welcome"),
                tr("ui.help.shortcuts"),
                tr("ui.help.commands"),
                tr("ui.help.controls"),
            ];
            set_list_rows(list_id, topics);
            help_list = Some(list_id);
        }
        if id == "search" {
            // The Search panel queries the documents the editor already has open; it is not a
            // second index and cannot show a file the window has not read.
            let field = panel.new_line_edit("", rect.0 + 8, rect.1 + 30, rect.2 - 16, 26);
            field.set_placeholder(&tr("ui.placeholder.search"));
            watch_focus(field.raw_id(), "search field");
            inner.add_widget(field.raw_id(), 0);
            inner.set_constraints(field.raw_id(), LayoutConstraints::new(26, Some(26)));
            inner.set_size_policy(field.raw_id(), SizePolicy::Fixed);
            extra_widgets.push(field.raw_id());
            search_field = Some(field.raw_id());
        }
        if id == "terminal" {
            // The field is the terminal's stdin: Return sends the line to the shell and clears it.
            let field = panel.new_line_edit("", rect.0 + 8, rect.1 + 30, rect.2 - 16, 26);
            field.set_placeholder(&tr("ui.placeholder.terminal"));
            watch_focus(field.raw_id(), "terminal input");
            inner.add_widget(field.raw_id(), 0);
            inner.set_constraints(field.raw_id(), LayoutConstraints::new(26, Some(26)));
            inner.set_size_policy(field.raw_id(), SizePolicy::Fixed);
            extra_widgets.push(field.raw_id());
            terminal_field = Some(field.raw_id());
        }
        if id == "run" {
            // The program is the path the person names; this build does not guess an artifact path
            // from a package name. The field is a real `LineEdit` and its text is the run config.
            let field = panel.new_line_edit("", rect.0 + 8, rect.1 + 30, rect.2 - 16, 26);
            field.set_placeholder(&tr("ui.placeholder.program"));
            watch_focus(field.raw_id(), "run program");
            inner.add_widget(field.raw_id(), 0);
            inner.set_constraints(field.raw_id(), LayoutConstraints::new(26, Some(26)));
            inner.set_size_policy(field.raw_id(), SizePolicy::Fixed);
            extra_widgets.push(field.raw_id());
            run_field = Some(field.raw_id());
        }
        if id == "widgets" {
            // The search field is the catalog's own search, not a second filter: typing narrows the
            // same 270-row contract list an empty query shows.
            let field = panel.new_line_edit("", rect.0 + 8, rect.1 + 30, rect.2 - 16, 26);
            field.set_placeholder(&tr("ui.placeholder.widget-search"));
            watch_focus(field.raw_id(), "widget search");
            inner.add_widget(field.raw_id(), 0);
            inner.set_constraints(field.raw_id(), LayoutConstraints::new(26, Some(26)));
            inner.set_size_policy(field.raw_id(), SizePolicy::Fixed);
            extra_widgets.push(field.raw_id());
            widgets_search = Some(field.raw_id());
        }
        if id == "source" {
            // The filter narrows the rows the last refresh read; it never re-requests symbols, so
            // typing costs one list rebuild and no server round trip.
            let field = panel.new_line_edit("", rect.0 + 8, rect.1 + 30, rect.2 - 16, 26);
            field.set_placeholder(&tr("ui.placeholder.source-filter"));
            watch_focus(field.raw_id(), "source outline filter");
            inner.add_widget(field.raw_id(), 0);
            inner.set_constraints(field.raw_id(), LayoutConstraints::new(26, Some(26)));
            inner.set_size_policy(field.raw_id(), SizePolicy::Fixed);
            extra_widgets.push(field.raw_id());
            source_field = Some(field.raw_id());
        }
        if list_id != 0 {
            inner.add_widget(list_id, 1);
            inner.set_size_policy(list_id, SizePolicy::Expanding);
        }
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
        if id == "assets" {
            // H-21: the gallery is the asset surface, and it owns the library's
            // `ThumbnailCache` (its strip is painted from cached thumbnails). The sample
            // assets are generated RGBA so the thumbnail path is real rather than a row of
            // placeholders; a project asset added later uses the same `set_images` path.
            let gallery = handle
                .mount_widget_by_name(
                    "image_gallery",
                    Rect::new(
                        rect.0 + 8,
                        rect.1 + 30,
                        rect.2 - 16,
                        rect.3.saturating_sub(58).max(24),
                    ),
                    "",
                )
                .expect("the library registers the image gallery widget");
            watch_focus(gallery.raw_id(), "asset gallery");
            let images = sample_asset_images();
            let filled = with_widget_mut(gallery.raw_id(), move |widget| {
                let Some(gallery) =
                    (widget as &mut dyn Any).downcast_mut::<rust_widgets::ImageGallery>()
                else {
                    return false;
                };
                gallery.set_images(images);
                true
            })
            .unwrap_or(false);
            if !filled {
                warn_failure(
                    "the asset gallery could not be filled",
                    "the mounted control was not an ImageGallery",
                );
            }
            let note = panel.new_label(
                &tr("ui.assets.note"),
                rect.0 + 8,
                rect.1 + rect.3 as i32 - 22,
                rect.2 - 16,
                16,
            );
            inner.add_widget(gallery.raw_id(), 1);
            inner.set_size_policy(gallery.raw_id(), SizePolicy::Expanding);
            inner.add_widget(note.raw_id(), 0);
            inner.set_constraints(note.raw_id(), LayoutConstraints::new(16, Some(16)));
            inner.set_size_policy(note.raw_id(), SizePolicy::Fixed);
            extra_widgets.extend([gallery.raw_id(), note.raw_id()]);
        }
        if id == "theme" {
            // H-22/H-23/H-24: a real theme surface. The combo switches the active theme with
            // the library's faded path, the metrics combo writes `Theme::metrics_profile`, and
            // the curve editor edits `Theme::motion.easing` as a cubic Bézier.
            let names = theme_names();
            let row = panel.new_panel(rect.0 + 8, rect.1 + 30, rect.2 - 16, 26);
            let themes = row.new_combo_box(0, 0, rect.2 - 96, 26);
            for name in &names {
                themes.add_item(name);
            }
            if let Some(index) =
                current_theme_name().and_then(|name| names.iter().position(|n| *n == name))
            {
                themes.set_current_index(index);
            }
            let switch = row.new_button(&tr("ui.button.switch-theme"), 0, 0, 72, 26);
            let mut row_layout = BoxLayout::new(Orientation::Horizontal, 4, 0);
            row_layout.add_widget(themes.raw_id(), 1);
            row_layout.set_size_policy(themes.raw_id(), SizePolicy::Expanding);
            row_layout.add_widget(switch.raw_id(), 0);
            row_layout.set_constraints(switch.raw_id(), LayoutConstraints::new(72, Some(72)));
            row_layout.set_size_policy(switch.raw_id(), SizePolicy::Fixed);
            row.set_layout(Box::new(row_layout));

            let metrics_row = panel.new_panel(rect.0 + 8, rect.1 + 62, rect.2 - 16, 26);
            let metrics_label = metrics_row.new_label(&tr("ui.theme.metrics"), 0, 0, 72, 26);
            let metrics = metrics_row.new_combo_box(0, 0, rect.2 - 88, 26);
            for token in ["material", "macos", "cupertino"] {
                metrics.add_item(token);
            }
            if let Some(index) = current_metrics_index() {
                metrics.set_current_index(index);
            }
            let mut metrics_layout = BoxLayout::new(Orientation::Horizontal, 4, 0);
            metrics_layout.add_widget(metrics_label.raw_id(), 0);
            metrics_layout
                .set_constraints(metrics_label.raw_id(), LayoutConstraints::new(72, Some(72)));
            metrics_layout.set_size_policy(metrics_label.raw_id(), SizePolicy::Fixed);
            metrics_layout.add_widget(metrics.raw_id(), 1);
            metrics_layout.set_size_policy(metrics.raw_id(), SizePolicy::Expanding);
            metrics_row.set_layout(Box::new(metrics_layout));

            let curve = handle
                .mount_widget_by_name(
                    "bezier_curve_editor",
                    Rect::new(
                        rect.0 + 8,
                        rect.1 + 94,
                        rect.2 - 16,
                        rect.3.saturating_sub(140).max(40),
                    ),
                    "",
                )
                .expect("the library registers the bezier curve editor");
            watch_focus(curve.raw_id(), "theme motion curve");
            seed_curve_editor(curve.raw_id());
            let note = panel.new_label(
                &tr("ui.theme.curve"),
                rect.0 + 8,
                rect.1 + rect.3 as i32 - 22,
                rect.2 - 16,
                16,
            );
            for (widget, height) in [
                (row.raw_id(), 26),
                (metrics_row.raw_id(), 26),
                (note.raw_id(), 16),
            ] {
                inner.add_widget(widget, 0);
                inner.set_constraints(widget, LayoutConstraints::new(height, Some(height)));
                inner.set_size_policy(widget, SizePolicy::Fixed);
            }
            inner.add_widget(curve.raw_id(), 1);
            inner.set_size_policy(curve.raw_id(), SizePolicy::Expanding);

            {
                let state = Arc::clone(&gui);
                let themes_id = themes.raw_id();
                ButtonHandle::from_raw(switch.raw_id()).on_click(move || {
                    let combo = rust_widgets::app::ComboBoxHandle::from_raw(themes_id);
                    let name = combo
                        .current_index()
                        .and_then(|index| combo.item_text(index));
                    if let Some(name) = name
                        && let Ok(mut gui) = state.lock()
                    {
                        switch_theme_faded(&mut gui, &name);
                    }
                    schedule_theme_crossfade(Arc::clone(&state));
                });
            }
            {
                let state = Arc::clone(&gui);
                subscribe_index(metrics.raw_id(), "current_index_changed", move |index| {
                    if let Ok(mut gui) = state.lock() {
                        apply_metrics_profile(&mut gui, index as usize);
                    }
                });
            }
            {
                let state = Arc::clone(&gui);
                let _ = with_widget_mut(curve.raw_id(), move |widget| {
                    let Some(curve) =
                        (widget as &mut dyn Any).downcast_mut::<rust_widgets::BezierCurveEditor>()
                    else {
                        return false;
                    };
                    curve
                        .curve_changed
                        .connect(move |value: Arc<((f32, f32), (f32, f32))>| {
                            let ((x1, y1), (x2, y2)) = *value;
                            if let Ok(mut gui) = state.lock() {
                                apply_motion_curve(&mut gui, x1, y1, x2, y2);
                            }
                        });
                    true
                });
            }
            if let Ok(mut gui) = gui.lock() {
                gui.translatable.push(Translatable {
                    id: switch.raw_id(),
                    key: "ui.button.switch-theme".to_owned(),
                    kind: LabelKind::Text,
                });
                gui.translatable.push(Translatable {
                    id: metrics_label.raw_id(),
                    key: "ui.theme.metrics".to_owned(),
                    kind: LabelKind::Text,
                });
            }
            extra_widgets.extend([
                row.raw_id(),
                themes.raw_id(),
                switch.raw_id(),
                metrics_row.raw_id(),
                metrics_label.raw_id(),
                metrics.raw_id(),
                curve.raw_id(),
                note.raw_id(),
            ]);
        }
        panel.set_layout(Box::new(inner));
        let items: Vec<String> = match id {
            "outline" => outline_items.clone(),
            "widgets" => widget_rows.clone(),
            "messages" => vec![tr("ui.status.ready")],
            "project" => project_items.clone(),
            "dependencies" => dependency_items.clone(),
            "actions" => action_items.clone(),
            "application" => application_items.clone(),
            // The Events, Data and Pages lists are filled from their models once the controls exist;
            // the Inspector and Data panels own a property grid instead of a list.
            "events" | "data" | "pages" | "inspector" | "assets" | "theme" | "help" | "search"
            | "inspection" | "vcs" | "breakpoints" | "stack" | "variables" | "notifications"
            | "history" | "extensions" | "tests" | "tasks" | "build" | "run" | "terminal"
            | "preview" | "hot" | "states" | "items" | "components" | "timeline" | "ai"
            | "source" => Vec::new(),
            "environments" => environment_rows(),
            "targets" => target_rows(),
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
        if id == "source" {
            source_list = Some(list_id);
        }
        if id == "widgets" {
            widgets_list = Some(list_id);
        }
        if id == "search" {
            search_list = Some(list_id);
        }
        if let Ok(mut gui) = gui.lock() {
            gui.placements.push((panel.raw_id(), area));
            gui.panel_areas.insert(id.to_owned(), area);
            let entry = gui.panel_widgets.entry(id.to_owned()).or_default();
            entry.push(panel.raw_id());
            entry.push(list_id);
            entry.extend(extra_widgets);
            if id == "messages" {
                gui.messages_list = list_id;
                gui.messages = vec![tr("ui.status.ready")];
            }
            if id == "inspection" {
                gui.inspection_list = list_id;
            }
            if id == "vcs" {
                gui.vcs_list = list_id;
            }
            if id == "project" {
                gui.project_list = list_id;
            }
            if id == "dependencies" {
                gui.dependencies_list = list_id;
            }
            if id == "breakpoints" {
                gui.breakpoints_list = list_id;
            }
            if id == "stack" {
                gui.stack_list = list_id;
            }
            if id == "variables" {
                gui.variables_list = list_id;
            }
            if id == "actions" {
                gui.actions_list = list_id;
            }
            if id == "notifications" {
                gui.notifications_list = list_id;
            }
            if id == "application" {
                gui.application_list = list_id;
            }
            if id == "history" {
                gui.history_list = list_id;
            }
            if id == "extensions" {
                gui.extensions_list = list_id;
            }
            if id == "tests" {
                gui.tests_list = list_id;
            }
            if id == "tasks" {
                gui.tasks_list = list_id;
            }
            if id == "build" {
                gui.build_list = list_id;
            }
            if id == "run" {
                gui.run_list = list_id;
            }
            if id == "terminal" {
                gui.terminal_list = list_id;
            }
            if id == "preview" {
                gui.preview_list = list_id;
            }
            if id == "hot" {
                gui.hot_list = list_id;
            }
            if id == "states" {
                gui.states_list = list_id;
            }
            if id == "items" {
                gui.items_list = list_id;
            }
            if id == "components" {
                gui.components_list = list_id;
            }
            if id == "timeline" {
                gui.timeline_list = list_id;
            }
            if id == "ai" {
                gui.ai_list = list_id;
            }
            if id == "source" {
                gui.source_list = list_id;
            }
            if id == "layout" {
                gui.layout_list = list_id;
            }
        }
    }

    // The Messages log's context menu: Copy the row under the pointer, copy every line, or clear.
    let messages_list = match gui.lock() {
        Ok(gui) => gui.messages_list,
        Err(_) => 0,
    };
    if messages_list != 0 {
        let menu_gui = Arc::clone(&gui);
        subscribe_string(messages_list, "context_menu_requested", move |payload| {
            let Some((x, y)) = point_of(payload) else {
                return;
            };
            let Some(index) = list_row_at(messages_list, x, y) else {
                return;
            };
            if let Ok(mut gui) = menu_gui.lock() {
                gui.diagnostic_row = Some(index);
                let _ = rust_widgets::list_box_set_current_index(messages_list, index);
                if gui.diagnostic_menu != 0
                    && !MenuHandle::from_raw(gui.diagnostic_menu).open_at(
                        x,
                        y,
                        Rect::new(0, 0, WIDTH, HEIGHT),
                    )
                {
                    warn_failure("the diagnostics context menu could not open", payload);
                }
            }
        });
    }

    // The Notifications list's context menu: dismiss the row under the pointer.
    let notifications_list = match gui.lock() {
        Ok(gui) => gui.notifications_list,
        Err(_) => 0,
    };
    if notifications_list != 0 {
        let notifications_menu = handle.new_menu(&bar, "", 0, 0, 0, 0);
        let dismiss = handle.new_menu_item(
            &notifications_menu,
            &tr("ui.context.notification.dismiss"),
            None,
        );
        {
            let state = Arc::clone(&gui);
            notifications_menu.on_triggered(move |_label| {
                if let Ok(mut gui) = state.lock()
                    && let Some(index) = gui.notification_row
                    && gui.notifications.dismiss(index)
                {
                    refresh_notifications(&mut gui);
                }
            });
        }
        if let Ok(mut gui) = gui.lock() {
            gui.notifications_menu = notifications_menu.raw_id();
            gui.translatable.push(Translatable {
                id: dismiss.raw_id(),
                key: "ui.context.notification.dismiss".to_owned(),
                kind: LabelKind::Text,
            });
        }
        let menu_gui = Arc::clone(&gui);
        subscribe_string(
            notifications_list,
            "context_menu_requested",
            move |payload| {
                let Some((x, y)) = point_of(payload) else {
                    return;
                };
                let Some(index) = list_row_at(notifications_list, x, y) else {
                    return;
                };
                if let Ok(mut gui) = menu_gui.lock() {
                    gui.notification_row = Some(index);
                    let _ = rust_widgets::list_box_set_current_index(notifications_list, index);
                    if gui.notifications_menu != 0
                        && !MenuHandle::from_raw(gui.notifications_menu).open_at(
                            x,
                            y,
                            Rect::new(0, 0, WIDTH, HEIGHT),
                        )
                    {
                        warn_failure("the notifications context menu could not open", payload);
                    }
                }
            },
        );
    }

    // The Version-control list's context menu: re-read Git now. The panel reads on show, so this is
    // the way to refresh a panel that stayed open while the repository changed (principle #22: a
    // stale read must not pass itself off as the current one).
    let vcs_list = match gui.lock() {
        Ok(gui) => gui.vcs_list,
        Err(_) => 0,
    };
    if vcs_list != 0 {
        let vcs_menu = handle.new_menu(&bar, "", 0, 0, 0, 0);
        let refresh = handle.new_menu_item(&vcs_menu, &tr("ui.context.vcs.refresh"), None);
        {
            let state = Arc::clone(&gui);
            vcs_menu.on_triggered(move |_label| {
                if let Ok(mut gui) = state.lock() {
                    gui.message = vcs_refresh(&mut gui);
                    update_status(&gui);
                }
            });
        }
        if let Ok(mut gui) = gui.lock() {
            gui.vcs_menu = vcs_menu.raw_id();
            gui.translatable.push(Translatable {
                id: refresh.raw_id(),
                key: "ui.context.vcs.refresh".to_owned(),
                kind: LabelKind::Text,
            });
        }
        let menu_gui = Arc::clone(&gui);
        subscribe_string(vcs_list, "context_menu_requested", move |payload| {
            let Some((x, y)) = point_of(payload) else {
                return;
            };
            if let Ok(gui) = menu_gui.lock()
                && gui.vcs_menu != 0
                && !MenuHandle::from_raw(gui.vcs_menu).open_at(x, y, Rect::new(0, 0, WIDTH, HEIGHT))
            {
                warn_failure("the version-control context menu could not open", payload);
            }
        });
    }

    // The Tests list's context menu: run (or re-run) the suite on a worker thread.
    let tests_list = match gui.lock() {
        Ok(gui) => gui.tests_list,
        Err(_) => 0,
    };
    if tests_list != 0 {
        let tests_menu = handle.new_menu(&bar, "", 0, 0, 0, 0);
        let mut tests_actions: BTreeMap<String, String> = BTreeMap::new();
        for (key, action) in [
            ("ui.context.tests.run", "run"),
            ("ui.context.tests.run-failed", "run-failed"),
            ("ui.context.tests.stop", "stop"),
        ] {
            let label = tr(key);
            let item = handle.new_menu_item(&tests_menu, &label, None);
            tests_actions.insert(label, action.to_owned());
            if let Ok(mut gui) = gui.lock() {
                gui.translatable.push(Translatable {
                    id: item.raw_id(),
                    key: key.to_owned(),
                    kind: LabelKind::Text,
                });
            }
        }
        {
            let state = Arc::clone(&gui);
            tests_menu.on_triggered(move |label| {
                let Some(action) = tests_actions.get(&label).cloned() else {
                    return;
                };
                if let Ok(mut gui) = state.lock() {
                    gui.message = match action.as_str() {
                        "run" => tests_run(&mut gui),
                        "run-failed" => tests_run_failed(&mut gui),
                        "stop" => tests_stop(&mut gui),
                        _ => return,
                    };
                    update_status(&gui);
                }
            });
        }
        if let Ok(mut gui) = gui.lock() {
            gui.tests_menu = tests_menu.raw_id();
        }
        let menu_gui = Arc::clone(&gui);
        subscribe_string(tests_list, "context_menu_requested", move |payload| {
            let Some((x, y)) = point_of(payload) else {
                return;
            };
            if let Ok(gui) = menu_gui.lock()
                && gui.tests_menu != 0
                && !MenuHandle::from_raw(gui.tests_menu).open_at(
                    x,
                    y,
                    Rect::new(0, 0, WIDTH, HEIGHT),
                )
            {
                warn_failure("the tests context menu could not open", payload);
            }
        });
    }

    // The Build-output list's context menu: build, or cancel the running build.
    let build_list = match gui.lock() {
        Ok(gui) => gui.build_list,
        Err(_) => 0,
    };
    if build_list != 0 {
        let build_menu = handle.new_menu(&bar, "", 0, 0, 0, 0);
        let mut build_actions: BTreeMap<String, String> = BTreeMap::new();
        for (key, action) in [
            ("ui.context.build.run", "run"),
            ("ui.context.build.check", "check"),
            ("ui.context.build.rebuild", "rebuild"),
            ("ui.context.build.clean", "clean"),
            ("ui.context.build.cancel", "cancel"),
            ("ui.context.build.copy-all", "copy-all"),
            ("ui.context.build.first-error", "first-error"),
        ] {
            let label = tr(key);
            let item = handle.new_menu_item(&build_menu, &label, None);
            build_actions.insert(label, action.to_owned());
            if let Ok(mut gui) = gui.lock() {
                gui.translatable.push(Translatable {
                    id: item.raw_id(),
                    key: key.to_owned(),
                    kind: LabelKind::Text,
                });
            }
        }
        {
            let state = Arc::clone(&gui);
            let actions = build_actions.clone();
            build_menu.on_triggered(move |label| {
                let Some(action) = actions.get(&label).cloned() else {
                    return;
                };
                if let Ok(mut gui) = state.lock() {
                    match action.as_str() {
                        // One submission, whether it arrives from this menu or from `build.run`.
                        "run" => {
                            gui.message =
                                start_build(&mut gui, crate::app::build::compile::Action::Build)
                        }
                        "check" => {
                            gui.message =
                                start_build(&mut gui, crate::app::build::compile::Action::Check)
                        }
                        "rebuild" => {
                            gui.message =
                                start_build(&mut gui, crate::app::build::compile::Action::Rebuild)
                        }
                        "clean" => {
                            gui.message =
                                start_build(&mut gui, crate::app::build::compile::Action::Clean)
                        }
                        "cancel" => gui.message = build_cancel(&mut gui),
                        "copy-all" => gui.message = build_copy_output(&mut gui),
                        "first-error" => gui.message = build_locate_first_error(&mut gui),
                        _ => {}
                    }
                    update_status(&gui);
                }
                schedule_build_poll(&state);
            });
        }
        if let Ok(mut gui) = gui.lock() {
            gui.build_menu = build_menu.raw_id();
        }
        let menu_gui = Arc::clone(&gui);
        subscribe_string(build_list, "context_menu_requested", move |payload| {
            let Some((x, y)) = point_of(payload) else {
                return;
            };
            if let Ok(gui) = menu_gui.lock()
                && gui.build_menu != 0
                && !MenuHandle::from_raw(gui.build_menu).open_at(
                    x,
                    y,
                    Rect::new(0, 0, WIDTH, HEIGHT),
                )
            {
                warn_failure("the build context menu could not open", payload);
            }
        });
    }

    // The Application-output list's context menu: start the named program, or stop the running one.
    let run_list = match gui.lock() {
        Ok(gui) => gui.run_list,
        Err(_) => 0,
    };
    if run_list != 0 {
        let run_menu = handle.new_menu(&bar, "", 0, 0, 0, 0);
        let mut run_actions: BTreeMap<String, String> = BTreeMap::new();
        for (key, action) in [
            ("ui.context.run.start", "start"),
            ("ui.context.run.stop", "stop"),
            ("ui.context.run.clear", "clear"),
            ("ui.context.run.copy-all", "copy-all"),
        ] {
            let label = tr(key);
            let item = handle.new_menu_item(&run_menu, &label, None);
            run_actions.insert(label, action.to_owned());
            if let Ok(mut gui) = gui.lock() {
                gui.translatable.push(Translatable {
                    id: item.raw_id(),
                    key: key.to_owned(),
                    kind: LabelKind::Text,
                });
            }
        }
        {
            let state = Arc::clone(&gui);
            let actions = run_actions.clone();
            run_menu.on_triggered(move |label| {
                let Some(action) = actions.get(&label).cloned() else {
                    return;
                };
                if let Ok(mut gui) = state.lock() {
                    match action.as_str() {
                        "start" => gui.message = run_panel_start(&mut gui),
                        "stop" => gui.message = run_panel_stop(&mut gui),
                        "clear" => gui.message = run_panel_clear(&mut gui),
                        "copy-all" => gui.message = run_panel_copy(&mut gui),
                        _ => {}
                    }
                    update_status(&gui);
                }
                schedule_run_poll(&state);
            });
        }
        if let Ok(mut gui) = gui.lock() {
            gui.run_menu = run_menu.raw_id();
        }
        let menu_gui = Arc::clone(&gui);
        subscribe_string(run_list, "context_menu_requested", move |payload| {
            let Some((x, y)) = point_of(payload) else {
                return;
            };
            if let Ok(gui) = menu_gui.lock()
                && gui.run_menu != 0
                && !MenuHandle::from_raw(gui.run_menu).open_at(x, y, Rect::new(0, 0, WIDTH, HEIGHT))
            {
                warn_failure("the run context menu could not open", payload);
            }
        });
    }

    // The Terminal list's context menu: start a shell session, or stop it.
    let terminal_list = match gui.lock() {
        Ok(gui) => gui.terminal_list,
        Err(_) => 0,
    };
    if terminal_list != 0 {
        let terminal_menu = handle.new_menu(&bar, "", 0, 0, 0, 0);
        let mut terminal_actions: BTreeMap<String, String> = BTreeMap::new();
        for (key, action) in [
            ("ui.context.terminal.start", "start"),
            ("ui.context.terminal.stop", "stop"),
        ] {
            let label = tr(key);
            let item = handle.new_menu_item(&terminal_menu, &label, None);
            terminal_actions.insert(label, action.to_owned());
            if let Ok(mut gui) = gui.lock() {
                gui.translatable.push(Translatable {
                    id: item.raw_id(),
                    key: key.to_owned(),
                    kind: LabelKind::Text,
                });
            }
        }
        {
            let state = Arc::clone(&gui);
            let actions = terminal_actions.clone();
            terminal_menu.on_triggered(move |label| {
                let Some(action) = actions.get(&label).cloned() else {
                    return;
                };
                if let Ok(mut gui) = state.lock() {
                    match action.as_str() {
                        "start" => gui.message = terminal_start(&mut gui),
                        "stop" => gui.message = terminal_stop(&mut gui),
                        _ => {}
                    }
                    update_status(&gui);
                }
            });
        }
        if let Ok(mut gui) = gui.lock() {
            gui.terminal_menu = terminal_menu.raw_id();
        }
        let menu_gui = Arc::clone(&gui);
        subscribe_string(terminal_list, "context_menu_requested", move |payload| {
            let Some((x, y)) = point_of(payload) else {
                return;
            };
            if let Ok(gui) = menu_gui.lock()
                && gui.terminal_menu != 0
                && !MenuHandle::from_raw(gui.terminal_menu).open_at(
                    x,
                    y,
                    Rect::new(0, 0, WIDTH, HEIGHT),
                )
            {
                warn_failure("the terminal context menu could not open", payload);
            }
        });
    }

    // The Preview list's context menu: start or stop the preview service (no process is spawned).
    let preview_list = match gui.lock() {
        Ok(gui) => gui.preview_list,
        Err(_) => 0,
    };
    if preview_list != 0 {
        let preview_menu = handle.new_menu(&bar, "", 0, 0, 0, 0);
        let mut preview_actions: BTreeMap<String, String> = BTreeMap::new();
        for (key, action) in [
            ("ui.context.preview.start", "start"),
            ("ui.context.preview.stop", "stop"),
        ] {
            let label = tr(key);
            let item = handle.new_menu_item(&preview_menu, &label, None);
            preview_actions.insert(label, action.to_owned());
            if let Ok(mut gui) = gui.lock() {
                gui.translatable.push(Translatable {
                    id: item.raw_id(),
                    key: key.to_owned(),
                    kind: LabelKind::Text,
                });
            }
        }
        {
            let state = Arc::clone(&gui);
            let actions = preview_actions.clone();
            preview_menu.on_triggered(move |label| {
                let Some(action) = actions.get(&label).cloned() else {
                    return;
                };
                if let Ok(mut gui) = state.lock() {
                    match action.as_str() {
                        "start" => {
                            gui.message = preview_start(&mut gui);
                        }
                        "stop" => {
                            gui.message = preview_stop(&mut gui);
                        }
                        _ => {}
                    }
                    refresh_preview(&mut gui);
                    update_status(&gui);
                }
            });
        }
        if let Ok(mut gui) = gui.lock() {
            gui.preview_menu = preview_menu.raw_id();
        }
        let menu_gui = Arc::clone(&gui);
        subscribe_string(preview_list, "context_menu_requested", move |payload| {
            let Some((x, y)) = point_of(payload) else {
                return;
            };
            if let Ok(gui) = menu_gui.lock()
                && gui.preview_menu != 0
                && !MenuHandle::from_raw(gui.preview_menu).open_at(
                    x,
                    y,
                    Rect::new(0, 0, WIDTH, HEIGHT),
                )
            {
                warn_failure("the preview context menu could not open", payload);
            }
        });
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
            // RW-12: the grid refuses a value its row kind cannot take (not a number, outside the
            // row's bounds, not one of a choice row's options) and says why. Painting that on the
            // row is the control's own report, not a second validation in the host.
            let gui = Arc::clone(&gui);
            subscribe_cell(grid_id, "cell_change_rejected", move |row, message| {
                if let Ok(mut gui) = gui.lock() {
                    let text = tr_args("ui.error.not-applied", &[("reason", message)]);
                    rust_widgets::set_widget_text(gui.property_error, &text);
                    set_property_row_error(gui.property_grid, row as usize, Some(&text));
                    gui.message = text;
                    update_status(&gui);
                }
            });
        }
        {
            let gui = Arc::clone(&gui);
            subscribe_cell(grid_id, "color_picker_requested", move |row, seed| {
                if let Ok(mut gui) = gui.lock() {
                    open_colour(&mut gui, row as usize, seed);
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
        let list_gui = Arc::clone(&gui);
        subscribe_index(list_id, "item_selected", move |index| {
            if let Ok(mut gui) = list_gui.lock() {
                // A row the model wrote itself is not a click; see `Gui::outline_syncing`.
                if gui.outline_syncing.load(Ordering::SeqCst) {
                    return;
                }
                let Some(node) = gui.scene.row_nodes().get(index as usize).copied() else {
                    return;
                };
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
        // The same menu the canvas opens, now row-specific: the ListBox publishes both the request
        // and the row under the pointer, so a right-click on another row selects that row first.
        let menu_gui = Arc::clone(&gui);
        subscribe_string(list_id, "context_menu_requested", move |payload| {
            let Some((x, y)) = point_of(payload) else {
                return;
            };
            let Some(index) = list_row_at(list_id, x, y) else {
                return;
            };
            if let Ok(mut gui) = menu_gui.lock() {
                let Some(node) = gui.scene.row_nodes().get(index).copied() else {
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
                    warn_failure("the outline context menu could not open", payload);
                }
            }
        });
    }

    // The widget library (blue21 W-4): a row is one control the pinned contract really registers.
    // Activation inserts it into the scene; the model refuses a control with no editable property,
    // and a refusal is shown in the status bar and the Messages panel rather than swallowed.
    if let Some(list_id) = widgets_list {
        if let Ok(mut gui) = gui.lock() {
            gui.widgets_list = list_id;
        }
        let list_gui = Arc::clone(&gui);
        subscribe_index(list_id, "item_activated", move |index| {
            if let Ok(mut gui) = list_gui.lock() {
                insert_widget_from_palette(&mut gui, index as usize);
            }
        });
        let menu_gui = Arc::clone(&gui);
        subscribe_string(list_id, "context_menu_requested", move |payload| {
            let Some((x, y)) = point_of(payload) else {
                return;
            };
            let Some(index) = list_row_at(list_id, x, y) else {
                return;
            };
            if let Ok(mut gui) = menu_gui.lock() {
                gui.widget_row = Some(index);
                let _ = rust_widgets::list_box_set_current_index(list_id, index);
                if gui.widget_menu != 0
                    && !MenuHandle::from_raw(gui.widget_menu).open_at(
                        x,
                        y,
                        Rect::new(0, 0, WIDTH, HEIGHT),
                    )
                {
                    warn_failure("the widget context menu could not open", payload);
                }
            }
        });
    }
    if let Some(list_id) = help_list {
        let help_gui = Arc::clone(&gui);
        subscribe_index(list_id, "item_activated", move |index| {
            if let Ok(mut gui) = help_gui.lock() {
                let action = match index {
                    0 => "help:welcome",
                    1 => "help:shortcuts",
                    2 => "help:commands",
                    _ => "help:controls",
                };
                apply_action(&mut gui, action);
            }
        });
    }
    if let Some(field_id) = search_field {
        if let Ok(mut gui) = gui.lock() {
            gui.search_field = field_id;
        }
        let search_gui = Arc::clone(&gui);
        subscribe_string(field_id, "text_changed", move |query| {
            if let Ok(mut gui) = search_gui.lock() {
                run_search(&mut gui, query);
            }
        });
    }
    if let Some(field_id) = run_field {
        if let Ok(mut gui) = gui.lock() {
            gui.run_field = field_id;
        }
        let run_gui = Arc::clone(&gui);
        subscribe_string(field_id, "text_changed", move |text| {
            if let Ok(mut gui) = run_gui.lock() {
                gui.run_program = text.to_owned();
            }
        });
    }
    if let Some(field_id) = terminal_field {
        if let Ok(mut gui) = gui.lock() {
            gui.terminal_field = field_id;
        }
        let terminal_gui = Arc::clone(&gui);
        subscribe_string(field_id, "text_changed", move |text| {
            if let Ok(mut gui) = terminal_gui.lock() {
                gui.terminal_input = text.to_owned();
            }
        });
        let enter_gui = Arc::clone(&gui);
        subscribe_unit(field_id, "return_pressed", move || {
            if let Ok(mut gui) = enter_gui.lock() {
                let line = format!("{}\n", gui.terminal_input);
                if gui.terminal.write(&line) {
                    gui.terminal_input.clear();
                    if gui.terminal_field != 0 {
                        rust_widgets::app::LineEditHandle::from_raw(gui.terminal_field).clear();
                    }
                }
            }
        });
    }
    if let Some(list_id) = search_list {
        if let Ok(mut gui) = gui.lock() {
            gui.search_list = list_id;
        }
        let search_gui = Arc::clone(&gui);
        subscribe_index(list_id, "item_activated", move |index| {
            if let Ok(mut gui) = search_gui.lock() {
                activate_search_hit(&mut gui, index as usize);
            }
        });
    }
    if let Some(field_id) = widgets_search {
        if let Ok(mut gui) = gui.lock() {
            gui.widgets_search = field_id;
        }
        // The subscriber locks the same `Gui`. A caller that wants to set the field's text
        // programmatically must do it on a later main-loop turn: `set_widget_text` emits this signal
        // synchronously, and taking the lock again on the same thread would deadlock the loop.
        let search_gui = Arc::clone(&gui);
        subscribe_string(field_id, "text_changed", move |query| {
            if let Ok(mut gui) = search_gui.lock() {
                filter_widgets(&mut gui, query);
            }
        });
    }
    if let Some(field_id) = source_field {
        if let Ok(mut gui) = gui.lock() {
            gui.source_field = field_id;
        }
        let filter_gui = Arc::clone(&gui);
        subscribe_string(field_id, "text_changed", move |query| {
            if let Ok(mut gui) = filter_gui.lock() {
                gui.source_filter = query.to_owned();
                let list = gui.source_list;
                let filter = gui.source_filter.clone();
                let rows = gui.source.render(&filter);
                set_list_rows(list, rows);
            }
        });
    }
    // Activating a source-outline row is a jump, not a selection: the row's symbol is where the
    // caret goes, and moving the highlight with the arrow keys must not move it.
    if let Some(list_id) = source_list {
        let list_gui = Arc::clone(&gui);
        subscribe_index(list_id, "item_activated", move |index| {
            if let Ok(mut gui) = list_gui.lock() {
                jump_to_source(&mut gui, index as usize);
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
            Area::RightPanel { index: 3, count: 4 },
        ));
        gui.panel_areas
            .insert("debug".to_owned(), Area::RightPanel { index: 3, count: 4 });
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
        let dirty_at_start = match gui.lock() {
            Ok(gui) => gui.groups.dirty_documents(),
            Err(poisoned) => poisoned.into_inner().groups.dirty_documents(),
        };
        for document in &documents {
            let label = tab_label(document, dirty_at_start.contains(document));
            if tab_strip.add_tab(&label, None).is_none() {
                warn_failure("a tab could not be added to the strip", &label);
            }
        }
        {
            let gui = Arc::clone(&gui);
            // The guard is read *before* the lock: `set_current_index` emits while the caller
            // already holds the `Gui`, so asking the `Gui` for the flag would deadlock first.
            let syncing = match gui.lock() {
                Ok(gui) => Arc::clone(&gui.syncing_editors),
                Err(poisoned) => Arc::clone(&poisoned.into_inner().syncing_editors),
            };
            subscribe_index(tab_strip.raw_id(), "current_changed", move |index| {
                if syncing.load(Ordering::SeqCst) {
                    return;
                }
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
        {
            let gui = Arc::clone(&gui);
            let strip = tab_strip.raw_id();
            subscribe_string(strip, "context_menu_requested", move |payload| {
                let Some((x, y)) = point_of(payload) else {
                    return;
                };
                let Some(index) = tab_at(strip, x, y) else {
                    return;
                };
                if let Ok(mut gui) = gui.lock() {
                    let Some(id) = gui.groups.group_ids().get(group).copied() else {
                        return;
                    };
                    if let Some(Err(reason)) =
                        gui.groups.tabs_mut(id).map(|tabs| tabs.activate(index))
                    {
                        warn_failure("a tab could not be activated before its menu", reason);
                    }
                    gui.sync_editors();
                    if gui.editor_menu != 0
                        && !MenuHandle::from_raw(gui.editor_menu).open_at(
                            x,
                            y,
                            Rect::new(0, 0, WIDTH, HEIGHT),
                        )
                    {
                        warn_failure("the editor context menu could not open", payload);
                    }
                }
            });
        }
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
        // Every committed edit is reported to the model. The library describes a keystroke as a
        // line-range change (it does not clone the document), so the report only says *which*
        // surface moved; `Gui::apply_pending_editor_text` reads its text on the next main-loop turn.
        //
        // Nothing here touches the `Gui`: the signal is delivered while a caller may already hold
        // that mutex — `sync_editors` loads a document into this very surface — so the closures only
        // write the small maps that sit outside it. The load itself is skipped through the same
        // guard the tab strips use, because a load is not a person's edit.
        {
            let (changed, syncing) = match gui.lock() {
                Ok(gui) => (
                    Arc::clone(&gui.editor_changed),
                    Arc::clone(&gui.syncing_editors),
                ),
                Err(poisoned) => {
                    let gui = poisoned.into_inner();
                    (
                        Arc::clone(&gui.editor_changed),
                        Arc::clone(&gui.syncing_editors),
                    )
                }
            };
            let editor_id = editor.raw_id();
            let wired = with_widget_mut(editor_id, move |widget| {
                let any: &mut dyn std::any::Any = widget;
                let Some(editor) = any.downcast_mut::<CodeEditor>() else {
                    return;
                };
                let per_keystroke = Arc::clone(&changed);
                let per_keystroke_guard = Arc::clone(&syncing);
                editor.text_change.connect_ref(move |_change: &TextChange| {
                    if per_keystroke_guard.load(Ordering::SeqCst) {
                        return;
                    }
                    if let Ok(mut pending) = per_keystroke.lock() {
                        pending.insert(editor_id);
                    }
                });
                let wholesale_guard = Arc::clone(&syncing);
                editor.text_changed.connect_ref(move |_text: &String| {
                    if wholesale_guard.load(Ordering::SeqCst) {
                        return;
                    }
                    if let Ok(mut pending) = changed.lock() {
                        pending.insert(editor_id);
                    }
                });
            });
            if wired.is_none() {
                warn_failure(
                    "an editor could not report its text changes",
                    format!("surface {editor_id} is not a mounted code editor"),
                );
            }
        }
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

    // The Inspector's colour editor (blue22 H-18): the library's picker, well, history and swatch
    // live in one hidden overlay. The grid asks for the picker through `color_picker_requested`
    // and still accepts a typed hex value through its own text editor.
    let color_panel = handle.new_panel(300, 130, 280, 372);
    let color_picker =
        get_control_backend_for_widget(rust_widgets::widget::WidgetKind::ColorPicker)
            .create_color_picker(color_panel.raw_id(), 0, 0, 280, 240);
    let color_well = get_control_backend_for_widget(rust_widgets::widget::WidgetKind::ColorWell)
        .create_color_well(color_panel.raw_id(), 0, 244, 32, 24);
    let color_history =
        get_control_backend_for_widget(rust_widgets::widget::WidgetKind::ColorHistory)
            .create_color_history(color_panel.raw_id(), 36, 244, 244, 24);
    let color_swatch = get_control_backend_for_widget(rust_widgets::widget::WidgetKind::Swatch)
        .create_swatch(color_panel.raw_id(), 0, 272, 280, 96);
    for id in [
        color_panel.raw_id(),
        color_picker,
        color_well,
        color_history,
        color_swatch,
    ] {
        if id != 0 {
            rust_widgets::hide_widget(id);
        }
    }
    if let Ok(mut gui) = gui.lock() {
        gui.color_panel = color_panel.raw_id();
        gui.color_picker = color_picker;
        gui.color_well = color_well;
        gui.color_history = color_history;
        gui.color_swatch = color_swatch;
    }
    wire_colour_editor(&gui);

    // Hover tooltips (blue22 H-20). The runtime forwards the hovered control and its `tooltip`
    // property to every registered manager. The manager is a child of the window with a small
    // bubble-sized geometry: a full-window overlay would intercept every click, so the tick below
    // moves it next to the hovered control only while a bubble is visible.
    let tooltip = get_control_backend_for_widget(rust_widgets::widget::WidgetKind::TooltipManager)
        .create_tooltip_manager(handle.raw_id(), 0, 0, 1, 1);
    if tooltip != 0 {
        rust_widgets::hide_widget(tooltip);
        if let Ok(mut gui) = gui.lock() {
            gui.tooltip = tooltip;
        }
    }
    if let Ok(gui) = gui.lock() {
        for (id, key) in [
            (gui.property_grid, "ui.panel.inspector"),
            (gui.data_grid, "ui.panel.data"),
            (gui.messages_list, "ui.panel.messages"),
            (gui.outline_list, "ui.panel.outline"),
            (gui.widgets_list, "ui.panel.widgets"),
            (gui.events_list, "ui.panel.events"),
            (gui.pages_list, "ui.panel.pages"),
            (gui.canvas_surface, "ui.canvas"),
        ] {
            set_control_tooltip(id, tr(key));
        }
    }
    schedule_tooltips(Arc::clone(&gui));

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
                gui.drag = gui.pointer.and_then(|(x, y)| {
                    let node = gui.scene.hit(x, y)?;
                    crate::app::canvas::drag::Drag::begin(gui.scene.tree(), node).ok()
                });
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
                        let state = Arc::clone(&gui);
                        if let Ok(mut gui) = state.lock()
                            && let Some(design) = canvas_point(&gui, point.x, point.y)
                        {
                            let previous = gui.pointer;
                            gui.pointer = Some(design);
                            if gui.drag_active && gui.drag_start.is_none() {
                                gui.drag_start = Some(design);
                            }
                            if gui.drag_active && gui.drag.is_some() && gui.ghost == 0 {
                                let start = gui.drag_start.unwrap_or(design);
                                let moved =
                                    start.0.abs_diff(design.0).max(start.1.abs_diff(design.1));
                                if moved >= DRAG_THRESHOLD {
                                    create_drag_ghost(&state, &mut gui);
                                }
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
                let Some(design) = canvas_point(&gui, x, y) else {
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
    // Edits reach the model the same way: the control reports a change, the poller reads the text it
    // now holds. `sync_editors` also drains the report before it loads a document back, so neither a
    // tab switch nor a command can overwrite a keystroke that was not read yet.
    schedule_editor_model(Arc::clone(&gui));

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
                apply_action(&mut gui, "toggle:states");
                apply_action(&mut gui, "toggle:items");
                apply_action(&mut gui, "toggle:components");
                apply_action(&mut gui, "toggle:ai");
                apply_action(&mut gui, "toggle:source");
                apply_action(&mut gui, "toggle:layout");
                // A planned command must answer honestly instead of "resolved".
                run_command(&mut gui, "file.save");
                crate::host::log::emit(
                    crate::host::log::Level::Info,
                    "app::run::workbench",
                    &format!("workbench smoke: planned command message {:?}", gui.message),
                );
                // A handled command runs, and says what it did (no history here, so it says that).
                run_command(&mut gui, "edit.undo");
                crate::host::log::emit(
                    crate::host::log::Level::Info,
                    "app::run::workbench",
                    &format!("workbench smoke: undo command message {:?}", gui.message),
                );
                crate::host::log::emit(
                    crate::host::log::Level::Info,
                    "app::run::workbench",
                    &format!(
                        "workbench smoke: layout panel shown {}",
                        gui.panels
                            .layout()
                            .shown()
                            .iter()
                            .any(|id| id.as_str() == "layout")
                    ),
                );
                crate::host::log::emit(
                    crate::host::log::Level::Info,
                    "app::run::workbench",
                    &format!("workbench smoke: ai panel journal {:?}", gui.ai_journal),
                );
                crate::host::log::emit(
                    crate::host::log::Level::Info,
                    "app::run::workbench",
                    &format!(
                        "workbench smoke: source outline document {:?}",
                        gui.source.document()
                    ),
                );
                crate::host::log::emit(
                    crate::host::log::Level::Info,
                    "app::run::workbench",
                    &format!(
                        "workbench smoke: source outline server configured {}, failure {:?}",
                        gui.source.is_configured(),
                        gui.source.failure()
                    ),
                );
                crate::host::log::emit(
                    crate::host::log::Level::Info,
                    "app::run::workbench",
                    &format!(
                        "workbench smoke: source outline symbols {}",
                        gui.source.symbol_count()
                    ),
                );
                crate::host::log::emit(
                    crate::host::log::Level::Info,
                    "app::run::workbench",
                    &format!(
                        "workbench smoke: source outline mapping {:?}",
                        gui.source.mapping()
                    ),
                );
                crate::host::log::emit(
                    crate::host::log::Level::Info,
                    "app::run::workbench",
                    &format!(
                        "workbench smoke: source outline associated symbols {}",
                        gui.source.associated_symbols()
                    ),
                );
                crate::host::log::emit(
                    crate::host::log::Level::Info,
                    "app::run::workbench",
                    &format!(
                        "workbench smoke: source outline node rows {}",
                        gui.source.node_count()
                    ),
                );
                if let Some(row) = gui.source.first_target() {
                    jump_to_source(&mut gui, row);
                    crate::host::log::emit(
                        crate::host::log::Level::Info,
                        "app::run::workbench",
                        &format!(
                            "workbench smoke: source outline jumped row {row}: {}",
                            gui.message
                        ),
                    );
                }
                crate::host::log::emit(
                    crate::host::log::Level::Info,
                    "app::run::workbench",
                    &format!(
                        "workbench smoke: states panel source {:?}",
                        gui.states_source
                    ),
                );
                apply_action(&mut gui, "group:split");
                apply_action(&mut gui, "layout:save");
                apply_action(&mut gui, "group:next");
                // The widget library's whole path: pick a real contract row and insert it. The
                // scene count says whether a node appeared, so a palette that only paints rows
                // is not reported as a working one.
                let before = gui.scene.row_nodes().len();
                if gui.widgets_search != 0 {
                    // The search path is part of the smoke: a query narrows the same catalog list,
                    // and the insert below then comes from the filtered rows. The field's text is
                    // not set here: `set_widget_text` emits `text_changed`, whose subscriber locks
                    // this same `Gui`, and calling it under the lock the smoke already holds would
                    // deadlock the loop (the real typing path holds no such lock).
                    filter_widgets(&mut gui, "button");
                    crate::host::log::emit(
                        crate::host::log::Level::Info,
                        "app::run::workbench",
                        &format!(
                            "workbench smoke: widget search \"button\" shows {} row(s)",
                            gui.widget_visible.len()
                        ),
                    );
                }
                if !gui.widget_visible.is_empty() {
                    // The favorites path is exercised without persisting: a smoke run must not
                    // write the person's data file.
                    let on = toggle_widget_favorite(&mut gui, 0, false);
                    crate::host::log::emit(
                        crate::host::log::Level::Info,
                        "app::run::workbench",
                        &format!(
                            "workbench smoke: first widget row favorite = {on}, {} row(s) still listed",
                            gui.widget_visible.len()
                        ),
                    );
                }
                if gui.widget_visible.is_empty() {
                    warn_failure(
                        "the widget smoke could not insert",
                        "the library has no rows; the catalog was not captured",
                    );
                } else {
                    insert_widget_from_palette(&mut gui, 0);
                    let inserted = gui.scene.primary();
                    crate::host::log::emit(
                        crate::host::log::Level::Info,
                        "app::run::workbench",
                        &format!(
                            "workbench smoke: widget library inserted a node ({before} -> {} node(s))",
                            gui.scene.row_nodes().len()
                        ),
                    );
                    // The same selected node is sent back, wrapped, duplicated and deleted: this
                    // exercises the layer, container and copy actions plus the Inspector reset.
                    scene_context_action(&mut gui, "send-to-back");
                    crate::host::log::emit(
                        crate::host::log::Level::Info,
                        "app::run::workbench",
                        "workbench smoke: sent the inserted node to the back",
                    );
                    let before_wrap = gui.scene.row_nodes().len();
                    scene_context_action(&mut gui, "wrap-container");
                    crate::host::log::emit(
                        crate::host::log::Level::Info,
                        "app::run::workbench",
                        &format!(
                            "workbench smoke: wrapped the inserted node ({before_wrap} -> {} node(s))",
                            gui.scene.row_nodes().len()
                        ),
                    );
                    let after_insert = gui.scene.row_nodes().len();
                    // The wrapper selected itself; select the child again and move it back out, so
                    // the inverse action is exercised rather than moving the wrapper itself.
                    if let Some(child) = inserted {
                        select_node(&mut gui, child, "smoke");
                        scene_context_action(&mut gui, "move-out");
                        crate::host::log::emit(
                            crate::host::log::Level::Info,
                            "app::run::workbench",
                            &format!(
                                "workbench smoke: moved the inserted node out of its wrapper ({after_insert} -> {} node(s))",
                                gui.scene.row_nodes().len()
                            ),
                        );
                        // The wrapper is now the sibling above the node, so the deterministic
                        // "move into the container above" rule can put it back.
                        scene_context_action(&mut gui, "move-into");
                        crate::host::log::emit(
                            crate::host::log::Level::Info,
                            "app::run::workbench",
                            "workbench smoke: moved the inserted node into the container above it",
                        );
                    }
                    scene_context_action(&mut gui, "duplicate");
                    crate::host::log::emit(
                        crate::host::log::Level::Info,
                        "app::run::workbench",
                        &format!(
                            "workbench smoke: duplicated the inserted node ({after_insert} -> {} node(s))",
                            gui.scene.row_nodes().len()
                        ),
                    );
                    let after_duplicate = gui.scene.row_nodes().len();
                    scene_context_action(&mut gui, "delete");
                    crate::host::log::emit(
                        crate::host::log::Level::Info,
                        "app::run::workbench",
                        &format!(
                            "workbench smoke: deleted the duplicated node ({after_duplicate} -> {} node(s), inspector {} node(s))",
                            gui.scene.row_nodes().len(),
                            gui.property.count()
                        ),
                    );
                    // The editor's own report reaches the model. This is the path a keystroke takes
                    // (control emits `text_change`, the model reads the text it now holds), and no
                    // other smoke step exercises it. It runs before the rename dialog below opens
                    // its modal: a modal refuses events aimed at anything else, which is the
                    // library's own rule and would make this step test the refusal.
                    {
                        let editor = gui.group_views.first().map(|view| view.editor).unwrap_or(0);
                        let document = gui
                            .groups
                            .group_ids()
                            .first()
                            .and_then(|group| gui.groups.tabs(*group))
                            .and_then(|tabs| tabs.active().map(|tab| tab.document().to_owned()));
                        if let (true, Some(document)) = (editor != 0, document) {
                            // The demo buffers have no file. A baseline is recorded here, exactly as
                            // opening a project file does, so the dirty path and the close guard are
                            // exercised without writing anything to disk.
                            gui.saved_texts.insert(document.clone(), String::new());
                            let before = gui.groups.text(&document).unwrap_or("").to_owned();
                            let _ = runtime::dispatch_event(
                                editor,
                                &rust_widgets::event::Event::text_input("// smoke"),
                            );
                            gui.apply_pending_editor_text();
                            let after = gui.groups.text(&document).unwrap_or("").to_owned();
                            // What the strip now shows for that view: the marker is written into
                            // the tab text, so reading it back is the end-to-end check.
                            let shown = gui
                                .groups
                                .tabs(gui.groups.focus())
                                .and_then(|tabs| tabs.active_index())
                                .and_then(|index| {
                                    gui.group_views.first().and_then(|view| {
                                        with_widget_mut(view.tabs, |widget| {
                                            let any: &mut dyn std::any::Any = widget;
                                            any.downcast_mut::<rust_widgets::widget::TabWidget>()
                                                .and_then(|tabs| {
                                                    tabs.tab_text(index).map(str::to_owned)
                                                })
                                        })
                                    })
                                })
                                .flatten();
                            let dirty = gui.groups.dirty_documents().contains(&document);
                            let allows_close = window_close_allowed(&mut gui);
                            let refusal = gui.message.clone();
                            // The refusal asks the library's question. Escape is the dismissing
                            // action, so the smoke answers "keep editing" through the dialog's own
                            // signal and then applies it here, instead of leaving a modal up over
                            // the rename step below.
                            let dialog = gui.close_dialog;
                            if dialog != 0 {
                                let _ = runtime::dispatch_event(
                                    dialog,
                                    &rust_widgets::event::Event::key_press(
                                        rust_widgets::event::key_codes::ESCAPE,
                                        0,
                                    ),
                                );
                            }
                            let answer = match gui.close_slot.lock() {
                                Ok(mut pending) => pending.take(),
                                Err(poisoned) => poisoned.into_inner().take(),
                            };
                            if let Some(answer) = answer {
                                apply_close_answer(&mut gui, answer);
                            }
                            let kept = gui.close_dialog == 0;
                            crate::host::log::emit(
                                crate::host::log::Level::Info,
                                "app::run::workbench",
                                &format!(
                                    "workbench smoke: an editor edit reached the model of {document} ({} -> {} chars, the model now carries the text: {}), dirty {dirty}, tab {shown:?}, window close allowed {allows_close} with {refusal:?}, dialog {dialog} answered and dismissed {kept}, history {:?}, message {:?}",
                                    before.len(),
                                    after.len(),
                                    after.contains("// smoke"),
                                    history_state(&gui),
                                    gui.message,
                                ),
                            );
                            // Per-document undo history: switch away from the document and back,
                            // then undo. With one buffer per document the history belongs to the
                            // document, so the typed text is still undoable; a surface whose text
                            // was swapped on every switch (the previous shape) would answer
                            // "nothing to undo" here.
                            let group = gui.groups.group_ids()[0];
                            let away = gui.groups.tabs_mut(group).map(|tabs| tabs.activate(1));
                            gui.sync_editors();
                            let back = gui.groups.tabs_mut(group).map(|tabs| tabs.activate(0));
                            gui.sync_editors();
                            run_command(&mut gui, "edit.undo");
                            let undone = gui.message.clone();
                            let after_undo = gui.groups.text(&document).unwrap_or("").to_owned();
                            let surface = with_widget_mut(editor, |widget| {
                                let any: &mut dyn std::any::Any = widget;
                                any.downcast_mut::<CodeEditor>().map(|editor| {
                                    (
                                        editor.buffer_summaries().len(),
                                        editor.active_language(),
                                        editor.text().len(),
                                    )
                                })
                            });
                            crate::host::log::emit(
                                crate::host::log::Level::Info,
                                "app::run::workbench",
                                &format!(
                                    "workbench smoke: switched away {away:?} and back {back:?}, undo said {undone:?}, the buffer is {} -> {} chars and still carries the text: {}; surface (buffers, language, text) {surface:?}, history {:?}",
                                    after.len(),
                                    after_undo.len(),
                                    after_undo.contains("// smoke"),
                                    history_state(&gui),
                                ),
                            );
                            // Neither the baseline nor the dirty mark was real, so both go away.
                            gui.saved_texts.remove(&document);
                            for group in gui.groups.group_ids() {
                                if let Some(tabs) = gui.groups.tabs_mut(group) {
                                    tabs.mark_dirty(&document, false);
                                }
                            }
                            sync_workbench_document(&mut gui);
                        }
                    }
                    // The rename dialog's whole path, on the node that survived the copy's delete:
                    // open it, type through the dialog's own setter, and accept with Enter. The
                    // answer is applied by the poller after this callback unwinds.
                    if let Some(child) = inserted {
                        select_node(&mut gui, child, "smoke");
                        // Copy/paste through the scene clipboard, then rename the original again
                        // (paste selects the copy).
                        scene_context_action(&mut gui, "copy-node");
                        let before_paste = gui.scene.row_nodes().len();
                        scene_context_action(&mut gui, "paste-node");
                        crate::host::log::emit(
                            crate::host::log::Level::Info,
                            "app::run::workbench",
                            &format!(
                                "workbench smoke: copied and pasted a node ({before_paste} -> {} node(s))",
                                gui.scene.row_nodes().len()
                            ),
                        );
                        select_node(&mut gui, child, "smoke");
                        scene_context_action(&mut gui, "rename");
                        let dialog = gui.rename_dialog;
                        if dialog != 0 {
                            let _ = with_widget_mut(dialog, |widget| {
                                if let Some(dialog) =
                                    (widget as &mut dyn Any).downcast_mut::<InputDialog>()
                                {
                                    dialog.set_text_value("renamed node");
                                }
                                true
                            });
                            let _ = runtime::dispatch_event(
                                dialog,
                                &rust_widgets::event::Event::key_press(
                                    rust_widgets::event::key_codes::ENTER,
                                    0,
                                ),
                            );
                            crate::host::log::emit(
                                crate::host::log::Level::Info,
                                "app::run::workbench",
                                "workbench smoke: answered the rename dialog with \"renamed node\"",
                            );
                        }
                    }
                }
            }
        });
        let quit_gui = Arc::clone(&gui);
        let _ = rust_widgets::schedule_on_main(Duration::from_millis(900), move || {
            if let Ok(mut gui) = quit_gui.lock() {
                // The confirming action of the same question, through the dialog's own signal. A
                // discard is a quit, so this step replaces the plain quit action: the session ends
                // because the person answered the dialog, not because a timer fired.
                let document = gui
                    .groups
                    .group_ids()
                    .first()
                    .and_then(|group| gui.groups.tabs(*group))
                    .and_then(|tabs| tabs.active().map(|tab| tab.document().to_owned()));
                let mut asked = false;
                if let Some(document) = document {
                    gui.saved_texts.insert(document.clone(), String::new());
                    let typed = format!("{}// quit", gui.groups.text(&document).unwrap_or(""));
                    gui.set_document_text(&document, &typed);
                    asked = !window_close_allowed(&mut gui);
                    let dialog = gui.close_dialog;
                    if dialog != 0 {
                        // Enter follows the focus ring, and the ring starts at the dismissing
                        // action: two Tabs move it to the confirming one, then Enter accepts. That
                        // is the dialog's own keyboard contract, not a host shortcut.
                        for key in [
                            rust_widgets::event::key_codes::TAB,
                            rust_widgets::event::key_codes::TAB,
                            rust_widgets::event::key_codes::ENTER,
                        ] {
                            let _ = runtime::dispatch_event(
                                dialog,
                                &rust_widgets::event::Event::key_press(key, 0),
                            );
                        }
                    }
                }
                crate::host::log::emit(
                    crate::host::log::Level::Info,
                    "app::run::workbench",
                    &format!(
                        "workbench smoke: close refused {asked}, answer queued {}; the poller quits",
                        gui.close_slot
                            .lock()
                            .map(|pending| pending.is_some())
                            .unwrap_or(false)
                    ),
                );
            }
        });
    }

    // Keyboard routing (T04.08/T05.05). The library offers each keypress to this window before
    // the focused control sees it, and only while the key is a command (a bare character, a
    // composing IME and a declined handler all continue to the control). The host resolves the
    // canonical shortcut against the keymap the window and the audit share; a triggered command
    // runs through the same dispatcher as a menu leaf.
    {
        let keys_gui = Arc::clone(&gui);
        let platform = window_platform();
        let keymap = window_keymap();
        handle.set_key_handler(move |shortcut, info| {
            let Some(key) = crate::app::commands::shortcuts::key_of_shortcut(shortcut, platform)
            else {
                return false;
            };
            let Ok(mut gui) = keys_gui.lock() else {
                return false;
            };
            let Some(command) = command_for_key(
                &keymap,
                key,
                gui.workbench.mode().name(),
                focused_panel(&gui),
                info.accepts_text_input(),
                info.is_ime_composing(),
            ) else {
                return false;
            };
            // The one place a real session can prove routing without a debugger: the chord and the
            // command it resolved to, on the same log stream as the menu and palette.
            crate::host::log::emit(
                crate::host::log::Level::Info,
                "app::run::workbench-keys",
                &format!("{key} -> {command}"),
            );
            run_command(&mut gui, &command);
            true
        });
    }

    // Save-before-closing (upstream RW-53). The window's close button is a proposal the library
    // asks this handler about before the window disappears; a buffer with unsaved changes refuses
    // it and says which documents are holding it. Quitting from the menu stays the explicit path
    // that discards, so a refused close is never a window nobody can close.
    {
        let close_gui = Arc::clone(&gui);
        handle.set_close_handler(move || match close_gui.lock() {
            Ok(mut gui) => window_close_allowed(&mut gui),
            Err(poisoned) => window_close_allowed(&mut poisoned.into_inner()),
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

/// Carries edits the editor surfaces reported into the model, between commands.
///
/// The signal says *which* surface changed; reading its text and writing the buffer happens here, on
/// the main loop, so a document is cloned only after an edit and never while a keystroke is being
/// dispatched. The poller does nothing when no edit is pending, and its tick is well below a person's
/// reading speed: the cost is one buffer copy per tick at most, not one per keystroke.
fn schedule_editor_model(gui: Arc<Mutex<Gui>>) {
    let next = Arc::clone(&gui);
    let _ = rust_widgets::schedule_on_main(Duration::from_millis(250), move || {
        if let Ok(mut gui) = gui.lock() {
            gui.apply_pending_editor_text();
        }
        schedule_editor_model(next);
    });
}

#[cfg(test)]
mod tests {
    use super::{
        GridRow, command_for_key, control_reference_text, dependency_rows, environment_rows,
        find_hits, onboarding_text, palette_command_entry, project_rows, property_item,
        same_property_item, sample_asset_images, shortcut_reference_text, shortcut_summary,
        tab_label, target_rows, tr, window_keymap,
    };
    use crate::app::canvas::widget_catalog::Catalog;
    use crate::app::commands::command::built_in;

    #[test]
    fn a_disabled_palette_entry_carries_its_reason_instead_of_bending_its_title() {
        use crate::app::commands::command::Context;
        use crate::app::commands::palette::{Recents, search};

        let registry = built_in();
        // No trust and no authorization: some commands are rightly disabled, which is the state
        // the control has to paint (RW-15).
        let context = Context::in_area("design");
        let entries = search(&registry, &context, "", &Recents::new(8));
        let disabled = entries
            .iter()
            .find(|entry| !entry.is_enabled())
            .expect("an untrusted context disables at least one command");
        let row = palette_command_entry(disabled, None);
        assert_eq!(row.id, disabled.id());
        assert_eq!(row.title, disabled.title());
        assert!(!row.title.contains("disabled"), "{}", row.title);
        assert!(!row.is_enabled());
        assert_eq!(row.disabled_reason(), disabled.reason());

        let enabled = entries
            .iter()
            .find(|entry| entry.is_enabled())
            .expect("an enabled command");
        let row = palette_command_entry(enabled, None);
        assert!(row.is_enabled());
        assert_eq!(row.disabled_reason(), None);

        // RW-16: a routed chord is shown, and a two-key sequence is not (the window handler
        // routes one key at a time).
        let row = palette_command_entry(enabled, Some("Ctrl+Z"));
        assert_eq!(row.shortcut.as_deref(), Some("Ctrl+Z"));
        let routed = super::routed_shortcuts();
        assert!(routed.contains_key("edit.undo"), "{routed:?}");
        assert!(
            !routed.contains_key("keys.show"),
            "a sequence has no single-key hint: {routed:?}"
        );
    }

    #[test]
    fn a_key_routes_only_when_the_keymap_and_the_focus_say_so() {
        use crate::app::commands::shortcuts::{Chord, Key, key_of_shortcut};
        use rust_widgets::shortcut::{Key as LibraryKey, Modifiers as LibraryModifiers, Shortcut};

        let platform = crate::app::commands::shortcuts::Platform::Linux;
        let keymap = platform.keymap();
        let undo = key_of_shortcut(
            &Shortcut::new(LibraryKey::Z, LibraryModifiers::PRIMARY),
            platform,
        )
        .expect("a host key");
        assert_eq!(
            command_for_key(&keymap, undo, "design", None, false, false).as_deref(),
            Some("edit.undo")
        );

        // macOS spells the *bindings* with Cmd, so the incoming key has to be spelled the same way
        // for the two to match. This is the assertion that would have caught a Ctrl-only bridge.
        let mac = crate::app::commands::shortcuts::Platform::MacOs;
        let mac_undo = key_of_shortcut(
            &Shortcut::new(LibraryKey::Z, LibraryModifiers::PRIMARY),
            mac,
        )
        .expect("a host key");
        assert!(mac_undo.modifiers().uses_meta(), "primary is Cmd on macOS");
        assert_eq!(
            command_for_key(&mac.keymap(), mac_undo, "design", None, false, false).as_deref(),
            Some("edit.undo")
        );

        // A bare character belongs to the text field, and an IME composition belongs to the IME.
        let bare = Key::parse("z").expect("a key");
        assert_eq!(
            command_for_key(&keymap, bare, "design", None, true, false),
            None
        );
        assert_eq!(
            command_for_key(&keymap, undo, "design", None, false, true),
            None,
            "a composing IME makes no key a command"
        );

        // A panel-scoped chord matches only while that panel owns the keyboard. The default table
        // has no panel-scoped binding, so this uses one of its own — the scope mechanism is what
        // is under test, not the default list.
        let escape = key_of_shortcut(
            &Shortcut::new(LibraryKey::Escape, LibraryModifiers::NONE),
            platform,
        )
        .expect("a host key");
        let mut panel_keymap = crate::app::commands::shortcuts::Keymap::new();
        panel_keymap
            .add(crate::app::commands::shortcuts::Binding::new(
                Chord::parse("Escape").expect("a chord"),
                "view.panel-hide-current",
                crate::app::commands::shortcuts::Scope::Panel("outline".to_owned()),
            ))
            .expect("no conflict");
        assert_eq!(
            command_for_key(
                &panel_keymap,
                escape,
                "design",
                Some("outline".to_owned()),
                false,
                false
            )
            .as_deref(),
            Some("view.panel-hide-current")
        );
        assert_eq!(
            command_for_key(&panel_keymap, escape, "design", None, false, false),
            None
        );

        // A two-key sequence is not routed by the window handler.
        let ctrl_k = key_of_shortcut(
            &Shortcut::new(LibraryKey::K, LibraryModifiers::PRIMARY),
            platform,
        )
        .expect("a host key");
        assert_eq!(
            command_for_key(&keymap, ctrl_k, "design", None, false, false),
            None
        );
    }

    #[test]
    fn the_keyboard_reference_and_summary_name_the_keymap_the_window_routes() {
        let reference = shortcut_reference_text();
        for binding in window_keymap().bindings() {
            assert!(
                reference.contains(&binding.chord().to_string()),
                "{} is missing from the reference",
                binding.chord()
            );
        }
        let summary = shortcut_summary();
        assert!(summary.starts_with("shortcuts: "), "{summary}");
        assert!(summary.contains("edit.undo"), "{summary}");
    }

    #[test]
    fn the_sample_assets_are_decoded_rgba_thumbnails() {
        let assets = sample_asset_images();
        assert_eq!(assets.len(), 6);
        for asset in &assets {
            assert!(asset.image.is_some(), "{} has no pixels", asset.path);
            assert_eq!((asset.width, asset.height), (96, 72));
        }
    }

    #[test]
    fn a_tab_label_marks_only_an_unsaved_buffer_and_keeps_the_document_name() {
        // The last path component names the tab; the marker is the title's own, so the two places
        // that show "unsaved" cannot drift.
        assert_eq!(tab_label("pages/home.json", false), "home.json");
        assert_eq!(tab_label("pages/home.json", true), "• home.json");
        assert_eq!(tab_label("main.rs", true), "• main.rs");
    }

    #[test]
    fn the_property_row_maps_to_the_library_item_the_grid_edits() {
        let row = GridRow::new("title", "Save", true);
        let item = property_item(&row);
        assert_eq!(item.name, "title");
        assert_eq!(item.value, "Save");
        assert!(item.editable);
        assert!(!item.mixed);

        let mixed = property_item(&GridRow::mixed("title"));
        assert!(mixed.mixed);
        assert!(mixed.placeholder.is_some());

        let bounded = property_item(&GridRow::new("width", "1", true).with_bounds(
            Some(0.0),
            Some(10.0),
            Some(1.0),
        ));
        assert_eq!(bounded.number_bounds.expect("bounds").max, Some(10.0));
    }

    #[test]
    fn the_row_comparison_sees_every_field_the_host_sets() {
        let base = GridRow::new("title", "Save", true);
        assert!(same_property_item(
            &property_item(&base),
            &property_item(&base)
        ));
        let variants = [
            GridRow::new("other", "Save", true),
            GridRow::new("title", "Open", true),
            GridRow::new("title", "Save", false),
            GridRow::mixed("title"),
            GridRow::new("title", "Save", true).with_bounds(Some(0.0), Some(1.0), None),
        ];
        for variant in &variants {
            assert!(
                !same_property_item(&property_item(&base), &property_item(variant)),
                "{variant:?} compares equal to {base:?}"
            );
        }
    }

    #[test]
    fn search_finds_literal_ranges_per_line() {
        let text = "alpha\nbeta alpha\n";
        let mut hits = Vec::new();
        let mut labels = Vec::new();
        find_hits("demo.rs", text, "alpha", 10, &mut hits, &mut labels);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].document, "demo.rs");
        assert_eq!(hits[0].offset, 0);
        assert_eq!(hits[1].offset, 11);
        assert_eq!(hits[1].length, 5);
        assert!(labels[1].contains("demo.rs:2:"), "{labels:?}");
    }

    #[test]
    fn the_help_references_are_built_from_live_tables() {
        let shortcuts = shortcut_reference_text();
        assert!(shortcuts.contains("edit.undo"), "{shortcuts}");

        let catalog = Catalog::from_host_contract("desktop").expect("the host catalog");
        let controls = control_reference_text(Some(&catalog));
        assert!(controls.contains("button"), "{controls}");
        assert!(
            control_reference_text(None).contains(&tr("ui.help.controls-unavailable")),
            "the unavailable message is used when there is no catalog"
        );
    }

    #[test]
    fn the_environments_panel_reports_local_facts_and_tools() {
        let rows = environment_rows();
        // Four host facts plus the five local desktop tools; nothing else is claimed.
        assert_eq!(rows.len(), 4 + 5, "{rows:?}");
        for key in [
            "ui.environment.backend",
            "ui.environment.platform",
            "ui.environment.dependency",
            "ui.environment.language",
        ] {
            assert!(
                rows.iter()
                    .any(|row| row.starts_with(&format!("{}: ", tr(key)))),
                "{key} is not reported: {rows:?}"
            );
        }
        assert!(
            rows.iter().all(|row| !row.is_empty()),
            "an empty row is not a fact: {rows:?}"
        );
        assert!(
            rows.iter()
                .any(|row| row.contains(crate::probe::DEPENDENCY_VERSION)),
            "the pinned dependency is reported: {rows:?}"
        );
    }

    #[test]
    fn the_targets_panel_reports_the_six_required_facts() {
        let rows = target_rows();
        assert_eq!(rows.len(), 6, "{rows:?}");
        for key in [
            "ui.target.profiles",
            "ui.target.target",
            "ui.target.toolchain",
            "ui.target.sdk",
            "ui.target.devices",
            "ui.target.certification",
        ] {
            assert!(
                rows.iter()
                    .any(|row| row.starts_with(&format!("{}: ", tr(key)))),
                "{key} is not reported: {rows:?}"
            );
        }
        assert!(
            rows.iter()
                .any(|row| row.contains(&tr("ui.target.verified"))),
            "the certification level is stated: {rows:?}"
        );
    }

    #[test]
    fn the_project_panels_say_when_no_project_is_open() {
        let none = tr("ui.project.none");
        assert_eq!(project_rows(&None), vec![none.clone()]);
        assert_eq!(dependency_rows(&None), vec![none]);
    }

    #[test]
    fn every_panel_has_its_own_identified_title_key() {
        use crate::app::locale::vocabulary::WINDOW_KEYS;
        use std::collections::BTreeSet;

        // The built-ins named by `panel_key`'s other arms, plus every extra panel the window adds.
        let ids = ["messages", "problems", "outline", "inspector"]
            .into_iter()
            .chain(super::EXTRA_PANELS.iter().map(|(id, _, _)| *id));
        let mut seen = BTreeSet::new();
        let mut keys = BTreeSet::new();
        for id in ids {
            assert!(seen.insert(id), "{id} is registered twice");
            let key = super::panel_key(id);
            if id != "outline" {
                assert_ne!(
                    key, "ui.panel.outline",
                    "{id} falls back to the outline key, so its title would be wrong"
                );
            }
            assert!(
                WINDOW_KEYS.contains(&key),
                "{id} names {key}, which is not an identified window key"
            );
            assert!(keys.insert(key), "{key} is the title of two panels");
        }
    }

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
    #[test]
    fn the_window_handles_exactly_the_registered_commands() {
        use std::collections::BTreeSet;

        let registry = crate::app::commands::command::built_in();
        let registered: BTreeSet<String> = registry
            .commands()
            .map(|command| command.id().to_string())
            .collect();
        let handled: BTreeSet<String> = super::HANDLED_COMMANDS
            .iter()
            .map(|id| (*id).to_owned())
            .collect();
        assert_eq!(
            handled, registered,
            "a registered command the window cannot run is a claim it cannot keep, and a handler \
             for an unregistered id cannot be reached through the registry"
        );
    }

    #[test]
    fn every_command_the_menu_exposes_has_a_handler_in_this_window() {
        for menu in crate::app::commands::menu::built_in() {
            // `commands()` answers `(title, command)`.
            for (title, command) in menu.commands() {
                let id = crate::app::commands::menu::Menu::command_id(&command);
                assert!(
                    super::HANDLED_COMMANDS.contains(&id),
                    "{title:?} binds {command}, which this window cannot run"
                );
            }
        }
    }

    #[test]
    fn every_default_shortcut_is_handled_or_named_by_the_menu_ledger() {
        for binding in crate::app::commands::shortcuts::built_in().bindings() {
            let command = binding.command();
            let handled = super::HANDLED_COMMANDS.contains(&command);
            let planned = crate::app::commands::menu::planned_task(command);
            assert!(
                handled || planned.is_some(),
                "{command} is bound to a key but is neither handled nor in the menu ledger"
            );
        }
    }

    #[test]
    fn an_unhandled_command_names_the_task_that_will_implement_it() {
        assert_eq!(
            crate::app::commands::menu::planned_task("file.new-project").as_deref(),
            Some("T02.02")
        );
        assert!(
            crate::app::commands::menu::planned_task("build.run").is_none(),
            "an implemented command is not a planned leaf"
        );
        assert!(
            crate::app::commands::menu::planned_task("edit.undo").is_none(),
            "undo is implemented, so it is a command leaf"
        );
        assert_eq!(
            crate::app::commands::menu::planned_task("file.close"),
            None,
            "a command leaf is not a planned leaf"
        );
    }

    #[test]
    fn every_ledger_panel_with_a_host_is_shown_by_the_one_command() {
        for panel in crate::app::shell::panels::REQUIRED_PANELS {
            let Some(host) = panel.host_id() else {
                continue;
            };
            assert_eq!(
                super::panel_action(host),
                format!("command:view.panel-show?panel={}", panel.id()),
                "{host} does not bind the panel command"
            );
        }
        // The built-in Messages list is not a §2.2 panel, so it keeps the window's own toggle.
        assert_eq!(super::panel_action("messages"), "toggle:messages");
    }

    #[test]
    fn the_panel_command_offers_exactly_the_ledger_ids() {
        use crate::app::commands::command::{CommandId, ParamType, built_in};

        let registry = built_in();
        let id = CommandId::parse("view.panel-show").expect("the id follows 域.动作");
        let command = registry.command(&id).expect("the command is registered");
        let ParamType::Choice(choices) = command.params()[0].kind() else {
            panic!("the panel parameter is a choice");
        };
        let ledger: Vec<String> = crate::app::shell::panels::REQUIRED_PANELS
            .iter()
            .map(|panel| panel.id().to_owned())
            .collect();
        assert_eq!(choices, &ledger);
    }

    #[test]
    fn a_quick_open_query_resolves_an_exact_path_or_the_best_fuzzy_match() {
        let entries = vec![
            "pages/main.json".to_owned(),
            "src/generated/ui_default.rs".to_owned(),
            "src/generated/ui_stripped.rs".to_owned(),
        ];
        assert_eq!(
            super::resolve_open_path(&entries, "pages/main.json").as_deref(),
            Some("pages/main.json")
        );
        assert_eq!(
            super::resolve_open_path(&entries, "  pages/main.json  ").as_deref(),
            Some("pages/main.json")
        );
        assert_eq!(
            super::resolve_open_path(&entries, "ui_stripped").as_deref(),
            Some("src/generated/ui_stripped.rs")
        );
        assert_eq!(super::resolve_open_path(&entries, "zzz"), None);
        assert_eq!(super::resolve_open_path(&entries, "   "), None);
    }

    #[test]
    fn the_go_to_line_field_accepts_line_or_line_column() {
        assert_eq!(super::parse_goto("42"), Ok((42, None)));
        assert_eq!(super::parse_goto("42:7"), Ok((42, Some(7))));
        assert_eq!(super::parse_goto("  42 : 7  "), Ok((42, Some(7))));
        assert!(super::parse_goto("0").is_err(), "lines are 1-based");
        assert!(super::parse_goto("1:0").is_err(), "columns are 1-based");
        assert!(super::parse_goto("x").is_err());
        assert!(super::parse_goto("42:x").is_err());
        assert!(super::parse_goto("").is_err());
    }

    #[test]
    fn a_workspace_symbol_query_prefers_an_exact_name_then_the_best_fuzzy_match() {
        use crate::app::run::workbench_outline::WorkspaceSymbol;

        let symbols = vec![
            WorkspaceSymbol {
                name: "build_page".to_owned(),
                uri: "file:///src/a.rs".to_owned(),
                line: 0,
                character: 0,
            },
            WorkspaceSymbol {
                name: "build".to_owned(),
                uri: "file:///src/b.rs".to_owned(),
                line: 4,
                character: 2,
            },
        ];
        let exact = super::best_workspace_symbol(&symbols, "build").expect("the exact name");
        assert_eq!(exact.name, "build");
        let fuzzy = super::best_workspace_symbol(&symbols, "bld").expect("a fuzzy name");
        assert_eq!(fuzzy.name, "build");
        assert!(super::best_workspace_symbol(&symbols, "zzz").is_none());
    }

    #[test]
    fn a_source_file_names_the_module_its_tests_live_in() {
        assert_eq!(
            super::test_module_filter("src/app/run/workbench_gui.rs"),
            Some("app::run::workbench_gui".to_owned())
        );
        assert_eq!(
            super::test_module_filter("src/foo/mod.rs"),
            Some("foo".to_owned())
        );
        assert_eq!(super::test_module_filter("src/lib.rs"), None);
        assert_eq!(super::test_module_filter("src/main.rs"), None);
        assert_eq!(super::test_module_filter("tests/integration.rs"), None);
        assert_eq!(super::test_module_filter("README.md"), None);
    }

    #[test]
    fn an_unreferenced_resource_is_the_only_cleanup_candidate() {
        use crate::project::lifecycle::session::ProjectSession;

        let directory = std::env::temp_dir().join(format!(
            "rustage-resource-cleanup-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(directory.join("pages")).expect("pages");
        std::fs::create_dir_all(directory.join("assets")).expect("assets");
        std::fs::write(
            directory.join("project.json"),
            r#"{
  "format": 2,
  "project": { "id": "p1a19fce34b8-a6bd9-0000", "name": "cleanup" },
  "pages": [ { "document": "pages/main.json", "title": "main" } ],
  "resources": [ { "path": "assets/used.png" }, { "path": "assets/unused.png" } ],
  "targets": [],
  "generated": [],
  "startup": "pages/main.json",
  "network": { "services": [] }
}"#,
        )
        .expect("project file");
        std::fs::write(
            directory.join("pages/main.json"),
            r#"{
  "format": 6,
  "page": { "id": 1, "name": "main", "root": 1 },
  "nodes": [
    { "id": 1, "type": "window", "children": [2], "properties": [], "layout": null },
    { "id": 2, "type": "image", "children": [], "properties": [
        { "name": "source", "value": { "asset": "assets/used.png" } }
    ], "layout": null }
  ],
  "states": null,
  "collections": [],
  "components": { "definitions": [], "instances": [], "variants": [] },
  "motion": { "motions": [] }
}"#,
        )
        .expect("page file");
        std::fs::write(directory.join("assets/used.png"), b"used").expect("used asset");
        std::fs::write(directory.join("assets/unused.png"), b"unused").expect("unused asset");

        let session = ProjectSession::open(directory.join("project.json")).expect("a project");
        let candidates = super::unused_resources(&session).expect("a readable project");
        let _ = std::fs::remove_dir_all(&directory);
        assert_eq!(candidates, vec!["assets/unused.png".to_owned()]);
    }

    #[test]
    fn a_command_history_row_names_the_clock_time() {
        assert_eq!(super::history_row(0, "a"), "00:00:00 a");
        assert_eq!(super::history_row(3_661_000, "b"), "01:01:01 b");
        assert_eq!(super::history_row(86_401_000, "c"), "00:00:01 c");
    }

    #[test]
    fn a_menu_action_carries_its_argument_after_the_question_mark() {
        let arguments = super::parse_action_arguments("panel=panel.pages&other=x");
        assert_eq!(
            arguments.get("panel").map(String::as_str),
            Some("panel.pages")
        );
        assert_eq!(arguments.get("other").map(String::as_str), Some("x"));
    }
}
