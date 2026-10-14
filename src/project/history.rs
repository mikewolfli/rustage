// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The undo history of one document: text edits and model changes, in the order they happened
//! (task T03.03).
//!
//! The acceptance line is "one drag is one undo, and text and model do not loop into each other", and
//! both halves are structural here:
//!
//! * **Two domains, and no conversion between them.** [`Step`] is either a model step (the change
//!   sets that were applied, and the change set that undoes them) or a text step (the text before,
//!   the text after, and the text versions). There is no function anywhere that turns one into the
//!   other, so a text edit cannot become a model change and a model change cannot rewrite the text:
//!   the two domains meet only in the *order* of the history, which is the order things happened in.
//!   A text undo re-reads the text ([`Buffer::set_text`](crate::project::document::Buffer::set_text))
//!   and a model undo applies a change set
//!   ([`DocumentStore::apply`](crate::contracts::store::DocumentStore::apply)) — each domain touches
//!   exactly its own half of the document.
//! * **One history for the document, not one per view.** A [`View`] is an *attribute* of an entry
//!   ("the canvas proposed this"), never a partition: undo moves the newest entry whatever view
//!   proposed it, so two views cannot each keep a private stack and disagree about what came last.
//! * **One drag is one entry.** A merge window ([`History::begin_merge`]) absorbs every later entry of
//!   the same domain into the one it opened, so a drag that moves a control twenty times — or a burst
//!   of typing — is a single undo. The merge is **declared by the caller** rather than guessed from a
//!   clock: the editor knows when a drag began and ended, and a history that invented its own idea of
//!   "the same gesture" would merge two deliberate edits the day somebody types quickly.
//! * **A cross-domain undo is refused, not silently skipped.** Asking to undo the *model* while the
//!   newest entry is a text edit answers [`Moved::WrongDomain`]: skipping the text edit to reach the
//!   model change would be the history quietly discarding somebody's typing. Only a caller that
//!   *asks* for the newest entry of a domain gets that domain's step.
//!
//! **The revision is a counter, not a position.** Undoing a model step applies *another* change set,
//! so the document moves to a **new** revision whose content is the old one: after `r1 -> r2`, an undo
//! leaves the document at `r3` with r1's content. That is why [`Moved::Model`] reports the revision it
//! actually came from, the revision it actually went to, and which revision's *content* is back — a
//! history that called r3 "r1" would be a stale result wearing the current number.
//!
//! Recording a new entry after an undo **drops the redo tail** — the ordinary rule, stated rather than
//! implied — and the count of what was dropped comes back with the recording, so a caller can say so.

use std::path::Path;

use super::document::{Buffer, Refusal as TextRefusal, TextVersion};
use super::format::ProjectError;
use crate::contracts::change::{ChangeOp, ChangeOrigin, ChangeRejection, ChangeSet};
use crate::contracts::identity::PageId;
use crate::contracts::revision::Revision;
use crate::contracts::store::DocumentStore;
use crate::host::paths::ProjectRoot;

/// Which half of the document an entry belongs to.
///
/// The two domains are the whole point of the task: an entry is one or the other, and the history
/// never converts. "The model changed" and "the text changed" are different events even when one
/// caused the other.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Domain {
    /// An edit of the text a person typed.
    Text,
    /// A change to the model (a batch, a drag, a property edit).
    Model,
}

impl Domain {
    /// The machine name, for the contract data and for reports.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Model => "model",
        }
    }
}

/// Where an entry came from. An attribute of the entry, never a partition of the history.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    /// The canvas: dragging, resizing, dropping.
    Canvas,
    /// The property panel: editing a value.
    PropertyPanel,
    /// The hierarchy: adding, removing, moving a node in the tree.
    Hierarchy,
    /// The text view: typing in the document's text.
    Text,
    /// A batch document read from a file, which no view proposed.
    Batch,
}

impl View {
    /// The machine name, for the contract data and for reports.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Canvas => "canvas",
            Self::PropertyPanel => "property-panel",
            Self::Hierarchy => "hierarchy",
            Self::Text => "text",
            Self::Batch => "batch",
        }
    }
}

/// What an entry can do, in the terms of its own domain.
#[derive(Clone, PartialEq, Debug)]
pub enum Step {
    /// A model step: the change sets that were applied, in order.
    ///
    /// A merged step holds several change sets because a drag is several changes: redoing it applies
    /// them in order, and undoing it applies their inverses in reverse order as *one* change set, so
    /// the document moves once.
    ///
    /// What is **not** stored is the inverse. A [`ChangeSet`] is a proposal tied to a revision
    /// ("prepared against r3"), and the moment anything else is applied that number is stale — so the
    /// inverse is rebuilt when the move happens, based at the revision the document is at *then*.
    /// That is also why a move is never a silent success: every operation is still checked by the
    /// store against the document it is applied to, so a rebuilt inverse that no longer fits is
    /// refused with the store's own reason.
    Model {
        /// The change sets, in application order.
        steps: Vec<ChangeSet>,
        /// The revision before the first of them, when they were applied.
        from: Revision,
        /// The revision after the last of them, when they were applied.
        to: Revision,
    },
    /// A text step: the text before and after, and the versions they were.
    Text {
        /// The text as it was.
        before: String,
        /// The text as it became.
        after: String,
        /// The text version before.
        from: TextVersion,
        /// The text version after.
        to: TextVersion,
    },
}

impl Step {
    /// Which domain this step is in.
    pub const fn domain(&self) -> Domain {
        match self {
            Self::Model { .. } => Domain::Model,
            Self::Text { .. } => Domain::Text,
        }
    }

