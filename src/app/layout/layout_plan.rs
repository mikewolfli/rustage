// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The layout contract, its computed result, its diagnostics and where a constraint came from
//! (task T10.01).
//!
//! The acceptance line is *"the preview and the generation reuse the same semantics rather than a
//! copy of the algorithm"*. The one way to make that true is to keep the algorithm in exactly one
//! place and let nothing else own a copy:
//!
//! * **The algorithm is the engine's, and only the engine's.** [`Engine`] is the seam the library
//!   implements; [`LibraryEngine`] is that implementation, and this module computes no box of its
//!   own, so there is no second algorithm to drift. `Layout::arrange_with_context` is the library
//!   entry it calls; the layouts without a hint channel forward that entry to `Layout::update`.
//! * **Both callers come through one door.** [`layout`] is the only caller of [`Engine::layout`], and
//!   it takes a [`Role`] — preview or generation — that changes *who is asking*, never the answer.
//! * **A result carries its own explanation.** A layout either yields the [`Computed`] boxes or a list
//!   of [`Diagnostic`]s, each naming the node and, when it can, the [`Source`] — node, property and
//!   origin — so "why is this box like this" is answerable and can be jumped to.
//! * **A constraint remembers where it came from.** [`Origin`] tells an explicit value from an
//!   inherited one from a default, so the property panel shows a source rather than a value with no
//!   history.
//! * **The device is part of the question.** [`Context`] carries the scale, the text scale and the
//!   minimum touch target, and [`Engine::layout_with_context`] is how a caller says "solve this for
//!   *that* device" without changing which algorithm answers.
//!
//! # What the adapter can and cannot feed the library (T10.01 boundary)
//!
//! [`LayoutInput`] carries a node, the space its parent offers it, and its constraints with their
//! **typed values**. The library's hint channel needs a min/preferred/max size per child, and a
//! layout needs one container rectangle, so the adapter maps every value it can and refuses the
//! rest by name rather than dropping it:
//!
//! * `available` is the fallback **preferred** size. `min`/`max`/`preferred` constraints override
//!   the corresponding [`AxisHints`] axis; `width`/`height` pin an axis; `grow` becomes the
//!   library stretch weight; `margin` becomes [`LayoutParams::margins`].
//! * `gap` and `padding` name the container's spacing. The library has one gap and one padding, so
//!   two different values for the same container are reported as a conflict instead of one
//!   silently winning.
//! * `x`/`y` become an [`AbsolutePosition`] and `cell` becomes a [`GridLayout`] cell. A value that
//!   the selected library layout has no channel for is reported as an unsupported constraint and
//!   the layout is refused; nothing is ignored.
//! * The container is either supplied through [`LibraryEngine::with_container`] or measured from
//!   the children's preferred extents plus the configured gap/padding. Measuring the outer rect is
//!   not a layout algorithm: every child rectangle still comes from the library.
//! * The library's layout entries return boxes, not a diagnostic list, so a child squeezed below
//!   its floor is visible only in the geometry. This adapter reports the failures it can observe —
//!   an unrepresentable value, a duplicate node, an unsupported or conflicting constraint, a child
//!   the layout did not place — and does not invent a second validation pass. Richer layout
//!   diagnostics remain the upstream gap tracked as blue26.
//! * One exact gap remains: `FlexLayout` reads `Hints::min`/`pref` but deliberately does not copy
//!   `Hints::max` into its solver, and `GridLayout::arrange` reads only `pref`. Those values still
//!   reach the library's hint contract, but the selected layout may not honour them.
//! * `FlexLayout` consumes [`LayoutContext`] itself (scale, text scale and the touch floor). The
//!   grid, stack and absolute layouts have no device-context channel, so the adapter pre-scales the
//!   grid gap/padding it constructs; the stack and absolute layouts have no spacing to scale.

#![allow(dead_code)]

use crate::app::layout::layout_choice::LayoutKind;
use crate::contracts::identity::NodeId;
use crate::contracts::model::PropertyName;
use rust_widgets::core::{Alignment, ObjectId, Rect, Size as LibrarySize};
use rust_widgets::layout::{
    AbsoluteLayout, AbsolutePosition, AlignItems, AxisHints, ChildInfo, FlexDirection, FlexLayout,
    FlexWrap, GridLayout, Hints, JustifyContent, Layout as LibraryLayout, LayoutContext,
    LayoutParams, StackLayout, StackMode,
};
use rust_widgets::style::EdgeOffsets;

/// Where a constraint's value came from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Origin {
    /// Written on this node.
    Explicit,
    /// Taken from an ancestor.
    Inherited,
    /// Supplied by the schema when nothing was written.
    Default,
}

impl Origin {
    /// The origin's name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Explicit => "explicit",
            Self::Inherited => "inherited",
            Self::Default => "default",
        }
    }

    /// The origin a name picks.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "explicit" => Some(Self::Explicit),
            "inherited" => Some(Self::Inherited),
            "default" => Some(Self::Default),
            _ => None,
        }
    }
}

/// The source of a value: which node, which property, and how it got there.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Source {
    /// The node the value belongs to.
    pub node: NodeId,
    /// The property's name.
    pub property: PropertyName,
    /// How the value got there.
    pub origin: Origin,
}

impl Source {
    /// The source in one line.
    pub fn render(&self) -> String {
        format!(
            "node {} {} ({})",
            self.node.get(),
            self.property.as_str(),
            self.origin.name()
        )
    }
}

/// A constraint's typed value.
///
/// The property name says *what* is constrained; this says *what value* it holds. The enum is
/// deliberately small: a scalar (a length, a factor, a position or an order), a size pair, a grid
/// cell, and a named choice (alignment or overflow). A value that a selected library layout cannot
/// consume is reported as unsupported, never silently ignored.
#[derive(Clone, Debug)]
pub enum Value {
    /// One number: a length, a factor, a position or an order.
    Scalar(f64),
    /// A width and a height.
    Size {
        /// Width.
        width: f64,
        /// Height.
        height: f64,
    },
    /// A grid cell.
    Cell {
        /// Row, from the top.
        row: u32,
        /// Column, from the left.
        column: u32,
    },
    /// A named choice, such as an alignment or an overflow mode.
    Word(String),
}

impl Value {
    /// The scalar, when this is one.
    pub fn scalar(&self) -> Option<f64> {
        match self {
            Self::Scalar(value) => Some(*value),
            _ => None,
        }
    }

    /// The size, when this is one.
    pub fn size(&self) -> Option<Size> {
        match self {
            Self::Size { width, height } => Some(Size::new(*width, *height)),
            _ => None,
        }
    }

    /// The grid cell, when this is one.
    pub fn cell(&self) -> Option<(u32, u32)> {
        match self {
            Self::Cell { row, column } => Some((*row, *column)),
            _ => None,
        }
    }

    /// The named choice, when this is one.
    pub fn word(&self) -> Option<&str> {
        match self {
            Self::Word(word) => Some(word),
            _ => None,
        }
    }

