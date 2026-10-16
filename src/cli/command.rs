// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The argument parser and the usage text.
//!
//! The surface is deliberately tiny. The scaffold answers `--help`, `--version` and reports its
//! own status; the other two options exist because the host needs reproducible producers for
//! artifacts it commits or inspects: `--render-icons` renders the icon strip in
//! [`crate::app::toolbar`] into a directory (the committed baselines under `tests/baselines/icons/`
//! would otherwise be pictures of whatever the machine last rendered, which is exactly what a
//! baseline must not be), and `--probe-apis` runs the dependency contract probes in
//! [`crate::probe`] and writes their report.

use std::{
    ffi::{OsStr, OsString},
    fmt,
    path::PathBuf,
    time::Duration,
};

use crate::host::log::Level;
use crate::project::{
    closing::Decision,
    configuration::SettingValue,
    create::Template,
    files::FileOperation,
    save::Verb,
    workspace::{WorkspaceFile, name_is_addressable},
};

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
}

/// What to create a project from.
///
/// The wizard's steps are its arguments: where the project goes, what it is called, and what it
/// starts from. A window would collect the same three things, and the creation path below it is the
/// same one either way.
#[derive(Debug, PartialEq, Eq)]
pub struct NewProjectOptions {
    /// The directory the project is created in.
    pub directory: PathBuf,
    /// The name a person sees; the directory's own name when nothing is given.
    pub name: Option<String>,
    /// What the project starts from.
    pub template: Template,
}

impl NewProjectOptions {
    /// Options for a directory, starting from a template.
    pub fn new(directory: impl Into<PathBuf>, template: Template) -> Self {
        Self {
            directory: directory.into(),
            name: None,
            template,
        }
    }
}

/// How a project should be opened.
///
/// `trust` is the answer to the question the open prints, and `diagnose` asks for the toolchain
/// check, which is only possible once the project is trusted. Neither is implied by the other: a
/// person can open a project and read it without deciding anything.
#[derive(Debug, PartialEq, Eq)]
pub struct OpenProjectOptions {
    /// The project document to open.
    pub path: PathBuf,
    /// Whether the person answering the question said yes.
    pub trust: bool,
    /// Whether to read the toolchain, which runs two programs.
    pub diagnose: bool,
}

impl OpenProjectOptions {
    /// Options that read a project and decide nothing.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            trust: false,
            diagnose: false,
        }
    }
}

/// Which entry to remove, and how to treat what points at it.
#[derive(Debug, PartialEq, Eq)]
pub struct DropEntryOptions {
    /// The project document.
    pub path: PathBuf,
    /// The page document of the entry, as the project writes it.
    pub document: String,
    /// Whether the references to it may be fixed, instead of refusing the removal.
    pub fix: bool,
    /// Whether the person answering the project's question said yes.
    pub trust: bool,
}

impl DropEntryOptions {
    /// Options that remove an entry and refuse while anything points at it.
    pub fn new(path: impl Into<PathBuf>, document: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            document: document.into(),
            fix: false,
            trust: false,
        }
    }
}

/// Which existing project to import, and how far to go.
#[derive(Debug, PartialEq, Eq)]
pub struct ImportOptions {
    /// The project document, or the directory that holds one.
    pub path: PathBuf,
    /// Whether the migration should be carried out instead of only described.
    pub apply: bool,
    /// Whether to report the files a past migration kept, instead of planning a new one.
    pub check_backup: bool,
    /// Whether the person answering the project's question said yes.
    pub trust: bool,
}

impl ImportOptions {
    /// Options that describe an import and write nothing.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            apply: false,
            check_backup: false,
            trust: false,
        }
    }
}

/// How a kit should be diagnosed (task T02.10).
///
/// Showing a kit runs nothing. `diagnose` asks this machine about each member — which runs the
/// programs the kit names, so it needs the directory to be trusted and the permission the decision
/// table gives a command — and `triple` is the output the kit is asked about. Which profile is being
/// built is not part of the question: that belongs to the project's configuration, and a kit is a
/// combination of tools.
#[derive(Debug, PartialEq, Eq)]
pub struct KitOptions {
    /// The kit document.
    pub path: PathBuf,
    /// Whether to ask this machine about each member.
    pub diagnose: bool,
    /// The output triple to ask about, when there is one.
    pub triple: Option<String>,
    /// Whether the person answering the directory's question said yes.
    pub trust: bool,
}

impl KitOptions {
    /// Options that show a kit and ask nothing.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            diagnose: false,
            triple: None,
            trust: false,
        }
    }
}

/// One edit to type into a buffer (task T03.01).
///
/// Text is replaced whole rather than inserted a character at a time: that is what a paste or an
/// import does, and per-keystroke editing is the undo domain's subject (T03.03). The order the edits
/// are given in is the order they happen in, so a sequence of them is a story about one buffer.
#[derive(Debug, PartialEq, Eq)]
pub enum DocumentEdit {
    /// The text itself.
    Text(String),
    /// A file whose text is read and used.
    File(PathBuf),
}

/// What to do with a page document's buffer (task T03.01).
///
/// Showing a buffer runs nothing and writes nothing: it reads the file and reports what the text is,
/// what the last model was, and what happened to identity. Whether the newest text reads is a state
/// rather than a failure, so a text that does not read is still reported in full — with the refusal
/// and with the model that is still previewable.
#[derive(Debug, PartialEq, Eq)]
pub struct DocumentOptions {
    /// The page document.
    pub file: PathBuf,
    /// The project root, when the caller names it; otherwise the nearest project above the file.
    pub root: Option<PathBuf>,
    /// The edits to type, in order.
    pub edits: Vec<DocumentEdit>,
}

impl DocumentOptions {
    /// Options that show a document and type nothing into it.
    pub fn new(file: impl Into<PathBuf>) -> Self {
        Self {
            file: file.into(),
            root: None,
            edits: Vec::new(),
        }
    }
}

/// What to do with a page document's model (task T03.02).
///
/// A batch is planned before it is applied: the plan says what each operation would do, what cannot
/// land and why, and which capability the store will ask about — and a blocked batch is refused as a
/// whole. `--apply` applies it to the model *in memory* (writing the file is T03.04's subject), and
/// `--undo` additionally applies the inverse the plan reports, which is how "all committed or all
/// refused" and "an edit says what would undo it" are both visible in one run.
#[derive(Debug, PartialEq, Eq)]
pub struct EditOptions {
    /// The page document.
    pub file: PathBuf,
    /// The batch to run, when there is one.
    pub batch: Option<PathBuf>,
    /// The project root, when the caller names it; otherwise the nearest project above the file.
    pub root: Option<PathBuf>,
    /// Whether to apply the batch to the model.
    pub apply: bool,
    /// Whether to apply the batch's inverse afterwards.
    pub undo: bool,
}

impl EditOptions {
    /// Options that show a document's model and run no batch.
    pub fn new(file: impl Into<PathBuf>) -> Self {
        Self {
            file: file.into(),
            batch: None,
            root: None,
            apply: false,
            undo: false,
        }
    }
}

/// One thing that happens to a document during a session (task T03.03).
///
/// The order they are given in is the order they happen in, so a run is a story about one document:
/// text typed, a batch applied, then an undo, then more text. That is the whole point of a history —
/// what happens after an undo is a new branch, and a caller has to be able to say so.
#[derive(Debug, PartialEq, Eq)]
pub enum HistoryEdit {
    /// The text of the document, replaced whole.
    Text(String),
    /// A file whose text is read and used as the document's text.
    File(PathBuf),
    /// A batch of model edits, planned against the model and applied to it.
    Batch(PathBuf),
    /// Take back this many steps.
    Undo(usize),
    /// Put back this many steps.
    Redo(usize),
}

impl HistoryEdit {
    /// Whether this is a move rather than an edit.
    pub const fn is_a_move(&self) -> bool {
        matches!(self, Self::Undo(_) | Self::Redo(_))
    }
}

/// Which undo domain a move is asked for (task T03.03).
///
/// Asking for a domain is not the same as asking for the newest step: a request for the model while
/// somebody's typing is the newest step is refused rather than answered, because undoing the model
/// change *behind* the typing would leave the text as nobody typed it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryDomain {
    /// The text.
    Text,
    /// The model.
    Model,
}

impl HistoryDomain {
    /// The name this domain is asked about on the command line.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Model => "model",
        }
    }

    /// The domain a name asks for, when it is one.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "text" => Some(Self::Text),
            "model" => Some(Self::Model),
            _ => None,
        }
    }
}

/// What to do with a page document's history (task T03.03).
///
/// One document, one history: the entries are in the order things happened, whatever proposed them,
/// and every entry belongs to one of two domains. What happens is a script: `--text`, `--text-file`,
/// `--batch`, `--undo` and `--redo` are all steps of it, in the order they are given. `--only` says
/// which domain the moves in the script have to be, and `--merge` makes every batch in it one step —
/// the way one drag is one thing to take back.
#[derive(Debug, PartialEq, Eq)]
pub struct HistoryOptions {
    /// The page document.
    pub file: PathBuf,
    /// The project root, when the caller names it; otherwise the nearest project above the file.
    pub root: Option<PathBuf>,
    /// What happens, in order.
    pub edits: Vec<HistoryEdit>,
    /// Whether the batches in this run are one step rather than one step each.
    pub merge: bool,
    /// The domain the moves have to be in, when the caller names one.
    pub only: Option<HistoryDomain>,
}

impl HistoryOptions {
    /// Options that show a document's history and change nothing.
    pub fn new(file: impl Into<PathBuf>) -> Self {
        Self {
            file: file.into(),
            root: None,
            edits: Vec::new(),
            merge: false,
            only: None,
        }
    }
}

/// How to write one document, or a set of them, back to disk (task T03.04).
///
/// One file is a single atomic save; more than one is a set that is all committed or all left as it
/// was, which is what "save all" and "commit a set of resources together" are. The verb says where
/// the text goes: [`Verb::Save`] writes back over each file, and the other two write elsewhere (and
/// so are only meaningful for a single file). `--plan` prints what would happen without writing.
#[derive(Debug, PartialEq, Eq)]
pub struct SaveOptions {
    /// The page documents to write, in order.
    pub files: Vec<PathBuf>,
    /// The edits to type into the single document before saving it, in order.
    pub edits: Vec<DocumentEdit>,
    /// The project root, when the caller names it; otherwise the nearest project above each file.
    pub root: Option<PathBuf>,
    /// Where the text goes.
    pub verb: Verb,
    /// Whether to print the plan and write nothing.
    pub plan_only: bool,
}

impl SaveOptions {
    /// Options that save files in place.
    pub fn new(files: Vec<PathBuf>) -> Self {
        Self {
            files,
            edits: Vec::new(),
            root: None,
            verb: Verb::Save,
            plan_only: false,
        }
    }
}

/// How to decide what a document should do when the file changed outside the designer (task T03.05).
///
/// The three versions are the base (what the document read), `mine` (the text after any `--set-text`
/// edits) and `theirs` (what is on disk now). Showing the decision writes nothing; `--recreate`
/// writes the document's text when the file is gone, and `--accept` writes it when a merge landed.
/// A conflict is never resolved by writing: it is reported, because picking a side silently is the
/// data loss this task exists to prevent.
#[derive(Debug, PartialEq, Eq)]
pub struct ConflictOptions {
    /// The page document.
    pub file: PathBuf,
    /// The project root, when the caller names it; otherwise the nearest project above the file.
    pub root: Option<PathBuf>,
    /// The bytes the document last agreed with the file, when the caller supplies them.
    ///
    /// A one-shot command reads the file and decides in the same breath, so the file is never out of
    /// date with itself; `--base` is how a caller replays the sequence a real designer lives through
    /// (read, somebody else changes the file, decide). Absent, the base is the file as it was read.
    pub base: Option<PathBuf>,
    /// The edits to type before deciding, in order.
    pub edits: Vec<DocumentEdit>,
    /// Whether to write the document's text back when the file has gone.
    pub recreate: bool,
    /// Whether to write the merged text when a merge landed.
    pub accept: bool,
}

impl ConflictOptions {
    /// Options that decide and write nothing.
    pub fn new(file: impl Into<PathBuf>) -> Self {
        Self {
            file: file.into(),
            root: None,
            base: None,
            edits: Vec::new(),
            recreate: false,
            accept: false,
        }
    }
}

/// What to do with the recovery store (task T03.06).
///
/// The store is a directory outside any project. `--record` keeps a copy of a document's text from a
/// session; with no action the command lists a project's copies and how each stands next to the file
/// now (the startup-recovery report); `--restore` prints the newest copy so a person can take it, and
/// `--discard` is the "no" that removes the copies and never the document.
#[derive(Debug, PartialEq, Eq)]
pub struct RecoveryOptions {
    /// The directory the recovery store lives in.
    pub store: PathBuf,
    /// The project root the copies belong to, when the caller names it.
    pub root: Option<PathBuf>,
    /// A document whose text should be copied into the store.
    pub record: Option<PathBuf>,
    /// A document whose newest copy should be printed.
    pub restore: Option<PathBuf>,
    /// A document whose copies should be refused (removed).
    pub discard: Option<PathBuf>,
    /// The session the copy is filed under; a timestamp when not named.
    pub session: Option<String>,
    /// Whether to plan a retention prune of the store.
    pub prune: bool,
    /// Whether to carry the prune out.
    pub apply: bool,
    /// How many copies to keep per document when pruning (default five).
    pub keep: Option<usize>,
}

impl RecoveryOptions {
    /// Options that list a store's copies and change nothing.
    pub fn new(store: impl Into<PathBuf>) -> Self {
        Self {
            store: store.into(),
            root: None,
            record: None,
            restore: None,
            discard: None,
            session: None,
            prune: false,
            apply: false,
            keep: None,
        }
    }
}

/// How to close a set of open documents (task T03.07).
///
/// `--close` names documents that are as they are on disk; `--unsaved <document>=<buffer-file>` adds
/// one whose buffer differs from its file, which is what makes the close need a decision. `--task`
/// names the work that belongs to the closing, so "work stops only when the close happens" is
/// observable. No `--decision` prints the plan and closes nothing; a decision carries it out.
#[derive(Debug, PartialEq, Eq)]
pub struct CloseOptions {
    /// Documents that have no unsaved changes.
    pub files: Vec<PathBuf>,
    /// The project root, when the caller names it; otherwise the nearest project above the first file.
    pub root: Option<PathBuf>,
    /// Documents with unsaved changes: their project-relative path and a file holding their buffer.
    pub unsaved: Vec<(String, PathBuf)>,
    /// The tasks that belong to the closing.
    pub tasks: Vec<String>,
    /// What the person chose; `None` prints the plan and closes nothing.
    pub decision: Option<Decision>,
}

impl CloseOptions {
    /// Options that plan a close and decide nothing.
    pub fn new(files: Vec<PathBuf>) -> Self {
        Self {
            files,
            root: None,
            unsaved: Vec::new(),
            tasks: Vec::new(),
            decision: None,
        }
    }
}

/// How to resolve a command through the registry (task T05.01).
///
/// The id and arguments are what a menu item or a shortcut would produce; the rest describes the
/// context the call happens in — where the keyboard is, whether the project is trusted, what effects
/// are authorized and what tasks are already running — so a disabled command can say which of those
/// it was, and an unauthorized one is a different answer from a disabled one.
#[derive(Debug, PartialEq, Eq)]
pub struct CommandOptions {
    /// The command to resolve; absent lists every registered command.
    pub id: Option<String>,
    /// The raw arguments, as a menu or a command line would supply them.
    pub arguments: Vec<(String, String)>,
    /// Where the keyboard is.
    pub focus: String,
    /// Whether the project is trusted.
    pub trust: bool,
    /// The effects that are authorized (names of [`EffectClass`](crate::contracts::permission::EffectClass)).
    pub authorize: Vec<String>,
    /// The tasks already running.
    pub running: Vec<String>,
}

impl CommandOptions {
    /// Options that list every command.
    pub fn listing() -> Self {
        Self {
            id: None,
            arguments: Vec::new(),
            focus: "design".to_owned(),
            trust: false,
            authorize: Vec::new(),
            running: Vec::new(),
        }
    }
}

/// How to resolve a key chord through the keymap (task T05.03).
///
/// The scene is what decides whether a bare key is a character or a command: with `--text-field` a
/// bare character goes to the field, and with `--composing` the keys belong to the input method.
#[derive(Debug, PartialEq, Eq)]
pub struct KeysOptions {
    /// The chord to press, as `Ctrl+K Ctrl+S`; absent lists the bindings.
    pub press: Option<String>,
    /// The mode the designer is in.
    pub mode: String,
    /// The focused panel, when one is.
    pub panel: Option<String>,
    /// Whether a text field has the keyboard.
    pub text_field: bool,
    /// Whether an input method is composing.
    pub composing: bool,
}

impl KeysOptions {
    /// Options that list the bindings.
    pub fn listing() -> Self {
        Self {
            press: None,
            mode: "design".to_owned(),
            panel: None,
            text_field: false,
            composing: false,
        }
    }
}

/// How to search the command palette (task T05.04).
#[derive(Debug, PartialEq, Eq)]
pub struct PaletteOptions {
    /// The query; an empty query lists every command.
    pub query: String,
    /// A command to show the help and parameters for, instead of searching.
    pub help_for: Option<String>,
    /// Where the keyboard is.
    pub focus: String,
    /// Whether the project is trusted.
    pub trust: bool,
    /// The authorized effects.
    pub authorize: Vec<String>,
    /// The tasks already running.
    pub running: Vec<String>,
    /// The recently run commands, most recent first.
    pub recent: Vec<String>,
}

impl PaletteOptions {
    /// Options that list every command.
    pub fn listing() -> Self {
        Self {
            query: String::new(),
            help_for: None,
            focus: "design".to_owned(),
            trust: false,
            authorize: Vec::new(),
            running: Vec::new(),
            recent: Vec::new(),
        }
    }
}

/// How to show the workbench and its panels (tasks T04.01/T04.02).
#[derive(Debug, PartialEq, Eq, Default)]
pub struct WorkbenchOptions {
    /// The mode to switch to.
    pub mode: Option<String>,
    /// The active project's name.
    pub project: Option<String>,
    /// The target the project would build for.
    pub target: Option<String>,
    /// The active document.
    pub document: Option<String>,
    /// Whether the document has unsaved changes.
    pub dirty: bool,
    /// A one-line status message.
    pub message: Option<String>,
    /// The services this host has, which decide which panels can be shown.
    pub services: Vec<String>,
    /// Panels to show.
    pub show: Vec<String>,
    /// Panels to hide.
    pub hide: Vec<String>,
    /// A panel to give the keyboard.
    pub focus: Option<String>,
    /// Panels to resize, as `(id, size)`.
    pub resize: Vec<(String, u32)>,
}

impl WorkbenchOptions {
    /// Options that show the workbench as it stands.
    pub fn new() -> Self {
        Self::default()
    }
}

/// How to preview and carry out a batch operation (task T05.07).
///
/// The selection is given as the items the operation applies to (`--select`) and the ones it does not
/// (`--inapplicable id=reason`), so the preview a person sees is the classification itself. No
/// `--choice` prints the preview and changes nothing.
#[derive(Debug, PartialEq, Eq)]
pub struct OperationOptions {
    /// The operation's name.
    pub name: String,
    /// A note for the person.
    pub note: Option<String>,
    /// The external effects the operation has.
    pub effects: Vec<String>,
    /// The selected items the operation applies to.
    pub select: Vec<String>,
    /// The selected items it does not apply to, as `(id, reason)`.
    pub inapplicable: Vec<(String, String)>,
    /// The authorized effects.
    pub authorize: Vec<String>,
    /// What to do: `apply-all`, `apply-none` or `cancel`.
    pub choice: Option<String>,
}

impl OperationOptions {
    /// Options for an operation, changing nothing.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            note: None,
            effects: Vec::new(),
            select: Vec::new(),
            inapplicable: Vec::new(),
            authorize: Vec::new(),
            choice: None,
        }
    }
}

/// A script of document-tab operations (task T04.03).
///
/// Each operation is `verb` or `verb:argument` (`move:from:to` takes two), in the order they happen,
/// so a run is a story about one tab strip: a document previewed, another pinned, one view closed and
/// the document itself closed.
#[derive(Debug, PartialEq, Eq)]
pub struct TabsOptions {
    /// How many tabs the strip shows before overflowing.
    pub limit: usize,
    /// The operations, in order.
    pub ops: Vec<String>,
}

impl TabsOptions {
    /// Options for an empty strip.
    pub fn new() -> Self {
        Self {
            limit: 8,
            ops: Vec::new(),
        }
    }
}

/// How to replay a script of editor-group operations (task T04.04).
///
/// Each operation is `verb` or `verb:argument` (`move:from:index:to:index` takes four, `select` takes
/// two), in the order they happen, so a run is a story about one editor area: two documents opened,
/// the area split, one view moved across, and the same buffer typed into from either side.
#[derive(Debug, PartialEq, Eq)]
pub struct GroupsOptions {
    /// How many tabs each group's strip shows before overflowing.
    pub limit: usize,
    /// The operations, in order.
    pub ops: Vec<String>,
}

impl GroupsOptions {
    /// Options for an editor area with one empty group.
    pub fn new() -> Self {
        Self {
            limit: 8,
            ops: Vec::new(),
        }
    }
}

/// What to copy, round-trip or paste on the clipboard (task T05.06).
///
/// One payload at a time, because the clipboard holds one thing: copy text, a resource reference or
/// a node subtree, optionally round-trip it, and optionally paste it into a project.
#[derive(Debug, PartialEq, Eq)]
pub struct ClipboardOptions {
    /// The payload size ceiling, in bytes.
    pub limit: usize,
    /// Copy this text.
    pub text: Option<String>,
    /// With a resource copy, the project it came from.
    pub resource_project: Option<String>,
    /// With a resource copy, the project-relative path.
    pub resource_path: Option<String>,
    /// With a node copy, the page document the subtree is read from.
    pub node_file: Option<PathBuf>,
    /// With a node copy, the id of the subtree's root.
    pub node_id: Option<u64>,
    /// Paste the payload into this project.
    pub paste_into: Option<String>,
    /// Encode the payload and decode it again, reporting whether the two agree.
    pub roundtrip: bool,
}

impl ClipboardOptions {
    /// Options carrying nothing yet.
    pub fn new() -> Self {
        Self {
            limit: 1 << 20,
            text: None,
            resource_project: None,
            resource_path: None,
            node_file: None,
            node_id: None,
            paste_into: None,
            roundtrip: false,
        }
    }
}

