// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The argument parser and the usage text.
//!
//! The surface is deliberately tiny. The scaffold answers `--help`, `--version` and reports its
//! own status; the other two options exist because the host needs reproducible producers for
//! artifacts it commits or inspects: `--render-icons` renders the icon strip in
//! [`crate::app::shell::toolbar`] into a directory (the committed baselines under `tests/baselines/icons/`
//! would otherwise be pictures of whatever the machine last rendered, which is exactly what a
//! baseline must not be), and `--probe-apis` runs the dependency contract probes in
//! [`crate::probe`] and writes their report.

use std::{
    ffi::{OsStr, OsString},
    fmt,
    path::PathBuf,
    time::Duration,
};

use crate::host::locale::Language;
use crate::host::log::Level;
use crate::project::{
    build::{
        configuration::SettingValue,
        workspace::{WorkspaceFile, name_is_addressable},
    },
    document::{closing::Decision, save::Verb},
    lifecycle::create::Template,
    record::files::FileOperation,
};

pub mod application;
pub mod build;
pub mod canvas;
pub mod collection;
pub mod commands;
pub mod component;
pub mod data;
pub mod document;
pub mod editor;
pub mod form;
pub mod generate;
pub mod host;
pub mod language;
pub mod layout;
pub mod logic;
pub mod preview;
pub mod project;
pub mod search;
pub mod shell;
pub mod style;

pub use application::*;
pub use build::*;
pub use canvas::*;
pub use collection::*;
pub use commands::*;
pub use component::*;
pub use data::*;
pub use document::*;
pub use editor::*;
pub use form::*;
pub use generate::*;
pub use host::*;
pub use language::*;
pub use layout::*;
pub use logic::*;
pub use preview::*;
pub use project::*;
pub use search::*;
pub use shell::*;
pub use style::*;

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Help,
    Version,
    Status,
    /// Writes the icon strip's light and dark renders into the given directory.
    RenderIcons(PathBuf),
    /// Runs the dependency contract probes and writes the report into the given directory.
    ProbeApis(PathBuf),
    /// Runs the host capability checks and writes the report into the given directory.
    ProbeHost(PathBuf),
    /// Writes the interface-vocabulary report and the English language pack (task T24.03).
    LocaleReport(PathBuf),
    /// Reads a project file, reports what this build makes of it, and writes the round trip next to
    /// it as evidence that nothing in it was dropped.
    ProjectCheck(PathBuf),
    /// Creates a project: the new-project wizard's work, without a window (task T02.02).
    NewProject(NewProjectOptions),
    /// Opens a project: what it is, what would be asked about it, and nothing else (task T02.03).
    OpenProject(OpenProjectOptions),
    /// Lists the projects that were opened before, and where they are now.
    Recent,
    /// Shows what a project has, classified the way a project tree groups it (task T02.04).
    ProjectTree(PathBuf),
    /// Removes an entry, fixing what points at it or refusing to remove it (task T02.04).
    DropEntry(DropEntryOptions),
    /// Previews or performs one file operation on a project (task T02.05).
    FileOp(FileOpOptions),
    /// Shows the build configuration of a project, and what each value came from (task T02.06).
    Configuration(ConfigurationOptions),
    /// Imports an existing project: what it is, what would change, and the migration itself
    /// (task T02.07).
    Import(ImportOptions),
    /// Checks what a project needs, re-scans it, and changes nothing (task T02.08).
    Validate(ValidateOptions),
    /// Shows a workspace, and plans or carries out one change to its members (task T02.09).
    Workspace(WorkspaceOptions),
    /// Creates an empty workspace document in a directory (task T02.09).
    NewWorkspace(NewWorkspaceOptions),
    /// Shows which tools a kit is made of, and what this machine has of them (task T02.10).
    Kit(KitOptions),
    /// Resolves a workspace's dependencies: what it is made of, and what changed since a record
    /// (task T02.11).
    Dependencies(DependenciesOptions),
    /// Shows what a buffer over a page document is holding after some text is typed into it
    /// (task T03.01).
    Document(DocumentOptions),
    /// Plans and applies a batch of edits to a page document's model (task T03.02).
    Edit(EditOptions),
    /// Replays a session of edits against a page document, then undoes or redoes them (task T03.03).
    History(HistoryOptions),
    /// Writes a document's text back to disk, or a set of them together (task T03.04).
    Save(SaveOptions),
    /// Decides what a document should do when the file changed outside the designer (task T03.05).
    Conflict(ConflictOptions),
    /// Keeps, offers and refuses recovery copies (task T03.06).
    Recovery(RecoveryOptions),
    /// Closes documents with a dirty-content decision (task T03.07).
    Close(CloseOptions),
    /// Resolves a typed command through the registry, or lists them (task T05.01).
    Invoke(CommandOptions),
    /// Resolves a key chord through the keymap, or lists the bindings (task T05.03).
    Keys(KeysOptions),
    /// Shows the menu tree and checks every implemented leaf against the registry (task T05.02).
    Menu,
    /// Searches the command palette, or shows one command's help (task T05.04).
    Palette(PaletteOptions),
    /// Shows the workbench and its panels, applying layout changes (tasks T04.01/T04.02).
    Workbench(WorkbenchOptions),
    /// Previews and carries out a batch operation on a selection (task T05.07).
    Operation(OperationOptions),
    /// Audits the command inventory: registry, menu tree and keymap against each other (task T05.08).
    Audit,
    /// Replays a script of document-tab operations (task T04.03).
    Tabs(TabsOptions),
    /// Replays a script of editor-group operations: splits, focus moves and tab migration
    /// (task T04.04).
    Groups(GroupsOptions),
    /// Copies, round-trips and pastes a clipboard payload (task T05.06).
    Clipboard(ClipboardOptions),
    /// Applies, saves, resets and locks editor-area layouts (task T04.05).
    Layout(LayoutOptions),
    /// Places windows and floating panels across displays, and reconciles them (task T04.06).
    Windows(WindowsOptions),
    /// Shows the toolbar, breadcrumb, status items and notices (task T04.07).
    Chrome(ChromeOptions),
    /// Replays in-place text edits over a piece table (task T06.01).
    Text(TextOptions),
    /// Shows syntax highlighting: cross-line state, the incremental cache and the theme (task T06.02).
    Highlight(HighlightOptions),
    /// Converts document offsets to visual rows and columns, and back (task T06.03).
    Coordinates(CoordinatesOptions),
    /// Runs multi-cursor editing: carets, insertions, line operations, comments, brackets (task T06.04).
    EditOps(EditOpsOptions),
    /// Reads a file's encoding, line endings and indentation, and shows whitespace, numbers, folds
    /// and a minimap (task T06.05).
    EditorView(EditorViewOptions),
    /// Checks a write against protected regions through every write entry point (task T06.06).
    WriteGuard(WriteGuardOptions),
    /// Classifies a document's size and shows which features are degraded, and why (task T06.07).
    Scale(ScaleOptions),
    /// Expands a language snippet, jumps its placeholders, and surrounds a selection (task T06.09).
    Snippet(SnippetOptions),
    /// Replays the editing regression scenarios and rechecks the size boundaries (task T06.08).
    Regression(RegressionOptions),
    /// Compares two texts or trees, and shows a read-only revision (task T08.07).
    Diff(DiffOptions),
    /// Grows a selection, bookmarks edit locations, and turns display elements on and off (task T06.10).
    Structural(StructuralOptions),
    /// Runs the language-service lifecycle: one session per project, diagnostics, restart (task T07.01).
    Language(LanguageOptions),
    /// Synchronises documents with the protocol and converts UTF-8/UTF-16 positions (task T07.02).
    Protocol(ProtocolOptions),
    /// Serves completions, hover, signature help and documentation, dropping stale replies (T07.03).
    Features(FeaturesOptions),
    /// Answers definitions, references and symbols, with the outline and breadcrumbs (task T07.04).
    Navigation(NavigationOptions),
    /// Serves diagnostics, semantic tokens and guarded code actions (task T07.05).
    Diagnostics(DiagnosticsOptions),
    /// Reviews and commits a cross-file edit as one transaction (task T07.06).
    WorkspaceEdit(WorkspaceEditOptions),
    /// Shows Rust/JSON/CSS schema documentation and target-relative completion (task T07.07).
    Schema(SchemaOptions),
    /// Runs the language-service failure regression: crash, slow, no toolchain, multi-project (T07.08).
    Health(HealthOptions),
    /// Shows inlay hints, code lenses and hierarchies under negotiated capabilities (task T07.09).
    Hints(HintsOptions),
    /// Rebuilds the analysis configuration after a member/feature/target/Kit switch (task T07.10).
    Analysis(AnalysisOptions),
    /// Builds the project file index with ignore, size and binary policy (task T08.01).
    Index(IndexOptions),
    /// Searches file and project text with a literal or a regex subset (task T08.02).
    Search(SearchOptions),
    /// Opens files and symbols by a query, ranked with recent access (task T08.03).
    QuickOpen(QuickOpenOptions),
    /// Builds replace candidates with a diff, checkboxes and a protection filter (task T08.04).
    Replace(ReplaceOptions),
    /// Runs a batch replace that stops midway, resumes and undoes (task T08.05).
    Batch(BatchOptions),
    /// Walks the navigation history and corrects moved files (task T08.06).
    HistoryNav(HistoryNavOptions),
    /// Runs the search/replace acceptance regression (task T08.08).
    SearchHealth(SearchHealthOptions),
    /// Shows the widget catalog: contract names, categories, search, schema and limits (task T09.01).
    Catalog(CatalogOptions),
    /// Maps nodes to render instances, hit tests and selection, with lifecycle (task T09.02).
    Scene(SceneOptions),
    /// Runs structural edits on the object tree: insert/delete/copy/rename/reorder (task T09.04).
    Structure(StructureOptions),
    /// Pans, zooms, fits and shows the design aids without touching the model (task T09.05).
    Viewport(ViewportOptions),
    /// Drags a node: candidates, preview, cancel and a batch transaction (task T09.06).
    Drag(DragOptions),
    /// Hidden/locked/paint order, object filter and locate, and multi-page tabs (task T09.07).
    Objects(ObjectsOptions),
    /// The design surface's keyboard and its thousand-node stress harness (task T09.08).
    Design(DesignOptions),
    /// The layout contract, computed result, diagnostics and constraint sources (task T10.01).
    LayoutPlan(LayoutPlanOptions),
    /// Choosing, switching and detaching a container's layout with migration (task T10.02).
    LayoutSwitch(LayoutSwitchOptions),
    /// Box constraints: min/max/preferred, grow, margin, gap, overflow and their validation (T10.03).
    Constraints(ConstraintsOptions),
    /// Align/distribute/match-size and the keyboard nudge, as constraint writes (task T10.04).
    Arrange(ArrangeOptions),
    /// Checks simulated frames against a target's support: size, orientation, safe area, scale (T10.05).
    Frames(FramesOptions),
    /// The focus order (tab sequence), navigation and unreachable nodes (task T10.06).
    FocusOrder(FocusOrderOptions),
    /// Size conditions and layout variants, blocking ones the target cannot express (task T10.07).
    Variants(VariantsOptions),
    /// Compares the preview with a library-generated probe on the same input (task T10.08).
    LayoutProbe(LayoutProbeOptions),
    /// The schema-driven property editor, where a refusal never touches the valid model (task T11.01).
    PropertyEditor(PropertyEditorOptions),
    /// Multi-selection common properties, mixed values and reset (task T11.02).
    MultiProperty(MultiPropertyOptions),
    /// The CSS rule view, selectors and the computed style with each value's source (task T11.03).
    Css(CssOptions),
    /// Design tokens, themes and expanding references at generation time (task T11.04).
    Tokens(TokensOptions),
    /// Interaction state previews and theme/state difference review (task T11.05).
    States(StatesOptions),
    /// Transitions/animations, their timing and preview control (task T11.06).
    Motion(MotionOptions),
    /// One style emitter for the preview and the generator, plus token migration (task T11.07).
    Style(StyleOptions),
    /// Style boundaries: undo, contrast, multi-select/theme/Unicode/state edges (task T11.08).
    StyleBoundary(StyleBoundaryOptions),
    /// Data schema, sample/empty/error datasets and their versions (task T13.01).
    Data(DataOptions),
    /// Typed bindings, conversions and the cycle/oscillation refusals (task T13.02).
    Binding(BindingOptions),
    /// Event declarations, typed payloads, handlers and the declared/wired split (task T13.03).
    Events(EventsOptions),
    /// Handler actions, permissions and subscription release (task T13.04).
    Dispatch(DispatchOptions),
    /// Page routes, the navigation graph, the back stack and the window lifecycle (task T13.05).
    Routes(RoutesOptions),
    /// Explicit states, guarded transitions and the sync/async boundary (task T13.06).
    Machine(MachineOptions),
    /// File/network adapters, sample doubles, cancel and retry policy (task T13.07).
    Adapter(AdapterOptions),
    /// Joint acceptance: the undo log, event/state vectors and the two-way loop (task T13.08).
    SessionLog(SessionLogOptions),
    /// The preview service, its session and its instance lifecycle (task T14.01).
    Preview(PreviewOptions),
    /// Live update versus rebuild, coalescing and source identification (task T14.02).
    HotUpdate(HotUpdateOptions),
    /// Preserving input/focus/scroll/selection across a rebuild (task T14.03).
    StateKeep(StateKeepOptions),
    /// The multi-viewport preview matrix over one document (task T14.04).
    Matrix(MatrixOptions),
    /// What an inspection shows, and what a read-only inspection refuses to do (task T14.05).
    Inspect(InspectOptions),
    /// Comparing a preview screenshot with its recorded baseline (task T14.06).
    Visual(VisualOptions),
    /// Failure, crash and reload/rebuild/state-reset recovery of the preview app (task T14.07).
    PreviewRecovery(PreviewRecoveryOptions),
    /// Many bind/rebuild cycles and the load and latency checks over them (task T14.08).
    Load(LoadOptions),
    /// Routes a keypress, or shows a context menu and the toolbar (task T05.05).
    Focus(FocusOptions),
    /// Starts the baseline desktop session and leaves it again.
    Launch(LaunchOptions),
    /// Plans a project's generated files and their commit policy (task T15.01).
    Artifact(ArtifactOptions),
    /// Runs the generation pipeline with its target, style and layout prechecks (task T15.02).
    Generate(GenerateOptions),
    /// Builds a node-to-source map and reports its staleness (task T15.03).
    Mapping(MappingOptions),
    /// Merges a regenerated body into the committed file, keeping hand-written regions (task T15.04).
    GenMerge(GenMergeOptions),
    /// Reads a generated file back into a model candidate (task T15.05).
    Reverse(ReverseOptions),
    /// Recalculates positions, guards a regeneration loop and undoes across views (task T15.06).
    Position(PositionOptions),
    /// Commits generated files and states the standalone build contract (task T15.07).
    Export(ExportOptions),
    /// Compares generation against recorded goldens (task T15.08).
    Golden(GoldenOptions),
    /// Probes a toolchain and chooses a feature set (task T16.01).
    Toolchain(ToolchainOptions),
    /// Checks, builds or rebuilds (task T16.02).
    Compile(CompileOptions),
    /// Reads a build's problems and links them to nodes (task T16.03).
    BuildDiag(BuildDiagOptions),
    /// Runs the delivered program on its own (task T16.04).
    Run(RunOptions),
    /// Orders configured tasks and checks their cache (task T16.05).
    Tasks(TasksOptions),
    /// Drives a terminal session (task T16.06).
    Terminal(TerminalOptions),
    /// Configures and invokes an external tool (task T16.07).
    External(ExternalOptions),
    /// Records the generate → check → build → run → feedback walk (task T16.08).
    E2e(E2eOptions),
    /// Builds a typed task graph and matches its output (task T16.09).
    Dag(DagOptions),
    /// States a reproducible, offline build (task T16.10).
    Reproducible(ReproducibleOptions),
    /// Runs the headless walk and exports its CI (task T16.11).
    Headless(HeadlessOptions),
    /// Defines components, instances, parameters, slots and cycle checks (task T12.01).
    Component(ComponentOptions),
    /// Extracts, inserts and edits a component and shows the affected instances (task T12.02).
    ComponentAuthor(ComponentAuthorOptions),
    /// Applies variants and local overrides, and detaches/resets with an inverse (task T12.03).
    ComponentOverride(ComponentOverrideOptions),
    /// Registers a custom widget's schema, events and target contracts (task T12.04).
    CustomWidget(CustomWidgetOptions),
    /// Imports assets and maintains their references, thumbnails and repair (task T12.05).
    ResourceImport(ResourceImportOptions),
    /// Checks a resource's per-target variant and capacity (task T12.06).
    ResourceTarget(ResourceTargetOptions),
    /// Imports and exports component, asset and template packages (task T12.07).
    AssetPackage(AssetPackageOptions),
    /// Upgrades a definition across instances, paste and asset deletion (task T12.08).
    ComponentUpgrade(ComponentUpgradeOptions),
    /// States the asset manifest, packaging and derived-resource provenance (task T12.09).
    AssetManifest(AssetManifestOptions),
    /// Designs the target application's window shell and its capability seams (task T26.01).
    AppShell(AppShellOptions),
    /// Declares the shared action model and its state, icons and shortcuts (task T26.02).
    AppAction(AppActionOptions),
    /// Edits the target application's menu hierarchy, separators and roles (task T26.03).
    AppMenu(AppMenuOptions),
    /// Edits the target application's tool bar and status bar (task T26.04).
    AppToolbar(AppToolbarOptions),
    /// Edits a context menu, its scope parameters and enablement (task T26.05).
    AppContext(AppContextOptions),
    /// Designs dialogs, pop-ups and wizards with modality and focus (task T26.06).
    AppDialog(AppDialogOptions),
    /// States the target's dock/split layout capability (task T26.07).
    AppDock(AppDockOptions),
    /// Walks the app shell across preview and both generated modes (task T26.08).
    AppRegression(AppRegressionOptions),
    /// Defines collection content, item/column identity, samples and data source (task T27.01).
    Collection(CollectionOptions),
    /// Edits table columns, sorting, filtering and the empty state (task T27.02).
    Table(TableOptions),
    /// Edits tree/list items, hierarchy, lazy loading and icon roles (task T27.03).
    Tree(TreeOptions),
    /// Edits a cell through a delegate with validate/commit/cancel (task T27.04).
    CellEdit(CellEditOptions),
    /// Manages the internal pages of tab/stack/scroll/split containers (task T27.05).
    Container(ContainerOptions),
    /// Edits rich text, images and icons with fonts, links and alt text (task T27.06).
    RichContent(RichContentOptions),
    /// Declares the supported graphic/chart/media views and their placeholders (task T27.07).
    View(ViewOptions),
    /// Walks a collection across large/empty/error and both generated modes (task T27.08).
    CollectionRegression(CollectionRegressionOptions),
    /// Validates a form with typed rules and async staleness (task T13.09).
    Form(FormOptions),
    /// Reduces events against the shared business core (task T17.01).
    LogicCore(LogicCoreOptions),
    /// Opens a typed local channel and moves frames (task T17.02).
    LogicIpc(LogicIpcOptions),
    /// Validates actions and queues them across the boundary (task T17.03).
    LogicActions(LogicActionsOptions),
    /// Debounces edits and admits a candidate build (task T17.04).
    LogicCandidate(LogicCandidateOptions),
    /// Plans a state migration and confirms a reset (task T17.05).
    LogicMigration(LogicMigrationOptions),
    /// Walks the controlled replacement of the running logic (task T17.06).
    LogicSwap(LogicSwapOptions),
    /// Judges readiness and isolates a late result (task T17.07).
    LogicCombination(LogicCombinationOptions),
    /// Runs the event vectors and decides on a fault (task T17.08).
    LogicRegression(LogicRegressionOptions),
}

