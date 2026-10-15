// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The buffer over a page document (task T03.01): the text someone typed, the last model that
//! could be read from it, and the two versions that keep those apart.
//!
//! The acceptance line for this task is "invalid text is kept, and the last valid model can still
//! be previewed". Both halves are structural here rather than a habit:
//!
//! * **The text is what it is.** A [`Buffer`] holds the text it was given, character for character.
//!   It never trims it, never repairs it, and never replaces it with the model re-rendered (which
//!   would silently throw away the formatting, the unknown members and the comments a person typed).
//!   A text that does not read is still the text, and it is what a later save would write.
//! * **The model is the last one that read.** There is no way to ask a buffer for "the model" without
//!   also getting the text version it came from: the state is either [`State::Reads`], where the
//!   model *is* that text's model, or [`State::TextDoesNotRead`], where the newest text did not read
//!   and the model carried in the same value is the last one that did — older than the text, and
//!   saying so. A failed read cannot clear it, because clearing is not one of the two shapes.
//!
//! Two versions, and they are two on purpose. [`TextVersion`] counts edits of the text; the model's
//! version counts *successful* reads. An edit that does not read moves the first and leaves the
//! second where it was, which is exactly the "the preview is behind the text" fact — expressed as
//! two numbers rather than as a flag somebody has to remember to set.
//!
//! The model's version is deliberately **not** [`Revision`](crate::contracts::revision::Revision):
//! that one belongs to the editable store (T00.05), which is the single place a node or a property
//! can change. A buffer holding the file's model is a different thing from the store holding the
//! live document, and borrowing the store's sequence here would be a second writer of it. What the
//! buffer *does* share with the store is identity: the ids in the text are the ids in the model, and
//! [`Buffer::store`] hands the page and its root to a [`DocumentStore`] with those same ids (the
//! remaining nodes arrive through change sets, which is T03.02's subject).
//!
//! Identity is an observation here, not a veto. A buffer reports what happened to each id — kept,
//! appeared, departed, came back, or came back carrying a different kind — because those are facts
//! about the file somebody edited, and refusing the read would be this build inventing policy about
//! what an editor may contain. What it does *not* do is re-label silently: an id that returns with a
//! different kind is `relabelled` in the report, and two nodes with one id is a text that does not
//! read at all (a model cannot hold one identity twice; the reader refuses it, and the refusal
//! travels with its reason).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use super::format::{PROJECT_FILE_NAME, ProjectError};
use super::page::PageDocument;
use crate::contracts::identity::NodeId;
use crate::contracts::model::{Node, NodeKind, Page};
use crate::contracts::permission::PermissionGate;
use crate::contracts::store::DocumentStore;
use crate::host::paths::ProjectRoot;

/// How many times the text has been replaced.
///
/// The first text a buffer holds is version 0; the acceptance that "the text is kept" is checked
/// against this number, because it is what says the newest edit is the newest text.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct TextVersion(u64);

impl TextVersion {
    /// The text a buffer is opened with.
    pub const FIRST: Self = Self(0);

    /// The number this version is written as.
    pub const fn get(self) -> u64 {
        self.0
    }

    const fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

impl std::fmt::Display for TextVersion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "#{}", self.0)
    }
}

/// The version of the model a buffer holds.
///
/// It counts reads that succeeded, so two edits that read produce two models while one edit that
/// does not read produces none. It is not a `Revision`: see the module documentation.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct ModelVersion(u64);

impl ModelVersion {
    /// The number this version is written as.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Why a text is not a model, or why a file is not a text.
///
/// One type for both because a caller that asks "can I preview this?" wants one answer, and each
/// variant is a different *reason* rather than a different kind of answer. What they have in common
/// is that the reason is carried: `not JSON` is not the same finding as `JSON that is not a page
/// document`, and neither is the same as `bytes this build cannot hold as text`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Refusal {
    /// The text is not JSON, and the reason says where it stopped.
    NotJson {
        /// Where and why, in the reader's own words.
        reason: String,
    },
    /// The text is JSON but not a page document: a missing member, a wrong type, a duplicate node
    /// identity, a child or root no node declares.
    NotAPageDocument {
        /// What the reader objected to.
        reason: String,
    },
    /// The file's bytes are not text this build can hold, so there is no text to keep.
    NotText {
        /// Why the bytes were refused.
        reason: String,
    },
    /// The file could not be read at all.
    Unreadable {
        /// Why.
        reason: String,
    },
    /// No project root was found for the file.
    NoProjectRoot {
        /// The directory the walk started at.
        searched: PathBuf,
    },
}

impl Refusal {
    /// The refusal in one sentence, with the reason it was refused.
    pub fn message(&self) -> String {
        match self {
            Self::NotJson { reason } => {
                format!("the text is not JSON: {reason}")
            }
            Self::NotAPageDocument { reason } => {
                format!("the text is JSON but not a page document: {reason}")
            }
            Self::NotText { reason } => {
                format!("the file is not text this build holds, so nothing was read: {reason}")
            }
            Self::Unreadable { reason } => format!("the file could not be read: {reason}"),
            Self::NoProjectRoot { searched } => format!(
                "there is no {PROJECT_FILE_NAME} in {} or above it, so this file belongs to no \
                 project",
                searched.display()
            ),
        }
    }

