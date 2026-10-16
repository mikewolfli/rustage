// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The viewport and its design aids: pan, zoom, fit, actual size, grid, rulers and guides
//! (task T09.05).
//!
//! The acceptance line is *"the design aids do not pollute the generated model"*. The aids are
//! everything a person switches on in order to work — a pan offset, a zoom, a grid, ruler ticks,
//! alignment guides — and none of it belongs to the document. The module keeps that true by
//! construction rather than by discipline:
//!
//! * **The viewport is a value beside the model, never inside it.** [`Viewport`] owns a
//!   [`Transform`], a [`Grid`], [`Rulers`] and [`Guides`]. Its methods take the model as read-only
//!   input and answer screen-space descriptions, so there is no path from an aid back into a node.
//! * **Fit and actual size move the transform only.** [`Transform::fit`] and [`Transform::actual_size`]
//!   change the pan and the zoom and leave every rectangle where it was, which is what "do not
//!   pollute" means for the two commands a person most expects to be harmless.
//! * **Snapping is a candidate, not an edit.** [`Grid::snap`] answers where a point *would* land on
//!   the grid; a drag that is cancelled therefore leaves nothing behind, because nothing was written.
//! * **Aids are an overlay.** [`Viewport::overlay`] returns an [`Overlay`] — grid lines, ruler ticks,
//!   guides — that a canvas draws *beside* the model, so what the model renders is the same whether
//!   the aids are on or off.
//! * **Aid cost follows what is visible.** [`Viewport::overlay`] omits the grid when its spacing is
//!   too small on screen to read, so panning far out cannot make the designer draw an unbounded
//!   number of lines.

#![allow(dead_code)]

use crate::app::scene::Rect;

/// The closest a zoom may get to nothing.
pub const MIN_ZOOM: f64 = 0.05;
/// The furthest a zoom may go.
pub const MAX_ZOOM: f64 = 64.0;
/// Below this many device pixels between grid lines, the grid is not drawn.
const MIN_GRID_PIXELS: f64 = 4.0;

/// A point, in the design space or on the screen depending on who is asking.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Point {
    /// Horizontal.
    pub x: f64,
    /// Vertical.
    pub y: f64,
}

impl Point {
    /// A point.
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// A viewport size, in device-independent pixels.
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

/// The model-to-screen mapping: a pan and a zoom.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Transform {
    pan: Point,
    zoom: f64,
}

impl Transform {
    /// The untransformed mapping: zoom 1, no pan.
    pub const fn identity() -> Self {
        Self {
            pan: Point::new(0.0, 0.0),
            zoom: 1.0,
        }
    }

    /// A mapping with a pan and a zoom (the zoom clamped to the allowed range).
    pub fn at(pan: Point, zoom: f64) -> Self {
        Self {
            pan,
            zoom: clamp_zoom(zoom),
        }
    }

    /// The pan.
    pub fn pan(&self) -> Point {
        self.pan
    }

    /// The zoom.
    pub fn zoom(&self) -> f64 {
        self.zoom
    }

    /// A model point on the screen.
    pub fn to_screen(self, point: Point) -> Point {
        Point::new(
            self.pan.x + point.x * self.zoom,
            self.pan.y + point.y * self.zoom,
        )
    }

    /// A screen point in the model.
    pub fn to_model(self, point: Point) -> Point {
        Point::new(
            (point.x - self.pan.x) / self.zoom,
            (point.y - self.pan.y) / self.zoom,
        )
    }

    /// Moves the view by a screen offset.
    pub fn pan_by(&mut self, dx: f64, dy: f64) {
        self.pan.x += dx;
        self.pan.y += dy;
    }

    /// Multiplies the zoom, clamped to the allowed range.
    pub fn zoom_by(&mut self, factor: f64) {
        self.zoom = clamp_zoom(self.zoom * factor);
    }

    /// Zooms about a screen anchor: the model point under the anchor stays put.
    pub fn zoom_at(&mut self, factor: f64, anchor: Point) {
        let model = self.to_model(anchor);
        self.zoom = clamp_zoom(self.zoom * factor);
        let landed = self.to_screen(model);
        self.pan.x += anchor.x - landed.x;
        self.pan.y += anchor.y - landed.y;
    }

