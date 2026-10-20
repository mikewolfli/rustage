// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The workbench's Pages panel (task T13.05, J02's window entry).
//!
//! The panel edits the same [`Navigator`] the CLI drives: pages, the entry and the navigation
//! edges. A route is only taken along a declared edge, a missing target is refused with the page
//! named, and the window's lifecycle is a row rather than a hidden flag — a closing window refuses
//! navigation and a second close says it is already closing.

use crate::app::data::routes::{Navigator, Route, Window};

/// The Pages panel's model.
pub(super) struct PagesState {
    navigator: Navigator,
    window: Window,
    revision: u64,
}

impl PagesState {
    /// Two pages, an entry at `home` and the `home -> detail` edge, with an open window on `home`.
    pub(super) fn new() -> Self {
        let navigator = Navigator::new()
            .with_page("home")
            .with_entry("home")
            .with_page("detail")
            .with_edge("home", "detail");
        let mut window = Window::open("home");
        window.ready();
        Self {
            navigator,
            window,
            revision: 0,
        }
    }

    /// One row per page (the entry and the current page are marked), then the window's state and
    /// the graph's diagnostics.
    pub(super) fn rows(&self) -> Vec<String> {
        let current = self
            .window
            .current()
            .map(|route| route.page.as_str())
            .unwrap_or("");
        let mut rows = Vec::new();
        for page in &self.navigator.pages {
            let mut marks = Vec::new();
            if self.navigator.entries.iter().any(|entry| entry == page) {
                marks.push("entry");
            }
            if page == current {
                marks.push("current");
            }
            rows.push(if marks.is_empty() {
                page.clone()
            } else {
                format!("{page} [{}]", marks.join(", "))
            });
        }
        rows.push(format!(
            "window: {} · current {} · back {} · forward {}",
            self.window.lifecycle.name(),
            current,
            self.window.back_stack().len(),
            self.window.forward_stack().len()
        ));
        if self.navigator.missing_entry() {
            rows.push("problem: no entry page is declared".to_owned());
        }
        let unreachable = self.navigator.unreachable();
        if !unreachable.is_empty() {
            rows.push(format!("problem: unreachable {}", unreachable.join(", ")));
        }
        for cycle in self.navigator.cycles() {
            rows.push(format!("cycle: {}", cycle.join(" -> ")));
        }
        for page in self.navigator.self_loops() {
            rows.push(format!("self-loop: {page}"));
        }
        rows
    }

    /// The page names, in declaration order.
    pub(super) fn pages(&self) -> Vec<String> {
        self.navigator.pages.clone()
    }

    /// The current page.
    pub(super) fn current(&self) -> Option<&str> {
        self.window.current().map(|route| route.page.as_str())
    }

    /// One accepted navigation, page declaration or close, for the panel's revision.
    pub(super) fn revision(&self) -> u64 {
        self.revision
    }

