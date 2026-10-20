// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Responsive chrome layout for the workbench window (F04/F10).
//!
//! The first workbench slice placed every control at a fixed pixel rectangle, so a resized
//! window kept the 1280x800 arrangement and left the extra space blank. This module makes the
//! chrome follow the window: the panels keep their share of the width, the editor takes what is
//! left, and the toolbar shares the top row evenly.
//!
//! # Why this is a [`rust_widgets::layout::Layout`] rather than a second algorithm
//!
//! The rectangles are computed here because the workbench's own chrome (a menu bar, one toolbar
//! row, two side columns, a bottom row, a status bar) is not a document subtree and has no
//! `View` tree in the model. The *designer document* layout still goes through the library's
//! `Layout` engine (`app::layout`); this module is the host window's arrangement, and it plugs
//! into the same seam the library's own layouts use. `WindowHandle::set_layout` re-applies it on
//! every `Resized` event, which is what makes "resize the window" a real path rather than a
//! startup-only calculation.
//!
//! A layout child registered through [`WorkbenchLayout::place`] is always written; a widget added
//! through the bare `Layout::add_widget` has no place and is left where the runtime put it, rather
//! than being given a guessed slot.

use std::any::Any;

use rust_widgets::core::{ObjectId, Rect};
use rust_widgets::layout::{BorderLayout, BorderRegion, Layout};

/// The gap between regions, and the window's outer margin.
const GAP: u32 = 8;
/// The menu bar row.
const MENU_HEIGHT: u32 = 24;
/// The toolbar row.
const TOOLBAR_HEIGHT: u32 = 34;
/// The tab strip above an editor group.
const TAB_HEIGHT: u32 = 28;
/// The status bar row.
const STATUS_HEIGHT: u32 = 28;
/// A side panel narrower than this is not useful, so it is dropped on a narrow window.
const MIN_SIDE: u32 = 840;

/// Where one widget sits in the workbench, expressed relative to the window rather than in
/// pixels so the arrangement survives a resize.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Area {
    /// The menu bar, full width at the top.
    Menu,
    /// One toolbar button; `count` buttons share the row.
    Toolbar { index: u32, count: u32 },
    /// One of the left panels, stacked in the content column.
    LeftPanel { index: u32, count: u32 },
    /// One of the right panels, stacked in the content column.
    RightPanel { index: u32, count: u32 },
    /// One of the bottom panels, side by side under the editor.
    BottomPanel { index: u32, count: u32 },
    /// One editor-group tab.
    Tab {
        /// Which editor group owns the tab.
        group: u32,
        /// The tab's position in the strip.
        index: u32,
        /// How many tabs the strip holds.
        count: u32,
    },
    /// An editor group's text area.
    Editor {
        /// Which editor group.
        group: u32,
    },
    /// The design canvas, over the first editor group's text area.
    Canvas,
    /// The status bar, full width at the bottom.
    Status,
    /// The command palette, centred over the content while it is open.
    Palette,
}

// The last client size any workbench layout pass ran for on this thread.
//
// A split or a panel toggle replaces the layout object, and each object would otherwise log its
// first pass again for the same size. The fact being reported is the window's size, so it is
// kept per thread rather than per layout instance.
thread_local! {
    static LAST_SIZE: std::cell::Cell<Option<(u32, u32, u32)>> = const { std::cell::Cell::new(None) };
}

/// The workbench's responsive layout.
pub(super) struct WorkbenchLayout {
    placements: Vec<(ObjectId, Area)>,
    /// How many editor groups are currently visible. The same model decides it, so the layout
    /// cannot disagree with [`crate::app::shell::groups::Groups`] about whether the editor is
    /// split.
    groups: u32,
}

impl WorkbenchLayout {
    /// A layout with no children yet.
    pub(super) fn new(groups: u32) -> Self {
        Self {
            placements: Vec::new(),
            groups: groups.max(1),
        }
    }

    /// Registers `id` at `area`. Registering the same widget twice keeps the last place, which
    /// is what a rebuild after a model change expects.
    pub(super) fn place(&mut self, id: ObjectId, area: Area) -> &mut Self {
        if let Some(entry) = self.placements.iter_mut().find(|(widget, _)| *widget == id) {
            entry.1 = area;
        } else {
            self.placements.push((id, area));
        }
        self
    }

