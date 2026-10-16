// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Transitions and animations: the support table, the timing parameters and the preview control
//! (task T11.06).
//!
//! The acceptance line is *"the support table comes from the contract, and a restricted target
//! refuses incompatible items"*. The module never guesses what a target can do:
//!
//! * **Support is what the contract says, not what a name suggests.** [`Capability`] carries the
//!   target's own flags and tables, and [`admit`] reads only those — a target that declares
//!   transitions but lists no animatable property admits nothing, because "transitions exist" is not
//!   "this property transitions" (principle 12).
//! * **A refusal names the item that is incompatible.** [`Verdict::Refused`] says which property,
//!   effect or easing the target cannot take, so a blocked motion is diagnosable rather than dropped
//!   (principle 22).
//! * **The timeline is the longest motion.** [`Timeline::duration_ms`] is the greatest delay plus
//!   duration, because the motions on one change run together; a preview has one length, not a sum.
//! * **The control clamps and says so.** [`Control::seek`] answers whether it had to clamp, so
//!   "scrubbed past the end" is visible instead of a silently wrong position.

#![allow(dead_code)]

use std::collections::BTreeSet;

/// How a motion's value moves over time.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Easing {
    /// Constant speed.
    Linear,
    /// Slow start.
    EaseIn,
    /// Slow end.
    EaseOut,
    /// Slow start and end.
    EaseInOut,
}

impl Easing {
    /// Every easing, in a stable order.
    pub const ALL: [Self; 4] = [Self::Linear, Self::EaseIn, Self::EaseOut, Self::EaseInOut];

    /// The easing's name, as a contract writes it.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Linear => "linear",
            Self::EaseIn => "ease-in",
            Self::EaseOut => "ease-out",
            Self::EaseInOut => "ease-in-out",
        }
    }

    /// Reads an easing's name.
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|easing| easing.name() == text)
    }
}

/// Which of the two effects a motion is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Effect {
    /// A transition: runs once, on a state change.
    Transition,
    /// An animation: runs forward, possibly more than once.
    Animation,
}

impl Effect {
    /// The effect's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Transition => "transition",
            Self::Animation => "animation",
        }
    }

    /// Reads an effect's name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "transition" => Some(Self::Transition),
            "animation" => Some(Self::Animation),
            _ => None,
        }
    }
}

/// A motion's time parameters.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Timing {
    /// How long it runs.
    pub duration_ms: u32,
    /// How long before it starts.
    pub delay_ms: u32,
    /// How it moves.
    pub easing: Easing,
}

impl Timing {
    /// Timing with the three parameters.
    pub fn new(duration_ms: u32, delay_ms: u32, easing: Easing) -> Self {
        Self {
            duration_ms,
            delay_ms,
            easing,
        }
    }

    /// Delay plus duration: how long until the motion settles.
    pub fn total_ms(&self) -> u32 {
        self.delay_ms.saturating_add(self.duration_ms)
    }

    /// The timing in one line.
    pub fn render(&self) -> String {
        format!(
            "{}ms (+{}ms delay, {})",
            self.duration_ms,
            self.delay_ms,
            self.easing.name()
        )
    }
}

/// One transition or animation on one property.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Motion {
    /// Whether it is a transition or an animation.
    pub effect: Effect,
    /// The property it moves.
    pub property: String,
    /// Its time parameters.
    pub timing: Timing,
}

/// What a target declares it supports (from its contract, never from this module).
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Capability {
    /// Whether the target supports transitions at all.
    pub transitions: bool,
    /// Whether the target supports animations at all.
    pub animations: bool,
    /// The properties the target can move.
    pub properties: BTreeSet<String>,
    /// The easings the target can run.
    pub easings: BTreeSet<String>,
}

impl Capability {
    /// A target that supports neither effect and lists nothing.
    pub fn new() -> Self {
        Self::default()
    }

    /// The same capability with the transition flag set.
    pub fn with_transitions(mut self, supported: bool) -> Self {
        self.transitions = supported;
        self
    }

    /// The same capability with the animation flag set.
    pub fn with_animations(mut self, supported: bool) -> Self {
        self.animations = supported;
        self
    }

    /// The same capability with these properties added to its table.
    pub fn with_properties(mut self, names: &[&str]) -> Self {
        self.properties
            .extend(names.iter().map(|name| (*name).to_owned()));
        self
    }

    /// The same capability with these easings added to its table.
    pub fn with_easings(mut self, names: &[&str]) -> Self {
        self.easings
            .extend(names.iter().map(|name| (*name).to_owned()));
        self
    }

    /// Whether the target supports an effect at all.
    pub fn supports(&self, effect: Effect) -> bool {
        match effect {
            Effect::Transition => self.transitions,
            Effect::Animation => self.animations,
        }
    }