    /// The read of a text, as the reader described it.
    fn of_reading(error: &ProjectError) -> Self {
        match error {
            // The reader's own words, with the position it stopped at: "line 3, column 5: …" is
            // what a person can act on, and paraphrasing it here would lose the only part that says
            // *where*.
            ProjectError::NotJson { error, .. } => Self::NotJson {
                reason: error.to_string(),
            },
            other => Self::NotAPageDocument {
                reason: other.message(),
            },
        }
    }
}

/// The last model a text read as, with the versions it belongs to.
#[derive(Clone, PartialEq, Debug)]
pub struct Model {
    document: PageDocument,
    version: ModelVersion,
    text: TextVersion,
}

impl Model {
    /// The page document as it was read.
    pub fn document(&self) -> &PageDocument {
        &self.document
    }

    /// The version of this model.
    pub const fn version(&self) -> ModelVersion {
        self.version
    }

    /// The text it was read from.
    pub const fn text(&self) -> TextVersion {
        self.text
    }

    /// The page.
    pub fn page(&self) -> &Page {
        self.document.page()
    }

    /// Whether it is behind a text that has been typed since.
    pub fn is_behind(&self, typed: TextVersion) -> bool {
        self.text < typed
    }

    /// One line for the page, then one per node, in identity order.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "page `{}` ({}), root node #{}",
            self.page().name(),
            self.page().id().get(),
            self.page().root().get()
        )];
        for node in self.document.nodes() {
            let children = if node.children().is_empty() {
                "no children".to_owned()
            } else {
                format!(
                    "children [{}]",
                    node.children()
                        .iter()
                        .map(|id| id.get().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            lines.push(format!(
                "  node #{} {:<16} {children}, {} propert{}",
                node.id().get(),
                node.kind().as_str(),
                node.properties().len(),
                if node.properties().len() == 1 {
                    "y"
                } else {
                    "ies"
                }
            ));
        }
        lines
    }
}

/// What a buffer is holding, in one of exactly two shapes.
///
/// The two shapes are the whole point: there is no third value meaning "the model, but the text may
/// be broken", because that value is how a stale preview ends up being read as a current one.
#[derive(Clone, PartialEq, Debug)]
pub enum State {
    /// The newest text reads, and this is its model.
    Reads {
        /// The model, which came from the newest text.
        model: Model,
    },
    /// The newest text does not read, and the model is the last one that did.
    TextDoesNotRead {
        /// Why the newest text is not a model.
        refusal: Refusal,
        /// The last model that did read, when there was one.
        model: Option<Model>,
        /// The text version that did not read.
        text: TextVersion,
    },
}

impl State {
    /// The model that can still be previewed, and the last one that read.
    pub fn model(&self) -> Option<&Model> {
        match self {
            Self::Reads { model } => Some(model),
            Self::TextDoesNotRead { model, .. } => model.as_ref(),
        }
    }

    /// Whether the model is the newest text's own.
    pub const fn is_current(&self) -> bool {
        matches!(self, Self::Reads { .. })
    }

    /// Why the newest text does not read, when it does not.
    pub fn refusal(&self) -> Option<&Refusal> {
        match self {
            Self::Reads { .. } => None,
            Self::TextDoesNotRead { refusal, .. } => Some(refusal),
        }
    }

    /// The text version this state is about.
    pub const fn text(&self) -> TextVersion {
        match self {
            Self::Reads { model } => model.text(),
            Self::TextDoesNotRead { text, .. } => *text,
        }
    }

    /// The state in one sentence, then the preview when there is a model to preview.
    pub fn render(&self) -> Vec<String> {
        let mut lines = match self {
            Self::Reads { model } => vec![format!(
                "text #{}, model m{}: the newest text reads, and this is its model",
                model.text().get(),
                model.version().get()
            )],
            Self::TextDoesNotRead {
                refusal,
                model,
                text,
            } => {
                let mut lines = vec![format!("text #{}: {}", text.get(), refusal.message())];
                match model {
                    Some(model) if model.is_behind(*text) => lines.push(format!(
                        "the model m{} from text #{} is still there, and it is behind the text: what \
                         follows is that model, not the newest text",
                        model.version().get(),
                        model.text().get()
                    )),
                    Some(model) => lines.push(format!(
                        "the model m{} is the model of this text: what follows is that model",
                        model.version().get()
                    )),
                    None => lines
                        .push("no text has read yet, so there is no model to preview".to_owned()),
                }
                lines
            }
        };
        if let Some(model) = self.model() {
            lines.extend(model.render());
        }
        lines
    }
}

/// What happened to each identity between the previous model and the newest one.
///
/// These are observations about the text somebody edited, reported rather than policed: an id that
/// comes back is a fact, and a node silently re-labelled under one id is a fact that has to be said
/// out loud. `relabelled` is the one that matters most — it means an identity now names a different
/// kind of node than it did, which every reference to that id in the project would still be pointing
/// at.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Identities {
    live: Vec<NodeId>,
    appeared: Vec<NodeId>,
    departed: Vec<NodeId>,
    returned: Vec<NodeId>,
    relabelled: Vec<(NodeId, NodeKind, NodeKind)>,
}

impl Identities {
    /// The ids the newest model holds, in order.
    pub fn live(&self) -> &[NodeId] {
        &self.live
    }

    /// The ids that are in the newest model and were in none before it.
    pub fn appeared(&self) -> &[NodeId] {
        &self.appeared
    }

    /// The ids that were in the previous model and are in none now.
    pub fn departed(&self) -> &[NodeId] {
        &self.departed
    }

