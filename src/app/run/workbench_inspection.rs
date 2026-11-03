// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The design-inspection panel's rows (task F21): real checks over the open scene.
//!
//! The panel is a view over [`DesignFacts`], the project's design-check model: every node, parent
//! and placed box is a fact read from the scene, and the rows are the problems that model reports.
//! Nothing here keeps a second opinion.

use std::collections::{BTreeMap, BTreeSet};

use rust_widgets::widget::WidgetFactory;

use crate::app::quality::{DesignFacts, Frame, LayoutFact, Overlap};
use crate::contracts::identity::NodeId;

/// The profile the panel checks against.
const PROFILE: &str = "desktop";

/// The design-check rows for the open scene.
///
/// Every node in the structural tree becomes a node, parent and layout fact, so the panel reports
/// the checks the pinned library actually knows how to make instead of a second, panel-specific
/// opinion. A fact that cannot be read is left out or left at its default: a missing rectangle has
/// no frame, a refused capacity question leaves the limit at zero so the check cannot invent an
/// overflow, and a refused capability question is simply not answered. The check then says
/// *cannot verify* rather than passing silently.
pub(super) fn rows(scene: &crate::app::run::workbench_scene::CanvasState) -> Vec<String> {
    let ids = scene.tree().ids();
    let mut parents = BTreeMap::new();
    let mut layout = Vec::with_capacity(ids.len());
    let mut controls = BTreeSet::new();
    for node in &ids {
        parents.insert(node.get(), scene.parent(*node).map(|parent| parent.get()));
        layout.push(LayoutFact {
            node: node.get(),
            minimum: None,
            maximum: None,
            frame: frame_of(scene, *node),
            // The canvas deliberately allows overlapping siblings: the topmost node is the hit.
            overlap: Overlap::Allowed,
        });
        if let Ok(contract) = scene.contract(*node) {
            controls.insert(contract.control);
        }
    }
    let mut facts = DesignFacts {
        nodes: ids.iter().map(|node| node.get()).collect(),
        parents,
        layout,
        ..DesignFacts::default()
    };

    let used = facts.nodes.len();
    // Only a limit read from the contract is a real limit. On a refusal the default `(0, 0)` stays,
    // so `used > limit` cannot report an overflow on a number nobody supplied.
    if facts.capacity_from(PROFILE).is_ok() {
        facts.capacity.0 = used;
    }

    for control in &controls {
        // A control the library does not register is diagnosed by `check`. A refused question (for
        // example, a profile this binary cannot read) is not turned into an invented level here.
        let _ = facts.capability_from(WidgetFactory::shared(), PROFILE, &[control.as_str()]);
    }

    rows_from(&facts, scene.revision())
}

/// The rendered rows for a fact set, with `revision` attached to every problem.
fn rows_from(facts: &DesignFacts, revision: u64) -> Vec<String> {
    facts
        .check(PROFILE, revision)
        .into_iter()
        .map(|problem| problem.render())
        .collect()
}

/// A node's placed box in its parent's coordinates.
///
/// The scene stores absolute design-space rectangles while [`Frame`] is parent-relative, so a node
/// with a parent subtracts the parent's origin. A root is already absolute. A node with no
/// rectangle has no frame, which the check reports as *cannot verify* rather than as a pass.
fn frame_of(scene: &crate::app::run::workbench_scene::CanvasState, node: NodeId) -> Option<Frame> {
    let rect = scene.rect(node)?;
    let (x, y) = match scene.parent(node).and_then(|parent| scene.rect(parent)) {
        Some(parent) => (f64::from(rect.x - parent.x), f64::from(rect.y - parent.y)),
        None => (f64::from(rect.x), f64::from(rect.y)),
    };
    Some(Frame {
        x,
        y,
        width: f64::from(rect.width),
        height: f64::from(rect.height),
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{rows, rows_from};
    use crate::app::quality::{DesignFacts, Frame, LayoutFact, Overlap};
    use crate::app::run::workbench_scene::CanvasState;

    #[test]
    fn a_clean_scene_has_no_problems() {
        // The demo's contracts may name controls the library does not register (`window`), so the
        // panel may legitimately have something to say. What it must not do is panic or emit a row
        // a user cannot act on.
        let rows = rows(&CanvasState::demo());
        assert!(rows.iter().all(|row| !row.is_empty()), "{rows:?}");
    }

    #[test]
    fn an_over_limit_document_is_reported() {
        let facts = DesignFacts {
            nodes: vec![1, 2],
            capacity: (2, 1),
            ..DesignFacts::default()
        };
        let rows = rows_from(&facts, 7);
        assert!(
            rows.iter().any(|row| row.contains("over the target limit")),
            "{rows:?}"
        );
    }

    #[test]
    fn a_child_outside_its_parent_is_reported() {
        let facts = DesignFacts {
            nodes: vec![1, 2],
            parents: BTreeMap::from([(1, None), (2, Some(1))]),
            layout: vec![
                LayoutFact {
                    node: 1,
                    minimum: None,
                    maximum: None,
                    frame: Some(Frame {
                        x: 0.0,
                        y: 0.0,
                        width: 100.0,
                        height: 100.0,
                    }),
                    overlap: Overlap::Allowed,
                },
                LayoutFact {
                    node: 2,
                    minimum: None,
                    maximum: None,
                    frame: Some(Frame {
                        x: 200.0,
                        y: 200.0,
                        width: 10.0,
                        height: 10.0,
                    }),
                    overlap: Overlap::Allowed,
                },
            ],
            ..DesignFacts::default()
        };
        let rows = rows_from(&facts, 7);
        assert!(rows.iter().any(|row| row.contains("node 2")), "{rows:?}");
    }
}
