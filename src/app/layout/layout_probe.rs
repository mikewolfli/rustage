// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Comparing the preview with a library-generated probe on the same input (task T10.08).
//!
//! The acceptance line is *"the library-generated probe and the preview are compared on the same
//! input"*. The comparison is only meaningful if both sides ran through the one seam on the one
//! input, so the module makes that structural:
//!
//! * **Both sides run through [`layout`].** A [`Run`] is what the T10.01 seam produced for a target,
//!   so the preview and the probe are two results of one function, not two functions to keep in step.
//! * **Comparison is over the node set, not the order.** [`compare`] matches boxes by node id, so a
//!   box that moved is a different box and a box that is missing is named rather than shifting the
//!   rest along.
//! * **Every divergence names its node and both values.** A [`Difference`] carries the preview's box
//!   and the probe's, so "they disagree at node 7" comes with what each said.
//! * **A probe is per target.** A [`Run`] names the target it was made for, so a multi-target check
//!   is several runs compared one at a time rather than a single blended result.
//!
//! The host-artifact half of this task — re-verifying the integration against a real target build —
//! is F15/F16's subject and is recorded as a boundary here.

#![allow(dead_code)]

use crate::app::layout::layout_plan::{Computed, Diagnostic, Engine, LayoutInput, Role, layout};
use crate::contracts::identity::NodeId;

/// What one engine said for one target: the boxes, or the diagnostics that stopped it.
#[derive(Clone, PartialEq, Debug)]
pub struct Run {
    /// The target it was made for.
    pub target: String,
    /// The boxes, when the engine could compute them.
    pub boxes: Vec<Computed>,
    /// Why it could not, when it could not.
    pub diagnostics: Vec<Diagnostic>,
}

impl Run {
    /// The box for a node, if there is one.
    pub fn box_of(&self, node: NodeId) -> Option<Computed> {
        self.boxes.iter().find(|box_| box_.node == node).copied()
    }

    /// The run in a report.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "run {}: {} box(es), {} diagnostic(s)",
            self.target,
            self.boxes.len(),
            self.diagnostics.len()
        )];
        for box_ in &self.boxes {
            lines.push(format!("  {}", box_.render()));
        }
        for diagnostic in &self.diagnostics {
            lines.push(format!("  {}", diagnostic.render()));
        }
        lines
    }
}

/// Runs an engine on an input through the one seam, as a named target.
pub fn run(engine: &dyn Engine, role: Role, target: &str, inputs: &[LayoutInput]) -> Run {
    let plan = layout(engine, role, inputs);
    Run {
        target: target.to_owned(),
        boxes: plan.computed,
        diagnostics: plan.diagnostics,
    }
}

/// One node the preview and the probe disagree about.
#[derive(Clone, PartialEq, Debug)]
pub struct Difference {
    /// The node.
    pub node: NodeId,
    /// What the preview said, if anything.
    pub preview: Option<Computed>,
    /// What the probe said, if anything.
    pub probe: Option<Computed>,
}

impl Difference {
    /// The difference in one line.
    pub fn render(&self) -> String {
        match (&self.preview, &self.probe) {
            (Some(preview), Some(probe)) => format!(
                "node {}: preview {}x{} at {},{} but probe {}x{} at {},{}",
                self.node.get(),
                preview.width,
                preview.height,
                preview.x,
                preview.y,
                probe.width,
                probe.height,
                probe.x,
                probe.y
            ),
            (Some(_), None) => format!(
                "node {}: the preview has a box the probe does not",
                self.node.get()
            ),
            (None, Some(_)) => format!(
                "node {}: the probe has a box the preview does not",
                self.node.get()
            ),
            (None, None) => format!("node {}: neither side has a box", self.node.get()),
        }
    }
}

/// What a comparison found.
#[derive(Clone, PartialEq, Debug)]
pub struct Report {
    /// The scenario that was compared.
    pub scenario: String,
    /// Every node the two sides disagree about.
    pub differences: Vec<Difference>,
}

impl Report {
    /// Whether the preview and the probe agreed everywhere.
    pub fn agreed(&self) -> bool {
        self.differences.is_empty()
    }

    /// The report in a report.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "compare {}: {} difference(s)",
            self.scenario,
            self.differences.len()
        )];
        lines.extend(
            self.differences
                .iter()
                .map(|difference| format!("  {}", difference.render())),
        );
        lines
    }
}

/// Compares a preview with a probe over the same input, node by node.
pub fn compare(scenario: &str, inputs: &[LayoutInput], preview: &Run, probe: &Run) -> Report {
    let mut differences = Vec::new();
    for input in inputs {
        let left = preview.box_of(input.node);
        let right = probe.box_of(input.node);
        if left != right {
            differences.push(Difference {
                node: input.node,
                preview: left,
                probe: right,
            });
        }
    }
    Report {
        scenario: scenario.to_owned(),
        differences,
    }
}

#[cfg(test)]
mod tests {
    use super::{Run, compare, run};
    use crate::app::layout::layout_plan::{Computed, Diagnostic, Engine, LayoutInput, Role, Size};
    use crate::contracts::identity::NodeId;

    fn node(value: u64) -> NodeId {
        NodeId::from_raw(value)
    }

    fn box_(node: NodeId, width: f64) -> Computed {
        Computed {
            node,
            x: 0.0,
            y: 0.0,
            width,
            height: 10.0,
        }
    }

