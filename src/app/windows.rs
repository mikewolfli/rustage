// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Windows, floating panels, displays and where a window sits (task T04.06).
//!
//! The acceptance line is *"after a display is unplugged or its DPI changes, a window is still
//! visible"*. That is a statement about **placement** surviving a change to the screens, so the
//! module is built around placement as a value that can be re-checked:
//!
//! * **A placement names a display and a rectangle on it.** [`Placement`] holds a [`DisplayId`], a
//!   [`Rect`] and the scale it was placed at — values only, no native window handle, so a placement
//!   can be written down and reasoned about without a screen. Which display a rectangle is *on* is
//!   answered by geometry ([`Displays::best_for`]), not by asking a backend.
//! * **Reconciling is the whole of "still visible".** [`Windows::reconcile`] takes the current set of
//!   displays and, for every window, moves it off a display that is gone, clamps a rectangle that no
//!   longer fits into the display it names, and reports a changed scale. The post-condition is
//!   checkable: [`Windows::visible`] names the windows that overlap some display by at least
//!   [`MIN_VISIBLE`], and reconcile leaves every window in it.
//! * **Ownership has consequences.** A view or a floating panel is *owned by* the main window
//!   ([`Kind`], [`Window::owner`]); closing the main window closes what it owns, so a project's view
//!   window cannot outlive the window it belongs to. This is the "main window ownership" of the task.
//!
//! **Where the displays come from.** The pinned dependency exposes a DPI *factor* and a `dpi_scaling`
//! capability, but no display enumeration: there is no way to ask which monitors exist, what their
//! bounds are, or which display a window is on (`feedback-20261015.md` §十二). The model therefore
//! takes the displays as an input — the caller plays the platform's part — and [`Displays::new`]
//! carries no fabricated default; when there are no displays, [`Windows::reconcile`] says so
//! ([`Adjustment::NoDisplay`]) rather than inventing one to keep a window "on screen".

#![allow(dead_code)]

use std::fmt;

/// A rectangle in logical coordinates. Coordinates are signed because a display to the left of, or
/// above, the primary one has negative origin.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rect {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

impl Rect {
    /// A rectangle.
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// The left edge.
    pub const fn x(self) -> i32 {
        self.x
    }

    /// The top edge.
    pub const fn y(self) -> i32 {
        self.y
    }

    /// The width.
    pub const fn width(self) -> u32 {
        self.width
    }

    /// The height.
    pub const fn height(self) -> u32 {
        self.height
    }

    /// The right edge (exclusive).
    pub const fn right(self) -> i32 {
        self.x + self.width as i32
    }

    /// The bottom edge (exclusive).
    pub const fn bottom(self) -> i32 {
        self.y + self.height as i32
    }

    /// The overlapping rectangle, when the two overlap at all.
    pub fn intersection(self, other: Self) -> Option<Self> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        if right <= x || bottom <= y {
            return None;
        }
        Some(Self {
            x,
            y,
            width: (right - x) as u32,
            height: (bottom - y) as u32,
        })
    }

    /// This rectangle moved so that it lies inside `outer`, keeping its size when it fits and
    /// filling `outer` when it does not.
    ///
    /// A rectangle exactly as wide as, or wider than, the display is aligned to the display's left
    /// (and clipped to its width): sliding a too-wide window is not possible, so the only honest
    /// placement is the display's own left edge.
    pub fn clamp_into(self, outer: Self) -> Self {
        let width = self.width.min(outer.width);
        let height = self.height.min(outer.height);
        let x = if self.width >= outer.width {
            outer.x
        } else {
            self.x.clamp(outer.x, outer.right() - width as i32)
        };
        let y = if self.height >= outer.height {
            outer.y
        } else {
            self.y.clamp(outer.y, outer.bottom() - height as i32)
        };
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// A rectangle as one line.
    pub fn render(self) -> String {
        format!("{}x{}+{}+{}", self.width, self.height, self.x, self.y)
    }
}

/// A display's identity. Opaque: nothing may order or group displays by the number.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct DisplayId(u64);

impl DisplayId {
    /// Rebuilds an id from the number it was written as.
    pub const fn from_raw(value: u64) -> Self {
        Self(value)
    }