/// One display on the command line, written `<id>:<x>,<y>,<w>,<h>,<scale>`.
///
/// The scale is held in thousandths so a display argument is a whole-number value: the command
/// type is compared with `==` in the parser's own tests, and a float would make that comparison a
/// question about rounding rather than about the arguments.
#[derive(Debug, PartialEq, Eq)]
pub struct DisplayArg {
    /// The display's id.
    pub id: u64,
    /// The left edge in logical coordinates.
    pub x: i32,
    /// The top edge.
    pub y: i32,
    /// The width.
    pub width: u32,
    /// The height.
    pub height: u32,
    /// The DPI scale factor, in thousandths (1000 = 1.0).
    pub scale_milli: u32,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ParseError {
    argument: OsString,
}

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "unexpected argument `{}`",
            self.argument.to_string_lossy()
        )
    }
}

impl std::error::Error for ParseError {}

/// Reads the global `--log-level <level>` option wherever it appears.
///
/// It is global rather than per-command because the logger is installed once for the process: a
/// `--launch` that logs and a `--probe-apis` that logs must not each invent their own switch.
/// Reads a display argument: `<id>:<x>,<y>,<w>,<h>,<scale>`.
pub fn parse_display(text: &str) -> Option<DisplayArg> {
    let (id, rest) = text.split_once(':')?;
    let id = id.trim().parse::<u64>().ok()?;
    let parts: Vec<&str> = rest.split(',').collect();
    let [x, y, width, height, scale] = parts.as_slice() else {
        return None;
    };
    let scale = scale.trim().parse::<f64>().ok()?;
    if !(scale.is_finite() && scale > 0.0) {
        return None;
    }
    let scale_milli = (scale * 1000.0).round() as u32;
    if scale_milli == 0 {
        return None;
    }
    Some(DisplayArg {
        id,
        x: x.trim().parse().ok()?,
        y: y.trim().parse().ok()?,
        width: width.trim().parse().ok()?,
        height: height.trim().parse().ok()?,
        scale_milli,
    })
}

/// Reads a size argument: `<w>x<h>`.
pub fn parse_size(text: &str) -> Option<(u32, u32)> {
    let (width, height) = text.split_once('x')?;
    Some((width.trim().parse().ok()?, height.trim().parse().ok()?))
}

