// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The extension manager's listing and its project-dependency check (roadmap task T22.05).
//!
//! The registry knows what is installed; a project knows what it uses. The acceptance line is
//! *"a removal first shows the project's dependencies, and leaves no dangling entry point"* — so the
//! two facts meet here and nowhere else:
//!
//! * **One row per installed extension.** [`Panel::render`] lists id, version, licence, state,
//!   pin, source, what the extension provides and how much of it this project uses.
//! * **The project is the authority on use.** [`Usage`] is the caller's report of the names the open
//!   project references. This module never inspects a document and never guesses that an installed
//!   extension is in use.
//! * **Dependencies are the intersection.** A [`Dependency`] names the extension, its version and
//!   exactly which of the names the project uses it provides. A name the extension does not declare
//!   is not attributable to it.
//! * **Removal is refused, not warned.** [`Removal::allowed`] is false while any dependency exists;
//!   a warning that still removes would leave the dangling entry point this task forbids.

use std::collections::BTreeSet;

use crate::app::extension::package::{Manifest, Registry};

/// The names the open project references, as its own report.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Usage {
    names: BTreeSet<String>,
}

impl Usage {
    /// An empty report: the project references nothing an extension could provide.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records one used name; a repeated name is the same use.
    pub fn declare(&mut self, name: &str) {
        self.names.insert(name.to_owned());
    }

    /// The used names, in order.
    pub fn names(&self) -> Vec<String> {
        self.names.iter().cloned().collect()
    }

    /// How many names the project reports.
    pub fn len(&self) -> usize {
        self.names.len()
    }

    /// Whether the report is empty.
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

/// One extension's hold on the open project.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Dependency {
    /// The extension.
    pub extension: String,
    /// The installed version, as written.
    pub version: String,
    /// The names the project uses that this extension provides.
    pub uses: Vec<String>,
}

impl Dependency {
    /// The dependency in one line.
    pub fn render(&self) -> String {
        format!(
            "{} {} provides {} project use(s): {}",
            self.extension,
            self.version,
            self.uses.len(),
            self.uses.join(", ")
        )
    }
}

/// What removing one extension would do to the open project.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Removal {
    /// The extension.
    pub id: String,
    /// The installed version, as written.
    pub version: String,
    /// The project's uses this extension provides.
    pub dependencies: Vec<Dependency>,
}

impl Removal {
    /// Whether the removal leaves no dangling entry point.
    pub fn allowed(&self) -> bool {
        self.dependencies.is_empty()
    }

    /// The dangling entry points, when there are any.
    pub fn dangling(&self) -> usize {
        self.dependencies
            .iter()
            .map(|dependency| dependency.uses.len())
            .sum()
    }

    /// The report a person reads before choosing: dependencies first, then the verdict.
    pub fn render(&self) -> Vec<String> {
        let mut lines: Vec<String> = self
            .dependencies
            .iter()
            .map(|dependency| format!("depends: {}", dependency.render()))
            .collect();
        if self.allowed() {
            lines.push(format!(
                "remove {} {}: no project dependency",
                self.id, self.version
            ));
        } else {
            lines.push(format!(
                "remove {} {}: refused, the project would leave {} dangling entry point(s)",
                self.id,
                self.version,
                self.dangling()
            ));
        }
        lines
    }
}

/// The manager's view of the registry plus one project's usage.
#[derive(Clone, Copy, Debug)]
pub struct Panel<'a> {
    registry: &'a Registry,
    usage: &'a Usage,
}

impl<'a> Panel<'a> {
    /// The panel over `registry` for `usage`.
    pub fn new(registry: &'a Registry, usage: &'a Usage) -> Self {
        Self { registry, usage }
    }

    /// The names this extension provides that the project uses.
    fn uses_of(&self, manifest: &Manifest) -> Vec<String> {
        manifest
            .provides
            .iter()
            .filter(|name| self.usage.names.contains(*name))
            .cloned()
            .collect()
    }

    /// The dependency on one installed extension, or `None` when the project does not use any of
    /// what it provides.
    pub fn dependency(&self, id: &str) -> Option<Dependency> {
        let manifest = self.registry.manifest(id)?;
        let uses = self.uses_of(manifest);
        if uses.is_empty() {
            return None;
        }
        Some(Dependency {
            extension: id.to_owned(),
            version: manifest.version.render(),
            uses,
        })
    }

    /// Every installed extension the project depends on, in id order.
    pub fn dependents(&self) -> Vec<Dependency> {
        self.registry
            .ids()
            .into_iter()
            .filter_map(|id| self.dependency(&id))
            .collect()
    }

    /// One row per installed extension.
    ///
    /// A row says what the extension is and what the project uses of it; nothing is summarised away,
    /// because the row is the panel.
    pub fn render(&self) -> Vec<String> {
        self.registry
            .ids()
            .into_iter()
            .map(|id| {
                let manifest = self.registry.manifest(&id).expect("the id was listed");
                let mut state = if self.registry.is_enabled(&id) {
                    "enabled".to_owned()
                } else {
                    "disabled".to_owned()
                };
                if self.registry.is_pinned(&id) {
                    state.push_str(",pinned");
                }
                let used = self.uses_of(manifest).len();
                format!(
                    "{} {} {} [{}] provides {} ({} project use(s)) from {}",
                    id,
                    manifest.version.render(),
                    manifest.license,
                    state,
                    manifest.provides.len(),
                    used,
                    self.registry.source(&id).unwrap_or("(unknown)"),
                )
            })
            .collect()
    }

