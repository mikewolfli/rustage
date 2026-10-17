// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The target application's tool bar and status bar (task T26.04).
//!
//! The acceptance line is *"do not fake the target app's toolbar with the designer's own toolbar"*.
//! This module is therefore a distinct schema from the designer's icon strip in
//! `crate::app::shell::toolbar`, and nothing here reads or writes that strip:
//!
//! * **It is the target application's model.** An item references a shared [`ActionId`] and says how
//!   that action is *presented* — icon, text or both — where the designer's strip only knows its own
//!   fixed set of icons. A test asserts this model has its own identity rather than the designer's.
//! * **Overflow is resolved explicitly.** [`ToolBar::resolve`] turns the requested items and a
//!   capacity into what is shown and what folds into the overflow menu, so "everything fits" is a
//!   computed result, not an assumption. [`Overflow::Wrap`] states that the host wraps instead, and
//!   then nothing overflows.
//! * **Visibility and order are the item list's order.** A hidden item is not shown and does not
//!   consume a slot; there is no second ordering to keep consistent.

#![allow(dead_code)]

use super::action::ActionId;

/// How an action appears in the tool bar.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Presentation {
    /// The icon only.
    Icon,
    /// The text only.
    Text,
    /// The icon and the text.
    Both,
}

impl Presentation {
    /// The presentation's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Icon => "icon",
            Self::Text => "text",
            Self::Both => "both",
        }
    }

    /// Reads a presentation's name, or `None` when it is not one.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "icon" => Some(Self::Icon),
            "text" => Some(Self::Text),
            "both" => Some(Self::Both),
            _ => None,
        }
    }
}

/// What happens when the items do not fit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Overflow {
    /// Items beyond the capacity fold into an overflow menu.
    Collapse,
    /// Every item stays on the bar and the host wraps.
    Wrap,
}

impl Overflow {
    /// The overflow's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Collapse => "collapse",
            Self::Wrap => "wrap",
        }
    }

    /// Reads an overflow name, or `None` when it is not one.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "collapse" => Some(Self::Collapse),
            "wrap" => Some(Self::Wrap),
            _ => None,
        }
    }
}

/// One item on the tool bar.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Item {
    /// The shared action it presents.
    pub action: ActionId,
    /// The label shown when the presentation includes text.
    pub label: String,
    /// How it is presented.
    pub presentation: Presentation,
    /// Whether it is visible; a hidden item is not shown and takes no slot.
    pub visible: bool,
}

impl Item {
    /// An item, refusing a blank label. It is visible and presents both.
    pub fn new(action: ActionId, label: &str) -> Result<Self, String> {
        let trimmed = label.trim();
        if trimmed.is_empty() {
            return Err(format!("tool bar item {} needs a label", action.as_str()));
        }
        Ok(Self {
            action,
            label: trimmed.to_owned(),
            presentation: Presentation::Both,
            visible: true,
        })
    }
}

/// The target application's tool bar.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ToolBar {
    /// The items, in order.
    pub items: Vec<Item>,
    /// What happens when they do not fit.
    pub overflow: Overflow,
}

/// What a tool bar actually shows at a given capacity.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Layout {
    /// The actions on the bar, in order.
    pub shown: Vec<ActionId>,
    /// The actions in the overflow menu, in order.
    pub overflowed: Vec<ActionId>,
    /// Whether an overflow control is shown.
    pub chevron: bool,
}

impl ToolBar {
    /// A tool bar with the given overflow behaviour and no items.
    pub fn new(overflow: Overflow) -> Self {
        Self {
            items: Vec::new(),
            overflow,
        }
    }

    /// Adds an item.
    pub fn place(&mut self, item: Item) {
        self.items.push(item);
    }

    /// The actions of the visible items, in order.
    pub fn visible_actions(&self) -> Vec<ActionId> {
        self.items
            .iter()
            .filter(|item| item.visible)
            .map(|item| item.action.clone())
            .collect()
    }

    /// Resolves the bar against a capacity: what is shown, what overflows, and whether a chevron is
    /// needed. A `Wrap` bar never overflows; a `Collapse` bar shows the first `capacity` items.
    pub fn resolve(&self, capacity: usize) -> Layout {
        let visible = self.visible_actions();
        match self.overflow {
            Overflow::Wrap => Layout {
                shown: visible,
                overflowed: Vec::new(),
                chevron: false,
            },
            Overflow::Collapse => {
                let mut shown = Vec::new();
                let mut overflowed = Vec::new();
                for (index, action) in visible.into_iter().enumerate() {
                    if index < capacity {
                        shown.push(action);
                    } else {
                        overflowed.push(action);
                    }
                }
                Layout {
                    chevron: !overflowed.is_empty(),
                    shown,
                    overflowed,
                }
            }
        }
    }

