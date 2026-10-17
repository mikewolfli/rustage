// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Package import plans: path traversal, dependency cycles and overwrites are refused, and a
//! cancelled plan has applied nothing (task T12.07).
//!
//! The acceptance line is *"refuse package path traversal / cycles / overwrite; cancellation has no
//! side effect"*. The design keeps that true by splitting planning from committing:
//!
//! * **A refusal produces no plan.** [`import_plan`] validates every entry and the whole dependency
//!   graph before it builds a [`Plan`], so a refused import cannot leave a half-built plan behind:
//!   the `Result` is `Err` with a named [`Refusal`], not a plan with an error written into it.
//! * **A plan is pending writes until it commits.** [`Plan::commit`] moves the writes into the
//!   applied set; [`Plan::cancel`] clears them. A plan cancelled before commit answers an empty
//!   applied set, so cancellation is the absence of an effect rather than a rollback that has to be
//!   trusted.
//! * **Three refusals stay three refusals.** An entry whose path escapes the root, a package that
//!   depends on itself, and an entry that would overwrite an existing one are [`Refusal`] variants
//!   with different text, because "your path is unsafe" and "your name is taken" are not the same
//!   fix.
//!
//! The plan is modelled in memory only; nothing here touches the filesystem, so a test can assert
//! what a cancellation applied without cleaning up after it.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

/// One entry a package contributes, at a project-relative logical path.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Entry {
    /// The entry's logical path.
    pub path: String,
    /// The entry's version.
    pub version: u32,
    /// The entry's license note.
    pub license: String,
}

/// A package's manifest.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Package {
    /// The package's name.
    pub name: String,
    /// The package's version.
    pub version: u32,
    /// The package's license note.
    pub license: String,
    /// The packages this one depends on.
    pub depends_on: Vec<String>,
    /// The entries it contributes.
    pub entries: Vec<Entry>,
}

impl Package {
    /// Builds a package, or `None` when the name is blank.
    pub fn new(name: &str, version: u32, license: &str) -> Option<Self> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return None;
        }
        Some(Self {
            name: trimmed.to_owned(),
            version,
            license: license.to_owned(),
            depends_on: Vec::new(),
            entries: Vec::new(),
        })
    }
}

/// Why an import was refused.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Refusal {
    /// An entry's path escapes the project root.
    EscapesRoot {
        /// The offending path.
        path: String,
    },
    /// The package depends on itself, directly or transitively.
    Cycle {
        /// The dependency path that closes the cycle, first name repeated last.
        path: Vec<String>,
    },
    /// An entry would overwrite an existing one.
    Overwrite {
        /// The offending path.
        path: String,
    },
}

impl Refusal {
    /// The refusal in one line.
    pub fn render(&self) -> String {
        match self {
            Self::EscapesRoot { path } => {
                format!("entry path {path:?} escapes the project root")
            }
            Self::Cycle { path } => {
                format!("package dependency cycle: {}", path.join(" -> "))
            }
            Self::Overwrite { path } => {
                format!("entry path {path:?} would overwrite an existing entry")
            }
        }
    }
}

/// A pending import: the writes it would make and the writes it has applied.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Plan {
    writes: Vec<Entry>,
    applied: Vec<Entry>,
}

impl Plan {
    /// The writes still pending.
    pub fn writes(&self) -> &[Entry] {
        &self.writes
    }

    /// The writes that have been applied.
    pub fn applied(&self) -> &[Entry] {
        &self.applied
    }

    /// Applies the pending writes.
    pub fn commit(&mut self) {
        self.applied.append(&mut self.writes);
    }

    /// Drops the pending writes. A plan cancelled here has applied nothing.
    pub fn cancel(&mut self) {
        self.writes.clear();
    }
}

/// Plans an import, refusing a path that escapes the root, a dependency cycle and an overwrite.
///
/// `registry` is the dependency graph of the packages already present; `existing` is the set of
/// logical paths the project already holds. Nothing is validated after a plan is built, so a plan
/// that exists is a plan that would apply.
pub fn import_plan(
    package: &Package,
    registry: &BTreeMap<String, Vec<String>>,
    existing: &BTreeSet<String>,
) -> Result<Plan, Refusal> {
    let mut edges = registry.clone();
    edges.insert(package.name.clone(), package.depends_on.clone());
    if let Some(path) = find_cycle(&package.name, &edges) {
        return Err(Refusal::Cycle { path });
    }
    for entry in &package.entries {
        if escapes(&entry.path) {
            return Err(Refusal::EscapesRoot {
                path: entry.path.clone(),
            });
        }
        if existing.contains(&entry.path) {
            return Err(Refusal::Overwrite {
                path: entry.path.clone(),
            });
        }
    }
    Ok(Plan {
        writes: package.entries.clone(),
        applied: Vec::new(),
    })
}

