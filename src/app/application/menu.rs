// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Menu hierarchy editing, platform roles and native behaviour (task T26.03).
//!
//! The acceptance line is *"refuse cycles / invalid roles; native vs non-native behaviour is
//! explicit"*. The design keeps the two refusals and the one statement apart:
//!
//! * **A role is a closed set and is parsed at the edge.** [`Role::parse`] answers `None` for an
//!   unknown name, so an "invalid role" cannot become a menu node in the first place; the caller
//!   turns that `None` into an explicit refusal rather than a placeholder.
//! * **A cycle is a named finding.** A submenu is an edge from one menu id to another, and
//!   [`MenuTree::cycles`] walks those edges, so a submenu that reaches back to itself (directly or
//!   through others) is reported as a path rather than silently written out. A submenu edge to a menu
//!   that was never defined is a separate finding, [`MenuTree::dangling`]: "referenced but missing"
//!   is not the same as "cyclic".
//! * **Native behaviour is per role and per platform.** [`Role::is_native_on`] says whether the target
//!   renders the role itself; a role the target does not render natively must be drawn by the
//!   application, which [`MenuTree::native_roles`] states on both sides.
//!
//! Ordering is insertion order of the node list, because a menu is a place: reordering it is moving
//! nodes, and there is no second ordering to keep in step.

#![allow(dead_code)]

use std::collections::BTreeMap;

use super::action::ActionId;

/// A platform menu role: an item whose behaviour the platform owns.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Role {
    /// The about item.
    About,
    /// The quit item.
    Quit,
    /// The preferences item.
    Preferences,
    /// The hide-application item.
    Hide,
    /// The hide-others item.
    HideOthers,
    /// The show-all item.
    ShowAll,
    /// The services item.
    Services,
    /// The minimize item.
    Minimize,
    /// The zoom item.
    Zoom,
    /// The window list.
    Window,
    /// The help item.
    Help,
}

impl Role {
    /// Every role, for a report that lists them all.
    pub const ALL: [Role; 11] = [
        Role::About,
        Role::Quit,
        Role::Preferences,
        Role::Hide,
        Role::HideOthers,
        Role::ShowAll,
        Role::Services,
        Role::Minimize,
        Role::Zoom,
        Role::Window,
        Role::Help,
    ];

    /// The role's stable name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::About => "about",
            Self::Quit => "quit",
            Self::Preferences => "preferences",
            Self::Hide => "hide",
            Self::HideOthers => "hide-others",
            Self::ShowAll => "show-all",
            Self::Services => "services",
            Self::Minimize => "minimize",
            Self::Zoom => "zoom",
            Self::Window => "window",
            Self::Help => "help",
        }
    }

    /// Reads a role's name, or `None` when it is not one of the known roles.
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|role| role.name() == text)
    }

    /// Whether `platform` renders this role itself.
    ///
    /// macOS renders every role; Windows has no role concept at all, so the application draws every
    /// item; a Linux desktop provides the application-level `Quit` and `About` items but not the
    /// window ones, which the application draws.
    pub fn is_native_on(&self, platform: Platform) -> bool {
        match platform {
            Platform::MacOs => true,
            Platform::Windows => false,
            Platform::Linux => matches!(self, Self::Quit | Self::About),
        }
    }
}

/// A platform the application is generated for.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Platform {
    /// macOS.
    MacOs,
    /// Windows.
    Windows,
    /// Linux.
    Linux,
}

impl Platform {
    /// The platform's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::MacOs => "macos",
            Self::Windows => "windows",
            Self::Linux => "linux",
        }
    }

    /// Reads a platform's name, or `None` when it is not one.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "macos" => Some(Self::MacOs),
            "windows" => Some(Self::Windows),
            "linux" => Some(Self::Linux),
            _ => None,
        }
    }
}

/// One node of a menu.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Node {
    /// An item that runs an action.
    Action {
        /// The label shown.
        label: String,
        /// The action it runs.
        action: ActionId,
    },
    /// A separator line.
    Separator,
    /// A submenu, naming the menu it opens.
    Submenu {
        /// The menu id it opens.
        id: String,
        /// The label shown in the parent.
        label: String,
    },
    /// A platform role.
    Role(Role),
}

