// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Page routes, the navigation graph, the back stack and the window lifecycle (task T13.05).
//!
//! The acceptance line is *"a cycle, a missing entry and a closing state are all diagnosable"*. Each
//! is a fact about navigation that has to be *sayable*, so each has an answer:
//!
//! * **A cycle is a fact, not a failure.** [`Navigator::cycles`] reports the loops the graph holds —
//!   navigating back and forth is legitimate — so a cycle is visible rather than silently present.
//! * **A missing entry is a missing entry.** [`Navigator::missing_entry`] answers whether nothing was
//!   declared as an entry, and [`Navigator::unreachable`] names the pages no entry can reach, which
//!   are different problems with different fixes.
//! * **A closing window is a state, and it refuses work.** [`Lifecycle`] separates *closing* from
//!   *closed*; [`Window::navigate`] refuses while closing and says which state it is in, so a
//!   navigation is never quietly dropped.
//! * **An undeclared page cannot be reached.** [`Window::navigate`] refuses a page the graph does not
//!   declare and an edge that does not exist, so the graph is what actually governs navigation.

#![allow(dead_code)]

use std::collections::BTreeMap;

/// A destination: a page and the parameters it carries.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Route {
    /// The page.
    pub page: String,
    /// The parameters.
    pub params: BTreeMap<String, String>,
}

impl Route {
    /// A route to a page with no parameters.
    pub fn new(page: impl Into<String>) -> Self {
        Self {
            page: page.into(),
            params: BTreeMap::new(),
        }
    }

    /// The same route with one more parameter.
    pub fn with(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.params.insert(key.into(), value.into());
        self
    }

    /// The route in one line.
    pub fn render(&self) -> String {
        if self.params.is_empty() {
            self.page.clone()
        } else {
            let pairs: Vec<String> = self
                .params
                .iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect();
            format!("{}?{}", self.page, pairs.join("&"))
        }
    }
}

/// The declared pages, the entries and the navigations between pages.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Navigator {
    /// The declared pages, in declaration order.
    pub pages: Vec<String>,
    /// The entry pages the project declares.
    pub entries: Vec<String>,
    /// The navigations, as `(from, to)`.
    pub edges: Vec<(String, String)>,
}

impl Navigator {
    /// A navigator with nothing declared.
    pub fn new() -> Self {
        Self::default()
    }

    /// The same navigator with one more page.
    pub fn with_page(mut self, page: impl Into<String>) -> Self {
        self.pages.push(page.into());
        self
    }

    /// The same navigator with one more entry.
    pub fn with_entry(mut self, page: impl Into<String>) -> Self {
        self.entries.push(page.into());
        self
    }

    /// The same navigator with one more navigation.
    pub fn with_edge(mut self, from: impl Into<String>, to: impl Into<String>) -> Self {
        self.edges.push((from.into(), to.into()));
        self
    }

    /// Whether a page is declared.
    pub fn has_page(&self, page: &str) -> bool {
        self.pages.iter().any(|declared| declared == page)
    }

    /// Whether there is a declared navigation from one page to another.
    pub fn can_navigate(&self, from: &str, to: &str) -> bool {
        self.edges
            .iter()
            .any(|(start, end)| start == from && end == to)
    }

    /// Whether nothing was declared as an entry.
    pub fn missing_entry(&self) -> bool {
        self.entries.is_empty()
    }

    /// The pages no entry can reach, in declaration order.
    pub fn unreachable(&self) -> Vec<String> {
        if self.entries.is_empty() {
            return self.pages.clone();
        }
        let mut reached: Vec<String> = self.entries.clone();
        let mut index = 0;
        while index < reached.len() {
            let page = reached[index].clone();
            index += 1;
            for (from, to) in &self.edges {
                if from == &page && !reached.contains(to) {
                    reached.push(to.clone());
                }
            }
        }
        self.pages
            .iter()
            .filter(|page| !reached.contains(page))
            .cloned()
            .collect()
    }

    /// The navigation loops the graph holds, as paths, reported rather than refused.
    ///
    /// A loop is legitimate — a person navigates back and forth — so this is a fact to show. Only
    /// one representative path per loop is reported, in page declaration order.
    pub fn cycles(&self) -> Vec<Vec<String>> {
        let mut found: Vec<Vec<String>> = Vec::new();
        let mut done: Vec<String> = Vec::new();
        for page in &self.pages {
            walk(page, self, &mut Vec::new(), &mut done, &mut found);
        }
        found
    }