    /// Fits `bounds` into a view with a margin, centred.
    pub fn fit(bounds: Rect, view: Size, margin: f64) -> Self {
        let width = f64::from(bounds.width).max(1.0);
        let height = f64::from(bounds.height).max(1.0);
        let usable_width = (view.width - 2.0 * margin).max(1.0);
        let usable_height = (view.height - 2.0 * margin).max(1.0);
        let zoom = clamp_zoom((usable_width / width).min(usable_height / height));
        let mut transform = Self {
            pan: Point::new(0.0, 0.0),
            zoom,
        };
        let centre = Point::new(
            f64::from(bounds.x) + width / 2.0,
            f64::from(bounds.y) + height / 2.0,
        );
        let landed = transform.to_screen(centre);
        transform.pan.x += view.width / 2.0 - landed.x;
        transform.pan.y += view.height / 2.0 - landed.y;
        transform
    }

    /// 100%: the model point at the view's centre stays there.
    pub fn actual_size(&mut self, view: Size) {
        let centre = Point::new(view.width / 2.0, view.height / 2.0);
        let model = self.to_model(centre);
        self.zoom = 1.0;
        let landed = self.to_screen(model);
        self.pan.x += centre.x - landed.x;
        self.pan.y += centre.y - landed.y;
    }
}

/// The grid: an aid that proposes where a point should land.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Grid {
    spacing: f64,
    visible: bool,
    snapping: bool,
}

impl Grid {
    /// A grid with a spacing, visible and snapping.
    pub const fn new(spacing: f64) -> Self {
        Self {
            spacing,
            visible: true,
            snapping: true,
        }
    }

    /// The distance between lines.
    pub fn spacing(&self) -> f64 {
        self.spacing
    }

    /// Whether the grid is drawn.
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Whether points snap to it.
    pub fn is_snapping(&self) -> bool {
        self.snapping
    }

    /// Sets the distance between lines.
    pub fn set_spacing(&mut self, spacing: f64) {
        self.spacing = spacing;
    }

    /// Turns drawing on or off.
    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }

    /// Turns snapping on or off.
    pub fn set_snapping(&mut self, snapping: bool) {
        self.snapping = snapping;
    }

    /// Where the point would land on the grid — a candidate, never an edit.
    pub fn snap(&self, point: Point) -> Point {
        if !self.snapping || self.spacing <= 0.0 {
            return point;
        }
        Point::new(
            round_to(point.x, self.spacing),
            round_to(point.y, self.spacing),
        )
    }

    /// The vertical and horizontal grid lines within `bounds`, as `(from, to)` pairs.
    pub fn lines(&self, bounds: Rect) -> Vec<(Point, Point)> {
        let mut lines = Vec::new();
        if self.spacing <= 0.0 {
            return lines;
        }
        let left = f64::from(bounds.x);
        let top = f64::from(bounds.y);
        let right = left + f64::from(bounds.width);
        let bottom = top + f64::from(bounds.height);
        for x in multiples(self.spacing, left, right) {
            lines.push((Point::new(x, top), Point::new(x, bottom)));
        }
        for y in multiples(self.spacing, top, bottom) {
            lines.push((Point::new(left, y), Point::new(right, y)));
        }
        lines
    }
}

/// A ruler tick, in model coordinates.
#[derive(Clone, PartialEq, Debug)]
pub struct Tick {
    /// Where it sits.
    pub position: f64,
    /// What it reads.
    pub label: String,
}

/// The rulers: a step between ticks.
#[derive(Clone, PartialEq, Debug)]
pub struct Rulers {
    step: f64,
}

impl Rulers {
    /// A ruler with a step.
    pub fn new(step: f64) -> Self {
        Self { step }
    }

    /// The step between ticks.
    pub fn step(&self) -> f64 {
        self.step
    }

    /// Sets the step between ticks.
    pub fn set_step(&mut self, step: f64) {
        self.step = step;
    }

