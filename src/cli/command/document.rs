// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The document commands: the buffer over a page, its history, saving, conflicts,
//! recovery and closing.

// Every branch of the flag table answers with `return`, including the ones that could be a
// trailing expression: the branches are independent flags, and writing them alike is worth more
// than saving a keyword. The returns that are *not* needless — the ones inside a loop that refuses
// a bad value — are why the table reads the way it does.
#![allow(clippy::needless_return)]

use super::*;

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

/// Parses this family's flags, answering `None` when the flag is another family's.
pub(super) fn parse_document(
    text: &str,
    argument: OsString,
    arguments: &mut std::vec::IntoIter<OsString>,
) -> Option<Result<Command, ParseError>> {
    match text {
        "--edit" => {
            let Some(path) = arguments.next() else {
                return Some(Err(ParseError { argument }));
            };
            let mut options = EditOptions::new(PathBuf::from(path));
            let rest = arguments;
            while let Some(argument) = rest.next() {
                match argument.to_str().unwrap_or_default() {
                    "--apply" => options.apply = true,
                    "--undo" => options.undo = true,
                    "--batch" => {
                        let Some(value) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(path) = value.to_str() else {
                            return Some(Err(ParseError { argument: value }));
                        };
                        options.batch = Some(PathBuf::from(path));
                    }
                    "--root" => {
                        let Some(value) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(path) = value.to_str() else {
                            return Some(Err(ParseError { argument: value }));
                        };
                        options.root = Some(PathBuf::from(path));
                    }
                    _ => return Some(Err(ParseError { argument })),
                }
            }
            if options.undo && !options.apply {
                return Some(Err(ParseError { argument }));
            }
            return Some(Ok(Command::Edit(options)));
        }
        "--history" => {
            let Some(path) = arguments.next() else {
                return Some(Err(ParseError { argument }));
            };
            let mut options = HistoryOptions::new(PathBuf::from(path));
            let rest = arguments;
            while let Some(argument) = rest.next() {
                let flag = argument.to_str().unwrap_or_default();
                match flag {
                    "--merge" => options.merge = true,
                    "--text" | "--text-file" | "--batch" => {
                        let Some(value) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(value) = value.to_str() else {
                            return Some(Err(ParseError { argument: value }));
                        };
                        options.edits.push(match flag {
                            "--text" => HistoryEdit::Text(value.to_owned()),
                            "--text-file" => HistoryEdit::File(PathBuf::from(value)),
                            _ => HistoryEdit::Batch(PathBuf::from(value)),
                        });
                    }
                    "--undo" | "--redo" => {
                        let Some(value) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(count) =
                            value.to_str().and_then(|value| value.parse::<usize>().ok())
                        else {
                            return Some(Err(ParseError { argument: value }));
                        };
                        options.edits.push(if flag == "--undo" {
                            HistoryEdit::Undo(count)
                        } else {
                            HistoryEdit::Redo(count)
                        });
                    }
                    "--only" => {
                        let Some(value) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(domain) = value.to_str().and_then(HistoryDomain::from_name) else {
                            return Some(Err(ParseError { argument: value }));
                        };
                        options.only = Some(domain);
                    }
                    "--root" => {
                        let Some(value) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(value) = value.to_str() else {
                            return Some(Err(ParseError { argument: value }));
                        };
                        options.root = Some(PathBuf::from(value));
                    }
                    _ => return Some(Err(ParseError { argument })),
                }
            }
            if options.only.is_some() && !options.edits.iter().any(HistoryEdit::is_a_move) {
                return Some(Err(ParseError { argument }));
            }
            return Some(Ok(Command::History(options)));
        }
        "--save" => {
            let Some(first) = arguments.next() else {
                return Some(Err(ParseError { argument }));
            };
            let mut options = SaveOptions::new(vec![PathBuf::from(&first)]);
            let rest = arguments;
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
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(text) = value.to_str() else {
                            return Some(Err(ParseError { argument: value }));
                        };
                        options.edits.push(if flag == "--set-text" {
                            DocumentEdit::Text(text.to_owned())
                        } else {
                            DocumentEdit::File(PathBuf::from(text))
                        });
                    }
                    "--save-as" | "--save-copy" => {
                        let Some(value) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(to) = value.to_str() else {
                            return Some(Err(ParseError { argument: value }));
                        };
                        options.verb = if flag == "--save-as" {
                            Verb::SaveAs { to: to.to_owned() }
                        } else {
                            Verb::CopyTo { to: to.to_owned() }
                        };
                    }
                    "--root" => {
                        let Some(value) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(path) = value.to_str() else {
                            return Some(Err(ParseError { argument: value }));
                        };
                        options.root = Some(PathBuf::from(path));
                    }
                    _ => return Some(Err(ParseError { argument })),
                }
            }
            if !matches!(options.verb, Verb::Save) && options.files.len() != 1 {
                // Save-as and save-a-copy name one destination, so they are one file's business;
                // writing several files elsewhere at once has no meaning. Refused here rather than
                // in the parser so the message can say why, instead of "unexpected argument".
                return Some(Ok(Command::Save(options)));
            }
            return Some(Ok(Command::Save(options)));
        }
        "--conflict" => {
            let Some(path) = arguments.next() else {
                return Some(Err(ParseError { argument }));
            };
            let mut options = ConflictOptions::new(PathBuf::from(path));
            let rest = arguments;
            while let Some(argument) = rest.next() {
                let flag = argument.to_str().unwrap_or_default();
                match flag {
                    "--recreate" => options.recreate = true,
                    "--accept" => options.accept = true,
                    "--base" => {
                        let Some(value) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(path) = value.to_str() else {
                            return Some(Err(ParseError { argument: value }));
                        };
                        options.base = Some(PathBuf::from(path));
                    }
                    "--set-text" | "--set-text-file" => {
                        let Some(value) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(text) = value.to_str() else {
                            return Some(Err(ParseError { argument: value }));
                        };
                        options.edits.push(if flag == "--set-text" {
                            DocumentEdit::Text(text.to_owned())
                        } else {
                            DocumentEdit::File(PathBuf::from(text))
                        });
                    }
                    "--root" => {
                        let Some(value) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(path) = value.to_str() else {
                            return Some(Err(ParseError { argument: value }));
                        };
                        options.root = Some(PathBuf::from(path));
                    }
                    _ => return Some(Err(ParseError { argument })),
                }
            }
            return Some(Ok(Command::Conflict(options)));
        }
        "--recovery" => {
            let Some(store) = arguments.next() else {
                return Some(Err(ParseError { argument }));
            };
            let mut options = RecoveryOptions::new(PathBuf::from(store));
            let rest = arguments;
            while let Some(argument) = rest.next() {
                let flag = argument.to_str().unwrap_or_default();
                match flag {
                    "--record" | "--restore" | "--discard" => {
                        let Some(value) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(path) = value.to_str() else {
                            return Some(Err(ParseError { argument: value }));
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
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(name) = value.to_str() else {
                            return Some(Err(ParseError { argument: value }));
                        };
                        options.session = Some(name.to_owned());
                    }
                    "--prune" => options.prune = true,
                    "--apply" => options.apply = true,
                    "--keep" => {
                        let Some(value) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(count) =
                            value.to_str().and_then(|value| value.parse::<usize>().ok())
                        else {
                            return Some(Err(ParseError { argument: value }));
                        };
                        options.keep = Some(count);
                    }
                    "--root" => {
                        let Some(value) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(path) = value.to_str() else {
                            return Some(Err(ParseError { argument: value }));
                        };
                        options.root = Some(PathBuf::from(path));
                    }
                    _ => return Some(Err(ParseError { argument })),
                }
            }
            return Some(Ok(Command::Recovery(options)));
        }
        "--close" => {
            let Some(first) = arguments.next() else {
                return Some(Err(ParseError { argument }));
            };
            let mut options = CloseOptions::new(vec![PathBuf::from(&first)]);
            let rest = arguments;
            while let Some(argument) = rest.next() {
                let flag = argument.to_str().unwrap_or_default();
                if !flag.starts_with("--") {
                    options.files.push(PathBuf::from(&argument));
                    continue;
                }
                match flag {
                    "--unsaved" => {
                        let Some(value) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(value) = value.to_str() else {
                            return Some(Err(ParseError { argument: value }));
                        };
                        let Some((document, buffer)) = value.split_once('=') else {
                            return Some(Err(ParseError {
                                argument: value.into(),
                            }));
                        };
                        options
                            .unsaved
                            .push((document.to_owned(), PathBuf::from(buffer)));
                    }
                    "--task" => {
                        let Some(value) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(name) = value.to_str() else {
                            return Some(Err(ParseError { argument: value }));
                        };
                        options.tasks.push(name.to_owned());
                    }
                    "--decision" => {
                        let Some(value) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(decision) = value.to_str().and_then(Decision::from_name) else {
                            return Some(Err(ParseError { argument: value }));
                        };
                        options.decision = Some(decision);
                    }
                    "--root" => {
                        let Some(value) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(path) = value.to_str() else {
                            return Some(Err(ParseError { argument: value }));
                        };
                        options.root = Some(PathBuf::from(path));
                    }
                    _ => return Some(Err(ParseError { argument })),
                }
            }
            return Some(Ok(Command::Close(options)));
        }
        "--document" => {
            let Some(path) = arguments.next() else {
                return Some(Err(ParseError { argument }));
            };
            let mut options = DocumentOptions::new(PathBuf::from(path));
            let rest = arguments;
            while let Some(argument) = rest.next() {
                match argument.to_str().unwrap_or_default() {
                    "--root" => {
                        let Some(value) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(path) = value.to_str() else {
                            return Some(Err(ParseError { argument: value }));
                        };
                        options.root = Some(PathBuf::from(path));
                    }
                    "--set-text" | "--set-text-file" => {
                        let Some(value) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(text) = value.to_str() else {
                            return Some(Err(ParseError { argument: value }));
                        };
                        if argument.to_str().unwrap_or_default() == "--set-text" {
                            options.edits.push(DocumentEdit::Text(text.to_owned()));
                        } else {
                            options.edits.push(DocumentEdit::File(PathBuf::from(text)));
                        }
                    }
                    _ => return Some(Err(ParseError { argument })),
                }
            }
            return Some(Ok(Command::Document(options)));
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;

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
}