    /// The value in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Scalar(value) => format!("{value}"),
            Self::Size { width, height } => format!("{width},{height}"),
            Self::Cell { row, column } => format!("{row},{column}"),
            Self::Word(word) => word.clone(),
        }
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Scalar(a), Self::Scalar(b)) => a.to_bits() == b.to_bits(),
            (
                Self::Size {
                    width: aw,
                    height: ah,
                },
                Self::Size {
                    width: bw,
                    height: bh,
                },
            ) => aw.to_bits() == bw.to_bits() && ah.to_bits() == bh.to_bits(),
            (
                Self::Cell {
                    row: ar,
                    column: ac,
                },
                Self::Cell {
                    row: br,
                    column: bc,
                },
            ) => ar == br && ac == bc,
            (Self::Word(a), Self::Word(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for Value {}

/// A constraint on one node, and where it came from.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Constraint {
    /// The node it constrains.
    pub node: NodeId,
    /// The property it names.
    pub property: PropertyName,
    /// Where its value came from.
    pub origin: Origin,
    /// The value it carries, when it has one.
    pub value: Option<Value>,
}

impl Constraint {
    /// A constraint with no value yet.
    pub fn new(node: NodeId, property: PropertyName, origin: Origin) -> Self {
        Self {
            node,
            property,
            origin,
            value: None,
        }
    }

    /// The same constraint carrying a typed value.
    pub fn with_value(mut self, value: Value) -> Self {
        self.value = Some(value);
        self
    }

    /// Where this constraint's value came from.
    pub fn source(&self) -> Source {
        Source {
            node: self.node,
            property: self.property.clone(),
            origin: self.origin,
        }
    }
}

/// A size, in device-independent pixels.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Size {
    /// Width.
    pub width: f64,
    /// Height.
    pub height: f64,
}

impl Size {
    /// A size.
    pub const fn new(width: f64, height: f64) -> Self {
        Self { width, height }
    }
}

/// The device facts a layout may read: the device scale, the text scale, and the smallest area a
/// finger can address.
///
/// These are the library's [`LayoutContext`] fields in the host's vocabulary, so a caller can state
/// "solve for this device" without importing the library into its own types. [`Context::nominal`]
/// is scale 1 with no touch floor; the library's own default additionally reads the platform's text
/// scale, which is not used here because a headless test or CLI run must stay deterministic.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Context {
    /// Scale applied to spacing, padding and margins.
    pub layout_scale: f32,
    /// Scale applied to font and metric sizes.
    pub font_scale: f32,
    /// The smallest touch target, in logical pixels.
    pub min_touch: Size,
}

impl Context {
    /// A context with no scaling and no touch floor.
    pub const fn nominal() -> Self {
        Self {
            layout_scale: 1.0,
            font_scale: 1.0,
            min_touch: Size::new(0.0, 0.0),
        }
    }

    /// A context with the device facts given.
    pub const fn new(layout_scale: f32, font_scale: f32, min_touch: Size) -> Self {
        Self {
            layout_scale,
            font_scale,
            min_touch,
        }
    }

    /// The one scale a layout reads, matching the library's `max(layout_scale, font_scale)`.
    ///
    /// An invalid or non-positive scale is treated as 1, which is the identity a caller that did
    /// not mean to scale anything expects.
    pub fn effective_scale(&self) -> f32 {
        let scale = self.layout_scale.max(self.font_scale);
        if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        }
    }

    /// The library's context for this device.
    fn library(&self) -> LayoutContext {
        LayoutContext {
            layout_scale: if self.layout_scale.is_finite() && self.layout_scale > 0.0 {
                self.layout_scale
            } else {
                1.0
            },
            font_scale: if self.font_scale.is_finite() && self.font_scale > 0.0 {
                self.font_scale
            } else {
                1.0
            },
            min_touch_size: LibrarySize::new(
                library_axis(self.min_touch.width),
                library_axis(self.min_touch.height),
            ),
        }
    }
}

impl Default for Context {
    fn default() -> Self {
        Self::nominal()
    }
}

/// The input one node's layout gets: the space it is offered and the constraints on it.
#[derive(Clone, PartialEq, Debug)]
pub struct LayoutInput {
    /// The node.
    pub node: NodeId,
    /// The space the parent offers.
    pub available: Size,
    /// The constraints on it, each carrying its source.
    pub constraints: Vec<Constraint>,
}

impl LayoutInput {
    /// An input with no constraints.
    pub fn new(node: NodeId, available: Size) -> Self {
        Self {
            node,
            available,
            constraints: Vec::new(),
        }
    }

    /// The same input with one more constraint.
    pub fn with(mut self, constraint: Constraint) -> Self {
        self.constraints.push(constraint);
        self
    }

    /// Where the constraint on a property came from, if there is one.
    pub fn source(&self, property: &PropertyName) -> Option<Source> {
        self.constraints
            .iter()
            .find(|constraint| &constraint.property == property)
            .map(Constraint::source)
    }

    /// The typed value written for a property, if there is one.
    pub fn value(&self, property: &PropertyName) -> Option<&Value> {
        self.constraints
            .iter()
            .find(|constraint| &constraint.property == property)
            .and_then(|constraint| constraint.value.as_ref())
    }
}

/// A computed box.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Computed {
    /// The node.
    pub node: NodeId,
    /// Left.
    pub x: f64,
    /// Top.
    pub y: f64,
    /// Width.
    pub width: f64,
    /// Height.
    pub height: f64,
}

impl Computed {
    /// The box in one line.
    pub fn render(&self) -> String {
        format!(
            "node {} at {},{} {}x{}",
            self.node.get(),
            self.x,
            self.y,
            self.width,
            self.height
        )
    }
}

/// Why a layout could not be computed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Diagnostic {
    /// The node it is about.
    pub node: NodeId,
    /// The property it is about, when one property is at fault.
    pub property: Option<PropertyName>,
    /// What is wrong.
    pub message: String,
    /// The constraint it came from, when there is one.
    pub source: Option<Source>,
}

impl Diagnostic {
    /// A diagnostic about a node.
    pub fn new(node: NodeId, message: impl Into<String>) -> Self {
        Self {
            node,
            property: None,
            message: message.into(),
            source: None,
        }
    }

    /// The same diagnostic, pointed at a constraint's source.
    pub fn at(mut self, source: Source) -> Self {
        self.property = Some(source.property.clone());
        self.source = Some(source);
        self
    }

    /// Where to jump to explain this: the source, when there is one.
    pub fn jump(&self) -> Option<&Source> {
        self.source.as_ref()
    }

    /// The diagnostic in one line.
    pub fn render(&self) -> String {
        match &self.source {
            Some(source) => format!(
                "node {}: {} [{}]",
                self.node.get(),
                self.message,
                source.render()
            ),
            None => format!("node {}: {}", self.node.get(), self.message),
        }
    }
}

/// The one layout engine. The library implements it; the designer does not.
pub trait Engine {
    /// Computes a box for each input, or explains why it cannot.
    fn layout(&self, inputs: &[LayoutInput]) -> Result<Vec<Computed>, Vec<Diagnostic>>;

    /// Computes a box for each input for one device context.
    ///
    /// The default forwards to [`Engine::layout`], which is the right answer for an engine that
    /// has no device channel. [`LibraryEngine`] overrides it so the library's
    /// `arrange_with_context` sees the scale and the touch floor.
    fn layout_with_context(
        &self,
        inputs: &[LayoutInput],
        _context: &Context,
    ) -> Result<Vec<Computed>, Vec<Diagnostic>> {
        self.layout(inputs)
    }
}