impl Node {
    /// An item that runs an action.
    pub fn action(label: &str, action: ActionId) -> Self {
        Self::Action {
            label: label.to_owned(),
            action,
        }
    }

    /// A separator.
    pub fn separator() -> Self {
        Self::Separator
    }

    /// A submenu pointing at a menu id.
    pub fn submenu(id: &str, label: &str) -> Self {
        Self::Submenu {
            id: id.to_owned(),
            label: label.to_owned(),
        }
    }

    /// A platform role node.
    pub fn role(role: Role) -> Self {
        Self::Role(role)
    }

    /// The node in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Action { label, action } => format!("{label} -> {}", action.as_str()),
            Self::Separator => "-".to_owned(),
            Self::Submenu { id, label } => format!("{label} > {id}"),
            Self::Role(role) => format!("role:{}", role.name()),
        }
    }
}

/// A menu tree: top-level menus by id, and the nodes each menu holds.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct MenuTree {
    roots: Vec<(String, String)>,
    menus: BTreeMap<String, Vec<Node>>,
}

impl MenuTree {
    /// An empty tree.
    pub fn new() -> Self {
        Self::default()
    }

    /// Defines a top-level menu, refusing a blank id or title and an id that is already used.
    pub fn add_root(&mut self, id: &str, title: &str, items: Vec<Node>) -> Result<(), String> {
        let id = check_id(id)?;
        let title = title.trim();
        if title.is_empty() {
            return Err(format!("menu {id} needs a title"));
        }
        if self.menus.contains_key(&id) {
            return Err(format!("menu {id} is defined twice"));
        }
        self.roots.push((id.clone(), title.to_owned()));
        self.menus.insert(id, items);
        Ok(())
    }

    /// Defines (or replaces) the items of a menu id that a submenu points at.
    pub fn place(&mut self, id: &str, items: Vec<Node>) -> Result<(), String> {
        let id = check_id(id)?;
        self.menus.insert(id, items);
        Ok(())
    }

    /// Adds a submenu node to an existing menu, refusing an unknown parent or a blank child id.
    pub fn add_submenu(&mut self, parent: &str, child: &str, label: &str) -> Result<(), String> {
        if !self.menus.contains_key(parent) {
            return Err(format!("menu {parent} is not defined"));
        }
        let child = check_id(child)?;
        let label = label.trim();
        if label.is_empty() {
            return Err(format!("submenu {child} needs a label"));
        }
        if let Some(nodes) = self.menus.get_mut(parent) {
            nodes.push(Node::submenu(&child, label));
        }
        Ok(())
    }

    /// The nodes of a menu, or an empty slice when it is not defined.
    pub fn children(&self, id: &str) -> &[Node] {
        self.menus.get(id).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Every role that appears anywhere in the tree, in first-seen order.
    pub fn roles(&self) -> Vec<Role> {
        let mut roles = Vec::new();
        for nodes in self.menus.values() {
            for node in nodes {
                if let Node::Role(role) = node
                    && !roles.contains(role)
                {
                    roles.push(*role);
                }
            }
        }
        roles
    }

    /// A submenu edge to a menu that was never defined, as `(parent, child)`.
    pub fn dangling(&self) -> Vec<(String, String)> {
        let mut found = Vec::new();
        for (id, nodes) in &self.menus {
            for node in nodes {
                if let Node::Submenu { id: child, .. } = node
                    && !self.menus.contains_key(child)
                {
                    found.push((id.clone(), child.clone()));
                }
            }
        }
        found
    }

    /// Every cycle in the submenu graph, each as a path that returns to where it started.
    pub fn cycles(&self) -> Vec<Vec<String>> {
        let mut found = Vec::new();
        let mut colour: BTreeMap<String, u8> = BTreeMap::new();
        let mut stack: Vec<String> = Vec::new();
        for id in self.menus.keys() {
            if colour.get(id).copied().unwrap_or(0) == 0 {
                self.walk(id, &mut colour, &mut stack, &mut found);
            }
        }
        found
    }

    fn walk(
        &self,
        id: &str,
        colour: &mut BTreeMap<String, u8>,
        stack: &mut Vec<String>,
        found: &mut Vec<Vec<String>>,
    ) {
        colour.insert(id.to_owned(), 1);
        stack.push(id.to_owned());
        if let Some(nodes) = self.menus.get(id) {
            for node in nodes {
                if let Node::Submenu { id: child, .. } = node {
                    if !self.menus.contains_key(child) {
                        continue;
                    }
                    match colour.get(child).copied().unwrap_or(0) {
                        0 => self.walk(child, colour, stack, found),
                        1 => {
                            let start = stack.iter().position(|id| id == child).unwrap_or(0);
                            let mut cycle = stack[start..].to_vec();
                            cycle.push(child.clone());
                            found.push(cycle);
                        }
                        _ => {}
                    }
                }
            }
        }
        stack.pop();
        colour.insert(id.to_owned(), 2);
    }

    /// Every problem: a cycle or a dangling submenu edge.
    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        for (parent, child) in self.dangling() {
            problems.push(format!(
                "menu {parent} refers to submenu {child} which is not defined"
            ));
        }
        for cycle in self.cycles() {
            problems.push(format!("menu {} contains a cycle", cycle.join(" -> ")));
        }
        problems
    }