pub fn log_level_of<I, S>(arguments: I) -> Result<Option<Level>, ParseError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let arguments: Vec<OsString> = arguments
        .into_iter()
        .map(|value| value.as_ref().to_os_string())
        .collect();
    let mut level = None;
    let mut index = 0;
    while index < arguments.len() {
        if arguments[index] == OsStr::new("--log-level") {
            let Some(value) = arguments.get(index + 1) else {
                return Err(ParseError {
                    argument: arguments[index].clone(),
                });
            };
            let Some(parsed) = value.to_str().and_then(Level::parse) else {
                return Err(ParseError {
                    argument: value.clone(),
                });
            };
            level = Some(parsed);
            index += 2;
            continue;
        }
        index += 1;
    }
    Ok(level)
}

/// Reads the global `--ui-language <tag>` option wherever it appears (task T24.03).
///
/// The interface language is a process-wide choice, so it is read here the same way `--log-level`
/// is: the value is validated now (an unknown language is a usage error) and the command itself does
/// not have to know about it. Whether the language is *bundled* is decided later, by
/// [`crate::app::locale::catalog::select`], because that is a fact about the build rather than about
/// the argument. The name is `--ui-language` rather than `--language` because the latter already
/// names the language-service command and the `--highlight` option.
pub fn ui_language_of<I, S>(arguments: I) -> Result<Option<Language>, ParseError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let arguments: Vec<OsString> = arguments
        .into_iter()
        .map(|value| value.as_ref().to_os_string())
        .collect();
    let mut language = None;
    let mut index = 0;
    while index < arguments.len() {
        if arguments[index] == OsStr::new("--ui-language") {
            let Some(value) = arguments.get(index + 1) else {
                return Err(ParseError {
                    argument: arguments[index].clone(),
                });
            };
            let Some(parsed) = value.to_str().and_then(Language::parse) else {
                return Err(ParseError {
                    argument: value.clone(),
                });
            };
            language = Some(parsed);
            index += 2;
            continue;
        }
        index += 1;
    }
    Ok(language)
}

