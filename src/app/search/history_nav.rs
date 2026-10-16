// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Navigation history: forward, back, edit points, problem jumps and file-move correction (task T08.06).
//!
//! The acceptance line is *"a vanished file does not jump to the wrong object"*. The failure the line
//! describes is a jump that lands somewhere plausible but wrong, so the module refuses rather than
//! guesses:
//!
//! * **A location is a uri and an offset, and it stays its own.** [`Location`] carries the file it is
//!   in; nothing in the history rewrites one location into another.
//! * **Resolution is checked, and a missing file is refused.** [`resolve`] takes the set of files that
//!   exist and answers [`JumpError::Vanished`] for one that does not — it never falls back to another
//!   file that merely has the same offset.
//! * **A moved file is corrected, not lost.** [`History::rename`] rewrites the recorded uris of a
//!   moved file, so a jump after a move lands on the same object at its new path; only a file that
//!   really went away is refused.
//! * **Edits and problems are their own origins.** [`Origin`] marks which of a visit, an edit point or
//!   a problem jump says where a person came from, so the list can be read without guessing.

#![allow(dead_code)]

/// Why a location is in the history.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Origin {
    /// A place a person navigated to.
    Visit,
    /// An edit point, recorded when editing settled somewhere.
    Edit,
    /// A problem jump.
    Problem,
}

impl Origin {
    /// Every origin, in a stable order.
    pub const ALL: [Self; 3] = [Self::Visit, Self::Edit, Self::Problem];

    /// The origin's name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Visit => "visit",
            Self::Edit => "edit",
            Self::Problem => "problem",
        }
    }

    /// The origin a name picks.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|origin| origin.name() == name)
    }
}

/// One place in the history.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Location {
    /// The file.
    pub uri: String,
    /// The byte offset.
    pub offset: usize,
    /// Why it is here.
    pub origin: Origin,
}

impl Location {
    /// A location.
    pub fn new(uri: impl Into<String>, offset: usize, origin: Origin) -> Self {
        Self {
            uri: uri.into(),
            offset,
            origin,
        }
    }

    /// The location in one line.
    pub fn render(&self) -> String {
        format!("{}:{} [{}]", self.uri, self.offset, self.origin.name())
    }
}

/// Why a jump was refused.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct JumpError {
    /// The file that is gone.
    pub uri: String,
}

impl JumpError {
    /// The refusal in one sentence.
    pub fn message(&self) -> String {
        format!("{} is gone; the jump is refused", self.uri)
    }
}

/// The navigation history: where a person has been, and where they can go.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct History {
    past: Vec<Location>,
    future: Vec<Location>,
    current: Option<Location>,
}

impl History {
    /// Empty history.
    pub fn new() -> Self {
        Self::default()
    }

    /// Where the person is now.
    pub fn current(&self) -> Option<&Location> {
        self.current.as_ref()
    }

    /// Records a visit, leaving the current place on the back stack.
    pub fn visit(&mut self, location: Location) {
        if let Some(current) = self.current.replace(location) {
            self.past.push(current);
        }
        // A new visit abandons the forward branch.
        self.future.clear();
    }

    /// Records a location of an origin.
    pub fn record(&mut self, uri: impl Into<String>, offset: usize, origin: Origin) {
        self.visit(Location::new(uri, offset, origin));
    }

    /// Goes back, returning the place it lands on.
    pub fn back(&mut self) -> Option<Location> {
        let previous = self.past.pop()?;
        if let Some(current) = self.current.replace(previous.clone()) {
            self.future.push(current);
        }
        Some(previous)
    }

    /// Goes forward, returning the place it lands on.
    pub fn forward(&mut self) -> Option<Location> {
        let next = self.future.pop()?;
        if let Some(current) = self.current.replace(next.clone()) {
            self.past.push(current);
        }
        Some(next)
    }

    /// Rewrites the recorded uris of a moved file, returning how many entries moved.
    pub fn rename(&mut self, from: &str, to: &str) -> usize {
        let mut moved = 0;
        let mut remap = |location: &mut Location| {
            if location.uri == from {
                location.uri = to.to_owned();
                moved += 1;
            }
        };
        for location in &mut self.past {
            remap(location);
        }
        if let Some(current) = &mut self.current {
            remap(current);
        }
        for location in &mut self.future {
            remap(location);
        }
        moved
    }

