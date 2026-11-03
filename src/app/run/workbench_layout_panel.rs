// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The layout panel's rows (roadmap F10, `panel.layout`).
//!
//! The panel reads the page document's own container declaration (page format v6, the root node's
//! `layout` object) and reports it in the library's words: the same declaration is copied verbatim
//! into the document generation consumes, so what the panel shows is what is generated — not a
//! second layout model.
//!
//! Diagnostics are the generator's: [`rust_widgets::json::LayoutIssue`] arrives in
//! `GenerationReport::layout_issues`, so a parameter this build does not honour is named by the
//! library's own schema rather than by a copy of it here.
//!
//! **Boundary.** Page format v6 declares the container kind and its parameters only. Per-node size
//! constraints (`min`/`max`/`preferred`), per-node margins and overflow have no spelling in the
//! library's declarative document yet (upstream RW-109), and the panel says so instead of showing
//! numbers nothing would consume.

use crate::project::record::page::PageLayout;
use rust_widgets::json::LayoutIssue;

/// What the page's root node declares about its container.
pub(super) struct Declaration<'a> {
    /// The root node's identity.
    pub node: u64,
    /// Its control kind, for the row that names it.
    pub kind: &'a str,
    /// The declaration, when the page carries one.
    pub layout: Option<&'a PageLayout>,
    /// How many children the container places.
    pub children: usize,
}

/// The panel's rows.
///
/// `issues` is `None` when no generation run was available: "not checked" is a different answer
/// from "checked, none found", and the two must not read the same.
pub(super) fn rows(
    source: &str,
    declaration: &Declaration<'_>,
    issues: Option<&[LayoutIssue]>,
) -> Vec<String> {
    let mut rows = vec![format!("page: {source}")];
    rows.push(format!(
        "container node {} [{}]: {} child node(s)",
        declaration.node, declaration.kind, declaration.children
    ));
    match declaration.layout {
        Some(layout) => {
            rows.push(format!("declaration: {}", layout.render()));
            rows.push(
                "parameters: as the library's layout schema reads them (one definition, upstream)"
                    .to_owned(),
            );
        }
        None => {
            rows.push(
                "declaration: none; the page does not say how its children are placed".to_owned(),
            );
            rows.push(
                "note: without a declaration the library places every child at the container's \
                 rect; the panel does not invent a placement"
                    .to_owned(),
            );
        }
    }
    rows.push(
        "generation: the stripped product carries the solved coordinates; the default product's \
         Node tree carries none (upstream RW-111)"
            .to_owned(),
    );
    rows.push(
        "not declared by this format: per-node min/max/preferred size, per-node margin, overflow \
         (upstream RW-109)"
            .to_owned(),
    );
    match issues {
        None => rows.push("diagnostics: not checked (no generation run is available)".to_owned()),
        Some([]) => rows.push("diagnostics: none".to_owned()),
        Some(issues) => {
            rows.push(format!("diagnostics: {}", issues.len()));
            for issue in issues {
                rows.push(format!("  {}", issue.describe()));
            }
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::{Declaration, rows};
    use crate::project::record::json::{Json, Number};
    use crate::project::record::page::PageLayout;
    use rust_widgets::json::{LayoutDiagnostic, LayoutIssue};

    fn declaration(layout: Option<&PageLayout>) -> Declaration<'_> {
        Declaration {
            node: 1,
            kind: "window",
            layout,
            children: 2,
        }
    }

    #[test]
    fn a_page_without_a_declaration_says_what_that_means() {
        let rows = rows("pages/main.json", &declaration(None), Some(&[]));
        assert_eq!(rows[0], "page: pages/main.json");
        assert!(
            rows.iter()
                .any(|row| row.starts_with("container node 1 [window]"))
        );
        assert!(rows.iter().any(|row| row.contains("declaration: none")));
        assert!(
            rows.iter()
                .any(|row| row.contains("does not invent a placement"))
        );
        assert!(rows.iter().any(|row| row.contains("RW-109")));
        assert!(rows.iter().any(|row| row.contains("RW-111")));
        assert!(rows.iter().any(|row| row == "diagnostics: none"));
    }

    #[test]
    fn a_declaration_is_reported_in_the_library_s_words() {
        let layout = PageLayout::new(
            "vbox",
            [
                ("spacing".to_owned(), Json::Number(Number::from_i64(8))),
                ("margin".to_owned(), Json::Number(Number::from_i64(4))),
            ],
        );
        let rows = rows("pages/main.json", &declaration(Some(&layout)), Some(&[]));
        assert!(
            rows.iter()
                .any(|row| row == "declaration: type=vbox, spacing=8, margin=4"),
            "{rows:?}"
        );
        assert!(
            rows.iter()
                .any(|row| row.contains("one definition, upstream"))
        );
    }

    #[test]
    fn the_generators_diagnostics_are_named_one_per_row() {
        let layout = PageLayout::new(
            "vbox",
            [("gap".to_owned(), Json::Number(Number::from_i64(8)))],
        );
        let issues = [
            LayoutIssue {
                path: Vec::new(),
                diagnostic: LayoutDiagnostic::UnknownField {
                    key: "gap".to_owned(),
                },
            },
            LayoutIssue {
                path: Vec::new(),
                diagnostic: LayoutDiagnostic::UnknownType {
                    declared: "cascade".to_owned(),
                },
            },
        ];
        let rows = rows(
            "pages/main.json",
            &declaration(Some(&layout)),
            Some(&issues),
        );
        assert!(rows.iter().any(|row| row == "diagnostics: 2"), "{rows:?}");
        assert!(
            rows.iter()
                .any(|row| row.contains("unknown layout field `gap`")),
            "{rows:?}"
        );
        assert!(
            rows.iter()
                .any(|row| row.contains("unknown layout type `cascade`")),
            "{rows:?}"
        );
    }

    #[test]
    fn an_unchecked_page_is_not_reported_as_clean() {
        let rows = rows("pages/main.json", &declaration(None), None);
        assert!(
            rows.iter()
                .any(|row| row.contains("diagnostics: not checked")),
            "{rows:?}"
        );
        assert!(!rows.iter().any(|row| row == "diagnostics: none"));
    }
}