    /// The ticks between two model positions.
    pub fn ticks(&self, from: f64, to: f64) -> Vec<Tick> {
        if self.step <= 0.0 {
            return Vec::new();
        }
        multiples(self.step, from, to)
            .into_iter()
            .map(|position| Tick {
                label: number(position),
                position,
            })
            .collect()
    }
}

/// Which way a guide runs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Axis {
    /// Top to bottom, at a fixed x.
    Vertical,
    /// Left to right, at a fixed y.
    Horizontal,
}

/// A guide line, in model coordinates.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Guide {
    /// Which way it runs.
    pub axis: Axis,
    /// Where it sits.
    pub position: f64,
}

/// The guides a moving box would show against the boxes around it, within a tolerance.
///
/// Only edges and centres that actually line up produce a guide, so the aid says something true.
pub fn align(moving: Rect, others: &[Rect], tolerance: f64) -> Vec<Guide> {
    let moving_columns = columns(moving);
    let moving_rows = rows(moving);
    let mut found = Vec::new();
    for other in others {
        for position in columns(*other) {
            if moving_columns
                .iter()
                .any(|edge| (edge - position).abs() <= tolerance)
            {
                found.push(Guide {
                    axis: Axis::Vertical,
                    position,
                });
            }
        }
        for position in rows(*other) {
            if moving_rows
                .iter()
                .any(|edge| (edge - position).abs() <= tolerance)
            {
                found.push(Guide {
                    axis: Axis::Horizontal,
                    position,
                });
            }
        }
    }
    dedupe(&mut found);
    found
}

/// What the design aids add to a frame — drawn beside the model, never in it.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Overlay {
    /// Grid lines as `(from, to)` pairs.
    pub grid: Vec<(Point, Point)>,
    /// Ruler ticks along the horizontal axis.
    pub ticks: Vec<Tick>,
    /// Alignment guides.
    pub guides: Vec<Guide>,
}

impl Overlay {
    /// Whether the overlay would draw nothing.
    pub fn is_empty(&self) -> bool {
        self.grid.is_empty() && self.ticks.is_empty() && self.guides.is_empty()
    }

    /// The overlay in a report.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "aids: {} grid line(s), {} tick(s), {} guide(s)",
            self.grid.len(),
            self.ticks.len(),
            self.guides.len()
        )];
        for tick in &self.ticks {
            lines.push(format!("  tick {} = {}", number(tick.position), tick.label));
        }
        for guide in &self.guides {
            let axis = match guide.axis {
                Axis::Vertical => "vertical",
                Axis::Horizontal => "horizontal",
            };
            lines.push(format!("  guide {axis} {}", number(guide.position)));
        }
        lines
    }
}

/// The viewport: the transform and the design aids that sit beside the model.
#[derive(Clone, PartialEq, Debug)]
pub struct Viewport {
    transform: Transform,
    grid: Grid,
    rulers: Rulers,
    guides: Vec<Guide>,
}

impl Viewport {
    /// A viewport at 100%, with a grid of `spacing` and a ruler of the same step, no guides.
    pub fn new(spacing: f64) -> Self {
        Self {
            transform: Transform::identity(),
            grid: Grid::new(spacing),
            rulers: Rulers::new(spacing),
            guides: Vec::new(),
        }
    }

    /// The transform.
    pub fn transform(&self) -> &Transform {
        &self.transform
    }

    /// The transform, to move it.
    pub fn transform_mut(&mut self) -> &mut Transform {
        &mut self.transform
    }

    /// The grid.
    pub fn grid(&self) -> &Grid {
        &self.grid
    }

    /// The grid, to adjust it.
    pub fn grid_mut(&mut self) -> &mut Grid {
        &mut self.grid
    }

    /// The rulers.
    pub fn rulers(&self) -> &Rulers {
        &self.rulers
    }

    /// The rulers, to adjust them.
    pub fn rulers_mut(&mut self) -> &mut Rulers {
        &mut self.rulers
    }

    /// The current alignment guides.
    pub fn guides(&self) -> &[Guide] {
        &self.guides
    }