    /// The number this id is written as.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// One display: where it is and how much it scales.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Display {
    id: DisplayId,
    bounds: Rect,
    scale: f64,
}

impl Display {
    /// A display. A scale that is not positive is not a scale, and the display is refused.
    pub fn new(id: DisplayId, bounds: Rect, scale: f64) -> Option<Self> {
        if !(scale.is_finite() && scale > 0.0) {
            return None;
        }
        Some(Self { id, bounds, scale })
    }

    /// The display's identity.
    pub const fn id(self) -> DisplayId {
        self.id
    }

    /// The area it shows, in logical coordinates.
    pub const fn bounds(self) -> Rect {
        self.bounds
    }

    /// The DPI scale factor.
    pub const fn scale(self) -> f64 {
        self.scale
    }
}

/// The displays a host has, as the caller reports them.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Displays {
    all: Vec<Display>,
}

impl Displays {
    /// A set of displays, in the order the caller lists them. The first is the primary one.
    pub fn new(all: Vec<Display>) -> Self {
        Self { all }
    }

    /// No displays at all (a headless session, or a platform this build cannot ask).
    pub fn none() -> Self {
        Self { all: Vec::new() }
    }

    /// Every display, in order.
    pub fn all(&self) -> &[Display] {
        &self.all
    }

    /// How many there are.
    pub fn len(&self) -> usize {
        self.all.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.all.is_empty()
    }

    /// The primary display: the first, which is the convention every desktop uses.
    pub fn primary(&self) -> Option<Display> {
        self.all.first().copied()
    }

    /// The display with an id, when the host still has it.
    pub fn get(&self, id: DisplayId) -> Option<Display> {
        self.all.iter().copied().find(|display| display.id == id)
    }

    /// The display a rectangle overlaps most, when it overlaps any.
    pub fn best_for(&self, rect: Rect) -> Option<DisplayId> {
        let mut best: Option<(u64, DisplayId)> = None;
        for display in &self.all {
            let Some(overlap) = rect.intersection(display.bounds) else {
                continue;
            };
            let area = u64::from(overlap.width()) * u64::from(overlap.height());
            if best.map(|(best_area, _)| area > best_area).unwrap_or(true) {
                best = Some((area, display.id));
            }
        }
        best.map(|(_, id)| id)
    }

    /// The set with a display removed (it was unplugged). Answers whether it was there.
    pub fn unplug(&mut self, id: DisplayId) -> bool {
        let before = self.all.len();
        self.all.retain(|display| display.id != id);
        self.all.len() != before
    }

    /// The set with a display added, or an existing one replaced.
    pub fn plug(&mut self, display: Display) {
        match self.all.iter_mut().find(|held| held.id == display.id) {
            Some(held) => *held = display,
            None => self.all.push(display),
        }
    }
}

/// How a window is used.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Kind {
    /// The main window: owns the windows that belong to it.
    Main,
    /// A window showing one document, the "new view" of a project.
    View { document: String },
    /// A floating panel, undocked from the main window.
    Panel { panel: String },
}

impl Kind {
    /// The kind's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::View { .. } => "view",
            Self::Panel { .. } => "panel",
        }
    }
}

/// A window's identity inside one session.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct WindowId(u64);

impl WindowId {
    /// Rebuilds an id from the number it was written as.
    pub const fn from_raw(value: u64) -> Self {
        Self(value)
    }

    /// The number this id is written as.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Where a window sits: a display, a rectangle on it, and the scale it was placed at.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Placement {
    display: DisplayId,
    rect: Rect,
    scale: f64,
}

impl Placement {
    /// A placement.
    pub const fn new(display: DisplayId, rect: Rect, scale: f64) -> Self {
        Self {
            display,
            rect,
            scale,
        }
    }

    /// The display it was placed on.
    pub const fn display(self) -> DisplayId {
        self.display
    }

    /// The rectangle.
    pub const fn rect(self) -> Rect {
        self.rect
    }

    /// The scale it was placed at.
    pub const fn scale(self) -> f64 {
        self.scale
    }

    /// The placement as one line.
    pub fn render(self) -> String {
        format!(
            "display {} at {} @{}",
            self.display.get(),
            self.rect.render(),
            self.scale
        )
    }
}