    /// The pages that navigate to themselves.
    pub fn self_loops(&self) -> Vec<String> {
        self.edges
            .iter()
            .filter(|(from, to)| from == to)
            .map(|(from, _)| from.clone())
            .collect()
    }

    fn targets(&self, page: &str) -> Vec<String> {
        self.edges
            .iter()
            .filter(|(from, _)| from == page)
            .map(|(_, to)| to.clone())
            .collect()
    }

    /// The graph in one line per fact, then its diagnostics.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "pages: {}; entries: {}; navigations: {}",
            self.pages.len(),
            self.entries.len(),
            self.edges.len()
        )];
        for (from, to) in &self.edges {
            lines.push(format!("  {from} -> {to}"));
        }
        if self.missing_entry() {
            lines.push("no entry is declared".to_owned());
        }
        let unreachable = self.unreachable();
        if !unreachable.is_empty() && !self.missing_entry() {
            lines.push(format!("unreachable: {}", unreachable.join(", ")));
        }
        for loop_path in self.cycles() {
            lines.push(format!("cycle: {}", loop_path.join(" -> ")));
        }
        for page in self.self_loops() {
            lines.push(format!("self-loop: {page} -> {page}"));
        }
        lines
    }
}

/// Depth-first search that records one path per loop.
fn walk(
    page: &str,
    navigator: &Navigator,
    visiting: &mut Vec<String>,
    done: &mut Vec<String>,
    found: &mut Vec<Vec<String>>,
) {
    if let Some(start) = visiting.iter().position(|seen| seen == page) {
        let mut cycle = visiting[start..].to_vec();
        cycle.push(page.to_owned());
        if !found.iter().any(|existing| same_cycle(existing, &cycle)) {
            found.push(cycle);
        }
        return;
    }
    if done.iter().any(|seen| seen == page) {
        return;
    }
    visiting.push(page.to_owned());
    for target in navigator.targets(page) {
        walk(&target, navigator, visiting, done, found);
    }
    visiting.pop();
    done.push(page.to_owned());
}

/// Whether two cycle paths describe the same loop, whichever node they start at.
fn same_cycle(a: &[String], b: &[String]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let rotations = a.len();
    (0..rotations).any(|offset| (0..a.len()).all(|index| a[(index + offset) % a.len()] == b[index]))
}

/// Where a window is in its life.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lifecycle {
    /// Started, not yet ready to navigate.
    Opening,
    /// Ready.
    Open,
    /// Closing: still holding resources, no longer taking navigation.
    Closing,
    /// Done.
    Closed,
}

impl Lifecycle {
    /// The state's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Opening => "opening",
            Self::Open => "open",
            Self::Closing => "closing",
            Self::Closed => "closed",
        }
    }

    /// Whether the window will take a navigation.
    pub fn accepts_navigation(&self) -> bool {
        matches!(self, Self::Open)
    }
}

/// One window: its lifecycle and its back/forward stacks.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Window {
    /// Where it is in its life.
    pub lifecycle: Lifecycle,
    /// The page it is showing, once it has one.
    pub current: Option<Route>,
    back: Vec<Route>,
    forward: Vec<Route>,
}

impl Window {
    /// A window that has just started on a page.
    pub fn open(page: impl Into<String>) -> Self {
        Self {
            lifecycle: Lifecycle::Opening,
            current: Some(Route::new(page)),
            back: Vec::new(),
            forward: Vec::new(),
        }
    }

    /// Moves an opening window to open.
    pub fn ready(&mut self) {
        if self.lifecycle == Lifecycle::Opening {
            self.lifecycle = Lifecycle::Open;
        }
    }

    /// The page it is showing.
    pub fn current(&self) -> Option<&Route> {
        self.current.as_ref()
    }

    /// The pages behind the current one, most recent last.
    pub fn back_stack(&self) -> &[Route] {
        &self.back
    }

    /// The pages ahead of the current one, next first.
    pub fn forward_stack(&self) -> &[Route] {
        &self.forward
    }