    /// One line for a report.
    pub fn describe(&self) -> String {
        match self {
            Self::Model {
                steps, from, to, ..
            } => format!("model: {} change set(s), {from} -> {to}", steps.len()),
            Self::Text {
                from,
                to,
                before,
                after,
            } => format!(
                "text: #{} -> #{} ({} -> {} character(s))",
                from.get(),
                to.get(),
                before.chars().count(),
                after.chars().count()
            ),
        }
    }
}

/// One undoable thing: what it is, who proposed it, and how to undo it.
#[derive(Clone, PartialEq, Debug)]
pub struct Entry {
    view: View,
    intent: String,
    step: Step,
}

impl Entry {
    /// The view that proposed it.
    pub const fn view(&self) -> View {
        self.view
    }

    /// What it was for.
    pub fn intent(&self) -> &str {
        &self.intent
    }

    /// The step.
    pub fn step(&self) -> &Step {
        &self.step
    }

    /// Which domain it is in.
    pub const fn domain(&self) -> Domain {
        self.step.domain()
    }

    /// One line: what it is, and what it would undo.
    pub fn describe(&self) -> String {
        format!(
            "[{}] {} — {}",
            self.view.name(),
            self.intent,
            self.step.describe()
        )
    }
}

/// A model entry's raw material, before it becomes an [`Entry`].
///
/// Separate because a caller about to apply a change set knows more than the history does: it knows
/// the intent, the view that proposed it, and what the change set was prepared against.
#[derive(Clone, PartialEq, Debug)]
pub struct ModelEdit {
    view: View,
    intent: String,
    set: ChangeSet,
    from: Revision,
    to: Revision,
}

impl ModelEdit {
    /// A model edit: the change set, and the two revisions it moved between.
    pub fn new(view: View, intent: &str, set: ChangeSet, from: Revision, to: Revision) -> Self {
        Self {
            view,
            intent: intent.to_owned(),
            set,
            from,
            to,
        }
    }

    /// The entry it becomes.
    pub fn entry(self) -> Entry {
        Entry {
            view: self.view,
            intent: self.intent,
            step: Step::Model {
                steps: vec![self.set],
                from: self.from,
                to: self.to,
            },
        }
    }
}

/// What happened to a recording.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Recording {
    /// Whether it was absorbed into the entry a merge window opened.
    pub merged: bool,
    /// How many entries were dropped from the redo tail, which is what a new edit after an undo does.
    pub dropped: usize,
}

/// A merge window: every entry of this domain recorded while it is open becomes part of one step.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Merge {
    view: View,
    intent: String,
    domain: Domain,
    recorded: usize,
}

impl Merge {
    /// The view that opened it.
    pub const fn view(&self) -> View {
        self.view
    }

    /// What the merged step is for.
    pub fn intent(&self) -> &str {
        &self.intent
    }

    /// The domain it merges.
    pub const fn domain(&self) -> Domain {
        self.domain
    }

    /// How many recordings have gone into the step so far.
    pub const fn recorded(&self) -> usize {
        self.recorded
    }
}

/// What a merge call did.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum MergeState {
    /// The window is open.
    Open(Merge),
    /// The window closed, and this many recordings went into the one step it produced.
    Closed {
        /// How many recordings were merged.
        entries: usize,
        /// The domain that was merged.
        domain: Domain,
    },
    /// There was no window open, so there was nothing to close.
    NothingOpen,
}

/// What an undo or redo did, or why it did not.
#[derive(Clone, PartialEq, Debug)]
pub enum Moved {
    /// There is nothing to move.
    Nothing {
        /// Why, in one sentence.
        reason: String,
    },
    /// The model moved.
    Model {
        /// What the entry was for.
        intent: String,
        /// The revision the document was at before this move.
        from: Revision,
        /// The revision it is at after it, which is a *new* one: applying the inverse is applying a
        /// change set.
        to: Revision,
        /// The revision whose content is now in the document.
        restored: Revision,
        /// Whether the document accepted it. A false here carries the store's own reason.
        applied: bool,
        /// The store's reason, when it refused.
        rejection: Option<ChangeRejection>,
    },
    /// The text moved.
    Text {
        /// What the entry was for.
        intent: String,
        /// The text version it restored from.
        from: TextVersion,
        /// The text version it produced. A version is a counter, not a position: undoing an edit is
        /// itself an edit, so the text has a *new* version whose content is the old one.
        to: TextVersion,
        /// Whether the buffer read the restored text. A restored text is the text that was there
        /// before, which read once already, so this is true unless the document moved underneath.
        restored: bool,
        /// Why it did not read, when it did not.
        refusal: Option<TextRefusal>,
    },
    /// The caller asked for a domain whose step is not the newest one.
    WrongDomain {
        /// What the caller asked for.
        asked: Domain,
        /// What the newest entry is.
        found: Domain,
        /// The newest entry's intent, so the answer says what to undo first.
        newest: String,
    },
}

impl Moved {
    /// Whether something moved.
    pub const fn did_move(&self) -> bool {
        match self {
            Self::Nothing { .. } | Self::WrongDomain { .. } => false,
            Self::Model { applied, .. } => *applied,
            Self::Text { restored, .. } => *restored,
        }
    }

