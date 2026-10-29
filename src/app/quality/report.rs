// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The problem centre, design checks, test/visual evidence and the evidence bundle (task F21).
//!
//! The module keeps the sources and versions of a quality claim explicit. A problem is not "an
//! error": it names where it came from (design, build, test, visual, performance), which target and
//! revision it is about, and the node or source position that answers *where*. A test result is not
//! "passed": it is passed, failed, skipped or **not run**, and the last one stays visibly not run.
//! Contrast is computed from the two colours rather than trusted from a theme name.

use std::collections::{BTreeMap, BTreeSet};

use rust_widgets::designer::generator::availability_with;
use rust_widgets::platform::profile::{SurfacePolicy, surface_policy};
use rust_widgets::widget::WidgetFactory;
use rust_widgets::widget::capability::TargetSpec;

use crate::app::quality::visual::VisualEvidence;
use crate::app::targets::snapshot::{Level, target_profile};
use crate::project::record::json::{Json, JsonObject};

/// Where a problem came from.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Source {
    /// The document model or a design check.
    Design,
    /// A compiler or build.
    Build,
    /// A test run.
    Test,
    /// A visual comparison.
    Visual,
    /// A performance measurement.
    Performance,
}

impl Source {
    /// The source's name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Design => "design",
            Self::Build => "build",
            Self::Test => "test",
            Self::Visual => "visual",
            Self::Performance => "performance",
        }
    }

    /// Reads a source name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "design" => Some(Self::Design),
            "build" => Some(Self::Build),
            "test" => Some(Self::Test),
            "visual" => Some(Self::Visual),
            "performance" => Some(Self::Performance),
            _ => None,
        }
    }
}

/// How a problem reads.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Severity {
    /// The product cannot be used.
    Error,
    /// The product works with a documented limit.
    Warning,
    /// Context only.
    Note,
}

impl Severity {
    /// The severity's name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Note => "note",
        }
    }

    /// Reads a severity name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "error" => Some(Self::Error),
            "warning" => Some(Self::Warning),
            "note" => Some(Self::Note),
            _ => None,
        }
    }
}

/// One problem with its source and the target/revision it belongs to.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Problem {
    /// Where it came from.
    pub source: Source,
    /// The target profile or toolchain it applies to.
    pub target: String,
    /// The document revision it was read at.
    pub revision: u64,
    /// The node it points at, when it has one.
    pub node: Option<u64>,
    /// The source position, when it has one.
    pub location: Option<String>,
    /// How serious it is.
    pub severity: Severity,
    /// What is wrong.
    pub message: String,
}

impl Problem {
    /// The problem in one line.
    pub fn render(&self) -> String {
        let place = self
            .node
            .map(|node| format!(" node {node}"))
            .or_else(|| self.location.clone().map(|location| format!(" {location}")))
            .unwrap_or_default();
        format!(
            "[{}] {} {}@r{}{place}: {}",
            self.severity.name(),
            self.source.name(),
            self.target,
            self.revision,
            self.message
        )
    }
}

/// A filter over the problem centre.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Filter {
    /// Only this source.
    pub source: Option<Source>,
    /// Only this target.
    pub target: Option<String>,
    /// Only problems at or after this revision.
    pub revision: Option<u64>,
    /// Only this severity.
    pub severity: Option<Severity>,
}

/// The problems this session has, from every source.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Center {
    problems: Vec<Problem>,
    /// The revision the centre currently reports. Problems older than it are retained only until
    /// the next [`Center::advance`], which is what keeps a stale build error out of "current".
    current: Option<u64>,
}

impl Center {
    /// An empty centre.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a problem, replacing an existing one with the same identity.
    ///
    /// Two runs that report the same source, target, revision, node, location and message are one
    /// problem, not two rows. Severity is deliberately not part of the identity: a later run that
    /// raises or lowers the severity updates the row instead of leaving both answers visible.
    pub fn add(&mut self, problem: Problem) {
        if let Some(existing) = self.problems.iter_mut().find(|existing| {
            existing.source == problem.source
                && existing.target == problem.target
                && existing.revision == problem.revision
                && existing.node == problem.node
                && existing.location == problem.location
                && existing.message == problem.message
        }) {
            *existing = problem;
            return;
        }
        self.problems.push(problem);
    }

    /// Makes `revision` the current one and drops problems from older revisions.
    ///
    /// The centre is a view of the current revision, not a log: keeping an old revision could make
    /// an already-fixed error look current. A caller that wants history records it in the evidence
    /// bundle before advancing.
    pub fn advance(&mut self, revision: u64) {
        self.current = Some(revision);
        self.problems.retain(|problem| problem.revision >= revision);
    }

    /// The problems matching `filter`, in insertion order.
    pub fn matching(&self, filter: &Filter) -> Vec<&Problem> {
        self.problems
            .iter()
            .filter(|problem| {
                self.current
                    .is_none_or(|current| problem.revision >= current)
                    && filter.source.is_none_or(|source| problem.source == source)
                    && filter
                        .target
                        .as_ref()
                        .is_none_or(|target| &problem.target == target)
                    && filter
                        .revision
                        .is_none_or(|revision| problem.revision >= revision)
                    && filter
                        .severity
                        .is_none_or(|severity| problem.severity == severity)
            })
            .collect()
    }

    /// What "go to" answers for a problem: its node or source position.
    pub fn navigate(problem: &Problem) -> Option<String> {
        problem
            .node
            .map(|node| format!("node {node}"))
            .or_else(|| problem.location.clone())
    }

    /// The centre in one line per problem.
    pub fn render(&self, filter: &Filter) -> Vec<String> {
        self.matching(filter)
            .into_iter()
            .map(Problem::render)
            .collect()
    }
}

/// A width and height in logical units.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Extent {
    /// The width.
    pub width: f64,
    /// The height.
    pub height: f64,
}

/// A placed box in the parent's coordinates.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Frame {
    /// The left edge.
    pub x: f64,
    /// The top edge.
    pub y: f64,
    /// The width.
    pub width: f64,
    /// The height.
    pub height: f64,
}

impl Frame {
    /// The right edge.
    fn right(self) -> f64 {
        self.x + self.width
    }

    /// The bottom edge.
    fn bottom(self) -> f64 {
        self.y + self.height
    }