    /// What removing `id` would do, or the reason it cannot be considered.
    pub fn removal(&self, id: &str) -> Result<Removal, String> {
        let manifest = self
            .registry
            .manifest(id)
            .ok_or_else(|| format!("no extension {id:?} is installed"))?;
        let version = manifest.version.render();
        let dependencies = self.dependency(id).into_iter().collect();
        Ok(Removal {
            id: id.to_owned(),
            version,
            dependencies,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{Panel, Usage};
    use crate::app::extension::package::{Host, Manifest, Package, Registry, Version};

    fn host() -> Host {
        Host {
            version: Version::parse("0.1.0").expect("a version"),
            capabilities: ["widget", "template"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
            permissions: ["fs", "network"].into_iter().map(str::to_owned).collect(),
        }
    }

    fn package(id: &str, provides: &[&str]) -> Package {
        let manifest = Manifest {
            id: id.to_owned(),
            version: Version::parse("1.2.0").expect("a version"),
            host_min: Version::parse("0.1.0").expect("a version"),
            host_max: Version::parse("0.2.0").expect("a version"),
            capabilities: vec!["widget".to_owned()],
            permissions: Vec::new(),
            provides: provides.iter().map(|name| (*name).to_owned()).collect(),
            license: "MIT".to_owned(),
            digest: String::new(),
        };
        let files = [("main.rs".to_owned(), b"fn main() {}".to_vec())]
            .into_iter()
            .collect::<BTreeMap<_, _>>();
        let mut package = Package::new(manifest, files);
        package.manifest.digest = package.digest();
        package
    }

    fn registry() -> Registry {
        let mut registry = Registry::new();
        registry
            .install(
                package("acme.chart", &["acme.chart", "acme.spark"]),
                &host(),
            )
            .expect("a chart package");
        registry
            .install(package("acme.form", &["acme.form"]), &host())
            .expect("a form package");
        registry
    }

    #[test]
    fn a_project_dependency_names_the_extension_and_the_exact_uses() {
        let registry = registry();
        let mut usage = Usage::new();
        usage.declare("acme.chart");
        usage.declare("acme.chart");
        usage.declare("acme.spark");
        usage.declare("button");
        let panel = Panel::new(&registry, &usage);

        let dependencies = panel.dependents();
        assert_eq!(dependencies.len(), 1, "{dependencies:?}");
        assert_eq!(dependencies[0].extension, "acme.chart");
        assert_eq!(dependencies[0].uses, vec!["acme.chart", "acme.spark"]);
        // A name the extension does not provide is not attributed to it.
        assert!(!dependencies[0].uses.contains(&"button".to_owned()));
        assert!(dependencies[0].render().contains("2 project use(s)"));
    }

    #[test]
    fn removal_is_refused_while_the_project_uses_the_extension() {
        let registry = registry();
        let mut usage = Usage::new();
        usage.declare("acme.form");
        let panel = Panel::new(&registry, &usage);

        let removal = panel.removal("acme.form").expect("an installed extension");
        assert!(!removal.allowed());
        assert_eq!(removal.dangling(), 1);
        let report = removal.render().join("\n");
        assert!(report.contains("depends: acme.form 1.2.0"), "{report}");
        assert!(report.contains("refused"), "{report}");
    }

    #[test]
    fn removal_is_allowed_when_nothing_in_the_project_names_it() {
        let registry = registry();
        let usage = Usage::new();
        let panel = Panel::new(&registry, &usage);

        let removal = panel.removal("acme.chart").expect("an installed extension");
        assert!(removal.allowed());
        assert_eq!(removal.dangling(), 0);
        let report = removal.render().join("\n");
        assert!(report.contains("no project dependency"), "{report}");
        assert!(!report.contains("depends:"), "{report}");
    }

    #[test]
    fn removing_an_unknown_extension_is_refused_rather_than_reported_clean() {
        let registry = registry();
        let usage = Usage::new();
        let panel = Panel::new(&registry, &usage);
        let error = panel.removal("ghost").expect_err("no such extension");
        assert!(error.contains("ghost"), "{error}");
    }

    #[test]
    fn the_panel_row_shows_the_state_the_pin_and_the_use_count() {
        let mut registry = registry();
        registry.disable("acme.form").expect("a disable");
        registry.pin("acme.chart").expect("a pin");
        let mut usage = Usage::new();
        usage.declare("acme.form");
        let panel = Panel::new(&registry, &usage);

        let rows = panel.render();
        assert_eq!(rows.len(), 2, "{rows:?}");
        // Ids are listed in order, so the chart row is first.
        assert!(
            rows[0].contains("acme.chart 1.2.0 MIT [enabled,pinned]"),
            "{rows:?}"
        );
        assert!(
            rows[0].contains("provides 2 (0 project use(s))"),
            "{rows:?}"
        );
        assert!(rows[1].contains("[disabled]"), "{rows:?}");
        assert!(rows[1].contains("(1 project use(s))"), "{rows:?}");
    }
}
