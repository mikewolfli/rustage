// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The project file index: ignore rules, size/binary policy, watch updates, no symlink loops
//! (task T08.01).
//!
//! The acceptance line is *"it does not follow symlinks without end, and it does not block the user
//! interface"*. Neither is something a caller can be trusted to remember, so both are properties of
//! the walk:
//!
//! * **A symlink is listed, never followed.** A [`Kind::Symlink`] node becomes an
//!   [`Entry`] with [`State::Symlink`] and is not descended into, so a link to a parent — or to
//!   itself — cannot make the walk run forever. There is no "follow links" switch to get wrong.
//! * **The work is bounded, and boundedness is visible.** [`Rules`] carries a maximum depth, a
//!   maximum entry count and a maximum size; reaching the entry count stops the walk and sets
//!   [`Index::truncated`], so the index reports what it did not see instead of stalling on a huge
//!   tree. (Running the walk off the UI thread is the host executor's job, T01.03; this module is the
//!   bounded unit of work that runs there.)
//! * **Every skip has a reason.** A skipped path is [`State::Ignored`], [`State::TooLarge`],
//!   [`State::Binary`], [`State::Symlink`] or [`State::TooDeep`], so "why is this file not in the
//!   index" is answerable.
//! * **A watch event changes the index without a rebuild.** [`Index::apply`] folds one created,
//!   modified or removed path in, so a person's edits update the index incrementally.

#![allow(dead_code)]

/// What a node in the tree is.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Kind {
    /// A file of a given size.
    File {
        /// Its size in bytes.
        size: usize,
    },
    /// A directory.
    Dir,
    /// A symlink to another path.
    Symlink {
        /// What it points at.
        target: String,
    },
}

/// One node of a supplied tree listing.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Node {
    /// The path, relative to the root.
    pub path: String,
    /// What it is.
    pub kind: Kind,
}

impl Node {
    /// A file node.
    pub fn file(path: impl Into<String>, size: usize) -> Self {
        Self {
            path: path.into(),
            kind: Kind::File { size },
        }
    }

    /// A directory node.
    pub fn dir(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            kind: Kind::Dir,
        }
    }

    /// A symlink node.
    pub fn symlink(path: impl Into<String>, target: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            kind: Kind::Symlink {
                target: target.into(),
            },
        }
    }
}

/// A flat listing of a directory tree.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Tree {
    /// The nodes.
    pub nodes: Vec<Node>,
}

impl Tree {
    /// A tree from nodes.
    pub fn new(nodes: Vec<Node>) -> Self {
        Self { nodes }
    }
}

/// The index's policy: what to ignore and how much to see.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Rules {
    /// Path components or `*.ext` patterns to ignore.
    pub ignore: Vec<String>,
    /// File extensions treated as binary.
    pub binary: Vec<String>,
    /// Files larger than this are skipped.
    pub max_bytes: usize,
    /// Paths deeper than this are skipped.
    pub max_depth: usize,
    /// At most this many entries; reaching it truncates the walk.
    pub max_entries: usize,
}

impl Rules {
    /// The usual policy: ignore `target` and `.git`, skip files over a mebibyte and paths deeper than
    /// sixteen, and cap the index at fifty thousand entries.
    pub fn new() -> Self {
        Self {
            ignore: vec!["target".to_owned(), ".git".to_owned()],
            binary: vec![
                "png".to_owned(),
                "jpg".to_owned(),
                "gif".to_owned(),
                "pdf".to_owned(),
                "wasm".to_owned(),
            ],
            max_bytes: 1 << 20,
            max_depth: 16,
            max_entries: 50_000,
        }
    }

    /// Whether any component of a path is ignored.
    pub fn ignores(&self, path: &str) -> bool {
        let name = path.rsplit('/').next().unwrap_or(path);
        self.ignore.iter().any(|pattern| {
            if let Some(suffix) = pattern.strip_prefix("*.") {
                path.ends_with(&format!(".{suffix}"))
            } else {
                path.split('/').any(|component| component == pattern) || name == pattern
            }
        })
    }

    /// Whether a file's extension says it is binary.
    pub fn is_binary(&self, path: &str) -> bool {
        match path.rsplit_once('.') {
            Some((_, extension)) => self.binary.iter().any(|known| known == extension),
            None => false,
        }
    }
}