    /// Whether this box contains `other`, touching edges included.
    fn contains(self, other: Self) -> bool {
        other.x >= self.x
            && other.y >= self.y
            && other.right() <= self.right()
            && other.bottom() <= self.bottom()
    }

    /// Whether the two boxes share area. Touching edges do not count.
    fn overlaps(self, other: Self) -> bool {
        self.x < other.right()
            && other.x < self.right()
            && self.y < other.bottom()
            && other.y < self.bottom()
    }

    /// The box in one line.
    fn render(self) -> String {
        format!("{},{} {}x{}", self.x, self.y, self.width, self.height)
    }
}

/// Whether a node may overlap its siblings.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Overlap {
    /// Siblings may share area.
    #[default]
    Allowed,
    /// Siblings must not share area.
    Forbidden,
}

impl Overlap {
    /// The policy's name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::Forbidden => "forbidden",
        }
    }

    /// Reads a policy name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "allowed" => Some(Self::Allowed),
            "forbidden" => Some(Self::Forbidden),
            _ => None,
        }
    }
}

/// One node's layout: the constraints it must satisfy and the box it was placed in.
///
/// Both the constraints and the frame are facts the caller reports. The library's arrangement
/// (`Layout::update`/`arrange_with_context`) is not consumed here yet, so a constraint the check
/// cannot verify is a *cannot verify* warning, never a silent pass.
#[derive(Clone, PartialEq, Debug)]
pub struct LayoutFact {
    /// The node.
    pub node: u64,
    /// The smallest it may be.
    pub minimum: Option<Extent>,
    /// The largest it may be.
    pub maximum: Option<Extent>,
    /// The box the layout placed it in, when one was arranged.
    pub frame: Option<Frame>,
    /// Whether it may share area with its siblings.
    pub overlap: Overlap,
}

impl LayoutFact {
    /// A node with no constraints and no frame.
    pub fn new(node: u64) -> Self {
        Self {
            node,
            minimum: None,
            maximum: None,
            frame: None,
            overlap: Overlap::Allowed,
        }
    }
}

/// What a binding writes.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum BindingTarget {
    /// A node's property.
    Property {
        /// The node.
        node: u64,
        /// The property.
        property: String,
    },
    /// A node's event.
    Event {
        /// The node.
        node: u64,
        /// The event.
        event: String,
    },
}

impl BindingTarget {
    /// The node the target belongs to, for navigation.
    pub fn node(&self) -> u64 {
        match self {
            Self::Property { node, .. } | Self::Event { node, .. } => *node,
        }
    }

    /// Whether this is a property or an event.
    fn what(&self) -> &'static str {
        match self {
            Self::Property { .. } => "property",
            Self::Event { .. } => "event",
        }
    }

    /// The target in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Property { node, property } => format!("node {node}.{property}"),
            Self::Event { node, event } => format!("node {node} event {event}"),
        }
    }
}

/// The declared value family on one side of a binding.
///
/// The fact is the pair *after* conversion. Which conversions may bridge which families is the
/// contract of `data::binding`; this check does not repeat it and does not claim a pair was
/// converted merely because the document names a conversion.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum BindingType {
    /// Free text.
    Text,
    /// A whole number.
    Integer,
    /// A fractional number.
    Number,
    /// A flag.
    Bool,
    /// A colour.
    Colour,
    /// One of a fixed set of names.
    Enum(Vec<String>),
}

impl BindingType {
    /// The type's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Integer => "integer",
            Self::Number => "number",
            Self::Bool => "bool",
            Self::Colour => "colour",
            Self::Enum(_) => "enum",
        }
    }

    /// Reads a type name, with `enum(a,b)` for a named set.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "text" => Some(Self::Text),
            "integer" => Some(Self::Integer),
            "number" => Some(Self::Number),
            "bool" => Some(Self::Bool),
            "colour" => Some(Self::Colour),
            _ => {
                let variants = text.strip_prefix("enum(")?.strip_suffix(')')?;
                let names: Vec<String> = variants
                    .split(',')
                    .map(|name| name.trim().to_owned())
                    .collect();
                if names.iter().any(String::is_empty) {
                    return None;
                }
                Some(Self::Enum(names))
            }
        }
    }

    /// Whether a target of this type accepts a source of `source`.
    ///
    /// Families are compared after conversion, so only a whole number widening to a fractional
    /// number is accepted across families; everything else must already match.
    pub fn accepts(&self, source: &Self) -> bool {
        match (self, source) {
            (Self::Number, Self::Integer | Self::Number) => true,
            (Self::Enum(target), Self::Enum(source)) => target == source,
            _ => self == source,
        }
    }
}

/// One binding as the document and the generator describe it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BindingFact {
    /// Where the value comes from, as the document names it.
    pub source: String,
    /// What it writes.
    pub target: BindingTarget,
    /// The source's type after conversion.
    pub source_type: BindingType,
    /// The type the target declares. `None` means the document does not declare that target.
    pub target_type: Option<BindingType>,
    /// Whether the target is generated output that must not be written back.
    pub generated: bool,
}

/// Where a capability or capacity fact came from.
///
/// The two sources are kept apart on purpose. A caller-supplied fact is only as good as the
/// document or argv that stated it; a contract-derived fact was read from the pinned library. The
/// check never rewrites one source with the other, so a report can say which witness it used.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Provenance {
    /// The caller stated the fact; the document or argv is the only witness.
    #[default]
    Supplied,
    /// The fact was read from the pinned `rust_widgets` contract.
    Contract,
}

impl Provenance {
    /// The provenance's name, for a report or the CLI.
    pub fn name(self) -> &'static str {
        match self {
            Self::Supplied => "supplied",
            Self::Contract => "contract",
        }
    }
}

/// One capability answer read from the library for one name.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CapabilityAnswer {
    /// The name that was asked about.
    pub name: String,
    /// The library's answer, kept as a [`Level`] rather than a yes/no.
    pub level: Level,
}

/// The library's resource limits for the profile this binary was built for.
///
/// Every field is copied from [`SurfacePolicy`]; none is derived or rounded here.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CapacityLimits {
    /// Upper bound on simultaneously mounted controls.
    pub max_widgets: usize,
    /// Largest square texture the surface may allocate, in pixels.
    pub max_texture: u32,
    /// Glyph atlas budget, in bytes.
    pub font_cache_bytes: usize,
    /// Depth of the platform event queue.
    pub event_queue: usize,
}

