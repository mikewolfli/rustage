// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The layout panel's rows (roadmap F10, `panel.layout`).
//!
//! The panel reads the page document's own container declaration (page format v6, the root node's
//! `layout` object) and its per-node constraints (page format v7) and reports them in the library's
//! words: the same declarations are copied verbatim into the document generation consumes, so what
//! the panel shows is what is generated — not a second layout model.
//!
//! Diagnostics are the generator's: [`rust_widgets::json::LayoutIssue`] arrives in
//! `GenerationReport::layout_issues`, so a parameter this build does not honour is named by the
//! library's own schema rather than by a copy of it here.
//!
//! **Geometry** is the generator's too, since 4.0.1: `GenerationReport::node_rects` (RW-116) carries
//! the solved rectangle of every node, keyed by the document's `id`. The panel shows those numbers
//! instead of re-solving the document — the canvas, the hit test and the generated source therefore
//! read one answer. `GenerationReport::layout_geometry_gaps` (RW-111) names what a product does not
//! carry, so the panel can say "the default product writes no coordinates" without pretending that
//! is a failure of the solve.
//!
//! Per-node size constraints and margins are page-format v7 members (upstream RW-109, released in
//! 4.0.1); the panel reports them from the document, not from a second table.

use crate::project::record::page::{PageConstraints, PageLayout};
use rust_widgets::designer::{GenerationGap, NodeRect};
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
    /// Every node's constraints, in node order (format 7).
    pub constraints: Vec<(u64, &'a PageConstraints)>,
}

/// The solve a generation run published, when one ran.
///
/// `None` (no run) is deliberately a different value from an empty `rects` list: "not solved" and
/// "solved to nothing" must not read the same (principle #22).
pub(super) struct Geometry<'a> {
    /// The client width the layout was solved against.
    pub width: u32,
    /// The client height.
    pub height: u32,
    /// Every node's solved rectangle, in pre-order, from `GenerationReport::node_rects`.
    pub rects: &'a [NodeRect],
    /// What the product does not carry, from `GenerationReport::layout_geometry_gaps`.
    pub gaps: &'a [GenerationGap],
}