impl Default for Rules {
    fn default() -> Self {
        Self::new()
    }
}

/// Why an entry is or is not in the index.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum State {
    /// Indexed.
    Indexed,
    /// Matched an ignore rule.
    Ignored,
    /// Larger than the size ceiling.
    TooLarge,
    /// A binary file.
    Binary,
    /// A symlink, which is not followed.
    Symlink,
    /// Deeper than the depth ceiling.
    TooDeep,
}

impl State {
    /// The state's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Indexed => "indexed",
            Self::Ignored => "ignored",
            Self::TooLarge => "too-large",
            Self::Binary => "binary",
            Self::Symlink => "symlink",
            Self::TooDeep => "too-deep",
        }
    }

    /// Whether the entry is in the index.
    pub fn is_indexed(&self) -> bool {
        matches!(self, Self::Indexed)
    }
}

/// One indexed (or skipped) path.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Entry {
    /// The path.
    pub path: String,
    /// Its size, for a file.
    pub size: usize,
    /// Its state.
    pub state: State,
}

impl Entry {
    /// The entry in one line.
    pub fn render(&self) -> String {
        format!(
            "{}: {} ({} byte(s))",
            self.path,
            self.state.name(),
            self.size
        )
    }
}

/// A watch event the index folds in.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Event {
    /// A path appeared.
    Created(Node),
    /// A path changed size.
    Modified {
        /// The path.
        path: String,
        /// Its new size.
        size: usize,
    },
    /// A path went away.
    Removed {
        /// The path.
        path: String,
    },
}

/// The project's file index.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Index {
    entries: Vec<Entry>,
    truncated: bool,
}

impl Index {
    /// Builds an index over a tree under a policy.
    ///
    /// The walk stops at the entry ceiling and reports it rather than looking forever.
    pub fn build(tree: &Tree, rules: &Rules) -> Self {
        let mut index = Self {
            entries: Vec::new(),
            truncated: false,
        };
        // A stable order makes the index reproducible.
        let mut nodes: Vec<&Node> = tree.nodes.iter().collect();
        nodes.sort_by(|a, b| a.path.cmp(&b.path));
        for node in nodes {
            if index.entries.len() >= rules.max_entries {
                index.truncated = true;
                break;
            }
            index.entries.push(classify(node, rules));
        }
        index
    }

    /// The entries.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// The indexed paths.
    pub fn indexed(&self) -> Vec<&Entry> {
        self.entries
            .iter()
            .filter(|e| e.state.is_indexed())
            .collect()
    }

    /// The skipped paths.
    pub fn skipped(&self) -> Vec<&Entry> {
        self.entries
            .iter()
            .filter(|e| !e.state.is_indexed())
            .collect()
    }

    /// Whether the walk stopped at the entry ceiling.
    pub fn truncated(&self) -> bool {
        self.truncated
    }

    /// Whether a path is indexed.
    pub fn contains(&self, path: &str) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.path == path && entry.state.is_indexed())
    }

    /// Folds one watch event into the index.
    pub fn apply(&mut self, event: Event, rules: &Rules) {
        match event {
            Event::Created(node) => {
                if self.entries.len() < rules.max_entries {
                    self.entries.retain(|entry| entry.path != node.path);
                    self.entries.push(classify(&node, rules));
                    self.entries.sort_by(|a, b| a.path.cmp(&b.path));
                } else {
                    self.truncated = true;
                }
            }
            Event::Modified { path, size } => {
                if let Some(entry) = self.entries.iter_mut().find(|entry| entry.path == path) {
                    entry.size = size;
                    entry.state = state_for_file(&path, size, rules);
                }
            }
            Event::Removed { path } => {
                self.entries.retain(|entry| entry.path != path);
            }
        }
    }

    /// The index in one report.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "index: {} entry(s), {} indexed, {} skipped{}",
            self.entries.len(),
            self.indexed().len(),
            self.skipped().len(),
            if self.truncated { ", truncated" } else { "" }
        )];
        for entry in &self.entries {
            lines.push(format!("  {}", entry.render()));
        }
        lines
    }
}

