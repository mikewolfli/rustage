// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The internal pages of a tab, stack, scroll or split container, and what navigation does to them
//! (task T27.05).
//!
//! The acceptance line is *"page/index changes keep references and focus in sync; the empty-page
//! policy is explicit"*. Position is not identity, so adding, removing, reordering or stepping
//! through pages is expressed by identity:
//!
//! * **The active page is an identity.** [`Pages::active`] is a [`PageId`], not an index, so
//!   [`Pages::reorder`] moves a page without changing which page is active.
//! * **Focus follows the active page.** [`Pages::focus`] is the [`NodeId`] of the active page's
//!   content. Adding a page, activating one, stepping to the next, and removing the active page all
//!   leave focus pointing at the page that is actually active — or at nothing when there is none.
//! * **The empty policy is stated.** [`ContainerKind::empty_policy`] names what a container with no
//!   pages does: a tab or stack container has nothing to show and removes itself, while a scroll or
//!   split container stays with an explicit placeholder. It is a value, not an undefined case.

#![allow(dead_code)]

use crate::contracts::identity::{NodeId, PageId};

/// The kind of container whose pages these are.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ContainerKind {
    /// A tab strip: one page visible, the rest behind tabs.
    Tabs,
    /// A stack: one page visible, pages stacked.
    Stack,
    /// A scroll container: its pages scroll together.
    Scroll,
    /// A split container: its pages share the area as panes.
    Split,
}

impl ContainerKind {
    /// The kind's name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Tabs => "tabs",
            Self::Stack => "stack",
            Self::Scroll => "scroll",
            Self::Split => "split",
        }
    }

    /// Reads a kind's name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "tabs" => Some(Self::Tabs),
            "stack" => Some(Self::Stack),
            "scroll" => Some(Self::Scroll),
            "split" => Some(Self::Split),
            _ => None,
        }
    }

    /// What this kind of container does when it holds no pages. Stated, not left undefined.
    pub fn empty_policy(self) -> EmptyPolicy {
        match self {
            Self::Tabs | Self::Stack => EmptyPolicy::RemoveContainer,
            Self::Scroll => EmptyPolicy::Placeholder("a scroll container with no pages".to_owned()),
            Self::Split => EmptyPolicy::Placeholder("a split container with no panes".to_owned()),
        }
    }
}

/// What a container does when it holds no pages.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum EmptyPolicy {
    /// The container has nothing to show and takes itself out of the layout.
    RemoveContainer,
    /// The container stays, showing an explicit placeholder.
    Placeholder(String),
}

impl EmptyPolicy {
    /// The policy in one line.
    pub fn render(&self) -> String {
        match self {
            Self::RemoveContainer => "the container is removed from the layout".to_owned(),
            Self::Placeholder(text) => format!("showing a placeholder: {text}"),
        }
    }

    /// Whether the container stays in the layout when it is empty.
    pub fn keeps_the_container(&self) -> bool {
        matches!(self, Self::Placeholder(_))
    }
}

/// One page inside a container.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Page {
    /// The page's stable identity.
    pub id: PageId,
    /// The page's title.
    pub title: String,
    /// The identity of the node the page shows.
    pub node: NodeId,
}

/// A container's pages, its active page and where focus is.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Pages {
    /// The container's kind.
    pub kind: ContainerKind,
    /// The pages, in display order.
    pub pages: Vec<Page>,
    /// The active page, by identity.
    pub active: Option<PageId>,
    /// The focused node, which follows the active page.
    pub focus: Option<NodeId>,
}

impl Pages {
    /// A container of a kind with no pages.
    pub fn new(kind: ContainerKind) -> Self {
        Self {
            kind,
            pages: Vec::new(),
            active: None,
            focus: None,
        }
    }

    /// The page with this identity.
    pub fn page(&self, id: PageId) -> Option<&Page> {
        self.pages.iter().find(|page| page.id == id)
    }

    /// The active page, when there is one.
    pub fn active_page(&self) -> Option<&Page> {
        self.active.and_then(|id| self.page(id))
    }

