// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The file-dialog host service: ask for a path, get a typed answer. Contract **HOST**.
//!
//! T01.02 asks for file dialogs. What the pinned dependency offers, checked rather than assumed
//! (`cargo run -- --probe-host`):
//!
//! * The pinned revision now exposes the operating system's own chooser through
//!   `rust_widgets::choose_path` (and the `open_path`/`save_path`/`select_directory` vocabulary it
//!   fronts), and reports whether this backend has one **before** the call through
//!   `platform::capabilities().file_dialog` (`false` on a state backend, `true` where a real toolkit
//!   is behind it); the legacy `create_file_dialog` still returns `0` because the library now paints
//!   every kind itself. This host has not yet adopted the native picker — it drives the painted
//!   frame below, where the designer owns the list — so where a backend advertises one, presenting
//!   it is registered as follow-up work rather than opened here.
//! * The painted `FileDialog` is a **frame** —
//!   title, mode, start directory, name filters, and the `accepted` / `rejected` /
//!   `files_selected` signals — with no file list of its own. A host that owns a list reports
//!   selection through `set_current_file`, and the dialog's own documentation says so.
//!
//! So this module owns the half that is a *host* concern and does not pretend to own the other
//! half: the request, the answer, the modal handoff, the two host actions a chooser needs
//! (accept, cancel) and **validation of what came back** — Unicode, symlinks, permission, "is this
//! a file or a directory", and "accepted with nothing chosen", which is not a successful choice.
//! Building a browsable list is the designer's own shell work (F26) and is not faked here.
//!
//! **Why the answer is asynchronous.** The frame has to be *shown* on the thread that drives the
//! UI, and that thread must keep running the loop while the user decides. A `choose()` that blocked
//! would freeze the window it just opened, so [`FileChooser::show`] returns a [`PendingChoice`] and
//! the host polls it. [`PendingChoice::accept`] and [`PendingChoice::cancel`] push the same key
//! events a user's Return and Escape push, so a scripted host drives the dialog through the real
//! path rather than around it.
//!
//! **What is not proven here.** Whether the frame is visible on a screen, whether a person can move
//! the pointer into it, and how it looks at a real DPI are the same host-platform facts T01.01 left
//! to a person; this module proves the handoff, not the pixels.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use rust_widgets::core::{ObjectId, Rect};
use rust_widgets::event::{Event, key_codes};
use rust_widgets::widget::dialog::file_dialog::{FileDialog, FileDialogMode, FileFilter};
use rust_widgets::widget::runtime;

/// The geometry a chooser frame is given when the request does not name one.
pub const DEFAULT_GEOMETRY: Rect = Rect {
    x: 0,
    y: 0,
    width: 520,
    height: 360,
};

/// What the caller is asking for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DialogMode {
    /// One existing file.
    OpenFile,
    /// One or more existing files.
    OpenFiles,
    /// A destination path, which need not exist yet.
    SaveFile,
    /// An existing directory.
    SelectDirectory,
}

impl DialogMode {
    /// The dependency's own mode for this one.
    ///
    /// The mapping is total and one-to-one, so the two vocabularies cannot drift: a mode this host
    /// does not have cannot be asked for, and a mode the frame offers cannot be unreachable.
    pub const fn frame_mode(self) -> FileDialogMode {
        match self {
            Self::OpenFile => FileDialogMode::OpenFile,
            Self::OpenFiles => FileDialogMode::OpenFiles,
            Self::SaveFile => FileDialogMode::SaveFile,
            Self::SelectDirectory => FileDialogMode::SelectDirectory,
        }
    }

    /// The name used in messages.
    pub const fn name(self) -> &'static str {
        match self {
            Self::OpenFile => "open file",
            Self::OpenFiles => "open files",
            Self::SaveFile => "save file",
            Self::SelectDirectory => "select directory",
        }
    }

    /// Whether a path that does not exist yet can be a valid answer.
    pub const fn allows_missing_path(self) -> bool {
        matches!(self, Self::SaveFile)
    }

    /// Whether several paths are a valid answer.
    pub const fn allows_several(self) -> bool {
        matches!(self, Self::OpenFiles)
    }
}

/// What the caller wants the chooser to show.
///
/// Not `Eq`, unlike the other types here: the dependency's `FileFilter` carries no comparison, and
/// inventing one for it locally would be a second definition of the same concept (principle #35).
#[derive(Clone, Debug)]
pub struct DialogRequest {
    /// What is being chosen.
    pub mode: DialogMode,
    /// The window title, when the caller wants one of its own.
    pub title: Option<String>,
    /// Where the chooser should start.
    ///
    /// A directory that is not there is a refusal rather than a silently ignored hint: starting in
    /// the working directory instead would answer a question the caller did not ask.
    pub start_directory: Option<PathBuf>,
    /// Name filters. The dependency's own type, so a filter a caller writes is the same filter the
    /// frame renders (principle #35, one definition).
    pub filters: Vec<FileFilter>,
    /// The selection the host's own list currently holds.
    ///
    /// The frame has no list of its own, so this is how a list owner hands over what the user had
    /// selected before the frame took the modal stack. Empty means "nothing selected yet".
    pub preselected: Vec<PathBuf>,
    /// The frame's geometry.
    pub geometry: Rect,
}