    /// Sets the alignment guides a drag would show.
    pub fn set_guides(&mut self, guides: Vec<Guide>) {
        self.guides = guides;
    }

    /// The model area a view of `view` covers.
    pub fn visible_model(&self, view: Size) -> Rect {
        let top_left = self.transform.to_model(Point::new(0.0, 0.0));
        let bottom_right = self.transform.to_model(Point::new(view.width, view.height));
        let x = top_left.x.floor() as i32;
        let y = top_left.y.floor() as i32;
        let width = (bottom_right.x.ceil() - f64::from(x)).max(0.0) as u32;
        let height = (bottom_right.y.ceil() - f64::from(y)).max(0.0) as u32;
        Rect::new(x, y, width, height)
    }

    /// A model rectangle on the screen, rounded to whole pixels.
    pub fn screen_rect(&self, rect: Rect) -> Rect {
        let top_left = self
            .transform
            .to_screen(Point::new(f64::from(rect.x), f64::from(rect.y)));
        let bottom_right = self.transform.to_screen(Point::new(
            f64::from(rect.x + rect.width as i32),
            f64::from(rect.y + rect.height as i32),
        ));
        let x = top_left.x.round() as i32;
        let y = top_left.y.round() as i32;
        let width = (bottom_right.x - top_left.x).round().max(0.0) as u32;
        let height = (bottom_right.y - top_left.y).round().max(0.0) as u32;
        Rect::new(x, y, width, height)
    }

    /// The aids to draw for a view of `view`: the grid, the ruler ticks and the guides.
    /// The grid is omitted when its lines would be too close together to read, so the number of
    /// lines follows what is visible rather than how far out the view has been panned.
    pub fn overlay(&self, view: Size) -> Overlay {
        let bounds = self.visible_model(view);
        let grid = if self.grid.is_visible()
            && self.grid.spacing() * self.transform.zoom() >= MIN_GRID_PIXELS
        {
            self.grid.lines(bounds)
        } else {
            Vec::new()
        };
        let from = f64::from(bounds.x);
        let to = from + f64::from(bounds.width);
        Overlay {
            grid,
            ticks: self.rulers.ticks(from, to),
            guides: self.guides.clone(),
        }
    }

    /// The viewport in a report.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "viewport: pan {},{} zoom {}",
            self.transform.pan.x, self.transform.pan.y, self.transform.zoom
        )];
        lines.push(format!(
            "grid: spacing {}, {}, {}",
            number(self.grid.spacing),
            if self.grid.visible {
                "visible"
            } else {
                "hidden"
            },
            if self.grid.snapping {
                "snapping"
            } else {
                "free"
            }
        ));
        lines.push(format!("guides: {}", self.guides.len()));
        for guide in &self.guides {
            let axis = match guide.axis {
                Axis::Vertical => "vertical",
                Axis::Horizontal => "horizontal",
            };
            lines.push(format!("  guide {axis} {}", number(guide.position)));
        }
        lines
    }
}

/// Multiples of `step` inside `[from, to]`.
fn multiples(step: f64, from: f64, to: f64) -> Vec<f64> {
    let first = (from / step).ceil();
    let last = (to / step).floor();
    if last < first || !first.is_finite() || !last.is_finite() {
        return Vec::new();
    }
    let mut values = Vec::new();
    let mut index = first;
    while index <= last {
        values.push(index * step);
        index += 1.0;
    }
    values
}

/// Rounds to the nearest multiple of `step`.
fn round_to(value: f64, step: f64) -> f64 {
    (value / step).round() * step
}

/// The vertical edges a box offers for alignment: left, centre, right.
fn columns(rect: Rect) -> [f64; 3] {
    let left = f64::from(rect.x);
    let width = f64::from(rect.width);
    [left, left + width / 2.0, left + width]
}

/// The horizontal edges a box offers for alignment: top, middle, bottom.
fn rows(rect: Rect) -> [f64; 3] {
    let top = f64::from(rect.y);
    let height = f64::from(rect.height);
    [top, top + height / 2.0, top + height]
}