pub fn help_text() -> &'static str {
    concat!(
        "rustage - Rust desktop UI designer\n",
        "\n",
        "     Usage: rustage [--help | --version | --launch [--exit-after-ms <ms>]]\n",
        "            rustage [--render-icons <directory> | --probe-apis <directory>]\n",
        "            rustage [--probe-host <directory> | --project-check <file>]\n",
        "            rustage [--locale-report <directory>]\n",
        "            rustage --new-project <directory> [--template <name>] [--name <name>]\n",
        "            rustage [--open-project <file> [--trust] [--diagnose] | --recent]\n",
        "            rustage [--project-tree <file>]\n",
        "            rustage --drop-entry <file> --document <path> [--fix] [--trust]\n",
        "            rustage --file-op <file> [--add <path> | --exclude <path> | --delete <path>]\n",
        "                                 [--move <from> <to>] [--apply] [--trust]\n",
        "\n",
        "            rustage --configuration <file> [--target <name>] [--override <key>=<value>]\n",
        "                                  [--remember | --forget]\n",
        "\n",
        "            rustage --import <file-or-directory> [--apply | --check-backup] [--trust]\n",
        "            rustage --validate <file> [--diagnose] [--trust]\n",
        "                                 [--record <file> | --rescan <file>]\n",
        "            rustage --kit <file> [--diagnose] [--trust] [--triple <triple>]\n",
        "            rustage --edit <page-document> [--batch <file>] [--root <directory>]\n",
        "                                       [--apply] [--undo]\n",
        "            rustage --document <page-document> [--root <directory>]\n",
        "                                       [--set-text <text> | --set-text-file <file>]...\n",
        "            rustage --history <page-document> [--root <directory>]\n",
        "                                      [--text <text> | --text-file <file> |\n",
        "                                       --batch <file> | --undo <n> | --redo <n>]...\n",
        "                                      [--merge] [--only <text|model>]\n",
        "            rustage --dependencies <directory> [--review <record> | --record <record>]\n",
        "                                               [--allow-lockfile-change] [--allow-network]\n",
        "                                               [--trust]\n",
        "            rustage --new-workspace <directory> [--name <name>]\n",
        "            rustage --workspace <file> [--add <name>=<root> | --remove <name> |\n",
        "                                       --activate <name> | --file <member>:<path>]\n",
        "                                      [--apply] [--trust]\n",
        "\n",
        "     With no options, prints the current project status.\n",
        "\n",
        "     Options:\n",
        "       -h, --help                    Print this help message\n",
        "       -V, --version                 Print the application version\n",
        "       --launch                      Start the baseline desktop session, then leave it again\n",
        "       --exit-after-ms <ms>          With --launch, leave the event loop after <ms>\n",
        "       --render-icons <directory>    Render the icon strip, light and dark, into <directory>\n",
        "       --probe-apis <directory>      Run the dependency contract probes and write api-probes.json\n",
        "       --probe-host <directory>      Check the host platform services and write host-readiness.json\n",
        "       --locale-report <directory>   Write the English language pack and the interface-vocabulary\n",
        "                                     report into <directory>\n",
        "       --project-check <file>        Read a project file, report what is understood and what is\n",
        "                                     kept, and write <file>.roundtrip.json as evidence\n",
        "       --new-project <directory>     Create a project; --template empty|basic-page|application\n",
        "                                     (default: basic-page), --name <name> (default: the directory)\n",
        "       --open-project <file>         Open a project read-only, and print what would be asked\n",
        "       --trust                       With --open-project, answer the question with yes\n",
        "       --diagnose                    With --open-project, read the toolchain (runs two programs)\n",
        "       --recent                      List the projects opened before, and where they are now\n",
        "       --project-tree <file>         Show what the project has, classified, with what is on disk\n",
        "       --drop-entry <file>           Remove the entry in --document; refuses while anything points\n",
        "                                     at it unless --fix is given, which fixes those references\n",
        "       --document <path>             With --drop-entry, the page document of the entry\n",
        "       --fix                         With --drop-entry, fix the references instead of refusing\n",
        "       --file-op <file>              Plan one file operation on the project; --add, --exclude,\n",
        "                                     --delete <path> or --move <from> <to> says which. Prints\n",
        "                                     what it would do unless --apply is given, which does it\n",
        "       --apply                       With --file-op, do it: move the files, then write the\n",
        "                                     document, putting the moves back if the write fails\n",
        "\n",
        "       --configuration <file>        Show which target would build this project, what each\n",
        "                                     value came from, and what this machine overrode\n",
        "       --target <name>               With --configuration, resolve for this target\n",
        "       --override <key>=<value>      With --configuration, set `profile` or `features` for this\n",
        "                                     run; a list is comma-separated\n",
        "       --remember                    With --configuration, record the target and the overrides\n",
        "                                     for this project on this machine\n",
        "       --forget                      With --configuration, forget everything recorded for it\n",
        "\n",
        "       --import <file-or-directory>  Read an existing project, say what it is, and what an\n",
        "                                     import would change; --apply migrates it, keeping the file\n",
        "                                     that was read, and --check-backup reports what was kept\n",
        "       --check-backup                With --import, report the files a past migration kept\n",
        "       --validate <file>             Check what the project needs and whether it is there, and\n",
        "                                     change nothing: the document is read, never written\n",
        "       --diagnose                    With --validate or --open-project, read the toolchain, which\n",
        "                                     runs two programs and needs the project to be trusted\n",
        "       --record <file>               With --validate, write a scan of the project to <file>, so a\n",
        "                                     later --rescan has something to compare against\n",
        "       --rescan <file>               With --validate, compare the project with the scan recorded in\n",
        "                                     <file> and name what appeared, vanished or changed\n",
        "       --kit <file>                  Show which tools a kit is made of, and what this machine has\n",
        "                                     of them; --diagnose asks it about each member (runs the\n",
        "                                     programs the kit names), --triple says which output to ask about\n",
        "       --triple <triple>             With --kit, the output triple to ask about: a kit never\n",
        "                                     carries which profile is being built\n",
        "       --edit <page-document>        Plan a batch of edits to a page document's model: what each\n",
        "                                     operation would do, what cannot land, and what would undo it\n",
        "       --batch <file>                With --edit, the batch document to run\n",
        "       --apply                       With --edit, apply the batch to the model in memory (writing\n",
        "                                     the file is T03.04's job)\n",
        "       --undo                        With --edit --apply, apply the batch's inverse afterwards\n",
        "       --document <page-document>    Show what a buffer over a page document is holding: the text,\n",
        "                                     the last model that read it, and what happened to identity\n",
        "       --root <directory>            With --document, the project root the file is read against\n",
        "                                     (default: the nearest project above the file)\n",
        "       --set-text <text>             With --document, replace the text with this text; repeatable,\n",
        "                                     and the edits happen in the order they are given\n",
        "       --set-text-file <file>        With --document, read this file's text and use it as an edit\n",
        "       --history <page-document>     Replay a script of edits and moves against a page document:\n",
        "                                     text and model edits are one history in two domains, so one\n",
        "                                     drag is one thing to take back\n",
        "       --text <text>                 With --history, replace the document's text with this text\n",
        "       --text-file <file>            With --history, read this file's text and use it as the text\n",
        "       --merge                       With --history, make every batch in the run one step (one\n",
        "                                     drag is one thing to take back)\n",
        "       --undo <n>                    With --history, take back n steps, here in the script\n",
        "       --redo <n>                    With --history, put back n steps, here in the script\n",
        "       --only <text|model>           With --history, require the moves to be in this domain; a\n",
        "                                     step in the other domain is refused, not skipped over\n",
        "       --save <page-document>...     Write each document's text back to disk: through a\n",
        "                                     temporary and a rename, so a failed write leaves the\n",
        "                                     original exactly as it was; more than one file is one set,\n",
        "                                     all committed or all left as they were\n",
        "       --set-text <text>             With --save (one file), type this text into the buffer\n",
        "                                     before saving; repeatable, in the order given\n",
        "       --set-text-file <file>        With --save (one file), read this file's text and type it\n",
        "       --save-as <path>              With --save, write the text to this project-relative path\n",
        "                                     and make it the document's file\n",
        "       --save-copy <path>            With --save, write a copy there, leaving the document's own\n",
        "                                     file alone\n",
        "       --plan                        With --save, print what would be written and write nothing\n",
        "       --conflict <page-document>    Decide what a document should do when the file changed\n",
        "                                     outside the designer: report the base/mine/theirs verdict\n",
        "                                     and never pick a side silently\n",
        "       --set-text <text>             With --conflict, type this text as the document's own\n",
        "                                     before deciding; repeatable, in the order given\n",
        "       --set-text-file <file>        With --conflict, read this file's text as an edit\n",
        "       --base <file>                 With --conflict, the bytes the document last agreed with\n",
        "                                     the file (replays the sequence a one-shot run cannot show)\n",
        "       --accept                      With --conflict, write the merged text when a merge landed\n",
        "       --recreate                    With --conflict, write the document's text back when the\n",
        "                                     file has gone\n",
        "       --recovery <store-dir>        Keep, offer and refuse recovery copies, which live outside\n",
        "                                     any project; with no action, list a project's copies and\n",
        "                                     how each stands next to the file now\n",
        "       --record <page-document>      With --recovery, keep a copy of this document's text\n",
        "       --restore <page-document>     With --recovery, print the newest copy for this document\n",
        "       --discard <page-document>     With --recovery, refuse: remove this document's copies\n",
        "                                     (the document itself is never touched)\n",
        "       --session <name>              With --recovery, the session a copy is filed under\n",
        "       --prune                       With --recovery, plan a retention prune: keep the newest\n",
        "                                     copies per document, and never offer a project file\n",
        "       --keep <n>                    With --recovery --prune, how many copies to keep (default 5)\n",
        "       --apply                       With --recovery --prune, carry the prune out\n",
        "       --close <page-document>...   Plan and carry out closing: a dirty document needs a\n",
        "                                     decision (save all, discard all or cancel), and cancelling\n",
        "                                     keeps every document and task as it was\n",
        "       --unsaved <document>=<file>  With --close, a document whose buffer differs from its file;\n",
        "                                     the file holds the buffer's text\n",
        "       --task <name>                With --close, work that belongs to the closing\n",
        "       --decision <what>            With --close, save-all, discard-all or cancel\n",
        "       --command [<id>]              Resolve a typed command through the registry, or list\n",
        "                                     every registered command; a disabled or unauthorized\n",
        "                                     command is refused and says which it was\n",
        "       --arg <name>=<value>          With --command, one raw argument\n",
        "       --focus <area>                With --command, where the keyboard is (a mode or panel id)\n",
        "       --authorize <effect>          With --command, an authorized effect (command, file-write,\n",
        "                                     network, deploy)\n",
        "       --running <task>              With --command, a task that is already running\n",
        "       --keys [--press <chord>]      Resolve a key chord through the keymap, or list the\n",
        "                                     bindings; with --text-field a typed character goes to the\n",
        "                                     field, and with --composing the keys belong to the input method\n",
        "       --press <chord>               With --keys, the chord to press, as `Ctrl+K Ctrl+S`\n",
        "       --mode <mode>                 With --keys, the mode the designer is in\n",
        "       --panel <panel>               With --keys, the focused panel\n",
        "       --text-field                  With --keys, a text field has the keyboard\n",
        "       --composing                   With --keys, an input method is composing\n",
        "       --menu                        Show the menu tree and check every implemented leaf\n",
        "                                     against the registry\n",
        "       --palette [<query>]           Search the command palette (fuzzy, on the registry), or\n",
        "                                     list every command; a disabled command shows its reason\n",
        "       --help-for <command-id>       With --palette, show one command's help and parameters\n",
        "       --recent <command-id>         With --palette, a recently run command (repeatable; first\n",
        "                                     is most recent)\n",
        "       --workbench                   Show the workbench (title, regions, status) and its panels;\n",
        "                                     a panel whose service is missing is not shown\n",
        "       --mode <mode>                 With --workbench, switch to a mode\n",
        "       --project <name>              With --workbench, the active project's name\n",
        "       --target <name>               With --workbench, the target the project would build for\n",
        "       --document <path>             With --workbench, the active document\n",
        "       --dirty                       With --workbench, mark the document as having unsaved changes\n",
        "       --message <text>              With --workbench, a one-line status message\n",
        "       --service <name>              With --workbench, a service this host has (repeatable)\n",
        "       --show <panel>                With --workbench, show a panel (repeatable)\n",
        "       --hide <panel>                With --workbench, hide a panel (repeatable)\n",
        "       --focus <panel>               With --workbench, give a panel the keyboard\n",
        "       --resize <panel>=<n>          With --workbench, resize a panel\n",
        "       --operation <name>            Preview a batch operation on a selection, and carry it\n",
        "                                     out; an inapplicable item is never skipped silently\n",
        "       --select <id>                 With --operation, a selected item the operation applies to\n",
        "       --inapplicable <id>=<reason>  With --operation, a selected item it does not apply to\n",
        "       --effect <effect>             With --operation, an external effect it has\n",
        "       --note <text>                 With --operation, a note for the person\n",
        "       --choice <what>               With --operation, apply-all, apply-none or cancel\n",
        "       --audit                       Check the whole command inventory: every menu leaf and key\n",
        "                                     binding names a registered command\n",
        "       --tabs [--limit <n>] <op>...  Replay document-tab operations; each op is `verb[:arg]`,\n",
        "                                     as in preview:pages/home.json or close-view:0\n",
        "       --groups [--limit <n>] <op>... Replay editor-group operations: split:left-right,\n",
        "                                     focus:next, up/down/left/right, move:<from>:<i>:<to>:<j>,\n",
        "                                     open:<doc>, view:<doc>, type:<text>, scroll:<n>; the\n",
        "                                     same document in two groups reads one buffer\n",
        "       --clipboard                   Copy, round-trip or paste one clipboard payload; a\n",
        "                                     pasted subtree gets new ids, and a reference that means\n",
        "                                     another file here is surfaced rather than carried\n",
        "       --copy-text <text>            With --clipboard, copy this text\n",
        "       --copy-resource <p>:<path>    With --clipboard, copy a resource reference from <p>\n",
        "       --copy-node <file>#<id>       With --clipboard, copy the subtree rooted at node <id>\n",
        "                                     of a page document\n",
        "       --paste-into <project>        With --clipboard, paste the payload into this project\n",
        "       --roundtrip                   With --clipboard, encode then decode and say whether the\n",
        "                                     two agree\n",
        "       --limit <n>                   With --clipboard, the payload size ceiling in bytes\n",
        "       --layout [--limit <n>] <op>...  Replay layout operations: preset:<name>, save:<name>,\n",
        "                                     recall:<name>, zen:<on|off>, fullscreen:<on|off>,\n",
        "                                     lock, unlock, reset, load:<file>, write:<file>,\n",
        "                                     open:<doc>, split:<axis>; a reset keeps every document,\n",
        "                                     and a bad layout file is refused with nothing changed\n",
        "       --service <name>              With --layout, a service this host has (repeatable)\n",
        "       --windows [--display <d>]...  Replay window and display operations: view:<doc>,\n",
        "                                     float:<panel>, close:<id>, move:<win>:<display>,\n",
        "                                     unplug:<display>, plug:<display>, rescale:<d>:<s>,\n",
        "                                     reconcile; after an unplug or a scale change every\n",
        "                                     window is moved back into view\n",
        "       --display <id>:<x>,<y>,<w>,<h>,<scale>\n",
        "                                     With --windows, a display; the first is primary\n",
        "       --main <w>x<h>                With --windows, the main window's size\n",
        "       --chrome <op>...              Replay window-chrome operations: focus:<area>,\n",
        "                                     type:<text>, backspace:<n>, project:<name>,\n",
        "                                     document:<path>, node:<label>,\n",
        "                                     notice:<severity>:<source>:<summary>, toolbar, status,\n",
        "                                     clear; a notice arriving does not move the keyboard\n",
        "       --text [--file <f>] <op>...   Replay in-place text edits: splice:<at>:<remove>:<insert>,\n",
        "                                     undo, redo, show; a one-character edit copies no text\n",
        "       --file <f>                    With --text, read the starting text from <f>\n",
        "       --highlight [--language <l>]  Replay syntax highlighting: line:<text>, step:<n>,\n",
        "                                     set:<line>:<text>, language:<l>, show; an unlexed\n",
        "                                     line reads unknown, and drawing never lexes\n",
        "       --language <l>                With --highlight, the language (plain, rust, json, css)\n",
        "       --theme <name>                With --highlight, the theme (dark, light)\n",
        "       --coordinates [--tab <n>]     Convert document offsets to visual rows and columns and\n",
        "                                     back: line:<text>, fold:<a>:<b>, unfold:<a>, tab:<n>,\n",
        "                                     wrap:<n>, visual:<offset>, offset:<row>:<col>,\n",
        "                                     mark:<kind>:<a>:<b>, show; the cursor, a diagnostic and a\n",
        "                                     protected region share one conversion\n",
        "       --tab <n>                     With --coordinates, columns a tab advances to\n",
        "       --wrap <n>                    With --coordinates, wrap at this many columns\n",
        "       --edit-ops [--file <f>] <op>...\n",
        "                                     Run multi-cursor editing: line:<text>,\n",
        "                                     carets:<o>:..., select:<a>:<c>, insert:<text>,\n",
        "                                     type:<ch>, backspace, delete, up, down, duplicate,\n",
        "                                     delete-lines, comment:<prefix>, matching:<o>, undo,\n",
        "                                     redo, show; one keystroke at several carets is one\n",
        "                                     transaction, and no edit splits a character\n",
        "       --editor-view [--tab <n>]     Read a file's encoding, line endings and indentation and\n",
        "                                     [--file <f>] <op>...\n",
        "                                     show the decoration: line:<text>, whitespace,\n",
        "                                     numbers[:relative], window:<a>:<n>[:<cur>],\n",
        "                                     fold:indent|braces, minimap:<n>, encode,\n",
        "                                     encode-with:<lf|crlf|cr>, show; a file that is not UTF-8\n",
        "                                     or that mixes line endings is refused, not corrupted\n",
        "       --tab <n>                     With --editor-view, columns a tab advances to\n",
        "       --write-guard <op>...         Check a write against protected regions through every\n",
        "                                     entry point: protect:<a>:<b>, writable,\n",
        "                                     read-only[:<reason>], input:<at>:<n>, paste:<at>:<n>,\n",
        "                                     replace:<at>:<n>:<m>, format:<at>:<n>:<m>,\n",
        "                                     external:<at>:<n>, show; each door prints allowed or\n",
        "                                     refused, and one offence refuses the whole write\n",
        "       --scale [--file <f>] <op>...  Classify a document's size and show the degradation that\n",
        "                                     follows: line:<text>, fill:<bytes>, long-line:<n>,\n",
        "                                     read, read-cancel, show; every feature is on or off\n",
        "                                     with a reason, and opening is cancellable\n",
        "       --chunk <n>                   With --scale, the bytes a cancellable read reads at a time\n",
        "       --snippet <op>...             Expand a snippet and walk its placeholders: text:<t>,\n",
        "                                     snippet:<prefix>:<body>, protect:<s>:<e>, read-only,\n",
        "                                     cursor:<at>, select:<s>:<e>, add-caret:<at>,\n",
        "                                     add-select:<s>:<e>, expand:<prefix>, tab, shift-tab,\n",
        "                                     type:<text>, surround:<open>:<close>, cancel, undo,\n",
        "                                     show; a body is literal text and $n placeholders only,\n",
        "                                     and one that reaches a protected region is refused\n",
        "       --regression [--lines <n>]    Replay the editing scenarios and recheck the size\n",
        "                                     boundaries: normal, million, unicode, multi-view;\n",
        "                                     prints a duration distribution and the checks\n",
        "       --lines <n>                   With --regression, the line count for every scenario\n",
        "       --diff [--left <f>] [--right <f>]  Compare two texts or trees: left:<t>, right:<t>,\n",
        "                                     tree-left:<p>=<t>, tree-right:<p>=<t>, show, trees,\n",
        "                                     read-only; a line-ending or BOM difference is a\n",
        "                                     difference unless --ignore-endings/--ignore-trailing\n",
        "       --ignore-endings              With --diff, treat CRLF and LF as the same ending\n",
        "       --ignore-trailing             With --diff, ignore trailing whitespace before an ending\n",
        "       --structural <op>...          Grow a selection and toggle display elements: text:<t>,\n",
        "                                     cursor:<at>, select:<s>:<e>, expand:<n>, syntax:<n>,\n",
        "                                     bookmark:<name>:<at>, drop:<name>,\n",
        "                                     edit:<at>:<remove>:<insert>, toggle:<name>,\n",
        "                                     baseline:<text>, inline, show; without a syntax\n",
        "                                     capability selection falls back to text\n",
        "       --language <op>...            Drive the language-service lifecycle: caps:<c1>,<c2>,\n",
        "                                     fail:<toolchain|spawn|handshake>, view:<project>,\n",
        "                                     restart:<project>, close:<project>, show; one\n",
        "                                     session per project, not per view\n",
        "       --protocol <op>...            Synchronise documents and convert positions:\n",
        "                                     open:<uri>=<t>, replace:<uri>=<t>,\n",
        "                                     edit:<uri>:<v>:<sl>,<sc>:<el>,<ec>=<t>, save:<uri>,\n",
        "                                     close:<uri>, units:<uri>:<byte>,\n",
        "                                     offset:<uri>:<line>,<char>, show; positions are\n",
        "                                     UTF-16, offsets are bytes\n",
        "       --features <op>...            Drive completion freshness: version:<n>, request,\n",
        "                                     cancel:<id>, respond:<id>:<version>:<a,b>, show;\n",
        "                                     a reply for another version never becomes the list\n",
        "       --navigation <op>...          Answer definitions, references and symbols, with the\n",
        "                                     outline and breadcrumbs: caps:<c1>,<c2>,\n",
        "                                     symbol:<uri>:<name>:<kind>:<s>:<e>:<ns>:<ne>,\n",
        "                                     definition/references/declaration/implementation:<name>,\n",
        "                                     document-symbol:<uri>, workspace-symbol:<name>,\n",
        "                                     outline:<uri>, breadcrumbs:<uri>:<offset>, show;\n",
        "                                     an unnegotiated capability reads as disabled\n",
        "       --diagnostics <op>...        Drive diagnostics, tokens and code actions: version:<n>,\n",
        "                                     diag:<severity>:<source>:<s>:<e>:<message>,\n",
        "                                     publish:<version>, protect:<s>:<e>, mark,\n",
        "                                     token:<kind>:<s>:<e>, tokens:<version>,\n",
        "                                     action:<title>:<at>:<remove>:<insert>, apply, show;\n",
        "                                     a fix across a generated region is refused whole\n",
        "       --workspace-edit <op>...      Review and commit a cross-file edit as one transaction:\n",
        "                                     label:<t>, open:<uri>:<version>=<t>,\n",
        "                                     protect:<uri>:<s>:<e>,\n",
        "                                     edit:<uri>:<baseline>:<at>:<remove>:<insert>,\n",
        "                                     review, commit, show; a protected region or an\n",
        "                                     external change blocks a half-commit\n",
        "       --schema <op>...              Show schema docs and target-relative completion:\n",
        "                                     lang:<rust|json|css>, entry:<key>:<requires|->:<detail>,\n",
        "                                     target:<name>:<cap1>,<cap2>, complete:<prefix>,\n",
        "                                     doc:<key>, show; completion omits what the\n",
        "                                     current target forbids\n",
        "       --health [crash|slow|no-toolchain|multi-project]...\n",
        "                                     Run the language-service failure regression; every\n",
        "                                     scenario proves basic editing still works\n",
        "       --hints <op>...               Show hints, lenses and hierarchies: caps:<c1>,<c2>,\n",
        "                                     version:<n>, inlay:<s>:<e>:<label>,\n",
        "                                     lens:<s>:<e>:<label>:<id>:<title>, publish:<v>,\n",
        "                                     dismiss:<kind>, invoke:<id>,\n",
        "                                     hierarchy:<call|type>:<name>, show; a lens runs a\n",
        "                                     command only when invoked\n",
        "       --analysis <op>...            Rebuild the analysis configuration: target:<name>,\n",
        "                                     known:<name>, member:<name>, feature:<name>,\n",
        "                                     kit:<name>, trust:<trusted|untrusted>, configure,\n",
        "                                     publish:<generation>, show; macros/build scripts\n",
        "                                     need trust, and old diagnostics are not current\n",
        "       --index <op>...               Build the project file index: node:<path>:<kind>:<size>,\n",
        "                                     ignore:<p1>,<p2>, binary:<e1>,<e2>, max-bytes:<n>,\n",
        "                                     max-depth:<n>, max-entries:<n>, build,\n",
        "                                     create:/remove:/modify:<path>, show; a symlink is\n",
        "                                     listed, never followed, and the walk is bounded\n",
        "       --search <op>...              Search file and project text: file:<path>=<t>,\n",
        "                                     pattern:<t>, regex, literal, no-case, whole-word,\n",
        "                                     range:<s>:<e>, max:<n>, search, cancel, show;\n",
        "                                     a bad pattern, a cancel and a truncation are all\n",
        "                                     visible in the result\n",
        "       --quick-open <op>...          Open files and symbols by a query: file:<path>,\n",
        "                                     touch:<path>, caps:<c1>,<c2>, symbol:<name>:<path>,\n",
        "                                     open:<query>, file-symbols:<query>,\n",
        "                                     workspace-symbols:<query>, show; file search works\n",
        "                                     with no service, symbol search does not\n",
        "       --replace <op>...             Build replace candidates: file:<path>=<t>, pattern:<t>,\n",
        "                                     regex, no-case, whole-word, template:<t>,\n",
        "                                     protect:<uri>:<s>:<e>, read-only:<uri>, build,\n",
        "                                     check:<i>, uncheck:<i>, check-all, uncheck-all,\n",
        "                                     show; the replacement syntax is checked first\n",
        "                                     and a protected region is never modified\n",
        "       --batch <op>...               Run a batch replace: open:<uri>:<version>=<t>,\n",
        "                                     protect:<uri>:<s>:<e>,\n",
        "                                     edit:<uri>:<baseline>:<at>:<remove>:<insert>,\n",
        "                                     run, undo, show; a mid-batch failure is partial\n",
        "                                     and recoverable, never reported as done\n",
        "       --history-nav <op>...         Walk the navigation history: visit:<uri>:<offset>,\n",
        "                                     edit:<uri>:<offset>, problem:<uri>:<offset>, back,\n",
        "                                     forward, rename:<from>:<to>, vanished:<uri>,\n",
        "                                     resolve, show; a vanished file is refused,\n",
        "                                     never redirected to a different file\n",
        "       --search-health [big-project|continuous-edits|conflict|cancel]...\n",
        "                                     Run the search/replace acceptance regression; results\n",
        "                                     carry a version and a stale candidate is re-checked\n",
        "       --catalog <op>...             Show the widget catalog: widget:<id>:<name>:<category>,\n",
        "                                     unsupported:<id>:<reason>, property:<id>:<name>:<type>,\n",
        "                                     limit:<id>:<note>, search:<query>, category:<name>,\n",
        "                                     favorite:<id>, unfavorite:<id>, schema:<id>, show;\n",
        "                                     names come from the contract, unsupported has a reason\n",
        "       --scene <op>...               Drive the design scene: add:<node>:<x>,<y>,<w>,<h>,\n",
        "                                     rebuild:<node>:<x>,<y>,<w>,<h>, remove:<node>,\n",
        "                                     hit:<x>,<y>, resolve:<node>:<gen>, raise:<node>,\n",
        "                                     show; a handle from before a delete or rebuild is\n",
        "                                     refused, not read as the new instance\n",
        "       --structure <op>...           Structural edits on the object tree: root:<kind>:<name>,\n",
        "                                     insert:<parent>:<kind>:<name>, delete:<node>,\n",
        "                                     copy:<node>, rename:<node>:<name>,\n",
        "                                     reorder:<node>:<index>, reparent:<node>:<parent>,\n",
        "                                     reference:<from>:<to>, show; no cycle or\n",
        "                                     duplicate id, and a delete lists its references\n",
        "       --viewport <op>...            Pan, zoom, fit, actual size and the design aids:\n",
        "                                     transform:<px>,<py>,<zoom>, pan:<dx>,<dy>,\n",
        "                                     zoom:<f>:<vw>,<vh>, zoom-at:<f>:<x>,<y>,\n",
        "                                     fit:<x>,<y>,<w>,<h>:<vw>,<vh>[:<margin>],\n",
        "                                     actual-size:<vw>,<vh>, spacing:<s>, grid:<on|off>,\n",
        "                                     snap:<on|off> or snap:<x>,<y>,\n",
        "                                     align:<x>,<y>,<w>,<h>:<x>,<y>,<w>,<h>[:<tol>],\n",
        "                                     to-screen:<x>,<y>, to-model:<x>,<y>,\n",
        "                                     overlay:<vw>,<vh>, show; the aids never touch the model\n",
        "       --drag <op>...                Drag a node: root:<kind>:<name>,\n",
        "                                     insert:<parent>:<kind>:<name>, begin:<node>,\n",
        "                                     candidates, aim:<parent|->:<index>, preview,\n",
        "                                     release, cancel, stage:<node>:<parent|->:<index>,\n",
        "                                     commit, discard, undo, redo, show; an invalid\n",
        "                                     drop writes nothing, and one undo takes back a batch\n",
        "       --objects <op>...             Design-time object state and pages: node:<id>:<kind>\n",
        "                                     [:visible=<bool>][:name=<text>], hide:<id>, unhide:<id>,\n",
        "                                     lock:<id>, unlock:<id>, raise:<id>, lower:<id>,\n",
        "                                     published:<id>, visible:<id>, filter-text:<text>,\n",
        "                                     filter-kind:<kind>, filter-state:<hidden|shown|locked|unlocked>,\n",
        "                                     filter-clear, filter, locate:<text>,\n",
        "                                     page:<id>:<name>:<root>, activate:<id>, close-page:<id>,\n",
        "                                     rename-page:<id>:<name>, state, show; hiding never\n",
        "                                     changes published visibility\n",
        "       --design <op>...              The design surface's keyboard and stress harness:\n",
        "                                     node:<parent|->:<kind>:<name>, select:<node>, all,\n",
        "                                     deselect, delete, undo, redo, next, previous, parent,\n",
        "                                     child, press:<chord>, stress:<nodes>:<rounds>, state,\n",
        "                                     show; the canvas is derived from the tree, so the two\n",
        "                                     agree, and a bounded history keeps memory from growing\n",
        "       --layout-plan <op>...         The layout contract and its diagnostics: available:<node>:<w>,<h>,\n",
        "                                     constraint:<node>:<property>:<origin>, box:<node>:<x>,<y>,<w>,<h>,\n",
        "                                     run:<preview|generation>, source:<node>, state, show;\n",
        "                                     preview and generation share one engine, not an algorithm\n",
        "       --layout-switch <op>...       Choose/switch/detach a container's layout: layout:<kind>,\n",
        "                                     child:<node>:<property>, switch, detach, state, show;\n",
        "                                     a conversion that cannot be expressed is refused with\n",
        "                                     the loss explained\n",
        "       --constraints <op>...         A node's box constraints: min:<node>:<w>,<h>,\n",
        "                                     max:<node>:<w>,<h>, preferred:<node>:<w>,<h>,\n",
        "                                     grow:<node>:<f>, margin:<node>:<f>, gap:<node>:<f>,\n",
        "                                     overflow:<node>:<visible|hidden|scroll|clip>, validate,\n",
        "                                     state, show; an illegal or conflicting constraint is\n",
        "                                     located at the node and the property\n",
        "       --arrange <op>...             Align/distribute/match-size and the nudge, as constraint\n",
        "                                     writes: layout:<kind>, select:<node>, align:<axis>:<mode>,\n",
        "                                     distribute:<axis>, match:<axis>,\n",
        "                                     nudge:<axis>:<direction>:<step>[:<spacing>], state, show;\n",
        "                                     an operation changes real constraints, never a coordinate\n",
        "       --frames <op>...              Check simulated frames against a target's support:\n",
        "                                     support-size:<w>,<h>, support-orientation:<portrait|landscape>,\n",
        "                                     support-safe-area:<true|false>, support-max-scale:<f>,\n",
        "                                     frame:<label>:<w>,<h>:<orientation>, frame-scale:<label>:<f>,\n",
        "                                     frame-insets:<label>:<t>,<r>,<b>,<l>, check, state, show;\n",
        "                                     each difference from the target is named and classified\n",
        "       --focus-order <op>...         The focus order (tab sequence): root:<kind>:<name>,\n",
        "                                     insert:<parent>:<kind>:<name>, focusable:<node>,\n",
        "                                     exclude:<node>, index:<node>:<n>, build, step:<node>:<delta>,\n",
        "                                     export, state, show; the order is exported as the\n",
        "                                     target's focus semantics, and unreachable stops are named\n",
        "       --variants <op>...            Size conditions and layout variants: capability:<kind>,\n",
        "                                     variant:<name>, when:<condition>, value:<key>=<value>, add,\n",
        "                                     state, show; a condition the target cannot evaluate\n",
        "                                     blocks the variant instead of being invented\n",
        "       --layout-probe <op>...        Compare the preview with a library-generated probe:\n",
        "                                     scenario:<name>, available:<node>:<w>,<h>,\n",
        "                                     box:<node>:<x>,<y>,<w>,<h>, probe-box:<node>:<x>,<y>,<w>,<h>,\n",
        "                                     compare, state, show; both sides run on the same input\n",
        "                                     and a divergence names the node and both values\n",
        "       --property-editor <op>...    The schema-driven property editor: schema:<property>:<type>,\n",
        "                                     set:<node>:<property>:<text>, value:<node>:<property>,\n",
        "                                     pending:<node>:<property>, state, show; a refused edit\n",
        "                                     keeps the text and never changes the valid model\n",
        "       --multi-property <op>...      A multi-selection's shared facts: node:<id>,\n",
        "                                     set:<property>:<value>:<origin>, common, value:<property>,\n",
        "                                     reset:<node>:<property>[:<fallback>], state, show; the common\n",
        "                                     set is the intersection, and a reset restores the source\n",
        "       --css <op>...                 The CSS rule view and computed style: css:<rule text>,\n",
        "                                     element:<compound>, value:<property>, source:<property>,\n",
        "                                     properties, state, show; CSS it cannot read is kept with a\n",
        "                                     reason, and a computed value names the rule it came from\n",
        "       --tokens <op>...              Design tokens and themes: token:<kind>:<name>:<value>,\n",
        "                                     theme:<name>:<token>=<value>[,...], use:<property>:<value>,\n",
        "                                     expand[:<theme>], rename:<from>:<to>, state, show; the model\n",
        "                                     keeps the reference and only expansion inlines it, traceably\n",
        "       --states <op>...              Interaction state styles and theme/state diff review:\n",
        "                                     css:<rule text>, element:<compound>, on:<state>,\n",
        "                                     declared:<event>, token:<kind>:<name>:<value>,\n",
        "                                     theme:<name>:<token>=<value>[,...], preview,\n",
        "                                     compare:<left>[:<right>], diff:<state>:<state>, state, show;\n",
        "                                     previewing a state triggers no business event\n",
        "       --motion <op>...              Transitions/animations and their preview: transitions:<on|off>,\n",
        "                                     animations:<on|off>, property:<name>, easing:<name>,\n",
        "                                     transition:<property>:<ms>:<delay>:<easing>,\n",
        "                                     animation:<property>:<ms>:<delay>:<easing>, check, play, pause,\n",
        "                                     seek:<ms>, state, show; the support table comes from the\n",
        "                                     contract and a restricted target refuses incompatible items\n",
        "       --style <op>...               One style emitter for both consumers: css:<rule text>,\n",
        "                                     element:<compound>, inline:<property>:<value>,\n",
        "                                     token:<kind>:<name>:<value>, theme:<name>:<token>=<value>[,...],\n",
        "                                     under:<theme|->, preview, generate, migrate, state, show;\n",
        "                                     preview and generation carry the same values\n",
        "       --style-boundary <op>...      Style edges and undo: css:<rule text>, element:<compound>,\n",
        "                                     token:<kind>:<name>:<value>, theme:<name>:<token>=<value>[,...],\n",
        "                                     inline:<property>:<value>, clear:<property>, rename:<from>:<to>,\n",
        "                                     use-theme:<theme|->, undo, emit, contrast:<a>:<b>, state, show;\n",
        "                                     an undo restores the reference and the computed value\n",
        "       --data <op>...                Data schema and datasets: schema:<name>:<version>,\n",
        "                                     field:<name>:<kind>[:required], sample:<field>=<value>,\n",
        "                                     runtime:<field>=<value>, error:<reason>, check, active,\n",
        "                                     state, show; a sample is labelled and never stands in for\n",
        "                                     runtime data, and it refuses to store a credential\n",
        "       --binding <op>...             Typed bindings between data and properties:\n",
        "                                     schema:<name>:<version>, field:<name>:<kind>,\n",
        "                                     slot:<node>.<property>:<type>,\n",
        "                                     bind:<source>=<target>:<direction>:<conversion>, check,\n",
        "                                     state, show; an incompatible pair, a lossy two-way binding\n",
        "                                     or a cycle is refused and named\n",
        "       --events <op>...              Event declarations and wiring: event:<name>,\n",
        "                                     field:<event>:<field>:<kind>[:required],\n",
        "                                     attach:<node>:<event>:<handler>, status:<node>:<event>,\n",
        "                                     fire:<node>:<event>:<field>=<value>[,...], check, state, show;\n",
        "                                     declared, wired and fired are three separate answers\n",
        "       --dispatch <op>...            Handler actions and permissions: slot:<node>.<prop>:<type>,\n",
        "                                     command:<id>, function:<name>, trust, authorize:<capability>,\n",
        "                                     bind:<node>:<event>:set:<prop>=<value> | command:<id> |\n",
        "                                     call:<fn>:<capability>, unbind:<node>:<event>,\n",
        "                                     dispatch:<node>:<event>, check, state, show; replacing or\n",
        "                                     removing a handler releases its subscription\n",
        "       --routes <op>...              Page routes and a window's life: page:<name>,\n",
        "                                     entry:<name>, edge:<from>:<to>, open:<page>, ready,\n",
        "                                     go:<page>, back, forward, close, finish-close,\n",
        "                                     check, state, show; a cycle, a missing entry and a\n",
        "                                     closing state are each diagnosable\n",
        "       --machine <op>...             A state machine: state:<name>, initial:<name>, final:<name>,\n",
        "                                     transition:<from>:<on>:<to>[:<guard>],\n",
        "                                     answer:<guard>:<true|false>:<sync|async>, start,\n",
        "                                     send:<event>[:<sync|async>], inspect, show; a synchronous\n",
        "                                     send refuses to consult the async seam\n",
        "       --adapter <op>...             File/network adapters and retry: sample:<outcome>,\n",
        "                                     live:<target>:<outcome>, request:<kind>:<target>, cancel,\n",
        "                                     online, offline, attempt:<n>, run, check, state, show;\n",
        "                                     an outcome is ok:/failed:/unknown:/cancelled, an unknown\n",
        "                                     result is never retried and offline design needs no service\n",
        "       --session-log <op>...         Joint acceptance: human:<label>:<before>:<after>,\n",
        "                                     business:<label>:<before>:<after>, undo,\n",
        "                                     loop:<identity|drifting>:<value>, state:<name>,\n",
        "                                     initial:<name>, final:<name>, transition:<from>:<on>:<to>,\n",
        "                                     expect:<event>:<state>, vector, check, show; a human edit\n",
        "                                     is undoable and a business side effect is not\n",
        "       --preview <op>...             The preview service's life: start:<session>, ready,\n",
        "                                     spawn:<pid>, install:<node>, rebuild:<node>, stop,\n",
        "                                     finish-stop, clean, state, show; stopping leaves no\n",
        "                                     dangling instance or process\n",
        "       --hot-update <op>...          Live update or rebuild: self:<source>, from:<source>,\n",
        "                                     property:<subject>, style:<subject>, existing:<subject>,\n",
        "                                     add-binding:<subject>, remove-binding:<subject>,\n",
        "                                     add-node:<subject>, remove-node:<subject>, text:<subject>,\n",
        "                                     flush, check, show; a UI or existing-binding edit does\n",
        "                                     not start cargo\n",
        "       --state-keep <op>...          Preserve the live state across a rebuild: keep:<node>:<kind>,\n",
        "                                     focus:<node>, scroll:<node>:<n>, input:<node>:<text>,\n",
        "                                     selection:<node>:<start>:<end>, present:<node>:<kind>,\n",
        "                                     restore, check, show; state is restored only onto a\n",
        "                                     node that is the same id and kind\n",
        "       --matrix <op>...               The viewport matrix: document:<name>,\n",
        "                                     viewport:<name>:<w>x<h>:<orientation>, theme:<name>:<t>,\n",
        "                                     language:<name>:<lang>, sample:<name>:<s>, commit,\n",
        "                                     start:<name>, stop:<name>, restart:<name>, remove:<name>,\n",
        "                                     check, show; viewports share the document and their\n",
        "                                     running sessions are isolated\n",
        "       --inspect <op>...              What an inspection shows: mode:<m>, adapter:<a>,\n",
        "                                     edit:<version>, instance:<name>:<version>, stop,\n",
        "                                     kind:<node>:<kind>, design:<node>:<property>:<text>,\n",
        "                                     computed:<node>:<property>:<text>,\n",
        "                                     runtime:<node>:<property>:<text>:<version>,\n",
        "                                     source:<node>:<file>:<line>:<column>, child:<node>:<parent>,\n",
        "                                     event:<name>:<payload>, data:<field>:<value>,\n",
        "                                     values:<node>:<property>, locate:<node>, trigger:<action>,\n",
        "                                     check, show; a read-only inspection does not run\n",
        "                                     business logic and the running layer needs a real adapter\n",
        "       --visual <op>...               Compare a preview screenshot with its recorded baseline:\n",
        "                                     platform:<p>, dpi:<n>, resources:<n>, fonts:<name>,\n",
        "                                     tolerance:<percent>:<channel-step>, record:<name>:<w>x<h>:<hex>,\n",
        "                                     shot:<name>:<w>x<h>:<hex>, drop:<name>, baseline, check;\n",
        "                                     a difference states the platform, dpi and resource version\n",
        "                                     and nothing is judged from pixels alone\n",
        "       --preview-recovery <op>...     Failure, crash and recovery of the preview app:\n",
        "                                     edit:<version>, launch, reload, rebuild, reset,\n",
        "                                     exit:<code>, crash:<signal>, fault:<reason>, clear,\n",
        "                                     check; a recovery states the edit and running versions\n",
        "                                     and does not pretend to have succeeded\n",
        "       --load <op>...                 Bind and rebuild many times and check what must not grow:\n",
        "                                     budget:<ms>, cycle:<viewport>:<subscriptions>:<nodes>[:<latency>],\n",
        "                                     repeat:<viewport>:<cycles>:<subscriptions>:<nodes>[:<latency>],\n",
        "                                     leak:<viewport>:<cycles>:<subscriptions>:<nodes>,\n",
        "                                     invalid:<text>, check, verdict; subscriptions and memory\n",
        "                                     do not keep growing and invalid input is kept\n",
        "       --focus <target>              Route a keypress, or show a context menu and the toolbar;\n",
        "                                     every entry point reads one state\n",
        "       --press <chord>               With --focus, the chord to press\n",
        "       --menu <title>                With --focus, show a menu as a context menu\n",
        "       --mode <mode>                 With --focus, the mode the designer is in\n",
        "       --dependencies <directory>    Resolve a Cargo workspace's dependencies and report the tree;\n",
        "                                     nothing runs until the directory is trusted and each effect of\n",
        "                                     the resolution has been authorized\n",
        "       --review <record>             With --dependencies, compare the resolution with a record and\n",
        "                                     name what was added, removed, re-versioned or re-featured\n",
        "       --allow-lockfile-change       With --dependencies, let the resolution write Cargo.lock\n",
        "                                     (offline: cargo resolves from what this machine has)\n",
        "       --allow-network               With --dependencies, let the resolution reach the registry;\n",
        "                                     without it the resolution is offline\n",
        "       --new-workspace <directory>   Create a workspace document in a directory, with no members\n",
        "                                     yet; --name <name> (default: the directory)\n",
        "       --workspace <file>            Show a workspace: its members, what each holds, and its files\n",
        "                                     qualified by the member that names them\n",
        "       --add <name>=<root>           With --workspace, name a root that is already there as a\n",
        "                                     member; without --apply this only plans it\n",
        "       --remove <name>               With --workspace, take a member out of the document; its\n",
        "                                     directory and everything in it stay where they are\n",
        "       --activate <name>             With --workspace, record which member is active on this\n",
        "                                     machine, which the document does not say\n",
        "       --file <member>:<path>        With --workspace, show that one file: two members can each\n",
        "                                     hold a path of the same name\n",
        "       --artifact <op>...             Plan a project's generated files: entry:<source>=<output>,\n",
        "                                     output:<source>, show; one source generates one file\n",
        "                                     and the commit policy is stated\n",
        "       --generate <op>...             Run the generation pipeline: document:<file>,\n",
        "                                     node:<kind>:<children>, target:<default|stripped>,\n",
        "                                     size:<w>x<h>, fn:<name>, style:<node>:<reason>,\n",
        "                                     layout:<node>:<reason>, check, run, twice, show; a blocked\n",
        "                                     precheck emits nothing and the same input is byte-stable\n",
        "       --mapping <op>...              Build a node-to-source map: source:<file>, file:<path>,\n",
        "                                     key:<node>:<key>, model:<n>, generator:<a>.<b>.<c>,\n",
        "                                     locate:<node>, at:<byte>, staleness:<n>, show\n",
        "       --gen-merge <op>...            Merge a regenerated body into a committed file: old:<file>,\n",
        "                                     new:<file>, show; hand-written regions are kept\n",
        "       --reverse <op>...              Read a generated file back: base:<file>, edited:<file>,\n",
        "                                     mine:<file>, theirs:<file>, support, show; an\n",
        "                                     unrecognised change keeps its text and can be cancelled\n",
        "       --position <op>...             Recalculate positions and guard a loop: text:<file>,\n",
        "                                     at:<byte>, line:<l>:<c>, edit:<start>:<end>:<inserted>,\n",
        "                                     apply:<byte>, guard:<limit>, enter, leave, undo, redo, show\n",
        "       --export <op>...               Commit generated files: root:<directory>,\n",
        "                                     artifact:<path>=<file>, commit, contract, show; writes are\n",
        "                                     staged so a failure keeps the old files\n",
        "       --golden <op>...               Compare generation with recorded goldens: golden:<name>=<file>,\n",
        "                                     produced:<name>=<file>, semantics:<file>, check, show\n",
        "       --toolchain <op>...            Probe a toolchain and choose a feature set: profile:<name>,\n",
        "                                     feature:<name>, target:<triple>, cargo:<version>,\n",
        "                                     rustc:<version>, installed:<triple>, sdk:<name>, check, show;\n",
        "                                     a missing environment and an unsupported capability are told apart\n",
        "       --compile <op>...              Check, build or rebuild: action:<check|build|rebuild>,\n",
        "                                     profile:<name>, feature:<name>, target:<triple>,\n",
        "                                     package:<name>, manifest:<path>, exit:<code>, cancel, show\n",
        "       --build-diag <op>...           Read a build's problems: output:<file>, model:<n>,\n",
        "                                     source:<file>, key:<node>:<key>,\n",
        "                                     span:<node>:<file>:<start>:<end>, current:<n>, show\n",
        "       --run <op>...                  Run the delivered program: program:<path>, arg:<value>,\n",
        "                                     env:<name>=<value>, directory:<path>, standalone:<yes|no>,\n",
        "                                     start, exit:<code>, signal:<n>, stop, restart, show\n",
        "       --tasks <op>...                Order configured tasks: task:<name>:<command>,\n",
        "                                     after:<name>:<dependency>, input:<name>:<path>,\n",
        "                                     fingerprint:<path>:<value>, output:<name>:<path>,\n",
        "                                     default:<name>, order, key:<name>, cleanup:<name>, show\n",
        "       --terminal <op>...            Drive a terminal session: session:<name>,\n",
        "                                     directory:<path>, env:<name>=<value>, size:<c>x<r>,\n",
        "                                     platform:<macos|linux|windows>, authorize, start,\n",
        "                                     write:<text>, copy:<start>:<end>, resize:<c>x<r>, show\n",
        "       --external <op>...             Configure and invoke an external tool: tool:<name>,\n",
        "                                     program:<path>, arg:<value>, input:<path>, output:<path>,\n",
        "                                     confirm:<yes|no>, placeholder:<name>=<value>, invoke,\n",
        "                                     confirmed, show\n",
        "       --e2e <op>...                  Record the generate/check/build/run/feedback walk:\n",
        "                                     generate:<ok|fail>:<note>, check:…, build:…, run:…,\n",
        "                                     feedback:…, skip, show\n",
        "       --dag <op>...                  Build a typed task graph: node:<name>, after:<name>:<dep>,\n",
        "                                     together:<name>:<dep>, background:<name>:<marker>,\n",
        "                                     terminal:<name>:<terminal>, default:<name>,\n",
        "                                     pattern:<owner>:<pattern>:<f>:<l>:<m>, match:<line>,\n",
        "                                     waves, ready:<name>:<text>, show\n",
        "       --reproducible <op>...         State a reproducible, offline build: locked:<yes|no>,\n",
        "                                     offline:<yes|no>, vendor:<path>, cache:<path>, source:<path>, show\n",
        "       --headless <op>...             Run the headless walk and export its CI: validate:<ok|fail>,\n",
        "                                     generate:<ok|fail>, check:<ok|fail>, profile:<name>,\n",
        "                                     workflow, json, show\n",
        "       --component <op>...            Define components and instances: define:<name>,\n",
        "                                     version:<n>, param:<name>[=<default>], slot:<name>,\n",
        "                                     control:<kind>, nested:<definition>, useslot:<name>,\n",
        "                                     commit, instance:<id>, of:<definition>, arg:<name>=<value>,\n",
        "                                     place, expand:<id>, show\n",
        "       --component-author <op>...     Extract/insert/edit a definition: kind:<kind>,\n",
        "                                     property:<name>=<value>, extract:<name>, use:<definition>,\n",
        "                                     insert:<id>, override:<id>:<name>=<value>, edit:<name>,\n",
        "                                     change:<name>=<value>, commit, resolve:<id>, show\n",
        "       --component-override <op>...   Detach/reset a variant: variant:<id>, of:<definition>,\n",
        "                                     kind:<kind>, target:<name>, override:<name>=<value>,\n",
        "                                     base:<name>=<value>, detach, reset, check:<target>, show\n",
        "       --custom-widget <op>...        Register a custom widget: widget:<name>,\n",
        "                                     field:<name>=<type>, event:<name>=<type>, impl:<target>,\n",
        "                                     register, bind:<name>=<value>, publish:<target>, validate, show\n",
        "       --resource-import <op>...      Import/replace/move an asset: asset:<id>:<path>:<kind>:<cap>:<bytes>,\n",
        "                                     refer:<path>:<node>, replace:<id>:<path>:<bytes>,\n",
        "                                     move:<id>:<path>, repoint:<from>:<to>, missing, show\n",
        "       --resource-target <op>...      Check per-target variants: resource:<name>:<kind>,\n",
        "                                     variant:<target>:<encoding>:<bytes>, check:<target>,\n",
        "                                     block:<target>, show\n",
        "       --asset-package <op>...        Plan a package import: package:<name>:<version>:<license>,\n",
        "                                     depends:<package>, entry:<path>:<version>, existing:<path>,\n",
        "                                     dependency:<name>:<dependency>, plan, commit, cancel, show\n",
        "       --component-upgrade <op>...    Upgrade a definition: definition:<name>:<version>,\n",
        "                                     property:<name>=<value>, instance:<id>:<definition>,\n",
        "                                     override:<id>:<name>=<value>, asset:<path>, change:<name>=<value>,\n",
        "                                     upgrade:<name>, delete-asset:<path>, roundtrip, static, show\n",
        "       --asset-manifest <op>...       State the asset manifest: asset:<id>:<path>:<packaging>,\n",
        "                                     derive:<id>:<source>:<transform>:<path>:<packaging>,\n",
        "                                     move:<id>:<path>, resolve:<id>, missing:<id>, static, show\n",
        "       --app-shell <op>...            Assemble the app shell: target:<name>, title:<text>,\n",
        "                                     size:<w>x<h>, region:<menu|toolbar|status|dock>,\n",
        "                                     provide:<menu|dock>, gap:<menu|dock>:<reason>, show\n",
        "       --app-action <op>...           Define shared actions: action:<id>, title:<text>,\n",
        "                                     icon:<name>, shortcut:<keys>, enabled:<yes|no>, checked:<yes|no>,\n",
        "                                     group:<name>, target:<kind>:<value>, reference:<kind>:<id>,\n",
        "                                     claim:<kind>:<id>, select:<id>, show\n",
        "       --app-menu <op>...             Edit the app's menus: root:<id>:<title>, define:<id>,\n",
        "                                     item:<menu>:<label>:<action>, separator:<menu>,\n",
        "                                     role:<menu>:<role>, submenu:<parent>:<child>:<label>,\n",
        "                                     platform:<macos|windows|linux>, show\n",
        "       --app-toolbar <op>...          Edit the app's tool/status bars: item:<id>, label:<text>,\n",
        "                                     present:<icon|text|both>, visible:<yes|no>,\n",
        "                                     overflow:<collapse|wrap>, capacity:<n>, status:<label>:<value>, show\n",
        "       --app-context <op>...          Edit the app's context menus: kind:<item|canvas|tree-node>,\n",
        "                                     item:<kind>:<action>, enable:<action>:<yes|no>,\n",
        "                                     param:<key>:<value>, click:<kind>, invoke:<action>, show\n",
        "       --app-dialog <op>...           Model the app's dialogs: main:<id>,\n",
        "                                     dialog:<id>:<owner>:<kind>:<modality>, default:<id>:<action>,\n",
        "                                     cancel:<id>:<action>, field:<name>:<value>, open:<id>,\n",
        "                                     close:<id>, accept:<id>, show\n",
        "       --app-dock <op>...             State the target's dock support: target:<name>,\n",
        "                                     layout:<single|split|docked>, panel:<name>,\n",
        "                                     orientation:<horizontal|vertical>, persist:<yes|no>, show\n",
        "       --app-regression <op>...       Walk the app shell: mode:<runtime|static>,\n",
        "                                     bind:<kind>:<action>, preview, generate,\n",
        "                                     activate:<kind>:<action>, show\n",
        "       --collection <op>...           Edit collection content: control:<n>, origin:<sample|source>,\n",
        "                                     column:<role>:<header>, row, set:<row>:<column>:<value>,\n",
        "                                     move-row:<from>:<to>, move-column:<from>:<to>, business, show\n",
        "       --table <op>...                Edit table columns: column:<name>:<type>, width:<name>:<n>,\n",
        "                                     sort:<name>:<asc|desc|none>, filter:<name>:<text>,\n",
        "                                     select:<index>, empty:<message>, source:<key>,\n",
        "                                     field:<name>:<raw>, map, show\n",
        "       --tree <op>...                 Edit tree items: node:<id>:<parent|->:<label>,\n",
        "                                     icon:<id>:<role>, expand:<id>, collapse:<id>, select:<id>,\n",
        "                                     reparent:<id>:<parent|->, lazy:<id>,\n",
        "                                     answer:<ok|fail|cancel|unknown>:<text>, load:<id>:<target>, show\n",
        "       --cell-edit <op>...            Edit a cell: row:<n>, column:<name>, current:<value>,\n",
        "                                     candidate:<value>, delegate:<text|number|check|choice>,\n",
        "                                     commit, cancel, show\n",
        "       --container <op>...            Manage container pages: kind:<tabs|stack|scroll|split>,\n",
        "                                     page:<id>:<title>:<node>, activate:<id>, remove:<id>,\n",
        "                                     reorder:<id>:<to>, next, previous, show\n",
        "       --rich-content <op>...         Edit rich content: font:<family>:<size>:<weight>,\n",
        "                                     run:<format>:<text>, link:<target>:<alt>, image:<asset>:<alt>,\n",
        "                                     limit-length:<n>, limit-format:<format>, trust,\n",
        "                                     network:<yes|no>, authorize, execute-link, show\n",
        "       --view <op>...                 Describe a specialised view: kind:<gauge|bar-chart|...>,\n",
        "                                     data:<text>, capability, editor, show\n",
        "       --collection-regression <op>... Run the collection walk: total:<n>, viewport:<height>,\n",
        "                                     first:<n>, mode:<static|live>, locale:<tag>, empty:<message>,\n",
        "                                     error:<reason>, select:<index>, edit:<index>:<value>,\n",
        "                                     key:<command>, check, show\n",
        "       --form <op>...                 Validate a form: field:<name>:<kind>, required:<name>,\n",
        "                                     same-as:<name>:<other>, different-from:<name>:<other>,\n",
        "                                     required-when:<name>:<other>=<value>, min-length:<name>:<n>,\n",
        "                                     at-least:<name>:<n>, one-of:<name>:<a>|<b>,\n",
        "                                     value:<name>=<text>, revision:<n>, pending:<field>:<revision>,\n",
        "                                     mode:<preview|runtime|static>, validate, submit, reset, show\n",
        "       --logic-core <op>...           Reduce events against the shared core: init:<state>,\n",
        "                                     transition:<state>:<event>:<next>,\n",
        "                                     guard:<state>:<event>:<field>=<type>:<value>,\n",
        "                                     set:<state>:<event>:<field>=<type>:<value>,\n",
        "                                     need:<state>:<event>:<function>:<into>=<type>:<value>,\n",
        "                                     seam:<function>=<type>:<value>, event:<name>=<type>:<value>, run, show\n",
        "       --logic-ipc <op>...            Open a typed local channel: session:<id>, version:<n>,\n",
        "                                     generation:<n>, manifest:<name>:<version>, capability:<name>,\n",
        "                                     frame:<kind>:<generation>:<payload>, receive:<encoded>, open, show\n",
        "       --logic-actions <op>...        Validate and queue actions: capacity:<n>,\n",
        "                                     item:<event>:<class>:<needs|ui>[:<deadline>], take,\n",
        "                                     tick:<now>, check, show\n",
        "       --logic-candidate <op>...      Debounce edits and admit a candidate: revision:<n>,\n",
        "                                     window:<ms>, edit:<source>, flush:<now>,\n",
        "                                     action:<check|build|rebuild>, outcome:<success|failure>,\n",
        "                                     stamp:<revision>, artifact:<path>, running:<path>, admit, show\n",
        "       --logic-migration <op>...      Plan a state migration: from:<n>, to:<n>,\n",
        "                                     from-field:<name>:<type>:<since>, to-field:<name>:<type>:<since>,\n",
        "                                     value:<name>=<type>:<value>, owner:<name>, task:<name>,\n",
        "                                     plan, reset, confirm, check, show\n",
        "       --logic-swap <op>...           Walk a controlled replacement: old:<name>, new:<name>,\n",
        "                                     limit:<ms>, pause:<elapsed>, drain, snapshot:<version>,\n",
        "                                     import:<yes|no>, confirm, resume, retire, fail:<reason>, show\n",
        "       --logic-combination <op>...    Judge readiness and isolate a late result: building:<yes|no>,\n",
        "                                     protocol:<yes|no>, plan:<style|document|binding|logic|protocol>,\n",
        "                                     interaction:<name>, needs:<change>, generation:<n>,\n",
        "                                     result:<generation>:<value>, check, show\n",
        "       --logic-regression <op>...     Run the event vectors: init:<state>,\n",
        "                                     transition:<state>:<event>:<next>, vector:<name>, say:<event>,\n",
        "                                     fault:unknown:<call>, fault:timeout:<step>,\n",
        "                                     fault:crash:<generation>, fault:rapid:<count>, run, decide, check, show\n",
        "     Global options:\n",
        "       --log-level <level>           error | warn | info | debug | trace (default: info)\n",
        "       --ui-language <tag>           Interface language; only `en` is bundled so far (default: follow\n",
        "                                     the system)\n",
    )
}

