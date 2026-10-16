// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Document tabs: opening, preview tabs and pinning, reordering, closing a set, and overflow
//! (task T04.03).
//!
//! The acceptance line is *"closing a view and closing a document are told apart correctly"*. That is a
//! distinction about **identity**, and the module keeps the two apart by name:
//!
//! * **A tab is a view; the document is what it shows.** [`Tabs::close_view`] closes one tab — the
//!   document stays open wherever else it appears, and its unsaved text is untouched. [`Tabs::close_document`]
//!   closes **every** view of one document and says how many there were. A caller that mixed the two
//!   would either lose a document by closing one window onto it, or leave views of a document it
//!   thought it had closed; naming them differently is what makes that hard to do by accident.
//! * **Opening is a preview until it is pinned.** [`Tabs::preview`] opens a document in the *preview*
//!   tab, replacing whatever preview tab is there (that is what single-clicking through a tree does);
//!   [`Tabs::pin`] makes the tab permanent, so the next preview replaces something else. The preview
//!   tab is never replaced when it is the *only* tab for its document — a preview that could close the
//!   last view of a dirty document would be a data-loss path.
//! * **A close set is a set, named by where it starts.** [`Tabs::close_others`] and
//!   [`Tabs::close_to_the_right`] both keep the tab they are asked about and remove the rest of the
//!   named set, so "close all but this" cannot remove the one the person was looking at.
//! * **Overflow is a list, not a loss.** Tabs past the strip's limit are [`Tabs::overflow`] rather than
//!   dropped, so a strip that is full still has every open document — the caller scrolls or shows a
//!   menu, and the model does not quietly forget.
//!
//! **The viewport belongs to the view.** Each [`Tab`] carries its own [`Viewport`] (scroll and
//! selection), and [`Tabs::take`]/[`Tabs::insert`] carry it along when a view is moved to another
//! group (task T04.04). Two views of one document therefore share the text and disagree about where
//! they are looking, which is the difference between "the same file open twice" and "one view
//! dragging the other around".

// The tab strip's own accessors are read by the window and the tab context menu (T04.06/T05.05) as
// those land; until then the tests and the `--tabs` script exercise them, and the silence is stated
// once here with its reason, as the neighbouring trees do.
#![allow(dead_code)]

/// Where one view is looking: its scroll position and its selection (task T04.04).
///
/// A viewport belongs to a **view**, not to the document. Two views of one document share the text
/// and each keep their own scroll and selection, which is what lets a person compare two parts of a
/// file without one view dragging the other along. It is the counterpart of the shared buffer: the
/// text has one owner, the viewport has one per viewer.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Viewport {
    scroll: u32,
    selection: Option<(u32, u32)>,
}

impl Viewport {
    /// The first visible line.
    pub const fn scroll(&self) -> u32 {
        self.scroll
    }

    /// The selection as an `(anchor, cursor)` pair, when there is one.
    pub const fn selection(&self) -> Option<(u32, u32)> {
        self.selection
    }

    /// The viewport as one line, for a report.
    pub fn render(&self) -> String {
        match self.selection {
            Some((anchor, cursor)) => {
                format!("scroll {} selection {anchor}..{cursor}", self.scroll)
            }
            None => format!("scroll {} no selection", self.scroll),
        }
    }
}

/// One tab: a view onto a document.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Tab {
    document: String,
    preview: bool,
    dirty: bool,
    viewport: Viewport,
}

impl Tab {
    /// A fresh view onto a document: no unsaved changes and a viewport at the top.
    fn view(document: impl Into<String>, preview: bool) -> Self {
        Self {
            document: document.into(),
            preview,
            dirty: false,
            viewport: Viewport::default(),
        }
    }

    /// The document this view shows.
    pub fn document(&self) -> &str {
        &self.document
    }

    /// Where this view is looking. Its own, never the document's and never another view's.
    pub const fn viewport(&self) -> &Viewport {
        &self.viewport
    }

    /// Whether it is the preview tab (replaced by the next preview) rather than a permanent one.
    pub const fn is_preview(&self) -> bool {
        self.preview
    }

    /// Whether it is permanent.
    pub const fn is_pinned(&self) -> bool {
        !self.preview
    }