/// Sorts guides and drops the ones that repeat, so an edge shared by two boxes is one guide.
fn dedupe(guides: &mut Vec<Guide>) {
    guides.sort_by(|a, b| {
        (a.axis as u8)
            .cmp(&(b.axis as u8))
            .then(a.position.total_cmp(&b.position))
    });
    guides.dedup_by(|a, b| a.axis == b.axis && (a.position - b.position).abs() < 1e-9);
}

/// Clamps a zoom into the allowed range, replacing a value that is not a number.
fn clamp_zoom(zoom: f64) -> f64 {
    if !zoom.is_finite() {
        return 1.0;
    }
    zoom.clamp(MIN_ZOOM, MAX_ZOOM)
}

/// A number as a ruler would read it: a whole number without a decimal point.
fn number(value: f64) -> String {
    if (value - value.round()).abs() < 1e-9 {
        format!("{}", value.round() as i64)
    } else {
        format!("{value:.1}")
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Axis, Grid, MAX_ZOOM, MIN_ZOOM, Point, Size, Transform, Viewport, align, clamp_zoom,
    };
    use crate::app::scene::{Rect, Scene};
    use crate::contracts::identity::NodeId;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    fn assert_point(point: Point, x: f64, y: f64) {
        assert!(
            close(point.x, x) && close(point.y, y),
            "expected {x},{y} but got {},{}",
            point.x,
            point.y
        );
    }

    fn node(value: u64) -> NodeId {
        NodeId::from_raw(value)
    }

    #[test]
    fn a_transform_round_trips_through_the_screen() {
        let mut transform = Transform::identity();
        transform.zoom_by(2.0);
        transform.pan_by(30.0, -12.0);
        let model = Point::new(11.0, 7.5);
        let screen = transform.to_screen(model);
        assert_point(transform.to_model(screen), model.x, model.y);
    }

    #[test]
    fn fit_centres_the_bounds_and_keeps_them_inside_the_view() {
        let view = Size::new(800.0, 600.0);
        let bounds = Rect::new(0, 0, 400, 200);
        let transform = Transform::fit(bounds, view, 20.0);
        let centre = transform.to_screen(Point::new(200.0, 100.0));
        assert_point(centre, 400.0, 300.0);
        assert!(transform.zoom() <= 2.0 && transform.zoom() >= 1.0);
    }

    #[test]
    fn actual_size_keeps_the_centre_and_sets_one_hundred_percent() {
        let view = Size::new(800.0, 600.0);
        let mut transform = Transform::fit(Rect::new(0, 0, 400, 200), view, 20.0);
        let before = transform.to_model(Point::new(400.0, 300.0));
        transform.actual_size(view);
        assert!(close(transform.zoom(), 1.0));
        let after = transform.to_model(Point::new(400.0, 300.0));
        assert_point(after, before.x, before.y);
    }

    #[test]
    fn zooming_about_an_anchor_keeps_that_point_still() {
        let mut transform = Transform::identity();
        let anchor = Point::new(120.0, 80.0);
        let before = transform.to_model(anchor);
        transform.zoom_at(3.0, anchor);
        let after = transform.to_model(anchor);
        assert_point(after, before.x, before.y);
    }

    #[test]
    fn zoom_is_clamped_at_both_ends() {
        assert!(close(clamp_zoom(1e9), MAX_ZOOM));
        assert!(close(clamp_zoom(1e-9), MIN_ZOOM));
        assert!(close(clamp_zoom(f64::NAN), 1.0));
    }

    #[test]
    fn snapping_is_a_candidate_and_writes_nothing() {
        let grid = Grid::new(10.0);
        assert_point(grid.snap(Point::new(13.0, 27.0)), 10.0, 30.0);
        let mut quiet = grid;
        quiet.set_snapping(false);
        let point = Point::new(13.0, 27.0);
        assert_point(quiet.snap(point), point.x, point.y);
    }

    #[test]
    fn the_grid_draws_lines_across_the_bounds() {
        let grid = Grid::new(50.0);
        let lines = grid.lines(Rect::new(0, 0, 100, 100));
        let vertical = lines
            .iter()
            .filter(|(from, to)| close(from.x, to.x))
            .count();
        let horizontal = lines
            .iter()
            .filter(|(from, to)| close(from.y, to.y))
            .count();
        assert_eq!(vertical, 3);
        assert_eq!(horizontal, 3);
    }

    #[test]
    fn ruler_ticks_land_on_the_step_and_read_as_whole_numbers() {
        let viewport = Viewport::new(50.0);
        let ticks = viewport.rulers().ticks(0.0, 120.0);
        let positions: Vec<f64> = ticks.iter().map(|tick| tick.position).collect();
        assert_eq!(positions, vec![0.0, 50.0, 100.0]);
        assert_eq!(ticks[1].label, "50");
    }

    #[test]
    fn alignment_guides_appear_for_shared_edges_and_centres() {
        let moving = Rect::new(100, 0, 50, 50);
        let others = [Rect::new(100, 200, 80, 20)];
        let guides = align(moving, &others, 1.0);
        assert!(
            guides
                .iter()
                .any(|guide| guide.axis == Axis::Vertical && close(guide.position, 100.0)),
            "a shared left edge should guide"
        );
        assert!(guides.iter().all(|guide| guide.axis == Axis::Vertical));
    }

    #[test]
    fn a_guide_that_is_shared_by_two_boxes_is_reported_once() {
        let moving = Rect::new(0, 0, 100, 100);
        let others = [Rect::new(0, 200, 10, 10), Rect::new(0, 400, 10, 10)];
        let left = align(moving, &others, 1.0)
            .into_iter()
            .filter(|guide| close(guide.position, 0.0))
            .count();
        assert_eq!(left, 1);
    }

    #[test]
    fn a_box_that_lines_up_with_nothing_gets_no_guide() {
        let moving = Rect::new(0, 0, 10, 10);
        let others = [Rect::new(500, 500, 10, 10)];
        assert!(align(moving, &others, 1.0).is_empty());
    }

    #[test]
    fn the_grid_is_omitted_when_its_lines_would_be_illegible() {
        let mut viewport = Viewport::new(10.0);
        viewport.transform_mut().zoom_by(0.05);
        let view = Size::new(800.0, 600.0);
        assert!(viewport.overlay(view).grid.is_empty());
        viewport.transform_mut().zoom_by(20.0);
        assert!(!viewport.overlay(view).grid.is_empty());
    }

    #[test]
    fn the_overlay_carries_the_grid_ticks_and_guides_but_not_the_model() {
        let mut viewport = Viewport::new(50.0);
        viewport.set_guides(align(
            Rect::new(0, 0, 100, 100),
            &[Rect::new(0, 200, 100, 20)],
            1.0,
        ));
        let overlay = viewport.overlay(Size::new(300.0, 200.0));
        assert!(!overlay.grid.is_empty());
        assert!(!overlay.ticks.is_empty());
        assert!(!overlay.guides.is_empty());
    }

    #[test]
    fn design_aids_do_not_pollute_the_model() {
        let mut scene = Scene::new();
        scene.insert(node(1), Rect::new(0, 0, 100, 100));
        scene.insert(node(2), Rect::new(10, 10, 20, 20));
        let before = scene.render();

        let view = Size::new(800.0, 600.0);
        let mut viewport = Viewport::new(10.0);
        viewport.transform_mut().pan_by(-40.0, 25.0);
        viewport.transform_mut().zoom_by(2.5);
        viewport
            .transform_mut()
            .zoom_at(1.5, Point::new(100.0, 100.0));
        let fitted = Transform::fit(Rect::new(0, 0, 100, 100), view, 20.0);
        *viewport.transform_mut() = fitted;
        viewport.transform_mut().actual_size(view);
        viewport.grid_mut().snap(Point::new(13.0, 27.0));
        viewport.set_guides(align(
            Rect::new(0, 0, 100, 100),
            &[Rect::new(0, 200, 100, 20)],
            1.0,
        ));
        let _ = viewport.overlay(view);

        assert_eq!(scene.render(), before);
    }
}