    /// Computes every registered rectangle for a client rect.
    fn geometry(&self, rect: Rect) -> Vec<(ObjectId, Rect)> {
        let width = rect.width;
        let height = rect.height;
        let side = width >= MIN_SIDE;
        let left_width = if side { (width / 5).clamp(180, 300) } else { 0 };
        let right_width = if side { (width / 4).clamp(200, 340) } else { 0 };
        let center_x = rect.x + left_width as i32 + if left_width > 0 { GAP as i32 } else { 0 };
        let center_width = width
            .saturating_sub(left_width)
            .saturating_sub(right_width)
            .saturating_sub(if left_width > 0 { GAP } else { 0 })
            .saturating_sub(if right_width > 0 { GAP } else { 0 });
        let toolbar_y = rect.y + MENU_HEIGHT as i32 + GAP as i32;
        let content_top = toolbar_y + TOOLBAR_HEIGHT as i32 + GAP as i32;
        let status_y = rect.y + height.saturating_sub(STATUS_HEIGHT) as i32;
        let bottom_height = (height / 7).clamp(90, 150);
        let bottom_y = status_y - GAP as i32 - bottom_height as i32;
        let content_height = (bottom_y - GAP as i32 - content_top).max(0) as u32;

        // The region shapes are still the workbench's own (this chrome is not a document subtree),
        // but the pixel allocation inside each region is the library's `BorderLayout`: fixed
        // tracks for a vertical stack, equal-stretch center tracks for a row. That keeps the
        // spacing/clamp rules out of the host (feedback-20261020 §3.2, closed by 2.8.11).
        let left_slices = slices(
            Rect::new(rect.x, content_top, left_width, content_height),
            self.count(|area| matches!(area, Area::LeftPanel { .. })),
            true,
        );
        let right_slices = slices(
            Rect::new(
                rect.x + width.saturating_sub(right_width) as i32,
                content_top,
                right_width,
                content_height,
            ),
            self.count(|area| matches!(area, Area::RightPanel { .. })),
            true,
        );
        let bottom_slices = slices(
            Rect::new(center_x, bottom_y, center_width, bottom_height),
            self.count(|area| matches!(area, Area::BottomPanel { .. })),
            false,
        );
        let group_slices = slices(
            Rect::new(center_x, content_top, center_width, content_height),
            self.groups,
            false,
        );

        let mut out = Vec::with_capacity(self.placements.len());
        for (id, area) in &self.placements {
            let placed = match *area {
                Area::Menu => Rect::new(rect.x, rect.y, width, MENU_HEIGHT),
                Area::Toolbar { index, count } => {
                    let count = count.max(1);
                    let usable = width.saturating_sub(GAP * 2 + GAP.saturating_mul(count - 1));
                    let button_width = usable / count;
                    let x = rect.x + GAP as i32 + index as i32 * (button_width + GAP) as i32;
                    Rect::new(x, toolbar_y, button_width, TOOLBAR_HEIGHT)
                }
                Area::LeftPanel { index, .. } => slice(&left_slices, index, rect),
                Area::RightPanel { index, .. } => slice(&right_slices, index, rect),
                Area::BottomPanel { index, .. } => slice(&bottom_slices, index, rect),
                Area::Tab {
                    group,
                    index,
                    count,
                } => {
                    let group_box = slice(&group_slices, group, rect);
                    let strip_width = group_box.width.min(120 * count.max(1));
                    let tab_slices = slices(
                        Rect::new(group_box.x, group_box.y, strip_width, TAB_HEIGHT),
                        count,
                        false,
                    );
                    slice(&tab_slices, index, rect)
                }
                Area::Editor { group } => {
                    let group_box = slice(&group_slices, group, rect);
                    Rect::new(
                        group_box.x,
                        group_box.y + TAB_HEIGHT as i32 + GAP as i32,
                        group_box.width,
                        content_height.saturating_sub(TAB_HEIGHT + GAP),
                    )
                }
                Area::Canvas => {
                    let group_box = slice(&group_slices, 0, rect);
                    Rect::new(
                        group_box.x,
                        group_box.y + TAB_HEIGHT as i32 + GAP as i32,
                        group_box.width,
                        content_height.saturating_sub(TAB_HEIGHT + GAP),
                    )
                }
                Area::Status => Rect::new(rect.x, status_y, width, STATUS_HEIGHT),
                Area::Palette => {
                    // An overlay, not a dock: it is given the whole client rect so it can centre
                    // itself, and the painter's creation order keeps it above the docked panels.
                    // The bounds keep it usable on a narrow or short window.
                    let palette_width = width.saturating_sub(GAP * 2).min(640);
                    let palette_height = height
                        .saturating_sub(MENU_HEIGHT + TOOLBAR_HEIGHT + STATUS_HEIGHT + GAP * 4)
                        .min(420);
                    Rect::new(
                        rect.x + (width.saturating_sub(palette_width) / 2) as i32,
                        content_top,
                        palette_width,
                        palette_height,
                    )
                }
            };
            out.push((*id, placed));
        }
        out
    }

