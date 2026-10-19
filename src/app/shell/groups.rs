// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Editor groups: splitting the editor area, several views of one document, moving between the
//! groups, and moving a tab from one to another (task T04.04).
//!
//! The acceptance line is *"edits synchronize, and scroll/selection viewports do not force-overwrite
//! each other"*. It is one sentence about two different things, so the module keeps them apart:
//!
//! * **One document record, many views.** A document's text lives once in [`Groups`], keyed by its
//!   identity, and a view is a [`Tab`](super::tabs::Tab) that reads it by name. Typing through any
//!   view therefore shows up in every view *by construction*: there is no per-view copy for the text
//!   to diverge into, which is what "edits synchronize" has to mean if it is to be checkable.
//! * **A viewport belongs to the view.** Scroll and selection live on the [`Viewport`], and
//!   [`Groups::scroll`]/[`Groups::select`] touch exactly one view. Two views of one document share
//!   the text and disagree about where they are looking; nothing in this module ever writes a second
//!   view's viewport, so "one view drags the other" cannot happen.
//!
//! Around that core, four decisions worth stating:
//!
//! * **An axis is named by the direction its panes run, not "horizontal".** `Axis::LeftRight` puts
//!   the panes side by side (a vertical divider); `Axis::TopBottom` stacks them (a horizontal
//!   divider). "Horizontal split" is exactly the phrase people disagree about, so it is not used.
//! * **Splitting nests on the same axis and wraps on a different one.** Splitting twice sideways
//!   gives three columns, not a column of two-column boxes; splitting the other way wraps the
//!   focused group, which is the layout a person drew. This is a rule about [`Node`], not a chain of
//!   ad-hoc reshapes.
//! * **Navigation is geometric.** Leaps are computed from each group's rectangle (equal subdivision
//!   of the area), so "focus the group to the right" means the one whose column is to the right, and
//!   not merely the next one in an internal order that happens to be traversal order.
//! * **Closing a group is not closing documents.** [`Groups::close_focus_group`] moves the group's
//!   views into a neighbour and says how many moved; it never drops them silently, and the last
//!   group is not closable, because a window with no editor is not a layout, it is a bug.

// The group model is rendered by the window (T04.06) and driven by the split/join commands
// (T04.05/T05.01) as those land; until then the tests and the `--groups` script exercise it, and the
// silence is stated once here with its reason, as the neighbouring trees do.
#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use super::tabs::Tabs;

/// The direction a split lays its panes out.
///
/// Named by the panes, because the divider's own orientation is the other reading and the ambiguous
/// word "horizontal" is the one people keep meaning differently.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Axis {
    /// Panes side by side, sharing a vertical divider.
    LeftRight,
    /// Panes stacked, sharing a horizontal divider.
    #[default]
    TopBottom,
}

impl Axis {
    /// The axis' name, as a command line or a layout file writes it.
    pub const fn name(self) -> &'static str {
        match self {
            Self::LeftRight => "left-right",
            Self::TopBottom => "top-bottom",
        }
    }

    /// The axis a name asks for, when it is one.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|axis| axis.name() == name)
    }

    /// Every axis.
    pub const ALL: [Self; 2] = [Self::LeftRight, Self::TopBottom];
}

impl fmt::Display for Axis {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// A direction to move the focus in, for keyboard group navigation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Direction {
    /// Toward the group whose column is to the left.
    Left,
    /// Toward the group whose column is to the right.
    Right,
    /// Toward the group whose row is above.
    Up,
    /// Toward the group whose row is below.
    Down,
}

impl Direction {
    /// The direction's name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Up => "up",
            Self::Down => "down",
        }
    }

    /// The direction a name asks for, when it is one.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|direction| direction.name() == name)
    }

    /// Every direction.
    pub const ALL: [Self; 4] = [Self::Left, Self::Right, Self::Up, Self::Down];
}

impl fmt::Display for Direction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// The arrangement of the groups as a value (task T04.05).
///
/// Leaves name **positions** in layout order, not [`GroupId`]s: a saved layout must be loadable into
/// a fresh editor area, and an id is a fact about one live session. A shape therefore describes the
/// structure (which axis, how many panes, how they nest) with nothing session-specific in it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Shape {
    /// The pane at this position in layout order.
    Leaf(usize),
    /// Several panes laid out along one axis.
    Split { axis: Axis, children: Vec<Shape> },
}

impl Shape {
    /// How many leaves the shape names.
    pub fn leaf_count(&self) -> usize {
        match self {
            Self::Leaf(_) => 1,
            Self::Split { children, .. } => children.iter().map(Self::leaf_count).sum(),
        }
    }

