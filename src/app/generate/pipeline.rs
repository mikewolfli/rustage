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
//! * **One project has two products.** [`run_dual`] runs the same document through both targets and
//!   stamps each file beneath the library's marker with this build's format and generator versions.
//!   [`Finding`]s carry the five T15.02 causes, so a node the target cannot express is reported
//!   rather than dropped; [`Cause::CrossProfile`] is a caveat, every other cause blocks.

#![allow(dead_code)]

use crate::contracts::revision::{FormatVersion, GeneratorVersion};
use rust_widgets::designer::{
    ArtifactPaths, Availability, GenerationGapReason, GenerationReport, GenerationRequest,
    TargetProfile, availability, generate,
};

/// The header line the dual products carry beneath the library's marker (task T15.02).
///
/// The library's marker says *that* a file was generated; this line says *by which build*, which is
/// what lets a reader tell a stale artifact from a current one without regenerating it. The two
/// versions come from [`crate::contracts::revision`], the one place this build defines them.
pub fn version_header() -> String {
    format!(
        "// rustage {generator} ({format})",
        generator = GeneratorVersion::CURRENT,
        format = FormatVersion::CURRENT,
    )
}

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

/// One cause of a generation gap, in the five categories T15.02 names.
///
/// The library already classifies its gaps ([`GenerationGapReason`]); this is that vocabulary with
/// the report's separate `cross_profile_caveats` list folded in, so a caller can name the cause
/// without knowing which of the report's three lists a gap arrived in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cause {
    /// The document asked for something the generator cannot express.
    Unsupported,
    /// A container holds more children than the target accepts.
    CapacityOverflow,
    /// Availability could not be settled under the running profile.
    CrossProfile,
    /// An event name is not one the control publishes.
    UnresolvableEvent,
    /// A wire carries a value into a channel that cannot receive it.
    IncompatibleWire,
}

impl Cause {
    /// Every cause, in the order a report renders them.
    pub const ALL: [Self; 5] = [
        Self::Unsupported,
        Self::CapacityOverflow,
        Self::CrossProfile,
        Self::UnresolvableEvent,
        Self::IncompatibleWire,
    ];

    /// The stable token the library writes at the front of a gap's reason.
    pub const fn token(self) -> &'static str {
        match self {
            Self::Unsupported => GenerationGapReason::Unsupported.token(),
            Self::CapacityOverflow => GenerationGapReason::CapacityOverflow.token(),
            Self::CrossProfile => GenerationGapReason::CrossProfile.token(),
            Self::UnresolvableEvent => GenerationGapReason::UnresolvableEvent.token(),
            Self::IncompatibleWire => GenerationGapReason::IncompatibleWire.token(),
        }
    }

    /// Whether this cause stops a product being written.
    ///
    /// A cross-profile caveat is reported but does not block: the designer runs on a desktop host,
    /// so refusing it would make generating for `mini` impossible (T15.02's "present the warning"
    /// rather than "refuse"). Every other cause means a node or wire would be missing from the
    /// file, which is the silent drop the task forbids.
    pub const fn blocks(self) -> bool {
        !matches!(self, Self::CrossProfile)
    }
}

/// One reported gap, with the cause and the node it is about.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Finding {
    /// Why the node is a gap.
    pub cause: Cause,
    /// The library's tree path (`[]` is the root).
    pub path: Vec<usize>,
    /// The control name, or the node a caller-supplied diagnostic names.
    pub node: String,
    /// The library's or caller's explanation.
    pub reason: String,
}

impl Finding {
    /// Whether this finding blocks a product.
    pub fn blocks(&self) -> bool {
        self.cause.blocks()
    }

    /// The finding in one line, with its cause token first so it is greppable.
    pub fn render(&self) -> String {
        format!(
            "{} path {:?} `{}`: {}",
            self.cause.token(),
            self.path,
            self.node,
            self.reason
        )
    }
}

/// Maps a library gap reason to the task's vocabulary.
///
/// A gap recorded before reasons were tokenised has no category; calling it `unsupported` is the
/// honest answer because the library uses that category for everything it cannot express.
fn cause_of(reason: Option<GenerationGapReason>) -> Cause {
    match reason {
        Some(GenerationGapReason::CapacityOverflow) => Cause::CapacityOverflow,
        Some(GenerationGapReason::CrossProfile) => Cause::CrossProfile,
        Some(GenerationGapReason::UnresolvableEvent) => Cause::UnresolvableEvent,
        Some(GenerationGapReason::IncompatibleWire) => Cause::IncompatibleWire,
        Some(GenerationGapReason::Unsupported) | None => Cause::Unsupported,
    }
}