    /// The number of tracks one area role needs, taken from the registered placements.
    fn count(&self, wanted: impl Fn(&Area) -> bool) -> u32 {
        self.placements
            .iter()
            .filter(|(_, area)| wanted(area))
            .filter_map(|(_, area)| match area {
                Area::LeftPanel { count, .. }
                | Area::RightPanel { count, .. }
                | Area::BottomPanel { count, .. } => Some(*count),
                _ => None,
            })
            .max()
            .unwrap_or(1)
            .max(1)
    }
}

/// One slice of a pre-allocated region, or the whole rect when the region is not there.
fn slice(slices: &[Rect], index: u32, fallback: Rect) -> Rect {
    slices.get(index as usize).copied().unwrap_or(fallback)
}

/// Splits `area` into `count` tracks with the library's [`BorderLayout`].
///
/// A vertical stack is a `North` track per slice (fixed height); a row is an equal-stretch
/// `Center` track per slice. The library clamps tracks to the parent and reduces spacing when it
/// cannot be paid, so a tiny window cannot place a control outside its region.
fn slices(area: Rect, count: u32, vertical: bool) -> Vec<Rect> {
    let count = count.max(1);
    let mut border = BorderLayout::new(GAP, 0);
    if vertical {
        let pane = area.height.saturating_sub(GAP.saturating_mul(count - 1)) / count;
        for index in 0..count {
            border.add(BorderRegion::North, u64::from(index) + 1, pane);
        }
    } else {
        for index in 0..count {
            border.add_with_stretch(BorderRegion::Center, u64::from(index) + 1, 0, 1);
        }
    }
    let mut out = vec![Rect::new(area.x, area.y, 0, 0); count as usize];
    border.update(area, &mut |id, rect| {
        if let Some(slot) = out.get_mut((id - 1) as usize) {
            *slot = rect;
        }
    });
    out
}

impl Layout for WorkbenchLayout {
    fn add_widget(&mut self, widget_id: ObjectId, _stretch: u32) {
        // This layout is built from explicit [`Area`]s by [`WorkbenchLayout::place`]. The generic
        // trait has no place to put a widget it was not told about, so it is left where the
        // widget runtime already positioned it rather than given a guessed slot. The workbench
        // builder registers every child through `place` before `set_layout`.
        let _ = widget_id;
    }

    fn remove_widget(&mut self, widget_id: ObjectId) {
        self.placements.retain(|(id, _)| *id != widget_id);
    }