    /// Adds a page and makes it the active one, so focus moves to its content.
    ///
    /// Refuses a duplicate identity: a page is one page, however many times it is named.
    pub fn add(&mut self, page: Page) -> Result<(), String> {
        if self.page(page.id).is_some() {
            return Err(format!("page {} already exists", page.id.get()));
        }
        let id = page.id;
        let node = page.node;
        self.pages.push(page);
        self.active = Some(id);
        self.focus = Some(node);
        Ok(())
    }

    /// Removes a page, keeping the active page and focus in sync.
    ///
    /// When the removed page was active, the page that takes its place becomes active and focus moves
    /// to it; when the container is now empty, both active and focus are cleared.
    pub fn remove(&mut self, id: PageId) -> Result<(), String> {
        let index = self
            .pages
            .iter()
            .position(|page| page.id == id)
            .ok_or_else(|| format!("page {} is not in this container", id.get()))?;
        self.pages.remove(index);
        if self.active == Some(id) {
            let next = self
                .pages
                .get(index)
                .or_else(|| self.pages.last())
                .map(|page| (page.id, page.node));
            match next {
                Some((id, node)) => {
                    self.active = Some(id);
                    self.focus = Some(node);
                }
                None => {
                    self.active = None;
                    self.focus = None;
                }
            }
        }
        Ok(())
    }

    /// Makes a page active and moves focus to its content.
    pub fn activate(&mut self, id: PageId) -> Result<(), String> {
        let node = self
            .page(id)
            .map(|page| page.node)
            .ok_or_else(|| format!("page {} is not in this container", id.get()))?;
        self.active = Some(id);
        self.focus = Some(node);
        Ok(())
    }

    /// Moves a page to another position.
    ///
    /// The active page is an identity, so it does not change when the page it names moves.
    pub fn reorder(&mut self, id: PageId, to: usize) -> Result<(), String> {
        let from = self
            .pages
            .iter()
            .position(|page| page.id == id)
            .ok_or_else(|| format!("page {} is not in this container", id.get()))?;
        let len = self.pages.len();
        if to >= len {
            return Err(format!("page index {to} is outside 0..{len}"));
        }
        let page = self.pages.remove(from);
        self.pages.insert(to, page);
        Ok(())
    }

    /// Steps to the next page, wrapping around, and answers its identity.
    pub fn next(&mut self) -> Result<PageId, String> {
        self.step(1)
    }

    /// Steps to the previous page, wrapping around, and answers its identity.
    pub fn previous(&mut self) -> Result<PageId, String> {
        self.step(-1)
    }

    fn step(&mut self, offset: isize) -> Result<PageId, String> {
        if self.pages.is_empty() {
            return Err("the container has no pages".to_owned());
        }
        let current = self
            .active
            .and_then(|id| self.pages.iter().position(|page| page.id == id))
            .unwrap_or(0);
        let len = self.pages.len() as isize;
        let step = (current as isize + offset).rem_euclid(len) as usize;
        let (id, node) = {
            let page = &self.pages[step];
            (page.id, page.node)
        };
        self.active = Some(id);
        self.focus = Some(node);
        Ok(id)
    }

    /// The policy applied when this container holds no pages.
    pub fn empty_policy(&self) -> EmptyPolicy {
        self.kind.empty_policy()
    }