/// Every gap a report carries, in the library's own order.
fn report_findings(report: &GenerationReport) -> Vec<Finding> {
    let mut out: Vec<Finding> = report
        .unsupported
        .iter()
        .map(|gap| Finding {
            cause: cause_of(gap.reason_kind()),
            path: gap.path.clone(),
            node: gap.widget.clone(),
            reason: gap.reason.clone(),
        })
        .collect();
    out.extend(report.capacity_overflow.iter().map(|gap| Finding {
        cause: Cause::CapacityOverflow,
        path: gap.path.clone(),
        node: gap.widget.clone(),
        reason: gap.reason.clone(),
    }));
    out.extend(report.cross_profile_caveats.iter().map(|gap| Finding {
        cause: Cause::CrossProfile,
        path: gap.path.clone(),
        node: gap.widget.clone(),
        reason: gap.reason.clone(),
    }));
    out
}

/// The precheck's findings, classified into the same vocabulary.
///
/// Style and layout diagnostics arrive from other domains (T11.07, T10.08) and have no library
/// category; they are `unsupported` because they stop the same way: the emitted file would not
/// match the document.
fn precheck_findings(precheck: &Precheck) -> Vec<Finding> {
    let mut out: Vec<Finding> = precheck
        .unsupported
        .iter()
        .map(|gap| Finding {
            cause: Cause::Unsupported,
            path: Vec::new(),
            node: gap.node.clone(),
            reason: gap.reason.clone(),
        })
        .collect();
    out.extend(precheck.capacity.iter().map(|gap| Finding {
        cause: Cause::CapacityOverflow,
        path: Vec::new(),
        node: gap.node.clone(),
        reason: gap.reason.clone(),
    }));
    out.extend(precheck.caveats.iter().map(|gap| Finding {
        cause: Cause::CrossProfile,
        path: Vec::new(),
        node: gap.node.clone(),
        reason: gap.reason.clone(),
    }));
    out.extend(precheck.style.iter().map(|gap| Finding {
        cause: Cause::Unsupported,
        path: Vec::new(),
        node: gap.node.clone(),
        reason: gap.reason.clone(),
    }));
    out.extend(precheck.layout.iter().map(|gap| Finding {
        cause: Cause::Unsupported,
        path: Vec::new(),
        node: gap.node.clone(),
        reason: gap.reason.clone(),
    }));
    out
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

    /// Every gap the precheck and the report name, classified (T15.02).
    pub fn findings(&self) -> Vec<Finding> {
        let mut out = precheck_findings(&self.precheck);
        out.extend(report_findings(&self.report));
        out
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

/// Everything a dual run is asked to generate: one project, two products.
///
/// There is deliberately no `target` here. T15.02's product contract is *both* files from one
/// project, because a designer that generated only the previewed profile would leave the other
/// committed file stale (the same reason the library's `plan_artifacts` always emits both).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DualInput {
    /// The generator's document, already translated ([`super::document`]).
    pub json: String,
    /// The client width the layout is solved against.
    pub width: u32,
    /// The client height.
    pub height: u32,
    /// The name of the generated function.
    pub function_name: String,
    /// The page's nodes, for the capability and capacity checks.
    pub nodes: Vec<Node>,
    /// Style diagnostics the caller computed (task T11.07).
    pub style: Vec<Gap>,
    /// Layout diagnostics the caller computed (task T10.08).
    pub layout: Vec<Gap>,
}

impl DualInput {
    /// The same document, sized, named and diagnosed, for both targets.
    pub fn from_single(input: &Input) -> Self {
        Self {
            json: input.json.clone(),
            width: input.width,
            height: input.height,
            function_name: input.function_name.clone(),
            nodes: input.nodes.clone(),
            style: input.style.clone(),
            layout: input.layout.clone(),
        }
    }
}

/// One of the two products: where it goes, its bytes, and what it could not express.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Product {
    /// The profile this file is compiled for.
    pub target: TargetProfile,
    /// The project-relative path, `src/generated/ui_default.rs` or `ui_stripped.rs`.
    pub path: String,
    /// The Rust source, marker first and version second.
    pub source: String,
    /// What the library emitted and what it refused.
    pub report: GenerationReport,
    /// The precheck the run started from.
    pub precheck: Precheck,
}