/// One window.
#[derive(Clone, PartialEq, Debug)]
pub struct Window {
    id: WindowId,
    kind: Kind,
    placement: Placement,
    owner: Option<WindowId>,
}

impl Window {
    /// The window's identity.
    pub const fn id(&self) -> WindowId {
        self.id
    }

    /// What the window is for.
    pub fn kind(&self) -> &Kind {
        &self.kind
    }

    /// Where it sits.
    pub const fn placement(&self) -> Placement {
        self.placement
    }

    /// The window that owns it, if any. Only the main window owns others.
    pub const fn owner(&self) -> Option<WindowId> {
        self.owner
    }
}

/// The smallest overlap, in logical pixels, that still counts as "a window is visible on screen".
pub const MIN_VISIBLE: (u32, u32) = (64, 32);

/// What reconciling did to one window.
#[derive(Clone, PartialEq, Debug)]
pub enum Adjustment {
    /// It was already on a display that still exists and inside its bounds.
    Kept { window: WindowId },
    /// Its display was gone, so it moved to another.
    Moved {
        window: WindowId,
        from: DisplayId,
        to: DisplayId,
    },
    /// Its display shrank (a DPI change, say), so its rectangle was pulled back inside.
    Clamped {
        window: WindowId,
        from: Rect,
        to: Rect,
    },
    /// The display's scale changed, which is reported whether or not the rectangle also moved.
    Rescaled {
        window: WindowId,
        from: f64,
        to: f64,
    },
    /// There is no display to place it on, so it was left as it was and said so.
    NoDisplay { window: WindowId },
}

impl Adjustment {
    /// The window this is about.
    pub const fn window(&self) -> WindowId {
        match self {
            Self::Kept { window }
            | Self::Moved { window, .. }
            | Self::Clamped { window, .. }
            | Self::Rescaled { window, .. }
            | Self::NoDisplay { window } => *window,
        }
    }

    /// The adjustment as one line.
    pub fn render(&self) -> String {
        match self {
            Self::Kept { window } => format!("window {} kept", window.get()),
            Self::Moved { window, from, to } => format!(
                "window {} moved from display {} to {}",
                window.get(),
                from.get(),
                to.get()
            ),
            Self::Clamped { window, from, to } => format!(
                "window {} clamped from {} to {}",
                window.get(),
                from.render(),
                to.render()
            ),
            Self::Rescaled { window, from, to } => {
                format!("window {} rescaled from {from} to {to}", window.get())
            }
            Self::NoDisplay { window } => format!(
                "window {} has no display to sit on; it was left where it was",
                window.get()
            ),
        }
    }
}

/// The windows of one session, and which one is the main window.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Windows {
    windows: Vec<Window>,
    main: Option<WindowId>,
    next: u64,
}

impl Windows {
    /// A session whose only window is the main one, placed on the primary display.
    ///
    /// The main window fills the display it is on; with no display it takes a documented fallback
    /// rectangle on a display id the host does not have, so that [`Windows::reconcile`] reports it
    /// as unplaced rather than this constructor pretending there was a screen.
    pub fn new(displays: &Displays) -> Self {
        let id = WindowId(0);
        let placement = match displays.primary() {
            Some(display) => Placement::new(display.id(), display.bounds(), display.scale()),
            None => Placement::new(DisplayId::from_raw(0), Rect::new(0, 0, 1024, 768), 1.0),
        };
        Self {
            windows: vec![Window {
                id,
                kind: Kind::Main,
                placement,
                owner: None,
            }],
            main: Some(id),
            next: 1,
        }
    }

    /// The main window's id, when there is one.
    pub const fn main(&self) -> Option<WindowId> {
        self.main
    }

    /// Every window, in creation order.
    pub fn all(&self) -> &[Window] {
        &self.windows
    }

    /// How many there are.
    pub fn len(&self) -> usize {
        self.windows.len()
    }

    /// Whether there are none (the main window has been closed).
    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    /// One window.
    pub fn get(&self, id: WindowId) -> Option<&Window> {
        self.windows.iter().find(|window| window.id == id)
    }

    /// The windows owned by a window, in creation order.
    pub fn owned_by(&self, owner: WindowId) -> Vec<WindowId> {
        self.windows
            .iter()
            .filter(|window| window.owner == Some(owner))
            .map(|window| window.id)
            .collect()
    }

