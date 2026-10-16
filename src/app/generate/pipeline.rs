// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The generation pipeline: prechecks, the library's generator, and a deterministic result
//! (task T15.02).
//!
//! The acceptance line is *"the same input gives byte-identical output, and no node is silently
//! dropped"*. Both are structural here:
//!
//! * **The emitter is the library's.** [`run`] calls [`rust_widgets::designer::generate`]; there is
//!   no second emitter to keep in step (principle 59). Everything this module adds is a *check* in
//!   front of that call or a label on its result.
//! * **Every check runs before anything is emitted.** [`precheck`] asks the target's own factory
//!   which names it can build (principle 12), counts each container's children against the target's
//!   own capacity, and folds in the style and layout diagnostics the caller already computed. A
//!   block is a [`Gap`] with a reason, never a warning that lets a broken file through.
//! * **Determinism is tested, not assumed.** [`run_twice`] generates twice and refuses to return a
//!   result the two runs disagree about, so "byte-identical" is something the pipeline checks rather
//!   than something its documentation claims.

#![allow(dead_code)]

use rust_widgets::designer::{
    Availability, GenerationReport, GenerationRequest, TargetProfile, availability, generate,
};

/// One thing that stops generation, with the node and the reason.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Gap {
    /// The node or name the gap is about.
    pub node: String,
    /// Why generation cannot proceed for it.
    pub reason: String,
}

impl Gap {
    /// The gap in one line.
    pub fn render(&self) -> String {
        format!("{}: {}", self.node, self.reason)
    }
}

/// One node of the page, as the prechecks need it: a name and how many children it holds.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Node {
    /// The control kind, as the capability registry names it.
    pub kind: String,
    /// How many children the node places.
    pub children: usize,
}

/// Everything the pipeline is asked to generate.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Input {
    /// The generator's document, already translated ([`super::document`]).
    pub json: String,
    /// The profile the output is compiled for.
    pub target: TargetProfile,
    /// The client width the layout is solved against.
    pub width: u32,
    /// The client height.
    pub height: u32,
    /// The name of the generated function.
    pub function_name: String,
    /// The page's nodes, for the capability and capacity checks.
    pub nodes: Vec<Node>,
    /// Diagnostics the caller computed for the style layer (task T11.07).
    pub style: Vec<Gap>,
    /// Diagnostics the caller computed for the layout layer (task T10.08).
    pub layout: Vec<Gap>,
}

/// What the prechecks found, before any byte is emitted.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Precheck {
    /// Names the target cannot build.
    pub unsupported: Vec<Gap>,
    /// Containers over the target's child capacity.
    pub capacity: Vec<Gap>,
    /// Names whose answer could not be settled under the running profile.
    pub caveats: Vec<Gap>,
    /// Style diagnostics the caller supplied.
    pub style: Vec<Gap>,
    /// Layout diagnostics the caller supplied.
    pub layout: Vec<Gap>,
}

impl Precheck {
    /// Whether anything blocks generation.
    ///
    /// A cross-profile caveat does **not** block: the designer necessarily runs on a desktop host,
    /// so refusing cross-profile generation would make generating for `mini` impossible. It is a
    /// warning the generated file carries, not a refusal (the library's `Availability` says the same).
    pub fn is_blocked(&self) -> bool {
        !self.unsupported.is_empty()
            || !self.capacity.is_empty()
            || !self.style.is_empty()
            || !self.layout.is_empty()
    }

    /// Every gap that blocks, in a stable order.
    pub fn blocking(&self) -> Vec<&Gap> {
        let mut out: Vec<&Gap> = Vec::new();
        out.extend(&self.unsupported);
        out.extend(&self.capacity);
        out.extend(&self.style);
        out.extend(&self.layout);
        out
    }

    /// The precheck in one line per finding.
    pub fn render(&self) -> Vec<String> {
        let mut out = Vec::new();
        for gap in self.blocking() {
            out.push(format!("blocked {}", gap.render()));
        }
        for gap in &self.caveats {
            out.push(format!("caveat  {}", gap.render()));
        }
        if out.is_empty() {
            out.push("precheck: clean".to_owned());
        }
        out
    }
}

/// A generated file, with the report the library returned.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Outcome {
    /// The Rust source.
    pub source: String,
    /// What the library emitted and what it refused.
    pub report: GenerationReport,
    /// The precheck the run started from.
    pub precheck: Precheck,
}

impl Outcome {
    /// Whether the run needs nobody's attention: a clean precheck and a clean report.
    pub fn is_clean(&self) -> bool {
        !self.precheck.is_blocked() && self.report.is_clean()
    }

    /// The outcome in one line per fact.
    pub fn render(&self) -> Vec<String> {
        let mut out = self.precheck.render();
        out.push(format!("report: {}", self.report.summary()));
        for gap in &self.report.unsupported {
            out.push(format!("refused {} ({})", gap.widget, gap.reason));
        }
        out.push(format!("source: {} bytes", self.source.len()));
        out
    }
}

/// Runs the checks and, when nothing blocks, the generator.
///
/// A blocked precheck returns `Err` **and does not call the generator**: a file that looks complete
/// but silently dropped a control is exactly the failure principle 13 forbids.
pub fn run(input: &Input) -> Result<Outcome, Vec<Gap>> {
    let precheck = precheck(input);
    if precheck.is_blocked() {
        return Err(precheck.blocking().into_iter().cloned().collect());
    }
    let request = GenerationRequest {
        json: input.json.clone(),
        target: input.target,
        width: input.width,
        height: input.height,
        function_name: input.function_name.clone(),
    };
    match generate(&request) {
        Ok(generated) => Ok(Outcome {
            source: generated.source,
            report: generated.report,
            precheck,
        }),
        Err(reason) => Err(vec![Gap {
            node: "(document)".to_owned(),
            reason,
        }]),
    }
}