/// Parses the arguments into a command.
///
/// The `--log-level` option is removed here so every family below can assume it is looking at the
/// command; [`log_level_of`] reads it separately, and both agree because they share this token
/// handling.
pub fn parse<I, S>(arguments: I) -> Result<Command, ParseError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let collected: Vec<OsString> = arguments
        .into_iter()
        .map(|value| value.as_ref().to_os_string())
        .collect();
    let mut stripped: Vec<OsString> = Vec::with_capacity(collected.len());
    let mut index = 0;
    while index < collected.len() {
        if collected[index] == OsStr::new("--log-level") {
            let Some(value) = collected.get(index + 1) else {
                return Err(ParseError {
                    argument: collected[index].clone(),
                });
            };
            if value.to_str().and_then(Level::parse).is_none() {
                return Err(ParseError {
                    argument: value.clone(),
                });
            }
            index += 2;
            continue;
        }
        if collected[index] == OsStr::new("--ui-language") {
            let Some(value) = collected.get(index + 1) else {
                return Err(ParseError {
                    argument: collected[index].clone(),
                });
            };
            if value.to_str().and_then(Language::parse).is_none() {
                return Err(ParseError {
                    argument: value.clone(),
                });
            }
            index += 2;
            continue;
        }
        stripped.push(collected[index].clone());
        index += 1;
    }

    let mut arguments = stripped.into_iter();
    let Some(argument) = arguments.next() else {
        return Ok(Command::Status);
    };
    let text = argument.to_str().unwrap_or_default();

    // Each family knows its own flags and answers `None` for anything else, so a flag belongs to
    // exactly one family and the order below is a reading order rather than a precedence rule.
    let families: [Family; 20] = [
        host::parse_host,
        project::parse_project,
        document::parse_document,
        shell::parse_shell,
        commands::parse_commands,
        editor::parse_editor,
        language::parse_language,
        search::parse_search,
        canvas::parse_canvas,
        layout::parse_layout,
        style::parse_style,
        data::parse_data,
        preview::parse_preview,
        generate::parse_generate,
        build::parse_build,
        component::parse_component,
        application::parse_application,
        collection::parse_collection,
        form::parse_form,
        logic::parse_logic,
    ];
    for family in families {
        if let Some(command) = family(text, argument.clone(), &mut arguments) {
            return command;
        }
    }

    // Only the two global flags are left, and neither takes an option.
    let command = match text {
        "--help" | "-h" => Command::Help,
        "--version" | "-V" => Command::Version,
        _ => return Err(ParseError { argument }),
    };
    if let Some(argument) = arguments.next() {
        return Err(ParseError { argument });
    }
    Ok(command)
}