    /// The ids that were live earlier in this buffer, went away, and are back. A returned id is not
    /// an appeared one: the difference is whether this buffer has seen it before.
    pub fn returned(&self) -> &[NodeId] {
        &self.returned
    }

    /// The ids whose kind changed: one identity, two different nodes.
    pub fn relabelled(&self) -> &[(NodeId, NodeKind, NodeKind)] {
        &self.relabelled
    }

    /// Whether nothing about identity changed.
    pub fn is_empty(&self) -> bool {
        self.appeared.is_empty()
            && self.departed.is_empty()
            && self.returned.is_empty()
            && self.relabelled.is_empty()
    }

    /// One line per kind of change, and a line saying so when there was none.
    pub fn render(&self) -> Vec<String> {
        if self.is_empty() {
            return vec![format!(
                "identity: the same {} node(s), with the same ids",
                self.live.len()
            )];
        }
        let mut lines = Vec::new();
        for (label, ids) in [
            ("appeared", &self.appeared),
            ("departed", &self.departed),
            ("returned", &self.returned),
        ] {
            if !ids.is_empty() {
                lines.push(format!(
                    "identity {label}: {}",
                    ids.iter()
                        .map(|id| format!("#{}", id.get()))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }
        for (id, from, to) in &self.relabelled {
            lines.push(format!(
                "identity relabelled: #{} was {} and is now {}",
                id.get(),
                from.as_str(),
                to.as_str()
            ));
        }
        lines
    }
}

/// One read: the document, and what it did to identity.
struct Read {
    document: PageDocument,
    identities: Identities,
    kinds: BTreeMap<NodeId, NodeKind>,
    seen: BTreeSet<NodeId>,
}

/// A page document open for editing: the text, and the last model that read from it.
#[derive(Debug)]
pub struct Buffer {
    path: PathBuf,
    text: String,
    typed: TextVersion,
    read: u64,
    state: State,
    /// The bytes the file held when this buffer read it, when it was opened from a file.
    ///
    /// It is what makes an external change detectable: a save compares the file against this, so a
    /// change somebody else made is refused rather than silently overwritten (T03.05's guard). A
    /// buffer built from text alone (a paste, a test) has no file it agreed with, and holds `None`.
    on_disk: Option<Vec<u8>>,
    /// The kind each live id had, which is what makes a re-labelling visible.
    kinds: BTreeMap<NodeId, NodeKind>,
    /// The ids this buffer has seen, in the order it first saw them.
    seen: BTreeSet<NodeId>,
    identities: Identities,
}

impl Buffer {
    /// Opens the page document at `relative` inside a project root.
    ///
    /// The file's bytes have to be text this build can hold; that is the only failure that happens
    /// here. A file whose *text* is not a page document opens anyway, with the text and no model,
    /// because keeping what somebody wrote is the point — sending it back as an error would mean the
    /// caller has nowhere to put it.
    pub fn open(root: &ProjectRoot, relative: impl AsRef<Path>) -> Result<Self, Refusal> {
        let relative = relative.as_ref();
        let path = root
            .resolve(relative)
            .map_err(|error| Refusal::Unreadable {
                reason: error.message(),
            })?;
        let bytes = std::fs::read(&path).map_err(|error| Refusal::Unreadable {
            reason: error.to_string(),
        })?;
        let text = String::from_utf8(bytes.clone()).map_err(|error| Refusal::NotText {
            reason: error.to_string(),
        })?;
        Ok(Self::of_text(root, path, text).remembering(bytes))
    }

    /// A buffer over text that was already obtained.
    ///
    /// Infallible by design: the text is kept whatever it says, and whether it reads is the buffer's
    /// state rather than this function's result.
    pub fn of_text(root: &ProjectRoot, path: impl Into<PathBuf>, text: impl Into<String>) -> Self {
        let path = path.into();
        let text = text.into();
        let mut buffer = Self {
            path: path.clone(),
            text: String::new(),
            typed: TextVersion::FIRST,
            read: 0,
            state: State::TextDoesNotRead {
                refusal: Refusal::NotJson {
                    reason: "nothing has been read yet".to_owned(),
                },
                model: None,
                text: TextVersion::FIRST,
            },
            on_disk: None,
            kinds: BTreeMap::new(),
            seen: BTreeSet::new(),
            identities: Identities::default(),
        };
        // The text a buffer is opened with is text #0: it is not an edit, it is where editing starts.
        match interpret(root, &path, &text, None, &buffer.seen) {
            Ok(read) => {
                buffer.adopt(&read);
                // The text a buffer opens with is model 0; the next read that succeeds is model 1.
                buffer.read = 1;
                buffer.state = State::Reads {
                    model: Model {
                        document: read.document,
                        version: ModelVersion(0),
                        text: TextVersion::FIRST,
                    },
                };
            }
            Err(refusal) => {
                buffer.state = State::TextDoesNotRead {
                    refusal,
                    model: None,
                    text: TextVersion::FIRST,
                };
            }
        }
        buffer.text = text;
        buffer
    }

    /// The bytes this buffer last agreed with the file, when it was opened from one.
    ///
    /// A save compares the file against these before it writes, so an edit made outside the
    /// designer is refused rather than overwritten — the guard T03.05 asks for, kept next to the
    /// text it belongs to rather than in a second place that could drift.
    pub fn on_disk(&self) -> Option<&[u8]> {
        self.on_disk.as_deref()
    }

    /// Records the bytes the file held, which is what an external change is measured against.
    fn remembering(mut self, bytes: Vec<u8>) -> Self {
        self.on_disk = Some(bytes);
        self
    }

    /// The file this buffer is over.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The text, exactly as it was given.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// How many times the text has been replaced.
    pub const fn typed(&self) -> TextVersion {
        self.typed
    }

    /// What the buffer is holding.
    pub fn state(&self) -> &State {
        &self.state
    }

    /// The last model that read, when one did.
    pub fn model(&self) -> Option<&Model> {
        self.state.model()
    }

    /// What happened to identity in the newest read.
    pub fn identities(&self) -> &Identities {
        &self.identities
    }

    /// Every id this buffer has seen live, in the order it first saw them.
    ///
    /// A buffer never issues an id: the ids are the text's, and this is the record of which ones
    /// have been live. That record is what makes a returning id a *return* rather than an
    /// appearance, and it is the only place that fact is kept.
    pub fn seen(&self) -> Vec<NodeId> {
        self.seen.iter().copied().collect()
    }

    /// How many ids this buffer knows about.
    pub fn known_ids(&self) -> usize {
        self.seen.len()
    }

    /// Replaces the text, and reads it.
    ///
    /// One entry point for an edit, and it always does the same three things: the text becomes the
    /// newest one (so the version moves whether or not it reads), the read is attempted, and the
    /// state records what happened. A text that does not read does **not** roll the text back to the
    /// last readable one: that would be the buffer quietly deleting what somebody typed.
    pub fn set_text(&mut self, root: &ProjectRoot, text: impl Into<String>) {
        let text = text.into();
        let previous = self.state.model().cloned();
        self.typed = self.typed.next();
        match interpret(root, &self.path, &text, Some(&self.kinds), &self.seen) {
            Ok(read) => {
                self.adopt(&read);
                let version = ModelVersion(self.read);
                self.read += 1;
                self.state = State::Reads {
                    model: Model {
                        document: read.document,
                        version,
                        text: self.typed,
                    },
                };
            }
            Err(refusal) => {
                self.state = State::TextDoesNotRead {
                    refusal,
                    model: previous,
                    text: self.typed,
                };
            }
        }
        self.text = text;
    }

    /// The page and its root node as a [`DocumentStore`], with the ids the text gave them.
    ///
    /// The store is the editable model (T00.05) and this is the seam onto it: the page and the root
    /// arrive here, and the rest of the nodes arrive through change sets — which is T03.02's subject,
    /// because changing a node means changing it through an inverse-carrying change set rather than
    /// by re-reading a file. `None` when nothing has read yet: a store built from no model would be
    /// an empty document pretending to be this file's.
    pub fn store(&self, gate: PermissionGate) -> Option<DocumentStore> {
        let model = self.model()?;
        let page = model.page().clone();
        let root = model.document().root_node()?;
        let node = Node::new(page.id(), root.id(), root.kind().clone());
        DocumentStore::new(gate, page, node)
    }

    /// The buffer in one report: the text, the state, and what happened to identity.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![
            format!("document: {}", self.path.display()),
            format!(
                "text #{}: {} character(s), {} line(s), kept exactly as it was given",
                self.typed.get(),
                self.text.chars().count(),
                self.text.lines().count()
            ),
        ];
        lines.extend(self.state.render());
        lines.extend(self.identities.render());
        lines.push(format!(
            "identity: {} id(s) known to this buffer, none of them handed out a second time",
            self.known_ids()
        ));
        lines
    }

    fn adopt(&mut self, read: &Read) {
        self.kinds = read.kinds.clone();
        self.seen = read.seen.clone();
        self.identities = read.identities.clone();
    }
}

/// Reads a text, and works out what it did to identity.
///
/// The reader is the page reader ([`PageDocument::parse`]), so "what is a page document" has one
/// answer in this build rather than two. What this adds is identity: which ids are live now, which
/// came, which went, which came back, and which came back different.
///
/// `before` is the previous model's ids, and it is `None` for a buffer's first read: nothing
/// "appeared" when a buffer opened, because there was no earlier model for it to appear beside. The
/// ids are still recorded as seen, so a later return is a return.
fn interpret(
    root: &ProjectRoot,
    path: &Path,
    text: &str,
    before: Option<&BTreeMap<NodeId, NodeKind>>,
    seen: &BTreeSet<NodeId>,
) -> Result<Read, Refusal> {
    let document =
        PageDocument::parse(root, path, text).map_err(|error| Refusal::of_reading(&error))?;

    let mut live = BTreeSet::new();
    let mut now = BTreeMap::new();
    for node in document.nodes() {
        live.insert(node.id());
        now.insert(node.id(), node.kind().clone());
    }

    let appeared: Vec<NodeId> = live
        .iter()
        .filter(|id| before.is_some_and(|kinds| !kinds.contains_key(*id)))
        .copied()
        .collect();
    let returned: Vec<NodeId> = appeared
        .iter()
        .filter(|id| seen.contains(*id))
        .copied()
        .collect();
    let appeared: Vec<NodeId> = appeared
        .iter()
        .filter(|id| !returned.contains(*id))
        .copied()
        .collect();
    let departed: Vec<NodeId> = before
        .into_iter()
        .flatten()
        .filter(|(id, _)| !live.contains(*id))
        .map(|(id, _)| *id)
        .collect();
    let relabelled: Vec<(NodeId, NodeKind, NodeKind)> = now
        .iter()
        .filter_map(|(id, kind)| {
            before
                .and_then(|kinds| kinds.get(id))
                .filter(|was| **was != *kind)
                .map(|was| (*id, was.clone(), kind.clone()))
        })
        .collect();

    let mut seen_now = seen.clone();
    for id in live.iter().copied() {
        seen_now.insert(id);
    }

    Ok(Read {
        document,
        identities: Identities {
            live: live.iter().copied().collect(),
            appeared,
            departed,
            returned,
            relabelled,
        },
        kinds: now,
        seen: seen_now,
    })
}

/// The project root a file belongs to: the nearest directory that holds a project document.
///
/// A page document is read against a root because a property may hold a project-relative path; the
/// file itself does not say which project it is in, so the answer comes from the directory tree. A
/// file that is in no project is refused rather than read against its own directory, which would
/// resolve every reference against the wrong place and quietly disagree with the project's own file.
pub fn root_of(path: &Path) -> Result<ProjectRoot, Refusal> {
    let start = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    };
    let mut directory = Some(start.as_path());
    while let Some(candidate) = directory {
        if candidate.join(PROJECT_FILE_NAME).is_file() {
            return ProjectRoot::new(candidate).map_err(|error| Refusal::Unreadable {
                reason: error.message(),
            });
        }
        directory = candidate.parent();
    }
    Err(Refusal::NoProjectRoot { searched: start })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::{Buffer, ModelVersion, Refusal, State, TextVersion, root_of};
    use crate::contracts::permission::PermissionGate;
    use crate::host::paths::ProjectRoot;