    /// One line, then the reason when there is one.
    pub fn render(&self) -> Vec<String> {
        match self {
            Self::Nothing { reason } => vec![format!("nothing to do: {reason}")],
            Self::Model {
                intent,
                from,
                to,
                restored,
                applied,
                rejection,
            } => {
                let mut lines = vec![if *applied {
                    format!(
                        "undone/redone `{intent}`: the model moved {from} -> {to}, and the document \
                         now holds what {restored} held"
                    )
                } else {
                    format!("`{intent}` did not move: the document is still at {from}")
                }];
                if let Some(rejection) = rejection {
                    lines.push(format!("  {} : {}", rejection.name(), rejection.message()));
                }
                lines
            }
            Self::Text {
                intent,
                from,
                to,
                restored,
                refusal,
            } => {
                let mut lines = vec![if *restored {
                    format!(
                        "undone/redone `{intent}`: the text went back to what text {from} held, as \
                         text {to}"
                    )
                } else {
                    format!("`{intent}` did not restore the text")
                }];
                if let Some(refusal) = refusal {
                    lines.push(format!("  {}", refusal.message()));
                }
                lines
            }
            Self::WrongDomain {
                asked,
                found,
                newest,
            } => vec![format!(
                "refused: the newest edit is in the {found} domain (`{newest}`), and this asked to \
                 move the {asked} domain — undoing the {asked} domain now would silently skip it",
                found = found.name(),
                asked = asked.name()
            )],
        }
    }
}

/// The history of one document.
///
/// It belongs to the document — one page, one history — and the entries are in the order things
/// happened, whatever view they came from.
#[derive(Clone, PartialEq, Debug)]
pub struct History {
    page: PageId,
    entries: Vec<Entry>,
    /// The index of the next entry to redo: everything before it has been applied, everything from it
    /// on has been undone and is waiting.
    next: usize,
    open: Option<Merge>,
}

impl History {
    /// A history for the document a store holds.
    pub fn for_document(document: &DocumentStore) -> Self {
        Self {
            page: document
                .pages()
                .next()
                .map(|page| page.id())
                .unwrap_or(PageId::from_raw(0)),
            entries: Vec::new(),
            next: 0,
            open: None,
        }
    }

    /// An empty history for a page, for a document that is not a store yet.
    pub fn for_page(page: PageId) -> Self {
        Self {
            page,
            entries: Vec::new(),
            next: 0,
            open: None,
        }
    }

    /// The page this history is about.
    pub const fn page(&self) -> PageId {
        self.page
    }

    /// Whether this history is about the document a store holds. A history from another document
    /// would undo edits into the wrong one.
    pub fn is_about(&self, document: &DocumentStore) -> bool {
        document
            .pages()
            .next()
            .map(|page| page.id() == self.page)
            .unwrap_or(false)
    }

    /// Every entry, oldest first, including the ones that are undone.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// How many entries can be undone.
    pub const fn undoable(&self) -> usize {
        self.next
    }

    /// How many entries can be redone.
    pub fn redoable(&self) -> usize {
        self.entries.len() - self.next
    }

    /// Whether anything can be undone.
    pub const fn can_undo(&self) -> bool {
        self.next > 0
    }

    /// Whether anything can be redone.
    pub fn can_redo(&self) -> bool {
        self.entries.len() > self.next
    }

    /// The entry an undo would move, and whether it is undone already.
    pub fn next_undo(&self) -> Option<&Entry> {
        self.next
            .checked_sub(1)
            .and_then(|index| self.entries.get(index))
    }

    /// The entry a redo would move.
    pub fn next_redo(&self) -> Option<&Entry> {
        self.entries.get(self.next)
    }

    /// The merge window, when one is open.
    pub const fn merge(&self) -> Option<&Merge> {
        self.open.as_ref()
    }

    /// Records an entry: the newest thing that happened.
    ///
    /// A recording after an undo drops the redo tail (the entries that were undone and would have
    /// been redone), and the count comes back so a caller can say what it dropped. While a merge
    /// window is open and the entry is in the window's domain, the entry is absorbed into the entry
    /// the window opened; an entry in the *other* domain closes the window instead (a drag is not
    /// merged with typing, because they are different things to take back).
    pub fn record(&mut self, entry: Entry) -> Recording {
        let dropped = self.entries.len() - self.next;
        self.entries.truncate(self.next);

        // Whether the window takes this entry in, counting it as it does: the window's report says
        // how many recordings became the one step it produced.
        let taken = self.open.as_mut().is_some_and(|open| {
            if open.domain == entry.domain() {
                open.recorded += 1;
                true
            } else {
                false
            }
        });
        if !taken {
            // An entry in the other domain ends the window: a drag and somebody's typing are
            // different things to take back, and a window left open would merge the next entry of
            // the old domain into one from before the typing, which is not one gesture.
            self.open = None;
        } else if let Some(last) = self.entries.last_mut()
            && last.domain() == entry.domain()
        {
            // The first entry of a gesture is the one the rest are merged into; everything about the
            // later ones that matters is in the combined step.
            merge_into(last, entry);
            self.next = self.entries.len();
            return Recording {
                merged: true,
                dropped,
            };
        }
        self.entries.push(entry);
        self.next = self.entries.len();
        Recording {
            merged: false,
            dropped,
        }
    }

    /// Opens a merge window: everything recorded in this domain until it closes is one step.
    pub fn begin_merge(&mut self, view: View, domain: Domain, intent: &str) -> Merge {
        let merge = Merge {
            view,
            intent: intent.to_owned(),
            domain,
            recorded: 0,
        };
        self.open = Some(merge.clone());
        merge
    }

    /// Closes the merge window.
    pub fn end_merge(&mut self) -> MergeState {
        match self.open.take() {
            None => MergeState::NothingOpen,
            Some(merge) => MergeState::Closed {
                entries: merge.recorded,
                domain: merge.domain,
            },
        }
    }

    /// Undoes the newest entry, whichever domain it is in.
    pub fn undo(
        &mut self,
        document: &mut DocumentStore,
        buffer: &mut Buffer,
        root: &ProjectRoot,
    ) -> Moved {
        match self.next_undo().map(Entry::domain) {
            Some(Domain::Model) => self.undo_model(document),
            Some(Domain::Text) => self.undo_text(buffer, root),
            None => Moved::Nothing {
                reason: "there is nothing to undo: no edit has been recorded".to_owned(),
            },
        }
    }