    /// Whether the document it shows has unsaved changes.
    pub const fn is_dirty(&self) -> bool {
        self.dirty
    }
}

/// The open tabs.
#[derive(Clone, Debug)]
pub struct Tabs {
    tabs: Vec<Tab>,
    active: Option<usize>,
    limit: usize,
}

impl Tabs {
    /// A strip that shows at most `limit` tabs before overflowing.
    pub fn new(limit: usize) -> Self {
        Self {
            tabs: Vec::new(),
            active: None,
            limit: limit.max(1),
        }
    }

    /// Every tab, in order.
    pub fn tabs(&self) -> &[Tab] {
        &self.tabs
    }

    /// How many tabs are open.
    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    /// Whether none are.
    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    /// The index of the active tab.
    pub const fn active_index(&self) -> Option<usize> {
        self.active
    }

    /// The active tab.
    pub fn active(&self) -> Option<&Tab> {
        self.active.and_then(|index| self.tabs.get(index))
    }

    /// Whether a document is open in at least one view.
    pub fn is_open(&self, document: &str) -> bool {
        self.tabs.iter().any(|tab| tab.document == document)
    }

    /// How many views of a document are open.
    pub fn views_of(&self, document: &str) -> usize {
        self.tabs
            .iter()
            .filter(|tab| tab.document == document)
            .count()
    }

    /// The tabs the strip shows: the first `limit` of them.
    pub fn visible(&self) -> &[Tab] {
        let end = self.limit.min(self.tabs.len());
        &self.tabs[..end]
    }

    /// The tabs past the strip's limit, which a caller scrolls to or lists in a menu.
    pub fn overflow(&self) -> &[Tab] {
        let end = self.limit.min(self.tabs.len());
        &self.tabs[end..]
    }

    /// Whether the active tab is inside the visible part of the strip.
    pub fn active_is_visible(&self) -> bool {
        match self.active {
            Some(index) => index < self.limit.min(self.tabs.len()),
            None => true,
        }
    }

    /// Opens a document in the preview tab.
    ///
    /// If the document is already open, its tab is activated and its kind is left alone. Otherwise the
    /// existing preview tab is replaced — unless that preview tab is the only view of a document with
    /// unsaved changes, in which case a new tab is opened instead and the dirty view is kept, because
    /// replacing it would be the model closing a view somebody was working in.
    pub fn preview(&mut self, document: impl Into<String>) {
        let document = document.into();
        if let Some(index) = self.index_of(&document) {
            self.active = Some(index);
            return;
        }
        if let Some(index) = self.tabs.iter().position(|tab| tab.preview) {
            let candidate = &self.tabs[index];
            let sole_dirty_view = candidate.dirty && self.views_of(&candidate.document) == 1;
            if !sole_dirty_view {
                self.tabs[index] = Tab::view(document, true);
                self.active = Some(index);
                return;
            }
        }
        self.tabs.push(Tab::view(document, true));
        self.active = Some(self.tabs.len() - 1);
    }

    /// Opens another view of a document, so one document can appear in more than one tab.
    ///
    /// This is what splitting a group or opening a project's file "in a new view" does. It is a
    /// separate entry point from [`Tabs::pin`] on purpose: pinning a document that is already open
    /// activates its view rather than making a second one, so "another view" has to be asked for.
    pub fn another_view(&mut self, document: impl Into<String>) {
        self.tabs.push(Tab::view(document, false));
        self.active = Some(self.tabs.len() - 1);
    }

    /// Records whether a document has unsaved changes, which the preview rule consults.
    pub fn mark_dirty(&mut self, document: &str, dirty: bool) {
        for tab in &mut self.tabs {
            if tab.document == document {
                tab.dirty = dirty;
            }
        }
    }

    /// Opens a document permanently, or pins the tab it already has.
    pub fn pin(&mut self, document: impl Into<String>) {
        let document = document.into();
        match self.index_of(&document) {
            Some(index) => {
                self.tabs[index].preview = false;
                self.active = Some(index);
            }
            None => {
                self.tabs.push(Tab::view(document, false));
                self.active = Some(self.tabs.len() - 1);
            }
        }
    }

    /// Activates the tab at an index, refusing one that is not there.
    pub fn activate(&mut self, index: usize) -> Result<(), String> {
        if index >= self.tabs.len() {
            return Err(format!("there is no tab {index}"));
        }
        self.active = Some(index);
        Ok(())
    }

