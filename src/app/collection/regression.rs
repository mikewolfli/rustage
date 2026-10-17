// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The end-to-end walk of a collection view: a large set, an empty set, an error, multi-language
//! content, the keyboard, and both generation modes (task T27.08).
//!
//! The acceptance line is *"internal content / selection / editing match the real artifact; a large
//! collection is not rendered in full"*. Two ideas carry it:
//!
//! * **Rendering is a window, not the whole collection.** [`window`] clamps the visible range to the
//!   viewport, and [`render_window`] asks for a label once per visible row and never for one outside
//!   it. The cost of drawing follows the viewport's height, not the collection's length, so a million
//!   rows cost the same as twenty.
//! * **What is shown is what is produced.** [`Snapshot`] holds exactly the rows in the window; its
//!   [`Snapshot::displayed`] and [`Snapshot::artifact`] are built from the same content, and
//!   [`content_matches`] checks that the artifact carries the displayed content unchanged in both
//!   generation modes. A selection is a mark on the artifact, not different content.
//! * **A failure is not an empty state.** [`GridState::Error`] is shown as an error; it is never
//!   rendered as the empty message, and the empty message is itself explicit rather than blank.

#![allow(dead_code)]

/// The part of a collection a view shows.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Viewport {
    /// How many rows fit.
    pub height: usize,
    /// The first visible row.
    pub first: usize,
}

impl Viewport {
    /// A viewport of this height, refusing a zero height and starting at the first row.
    pub fn new(height: usize) -> Result<Self, String> {
        if height == 0 {
            return Err("a viewport needs a positive height".to_owned());
        }
        Ok(Self { height, first: 0 })
    }

    /// The same viewport, scrolled to a row.
    pub fn scrolled_to(mut self, first: usize) -> Self {
        self.first = first;
        self
    }
}

/// The visible range of a collection.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Window {
    /// The first visible row.
    pub first: usize,
    /// How many rows are visible.
    pub count: usize,
}

impl Window {
    /// One past the last visible row.
    pub fn end(&self) -> usize {
        self.first + self.count
    }

    /// Whether nothing is visible.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// The visible row indices.
    pub fn indices(&self) -> std::ops::Range<usize> {
        self.first..self.end()
    }
}

/// The visible window of a collection of `total` rows, bounded by the viewport rather than the
/// collection's length.
pub fn window(total: usize, viewport: &Viewport) -> Window {
    let first = viewport.first.min(total);
    let count = viewport.height.min(total - first);
    Window { first, count }
}

/// What a collection view is showing.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum GridState {
    /// Rows to show.
    Ready,
    /// No rows, with the message the view shows.
    Empty(String),
    /// A failure, with the reason it shows.
    Error(String),
}

impl GridState {
    /// Whether the state is an error.
    pub fn is_error(&self) -> bool {
        matches!(self, Self::Error(_))
    }
}

/// Renders the visible window of a collection.
///
/// `label` is called exactly once for each visible row and never for a row outside the window, so the
/// work is the viewport's height rather than the collection's length. An error is shown as an error,
/// never as the empty state.
pub fn render_window<F: FnMut(usize) -> String>(
    total: usize,
    viewport: &Viewport,
    state: &GridState,
    mut label: F,
) -> Vec<String> {
    match state {
        GridState::Error(reason) => return vec![format!("error: {reason}")],
        GridState::Empty(message) => return vec![format!("empty: {message}")],
        GridState::Ready if total == 0 => return vec!["empty: no rows".to_owned()],
        GridState::Ready => {}
    }
    let visible = window(total, viewport);
    let mut lines = vec![format!(
        "rows {}..{} of {total}",
        visible.first,
        visible.end()
    )];
    for index in visible.indices() {
        lines.push(label(index));
    }
    lines
}

/// Which of the two generation modes a walk is in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// The static artifact.
    Static,
    /// The live artifact.
    Live,
}