impl CapacityLimits {
    /// Reads the four limits from the library's policy without changing them.
    pub fn from_surface_policy(policy: SurfacePolicy) -> Self {
        Self {
            max_widgets: policy.max_widgets,
            max_texture: policy.max_texture,
            font_cache_bytes: policy.font_cache_bytes,
            event_queue: policy.event_queue,
        }
    }
}

/// Facts a design check reads. It is deliberately small: every field is something the document
/// already knows, not a second document.
///
/// Layout and binding facts are *reported by the caller*: the library's arranged boxes
/// (`Layout::update`/`arrange_with_context`) and binding contract (`BindingPipeline`,
/// `bind_widget_property`) are not consumed here yet. Where a fact is missing the check says
/// *cannot verify*; it does not invent a library value or treat the gap as a pass.
///
/// Capability and capacity facts can come from either side, and they stay distinguishable:
/// [`Self::capabilities`] and a `Supplied` [`Self::capacity_provenance`] are caller-supplied,
/// while [`Self::contract_capabilities`] and a `Contract` provenance were read from the pinned
/// library by [`Self::capability_from`] and [`Self::capacity_from`].
#[derive(Clone, PartialEq, Debug, Default)]
pub struct DesignFacts {
    /// Node ids in the tree.
    pub nodes: Vec<u64>,
    /// Each node's parent (`None` for a root).
    pub parents: BTreeMap<u64, Option<u64>>,
    /// Capabilities the caller stated and whether the target has them.
    pub capabilities: BTreeMap<String, bool>,
    /// The control count and the target's limit.
    pub capacity: (usize, usize),
    /// Whether `capacity.1` was supplied by the caller or read from the library.
    pub capacity_provenance: Provenance,
    /// The full limit set, present only when [`Self::capacity_from`] set the limit.
    pub capacity_limits: Option<CapacityLimits>,
    /// Capability answers read from the library, keyed by name.
    pub contract_capabilities: BTreeMap<String, Level>,
    /// Resources the document references and whether they exist.
    pub resources: BTreeMap<String, bool>,
    /// The focus order, as written.
    pub focus_order: Vec<u64>,
    /// Each node's constraints and placed box.
    pub layout: Vec<LayoutFact>,
    /// Each binding's source, target and declared types.
    pub bindings: Vec<BindingFact>,
}

impl DesignFacts {
    /// Records whether the library can supply each named capability for `profile`.
    ///
    /// The answer comes from [`availability_with`], so `Local` and `CrossProfile` are allowed and
    /// `Unavailable`/`Unknown` are diagnosed by [`Self::check`]. The answers are kept in
    /// [`Self::contract_capabilities`], apart from a caller's yes/no in [`Self::capabilities`].
    ///
    /// A name the caller already stated is refused rather than overwritten: a caller that wants the
    /// contract's answer must not leave a contradictory second fact behind.
    pub fn capability_from(
        &mut self,
        factory: &WidgetFactory,
        profile: &str,
        names: &[&str],
    ) -> Result<Vec<CapabilityAnswer>, String> {
        let target = target_profile(profile).ok_or_else(|| unknown_profile(profile))?;
        if let Some(supplied) = names
            .iter()
            .find(|name| self.capabilities.contains_key(**name))
        {
            return Err(format!(
                "capability {supplied:?} was supplied by the caller and is not overwritten by the contract"
            ));
        }
        Ok(names
            .iter()
            .map(|name| {
                let level = Level::of(availability_with(factory, name, target));
                self.contract_capabilities.insert((*name).to_owned(), level);
                CapabilityAnswer {
                    name: (*name).to_owned(),
                    level,
                }
            })
            .collect())
    }

    /// Reads the target's control limit from the library's surface policy.
    ///
    /// `used` stays whatever the caller recorded in [`Self::capacity`]; only the limit comes from
    /// the contract, and [`Self::capacity_provenance`] says so. The full policy is kept in
    /// [`Self::capacity_limits`] so a report can name the texture, font-cache and queue limits it
    /// did not check.
    ///
    /// The library exposes [`surface_policy`] for the profile this binary was built with and has
    /// no per-profile lookup. Asking for another profile therefore names that gap instead of
    /// applying this profile's numbers to it.
    pub fn capacity_from(&mut self, profile: &str) -> Result<CapacityLimits, String> {
        if target_profile(profile).is_none() {
            return Err(unknown_profile(profile));
        }
        let current = TargetSpec::current().profile;
        if profile != current {
            return Err(format!(
                "the library's surface policy describes the profile this binary was built with ({current}); it exposes no per-profile lookup, so the {profile:?} limit cannot be read from the contract"
            ));
        }
        let limits = CapacityLimits::from_surface_policy(surface_policy());
        self.capacity.1 = limits.max_widgets;
        self.capacity_provenance = Provenance::Contract;
        self.capacity_limits = Some(limits);
        Ok(limits)
    }

    /// Which side stated the capability fact for `name`.
    ///
    /// A contract answer wins when both sides named the same capability, because
    /// [`Self::capability_from`] refuses that collision; `None` means neither side named it.
    pub fn capability_provenance(&self, name: &str) -> Option<Provenance> {
        if self.contract_capabilities.contains_key(name) {
            Some(Provenance::Contract)
        } else if self.capabilities.contains_key(name) {
            Some(Provenance::Supplied)
        } else {
            None
        }
    }