/// The panel's rows.
///
/// `issues` is `None` when no generation run was available: "not checked" is a different answer
/// from "checked, none found", and the two must not read the same.
pub(super) fn rows(
    source: &str,
    declaration: &Declaration<'_>,
    issues: Option<&[LayoutIssue]>,
    geometry: Option<Geometry<'_>>,
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
    push_geometry(&mut rows, geometry);
    push_constraints(&mut rows, &declaration.constraints);
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

/// The solved rectangles and what the product does not carry.
fn push_geometry(rows: &mut Vec<String>, geometry: Option<Geometry<'_>>) {
    let Some(geometry) = geometry else {
        rows.push("geometry: not solved (no generation run is available)".to_owned());
        return;
    };
    rows.push(format!(
        "geometry: solved against {}x{} for {} node(s)",
        geometry.width,
        geometry.height,
        geometry.rects.len()
    ));
    for rect in geometry.rects {
        rows.push(format!(
            "  node {} (path {:?}): x={} y={} {}x{}",
            rect.id.as_deref().unwrap_or("-"),
            rect.path,
            rect.x,
            rect.y,
            rect.width,
            rect.height
        ));
    }
    for gap in geometry.gaps {
        rows.push(format!(
            "not carried by this product: {} path {:?}: {}",
            gap.widget, gap.path, gap.reason
        ));
    }
}

/// The per-node constraints the page declares, if any.
fn push_constraints(rows: &mut Vec<String>, constraints: &[(u64, &PageConstraints)]) {
    if constraints.is_empty() {
        rows.push("constraints: no node declares size, margin or overflow".to_owned());
        return;
    }
    rows.push(format!("constraints: {} node(s)", constraints.len()));
    for (id, declaration) in constraints {
        rows.push(format!("  node {id}: {}", declaration.render()));
    }
    rows.push(
        "constraint checks: the library names a member this layout does not honour under \
         diagnostics (for example `overflow`, which has no runtime carrier yet)"
            .to_owned(),
    );
}

#[cfg(test)]
mod tests {
    use super::{Declaration, Geometry, rows};
    use crate::project::record::json::{Json, Number};
    use crate::project::record::page::{PageConstraints, PageLayout};
    use rust_widgets::designer::{GenerationGap, NodeRect};
    use rust_widgets::json::{LayoutDiagnostic, LayoutIssue};

    fn declaration(layout: Option<&PageLayout>) -> Declaration<'_> {
        Declaration {
            node: 1,
            kind: "window",
            layout,
            children: 2,
            constraints: Vec::new(),
        }
    }

    fn counts() -> [NodeRect; 2] {
        [
            NodeRect {
                path: Vec::new(),
                id: Some("1".to_owned()),
                x: 0,
                y: 0,
                width: 320,
                height: 240,
            },
            NodeRect {
                path: vec![0],
                id: Some("2".to_owned()),
                x: 8,
                y: 8,
                width: 304,
                height: 24,
            },
        ]
    }

    #[test]
    fn a_page_without_a_declaration_says_what_that_means() {
        let rows = rows("pages/main.json", &declaration(None), Some(&[]), None);
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
        assert!(rows.iter().any(|row| row.contains("geometry: not solved")));
        assert!(
            rows.iter()
                .any(|row| row == "constraints: no node declares size, margin or overflow")
        );
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
        let rows = rows(
            "pages/main.json",
            &declaration(Some(&layout)),
            Some(&[]),
            None,
        );
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
            None,
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
        let rows = rows("pages/main.json", &declaration(None), None, None);
        assert!(
            rows.iter()
                .any(|row| row.contains("diagnostics: not checked")),
            "{rows:?}"
        );
        assert!(!rows.iter().any(|row| row == "diagnostics: none"));
    }

    #[test]
    fn the_solved_rectangles_are_the_generators_own() {
        // RW-116: the panel reports `GenerationReport::node_rects`, keyed by the document id, so a
        // canvas and the generated constant coordinates read one solve.
        let rects = counts();
        let geometry = Geometry {
            width: 320,
            height: 240,
            rects: &rects,
            gaps: &[],
        };
        let rows = rows(
            "pages/main.json",
            &declaration(None),
            Some(&[]),
            Some(geometry),
        );
        assert!(
            rows.iter()
                .any(|row| row == "geometry: solved against 320x240 for 2 node(s)"),
            "{rows:?}"
        );
        assert!(
            rows.iter()
                .any(|row| row == "  node 1 (path []): x=0 y=0 320x240"),
            "{rows:?}"
        );
        assert!(
            rows.iter()
                .any(|row| row == "  node 2 (path [0]): x=8 y=8 304x24"),
            "{rows:?}"
        );
    }

    #[test]
    fn declared_constraints_are_reported_per_node() {
        let constraints = PageConstraints::new([
            ("min_width".to_owned(), Json::Number(Number::from_i64(120))),
            (
                "margin".to_owned(),
                Json::Array(vec![
                    Json::Number(Number::from_i64(8)),
                    Json::Number(Number::from_i64(4)),
                ]),
            ),
        ]);
        let mut declaration = declaration(None);
        declaration.constraints = vec![(2, &constraints)];
        let rows = rows("pages/main.json", &declaration, Some(&[]), None);
        assert!(
            rows.iter().any(|row| row == "constraints: 1 node(s)"),
            "{rows:?}"
        );
        assert!(
            rows.iter()
                .any(|row| row == "  node 2: min_width=120, margin=[8,4]"),
            "{rows:?}"
        );
        assert!(
            rows.iter()
                .any(|row| row.contains("`overflow`, which has no runtime carrier")),
            "{rows:?}"
        );
    }

    #[test]
    fn a_product_that_carries_no_coordinates_says_so() {
        // RW-111: the default product's header says it carries no solved coordinates; the panel
        // repeats the generator's gap rather than calling the solve a failure.
        let rects = counts();
        let gaps = [GenerationGap {
            path: Vec::new(),
            widget: "window".to_owned(),
            reason: "this product does not carry the solved coordinates".to_owned(),
        }];
        let geometry = Geometry {
            width: 320,
            height: 240,
            rects: &rects,
            gaps: &gaps,
        };
        let rows = rows(
            "pages/main.json",
            &declaration(None),
            Some(&[]),
            Some(geometry),
        );
        assert!(
            rows.iter().any(|row| row
                == "not carried by this product: window path []: this product does not carry the \
                    solved coordinates"),
            "{rows:?}"
        );
    }
}