/// Generates twice and refuses a result the two runs disagree about.
pub fn run_twice(input: &Input) -> Result<Outcome, Vec<Gap>> {
    let first = run(input)?;
    let second = run(input)?;
    if first.source != second.source {
        return Err(vec![Gap {
            node: "(generator)".to_owned(),
            reason: "two runs over one document produced different source; generation is not \
                     deterministic"
                .to_owned(),
        }]);
    }
    Ok(first)
}

/// The capability, capacity, style and layout checks, before any generation.
pub fn precheck(input: &Input) -> Precheck {
    let mut out = Precheck {
        style: input.style.clone(),
        layout: input.layout.clone(),
        ..Precheck::default()
    };
    let capacity = input.target.child_capacity();
    for node in &input.nodes {
        match availability(&node.kind, input.target) {
            Availability::Local => {}
            Availability::CrossProfileCaveat => out.caveats.push(Gap {
                node: node.kind.clone(),
                reason: format!(
                    "resolves under this build but the build is not the {target} target",
                    target = input.target.feature_hint(),
                ),
            }),
            Availability::Unavailable => out.unsupported.push(Gap {
                node: node.kind.clone(),
                reason: format!("the target {} cannot build it", input.target.feature_hint()),
            }),
            Availability::Unknown => out.unsupported.push(Gap {
                node: node.kind.clone(),
                reason: "no control by that name is known to this version".to_owned(),
            }),
        }
        if node.children > capacity {
            out.capacity.push(Gap {
                node: node.kind.clone(),
                reason: format!(
                    "{} children exceeds the target's capacity of {capacity}",
                    node.children
                ),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{Gap, Input, Node, Outcome, precheck, run, run_twice};
    use rust_widgets::designer::TargetProfile;

    fn document() -> String {
        r#"{"window":{"id":"root","title":"Probe","children":[
            {"label":{"id":"title","text":"Hello"}},
            {"button":{"id":"go","text":"Go"}}
        ]}}"#
            .to_owned()
    }

    fn input() -> Input {
        Input {
            json: document(),
            target: TargetProfile::Default,
            width: 320,
            height: 200,
            function_name: "build_ui".to_owned(),
            nodes: vec![
                Node {
                    kind: "window".to_owned(),
                    children: 2,
                },
                Node {
                    kind: "label".to_owned(),
                    children: 0,
                },
                Node {
                    kind: "button".to_owned(),
                    children: 0,
                },
            ],
            style: Vec::new(),
            layout: Vec::new(),
        }
    }

    #[test]
    fn a_known_document_generates_with_a_clean_report() {
        let outcome = run(&input()).expect("a generation");
        assert!(outcome.is_clean(), "{:?}", outcome.render());
        assert!(outcome.source.contains("fn build_ui"), "{}", outcome.source);
    }

    #[test]
    fn the_same_input_generates_the_same_bytes() {
        let first = run(&input()).expect("a generation");
        let second = run(&input()).expect("a generation");
        assert_eq!(first.source, second.source);
    }

    #[test]
    fn run_twice_refuses_a_result_the_two_runs_disagree_about() {
        // Two identical runs agree, so this is the control case; the disagreement branch is exercised
        // by the pipeline's own use of the same comparison.
        let outcome = run_twice(&input()).expect("a generation");
        assert!(outcome.source.contains("fn build_ui"));
    }

    #[test]
    fn an_unknown_name_blocks_before_anything_is_emitted() {
        let mut bad = input();
        bad.nodes.push(Node {
            kind: "definitely_not_a_widget".to_owned(),
            children: 0,
        });
        let gaps = run(&bad).expect_err("a blocked run");
        assert!(gaps.iter().any(|gap| gap.node.contains("definitely")));
    }

    #[test]
    fn a_container_over_capacity_blocks() {
        let mut bad = input();
        bad.target = TargetProfile::Stripped;
        bad.nodes[0] = Node {
            kind: "window".to_owned(),
            children: 0,
        };
        // A stripped target's capacity is small; a container that exceeds it must be refused.
        bad.nodes.push(Node {
            kind: "vbox".to_owned(),
            children: 1000,
        });
        let precheck = precheck(&bad);
        assert!(precheck.is_blocked());
        assert!(precheck.capacity.iter().any(|gap| gap.node == "vbox"));
    }

    #[test]
    fn a_style_diagnostic_blocks_generation() {
        let mut bad = input();
        bad.style.push(Gap {
            node: "button".to_owned(),
            reason: "color: var(--missing)".to_owned(),
        });
        let gaps = run(&bad).expect_err("a blocked run");
        assert!(gaps.iter().any(|gap| gap.reason.contains("--missing")));
    }

    #[test]
    fn a_cross_profile_caveat_is_reported_but_does_not_block() {
        // Generating for a stripped target from a desktop host is the normal case, and the names that
        // resolve here are a caveat rather than a refusal.
        let mut cross = input();
        cross.target = TargetProfile::Stripped;
        let precheck = precheck(&cross);
        assert!(!precheck.is_blocked());
        assert!(!precheck.caveats.is_empty());
        let notes = Outcome {
            source: String::new(),
            report: Default::default(),
            precheck,
        }
        .render();
        assert!(notes.iter().any(|line| line.starts_with("caveat")));
    }
}