/// Who is asking for a layout.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    /// The development-time preview.
    Preview,
    /// The generated code.
    Generation,
}

impl Role {
    /// The role's name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Preview => "preview",
            Self::Generation => "generation",
        }
    }

    /// The role a name picks.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "preview" => Some(Self::Preview),
            "generation" => Some(Self::Generation),
            _ => None,
        }
    }
}

/// What one caller got back: the role, the boxes, and any diagnostics.
#[derive(Clone, PartialEq, Debug)]
pub struct Plan {
    /// Who asked.
    pub role: Role,
    /// The boxes, when the engine could compute them.
    pub computed: Vec<Computed>,
    /// Why it could not, when it could not.
    pub diagnostics: Vec<Diagnostic>,
}

impl Plan {
    /// Whether the plan carries boxes rather than refusals.
    pub fn is_computed(&self) -> bool {
        self.diagnostics.is_empty()
    }

    /// The plan in a report.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "plan ({}): {} box(es), {} diagnostic(s)",
            self.role.name(),
            self.computed.len(),
            self.diagnostics.len()
        )];
        for box_ in &self.computed {
            lines.push(format!("  {}", box_.render()));
        }
        for diagnostic in &self.diagnostics {
            lines.push(format!("  {}", diagnostic.render()));
        }
        lines
    }
}

/// Lays out by asking the one engine. This is the only caller of [`Engine::layout`].
///
/// The role is recorded, not applied: a preview and a generation differ in what they *do* with the
/// boxes, never in how the boxes are found, which is what keeps the two from diverging.
pub fn layout(engine: &dyn Engine, role: Role, inputs: &[LayoutInput]) -> Plan {
    plan_of(role, engine.layout(inputs))
}

/// Lays out for one device context, through the same seam as [`layout`].
pub fn layout_with_context(
    engine: &dyn Engine,
    role: Role,
    inputs: &[LayoutInput],
    context: &Context,
) -> Plan {
    plan_of(role, engine.layout_with_context(inputs, context))
}

/// Wraps an engine's answer in a [`Plan`] under the asking role.
fn plan_of(role: Role, result: Result<Vec<Computed>, Vec<Diagnostic>>) -> Plan {
    match result {
        Ok(computed) => Plan {
            role,
            computed,
            diagnostics: Vec::new(),
        },
        Err(diagnostics) => Plan {
            role,
            computed: Vec::new(),
            diagnostics,
        },
    }
}

/// Which library layout solves a host layout kind.
///
/// The selection is explicit rather than inferred at each call: a host plan says "a row", and
/// this says "`FlexLayout` with `FlexDirection::Row`". The library owns how the children are
/// placed; this enum owns only which library type is asked.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Selection {
    /// A `FlexLayout` along the row axis.
    FlexRow,
    /// A `FlexLayout` along the column axis.
    FlexColumn,
    /// A `StackLayout` in `StackMode::All`, so every layer is placed.
    Stack,
    /// A `GridLayout`; `columns` is `0` for the adapter's square-ish default.
    Grid {
        /// The number of columns, or `0` to choose one from the child count.
        columns: u32,
    },
    /// An `AbsoluteLayout`.
    Absolute,
}

impl Selection {
    /// The library selection a host layout kind maps to.
    pub fn of(kind: LayoutKind) -> Self {
        match kind {
            LayoutKind::Row => Self::FlexRow,
            LayoutKind::Column => Self::FlexColumn,
            LayoutKind::Stack => Self::Stack,
            LayoutKind::Grid => Self::Grid { columns: 0 },
            LayoutKind::Free => Self::Absolute,
        }
    }

    /// The library type this selection asks.
    pub const fn name(self) -> &'static str {
        match self {
            Self::FlexRow | Self::FlexColumn => "FlexLayout",
            Self::Stack => "StackLayout",
            Self::Grid { .. } => "GridLayout",
            Self::Absolute => "AbsoluteLayout",
        }
    }
}

/// The real engine: one of the library's layouts, driven through its hint channel.
///
/// The adapter builds the layout on every call and asks it to place the children inside one
/// container rectangle. Construction is cheap and stateless, which keeps preview and generation
/// from sharing a cached arrangement that could drift between them.
#[derive(Clone, PartialEq, Debug)]
pub struct LibraryEngine {
    selection: Selection,
    context: Context,
    gap: u32,
    padding: u32,
    container: Option<Size>,
}

impl LibraryEngine {
    /// The library layout that matches a host plan.
    pub fn new(kind: LayoutKind) -> Self {
        Self::with_selection(Selection::of(kind))
    }

    /// The library layout named directly, for a plan that needs the choice spelled out.
    pub fn with_selection(selection: Selection) -> Self {
        Self {
            selection,
            context: Context::nominal(),
            gap: 0,
            padding: 0,
            container: None,
        }
    }

    /// The device context [`Engine::layout`] uses.
    pub fn with_context(mut self, context: Context) -> Self {
        self.context = context;
        self
    }

    /// The gap between children, before device scaling.
    pub fn with_gap(mut self, gap: u32) -> Self {
        self.gap = gap;
        self
    }

    /// The padding inside the container, before device scaling.
    pub fn with_padding(mut self, padding: u32) -> Self {
        self.padding = padding;
        self
    }

    /// The container rectangle, in logical pixels, rather than measuring one from the children.
    pub fn with_container(mut self, container: Size) -> Self {
        self.container = Some(container);
        self
    }

    /// Which library layout this engine asks.
    pub fn selection(&self) -> Selection {
        self.selection
    }