    /// The roles `platform` renders natively and the roles the application must draw itself.
    pub fn native_roles(&self, platform: Platform) -> (Vec<Role>, Vec<Role>) {
        let mut native = Vec::new();
        let mut drawn = Vec::new();
        for role in self.roles() {
            if role.is_native_on(platform) {
                native.push(role);
            } else {
                drawn.push(role);
            }
        }
        (native, drawn)
    }

    /// The tree in a report: one indented line per node, following submenu edges without looping.
    pub fn render(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for (id, title) in &self.roots {
            lines.push(title.clone());
            let mut seen = vec![id.clone()];
            self.render_menu(id, &mut seen, 1, &mut lines);
        }
        lines
    }

    fn render_menu(&self, id: &str, seen: &mut Vec<String>, depth: usize, lines: &mut Vec<String>) {
        for node in self.children(id) {
            lines.push(format!("{}{}", "  ".repeat(depth), node.render()));
            if let Node::Submenu { id: child, .. } = node
                && !seen.contains(child)
            {
                seen.push(child.clone());
                self.render_menu(child, seen, depth + 1, lines);
            }
        }
    }
}

/// Validates a menu id.
fn check_id(id: &str) -> Result<String, String> {
    let trimmed = id.trim();
    if trimmed.is_empty() || trimmed.split_whitespace().count() != 1 {
        return Err(format!("{id:?} is not a menu id"));
    }
    Ok(trimmed.to_owned())
}

#[cfg(test)]
mod tests {
    use super::{MenuTree, Node, Platform, Role};
    use crate::app::application::action::ActionId;

    fn id(name: &str) -> ActionId {
        ActionId::new(name).expect("an id")
    }

    fn tree_with_a_cycle() -> MenuTree {
        let mut tree = MenuTree::new();
        tree.add_root("file", "File", vec![Node::submenu("file.child", "More")])
            .expect("a root");
        tree.place("file.child", vec![Node::submenu("file", "Back")])
            .expect("a menu");
        tree
    }

    #[test]
    fn a_role_is_closed_and_an_unknown_name_is_refused_at_the_edge() {
        for role in Role::ALL {
            assert_eq!(Role::parse(role.name()), Some(role));
        }
        assert_eq!(Role::parse("teleport"), None);
    }

    #[test]
    fn a_submenu_that_contains_itself_is_a_cycle() {
        let mut tree = MenuTree::new();
        tree.add_root("a", "A", vec![Node::submenu("a", "A again")])
            .expect("a root");

        let cycles = tree.cycles();
        assert_eq!(cycles.len(), 1);
        assert_eq!(cycles[0], vec!["a".to_owned(), "a".to_owned()]);
        assert!(
            tree.problems()[0].contains("cycle"),
            "{:?}",
            tree.problems()
        );
    }

