// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Context menus, scope parameters and dynamic enablement (task T26.05).
//!
//! The acceptance line is *"a right-click triggers the correct context type; a disabled action does
//! not execute"*. The two halves are separate on purpose:
//!
//! * **A right-click resolves to a registered context type, or is refused.**
//!   [`Registry::request`] looks the [`ContextKind`] up and refuses an unregistered one, so the
//!   canvas menu can never be shown for a tree node: the type of the thing clicked *is* which menu
//!   is offered, and an unknown type is a distinct refusal rather than an empty menu.
//! * **A disabled action cannot run its handler.** [`Request::invoke`] decides before it calls
//!   anything: a disabled or out-of-context action answers [`Invocation::Skipped`] and the handler is
//!   never entered. The test proves this with a call counter that must stay at zero, which is the
//!   only way "did not execute" is observable rather than asserted.
//!
//! The scope parameters travel with the request, so a handler that does run receives the exact thing
//! that was clicked, not a guess reconstructed later.

#![allow(dead_code)]

use std::collections::BTreeMap;

use super::action::ActionId;

/// The type of thing a right-click landed on.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum ContextKind {
    /// A list or collection item.
    Item,
    /// The design canvas.
    Canvas,
    /// A tree node.
    TreeNode,
}

impl ContextKind {
    /// Every kind, for a report.
    pub const ALL: [ContextKind; 3] = [
        ContextKind::Item,
        ContextKind::Canvas,
        ContextKind::TreeNode,
    ];

    /// The kind's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Item => "item",
            Self::Canvas => "canvas",
            Self::TreeNode => "tree-node",
        }
    }

    /// Reads a kind's name, or `None` when it is not one.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "item" => Some(Self::Item),
            "canvas" => Some(Self::Canvas),
            "tree-node" => Some(Self::TreeNode),
            _ => None,
        }
    }
}

/// The parameters passed to a context handler: the scope of the thing that was clicked.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ScopeParams {
    /// The parameters, by name.
    pub pairs: BTreeMap<String, String>,
}

impl ScopeParams {
    /// No parameters.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets a parameter, replacing one with the same name.
    pub fn set(&mut self, name: &str, value: &str) {
        self.pairs.insert(name.trim().to_owned(), value.to_owned());
    }

    /// One parameter, if it is set.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.pairs.get(name).map(String::as_str)
    }

    /// The parameters in one line.
    pub fn render(&self) -> String {
        self.pairs
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// A context menu for one kind of thing.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ContextMenu {
    /// What it is shown for.
    pub kind: ContextKind,
    /// The actions it offers, in order.
    pub items: Vec<ActionId>,
}

impl ContextMenu {
    /// A context menu for a kind.
    pub fn new(kind: ContextKind) -> Self {
        Self {
            kind,
            items: Vec::new(),
        }
    }

    /// Adds an action to the menu.
    pub fn add(&mut self, action: ActionId) {
        self.items.push(action);
    }

    /// Whether the menu offers an action.
    pub fn contains(&self, action: &ActionId) -> bool {
        self.items.contains(action)
    }
}

/// What one attempted action did.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Invocation {
    /// The action ran, with the scope parameters it received.
    Fired {
        /// The action.
        action: ActionId,
        /// The parameters passed to the handler.
        params: ScopeParams,
    },
    /// The action did not run, and why.
    Skipped {
        /// The action.
        action: ActionId,
        /// Why it did not run.
        reason: String,
    },
}

impl Invocation {
    /// Whether the handler ran.
    pub fn fired(&self) -> bool {
        matches!(self, Self::Fired { .. })
    }

    /// The invocation in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Fired { action, params } => {
                let rendered = params.render();
                if rendered.is_empty() {
                    format!("fired {}", action.as_str())
                } else {
                    format!("fired {} ({rendered})", action.as_str())
                }
            }
            Self::Skipped { action, reason } => {
                format!("skipped {}: {reason}", action.as_str())
            }
        }
    }
}