    /// Solves the inputs with the library under `context`.
    fn solve(
        &self,
        inputs: &[LayoutInput],
        context: Context,
    ) -> Result<Vec<Computed>, Vec<Diagnostic>> {
        let scale = context.effective_scale();
        let mut entries: Vec<Entry> = Vec::with_capacity(inputs.len());
        let mut diagnostics = Vec::new();
        let mut gap_requests: Vec<(NodeId, u32, Source)> = Vec::new();
        let mut padding_requests: Vec<(NodeId, u32, Source)> = Vec::new();

        for input in inputs {
            if entries.iter().any(|entry| entry.node == input.node) {
                diagnostics.push(Diagnostic::new(
                    input.node,
                    "the same node was offered space more than once",
                ));
                continue;
            }
            let Some(available) = library_size(input.available) else {
                diagnostics.push(Diagnostic::new(
                    input.node,
                    format!(
                        "offered size {},{} is not a finite non-negative size",
                        input.available.width, input.available.height
                    ),
                ));
                continue;
            };

            let mut seen: std::collections::BTreeMap<&str, &Value> =
                std::collections::BTreeMap::new();
            let mut minimum: Option<LibrarySize> = None;
            let mut maximum: Option<LibrarySize> = None;
            let mut preferred: Option<LibrarySize> = None;
            let mut exact_width: Option<u32> = None;
            let mut exact_height: Option<u32> = None;
            let mut grow: Option<u32> = None;
            let mut margin: Option<u32> = None;
            let mut position_x: Option<i32> = None;
            let mut position_y: Option<i32> = None;
            let mut cell: Option<(u32, u32)> = None;
            let mut align: Option<String> = None;
            let mut order: Option<i32> = None;

            for constraint in &input.constraints {
                let property = constraint.property.as_str();
                let source = constraint.source();
                let Some(value) = constraint.value.as_ref() else {
                    // A bare `grow` is the legacy spelling and means weight one. Every other
                    // property needs its value, or it is a name with nothing to feed.
                    if property == "grow" {
                        grow = Some(grow.unwrap_or(1));
                    } else if is_known_property(property) {
                        diagnostics.push(
                            Diagnostic::new(
                                input.node,
                                format!(
                                    "the {property} constraint carries no value; the library needs a value, not only a name"
                                ),
                            )
                            .at(source),
                        );
                    }
                    continue;
                };

                if let Some(previous) = seen.insert(property, value) {
                    if previous != value {
                        diagnostics.push(
                            Diagnostic::new(
                                input.node,
                                format!(
                                    "the node has two different {property} values ({} and {}); the library accepts one",
                                    previous.render(),
                                    value.render()
                                ),
                            )
                            .at(source),
                        );
                    }
                    continue;
                }

                if let Some(reason) = unsupported_reason(self.selection, property) {
                    diagnostics.push(
                        Diagnostic::new(
                            input.node,
                            format!(
                                "the {property} constraint is unsupported by {}: {reason}",
                                self.selection.name()
                            ),
                        )
                        .at(source),
                    );
                    continue;
                }

                let mapped: Result<(), String> = match property {
                    "min" => value_size(value).map(|size| minimum = Some(size)),
                    "max" => value_size(value).map(|size| maximum = Some(size)),
                    "preferred" => value_size(value).map(|size| preferred = Some(size)),
                    "width" => value_pixels(value).map(|value| exact_width = Some(value)),
                    "height" => value_pixels(value).map(|value| exact_height = Some(value)),
                    "grow" => value_factor(value).map(|value| grow = Some(value)),
                    "margin" => value_pixels(value).map(|value| margin = Some(value)),
                    "gap" => value_pixels(value)
                        .map(|value| gap_requests.push((input.node, value, source.clone()))),
                    "padding" => value_pixels(value)
                        .map(|value| padding_requests.push((input.node, value, source.clone()))),
                    "x" => value_i32(value).map(|value| position_x = Some(value)),
                    "y" => value_i32(value).map(|value| position_y = Some(value)),
                    "cell" => value
                        .cell()
                        .ok_or_else(|| format!("expected <row>,<column>, got {}", value.render()))
                        .map(|value| cell = Some(value)),
                    "align" => value
                        .word()
                        .filter(|word| align_items(word).is_some())
                        .map(|word| align = Some(word.to_owned()))
                        .ok_or_else(|| {
                            format!(
                                "expected start, centre, end or stretch, got {}",
                                value.render()
                            )
                        }),
                    "order" => value_f64(value).map(|value| {
                        order = Some(value.round().clamp(i32::MIN as f64, i32::MAX as f64) as i32);
                    }),
                    other => Err(format!(
                        "the {other} constraint is not one the library layout can consume"
                    )),
                };
                if let Err(reason) = mapped {
                    diagnostics.push(
                        Diagnostic::new(input.node, format!("{property}: {reason}")).at(source),
                    );
                }
            }

            let (mut min_width, mut min_height) = (0, 0);
            let (mut max_width, mut max_height) = (u32::MAX, u32::MAX);
            let (mut pref_width, mut pref_height) = (available.width, available.height);
            if let Some(size) = minimum {
                (min_width, min_height) = (size.width, size.height);
            }
            if let Some(size) = maximum {
                (max_width, max_height) = (size.width, size.height);
            }
            if let Some(size) = preferred {
                (pref_width, pref_height) = (size.width, size.height);
            }
            if let Some(width) = exact_width {
                (min_width, pref_width, max_width) = (width, width, width);
            }
            if let Some(height) = exact_height {
                (min_height, pref_height, max_height) = (height, height, height);
            }
            let hints = Hints {
                width: AxisHints::new(min_width, pref_width, max_width),
                height: AxisHints::new(min_height, pref_height, max_height),
            };
            let mut params = LayoutParams::new();
            if let Some(margin) = margin {
                params.margins = EdgeOffsets::all(scale_axis(margin, scale));
            }
            if let Some(weight) = grow.filter(|weight| *weight > 0) {
                params.fill = true;
                params.stretch = weight;
            }
            let position = if position_x.is_some() || position_y.is_some() {
                Some((position_x.unwrap_or(0), position_y.unwrap_or(0)))
            } else {
                None
            };
            entries.push(Entry {
                node: input.node,
                hints,
                params,
                position,
                cell,
                align,
                order,
                preferred: LibrarySize::new(hints.width.pref, hints.height.pref),
                grow: grow.unwrap_or(0),
            });
        }

        let gap = resolve_layout_value("gap", self.gap, &gap_requests, &mut diagnostics);
        let padding =
            resolve_layout_value("padding", self.padding, &padding_requests, &mut diagnostics);
        if !diagnostics.is_empty() {
            return Err(diagnostics);
        }
        if entries.is_empty() {
            return Ok(Vec::new());
        }

        let rect = match self.container {
            Some(container) => match library_size(container) {
                Some(size) => Rect::new(0, 0, size.width, size.height),
                None => {
                    return Err(vec![Diagnostic::new(
                        entries[0].node,
                        format!(
                            "the container size {},{} is not a finite non-negative size",
                            container.width, container.height
                        ),
                    )]);
                }
            },
            None => {
                let size = self.container_for(&entries, scale, gap, padding);
                Rect::new(0, 0, size.width, size.height)
            }
        };

        let children: Vec<ChildInfo> = entries
            .iter()
            .map(|entry| ChildInfo::new(entry.node.get(), entry.hints).with_params(entry.params))
            .collect();

        let scaled_gap = scale_axis(gap, scale);
        let scaled_padding = scale_axis(padding, scale);
        let library_context = context.library();
        let placed = match self.selection {
            Selection::FlexRow | Selection::FlexColumn => {
                let direction = if self.selection == Selection::FlexRow {
                    FlexDirection::Row
                } else {
                    FlexDirection::Column
                };
                let mut flex = FlexLayout::with_params(
                    direction,
                    FlexWrap::NoWrap,
                    JustifyContent::FlexStart,
                    AlignItems::Stretch,
                    as_i32(gap),
                    as_i32(padding),
                );
                for (child, entry) in children.iter().zip(&entries) {
                    flex.add_widget(child.id, entry.grow);
                }
                for (child, entry) in children.iter().zip(&entries) {
                    if let Some(word) = &entry.align
                        && let Some(items) = flex
                            .items_mut()
                            .iter_mut()
                            .find(|item| item.widget_id == Some(child.id))
                    {
                        items.align_self = align_items(word);
                    }
                    if let Some(order) = entry.order {
                        flex.set_item_order(child.id, order);
                    }
                }
                emit(&flex, rect, &children, &library_context)
            }
            Selection::Stack => {
                let mut stack = StackLayout::new();
                stack.set_mode(StackMode::All);
                for child in &children {
                    stack.add_widget(child.id, 0);
                }
                emit(&stack, rect, &children, &library_context)
            }
            Selection::Grid { columns } => {
                let (rows, columns) = grid_shape(entries.len(), columns);
                let mut grid = GridLayout::new(rows, columns, scaled_gap, scaled_padding);
                for (index, entry) in entries.iter().enumerate() {
                    let (row, column) = entry.cell.unwrap_or_else(|| {
                        (
                            (index / columns as usize) as u32,
                            (index % columns as usize) as u32,
                        )
                    });
                    let id = entry.node.get();
                    grid.set_widget(row, column, id);
                    if let Some(word) = &entry.align
                        && let Some(alignment) = grid_alignment(word)
                    {
                        grid.set_cell_alignment(id, Some(alignment));
                    }
                }
                emit(&grid, rect, &children, &library_context)
            }
            Selection::Absolute => {
                let mut absolute = AbsoluteLayout::new();
                for (index, entry) in entries.iter().enumerate() {
                    absolute.add_widget(entry.node.get(), 0);
                    let (x, y) = entry.position.unwrap_or((0, 0));
                    let mut position = AbsolutePosition::new(x, y);
                    position.width = Some(entry.hints.width.pref);
                    position.height = Some(entry.hints.height.pref);
                    position.margins = (
                        entry.params.margins.left as i32,
                        entry.params.margins.top as i32,
                        entry.params.margins.right as i32,
                        entry.params.margins.bottom as i32,
                    );
                    let _ = absolute.set_position(index, position);
                }
                emit(&absolute, rect, &children, &library_context)
            }
        };

        let mut computed = Vec::with_capacity(entries.len());
        let mut missing = Vec::new();
        for entry in &entries {
            match placed.iter().rev().find(|(id, _)| *id == entry.node.get()) {
                Some((_, child)) => computed.push(Computed {
                    node: entry.node,
                    x: child.x as f64,
                    y: child.y as f64,
                    width: child.width as f64,
                    height: child.height as f64,
                }),
                None => missing.push(Diagnostic::new(
                    entry.node,
                    format!(
                        "the {} library layout placed no box for this node",
                        self.selection.name()
                    ),
                )),
            }
        }
        if missing.is_empty() {
            Ok(computed)
        } else {
            Err(missing)
        }
    }