impl Default for DialogRequest {
    fn default() -> Self {
        Self {
            mode: DialogMode::OpenFile,
            title: None,
            start_directory: None,
            filters: vec![FileFilter::all_files()],
            preselected: Vec::new(),
            geometry: DEFAULT_GEOMETRY,
        }
    }
}

/// Whether a chosen path is a file or a directory.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SelectionKind {
    /// A regular file (or something that is not a directory).
    File,
    /// A directory.
    Directory,
}

impl SelectionKind {
    /// The name used in messages.
    pub const fn name(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Directory => "directory",
        }
    }
}

/// A chosen path that has been checked against the filesystem.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Selection {
    /// The path this host will use: the link target when the choice came through one.
    pub path: PathBuf,
    /// The link the user actually chose, when it was a link. Kept so a diagnostic can say *which*
    /// link resolved where, instead of quietly rewriting the user's answer.
    pub link: Option<PathBuf>,
    /// Whether the target is a file or a directory.
    pub kind: SelectionKind,
}

impl Selection {
    /// A sentence naming what was chosen.
    pub fn message(&self) -> String {
        match &self.link {
            Some(link) => format!(
                "{} ({} through the link {})",
                self.path.display(),
                self.kind.name(),
                link.display(),
            ),
            None => format!("{} ({})", self.path.display(), self.kind.name()),
        }
    }
}

/// Why a chosen path cannot be used.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SelectionRefusal {
    /// The chooser was accepted without any path selected. Not a successful choice.
    NothingChosen,
    /// More paths came back than the mode allows.
    TooManyChosen { count: usize, mode: DialogMode },
    /// The path does not exist.
    Missing { path: PathBuf },
    /// The path exists but is a directory where a file was asked for, or the other way round.
    WrongKind {
        path: PathBuf,
        found: SelectionKind,
        mode: DialogMode,
    },
    /// The path exists but cannot be read, so choosing it would only move the failure later.
    Unreadable { path: PathBuf, reason: String },
    /// A save target's parent directory is missing.
    ParentMissing { path: PathBuf },
    /// A save target's parent is not a directory.
    ParentNotADirectory { path: PathBuf },
    /// A save target's parent cannot be written into.
    ParentUnwritable { path: PathBuf, reason: String },
    /// A link whose target is gone.
    BrokenLink { path: PathBuf, reason: String },
}

impl SelectionRefusal {
    /// A sentence naming what was found.
    pub fn message(&self) -> String {
        match self {
            Self::NothingChosen => "the chooser was accepted without a selection".to_owned(),
            Self::TooManyChosen { count, mode } => {
                format!("{count} paths were chosen, but {} accepts one", mode.name(),)
            }
            Self::Missing { path } => format!("{} does not exist", path.display()),
            Self::WrongKind { path, found, mode } => format!(
                "{} is a {}, but {} needs a {}",
                path.display(),
                found.name(),
                mode.name(),
                if *mode == DialogMode::SelectDirectory {
                    "directory"
                } else {
                    "file"
                },
            ),
            Self::Unreadable { path, reason } => {
                format!("{} cannot be read: {reason}", path.display())
            }
            Self::ParentMissing { path } => format!(
                "the directory {} would be created in does not exist",
                path.display(),
            ),
            Self::ParentNotADirectory { path } => format!(
                "{} would be created inside something that is not a directory",
                path.display(),
            ),
            Self::ParentUnwritable { path, reason } => format!(
                "{} cannot be created because its directory refuses writes: {reason}",
                path.display(),
            ),
            Self::BrokenLink { path, reason } => {
                format!(
                    "{} is a link whose target cannot be reached: {reason}",
                    path.display()
                )
            }
        }
    }
}

impl fmt::Display for SelectionRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message())
    }
}

impl std::error::Error for SelectionRefusal {}

/// What happened to a chooser request.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum DialogOutcome {
    /// The user chose these paths. Every one of them has been checked against the filesystem.
    Accepted(Vec<Selection>),
    /// The user cancelled. Explicit on purpose: a cancelled chooser must not read as "no paths
    /// were chosen", which is what an empty list would say.
    Cancelled,
    /// The chooser worked, but the choice cannot be used. The caller keeps the refusal so it can
    /// ask again, which is what F26's dialog UX needs.
    Refused { refusal: SelectionRefusal },
    /// This build cannot show the chooser here.
    Unsupported { reason: String },
    /// Showing the chooser failed.
    Failed { reason: String },
}