    /// Navigates to a declared page along a declared navigation.
    ///
    /// Refused while the window is not open, for a page the graph does not declare, and for a
    /// navigation the graph does not have — so the graph governs where a window can go.
    pub fn navigate(&mut self, navigator: &Navigator, route: Route) -> Result<(), String> {
        if !self.lifecycle.accepts_navigation() {
            return Err(format!(
                "the window is {}, so it takes no navigation",
                self.lifecycle.name()
            ));
        }
        if !navigator.has_page(&route.page) {
            return Err(format!("no page {:?} is declared", route.page));
        }
        if let Some(current) = &self.current
            && !navigator.can_navigate(&current.page, &route.page)
        {
            return Err(format!(
                "there is no navigation from {:?} to {:?}",
                current.page, route.page
            ));
        }
        if let Some(current) = self.current.replace(route) {
            self.back.push(current);
        }
        self.forward.clear();
        Ok(())
    }

    /// Goes back one page, if there is one.
    pub fn back(&mut self) -> Option<Route> {
        let previous = self.back.pop()?;
        if let Some(current) = self.current.replace(previous) {
            self.forward.push(current);
        }
        self.current.clone()
    }

    /// Goes forward one page, if there is one.
    pub fn forward(&mut self) -> Option<Route> {
        if self.forward.is_empty() {
            return None;
        }
        let next = self.forward.remove(0);
        if let Some(current) = self.current.replace(next) {
            self.back.push(current);
        }
        self.current.clone()
    }

    /// Starts closing the window.
    pub fn close(&mut self) -> Result<(), String> {
        match self.lifecycle {
            Lifecycle::Open => {
                self.lifecycle = Lifecycle::Closing;
                Ok(())
            }
            Lifecycle::Closing => Err("the window is already closing".to_owned()),
            Lifecycle::Closed => Err("the window is already closed".to_owned()),
            Lifecycle::Opening => Err("the window is still opening".to_owned()),
        }
    }

    /// Finishes closing a window that is closing.
    pub fn finish_close(&mut self) -> Result<(), String> {
        if self.lifecycle == Lifecycle::Closing {
            self.lifecycle = Lifecycle::Closed;
            Ok(())
        } else {
            Err(format!(
                "the window is {}, not closing",
                self.lifecycle.name()
            ))
        }
    }