    /// Runs the structural, layout, binding, capability, capacity, resource and focus checks.
    pub fn check(&self, target: &str, revision: u64) -> Vec<Problem> {
        let mut problems = Vec::new();
        let mut seen = BTreeSet::new();
        for node in &self.nodes {
            if !seen.insert(*node) {
                problems.push(problem(
                    Source::Design,
                    target,
                    revision,
                    Some(*node),
                    Severity::Error,
                    format!("node {node} is declared twice"),
                ));
            }
        }
        for (node, parent) in &self.parents {
            if let Some(parent) = parent
                && !seen.contains(parent)
            {
                problems.push(problem(
                    Source::Design,
                    target,
                    revision,
                    Some(*node),
                    Severity::Error,
                    format!("node {node} names a missing parent {parent}"),
                ));
            }
        }
        for (capability, available) in &self.capabilities {
            if !available {
                problems.push(problem(
                    Source::Design,
                    target,
                    revision,
                    None,
                    Severity::Error,
                    format!("target {target} does not support {capability}"),
                ));
            }
        }
        for (capability, level) in &self.contract_capabilities {
            let reason = match level {
                Level::Local | Level::CrossProfile => continue,
                Level::Unavailable => {
                    "the library knows this name but no constructor under the running profile can build it"
                }
                Level::Unknown => "the library registers no control by that name",
            };
            problems.push(problem(
                Source::Design,
                target,
                revision,
                None,
                Severity::Error,
                format!("target {target} does not support {capability}: {reason}"),
            ));
        }
        if self.capacity.0 > self.capacity.1 {
            let origin = match self.capacity_provenance {
                Provenance::Contract => " (the library's surface policy)",
                Provenance::Supplied => "",
            };
            problems.push(problem(
                Source::Design,
                target,
                revision,
                None,
                Severity::Error,
                format!(
                    "the document uses {} controls, over the target limit of {}{origin}",
                    self.capacity.0, self.capacity.1
                ),
            ));
        }
        for (resource, present) in &self.resources {
            if !present {
                problems.push(problem(
                    Source::Design,
                    target,
                    revision,
                    None,
                    Severity::Error,
                    format!("resource {resource:?} is referenced but not present"),
                ));
            }
        }
        let mut focus = BTreeSet::new();
        for node in &self.focus_order {
            if !focus.insert(*node) {
                problems.push(problem(
                    Source::Design,
                    target,
                    revision,
                    Some(*node),
                    Severity::Warning,
                    format!("node {node} appears twice in the focus order"),
                ));
            }
        }
        self.check_layout(target, revision, &mut problems);
        self.check_bindings(target, revision, &mut problems);
        problems
    }

    /// The parent a node declares, if it declares one.
    fn parent_of(&self, node: u64) -> Option<u64> {
        self.parents.get(&node).copied().flatten()
    }

    /// Constraints that cannot be satisfied, boxes outside their parent and forbidden overlaps.
    fn check_layout(&self, target: &str, revision: u64, problems: &mut Vec<Problem>) {
        let frames: BTreeMap<u64, Frame> = self
            .layout
            .iter()
            .filter_map(|fact| fact.frame.map(|frame| (fact.node, frame)))
            .collect();
        for fact in &self.layout {
            if let (Some(minimum), Some(maximum)) = (fact.minimum, fact.maximum) {
                if minimum.width > maximum.width {
                    problems.push(problem(
                        Source::Design,
                        target,
                        revision,
                        Some(fact.node),
                        Severity::Error,
                        format!(
                            "node {} minimum width {} is above the maximum width {}",
                            fact.node, minimum.width, maximum.width
                        ),
                    ));
                }
                if minimum.height > maximum.height {
                    problems.push(problem(
                        Source::Design,
                        target,
                        revision,
                        Some(fact.node),
                        Severity::Error,
                        format!(
                            "node {} minimum height {} is above the maximum height {}",
                            fact.node, minimum.height, maximum.height
                        ),
                    ));
                }
            }
            let Some(frame) = fact.frame else {
                continue;
            };
            if let Some(Some(parent)) = self.parents.get(&fact.node) {
                match frames.get(parent) {
                    Some(parent_frame) if !parent_frame.contains(frame) => {
                        problems.push(problem(
                            Source::Design,
                            target,
                            revision,
                            Some(fact.node),
                            Severity::Error,
                            format!(
                                "node {} is placed outside its parent {}: {} is not inside {}",
                                fact.node,
                                parent,
                                frame.render(),
                                parent_frame.render()
                            ),
                        ));
                    }
                    Some(_) => {}
                    None => {
                        problems.push(problem(
                            Source::Design,
                            target,
                            revision,
                            Some(fact.node),
                            Severity::Warning,
                            format!(
                                "the layout check cannot verify node {} inside parent {}: the parent has no frame",
                                fact.node, parent
                            ),
                        ));
                    }
                }
            }
            if fact.overlap == Overlap::Forbidden && !self.parents.contains_key(&fact.node) {
                problems.push(problem(
                    Source::Design,
                    target,
                    revision,
                    Some(fact.node),
                    Severity::Warning,
                    format!(
                        "the layout check cannot verify overlap for node {}: its parent is not declared",
                        fact.node
                    ),
                ));
            }
        }
        let mut ordered: Vec<&LayoutFact> = self
            .layout
            .iter()
            .filter(|fact| fact.frame.is_some())
            .collect();
        ordered.sort_by_key(|fact| fact.node);
        for (index, first) in ordered.iter().enumerate() {
            for second in &ordered[index + 1..] {
                if first.node == second.node
                    || (first.overlap != Overlap::Forbidden && second.overlap != Overlap::Forbidden)
                    || !self.parents.contains_key(&first.node)
                    || !self.parents.contains_key(&second.node)
                    || self.parent_of(first.node) != self.parent_of(second.node)
                {
                    continue;
                }
                let (Some(first_frame), Some(second_frame)) = (first.frame, second.frame) else {
                    continue;
                };
                if first_frame.overlaps(second_frame) {
                    problems.push(problem(
                        Source::Design,
                        target,
                        revision,
                        Some(first.node),
                        Severity::Error,
                        format!(
                            "nodes {} and {} overlap, and overlap is not allowed",
                            first.node, second.node
                        ),
                    ));
                }
            }
        }
    }

