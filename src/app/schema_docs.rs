// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Schema documentation for Rust, JSON and CSS, and completion relative to a target (task T07.07).
//!
//! The acceptance line is *"completion does not recommend an item the current target forbids"*. A
//! target is what the project is being built for, so an item that needs a capability the target does
//! not have must be **absent from the list**, not merely dimmed:
//!
//! * **An entry says what it needs.** [`Entry::requires`] names the capability an item depends on
//!   (or `None` for something always available), and the catalog's `requires` field is the only place
//!   that is written.
//! * **Completion filters by the target.** [`Catalog::completions`] returns only the entries whose
//!   required capability the [`Target`] allows, so a forbidden item cannot be picked.
//! * **What was filtered is still nameable.** [`Catalog::filtered`] lists the entries the target
//!   removed, and [`Target::forbids`] names the capability, so "why is it not offered" has an answer
//!   rather than being an invisible hole.
//! * **The language vocabulary is the one from [T06.02](crate::app::highlight).** [`Language`] is the
//!   same `Plain`/`Rust`/`Json`/`Css` set, not a second one.
//!
//! The documentation text is data; where it is shown (hover, a docs panel) is the designer's subject.

#![allow(dead_code)]

use std::collections::BTreeSet;

use crate::app::features::CompletionItem;
use crate::app::highlight::Language;

/// What a project is being built for, and the capabilities it therefore has.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Target {
    name: String,
    available: BTreeSet<String>,
}

impl Target {
    /// A target with a set of available capability names.
    pub fn new(name: impl Into<String>, available: impl IntoIterator<Item = String>) -> Self {
        Self {
            name: name.into(),
            available: available.into_iter().collect(),
        }
    }

    /// The target's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether the target has a capability.
    pub fn allows(&self, capability: &str) -> bool {
        self.available.contains(capability)
    }

    /// Whether the target forbids a capability.
    pub fn forbids(&self, capability: &str) -> bool {
        !self.allows(capability)
    }

    /// The target in one line.
    pub fn render(&self) -> String {
        let names: Vec<&str> = self.available.iter().map(String::as_str).collect();
        format!("target {}: {}", self.name, names.join(", "))
    }
}

/// One documentation entry: a key, what it needs, and what it means.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Entry {
    /// The key (a property name, a JSON key, an item name).
    pub key: String,
    /// A short detail beside the key.
    pub detail: String,
    /// The capability it needs, or `None` for something always available.
    pub requires: Option<String>,
    /// The documentation.
    pub documentation: String,
}

impl Entry {
    /// An entry that is always available.
    pub fn new(
        key: impl Into<String>,
        detail: impl Into<String>,
        documentation: impl Into<String>,
    ) -> Self {
        Self {
            key: key.into(),
            detail: detail.into(),
            requires: None,
            documentation: documentation.into(),
        }
    }

    /// The same entry, requiring a capability.
    pub fn requiring(mut self, capability: impl Into<String>) -> Self {
        self.requires = Some(capability.into());
        self
    }

    /// The entry in one line.
    pub fn render(&self) -> String {
        match &self.requires {
            Some(capability) => format!(
                "{} ({detail}) needs {capability}",
                self.key,
                detail = self.detail
            ),
            None => format!("{} ({})", self.key, self.detail),
        }
    }
}

/// The documentation for one language.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Catalog {
    language: Language,
    entries: Vec<Entry>,
}

impl Catalog {
    /// An empty catalog for a language.
    pub fn new(language: Language) -> Self {
        Self {
            language,
            entries: Vec::new(),
        }
    }

    /// The language.
    pub fn language(&self) -> Language {
        self.language
    }

    /// Changes the language, keeping the entries.
    pub fn set_language(&mut self, language: Language) {
        self.language = language;
    }

    /// Adds an entry.
    pub fn add(&mut self, entry: Entry) {
        self.entries.push(entry);
    }

    /// How many entries there are.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether it is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The entry for a key.
    pub fn documentation(&self, key: &str) -> Option<&Entry> {
        self.entries.iter().find(|entry| entry.key == key)
    }