    /// How many places are on the back stack.
    pub fn back_count(&self) -> usize {
        self.past.len()
    }

    /// How many places are on the forward stack.
    pub fn forward_count(&self) -> usize {
        self.future.len()
    }

    /// The history in one report.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "history: {} back, {} forward",
            self.past.len(),
            self.future.len()
        )];
        for location in &self.past {
            lines.push(format!("  back  {}", location.render()));
        }
        if let Some(current) = &self.current {
            lines.push(format!("  now   {}", current.render()));
        }
        for location in &self.future {
            lines.push(format!("  ahead {}", location.render()));
        }
        lines
    }
}

/// Resolves a location against the files that exist, refusing a vanished one.
pub fn resolve(location: &Location, exists: impl Fn(&str) -> bool) -> Result<Location, JumpError> {
    if exists(&location.uri) {
        Ok(location.clone())
    } else {
        Err(JumpError {
            uri: location.uri.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{History, JumpError, Location, Origin, resolve};
    use std::collections::BTreeSet;

    #[test]
    fn back_and_forward_walk_the_history() {
        let mut history = History::new();
        history.record("a.rs", 0, Origin::Visit);
        history.record("b.rs", 10, Origin::Edit);
        history.record("c.rs", 20, Origin::Problem);
        assert_eq!(history.current().expect("now").uri, "c.rs");
        assert_eq!(history.back().expect("back").uri, "b.rs");
        assert_eq!(history.back().expect("back").uri, "a.rs");
        assert!(history.back().is_none());
        assert_eq!(history.forward().expect("forward").uri, "b.rs");
        // A new visit abandons the forward branch.
        history.record("d.rs", 0, Origin::Visit);
        assert_eq!(history.forward_count(), 0);
    }

    #[test]
    fn a_vanished_file_is_refused_not_redirected() {
        let mut existing = BTreeSet::new();
        existing.insert("a.rs".to_owned());
        existing.insert("b.rs".to_owned());
        let location = Location::new("a.rs", 5, Origin::Visit);
        // While a.rs exists, it resolves to itself.
        assert_eq!(
            resolve(&location, |uri| existing.contains(uri)).expect("resolves"),
            location
        );
        // Once a.rs is gone, the jump is refused — never redirected to b.rs.
        existing.remove("a.rs");
        match resolve(&location, |uri| existing.contains(uri)) {
            Err(JumpError { uri }) => assert_eq!(uri, "a.rs"),
            Ok(other) => panic!("a vanished file resolved to {other:?}"),
        }
    }

    #[test]
    fn a_moved_file_is_corrected_in_the_history() {
        let mut history = History::new();
        history.record("old/a.rs", 3, Origin::Visit);
        history.record("b.rs", 0, Origin::Visit);
        let moved = history.rename("old/a.rs", "new/a.rs");
        assert_eq!(moved, 1);
        // The history now names the new path and no longer the old one.
        let rendered = history.render();
        assert!(rendered.iter().any(|line| line.contains("new/a.rs")));
        assert!(!rendered.iter().any(|line| line.contains("old/a.rs")));
        // And a back jump lands on the same object at its new path.
        assert_eq!(history.back().expect("back").uri, "new/a.rs");
    }

    #[test]
    fn the_current_place_is_renamed_too() {
        let mut history = History::new();
        history.record("old/a.rs", 7, Origin::Edit);
        assert_eq!(history.rename("old/a.rs", "new/a.rs"), 1);
        let current = history.current().expect("now");
        assert_eq!(current.uri, "new/a.rs");
        assert_eq!(current.offset, 7);
        assert_eq!(current.origin, Origin::Edit);
    }

    #[test]
    fn an_origin_name_round_trips() {
        for origin in Origin::ALL {
            assert_eq!(Origin::from_name(origin.name()), Some(origin));
        }
        assert_eq!(Origin::from_name("nowhere"), None);
    }
}