    struct TempDirectory(PathBuf);

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct Sandbox {
        root: PathBuf,
        _guard: TempDirectory,
    }

    fn sandbox() -> Sandbox {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("rustage-document-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&root).expect("a temporary directory is creatable");
        Sandbox {
            root: root.clone(),
            _guard: TempDirectory(root),
        }
    }

    impl Sandbox {
        fn write(&self, relative: &str, text: &str) -> PathBuf {
            let path = self.root.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("a directory");
            }
            std::fs::write(&path, text).expect("a file");
            path
        }

        fn project(&self) -> ProjectRoot {
            self.write("project.json", "{\"format\": 2}");
            ProjectRoot::new(&self.root).expect("a project root")
        }
    }

    /// A page with two children, one of which has a child of its own.
    ///
    /// The shape is the page reader's: `properties` is an array of `name`/`value` entries and a
    /// value names exactly one family (see `project::page`).
    fn page_text() -> String {
        "{\"format\": 1, \"page\": {\"id\": 7, \"name\": \"Main\", \"root\": 0}, \
          \"nodes\": [ \
            {\"id\": 0, \"type\": \"Frame\", \"children\": [1, 2], \
             \"properties\": [{\"name\": \"width\", \"value\": {\"number\": 640}}]}, \
            {\"id\": 1, \"type\": \"Label\", \"children\": [], \
             \"properties\": [{\"name\": \"text\", \"value\": {\"text\": \"hello\"}}]}, \
            {\"id\": 2, \"type\": \"Column\", \"children\": [3], \"properties\": []}, \
            {\"id\": 3, \"type\": \"Button\", \"children\": [], \"properties\": []} \
          ]}"
        .to_owned()
    }