    /// Opens a view window for a document, owned by the main window.
    ///
    /// A view window has no owner if there is no main window, and that is refused rather than
    /// created: a window that belongs to nothing is a window nobody will close.
    pub fn open_view(
        &mut self,
        document: impl Into<String>,
        displays: &Displays,
    ) -> Result<WindowId, String> {
        let Some(main) = self.main else {
            return Err("there is no main window to own a view; it was closed".to_owned());
        };
        let base = self
            .get(main)
            .map(|window| window.placement)
            .unwrap_or_else(|| {
                Placement::new(DisplayId::from_raw(0), Rect::new(0, 0, 1024, 768), 1.0)
            });
        let placement = Self::default_placement(base, displays, 800, 600, 40);
        Ok(self.add(
            Kind::View {
                document: document.into(),
            },
            placement,
            Some(main),
        ))
    }

    /// Floats a panel as its own window, owned by the main window.
    pub fn float_panel(
        &mut self,
        panel: impl Into<String>,
        displays: &Displays,
    ) -> Result<WindowId, String> {
        let Some(main) = self.main else {
            return Err(
                "there is no main window to own a floating panel; it was closed".to_owned(),
            );
        };
        let base = self
            .get(main)
            .map(|window| window.placement)
            .unwrap_or_else(|| {
                Placement::new(DisplayId::from_raw(0), Rect::new(0, 0, 1024, 768), 1.0)
            });
        let placement = Self::default_placement(base, displays, 320, 240, 80);
        Ok(self.add(
            Kind::Panel {
                panel: panel.into(),
            },
            placement,
            Some(main),
        ))
    }

    /// Moves a window onto a display, clamped so it stays inside that display.
    pub fn move_to(
        &mut self,
        id: WindowId,
        display: DisplayId,
        displays: &Displays,
    ) -> Result<(), String> {
        let Some(target) = displays.get(display) else {
            return Err(format!("there is no display {}", display.get()));
        };
        let window = self
            .windows
            .iter_mut()
            .find(|window| window.id == id)
            .ok_or_else(|| format!("there is no window {}", id.get()))?;
        window.placement.display = display;
        window.placement.scale = target.scale();
        window.placement.rect = window.placement.rect.clamp_into(target.bounds());
        Ok(())
    }

    /// Resizes a window, keeping its position.
    pub fn resize(&mut self, id: WindowId, width: u32, height: u32) -> Result<(), String> {
        let window = self
            .windows
            .iter_mut()
            .find(|window| window.id == id)
            .ok_or_else(|| format!("there is no window {}", id.get()))?;
        window.placement.rect = Rect::new(
            window.placement.rect.x(),
            window.placement.rect.y(),
            width,
            height,
        );
        Ok(())
    }

    /// Moves a window's top-left corner, keeping its size.
    ///
    /// Not clamped: where a person drags a window is where it goes. The next
    /// [`Windows::reconcile`] is what brings it back on screen, which is the same path an unplug
    /// takes — there is one rule for "a window ended up somewhere it should not be".
    pub fn place_at(&mut self, id: WindowId, x: i32, y: i32) -> Result<(), String> {
        let window = self
            .windows
            .iter_mut()
            .find(|window| window.id == id)
            .ok_or_else(|| format!("there is no window {}", id.get()))?;
        window.placement.rect = Rect::new(
            x,
            y,
            window.placement.rect.width(),
            window.placement.rect.height(),
        );
        Ok(())
    }

    /// Closes a window, and everything it owns.
    ///
    /// Closing the main window closes its views and floating panels with it (they belong to it);
    /// closing a child closes just that one. Answers the ids that were closed.
    pub fn close(&mut self, id: WindowId) -> Result<Vec<WindowId>, String> {
        if self.get(id).is_none() {
            return Err(format!("there is no window {}", id.get()));
        }
        let mut closing = vec![id];
        if Some(id) == self.main {
            closing = self.windows.iter().map(|window| window.id).collect();
            self.main = None;
        }
        self.windows.retain(|window| !closing.contains(&window.id));
        Ok(closing)
    }

    /// Whether a window overlaps some display by at least [`MIN_VISIBLE`].
    pub fn is_visible(&self, id: WindowId, displays: &Displays) -> bool {
        let Some(window) = self.get(id) else {
            return false;
        };
        Self::placement_visible(window.placement.rect, displays)
    }