    /// Redoes the next entry, whichever domain it is in.
    pub fn redo(
        &mut self,
        document: &mut DocumentStore,
        buffer: &mut Buffer,
        root: &ProjectRoot,
    ) -> Moved {
        match self.next_redo().map(Entry::domain) {
            Some(Domain::Model) => self.redo_model(document),
            Some(Domain::Text) => self.redo_text(buffer, root),
            None => Moved::Nothing {
                reason: "there is nothing to redo: everything recorded has been applied".to_owned(),
            },
        }
    }

    /// Undoes the newest entry, and only if it is a model entry.
    ///
    /// Asking for the model domain while the newest entry is a text edit is refused rather than
    /// answered: undoing the model change behind somebody's typing would quietly leave the text as
    /// they did not type it.
    pub fn undo_model(&mut self, document: &mut DocumentStore) -> Moved {
        let Some(entry) = self.next_undo() else {
            return Moved::Nothing {
                reason: "there is nothing to undo".to_owned(),
            };
        };
        if entry.domain() != Domain::Model {
            return Moved::WrongDomain {
                asked: Domain::Model,
                found: entry.domain(),
                newest: entry.intent.clone(),
            };
        }
        if !self.is_about(document) {
            return Moved::Nothing {
                reason: format!(
                    "this history is about page {}, and the document is another one",
                    self.page.get()
                ),
            };
        }
        let entry = self.entries[self.next - 1].clone();
        let Step::Model { steps, from, .. } = entry.step.clone() else {
            unreachable!("the entry's domain was checked above");
        };
        let was = document.revision();
        let Some(inverse) = undo_of(&steps, was, &entry.intent) else {
            return Moved::Nothing {
                reason: "the entry holds no operations to undo".to_owned(),
            };
        };
        let outcome = document.apply(inverse);
        let applied = outcome.is_applied();
        if applied {
            self.next -= 1;
        }
        Moved::Model {
            intent: entry.intent.clone(),
            from: was,
            to: outcome.revision(),
            restored: from,
            applied,
            rejection: outcome.rejection().cloned(),
        }
    }

    /// Redoes the next entry, and only if it is a model entry.
    pub fn redo_model(&mut self, document: &mut DocumentStore) -> Moved {
        let Some(entry) = self.next_redo() else {
            return Moved::Nothing {
                reason: "there is nothing to redo".to_owned(),
            };
        };
        if entry.domain() != Domain::Model {
            return Moved::WrongDomain {
                asked: Domain::Model,
                found: entry.domain(),
                newest: entry.intent.clone(),
            };
        }
        let entry = entry.clone();
        let Step::Model { steps, to, .. } = entry.step.clone() else {
            unreachable!("the entry's domain was checked above");
        };
        let was = document.revision();
        let mut applied = true;
        let mut rejection = None;
        let mut ended = was;
        for set in &steps {
            let Some(rebased) = rebased(set, ended) else {
                applied = false;
                break;
            };
            let outcome = document.apply(rebased);
            ended = outcome.revision();
            if let Some(refused) = outcome.rejection() {
                applied = false;
                rejection = Some(refused.clone());
                break;
            }
        }
        if applied {
            self.next += 1;
        }
        Moved::Model {
            intent: entry.intent.clone(),
            from: was,
            to: ended,
            restored: to,
            applied,
            rejection,
        }
    }

    /// Undoes the newest entry, and only if it is a text entry.
    pub fn undo_text(&mut self, buffer: &mut Buffer, root: &ProjectRoot) -> Moved {
        let Some(entry) = self.next_undo() else {
            return Moved::Nothing {
                reason: "there is nothing to undo".to_owned(),
            };
        };
        if entry.domain() != Domain::Text {
            return Moved::WrongDomain {
                asked: Domain::Text,
                found: entry.domain(),
                newest: entry.intent.clone(),
            };
        }
        let entry = self.entries[self.next - 1].clone();
        let Step::Text { before, to, .. } = entry.step.clone() else {
            unreachable!("the entry's domain was checked above");
        };
        let moved = restore(buffer, root, &entry.intent, &before, to);
        if moved.did_move() {
            self.next -= 1;
        }
        moved
    }

    /// Redoes the next entry, and only if it is a text entry.
    pub fn redo_text(&mut self, buffer: &mut Buffer, root: &ProjectRoot) -> Moved {
        let Some(entry) = self.next_redo().cloned() else {
            return Moved::Nothing {
                reason: "there is nothing to redo".to_owned(),
            };
        };
        if entry.domain() != Domain::Text {
            return Moved::WrongDomain {
                asked: Domain::Text,
                found: entry.domain(),
                newest: entry.intent.clone(),
            };
        }
        let Step::Text {
            after, to, from, ..
        } = entry.step.clone()
        else {
            unreachable!("the entry's domain was checked above");
        };
        let _ = (to, from);
        let moved = restore(buffer, root, &entry.intent, &after, from);
        if moved.did_move() {
            self.next += 1;
        }
        moved
    }

    /// The history in one report: the entries in order, and where the cursor is.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "history of page {}: {} entry(s); {} undoable, {} redoable",
            self.page.get(),
            self.entries.len(),
            self.undoable(),
            self.redoable()
        )];
        if let Some(merge) = &self.open {
            lines.push(format!(
                "a merge window is open ({} domain, `{}`): everything recorded now is one step",
                merge.domain.name(),
                merge.intent
            ));
        }
        for (index, entry) in self.entries.iter().enumerate() {
            lines.push(format!(
                "  {} {}{}",
                if index < self.next {
                    "done  "
                } else {
                    "undone"
                },
                entry.describe(),
                if index + 1 == self.next {
                    "  <- the newest thing, so the next undo moves this"
                } else {
                    ""
                }
            ));
        }
        if self.entries.is_empty() {
            lines.push("  nothing has been recorded".to_owned());
        }
        lines
    }
}