    /// The smallest container that can pay every child its preferred extent, plus the gap and
    /// padding the inputs name (or the engine's configured values when they name none).
    ///
    /// This is an outer measurement, not a placement: the children's boxes are still the
    /// library's. It exists because [`LayoutInput`] names no container rectangle of its own.
    fn container_for(&self, entries: &[Entry], scale: f32, gap: u32, padding: u32) -> LibrarySize {
        let gap = scale_axis(gap, scale);
        let padding = scale_axis(padding, scale);
        let count = entries.len() as u32;
        let max_width = entries
            .iter()
            .map(|entry| entry.preferred.width)
            .max()
            .unwrap_or(0);
        let max_height = entries
            .iter()
            .map(|entry| entry.preferred.height)
            .max()
            .unwrap_or(0);
        let sum_widths = entries.iter().fold(0u32, |total, entry| {
            total.saturating_add(entry.preferred.width)
        });
        let sum_heights = entries.iter().fold(0u32, |total, entry| {
            total.saturating_add(entry.preferred.height)
        });
        let between =
            |sum: u32| -> u32 { sum.saturating_add(gap.saturating_mul(count.saturating_sub(1))) };
        let both_sides = |extent: u32| -> u32 { extent.saturating_add(padding.saturating_mul(2)) };
        match self.selection {
            Selection::FlexRow => {
                LibrarySize::new(both_sides(between(sum_widths)), both_sides(max_height))
            }
            Selection::FlexColumn => {
                LibrarySize::new(both_sides(max_width), both_sides(between(sum_heights)))
            }
            Selection::Grid { columns } => {
                let (rows, columns) = grid_shape(entries.len(), columns);
                LibrarySize::new(
                    both_sides(
                        max_width
                            .saturating_mul(columns)
                            .saturating_add(gap.saturating_mul(columns.saturating_sub(1))),
                    ),
                    both_sides(
                        max_height
                            .saturating_mul(rows)
                            .saturating_add(gap.saturating_mul(rows.saturating_sub(1))),
                    ),
                )
            }
            // Neither layout consumes padding: a stack repeats the container rect and the
            // absolute layout places from the origin.
            Selection::Stack | Selection::Absolute => LibrarySize::new(max_width, max_height),
        }
    }
}

impl Engine for LibraryEngine {
    fn layout(&self, inputs: &[LayoutInput]) -> Result<Vec<Computed>, Vec<Diagnostic>> {
        self.solve(inputs, self.context)
    }

    fn layout_with_context(
        &self,
        inputs: &[LayoutInput],
        context: &Context,
    ) -> Result<Vec<Computed>, Vec<Diagnostic>> {
        self.solve(inputs, *context)
    }
}

/// One validated child: everything the library's hint and placement channels can carry.
struct Entry {
    node: NodeId,
    hints: Hints,
    params: LayoutParams,
    position: Option<(i32, i32)>,
    cell: Option<(u32, u32)>,
    align: Option<String>,
    order: Option<i32>,
    preferred: LibrarySize,
    grow: u32,
}

/// Runs one library layout's hint+context entry and collects the placements.
fn emit(
    layout: &dyn LibraryLayout,
    rect: Rect,
    children: &[ChildInfo],
    context: &LayoutContext,
) -> Vec<(ObjectId, Rect)> {
    let mut placed = Vec::new();
    layout.arrange_with_context(rect, children, context, &mut |id, child| {
        placed.push((id, child));
    });
    placed
}

/// Whether a property is one the layout contract knows by name.
fn is_known_property(name: &str) -> bool {
    matches!(
        name,
        "min"
            | "max"
            | "preferred"
            | "width"
            | "height"
            | "grow"
            | "margin"
            | "gap"
            | "padding"
            | "x"
            | "y"
            | "cell"
            | "align"
            | "order"
            | "overflow"
    )
}

/// Why a selected library layout cannot carry a property's value, or `None` when it can.
fn unsupported_reason(selection: Selection, property: &str) -> Option<&'static str> {
    match selection {
        Selection::FlexRow | Selection::FlexColumn => match property {
            "min" | "max" | "preferred" | "width" | "height" | "grow" | "margin" | "gap"
            | "padding" | "align" | "order" => None,
            "x" | "y" => Some(
                "a flex layout places children in flow; free positions belong to the absolute layout",
            ),
            "cell" => Some("only the grid layout has cells"),
            "overflow" => Some("the library layout has no overflow channel"),
            _ => Some("the library flex layout has no channel for this constraint"),
        },
        Selection::Grid { .. } => match property {
            "preferred" | "gap" | "padding" | "align" | "cell" => None,
            "min" | "max" => Some(
                "GridLayout reads only the preferred hint, so a min/max value would be ignored",
            ),
            "width" | "height" => {
                Some("GridLayout fills a cell, so an exact width/height has no channel")
            }
            "margin" => Some("GridLayout does not read per-child margins"),
            "grow" => Some("GridLayout has no per-child stretch channel"),
            "order" => Some("GridLayout places by cell, not by order"),
            "x" | "y" => Some("the grid places children in cells, not at free coordinates"),
            "overflow" => Some("the library layout has no overflow channel"),
            _ => Some("the library grid layout has no channel for this constraint"),
        },
        Selection::Stack => {
            Some("StackLayout ignores the hint channel; only its layer rect is honoured")
        }
        Selection::Absolute => match property {
            "x" | "y" | "width" | "height" | "preferred" | "margin" => None,
            "min" | "max" => {
                Some("AbsoluteLayout places an explicit size and has no min/max channel")
            }
            "grow" => Some("AbsoluteLayout has no stretch channel"),
            "gap" | "padding" => Some("AbsoluteLayout has no spacing channel"),
            "align" => {
                Some("a free layout's alignment is an x/y value; no anchor is selected here")
            }
            "order" => Some("AbsoluteLayout places by position, not by order"),
            "cell" => Some("only the grid layout has cells"),
            "overflow" => Some("the library layout has no overflow channel"),
            _ => Some("the library absolute layout has no channel for this constraint"),
        },
    }
}