/// Classifies a node under a policy.
fn classify(node: &Node, rules: &Rules) -> Entry {
    let depth = node.path.split('/').filter(|part| !part.is_empty()).count();
    let (size, state) = if rules.ignores(&node.path) {
        (0, State::Ignored)
    } else if depth > rules.max_depth {
        (0, State::TooDeep)
    } else {
        match &node.kind {
            Kind::Symlink { .. } => (0, State::Symlink),
            Kind::Dir => (0, State::Indexed),
            Kind::File { size } => (*size, state_for_file(&node.path, *size, rules)),
        }
    };
    Entry {
        path: node.path.clone(),
        size,
        state,
    }
}

/// The state a file of a size has under a policy.
fn state_for_file(path: &str, size: usize, rules: &Rules) -> State {
    if size > rules.max_bytes {
        State::TooLarge
    } else if rules.is_binary(path) {
        State::Binary
    } else {
        State::Indexed
    }
}

#[cfg(test)]
mod tests {
    use super::{Event, Index, Kind, Node, Rules, State, Tree};

    fn tree() -> Tree {
        Tree::new(vec![
            Node::dir("src"),
            Node::file("src/main.rs", 100),
            Node::file("target/debug/rustage", 5000),
            Node::file("assets/logo.png", 200),
            Node::file("notes.md", 50),
            Node::symlink("loop", ".."),
            Node::file("big.bin", 2_000_000),
        ])
    }

    #[test]
    fn a_symlink_is_listed_but_not_followed() {
        let index = Index::build(&tree(), &Rules::new());
        let link = index
            .entries()
            .iter()
            .find(|entry| entry.path == "loop")
            .expect("the symlink");
        assert_eq!(link.state, State::Symlink);
        assert!(!index.contains("loop"));
        // It is never followed, so nothing under the link's target appears.
        assert!(
            index
                .entries()
                .iter()
                .all(|entry| !entry.path.starts_with("loop/"))
        );
    }

    #[test]
    fn an_ignore_rule_skips_a_directory_component() {
        let index = Index::build(&tree(), &Rules::new());
        let ignored = index
            .entries()
            .iter()
            .find(|entry| entry.path == "target/debug/rustage")
            .expect("the ignored file");
        assert_eq!(ignored.state, State::Ignored);
    }

    #[test]
    fn a_large_or_binary_file_is_skipped_with_a_reason() {
        let index = Index::build(&tree(), &Rules::new());
        assert_eq!(state(&index, "big.bin"), State::TooLarge);
        assert_eq!(state(&index, "assets/logo.png"), State::Binary);
        assert_eq!(state(&index, "src/main.rs"), State::Indexed);
    }

    #[test]
    fn the_walk_stops_at_the_entry_ceiling_rather_than_looking_forever() {
        let mut rules = Rules::new();
        rules.max_entries = 3;
        let index = Index::build(&tree(), &rules);
        assert!(index.truncated());
        assert_eq!(index.entries().len(), 3);
    }

    #[test]
    fn a_deep_path_is_too_deep() {
        let mut rules = Rules::new();
        rules.max_depth = 2;
        let tree = Tree::new(vec![Node::file("a/b/c/deep.rs", 10)]);
        let index = Index::build(&tree, &rules);
        assert_eq!(state(&index, "a/b/c/deep.rs"), State::TooDeep);
    }

    #[test]
    fn a_watch_event_changes_the_index_without_a_rebuild() {
        let mut index = Index::build(&tree(), &Rules::new());
        assert!(!index.contains("src/new.rs"));
        index.apply(Event::Created(Node::file("src/new.rs", 10)), &Rules::new());
        assert!(index.contains("src/new.rs"));
        // A modification past the size ceiling reclassifies the file.
        index.apply(
            Event::Modified {
                path: "src/new.rs".to_owned(),
                size: 9_000_000,
            },
            &Rules::new(),
        );
        assert_eq!(state(&index, "src/new.rs"), State::TooLarge);
        // Removal drops it.
        index.apply(
            Event::Removed {
                path: "src/new.rs".to_owned(),
            },
            &Rules::new(),
        );
        assert!(!index.contains("src/new.rs"));
    }

    #[test]
    fn a_symlink_node_kind_is_recognised() {
        let node = Node::symlink("link", "../elsewhere");
        assert!(matches!(node.kind, Kind::Symlink { target } if target == "../elsewhere"));
    }

    fn state(index: &Index, path: &str) -> State {
        index
            .entries()
            .iter()
            .find(|entry| entry.path == path)
            .map(|entry| entry.state.clone())
            .unwrap_or(State::Ignored)
    }
}