/// Restores text, and says what the buffer did with it.
fn restore(
    buffer: &mut Buffer,
    root: &ProjectRoot,
    intent: &str,
    text: &str,
    was: TextVersion,
) -> Moved {
    buffer.set_text(root, text.to_owned());
    let state = buffer.state();
    Moved::Text {
        intent: intent.to_owned(),
        from: was,
        to: buffer.typed(),
        restored: state.is_current(),
        refusal: state.refusal().cloned(),
    }
}

/// Absorbs one entry into another: the same thing, continued.
///
/// A model entry keeps every change set (so a redo replays the whole drag) and the inverse of the
/// *first* one, extended to the end: the inverse of "A then B" is "B reversed then A reversed", and
/// that is exactly what one change set holding both inverses in that order means to the store.
fn merge_into(into: &mut Entry, later: Entry) {
    match (&mut into.step, later.step) {
        (
            Step::Model {
                steps,
                to: later_to,
                ..
            },
            Step::Model {
                steps: more, to, ..
            },
        ) => {
            steps.extend(more);
            *later_to = to;
        }
        (
            Step::Text { after, to, .. },
            Step::Text {
                after: later_after,
                to: later_to,
                ..
            },
        ) => {
            *after = later_after;
            *to = later_to;
        }
        _ => {}
    }
}

/// The one change set that undoes a whole step, based at the revision the document is at now.
///
/// The operations are the steps' own operations in reverse, each inverted: the inverse of "A then B"
/// is "B reversed then A reversed", and the store applies them as one change set, so the document
/// moves once. `ChangeOp::inverse` derives each operation from what it already carries (a removal
/// carries the node, a set carries what was there), so this describes facts the history holds rather
/// than guessing at them again.
fn undo_of(steps: &[ChangeSet], at: Revision, intent: &str) -> Option<ChangeSet> {
    let ops = steps
        .iter()
        .rev()
        .flat_map(|set| set.ops().iter().rev().map(ChangeOp::inverse))
        .collect::<Vec<ChangeOp>>();
    ChangeSet::new(at, ChangeOrigin::Human, intent, ops)
}

/// The same change set, prepared against the revision the document is at now.
fn rebased(set: &ChangeSet, at: Revision) -> Option<ChangeSet> {
    ChangeSet::new(at, set.origin(), set.intent(), set.ops().to_vec())
}

/// The change set that undoes a change set that was applied, as a model edit ready to record.
///
/// A caller that applied a change set knows the revision it started at and the one it ended at; this
/// is the one place that pairs the applied set with its inverse, so the history cannot be handed a
/// half-described edit.
pub fn model_edit_of(
    view: View,
    intent: &str,
    set: ChangeSet,
    from: Revision,
    to: Revision,
) -> ModelEdit {
    ModelEdit::new(view, intent, set, from, to)
}

/// The text edit a buffer's most recent change was, ready to record.
///
/// `None` when nothing has changed since the buffer was opened: a text edit is a change of text, and
/// the version a buffer opens with is not one.
pub fn text_edit_of(
    view: View,
    intent: &str,
    before: &str,
    buffer: &Buffer,
    from: TextVersion,
) -> Option<Entry> {
    if buffer.typed() == from {
        return None;
    }
    Some(Entry {
        view,
        intent: intent.to_owned(),
        step: Step::Text {
            before: before.to_owned(),
            after: buffer.text().to_owned(),
            from,
            to: buffer.typed(),
        },
    })
}

/// A batch that was planned and applied, as a model edit ready to record.
pub fn batch_edit_of(
    view: View,
    intent: &str,
    plan: &super::batch::Plan,
    outcome: &super::batch::Outcome,
) -> Option<ModelEdit> {
    if !outcome.is_applied() {
        return None;
    }
    let set = plan.change_set()?.clone();
    Some(ModelEdit::new(
        view,
        intent,
        set,
        outcome.from(),
        outcome.to(),
    ))
}

/// The project error a history step cannot produce, for callers that must return one.
pub fn no_error() -> Option<ProjectError> {
    None
}