    /// Declares a page. An empty or duplicate name is refused and changes nothing.
    pub(super) fn add_page(&mut self, name: &str) -> Result<u64, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("a page name is required".to_owned());
        }
        if self.navigator.has_page(name) {
            return Err(format!("page {name:?} is already declared"));
        }
        self.navigator.pages.push(name.to_owned());
        self.revision += 1;
        Ok(self.revision)
    }

    /// Declares a navigation between two declared pages.
    pub(super) fn add_edge(&mut self, from: &str, to: &str) -> Result<u64, String> {
        if !self.navigator.has_page(from) {
            return Err(format!("no page {from:?} is declared"));
        }
        if !self.navigator.has_page(to) {
            return Err(format!("no page {to:?} is declared"));
        }
        if self
            .navigator
            .edges
            .iter()
            .any(|(edge_from, edge_to)| edge_from == from && edge_to == to)
        {
            return Err(format!(
                "there is already a navigation from {from:?} to {to:?}"
            ));
        }
        self.navigator.edges.push((from.to_owned(), to.to_owned()));
        self.revision += 1;
        Ok(self.revision)
    }

    /// Opens a page through the button a person actually presses: navigate while the window is
    /// open, start a fresh window when it is closing or closed.
    pub(super) fn open_page(&mut self, page: &str) -> Result<u64, String> {
        if self.window.lifecycle.accepts_navigation() {
            self.navigate(page)
        } else {
            self.open(page)
        }
    }

    /// Navigates to a declared page along a declared edge.
    pub(super) fn navigate(&mut self, page: &str) -> Result<u64, String> {
        self.window.navigate(&self.navigator, Route::new(page))?;
        self.revision += 1;
        Ok(self.revision)
    }

    /// Goes back one page, if the back stack has one.
    pub(super) fn back(&mut self) -> Result<String, String> {
        self.window
            .back()
            .map(|route| {
                self.revision += 1;
                route.render()
            })
            .ok_or_else(|| "the back stack is empty".to_owned())
    }

    /// Goes forward one page, if the forward stack has one.
    pub(super) fn forward(&mut self) -> Result<String, String> {
        self.window
            .forward()
            .map(|route| {
                self.revision += 1;
                route.render()
            })
            .ok_or_else(|| "the forward stack is empty".to_owned())
    }

    /// Starts closing the window, or finishes a close that is already in progress.
    pub(super) fn close(&mut self) -> Result<String, String> {
        if self.window.lifecycle == crate::app::data::routes::Lifecycle::Closing {
            self.window.finish_close()?;
            self.revision += 1;
            return Ok("window closed".to_owned());
        }
        self.window.close()?;
        self.revision += 1;
        Ok("window closing".to_owned())
    }

    /// Opens a fresh window on a declared page.
    pub(super) fn open(&mut self, page: &str) -> Result<u64, String> {
        if !self.navigator.has_page(page) {
            return Err(format!("no page {page:?} is declared"));
        }
        let mut window = Window::open(page);
        window.ready();
        self.window = window;
        self.revision += 1;
        Ok(self.revision)
    }
}

#[cfg(test)]
mod tests {
    use super::PagesState;

    #[test]
    fn the_demo_graph_has_a_current_page_and_a_declared_edge() {
        let state = PagesState::new();
        assert_eq!(state.current(), Some("home"));
        assert!(
            state.rows()[0].contains("entry, current"),
            "{:?}",
            state.rows()
        );
        assert!(state.rows().iter().any(|row| row.contains("detail")));
    }

    #[test]
    fn navigating_follows_a_declared_edge_and_back_returns() {
        let mut state = PagesState::new();
        state.navigate("detail").expect("a declared edge");
        assert_eq!(state.current(), Some("detail"));
        assert_eq!(state.back().expect("a back entry"), "home");
        assert_eq!(state.current(), Some("home"));
        assert_eq!(state.forward().expect("a forward entry"), "detail");
    }

    #[test]
    fn a_missing_page_and_an_undeclared_edge_are_refused() {
        let mut state = PagesState::new();
        let reason = state.navigate("missing").expect_err("no page");
        assert!(reason.contains("no page"), "{reason}");
        state.add_page("settings").expect("a new page");
        let reason = state.navigate("settings").expect_err("no edge from home");
        assert!(reason.contains("no navigation"), "{reason}");
    }

    #[test]
    fn a_new_page_is_declared_once_and_a_duplicate_is_refused() {
        let mut state = PagesState::new();
        let version = state.add_page("settings").expect("a new page");
        assert_eq!(version, 1);
        assert!(state.pages().contains(&"settings".to_owned()));
        assert!(state.add_page("settings").is_err());
    }

    #[test]
    fn an_edge_to_an_undeclared_page_is_refused() {
        let mut state = PagesState::new();
        let reason = state.add_edge("home", "nowhere").expect_err("no page");
        assert!(reason.contains("nowhere"), "{reason}");
        let reason = state.add_edge("home", "detail").expect_err("duplicate");
        assert!(reason.contains("already"), "{reason}");
    }

    #[test]
    fn a_closing_window_refuses_navigation_and_a_second_close_is_diagnosable() {
        let mut state = PagesState::new();
        assert_eq!(state.close().expect("a close"), "window closing");
        let reason = state.navigate("detail").expect_err("closing");
        assert!(reason.contains("closing"), "{reason}");
        assert_eq!(state.close().expect("a finish"), "window closed");
        assert!(state.close().is_err());
        assert_eq!(state.open("home").expect("a new window"), 3);
        assert_eq!(state.current(), Some("home"));
    }
}