    fn buffer(sandbox: &Sandbox) -> Buffer {
        let root = sandbox.project();
        Buffer::of_text(&root, sandbox.root.join("pages/main.json"), page_text())
    }

    #[test]
    fn a_text_that_reads_is_its_own_model_and_the_two_versions_are_two() {
        let sandbox = sandbox();
        let mut buffer = buffer(&sandbox);

        assert_eq!(buffer.typed(), TextVersion::FIRST);
        assert!(buffer.state().is_current(), "{:?}", buffer.state().render());
        assert_eq!(buffer.state().model().expect("a model").version().get(), 0);
        assert_eq!(buffer.state().model().expect("a model").text().get(), 0);
        assert_eq!(
            buffer.state().model().expect("a model").page().name(),
            "Main"
        );
        assert_eq!(
            buffer
                .state()
                .model()
                .expect("a model")
                .document()
                .nodes()
                .len(),
            4
        );
        assert!(buffer.identities().is_empty());
        assert_eq!(buffer.identities().live().len(), 4);
        assert_eq!(buffer.known_ids(), 4);

        // An edit that reads: the text version moves, and so does the model's.
        let root = sandbox.project();
        buffer.set_text(&root, page_text());
        assert_eq!(buffer.typed().get(), 1);
        let model = buffer.state().model().expect("a model");
        assert_eq!(model.version().get(), 1);
        assert_eq!(model.text().get(), 1);
        assert!(!model.is_behind(buffer.typed()));
        // The ids are the ones the text declares, and an identical edit changed none of them.
        assert_eq!(
            buffer.model().expect("a model").document().nodes().len(),
            buffer.identities().live().len()
        );
        assert!(buffer.identities().is_empty());
    }

