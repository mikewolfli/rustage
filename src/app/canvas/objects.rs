// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Design-time object state — hidden, locked and paint order — object filtering and locating, and
//! multi-page tabs (task T09.07).
//!
//! The acceptance line is *"hiding does not change published visibility"*. There are two visibilities
//! here and they are different things: a node is **hidden** when a person puts it out of the way
//! while designing, and **published** when the document says the generated app should draw it. The
//! module keeps the first and reads the second, in a way that cannot blur them:
//!
//! * **The design state is its own map, and holds none of the document.** [`Objects`] maps a node id
//!   to a [`State`]; it keeps no [`Node`], so there is no field through which a hide could reach a
//!   published value. Hiding is invisible to the document *by construction*, not by discipline.
//! * **Published visibility is read from the document.** [`published`] answers what the model says —
//!   the `visible` property, defaulting to drawn — which is the generated app's question, not the
//!   designer's.
//! * **Paint order is one order, shared.** [`Objects::order`] is what the canvas and the object tree
//!   both read, so raising a node moves it on both surfaces rather than on one.
//! * **A filter queries the document and the design state together.** [`Filter`] matches a node by
//!   kind, by label and by design state, and [`Objects::filter`] answers the matching ids without
//!   changing anything, so "find the hidden images" is a read.
//! * **Pages are tabs, and there is always one.** [`Pages`] holds the open pages and the active one;
//!   closing the last page is refused rather than leaving a designer with nothing to look at.

#![allow(dead_code)]

use std::collections::BTreeMap;

use crate::app::search::quick_open::fuzzy_score;
use crate::contracts::identity::{NodeId, PageId};
use crate::contracts::model::{Node, NodeKind, Page, PropertyName, PropertyValue};

/// The property name a document uses for published visibility.
fn visible_property() -> PropertyName {
    PropertyName::new("visible").expect("a fixed, non-blank name")
}

/// The property name a node is found by.
fn label_property() -> PropertyName {
    PropertyName::new("name").expect("a fixed, non-blank name")
}

/// Whether the document publishes a node, read from the node itself (default: drawn).
pub fn published(node: &Node) -> bool {
    match node.property(&visible_property()) {
        Some(PropertyValue::Bool(value)) => *value,
        _ => true,
    }
}

/// The text a node is found by: its `name` property, if the document sets one.
pub fn label(node: &Node) -> &str {
    match node.property(&label_property()) {
        Some(PropertyValue::Text(text)) => text,
        _ => "",
    }
}

/// A node's design-time state: out of the way, and fixed in place. Not a document fact.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct State {
    hidden: bool,
    locked: bool,
}

impl State {
    /// The default state: shown and unlocked.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the node is put out of the way while designing.
    pub fn is_hidden(&self) -> bool {
        self.hidden
    }

    /// Whether the node is fixed in place while designing.
    pub fn is_locked(&self) -> bool {
        self.locked
    }

    /// Hides or shows the node. Writes no document.
    pub fn set_hidden(&mut self, hidden: bool) {
        self.hidden = hidden;
    }

    /// Locks or unlocks the node. Writes no document.
    pub fn set_locked(&mut self, locked: bool) {
        self.locked = locked;
    }

    /// The state in one word, for a report.
    pub fn render(&self) -> &'static str {
        match (self.hidden, self.locked) {
            (true, true) => "hidden locked",
            (true, false) => "hidden",
            (false, true) => "locked",
            (false, false) => "shown",
        }
    }
}

/// The design state of a document's nodes: what is hidden or locked, and the paint order.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Objects {
    states: BTreeMap<NodeId, State>,
    order: Vec<NodeId>,
}

impl Objects {
    /// No state and no paint order.
    pub fn new() -> Self {
        Self::default()
    }

    /// A node's design state, or the default (shown, unlocked).
    pub fn state(&self, node: NodeId) -> State {
        self.states.get(&node).copied().unwrap_or_default()
    }

    /// Whether a node is hidden in the designer.
    pub fn is_hidden(&self, node: NodeId) -> bool {
        self.state(node).is_hidden()
    }

    /// Whether a node is locked in the designer.
    pub fn is_locked(&self, node: NodeId) -> bool {
        self.state(node).is_locked()
    }

    /// Hides or shows a node in the designer. Reads and writes no document.
    pub fn set_hidden(&mut self, node: NodeId, hidden: bool) {
        self.states.entry(node).or_default().set_hidden(hidden);
    }