    /// The tool bar in a report: one line per item, and the overflow behaviour.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!("tool bar overflow: {}", self.overflow.name())];
        for item in &self.items {
            lines.push(format!(
                "{} {:?} [{}] {}",
                item.action.as_str(),
                item.label,
                item.presentation.name(),
                if item.visible { "visible" } else { "hidden" },
            ));
        }
        lines
    }
}

/// One field of the status bar.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StatusField {
    /// The field's name.
    pub label: String,
    /// The field's value.
    pub value: String,
}

/// The target application's status bar.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct StatusBar {
    /// The fields, in order.
    pub fields: Vec<StatusField>,
}

impl StatusBar {
    /// An empty status bar.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets a field's value, replacing one with the same label in place or appending it.
    pub fn set(&mut self, label: &str, value: &str) {
        let label = label.trim();
        match self.fields.iter_mut().find(|field| field.label == label) {
            Some(field) => field.value = value.to_owned(),
            None => self.fields.push(StatusField {
                label: label.to_owned(),
                value: value.to_owned(),
            }),
        }
    }

    /// The status bar in a report.
    pub fn render(&self) -> Vec<String> {
        self.fields
            .iter()
            .map(|field| format!("{}: {}", field.label, field.value))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{Item, Overflow, Presentation, StatusBar, ToolBar};
    use crate::app::application::action::ActionId;

    fn id(name: &str) -> ActionId {
        ActionId::new(name).expect("an id")
    }

    fn bar() -> ToolBar {
        let mut bar = ToolBar::new(Overflow::Collapse);
        bar.place(Item::new(id("file.new"), "New").expect("an item"));
        bar.place(Item::new(id("file.open"), "Open").expect("an item"));
        bar.place(Item::new(id("file.save"), "Save").expect("an item"));
        bar
    }

    #[test]
    fn this_is_the_target_toolbar_schema_and_not_the_designers_strip() {
        // The designer's strip is `crate::app::shell::toolbar`; here an item carries a shared action
        // id and a presentation, which the designer's model has no notion of.
        let item = Item::new(id("file.save"), "Save").expect("an item");
        assert_eq!(item.action.as_str(), "file.save");
        assert_eq!(item.presentation, Presentation::Both);
        assert!(item.visible);
    }

    #[test]
    fn a_presentation_and_an_overflow_are_closed_names() {
        for presentation in [Presentation::Icon, Presentation::Text, Presentation::Both] {
            assert_eq!(Presentation::parse(presentation.name()), Some(presentation));
        }
        assert_eq!(Presentation::parse("glyph"), None);
        for overflow in [Overflow::Collapse, Overflow::Wrap] {
            assert_eq!(Overflow::parse(overflow.name()), Some(overflow));
        }
        assert_eq!(Overflow::parse("scroll"), None);
    }

    #[test]
    fn a_hidden_item_is_not_shown_and_takes_no_slot() {
        let mut bar = bar();
        bar.items[1].visible = false;
        assert_eq!(bar.visible_actions(), vec![id("file.new"), id("file.save")],);
        let layout = bar.resolve(1);
        assert_eq!(layout.shown, vec![id("file.new")]);
        assert_eq!(layout.overflowed, vec![id("file.save")]);
        assert!(layout.chevron);
    }

    #[test]
    fn collapsing_moves_the_items_beyond_the_capacity_into_the_overflow() {
        let layout = bar().resolve(2);
        assert_eq!(layout.shown, vec![id("file.new"), id("file.open")]);
        assert_eq!(layout.overflowed, vec![id("file.save")]);
        assert!(layout.chevron);

        let layout = bar().resolve(3);
        assert_eq!(layout.shown.len(), 3);
        assert!(layout.overflowed.is_empty());
        assert!(!layout.chevron);
    }

    #[test]
    fn a_wrapping_bar_never_overflows() {
        let mut bar = bar();
        bar.overflow = Overflow::Wrap;
        let layout = bar.resolve(1);
        assert_eq!(layout.shown.len(), 3);
        assert!(layout.overflowed.is_empty());
        assert!(!layout.chevron);
    }

    #[test]
    fn a_capacity_of_zero_overflows_everything() {
        let layout = bar().resolve(0);
        assert!(layout.shown.is_empty());
        assert_eq!(layout.overflowed.len(), 3);
        assert!(layout.chevron);
    }

    #[test]
    fn the_tool_bar_renders_its_overflow_behaviour_and_items() {
        let report = bar().render().join("\n");
        assert!(report.contains("overflow: collapse"), "{report}");
        assert!(report.contains("file.save"), "{report}");
        assert!(report.contains("visible"), "{report}");
    }

    #[test]
    fn a_tool_bar_item_needs_a_label() {
        assert!(Item::new(id("file.new"), "  ").is_err());
    }

    #[test]
    fn the_status_bar_replaces_a_field_set_again() {
        let mut bar = StatusBar::new();
        bar.set("line", "3");
        bar.set("column", "12");
        bar.set("line", "4");
        assert_eq!(bar.render(), vec!["line: 4", "column: 12"]);
    }
}