    /// Whether the target can move a property.
    pub fn supports_property(&self, property: &str) -> bool {
        self.properties.contains(property)
    }

    /// Whether the target can run an easing.
    pub fn supports_easing(&self, easing: Easing) -> bool {
        self.easings.contains(easing.name())
    }

    /// The support table in one line per row.
    pub fn render(&self) -> Vec<String> {
        let properties: Vec<&str> = self.properties.iter().map(String::as_str).collect();
        let easings: Vec<&str> = self.easings.iter().map(String::as_str).collect();
        vec![
            format!(
                "transitions: {}",
                if self.transitions { "on" } else { "off" }
            ),
            format!("animations: {}", if self.animations { "on" } else { "off" }),
            format!("properties: {}", list_or_none(&properties)),
            format!("easings: {}", list_or_none(&easings)),
        ]
    }
}

/// Whether a motion is compatible with a target.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Verdict {
    /// The target can run the motion.
    Accepted,
    /// It cannot, and this says which item is incompatible.
    Refused {
        /// Why it cannot.
        reason: String,
    },
}

impl Verdict {
    /// Whether the motion was accepted.
    pub fn is_accepted(&self) -> bool {
        matches!(self, Self::Accepted)
    }

    /// The refusal's reason, if it was refused.
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Accepted => None,
            Self::Refused { reason } => Some(reason),
        }
    }

    /// The verdict in one line.
    pub fn render(&self, label: &str) -> String {
        match self {
            Self::Accepted => format!("{label}: accepted"),
            Self::Refused { reason } => format!("{label}: REFUSED — {reason}"),
        }
    }
}

/// Admits or refuses a motion against a target's capability.
///
/// The order is the order a designer would ask: the effect first, then the property, then the easing.
pub fn admit(motion: &Motion, capability: &Capability) -> Verdict {
    if !capability.supports(motion.effect) {
        return Verdict::Refused {
            reason: format!("the target does not support {}", motion.effect.name()),
        };
    }
    if !capability.supports_property(&motion.property) {
        return Verdict::Refused {
            reason: format!("the target cannot move {}", motion.property),
        };
    }
    if !capability.supports_easing(motion.timing.easing) {
        return Verdict::Refused {
            reason: format!(
                "the target does not support the {} easing",
                motion.timing.easing.name()
            ),
        };
    }
    Verdict::Accepted
}

/// The motions of a design, and how long the whole set takes.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Timeline {
    /// The motions.
    pub motions: Vec<Motion>,
}

impl Timeline {
    /// A timeline with no motions.
    pub fn new() -> Self {
        Self::default()
    }

    /// The same timeline with one more motion.
    pub fn with(mut self, motion: Motion) -> Self {
        self.motions.push(motion);
        self
    }

    /// How long the set takes: the longest delay plus duration, or zero when there is none.
    pub fn duration_ms(&self) -> u32 {
        self.motions
            .iter()
            .map(|motion| motion.timing.total_ms())
            .max()
            .unwrap_or(0)
    }

    /// Admits every motion, in order.
    pub fn admit(&self, capability: &Capability) -> Vec<(Motion, Verdict)> {
        self.motions
            .iter()
            .map(|motion| (motion.clone(), admit(motion, capability)))
            .collect()
    }
}

/// The preview's playback control.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Control {
    /// Whether the preview is running.
    pub playing: bool,
    /// Where it is, in milliseconds.
    pub position_ms: u32,
    /// How long the timeline is.
    pub duration_ms: u32,
}

impl Control {
    /// A control paused at the start of a timeline.
    pub fn new(duration_ms: u32) -> Self {
        Self {
            playing: false,
            position_ms: 0,
            duration_ms,
        }
    }

    /// Starts the preview.
    pub fn play(&mut self) {
        self.playing = true;
    }

    /// Pauses the preview where it is.
    pub fn pause(&mut self) {
        self.playing = false;
    }

    /// Seeks to a position, clamped to the timeline; answers whether it had to clamp.
    pub fn seek(&mut self, position_ms: u32) -> bool {
        let clamped = position_ms > self.duration_ms;
        self.position_ms = position_ms.min(self.duration_ms);
        clamped
    }

    /// The control in one line.
    pub fn render(&self) -> String {
        format!(
            "{} at {}ms / {}ms",
            if self.playing { "playing" } else { "paused" },
            self.position_ms,
            self.duration_ms
        )
    }
}