    /// Locks or unlocks a node in the designer. Reads and writes no document.
    pub fn set_locked(&mut self, node: NodeId, locked: bool) {
        self.states.entry(node).or_default().set_locked(locked);
    }

    /// The paint order, bottom to top.
    pub fn order(&self) -> &[NodeId] {
        &self.order
    }

    /// Registers a node at the top of the paint order.
    pub fn add(&mut self, node: NodeId) {
        self.order.retain(|existing| *existing != node);
        self.order.push(node);
    }

    /// Removes a node from the paint order; its design state is kept.
    pub fn remove(&mut self, node: NodeId) {
        self.order.retain(|existing| *existing != node);
    }

    /// Moves a node one step up the paint order and says whether it could.
    pub fn raise(&mut self, node: NodeId) -> bool {
        self.shift(node, 1)
    }

    /// Moves a node one step down the paint order and says whether it could.
    pub fn lower(&mut self, node: NodeId) -> bool {
        self.shift(node, -1)
    }

    /// The ids matching a filter, over the documents given and this design state, in id order.
    pub fn filter<'a>(
        &self,
        nodes: impl IntoIterator<Item = &'a Node>,
        filter: &Filter,
    ) -> Vec<NodeId> {
        let mut matched: Vec<NodeId> = nodes
            .into_iter()
            .filter(|node| self.matches(node, filter))
            .map(Node::id)
            .collect();
        matched.sort();
        matched
    }

    /// The ids whose label the text fuzzy-matches, over the documents given, in id order.
    pub fn locate<'a>(&self, nodes: impl IntoIterator<Item = &'a Node>, text: &str) -> Vec<NodeId> {
        let mut matched: Vec<NodeId> = nodes
            .into_iter()
            .filter(|node| fuzzy_score(text, label(node)).is_some())
            .map(Node::id)
            .collect();
        matched.sort();
        matched
    }

    /// Whether one node passes a filter.
    fn matches(&self, node: &Node, filter: &Filter) -> bool {
        if let Some(kind) = &filter.kind
            && node.kind() != kind
        {
            return false;
        }
        if let Some(text) = &filter.text
            && fuzzy_score(text, label(node)).is_none()
        {
            return false;
        }
        if let Some(state) = filter.state {
            let current = self.state(node.id());
            let holds = match state {
                StateFilter::Hidden => current.is_hidden(),
                StateFilter::Shown => !current.is_hidden(),
                StateFilter::Locked => current.is_locked(),
                StateFilter::Unlocked => !current.is_locked(),
            };
            if !holds {
                return false;
            }
        }
        true
    }

    /// Moves a node by `delta` positions in the paint order, saying whether it moved.
    fn shift(&mut self, node: NodeId, delta: isize) -> bool {
        let Some(index) = self.order.iter().position(|existing| *existing == node) else {
            return false;
        };
        let target = index as isize + delta;
        if target < 0 || target >= self.order.len() as isize {
            return false;
        }
        self.order.swap(index, target as usize);
        true
    }

    /// The design state in a report.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!("objects: {} state(s)", self.states.len())];
        for (node, state) in &self.states {
            lines.push(format!("  node {} {}", node.get(), state.render()));
        }
        lines.push(format!("order: {} node(s)", self.order.len()));
        for node in &self.order {
            lines.push(format!("  node {}", node.get()));
        }
        lines
    }
}

/// Which design state a filter asks for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StateFilter {
    /// Hidden in the designer.
    Hidden,
    /// Not hidden.
    Shown,
    /// Locked in the designer.
    Locked,
    /// Not locked.
    Unlocked,
}

/// What a filter asks of a node: each part is optional, and all parts must hold.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Filter {
    /// A kind the node must be.
    pub kind: Option<NodeKind>,
    /// A text the node's label must match.
    pub text: Option<String>,
    /// A design state the node must be in.
    pub state: Option<StateFilter>,
}

impl Filter {
    /// A filter that matches everything.
    pub fn new() -> Self {
        Self::default()
    }
}

/// The open pages and which one is being looked at.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Pages {
    pages: Vec<Page>,
    active: usize,
}

impl Pages {
    /// No pages.
    pub fn new() -> Self {
        Self::default()
    }

    /// The open pages, in tab order.
    pub fn pages(&self) -> &[Page] {
        &self.pages
    }

    /// How many pages are open.
    pub fn len(&self) -> usize {
        self.pages.len()
    }