    /// Unknown targets, unwritable generated targets, type mismatches and duplicate bindings.
    fn check_bindings(&self, target: &str, revision: u64, problems: &mut Vec<Problem>) {
        let mut seen = BTreeSet::new();
        for binding in &self.bindings {
            if !seen.insert((binding.source.clone(), binding.target.clone())) {
                problems.push(problem(
                    Source::Design,
                    target,
                    revision,
                    Some(binding.target.node()),
                    Severity::Error,
                    format!(
                        "binding {} -> {} is declared twice",
                        binding.source,
                        binding.target.render()
                    ),
                ));
            }
            let Some(declared) = &binding.target_type else {
                problems.push(problem(
                    Source::Design,
                    target,
                    revision,
                    Some(binding.target.node()),
                    Severity::Error,
                    format!(
                        "binding {} -> {} names an unknown target {}",
                        binding.source,
                        binding.target.render(),
                        binding.target.what()
                    ),
                ));
                continue;
            };
            if binding.generated {
                problems.push(problem(
                    Source::Design,
                    target,
                    revision,
                    Some(binding.target.node()),
                    Severity::Error,
                    format!(
                        "binding {} -> {} writes to generated output, which is not writable",
                        binding.source,
                        binding.target.render()
                    ),
                ));
                continue;
            }
            if !declared.accepts(&binding.source_type) {
                problems.push(problem(
                    Source::Design,
                    target,
                    revision,
                    Some(binding.target.node()),
                    Severity::Error,
                    format!(
                        "binding {} -> {} gives {} to a {} target",
                        binding.source,
                        binding.target.render(),
                        binding.source_type.name(),
                        declared.name()
                    ),
                ));
            }
        }
    }
}

/// The refusal for a profile the library's target does not declare.
fn unknown_profile(profile: &str) -> String {
    format!(
        "{profile:?} is not a profile the library's target declares (desktop, tablet, mobile, mini, embedded)"
    )
}

/// Builds one problem with the common fields.
fn problem(
    source: Source,
    target: &str,
    revision: u64,
    node: Option<u64>,
    severity: Severity,
    message: String,
) -> Problem {
    Problem {
        source,
        target: target.to_owned(),
        revision,
        node,
        location: None,
        severity,
        message,
    }
}

/// A relative luminance from an sRGB channel.
fn linear(channel: u8) -> f64 {
    let value = f64::from(channel) / 255.0;
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

/// The WCAG relative luminance of an RGB colour.
pub fn luminance(rgb: (u8, u8, u8)) -> f64 {
    0.2126 * linear(rgb.0) + 0.7152 * linear(rgb.1) + 0.0722 * linear(rgb.2)
}

/// The WCAG contrast ratio of two colours, between 1 and 21.
pub fn contrast(foreground: (u8, u8, u8), background: (u8, u8, u8)) -> f64 {
    let first = luminance(foreground);
    let second = luminance(background);
    let (lighter, darker) = if first >= second {
        (first, second)
    } else {
        (second, first)
    };
    (lighter + 0.05) / (darker + 0.05)
}

/// One test's state.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TestStatus {
    /// It ran and passed.
    Passed,
    /// It ran and failed.
    Failed,
    /// It was explicitly ignored.
    Skipped,
    /// It exists but has not run in this session.
    NotRun,
}

impl TestStatus {
    /// The status name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
            Self::NotRun => "not-run",
        }
    }

    /// Reads a status name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "passed" | "pass" => Some(Self::Passed),
            "failed" | "fail" => Some(Self::Failed),
            "skipped" | "skip" => Some(Self::Skipped),
            "not-run" | "notrun" => Some(Self::NotRun),
            _ => None,
        }
    }
}

/// One test and its state.
#[derive(Clone, PartialEq, Debug)]
pub struct TestResult {
    /// Its name.
    pub name: String,
    /// What happened.
    pub status: TestStatus,
    /// How long it took, when it ran.
    pub seconds: Option<f64>,
}

/// The tests this session knows about.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Tests {
    results: BTreeMap<String, TestResult>,
    /// The document revision the results were recorded at, when a run recorded them.
    revision: Option<u64>,
}

impl Tests {
    /// No tests yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a test result, replacing an earlier result for the same name.
    pub fn record(&mut self, name: impl Into<String>, status: TestStatus, seconds: Option<f64>) {
        let name = name.into();
        self.results.insert(
            name.clone(),
            TestResult {
                name,
                status,
                seconds,
            },
        );
    }

    /// Records a discovered test as not run, unless it already has a result.
    ///
    /// Re-listing a suite must not forget that a test passed a moment ago, so discovery adds the
    /// tests it has not seen and leaves every existing result alone.
    pub fn note_discovered(&mut self, name: impl Into<String>) {
        let name = name.into();
        self.results.entry(name.clone()).or_insert(TestResult {
            name,
            status: TestStatus::NotRun,
            seconds: None,
        });
    }

    /// The revision the results were recorded at, when a run recorded them.
    pub fn revision(&self) -> Option<u64> {
        self.revision
    }

    /// Records the revision a run just ran at.
    ///
    /// Re-listing a suite does not move this: discovery is not a run, so a result stays attributed
    /// to the revision that produced it.
    pub fn set_revision(&mut self, revision: u64) {
        self.revision = Some(revision);
    }

    /// Whether the results belong to an older revision than `current`.
    ///
    /// A stale result is not wrong, it is old: the document changed after it was produced, so it
    /// must not be reported as the current run's outcome. A suite that never ran has no revision and
    /// is not stale — it is simply not run, which [`Self::tally`] already says.
    pub fn is_stale(&self, current: u64) -> bool {
        self.revision.is_some_and(|revision| revision != current)
    }

    /// The results in name order.
    pub fn results(&self) -> Vec<&TestResult> {
        self.results.values().collect()
    }

    /// The number of tests in each state.
    pub fn tally(&self) -> BTreeMap<&'static str, usize> {
        let mut tally = BTreeMap::new();
        for result in self.results.values() {
            *tally.entry(result.status.name()).or_insert(0) += 1;
        }
        tally
    }

    /// Writes JUnit XML. A test that did not run has no `<testcase>` and is listed in a
    /// `<properties>` entry, because JUnit has no "not run" state and pretending it passed is the
    /// failure this module exists to avoid.
    pub fn junit(&self) -> String {
        let failures = self
            .results
            .values()
            .filter(|result| result.status == TestStatus::Failed)
            .count();
        let skipped = self
            .results
            .values()
            .filter(|result| result.status == TestStatus::Skipped)
            .count();
        let total = self.results.len();
        let mut xml = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<testsuite name=\"rustage\" tests=\"{total}\" failures=\"{failures}\" skipped=\"{skipped}\">\n"
        );
        for result in self.results.values() {
            if result.status == TestStatus::NotRun {
                continue;
            }
            let time = result.seconds.unwrap_or(0.0);
            xml.push_str(&format!(
                "  <testcase name=\"{}\" time=\"{time:.3}\">",
                xml_escape(&result.name)
            ));
            match result.status {
                TestStatus::Failed => xml.push_str("<failure/>"),
                TestStatus::Skipped => xml.push_str("<skipped/>"),
                TestStatus::Passed | TestStatus::NotRun => {}
            }
            xml.push_str("</testcase>\n");
        }
        let not_run: Vec<&str> = self
            .results
            .values()
            .filter(|result| result.status == TestStatus::NotRun)
            .map(|result| result.name.as_str())
            .collect();
        let mut properties = Vec::new();
        if let Some(revision) = self.revision {
            properties.push(format!(
                "<property name=\"revision\" value=\"{revision}\"/>"
            ));
        }
        if !not_run.is_empty() {
            properties.push(format!(
                "<property name=\"not-run\" value=\"{}\"/>",
                xml_escape(&not_run.join(", "))
            ));
        }
        if !properties.is_empty() {
            xml.push_str(&format!(
                "  <properties>{}</properties>\n",
                properties.concat()
            ));
        }
        xml.push_str("</testsuite>\n");
        xml
    }
}