    /// The completions for a prefix that the target allows.
    pub fn completions(&self, prefix: &str, target: &Target) -> Vec<CompletionItem> {
        self.entries
            .iter()
            .filter(|entry| entry.key.starts_with(prefix))
            .filter(|entry| entry_allowed(entry, target))
            .map(|entry| CompletionItem {
                label: entry.key.clone(),
                insert_text: entry.key.clone(),
                kind: crate::app::features::ItemKind::Text,
                detail: Some(entry.detail.clone()),
                documentation: Some(entry.documentation.clone()),
            })
            .collect()
    }

    /// The entries a prefix would have offered but the target removed.
    pub fn filtered(&self, prefix: &str, target: &Target) -> Vec<&Entry> {
        self.entries
            .iter()
            .filter(|entry| entry.key.starts_with(prefix))
            .filter(|entry| !entry_allowed(entry, target))
            .collect()
    }

    /// The catalog in one report, for a target.
    pub fn render(&self, prefix: &str, target: &Target) -> Vec<String> {
        let mut lines = vec![format!(
            "{} catalog: {} entries (target {})",
            self.language.name(),
            self.entries.len(),
            target.name()
        )];
        for entry in self
            .entries
            .iter()
            .filter(|entry| entry.key.starts_with(prefix))
        {
            let state = if entry_allowed(entry, target) {
                "offered"
            } else {
                "forbidden"
            };
            lines.push(format!("  {} {state}", entry.render()));
        }
        lines
    }
}

/// Whether an entry's requirement is satisfied by the target.
fn entry_allowed(entry: &Entry, target: &Target) -> bool {
    match &entry.requires {
        Some(capability) => target.allows(capability),
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::{Catalog, Entry, Target};
    use crate::app::highlight::Language;

    fn css() -> Catalog {
        let mut catalog = Catalog::new(Language::Css);
        catalog.add(Entry::new("color", "text colour", "sets the text colour"));
        catalog.add(
            Entry::new("display", "box model", "sets the display type").requiring("layout-grid"),
        );
        catalog.add(
            Entry::new("grid-template-columns", "grid", "the grid columns")
                .requiring("layout-grid"),
        );
        catalog
    }

    #[test]
    fn completion_does_not_offer_what_the_target_forbids() {
        let catalog = css();
        let desktop = Target::new("desktop", ["layout-grid".to_owned()]);
        let embedded = Target::new("embedded", Vec::new());
        // On the desktop target the grid property is offered.
        let offered: Vec<String> = catalog
            .completions("g", &desktop)
            .iter()
            .map(|item| item.label.clone())
            .collect();
        assert_eq!(offered, vec!["grid-template-columns".to_owned()]);
        // On the embedded target it is gone entirely.
        assert!(catalog.completions("g", &embedded).is_empty());
        // And the reason is nameable.
        let removed = catalog.filtered("g", &embedded);
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].key, "grid-template-columns");
        assert!(embedded.forbids("layout-grid"));
    }

    #[test]
    fn an_entry_without_a_requirement_is_always_offered() {
        let catalog = css();
        let embedded = Target::new("embedded", Vec::new());
        let offered: Vec<String> = catalog
            .completions("c", &embedded)
            .iter()
            .map(|item| item.label.clone())
            .collect();
        assert_eq!(offered, vec!["color".to_owned()]);
    }

    #[test]
    fn documentation_is_reachable_by_key() {
        let catalog = css();
        let entry = catalog.documentation("color").expect("an entry");
        assert_eq!(entry.detail, "text colour");
        assert!(entry.documentation.contains("text colour"));
        assert!(catalog.documentation("nowhere").is_none());
    }

    #[test]
    fn a_targets_capabilities_are_queryable() {
        let target = Target::new("desktop", ["layout-grid".to_owned(), "web".to_owned()]);
        assert!(target.allows("web"));
        assert!(target.forbids("embedded"));
        assert!(target.render().contains("layout-grid"));
    }

    #[test]
    fn the_report_names_each_entry_as_offered_or_forbidden() {
        let catalog = css();
        let embedded = Target::new("embedded", Vec::new());
        let report = catalog.render("g", &embedded);
        assert!(report[0].contains("css catalog"));
        assert!(report.iter().any(|line| line.contains("forbidden")));
    }
}
