// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The test tree, ignored/parameterized marking and targeted reruns (roadmap task T21.10).
//!
//! Discovery gives a flat list of test paths. That list answers "which tests exist" but not the
//! questions a test panel asks: which module a test belongs to, which tests are parameterized, which
//! are ignored, and which of the results still belong to the document revision being edited.
//!
//! * **The tree is built from the harness's own listing.** A path is split on `::`; the last
//!   segment is the test and the earlier ones are modules, so the hierarchy cannot disagree with the
//!   names libtest printed.
//! * **Parameterized and ignored are marked, not guessed.** A test whose leaf segment carries
//!   `[..]` is parameterized; `ignored` comes from the discovery listing's `--list --ignored` pass,
//!   never from the name.
//! * **Ignored tests are not run automatically.** [`TestTree::auto_run`] returns only the tests that
//!   are not ignored, and an empty answer is a reason to run nothing rather than "run everything".
//!   A test that has side effects is exactly the one an acceptance run must not start by itself.
//! * **A targeted rerun is by name.** [`TestTree::failed`] and [`TestTree::matching`] return exact
//!   paths, so a rerun cannot silently widen to a substring that matches other tests.
//! * **Stale results stay stale.** This module does not relabel results; [`Tests::is_stale`] is the
//!   revision fact, and the command prints it. Not running a test is already "not run", never a pass.

#![allow(dead_code)]

use std::collections::BTreeMap;

use crate::app::quality::report::{TestStatus, Tests};
use crate::app::quality::test_runner::Discovery;

/// One test in the tree.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Leaf {
    /// The full path libtest printed.
    pub name: String,
    /// The module path (everything before the last `::`).
    pub module: String,
    /// Whether discovery found it in `--list --ignored`.
    pub ignored: bool,
    /// Whether its leaf segment carries a `[..]` parameter marker.
    pub parameterized: bool,
}

/// A module or the root.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
struct Node {
    children: BTreeMap<String, Node>,
    tests: Vec<Leaf>,
}

/// The discovered tests as a module tree.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct TestTree {
    root: Node,
}

impl TestTree {
    /// Builds the tree from one discovery.
    pub fn from_discovery(discovery: &Discovery) -> Self {
        let mut tree = Self::default();
        for test in &discovery.tests {
            let mut parts: Vec<&str> = test.name.split("::").collect();
            let leaf = parts.pop().unwrap_or(test.name.as_str());
            let mut node = &mut tree.root;
            for part in &parts {
                node = node.children.entry((*part).to_owned()).or_default();
            }
            node.tests.push(Leaf {
                name: test.name.clone(),
                module: parts.join("::"),
                ignored: test.ignored,
                parameterized: is_parameterized(leaf),
            });
        }
        tree.sort();
        tree
    }

    fn sort(&mut self) {
        fn walk(node: &mut Node) {
            node.tests.sort_by(|left, right| left.name.cmp(&right.name));
            for child in node.children.values_mut() {
                walk(child);
            }
        }
        walk(&mut self.root);
    }

    /// Every test, in tree order.
    pub fn leaves(&self) -> Vec<&Leaf> {
        fn walk<'a>(node: &'a Node, out: &mut Vec<&'a Leaf>) {
            out.extend(node.tests.iter());
            for child in node.children.values() {
                walk(child, out);
            }
        }
        let mut out = Vec::new();
        walk(&self.root, &mut out);
        out
    }

    /// How many tests the tree holds.
    pub fn len(&self) -> usize {
        self.leaves().len()
    }

    /// Whether the tree is empty.
    pub fn is_empty(&self) -> bool {
        self.root.children.is_empty() && self.root.tests.is_empty()
    }

    /// How many tests discovery marked ignored.
    pub fn ignored_count(&self) -> usize {
        self.leaves().iter().filter(|leaf| leaf.ignored).count()
    }

    /// How many tests carry a parameter marker.
    pub fn parameterized_count(&self) -> usize {
        self.leaves()
            .iter()
            .filter(|leaf| leaf.parameterized)
            .count()
    }

    /// The tests an automatic run may start: every test discovery did not mark ignored.
    pub fn auto_run(&self) -> Vec<String> {
        self.leaves()
            .iter()
            .filter(|leaf| !leaf.ignored)
            .map(|leaf| leaf.name.clone())
            .collect()
    }

    /// The non-ignored tests whose path contains `needle`.
    ///
    /// An empty needle is not a wildcard: it returns the same set as [`Self::auto_run`], because
    /// "match everything" and "run everything" must not be different answers.
    pub fn matching(&self, needle: &str) -> Vec<String> {
        self.leaves()
            .iter()
            .filter(|leaf| !leaf.ignored && leaf.name.contains(needle))
            .map(|leaf| leaf.name.clone())
            .collect()
    }

    /// The non-ignored tests whose recorded result failed.
    ///
    /// A fresh run is asked for by exact path, so a rerun cannot pick up a test whose name merely
    /// contains a failed one's name.
    pub fn failed(&self, tests: &Tests) -> Vec<String> {
        let failed: Vec<&str> = tests
            .results()
            .into_iter()
            .filter(|result| result.status == TestStatus::Failed)
            .map(|result| result.name.as_str())
            .collect();
        self.leaves()
            .iter()
            .filter(|leaf| !leaf.ignored && failed.contains(&leaf.name.as_str()))
            .map(|leaf| leaf.name.clone())
            .collect()
    }

    /// The tree in one line per node.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "test tree: {} test(s) ({} ignored, {} parameterized)",
            self.len(),
            self.ignored_count(),
            self.parameterized_count()
        )];
        render_node(&self.root, 0, &mut lines);
        lines
    }
}