/// Escapes the five XML metacharacters.
fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// The evidence bundle written for one revision.
#[derive(Clone, PartialEq, Debug)]
pub struct Evidence {
    /// The document revision.
    pub revision: u64,
    /// The target.
    pub target: String,
    /// The problem centre.
    pub center: Center,
    /// The test results.
    pub tests: Tests,
    /// The visual evidence, which distinguishes "not collected" from a comparison.
    pub visual: VisualEvidence,
}

impl Evidence {
    /// The bundle as JSON, with every problem and test carrying its state.
    pub fn json(&self) -> String {
        let problems: Vec<Json> = self
            .center
            .problems
            .iter()
            .map(|problem| {
                let mut object = JsonObject::new();
                object.insert("source", Json::string(problem.source.name()));
                object.insert("severity", Json::string(problem.severity.name()));
                object.insert("target", Json::string(&problem.target));
                object.insert("revision", Json::number(problem.revision));
                if let Some(node) = problem.node {
                    object.insert("node", Json::number(node));
                }
                if let Some(location) = &problem.location {
                    object.insert("location", Json::string(location));
                }
                object.insert("message", Json::string(&problem.message));
                Json::Object(object)
            })
            .collect();
        let tests: Vec<Json> = self
            .tests
            .results()
            .into_iter()
            .map(|result| {
                let mut object = JsonObject::new();
                object.insert("name", Json::string(&result.name));
                object.insert("status", Json::string(result.status.name()));
                if let Some(seconds) = result.seconds {
                    object.insert("seconds", Json::number(seconds as u64));
                }
                Json::Object(object)
            })
            .collect();
        let mut root = JsonObject::new();
        root.insert("revision", Json::number(self.revision));
        root.insert("target", Json::string(&self.target));
        root.insert("problems", Json::Array(problems));
        root.insert("tests", Json::Array(tests));
        root.insert("visual", self.visual.json());
        Json::Object(root).render()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BindingFact, BindingTarget, BindingType, Center, DesignFacts, Evidence, Extent, Filter,
        Frame, LayoutFact, Level, Overlap, Problem, Provenance, Severity, Source, TestStatus,
        Tests, contrast,
    };
    use crate::app::quality::VisualCondition;
    use rust_widgets::widget::WidgetFactory;
    use rust_widgets::widget::capability::TargetSpec;

    fn problem(source: Source, target: &str, revision: u64, message: &str) -> Problem {
        Problem {
            source,
            target: target.to_owned(),
            revision,
            node: Some(7),
            location: Some("src/main.rs:3:1".to_owned()),
            severity: Severity::Error,
            message: message.to_owned(),
        }
    }

    #[test]
    fn the_centre_keeps_source_target_and_revision_apart() {
        let mut center = Center::new();
        center.add(problem(Source::Design, "desktop", 4, "missing capability"));
        center.add(problem(Source::Build, "desktop", 5, "compile failed"));
        let design = Filter {
            source: Some(Source::Design),
            ..Filter::default()
        };
        assert_eq!(center.matching(&design).len(), 1);
        let recent = Filter {
            revision: Some(5),
            ..Filter::default()
        };
        assert_eq!(center.matching(&recent).len(), 1);
        assert_eq!(
            Center::navigate(&problem(Source::Design, "desktop", 4, "x")),
            Some("node 7".to_owned())
        );
    }

    #[test]
    fn the_same_problem_is_one_row_and_an_older_one_is_not_current() {
        let mut center = Center::new();
        center.add(problem(Source::Build, "desktop", 4, "compile failed"));
        center.add(problem(Source::Build, "desktop", 4, "compile failed"));
        assert_eq!(center.matching(&Filter::default()).len(), 1);
        center.advance(5);
        assert!(center.matching(&Filter::default()).is_empty());
        center.add(problem(Source::Build, "desktop", 5, "compile failed"));
        assert_eq!(center.matching(&Filter::default()).len(), 1);
    }

    #[test]
    fn a_design_check_names_each_breakage() {
        let mut parents = std::collections::BTreeMap::new();
        parents.insert(1, None);
        parents.insert(2, Some(99));
        let mut capabilities = std::collections::BTreeMap::new();
        capabilities.insert("graphics".to_owned(), false);
        let mut resources = std::collections::BTreeMap::new();
        resources.insert("hero.png".to_owned(), false);
        let facts = DesignFacts {
            nodes: vec![1, 2, 2],
            parents,
            capabilities,
            capacity: (12, 8),
            resources,
            focus_order: vec![1, 1],
            ..DesignFacts::default()
        };
        let problems = facts.check("mini", 3);
        assert!(problems.iter().any(|p| p.message.contains("twice")));
        assert!(
            problems
                .iter()
                .any(|p| p.message.contains("missing parent"))
        );
        assert!(problems.iter().any(|p| p.message.contains("graphics")));
        assert!(
            problems
                .iter()
                .any(|p| p.message.contains("over the target limit"))
        );
        assert!(problems.iter().any(|p| p.message.contains("hero.png")));
        assert!(problems.iter().any(|p| p.message.contains("focus order")));
    }