    /// Every window that is visible right now.
    pub fn visible(&self, displays: &Displays) -> Vec<WindowId> {
        self.windows
            .iter()
            .filter(|window| Self::placement_visible(window.placement.rect, displays))
            .map(|window| window.id)
            .collect()
    }

    /// Re-checks every window against the current displays, keeping each one visible, and reports
    /// what had to change.
    ///
    /// A window whose display is gone moves to the display it overlaps most (or the primary one); a
    /// window whose rectangle no longer fits the display it names is pulled back inside; a window on
    /// a display whose scale changed is reported. When there are no displays at all, every window is
    /// left where it was and reported as unplaced — a window cannot be made visible on a screen that
    /// is not there.
    pub fn reconcile(&mut self, displays: &Displays) -> Vec<Adjustment> {
        if displays.is_empty() {
            return self
                .windows
                .iter()
                .map(|window| Adjustment::NoDisplay { window: window.id })
                .collect();
        }

        let mut adjustments = Vec::with_capacity(self.windows.len());
        for window in &mut self.windows {
            let id = window.id;
            let mut placement = window.placement;

            // 1. A display that is gone: move to the best remaining one.
            let mut moved_from = None;
            let target = match displays.get(placement.display) {
                Some(display) => display,
                None => {
                    let to = displays
                        .best_for(placement.rect)
                        .or_else(|| displays.primary().map(Display::id))
                        .expect("there is at least one display");
                    moved_from = Some(placement.display);
                    placement.display = to;
                    placement.scale = displays.get(to).expect("the target exists").scale();
                    displays.get(to).expect("the target exists")
                }
            };

            // 2. A display whose scale changed: report it and adopt the new scale.
            let mut rescaled = None;
            if placement.scale != target.scale() {
                rescaled = Some((placement.scale, target.scale()));
                placement.scale = target.scale();
            }

            // 3. A rectangle that no longer fits: pull it back inside.
            let original = placement.rect;
            let clamped = original.clamp_into(target.bounds());
            let was_clamped = clamped != original;
            placement.rect = clamped;
            window.placement = placement;

            let mut reported = false;
            if let Some((from, to)) = rescaled {
                adjustments.push(Adjustment::Rescaled {
                    window: id,
                    from,
                    to,
                });
                reported = true;
            }
            if let Some(from) = moved_from {
                adjustments.push(Adjustment::Moved {
                    window: id,
                    from,
                    to: placement.display,
                });
                reported = true;
            }
            if was_clamped {
                adjustments.push(Adjustment::Clamped {
                    window: id,
                    from: original,
                    to: placement.rect,
                });
                reported = true;
            }
            if !reported {
                adjustments.push(Adjustment::Kept { window: id });
            }
        }
        adjustments
    }

    /// The session in one report.
    pub fn render(&self, displays: &Displays) -> Vec<String> {
        let mut lines = vec![format!(
            "{} window(s); main {}; {} display(s)",
            self.len(),
            match self.main {
                Some(id) => id.get().to_string(),
                None => "(none)".to_owned(),
            },
            displays.len()
        )];
        for window in &self.windows {
            let owner = match window.owner {
                Some(owner) => format!(" owned by {}", owner.get()),
                None => String::new(),
            };
            let visible = if Self::placement_visible(window.placement.rect, displays) {
                "visible"
            } else {
                "NOT VISIBLE"
            };
            let what = match &window.kind {
                Kind::Main => String::new(),
                Kind::View { document } => format!(" {document}"),
                Kind::Panel { panel } => format!(" {panel}"),
            };
            lines.push(format!(
                "  window {} [{}]{what}{owner} {} {visible}",
                window.id.get(),
                window.kind.name(),
                window.placement.render()
            ));
        }
        lines
    }

    /// A default placement for a new window: on the same display as `base`, offset, and clamped so
    /// it starts visible.
    fn default_placement(
        base: Placement,
        displays: &Displays,
        width: u32,
        height: u32,
        offset: i32,
    ) -> Placement {
        let rect = Rect::new(
            base.rect.x() + offset,
            base.rect.y() + offset,
            width,
            height,
        );
        match displays.get(base.display) {
            Some(display) => Placement::new(
                display.id(),
                rect.clamp_into(display.bounds()),
                display.scale(),
            ),
            None => Placement::new(base.display, rect, base.scale),
        }
    }