impl Mode {
    /// The mode's name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Static => "static",
            Self::Live => "live",
        }
    }

    /// Reads a mode's name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "static" => Some(Self::Static),
            "live" => Some(Self::Live),
            _ => None,
        }
    }
}

/// The rows a view shows in one window, in one generation mode.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Snapshot {
    /// The generation mode.
    pub mode: Mode,
    /// The visible window.
    pub window: Window,
    /// The visible rows' content, in order.
    pub lines: Vec<String>,
}

impl Snapshot {
    /// Whether the mode is the live one.
    pub fn is_live(&self) -> bool {
        self.mode == Mode::Live
    }

    /// The lines a person sees for this window.
    pub fn displayed(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "{} window {}..{}",
            self.mode.name(),
            self.window.first,
            self.window.end()
        )];
        lines.extend(self.lines.iter().cloned());
        lines
    }

    /// The lines the generator writes for the same window.
    ///
    /// The content is exactly the displayed content; in the live mode a selected row is marked, and
    /// stripping the mark recovers what was displayed, so a selection never changes the content.
    pub fn artifact(&self, selection: &[usize]) -> Vec<String> {
        let mut lines = self.displayed();
        if self.mode == Mode::Live {
            for (offset, line) in self.lines.iter().enumerate() {
                let index = self.window.first + offset;
                if selection.contains(&index) {
                    lines[1 + offset] = format!("* {line}");
                }
            }
        }
        lines
    }
}

/// Builds a window's snapshot, calling `label` only for visible rows.
pub fn snapshot<F: FnMut(usize) -> String>(
    total: usize,
    viewport: &Viewport,
    mode: Mode,
    label: F,
) -> Snapshot {
    let visible = window(total, viewport);
    let lines = render_window(total, viewport, &GridState::Ready, label);
    Snapshot {
        mode,
        window: visible,
        // The first line is the window header; the rest are the visible rows.
        lines: lines.into_iter().skip(1).collect(),
    }
}

/// Whether the artifact carries exactly the content that was displayed.
pub fn content_matches(snapshot: &Snapshot, selection: &[usize]) -> bool {
    let stripped: Vec<String> = snapshot
        .artifact(selection)
        .into_iter()
        .map(|line| line.strip_prefix("* ").map(str::to_owned).unwrap_or(line))
        .collect();
    stripped == snapshot.displayed()
}