/// A constraint value as a library size, or the reason it cannot be one.
fn value_size(value: &Value) -> Result<LibrarySize, String> {
    let size = value
        .size()
        .ok_or_else(|| format!("expected <width>,<height>, got {}", value.render()))?;
    library_size(size).ok_or_else(|| {
        format!(
            "size {},{} is not a finite non-negative size",
            size.width, size.height
        )
    })
}

/// A constraint value as a non-negative pixel count, or the reason it cannot be one.
fn value_pixels(value: &Value) -> Result<u32, String> {
    let number = value
        .scalar()
        .ok_or_else(|| format!("expected a number, got {}", value.render()))?;
    if number.is_finite() && number >= 0.0 && number <= u32::MAX as f64 {
        Ok(number.round() as u32)
    } else {
        Err(format!("{number} is not a finite non-negative pixel count"))
    }
}

/// A constraint value as a whole-number stretch or order factor.
fn value_factor(value: &Value) -> Result<u32, String> {
    let number = value
        .scalar()
        .ok_or_else(|| format!("expected a number, got {}", value.render()))?;
    if number.is_finite() && number >= 0.0 && number <= u32::MAX as f64 && number.fract() == 0.0 {
        Ok(number as u32)
    } else {
        Err(format!(
            "{number} is not a finite non-negative whole-number factor"
        ))
    }
}

/// A constraint value as a finite number.
fn value_f64(value: &Value) -> Result<f64, String> {
    value
        .scalar()
        .filter(|number| number.is_finite())
        .ok_or_else(|| format!("expected a finite number, got {}", value.render()))
}

/// A constraint value as a signed device-independent coordinate.
fn value_i32(value: &Value) -> Result<i32, String> {
    let number = value_f64(value)?;
    Ok(number.round().clamp(i32::MIN as f64, i32::MAX as f64) as i32)
}

/// The flex cross-axis alignment a word picks.
fn align_items(word: &str) -> Option<AlignItems> {
    match word {
        "start" => Some(AlignItems::FlexStart),
        "centre" | "center" => Some(AlignItems::Center),
        "end" => Some(AlignItems::FlexEnd),
        "stretch" => Some(AlignItems::Stretch),
        _ => None,
    }
}

/// The grid cell alignment a word picks. `stretch` is the grid's default fill, so it needs no
/// alignment entry.
fn grid_alignment(word: &str) -> Option<Alignment> {
    match word {
        "start" => Some(Alignment::Left),
        "centre" | "center" => Some(Alignment::Center),
        "end" => Some(Alignment::Right),
        _ => None,
    }
}

/// One container-level value from the per-node requests, or a conflict diagnostic.
fn resolve_layout_value(
    name: &str,
    configured: u32,
    requests: &[(NodeId, u32, Source)],
    diagnostics: &mut Vec<Diagnostic>,
) -> u32 {
    let mut value = configured;
    let mut first: Option<&Source> = None;
    for (node, requested, source) in requests {
        match first {
            None => {
                value = *requested;
                first = Some(source);
            }
            Some(seen) if value != *requested => diagnostics.push(
                Diagnostic::new(
                    *node,
                    format!(
                        "the {name} value {requested} conflicts with {value} from {}",
                        seen.render()
                    ),
                )
                .at(source.clone()),
            ),
            Some(_) => {}
        }
    }
    value
}

/// A host size as a library size, or `None` when an axis cannot be represented.
fn library_size(size: Size) -> Option<LibrarySize> {
    fn axis(value: f64) -> Option<u32> {
        if value.is_finite() && value >= 0.0 && value <= u32::MAX as f64 {
            Some(value.round() as u32)
        } else {
            None
        }
    }
    Some(LibrarySize::new(axis(size.width)?, axis(size.height)?))
}

/// A device pixel extent as a library axis.
fn library_axis(value: f64) -> u32 {
    if value.is_finite() && value > 0.0 {
        value.round().min(u32::MAX as f64) as u32
    } else {
        0
    }
}

/// One axis scaled the way the library scales spacing and padding.
fn scale_axis(value: u32, scale: f32) -> u32 {
    if !scale.is_finite() || scale <= 0.0 {
        return value;
    }
    (value as f32 * scale).round().clamp(0.0, u32::MAX as f32) as u32
}