    /// Whether no page is open.
    pub fn is_empty(&self) -> bool {
        self.pages.is_empty()
    }

    /// The active page, if there is one.
    pub fn active(&self) -> Option<&Page> {
        self.pages.get(self.active)
    }

    /// Where the active page is.
    pub fn active_index(&self) -> usize {
        self.active
    }

    /// Adds a page, refusing a duplicate identity, and makes it active.
    pub fn add(&mut self, page: Page) -> Result<(), String> {
        if self.pages.iter().any(|open| open.id() == page.id()) {
            return Err(format!("page {} is already open", page.id().get()));
        }
        self.pages.push(page);
        self.active = self.pages.len() - 1;
        Ok(())
    }

    /// Makes a page active, refusing one that is not open.
    pub fn activate(&mut self, id: PageId) -> Result<(), String> {
        let Some(index) = self.pages.iter().position(|page| page.id() == id) else {
            return Err(format!("page {} is not open", id.get()));
        };
        self.active = index;
        Ok(())
    }

    /// Renames a page, refusing a blank name and one that is not open.
    pub fn rename(&mut self, id: PageId, name: &str) -> Result<(), String> {
        let Some(index) = self.pages.iter().position(|page| page.id() == id) else {
            return Err(format!("page {} is not open", id.get()));
        };
        let renamed = Page::new(id, name, self.pages[index].root())
            .ok_or_else(|| format!("{name:?} is not a page name"))?;
        self.pages[index] = renamed;
        Ok(())
    }

    /// Closes a page, refusing the last one and a page that is not open.
    ///
    /// The active page moves to one that still exists, so closing what a person was looking at does
    /// not leave the selection on nothing.
    pub fn close(&mut self, id: PageId) -> Result<(), String> {
        let Some(index) = self.pages.iter().position(|page| page.id() == id) else {
            return Err(format!("page {} is not open", id.get()));
        };
        if self.pages.len() == 1 {
            return Err("the last page cannot be closed".to_owned());
        }
        self.pages.remove(index);
        if index <= self.active && self.active > 0 {
            self.active -= 1;
        }
        if self.active >= self.pages.len() {
            self.active = self.pages.len() - 1;
        }
        Ok(())
    }