/// Applies a grid key to a viewport, clamping the scroll so it never runs past the collection.
pub fn keyboard(command: &str, total: usize, viewport: &Viewport) -> Result<Viewport, String> {
    let mut next = viewport.clone();
    let last = total.saturating_sub(next.height);
    match command {
        "down" => next.first = (next.first + 1).min(last),
        "up" => next.first = next.first.saturating_sub(1),
        "page-down" => next.first = (next.first + next.height).min(last),
        "page-up" => next.first = next.first.saturating_sub(next.height),
        "home" => next.first = 0,
        "end" => next.first = last,
        other => return Err(format!("{other:?} is not a grid key")),
    }
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::{
        GridState, Mode, Viewport, content_matches, keyboard, render_window, snapshot, window,
    };
    use crate::app::collection::delegate::{DelegateKind, Edit, Saves};

    #[test]
    fn a_large_collection_renders_only_the_window() {
        let viewport = Viewport::new(20).expect("a viewport");
        let mut calls = 0;
        let lines = render_window(1_000_000, &viewport, &GridState::Ready, |index| {
            calls += 1;
            format!("row {index}")
        });
        // One header plus the visible rows, and the label ran once per visible row.
        assert_eq!(lines.len(), 21);
        assert_eq!(calls, 20);
    }

    #[test]
    fn the_last_window_does_not_run_past_the_end() {
        let viewport = Viewport::new(20).expect("a viewport").scrolled_to(999_997);
        let visible = window(1_000_000, &viewport);
        assert_eq!(visible.first, 999_997);
        assert_eq!(visible.count, 3);
        assert_eq!(visible.end(), 1_000_000);
    }

    #[test]
    fn a_zero_height_viewport_is_refused() {
        assert!(Viewport::new(0).is_err());
    }

    #[test]
    fn an_empty_collection_states_the_empty_policy() {
        let viewport = Viewport::new(10).expect("a viewport");
        let ready = render_window(0, &viewport, &GridState::Ready, |index| index.to_string());
        assert_eq!(ready, vec!["empty: no rows".to_owned()]);
        let named = render_window(
            0,
            &viewport,
            &GridState::Empty("no orders yet".to_owned()),
            |index| index.to_string(),
        );
        assert!(named[0].contains("no orders yet"));
    }

    #[test]
    fn an_error_is_shown_as_an_error_and_not_as_empty() {
        let viewport = Viewport::new(10).expect("a viewport");
        let state = GridState::Error("the source is unreachable".to_owned());
        assert!(state.is_error());
        let lines = render_window(50, &viewport, &state, |index| index.to_string());
        assert_eq!(lines.len(), 1);
        assert!(lines[0].starts_with("error:"));
        assert!(!lines[0].contains("empty"));
    }

    #[test]
    fn multi_language_content_reaches_the_artifact_unchanged() {
        let viewport = Viewport::new(2).expect("a viewport");
        let shown = snapshot(3, &viewport, Mode::Static, |index| match index {
            0 => "日本語".to_owned(),
            1 => "café".to_owned(),
            _ => "العربية".to_owned(),
        });
        assert_eq!(shown.lines, vec!["日本語", "café"]);
        assert!(content_matches(&shown, &[]));
    }

    #[test]
    fn the_live_artifact_marks_the_selection_and_static_does_not() {
        let viewport = Viewport::new(3).expect("a viewport");
        let label = |index: usize| format!("row {index}");
        let live = snapshot(10, &viewport, Mode::Live, label);
        let marked = live.artifact(&[1]);
        assert!(marked.iter().any(|line| line == "* row 1"));
        assert!(content_matches(&live, &[1]));

        let statik = snapshot(10, &viewport, Mode::Static, label);
        assert!(
            !statik
                .artifact(&[1])
                .iter()
                .any(|line| line.starts_with("* "))
        );
        assert!(content_matches(&statik, &[1]));
    }

    #[test]
    fn scrolling_is_clamped_to_the_collection() {
        let viewport = Viewport::new(10).expect("a viewport");
        let down = keyboard("page-down", 25, &viewport).expect("a key");
        assert_eq!(down.first, 10);
        let end = keyboard("end", 25, &viewport).expect("a key");
        assert_eq!(end.first, 15);
        let home = keyboard("home", 25, &end).expect("a key");
        assert_eq!(home.first, 0);
        assert!(keyboard("swipe", 25, &viewport).is_err());
    }

    #[test]
    fn a_committed_edit_reaches_the_window_and_the_artifact_together() {
        let mut edit = Edit::begin(3, "name", "old");
        edit.set("renamed");
        let mut saves = Saves::new();
        assert!(edit.commit(&DelegateKind::Text, &mut saves));
        assert_eq!(edit.value(), Some("renamed"));

        let committed = edit.value().unwrap_or_default().to_owned();
        let viewport = Viewport::new(2).expect("a viewport").scrolled_to(3);
        let shown = snapshot(10, &viewport, Mode::Static, move |index| {
            if index == 3 {
                committed.clone()
            } else {
                format!("row {index}")
            }
        });
        assert_eq!(shown.lines, vec!["renamed", "row 4"]);
        assert!(content_matches(&shown, &[]));
        assert_eq!(saves.len(), 1);
    }

    #[test]
    fn the_visible_window_is_bounded_by_the_viewport() {
        let viewport = Viewport::new(5).expect("a viewport");
        let visible = window(2, &viewport);
        assert_eq!(visible.count, 2);
        assert!(!visible.is_empty());
    }
}