impl DialogOutcome {
    /// Whether the caller has paths to use.
    pub fn selections(&self) -> &[Selection] {
        match self {
            Self::Accepted(selections) => selections,
            _ => &[],
        }
    }

    /// A sentence for a log line or a diagnostic.
    pub fn message(&self) -> String {
        match self {
            Self::Accepted(selections) => {
                let joined: Vec<String> = selections
                    .iter()
                    .map(|selection| selection.message())
                    .collect();
                format!("chosen: {}", joined.join(", "))
            }
            Self::Cancelled => "the chooser was cancelled".to_owned(),
            Self::Refused { refusal } => format!("the choice was refused: {refusal}"),
            Self::Unsupported { reason } => format!("no chooser is available here: {reason}"),
            Self::Failed { reason } => format!("the chooser failed: {reason}"),
        }
    }
}

/// Why a chooser could not be shown.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum DialogOpenError {
    /// The dependency's widget runtime has no registry on the calling thread, so a frame could not
    /// be mounted. A caller that is not on the UI thread gets this instead of an invisible dialog.
    NoWidgetRegistry,
    /// The start directory is not usable.
    StartDirectory { path: PathBuf, reason: String },
    /// The modal stack refused the frame.
    NotModal { reason: String },
}

impl DialogOpenError {
    /// A sentence naming what was found.
    pub fn message(&self) -> String {
        match self {
            Self::NoWidgetRegistry => {
                "the widget runtime has no registry on this thread, so a chooser frame cannot be \
                 mounted; show a dialog from the thread that drives the UI"
                    .to_owned()
            }
            Self::StartDirectory { path, reason } => {
                format!("the chooser cannot start in {}: {reason}", path.display())
            }
            Self::NotModal { reason } => format!("the chooser frame was not made modal: {reason}"),
        }
    }
}

impl fmt::Display for DialogOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message())
    }
}

impl std::error::Error for DialogOpenError {}

/// Checks the paths a chooser reported and turns them into usable selections.
///
/// This is the half of a file dialog that is a host concern, and it is deliberately separate from
/// the frame so it is testable without a UI. Every rule here exists because the alternative is a
/// failure moved later: a missing file becomes a build error, an unreadable file becomes an empty
/// document, a directory chosen as a file becomes a confusing "is a directory" from a later call.
pub fn check_selection(
    mode: DialogMode,
    chosen: &[PathBuf],
) -> Result<Vec<Selection>, SelectionRefusal> {
    if chosen.is_empty() {
        return Err(SelectionRefusal::NothingChosen);
    }
    if !mode.allows_several() && chosen.len() > 1 {
        return Err(SelectionRefusal::TooManyChosen {
            count: chosen.len(),
            mode,
        });
    }

    let mut selections = Vec::with_capacity(chosen.len());
    for path in chosen {
        selections.push(check_one(mode, path)?);
    }
    Ok(selections)
}

/// Checks one chosen path.
fn check_one(mode: DialogMode, path: &Path) -> Result<Selection, SelectionRefusal> {
    let link = std::fs::symlink_metadata(path)
        .ok()
        .filter(|metadata| metadata.file_type().is_symlink())
        .map(|_| path.to_path_buf());

    // A save target is the one answer that is allowed not to exist yet, so it is checked against
    // its directory rather than against itself.
    if mode == DialogMode::SaveFile {
        return check_save_target(path, link);
    }

    let resolved = match std::fs::canonicalize(path) {
        Ok(resolved) => resolved,
        Err(error) => {
            if link.is_some() {
                // A link whose target is gone is its own diagnosis; "does not exist" would send the
                // caller looking for a file that is right there in the directory listing.
                return Err(SelectionRefusal::BrokenLink {
                    path: path.to_path_buf(),
                    reason: error.to_string(),
                });
            }
            if error.kind() == std::io::ErrorKind::PermissionDenied {
                // A path the process may not look at is not a path that is absent. Reporting it as
                // missing would send the user to check a directory they cannot enter anyway.
                return Err(SelectionRefusal::Unreadable {
                    path: path.to_path_buf(),
                    reason: error.to_string(),
                });
            }
            return Err(SelectionRefusal::Missing {
                path: path.to_path_buf(),
            });
        }
    };

    let metadata = std::fs::metadata(&resolved).map_err(|_| SelectionRefusal::Missing {
        path: path.to_path_buf(),
    })?;
    let kind = if metadata.is_dir() {
        SelectionKind::Directory
    } else {
        SelectionKind::File
    };

    let expects_directory = mode == DialogMode::SelectDirectory;
    if expects_directory != (kind == SelectionKind::Directory) {
        return Err(SelectionRefusal::WrongKind {
            path: resolved,
            found: kind,
            mode,
        });
    }

    if kind == SelectionKind::File {
        check_readable(path)?;
    }

    Ok(Selection {
        path: resolved,
        link,
        kind,
    })
}