    #[test]
    fn an_invalid_text_is_kept_and_the_last_model_still_previews() {
        let sandbox = sandbox();
        let mut buffer = buffer(&sandbox);
        let root = sandbox.project();
        let readable = buffer.model().expect("a model").clone();
        let typed_ids: Vec<u64> = buffer
            .identities()
            .live()
            .iter()
            .map(|id| id.get())
            .collect();

        // A text with a syntax error in the middle: what typing a bracket too many looks like.
        let broken = page_text().replace("\"Frame\"", "\"Frame\",");
        buffer.set_text(&root, &broken);

        assert!(!buffer.state().is_current());
        assert_eq!(
            buffer.text(),
            broken,
            "the text is kept exactly as it was given"
        );
        assert_eq!(buffer.typed().get(), 1);
        match buffer.state() {
            State::TextDoesNotRead {
                refusal,
                model,
                text,
            } => {
                assert_eq!(*text, buffer.typed());
                assert!(
                    matches!(refusal, Refusal::NotJson { .. }),
                    "a syntax error is a JSON refusal: {refusal:?}"
                );
                assert!(refusal.message().starts_with("the text is not JSON:"));
                assert_eq!(model.as_ref(), Some(&readable));
            }
            other => panic!("the newest text does not read: {other:?}"),
        }
        // The preview is the last model, and it says which text it came from and that it is behind.
        let preview = buffer.state().render().join("\n");
        assert!(
            preview.contains("the model m0 from text #0 is still there"),
            "{preview}"
        );
        assert!(preview.contains("behind the text"), "{preview}");
        assert!(preview.contains("page `Main`"), "{preview}");
        assert!(preview.contains("node #3 Button"), "{preview}");
        // Nothing about identity changed, because nothing about the model did.
        assert!(buffer.identities().is_empty());
        assert_eq!(
            buffer
                .identities()
                .live()
                .iter()
                .map(|id| id.get())
                .collect::<Vec<_>>(),
            typed_ids
        );
        // And the model's version is where it was: an edit that did not read is not a model.
        assert_eq!(buffer.model().expect("a model").version(), ModelVersion(0));

        // A text that reads again moves the model on, and it is the newest text's own.
        buffer.set_text(&root, page_text());
        assert!(buffer.state().is_current());
        assert_eq!(buffer.state().model().expect("a model").version().get(), 1);
        assert_eq!(buffer.state().model().expect("a model").text().get(), 2);
    }

    #[test]
    fn the_buffer_never_rewrites_the_text_it_holds() {
        let sandbox = sandbox();
        let root = sandbox.project();
        // A text a person would write: extra spacing, a member this build does not know, an
        // unknown property on a node. Every one of those is something a re-render would drop.
        let typed = "{\n  \"format\": 1,\n  \"page\": {\"id\": 7, \"name\": \"Main\", \
                     \"root\": 0, \"note\": \"mine\"},\n  \"nodes\": [\n    \
                     {\"id\": 0, \"type\": \"Frame\", \"children\": [9], \
                     \"properties\": []},\n    {\"id\": 9, \"type\": \"Label\", \
                     \"children\": [], \"properties\": [], \"comment\": \"keep\"}\n  ]\n}\n";
        let mut buffer = Buffer::of_text(&root, sandbox.root.join("pages/main.json"), typed);

        assert!(buffer.state().is_current());
        assert_eq!(
            buffer.text(),
            typed,
            "the text is kept character for character"
        );
        assert!(buffer.known_ids() >= 2);
        // The model is the reader's interpretation, and it is *not* what the buffer holds as text:
        // the model's own rendering would not have the spacing or the unknown members in it.
        let rendered = buffer
            .model()
            .expect("a model")
            .document()
            .render()
            .expect("a rendering");
        assert_ne!(
            rendered, typed,
            "the model's own spelling is not the text's"
        );
        assert!(rendered.starts_with('{'), "{rendered}");
        assert!(
            buffer.text().contains("\n  \"page\""),
            "the text keeps its own lines"
        );
        // The unknown members are kept by the *model* as well — that is the page reader's job, not
        // this buffer's — so "the text is what it is" is about spelling, not about content.
        assert!(rendered.contains("\"note\""));
        assert!(buffer.text().contains("\"comment\": \"keep\""));

        // A text that does not read is kept just as exactly, and a text that does read after it
        // replaces it as exactly.
        let broken =
            "{\"format\": 1, \"page\": {\"id\": 7, \"name\": \"\", \"root\": 0}, \"nodes\": []}";
        buffer.set_text(&root, broken);
        assert_eq!(buffer.text(), broken);
        assert!(!buffer.state().is_current());
        assert!(buffer.state().refusal().is_some());
        // It is JSON: it is the page that is wrong, and the refusal says which one it is. A save
        // (T03.04) writes this text, whatever it says, because it is the same string.
        assert!(matches!(
            buffer.state().refusal(),
            Some(Refusal::NotAPageDocument { .. })
        ));
    }

    #[test]
    fn an_edit_that_does_not_touch_a_node_keeps_its_id() {
        let sandbox = sandbox();
        let mut buffer = buffer(&sandbox);
        let root = sandbox.project();
        let before: Vec<(u64, String)> = buffer
            .model()
            .expect("a model")
            .document()
            .nodes()
            .iter()
            .map(|node| (node.id().get(), node.kind().as_str().to_owned()))
            .collect();

        // A property changes and a child moves; nothing is added or removed.
        let moved = page_text()
            .replace("\"number\": 640", "\"number\": 800")
            .replace("\"children\": [1, 2]", "\"children\": [2, 1]");
        buffer.set_text(&root, &moved);

        assert!(buffer.state().is_current());
        let after: Vec<(u64, String)> = buffer
            .model()
            .expect("a model")
            .document()
            .nodes()
            .iter()
            .map(|node| (node.id().get(), node.kind().as_str().to_owned()))
            .collect();
        assert_eq!(
            before, after,
            "identity is the text's, and the edit kept it"
        );
        assert!(buffer.identities().is_empty());
        assert!(
            buffer.identities().render()[0].contains("the same 4 node(s)"),
            "{:?}",
            buffer.identities().render()
        );
    }