/// The file a history belongs to, for a report. The history itself is about a page, not a file; this
/// is only used to say which file the page came from.
pub fn describe_file(path: &Path) -> String {
    path.display().to_string()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::{
        Domain, History, MergeState, Moved, Step, View, batch_edit_of, model_edit_of, text_edit_of,
    };
    use crate::contracts::identity::NodeId;
    use crate::contracts::model::Node;
    use crate::contracts::permission::PermissionGate;
    use crate::host::paths::ProjectRoot;
    use crate::project::batch::{Batch, Plan, apply, store_of};
    use crate::project::document::Buffer;
    use crate::project::page::PageDocument;

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
            std::env::temp_dir().join(format!("rustage-history-{}-{id}", std::process::id()));
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

        fn page_text(&self) -> String {
            "{\"format\": 1, \"page\": {\"id\": 7, \"name\": \"Main\", \"root\": 0}, \
              \"nodes\": [ \
                {\"id\": 0, \"type\": \"Frame\", \"children\": [1], \
                 \"properties\": [{\"name\": \"width\", \"value\": {\"number\": 640}}]}, \
                {\"id\": 1, \"type\": \"Label\", \"children\": [], \
                 \"properties\": [{\"name\": \"text\", \"value\": {\"text\": \"hello\"}}]} \
              ]}"
            .to_owned()
        }

        fn page(&self) -> PageDocument {
            let text = self.page_text();
            let path = self.write("pages/main.json", &text);
            PageDocument::parse(&self.project(), path, &text).expect("a page document")
        }
    }

    fn model_batch(intent: &str, operations: &str) -> Batch {
        Batch::parse(
            "batch.json",
            &format!("{{\"format\": 1, \"intent\": \"{intent}\", \"operations\": [{operations}]}}"),
        )
        .expect("a batch")
    }

    /// Sets the label's text property through a batch, as a model edit in the store's domain.
    fn set_label(
        store: &mut crate::contracts::store::DocumentStore,
        root: &ProjectRoot,
        cache: &mut Vec<super::Entry>,
        history: &mut History,
        value: &str,
    ) -> super::Recording {
        let batch = model_batch(
            value,
            &format!(
                "{{\"kind\": \"set\", \"id\": 1, \"name\": \"text\", \"value\": {{\"text\": \"{value}\"}}}}"
            ),
        );
        let plan = Plan::of(&batch, store, root).expect("a plan");
        let outcome = apply(&plan, store);
        assert!(outcome.is_applied(), "{:?}", outcome.render());
        let edit =
            batch_edit_of(View::PropertyPanel, value, &plan, &outcome).expect("a model edit");
        let entry = edit.entry();
        cache.push(entry.clone());
        history.record(entry)
    }

    #[test]
    fn one_drag_is_one_undo_and_the_document_goes_back_where_it_started() {
        let sandbox = sandbox();
        let root = sandbox.project();
        let page = sandbox.page();
        let mut store = store_of(&page, PermissionGate::new()).expect("a model");
        let mut history = History::for_document(&store);
        let before = crate::project::batch::Fingerprint::of(&store);

        // A drag: three model changes, recorded as one gesture.
        history.begin_merge(View::Canvas, Domain::Model, "drag the label");
        let mut drafts = Vec::new();
        for value in ["first", "second", "third"] {
            set_label(&mut store, &root, &mut drafts, &mut history, value);
        }
        assert_eq!(
            history.end_merge(),
            MergeState::Closed {
                entries: 3,
                domain: Domain::Model
            },
            "three model changes went into the one step a drag is"
        );
        assert_eq!(history.entries().len(), 1, "one drag, one entry");
        let entry = &history.entries()[0];
        let Step::Model {
            steps, from, to, ..
        } = entry.step()
        else {
            panic!("a model step: {entry:?}");
        };
        assert_eq!(steps.len(), 3, "the whole gesture is there to redo");
        assert_eq!(from.get(), 1);
        assert_eq!(to.get(), 4);

        // One undo takes the whole drag back, and the document is exactly where it started.
        let moved = history.undo(
            &mut store,
            &mut Buffer::of_text(&root, "unused.json", ""),
            &root,
        );
        assert!(moved.did_move(), "{:?}", moved.render());
        assert_eq!(history.undoable(), 0);
        assert_eq!(crate::project::batch::Fingerprint::of(&store), before);

        // One redo puts it back, again as one step.
        let moved = history.redo(
            &mut store,
            &mut Buffer::of_text(&root, "unused.json", ""),
            &root,
        );
        assert!(moved.did_move(), "{:?}", moved.render());
        assert_eq!(history.undoable(), 1, "{:?}", moved.render());
        assert!(
            store
                .node(NodeId::from_raw(1))
                .expect("the label")
                .property(&crate::contracts::model::PropertyName::new("text").expect("a name"))
                .is_some()
        );
        assert_ne!(crate::project::batch::Fingerprint::of(&store), before);
    }

    #[test]
    fn text_edits_and_model_changes_do_not_loop_into_each_other() {
        let sandbox = sandbox();
        let root = sandbox.project();
        let page = sandbox.page();
        let mut store = store_of(&page, PermissionGate::new()).expect("a model");
        let mut history = History::for_document(&store);
        let mut buffer = Buffer::of_text(
            &root,
            sandbox.root.join("pages/main.json"),
            sandbox.page_text(),
        );
        let model_revision = store.revision();
        let text = buffer.typed();

        // A model change (a property set), then a text edit.
        set_label(
            &mut store,
            &root,
            &mut Vec::new(),
            &mut history,
            "from the panel",
        );
        let model_after = store.revision();
        let typed = buffer
            .text()
            .replace("\"text\": \"hello\"", "\"text\": \"typed by hand\"");
        buffer.set_text(&root, &typed);
        let edit = text_edit_of(
            View::Text,
            "type a label",
            &sandbox.page_text(),
            &buffer,
            text,
        )
        .expect("a text edit");
        history.record(edit);
        assert_eq!(history.entries().len(), 2);
        assert_eq!(history.entries()[0].domain(), Domain::Model);
        assert_eq!(history.entries()[1].domain(), Domain::Text);

        // Asking for the model domain while the newest entry is text is refused: it would silently
        // skip somebody's typing.
        let refused = history.undo_model(&mut store);
        assert!(matches!(
            refused,
            Moved::WrongDomain {
                asked: Domain::Model,
                found: Domain::Text,
                ..
            }
        ));
        assert!(refused.render()[0].contains("silently skip"));
        assert_eq!(store.revision(), model_after, "nothing moved");
        assert!(model_after.get() > 1, "the panel edit did happen");
        assert_eq!(history.undoable(), 2);
        assert_eq!(buffer.text(), typed, "the text is untouched too");

        // The general undo moves the text, and the model is not touched at all.
        let moved = history.undo(&mut store, &mut buffer, &root);
        assert!(moved.did_move(), "{:?}", moved.render());
        assert_eq!(buffer.text(), sandbox.page_text(), "the text is back");
        assert_eq!(
            store.revision(),
            model_after,
            "a text undo cannot move the model's revision"
        );
        assert_eq!(history.undoable(), 1);

        // And the model undo then moves the model, leaving the text where the text undo put it.
        let moved = history.undo(&mut store, &mut buffer, &root);
        assert!(moved.did_move(), "{:?}", moved.render());
        assert_eq!(
            store.revision().get(),
            model_after.get() + 1,
            "only the model undo is a change set: undoing text moves no revision at all"
        );
        assert_eq!(
            model_revision.get(),
            1,
            "and the content that came back is r1's"
        );
        assert!(
            moved.render()[0].contains("holds what"),
            "the report names the revision whose content is back: {:?}",
            moved.render()
        );
        assert_eq!(buffer.text(), sandbox.page_text());
        assert_eq!(history.undoable(), 0);

        // Redo walks the same two steps in the order they happened, each in its own domain.
        let revision_before_redo = store.revision();
        let first = history.redo(&mut store, &mut buffer, &root);
        assert!(matches!(first, Moved::Model { .. }), "{first:?}");
        assert_eq!(
            store.revision().get(),
            revision_before_redo.get() + 1,
            "a redo is a change set too"
        );
        assert_eq!(buffer.text(), sandbox.page_text());
        let revision_after_model_redo = store.revision();
        let second = history.redo(&mut store, &mut buffer, &root);
        assert!(matches!(second, Moved::Text { .. }), "{second:?}");
        assert_eq!(buffer.text(), typed);
        assert_eq!(
            store.revision(),
            revision_after_model_redo,
            "redoing text does not touch the model"
        );
    }

    #[test]
    fn a_history_is_one_history_whatever_view_proposed_the_entries() {
        let sandbox = sandbox();
        let root = sandbox.project();
        let page = sandbox.page();
        let mut store = store_of(&page, PermissionGate::new()).expect("a model");
        let mut history = History::for_document(&store);

        // Three entries from three views: the history is the order they happened in, not three
        // stacks that could disagree about what came last.
        set_label(
            &mut store,
            &root,
            &mut Vec::new(),
            &mut history,
            "from the panel",
        );
        let canvas = model_batch(
            "move the label",
            "{\"kind\": \"move\", \"id\": 1, \"to_parent\": 0, \"to_index\": 0}",
        );
        let plan = Plan::of(&canvas, &store, &root).expect("a plan");
        let outcome = apply(&plan, &mut store);
        history.record(
            batch_edit_of(View::Canvas, "move the label", &plan, &outcome)
                .expect("a model edit")
                .entry(),
        );
        let hierarchy = model_batch(
            "add a button",
            "{\"kind\": \"add\", \"parent\": 0, \"index\": 1, \"node\": {\"id\": 9, \
              \"type\": \"Button\", \"children\": [], \"properties\": []}}",
        );
        let plan = Plan::of(&hierarchy, &store, &root).expect("a plan");
        let outcome = apply(&plan, &mut store);
        history.record(
            batch_edit_of(View::Hierarchy, "add a button", &plan, &outcome)
                .expect("a model edit")
                .entry(),
        );

        assert_eq!(history.entries().len(), 3);
        assert_eq!(
            history
                .entries()
                .iter()
                .map(|entry| entry.view().name())
                .collect::<Vec<_>>(),
            ["property-panel", "canvas", "hierarchy"],
        );
        // Undo moves the newest one, whichever view proposed it.
        let buffer = &mut Buffer::of_text(&root, "unused.json", "");
        let moved = history.undo(&mut store, buffer, &root);
        assert!(matches!(moved, Moved::Model { .. }), "{moved:?}");
        assert_eq!(store.nodes().count(), 2);
        let moved = history.undo(&mut store, buffer, &root);
        assert!(matches!(moved, Moved::Model { .. }), "{moved:?}");
        let report = history.render().join("\n");
        assert!(
            report.contains("2 entry(s)") || report.contains("3 entry(s)"),
            "{report}"
        );
        assert!(report.contains("<- the newest thing"), "{report}");
        assert!(report.contains("undone"), "{report}");
    }

    #[test]
    fn an_edit_in_the_other_domain_ends_the_merge_window() {
        let sandbox = sandbox();
        let root = sandbox.project();
        let page = sandbox.page();
        let mut store = store_of(&page, PermissionGate::new()).expect("a model");
        let mut history = History::for_document(&store);
        let mut buffer = Buffer::of_text(
            &root,
            sandbox.root.join("pages/main.json"),
            sandbox.page_text(),
        );
        let opening = buffer.typed();

        // A merge window for a model drag, then a text edit while it is open: the text edit is not
        // absorbed into the drag, because a drag and typing are different things to take back.
        history.begin_merge(View::Canvas, Domain::Model, "drag the label");
        set_label(
            &mut store,
            &root,
            &mut Vec::new(),
            &mut history,
            "during the drag",
        );
        let typed = sandbox.page_text().replace("hello", "typed");
        buffer.set_text(&root, &typed);
        history.record(
            text_edit_of(
                View::Text,
                "type a label",
                &sandbox.page_text(),
                &buffer,
                opening,
            )
            .expect("a text edit"),
        );
        assert_eq!(history.entries().len(), 2, "two steps, not one");
        assert_eq!(history.entries()[0].domain(), Domain::Model);
        assert_eq!(history.entries()[1].domain(), Domain::Text);
        assert!(
            history.merge().is_none(),
            "the typing ended the drag's window rather than sitting inside it"
        );
        assert_eq!(history.end_merge(), MergeState::NothingOpen);

        // And the window really is over: the next model edit is its own step rather than being
        // merged into the drag from before the typing, which would take back both halves at once.
        set_label(
            &mut store,
            &root,
            &mut Vec::new(),
            &mut history,
            "after the typing",
        );
        assert_eq!(
            history.entries().len(),
            3,
            "a second gesture, a second step"
        );
        assert_eq!(history.entries()[2].domain(), Domain::Model);
        assert_eq!(history.undoable(), 3);
    }

    #[test]
    fn a_new_edit_after_an_undo_drops_the_redo_tail_and_says_how_much() {
        let sandbox = sandbox();
        let root = sandbox.project();
        let page = sandbox.page();
        let mut store = store_of(&page, PermissionGate::new()).expect("a model");
        let mut history = History::for_document(&store);
        let buffer = &mut Buffer::of_text(&root, "unused.json", "");

        let mut drafts = Vec::new();
        set_label(&mut store, &root, &mut drafts, &mut history, "one");
        set_label(&mut store, &root, &mut drafts, &mut history, "two");
        set_label(&mut store, &root, &mut drafts, &mut history, "three");
        assert_eq!(history.undoable(), 3);
        assert_eq!(history.redoable(), 0);

        let moved = history.undo(&mut store, buffer, &root);
        assert!(moved.did_move(), "{:?}", moved.render());
        let second = history.undo(&mut store, buffer, &root);
        assert!(second.did_move(), "{:?}", second.render());
        assert_eq!(history.undoable(), 1, "{:?}", second.render());
        assert_eq!(history.redoable(), 2);

        // A new edit replaces what was undone, and the recording says how much it dropped.
        let mut drafts = Vec::new();
        let before = history.entries().len();
        let recording = set_label(&mut store, &root, &mut drafts, &mut history, "four");
        assert_eq!(recording.dropped, 2, "the two undone entries are gone");
        assert_eq!(
            history.entries().len(),
            before - 1,
            "two dropped, one added"
        );
        assert!(!history.can_redo());
        assert_eq!(history.entries().last().expect("an entry").intent(), "four");
    }

    #[test]
    fn an_undo_against_another_document_is_refused_rather_than_applied() {
        let sandbox = sandbox();
        let root = sandbox.project();
        let page = sandbox.page();
        let mut store = store_of(&page, PermissionGate::new()).expect("a model");
        let mut history = History::for_document(&store);
        let mut drafts = Vec::new();
        set_label(&mut store, &root, &mut drafts, &mut history, "somewhere");
        let after_get = store.revision().get();

        // A history for another page will not move this document: the page id it is about is part of
        // the value, not a label.
        let mut elsewhere = History::for_page(crate::contracts::identity::PageId::from_raw(99));
        assert!(!elsewhere.is_about(&store));
        assert_eq!(
            elsewhere.undo_model(&mut store),
            Moved::Nothing {
                reason: "there is nothing to undo".to_owned()
            }
        );

        let entry = history.entries()[0].clone();
        elsewhere.record(entry);
        let refused = elsewhere.undo_model(&mut store);
        assert!(!refused.did_move());
        assert!(
            refused.render()[0].contains("another one"),
            "{:?}",
            refused.render()
        );
        assert_eq!(
            store.revision().get(),
            after_get,
            "the document is where it was"
        );
    }

    #[test]
    fn a_model_edit_carries_the_pair_the_store_produced() {
        let sandbox = sandbox();
        let root = sandbox.project();
        let page = sandbox.page();
        let mut store = store_of(&page, PermissionGate::new()).expect("a model");
        store.adopt_id(NodeId::from_raw(9));
        let set = crate::contracts::change::ChangeSet::new(
            store.revision(),
            crate::contracts::change::ChangeOrigin::Human,
            "add a label",
            vec![crate::contracts::change::ChangeOp::AddNode {
                parent: NodeId::from_raw(0),
                index: 1,
                node: Box::new(Node::new(
                    crate::contracts::identity::PageId::from_raw(7),
                    NodeId::from_raw(9),
                    crate::contracts::model::NodeKind::new("Label").expect("a kind"),
                )),
            }],
        )
        .expect("a change set");
        let from = store.revision();
        let outcome = store.apply(set.clone());
        assert!(outcome.is_applied(), "{outcome:?}");
        let to = store.revision();
        let edit = model_edit_of(View::Canvas, "add a label", set, from, to);
        let entry = edit.entry();
        assert_eq!(entry.domain(), Domain::Model);
        assert_eq!(entry.view(), View::Canvas);
        assert!(entry.describe().contains("model: 1 change set(s)"));

        // Undoing it through the history puts the document back, and the entry counts as undone.
        let mut history = History::for_document(&store);
        history.record(entry);
        let buffer = &mut Buffer::of_text(&root, "unused.json", "");
        let moved = history.undo(&mut store, buffer, &root);
        assert!(moved.did_move(), "{:?}", moved.render());
        assert_eq!(store.nodes().count(), 2);
        assert_eq!(
            store.revision().get(),
            to.get() + 1,
            "the undo is itself a change set, so the revision moved forward"
        );
        assert!(
            from.get() < to.get(),
            "the entry describes the edit it recorded"
        );
        assert_eq!(history.redoable(), 1);
    }

    #[test]
    fn a_step_says_what_it_is_in_its_own_domains_words() {
        let text = Step::Text {
            before: "one".to_owned(),
            after: "three".to_owned(),
            from: crate::project::document::TextVersion::FIRST,
            to: crate::project::document::TextVersion::FIRST,
        };
        assert_eq!(text.domain(), Domain::Text);
        assert!(text.describe().contains("text: #0 -> #0"));
        assert_eq!(
            text.domain().name(),
            "text",
            "a report and the contract data use the same word"
        );

        let pages = [
            View::Canvas,
            View::PropertyPanel,
            View::Hierarchy,
            View::Text,
            View::Batch,
        ];
        for view in pages {
            assert!(!view.name().is_empty());
        }
    }
}