/// A right-click resolved to a menu and a snapshot of what is enabled.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Request {
    /// The type that was clicked.
    pub kind: ContextKind,
    /// The scope parameters of the click.
    pub params: ScopeParams,
    /// The actions the menu offers, in order.
    pub items: Vec<ActionId>,
    /// Whether each offered action was enabled when the menu opened.
    enabled: BTreeMap<ActionId, bool>,
}

impl Request {
    /// Runs one action if it is in the context and enabled; otherwise skips it without calling the
    /// handler.
    pub fn invoke(
        &self,
        action: &ActionId,
        mut handler: impl FnMut(&ActionId, &ScopeParams),
    ) -> Invocation {
        if !self.items.contains(action) {
            return Invocation::Skipped {
                action: action.clone(),
                reason: "not in this context".to_owned(),
            };
        }
        if !self.enabled.get(action).copied().unwrap_or(true) {
            return Invocation::Skipped {
                action: action.clone(),
                reason: "disabled".to_owned(),
            };
        }
        handler(action, &self.params);
        Invocation::Fired {
            action: action.clone(),
            params: self.params.clone(),
        }
    }

    /// Whether an offered action is enabled in this request.
    pub fn is_enabled(&self, action: &ActionId) -> bool {
        self.enabled.get(action).copied().unwrap_or(true)
    }
}

/// The context menus this application offers, and the dynamic enablement of their actions.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Registry {
    menus: BTreeMap<ContextKind, ContextMenu>,
    enabled: BTreeMap<ActionId, bool>,
}

impl Registry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a menu for its kind, replacing one already registered for that kind.
    pub fn register(&mut self, menu: ContextMenu) {
        self.menus.insert(menu.kind, menu);
    }

    /// Adds an action to the menu for a kind, creating the menu if it does not exist yet.
    pub fn add_item(&mut self, kind: ContextKind, action: ActionId) {
        self.menus
            .entry(kind)
            .or_insert_with(|| ContextMenu::new(kind))
            .add(action);
    }

    /// Sets whether an action is enabled wherever it appears.
    pub fn set_enabled(&mut self, action: ActionId, enabled: bool) {
        self.enabled.insert(action, enabled);
    }

    /// The menu registered for a kind, if any.
    pub fn menu(&self, kind: ContextKind) -> Option<&ContextMenu> {
        self.menus.get(&kind)
    }

    /// Resolves a right-click on a kind into a request, refusing an unregistered kind.
    pub fn request(&self, kind: ContextKind, params: ScopeParams) -> Result<Request, String> {
        let Some(menu) = self.menus.get(&kind) else {
            return Err(format!("no context menu is registered for {}", kind.name()));
        };
        let enabled = menu
            .items
            .iter()
            .map(|action| {
                (
                    action.clone(),
                    self.enabled.get(action).copied().unwrap_or(true),
                )
            })
            .collect();
        Ok(Request {
            kind,
            params,
            items: menu.items.clone(),
            enabled,
        })
    }

    /// Resolves a right-click named as text, refusing a name that is not a context kind.
    pub fn request_named(&self, kind: &str, params: ScopeParams) -> Result<Request, String> {
        let kind =
            ContextKind::parse(kind).ok_or_else(|| format!("{kind:?} is not a context kind"))?;
        self.request(kind, params)
    }
}

#[cfg(test)]
mod tests {
    use super::{ContextKind, ContextMenu, Invocation, Registry, ScopeParams};
    use crate::app::application::action::ActionId;
    use std::cell::Cell;

    fn id(name: &str) -> ActionId {
        ActionId::new(name).expect("an id")
    }

    fn registry() -> Registry {
        let mut registry = Registry::new();
        let mut tree = ContextMenu::new(ContextKind::TreeNode);
        tree.add(id("node.rename"));
        tree.add(id("node.delete"));
        registry.register(tree);
        let mut canvas = ContextMenu::new(ContextKind::Canvas);
        canvas.add(id("canvas.paste"));
        registry.register(canvas);
        registry
    }