/// How to replay a script of layout operations (task T04.05).
///
/// Each operation is `verb` or `verb:argument`, in the order they happen, so a run is a story about
/// one editor area: a document opened, the area split, the layout saved, a preset applied, the
/// layout locked, and a bad configuration refused.
#[derive(Debug, PartialEq, Eq)]
pub struct LayoutOptions {
    /// How many tabs each group's strip shows before overflowing.
    pub limit: usize,
    /// The services this host has, which decide which panels may be shown.
    pub services: Vec<String>,
    /// The operations, in order.
    pub ops: Vec<String>,
}

impl LayoutOptions {
    /// Options for a default layout and no services.
    pub fn new() -> Self {
        Self {
            limit: 8,
            services: Vec::new(),
            ops: Vec::new(),
        }
    }
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

/// How to replay a script of window and display operations (task T04.06).
#[derive(Debug, PartialEq, Eq)]
pub struct WindowsOptions {
    /// The displays this host has, in order (the first is primary).
    pub displays: Vec<DisplayArg>,
    /// With `--main <w>x<h>`, the main window's size.
    pub main_width: Option<u32>,
    /// With `--main <w>x<h>`, the main window's height.
    pub main_height: Option<u32>,
    /// The operations, in order.
    pub ops: Vec<String>,
}

impl WindowsOptions {
    /// Options for a session with no displays named yet and one main window.
    pub fn new() -> Self {
        Self {
            displays: Vec::new(),
            main_width: None,
            main_height: None,
            ops: Vec::new(),
        }
    }
}

/// How to replay a script of window-chrome operations (task T04.07).
#[derive(Debug, PartialEq, Eq)]
pub struct ChromeOptions {
    /// The operations, in order: `focus:<area>`, `type:<text>`, `backspace:<n>`,
    /// `project:<name>`, `document:<path>`, `node:<label>`, `notice:<severity>:<source>:<summary>`,
    /// `toolbar`, `status`, `clear`.
    pub ops: Vec<String>,
}

impl ChromeOptions {
    /// Options for a fresh chrome.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to replay in-place text edits (task T06.01).
#[derive(Debug, PartialEq, Eq)]
pub struct TextOptions {
    /// The file to read the starting text from, when there is one.
    pub file: Option<PathBuf>,
    /// The operations: `splice:<at>:<remove>:<insert>`, `undo`, `redo`, `show`.
    pub ops: Vec<String>,
}

impl TextOptions {
    /// Options for an empty text.
    pub fn new() -> Self {
        Self {
            file: None,
            ops: Vec::new(),
        }
    }
}

/// How to show syntax highlighting (task T06.02).
#[derive(Debug, PartialEq, Eq)]
pub struct HighlightOptions {
    /// The language, when the caller names one rather than relying on the file's extension.
    pub language: Option<String>,
    /// The theme the roles are coloured by.
    pub theme: Option<String>,
    /// The file to read the text from, when there is one.
    pub file: Option<PathBuf>,
    /// The operations: `line:<text>`, `step:<n>`, `set:<line>:<text>`, `language:<l>`, `show`.
    pub ops: Vec<String>,
}

impl HighlightOptions {
    /// Options for an empty plain-text document.
    pub fn new() -> Self {
        Self {
            language: None,
            theme: None,
            file: None,
            ops: Vec::new(),
        }
    }
}

/// How to convert between document offsets and visual positions (task T06.03).
#[derive(Debug, PartialEq, Eq)]
pub struct CoordinatesOptions {
    /// How many columns a tab advances to.
    pub tab: Option<u32>,
    /// Wrap at this many columns.
    pub wrap: Option<u32>,
    /// The file to read the text from, when there is one.
    pub file: Option<PathBuf>,
    /// The operations: `line:<text>`, `fold:<start>:<end>`, `unfold:<start>`, `tab:<n>`,
    /// `wrap:<n>`, `visual:<offset>`, `offset:<row>:<column>`, `mark:<kind>:<start>:<end>`, `show`.
    pub ops: Vec<String>,
}

impl CoordinatesOptions {
    /// Options for an empty document with a four-column tab.
    pub fn new() -> Self {
        Self {
            tab: None,
            wrap: None,
            file: None,
            ops: Vec::new(),
        }
    }
}

/// How to run multi-cursor editing (task T06.04).
#[derive(Debug, PartialEq, Eq)]
pub struct EditOpsOptions {
    /// The file to read the starting text from, when there is one.
    pub file: Option<PathBuf>,
    /// The operations: `line:<text>`, `carets:<o>:...`, `select:<anchor>:<cursor>`, `insert:<text>`,
    /// `type:<char>`, `backspace`, `delete`, `up`, `down`, `duplicate`, `delete-lines`,
    /// `comment:<prefix>`, `matching:<offset>`, `undo`, `redo`, `show`.
    pub ops: Vec<String>,
}

impl EditOpsOptions {
    /// Options for an empty document.
    pub fn new() -> Self {
        Self {
            file: None,
            ops: Vec::new(),
        }
    }
}

/// How to read a file's policy and show the editor's decoration (task T06.05).
#[derive(Debug, PartialEq, Eq)]
pub struct EditorViewOptions {
    /// How many columns a tab advances to.
    pub tab: Option<u32>,
    /// The file to read, when there is one; otherwise the `line:` operations build the text.
    pub file: Option<PathBuf>,
    /// The operations: `line:<text>`, `whitespace`, `numbers[:relative]`,
    /// `window:<first>:<count>[:<current>]`, `fold:indent|braces`, `minimap:<height>`,
    /// `encode`, `encode-with:<lf|crlf|cr>`, `show`.
    pub ops: Vec<String>,
}

impl EditorViewOptions {
    /// Options for an empty document with a four-column tab.
    pub fn new() -> Self {
        Self {
            tab: None,
            file: None,
            ops: Vec::new(),
        }
    }
}

/// How to check writes against protected regions (task T06.06).
#[derive(Debug, PartialEq, Eq)]
pub struct WriteGuardOptions {
    /// The operations: `protect:<start>:<end>`, `writable`, `read-only[:<reason>]`,
    /// `input:<at>:<insert>`, `paste:<at>:<insert>`, `replace:<at>:<remove>:<insert>`,
    /// `format:<at>:<remove>:<insert>`, `external:<at>:<remove>`, `show`.
    pub ops: Vec<String>,
}

impl WriteGuardOptions {
    /// Options for a guard over no regions of a writable document.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to classify a document's size and show the degradation that follows (task T06.07).
#[derive(Debug, PartialEq, Eq)]
pub struct ScaleOptions {
    /// The file to classify, when there is one.
    pub file: Option<PathBuf>,
    /// The chunk size a cancellable read uses, in bytes.
    pub chunk: Option<usize>,
    /// The operations: `line:<text>`, `fill:<bytes>`, `long-line:<columns>`, `read`, `read-cancel`,
    /// `show`.
    pub ops: Vec<String>,
}

impl ScaleOptions {
    /// Options for an empty document.
    pub fn new() -> Self {
        Self {
            file: None,
            chunk: None,
            ops: Vec::new(),
        }
    }
}

/// How to expand a blueprint snippet and walk its placeholders (task T06.09).
#[derive(Debug, PartialEq, Eq)]
pub struct SnippetOptions {
    /// The operations, in the order they run: `text:<text>` and `snippet:<prefix>:<body>` set up the
    /// document and the snippet; `protect:<start>:<end>` and `read-only[:<reason>]` set the guard;
    /// `cursor:<at>`, `select:<start>:<end>`, `add-caret:<at>` and `add-select:<start>:<end>` place
    /// the carets; `expand:<prefix>`, `tab`, `shift-tab`, `type:<text>`, `cancel`, `undo` and
    /// `surround:<open>:<close>` act; `show` reports.
    pub ops: Vec<String>,
}

impl SnippetOptions {
    /// Options for an empty document with no snippet.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to replay the editing regression scenarios (task T06.08).
#[derive(Debug, PartialEq, Eq)]
pub struct RegressionOptions {
    /// The line count to use instead of each scenario's default.
    pub lines: Option<usize>,
    /// The scenarios to run: `normal`, `million`, `unicode`, `multi-view`; all of them when empty.
    pub scenarios: Vec<String>,
}

impl RegressionOptions {
    /// Options for every scenario at its default size.
    pub fn new() -> Self {
        Self {
            lines: None,
            scenarios: Vec::new(),
        }
    }
}

/// How to compare two texts or trees (task T08.07).
#[derive(Debug, PartialEq, Eq)]
pub struct DiffOptions {
    /// A file to read as the left side.
    pub left: Option<PathBuf>,
    /// A file to read as the right side.
    pub right: Option<PathBuf>,
    /// Treat `\r\n` and `\n` as the same ending; off by default, so an ending shows.
    pub ignore_endings: bool,
    /// Ignore trailing whitespace before an ending; off by default.
    pub ignore_trailing: bool,
    /// The operations: `left:<text>`, `right:<text>`, `tree-left:<path>=<text>`,
    /// `tree-right:<path>=<text>`, `show`, `trees`, `read-only`.
    pub ops: Vec<String>,
}

impl DiffOptions {
    /// Options for two empty texts, compared strictly.
    pub fn new() -> Self {
        Self {
            left: None,
            right: None,
            ignore_endings: false,
            ignore_trailing: false,
            ops: Vec::new(),
        }
    }
}

/// How to grow a selection, keep bookmarks, and toggle display elements (task T06.10).
#[derive(Debug, PartialEq, Eq)]
pub struct StructuralOptions {
    /// The operations, in the order they run: `text:<t>`, `cursor:<at>`, `select:<s>:<e>`,
    /// `expand:<n>`, `bookmark:<name>:<at>`, `drop:<name>`, `edit:<at>:<remove>:<insert>`,
    /// `toggle:<name>`, `baseline:<text>`, `inline`, `show`.
    pub ops: Vec<String>,
}

impl StructuralOptions {
    /// Options for an empty document.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to drive the language-service lifecycle (task T07.01).
#[derive(Debug, PartialEq, Eq)]
pub struct LanguageOptions {
    /// The operations: `caps:<c1>,<c2>`, `fail:<toolchain|spawn|handshake>`, `view:<project>`,
    /// `restart:<project>`, `close:<project>`, `show`.
    pub ops: Vec<String>,
}

impl LanguageOptions {
    /// Options for a registry with nothing open.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to synchronise documents and convert positions (task T07.02).
#[derive(Debug, PartialEq, Eq)]
pub struct ProtocolOptions {
    /// The operations: `open:<uri>=<text>`, `replace:<uri>=<text>`,
    /// `edit:<uri>:<version>:<sl>,<sc>:<el>,<ec>=<text>`, `save:<uri>`, `close:<uri>`,
    /// `units:<uri>:<byte>`, `offset:<uri>:<line>,<char>`, `show`.
    pub ops: Vec<String>,
}

impl ProtocolOptions {
    /// Options for no documents open.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to drive completion freshness (task T07.03).
#[derive(Debug, PartialEq, Eq)]
pub struct FeaturesOptions {
    /// The operations: `version:<n>`, `request`, `cancel:<id>`, `respond:<id>:<version>:<a,b>`,
    /// `show`.
    pub ops: Vec<String>,
}

impl FeaturesOptions {
    /// Options for a session at version 1.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to ask a project's navigation (task T07.04).
#[derive(Debug, PartialEq, Eq)]
pub struct NavigationOptions {
    /// The operations: `caps:<c1>,<c2>`, `symbol:<uri>:<name>:<kind>:<s>:<e>:<ns>:<ne>`,
    /// `definition:<name>`, `references:<name>`, `declaration:<name>`, `implementation:<name>`,
    /// `document-symbol:<uri>`, `workspace-symbol:<name>`, `outline:<uri>`,
    /// `breadcrumbs:<uri>:<offset>`, `show`.
    pub ops: Vec<String>,
}

impl NavigationOptions {
    /// Options with every capability negotiated.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to drive diagnostics, tokens and code actions (task T07.05).
#[derive(Debug, PartialEq, Eq)]
pub struct DiagnosticsOptions {
    /// The operations: `version:<n>`, `diag:<severity>:<source>:<s>:<e>:<message>`,
    /// `publish:<version>`, `protect:<s>:<e>`, `mark`, `token:<kind>:<s>:<e>`,
    /// `tokens:<version>`, `action:<title>:<at>:<remove>:<insert>`, `apply`, `show`.
    pub ops: Vec<String>,
}

impl DiagnosticsOptions {
    /// Options for an empty document at version 1.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to review and commit a cross-file edit (task T07.06).
#[derive(Debug, PartialEq, Eq)]
pub struct WorkspaceEditOptions {
    /// The operations: `label:<text>`, `open:<uri>:<version>=<text>`, `protect:<uri>:<s>:<e>`,
    /// `edit:<uri>:<baseline>:<at>:<remove>:<insert>`, `review`, `commit`, `show`.
    pub ops: Vec<String>,
}

impl WorkspaceEditOptions {
    /// Options for an empty edit over no documents.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to show schema documentation and target-relative completion (task T07.07).
#[derive(Debug, PartialEq, Eq)]
pub struct SchemaOptions {
    /// The operations: `lang:<rust|json|css>`, `entry:<key>:<requires|->:<detail>`,
    /// `target:<name>:<cap1>,<cap2>`, `complete:<prefix>`, `doc:<key>`, `show`.
    pub ops: Vec<String>,
}

impl SchemaOptions {
    /// Options for an empty CSS catalog and no target.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to run the language-service failure regression (task T07.08).
#[derive(Debug, PartialEq, Eq)]
pub struct HealthOptions {
    /// The scenarios to run: `crash`, `slow`, `no-toolchain`, `multi-project`; all of them when empty.
    pub scenarios: Vec<String>,
}

impl HealthOptions {
    /// Options for every scenario.
    pub fn new() -> Self {
        Self {
            scenarios: Vec::new(),
        }
    }
}

/// How to show hints, lenses and hierarchies (task T07.09).
#[derive(Debug, PartialEq, Eq)]
pub struct HintsOptions {
    /// The operations: `caps:<c1>,<c2>`, `version:<n>`, `inlay:<s>:<e>:<label>`,
    /// `lens:<s>:<e>:<label>:<id>:<title>`, `publish:<version>`, `dismiss:<kind>`,
    /// `invoke:<id>`, `hierarchy:<call|type>:<name>`, `show`.
    pub ops: Vec<String>,
}

impl HintsOptions {
    /// Options for a layer at version 1 with every capability.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to rebuild the analysis configuration (task T07.10).
#[derive(Debug, PartialEq, Eq)]
pub struct AnalysisOptions {
    /// The operations: `target:<name>`, `known:<name>`, `member:<name>`, `feature:<name>`,
    /// `kit:<name>`, `trust:<trusted|untrusted>`, `configure`, `publish:<generation>`, `show`.
    pub ops: Vec<String>,
}

impl AnalysisOptions {
    /// Options for no configuration yet.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to build the project file index (task T08.01).
#[derive(Debug, PartialEq, Eq)]
pub struct IndexOptions {
    /// The operations: `node:<path>:<file|dir|symlink>:<size|target>`, `ignore:<p1>,<p2>`,
    /// `binary:<e1>,<e2>`, `max-bytes:<n>`, `max-depth:<n>`, `max-entries:<n>`, `build`,
    /// `create:<path>:<size>`, `remove:<path>`, `modify:<path>:<size>`, `show`.
    pub ops: Vec<String>,
}

impl IndexOptions {
    /// Options for an empty tree under the usual policy.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to search file and project text (task T08.02).
#[derive(Debug, PartialEq, Eq)]
pub struct SearchOptions {
    /// The operations: `file:<path>=<text>`, `pattern:<text>`, `regex`, `literal`, `no-case`,
    /// `whole-word`, `range:<start>:<end>`, `max:<n>`, `search`, `cancel`, `show`.
    pub ops: Vec<String>,
}

impl SearchOptions {
    /// Options for a literal search over no files.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to open files and symbols by a query (task T08.03).
#[derive(Debug, PartialEq, Eq)]
pub struct QuickOpenOptions {
    /// The operations: `file:<path>`, `touch:<path>`, `caps:<c1>,<c2>`, `symbol:<name>:<path>`,
    /// `open:<query>`, `file-symbols:<query>`, `workspace-symbols:<query>`, `show`.
    pub ops: Vec<String>,
}

impl QuickOpenOptions {
    /// Options for no files and every capability.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to build replace candidates (task T08.04).
#[derive(Debug, PartialEq, Eq)]
pub struct ReplaceOptions {
    /// The operations: `file:<path>=<text>`, `pattern:<t>`, `regex`, `no-case`, `whole-word`,
    /// `template:<t>`, `protect:<uri>:<s>:<e>`, `read-only:<uri>`, `build`, `check:<i>`,
    /// `uncheck:<i>`, `check-all`, `uncheck-all`, `show`.
    pub ops: Vec<String>,
}

impl ReplaceOptions {
    /// Options for no files and no pattern.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to run a batch replace (task T08.05).
#[derive(Debug, PartialEq, Eq)]
pub struct BatchOptions {
    /// The operations: `open:<uri>:<version>=<text>`, `protect:<uri>:<s>:<e>`,
    /// `edit:<uri>:<baseline>:<at>:<remove>:<insert>`, `run`, `undo`, `show`.
    pub ops: Vec<String>,
}

impl BatchOptions {
    /// Options for an empty batch over no documents.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to walk the navigation history (task T08.06).
#[derive(Debug, PartialEq, Eq)]
pub struct HistoryNavOptions {
    /// The operations: `visit:<uri>:<offset>`, `edit:<uri>:<offset>`, `problem:<uri>:<offset>`,
    /// `back`, `forward`, `rename:<from>:<to>`, `vanished:<uri>`, `resolve`, `show`.
    pub ops: Vec<String>,
}

impl HistoryNavOptions {
    /// Options for an empty history.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to run the search/replace acceptance regression (task T08.08).
#[derive(Debug, PartialEq, Eq)]
pub struct SearchHealthOptions {
    /// The scenarios to run: `big-project`, `continuous-edits`, `conflict`, `cancel`; all when empty.
    pub scenarios: Vec<String>,
}

impl SearchHealthOptions {
    /// Options for every scenario.
    pub fn new() -> Self {
        Self {
            scenarios: Vec::new(),
        }
    }
}

/// How to show the widget catalog (task T09.01).
#[derive(Debug, PartialEq, Eq)]
pub struct CatalogOptions {
    /// The operations: `widget:<id>:<name>:<category>`, `unsupported:<id>:<reason>`,
    /// `property:<id>:<name>:<type>[:<default>]`, `limit:<id>:<note>`, `search:<query>`,
    /// `category:<name>`, `favorite:<id>`, `unfavorite:<id>`, `schema:<id>`, `show`.
    pub ops: Vec<String>,
}

impl CatalogOptions {
    /// Options for an empty catalog.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to drive the design scene (task T09.02).
#[derive(Debug, PartialEq, Eq)]
pub struct SceneOptions {
    /// The operations: `add:<node>:<x>,<y>,<w>,<h>`, `rebuild:<node>:<x>,<y>,<w>,<h>`,
    /// `remove:<node>`, `hit:<x>,<y>`, `resolve:<node>:<generation>`, `raise:<node>`, `show`.
    pub ops: Vec<String>,
}

impl SceneOptions {
    /// Options for an empty scene.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to run structural edits on the object tree (task T09.04).
#[derive(Debug, PartialEq, Eq)]
pub struct StructureOptions {
    /// The operations: `root:<kind>:<name>`, `insert:<parent>:<kind>:<name>`, `delete:<node>`,
    /// `copy:<node>`, `rename:<node>:<name>`, `reorder:<node>:<index>`, `reparent:<node>:<parent|->`,
    /// `reference:<from>:<to>`, `show`.
    pub ops: Vec<String>,
}

impl StructureOptions {
    /// Options for an empty tree.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to pan, zoom and show the design aids (task T09.05).
#[derive(Debug, PartialEq, Eq)]
pub struct ViewportOptions {
    /// The operations: `transform:<panx>,<pany>,<zoom>`, `pan:<dx>,<dy>`, `zoom:<factor>:`
    /// `<vw>,<vh>`, `zoom-at:<factor>:<ax>,<ay>`, `fit:<x>,<y>,<w>,<h>:<vw>,<vh>[:<margin>]`,
    /// `actual-size:<vw>,<vh>`, `spacing:<s>`, `ruler:<step>`, `grid:<on|off>`, `snap:<on|off>`,
    /// `snap:<x>,<y>`, `align:<x>,<y>,<w>,<h>:<x>,<y>,<w>,<h>[:<tol>]`, `to-screen:<x>,<y>`,
    /// `to-model:<x>,<y>`, `overlay:<vw>,<vh>`, `state`, `show`.
    pub ops: Vec<String>,
}

impl ViewportOptions {
    /// Options for an unmodified viewport.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to drag a node and commit the change as a transaction (task T09.06).
#[derive(Debug, PartialEq, Eq)]
pub struct DragOptions {
    /// The operations: `root:<kind>:<name>`, `insert:<parent>:<kind>:<name>`, `begin:<node>`,
    /// `candidates`, `aim:<parent|->:<index>`, `preview`, `release`, `cancel`,
    /// `stage:<node>:<parent|->:<index>`, `commit`, `discard`, `undo`, `redo`, `state`, `show`.
    pub ops: Vec<String>,
}

impl DragOptions {
    /// Options for an empty design.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to inspect design-time object state, filter and locate, and pages (task T09.07).
#[derive(Debug, PartialEq, Eq)]
pub struct ObjectsOptions {
    /// The operations: `node:<id>:<kind>[:visible=<bool>][:name=<text>]`, `hide:<id>`,
    /// `unhide:<id>`, `lock:<id>`, `unlock:<id>`, `raise:<id>`, `lower:<id>`, `published:<id>`,
    /// `visible:<id>`, `filter-text:<text>`, `filter-kind:<kind>`,
    /// `filter-state:<hidden|shown|locked|unlocked>`, `filter-clear`, `filter`, `locate:<text>`,
    /// `page:<id>:<name>:<root>`, `activate:<id>`, `close-page:<id>`, `rename-page:<id>:<name>`,
    /// `state`, `show`.
    pub ops: Vec<String>,
}

impl ObjectsOptions {
    /// Options for an empty document.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to drive the design surface's keyboard and stress harness (task T09.08).
#[derive(Debug, PartialEq, Eq)]
pub struct DesignOptions {
    /// The operations: `node:<parent|->:<kind>:<name>`, `select:<node>`, `all`, `deselect`,
    /// `delete`, `undo`, `redo`, `next`, `previous`, `parent`, `child`, `press:<chord>`,
    /// `stress:<nodes>:<rounds>`, `state`, `show`.
    pub ops: Vec<String>,
}

impl DesignOptions {
    /// Options for an empty surface.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to exercise the layout contract and its diagnostics (task T10.01).
#[derive(Debug, PartialEq, Eq)]
pub struct LayoutPlanOptions {
    /// The operations: `available:<node>:<w>,<h>`, `constraint:<node>:<property>:<origin>`,
    /// `box:<node>:<x>,<y>,<w>,<h>`, `run:<preview|generation>`, `source:<node>`, `state`, `show`.
    pub ops: Vec<String>,
}

impl LayoutPlanOptions {
    /// Options for an empty plan.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to switch a container's layout and migrate its children (task T10.02).
#[derive(Debug, PartialEq, Eq)]
pub struct LayoutSwitchOptions {
    /// The operations: `layout:<kind>`, `child:<node>:<property>`, `switch`, `detach`,
    /// `state`, `show`.
    pub ops: Vec<String>,
}

impl LayoutSwitchOptions {
    /// Options for a container with no children.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to set and validate a node's box constraints (task T10.03).
#[derive(Debug, PartialEq, Eq)]
pub struct ConstraintsOptions {
    /// The operations: `min:<node>:<w>,<h>`, `max:<node>:<w>,<h>`, `preferred:<node>:<w>,<h>`,
    /// `grow:<node>:<f>`, `margin:<node>:<f>`, `gap:<node>:<f>`, `overflow:<node>:<kind>`,
    /// `validate`, `state`, `show`.
    pub ops: Vec<String>,
}

impl ConstraintsOptions {
    /// Options for a node with no constraints.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to align, distribute and nudge a selection as constraint writes (task T10.04).
#[derive(Debug, PartialEq, Eq)]
pub struct ArrangeOptions {
    /// The operations: `layout:<kind>`, `select:<node>`, `align:<axis>:<mode>`,
    /// `distribute:<axis>`, `match:<axis>`, `nudge:<axis>:<direction>:<step>[:<spacing>]`,
    /// `state`, `show`.
    pub ops: Vec<String>,
}

impl ArrangeOptions {
    /// Options for an empty selection.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to check simulated frames against a target's support (task T10.05).
#[derive(Debug, PartialEq, Eq)]
pub struct FramesOptions {
    /// The operations: `support-size:<w>,<h>`, `support-orientation:<portrait|landscape>`,
    /// `support-safe-area:<true|false>`, `support-max-scale:<f>`, `frame:<label>:<w>,<h>:<orientation>`,
    /// `frame-scale:<label>:<f>`, `frame-insets:<label>:<t>,<r>,<b>,<l>`, `check`, `state`, `show`.
    pub ops: Vec<String>,
}

impl FramesOptions {
    /// Options for no frames and no support.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to build the focus order and inspect it (task T10.06).
#[derive(Debug, PartialEq, Eq)]
pub struct FocusOrderOptions {
    /// The operations: `root:<kind>:<name>`, `insert:<parent>:<kind>:<name>`, `focusable:<node>`,
    /// `exclude:<node>`, `index:<node>:<n>`, `build`, `step:<node>:<delta>`, `export`, `state`, `show`.
    pub ops: Vec<String>,
}

impl FocusOrderOptions {
    /// Options for an empty tree.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to declare a target's responsive capability and build layout variants (task T10.07).
#[derive(Debug, PartialEq, Eq)]
pub struct VariantsOptions {
    /// The operations: `capability:<kind>`, `variant:<name>`, `when:<condition>`,
    /// `value:<key>=<value>`, `add`, `state`, `show`.
    pub ops: Vec<String>,
}

impl VariantsOptions {
    /// Options for a target that supports nothing.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to compare the preview with a library-generated probe (task T10.08).
#[derive(Debug, PartialEq, Eq)]
pub struct LayoutProbeOptions {
    /// The operations: `scenario:<name>`, `available:<node>:<w>,<h>`, `box:<node>:<x>,<y>,<w>,<h>`,
    /// `probe-box:<node>:<x>,<y>,<w>,<h>`, `compare`, `state`, `show`.
    pub ops: Vec<String>,
}

impl LayoutProbeOptions {
    /// Options for an empty comparison.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to drive the schema-driven property editor (task T11.01).
#[derive(Debug, PartialEq, Eq)]
pub struct PropertyEditorOptions {
    /// The operations: `schema:<property>:<type>`, `set:<node>:<property>:<text>`,
    /// `value:<node>:<property>`, `pending:<node>:<property>`, `state`, `show`.
    pub ops: Vec<String>,
}

impl PropertyEditorOptions {
    /// Options for an editor with an empty schema.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to inspect a multi-selection's shared properties and reset an override (task T11.02).
#[derive(Debug, PartialEq, Eq)]
pub struct MultiPropertyOptions {
    /// The operations: `node:<id>`, `set:<property>:<value>:<origin>`, `common`,
    /// `value:<property>`, `reset:<node>:<property>[:<fallback>]`, `state`, `show`.
    pub ops: Vec<String>,
}

impl MultiPropertyOptions {
    /// Options for an empty selection.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to read a CSS rule view and compute an element's style (task T11.03).
#[derive(Debug, PartialEq, Eq)]
pub struct CssOptions {
    /// The operations: `css:<rule text>` (append CSS text), `element:<compound>`,
    /// `value:<property>`, `source:<property>`, `properties`, `state`, `show`.
    pub ops: Vec<String>,
}

impl CssOptions {
    /// Options for an empty sheet.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to define design tokens and expand references at generation time (task T11.04).
#[derive(Debug, PartialEq, Eq)]
pub struct TokensOptions {
    /// The operations: `token:<kind>:<name>:<value>`, `theme:<name>:<token>=<value>[,...]`,
    /// `use:<property>:<value>`, `expand[:<theme>]`, `rename:<from>:<to>`, `state`, `show`.
    pub ops: Vec<String>,
}

impl TokensOptions {
    /// Options for an empty token set.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to preview interaction states and review theme/state differences (task T11.05).
#[derive(Debug, PartialEq, Eq)]
pub struct StatesOptions {
    /// The operations: `css:<rule text>`, `element:<compound>`, `on:<state>`,
    /// `declared:<event>`, `token:<kind>:<name>:<value>`, `theme:<name>:<token>=<value>[,...]`,
    /// `preview`, `compare:<left>[:<right>]`, `diff:<state>:<state>`, `state`, `show`.
    pub ops: Vec<String>,
}

impl StatesOptions {
    /// Options with no states requested.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to configure transitions/animations and control their preview (task T11.06).
#[derive(Debug, PartialEq, Eq)]
pub struct MotionOptions {
    /// The operations: `transitions:<on|off>`, `animations:<on|off>`, `property:<name>`,
    /// `easing:<name>`, `transition:<property>:<ms>:<delay_ms>:<easing>`,
    /// `animation:<property>:<ms>:<delay_ms>:<easing>`, `check`, `play`, `pause`, `seek:<ms>`,
    /// `state`, `show`.
    pub ops: Vec<String>,
}

impl MotionOptions {
    /// Options with an empty support table.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to emit a style for the preview and the generator, and migrate literals to tokens (T11.07).
#[derive(Debug, PartialEq, Eq)]
pub struct StyleOptions {
    /// The operations: `css:<rule text>`, `element:<compound>`, `inline:<property>:<value>`,
    /// `token:<kind>:<name>:<value>`, `theme:<name>:<token>=<value>[,...]`, `under:<theme|->`,
    /// `preview`, `generate`, `migrate`, `state`, `show`.
    pub ops: Vec<String>,
}

impl StyleOptions {
    /// Options with no input.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to exercise the style boundaries and undo a style edit (task T11.08).
#[derive(Debug, PartialEq, Eq)]
pub struct StyleBoundaryOptions {
    /// The operations: `css:<rule text>`, `element:<compound>`, `token:<kind>:<name>:<value>`,
    /// `theme:<name>:<token>=<value>[,...]`, `inline:<property>:<value>`, `clear:<property>`,
    /// `rename:<from>:<to>`, `use-theme:<theme|->`, `undo`, `emit`, `contrast:<a>:<b>`, `state`,
    /// `show`.
    pub ops: Vec<String>,
}

impl StyleBoundaryOptions {
    /// Options with an empty session.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to declare a data schema and the sample/runtime/error datasets (task T13.01).
#[derive(Debug, PartialEq, Eq)]
pub struct DataOptions {
    /// The operations: `schema:<name>:<version>`, `field:<name>:<kind>[:required]`,
    /// `sample:<field>=<value>`, `runtime:<field>=<value>`, `error:<reason>`, `check`, `active`,
    /// `state`, `show`.
    pub ops: Vec<String>,
}

impl DataOptions {
    /// Options with an empty schema.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to declare typed bindings between data and node properties (task T13.02).
#[derive(Debug, PartialEq, Eq)]
pub struct BindingOptions {
    /// The operations: `schema:<name>:<version>`, `field:<name>:<kind>`, `slot:<node>.<property>:<type>`,
    /// `bind:<source>=<target>:<direction>:<conversion>`, `check`, `state`, `show`; a source/target
    /// is `f:<field>` or `<node>.<property>`.
    pub ops: Vec<String>,
}

impl BindingOptions {
    /// Options with an empty graph.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to declare events, wire handlers and fire triggers (task T13.03).
#[derive(Debug, PartialEq, Eq)]
pub struct EventsOptions {
    /// The operations: `event:<name>`, `field:<event>:<field>:<kind>[:required]`,
    /// `attach:<node>:<event>:<handler>`, `status:<node>:<event>`,
    /// `fire:<node>:<event>:<field>=<value>[,...]`, `check`, `state`, `show`.
    pub ops: Vec<String>,
}

impl EventsOptions {
    /// Options with no declarations.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to bind handler actions, check permissions and release subscriptions (task T13.04).
#[derive(Debug, PartialEq, Eq)]
pub struct DispatchOptions {
    /// The operations: `slot:<node>.<property>:<type>`, `command:<id>`, `function:<name>`, `trust`,
    /// `authorize:<capability>`, `bind:<node>:<event>:set:<property>=<value>`,
    /// `bind:<node>:<event>:command:<id>`, `bind:<node>:<event>:call:<function>:<capability>`,
    /// `unbind:<node>:<event>`, `dispatch:<node>:<event>`, `check`, `state`, `show`.
    pub ops: Vec<String>,
}

impl DispatchOptions {
    /// Options with nothing bound.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to declare pages, navigate and drive a window's lifecycle (task T13.05).
#[derive(Debug, PartialEq, Eq)]
pub struct RoutesOptions {
    /// The operations: `page:<name>`, `entry:<name>`, `edge:<from>:<to>`, `open:<page>`, `ready`,
    /// `go:<page>`, `back`, `forward`, `close`, `finish-close`, `check`, `state`, `show`.
    pub ops: Vec<String>,
}

impl RoutesOptions {
    /// Options with no pages declared.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to declare a state machine and send events to it (task T13.06).
#[derive(Debug, PartialEq, Eq)]
pub struct MachineOptions {
    /// The operations: `state:<name>`, `initial:<name>`, `final:<name>`,
    /// `transition:<from>:<on>:<to>[:<guard>]`, `answer:<guard>:<true|false>:<sync|async>`, `start`,
    /// `send:<event>[:<sync|async>]`, `inspect`, `show`.
    pub ops: Vec<String>,
}

impl MachineOptions {
    /// Options with no states declared.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to drive a file/network adapter with a sample double and a retry policy (task T13.07).
#[derive(Debug, PartialEq, Eq)]
pub struct AdapterOptions {
    /// The operations: `sample:<outcome>`, `live:<target>:<outcome>`, `request:<kind>:<target>`,
    /// `cancel`, `online`, `offline`, `attempt:<n>`, `run`, `check`, `state`, `show`; an outcome is
    /// `ok:<value>`/`failed:<reason>`/`unknown:<reason>`/`cancelled`.
    pub ops: Vec<String>,
}

impl AdapterOptions {
    /// Options with no adapter.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to drive the joint acceptance: the undo log, vectors and the two-way loop (task T13.08).
#[derive(Debug, PartialEq, Eq)]
pub struct SessionLogOptions {
    /// The operations: `human:<label>:<before>:<after>`, `business:<label>:<before>:<after>`,
    /// `undo`, `loop:<identity|drifting>:<value>`, `state:<name>`, `initial:<name>`, `final:<name>`,
    /// `transition:<from>:<on>:<to>`, `expect:<event>:<state>`, `vector`, `check`, `show`.
    pub ops: Vec<String>,
}

impl SessionLogOptions {
    /// Options with an empty log.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to drive the preview service's lifecycle (task T14.01).
#[derive(Debug, PartialEq, Eq)]
pub struct PreviewOptions {
    /// The operations: `start:<session>`, `ready`, `spawn:<pid>`, `install:<node>`, `rebuild:<node>`,
    /// `stop`, `finish-stop`, `clean`, `state`, `show`.
    pub ops: Vec<String>,
}

impl PreviewOptions {
    /// Options with a stopped service.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to classify edits as live updates or rebuilds (task T14.02).
#[derive(Debug, PartialEq, Eq)]
pub struct HotUpdateOptions {
    /// The operations: `self:<source>`, `from:<source>`, `property:<subject>`, `style:<subject>`,
    /// `existing:<subject>`, `add-binding:<subject>`, `remove-binding:<subject>`, `add-node:<subject>`,
    /// `remove-node:<subject>`, `text:<subject>`, `flush`, `check`, `show`.
    pub ops: Vec<String>,
}

impl HotUpdateOptions {
    /// Options with nothing pending.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to capture and restore the live UI state across a rebuild (task T14.03).
#[derive(Debug, PartialEq, Eq)]
pub struct StateKeepOptions {
    /// The operations: `keep:<node>:<kind>`, `focus:<node>`, `scroll:<node>:<n>`,
    /// `input:<node>:<text>`, `selection:<node>:<start>:<end>`, `present:<node>:<kind>`, `restore`,
    /// `check`, `show`.
    pub ops: Vec<String>,
}

impl StateKeepOptions {
    /// Options with an empty capture.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to drive the preview viewport matrix (task T14.04).
#[derive(Debug, PartialEq, Eq)]
pub struct MatrixOptions {
    /// The operations: `document:<name>`, `viewport:<name>:<w>x<h>:<orientation>`, `theme:<name>:<t>`,
    /// `language:<name>:<lang>`, `sample:<name>:<s>`, `commit`, `start:<name>`, `stop:<name>`,
    /// `restart:<name>`, `remove:<name>`, `check`, `show`.
    pub ops: Vec<String>,
}

impl MatrixOptions {
    /// Options with an empty matrix.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to drive a preview inspection (task T14.05).
#[derive(Debug, PartialEq, Eq)]
pub struct InspectOptions {
    /// The operations: `mode:<m>`, `adapter:<a>`, `edit:<version>`, `instance:<name>:<version>`,
    /// `stop`, `kind:<node>:<kind>`, `design:<node>:<property>:<text>`,
    /// `computed:<node>:<property>:<text>`, `runtime:<node>:<property>:<text>:<version>`,
    /// `source:<node>:<file>:<line>:<column>`, `child:<node>:<parent>`, `event:<name>:<payload>`,
    /// `data:<field>:<value>`, `values:<node>:<property>`, `locate:<node>`, `trigger:<action>`,
    /// `check`, `show`.
    pub ops: Vec<String>,
}

impl InspectOptions {
    /// Options with an empty inspection.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to compare a preview screenshot with its recorded baseline (task T14.06).
#[derive(Debug, PartialEq, Eq)]
pub struct VisualOptions {
    /// The operations: `platform:<p>`, `dpi:<n>`, `resources:<n>`, `fonts:<name>`,
    /// `tolerance:<percent>:<channel-step>`, `record:<name>:<w>x<h>:<digits>`,
    /// `shot:<name>:<w>x<h>:<digits>`, `drop:<name>`, `baseline`, `check`, `show`.
    pub ops: Vec<String>,
}

impl VisualOptions {
    /// Options with no baselines and no shots.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to drive the preview app's failure, crash and recovery (task T14.07).
#[derive(Debug, PartialEq, Eq)]
pub struct PreviewRecoveryOptions {
    /// The operations: `edit:<version>`, `launch`, `reload`, `rebuild`, `reset`, `exit:<code>`,
    /// `crash:<signal>`, `fault:<reason>`, `clear`, `check`, `show`.
    pub ops: Vec<String>,
}

impl PreviewRecoveryOptions {
    /// Options with a preview that has never started.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to run bind/rebuild cycles and check what must not grow (task T14.08).
#[derive(Debug, PartialEq, Eq)]
pub struct LoadOptions {
    /// The operations: `budget:<ms>`, `cycle:<viewport>:<subscriptions>:<nodes>[:<latency>]`,
    /// `repeat:<viewport>:<cycles>:<subscriptions>:<nodes>[:<latency>]`,
    /// `leak:<viewport>:<cycles>:<subscriptions>:<nodes>`, `invalid:<text>`, `check`, `verdict`,
    /// `show`.
    pub ops: Vec<String>,
}

impl LoadOptions {
    /// Options with no cycles and no budget.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to route a keypress and show the entry points that share one state (task T05.05).
#[derive(Debug, PartialEq, Eq)]
pub struct FocusOptions {
    /// Where the keyboard is: a target name (canvas, outline, inspector, editor, tabs, status, welcome).
    pub target: String,
    /// The mode the designer is in.
    pub mode: String,
    /// The chord to press; absent shows the toolbar and the menu instead.
    pub press: Option<String>,
    /// A menu to show as a context menu, by title.
    pub menu: Option<String>,
    /// Whether the project is trusted.
    pub trust: bool,
    /// The authorized effects.
    pub authorize: Vec<String>,
    /// The tasks already running.
    pub running: Vec<String>,
}

impl FocusOptions {
    /// Options that show the toolbar for the canvas.
    pub fn new(target: impl Into<String>) -> Self {
        Self {
            target: target.into(),
            mode: "design".to_owned(),
            press: None,
            menu: None,
            trust: false,
            authorize: Vec::new(),
            running: Vec::new(),
        }
    }
}

/// How a workspace's dependencies should be resolved (task T02.11).
///
/// Showing the resolution runs nothing: it prints the command and the effects that command would
/// have. Nothing is started until every effect has been authorized, because a dependency command is
/// the one command in this build that can write a file or reach the network. The flags *are* those
/// effects: `--allow-lockfile-change` lets the resolution write the lockfile, `--allow-network` lets
/// it reach the registry, and neither asks for an upgrade — the resolution stays locked unless the
/// lockfile itself may change, and then the change is what the report is about.
#[derive(Debug, PartialEq, Eq)]
pub struct DependenciesOptions {
    /// The workspace directory: the one the manifest is in.
    pub directory: PathBuf,
    /// The record to compare the resolution against, when there is one.
    pub review: Option<PathBuf>,
    /// Where to write the record, when the caller asked for one.
    pub record: Option<PathBuf>,
    /// Whether the lockfile may be written.
    pub allow_lockfile_change: bool,
    /// Whether the registry may be reached.
    pub allow_network: bool,
    /// Whether the person answering the directory's question said yes.
    pub trust: bool,
}

impl DependenciesOptions {
    /// Options that resolve a workspace and change nothing.
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
            review: None,
            record: None,
            allow_lockfile_change: false,
            allow_network: false,
            trust: false,
        }
    }
}

/// What to do with a workspace.
#[derive(Debug, PartialEq, Eq)]
pub enum WorkspaceAction {
    /// Show it: its members, what each holds, and its files qualified by member.
    Report,
    /// Name a root that is already there as a member.
    Add {
        /// The name the member is addressed by.
        name: String,
        /// The root, relative to the workspace root.
        root: String,
    },
    /// Take a member out of the document, and touch no directory.
    Remove {
        /// The member's name.
        name: String,
    },
    /// Record which member is active, on this machine.
    Activate {
        /// The member's name.
        name: String,
    },
    /// Show one file, addressed as `member:path`.
    File {
        /// The `member:path` name.
        name: String,
    },
}

/// How a workspace should be looked at, and how far to go (task T02.09).
///
/// Showing a workspace writes nothing; a change is planned first and only carried out with `apply`,
/// which is what asks the directory's question and takes the write permission. What a change writes
/// is the workspace document and nothing else: a member is a line in it, so adding names a root that
/// is already there and removing leaves the directory exactly as it is.
#[derive(Debug, PartialEq, Eq)]
pub struct WorkspaceOptions {
    /// The workspace document.
    pub path: PathBuf,
    /// What to do.
    pub action: WorkspaceAction,
    /// Whether a planned change should be carried out.
    pub apply: bool,
    /// Whether the person answering the directory's question said yes.
    pub trust: bool,
}

impl WorkspaceOptions {
    /// Options that show a workspace and change nothing.
    pub fn new(path: impl Into<PathBuf>, action: WorkspaceAction) -> Self {
        Self {
            path: path.into(),
            action,
            apply: false,
            trust: false,
        }
    }
}

/// What to create a workspace from.
#[derive(Debug, PartialEq, Eq)]
pub struct NewWorkspaceOptions {
    /// The directory the workspace document goes in, which has to be there.
    pub directory: PathBuf,
    /// The name a person sees; the directory's own name when nothing is given.
    pub name: Option<String>,
}

impl NewWorkspaceOptions {
    /// Options for a directory.
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
            name: None,
        }
    }
}

/// How a project should be checked, and what should be recorded or compared (task T02.08).
///
/// Checking writes nothing outside the two paths below, and never the project document: `record` is
/// where a scan of the project is written, and `rescan` is the scan to compare against. Both are
/// optional, and neither is the project.
#[derive(Debug, PartialEq, Eq)]
pub struct ValidateOptions {
    /// The project document.
    pub path: PathBuf,
    /// Whether to read the toolchain, which runs two programs.
    pub diagnose: bool,
    /// Whether the person answering the project's question said yes.
    pub trust: bool,
    /// Where to write a scan of the project, when one should be kept.
    pub record: Option<PathBuf>,
    /// The scan to compare against, when a re-scan was asked for.
    pub rescan: Option<PathBuf>,
}

impl ValidateOptions {
    /// Options that check a project and write nothing.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            diagnose: false,
            trust: false,
            record: None,
            rescan: None,
        }
    }
}

/// An override given on the command line: which setting, and what value.
#[derive(Debug, PartialEq, Eq)]
pub struct OverrideWord {
    /// The setting's word.
    pub key: String,
    /// The value, as a word or as a list.
    pub value: SettingValue,
}

/// How a project's configuration should be shown, and whether anything should be recorded.
#[derive(Debug, PartialEq, Eq)]
pub struct ConfigurationOptions {
    /// The project document.
    pub path: PathBuf,
    /// The target to resolve for, instead of the one this machine chose.
    pub target: Option<String>,
    /// Overrides for this run, which are also what `--remember` records.
    pub overrides: Vec<OverrideWord>,
    /// Whether the target and the overrides should be recorded for this project on this machine.
    pub remember: bool,
    /// Whether everything recorded for this project should be forgotten.
    pub forget: bool,
}

impl ConfigurationOptions {
    /// Options that show a project's configuration and record nothing.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            target: None,
            overrides: Vec::new(),
            remember: false,
            forget: false,
        }
    }
}

/// Which file operation to plan, and whether to do it.
///
/// Without `apply` this only reads the project and prints what the operation would do, which is
/// what the design surface shows before anyone agrees to it. With `apply` the plan is carried out:
/// the files move first, then the document is written, and a failed write puts the moves back.
#[derive(Debug, PartialEq, Eq)]
pub struct FileOpOptions {
    /// The project document.
    pub path: PathBuf,
    /// The operation, as given on the command line.
    pub operation: FileOperation,
    /// Whether the plan should be carried out instead of only printed.
    pub apply: bool,
    /// Whether the person answering the project's question said yes.
    pub trust: bool,
}

impl FileOpOptions {
    /// Options that preview an operation on a project.
    pub fn new(path: impl Into<PathBuf>, operation: FileOperation) -> Self {
        Self {
            path: path.into(),
            operation,
            apply: false,
            trust: false,
        }
    }
}

/// How long the baseline session should live.
///
/// A start that cannot end is not evidence and not usable by a person, so the duration is part of
/// the command rather than a hidden constant; the default matches
/// [`crate::host::platform::BASELINE_SESSION`].
#[derive(Debug, PartialEq, Eq)]
pub struct LaunchOptions {
    pub exit_after: Duration,
}

impl Default for LaunchOptions {
    fn default() -> Self {
        Self {
            exit_after: crate::host::platform::BASELINE_SESSION,
        }
    }
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

pub fn parse<I, S>(arguments: I) -> Result<Command, ParseError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    // The global option is removed here so every arm below can assume it is looking at the command;
    // `log_level_of` reads it separately, and both agree because they share this token handling.
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
        stripped.push(collected[index].clone());
        index += 1;
    }

