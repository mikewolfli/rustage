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
        Command, ConfigurationOptions, DependenciesOptions, DocumentEdit, DocumentOptions,
        DropEntryOptions, EditOptions, FileOpOptions, HistoryDomain, HistoryEdit, HistoryOptions,
        ImportOptions, KitOptions, LaunchOptions, NewProjectOptions, NewWorkspaceOptions,
        OpenProjectOptions, ValidateOptions, WorkspaceAction, WorkspaceOptions, help_text,
        log_level_of, parse,
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