    #[test]
    fn a_context_kind_is_one_of_three_and_an_unknown_name_is_refused() {
        for kind in ContextKind::ALL {
            assert_eq!(ContextKind::parse(kind.name()), Some(kind));
        }
        assert_eq!(ContextKind::parse("tree-node"), Some(ContextKind::TreeNode));
        assert_eq!(ContextKind::parse("pixel"), None);
        let registry = registry();
        let refusal = registry
            .request_named("pixel", ScopeParams::new())
            .expect_err("a refusal");
        assert!(refusal.contains("not a context kind"), "{refusal}");
    }

    #[test]
    fn adding_an_item_creates_the_menu_for_a_kind() {
        let mut registry = Registry::new();
        assert!(registry.menu(ContextKind::Item).is_none());
        registry.add_item(ContextKind::Item, id("item.remove"));
        let menu = registry.menu(ContextKind::Item).expect("a menu");
        assert!(menu.contains(&id("item.remove")));
    }

    #[test]
    fn a_right_click_triggers_the_menu_of_the_type_that_was_clicked() {
        let registry = registry();
        let mut params = ScopeParams::new();
        params.set("node", "7");

        let request = registry
            .request(ContextKind::TreeNode, params.clone())
            .expect("a request");
        assert_eq!(request.kind, ContextKind::TreeNode);
        assert!(request.items.contains(&id("node.rename")));
        assert!(!request.items.contains(&id("canvas.paste")));

        let request = registry
            .request(ContextKind::Canvas, params)
            .expect("a request");
        assert_eq!(request.kind, ContextKind::Canvas);
        assert!(request.items.contains(&id("canvas.paste")));
        assert!(!request.items.contains(&id("node.rename")));
    }

    #[test]
    fn an_unregistered_context_kind_is_refused_not_shown_empty() {
        let registry = registry();
        let refusal = registry
            .request(ContextKind::Item, ScopeParams::new())
            .expect_err("a refusal");
        assert!(refusal.contains("no context menu"), "{refusal}");
        assert!(refusal.contains("item"), "{refusal}");
    }

    #[test]
    fn an_enabled_action_fires_and_receives_the_scope_parameters() {
        let registry = registry();
        let mut params = ScopeParams::new();
        params.set("node", "7");
        let request = registry
            .request(ContextKind::TreeNode, params)
            .expect("a request");

        assert_eq!(request.params.get("node"), Some("7"));
        let calls = Cell::new(0);
        let seen = std::cell::RefCell::new(String::new());
        let invocation = request.invoke(&id("node.rename"), |action, params| {
            calls.set(calls.get() + 1);
            *seen.borrow_mut() = format!("{} {}", action.as_str(), params.render());
        });
        assert!(invocation.fired());
        assert!(matches!(invocation, Invocation::Fired { .. }));
        assert_eq!(calls.get(), 1);
        assert_eq!(&*seen.borrow(), "node.rename node=7");
    }

    #[test]
    fn a_disabled_action_does_not_execute() {
        let mut registry = registry();
        registry.set_enabled(id("node.delete"), false);
        let request = registry
            .request(ContextKind::TreeNode, ScopeParams::new())
            .expect("a request");
        assert!(!request.is_enabled(&id("node.delete")));

        let calls = Cell::new(0);
        let invocation = request.invoke(&id("node.delete"), |_, _| {
            calls.set(calls.get() + 1);
        });
        assert_eq!(calls.get(), 0, "a disabled action must not run its handler");
        assert_eq!(
            invocation,
            Invocation::Skipped {
                action: id("node.delete"),
                reason: "disabled".to_owned(),
            }
        );
        assert!(invocation.render().contains("disabled"));
    }

    #[test]
    fn an_action_that_is_not_in_the_context_does_not_execute() {
        let registry = registry();
        let request = registry
            .request(ContextKind::Canvas, ScopeParams::new())
            .expect("a request");
        let calls = Cell::new(0);
        let invocation = request.invoke(&id("node.delete"), |_, _| {
            calls.set(calls.get() + 1);
        });
        assert_eq!(calls.get(), 0);
        assert!(invocation.render().contains("not in this context"));
    }
}