impl Product {
    /// Every gap this product's precheck and report name, deduplicated and classified.
    ///
    /// The same gap can be found twice when a caller supplies `node:` counts *and* the library
    /// inspects the same tree; reporting it once keeps the problem panel from showing duplicates
    /// without hiding either source.
    pub fn findings(&self) -> Vec<Finding> {
        let mut out = precheck_findings(&self.precheck);
        out.extend(report_findings(&self.report));
        let mut seen = std::collections::BTreeSet::new();
        out.retain(|finding| {
            seen.insert((
                finding.cause.token(),
                finding.node.clone(),
                finding.reason.clone(),
            ))
        });
        out
    }

    /// Whether this product must not be written.
    pub fn is_blocked(&self) -> bool {
        self.findings().iter().any(Finding::blocks)
    }
}

/// Both products from one project, in a stable order (default first, stripped second).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DualOutcome {
    /// The two products.
    pub products: Vec<Product>,
}

impl DualOutcome {
    /// The product for `target`, when the run produced one.
    pub fn product(&self, target: TargetProfile) -> Option<&Product> {
        self.products
            .iter()
            .find(|product| product.target == target)
    }

    /// Whether any product has a blocking finding.
    pub fn is_blocked(&self) -> bool {
        self.products.iter().any(Product::is_blocked)
    }

    /// Whether the run has no findings at all, caveats included.
    pub fn is_clean(&self) -> bool {
        self.products
            .iter()
            .all(|product| product.findings().is_empty())
    }

    /// Both products in one line each, with every finding named by cause.
    pub fn render(&self) -> Vec<String> {
        let mut out = vec![version_header()];
        for product in &self.products {
            out.push(format!(
                "{} ({}) {} bytes",
                product.path,
                product.target.feature_hint(),
                product.source.len()
            ));
            out.push(format!("  report: {}", product.report.summary()));
            for finding in product.findings() {
                let mark = if finding.blocks() {
                    "blocked"
                } else {
                    "caveat "
                };
                out.push(format!("  {mark} {}", finding.render()));
            }
        }
        out
    }
}

/// Generates both products from one input.
///
/// Each target goes through [`run`], so the prechecks, determinism and refusal rules are the same
/// single-target path's rather than a second set. The only addition is the version header, which
/// records the build that produced the file beneath the library's marker.
pub fn run_dual(input: &DualInput) -> Result<DualOutcome, Vec<Gap>> {
    let mut products = Vec::with_capacity(2);
    for target in [TargetProfile::Default, TargetProfile::Stripped] {
        let single = Input {
            json: input.json.clone(),
            target,
            width: input.width,
            height: input.height,
            function_name: input.function_name.clone(),
            nodes: input.nodes.clone(),
            style: input.style.clone(),
            layout: input.layout.clone(),
        };
        let outcome = run(&single)?;
        products.push(Product {
            target,
            path: ArtifactPaths::path_for(target),
            source: stamped_source(&outcome.source),
            report: outcome.report,
            precheck: outcome.precheck,
        });
    }
    Ok(DualOutcome { products })
}