/// Whether a leaf segment carries a `[..]` parameter marker.
fn is_parameterized(leaf: &str) -> bool {
    leaf.contains('[') && leaf.ends_with(']')
}

/// Renders a node's tests and children at an indent.
fn render_node(node: &Node, depth: usize, lines: &mut Vec<String>) {
    let indent = "  ".repeat(depth);
    for leaf in &node.tests {
        let segment = leaf.name.rsplit("::").next().unwrap_or(leaf.name.as_str());
        let mut markers = String::new();
        if leaf.parameterized {
            markers.push_str(" [parameterized]");
        }
        if leaf.ignored {
            markers.push_str(" [ignored]");
        }
        lines.push(format!("{indent}{segment}{markers}"));
    }
    for (name, child) in &node.children {
        lines.push(format!("{indent}{name}::"));
        render_node(child, depth + 1, lines);
    }
}

#[cfg(test)]
mod tests {
    use super::TestTree;
    use crate::app::quality::report::{TestStatus, Tests};
    use crate::app::quality::test_runner::{DiscoveredTest, Discovery};

    fn discovery(tests: &[(&str, bool)]) -> Discovery {
        Discovery {
            tests: tests
                .iter()
                .map(|(name, ignored)| DiscoveredTest {
                    name: (*name).to_owned(),
                    ignored: *ignored,
                })
                .collect(),
            benchmarks: 0,
            truncated: false,
            unparsed: 0,
            incomplete: None,
        }
    }

    fn tree() -> TestTree {
        TestTree::from_discovery(&discovery(&[
            ("app::quality::tests::a_test", false),
            ("app::quality::tests::param[one]", false),
            ("app::quality::tests::param[two]", false),
            ("app::quality::tests::slow", true),
            ("app::preview::tests::other", false),
            ("root_level", false),
        ]))
    }

    #[test]
    fn the_tree_groups_by_module_and_marks_parameters_and_ignored_tests() {
        let tree = tree();
        assert_eq!(tree.len(), 6);
        assert_eq!(tree.ignored_count(), 1);
        assert_eq!(tree.parameterized_count(), 2);
        let lines = tree.render();
        assert!(lines[0].contains("6 test(s)"), "{lines:?}");
        assert!(lines.iter().any(|line| line == "app::"), "{lines:?}");
        assert!(lines.iter().any(|line| line == "  quality::"), "{lines:?}");
        assert!(
            lines
                .iter()
                .any(|line| line == "      param[one] [parameterized]"),
            "{lines:?}"
        );
        assert!(
            lines.iter().any(|line| line == "      slow [ignored]"),
            "{lines:?}"
        );
        assert!(lines.iter().any(|line| line == "root_level"), "{lines:?}");
    }

    #[test]
    fn an_automatic_run_excludes_ignored_tests() {
        let names = tree().auto_run();
        assert!(!names.iter().any(|name| name.ends_with("slow")));
        assert!(names.iter().any(|name| name.ends_with("a_test")));
        assert_eq!(names.len(), 5);
    }

    #[test]
    fn a_filter_is_a_substring_of_the_path_and_never_widens_to_ignored_tests() {
        let tree = tree();
        assert_eq!(
            tree.matching("preview"),
            vec!["app::preview::tests::other".to_owned()]
        );
        assert!(tree.matching("slow").is_empty(), "ignored is not selected");
        assert_eq!(tree.matching("").len(), 5);
    }

    #[test]
    fn an_empty_tree_runs_nothing_and_says_so() {
        let tree = TestTree::from_discovery(&discovery(&[]));
        assert!(tree.is_empty());
        assert!(tree.auto_run().is_empty());
        assert!(tree.render()[0].contains("0 test(s)"));
    }

    #[test]
    fn a_targeted_rerun_names_only_the_failed_tests() {
        let mut tests = Tests::new();
        tests.record("app::quality::tests::a_test", TestStatus::Failed, None);
        tests.record("app::quality::tests::other", TestStatus::Passed, None);
        tests.record("app::quality::tests::slow", TestStatus::Failed, None);
        let names = tree().failed(&tests);
        assert_eq!(names, vec!["app::quality::tests::a_test".to_owned()]);
        // The ignored test failed in an earlier explicit run, but an automatic rerun must not pick it.
        assert!(!names.iter().any(|name| name.ends_with("slow")));
    }
}