    /// The pages in a report.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "pages: {} (active {})",
            self.pages.len(),
            self.active
        )];
        for (index, page) in self.pages.iter().enumerate() {
            let marker = if index == self.active { "*" } else { " " };
            lines.push(format!(
                "{marker} page {} {:?} root {}",
                page.id().get(),
                page.name(),
                page.root().get()
            ));
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{Filter, Objects, Pages, StateFilter, label, published};
    use crate::contracts::identity::{NodeId, PageId};
    use crate::contracts::model::{Node, NodeKind, Page, PropertyName, PropertyValue};
    use std::collections::BTreeMap;

    fn node(value: u64) -> NodeId {
        NodeId::from_raw(value)
    }

    fn page(value: u64) -> PageId {
        PageId::from_raw(value)
    }

    /// A document node of a kind, with an optional `visible` and `name`.
    fn document(id: NodeId, kind: &str, visible: Option<bool>, name: Option<&str>) -> Node {
        let mut properties = BTreeMap::new();
        if let Some(visible) = visible {
            properties.insert(
                PropertyName::new("visible").expect("a name"),
                PropertyValue::Bool(visible),
            );
        }
        if let Some(name) = name {
            properties.insert(
                PropertyName::new("name").expect("a name"),
                PropertyValue::Text(name.to_owned()),
            );
        }
        Node::record(
            page(0),
            id,
            NodeKind::new(kind).expect("a kind"),
            properties,
        )
    }

    #[test]
    fn hiding_does_not_change_published_visibility() {
        let drawn = document(node(1), "button", Some(true), None);
        let hidden_from_app = document(node(2), "button", Some(false), None);
        let before_drawn = drawn.clone();
        let before_hidden = hidden_from_app.clone();

        let mut objects = Objects::new();
        objects.set_hidden(node(1), true);
        objects.set_hidden(node(2), false);

        assert_eq!(drawn, before_drawn, "hiding writes no document");
        assert_eq!(hidden_from_app, before_hidden);
        assert!(objects.is_hidden(node(1)), "the designer hides node 1");
        assert!(published(&drawn), "the app still draws node 1");
        assert!(!published(&hidden_from_app), "the app still skips node 2");
    }

    #[test]
    fn a_node_with_no_visible_property_is_drawn() {
        let node = document(node(1), "button", None, None);
        assert!(published(&node));
    }

    #[test]
    fn hiding_and_locking_are_independent() {
        let mut objects = Objects::new();
        objects.set_hidden(node(1), true);
        objects.set_locked(node(1), true);
        objects.set_hidden(node(1), false);
        assert!(!objects.is_hidden(node(1)));
        assert!(objects.is_locked(node(1)), "showing does not unlock");
    }

    #[test]
    fn raising_and_lowering_change_the_paint_order() {
        let mut objects = Objects::new();
        for id in [1, 2, 3] {
            objects.add(node(id));
        }
        assert!(objects.raise(node(1)));
        assert_eq!(objects.order(), &[node(2), node(1), node(3)]);
        assert!(objects.lower(node(1)));
        assert!(!objects.raise(node(3)), "the top cannot rise further");
        assert_eq!(objects.order(), &[node(1), node(2), node(3)]);
    }

    #[test]
    fn a_filter_matches_kind_label_and_state() {
        let nodes = [
            document(node(1), "image", None, Some("hero")),
            document(node(2), "button", None, Some("save")),
            document(node(3), "image", None, Some("thumb")),
        ];
        let mut objects = Objects::new();
        objects.set_hidden(node(3), true);

        let images = Filter {
            kind: NodeKind::new("image"),
            ..Filter::new()
        };
        assert_eq!(
            objects.filter(nodes.iter(), &images),
            vec![node(1), node(3)]
        );

        let hidden = Filter {
            state: Some(StateFilter::Hidden),
            ..Filter::new()
        };
        assert_eq!(objects.filter(nodes.iter(), &hidden), vec![node(3)]);

        let hero = Filter {
            text: Some("hero".to_owned()),
            ..Filter::new()
        };
        assert_eq!(objects.filter(nodes.iter(), &hero), vec![node(1)]);
    }

    #[test]
    fn locating_finds_by_label() {
        let nodes = [
            document(node(1), "image", None, Some("hero banner")),
            document(node(2), "button", None, Some("save")),
        ];
        let objects = Objects::new();
        assert_eq!(objects.locate(nodes.iter(), "hb"), vec![node(1)]);
        assert_eq!(label(&nodes[0]), "hero banner");
        assert!(objects.locate(nodes.iter(), "zzz").is_empty());
    }

    #[test]
    fn the_last_page_cannot_be_closed() {
        let mut pages = Pages::new();
        pages
            .add(Page::new(page(1), "main", node(0)).expect("a page"))
            .expect("the first page");
        assert!(pages.close(page(1)).is_err());
        assert_eq!(pages.len(), 1);
    }

    #[test]
    fn closing_the_active_page_moves_to_a_page_that_exists() {
        let mut pages = Pages::new();
        for (id, name) in [(1, "main"), (2, "settings"), (3, "about")] {
            pages
                .add(Page::new(page(id), name, node(0)).expect("a page"))
                .expect("a fresh page");
        }
        pages.activate(page(3)).expect("an open page");
        pages.close(page(3)).expect("not the last page");
        assert_eq!(pages.active().map(|page| page.name()), Some("settings"));
    }

    #[test]
    fn adding_a_duplicate_page_is_refused() {
        let mut pages = Pages::new();
        pages
            .add(Page::new(page(1), "main", node(0)).expect("a page"))
            .expect("the first page");
        assert!(
            pages
                .add(Page::new(page(1), "again", node(1)).expect("a page"))
                .is_err()
        );
    }

    #[test]
    fn activating_a_page_that_is_not_open_is_refused() {
        let mut pages = Pages::new();
        pages
            .add(Page::new(page(1), "main", node(0)).expect("a page"))
            .expect("the first page");
        assert!(pages.activate(page(9)).is_err());
        assert_eq!(
            pages.active().map(|page| page.name()),
            Some("main"),
            "a refusal leaves the active page where it was"
        );
    }

    #[test]
    fn renaming_a_page_keeps_its_identity_and_root() {
        let mut pages = Pages::new();
        pages
            .add(Page::new(page(1), "main", node(7)).expect("a page"))
            .expect("the first page");
        pages.rename(page(1), "home").expect("an open page");
        let active = pages.active().expect("an active page");
        assert_eq!(active.name(), "home");
        assert_eq!(active.id(), page(1));
        assert_eq!(active.root(), node(7));
    }
}