/// `a, b, c`, or `(none)` when the list is empty.
fn list_or_none(names: &[&str]) -> String {
    if names.is_empty() {
        "(none)".to_owned()
    } else {
        names.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::{Capability, Control, Easing, Effect, Motion, Timeline, Timing, Verdict, admit};

    fn target() -> Capability {
        Capability::new()
            .with_transitions(true)
            .with_properties(&["color"])
            .with_easings(&["linear"])
    }

    fn motion(effect: Effect, property: &str, easing: Easing) -> Motion {
        Motion {
            effect,
            property: property.to_owned(),
            timing: Timing::new(200, 0, easing),
        }
    }

    #[test]
    fn a_supported_motion_is_accepted() {
        let verdict = admit(
            &motion(Effect::Transition, "color", Easing::Linear),
            &target(),
        );
        assert_eq!(verdict, Verdict::Accepted);
        assert!(verdict.is_accepted());
        assert_eq!(verdict.render("color"), "color: accepted");
    }

    #[test]
    fn a_target_that_does_not_support_transitions_refuses_them() {
        let capability = Capability::new()
            .with_properties(&["color"])
            .with_easings(&["linear"]);
        let verdict = admit(
            &motion(Effect::Transition, "color", Easing::Linear),
            &capability,
        );
        assert_eq!(
            verdict,
            Verdict::Refused {
                reason: "the target does not support transition".to_owned()
            }
        );
    }

    #[test]
    fn an_animation_is_refused_when_only_transitions_are_supported() {
        let verdict = admit(
            &motion(Effect::Animation, "color", Easing::Linear),
            &target(),
        );
        assert!(verdict.reason().expect("a reason").contains("animation"));
    }

    #[test]
    fn a_property_outside_the_contract_is_refused_and_named() {
        let verdict = admit(
            &motion(Effect::Transition, "gap", Easing::Linear),
            &target(),
        );
        assert_eq!(verdict.reason(), Some("the target cannot move gap"),);
        assert!(verdict.render("gap").contains("REFUSED"));
    }

    #[test]
    fn an_easing_outside_the_contract_is_refused_and_named() {
        let verdict = admit(
            &motion(Effect::Transition, "color", Easing::EaseInOut),
            &target(),
        );
        assert_eq!(
            verdict.reason(),
            Some("the target does not support the ease-in-out easing"),
        );
    }

    #[test]
    fn the_table_comes_from_the_capability_not_from_the_effect_flag() {
        let capability = Capability::new()
            .with_transitions(true)
            .with_easings(&["linear"]);
        let verdict = admit(
            &motion(Effect::Transition, "color", Easing::Linear),
            &capability,
        );
        assert!(
            verdict.reason().is_some(),
            "transitions alone admit nothing"
        );
    }

    #[test]
    fn the_timeline_is_the_longest_delay_plus_duration() {
        let timeline = Timeline::new()
            .with(Motion {
                effect: Effect::Transition,
                property: "color".to_owned(),
                timing: Timing::new(200, 0, Easing::Linear),
            })
            .with(Motion {
                effect: Effect::Transition,
                property: "gap".to_owned(),
                timing: Timing::new(100, 150, Easing::EaseIn),
            });
        assert_eq!(timeline.duration_ms(), 250);
    }

    #[test]
    fn an_empty_timeline_has_no_length() {
        assert_eq!(Timeline::new().duration_ms(), 0);
    }

    #[test]
    fn admitting_a_timeline_reports_each_motion() {
        let timeline = Timeline::new()
            .with(motion(Effect::Transition, "color", Easing::Linear))
            .with(motion(Effect::Transition, "gap", Easing::Linear));
        let verdicts = timeline.admit(&target());
        assert_eq!(verdicts.len(), 2);
        assert!(verdicts[0].1.is_accepted());
        assert!(!verdicts[1].1.is_accepted());
    }

    #[test]
    fn the_control_plays_pauses_and_seeks() {
        let mut control = Control::new(400);
        assert!(!control.seek(120));
        control.play();
        assert_eq!(control.position_ms, 120);
        assert!(control.render().starts_with("playing"));
        control.pause();
        assert!(control.render().starts_with("paused"));
    }

    #[test]
    fn seeking_past_the_end_clamps_and_says_so() {
        let mut control = Control::new(400);
        assert!(control.seek(900));
        assert_eq!(control.position_ms, 400);
        assert!(!control.seek(400));
    }

    #[test]
    fn the_capability_report_lists_what_it_supports() {
        let capability = Capability::new().with_transitions(true);
        let lines = capability.render();
        assert!(lines.iter().any(|line| line == "transitions: on"));
        assert!(lines.iter().any(|line| line == "animations: off"));
        assert!(lines.iter().any(|line| line == "properties: (none)"));
    }

    #[test]
    fn a_timing_names_its_parts() {
        let timing = Timing::new(200, 50, Easing::EaseIn);
        assert_eq!(timing.total_ms(), 250);
        assert_eq!(timing.render(), "200ms (+50ms delay, ease-in)");
    }
}