    let mut arguments = stripped.into_iter();
    let Some(argument) = arguments.next() else {
        return Ok(Command::Status);
    };
    let text = argument.to_str().unwrap_or_default();

    let command = match text {
        "--help" | "-h" => Command::Help,
        "--version" | "-V" => Command::Version,
        "--launch" => {
            let mut options = LaunchOptions::default();
            let mut rest = arguments;
            while let Some(argument) = rest.next() {
                if argument == OsStr::new("--exit-after-ms") {
                    let Some(value) = rest.next() else {
                        return Err(ParseError { argument });
                    };
                    let Some(milliseconds) =
                        value.to_str().and_then(|value| value.parse::<u64>().ok())
                    else {
                        return Err(ParseError { argument: value });
                    };
                    options.exit_after = Duration::from_millis(milliseconds);
                    continue;
                }
                return Err(ParseError { argument });
            }
            return Ok(Command::Launch(options));
        }
        "--new-project" => {
            let Some(directory) = arguments.next() else {
                return Err(ParseError { argument });
            };
            let mut options = NewProjectOptions::new(PathBuf::from(directory), Template::BasicPage);
            let mut rest = arguments;
            while let Some(argument) = rest.next() {
                let flag = argument.to_str().unwrap_or_default();
                match flag {
                    "--template" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(template) = value.to_str().and_then(Template::from_name) else {
                            return Err(ParseError { argument: value });
                        };
                        options.template = template;
                    }
                    "--name" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(name) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        options.name = Some(name.to_owned());
                    }
                    _ => return Err(ParseError { argument }),
                }
            }
            return Ok(Command::NewProject(options));
        }
        "--open-project" => {
            let Some(path) = arguments.next() else {
                return Err(ParseError { argument });
            };
            let mut options = OpenProjectOptions::new(PathBuf::from(path));
            for argument in arguments {
                match argument.to_str().unwrap_or_default() {
                    "--trust" => options.trust = true,
                    "--diagnose" => options.diagnose = true,
                    _ => return Err(ParseError { argument }),
                }
            }
            return Ok(Command::OpenProject(options));
        }
        "--recent" => {
            let mut rest = arguments;
            if let Some(argument) = rest.next() {
                return Err(ParseError { argument });
            }
            return Ok(Command::Recent);
        }
        "--project-tree" => {
            let Some(path) = arguments.next() else {
                return Err(ParseError { argument });
            };
            let mut rest = arguments;
            if let Some(argument) = rest.next() {
                return Err(ParseError { argument });
            }
            return Ok(Command::ProjectTree(PathBuf::from(path)));
        }
        "--drop-entry" => {
            let Some(path) = arguments.next() else {
                return Err(ParseError { argument });
            };
            let mut options = DropEntryOptions::new(PathBuf::from(path), "");
            let mut rest = arguments;
            while let Some(argument) = rest.next() {
                match argument.to_str().unwrap_or_default() {
                    "--document" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(document) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        options.document = document.to_owned();
                    }
                    "--fix" => options.fix = true,
                    "--trust" => options.trust = true,
                    _ => return Err(ParseError { argument }),
                }
            }
            if options.document.is_empty() {
                return Err(ParseError { argument });
            }
            return Ok(Command::DropEntry(options));
        }
        "--file-op" => {
            let Some(path) = arguments.next() else {
                return Err(ParseError { argument });
            };
            let mut rest = arguments;
            let mut operation = None;
            let mut apply = false;
            let mut trust = false;
            while let Some(argument) = rest.next() {
                match argument.to_str().unwrap_or_default() {
                    "--add" | "--exclude" | "--delete" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(path) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        operation = Some(match argument.to_str().unwrap_or_default() {
                            "--add" => FileOperation::Add {
                                path: path.to_owned(),
                            },
                            "--exclude" => FileOperation::Exclude {
                                path: path.to_owned(),
                            },
                            _ => FileOperation::Delete {
                                path: path.to_owned(),
                            },
                        });
                    }
                    "--move" | "--rename" => {
                        let Some(from) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(to) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let (Some(from), Some(to)) = (from.to_str(), to.to_str()) else {
                            return Err(ParseError { argument });
                        };
                        operation = Some(FileOperation::Move {
                            from: from.to_owned(),
                            to: to.to_owned(),
                        });
                    }
                    "--apply" => apply = true,
                    "--trust" => trust = true,
                    _ => return Err(ParseError { argument }),
                }
            }
            let Some(operation) = operation else {
                return Err(ParseError { argument });
            };
            let mut options = FileOpOptions::new(PathBuf::from(path), operation);
            options.apply = apply;
            options.trust = trust;
            return Ok(Command::FileOp(options));
        }
        "--kit" => {
            let Some(path) = arguments.next() else {
                return Err(ParseError { argument });
            };
            let mut options = KitOptions::new(PathBuf::from(path));
            let mut rest = arguments;
            while let Some(argument) = rest.next() {
                match argument.to_str().unwrap_or_default() {
                    "--diagnose" => options.diagnose = true,
                    "--trust" => options.trust = true,
                    "--triple" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(triple) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        if triple.trim().is_empty() {
                            return Err(ParseError { argument: value });
                        }
                        options.triple = Some(triple.trim().to_owned());
                    }
                    _ => return Err(ParseError { argument }),
                }
            }
            return Ok(Command::Kit(options));
        }
        "--dependencies" => {
            let Some(directory) = arguments.next() else {
                return Err(ParseError { argument });
            };
            let mut options = DependenciesOptions::new(PathBuf::from(directory));
            let mut rest = arguments;
            while let Some(argument) = rest.next() {
                match argument.to_str().unwrap_or_default() {
                    "--allow-lockfile-change" => options.allow_lockfile_change = true,
                    "--allow-network" => options.allow_network = true,
                    "--trust" => options.trust = true,
                    "--review" | "--record" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(path) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        if argument.to_str().unwrap_or_default() == "--review" {
                            options.review = Some(PathBuf::from(path));
                        } else {
                            options.record = Some(PathBuf::from(path));
                        }
                    }
                    _ => return Err(ParseError { argument }),
                }
            }
            return Ok(Command::Dependencies(options));
        }
        "--edit" => {
            let Some(path) = arguments.next() else {
                return Err(ParseError { argument });
            };
            let mut options = EditOptions::new(PathBuf::from(path));
            let mut rest = arguments;
            while let Some(argument) = rest.next() {
                match argument.to_str().unwrap_or_default() {
                    "--apply" => options.apply = true,
                    "--undo" => options.undo = true,
                    "--batch" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(path) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        options.batch = Some(PathBuf::from(path));
                    }
                    "--root" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(path) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        options.root = Some(PathBuf::from(path));
                    }
                    _ => return Err(ParseError { argument }),
                }
            }
            if options.undo && !options.apply {
                return Err(ParseError { argument });
            }
            return Ok(Command::Edit(options));
        }
        "--history" => {
            let Some(path) = arguments.next() else {
                return Err(ParseError { argument });
            };
            let mut options = HistoryOptions::new(PathBuf::from(path));
            let mut rest = arguments;
            while let Some(argument) = rest.next() {
                let flag = argument.to_str().unwrap_or_default();
                match flag {
                    "--merge" => options.merge = true,
                    "--text" | "--text-file" | "--batch" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(value) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        options.edits.push(match flag {
                            "--text" => HistoryEdit::Text(value.to_owned()),
                            "--text-file" => HistoryEdit::File(PathBuf::from(value)),
                            _ => HistoryEdit::Batch(PathBuf::from(value)),
                        });
                    }
                    "--undo" | "--redo" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(count) =
                            value.to_str().and_then(|value| value.parse::<usize>().ok())
                        else {
                            return Err(ParseError { argument: value });
                        };
                        options.edits.push(if flag == "--undo" {
                            HistoryEdit::Undo(count)
                        } else {
                            HistoryEdit::Redo(count)
                        });
                    }
                    "--only" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(domain) = value.to_str().and_then(HistoryDomain::from_name) else {
                            return Err(ParseError { argument: value });
                        };
                        options.only = Some(domain);
                    }
                    "--root" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(value) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        options.root = Some(PathBuf::from(value));
                    }
                    _ => return Err(ParseError { argument }),
                }
            }
            if options.only.is_some() && !options.edits.iter().any(HistoryEdit::is_a_move) {
                return Err(ParseError { argument });
            }
            return Ok(Command::History(options));
        }
        "--save" => {
            let Some(first) = arguments.next() else {
                return Err(ParseError { argument });
            };
            let mut options = SaveOptions::new(vec![PathBuf::from(&first)]);
            let mut rest = arguments;
            while let Some(argument) = rest.next() {
                let flag = argument.to_str().unwrap_or_default();
                // A bare path is another file in the set; every option starts with `--`.
                if !flag.starts_with("--") {
                    options.files.push(PathBuf::from(&argument));
                    continue;
                }
                match flag {
                    "--plan" => options.plan_only = true,
                    "--set-text" | "--set-text-file" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(text) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        options.edits.push(if flag == "--set-text" {
                            DocumentEdit::Text(text.to_owned())
                        } else {
                            DocumentEdit::File(PathBuf::from(text))
                        });
                    }
                    "--save-as" | "--save-copy" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(to) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        options.verb = if flag == "--save-as" {
                            Verb::SaveAs { to: to.to_owned() }
                        } else {
                            Verb::CopyTo { to: to.to_owned() }
                        };
                    }
                    "--root" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(path) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        options.root = Some(PathBuf::from(path));
                    }
                    _ => return Err(ParseError { argument }),
                }
            }
            if !matches!(options.verb, Verb::Save) && options.files.len() != 1 {
                // Save-as and save-a-copy name one destination, so they are one file's business;
                // writing several files elsewhere at once has no meaning. Refused here rather than
                // in the parser so the message can say why, instead of "unexpected argument".
                return Ok(Command::Save(options));
            }
            return Ok(Command::Save(options));
        }
        "--conflict" => {
            let Some(path) = arguments.next() else {
                return Err(ParseError { argument });
            };
            let mut options = ConflictOptions::new(PathBuf::from(path));
            let mut rest = arguments;
            while let Some(argument) = rest.next() {
                let flag = argument.to_str().unwrap_or_default();
                match flag {
                    "--recreate" => options.recreate = true,
                    "--accept" => options.accept = true,
                    "--base" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(path) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        options.base = Some(PathBuf::from(path));
                    }
                    "--set-text" | "--set-text-file" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(text) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        options.edits.push(if flag == "--set-text" {
                            DocumentEdit::Text(text.to_owned())
                        } else {
                            DocumentEdit::File(PathBuf::from(text))
                        });
                    }
                    "--root" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(path) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        options.root = Some(PathBuf::from(path));
                    }
                    _ => return Err(ParseError { argument }),
                }
            }
            return Ok(Command::Conflict(options));
        }
        "--recovery" => {
            let Some(store) = arguments.next() else {
                return Err(ParseError { argument });
            };
            let mut options = RecoveryOptions::new(PathBuf::from(store));
            let mut rest = arguments;
            while let Some(argument) = rest.next() {
                let flag = argument.to_str().unwrap_or_default();
                match flag {
                    "--record" | "--restore" | "--discard" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(path) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        let path = PathBuf::from(path);
                        match flag {
                            "--record" => options.record = Some(path),
                            "--restore" => options.restore = Some(path),
                            _ => options.discard = Some(path),
                        }
                    }
                    "--session" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(name) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        options.session = Some(name.to_owned());
                    }
                    "--prune" => options.prune = true,
                    "--apply" => options.apply = true,
                    "--keep" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(count) =
                            value.to_str().and_then(|value| value.parse::<usize>().ok())
                        else {
                            return Err(ParseError { argument: value });
                        };
                        options.keep = Some(count);
                    }
                    "--root" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(path) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        options.root = Some(PathBuf::from(path));
                    }
                    _ => return Err(ParseError { argument }),
                }
            }
            return Ok(Command::Recovery(options));
        }
        "--close" => {
            let Some(first) = arguments.next() else {
                return Err(ParseError { argument });
            };
            let mut options = CloseOptions::new(vec![PathBuf::from(&first)]);
            let mut rest = arguments;
            while let Some(argument) = rest.next() {
                let flag = argument.to_str().unwrap_or_default();
                if !flag.starts_with("--") {
                    options.files.push(PathBuf::from(&argument));
                    continue;
                }
                match flag {
                    "--unsaved" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(value) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        let Some((document, buffer)) = value.split_once('=') else {
                            return Err(ParseError {
                                argument: value.into(),
                            });
                        };
                        options
                            .unsaved
                            .push((document.to_owned(), PathBuf::from(buffer)));
                    }
                    "--task" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(name) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        options.tasks.push(name.to_owned());
                    }
                    "--decision" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(decision) = value.to_str().and_then(Decision::from_name) else {
                            return Err(ParseError { argument: value });
                        };
                        options.decision = Some(decision);
                    }
                    "--root" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(path) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        options.root = Some(PathBuf::from(path));
                    }
                    _ => return Err(ParseError { argument }),
                }
            }
            return Ok(Command::Close(options));
        }
        "--command" => {
            let mut options = CommandOptions::listing();
            let mut rest = arguments;
            while let Some(argument) = rest.next() {
                let flag = argument.to_str().unwrap_or_default();
                if flag.starts_with("--") {
                    match flag {
                        "--arg" => {
                            let Some(value) = rest.next() else {
                                return Err(ParseError { argument });
                            };
                            let Some(value) = value.to_str() else {
                                return Err(ParseError { argument: value });
                            };
                            let Some((name, value)) = value.split_once('=') else {
                                return Err(ParseError {
                                    argument: value.into(),
                                });
                            };
                            options.arguments.push((name.to_owned(), value.to_owned()));
                        }
                        "--focus" => {
                            let Some(value) = rest.next() else {
                                return Err(ParseError { argument });
                            };
                            let Some(area) = value.to_str() else {
                                return Err(ParseError { argument: value });
                            };
                            options.focus = area.to_owned();
                        }
                        "--trust" => options.trust = true,
                        "--authorize" => {
                            let Some(value) = rest.next() else {
                                return Err(ParseError { argument });
                            };
                            let Some(name) = value.to_str() else {
                                return Err(ParseError { argument: value });
                            };
                            options.authorize.push(name.to_owned());
                        }
                        "--running" => {
                            let Some(value) = rest.next() else {
                                return Err(ParseError { argument });
                            };
                            let Some(name) = value.to_str() else {
                                return Err(ParseError { argument: value });
                            };
                            options.running.push(name.to_owned());
                        }
                        _ => return Err(ParseError { argument }),
                    }
                    continue;
                }
                if options.id.is_some() {
                    return Err(ParseError { argument });
                }
                options.id = Some(flag.to_owned());
            }
            return Ok(Command::Invoke(options));
        }
        "--menu" => return Ok(Command::Menu),
        "--audit" => return Ok(Command::Audit),
        "--keys" => {
            let mut options = KeysOptions::listing();
            let mut rest = arguments;
            while let Some(argument) = rest.next() {
                let flag = argument.to_str().unwrap_or_default();
                match flag {
                    "--press" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(chord) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        options.press = Some(chord.to_owned());
                    }
                    "--mode" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(mode) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        options.mode = mode.to_owned();
                    }
                    "--panel" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(panel) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        options.panel = Some(panel.to_owned());
                    }
                    "--text-field" => options.text_field = true,
                    "--composing" => options.composing = true,
                    _ => return Err(ParseError { argument }),
                }
            }
            return Ok(Command::Keys(options));
        }
        "--palette" => {
            let mut options = PaletteOptions::listing();
            let mut rest = arguments;
            while let Some(argument) = rest.next() {
                let flag = argument.to_str().unwrap_or_default();
                if flag.starts_with("--") {
                    match flag {
                        "--help-for" => {
                            let Some(value) = rest.next() else {
                                return Err(ParseError { argument });
                            };
                            let Some(id) = value.to_str() else {
                                return Err(ParseError { argument: value });
                            };
                            options.help_for = Some(id.to_owned());
                        }
                        "--focus" => {
                            let Some(value) = rest.next() else {
                                return Err(ParseError { argument });
                            };
                            let Some(area) = value.to_str() else {
                                return Err(ParseError { argument: value });
                            };
                            options.focus = area.to_owned();
                        }
                        "--trust" => options.trust = true,
                        "--authorize" | "--running" | "--recent" => {
                            let Some(value) = rest.next() else {
                                return Err(ParseError { argument });
                            };
                            let Some(name) = value.to_str() else {
                                return Err(ParseError { argument: value });
                            };
                            match flag {
                                "--authorize" => options.authorize.push(name.to_owned()),
                                "--running" => options.running.push(name.to_owned()),
                                _ => options.recent.push(name.to_owned()),
                            }
                        }
                        _ => return Err(ParseError { argument }),
                    }
                    continue;
                }
                if options.query.is_empty() {
                    options.query = flag.to_owned();
                } else {
                    options.query.push(' ');
                    options.query.push_str(flag);
                }
            }
            return Ok(Command::Palette(options));
        }
        "--workbench" => {
            let mut options = WorkbenchOptions::new();
            let mut rest = arguments;
            while let Some(argument) = rest.next() {
                let flag = argument.to_str().unwrap_or_default();
                match flag {
                    "--dirty" => options.dirty = true,
                    "--mode" | "--project" | "--target" | "--document" | "--message"
                    | "--focus" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(value) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        match flag {
                            "--mode" => options.mode = Some(value.to_owned()),
                            "--project" => options.project = Some(value.to_owned()),
                            "--target" => options.target = Some(value.to_owned()),
                            "--document" => options.document = Some(value.to_owned()),
                            "--focus" => options.focus = Some(value.to_owned()),
                            _ => options.message = Some(value.to_owned()),
                        }
                    }
                    "--service" | "--show" | "--hide" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(name) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        match flag {
                            "--service" => options.services.push(name.to_owned()),
                            "--show" => options.show.push(name.to_owned()),
                            _ => options.hide.push(name.to_owned()),
                        }
                    }
                    "--resize" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(value) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        let Some((id, size)) = value.split_once('=') else {
                            return Err(ParseError {
                                argument: value.into(),
                            });
                        };
                        let Some(size) = size.parse::<u32>().ok() else {
                            return Err(ParseError {
                                argument: value.into(),
                            });
                        };
                        options.resize.push((id.to_owned(), size));
                    }
                    _ => return Err(ParseError { argument }),
                }
            }
            return Ok(Command::Workbench(options));
        }
        "--operation" => {
            let Some(name) = arguments.next() else {
                return Err(ParseError { argument });
            };
            let Some(name) = name.to_str() else {
                return Err(ParseError { argument: name });
            };
            let mut options = OperationOptions::new(name);
            let mut rest = arguments;
            while let Some(argument) = rest.next() {
                let flag = argument.to_str().unwrap_or_default();
                match flag {
                    "--note" | "--choice" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(value) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        if flag == "--note" {
                            options.note = Some(value.to_owned());
                        } else {
                            options.choice = Some(value.to_owned());
                        }
                    }
                    "--effect" | "--select" | "--authorize" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(value) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        match flag {
                            "--effect" => options.effects.push(value.to_owned()),
                            "--select" => options.select.push(value.to_owned()),
                            _ => options.authorize.push(value.to_owned()),
                        }
                    }
                    "--inapplicable" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(value) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        let Some((id, reason)) = value.split_once('=') else {
                            return Err(ParseError {
                                argument: value.into(),
                            });
                        };
                        options
                            .inapplicable
                            .push((id.to_owned(), reason.to_owned()));
                    }
                    _ => return Err(ParseError { argument }),
                }
            }
            return Ok(Command::Operation(options));
        }
        "--tabs" => {
            let mut options = TabsOptions::new();
            let mut rest = arguments;
            while let Some(argument) = rest.next() {
                let value = argument.to_str().unwrap_or_default();
                if value == "--limit" {
                    let Some(count) = rest.next() else {
                        return Err(ParseError { argument });
                    };
                    let Some(count) = count.to_str().and_then(|value| value.parse::<usize>().ok())
                    else {
                        return Err(ParseError { argument: count });
                    };
                    options.limit = count;
                    continue;
                }
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::Tabs(options));
        }
        "--groups" => {
            let mut options = GroupsOptions::new();
            let mut rest = arguments;
            while let Some(argument) = rest.next() {
                let value = argument.to_str().unwrap_or_default();
                if value == "--limit" {
                    let Some(count) = rest.next() else {
                        return Err(ParseError { argument });
                    };
                    let Some(count) = count.to_str().and_then(|value| value.parse::<usize>().ok())
                    else {
                        return Err(ParseError { argument: count });
                    };
                    options.limit = count;
                    continue;
                }
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::Groups(options));
        }
        "--clipboard" => {
            let mut options = ClipboardOptions::new();
            let mut rest = arguments;
            while let Some(argument) = rest.next() {
                // Owned, so the arm below can move `argument` into a `ParseError` without fighting
                // the borrow of a `&str` derived from it.
                let flag = argument.to_str().unwrap_or_default().to_owned();
                if flag == "--roundtrip" {
                    options.roundtrip = true;
                    continue;
                }
                match flag.as_str() {
                    "--limit" | "--copy-text" | "--copy-resource" | "--copy-node"
                    | "--paste-into" => {
                        let Some(raw) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(text) = raw.to_str() else {
                            return Err(ParseError { argument: raw });
                        };
                        match flag.as_str() {
                            "--limit" => match text.parse::<usize>() {
                                Ok(limit) => options.limit = limit,
                                Err(_) => return Err(ParseError { argument: raw }),
                            },
                            "--copy-text" => options.text = Some(text.to_owned()),
                            "--copy-resource" => match text.split_once(':') {
                                Some((project, path)) => {
                                    options.resource_project = Some(project.to_owned());
                                    options.resource_path = Some(path.to_owned());
                                }
                                None => return Err(ParseError { argument: raw }),
                            },
                            "--copy-node" => match text.split_once('#') {
                                Some((file, id)) => match id.parse::<u64>() {
                                    Ok(id) => {
                                        options.node_file = Some(PathBuf::from(file));
                                        options.node_id = Some(id);
                                    }
                                    Err(_) => return Err(ParseError { argument: raw }),
                                },
                                None => return Err(ParseError { argument: raw }),
                            },
                            _ => options.paste_into = Some(text.to_owned()),
                        }
                    }
                    _ => return Err(ParseError { argument }),
                }
            }
            return Ok(Command::Clipboard(options));
        }
        "--layout" => {
            let mut options = LayoutOptions::new();
            let mut rest = arguments;
            while let Some(argument) = rest.next() {
                let value = argument.to_str().unwrap_or_default();
                if value == "--limit" || value == "--service" {
                    let Some(entry) = rest.next() else {
                        return Err(ParseError { argument });
                    };
                    if value == "--limit" {
                        let Some(count) =
                            entry.to_str().and_then(|value| value.parse::<usize>().ok())
                        else {
                            return Err(ParseError { argument: entry });
                        };
                        options.limit = count;
                    } else {
                        let Some(service) = entry.to_str() else {
                            return Err(ParseError { argument: entry });
                        };
                        options.services.push(service.to_owned());
                    }
                    continue;
                }
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::Layout(options));
        }
        "--windows" => {
            let mut options = WindowsOptions::new();
            let mut rest = arguments;
            while let Some(argument) = rest.next() {
                let flag = argument.to_str().unwrap_or_default().to_owned();
                match flag.as_str() {
                    "--display" => {
                        let Some(raw) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(text) = raw.to_str() else {
                            return Err(ParseError { argument: raw });
                        };
                        match parse_display(text) {
                            Some(display) => options.displays.push(display),
                            None => return Err(ParseError { argument: raw }),
                        }
                    }
                    "--main" => {
                        let Some(raw) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(text) = raw.to_str() else {
                            return Err(ParseError { argument: raw });
                        };
                        match parse_size(text) {
                            Some((width, height)) => {
                                options.main_width = Some(width);
                                options.main_height = Some(height);
                            }
                            None => return Err(ParseError { argument: raw }),
                        }
                    }
                    _ if flag.starts_with("--") => return Err(ParseError { argument }),
                    _ => options.ops.push(flag),
                }
            }
            return Ok(Command::Windows(options));
        }
        "--chrome" => {
            let mut options = ChromeOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::Chrome(options));
        }
        "--text" => {
            let mut options = TextOptions::new();
            let mut rest = arguments;
            while let Some(argument) = rest.next() {
                let value = argument.to_str().unwrap_or_default();
                if value == "--file" {
                    let Some(file) = rest.next() else {
                        return Err(ParseError { argument });
                    };
                    options.file = Some(PathBuf::from(file));
                    continue;
                }
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::Text(options));
        }
        "--highlight" => {
            let mut options = HighlightOptions::new();
            let mut rest = arguments;
            while let Some(argument) = rest.next() {
                let flag = argument.to_str().unwrap_or_default().to_owned();
                match flag.as_str() {
                    "--language" | "--theme" | "--file" => {
                        let Some(raw) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(text) = raw.to_str() else {
                            return Err(ParseError { argument: raw });
                        };
                        match flag.as_str() {
                            "--language" => options.language = Some(text.to_owned()),
                            "--theme" => options.theme = Some(text.to_owned()),
                            _ => options.file = Some(PathBuf::from(text)),
                        }
                    }
                    _ if flag.starts_with("--") => return Err(ParseError { argument }),
                    _ => options.ops.push(flag),
                }
            }
            return Ok(Command::Highlight(options));
        }
        "--coordinates" => {
            let mut options = CoordinatesOptions::new();
            let mut rest = arguments;
            while let Some(argument) = rest.next() {
                let flag = argument.to_str().unwrap_or_default().to_owned();
                match flag.as_str() {
                    "--tab" | "--wrap" | "--file" => {
                        let Some(raw) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(text) = raw.to_str() else {
                            return Err(ParseError { argument: raw });
                        };
                        match flag.as_str() {
                            "--tab" | "--wrap" => {
                                let Some(number) = text.parse::<u32>().ok() else {
                                    return Err(ParseError { argument: raw });
                                };
                                if flag == "--tab" {
                                    options.tab = Some(number);
                                } else {
                                    options.wrap = Some(number);
                                }
                            }
                            _ => options.file = Some(PathBuf::from(text)),
                        }
                    }
                    _ if flag.starts_with("--") => return Err(ParseError { argument }),
                    _ => options.ops.push(flag),
                }
            }
            return Ok(Command::Coordinates(options));
        }
        "--edit-ops" => {
            let mut options = EditOpsOptions::new();
            let mut rest = arguments;
            while let Some(argument) = rest.next() {
                let flag = argument.to_str().unwrap_or_default().to_owned();
                match flag.as_str() {
                    "--file" => {
                        let Some(raw) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(text) = raw.to_str() else {
                            return Err(ParseError { argument: raw });
                        };
                        options.file = Some(PathBuf::from(text));
                    }
                    _ if flag.starts_with("--") => return Err(ParseError { argument }),
                    _ => options.ops.push(flag),
                }
            }
            return Ok(Command::EditOps(options));
        }
        "--editor-view" => {
            let mut options = EditorViewOptions::new();
            let mut rest = arguments;
            while let Some(argument) = rest.next() {
                let flag = argument.to_str().unwrap_or_default().to_owned();
                match flag.as_str() {
                    "--tab" | "--file" => {
                        let Some(raw) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(text) = raw.to_str() else {
                            return Err(ParseError { argument: raw });
                        };
                        if flag == "--tab" {
                            let Some(number) = text.parse::<u32>().ok() else {
                                return Err(ParseError { argument: raw });
                            };
                            options.tab = Some(number);
                        } else {
                            options.file = Some(PathBuf::from(text));
                        }
                    }
                    _ if flag.starts_with("--") => return Err(ParseError { argument }),
                    _ => options.ops.push(flag),
                }
            }
            return Ok(Command::EditorView(options));
        }
        "--write-guard" => {
            let mut options = WriteGuardOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::WriteGuard(options));
        }
        "--scale" => {
            let mut options = ScaleOptions::new();
            let mut rest = arguments;
            while let Some(argument) = rest.next() {
                let flag = argument.to_str().unwrap_or_default().to_owned();
                match flag.as_str() {
                    "--file" | "--chunk" => {
                        let Some(raw) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(text) = raw.to_str() else {
                            return Err(ParseError { argument: raw });
                        };
                        if flag == "--chunk" {
                            let Some(number) = text.parse::<usize>().ok() else {
                                return Err(ParseError { argument: raw });
                            };
                            options.chunk = Some(number);
                        } else {
                            options.file = Some(PathBuf::from(text));
                        }
                    }
                    _ if flag.starts_with("--") => return Err(ParseError { argument }),
                    _ => options.ops.push(flag),
                }
            }
            return Ok(Command::Scale(options));
        }
        "--snippet" => {
            let mut options = SnippetOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::Snippet(options));
        }
        "--regression" => {
            let mut options = RegressionOptions::new();
            let mut rest = arguments;
            while let Some(argument) = rest.next() {
                let flag = argument.to_str().unwrap_or_default().to_owned();
                match flag.as_str() {
                    "--lines" => {
                        let Some(raw) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(text) = raw.to_str() else {
                            return Err(ParseError { argument: raw });
                        };
                        let Some(number) = text.parse::<usize>().ok() else {
                            return Err(ParseError { argument: raw });
                        };
                        options.lines = Some(number);
                    }
                    _ if flag.starts_with("--") => return Err(ParseError { argument }),
                    _ => options.scenarios.push(flag),
                }
            }
            return Ok(Command::Regression(options));
        }
        "--diff" => {
            let mut options = DiffOptions::new();
            let mut rest = arguments;
            while let Some(argument) = rest.next() {
                let flag = argument.to_str().unwrap_or_default().to_owned();
                match flag.as_str() {
                    "--left" | "--right" => {
                        let Some(raw) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(value) = raw.to_str() else {
                            return Err(ParseError { argument: raw });
                        };
                        if flag == "--left" {
                            options.left = Some(PathBuf::from(value));
                        } else {
                            options.right = Some(PathBuf::from(value));
                        }
                    }
                    "--ignore-endings" => options.ignore_endings = true,
                    "--ignore-trailing" => options.ignore_trailing = true,
                    _ if flag.starts_with("--") => return Err(ParseError { argument }),
                    _ => options.ops.push(flag),
                }
            }
            return Ok(Command::Diff(options));
        }
        "--structural" => {
            let mut options = StructuralOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::Structural(options));
        }
        "--language" => {
            let mut options = LanguageOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::Language(options));
        }
        "--protocol" => {
            let mut options = ProtocolOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::Protocol(options));
        }
        "--features" => {
            let mut options = FeaturesOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::Features(options));
        }
        "--navigation" => {
            let mut options = NavigationOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::Navigation(options));
        }
        "--diagnostics" => {
            let mut options = DiagnosticsOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::Diagnostics(options));
        }
        "--workspace-edit" => {
            let mut options = WorkspaceEditOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::WorkspaceEdit(options));
        }
        "--schema" => {
            let mut options = SchemaOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::Schema(options));
        }
        "--health" => {
            let mut options = HealthOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.scenarios.push(value.to_owned());
            }
            return Ok(Command::Health(options));
        }
        "--hints" => {
            let mut options = HintsOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::Hints(options));
        }
        "--analysis" => {
            let mut options = AnalysisOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::Analysis(options));
        }
        "--index" => {
            let mut options = IndexOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::Index(options));
        }
        "--search" => {
            let mut options = SearchOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::Search(options));
        }
        "--quick-open" => {
            let mut options = QuickOpenOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::QuickOpen(options));
        }
        "--replace" => {
            let mut options = ReplaceOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::Replace(options));
        }
        "--batch" => {
            let mut options = BatchOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::Batch(options));
        }
        "--history-nav" => {
            let mut options = HistoryNavOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::HistoryNav(options));
        }
        "--search-health" => {
            let mut options = SearchHealthOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.scenarios.push(value.to_owned());
            }
            return Ok(Command::SearchHealth(options));
        }
        "--catalog" => {
            let mut options = CatalogOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::Catalog(options));
        }
        "--scene" => {
            let mut options = SceneOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::Scene(options));
        }
        "--structure" => {
            let mut options = StructureOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::Structure(options));
        }
        "--viewport" => {
            let mut options = ViewportOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::Viewport(options));
        }
        "--drag" => {
            let mut options = DragOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::Drag(options));
        }
        "--objects" => {
            let mut options = ObjectsOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::Objects(options));
        }
        "--design" => {
            let mut options = DesignOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::Design(options));
        }
        "--layout-plan" => {
            let mut options = LayoutPlanOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::LayoutPlan(options));
        }
        "--layout-switch" => {
            let mut options = LayoutSwitchOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::LayoutSwitch(options));
        }
        "--constraints" => {
            let mut options = ConstraintsOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::Constraints(options));
        }
        "--arrange" => {
            let mut options = ArrangeOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::Arrange(options));
        }
        "--frames" => {
            let mut options = FramesOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::Frames(options));
        }
        "--focus-order" => {
            let mut options = FocusOrderOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::FocusOrder(options));
        }
        "--variants" => {
            let mut options = VariantsOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::Variants(options));
        }
        "--layout-probe" => {
            let mut options = LayoutProbeOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::LayoutProbe(options));
        }
        "--property-editor" => {
            let mut options = PropertyEditorOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::PropertyEditor(options));
        }
        "--multi-property" => {
            let mut options = MultiPropertyOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::MultiProperty(options));
        }
        "--css" => {
            let mut options = CssOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::Css(options));
        }
        "--tokens" => {
            let mut options = TokensOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::Tokens(options));
        }
        "--states" => {
            let mut options = StatesOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::States(options));
        }
        "--motion" => {
            let mut options = MotionOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::Motion(options));
        }
        "--style" => {
            let mut options = StyleOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::Style(options));
        }
        "--style-boundary" => {
            let mut options = StyleBoundaryOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::StyleBoundary(options));
        }
        "--data" => {
            let mut options = DataOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::Data(options));
        }
        "--binding" => {
            let mut options = BindingOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::Binding(options));
        }
        "--events" => {
            let mut options = EventsOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::Events(options));
        }
        "--dispatch" => {
            let mut options = DispatchOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::Dispatch(options));
        }
        "--routes" => {
            let mut options = RoutesOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::Routes(options));
        }
        "--machine" => {
            let mut options = MachineOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::Machine(options));
        }
        "--adapter" => {
            let mut options = AdapterOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::Adapter(options));
        }
        "--session-log" => {
            let mut options = SessionLogOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::SessionLog(options));
        }
        "--preview" => {
            let mut options = PreviewOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::Preview(options));
        }
        "--hot-update" => {
            let mut options = HotUpdateOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::HotUpdate(options));
        }
        "--state-keep" => {
            let mut options = StateKeepOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::StateKeep(options));
        }
        "--matrix" => {
            let mut options = MatrixOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::Matrix(options));
        }
        "--inspect" => {
            let mut options = InspectOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::Inspect(options));
        }
        "--visual" => {
            let mut options = VisualOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::Visual(options));
        }
        "--preview-recovery" => {
            let mut options = PreviewRecoveryOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::PreviewRecovery(options));
        }
        "--load" => {
            let mut options = LoadOptions::new();
            for argument in arguments {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Err(ParseError { argument });
                }
                options.ops.push(value.to_owned());
            }
            return Ok(Command::Load(options));
        }
        "--focus" => {
            let Some(target) = arguments.next() else {
                return Err(ParseError { argument });
            };
            let Some(target) = target.to_str() else {
                return Err(ParseError { argument: target });
            };
            let mut options = FocusOptions::new(target);
            let mut rest = arguments;
            while let Some(argument) = rest.next() {
                let flag = argument.to_str().unwrap_or_default();
                match flag {
                    "--trust" => options.trust = true,
                    "--mode" | "--press" | "--menu" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(value) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        match flag {
                            "--mode" => options.mode = value.to_owned(),
                            "--press" => options.press = Some(value.to_owned()),
                            _ => options.menu = Some(value.to_owned()),
                        }
                    }
                    "--authorize" | "--running" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(value) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        if flag == "--authorize" {
                            options.authorize.push(value.to_owned());
                        } else {
                            options.running.push(value.to_owned());
                        }
                    }
                    _ => return Err(ParseError { argument }),
                }
            }
            return Ok(Command::Focus(options));
        }
        "--document" => {
            let Some(path) = arguments.next() else {
                return Err(ParseError { argument });
            };
            let mut options = DocumentOptions::new(PathBuf::from(path));
            let mut rest = arguments;
            while let Some(argument) = rest.next() {
                match argument.to_str().unwrap_or_default() {
                    "--root" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(path) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        options.root = Some(PathBuf::from(path));
                    }
                    "--set-text" | "--set-text-file" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(text) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        if argument.to_str().unwrap_or_default() == "--set-text" {
                            options.edits.push(DocumentEdit::Text(text.to_owned()));
                        } else {
                            options.edits.push(DocumentEdit::File(PathBuf::from(text)));
                        }
                    }
                    _ => return Err(ParseError { argument }),
                }
            }
            return Ok(Command::Document(options));
        }
        "--new-workspace" => {
            let Some(directory) = arguments.next() else {
                return Err(ParseError { argument });
            };
            let mut options = NewWorkspaceOptions::new(PathBuf::from(directory));
            let mut rest = arguments;
            while let Some(argument) = rest.next() {
                match argument.to_str().unwrap_or_default() {
                    "--name" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(name) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        options.name = Some(name.to_owned());
                    }
                    _ => return Err(ParseError { argument }),
                }
            }
            return Ok(Command::NewWorkspace(options));
        }
        "--workspace" => {
            let Some(path) = arguments.next() else {
                return Err(ParseError { argument });
            };
            let mut options = WorkspaceOptions::new(PathBuf::from(path), WorkspaceAction::Report);
            let mut rest = arguments;
            while let Some(argument) = rest.next() {
                let flag = argument.to_str().unwrap_or_default();
                match flag {
                    "--add" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(pair) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        let Some((name, root)) = pair.split_once('=') else {
                            return Err(ParseError { argument: value });
                        };
                        if !name_is_addressable(name) || root.trim().is_empty() {
                            return Err(ParseError { argument: value });
                        }
                        options.action = WorkspaceAction::Add {
                            name: name.to_owned(),
                            root: root.to_owned(),
                        };
                    }
                    "--remove" | "--activate" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(name) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        if !name_is_addressable(name) {
                            return Err(ParseError { argument: value });
                        }
                        options.action = if flag == "--remove" {
                            WorkspaceAction::Remove {
                                name: name.to_owned(),
                            }
                        } else {
                            WorkspaceAction::Activate {
                                name: name.to_owned(),
                            }
                        };
                    }
                    "--file" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(name) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        if WorkspaceFile::parse(name).is_err() {
                            return Err(ParseError { argument: value });
                        }
                        options.action = WorkspaceAction::File {
                            name: name.to_owned(),
                        };
                    }
                    "--apply" => options.apply = true,
                    "--trust" => options.trust = true,
                    _ => return Err(ParseError { argument }),
                }
            }
            return Ok(Command::Workspace(options));
        }
        "--validate" => {
            let Some(path) = arguments.next() else {
                return Err(ParseError { argument });
            };
            let mut options = ValidateOptions::new(PathBuf::from(path));
            let mut rest = arguments;
            while let Some(argument) = rest.next() {
                match argument.to_str().unwrap_or_default() {
                    "--diagnose" => options.diagnose = true,
                    "--trust" => options.trust = true,
                    "--record" | "--rescan" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(path) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        if argument.to_str().unwrap_or_default() == "--record" {
                            options.record = Some(PathBuf::from(path));
                        } else {
                            options.rescan = Some(PathBuf::from(path));
                        }
                    }
                    _ => return Err(ParseError { argument }),
                }
            }
            return Ok(Command::Validate(options));
        }
        "--configuration" => {
            let Some(path) = arguments.next() else {
                return Err(ParseError { argument });
            };
            let mut options = ConfigurationOptions::new(PathBuf::from(path));
            let mut rest = arguments;
            while let Some(argument) = rest.next() {
                match argument.to_str().unwrap_or_default() {
                    "--target" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(target) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        options.target = Some(target.trim().to_owned());
                    }
                    "--override" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(word) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        let Some((key, value)) = word.split_once('=') else {
                            return Err(ParseError { argument: value });
                        };
                        if key.trim().is_empty() {
                            return Err(ParseError {
                                argument: word.into(),
                            });
                        }
                        // One spelling on the command line, two shapes in the model: a setting that
                        // holds a list takes a comma-separated one, and an empty word means the empty
                        // list rather than a missing value.
                        let value = if key.trim() == "features" || value.contains(',') {
                            SettingValue::list(value.split(','))
                        } else {
                            SettingValue::Word(value.trim().to_owned())
                        };
                        options.overrides.push(OverrideWord {
                            key: key.trim().to_owned(),
                            value,
                        });
                    }
                    "--remember" => options.remember = true,
                    "--forget" => options.forget = true,
                    _ => return Err(ParseError { argument }),
                }
            }
            return Ok(Command::Configuration(options));
        }
        "--import" => {
            let Some(path) = arguments.next() else {
                return Err(ParseError { argument });
            };
            let mut options = ImportOptions::new(PathBuf::from(path));
            for argument in arguments {
                match argument.to_str().unwrap_or_default() {
                    "--apply" => options.apply = true,
                    "--check-backup" => options.check_backup = true,
                    "--trust" => options.trust = true,
                    _ => return Err(ParseError { argument }),
                }
            }
            if options.apply && options.check_backup {
                return Err(ParseError { argument });
            }
            return Ok(Command::Import(options));
        }
        "--render-icons" | "--probe-apis" | "--probe-host" | "--project-check" => {
            let Some(value) = arguments.next() else {
                return Err(ParseError { argument });
            };
            let mut rest = arguments;
            if let Some(argument) = rest.next() {
                return Err(ParseError { argument });
            }
            let path = PathBuf::from(value);
            return Ok(match text {
                "--render-icons" => Command::RenderIcons(path),
                "--probe-apis" => Command::ProbeApis(path),
                "--project-check" => Command::ProjectCheck(path),
                _ => Command::ProbeHost(path),
            });
        }
        _ => return Err(ParseError { argument }),
    };

    if let Some(argument) = arguments.next() {
        return Err(ParseError { argument });
    }

    Ok(command)
}