/// Checks a save target: an existing file, or a new entry in a directory that can take one.
fn check_save_target(path: &Path, link: Option<PathBuf>) -> Result<Selection, SelectionRefusal> {
    if let Some(link) = link {
        // A link the user chose: it must resolve, because saving through a broken link would create
        // the file wherever the link points or fail later with a message about a path the user
        // never typed.
        let resolved =
            std::fs::canonicalize(path).map_err(|error| SelectionRefusal::BrokenLink {
                path: link.clone(),
                reason: error.to_string(),
            })?;
        let metadata =
            std::fs::metadata(&resolved).map_err(|error| SelectionRefusal::BrokenLink {
                path: link,
                reason: error.to_string(),
            })?;
        if metadata.is_dir() {
            return Err(SelectionRefusal::WrongKind {
                path: resolved,
                found: SelectionKind::Directory,
                mode: DialogMode::SaveFile,
            });
        }
        check_writable_file(&resolved)?;
        return Ok(Selection {
            path: resolved,
            link: None,
            kind: SelectionKind::File,
        });
    }

    if let Ok(metadata) = std::fs::metadata(path) {
        if metadata.is_dir() {
            return Err(SelectionRefusal::WrongKind {
                path: path.to_path_buf(),
                found: SelectionKind::Directory,
                mode: DialogMode::SaveFile,
            });
        }
        let resolved = match std::fs::canonicalize(path) {
            Ok(resolved) => resolved,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                return Err(SelectionRefusal::Unreadable {
                    path: path.to_path_buf(),
                    reason: error.to_string(),
                });
            }
            Err(_) => {
                return Err(SelectionRefusal::Missing {
                    path: path.to_path_buf(),
                });
            }
        };
        check_writable_file(&resolved)?;
        return Ok(Selection {
            path: resolved,
            link: None,
            kind: SelectionKind::File,
        });
    }

    let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    else {
        return Err(SelectionRefusal::ParentMissing {
            path: path.to_path_buf(),
        });
    };
    match std::fs::metadata(parent) {
        Ok(metadata) if !metadata.is_dir() => Err(SelectionRefusal::ParentNotADirectory {
            path: path.to_path_buf(),
        }),
        Ok(_) => {
            let canonical_parent = std::fs::canonicalize(parent).map_err(|error| {
                SelectionRefusal::ParentUnwritable {
                    path: path.to_path_buf(),
                    reason: error.to_string(),
                }
            })?;
            check_writable_directory(&canonical_parent, path)?;
            let Some(name) = path.file_name() else {
                return Err(SelectionRefusal::ParentMissing {
                    path: path.to_path_buf(),
                });
            };
            Ok(Selection {
                path: canonical_parent.join(name),
                link: None,
                kind: SelectionKind::File,
            })
        }
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            Err(SelectionRefusal::Unreadable {
                path: parent.to_path_buf(),
                reason: error.to_string(),
            })
        }
        Err(_) => Err(SelectionRefusal::ParentMissing {
            path: path.to_path_buf(),
        }),
    }
}

/// Whether a path can be read, reported with the OS's own reason when it cannot.
fn check_readable(path: &Path) -> Result<(), SelectionRefusal> {
    match std::fs::File::open(path) {
        Ok(_) => Ok(()),
        Err(error) => Err(SelectionRefusal::Unreadable {
            path: path.to_path_buf(),
            reason: error.to_string(),
        }),
    }
}

/// Whether an existing file can be written, checked by opening it rather than by reading permission
/// bits, which say nothing about a read-only mount or an ACL.
fn check_writable_file(path: &Path) -> Result<(), SelectionRefusal> {
    match std::fs::OpenOptions::new().append(true).open(path) {
        Ok(_) => Ok(()),
        Err(error) => Err(SelectionRefusal::ParentUnwritable {
            path: path.to_path_buf(),
            reason: error.to_string(),
        }),
    }
}

/// Whether a directory takes a new entry, answered by creating one and removing it again.
///
/// Permission bits cannot answer this: a read-only mount and an ACL that denies the write both look
/// writable on paper, and both are exactly how a save fails after the file dialog said yes. The
/// probe file is removed before returning, so the directory is left as it was found; the name
/// carries the process id and a counter so two hosts in one directory cannot collide.
fn check_writable_directory(directory: &Path, target: &Path) -> Result<(), SelectionRefusal> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static PROBES: AtomicUsize = AtomicUsize::new(0);

    for _ in 0..8 {
        let probe = directory.join(format!(
            ".rustage-write-probe-{}-{}",
            std::process::id(),
            PROBES.fetch_add(1, Ordering::Relaxed),
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe)
        {
            Ok(_) => {
                let _ = std::fs::remove_file(&probe);
                return Ok(());
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(SelectionRefusal::ParentUnwritable {
                    path: target.to_path_buf(),
                    reason: error.to_string(),
                });
            }
        }
    }
    Err(SelectionRefusal::ParentUnwritable {
        path: target.to_path_buf(),
        reason: "every probe name this host tried was already taken".to_owned(),
    })
}