    /// Every leaf position, in order.
    pub fn leaves(&self) -> Vec<usize> {
        let mut out = Vec::new();
        self.collect_leaves(&mut out);
        out
    }

    fn collect_leaves(&self, out: &mut Vec<usize>) {
        match self {
            Self::Leaf(index) => out.push(*index),
            Self::Split { children, .. } => {
                for child in children {
                    child.collect_leaves(out);
                }
            }
        }
    }

    /// The shape as one line, for a report.
    pub fn render(&self) -> String {
        match self {
            Self::Leaf(index) => format!("leaf {index}"),
            Self::Split { axis, children } => format!(
                "{}[{}]",
                axis.name(),
                children
                    .iter()
                    .map(Self::render)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }

    /// A single pane: the shape of an editor area that was never split.
    pub fn single() -> Self {
        Self::Leaf(0)
    }
}

/// A group's stable identity inside one editor area.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct GroupId(u64);

impl GroupId {
    /// Rebuilds an id from the number it was written as (for a script or a layout file).
    pub const fn from_raw(value: u64) -> Self {
        Self(value)
    }

    /// The number this id is written as.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// The split tree: leaves are groups, internal nodes are splits.
#[derive(Clone, Debug)]
enum Node {
    /// One group.
    Leaf(GroupId),
    /// Several children laid out along one axis.
    Split { axis: Axis, children: Vec<Node> },
}

/// A group: its own strip of tabs.
#[derive(Clone, Debug)]
struct Group {
    tabs: Tabs,
}

/// A rectangle in the editor area, in arbitrary cells. Only ratios matter, so the area is fixed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Rect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl Rect {
    /// The whole area the split tree is laid out in.
    const AREA: Self = Self {
        x: 0,
        y: 0,
        width: 1000,
        height: 1000,
    };

    fn center(&self) -> (u32, u32) {
        (self.x + self.width / 2, self.y + self.height / 2)
    }

    /// Splits this rectangle into `parts` along an axis, remainder given to the last part.
    fn divide(&self, axis: Axis, parts: usize) -> Vec<Self> {
        let parts_u32 = u32::try_from(parts.max(1)).unwrap_or(1);
        let mut rects = Vec::with_capacity(parts);
        match axis {
            Axis::LeftRight => {
                let each = self.width / parts_u32;
                let mut x = self.x;
                for part in 0..parts_u32 {
                    let width = if part + 1 == parts_u32 {
                        self.x + self.width - x
                    } else {
                        each
                    };
                    rects.push(Self {
                        x,
                        y: self.y,
                        width,
                        height: self.height,
                    });
                    x += width;
                }
            }
            Axis::TopBottom => {
                let each = self.height / parts_u32;
                let mut y = self.y;
                for part in 0..parts_u32 {
                    let height = if part + 1 == parts_u32 {
                        self.y + self.height - y
                    } else {
                        each
                    };
                    rects.push(Self {
                        x: self.x,
                        y,
                        width: self.width,
                        height,
                    });
                    y += height;
                }
            }
        }
        rects
    }
}

/// The editor area: a tree of groups, one shared buffer per open document, and which group has the
/// keyboard.
#[derive(Clone, Debug)]
pub struct Groups {
    root: Node,
    groups: BTreeMap<GroupId, Group>,
    focus: GroupId,
    next_id: u64,
    documents: BTreeMap<String, String>,
    limit: usize,
}

impl Groups {
    /// An editor area with one empty group.
    pub fn new(limit: usize) -> Self {
        let id = GroupId(0);
        let mut groups = BTreeMap::new();
        groups.insert(
            id,
            Group {
                tabs: Tabs::new(limit),
            },
        );
        Self {
            root: Node::Leaf(id),
            groups,
            focus: id,
            next_id: 1,
            documents: BTreeMap::new(),
            limit,
        }
    }

    /// The group with the keyboard.
    pub const fn focus(&self) -> GroupId {
        self.focus
    }

    /// How many groups there are.
    pub fn len(&self) -> usize {
        self.groups.len()
    }

    /// Whether the editor area is empty of groups. It never is, but a caller asking is not an error.
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    /// The groups, in layout order (the order the split tree walks them).
    pub fn group_ids(&self) -> Vec<GroupId> {
        let mut ids = Vec::new();
        Self::leaves(&self.root, &mut ids);
        ids
    }

    /// One group's strip of tabs, if the group exists.
    pub fn tabs(&self, group: GroupId) -> Option<&Tabs> {
        self.groups.get(&group).map(|group| &group.tabs)
    }

    /// A group's tabs for an operation that changes the active tab.
    ///
    /// Reading a group is enough for rendering; switching tabs is an edit, and an editor area
    /// without this accessor would force the window layer to keep a second active-tab mirror.
    pub fn tabs_mut(&mut self, group: GroupId) -> Option<&mut Tabs> {
        self.groups.get_mut(&group).map(|group| &mut group.tabs)
    }

    /// The group that has the keyboard, its strip.
    fn focus_tabs(&self) -> &Tabs {
        &self.groups[&self.focus].tabs
    }

    /// The focused group's strip, mutably.
    fn focus_tabs_mut(&mut self) -> &mut Tabs {
        &mut self
            .groups
            .get_mut(&self.focus)
            .expect("the focus is a group")
            .tabs
    }

    /// The document's text, wherever it is open. One record, read by every view of it.
    pub fn text(&self, document: &str) -> Option<&str> {
        self.documents.get(document).map(String::as_str)
    }

    /// Every open document, in name order.
    pub fn documents(&self) -> Vec<&str> {
        self.documents.keys().map(String::as_str).collect()
    }

    /// Every `(group, tab index)` that shows a document.
    pub fn views_of(&self, document: &str) -> Vec<(GroupId, usize)> {
        let mut views = Vec::new();
        for id in self.group_ids() {
            let group = &self.groups[&id];
            for (index, tab) in group.tabs.tabs().iter().enumerate() {
                if tab.document() == document {
                    views.push((id, index));
                }
            }
        }
        views
    }

    /// Splits the focused group along an axis, and focuses the new, empty group.
    ///
    /// Same axis as the parent split: the new group becomes a sibling, so a second sideways split
    /// makes three columns. Different axis: the focused group is wrapped, so a split the other way
    /// lays a row beside a column instead of renaming what is already there.
    pub fn split(&mut self, axis: Axis) -> GroupId {
        let id = GroupId(self.next_id);
        self.next_id += 1;
        let focus = self.focus;
        Self::split_at(&mut self.root, focus, axis, id);
        self.groups.insert(
            id,
            Group {
                tabs: Tabs::new(self.limit),
            },
        );
        self.focus = id;
        id
    }

    /// Moves the focus to the next group in layout order, wrapping.
    pub fn focus_next(&mut self) {
        self.step_focus(1);
    }

    /// Moves the focus to the previous group in layout order, wrapping.
    pub fn focus_previous(&mut self) {
        self.step_focus(-1);
    }

    /// Moves the focus to the group in a direction, when there is one; reports whether it moved.
    ///
    /// The leap is decided from the rectangles the split tree implies, so "right" means the group
    /// whose column is to the right of the keyboard's, not the next one in traversal order.
    pub fn focus_direction(&mut self, direction: Direction) -> bool {
        let mut rects = BTreeMap::new();
        Self::rects(&self.root, Rect::AREA, &mut rects);
        let Some(from) = rects.get(&self.focus).copied() else {
            return false;
        };
        let (from_x, from_y) = from.center();
        let mut best: Option<(u32, GroupId)> = None;
        for (id, rect) in &rects {
            if *id == self.focus || !Self::is_in_direction(direction, &from, rect) {
                continue;
            }
            let (x, y) = rect.center();
            let distance = from_x.abs_diff(x) + from_y.abs_diff(y);
            match best {
                Some((best_distance, _)) if best_distance <= distance => {}
                _ => best = Some((distance, *id)),
            }
        }
        match best {
            Some((_, id)) => {
                self.focus = id;
                true
            }
            None => false,
        }
    }

    /// Focuses a named group, refusing one that is not there.
    pub fn focus_group(&mut self, group: GroupId) -> Result<(), String> {
        if !self.groups.contains_key(&group) {
            return Err(format!("there is no group {}", group.get()));
        }
        self.focus = group;
        Ok(())
    }

    /// Opens a document in the focused group's preview tab, registering it if it is new.
    pub fn open(&mut self, document: impl Into<String>) {
        let document = document.into();
        self.documents.entry(document.clone()).or_default();
        self.focus_tabs_mut().preview(document);
    }

    /// Opens another view of a document in the focused group, so one document can be shown twice.
    ///
    /// This is the "same buffer, several views" case when the caller first splits: the same named
    /// document is in two groups, and (see the module note) both views read the one buffer.
    pub fn another_view(&mut self, document: impl Into<String>) {
        let document = document.into();
        self.documents.entry(document.clone()).or_default();
        self.focus_tabs_mut().another_view(document);
    }

    /// Pins a document in the focused group, registering it if it is new.
    pub fn pin(&mut self, document: impl Into<String>) {
        let document = document.into();
        self.documents.entry(document.clone()).or_default();
        self.focus_tabs_mut().pin(document);
    }

    /// Types new text into the document the focused view shows.
    ///
    /// It writes the one buffer, which is why every other view of the same document sees it. A
    /// focused group with no view of any document has nothing to type into, and says so.
    pub fn type_into(&mut self, text: impl Into<String>) -> Result<(), String> {
        let document = self
            .focus_tabs()
            .active()
            .map(|tab| tab.document().to_owned())
            .ok_or_else(|| "the focused group shows no document to type into".to_owned())?;
        self.documents.insert(document, text.into());
        Ok(())
    }

    /// Scrolls the focused view. Exactly one view's viewport changes.
    pub fn scroll(&mut self, offset: u32) -> Result<(), String> {
        let index = self
            .focus_tabs()
            .active_index()
            .ok_or_else(|| "the focused group shows no document to scroll".to_owned())?;
        self.focus_tabs_mut().scroll_to(index, offset)
    }

    /// Sets the focused view's selection. Exactly one view's viewport changes.
    pub fn select(&mut self, anchor: u32, cursor: u32) -> Result<(), String> {
        let index = self
            .focus_tabs()
            .active_index()
            .ok_or_else(|| "the focused group shows no document to select in".to_owned())?;
        self.focus_tabs_mut().select(index, anchor, cursor)
    }

    /// Moves a tab out of one group and into another, carrying its viewport and dirty flag.
    pub fn move_tab(
        &mut self,
        from: GroupId,
        from_index: usize,
        to: GroupId,
        to_index: usize,
    ) -> Result<(), String> {
        if from == to {
            let tabs = &mut self
                .groups
                .get_mut(&from)
                .ok_or_else(|| format!("there is no group {}", from.get()))?
                .tabs;
            let tab = tabs.take(from_index)?;
            tabs.insert(to_index, tab)?;
            return Ok(());
        }
        if !self.groups.contains_key(&to) {
            return Err(format!("there is no group {}", to.get()));
        }
        let tab = self
            .groups
            .get_mut(&from)
            .ok_or_else(|| format!("there is no group {}", from.get()))?
            .tabs
            .take(from_index)?;
        self.groups
            .get_mut(&to)
            .expect("the target group was checked")
            .tabs
            .insert(to_index, tab)?;
        Ok(())
    }

    /// Closes the focused group, moving its views to a neighbour, and says how many moved.
    ///
    /// The last group is not closable: closing it would leave an editor area with nowhere to edit,
    /// and the tabs would have to be dropped. Refusing is the only outcome that keeps them.
    pub fn close_focus_group(&mut self) -> Result<usize, String> {
        if self.groups.len() <= 1 {
            return Err(
                "the last group cannot be closed; a window needs somewhere to edit".to_owned(),
            );
        }
        let closing = self.focus;
        let order = self.group_ids();
        let position = order
            .iter()
            .position(|id| *id == closing)
            .expect("the focus is a group");
        let neighbour = if position > 0 {
            order[position - 1]
        } else {
            order[position + 1]
        };

        let moved = self
            .groups
            .get(&closing)
            .map(|group| group.tabs.len())
            .unwrap_or(0);
        let tabs: Vec<_> = self
            .groups
            .get_mut(&closing)
            .expect("the focus is a group")
            .tabs
            .tabs()
            .to_vec();
        let target = &mut self
            .groups
            .get_mut(&neighbour)
            .expect("the neighbour is a group")
            .tabs;
        for tab in tabs {
            let at = target.len();
            target
                .insert(at, tab)
                .expect("appending to a strip is in range");
        }

        Self::remove_leaf(&mut self.root, closing);
        Self::collapse(&mut self.root);
        self.groups.remove(&closing);
        self.focus = neighbour;
        Ok(moved)
    }

    /// The arrangement of the groups: leaves name positions in layout order.
    pub fn shape(&self) -> Shape {
        let order = self.group_ids();
        Self::shape_of(&self.root, &order)
    }

    /// Per group (in layout order), the documents its strip shows.
    pub fn arrangement(&self) -> Vec<Vec<String>> {
        self.group_ids()
            .iter()
            .map(|id| {
                self.groups[id]
                    .tabs
                    .tabs()
                    .iter()
                    .map(|tab| tab.document().to_owned())
                    .collect()
            })
            .collect()
    }

    /// Rearranges the editor area to a shape and an arrangement, and **never closes a document**.
    ///
    /// This is how a layout is applied (task T04.05). Two rules make it safe to call with a layout a
    /// person did not write:
    ///
    /// * **Every document named by the arrangement is opened**, in the pane the arrangement places
    ///   it in; the text each one had is kept, so rearranging is not a way to lose an edit.
    /// * **A document that is open but not named is kept**, placed in the first pane rather than
    ///   dropped. A layout describes an arrangement, not a session, so applying one cannot make a
    ///   document disappear; that is what makes "reset loses no document" a property rather than a
    ///   promise.
    ///
    /// A shape that does not agree with the arrangement is refused and nothing is changed.
    pub fn rearrange(&mut self, shape: &Shape, arrangement: &[Vec<String>]) -> Result<(), String> {
        let leaves = shape.leaf_count();
        if leaves == 0 || arrangement.is_empty() {
            return Err("an arrangement needs at least one pane".to_owned());
        }
        if arrangement.len() != leaves {
            return Err(format!(
                "the arrangement names {} pane(s) but the shape has {leaves}",
                arrangement.len()
            ));
        }
        for leaf in shape.leaves() {
            if leaf >= arrangement.len() {
                return Err(format!(
                    "the shape names pane {leaf}, past the {} in the arrangement",
                    arrangement.len()
                ));
            }
        }

        // Whatever is open now, with its text, so nothing that is not named goes missing.
        let old_documents = std::mem::take(&mut self.documents);
        let open: Vec<String> = self.arrangement().into_iter().flatten().collect();
        let mut named: BTreeSet<String> = BTreeSet::new();
        for documents in arrangement {
            for document in documents {
                named.insert(document.clone());
            }
        }

        let mut groups = BTreeMap::new();
        let mut ids = Vec::with_capacity(leaves);
        for (index, documents) in arrangement.iter().enumerate() {
            let id = GroupId(index as u64);
            ids.push(id);
            let mut tabs = Tabs::new(self.limit);
            for document in documents {
                tabs.pin(document.clone());
            }
            groups.insert(id, Group { tabs });
        }
        if let Some(first) = ids.first().copied() {
            for document in &open {
                if !named.contains(document) {
                    groups
                        .get_mut(&first)
                        .expect("the first pane exists")
                        .tabs
                        .pin(document.clone());
                }
            }
        }

        let root = Self::build_shape(shape, &ids)?;
        self.root = root;
        self.groups = groups;
        self.next_id = leaves as u64;
        self.focus = ids[0];

        // The documents that are open now, each keeping the text it had; anything else is forgotten.
        let mut documents = BTreeMap::new();
        for document in self.arrangement().into_iter().flatten() {
            let text = old_documents.get(&document).cloned().unwrap_or_default();
            documents.insert(document, text);
        }
        self.documents = documents;
        Ok(())
    }

    /// The shape of a subtree, with each leaf named by its position in layout order.
    fn shape_of(node: &Node, order: &[GroupId]) -> Shape {
        match node {
            Node::Leaf(id) => Shape::Leaf(order.iter().position(|group| group == id).unwrap_or(0)),
            Node::Split { axis, children } => Shape::Split {
                axis: *axis,
                children: children
                    .iter()
                    .map(|child| Self::shape_of(child, order))
                    .collect(),
            },
        }
    }

    /// Builds a split tree from a shape and the groups the leaves name.
    fn build_shape(shape: &Shape, ids: &[GroupId]) -> Result<Node, String> {
        match shape {
            Shape::Leaf(index) => ids.get(*index).copied().map(Node::Leaf).ok_or_else(|| {
                format!("the shape names pane {index}, which is not in the arrangement")
            }),
            Shape::Split { axis, children } => {
                if children.is_empty() {
                    return Err("a split with no panes is not an arrangement".to_owned());
                }
                let mut built = Vec::with_capacity(children.len());
                for child in children {
                    built.push(Self::build_shape(child, ids)?);
                }
                Ok(Node::Split {
                    axis: *axis,
                    children: built,
                })
            }
        }
    }

    /// The editor area in one report.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "{} group(s); focus {}; {} document(s)",
            self.len(),
            self.focus.get(),
            self.documents.len()
        )];
        for id in self.group_ids() {
            let group = &self.groups[&id];
            let marker = if id == self.focus { " [focus]" } else { "" };
            lines.push(format!(" group {}{marker}", id.get()));
            for (index, tab) in group.tabs.tabs().iter().enumerate() {
                let active = if group.tabs.active_index() == Some(index) {
                    "*"
                } else {
                    " "
                };
                let kind = if tab.is_preview() {
                    "preview"
                } else {
                    "pinned"
                };
                lines.push(format!(
                    "  {active} {index}: {} [{kind}] {}",
                    tab.document(),
                    tab.viewport().render()
                ));
            }
        }
        if !self.documents.is_empty() {
            let summary = self
                .documents
                .iter()
                .map(|(name, text)| format!("{name} ({} char(s))", text.chars().count()))
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(format!("documents: {summary}"));
        }
        lines
    }

    /// The leaves of a tree, in order.
    fn leaves(node: &Node, out: &mut Vec<GroupId>) {
        match node {
            Node::Leaf(id) => out.push(*id),
            Node::Split { children, .. } => {
                for child in children {
                    Self::leaves(child, out);
                }
            }
        }
    }

    /// Each group's rectangle, by equal subdivision of the area.
    fn rects(node: &Node, area: Rect, out: &mut BTreeMap<GroupId, Rect>) {
        match node {
            Node::Leaf(id) => {
                out.insert(*id, area);
            }
            Node::Split { axis, children } => {
                for (child, rect) in children.iter().zip(area.divide(*axis, children.len())) {
                    Self::rects(child, rect, out);
                }
            }
        }
    }

    /// Whether `rect` lies in `direction` of `from`, with no overlap.
    fn is_in_direction(direction: Direction, from: &Rect, rect: &Rect) -> bool {
        match direction {
            Direction::Left => rect.x + rect.width <= from.x,
            Direction::Right => rect.x >= from.x + from.width,
            Direction::Up => rect.y + rect.height <= from.y,
            Direction::Down => rect.y >= from.y + from.height,
        }
    }

    /// Moves the focus by one step through layout order, wrapping.
    fn step_focus(&mut self, step: isize) {
        let order = self.group_ids();
        if order.is_empty() {
            return;
        }
        let position = order.iter().position(|id| *id == self.focus).unwrap_or(0);
        let count = order.len() as isize;
        let next = ((position as isize + step).rem_euclid(count)) as usize;
        self.focus = order[next];
    }

    /// Inserts a new leaf beside (or wrapping) the target group.
    ///
    /// Returns whether the target was found in this subtree.
    fn split_at(node: &mut Node, target: GroupId, axis: Axis, new: GroupId) -> bool {
        match node {
            Node::Leaf(id) if *id == target => {
                *node = Node::Split {
                    axis,
                    children: vec![Node::Leaf(*id), Node::Leaf(new)],
                };
                true
            }
            Node::Leaf(_) => false,
            Node::Split {
                axis: own,
                children,
            } => {
                if *own == axis {
                    for (position, child) in children.iter().enumerate() {
                        if matches!(child, Node::Leaf(id) if *id == target) {
                            children.insert(position + 1, Node::Leaf(new));
                            return true;
                        }
                    }
                }
                for child in children.iter_mut() {
                    if Self::split_at(child, target, axis, new) {
                        return true;
                    }
                }
                false
            }
        }
    }

    /// Removes a leaf from the tree. Returns whether it was in this subtree.
    fn remove_leaf(node: &mut Node, target: GroupId) -> bool {
        if let Node::Split { children, .. } = node {
            if let Some(position) = children
                .iter()
                .position(|child| matches!(child, Node::Leaf(id) if *id == target))
            {
                children.remove(position);
                return true;
            }
            for child in children.iter_mut() {
                if Self::remove_leaf(child, target) {
                    return true;
                }
            }
        }
        false
    }

    /// Collapses splits that have one child (or none) into that child.
    fn collapse(node: &mut Node) {
        if let Node::Split { children, .. } = node {
            for child in children.iter_mut() {
                Self::collapse(child);
            }
            let single = match children.as_slice() {
                [only] => Some(only.clone()),
                _ => None,
            };
            if let Some(only) = single {
                *node = only;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Axis, Direction, Groups, Shape};

    fn groups() -> Groups {
        Groups::new(4)
    }

    #[test]
    fn one_document_in_two_groups_reads_one_buffer_for_both_views() {
        let mut area = groups();
        area.open("pages/a.json");
        area.split(Axis::LeftRight);
        area.another_view("pages/a.json");

        let views = area.views_of("pages/a.json");
        assert_eq!(
            views.len(),
            2,
            "the document is shown in two views: {views:?}"
        );

        area.type_into("hello").unwrap();
        assert_eq!(area.text("pages/a.json"), Some("hello"));
        // Both views read the one record, so neither can be holding something else.
        assert_eq!(area.views_of("pages/a.json").len(), 2);
    }

    #[test]
    fn one_views_viewport_never_overwrites_anothers() {
        let mut area = groups();
        area.open("pages/a.json");
        let first = area.focus();
        area.split(Axis::TopBottom);
        area.another_view("pages/a.json");
        let second = area.focus();

        area.scroll(120).unwrap();
        area.select(4, 9).unwrap();

        // The view in the *other* group did not move.
        let first_view = &area.tabs(first).unwrap().tabs()[0];
        assert_eq!(first_view.viewport().scroll(), 0);
        assert_eq!(first_view.viewport().selection(), None);
        assert_eq!(
            area.tabs(second).unwrap().tabs()[0].viewport().scroll(),
            120
        );

        // And scrubbing the first view leaves the second one where it was.
        area.focus_group(first).unwrap();
        area.scroll(7).unwrap();
        assert_eq!(area.tabs(first).unwrap().tabs()[0].viewport().scroll(), 7);
        assert_eq!(
            area.tabs(first).unwrap().tabs()[0].viewport().selection(),
            None
        );
        assert_eq!(
            area.tabs(second).unwrap().tabs()[0].viewport().scroll(),
            120
        );
        assert_eq!(
            area.tabs(second).unwrap().tabs()[0].viewport().selection(),
            Some((4, 9))
        );
    }

    #[test]
    fn a_second_split_on_the_same_axis_makes_a_sibling_and_a_different_axis_wraps() {
        let mut area = groups();
        let first = area.focus();
        area.split(Axis::LeftRight);
        let second = area.focus();
        area.split(Axis::LeftRight);
        let third = area.focus();

        // Three columns in a row: the middle split is the root, with three leaves.
        assert_eq!(area.group_ids(), vec![first, second, third]);

        // Splitting the other way wraps only the focused group, keeping the others' columns.
        area.focus_group(first).unwrap();
        area.split(Axis::TopBottom);
        assert_eq!(area.len(), 4);
        assert_eq!(area.group_ids().len(), 4);
    }

    #[test]
    fn navigation_walks_layout_order_and_follows_geometry() {
        let mut area = groups();
        let first = area.focus();
        area.split(Axis::LeftRight);
        area.split(Axis::LeftRight);

        // Layout order is left to right, so next from the third wraps to the first.
        area.focus_next();
        assert_eq!(area.focus(), first);

        // Geometry: from the first column, right is the second; from there, right is the third.
        area.focus_direction(Direction::Right);
        assert_eq!(area.focus(), area.group_ids()[1]);
        area.focus_direction(Direction::Right);
        assert_eq!(area.focus(), area.group_ids()[2]);
        // Nothing is further right than the third column.
        assert!(!area.focus_direction(Direction::Right));
        // And left goes back.
        area.focus_direction(Direction::Left);
        assert_eq!(area.focus(), area.group_ids()[1]);
    }

    #[test]
    fn a_tab_migrates_between_groups_carrying_its_viewport() {
        let mut area = groups();
        area.open("pages/a.json");
        let first = area.focus();
        area.scroll(30).unwrap();
        area.select(1, 2).unwrap();

        area.split(Axis::LeftRight);
        let second = area.focus();
        area.move_tab(first, 0, second, 0).unwrap();

        assert!(area.tabs(first).unwrap().is_empty());
        let moved = &area.tabs(second).unwrap().tabs()[0];
        assert_eq!(moved.document(), "pages/a.json");
        assert_eq!(moved.viewport().scroll(), 30);
        assert_eq!(moved.viewport().selection(), Some((1, 2)));
        // The document record stays where it was: moving a view moves a view.
        assert_eq!(area.text("pages/a.json"), Some(""));
    }

    #[test]
    fn moving_a_tab_to_a_group_that_is_not_there_is_refused() {
        let mut area = groups();
        area.open("pages/a.json");
        let first = area.focus();
        area.split(Axis::LeftRight);
        let second = area.focus();
        area.open("pages/b.json");

        assert!(area.move_tab(first, 0, second, 0).is_ok());
        assert!(area.move_tab(first, 9, second, 0).is_err());
        // A move onto a nonexistent target names it rather than losing the tab.
        let unknown = super::GroupId(99);
        assert!(area.move_tab(second, 0, unknown, 0).is_err());
        // The refused move left the tab where it was.
        assert_eq!(area.tabs(second).unwrap().len(), 2);
    }

    #[test]
    fn closing_a_group_moves_its_views_to_a_neighbour_and_keeps_the_documents_open() {
        let mut area = groups();
        area.open("pages/a.json");
        let first = area.focus();
        area.split(Axis::LeftRight);
        area.focus();
        area.open("pages/b.json");

        let moved = area.close_focus_group().unwrap();
        assert_eq!(moved, 1);
        assert_eq!(area.len(), 1);
        assert_eq!(area.focus(), first);
        // The document that was only in the closed group is still open, in the neighbour.
        assert_eq!(area.tabs(first).unwrap().len(), 2);
        assert_eq!(area.text("pages/b.json"), Some(""));
    }

    #[test]
    fn the_last_group_cannot_be_closed_and_nothing_is_lost_when_it_is_refused() {
        let mut area = groups();
        area.open("pages/a.json");
        let before = area.clone();

        assert!(area.close_focus_group().is_err());
        assert_eq!(area.len(), 1);
        assert_eq!(area.tabs(area.focus()).unwrap().len(), 1);
        assert_eq!(area.render(), before.render());
    }

    #[test]
    fn typing_with_no_view_is_refused_rather_than_writing_somewhere_else() {
        let mut area = groups();
        let error = area.type_into("nowhere").unwrap_err();
        assert!(error.contains("no document"), "{error}");
        assert!(area.documents().is_empty());
    }

    #[test]
    fn rearranging_keeps_documents_the_arrangement_does_not_name() {
        let mut area = groups();
        area.open("pages/a.json");
        area.split(Axis::LeftRight);
        area.open("pages/b.json");
        area.type_into("keep me").unwrap();

        // Collapsing to a single pane (what reset does) must not drop either document.
        area.rearrange(&Shape::single(), &[vec![]]).unwrap();

        assert_eq!(area.len(), 1);
        let open: Vec<String> = area
            .tabs(area.focus())
            .unwrap()
            .tabs()
            .iter()
            .map(|tab| tab.document().to_owned())
            .collect();
        assert!(open.contains(&"pages/a.json".to_owned()), "{open:?}");
        assert!(open.contains(&"pages/b.json".to_owned()), "{open:?}");
        // And the text that was typed is still there: rearranging is not a way to lose an edit.
        assert_eq!(area.text("pages/b.json"), Some("keep me"));
    }

    #[test]
    fn a_shape_and_its_arrangement_rebuild_into_the_same_shape() {
        let mut area = groups();
        area.split(Axis::LeftRight);
        area.split(Axis::TopBottom);
        let shape = area.shape();
        assert_eq!(shape.leaf_count(), 3);
        let arrangement = area.arrangement();

        let mut rebuilt = Groups::new(4);
        rebuilt.rearrange(&shape, &arrangement).unwrap();
        assert_eq!(rebuilt.shape(), shape);
        assert_eq!(rebuilt.arrangement(), arrangement);
    }

    #[test]
    fn an_arrangement_that_disagrees_with_its_shape_is_refused_without_change() {
        let mut area = groups();
        area.open("pages/a.json");
        let before = area.render();

        let two = Shape::Split {
            axis: Axis::LeftRight,
            children: vec![Shape::Leaf(0), Shape::Leaf(1)],
        };
        // The shape says two panes; the arrangement names one.
        assert!(area.rearrange(&two, &[vec![]]).is_err());
        assert_eq!(area.render(), before);

        // A leaf past the arrangement is refused too.
        let stray = Shape::Split {
            axis: Axis::LeftRight,
            children: vec![Shape::Leaf(0), Shape::Leaf(9)],
        };
        assert!(area.rearrange(&stray, &[vec![], vec![]]).is_err());
        assert_eq!(area.render(), before);
    }

    #[test]
    fn every_axis_and_direction_has_a_name_and_a_report_that_agrees() {
        let mut area = groups();
        area.open("pages/a.json");
        for axis in Axis::ALL {
            assert_eq!(Axis::from_name(axis.name()), Some(axis));
        }
        assert!(Axis::from_name("diagonal").is_none());
        for direction in Direction::ALL {
            assert_eq!(Direction::from_name(direction.name()), Some(direction));
        }
        assert!(Direction::from_name("sideways").is_none());

        area.split(Axis::LeftRight);
        area.another_view("pages/a.json");
        let report = area.render().join("\n");
        assert!(report.contains("2 group(s)"), "{report}");
        assert!(report.contains("focus 1"), "{report}");
        assert!(report.contains("pages/a.json"), "{report}");
        assert!(report.contains("scroll 0"), "{report}");
    }
}