/// One family's flag parser: the flag, its own token, and the arguments still to read.
type Family =
    fn(&str, OsString, &mut std::vec::IntoIter<OsString>) -> Option<Result<Command, ParseError>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_arguments_show_status() {
        assert_eq!(parse([] as [&str; 0]), Ok(Command::Status));
    }

    #[test]
    fn parses_help_and_version_options() {
        assert_eq!(parse(["--help"]), Ok(Command::Help));
        assert_eq!(parse(["-h"]), Ok(Command::Help));
        assert_eq!(parse(["--version"]), Ok(Command::Version));
        assert_eq!(parse(["-V"]), Ok(Command::Version));
    }

    #[test]
    fn the_usage_block_keeps_the_alignment_it_is_written_with() {
        let text = help_text();

        assert!(
            text.contains("\n            rustage [--render-icons"),
            "a usage continuation line has to stay under the command name:\n{text}"
        );
        assert!(
            text.contains("\n                                 [--move <from> <to>]"),
            "an option's continuation has to line up under the option:\n{text}"
        );
        assert!(
            text.contains("\n       --log-level <level>"),
            "the option column has to be indented:\n{text}"
        );
        assert!(
            !text.contains("\nrustage [--render-icons"),
            "the flattening this replaces put every line at the left margin:\n{text}"
        );
    }

    #[test]
    fn rejects_unknown_and_extra_arguments() {
        assert!(parse(["--help", "--version"]).is_err());
        // A missing or repeated directory is a usage error rather than an empty path: exporting
        // into "" would write into the process's working directory.
        assert!(parse(["--render-icons"]).is_err());
        assert!(parse(["--render-icons", "a", "b"]).is_err());
        assert!(parse(["--probe-apis"]).is_err());
        assert!(parse(["--probe-apis", "a", "b"]).is_err());
        assert!(parse(["--probe-host"]).is_err());
        assert!(parse(["--probe-host", "a", "b"]).is_err());
    }
}