    /// An engine that answers the boxes it was told.
    struct Supplied(Vec<Computed>);

    impl Engine for Supplied {
        fn layout(&self, inputs: &[LayoutInput]) -> Result<Vec<Computed>, Vec<Diagnostic>> {
            let mut boxes = Vec::new();
            let mut diagnostics = Vec::new();
            for input in inputs {
                match self.0.iter().find(|box_| box_.node == input.node) {
                    Some(box_) => boxes.push(*box_),
                    None => diagnostics.push(Diagnostic::new(input.node, "no box")),
                }
            }
            if diagnostics.is_empty() {
                Ok(boxes)
            } else {
                Err(diagnostics)
            }
        }
    }

    fn inputs() -> Vec<LayoutInput> {
        vec![
            LayoutInput::new(node(1), Size::new(400.0, 300.0)),
            LayoutInput::new(node(2), Size::new(400.0, 300.0)),
        ]
    }

    #[test]
    fn both_roles_produce_the_same_boxes_from_one_engine() {
        let engine = Supplied(vec![box_(node(1), 10.0), box_(node(2), 20.0)]);
        let preview = run(&engine, Role::Preview, "preview", &inputs());
        let probe = run(&engine, Role::Generation, "lib", &inputs());
        assert_eq!(preview.boxes, probe.boxes);
        assert!(compare("same", &inputs(), &preview, &probe).agreed());
    }

    #[test]
    fn a_differing_box_is_reported_with_both_values() {
        let preview = Run {
            target: "preview".to_owned(),
            boxes: vec![box_(node(1), 10.0), box_(node(2), 20.0)],
            diagnostics: Vec::new(),
        };
        let probe = Run {
            target: "lib".to_owned(),
            boxes: vec![box_(node(1), 10.0), box_(node(2), 33.0)],
            diagnostics: Vec::new(),
        };
        let report = compare("wide", &inputs(), &preview, &probe);
        assert!(!report.agreed());
        assert_eq!(report.differences.len(), 1);
        let difference = &report.differences[0];
        assert_eq!(difference.node, node(2));
        assert_eq!(difference.preview.map(|box_| box_.width), Some(20.0));
        assert_eq!(difference.probe.map(|box_| box_.width), Some(33.0));
        assert!(difference.render().contains("preview"));
    }

    #[test]
    fn a_node_the_probe_has_no_box_for_is_named() {
        let preview = Run {
            target: "preview".to_owned(),
            boxes: vec![box_(node(1), 10.0), box_(node(2), 20.0)],
            diagnostics: Vec::new(),
        };
        let probe = Run {
            target: "lib".to_owned(),
            boxes: vec![box_(node(1), 10.0)],
            diagnostics: vec![Diagnostic::new(node(2), "no box")],
        };
        let report = compare("missing", &inputs(), &preview, &probe);
        assert_eq!(report.differences.len(), 1);
        assert!(report.differences[0].render().contains("probe does not"));
    }

    #[test]
    fn a_node_the_preview_has_no_box_for_is_named() {
        let preview = Run {
            target: "preview".to_owned(),
            boxes: vec![box_(node(1), 10.0)],
            diagnostics: vec![Diagnostic::new(node(2), "no box")],
        };
        let probe = Run {
            target: "lib".to_owned(),
            boxes: vec![box_(node(1), 10.0), box_(node(2), 20.0)],
            diagnostics: Vec::new(),
        };
        let report = compare("missing", &inputs(), &preview, &probe);
        assert_eq!(report.differences.len(), 1);
        assert!(report.differences[0].render().contains("preview does not"));
    }

    #[test]
    fn a_run_carries_the_engine_refusal() {
        let engine = Supplied(vec![box_(node(1), 1.0)]);
        let run = run(&engine, Role::Preview, "preview", &inputs());
        // The seam answers boxes or diagnostics, never both, so one missing box empties the run.
        assert!(run.boxes.is_empty());
        assert_eq!(run.diagnostics.len(), 1, "only node 2 had no box");
        assert!(run.render()[0].contains("1 diagnostic(s)"));
    }

    #[test]
    fn both_sides_missing_a_box_is_agreement() {
        let left = Run {
            target: "a".to_owned(),
            boxes: Vec::new(),
            diagnostics: Vec::new(),
        };
        let right = Run {
            target: "b".to_owned(),
            boxes: Vec::new(),
            diagnostics: Vec::new(),
        };
        assert!(compare("none", &inputs(), &left, &right).agreed());
    }

    #[test]
    fn the_report_names_the_scenario() {
        let engine = Supplied(vec![box_(node(1), 1.0), box_(node(2), 2.0)]);
        let preview = run(&engine, Role::Preview, "preview", &inputs());
        let probe = run(&engine, Role::Generation, "lib", &inputs());
        let report = compare("nested/boundary", &inputs(), &preview, &probe);
        assert!(report.render()[0].contains("nested/boundary"));
        assert!(report.agreed());
    }

    #[test]
    fn a_run_finds_a_node_box_by_id() {
        let run = Run {
            target: "lib".to_owned(),
            boxes: vec![box_(node(7), 42.0)],
            diagnostics: Vec::new(),
        };
        assert_eq!(run.box_of(node(7)).map(|box_| box_.width), Some(42.0));
        assert!(run.box_of(node(9)).is_none());
    }
}