    fn add(&mut self, kind: Kind, placement: Placement, owner: Option<WindowId>) -> WindowId {
        let id = WindowId(self.next);
        self.next += 1;
        self.windows.push(Window {
            id,
            kind,
            placement,
            owner,
        });
        id
    }

    /// Whether a rectangle overlaps some display by at least [`MIN_VISIBLE`].
    fn placement_visible(rect: Rect, displays: &Displays) -> bool {
        displays.all().iter().any(|display| {
            rect.intersection(display.bounds())
                .map(|overlap| {
                    overlap.width() >= MIN_VISIBLE.0 && overlap.height() >= MIN_VISIBLE.1
                })
                .unwrap_or(false)
        })
    }
}

impl fmt::Display for Adjustment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.render())
    }
}

#[cfg(test)]
mod tests {
    use super::{Adjustment, Display, DisplayId, Displays, MIN_VISIBLE, Rect, Windows};

    fn display(id: u64, x: i32, y: i32, width: u32, height: u32, scale: f64) -> Display {
        Display::new(
            DisplayId::from_raw(id),
            Rect::new(x, y, width, height),
            scale,
        )
        .unwrap()
    }

    fn one_display() -> Displays {
        Displays::new(vec![display(0, 0, 0, 1280, 800, 1.0)])
    }

    fn two_displays() -> Displays {
        Displays::new(vec![
            display(0, 0, 0, 1280, 800, 1.0),
            display(1, 1280, 0, 1920, 1080, 1.0),
        ])
    }

    #[test]
    fn the_main_window_starts_on_the_primary_display_and_owns_what_it_opens() {
        let displays = one_display();
        let mut windows = Windows::new(&displays);
        let main = windows.main().unwrap();
        assert_eq!(
            windows.get(main).unwrap().placement().display(),
            DisplayId::from_raw(0)
        );

        let view = windows.open_view("pages/a.json", &displays).unwrap();
        let panel = windows.float_panel("outline", &displays).unwrap();
        assert_eq!(windows.get(view).unwrap().owner(), Some(main));
        assert_eq!(windows.get(panel).unwrap().owner(), Some(main));
        assert_eq!(windows.owned_by(main), vec![view, panel]);
    }

    #[test]
    fn unplugging_the_display_a_window_is_on_moves_it_and_it_stays_visible() {
        let mut displays = two_displays();
        let mut windows = Windows::new(&displays);
        // The main window is on display 0; a view on the second display.
        let main = windows.main().unwrap();
        let view = windows.open_view("pages/a.json", &displays).unwrap();
        // Move the view onto display 1 by hand, as a person dragging it would.
        windows
            .move_to(view, DisplayId::from_raw(1), &displays)
            .unwrap();

        // Unplug display 1 and reconcile.
        displays.unplug(DisplayId::from_raw(1));
        let adjustments = windows.reconcile(&displays);

        assert!(
            windows.is_visible(view, &displays),
            "the view is still on screen"
        );
        assert!(windows.is_visible(main, &displays));
        assert!(
            adjustments.iter().any(|adjustment| matches!(
                adjustment,
                Adjustment::Moved { window, .. } if *window == view
            )),
            "the move is reported: {adjustments:?}"
        );
    }

    #[test]
    fn a_scale_change_is_reported_and_the_window_is_pulled_back_inside() {
        let mut displays = one_display();
        let mut windows = Windows::new(&displays);
        let main = windows.main().unwrap();
        // A window near the right edge of a 1280-wide display.
        windows
            .move_to(main, DisplayId::from_raw(0), &displays)
            .unwrap();
        windows.resize(main, 1200, 780).unwrap();
        windows.place_at(main, 80, 10).unwrap();

        // The same display now reproduces at a scale of 2.0 and its logical bounds shrink.
        displays.plug(display(0, 0, 0, 800, 600, 2.0));
        let adjustments = windows.reconcile(&displays);

        assert!(windows.is_visible(main, &displays));
        assert!(
            adjustments.iter().any(|adjustment| matches!(
                adjustment,
                Adjustment::Rescaled { from, to, .. } if *from == 1.0 && *to == 2.0
            )),
            "the scale change is reported: {adjustments:?}"
        );
        let rect = windows.get(main).unwrap().placement().rect();
        assert!(
            rect.right() <= 800 && rect.bottom() <= 600,
            "pulled inside: {}",
            rect.render()
        );
    }