    #[test]
    fn contrast_is_computed_from_colours() {
        assert!((contrast((0, 0, 0), (255, 255, 255)) - 21.0).abs() < 0.01);
        assert!((contrast((255, 255, 255), (255, 255, 255)) - 1.0).abs() < 0.01);
        assert!(contrast((0x0A, 0x84, 0xFF), (255, 255, 255)) < 4.5);
    }

    #[test]
    fn a_not_run_test_is_not_reported_as_passed() {
        let mut tests = Tests::new();
        tests.record("a", TestStatus::Passed, Some(0.1));
        tests.record("b", TestStatus::NotRun, None);
        let tally = tests.tally();
        assert_eq!(tally.get("passed"), Some(&1));
        assert_eq!(tally.get("not-run"), Some(&1));
        let xml = tests.junit();
        assert!(xml.contains("not-run"));
        assert!(!xml.contains("<testcase name=\"b\""));
    }

    #[test]
    fn a_visual_capture_refuses_a_different_environment() {
        let first = VisualCondition {
            platform: "macos".to_owned(),
            scale: 200,
            font_digest: "abc".to_owned(),
            resources: 0,
        };
        let second = VisualCondition {
            scale: 100,
            ..first.clone()
        };
        assert!(first.compare(&second).is_err());
    }

    #[test]
    fn the_evidence_bundle_carries_every_state() {
        let mut center = Center::new();
        center.add(problem(Source::Test, "desktop", 2, "failed"));
        let mut tests = Tests::new();
        tests.record("suite", TestStatus::Failed, None);
        let evidence = Evidence {
            revision: 2,
            target: "desktop".to_owned(),
            center,
            tests,
            visual: crate::app::quality::VisualEvidence::NotCollected,
        };
        let json = evidence.json();
        assert!(json.contains("\"status\": \"failed\""), "{json}");
        assert!(json.contains("\"source\": \"test\""), "{json}");
        assert!(json.contains("\"state\": \"not-collected\""), "{json}");
    }

    #[test]
    fn a_layout_violation_produces_a_design_problem() {
        let mut parents = std::collections::BTreeMap::new();
        parents.insert(1, None);
        parents.insert(2, Some(1));
        parents.insert(3, Some(1));
        parents.insert(4, Some(1));
        let facts = DesignFacts {
            nodes: vec![1, 2, 3, 4],
            parents,
            layout: vec![
                LayoutFact {
                    node: 1,
                    frame: Some(Frame {
                        x: 0.0,
                        y: 0.0,
                        width: 200.0,
                        height: 200.0,
                    }),
                    ..LayoutFact::new(1)
                },
                LayoutFact {
                    node: 2,
                    minimum: Some(Extent {
                        width: 120.0,
                        height: 10.0,
                    }),
                    maximum: Some(Extent {
                        width: 80.0,
                        height: 10.0,
                    }),
                    frame: Some(Frame {
                        x: 10.0,
                        y: 10.0,
                        width: 20.0,
                        height: 20.0,
                    }),
                    ..LayoutFact::new(2)
                },
                LayoutFact {
                    node: 3,
                    frame: Some(Frame {
                        x: 190.0,
                        y: 190.0,
                        width: 20.0,
                        height: 20.0,
                    }),
                    ..LayoutFact::new(3)
                },
                LayoutFact {
                    node: 4,
                    frame: Some(Frame {
                        x: 185.0,
                        y: 185.0,
                        width: 10.0,
                        height: 10.0,
                    }),
                    overlap: Overlap::Forbidden,
                    ..LayoutFact::new(4)
                },
            ],
            ..DesignFacts::default()
        };
        let problems = facts.check("desktop", 9);
        assert!(problems.iter().any(|p| p.message.contains("minimum width")));
        assert!(
            problems
                .iter()
                .any(|p| p.message.contains("outside its parent"))
        );
        assert!(problems.iter().any(|p| p.message.contains("overlap")));
        assert!(
            problems
                .iter()
                .all(|p| p.source == Source::Design && p.target == "desktop" && p.revision == 9)
        );
    }

    #[test]
    fn a_valid_layout_produces_no_problems() {
        let mut parents = std::collections::BTreeMap::new();
        parents.insert(1, None);
        parents.insert(2, Some(1));
        let facts = DesignFacts {
            nodes: vec![1, 2],
            parents,
            layout: vec![
                LayoutFact {
                    node: 1,
                    frame: Some(Frame {
                        x: 0.0,
                        y: 0.0,
                        width: 100.0,
                        height: 100.0,
                    }),
                    ..LayoutFact::new(1)
                },
                LayoutFact {
                    node: 2,
                    minimum: Some(Extent {
                        width: 10.0,
                        height: 10.0,
                    }),
                    maximum: Some(Extent {
                        width: 50.0,
                        height: 50.0,
                    }),
                    frame: Some(Frame {
                        x: 10.0,
                        y: 10.0,
                        width: 20.0,
                        height: 20.0,
                    }),
                    overlap: Overlap::Forbidden,
                },
            ],
            ..DesignFacts::default()
        };
        assert!(facts.check("desktop", 1).is_empty());
    }