    /// The container as lines, marking the active page and where focus is, or stating the empty policy.
    pub fn render(&self) -> Vec<String> {
        let active = match self.active {
            Some(id) => id.get().to_string(),
            None => "none".to_owned(),
        };
        let mut lines = vec![format!(
            "{} container ({} page(s), active {active})",
            self.kind.name(),
            self.pages.len()
        )];
        if self.pages.is_empty() {
            lines.push(format!("no pages: {}", self.empty_policy().render()));
            return lines;
        }
        for page in &self.pages {
            let marker = if self.active == Some(page.id) {
                "*"
            } else {
                " "
            };
            let focused = if self.focus == Some(page.node) {
                " (focused)"
            } else {
                ""
            };
            lines.push(format!(
                "{marker} [{}] {} -> node {}{focused}",
                page.id.get(),
                page.title,
                page.node.get()
            ));
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{ContainerKind, EmptyPolicy, Page, Pages};
    use crate::contracts::identity::{NodeId, PageId};

    fn page(id: u64, node: u64) -> Page {
        Page {
            id: PageId::from_raw(id),
            title: format!("page {id}"),
            node: NodeId::from_raw(node),
        }
    }

    fn filled() -> Pages {
        let mut pages = Pages::new(ContainerKind::Tabs);
        pages.add(page(1, 11)).expect("a page");
        pages.add(page(2, 22)).expect("a page");
        pages.add(page(3, 33)).expect("a page");
        pages
    }

    #[test]
    fn adding_a_page_activates_it_and_focuses_its_node() {
        let mut pages = Pages::new(ContainerKind::Tabs);
        pages.add(page(1, 11)).expect("a page");
        assert_eq!(pages.active, Some(PageId::from_raw(1)));
        assert_eq!(pages.focus, Some(NodeId::from_raw(11)));
    }

    #[test]
    fn removing_the_active_page_moves_focus_to_the_next_one() {
        let mut pages = filled();
        pages.activate(PageId::from_raw(2)).expect("an activation");
        pages.remove(PageId::from_raw(2)).expect("a removal");
        // Page 3 slid into the removed page's place and became active.
        assert_eq!(pages.active, Some(PageId::from_raw(3)));
        assert_eq!(pages.focus, Some(NodeId::from_raw(33)));
    }

    #[test]
    fn removing_the_only_page_clears_the_active_page_and_focus() {
        let mut pages = Pages::new(ContainerKind::Tabs);
        pages.add(page(1, 11)).expect("a page");
        pages.remove(PageId::from_raw(1)).expect("a removal");
        assert_eq!(pages.active, None);
        assert_eq!(pages.focus, None);
    }

    #[test]
    fn reordering_keeps_the_active_page_by_identity() {
        let mut pages = filled();
        pages.activate(PageId::from_raw(2)).expect("an activation");
        pages.reorder(PageId::from_raw(1), 2).expect("a reorder");
        assert_eq!(pages.active, Some(PageId::from_raw(2)));
        assert_eq!(pages.focus, Some(NodeId::from_raw(22)));
        // The page that moved is still the same page, at a new position.
        assert_eq!(pages.pages[2].id, PageId::from_raw(1));
    }

    #[test]
    fn stepping_keeps_focus_in_sync_and_wraps() {
        let mut pages = filled();
        pages.activate(PageId::from_raw(1)).expect("an activation");
        assert_eq!(pages.next().expect("a step"), PageId::from_raw(2));
        assert_eq!(pages.focus, Some(NodeId::from_raw(22)));
        assert_eq!(pages.next().expect("a step"), PageId::from_raw(3));
        assert_eq!(pages.next().expect("a step"), PageId::from_raw(1));
        assert_eq!(pages.previous().expect("a step"), PageId::from_raw(3));
        assert_eq!(pages.focus, Some(NodeId::from_raw(33)));
    }

    #[test]
    fn a_duplicate_page_id_is_refused() {
        let mut pages = filled();
        assert!(
            pages
                .add(page(2, 99))
                .unwrap_err()
                .contains("already exists")
        );
    }

    #[test]
    fn an_empty_tab_container_removes_itself_and_says_so() {
        let pages = Pages::new(ContainerKind::Tabs);
        assert_eq!(pages.empty_policy(), EmptyPolicy::RemoveContainer);
        assert!(!pages.empty_policy().keeps_the_container());
        assert!(pages.render().iter().any(|line| line.contains("removed")));
    }

    #[test]
    fn an_empty_scroll_container_shows_a_placeholder() {
        let pages = Pages::new(ContainerKind::Scroll);
        assert!(pages.empty_policy().keeps_the_container());
        assert!(
            pages
                .render()
                .iter()
                .any(|line| line.contains("placeholder"))
        );
    }

    #[test]
    fn stepping_an_empty_container_is_refused() {
        let mut pages = Pages::new(ContainerKind::Stack);
        assert!(pages.next().is_err());
        assert!(pages.previous().is_err());
    }
}