    fn update(&self, rect: Rect, widgets: &mut dyn FnMut(ObjectId, Rect)) {
        let changed = LAST_SIZE.with(|last| {
            let key = (rect.width, rect.height, self.groups);
            if last.get() == Some(key) {
                false
            } else {
                last.set(Some(key));
                true
            }
        });
        if changed {
            crate::host::log::emit(
                crate::host::log::Level::Info,
                "app::run::workbench-layout",
                &format!(
                    "workbench layout for {}x{}: {} widget(s), {} editor group(s)",
                    rect.width,
                    rect.height,
                    self.placements.len(),
                    self.groups
                ),
            );
        }
        for (id, placed) in self.geometry(rect) {
            widgets(id, placed);
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::{Area, WorkbenchLayout};
    use rust_widgets::core::Rect;
    use rust_widgets::layout::Layout;
    use std::collections::BTreeMap;

    fn run(width: u32, height: u32) -> BTreeMap<u64, Rect> {
        let mut layout = WorkbenchLayout::new(2);
        layout
            .place(1, Area::Menu)
            .place(2, Area::Toolbar { index: 0, count: 4 })
            .place(3, Area::Toolbar { index: 3, count: 4 })
            .place(4, Area::LeftPanel { index: 0, count: 2 })
            .place(5, Area::LeftPanel { index: 1, count: 2 })
            .place(6, Area::RightPanel { index: 0, count: 2 })
            .place(7, Area::RightPanel { index: 1, count: 2 })
            .place(8, Area::BottomPanel { index: 0, count: 2 })
            .place(9, Area::BottomPanel { index: 1, count: 2 })
            .place(
                10,
                Area::Tab {
                    group: 0,
                    index: 0,
                    count: 3,
                },
            )
            .place(11, Area::Editor { group: 0 })
            .place(12, Area::Editor { group: 1 })
            .place(13, Area::Status)
            .place(14, Area::Palette);
        let mut out = BTreeMap::new();
        layout.update(Rect::new(0, 0, width, height), &mut |id, rect| {
            out.insert(id, rect);
        });
        out
    }

    #[test]
    fn the_chrome_follows_the_window_size() {
        let small = run(1000, 700);
        let large = run(1600, 1000);

        assert_eq!(small[&1].width, 1000);
        assert_eq!(large[&1].width, 1600);
        assert!(large[&4].width > small[&4].width, "the side column scales");
        assert!(large[&11].height > small[&11].height, "the editor grows");
        assert!(
            large[&13].y > small[&13].y,
            "the status bar stays at the bottom"
        );
    }

    #[test]
    fn a_narrow_window_drops_the_side_columns_instead_of_overlapping_the_editor() {
        let narrow = run(700, 500);
        assert_eq!(narrow[&4].width, 0);
        assert_eq!(narrow[&6].width, 0);
        assert!(narrow[&11].width > 0, "the editor still has the centre");
        assert!(narrow[&11].x >= 0);
    }

    #[test]
    fn the_editor_groups_do_not_overlap_when_split() {
        let layout = run(1280, 800);
        assert_eq!(layout[&11].x + layout[&11].width as i32, layout[&12].x - 8);
        assert_eq!(layout[&11].width, layout[&12].width);
    }

    #[test]
    fn the_bottom_panels_share_one_row() {
        let layout = run(1280, 800);
        assert_eq!(
            layout[&8].y, layout[&9].y,
            "the bottom panels must share a row"
        );
        assert_eq!(layout[&8].height, layout[&9].height);
        assert_eq!(layout[&8].x + layout[&8].width as i32, layout[&9].x - 8);
        assert!(layout[&8].width > 0 && layout[&8].height > 0);
    }

    #[test]
    fn the_palette_is_centred_and_stays_inside_the_window() {
        let layout = run(1000, 700);
        let palette = layout[&14];
        assert!(palette.width > 0 && palette.height > 0);
        assert_eq!(palette.x, (1000 - palette.width as i32) / 2);
        assert!(palette.x + palette.width as i32 <= 1000);
        assert!(palette.y + palette.height as i32 <= 700);

        let tiny = run(320, 240);
        assert!(tiny[&14].width <= 320);
        assert!(tiny[&14].height <= 240);
    }

    #[test]
    fn every_rectangle_stays_inside_a_tiny_window() {
        let tiny = run(320, 240);
        for (id, rect) in tiny {
            assert!(rect.x >= 0, "widget {id} starts outside");
            assert!(rect.y >= 0, "widget {id} starts outside");
            assert!(rect.width <= 320, "widget {id} is wider than the window");
            assert!(rect.height <= 240, "widget {id} is taller than the window");
        }
    }
}