    #[test]
    fn with_no_display_every_window_is_reported_unplaced_and_left_alone() {
        let displays = one_display();
        let mut windows = Windows::new(&displays);
        let before = windows.clone();

        let adjustments = windows.reconcile(&Displays::none());

        assert!(
            adjustments
                .iter()
                .all(|adjustment| matches!(adjustment, Adjustment::NoDisplay { .. })),
            "{adjustments:?}"
        );
        assert_eq!(
            windows, before,
            "nothing is invented to keep it 'on screen'"
        );
        assert!(windows.visible(&Displays::none()).is_empty());
    }

    #[test]
    fn closing_the_main_window_closes_what_it_owns() {
        let displays = one_display();
        let mut windows = Windows::new(&displays);
        let main = windows.main().unwrap();
        windows.open_view("pages/a.json", &displays).unwrap();
        windows.float_panel("outline", &displays).unwrap();
        assert_eq!(windows.len(), 3);

        let closed = windows.close(main).unwrap();
        assert_eq!(closed.len(), 3);
        assert!(windows.is_empty());
        assert_eq!(windows.main(), None);

        // With no main window there is nothing to own a new view.
        assert!(windows.open_view("pages/b.json", &displays).is_err());
        assert!(windows.close(main).is_err());
    }

    #[test]
    fn a_display_a_window_overlaps_most_is_the_one_it_is_on() {
        let displays = two_displays();
        // A rectangle mostly on display 1.
        assert_eq!(
            displays.best_for(Rect::new(1300, 10, 200, 200)),
            Some(DisplayId::from_raw(1))
        );
        // An off-screen rectangle overlaps nothing.
        assert_eq!(displays.best_for(Rect::new(4000, 4000, 200, 200)), None);
        // Spanning both, the larger overlap wins.
        assert_eq!(
            displays.best_for(Rect::new(1000, 0, 1000, 100)),
            Some(DisplayId::from_raw(1))
        );
    }

    #[test]
    fn a_rectangle_too_big_for_a_display_is_aligned_to_its_origin() {
        let outer = Rect::new(100, 50, 300, 200);
        assert_eq!(Rect::new(0, 0, 400, 400).clamp_into(outer), outer);
        // A fitting one is slid inside rather than resized.
        assert_eq!(
            Rect::new(0, 0, 100, 100).clamp_into(outer),
            Rect::new(100, 50, 100, 100)
        );
    }

    #[test]
    fn a_scale_that_is_not_a_scale_is_not_a_display() {
        assert!(Display::new(DisplayId::from_raw(0), Rect::new(0, 0, 10, 10), 0.0).is_none());
        assert!(Display::new(DisplayId::from_raw(0), Rect::new(0, 0, 10, 10), -1.0).is_none());
        assert!(Display::new(DisplayId::from_raw(0), Rect::new(0, 0, 10, 10), f64::NAN).is_none());
        assert!(Display::new(DisplayId::from_raw(0), Rect::new(0, 0, 10, 10), 1.5).is_some());
    }

    #[test]
    fn every_window_reports_its_kind_owner_and_visibility() {
        let displays = one_display();
        let mut windows = Windows::new(&displays);
        windows.open_view("pages/a.json", &displays).unwrap();
        let report = windows.render(&displays).join("\n");
        assert!(report.contains("2 window(s)"), "{report}");
        assert!(report.contains("window 0 [main]"), "{report}");
        assert!(
            report.contains("window 1 [view] pages/a.json owned by 0"),
            "{report}"
        );
        assert!(report.contains("visible"), "{report}");
        assert_eq!(MIN_VISIBLE, (64, 32));

        // A window far off every display is reported NOT VISIBLE rather than quietly dropped.
        let main = windows.main().unwrap();
        windows.place_at(main, 9000, 9000).unwrap();
        let report = windows.render(&displays).join("\n");
        assert!(report.contains("NOT VISIBLE"), "{report}");
    }
}