/// Whether a path escapes the project root.
fn escapes(path: &str) -> bool {
    let trimmed = path.trim();
    trimmed.is_empty()
        || trimmed.starts_with('/')
        || trimmed.contains('\\')
        || trimmed.split('/').any(|segment| segment == "..")
}

/// Finds a cycle reachable from `start`, returning the path with `start` at both ends.
fn find_cycle(start: &str, edges: &BTreeMap<String, Vec<String>>) -> Option<Vec<String>> {
    fn walk(
        node: &str,
        edges: &BTreeMap<String, Vec<String>>,
        path: &mut Vec<String>,
    ) -> Option<Vec<String>> {
        if let Some(index) = path.iter().position(|seen| seen == node) {
            let mut cycle = path[index..].to_vec();
            cycle.push(node.to_owned());
            return Some(cycle);
        }
        path.push(node.to_owned());
        if let Some(children) = edges.get(node) {
            for child in children {
                if let Some(cycle) = walk(child, edges, path) {
                    return Some(cycle);
                }
            }
        }
        path.pop();
        None
    }
    walk(start, edges, &mut Vec::new())
}

#[cfg(test)]
mod tests {
    use super::{Entry, Package, Refusal, import_plan};
    use std::collections::{BTreeMap, BTreeSet};

    fn package(paths: &[&str]) -> Package {
        let mut package = Package::new("icons", 1, "MIT").expect("a package");
        for path in paths {
            package.entries.push(Entry {
                path: path.to_string(),
                version: 1,
                license: "MIT".to_owned(),
            });
        }
        package
    }

    #[test]
    fn an_entry_that_escapes_the_root_is_refused() {
        let refusal = import_plan(&package(&["../secret"]), &BTreeMap::new(), &BTreeSet::new())
            .expect_err("an escape");
        assert_eq!(
            refusal,
            Refusal::EscapesRoot {
                path: "../secret".to_owned()
            }
        );
        assert!(refusal.render().contains("escapes the project root"));
    }

    #[test]
    fn an_absolute_entry_path_is_refused() {
        let refusal = import_plan(
            &package(&["/etc/passwd"]),
            &BTreeMap::new(),
            &BTreeSet::new(),
        )
        .expect_err("an absolute path");
        assert!(matches!(refusal, Refusal::EscapesRoot { .. }));
    }

    #[test]
    fn a_package_that_depends_on_itself_is_refused() {
        let mut package = package(&["icons/a.svg"]);
        package.depends_on.push("icons".to_owned());
        let refusal =
            import_plan(&package, &BTreeMap::new(), &BTreeSet::new()).expect_err("a self cycle");
        assert_eq!(
            refusal,
            Refusal::Cycle {
                path: vec!["icons".to_owned(), "icons".to_owned()]
            }
        );
    }

    #[test]
    fn a_transitive_dependency_cycle_is_refused() {
        let mut registry = BTreeMap::new();
        registry.insert("base".to_owned(), vec!["icons".to_owned()]);
        let mut package = package(&["icons/a.svg"]);
        package.depends_on.push("base".to_owned());
        let refusal = import_plan(&package, &registry, &BTreeSet::new()).expect_err("a cycle");
        assert!(
            refusal.render().contains("icons -> base -> icons"),
            "{refusal:?}"
        );
    }

    #[test]
    fn overwriting_an_existing_entry_is_refused() {
        let mut existing = BTreeSet::new();
        existing.insert("icons/a.svg".to_owned());
        let refusal = import_plan(&package(&["icons/a.svg"]), &BTreeMap::new(), &existing)
            .expect_err("an overwrite");
        assert_eq!(
            refusal,
            Refusal::Overwrite {
                path: "icons/a.svg".to_owned()
            }
        );
    }

    #[test]
    fn a_cancelled_plan_has_applied_nothing() {
        let mut plan = import_plan(
            &package(&["icons/a.svg", "icons/b.svg"]),
            &BTreeMap::new(),
            &BTreeSet::new(),
        )
        .expect("a plan");
        assert_eq!(plan.writes().len(), 2);
        plan.cancel();
        assert!(plan.applied().is_empty());
        assert!(plan.writes().is_empty());
    }

    #[test]
    fn a_committed_plan_applies_its_writes() {
        let mut plan = import_plan(
            &package(&["icons/a.svg"]),
            &BTreeMap::new(),
            &BTreeSet::new(),
        )
        .expect("a plan");
        plan.commit();
        assert_eq!(plan.applied().len(), 1);
        assert!(plan.writes().is_empty());
    }

    #[test]
    fn a_blank_package_name_is_not_a_package() {
        assert!(Package::new("  ", 1, "MIT").is_none());
    }
}