    /// Moves a tab to another position.
    pub fn reorder(&mut self, from: usize, to: usize) -> Result<(), String> {
        if from >= self.tabs.len() || to >= self.tabs.len() {
            return Err(format!(
                "cannot move tab {from} to {to}: the strip has {} tab(s)",
                self.tabs.len()
            ));
        }
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);
        // Keep the active tab active across the move.
        if let Some(active) = self.active {
            self.active = Some(match active {
                index if index == from => to,
                index if from < index && index <= to => index - 1,
                index if to <= index && index < from => index + 1,
                index => index,
            });
        }
        Ok(())
    }

    /// Closes one view. The document stays open wherever else it appears.
    ///
    /// Closing a view and taking a view out to move it elsewhere are the same removal; the
    /// difference is only what the caller does with the tab afterwards, so this delegates to
    /// [`Tabs::take`] rather than repeating the bookkeeping.
    pub fn close_view(&mut self, index: usize) -> Result<Tab, String> {
        self.take(index)
    }

    /// Takes one view out of the strip and hands it back, without closing anything.
    ///
    /// This is how a tab migrates to another group (task T04.04): the view leaves this strip and is
    /// inserted into another, carrying its dirty flag and its viewport with it. It is deliberately
    /// the same removal [`Tabs::close_view`] performs, so there is one place that settles the active
    /// pointer rather than two that could disagree.
    pub fn take(&mut self, index: usize) -> Result<Tab, String> {
        if index >= self.tabs.len() {
            return Err(format!("there is no tab {index}"));
        }
        let tab = self.tabs.remove(index);
        self.active = Self::settle_active(self.active, index, self.tabs.len());
        Ok(tab)
    }

    /// Puts a view back into the strip at a position, and makes it active.
    ///
    /// The counterpart of [`Tabs::take`]: a tab that arrived from another group lands where the
    /// caller says. A position past the end is refused rather than clamped, because a silently
    /// clamped move is a move that happened somewhere the caller did not ask for.
    pub fn insert(&mut self, index: usize, tab: Tab) -> Result<(), String> {
        if index > self.tabs.len() {
            return Err(format!(
                "cannot put a view at {index}: the strip has {} tab(s)",
                self.tabs.len()
            ));
        }
        self.tabs.insert(index, tab);
        self.active = Some(index);
        Ok(())
    }

    /// Moves one view's scroll position. Only that view's; no other view moves.
    pub fn scroll_to(&mut self, index: usize, offset: u32) -> Result<(), String> {
        let Some(tab) = self.tabs.get_mut(index) else {
            return Err(format!("there is no tab {index}"));
        };
        tab.viewport.scroll = offset;
        Ok(())
    }

    /// Sets one view's selection. Only that view's; no other view is affected.
    pub fn select(&mut self, index: usize, anchor: u32, cursor: u32) -> Result<(), String> {
        let Some(tab) = self.tabs.get_mut(index) else {
            return Err(format!("there is no tab {index}"));
        };
        tab.viewport.selection = Some((anchor, cursor));
        Ok(())
    }

    /// Closes every view of a document, and says how many there were.
    pub fn close_document(&mut self, document: &str) -> usize {
        let before = self.tabs.len();
        let active_document = self
            .active
            .and_then(|index| self.tabs.get(index).map(|tab| tab.document.clone()));
        self.tabs.retain(|tab| tab.document != document);
        let closed = before - self.tabs.len();
        if closed > 0 {
            self.active = match active_document {
                Some(document) if self.is_open(&document) => self.index_of(&document),
                _ => self
                    .tabs
                    .len()
                    .checked_sub(1)
                    .filter(|_| !self.tabs.is_empty()),
            };
        }
        closed
    }

    /// Closes every tab except the one at `keep`.
    pub fn close_others(&mut self, keep: usize) -> Result<usize, String> {
        if keep >= self.tabs.len() {
            return Err(format!("there is no tab {keep}"));
        }
        let kept = self.tabs[keep].clone();
        let closed = self.tabs.len() - 1;
        self.tabs = vec![kept];
        self.active = Some(0);
        Ok(closed)
    }

    /// Closes every tab to the right of `from`.
    pub fn close_to_the_right(&mut self, from: usize) -> Result<usize, String> {
        if from >= self.tabs.len() {
            return Err(format!("there is no tab {from}"));
        }
        let closed = self.tabs.len() - from - 1;
        self.tabs.truncate(from + 1);
        if let Some(active) = self.active
            && active > from
        {
            self.active = Some(from);
        }
        Ok(closed)
    }

    fn index_of(&self, document: &str) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.document == document)
    }

    /// Where the active tab should be after one is removed at `removed`.
    fn settle_active(active: Option<usize>, removed: usize, len: usize) -> Option<usize> {
        match active {
            None => None,
            Some(index) if len == 0 => {
                let _ = index;
                None
            }
            Some(index) if index == removed => Some(index.min(len - 1)),
            Some(index) if index > removed => Some(index - 1),
            Some(index) => Some(index),
        }
    }

    /// The strip in one report.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "{} tab(s), {} visible, {} overflowed, limit {}",
            self.len(),
            self.visible().len(),
            self.overflow().len(),
            self.limit
        )];
        for (index, tab) in self.tabs.iter().enumerate() {
            let active = if self.active == Some(index) { "*" } else { " " };
            let kind = if tab.preview { "preview" } else { "pinned" };
            let overflowed = if index >= self.limit {
                " (overflow)"
            } else {
                ""
            };
            lines.push(format!(
                " {active} {index}: {} [{kind}]{overflowed}",
                tab.document
            ));
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tabs() -> Tabs {
        Tabs::new(4)
    }

    #[test]
    fn closing_a_view_leaves_the_document_open_and_closing_the_document_closes_every_view() {
        let mut tabs = tabs();
        tabs.pin("pages/home.json");
        tabs.pin("styles/app.css");
        tabs.another_view("pages/home.json"); // a second view of the same document
        assert_eq!(tabs.views_of("pages/home.json"), 2);
        assert!(tabs.is_open("pages/home.json"));

        // Closing one view leaves the other and never touches the other document.
        let closed = tabs.close_view(0).expect("a tab");
        assert_eq!(closed.document(), "pages/home.json");
        assert_eq!(tabs.views_of("pages/home.json"), 1);
        assert!(tabs.is_open("pages/home.json"));
        assert!(tabs.is_open("styles/app.css"));

        // Closing the document closes every view of it and leaves the rest.
        let closed = tabs.close_document("pages/home.json");
        assert_eq!(closed, 1);
        assert!(!tabs.is_open("pages/home.json"));
        assert_eq!(tabs.views_of("pages/home.json"), 0);
        assert!(
            tabs.is_open("styles/app.css"),
            "the other document is untouched"
        );

        // Closing it again is not an error and closes nothing.
        assert_eq!(tabs.close_document("pages/home.json"), 0);
    }

    #[test]
    fn a_preview_tab_is_replaced_by_the_next_preview_and_pinning_keeps_it() {
        let mut tabs = tabs();
        tabs.preview("pages/one.json");
        tabs.preview("pages/two.json");
        assert_eq!(tabs.len(), 1, "the preview tab was replaced");
        assert_eq!(tabs.tabs()[0].document(), "pages/two.json");
        assert!(tabs.tabs()[0].is_preview());

        tabs.pin("pages/two.json");
        assert!(tabs.tabs()[0].is_pinned(), "pinning makes it permanent");
        tabs.preview("pages/three.json");
        assert_eq!(tabs.len(), 2, "the pinned tab was not replaced");
        assert_eq!(tabs.tabs()[1].document(), "pages/three.json");
    }

    #[test]
    fn a_preview_does_not_replace_the_only_view_of_a_dirty_document() {
        let mut tabs = tabs();
        tabs.preview("pages/home.json");
        tabs.mark_dirty("pages/home.json", true);
        // A second preview must not replace the only view of a document with unsaved changes.
        tabs.preview("pages/other.json");
        assert!(
            tabs.is_open("pages/home.json"),
            "the dirty document keeps its view"
        );
        assert!(tabs.is_open("pages/other.json"));
        assert_eq!(tabs.len(), 2);

        // Once that document is the only view of a *clean* one, the preview may replace it again.
        tabs.mark_dirty("pages/home.json", false);
        tabs.pin("pages/home.json");
        assert_eq!(tabs.close_document("pages/other.json"), 1);
        tabs.preview("pages/third.json");
        assert!(tabs.is_open("pages/third.json"));
    }

    #[test]
    fn opening_a_document_that_is_already_open_activates_it_rather_than_duplicating() {
        let mut tabs = tabs();
        tabs.pin("pages/home.json");
        tabs.pin("styles/app.css");
        tabs.preview("pages/home.json");
        assert_eq!(tabs.len(), 2);
        assert_eq!(tabs.active().unwrap().document(), "pages/home.json");
        tabs.pin("pages/home.json");
        assert_eq!(tabs.len(), 2, "pinning an open document does not add a tab");
        // Only an explicit "another view" makes a second one.
        tabs.another_view("pages/home.json");
        assert_eq!(tabs.views_of("pages/home.json"), 2);
        assert_eq!(tabs.len(), 3);
    }

    #[test]
    fn reordering_keeps_the_active_tab_active() {
        let mut tabs = tabs();
        tabs.pin("a");
        tabs.pin("b");
        tabs.pin("c");
        tabs.activate(2).expect("a tab");
        assert_eq!(tabs.active().unwrap().document(), "c");
        // Move "a" (index 1... take index 0) to the end.
        tabs.reorder(0, 2).expect("a move");
        assert_eq!(
            tabs.tabs().iter().map(Tab::document).collect::<Vec<_>>(),
            ["b", "c", "a"]
        );
        assert_eq!(
            tabs.active().unwrap().document(),
            "c",
            "the active tab followed the move"
        );
        assert!(tabs.reorder(0, 9).is_err());
    }

    #[test]
    fn a_close_set_keeps_the_tab_it_names() {
        let mut tabs = tabs();
        for name in ["a", "b", "c", "d"] {
            tabs.pin(name);
        }
        let closed = tabs.close_others(2).expect("a tab");
        assert_eq!(closed, 3);
        assert_eq!(
            tabs.tabs().iter().map(Tab::document).collect::<Vec<_>>(),
            ["c"]
        );
        assert_eq!(tabs.active().unwrap().document(), "c");

        let mut right = Tabs::new(4);
        for name in ["a", "b", "c", "d"] {
            right.pin(name);
        }
        let closed = right.close_to_the_right(1).expect("a tab");
        assert_eq!(closed, 2);
        assert_eq!(
            right.tabs().iter().map(Tab::document).collect::<Vec<_>>(),
            ["a", "b"]
        );
    }

    #[test]
    fn tabs_past_the_limit_are_overflowed_rather_than_dropped() {
        let mut tabs = Tabs::new(2);
        for name in ["a", "b", "c", "d"] {
            tabs.pin(name);
        }
        assert_eq!(tabs.len(), 4, "nothing was dropped");
        assert_eq!(tabs.visible().len(), 2);
        assert_eq!(tabs.overflow().len(), 2);
        assert_eq!(tabs.overflow()[0].document(), "c");
        assert!(
            !tabs.active_is_visible(),
            "the active tab is in the overflow"
        );
        tabs.activate(0).expect("a tab");
        assert!(tabs.active_is_visible());
    }

    #[test]
    fn activating_a_tab_that_is_not_there_is_refused() {
        let mut tabs = tabs();
        tabs.pin("a");
        assert!(tabs.activate(3).is_err());
        assert!(tabs.close_view(3).is_err());
        assert!(tabs.close_others(3).is_err());
        assert_eq!(tabs.len(), 1, "nothing changed");
    }

    #[test]
    fn closing_the_active_tab_moves_the_keyboard_to_a_tab_that_exists() {
        let mut tabs = tabs();
        for name in ["a", "b", "c"] {
            tabs.pin(name);
        }
        tabs.activate(1).expect("a tab");
        tabs.close_view(1).expect("a tab");
        assert_eq!(tabs.len(), 2);
        let active = tabs.active().expect("something is still active");
        assert!(matches!(active.document(), "a" | "c"));

        // Closing every tab leaves nothing active rather than a dangling index.
        tabs.close_document("a");
        tabs.close_document("c");
        assert!(tabs.is_empty());
        assert!(tabs.active().is_none());
    }
}