/// A library dimension as the `i32` a layout constructor takes.
fn as_i32(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

/// The row/column shape of a row-major grid.
fn grid_shape(count: usize, requested_columns: u32) -> (u32, u32) {
    if count == 0 {
        return (1, 1);
    }
    let count = count as u32;
    let columns = if requested_columns == 0 {
        (count as f64).sqrt().ceil() as u32
    } else {
        requested_columns
    };
    let columns = columns.max(1).min(count);
    (count.div_ceil(columns), columns)
}

#[cfg(test)]
mod tests {
    use super::{
        Computed, Constraint, Context, Diagnostic, Engine, LayoutInput, LibraryEngine, Origin,
        Role, Selection, Size, Source, Value, layout, layout_with_context,
    };
    use crate::app::layout::layout_choice::LayoutKind;
    use crate::contracts::identity::NodeId;
    use crate::contracts::model::PropertyName;
    use std::cell::Cell;

    fn node(value: u64) -> NodeId {
        NodeId::from_raw(value)
    }

    fn name(text: &str) -> PropertyName {
        PropertyName::new(text).expect("a name")
    }

    /// A stand-in engine: it answers the boxes it was told, and diagnoses a node it was not told.
    struct Supplied {
        boxes: Vec<Computed>,
        calls: Cell<usize>,
    }

    impl Supplied {
        fn new(boxes: Vec<Computed>) -> Self {
            Self {
                boxes,
                calls: Cell::new(0),
            }
        }
    }

    impl Engine for Supplied {
        fn layout(&self, inputs: &[LayoutInput]) -> Result<Vec<Computed>, Vec<Diagnostic>> {
            self.calls.set(self.calls.get() + 1);
            let mut computed = Vec::new();
            let mut diagnostics = Vec::new();
            for input in inputs {
                match self.boxes.iter().find(|box_| box_.node == input.node) {
                    Some(box_) => computed.push(*box_),
                    None => diagnostics.push(Diagnostic::new(
                        input.node,
                        "the engine was given no box for this node",
                    )),
                }
            }
            if diagnostics.is_empty() {
                Ok(computed)
            } else {
                Err(diagnostics)
            }
        }
    }

    #[test]
    fn the_preview_and_the_generation_get_the_same_boxes_from_one_engine() {
        let box_ = Computed {
            node: node(1),
            x: 10.0,
            y: 20.0,
            width: 100.0,
            height: 40.0,
        };
        let engine = Supplied::new(vec![box_]);
        let inputs = [LayoutInput::new(node(1), Size::new(400.0, 300.0))];

        let preview = layout(&engine, Role::Preview, &inputs);
        let generation = layout(&engine, Role::Generation, &inputs);

        assert_eq!(preview.computed, generation.computed);
        assert_eq!(preview.diagnostics, generation.diagnostics);
        assert_ne!(preview.role, generation.role);
        assert_eq!(engine.calls.get(), 2, "both callers asked the one engine");
    }

    #[test]
    fn a_refusal_is_the_same_for_both_callers() {
        let engine = Supplied::new(Vec::new());
        let inputs = [LayoutInput::new(node(7), Size::new(10.0, 10.0))];
        let preview = layout(&engine, Role::Preview, &inputs);
        let generation = layout(&engine, Role::Generation, &inputs);
        assert!(!preview.is_computed());
        assert_eq!(preview.diagnostics, generation.diagnostics);
    }

    #[test]
    fn a_constraint_carries_its_source() {
        let constraint = Constraint::new(node(2), name("width"), Origin::Inherited);
        let source = constraint.source();
        assert_eq!(source.node, node(2));
        assert_eq!(source.property.as_str(), "width");
        assert_eq!(source.origin, Origin::Inherited);
        assert!(source.render().contains("inherited"));
    }

    #[test]
    fn an_input_answers_where_a_propertys_value_came_from() {
        let input = LayoutInput::new(node(1), Size::new(80.0, 24.0))
            .with(Constraint::new(node(1), name("width"), Origin::Explicit))
            .with(Constraint::new(node(1), name("height"), Origin::Default));
        let width = input.source(&name("width")).expect("a source");
        assert_eq!(width.origin, Origin::Explicit);
        assert_eq!(
            input.source(&name("height")).map(|s| s.origin),
            Some(Origin::Default)
        );
        assert!(input.source(&name("gap")).is_none());
    }

    #[test]
    fn a_diagnostic_can_be_jumped_to_its_constraint() {
        let source = Source {
            node: node(3),
            property: name("min-width"),
            origin: Origin::Explicit,
        };
        let diagnostic = Diagnostic::new(node(3), "min-width exceeds max-width").at(source.clone());
        assert_eq!(diagnostic.jump(), Some(&source));
        assert!(diagnostic.render().contains("min-width"));
        assert_eq!(
            diagnostic.property.as_ref().map(PropertyName::as_str),
            Some("min-width")
        );
    }

    #[test]
    fn a_diagnostic_without_a_source_has_no_jump_target() {
        let diagnostic = Diagnostic::new(node(4), "the engine refused");
        assert!(diagnostic.jump().is_none());
        assert_eq!(diagnostic.render(), "node 4: the engine refused");
    }

    #[test]
    fn the_origin_names_round_trip() {
        for origin in [Origin::Explicit, Origin::Inherited, Origin::Default] {
            assert_eq!(Origin::from_name(origin.name()), Some(origin));
        }
        assert!(Origin::from_name("guessed").is_none());
    }

    #[test]
    fn the_role_names_round_trip() {
        for role in [Role::Preview, Role::Generation] {
            assert_eq!(Role::from_name(role.name()), Some(role));
        }
        assert!(Role::from_name("other").is_none());
    }

    #[test]
    fn the_library_solves_a_box_list() {
        let engine = LibraryEngine::new(LayoutKind::Row);
        assert_eq!(engine.selection(), Selection::FlexRow);
        assert_eq!(engine.selection().name(), "FlexLayout");
        let inputs = [
            LayoutInput::new(node(1), Size::new(30.0, 20.0)),
            LayoutInput::new(node(2), Size::new(20.0, 10.0)),
        ];

        let computed = engine
            .layout(&inputs)
            .expect("the library solves two boxes");

        assert_eq!(
            computed,
            vec![
                Computed {
                    node: node(1),
                    x: 0.0,
                    y: 0.0,
                    width: 30.0,
                    height: 20.0,
                },
                // The row places the second child after the first; the cross axis is the
                // child's own preferred height here, which is the library's placement rule.
                Computed {
                    node: node(2),
                    x: 30.0,
                    y: 0.0,
                    width: 20.0,
                    height: 10.0,
                },
            ]
        );
    }

    #[test]
    fn preview_and_generation_get_the_same_library_boxes() {
        let engine = LibraryEngine::new(LayoutKind::Column);
        let inputs = [
            LayoutInput::new(node(1), Size::new(40.0, 10.0)),
            LayoutInput::new(node(2), Size::new(10.0, 20.0)),
        ];

        let preview = layout(&engine, Role::Preview, &inputs);
        let generation = layout(&engine, Role::Generation, &inputs);

        assert!(preview.is_computed());
        assert!(!preview.computed.is_empty());
        assert_eq!(preview.computed, generation.computed);
        assert_ne!(preview.role, generation.role);
    }

    #[test]
    fn a_context_aware_call_applies_the_device_scale() {
        let engine = LibraryEngine::new(LayoutKind::Row).with_gap(4);
        let inputs = [
            LayoutInput::new(node(1), Size::new(10.0, 10.0)),
            LayoutInput::new(node(2), Size::new(10.0, 10.0)),
        ];

        let nominal = layout(&engine, Role::Preview, &inputs);
        let scaled = layout_with_context(
            &engine,
            Role::Preview,
            &inputs,
            &Context::new(2.0, 1.0, Size::new(0.0, 0.0)),
        );

        assert_eq!(nominal.computed[1].x, 14.0, "10 + the nominal gap of 4");
        assert_eq!(scaled.computed[1].x, 18.0, "10 + the scaled gap of 8");
        assert_ne!(nominal.computed, scaled.computed);
        assert_ne!(nominal.computed, scaled.computed);
    }

    #[test]
    fn a_context_aware_call_applies_the_touch_floor() {
        let engine = LibraryEngine::new(LayoutKind::Row);
        let inputs = [LayoutInput::new(node(1), Size::new(10.0, 10.0))];

        let nominal = layout(&engine, Role::Preview, &inputs);
        let touched = layout_with_context(
            &engine,
            Role::Preview,
            &inputs,
            &Context::new(1.0, 1.0, Size::new(44.0, 44.0)),
        );

        assert_eq!(nominal.computed[0].width, 10.0);
        assert_eq!(nominal.computed[0].height, 10.0);
        assert_eq!(touched.computed[0].width, 44.0);
        assert_eq!(touched.computed[0].height, 44.0);
    }

    #[test]
    fn a_grow_constraint_reaches_the_library_stretch_channel() {
        let engine = LibraryEngine::new(LayoutKind::Row).with_container(Size::new(100.0, 20.0));
        let mut second = LayoutInput::new(node(2), Size::new(10.0, 10.0));
        second
            .constraints
            .push(Constraint::new(node(2), name("grow"), Origin::Explicit));
        let inputs = [LayoutInput::new(node(1), Size::new(10.0, 10.0)), second];

        let computed = engine.layout(&inputs).expect("the library solves the row");

        assert_eq!(
            computed[0].width, 10.0,
            "a child without grow keeps its size"
        );
        assert_eq!(computed[1].width, 90.0, "grow takes the remaining room");
    }

    #[test]
    fn an_unsized_input_is_refused_by_the_library_engine() {
        let engine = LibraryEngine::new(LayoutKind::Row);
        let plan = layout(
            &engine,
            Role::Preview,
            &[LayoutInput::new(node(1), Size::new(-1.0, 10.0))],
        );
        assert!(!plan.is_computed());
        assert_eq!(plan.diagnostics.len(), 1);
        assert!(plan.diagnostics[0].message.contains("finite"));
    }

    #[test]
    fn the_grid_selection_is_explicit_and_fills_row_major() {
        let engine = LibraryEngine::with_selection(Selection::Grid { columns: 2 });
        let inputs = [
            LayoutInput::new(node(1), Size::new(10.0, 10.0)),
            LayoutInput::new(node(2), Size::new(10.0, 10.0)),
            LayoutInput::new(node(3), Size::new(10.0, 10.0)),
            LayoutInput::new(node(4), Size::new(10.0, 10.0)),
        ];

        let computed = engine.layout(&inputs).expect("the grid places four cells");
        let positions: Vec<(u64, f64, f64)> = computed
            .iter()
            .map(|box_| (box_.node.get(), box_.x, box_.y))
            .collect();
        assert_eq!(
            positions,
            vec![
                (1, 0.0, 0.0),
                (2, 10.0, 0.0),
                (3, 0.0, 10.0),
                (4, 10.0, 10.0),
            ]
        );
    }

    fn valued(input: LayoutInput, property: &str, value: Value) -> LayoutInput {
        let constraint =
            Constraint::new(input.node, name(property), Origin::Explicit).with_value(value);
        input.with(constraint)
    }

    #[test]
    fn a_minimum_constraint_reaches_the_library_hint_floor() {
        let engine = LibraryEngine::new(LayoutKind::Row).with_container(Size::new(100.0, 20.0));
        let inputs = [valued(
            LayoutInput::new(node(1), Size::new(10.0, 10.0)),
            "min",
            Value::Size {
                width: 30.0,
                height: 12.0,
            },
        )];

        let computed = engine.layout(&inputs).expect("the floor is representable");
        assert_eq!(computed[0].width, 30.0, "the minimum width is the floor");
        assert_eq!(computed[0].height, 12.0, "the minimum height is the floor");
    }

    #[test]
    fn a_preferred_constraint_reaches_the_library_hint() {
        let engine = LibraryEngine::new(LayoutKind::Row).with_container(Size::new(100.0, 20.0));
        let inputs = [valued(
            LayoutInput::new(node(1), Size::new(10.0, 10.0)),
            "preferred",
            Value::Size {
                width: 40.0,
                height: 15.0,
            },
        )];

        let computed = engine
            .layout(&inputs)
            .expect("the preferred size is representable");
        assert_eq!(computed[0].width, 40.0);
        assert_eq!(computed[0].height, 15.0);
    }

    #[test]
    fn a_width_constraint_pins_a_flow_child() {
        let engine = LibraryEngine::new(LayoutKind::Row).with_container(Size::new(100.0, 20.0));
        let inputs = [valued(
            LayoutInput::new(node(1), Size::new(10.0, 10.0)),
            "width",
            Value::Scalar(30.0),
        )];

        let computed = engine.layout(&inputs).expect("the width is representable");
        assert_eq!(computed[0].width, 30.0);
    }

    #[test]
    fn a_margin_constraint_reaches_the_child_params() {
        let engine = LibraryEngine::new(LayoutKind::Row).with_container(Size::new(100.0, 20.0));
        let inputs = [
            valued(
                LayoutInput::new(node(1), Size::new(10.0, 10.0)),
                "margin",
                Value::Scalar(4.0),
            ),
            LayoutInput::new(node(2), Size::new(10.0, 10.0)),
        ];

        let computed = engine.layout(&inputs).expect("the margin is representable");
        assert_eq!(computed[1].x, 18.0, "4 + 10 + 4 before the second child");
    }

    #[test]
    fn a_gap_constraint_reaches_the_container_gap() {
        let engine = LibraryEngine::new(LayoutKind::Row);
        let inputs = [
            valued(
                LayoutInput::new(node(1), Size::new(10.0, 10.0)),
                "gap",
                Value::Scalar(5.0),
            ),
            LayoutInput::new(node(2), Size::new(10.0, 10.0)),
        ];

        let computed = engine.layout(&inputs).expect("the gap is representable");
        assert_eq!(computed[1].x, 15.0, "10 + the requested gap of 5");
    }

    #[test]
    fn conflicting_gap_values_are_refused() {
        let engine = LibraryEngine::new(LayoutKind::Row);
        let inputs = [
            valued(
                LayoutInput::new(node(1), Size::new(10.0, 10.0)),
                "gap",
                Value::Scalar(5.0),
            ),
            valued(
                LayoutInput::new(node(2), Size::new(10.0, 10.0)),
                "gap",
                Value::Scalar(8.0),
            ),
        ];

        let plan = layout(&engine, Role::Preview, &inputs);
        assert!(!plan.is_computed());
        assert!(
            plan.diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("conflicts"))
        );
    }

    #[test]
    fn free_x_and_y_values_reach_the_absolute_position() {
        let engine = LibraryEngine::new(LayoutKind::Free).with_container(Size::new(100.0, 100.0));
        let input = LayoutInput::new(node(1), Size::new(10.0, 10.0));
        let input = valued(input, "x", Value::Scalar(20.0));
        let input = valued(input, "y", Value::Scalar(30.0));

        let computed = engine
            .layout(&[input])
            .expect("the absolute layout places the position");
        assert_eq!(computed[0].x, 20.0);
        assert_eq!(computed[0].y, 30.0);
    }

    #[test]
    fn a_grid_cell_places_a_node_explicitly() {
        let engine = LibraryEngine::with_selection(Selection::Grid { columns: 2 })
            .with_container(Size::new(40.0, 40.0));
        let input = LayoutInput::new(node(1), Size::new(10.0, 10.0));
        let input = valued(input, "cell", Value::Cell { row: 1, column: 1 });

        let computed = engine.layout(&[input]).expect("the cell is addressable");
        assert_eq!((computed[0].x, computed[0].y), (20.0, 20.0));
    }

    #[test]
    fn an_unsupported_value_is_refused_at_its_node() {
        let engine = LibraryEngine::new(LayoutKind::Row);
        let input = LayoutInput::new(node(1), Size::new(10.0, 10.0));
        let input = valued(input, "x", Value::Scalar(20.0));

        let plan = layout(&engine, Role::Preview, &[input]);
        assert!(!plan.is_computed());
        assert_eq!(plan.diagnostics[0].node, node(1));
        assert!(plan.diagnostics[0].message.contains("unsupported"));
        assert!(
            plan.diagnostics[0]
                .jump()
                .is_some_and(|source| source.property.as_str() == "x")
        );
    }
}