pub fn help_text() -> &'static str {
    concat!(
        "rustage - Rust desktop UI designer\n",
        "\n",
        "     Usage: rustage [--help | --version | --launch [--exit-after-ms <ms>]]\n",
        "            rustage [--render-icons <directory> | --probe-apis <directory>]\n",
        "            rustage [--probe-host <directory> | --project-check <file>]\n",
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
        "     Global options:\n",
        "       --log-level <level>           error | warn | info | debug | trace (default: info)\n",
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        AdapterOptions, AnalysisOptions, ArrangeOptions, BatchOptions, BindingOptions,
        CatalogOptions, ChromeOptions, ClipboardOptions, Command, ConfigurationOptions,
        ConstraintsOptions, CoordinatesOptions, CssOptions, DataOptions, DependenciesOptions,
        DesignOptions, DiagnosticsOptions, DiffOptions, DispatchOptions, DisplayArg, DocumentEdit,
        DocumentOptions, DragOptions, DropEntryOptions, EditOpsOptions, EditOptions,
        EditorViewOptions, EventsOptions, FeaturesOptions, FileOpOptions, FocusOrderOptions,
        FramesOptions, GroupsOptions, HealthOptions, HighlightOptions, HintsOptions, HistoryDomain,
        HistoryEdit, HistoryNavOptions, HistoryOptions, HotUpdateOptions, ImportOptions,
        IndexOptions, InspectOptions, KitOptions, LanguageOptions, LaunchOptions, LayoutOptions,
        LayoutPlanOptions, LayoutProbeOptions, LayoutSwitchOptions, LoadOptions, MachineOptions,
        MatrixOptions, MotionOptions, MultiPropertyOptions, NavigationOptions, NewProjectOptions,
        NewWorkspaceOptions, ObjectsOptions, OpenProjectOptions, PreviewOptions,
        PreviewRecoveryOptions, PropertyEditorOptions, ProtocolOptions, QuickOpenOptions,
        RegressionOptions, ReplaceOptions, RoutesOptions, ScaleOptions, SceneOptions,
        SchemaOptions, SearchHealthOptions, SearchOptions, SessionLogOptions, SnippetOptions,
        StateKeepOptions, StatesOptions, StructuralOptions, StructureOptions, StyleBoundaryOptions,
        StyleOptions, TextOptions, TokensOptions, ValidateOptions, VariantsOptions,
        ViewportOptions, VisualOptions, WindowsOptions, WorkspaceAction, WorkspaceEditOptions,
        WorkspaceOptions, WriteGuardOptions, help_text, log_level_of, parse,
    };
    use crate::host::log::Level;
    use crate::project::{configuration::SettingValue, create::Template, files::FileOperation};
    use std::time::Duration;

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
    fn parses_the_icon_export_directory() {
        assert_eq!(
            parse(["--render-icons", "out/icons"]),
            Ok(Command::RenderIcons(PathBuf::from("out/icons")))
        );
    }

    #[test]
    fn parses_the_probe_report_directory() {
        assert_eq!(
            parse(["--probe-apis", "out/probe"]),
            Ok(Command::ProbeApis(PathBuf::from("out/probe")))
        );
        assert_eq!(
            parse(["--probe-host", "out/host"]),
            Ok(Command::ProbeHost(PathBuf::from("out/host")))
        );
        assert_eq!(
            parse(["--project-check", "demo/project.json"]),
            Ok(Command::ProjectCheck(PathBuf::from("demo/project.json")))
        );
    }

    #[test]
    fn parses_the_new_project_options() {
        assert_eq!(
            parse(["--new-project", "demo"]),
            Ok(Command::NewProject(NewProjectOptions::new(
                PathBuf::from("demo"),
                Template::BasicPage
            )))
        );
        assert_eq!(
            parse([
                "--new-project",
                "demo",
                "--template",
                "application",
                "--name",
                "My App"
            ]),
            Ok(Command::NewProject(NewProjectOptions {
                directory: PathBuf::from("demo"),
                name: Some("My App".to_owned()),
                template: Template::Application,
            }))
        );

        // A template this build does not have, and a flag nobody knows, are usage errors.
        assert!(parse(["--new-project", "demo", "--template", "nowhere"]).is_err());
        assert!(parse(["--new-project"]).is_err());
        assert!(parse(["--new-project", "demo", "--template"]).is_err());
        assert!(parse(["--new-project", "demo", "--wat"]).is_err());
    }

    #[test]
    fn parses_the_open_project_options() {
        assert_eq!(
            parse(["--open-project", "demo/project.json"]),
            Ok(Command::OpenProject(OpenProjectOptions::new(
                PathBuf::from("demo/project.json")
            )))
        );
        assert_eq!(
            parse([
                "--open-project",
                "demo/project.json",
                "--trust",
                "--diagnose"
            ]),
            Ok(Command::OpenProject(OpenProjectOptions {
                path: PathBuf::from("demo/project.json"),
                trust: true,
                diagnose: true,
            }))
        );
        assert_eq!(parse(["--recent"]), Ok(Command::Recent));
        assert!(parse(["--open-project"]).is_err());
        assert!(parse(["--open-project", "demo/project.json", "--wat"]).is_err());
        assert!(parse(["--recent", "extra"]).is_err());
    }

    #[test]
    fn parses_the_project_tree_and_the_entry_removal() {
        assert_eq!(
            parse(["--project-tree", "demo/project.json"]),
            Ok(Command::ProjectTree(PathBuf::from("demo/project.json")))
        );
        assert_eq!(
            parse([
                "--drop-entry",
                "demo/project.json",
                "--document",
                "pages/main.json"
            ]),
            Ok(Command::DropEntry(DropEntryOptions::new(
                PathBuf::from("demo/project.json"),
                "pages/main.json"
            )))
        );
        assert_eq!(
            parse([
                "--drop-entry",
                "demo/project.json",
                "--document",
                "pages/main.json",
                "--fix",
                "--trust"
            ]),
            Ok(Command::DropEntry(DropEntryOptions {
                path: PathBuf::from("demo/project.json"),
                document: "pages/main.json".to_owned(),
                fix: true,
                trust: true,
            }))
        );
        assert!(parse(["--project-tree"]).is_err());
        assert!(parse(["--drop-entry", "demo/project.json"]).is_err());
        assert!(parse(["--drop-entry", "demo/project.json", "--wat"]).is_err());
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
    fn the_log_level_option_is_read_and_removed_from_the_command() {
        assert_eq!(log_level_of(["--help"]).unwrap(), None);
        assert_eq!(
            log_level_of(["--log-level", "debug", "--launch"]).unwrap(),
            Some(Level::Debug)
        );
        assert_eq!(
            parse(["--log-level", "warn", "--launch"]),
            Ok(Command::Launch(LaunchOptions::default()))
        );
        assert_eq!(
            parse(["--launch", "--log-level", "trace", "--exit-after-ms", "10"]),
            Ok(Command::Launch(LaunchOptions {
                exit_after: Duration::from_millis(10)
            })),
        );

        // A missing or unknown level is a usage error, not a silent fallback to the default.
        assert!(log_level_of(["--log-level"]).is_err());
        assert!(log_level_of(["--log-level", "shout"]).is_err());
        assert!(parse(["--log-level", "shout", "--help"]).is_err());
    }

    #[test]
    fn parses_the_import_options() {
        assert_eq!(
            parse(["--import", "demo/project.json"]),
            Ok(Command::Import(ImportOptions::new(PathBuf::from(
                "demo/project.json"
            ))))
        );
        assert_eq!(
            parse(["\u{2d}\u{2d}import", "demo", "--apply", "--trust"]),
            Ok(Command::Import(ImportOptions {
                path: PathBuf::from("demo"),
                apply: true,
                check_backup: false,
                trust: true,
            }))
        );
        assert_eq!(
            parse(["\u{2d}\u{2d}import", "demo", "--check-backup"]),
            Ok(Command::Import(ImportOptions {
                path: PathBuf::from("demo"),
                apply: false,
                check_backup: true,
                trust: false,
            }))
        );

        // A missing path, an unknown flag, and asking to migrate and to only report at the same time
        // are usage errors: none of them can be read as "do nothing", which would look like success.
        assert!(parse(["\u{2d}\u{2d}import"]).is_err());
        assert!(parse(["\u{2d}\u{2d}import", "demo", "--wat"]).is_err());
        assert!(parse(["\u{2d}\u{2d}import", "demo", "--apply", "--check-backup"]).is_err());
    }

    #[test]
    fn parses_the_edit_options() {
        assert_eq!(
            parse(["--edit", "pages/main.json"]),
            Ok(Command::Edit(EditOptions::new(PathBuf::from(
                "pages/main.json"
            ))))
        );
        assert_eq!(
            parse([
                "--edit",
                "pages/main.json",
                "--batch",
                "batch.json",
                "--root",
                "area",
                "--apply",
                "--undo",
            ]),
            Ok(Command::Edit(EditOptions {
                file: PathBuf::from("pages/main.json"),
                batch: Some(PathBuf::from("batch.json")),
                root: Some(PathBuf::from("area")),
                apply: true,
                undo: true,
            }))
        );
        assert!(parse(["--edit"]).is_err());
        assert!(parse(["--edit", "pages/main.json", "--wat"]).is_err());
        assert!(parse(["--edit", "pages/main.json", "--batch"]).is_err());
        assert!(parse(["--edit", "pages/main.json", "--undo"]).is_err());
    }

    #[test]
    fn parses_the_document_options() {
        assert_eq!(
            parse(["--document", "pages/main.json"]),
            Ok(Command::Document(DocumentOptions::new(PathBuf::from(
                "pages/main.json"
            ))))
        );
        assert_eq!(
            parse([
                "--document",
                "pages/main.json",
                "--root",
                "area",
                "--set-text",
                "{}",
                "--set-text-file",
                "typed.json",
                "--set-text",
                "{ nope",
            ]),
            Ok(Command::Document(DocumentOptions {
                file: PathBuf::from("pages/main.json"),
                root: Some(PathBuf::from("area")),
                edits: vec![
                    DocumentEdit::Text("{}".to_owned()),
                    DocumentEdit::File(PathBuf::from("typed.json")),
                    DocumentEdit::Text("{ nope".to_owned()),
                ],
            }))
        );
        assert!(parse(["--document"]).is_err());
        assert!(parse(["--document", "pages/main.json", "--wat"]).is_err());
        assert!(parse(["--document", "pages/main.json", "--root"]).is_err());
        assert!(parse(["--document", "pages/main.json", "--set-text"]).is_err());
        assert!(parse(["--document", "pages/main.json", "--set-text-file"]).is_err());
    }

    #[test]
    fn parses_the_history_options() {
        assert_eq!(
            parse(["--history", "pages/main.json"]),
            Ok(Command::History(HistoryOptions::new(PathBuf::from(
                "pages/main.json"
            ))))
        );
        // The script's order is the order of the flags: edits and moves are one sequence, because
        // what happens after an undo is a new branch.
        assert_eq!(
            parse([
                "--history",
                "pages/main.json",
                "--root",
                "area",
                "--text",
                "{}",
                "--text-file",
                "typed.json",
                "--batch",
                "batch.json",
                "--merge",
                "--undo",
                "1",
                "--redo",
                "2",
                "--only",
                "model",
            ]),
            Ok(Command::History(HistoryOptions {
                file: PathBuf::from("pages/main.json"),
                root: Some(PathBuf::from("area")),
                edits: vec![
                    HistoryEdit::Text("{}".to_owned()),
                    HistoryEdit::File(PathBuf::from("typed.json")),
                    HistoryEdit::Batch(PathBuf::from("batch.json")),
                    HistoryEdit::Undo(1),
                    HistoryEdit::Redo(2),
                ],
                merge: true,
                only: Some(HistoryDomain::Model),
            }))
        );
        assert!(parse(["--history"]).is_err());
        assert!(parse(["--history", "pages/main.json", "--wat"]).is_err());
        assert!(parse(["--history", "pages/main.json", "--text"]).is_err());
        assert!(parse(["--history", "pages/main.json", "--batch"]).is_err());
        assert!(parse(["--history", "pages/main.json", "--undo"]).is_err());
        assert!(parse(["--history", "pages/main.json", "--undo", "many"]).is_err());
        assert!(parse(["--history", "pages/main.json", "--only", "both"]).is_err());
        // A domain with nothing to move in it is a usage error rather than a run that ignores it.
        assert!(parse(["--history", "pages/main.json", "--only", "text"]).is_err());
        assert!(
            parse([
                "--history",
                "pages/main.json",
                "--only",
                "text",
                "--undo",
                "0"
            ])
            .is_ok()
        );
    }

    #[test]
    fn parses_the_dependency_options() {
        assert_eq!(
            parse(["--dependencies", "area"]),
            Ok(Command::Dependencies(DependenciesOptions::new(
                PathBuf::from("area")
            )))
        );
        assert_eq!(
            parse([
                "--dependencies",
                "area",
                "--review",
                "area/deps.json",
                "--record",
                "area/now.json",
                "--allow-lockfile-change",
                "--allow-network",
                "--trust",
            ]),
            Ok(Command::Dependencies(DependenciesOptions {
                directory: PathBuf::from("area"),
                review: Some(PathBuf::from("area/deps.json")),
                record: Some(PathBuf::from("area/now.json")),
                allow_lockfile_change: true,
                allow_network: true,
                trust: true,
            }))
        );
        assert!(parse(["--dependencies"]).is_err());
        assert!(parse(["--dependencies", "area", "--wat"]).is_err());
        assert!(parse(["--dependencies", "area", "--record"]).is_err());
        assert!(parse(["--dependencies", "area", "--review"]).is_err());
        // Neither flag is spelled in a way that could be mistaken for the other:
        assert!(parse(["--dependencies", "area", "--lockfile"]).is_err());
    }

    #[test]
    fn parses_the_kit_options() {
        assert_eq!(
            parse(["--kit", "area/kit.json"]),
            Ok(Command::Kit(KitOptions::new(PathBuf::from(
                "area/kit.json"
            ))))
        );
        assert_eq!(
            parse([
                "--kit",
                "area/kit.json",
                "--diagnose",
                "--trust",
                "--triple",
                "x86_64-unknown-linux-gnu",
            ]),
            Ok(Command::Kit(KitOptions {
                path: PathBuf::from("area/kit.json"),
                diagnose: true,
                trust: true,
                triple: Some("x86_64-unknown-linux-gnu".to_owned()),
            }))
        );

        // A missing path, an unknown flag, an empty triple and a value that is not there are usage
        // errors: none of them can be read as "it is fine", which is what a silent default would be.
        assert!(parse(["--kit"]).is_err());
        assert!(parse(["--kit", "area/kit.json", "--wat"]).is_err());
        assert!(parse(["--kit", "area/kit.json", "--triple", "  "]).is_err());
        assert!(parse(["--kit", "area/kit.json", "--triple"]).is_err());
    }

    #[test]
    fn parses_the_workspace_options() {
        assert_eq!(
            parse(["--workspace", "area/workspace.json"]),
            Ok(Command::Workspace(WorkspaceOptions::new(
                PathBuf::from("area/workspace.json"),
                WorkspaceAction::Report
            )))
        );
        assert_eq!(
            parse([
                "\u{2d}\u{2d}workspace",
                "area/workspace.json",
                "--add",
                "app=members/app",
                "--apply",
                "--trust"
            ]),
            Ok(Command::Workspace(WorkspaceOptions {
                path: PathBuf::from("area/workspace.json"),
                action: WorkspaceAction::Add {
                    name: "app".to_owned(),
                    root: "members/app".to_owned(),
                },
                apply: true,
                trust: true,
            }))
        );
        assert_eq!(
            parse([
                "\u{2d}\u{2d}workspace",
                "area/workspace.json",
                "--remove",
                "lib"
            ]),
            Ok(Command::Workspace(WorkspaceOptions::new(
                PathBuf::from("area/workspace.json"),
                WorkspaceAction::Remove {
                    name: "lib".to_owned(),
                }
            )))
        );
        assert_eq!(
            parse([
                "\u{2d}\u{2d}workspace",
                "area/workspace.json",
                "--activate",
                "lib"
            ]),
            Ok(Command::Workspace(WorkspaceOptions::new(
                PathBuf::from("area/workspace.json"),
                WorkspaceAction::Activate {
                    name: "lib".to_owned(),
                }
            )))
        );
        assert_eq!(
            parse([
                "\u{2d}\u{2d}workspace",
                "area/workspace.json",
                "--file",
                "lib:pages/main.json"
            ]),
            Ok(Command::Workspace(WorkspaceOptions::new(
                PathBuf::from("area/workspace.json"),
                WorkspaceAction::File {
                    name: "lib:pages/main.json".to_owned(),
                }
            )))
        );
        assert_eq!(
            parse(["\u{2d}\u{2d}new-workspace", "area", "--name", "Area"]),
            Ok(Command::NewWorkspace(NewWorkspaceOptions {
                directory: PathBuf::from("area"),
                name: Some("Area".to_owned()),
            }))
        );

        // A name that cannot be addressed, a pair with no `=`, a path with no member, and a missing
        // value are all usage errors: none of them can be read as "do nothing", which looks like
        // success.
        assert!(
            parse([
                "\u{2d}\u{2d}workspace",
                "area/workspace.json",
                "--add",
                "app"
            ])
            .is_err()
        );
        assert!(
            parse([
                "\u{2d}\u{2d}workspace",
                "area/workspace.json",
                "--add",
                "=members/app"
            ])
            .is_err()
        );
        assert!(
            parse([
                "\u{2d}\u{2d}workspace",
                "area/workspace.json",
                "--add",
                "two members=x"
            ])
            .is_err()
        );
        assert!(parse(["\u{2d}\u{2d}workspace", "area/workspace.json", "--remove"]).is_err());
        assert!(
            parse([
                "\u{2d}\u{2d}workspace",
                "area/workspace.json",
                "--file",
                "pages/main.json"
            ])
            .is_err()
        );
        assert!(parse(["\u{2d}\u{2d}workspace"]).is_err());
        assert!(parse(["\u{2d}\u{2d}new-workspace"]).is_err());
    }

    #[test]
    fn parses_the_validate_options() {
        assert_eq!(
            parse(["--validate", "demo/project.json"]),
            Ok(Command::Validate(ValidateOptions::new(PathBuf::from(
                "demo/project.json"
            ))))
        );
        assert_eq!(
            parse([
                "--validate",
                "demo/project.json",
                "--diagnose",
                "--trust",
                "--record",
                "out/scan.json",
            ]),
            Ok(Command::Validate(ValidateOptions {
                path: PathBuf::from("demo/project.json"),
                diagnose: true,
                trust: true,
                record: Some(PathBuf::from("out/scan.json")),
                rescan: None,
            }))
        );
        assert_eq!(
            parse(["--validate", "demo", "--rescan", "out/scan.json"]),
            Ok(Command::Validate(ValidateOptions {
                path: PathBuf::from("demo"),
                diagnose: false,
                trust: false,
                record: None,
                rescan: Some(PathBuf::from("out/scan.json")),
            }))
        );

        // A missing path, an unknown flag, and a flag whose value is not there are usage errors:
        // none of them can be read as "check it and change nothing", which would look like success.
        assert!(parse(["--validate"]).is_err());
        assert!(parse(["--validate", "demo", "--wat"]).is_err());
        assert!(parse(["--validate", "demo", "--record"]).is_err());
        assert!(parse(["--validate", "demo", "--rescan"]).is_err());
    }

    #[test]
    fn parses_the_configuration_options() {
        assert_eq!(
            parse(["--configuration", "demo/project.json"]),
            Ok(Command::Configuration(ConfigurationOptions::new(
                PathBuf::from("demo/project.json")
            )))
        );
        assert_eq!(
            parse([
                "--configuration",
                "demo/project.json",
                "--target",
                "release",
                "--override",
                "profile=release",
                "--override",
                "features=desktop,gpu",
                "--remember"
            ]),
            Ok(Command::Configuration(ConfigurationOptions {
                path: PathBuf::from("demo/project.json"),
                target: Some("release".to_owned()),
                overrides: vec![
                    super::OverrideWord {
                        key: "profile".to_owned(),
                        value: SettingValue::Word("release".to_owned()),
                    },
                    super::OverrideWord {
                        key: "features".to_owned(),
                        value: SettingValue::List(vec!["desktop".to_owned(), "gpu".to_owned()]),
                    },
                ],
                remember: true,
                forget: false,
            }))
        );
        assert_eq!(
            parse(["--configuration", "p.json", "--forget"]),
            Ok(Command::Configuration(ConfigurationOptions {
                path: PathBuf::from("p.json"),
                target: None,
                overrides: Vec::new(),
                remember: false,
                forget: true,
            }))
        );

        // A missing path, a missing value, an override without `=` and an unknown flag are usage
        // errors: none of them can be read as "record nothing", which would look like success.
        assert!(parse(["--configuration"]).is_err());
        assert!(parse(["--configuration", "p.json", "--target"]).is_err());
        assert!(parse(["--configuration", "p.json", "--override"]).is_err());
        assert!(parse(["--configuration", "p.json", "--override", "profile"]).is_err());
        assert!(parse(["--configuration", "p.json", "--override", "=release"]).is_err());
        assert!(parse(["--configuration", "p.json", "--wat"]).is_err());
    }

    #[test]
    fn parses_the_file_operation_options() {
        assert_eq!(
            parse(["--file-op", "demo/project.json", "--add", "assets/logo.svg"]),
            Ok(Command::FileOp(FileOpOptions::new(
                PathBuf::from("demo/project.json"),
                FileOperation::Add {
                    path: "assets/logo.svg".to_owned()
                }
            )))
        );
        assert_eq!(
            parse(["\u{2d}\u{2d}file-op", "p.json", "--x"]),
            Err(super::ParseError {
                argument: std::ffi::OsString::from("--x")
            })
        );

        // `--move` takes two paths; `--rename` is the same operation under the name a person uses
        // when the file does not change places.
        for spelling in ["--move", "--rename"] {
            assert_eq!(
                parse(["--file-op", "p.json", spelling, "a.svg", "b.svg"]),
                Ok(Command::FileOp(FileOpOptions::new(
                    PathBuf::from("p.json"),
                    FileOperation::Move {
                        from: "a.svg".to_owned(),
                        to: "b.svg".to_owned()
                    }
                )))
            );
        }
        assert_eq!(
            parse([
                "--file-op",
                "p.json",
                "--delete",
                "a.svg",
                "--apply",
                "--trust"
            ]),
            Ok(Command::FileOp(FileOpOptions {
                path: PathBuf::from("p.json"),
                operation: FileOperation::Delete {
                    path: "a.svg".to_owned()
                },
                apply: true,
                trust: true,
            }))
        );

        // A missing path, a missing operation, a half-given move and an unknown flag are usage
        // errors: none of them can be read as "do nothing", which would look like success.
        assert!(parse(["--file-op"]).is_err());
        assert!(parse(["--file-op", "p.json"]).is_err());
        assert!(parse(["--file-op", "p.json", "--add"]).is_err());
        assert!(parse(["--file-op", "p.json", "--move", "a.svg"]).is_err());
        assert!(parse(["--file-op", "p.json", "--move", "a.svg", "b.svg", "--wat"]).is_err());
    }

    #[test]
    fn parses_the_editor_group_operations() {
        assert_eq!(
            parse(["--groups"]),
            Ok(Command::Groups(GroupsOptions::new())),
        );
        assert_eq!(
            parse([
                "--groups",
                "--limit",
                "3",
                "split:left-right",
                "view:pages/a.json"
            ]),
            Ok(Command::Groups(GroupsOptions {
                limit: 3,
                ops: vec![
                    "split:left-right".to_owned(),
                    "view:pages/a.json".to_owned()
                ],
            })),
        );

        // A limit that is missing or unparsable is a usage error, and an unknown flag is not an op.
        assert!(parse(["--groups", "--limit"]).is_err());
        assert!(parse(["--groups", "--limit", "many"]).is_err());
        assert!(parse(["--groups", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_clipboard_options() {
        assert_eq!(
            parse(["--clipboard"]),
            Ok(Command::Clipboard(ClipboardOptions::new())),
        );

        let mut expected = ClipboardOptions::new();
        expected.limit = 32;
        expected.text = Some("hi".to_owned());
        expected.paste_into = Some("Beta".to_owned());
        expected.roundtrip = true;
        assert_eq!(
            parse([
                "--clipboard",
                "--limit",
                "32",
                "--copy-text",
                "hi",
                "--paste-into",
                "Beta",
                "--roundtrip"
            ]),
            Ok(Command::Clipboard(expected)),
        );

        let Ok(Command::Clipboard(resource)) =
            parse(["--clipboard", "--copy-resource", "Alpha:assets/logo.svg"])
        else {
            panic!("expected a clipboard resource copy");
        };
        assert_eq!(resource.resource_project.as_deref(), Some("Alpha"));
        assert_eq!(resource.resource_path.as_deref(), Some("assets/logo.svg"));

        let Ok(Command::Clipboard(node)) =
            parse(["--clipboard", "--copy-node", "pages/home.json#3"])
        else {
            panic!("expected a clipboard node copy");
        };
        assert_eq!(node.node_file, Some(PathBuf::from("pages/home.json")));
        assert_eq!(node.node_id, Some(3));

        // A limit that is missing or unparsable, and a malformed resource or node, are usage errors.
        assert!(parse(["--clipboard", "--limit", "x"]).is_err());
        assert!(parse(["--clipboard", "--limit"]).is_err());
        assert!(parse(["--clipboard", "--copy-resource", "nocolon"]).is_err());
        assert!(parse(["--clipboard", "--copy-node", "no-hash"]).is_err());
        assert!(parse(["--clipboard", "--huh"]).is_err());
    }

    #[test]
    fn parses_the_layout_operations() {
        assert_eq!(
            parse(["--layout"]),
            Ok(Command::Layout(LayoutOptions::new()))
        );
        assert_eq!(
            parse([
                "--layout",
                "--limit",
                "3",
                "--service",
                "document-tree",
                "preset:columns",
                "zen:on"
            ]),
            Ok(Command::Layout(LayoutOptions {
                limit: 3,
                services: vec!["document-tree".to_owned()],
                ops: vec!["preset:columns".to_owned(), "zen:on".to_owned()],
            })),
        );

        assert!(parse(["--layout", "--limit"]).is_err());
        assert!(parse(["--layout", "--limit", "many"]).is_err());
        assert!(parse(["--layout", "--service"]).is_err());
        assert!(parse(["--layout", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_window_operations() {
        assert_eq!(
            parse(["--windows"]),
            Ok(Command::Windows(WindowsOptions::new())),
        );

        let mut expected = WindowsOptions::new();
        expected.displays.push(DisplayArg {
            id: 0,
            x: 0,
            y: 0,
            width: 1280,
            height: 800,
            scale_milli: 1000,
        });
        expected.main_width = Some(900);
        expected.main_height = Some(700);
        expected.ops = vec![
            "view:pages/a.json".to_owned(),
            "unplug:1".to_owned(),
            "reconcile".to_owned(),
        ];
        assert_eq!(
            parse([
                "--windows",
                "--display",
                "0:0,0,1280,800,1.0",
                "--main",
                "900x700",
                "view:pages/a.json",
                "unplug:1",
                "reconcile"
            ]),
            Ok(Command::Windows(expected)),
        );

        // A scale is kept in thousandths, so a fractional scale survives.
        let Ok(Command::Windows(scaled)) = parse(["--windows", "--display", "2:0,0,800,600,1.5"])
        else {
            panic!("expected a windows command");
        };
        assert_eq!(scaled.displays[0].scale_milli, 1500);
        assert_eq!(scaled.displays[0].width, 800);

        // A missing, malformed or zero-scaled display, and a malformed size, are usage errors.
        assert!(parse(["--windows", "--display"]).is_err());
        assert!(parse(["--windows", "--display", "bad"]).is_err());
        assert!(parse(["--windows", "--display", "0:0,0,1280,800,0"]).is_err());
        assert!(parse(["--windows", "--display", "0:0,0,1280,800"]).is_err());
        assert!(parse(["--windows", "--main", "900"]).is_err());
        assert!(parse(["--windows", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_chrome_operations() {
        assert_eq!(
            parse(["--chrome"]),
            Ok(Command::Chrome(ChromeOptions::new()))
        );
        assert_eq!(
            parse([
                "--chrome",
                "focus:editor",
                "notice:error:build.run:the linker failed",
                "status"
            ]),
            Ok(Command::Chrome(ChromeOptions {
                ops: vec![
                    "focus:editor".to_owned(),
                    "notice:error:build.run:the linker failed".to_owned(),
                    "status".to_owned(),
                ],
            })),
        );

        assert!(parse(["--chrome", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_in_place_text_operations() {
        assert_eq!(parse(["--text"]), Ok(Command::Text(TextOptions::new())));
        let mut expected = TextOptions::new();
        expected.file = Some(PathBuf::from("pages/home.json"));
        expected.ops = vec!["splice:0:0:x".to_owned(), "undo".to_owned()];
        assert_eq!(
            parse([
                "--text",
                "--file",
                "pages/home.json",
                "splice:0:0:x",
                "undo"
            ]),
            Ok(Command::Text(expected)),
        );

        assert!(parse(["--text", "--file"]).is_err());
        assert!(parse(["--text", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_highlight_operations() {
        assert_eq!(
            parse(["--highlight"]),
            Ok(Command::Highlight(HighlightOptions::new())),
        );
        let mut expected = HighlightOptions::new();
        expected.language = Some("rust".to_owned());
        expected.theme = Some("light".to_owned());
        expected.ops = vec!["line:let x = 1;".to_owned(), "step:1".to_owned()];
        assert_eq!(
            parse([
                "--highlight",
                "--language",
                "rust",
                "--theme",
                "light",
                "line:let x = 1;",
                "step:1"
            ]),
            Ok(Command::Highlight(expected)),
        );

        assert!(parse(["--highlight", "--language"]).is_err());
        assert!(parse(["--highlight", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_coordinate_operations() {
        assert_eq!(
            parse(["--coordinates"]),
            Ok(Command::Coordinates(CoordinatesOptions::new())),
        );
        let mut expected = CoordinatesOptions::new();
        expected.tab = Some(8);
        expected.wrap = Some(80);
        expected.ops = vec![
            "line:\tlet x = 1;".to_owned(),
            "visual:5".to_owned(),
            "mark:cursor:3:3".to_owned(),
        ];
        assert_eq!(
            parse([
                "--coordinates",
                "--tab",
                "8",
                "--wrap",
                "80",
                "line:\tlet x = 1;",
                "visual:5",
                "mark:cursor:3:3"
            ]),
            Ok(Command::Coordinates(expected)),
        );

        assert!(parse(["--coordinates", "--tab"]).is_err());
        assert!(parse(["--coordinates", "--tab", "wide"]).is_err());
        assert!(parse(["--coordinates", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_editing_operations() {
        assert_eq!(
            parse(["--edit-ops"]),
            Ok(Command::EditOps(EditOpsOptions::new())),
        );
        let mut expected = EditOpsOptions::new();
        expected.file = Some(PathBuf::from("main.rs"));
        expected.ops = vec![
            "carets:1:3".to_owned(),
            "insert:-".to_owned(),
            "undo".to_owned(),
        ];
        assert_eq!(
            parse([
                "--edit-ops",
                "--file",
                "main.rs",
                "carets:1:3",
                "insert:-",
                "undo"
            ]),
            Ok(Command::EditOps(expected)),
        );

        assert!(parse(["--edit-ops", "--file"]).is_err());
        assert!(parse(["--edit-ops", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_editor_view_operations() {
        assert_eq!(
            parse(["--editor-view"]),
            Ok(Command::EditorView(EditorViewOptions::new())),
        );
        let mut expected = EditorViewOptions::new();
        expected.tab = Some(2);
        expected.file = Some(PathBuf::from("main.rs"));
        expected.ops = vec![
            "whitespace".to_owned(),
            "fold:indent".to_owned(),
            "minimap:4".to_owned(),
        ];
        assert_eq!(
            parse([
                "--editor-view",
                "--tab",
                "2",
                "--file",
                "main.rs",
                "whitespace",
                "fold:indent",
                "minimap:4"
            ]),
            Ok(Command::EditorView(expected)),
        );

        assert!(parse(["--editor-view", "--file"]).is_err());
        assert!(parse(["--editor-view", "--tab", "wide"]).is_err());
        assert!(parse(["--editor-view", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_write_guard_operations() {
        assert_eq!(
            parse(["--write-guard"]),
            Ok(Command::WriteGuard(WriteGuardOptions::new())),
        );
        let mut expected = WriteGuardOptions::new();
        expected.ops = vec![
            "protect:4:10".to_owned(),
            "input:6:1".to_owned(),
            "read-only".to_owned(),
            "show".to_owned(),
        ];
        assert_eq!(
            parse([
                "--write-guard",
                "protect:4:10",
                "input:6:1",
                "read-only",
                "show"
            ]),
            Ok(Command::WriteGuard(expected)),
        );

        assert!(parse(["--write-guard", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_scale_operations() {
        assert_eq!(parse(["--scale"]), Ok(Command::Scale(ScaleOptions::new())),);
        let mut expected = ScaleOptions::new();
        expected.file = Some(PathBuf::from("big.txt"));
        expected.chunk = Some(4096);
        expected.ops = vec!["read".to_owned(), "show".to_owned()];
        assert_eq!(
            parse([
                "--scale", "--file", "big.txt", "--chunk", "4096", "read", "show"
            ]),
            Ok(Command::Scale(expected)),
        );

        assert!(parse(["--scale", "--file"]).is_err());
        assert!(parse(["--scale", "--chunk", "lots"]).is_err());
        assert!(parse(["--scale", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_snippet_operations() {
        assert_eq!(
            parse(["--snippet"]),
            Ok(Command::Snippet(SnippetOptions::new())),
        );
        let mut expected = SnippetOptions::new();
        expected.ops = vec![
            "snippet:fn:fn ${1:name}".to_owned(),
            "expand:fn".to_owned(),
            "tab".to_owned(),
            "show".to_owned(),
        ];
        assert_eq!(
            parse([
                "--snippet",
                "snippet:fn:fn ${1:name}",
                "expand:fn",
                "tab",
                "show"
            ]),
            Ok(Command::Snippet(expected)),
        );
        assert!(parse(["--snippet", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_regression_operations() {
        assert_eq!(
            parse(["--regression"]),
            Ok(Command::Regression(RegressionOptions::new())),
        );
        let mut expected = RegressionOptions::new();
        expected.lines = Some(1000);
        expected.scenarios = vec!["normal".to_owned(), "unicode".to_owned()];
        assert_eq!(
            parse(["--regression", "--lines", "1000", "normal", "unicode"]),
            Ok(Command::Regression(expected)),
        );
        assert!(parse(["--regression", "--lines"]).is_err());
        assert!(parse(["--regression", "--lines", "lots"]).is_err());
        assert!(parse(["--regression", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_diff_operations() {
        assert_eq!(parse(["--diff"]), Ok(Command::Diff(DiffOptions::new())));
        let mut expected = DiffOptions::new();
        expected.left = Some(PathBuf::from("a.txt"));
        expected.right = Some(PathBuf::from("b.txt"));
        expected.ignore_endings = true;
        expected.ops = vec!["show".to_owned(), "trees".to_owned()];
        assert_eq!(
            parse([
                "--diff",
                "--left",
                "a.txt",
                "--right",
                "b.txt",
                "--ignore-endings",
                "show",
                "trees"
            ]),
            Ok(Command::Diff(expected)),
        );
        assert!(parse(["--diff", "--left"]).is_err());
        assert!(parse(["--diff", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_structural_operations() {
        assert_eq!(
            parse(["--structural"]),
            Ok(Command::Structural(StructuralOptions::new())),
        );
        let mut expected = StructuralOptions::new();
        expected.ops = vec![
            "text:hello".to_owned(),
            "cursor:2".to_owned(),
            "expand:3".to_owned(),
            "toggle:inline-diff".to_owned(),
        ];
        assert_eq!(
            parse([
                "--structural",
                "text:hello",
                "cursor:2",
                "expand:3",
                "toggle:inline-diff"
            ]),
            Ok(Command::Structural(expected)),
        );
        assert!(parse(["--structural", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_language_operations() {
        assert_eq!(
            parse(["--language"]),
            Ok(Command::Language(LanguageOptions::new())),
        );
        let mut expected = LanguageOptions::new();
        expected.ops = vec![
            "caps:completion,hover".to_owned(),
            "view:app".to_owned(),
            "restart:app".to_owned(),
        ];
        assert_eq!(
            parse([
                "--language",
                "caps:completion,hover",
                "view:app",
                "restart:app"
            ]),
            Ok(Command::Language(expected)),
        );
        assert!(parse(["--language", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_protocol_operations() {
        assert_eq!(
            parse(["--protocol"]),
            Ok(Command::Protocol(ProtocolOptions::new())),
        );
        let mut expected = ProtocolOptions::new();
        expected.ops = vec![
            "open:file:///a.rs=中a".to_owned(),
            "edit:file:///a.rs:1:0,0:0,1=字".to_owned(),
            "units:file:///a.rs:4".to_owned(),
        ];
        assert_eq!(
            parse([
                "--protocol",
                "open:file:///a.rs=中a",
                "edit:file:///a.rs:1:0,0:0,1=字",
                "units:file:///a.rs:4"
            ]),
            Ok(Command::Protocol(expected)),
        );
        assert!(parse(["--protocol", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_features_operations() {
        assert_eq!(
            parse(["--features"]),
            Ok(Command::Features(FeaturesOptions::new())),
        );
        let mut expected = FeaturesOptions::new();
        expected.ops = vec![
            "version:1".to_owned(),
            "request".to_owned(),
            "respond:0:1:alpha,beta".to_owned(),
        ];
        assert_eq!(
            parse([
                "--features",
                "version:1",
                "request",
                "respond:0:1:alpha,beta"
            ]),
            Ok(Command::Features(expected)),
        );
        assert!(parse(["--features", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_navigation_operations() {
        assert_eq!(
            parse(["--navigation"]),
            Ok(Command::Navigation(NavigationOptions::new())),
        );
        let mut expected = NavigationOptions::new();
        expected.ops = vec![
            "caps:definition,references".to_owned(),
            "symbol:file:///a.rs:foo:function:0:10:0:3".to_owned(),
            "definition:foo".to_owned(),
        ];
        assert_eq!(
            parse([
                "--navigation",
                "caps:definition,references",
                "symbol:file:///a.rs:foo:function:0:10:0:3",
                "definition:foo"
            ]),
            Ok(Command::Navigation(expected)),
        );
        assert!(parse(["--navigation", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_diagnostics_operations() {
        assert_eq!(
            parse(["--diagnostics"]),
            Ok(Command::Diagnostics(DiagnosticsOptions::new())),
        );
        let mut expected = DiagnosticsOptions::new();
        expected.ops = vec![
            "diag:error:compiler:0:3:oops".to_owned(),
            "publish:1".to_owned(),
        ];
        assert_eq!(
            parse(["--diagnostics", "diag:error:compiler:0:3:oops", "publish:1"]),
            Ok(Command::Diagnostics(expected)),
        );
        assert!(parse(["--diagnostics", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_workspace_edit_operations() {
        assert_eq!(
            parse(["--workspace-edit"]),
            Ok(Command::WorkspaceEdit(WorkspaceEditOptions::new())),
        );
        let mut expected = WorkspaceEditOptions::new();
        expected.ops = vec![
            "label:rename".to_owned(),
            "edit:a.rs:1:0:1:X".to_owned(),
            "commit".to_owned(),
        ];
        assert_eq!(
            parse([
                "--workspace-edit",
                "label:rename",
                "edit:a.rs:1:0:1:X",
                "commit"
            ]),
            Ok(Command::WorkspaceEdit(expected)),
        );
        assert!(parse(["--workspace-edit", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_schema_operations() {
        assert_eq!(
            parse(["--schema"]),
            Ok(Command::Schema(SchemaOptions::new()))
        );
        let mut expected = SchemaOptions::new();
        expected.ops = vec![
            "lang:css".to_owned(),
            "entry:color:-:text colour".to_owned(),
            "target:embedded:".to_owned(),
            "complete:c".to_owned(),
        ];
        assert_eq!(
            parse([
                "--schema",
                "lang:css",
                "entry:color:-:text colour",
                "target:embedded:",
                "complete:c"
            ]),
            Ok(Command::Schema(expected)),
        );
        assert!(parse(["--schema", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_health_operations() {
        assert_eq!(
            parse(["--health"]),
            Ok(Command::Health(HealthOptions::new()))
        );
        let mut expected = HealthOptions::new();
        expected.scenarios = vec!["crash".to_owned(), "slow".to_owned()];
        assert_eq!(
            parse(["--health", "crash", "slow"]),
            Ok(Command::Health(expected)),
        );
        assert!(parse(["--health", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_hints_operations() {
        assert_eq!(parse(["--hints"]), Ok(Command::Hints(HintsOptions::new())));
        let mut expected = HintsOptions::new();
        expected.ops = vec![
            "inlay:2:2:: i32".to_owned(),
            "invoke:showReferences".to_owned(),
        ];
        assert_eq!(
            parse(["--hints", "inlay:2:2:: i32", "invoke:showReferences"]),
            Ok(Command::Hints(expected)),
        );
        assert!(parse(["--hints", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_analysis_operations() {
        assert_eq!(
            parse(["--analysis"]),
            Ok(Command::Analysis(AnalysisOptions::new())),
        );
        let mut expected = AnalysisOptions::new();
        expected.ops = vec![
            "target:wasm32-unknown-unknown".to_owned(),
            "trust:untrusted".to_owned(),
            "configure".to_owned(),
        ];
        assert_eq!(
            parse([
                "--analysis",
                "target:wasm32-unknown-unknown",
                "trust:untrusted",
                "configure"
            ]),
            Ok(Command::Analysis(expected)),
        );
        assert!(parse(["--analysis", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_index_operations() {
        assert_eq!(parse(["--index"]), Ok(Command::Index(IndexOptions::new())));
        let mut expected = IndexOptions::new();
        expected.ops = vec![
            "node:src/main.rs:file:100".to_owned(),
            "node:loop:symlink:..".to_owned(),
            "build".to_owned(),
        ];
        assert_eq!(
            parse([
                "--index",
                "node:src/main.rs:file:100",
                "node:loop:symlink:..",
                "build"
            ]),
            Ok(Command::Index(expected)),
        );
        assert!(parse(["--index", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_search_operations() {
        assert_eq!(
            parse(["--search"]),
            Ok(Command::Search(SearchOptions::new()))
        );
        let mut expected = SearchOptions::new();
        expected.ops = vec![
            "pattern:foo".to_owned(),
            "no-case".to_owned(),
            "search".to_owned(),
        ];
        assert_eq!(
            parse(["--search", "pattern:foo", "no-case", "search"]),
            Ok(Command::Search(expected)),
        );
        assert!(parse(["--search", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_quick_open_operations() {
        assert_eq!(
            parse(["--quick-open"]),
            Ok(Command::QuickOpen(QuickOpenOptions::new())),
        );
        let mut expected = QuickOpenOptions::new();
        expected.ops = vec!["file:src/main.rs".to_owned(), "open:main".to_owned()];
        assert_eq!(
            parse(["--quick-open", "file:src/main.rs", "open:main"]),
            Ok(Command::QuickOpen(expected)),
        );
        assert!(parse(["--quick-open", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_replace_operations() {
        assert_eq!(
            parse(["--replace"]),
            Ok(Command::Replace(ReplaceOptions::new())),
        );
        let mut expected = ReplaceOptions::new();
        expected.ops = vec![
            "pattern:x".to_owned(),
            "template:N".to_owned(),
            "build".to_owned(),
        ];
        assert_eq!(
            parse(["--replace", "pattern:x", "template:N", "build"]),
            Ok(Command::Replace(expected)),
        );
        assert!(parse(["--replace", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_batch_operations() {
        assert_eq!(parse(["--batch"]), Ok(Command::Batch(BatchOptions::new())));
        let mut expected = BatchOptions::new();
        expected.ops = vec![
            "open:a.rs:1=aaa".to_owned(),
            "edit:a.rs:1:0:1:X".to_owned(),
            "run".to_owned(),
        ];
        assert_eq!(
            parse(["--batch", "open:a.rs:1=aaa", "edit:a.rs:1:0:1:X", "run"]),
            Ok(Command::Batch(expected)),
        );
        assert!(parse(["--batch", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_history_nav_operations() {
        assert_eq!(
            parse(["--history-nav"]),
            Ok(Command::HistoryNav(HistoryNavOptions::new())),
        );
        let mut expected = HistoryNavOptions::new();
        expected.ops = vec![
            "visit:a.rs:0".to_owned(),
            "back".to_owned(),
            "resolve".to_owned(),
        ];
        assert_eq!(
            parse(["--history-nav", "visit:a.rs:0", "back", "resolve"]),
            Ok(Command::HistoryNav(expected)),
        );
        assert!(parse(["--history-nav", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_search_health_operations() {
        assert_eq!(
            parse(["--search-health"]),
            Ok(Command::SearchHealth(SearchHealthOptions::new())),
        );
        let mut expected = SearchHealthOptions::new();
        expected.scenarios = vec!["conflict".to_owned(), "cancel".to_owned()];
        assert_eq!(
            parse(["--search-health", "conflict", "cancel"]),
            Ok(Command::SearchHealth(expected)),
        );
        assert!(parse(["--search-health", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_catalog_operations() {
        assert_eq!(
            parse(["--catalog"]),
            Ok(Command::Catalog(CatalogOptions::new())),
        );
        let mut expected = CatalogOptions::new();
        expected.ops = vec![
            "widget:button:Button:controls".to_owned(),
            "property:button:label:text".to_owned(),
            "search:b".to_owned(),
        ];
        assert_eq!(
            parse([
                "--catalog",
                "widget:button:Button:controls",
                "property:button:label:text",
                "search:b"
            ]),
            Ok(Command::Catalog(expected)),
        );
        assert!(parse(["--catalog", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_scene_operations() {
        assert_eq!(parse(["--scene"]), Ok(Command::Scene(SceneOptions::new())));
        let mut expected = SceneOptions::new();
        expected.ops = vec!["add:1:0,0,10,10".to_owned(), "hit:5,5".to_owned()];
        assert_eq!(
            parse(["--scene", "add:1:0,0,10,10", "hit:5,5"]),
            Ok(Command::Scene(expected)),
        );
        assert!(parse(["--scene", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_structure_operations() {
        assert_eq!(
            parse(["--structure"]),
            Ok(Command::Structure(StructureOptions::new())),
        );
        let mut expected = StructureOptions::new();
        expected.ops = vec![
            "root:container:panel".to_owned(),
            "insert:0:widget:button".to_owned(),
            "show".to_owned(),
        ];
        assert_eq!(
            parse([
                "--structure",
                "root:container:panel",
                "insert:0:widget:button",
                "show"
            ]),
            Ok(Command::Structure(expected)),
        );
        assert!(parse(["--structure", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_viewport_operations() {
        assert_eq!(
            parse(["--viewport"]),
            Ok(Command::Viewport(ViewportOptions::new())),
        );
        let mut expected = ViewportOptions::new();
        expected.ops = vec![
            "fit:0,0,100,100:800,600".to_owned(),
            "zoom-at:2:10,10".to_owned(),
            "overlay:800,600".to_owned(),
        ];
        assert_eq!(
            parse([
                "--viewport",
                "fit:0,0,100,100:800,600",
                "zoom-at:2:10,10",
                "overlay:800,600"
            ]),
            Ok(Command::Viewport(expected)),
        );
        assert!(parse(["--viewport", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_drag_operations() {
        assert_eq!(parse(["--drag"]), Ok(Command::Drag(DragOptions::new())));
        let mut expected = DragOptions::new();
        expected.ops = vec![
            "root:container:panel".to_owned(),
            "begin:0".to_owned(),
            "aim:-:0".to_owned(),
            "release".to_owned(),
        ];
        assert_eq!(
            parse([
                "--drag",
                "root:container:panel",
                "begin:0",
                "aim:-:0",
                "release"
            ]),
            Ok(Command::Drag(expected)),
        );
        assert!(parse(["--drag", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_objects_operations() {
        assert_eq!(
            parse(["--objects"]),
            Ok(Command::Objects(ObjectsOptions::new())),
        );
        let mut expected = ObjectsOptions::new();
        expected.ops = vec![
            "node:1:button:visible=false".to_owned(),
            "hide:1".to_owned(),
            "filter".to_owned(),
        ];
        assert_eq!(
            parse([
                "--objects",
                "node:1:button:visible=false",
                "hide:1",
                "filter"
            ]),
            Ok(Command::Objects(expected)),
        );
        assert!(parse(["--objects", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_design_operations() {
        assert_eq!(
            parse(["--design"]),
            Ok(Command::Design(DesignOptions::new())),
        );
        let mut expected = DesignOptions::new();
        expected.ops = vec![
            "node:-:container:panel".to_owned(),
            "all".to_owned(),
            "press:Ctrl+Z".to_owned(),
        ];
        assert_eq!(
            parse(["--design", "node:-:container:panel", "all", "press:Ctrl+Z"]),
            Ok(Command::Design(expected)),
        );
        assert!(parse(["--design", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_layout_plan_operations() {
        assert_eq!(
            parse(["--layout-plan"]),
            Ok(Command::LayoutPlan(LayoutPlanOptions::new())),
        );
        let mut expected = LayoutPlanOptions::new();
        expected.ops = vec![
            "available:1:400,300".to_owned(),
            "constraint:1:width:explicit".to_owned(),
            "run:preview".to_owned(),
        ];
        assert_eq!(
            parse([
                "--layout-plan",
                "available:1:400,300",
                "constraint:1:width:explicit",
                "run:preview"
            ]),
            Ok(Command::LayoutPlan(expected)),
        );
        assert!(parse(["--layout-plan", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_layout_switch_operations() {
        assert_eq!(
            parse(["--layout-switch"]),
            Ok(Command::LayoutSwitch(LayoutSwitchOptions::new())),
        );
        let mut expected = LayoutSwitchOptions::new();
        expected.ops = vec![
            "layout:row".to_owned(),
            "child:1:x".to_owned(),
            "switch".to_owned(),
        ];
        assert_eq!(
            parse(["--layout-switch", "layout:row", "child:1:x", "switch"]),
            Ok(Command::LayoutSwitch(expected)),
        );
        assert!(parse(["--layout-switch", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_constraint_operations() {
        assert_eq!(
            parse(["--constraints"]),
            Ok(Command::Constraints(ConstraintsOptions::new())),
        );
        let mut expected = ConstraintsOptions::new();
        expected.ops = vec![
            "min:1:10,10".to_owned(),
            "max:1:100,100".to_owned(),
            "validate".to_owned(),
        ];
        assert_eq!(
            parse(["--constraints", "min:1:10,10", "max:1:100,100", "validate"]),
            Ok(Command::Constraints(expected)),
        );
        assert!(parse(["--constraints", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_arrange_operations() {
        assert_eq!(
            parse(["--arrange"]),
            Ok(Command::Arrange(ArrangeOptions::new())),
        );
        let mut expected = ArrangeOptions::new();
        expected.ops = vec![
            "layout:row".to_owned(),
            "select:1".to_owned(),
            "align:vertical:centre".to_owned(),
        ];
        assert_eq!(
            parse([
                "--arrange",
                "layout:row",
                "select:1",
                "align:vertical:centre"
            ]),
            Ok(Command::Arrange(expected)),
        );
        assert!(parse(["--arrange", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_frames_operations() {
        assert_eq!(
            parse(["--frames"]),
            Ok(Command::Frames(FramesOptions::new()))
        );
        let mut expected = FramesOptions::new();
        expected.ops = vec![
            "support-size:390,844".to_owned(),
            "frame:phone:390,844:portrait".to_owned(),
            "check".to_owned(),
        ];
        assert_eq!(
            parse([
                "--frames",
                "support-size:390,844",
                "frame:phone:390,844:portrait",
                "check"
            ]),
            Ok(Command::Frames(expected)),
        );
        assert!(parse(["--frames", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_focus_order_operations() {
        assert_eq!(
            parse(["--focus-order"]),
            Ok(Command::FocusOrder(FocusOrderOptions::new())),
        );
        let mut expected = FocusOrderOptions::new();
        expected.ops = vec![
            "root:container:panel".to_owned(),
            "focusable:1".to_owned(),
            "step:1:1".to_owned(),
        ];
        assert_eq!(
            parse([
                "--focus-order",
                "root:container:panel",
                "focusable:1",
                "step:1:1"
            ]),
            Ok(Command::FocusOrder(expected)),
        );
        assert!(parse(["--focus-order", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_variants_operations() {
        assert_eq!(
            parse(["--variants"]),
            Ok(Command::Variants(VariantsOptions::new())),
        );
        let mut expected = VariantsOptions::new();
        expected.ops = vec![
            "capability:min-width".to_owned(),
            "variant:wide".to_owned(),
            "when:min-width=600".to_owned(),
        ];
        assert_eq!(
            parse([
                "--variants",
                "capability:min-width",
                "variant:wide",
                "when:min-width=600"
            ]),
            Ok(Command::Variants(expected)),
        );
        assert!(parse(["--variants", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_layout_probe_operations() {
        assert_eq!(
            parse(["--layout-probe"]),
            Ok(Command::LayoutProbe(LayoutProbeOptions::new())),
        );
        let mut expected = LayoutProbeOptions::new();
        expected.ops = vec![
            "scenario:nested".to_owned(),
            "available:1:400,300".to_owned(),
            "compare".to_owned(),
        ];
        assert_eq!(
            parse([
                "--layout-probe",
                "scenario:nested",
                "available:1:400,300",
                "compare"
            ]),
            Ok(Command::LayoutProbe(expected)),
        );
        assert!(parse(["--layout-probe", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_property_editor_operations() {
        assert_eq!(
            parse(["--property-editor"]),
            Ok(Command::PropertyEditor(PropertyEditorOptions::new())),
        );
        let mut expected = PropertyEditorOptions::new();
        expected.ops = vec![
            "schema:width:number:0:100".to_owned(),
            "set:1:width:120".to_owned(),
            "value:1:width".to_owned(),
        ];
        assert_eq!(
            parse([
                "--property-editor",
                "schema:width:number:0:100",
                "set:1:width:120",
                "value:1:width"
            ]),
            Ok(Command::PropertyEditor(expected)),
        );
        assert!(parse(["--property-editor", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_multi_property_operations() {
        assert_eq!(
            parse(["--multi-property"]),
            Ok(Command::MultiProperty(MultiPropertyOptions::new())),
        );
        let mut expected = MultiPropertyOptions::new();
        expected.ops = vec![
            "node:1".to_owned(),
            "set:width:120:explicit".to_owned(),
            "common".to_owned(),
        ];
        assert_eq!(
            parse([
                "--multi-property",
                "node:1",
                "set:width:120:explicit",
                "common"
            ]),
            Ok(Command::MultiProperty(expected)),
        );
        assert!(parse(["--multi-property", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_css_operations() {
        assert_eq!(parse(["--css"]), Ok(Command::Css(CssOptions::new())),);
        let mut expected = CssOptions::new();
        expected.ops = vec![
            "css:.a { color: red }".to_owned(),
            "element:button.a".to_owned(),
            "value:color".to_owned(),
        ];
        assert_eq!(
            parse([
                "--css",
                "css:.a { color: red }",
                "element:button.a",
                "value:color"
            ]),
            Ok(Command::Css(expected)),
        );
        assert!(parse(["--css", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_tokens_operations() {
        assert_eq!(
            parse(["--tokens"]),
            Ok(Command::Tokens(TokensOptions::new())),
        );
        let mut expected = TokensOptions::new();
        expected.ops = vec![
            "token:colour:--accent:#3b82f6".to_owned(),
            "use:color:var(--accent)".to_owned(),
            "expand".to_owned(),
        ];
        assert_eq!(
            parse([
                "--tokens",
                "token:colour:--accent:#3b82f6",
                "use:color:var(--accent)",
                "expand"
            ]),
            Ok(Command::Tokens(expected)),
        );
        assert!(parse(["--tokens", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_states_operations() {
        assert_eq!(
            parse(["--states"]),
            Ok(Command::States(StatesOptions::new())),
        );
        let mut expected = StatesOptions::new();
        expected.ops = vec![
            "css:button:hover { color: blue }".to_owned(),
            "on:hover".to_owned(),
            "preview".to_owned(),
        ];
        assert_eq!(
            parse([
                "--states",
                "css:button:hover { color: blue }",
                "on:hover",
                "preview"
            ]),
            Ok(Command::States(expected)),
        );
        assert!(parse(["--states", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_motion_operations() {
        assert_eq!(
            parse(["--motion"]),
            Ok(Command::Motion(MotionOptions::new())),
        );
        let mut expected = MotionOptions::new();
        expected.ops = vec![
            "transitions:on".to_owned(),
            "property:color".to_owned(),
            "transition:color:200:0:linear".to_owned(),
            "check".to_owned(),
        ];
        assert_eq!(
            parse([
                "--motion",
                "transitions:on",
                "property:color",
                "transition:color:200:0:linear",
                "check"
            ]),
            Ok(Command::Motion(expected)),
        );
        assert!(parse(["--motion", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_style_operations() {
        assert_eq!(parse(["--style"]), Ok(Command::Style(StyleOptions::new())),);
        let mut expected = StyleOptions::new();
        expected.ops = vec![
            "css:button { color: #3b82f6 }".to_owned(),
            "inline:gap:8".to_owned(),
            "generate".to_owned(),
        ];
        assert_eq!(
            parse([
                "--style",
                "css:button { color: #3b82f6 }",
                "inline:gap:8",
                "generate"
            ]),
            Ok(Command::Style(expected)),
        );
        assert!(parse(["--style", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_style_boundary_operations() {
        assert_eq!(
            parse(["--style-boundary"]),
            Ok(Command::StyleBoundary(StyleBoundaryOptions::new())),
        );
        let mut expected = StyleBoundaryOptions::new();
        expected.ops = vec![
            "inline:color:var(--accent)".to_owned(),
            "undo".to_owned(),
            "emit".to_owned(),
        ];
        assert_eq!(
            parse([
                "--style-boundary",
                "inline:color:var(--accent)",
                "undo",
                "emit"
            ]),
            Ok(Command::StyleBoundary(expected)),
        );
        assert!(parse(["--style-boundary", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_data_operations() {
        assert_eq!(parse(["--data"]), Ok(Command::Data(DataOptions::new())),);
        let mut expected = DataOptions::new();
        expected.ops = vec![
            "schema:user:3".to_owned(),
            "field:name:text:required".to_owned(),
            "active".to_owned(),
        ];
        assert_eq!(
            parse([
                "--data",
                "schema:user:3",
                "field:name:text:required",
                "active"
            ]),
            Ok(Command::Data(expected)),
        );
        assert!(parse(["--data", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_binding_operations() {
        assert_eq!(
            parse(["--binding"]),
            Ok(Command::Binding(BindingOptions::new())),
        );
        let mut expected = BindingOptions::new();
        expected.ops = vec![
            "field:count:integer".to_owned(),
            "slot:1.width:number".to_owned(),
            "bind:f:count=1.width:one-way:identity".to_owned(),
            "check".to_owned(),
        ];
        assert_eq!(
            parse([
                "--binding",
                "field:count:integer",
                "slot:1.width:number",
                "bind:f:count=1.width:one-way:identity",
                "check"
            ]),
            Ok(Command::Binding(expected)),
        );
        assert!(parse(["--binding", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_events_operations() {
        assert_eq!(
            parse(["--events"]),
            Ok(Command::Events(EventsOptions::new())),
        );
        let mut expected = EventsOptions::new();
        expected.ops = vec![
            "event:pressed".to_owned(),
            "field:pressed:count:integer:required".to_owned(),
            "attach:1:pressed:on_press".to_owned(),
        ];
        assert_eq!(
            parse([
                "--events",
                "event:pressed",
                "field:pressed:count:integer:required",
                "attach:1:pressed:on_press"
            ]),
            Ok(Command::Events(expected)),
        );
        assert!(parse(["--events", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_dispatch_operations() {
        assert_eq!(
            parse(["--dispatch"]),
            Ok(Command::Dispatch(DispatchOptions::new())),
        );
        let mut expected = DispatchOptions::new();
        expected.ops = vec![
            "slot:1.label:text:100".to_owned(),
            "command:save".to_owned(),
            "bind:1:pressed:command:save".to_owned(),
        ];
        assert_eq!(
            parse([
                "--dispatch",
                "slot:1.label:text:100",
                "command:save",
                "bind:1:pressed:command:save"
            ]),
            Ok(Command::Dispatch(expected)),
        );
        assert!(parse(["--dispatch", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_routes_operations() {
        assert_eq!(
            parse(["--routes"]),
            Ok(Command::Routes(RoutesOptions::new())),
        );
        let mut expected = RoutesOptions::new();
        expected.ops = vec![
            "page:home".to_owned(),
            "entry:home".to_owned(),
            "edge:home:detail".to_owned(),
        ];
        assert_eq!(
            parse(["--routes", "page:home", "entry:home", "edge:home:detail"]),
            Ok(Command::Routes(expected)),
        );
        assert!(parse(["--routes", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_machine_operations() {
        assert_eq!(
            parse(["--machine"]),
            Ok(Command::Machine(MachineOptions::new())),
        );
        let mut expected = MachineOptions::new();
        expected.ops = vec![
            "state:idle".to_owned(),
            "transition:idle:go:done".to_owned(),
            "send:go".to_owned(),
        ];
        assert_eq!(
            parse([
                "--machine",
                "state:idle",
                "transition:idle:go:done",
                "send:go"
            ]),
            Ok(Command::Machine(expected)),
        );
        assert!(parse(["--machine", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_adapter_operations() {
        assert_eq!(
            parse(["--adapter"]),
            Ok(Command::Adapter(AdapterOptions::new())),
        );
        let mut expected = AdapterOptions::new();
        expected.ops = vec![
            "sample:ok:bytes".to_owned(),
            "request:file-read:user.png".to_owned(),
            "run".to_owned(),
        ];
        assert_eq!(
            parse([
                "--adapter",
                "sample:ok:bytes",
                "request:file-read:user.png",
                "run"
            ]),
            Ok(Command::Adapter(expected)),
        );
        assert!(parse(["--adapter", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_session_log_operations() {
        assert_eq!(
            parse(["--session-log"]),
            Ok(Command::SessionLog(SessionLogOptions::new())),
        );
        let mut expected = SessionLogOptions::new();
        expected.ops = vec![
            "human:set width:8:16".to_owned(),
            "business:sent the order:none:order-7".to_owned(),
            "undo".to_owned(),
        ];
        assert_eq!(
            parse([
                "--session-log",
                "human:set width:8:16",
                "business:sent the order:none:order-7",
                "undo"
            ]),
            Ok(Command::SessionLog(expected)),
        );
        assert!(parse(["--session-log", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_preview_operations() {
        assert_eq!(
            parse(["--preview"]),
            Ok(Command::Preview(PreviewOptions::new())),
        );
        let mut expected = PreviewOptions::new();
        expected.ops = vec![
            "start:session-1".to_owned(),
            "ready".to_owned(),
            "install:1".to_owned(),
        ];
        assert_eq!(
            parse(["--preview", "start:session-1", "ready", "install:1"]),
            Ok(Command::Preview(expected)),
        );
        assert!(parse(["--preview", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_hot_update_operations() {
        assert_eq!(
            parse(["--hot-update"]),
            Ok(Command::HotUpdate(HotUpdateOptions::new())),
        );
        let mut expected = HotUpdateOptions::new();
        expected.ops = vec![
            "property:width".to_owned(),
            "add-node:label".to_owned(),
            "flush".to_owned(),
        ];
        assert_eq!(
            parse(["--hot-update", "property:width", "add-node:label", "flush"]),
            Ok(Command::HotUpdate(expected)),
        );
        assert!(parse(["--hot-update", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_state_keep_operations() {
        assert_eq!(
            parse(["--state-keep"]),
            Ok(Command::StateKeep(StateKeepOptions::new())),
        );
        let mut expected = StateKeepOptions::new();
        expected.ops = vec![
            "keep:1:text-field".to_owned(),
            "scroll:1:40".to_owned(),
            "restore".to_owned(),
        ];
        assert_eq!(
            parse([
                "--state-keep",
                "keep:1:text-field",
                "scroll:1:40",
                "restore"
            ]),
            Ok(Command::StateKeep(expected)),
        );
        assert!(parse(["--state-keep", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_matrix_operations() {
        assert_eq!(
            parse(["--matrix"]),
            Ok(Command::Matrix(MatrixOptions::new())),
        );
        let mut expected = MatrixOptions::new();
        expected.ops = vec![
            "document:design.json".to_owned(),
            "viewport:phone:390x844:portrait".to_owned(),
            "start:phone".to_owned(),
        ];
        assert_eq!(
            parse([
                "--matrix",
                "document:design.json",
                "viewport:phone:390x844:portrait",
                "start:phone"
            ]),
            Ok(Command::Matrix(expected)),
        );
        assert!(parse(["--matrix", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_inspect_operations() {
        assert_eq!(
            parse(["--inspect"]),
            Ok(Command::Inspect(InspectOptions::new())),
        );
        let mut expected = InspectOptions::new();
        expected.ops = vec![
            "adapter:real".to_owned(),
            "instance:phone:4".to_owned(),
            "values:1:width".to_owned(),
            "trigger:send-order".to_owned(),
            "check".to_owned(),
        ];
        assert_eq!(
            parse([
                "--inspect",
                "adapter:real",
                "instance:phone:4",
                "values:1:width",
                "trigger:send-order",
                "check"
            ]),
            Ok(Command::Inspect(expected)),
        );
        assert!(parse(["--inspect", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_visual_comparison_operations() {
        assert_eq!(
            parse(["--visual"]),
            Ok(Command::Visual(VisualOptions::new())),
        );
        let mut expected = VisualOptions::new();
        expected.ops = vec![
            "platform:linux/x86_64".to_owned(),
            "record:home:3x2:012345".to_owned(),
            "tolerance:50:2".to_owned(),
            "shot:home:3x2:012345".to_owned(),
        ];
        assert_eq!(
            parse([
                "--visual",
                "platform:linux/x86_64",
                "record:home:3x2:012345",
                "tolerance:50:2",
                "shot:home:3x2:012345"
            ]),
            Ok(Command::Visual(expected)),
        );
        assert!(parse(["--visual", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_preview_recovery_operations() {
        assert_eq!(
            parse(["--preview-recovery"]),
            Ok(Command::PreviewRecovery(PreviewRecoveryOptions::new())),
        );
        let mut expected = PreviewRecoveryOptions::new();
        expected.ops = vec![
            "edit:7".to_owned(),
            "launch".to_owned(),
            "crash:11".to_owned(),
            "reload".to_owned(),
            "check".to_owned(),
        ];
        assert_eq!(
            parse([
                "--preview-recovery",
                "edit:7",
                "launch",
                "crash:11",
                "reload",
                "check"
            ]),
            Ok(Command::PreviewRecovery(expected)),
        );
        assert!(parse(["--preview-recovery", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_load_operations() {
        assert_eq!(parse(["--load"]), Ok(Command::Load(LoadOptions::new())));
        let mut expected = LoadOptions::new();
        expected.ops = vec![
            "budget:16".to_owned(),
            "repeat:phone:100:12:40:8".to_owned(),
            "invalid:bind:phone:missing<-nothing".to_owned(),
            "check".to_owned(),
        ];
        assert_eq!(
            parse([
                "--load",
                "budget:16",
                "repeat:phone:100:12:40:8",
                "invalid:bind:phone:missing<-nothing",
                "check"
            ]),
            Ok(Command::Load(expected)),
        );
        assert!(parse(["--load", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_baseline_session_and_its_duration() {
        // `--launch` used to be an unknown argument; it is a command now, and its default session
        // is bounded so a smoke test can end on its own.
        assert_eq!(
            parse(["--launch"]),
            Ok(Command::Launch(LaunchOptions::default()))
        );
        assert_eq!(
            parse(["--launch", "--exit-after-ms", "1500"]),
            Ok(Command::Launch(LaunchOptions {
                exit_after: Duration::from_millis(1500)
            })),
        );
        assert_ne!(
            LaunchOptions::default().exit_after,
            Duration::from_millis(1500)
        );

        // A duration that is missing, unparsable or followed by anything else is a usage error.
        assert!(parse(["--launch", "--exit-after-ms"]).is_err());
        assert!(parse(["--launch", "--exit-after-ms", "soon"]).is_err());
        assert!(parse(["--launch", "--exit-after-ms", "10", "extra"]).is_err());
        assert!(parse(["--launch", "--unknown"]).is_err());
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