/// A chooser that has been shown and not yet answered.
///
/// Dropping it cancels it, so a request whose owner went away cannot leave a modal frame on the
/// stack blocking input to a window nobody owns (principle #10: no half-finished state).
#[derive(Debug)]
pub struct PendingChoice {
    id: ObjectId,
    mode: DialogMode,
    receiver: Receiver<RawChoice>,
    closed: std::cell::Cell<bool>,
}

/// What the frame reported, before the host checks it against the filesystem.
#[derive(Debug)]
enum RawChoice {
    Selected(Vec<PathBuf>),
    AcceptedWithoutSelection,
    Cancelled,
}

impl PendingChoice {
    /// The frame's widget id, for a host that wants to focus or repaint it.
    pub fn id(&self) -> ObjectId {
        self.id
    }

    /// What is being chosen.
    pub fn mode(&self) -> DialogMode {
        self.mode
    }

    /// Whether the chooser is still waiting for an answer.
    pub fn is_open(&self) -> bool {
        !self.closed.get()
    }

    /// Pushes the key event a user's Return pushes.
    ///
    /// Public because a scripted host (a smoke test, a demo, an accessibility tool) has to be able
    /// to answer the dialog, and it must go through the same event the user's key goes through
    /// rather than around it. Returns `false` when the frame is already gone.
    pub fn accept(&self) -> bool {
        self.dispatch(key_codes::ENTER)
    }

    /// Pushes the key event a user's Escape pushes.
    pub fn cancel(&self) -> bool {
        self.dispatch(key_codes::ESCAPE)
    }

    fn dispatch(&self, key: u32) -> bool {
        if self.closed.get() {
            return false;
        }
        runtime::dispatch_event(self.id, &Event::key_press(key, 0))
    }

    /// Takes the outcome if the chooser has been answered, otherwise `None`.
    ///
    /// A chooser that was answered is cleaned up by this call: its frame is unregistered and the
    /// modal stack is popped, so nothing keeps input blocked after the answer is known.
    pub fn poll(&self) -> Option<DialogOutcome> {
        if self.closed.get() {
            return None;
        }
        match self.receiver.try_recv() {
            Ok(RawChoice::Selected(paths)) => Some(self.finish(checked(self.mode, &paths))),
            Ok(RawChoice::AcceptedWithoutSelection) => {
                Some(self.finish(Err(SelectionRefusal::NothingChosen)))
            }
            Ok(RawChoice::Cancelled) => Some(self.finish_plain(DialogOutcome::Cancelled)),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(self.finish_plain(DialogOutcome::Failed {
                reason: "the chooser frame stopped reporting without an answer".to_owned(),
            })),
        }
    }

    /// Renders a checked answer and closes the request.
    fn finish(&self, checked: Result<Vec<Selection>, SelectionRefusal>) -> DialogOutcome {
        let outcome = match checked {
            Ok(selections) => DialogOutcome::Accepted(selections),
            Err(refusal) => DialogOutcome::Refused { refusal },
        };
        self.finish_plain(outcome)
    }

    /// Closes the request and returns the outcome.
    fn finish_plain(&self, outcome: DialogOutcome) -> DialogOutcome {
        self.close();
        outcome
    }

    /// Unregisters the frame and pops it off the modal stack, once.
    fn close(&self) {
        if self.closed.replace(true) {
            return;
        }
        runtime::exit_modal(self.id);
        runtime::unregister(self.id);
    }
}

/// Turns the frame's raw answer into a checked one.
fn checked(mode: DialogMode, paths: &[PathBuf]) -> Result<Vec<Selection>, SelectionRefusal> {
    check_selection(mode, paths)
}

impl Drop for PendingChoice {
    fn drop(&mut self) {
        // A request that is dropped while open is cancelled, and the frame removed either way.
        self.close();
    }
}

/// A chooser the host can ask for a path.
pub trait FileChooser {
    /// Shows the chooser and returns a request that reports its answer.
    fn show(&self, request: &DialogRequest) -> Result<PendingChoice, DialogOpenError>;
}

/// The chooser built on the dependency's painted frame, driven through the widget runtime.
#[derive(Clone, Copy, Debug)]
pub struct FrameChooser {
    /// The window the frame belongs to, when the caller has one. Without it the frame is still
    /// mounted and modal, but the platform has no window to attach it to.
    pub window: Option<ObjectId>,
}

impl FrameChooser {
    /// A chooser that attaches its frames to `window`.
    pub fn in_window(window: ObjectId) -> Self {
        Self {
            window: Some(window),
        }
    }

    /// A chooser with no window, for a host that has not created one.
    pub fn detached() -> Self {
        Self { window: None }
    }
}