    #[test]
    fn identity_that_comes_and_goes_is_reported_rather_than_policed() {
        let sandbox = sandbox();
        let mut buffer = buffer(&sandbox);
        let root = sandbox.project();

        // A node goes away: its id departs, and the child it took with it departs with it.
        // A node and its child go away, and the reference to the node goes with them: a child no
        // node declares is a text that does not read at all, which is a different case (tested as one).
        let without = page_text()
            .replace(
                "{\"id\": 2, \"type\": \"Column\", \"children\": [3], \"properties\": []}, ",
                "",
            )
            .replace(
                ", {\"id\": 3, \"type\": \"Button\", \"children\": [], \"properties\": []}",
                "",
            )
            .replace("\"children\": [1, 2]", "\"children\": [1]");
        buffer.set_text(&root, &without);
        assert!(buffer.state().is_current(), "{:?}", buffer.state().render());
        let departed: Vec<u64> = buffer
            .identities()
            .departed()
            .iter()
            .map(|id| id.get())
            .collect();
        assert_eq!(departed, vec![2, 3]);
        assert!(buffer.identities().render()[0].contains("departed"));
        assert_eq!(
            buffer.known_ids(),
            4,
            "the ids stay known, so they are not handed out again"
        );
        assert_eq!(buffer.seen().len(), 4);

        // It comes back: that is a return, not an appearance, because this buffer has seen it.
        buffer.set_text(&root, page_text());
        assert!(buffer.state().is_current());
        let returned: Vec<u64> = buffer
            .identities()
            .returned()
            .iter()
            .map(|id| id.get())
            .collect();
        assert_eq!(returned, vec![2, 3]);
        assert!(buffer.identities().appeared().is_empty());
        assert!(buffer.identities().departed().is_empty());
        assert!(buffer.identities().render()[0].contains("returned"));

        // A genuinely new node appears under a new id.
        let with_new = page_text()
            .replace(
                "{\"id\": 3, \"type\": \"Button\", \"children\": [], \"properties\": []}",
                "{\"id\": 3, \"type\": \"Button\", \"children\": [], \"properties\": []}, \
                 {\"id\": 4, \"type\": \"Label\", \"children\": [], \"properties\": []}",
            )
            // A node nothing reaches is not part of the tree, and the reader says so.
            .replace("\"children\": [1, 2]", "\"children\": [1, 2, 4]");
        buffer.set_text(&root, &with_new);
        let appeared: Vec<u64> = buffer
            .identities()
            .appeared()
            .iter()
            .map(|id| id.get())
            .collect();
        assert_eq!(appeared, vec![4]);
        assert!(buffer.identities().returned().is_empty());
        assert_eq!(buffer.known_ids(), 5);

        // And an id that comes back as a different node is said out loud rather than accepted
        // quietly: every reference to that id in the project still points at it.
        let relabelled = page_text().replace(
            "{\"id\": 3, \"type\": \"Button\", \"children\": [], \"properties\": []}",
            "{\"id\": 3, \"type\": \"Slider\", \"children\": [], \"properties\": []}",
        );
        buffer.set_text(&root, &relabelled);
        assert!(buffer.state().is_current(), "{:?}", buffer.state().render());
        let relabelled = buffer.identities().relabelled().to_vec();
        assert_eq!(relabelled.len(), 1);
        assert_eq!(relabelled[0].0.get(), 3);
        assert_eq!(relabelled[0].1.as_str(), "Button");
        assert_eq!(relabelled[0].2.as_str(), "Slider");
        assert!(
            buffer
                .identities()
                .render()
                .iter()
                .any(|line| line.contains("#3 was Button and is now Slider")),
            "{:?}",
            buffer.identities().render()
        );
    }

    #[test]
    fn a_text_that_gives_one_identity_twice_does_not_read() {
        let sandbox = sandbox();
        let mut buffer = buffer(&sandbox);
        let root = sandbox.project();
        let readable = buffer.model().expect("a model").clone();

        let twice = page_text().replace(
            "{\"id\": 3, \"type\": \"Button\", \"children\": [], \"properties\": []}",
            "{\"id\": 3, \"type\": \"Button\", \"children\": [], \"properties\": []}, \
             {\"id\": 3, \"type\": \"Label\", \"children\": [], \"properties\": []}",
        );
        buffer.set_text(&root, &twice);

        assert!(!buffer.state().is_current());
        let refusal = buffer.state().refusal().expect("a refusal");
        assert!(
            matches!(refusal, Refusal::NotAPageDocument { .. }),
            "one identity twice is not a JSON problem: {refusal:?}"
        );
        assert!(
            refusal
                .message()
                .contains("the text is JSON but not a page document"),
            "{}",
            refusal.message()
        );
        assert_eq!(buffer.state().model(), Some(&readable));
        assert_eq!(buffer.text(), twice);
    }