    #[test]
    fn two_menus_that_reach_back_to_each_other_are_refused() {
        let tree = tree_with_a_cycle();
        let cycles = tree.cycles();
        assert_eq!(cycles.len(), 1);
        assert!(cycles[0].contains(&"file".to_owned()));
        assert!(cycles[0].contains(&"file.child".to_owned()));
        assert!(!tree.problems().is_empty());
    }

    #[test]
    fn a_submenu_edge_to_a_menu_that_was_never_defined_is_missing_not_cyclic() {
        let mut tree = MenuTree::new();
        tree.add_root("file", "File", vec![Node::submenu("absent", "More")])
            .expect("a root");

        assert!(tree.cycles().is_empty());
        let dangling = tree.dangling();
        assert_eq!(dangling, vec![("file".to_owned(), "absent".to_owned())]);
        assert!(tree.problems()[0].contains("not defined"));
    }

    #[test]
    fn a_blank_or_duplicate_menu_id_is_refused() {
        let mut tree = MenuTree::new();
        assert!(tree.add_root("  ", "File", Vec::new()).is_err());
        tree.add_root("file", "File", Vec::new()).expect("a root");
        assert!(tree.add_root("file", "File again", Vec::new()).is_err());
        assert!(tree.add_submenu("absent", "child", "More").is_err());
    }

    #[test]
    fn a_submenu_node_is_added_to_its_parent() {
        let mut tree = MenuTree::new();
        tree.add_root("file", "File", Vec::new()).expect("a root");
        tree.add_submenu("file", "file.recent", "Recent")
            .expect("a submenu node");
        let children = tree.children("file");
        assert_eq!(children.len(), 1);
        assert!(children[0].render().contains("Recent > file.recent"));
        // The pointed-at menu is not defined yet, which the tree reports as missing.
        assert_eq!(
            tree.dangling(),
            vec![("file".to_owned(), "file.recent".to_owned())]
        );
    }

    #[test]
    fn native_behaviour_is_stated_per_role_and_per_platform() {
        let mut tree = MenuTree::new();
        tree.add_root(
            "app",
            "App",
            vec![
                Node::role(Role::About),
                Node::role(Role::Quit),
                Node::role(Role::Preferences),
            ],
        )
        .expect("a root");

        let (native, drawn) = tree.native_roles(Platform::MacOs);
        assert_eq!(native, vec![Role::About, Role::Quit, Role::Preferences]);
        assert!(drawn.is_empty());

        let (native, drawn) = tree.native_roles(Platform::Windows);
        assert!(native.is_empty());
        assert_eq!(drawn, vec![Role::About, Role::Quit, Role::Preferences]);

        // A Linux desktop owns the application-level items, so the application draws only the rest.
        let (native, drawn) = tree.native_roles(Platform::Linux);
        assert_eq!(native, vec![Role::About, Role::Quit]);
        assert_eq!(drawn, vec![Role::Preferences]);
    }

    #[test]
    fn a_platform_name_is_one_of_three() {
        assert_eq!(Platform::parse("macos"), Some(Platform::MacOs));
        assert_eq!(Platform::parse("windows"), Some(Platform::Windows));
        assert_eq!(Platform::parse("linux"), Some(Platform::Linux));
        assert_eq!(Platform::parse("beos"), None);
    }

    #[test]
    fn the_tree_renders_nested_menus_without_looping() {
        let tree = tree_with_a_cycle();
        let report = tree.render().join("\n");
        assert!(report.contains("File"), "{report}");
        assert!(report.contains("More > file.child"), "{report}");
        // The back edge is shown once as a label and not followed again.
        assert_eq!(report.matches("Back > file").count(), 1, "{report}");
    }

    #[test]
    fn an_action_node_keeps_the_shared_action_id() {
        let mut tree = MenuTree::new();
        tree.add_root("file", "File", vec![Node::action("Save", id("file.save"))])
            .expect("a root");
        let render = tree.children("file")[0].render();
        assert_eq!(render, "Save -> file.save");
    }
}
