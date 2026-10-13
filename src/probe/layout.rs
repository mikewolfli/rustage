// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Layout probes: the designer's preview and its generator must both solve geometry with the
//! library's own layout engine, never a private one (principle #14: layout must not fork).
//!
//! These probes drive [`FlexLayout`] through its two entry points — the legacy
//! `add_widget`/`update` path and the hint channel `arrange` — and read the rectangles back. The
//! observed geometry is asserted exactly, because "the layout ran" is not evidence that it placed
//! anything: a layout that returned zero rectangles would satisfy a weaker check.

use rust_widgets::core::Rect;
use rust_widgets::layout::{ChildInfo, FlexLayout, Hints, Layout};

use super::{ApiFamily, ProbeResult, require};

/// The family these probes belong to.
const FAMILY: ApiFamily = ApiFamily::Layout;

/// Runs every layout probe.
pub fn probes() -> Vec<ProbeResult> {
    vec![child_registry(), row_geometry(), hint_channel_geometry()]
}

/// `FlexLayout::add_widget` + `Layout::child_ids`: the registry the preview walks.
fn child_registry() -> ProbeResult {
    const API: &str = "layout::FlexLayout::add_widget/child_ids";
    let mut flex = FlexLayout::new();
    flex.add_widget(11, 1);
    flex.add_widget(22, 1);
    flex.add_widget(33, 0);

    let ids = flex.child_ids();
    require!(
        ids == vec![11, 22, 33],
        FAMILY,
        API,
        format!("child_ids returned {ids:?}, expected [11, 22, 33]")
    );
    require!(
        flex.item_count() == 3,
        FAMILY,
        API,
        format!("item_count returned {}, expected 3", flex.item_count())
    );

    ProbeResult::verified(FAMILY, API, vec![format!("child_ids -> {ids:?}")])
}

/// `Layout::update`: two equal-weight children split a 100 px row, reading back exact rectangles.
fn row_geometry() -> ProbeResult {
    const API: &str = "layout::Layout::update";
    let mut flex = FlexLayout::new();
    flex.add_widget(1, 1);
    flex.add_widget(2, 1);

    let first = collect(&flex, Rect::new(0, 0, 100, 40));
    let second = collect(&flex, Rect::new(0, 0, 100, 40));

    let expected = vec![(1, Rect::new(0, 0, 50, 40)), (2, Rect::new(50, 0, 50, 40))];
    require!(
        first == expected,
        FAMILY,
        API,
        format!(
            "row geometry was {}, expected {}",
            render(&first),
            render(&expected)
        )
    );
    require!(
        first == second,
        FAMILY,
        API,
        "two runs over one layout disagreed; the layout is not deterministic"
    );

    ProbeResult::verified(FAMILY, API, vec![render(&first)])
}

/// `Layout::arrange`: the hint channel places two fixed-size children left to right.
fn hint_channel_geometry() -> ProbeResult {
    const API: &str = "layout::Layout::arrange";
    let mut flex = FlexLayout::new();
    flex.add_widget(1, 0);
    flex.add_widget(2, 0);

    let children = [
        ChildInfo::new(1, Hints::fixed(30, 20)),
        ChildInfo::new(2, Hints::fixed(30, 20)),
    ];
    let mut placed = Vec::new();
    flex.arrange(Rect::new(0, 0, 100, 40), &children, &mut |id, rect| {
        placed.push((id, rect));
    });

    let ids: Vec<u64> = placed.iter().map(|(id, _)| *id).collect();
    require!(
        ids == vec![1, 2],
        FAMILY,
        API,
        format!("arrange placed {ids:?}, expected [1, 2]")
    );

    // The main axis is what the hint channel governs: each child keeps the 30 px it stated, and
    // they are packed from the leading edge in declaration order.
    let main_axis: Vec<(u64, i32, u32)> =
        placed.iter().map(|(id, r)| (*id, r.x, r.width)).collect();
    let expected_main = vec![(1, 0, 30), (2, 30, 30)];
    require!(
        main_axis == expected_main,
        FAMILY,
        API,
        format!("hint-channel main axis was {main_axis:?}, expected {expected_main:?}")
    );

    // The cross axis must stay inside the band. It is not asserted exactly because the default
    // `AlignItems::Stretch` may fill it, which is a legitimate layout choice rather than a
    // contract the designer depends on.
    let inside = placed
        .iter()
        .all(|(_, r)| r.y >= 0 && r.y + r.height as i32 <= 40);
    require!(
        inside,
        FAMILY,
        API,
        format!("a child left the band: {}", render(&placed))
    );

    ProbeResult::verified(FAMILY, API, vec![render(&placed)])
}

/// Runs `update` and collects the placements in callback order.
fn collect(flex: &FlexLayout, rect: Rect) -> Vec<(u64, Rect)> {
    let mut placed = Vec::new();
    flex.update(rect, &mut |id, child| placed.push((id, child)));
    placed
}

/// Renders placements for a note or a failure message.
fn render(placed: &[(u64, Rect)]) -> String {
    let parts: Vec<String> = placed
        .iter()
        .map(|(id, r)| format!("{id} -> ({},{},{},{})", r.x, r.y, r.width, r.height))
        .collect();
    parts.join(", ")
}

#[cfg(test)]
mod tests {
    use super::probes;
    use crate::probe::ProbeOutcome;

    #[test]
    fn every_layout_probe_is_verified() {
        for probe in probes() {
            assert!(
                matches!(probe.outcome, ProbeOutcome::Verified),
                "layout probe {} was {:?}",
                probe.api,
                probe.outcome
            );
        }
    }

    #[test]
    fn the_probe_set_is_deterministic() {
        assert_eq!(probes(), probes());
    }
}