    #[test]
    fn a_file_that_is_not_text_is_not_a_buffer_and_a_file_that_is_text_is_one() {
        let sandbox = sandbox();
        let root = sandbox.project();
        // Bytes that are not UTF-8: there is no text to keep, so this is refused at the door, and
        // the file itself is not touched.
        let bytes = sandbox.write("pages/bytes.json", "");
        std::fs::write(&bytes, [0xff, 0xfe, 0xfd]).expect("bytes");
        let refusal = Buffer::open(&root, "pages/bytes.json").expect_err("not text");
        assert!(matches!(refusal, Refusal::NotText { .. }));
        assert!(refusal.message().contains("not text this build holds"));
        assert_eq!(
            std::fs::read(&bytes).expect("the bytes"),
            [0xff, 0xfe, 0xfd]
        );

        // A file whose text does not read *is* a buffer: the text is kept, and the reason is a state.
        sandbox.write("pages/broken.json", "{ nope");
        let buffer = Buffer::open(&root, "pages/broken.json").expect("a buffer");
        assert_eq!(buffer.text(), "{ nope");
        assert!(!buffer.state().is_current());
        assert!(matches!(
            buffer.state().refusal(),
            Some(Refusal::NotJson { .. })
        ));
        assert_eq!(buffer.model(), None);
        assert!(
            buffer.state().render()[1].contains("no text has read yet"),
            "{:?}",
            buffer.state().render()
        );

        // A file that is there with text that reads: the ordinary case.
        sandbox.write("pages/main.json", &page_text());
        let buffer = Buffer::open(&root, "pages/main.json").expect("a buffer");
        assert!(buffer.state().is_current());
        assert_eq!(buffer.model().expect("a model").document().nodes().len(), 4);

        // And a file that is not there is a refusal with the reason, not an empty buffer.
        let refusal = Buffer::open(&root, "pages/gone.json").expect_err("no file");
        assert!(matches!(refusal, Refusal::Unreadable { .. }));
    }

    #[test]
    fn a_file_is_read_against_the_project_it_is_in() {
        let sandbox = sandbox();
        sandbox.write("project.json", "{\"format\": 2}");
        sandbox.write("pages/main.json", &page_text());
        sandbox.write("pages/deep/other.json", &page_text());

        let root = root_of(&sandbox.root.join("pages/main.json")).expect("a root");
        assert_eq!(
            root.as_path(),
            std::fs::canonicalize(&sandbox.root).expect("the sandbox")
        );
        let deeper = root_of(&sandbox.root.join("pages/deep/other.json")).expect("a root");
        assert_eq!(deeper.as_path(), root.as_path());
        // A directory works too, and a file outside any project is refused rather than read against
        // its own directory.
        assert!(root_of(&sandbox.root).is_ok());
        let outside =
            std::env::temp_dir().join(format!("rustage-document-loose-{}", std::process::id()));
        std::fs::create_dir_all(&outside).expect("a directory");
        let refusal = root_of(&outside).expect_err("no project above it");
        let _ = std::fs::remove_dir_all(&outside);
        assert!(matches!(refusal, Refusal::NoProjectRoot { .. }));
        assert!(refusal.message().contains("belongs to no project"));
    }

    #[test]
    fn a_buffer_hands_its_last_model_to_the_store_with_the_same_identities() {
        let sandbox = sandbox();
        let mut buffer = buffer(&sandbox);
        let model = buffer.model().expect("a model").clone();
        let page_id = model.page().id();
        let root_id = model.document().page().root();

        let store = buffer
            .store(PermissionGate::new())
            .expect("the store the buffer hands over");
        assert_eq!(store.page(page_id).map(|page| page.name()), Some("Main"));
        let root = store.node(root_id).expect("the root node");
        assert_eq!(root.id(), root_id);
        assert_eq!(root.kind().as_str(), "Frame");
        assert_eq!(
            root.children(),
            &[],
            "the other nodes arrive through change sets"
        );
        assert!(buffer.identities().live().contains(&root_id));

        // An unreadable text does not take the last model away, so the store the buffer hands over
        // is still the last one that read — which is what "still previewable" means.
        let root = sandbox.project();
        let model = buffer.model().expect("a model").clone();
        buffer.set_text(&root, "{ nope");
        assert!(!buffer.state().is_current());
        assert_eq!(buffer.model(), Some(&model));
        assert!(buffer.store(PermissionGate::new()).is_some());

        // A buffer that never read has no model to hand over, and no store pretending to be one.
        let never = Buffer::of_text(&root, sandbox.root.join("pages/other.json"), "{ nope");
        assert_eq!(never.model(), None);
        assert!(never.store(PermissionGate::new()).is_none());
    }

    #[test]
    fn the_report_says_what_the_buffer_is_holding() {
        let sandbox = sandbox();
        let mut buffer = buffer(&sandbox);
        let root = sandbox.project();
        let reading = buffer.render().join("\n");
        assert!(
            reading.contains("kept exactly as it was given"),
            "{reading}"
        );
        assert!(reading.contains("the newest text reads"), "{reading}");
        assert!(reading.contains("page `Main`"), "{reading}");
        assert!(reading.contains("id(s) known to this buffer"), "{reading}");

        buffer.set_text(&root, "{\"format\": 1, \"page\": ");
        let broken = buffer.render().join("\n");
        assert!(broken.contains("text #1: the text is not JSON"), "{broken}");
        assert!(broken.contains("the model m0 from text #0"), "{broken}");
        assert!(broken.contains("page `Main`"), "{broken}");
        assert!(broken.contains("identity: the same 4 node(s)"), "{broken}");
    }
}