    /// The window in one line per fact.
    pub fn render(&self) -> Vec<String> {
        vec![
            format!("lifecycle: {}", self.lifecycle.name()),
            format!(
                "current: {}",
                self.current
                    .as_ref()
                    .map(Route::render)
                    .unwrap_or_else(|| "(none)".to_owned())
            ),
            format!("back: {}", self.back.len()),
            format!("forward: {}", self.forward.len()),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::{Lifecycle, Navigator, Route, Window};

    fn navigator() -> Navigator {
        Navigator::new()
            .with_page("home")
            .with_page("detail")
            .with_page("settings")
            .with_page("orphan")
            .with_entry("home")
            .with_edge("home", "detail")
            .with_edge("detail", "home")
            .with_edge("home", "settings")
    }

    fn window() -> Window {
        let mut window = Window::open("home");
        window.ready();
        window
    }

    #[test]
    fn a_route_carries_its_page_and_parameters() {
        let route = Route::new("detail").with("id", "7").with("tab", "info");
        assert_eq!(route.render(), "detail?id=7&tab=info");
        assert_eq!(Route::new("home").render(), "home");
    }

    #[test]
    fn a_navigator_with_no_entry_says_so() {
        let navigator = Navigator::new().with_page("home");
        assert!(navigator.missing_entry());
        assert!(
            navigator
                .render()
                .iter()
                .any(|line| line == "no entry is declared")
        );
    }

    #[test]
    fn a_page_no_entry_can_reach_is_reported() {
        assert_eq!(navigator().unreachable(), vec!["orphan"]);
        assert!(
            navigator()
                .render()
                .iter()
                .any(|line| line == "unreachable: orphan")
        );
    }

    #[test]
    fn a_navigation_loop_is_reported_rather_than_refused() {
        let cycles = navigator().cycles();
        assert_eq!(cycles.len(), 1);
        assert_eq!(cycles[0], vec!["home", "detail", "home"]);
    }

    #[test]
    fn an_acyclic_graph_has_no_cycles() {
        let navigator = Navigator::new()
            .with_page("a")
            .with_page("b")
            .with_entry("a")
            .with_edge("a", "b");
        assert!(navigator.cycles().is_empty());
    }

    #[test]
    fn a_self_loop_is_reported_separately() {
        let navigator = Navigator::new()
            .with_page("home")
            .with_entry("home")
            .with_edge("home", "home");
        assert_eq!(navigator.self_loops(), vec!["home"]);
        assert_eq!(navigator.cycles(), vec![vec!["home", "home"]]);
    }

    #[test]
    fn navigating_along_a_declared_edge_moves_and_stacks() {
        let navigator = navigator();
        let mut window = window();
        window
            .navigate(&navigator, Route::new("detail"))
            .expect("a navigation");
        assert_eq!(window.current().expect("a page").page, "detail");
        assert_eq!(window.back_stack().len(), 1);
        assert_eq!(window.back_stack()[0].page, "home");
    }

    #[test]
    fn an_undeclared_page_cannot_be_reached() {
        let navigator = navigator();
        let mut window = window();
        let outcome = window.navigate(&navigator, Route::new("gone"));
        assert!(outcome.unwrap_err().contains("no page"));
        assert_eq!(window.current().expect("a page").page, "home");
    }

    #[test]
    fn a_navigation_the_graph_does_not_have_is_refused() {
        let navigator = navigator();
        let mut window = window();
        window
            .navigate(&navigator, Route::new("detail"))
            .expect("a navigation");
        let outcome = window.navigate(&navigator, Route::new("settings"));
        assert!(outcome.unwrap_err().contains("no navigation from"));
    }

    #[test]
    fn back_and_forward_walk_the_stacks() {
        let navigator = navigator();
        let mut window = window();
        window
            .navigate(&navigator, Route::new("detail"))
            .expect("a navigation");
        assert_eq!(window.back().expect("a page").page, "home");
        assert_eq!(window.current().expect("a page").page, "home");
        assert_eq!(window.forward_stack().len(), 1);
        assert_eq!(window.forward().expect("a page").page, "detail");
        assert_eq!(window.back_stack().len(), 1);
    }

    #[test]
    fn navigating_clears_the_forward_stack() {
        let navigator = navigator();
        let mut window = window();
        window
            .navigate(&navigator, Route::new("detail"))
            .expect("a navigation");
        window.back().expect("a page");
        assert_eq!(window.forward_stack().len(), 1);
        window
            .navigate(&navigator, Route::new("settings"))
            .expect("a navigation");
        assert!(window.forward_stack().is_empty());
    }

    #[test]
    fn going_back_with_nothing_behind_does_nothing() {
        let mut window = window();
        assert!(window.back().is_none());
        assert!(window.forward().is_none());
    }

    #[test]
    fn a_closing_window_refuses_navigation_and_says_which_state_it_is_in() {
        let navigator = navigator();
        let mut window = window();
        window.close().expect("a close");
        assert_eq!(window.lifecycle, Lifecycle::Closing);
        assert!(window.lifecycle.name() == "closing");
        let outcome = window.navigate(&navigator, Route::new("detail"));
        assert!(outcome.unwrap_err().contains("closing"), "reason");
    }

    #[test]
    fn closing_twice_is_refused_and_closing_finishes() {
        let mut window = window();
        window.close().expect("a close");
        assert!(window.close().unwrap_err().contains("already closing"));
        window.finish_close().expect("a finish");
        assert_eq!(window.lifecycle, Lifecycle::Closed);
        assert!(window.close().unwrap_err().contains("already closed"));
        assert!(window.finish_close().is_err());
    }

    #[test]
    fn an_opening_window_is_not_ready_to_navigate() {
        let navigator = navigator();
        let mut window = Window::open("home");
        assert_eq!(window.lifecycle, Lifecycle::Opening);
        assert!(!window.lifecycle.accepts_navigation());
        assert!(window.navigate(&navigator, Route::new("detail")).is_err());
        window.ready();
        assert!(window.navigate(&navigator, Route::new("detail")).is_ok());
    }

    #[test]
    fn the_window_renders_its_state_and_stacks() {
        let navigator = navigator();
        let mut window = window();
        window
            .navigate(&navigator, Route::new("detail"))
            .expect("a navigation");
        let lines = window.render();
        assert_eq!(lines[0], "lifecycle: open");
        assert_eq!(lines[1], "current: detail");
        assert_eq!(lines[2], "back: 1");
        assert_eq!(lines[3], "forward: 0");
    }

    #[test]
    fn the_navigator_renders_its_graph() {
        let lines = navigator().render();
        assert_eq!(lines[0], "pages: 4; entries: 1; navigations: 3");
        assert!(lines.iter().any(|line| line == "  home -> detail"));
        assert!(
            lines
                .iter()
                .any(|line| line == "cycle: home -> detail -> home")
        );
    }
}