    #[test]
    fn a_binding_type_violation_is_a_design_problem_of_the_right_source() {
        let facts = DesignFacts {
            bindings: vec![BindingFact {
                source: "f.count".to_owned(),
                target: BindingTarget::Property {
                    node: 2,
                    property: "label".to_owned(),
                },
                source_type: BindingType::Text,
                target_type: Some(BindingType::Number),
                generated: false,
            }],
            ..DesignFacts::default()
        };
        let problems = facts.check("mini", 4);
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].source, Source::Design);
        assert_eq!(problems[0].target, "mini");
        assert_eq!(problems[0].revision, 4);
        assert_eq!(problems[0].node, Some(2));
        assert!(
            problems[0]
                .message
                .contains("gives text to a number target")
        );
    }

    #[test]
    fn an_unsupported_binding_target_is_diagnosed_not_dropped() {
        let facts = DesignFacts {
            bindings: vec![
                BindingFact {
                    source: "f.count".to_owned(),
                    target: BindingTarget::Property {
                        node: 2,
                        property: "tone".to_owned(),
                    },
                    source_type: BindingType::Text,
                    target_type: None,
                    generated: false,
                },
                BindingFact {
                    source: "f.saved".to_owned(),
                    target: BindingTarget::Event {
                        node: 3,
                        event: "finished".to_owned(),
                    },
                    source_type: BindingType::Bool,
                    target_type: None,
                    generated: false,
                },
            ],
            ..DesignFacts::default()
        };
        let problems = facts.check("desktop", 2);
        assert_eq!(problems.len(), 2);
        assert!(
            problems
                .iter()
                .any(|p| p.message.contains("unknown target property"))
        );
        assert!(
            problems
                .iter()
                .any(|p| p.message.contains("unknown target event"))
        );
        assert!(problems.iter().all(|p| p.source == Source::Design));
    }

    #[test]
    fn a_duplicate_binding_and_a_generated_target_are_named() {
        let binding = BindingFact {
            source: "1.label".to_owned(),
            target: BindingTarget::Property {
                node: 5,
                property: "text".to_owned(),
            },
            source_type: BindingType::Text,
            target_type: Some(BindingType::Text),
            generated: false,
        };
        let facts = DesignFacts {
            bindings: vec![
                binding.clone(),
                binding,
                BindingFact {
                    source: "f.total".to_owned(),
                    target: BindingTarget::Property {
                        node: 6,
                        property: "width".to_owned(),
                    },
                    source_type: BindingType::Number,
                    target_type: Some(BindingType::Number),
                    generated: true,
                },
            ],
            ..DesignFacts::default()
        };
        let problems = facts.check("desktop", 1);
        assert!(
            problems
                .iter()
                .any(|p| p.message.contains("declared twice"))
        );
        assert!(
            problems
                .iter()
                .any(|p| p.message.contains("generated output"))
        );
    }

    #[test]
    fn a_valid_binding_produces_no_problems() {
        let facts = DesignFacts {
            bindings: vec![BindingFact {
                source: "f.count".to_owned(),
                target: BindingTarget::Property {
                    node: 2,
                    property: "width".to_owned(),
                },
                source_type: BindingType::Integer,
                target_type: Some(BindingType::Number),
                generated: false,
            }],
            ..DesignFacts::default()
        };
        assert!(facts.check("desktop", 1).is_empty());
    }

    #[test]
    fn a_local_library_capability_is_not_diagnosed() {
        let current = TargetSpec::current().profile;
        let mut facts = DesignFacts::default();
        let answers = facts
            .capability_from(WidgetFactory::shared(), current, &["button"])
            .expect("the library's own profile");
        assert_eq!(answers.len(), 1);
        assert_eq!(answers[0].level, Level::Local);
        assert_eq!(
            facts.capability_provenance("button"),
            Some(Provenance::Contract)
        );
        assert!(facts.check(current, 1).is_empty());
    }

    #[test]
    fn an_unknown_library_capability_is_diagnosed() {
        let current = TargetSpec::current().profile;
        let mut facts = DesignFacts::default();
        let answers = facts
            .capability_from(WidgetFactory::shared(), current, &["not_a_widget"])
            .expect("the library's own profile");
        assert_eq!(answers[0].level, Level::Unknown);
        let problems = facts.check(current, 1);
        assert_eq!(problems.len(), 1);
        assert!(problems[0].message.contains("not_a_widget"));
        assert!(problems[0].message.contains("registers no control"));
    }

    #[test]
    fn an_unavailable_library_capability_is_diagnosed() {
        // A desktop build registers a constructor for every capability, so `Unavailable` cannot be
        // produced by this binary's factory. The answer a crippled build reports is still checked.
        let mut facts = DesignFacts::default();
        facts
            .contract_capabilities
            .insert("gated".to_owned(), Level::Unavailable);
        let problems = facts.check("mini", 1);
        assert_eq!(problems.len(), 1);
        assert!(problems[0].message.contains("gated"));
        assert!(problems[0].message.contains("no constructor"));
    }

    #[test]
    fn capacity_comes_from_the_surface_policy_and_overflow_names_the_library_limit() {
        let current = TargetSpec::current().profile;
        let mut facts = DesignFacts::default();
        let limits = facts
            .capacity_from(current)
            .expect("the library's own profile");
        assert_eq!(facts.capacity_provenance, Provenance::Contract);
        assert_eq!(facts.capacity_limits, Some(limits));
        assert_eq!(facts.capacity.1, limits.max_widgets);

        facts.capacity.0 = limits.max_widgets + 1;
        let problems = facts.check(current, 1);
        assert_eq!(problems.len(), 1);
        assert!(
            problems[0]
                .message
                .contains(&format!("over the target limit of {}", limits.max_widgets))
        );
        assert!(problems[0].message.contains("surface policy"));
    }

    #[test]
    fn a_capacity_limit_for_another_profile_names_the_library_gap() {
        let current = TargetSpec::current().profile;
        let other = if current == "mini" { "desktop" } else { "mini" };
        let mut facts = DesignFacts {
            capacity: (10, 20),
            ..DesignFacts::default()
        };
        let refusal = facts
            .capacity_from(other)
            .expect_err("the library has no per-profile lookup");
        assert!(refusal.contains("surface policy"));
        assert!(refusal.contains("no per-profile lookup"));
        assert_eq!(facts.capacity, (10, 20));
        assert_eq!(facts.capacity_provenance, Provenance::Supplied);
        assert_eq!(facts.capacity_limits, None);
    }

    #[test]
    fn caller_supplied_facts_stay_distinguishable_from_contract_facts() {
        let current = TargetSpec::current().profile;
        let mut facts = DesignFacts::default();
        facts.capabilities.insert("graphics".to_owned(), false);
        assert_eq!(
            facts.capability_provenance("graphics"),
            Some(Provenance::Supplied)
        );
        assert_eq!(facts.capacity_provenance, Provenance::Supplied);

        facts
            .capability_from(WidgetFactory::shared(), current, &["button"])
            .expect("the library's own profile");
        assert_eq!(
            facts.capability_provenance("button"),
            Some(Provenance::Contract)
        );

        // The contract does not overwrite a caller's fact; the collision is refused.
        let refusal = facts
            .capability_from(WidgetFactory::shared(), current, &["graphics"])
            .expect_err("graphics was supplied by the caller");
        assert!(refusal.contains("supplied by the caller"));
        assert_eq!(
            facts.capability_provenance("graphics"),
            Some(Provenance::Supplied)
        );
        assert!(!facts.contract_capabilities.contains_key("graphics"));
    }
}