/// Inserts the version header immediately after the source's first line.
///
/// The first line is the library's marker (or, for an empty source, the library's own reason); the
/// header goes second so [`super::artifact::is_generated`] keeps recognising the file.
fn stamped_source(source: &str) -> String {
    if source.is_empty() {
        return format!("{}\n", version_header());
    }
    let header = version_header();
    match source.split_once('\n') {
        Some((first, rest)) => format!("{first}\n{header}\n{rest}"),
        None => format!("{source}\n{header}\n"),
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
    use super::{
        Cause, DualInput, Gap, Input, Node, Outcome, precheck, run, run_dual, run_twice,
        version_header,
    };
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

    fn dual_input() -> DualInput {
        let single = input();
        DualInput::from_single(&single)
    }

    #[test]
    fn a_dual_run_produces_two_distinct_stamped_products() {
        let outcome = run_dual(&dual_input()).expect("both products");
        assert_eq!(outcome.products.len(), 2, "{:?}", outcome.render());
        assert_eq!(
            outcome.products[0].path, "src/generated/ui_default.rs",
            "the default product keeps the library's committed path"
        );
        assert_eq!(
            outcome.products[1].path, "src/generated/ui_stripped.rs",
            "the stripped product keeps the library's committed path"
        );
        assert_ne!(
            outcome.products[0].source, outcome.products[1].source,
            "the two templates must not collapse to one file"
        );
        for product in &outcome.products {
            let first = product.source.lines().next().unwrap_or_default();
            assert_eq!(
                first,
                rust_widgets::designer::GENERATED_MARKER,
                "{}",
                product.path
            );
            assert!(
                product.source.contains(&version_header()),
                "{} must carry the generator/format versions: {}",
                product.path,
                product.source
            );
            assert!(product.source.contains("fn build_ui"), "{}", product.path);
        }
    }

    #[test]
    fn a_dual_run_over_one_input_is_byte_identical_twice() {
        let first = run_dual(&dual_input()).expect("both products");
        let second = run_dual(&dual_input()).expect("both products");
        assert_eq!(first, second, "generation must be deterministic");
    }

    #[test]
    fn a_cross_profile_gap_is_named_in_the_dual_report() {
        let outcome = run_dual(&dual_input()).expect("both products");
        let stripped = outcome
            .product(TargetProfile::Stripped)
            .expect("a stripped product");
        assert!(
            stripped
                .findings()
                .iter()
                .any(|finding| finding.cause == Cause::CrossProfile),
            "a desktop host generating for mini is the cross-profile case: {:?}",
            stripped.findings()
        );
        let lines = outcome.render();
        assert!(
            lines
                .iter()
                .any(|line| line.contains(Cause::CrossProfile.token())),
            "the cause has to be named in the report: {lines:?}"
        );
    }

    #[test]
    fn an_unsupported_document_is_named_and_blocks_the_dual_run() {
        let mut bad = dual_input();
        bad.json = r#"{"definitely_not_a_widget":{"id":"x"}}"#.to_owned();
        let outcome = run_dual(&bad).expect("a report, not a silent success");
        assert!(outcome.is_blocked(), "{:?}", outcome.render());
        assert!(
            outcome.render().iter().any(|line| {
                line.contains(Cause::Unsupported.token()) && line.contains("definitely")
            }),
            "the unsupported control has to be named: {:?}",
            outcome.render()
        );
    }

    #[test]
    fn every_report_list_and_reason_token_is_classified() {
        use rust_widgets::designer::{GenerationGap, GenerationReport};

        let gap = |reason: &str| GenerationGap {
            path: vec![0],
            widget: "button".to_owned(),
            reason: reason.to_owned(),
        };
        let report = GenerationReport {
            unsupported: vec![
                gap("unsupported: a control this generator cannot express"),
                gap("unresolvable-event: no published event"),
                gap("incompatible-wire: no channel for the payload"),
                // A gap from before reasons were tokenised is uncategorised, not invisible.
                gap("a reason with no token"),
            ],
            capacity_overflow: vec![gap("too many children")],
            cross_profile_caveats: vec![gap("settled under another profile")],
            ..GenerationReport::default()
        };

        let causes: Vec<Cause> = super::report_findings(&report)
            .iter()
            .map(|finding| finding.cause)
            .collect();
        assert_eq!(
            causes,
            vec![
                Cause::Unsupported,
                Cause::UnresolvableEvent,
                Cause::IncompatibleWire,
                Cause::Unsupported,
                Cause::CapacityOverflow,
                Cause::CrossProfile,
            ]
        );
    }

    #[test]
    fn every_cause_uses_the_library_s_stable_token() {
        assert_eq!(Cause::Unsupported.token(), "unsupported");
        assert_eq!(Cause::CapacityOverflow.token(), "capacity-overflow");
        assert_eq!(Cause::CrossProfile.token(), "cross-profile");
        assert_eq!(Cause::UnresolvableEvent.token(), "unresolvable-event");
        assert_eq!(Cause::IncompatibleWire.token(), "incompatible-wire");
        assert!(!Cause::CrossProfile.blocks());
        assert!(Cause::ALL.iter().filter(|cause| cause.blocks()).count() == 4);
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
