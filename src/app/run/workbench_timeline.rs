// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The transition-preview panel's rows (roadmap F11, `panel.timeline`).
//!
//! The rows come from the page document's `motion` member (page format v5). The document types are
//! turned into the host's [`Timeline`] and [`Control`] only to reuse their timing and playback
//! semantics, so the panel does not compute a second total duration.
//!
//! **Boundary.** Format v5 stores the declared motions. It does not store the target's capability
//! table, a state comparison or a running preview; the panel says those are not read rather than
//! drawing a play button that cannot animate anything.

use crate::app::style::motion::{Control, Easing, Effect, Motion, Timeline, Timing};
use crate::project::record::page::{PageEasing, PageEffect, PageMotions};

/// The rows for one page's transition declaration.
pub(super) fn rows(source: &str, motions: &PageMotions) -> Vec<String> {
    let mut rows = vec![format!("page: {source}")];
    if motions.motions().is_empty() {
        rows.push("no motion is declared in this page".to_owned());
        rows.push("playback: paused at 0ms / 0ms".to_owned());
        return rows;
    }
    let timeline = Timeline {
        motions: motions
            .motions()
            .iter()
            .map(|motion| Motion {
                effect: match motion.effect() {
                    PageEffect::Transition => Effect::Transition,
                    PageEffect::Animation => Effect::Animation,
                },
                property: motion.property().to_owned(),
                timing: Timing::new(
                    motion.duration_ms(),
                    motion.delay_ms(),
                    match motion.easing() {
                        PageEasing::Linear => Easing::Linear,
                        PageEasing::EaseIn => Easing::EaseIn,
                        PageEasing::EaseOut => Easing::EaseOut,
                        PageEasing::EaseInOut => Easing::EaseInOut,
                    },
                ),
            })
            .collect(),
    };
    rows.push(format!(
        "motions: {} · duration: {}ms",
        timeline.motions.len(),
        timeline.duration_ms()
    ));
    for motion in &timeline.motions {
        rows.push(format!(
            "{} {} {}",
            motion.effect.name(),
            motion.property,
            motion.timing.render()
        ));
    }
    let control = Control::new(timeline.duration_ms());
    rows.push(format!("playback: {}", control.render()));
    rows.push("target capability: not checked".to_owned());
    rows.push("state comparison: not declared in page format v5".to_owned());
    rows
}

#[cfg(test)]
mod tests {
    use super::rows;
    use crate::project::record::page::{PageEasing, PageEffect, PageMotion, PageMotions};

    #[test]
    fn an_empty_motion_section_says_so() {
        let rows = rows("pages/main.json", &PageMotions::default());
        assert_eq!(rows[0], "page: pages/main.json");
        assert!(rows.iter().any(|row| row.contains("no motion")));
        assert!(rows.iter().any(|row| row.contains("paused at 0ms / 0ms")));
    }

    #[test]
    fn motions_are_rendered_with_their_timing_and_the_total() {
        let motions = PageMotions::new(vec![
            PageMotion::new(PageEffect::Transition, "color", 200, 0, PageEasing::EaseOut),
            PageMotion::new(
                PageEffect::Animation,
                "opacity",
                400,
                100,
                PageEasing::Linear,
            ),
        ]);
        let rows = rows("pages/main.json", &motions);
        assert!(
            rows.iter().any(|row| row == "motions: 2 · duration: 500ms"),
            "{rows:?}"
        );
        assert!(
            rows.iter()
                .any(|row| row.contains("transition color 200ms (+0ms delay, ease-out)")),
            "{rows:?}"
        );
        assert!(
            rows.iter()
                .any(|row| row.contains("animation opacity 400ms (+100ms delay, linear)")),
            "{rows:?}"
        );
        assert!(
            rows.iter()
                .any(|row| row == "playback: paused at 0ms / 500ms"),
            "{rows:?}"
        );
    }
}