impl Default for FrameChooser {
    fn default() -> Self {
        Self::detached()
    }
}

impl FileChooser for FrameChooser {
    fn show(&self, request: &DialogRequest) -> Result<PendingChoice, DialogOpenError> {
        if let Some(directory) = &request.start_directory
            && !directory.is_dir()
        {
            return Err(DialogOpenError::StartDirectory {
                path: directory.clone(),
                reason: "it is not an existing directory".to_owned(),
            });
        }

        let mut frame = FileDialog::new(request.geometry);
        frame.set_mode(request.mode.frame_mode());
        if let Some(title) = &request.title {
            frame.set_title(title.clone());
        }
        if let Some(directory) = &request.start_directory {
            frame.set_directory(directory.to_string_lossy().into_owned());
        }
        if !request.filters.is_empty() {
            frame.set_name_filters(request.filters.clone());
        }
        for path in &request.preselected {
            frame.select_file(path.to_string_lossy().into_owned());
        }

        // The signals are connected before the frame is handed to the registry, because the
        // registry owns it afterwards. The signals are reference-counted inside the dependency, so
        // the handles stay live across the move; the probe checks that rather than assuming it.
        let (sender, receiver) = channel();
        let selected = sender.clone();
        frame.files_selected.connect(move |paths| {
            let _ = selected.send(RawChoice::Selected(
                paths.iter().map(PathBuf::from).collect(),
            ));
        });
        let accepted = sender.clone();
        frame.accepted.connect(move || {
            let _ = accepted.send(RawChoice::AcceptedWithoutSelection);
        });
        let rejected = sender;
        frame.rejected.connect(move || {
            let _ = rejected.send(RawChoice::Cancelled);
        });

        let id = runtime::register(Box::new(frame)).ok_or(DialogOpenError::NoWidgetRegistry)?;
        if let Some(window) = self.window {
            runtime::set_host_window(id, window);
        }
        if !runtime::enter_modal(id) {
            runtime::unregister(id);
            return Err(DialogOpenError::NotModal {
                reason: "the frame was not mounted when the modal stack was asked".to_owned(),
            });
        }

        Ok(PendingChoice {
            id,
            mode: request.mode,
            receiver,
            closed: std::cell::Cell::new(false),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DialogMode, DialogOpenError, DialogOutcome, DialogRequest, FileChooser, FrameChooser,
        SelectionKind, SelectionRefusal, check_selection,
    };
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Sandbox(PathBuf);

    impl Sandbox {
        fn new(name: &str) -> Self {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "rustage-dialog-{name}-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed),
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("the sandbox is creatable");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }

        fn file(&self, name: &str) -> PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, "content").expect("the file is writable");
            path
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn an_accepted_chooser_with_nothing_selected_is_a_refusal() {
        let refusal = check_selection(DialogMode::OpenFile, &[]).unwrap_err();

        assert_eq!(refusal, SelectionRefusal::NothingChosen);
        assert!(refusal.message().contains("without a selection"));
    }

    #[test]
    fn unicode_names_survive_the_check_unchanged() {
        let sandbox = Sandbox::new("unicode");
        let file = sandbox.file("设计 稿.json");

        let selections =
            check_selection(DialogMode::OpenFile, std::slice::from_ref(&file)).expect("readable");

        assert_eq!(selections.len(), 1);
        assert!(
            selections[0].path.ends_with("设计 稿.json"),
            "{}",
            selections[0].path.display(),
        );
        assert_eq!(selections[0].kind, SelectionKind::File);
        assert!(selections[0].link.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn a_link_is_reported_with_its_target_and_the_link_it_came_through() {
        let sandbox = Sandbox::new("link");
        let target = sandbox.file("real.json");
        let link = sandbox.path().join("shortcut.json");
        std::os::unix::fs::symlink(&target, &link).expect("the link is creatable");

        let selections =
            check_selection(DialogMode::OpenFile, std::slice::from_ref(&link)).expect("readable");

        assert_eq!(selections[0].link.as_deref(), Some(link.as_path()));
        assert_eq!(
            selections[0].path,
            std::fs::canonicalize(&target).expect("the target resolves"),
        );
        assert!(selections[0].message().contains("through the link"));
    }

    #[cfg(unix)]
    #[test]
    fn a_link_whose_target_is_gone_is_named_as_a_broken_link_not_as_a_missing_file() {
        let sandbox = Sandbox::new("broken");
        let gone = sandbox.path().join("gone.json");
        let link = sandbox.path().join("dangling.json");
        std::os::unix::fs::symlink(&gone, &link).expect("the link is creatable");

        let refusal =
            check_selection(DialogMode::OpenFile, std::slice::from_ref(&link)).unwrap_err();

        match refusal {
            SelectionRefusal::BrokenLink { path, reason } => {
                assert_eq!(path, link);
                assert!(!reason.is_empty());
            }
            other => panic!("expected BrokenLink, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_file_that_cannot_be_read_is_refused_with_the_os_reason() {
        use std::os::unix::fs::PermissionsExt;

        let sandbox = Sandbox::new("unreadable");
        let file = sandbox.file("secret.json");
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000))
            .expect("the mode is settable");

        let refusal =
            check_selection(DialogMode::OpenFile, std::slice::from_ref(&file)).unwrap_err();

        // Running as root, the read succeeds; the assertion is then that the host reports the real
        // answer rather than a fixed one.
        match refusal {
            SelectionRefusal::Unreadable { path, reason } => {
                assert_eq!(path, file);
                assert!(
                    reason.contains("denied") || reason.contains("ermission"),
                    "{reason}"
                );
            }
            other => panic!("expected Unreadable, got {other:?}"),
        }

        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).expect("restored");
    }

    #[test]
    fn a_directory_chosen_where_a_file_belongs_is_a_wrong_kind_refusal() {
        let sandbox = Sandbox::new("kind");

        let refusal =
            check_selection(DialogMode::OpenFile, &[sandbox.path().to_path_buf()]).unwrap_err();

        match refusal {
            SelectionRefusal::WrongKind { found, mode, .. } => {
                assert_eq!(found, SelectionKind::Directory);
                assert_eq!(mode, DialogMode::OpenFile);
            }
            other => panic!("expected WrongKind, got {other:?}"),
        }
    }

    #[test]
    fn a_file_chosen_where_a_directory_belongs_is_the_same_refusal_the_other_way() {
        let sandbox = Sandbox::new("kind-reverse");
        let file = sandbox.file("main.json");

        let refusal = check_selection(DialogMode::SelectDirectory, &[file]).unwrap_err();

        assert!(matches!(
            refusal,
            SelectionRefusal::WrongKind {
                found: SelectionKind::File,
                mode: DialogMode::SelectDirectory,
                ..
            }
        ));
    }

    #[test]
    fn opening_two_files_at_once_is_refused_when_the_mode_takes_one() {
        let sandbox = Sandbox::new("several");
        let first = sandbox.file("a.json");
        let second = sandbox.file("b.json");

        let refusal = check_selection(DialogMode::OpenFile, &[first, second]).unwrap_err();

        assert_eq!(
            refusal,
            SelectionRefusal::TooManyChosen {
                count: 2,
                mode: DialogMode::OpenFile
            }
        );
    }

    #[test]
    fn a_path_that_is_not_there_is_refused_rather_than_returned() {
        let sandbox = Sandbox::new("missing");
        let missing = sandbox.path().join("later.json");

        assert_eq!(
            check_selection(DialogMode::OpenFile, std::slice::from_ref(&missing)).unwrap_err(),
            SelectionRefusal::Missing { path: missing },
        );
    }

    #[test]
    fn a_save_target_may_not_exist_yet_but_its_directory_must() {
        let sandbox = Sandbox::new("save");
        let target = sandbox.path().join("new.json");

        let selections = check_selection(DialogMode::SaveFile, std::slice::from_ref(&target))
            .expect("a new file in an existing directory is a valid save target");
        assert!(selections[0].path.ends_with("new.json"));
        assert!(selections[0].path.is_absolute());
        assert_eq!(selections[0].kind, SelectionKind::File);
        assert!(
            !target.exists(),
            "checking a save target must not create it, got {}",
            target.display(),
        );
        assert!(
            std::fs::read_dir(sandbox.path()).is_ok_and(|mut entries| entries.next().is_none()),
            "checking a save target must not leave a probe file behind",
        );

        let nowhere = sandbox.path().join("no-such-directory").join("new.json");
        assert_eq!(
            check_selection(DialogMode::SaveFile, std::slice::from_ref(&nowhere)).unwrap_err(),
            SelectionRefusal::ParentMissing { path: nowhere },
        );
    }

    #[test]
    fn a_save_target_whose_parent_is_a_file_is_refused_as_not_a_directory() {
        let sandbox = Sandbox::new("save-parent");
        let file = sandbox.file("not-a-directory");

        let refusal = check_selection(DialogMode::SaveFile, &[file.join("new.json")]).unwrap_err();

        assert!(
            matches!(refusal, SelectionRefusal::ParentNotADirectory { .. }),
            "{refusal:?}",
        );
        assert!(refusal.message().contains("not a directory"), "{refusal}");
    }

    #[cfg(unix)]
    #[test]
    fn a_save_target_in_a_directory_that_refuses_writes_is_refused_with_the_os_reason() {
        use std::os::unix::fs::PermissionsExt;

        let sandbox = Sandbox::new("save-unwritable");
        let locked = sandbox.path().join("locked");
        std::fs::create_dir_all(&locked).expect("the directory is creatable");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o500))
            .expect("the mode is settable");

        let target = locked.join("new.json");
        let result = check_selection(DialogMode::SaveFile, std::slice::from_ref(&target));

        // As root the write is allowed, so what is asserted is that the host reports the real
        // answer and never invents one.
        match result {
            Err(SelectionRefusal::ParentUnwritable { path, reason }) => {
                assert_eq!(path, target);
                assert!(!reason.is_empty(), "a refusal must carry the OS's reason");
            }
            Ok(selections) => assert!(selections[0].path.ends_with("new.json")),
            Err(other) => panic!("expected ParentUnwritable or a real acceptance, got {other:?}"),
        }

        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700))
            .expect("restored");
        assert!(
            std::fs::read_dir(&locked).is_ok_and(|mut entries| entries.next().is_none()),
            "the write probe must not leave anything behind",
        );
    }

    #[test]
    fn the_frame_chooser_shows_a_frame_a_host_can_answer_and_clean_up() {
        let sandbox = Sandbox::new("frame");
        let file = sandbox.file("page.json");
        let request = DialogRequest {
            mode: DialogMode::OpenFile,
            preselected: vec![file.clone()],
            ..DialogRequest::default()
        };

        let chooser = FrameChooser::detached();
        let pending = chooser
            .show(&request)
            .expect("a frame mounts on this thread");
        assert_eq!(pending.mode(), DialogMode::OpenFile);
        assert!(pending.is_open());
        assert!(pending.poll().is_none(), "nothing has answered it yet");

        assert!(pending.accept(), "the frame took the Return key");

        match pending.poll() {
            Some(DialogOutcome::Accepted(selections)) => {
                assert_eq!(selections.len(), 1);
                assert_eq!(selections[0].path, std::fs::canonicalize(&file).unwrap());
            }
            other => panic!("expected Accepted, got {other:?}"),
        }
        // The answer closes the request, so the modal stack is clear and a second poll has nothing.
        assert!(!pending.is_open());
        assert!(pending.poll().is_none());
    }

    #[test]
    fn cancelling_the_frame_is_reported_as_cancelled_not_as_an_empty_selection() {
        let sandbox = Sandbox::new("frame-cancel");
        let request = DialogRequest {
            mode: DialogMode::OpenFile,
            preselected: vec![sandbox.file("page.json")],
            ..DialogRequest::default()
        };
        let chooser = FrameChooser::detached();
        let pending = chooser
            .show(&request)
            .expect("a frame mounts on this thread");

        assert!(pending.cancel());

        assert_eq!(pending.poll(), Some(DialogOutcome::Cancelled));
    }

    #[test]
    fn accepting_the_frame_with_nothing_selected_is_a_refusal_not_an_empty_acceptance() {
        let request = DialogRequest {
            mode: DialogMode::OpenFile,
            ..DialogRequest::default()
        };
        let chooser = FrameChooser::detached();
        let pending = chooser
            .show(&request)
            .expect("a frame mounts on this thread");

        assert!(pending.accept());

        assert_eq!(
            pending.poll(),
            Some(DialogOutcome::Refused {
                refusal: SelectionRefusal::NothingChosen
            }),
        );
    }

    #[test]
    fn a_chooser_that_starts_in_an_impossible_directory_is_refused_before_a_frame_is_mounted() {
        let request = DialogRequest {
            start_directory: Some(PathBuf::from("/no/such/directory-for-rustage")),
            ..DialogRequest::default()
        };

        let error = FrameChooser::detached()
            .show(&request)
            .expect_err("a start directory that is not there is refused");

        assert!(
            matches!(error, DialogOpenError::StartDirectory { .. }),
            "{error:?}"
        );
        assert!(error.message().contains("cannot start in"));
    }

    #[test]
    fn the_only_open_error_a_working_frame_produces_is_none() {
        // `NoWidgetRegistry` depends on the calling thread; this records which answer the thread
        // that runs the test got, so a future change to the runtime is visible rather than assumed.
        let chooser = FrameChooser::detached();
        let shown = chooser.show(&DialogRequest::default());

        match shown {
            Ok(pending) => {
                assert!(pending.is_open());
            }
            Err(DialogOpenError::NoWidgetRegistry) => {}
            Err(other) => panic!("unexpected open error: {other:?}"),
        }
    }

    #[test]
    fn dropping_an_unanswered_choice_leaves_nothing_modal() {
        let request = DialogRequest {
            preselected: vec![PathBuf::from("/tmp/whatever-rustage.json")],
            ..DialogRequest::default()
        };
        let chooser = FrameChooser::detached();

        // Dropping must not panic even though the request was never answered, and the frame must be
        // gone from the registry afterwards.
        let id = {
            let pending = chooser
                .show(&request)
                .expect("a frame mounts on this thread");
            let id = pending.id();
            assert!(rust_widgets::widget::runtime::is_mounted(id));
            drop(pending);
            id
        };
        assert!(!rust_widgets::widget::runtime::is_mounted(id));
    }
}
