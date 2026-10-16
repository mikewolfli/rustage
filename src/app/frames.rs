// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Checking the simulated frames against what the target actually supports: size, orientation, zoom
//! and safe area (task T10.05).
//!
//! The acceptance line is *"the difference between the simulation and the target's support is
//! explicit"*. The check is therefore a comparison of two things that are named separately, never an
//! assumption that they match:
//!
//! * **The simulation is the frames; the target's support is an input.** [`check`] compares a list of
//!   [`Frame`]s with a [`Support`] and answers the [`Difference`]s, so "what I am looking at" and
//!   "what the target can do" are two values, not one.
//! * **Every difference is named and classified.** A [`Difference`] says what differs and whether it
//!   [`blocks_generation`](Difference::blocks_generation), so a note a person can ignore is told apart
//!   from a reason the target would not build.
//! * **Orientation is stated, not inferred.** A square frame can be either, so the frame carries its
//!   orientation rather than having it guessed from the numbers.
//! * **Safe area is a promise the target may not keep.** A frame with insets on a target that does not
//!   honour them is a difference, because the preview would show a margin the target would not apply.

#![allow(dead_code)]

use crate::app::layout_plan::Size;

/// Which way a frame is held.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Orientation {
    /// Taller than wide.
    Portrait,
    /// Wider than tall.
    Landscape,
}

impl Orientation {
    /// Every orientation, in a stable order.
    pub const ALL: [Self; 2] = [Self::Portrait, Self::Landscape];

    /// The orientation's name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Portrait => "portrait",
            Self::Landscape => "landscape",
        }
    }

    /// The orientation a name picks.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|orientation| orientation.name() == name)
    }
}

/// The space a frame keeps clear around its content.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Insets {
    /// Top.
    pub top: f64,
    /// Right.
    pub right: f64,
    /// Bottom.
    pub bottom: f64,
    /// Left.
    pub left: f64,
}

impl Insets {
    /// Insets.
    pub const fn new(top: f64, right: f64, bottom: f64, left: f64) -> Self {
        Self {
            top,
            right,
            bottom,
            left,
        }
    }

    /// Whether nothing is inset.
    pub fn is_empty(&self) -> bool {
        self.top == 0.0 && self.right == 0.0 && self.bottom == 0.0 && self.left == 0.0
    }
}

/// One simulated device frame.
#[derive(Clone, PartialEq, Debug)]
pub struct Frame {
    /// A label for the report.
    pub label: String,
    /// The viewport size.
    pub size: Size,
    /// How it is held.
    pub orientation: Orientation,
    /// The device scale (zoom the target would render at).
    pub scale: f64,
    /// The safe area it keeps clear.
    pub insets: Insets,
}

impl Frame {
    /// A frame at scale 1 with no insets.
    pub fn new(label: impl Into<String>, size: Size, orientation: Orientation) -> Self {
        Self {
            label: label.into(),
            size,
            orientation,
            scale: 1.0,
            insets: Insets::default(),
        }
    }

    /// The same frame at a scale.
    pub fn at_scale(mut self, scale: f64) -> Self {
        self.scale = scale;
        self
    }

    /// The same frame with insets.
    pub fn with_insets(mut self, insets: Insets) -> Self {
        self.insets = insets;
        self
    }
}

/// What a target actually supports.
#[derive(Clone, PartialEq, Debug)]
pub struct Support {
    /// The viewport sizes the target accepts.
    pub sizes: Vec<Size>,
    /// The orientations it accepts.
    pub orientations: Vec<Orientation>,
    /// Whether it honours a safe area.
    pub honours_safe_area: bool,
    /// The largest device scale it renders at.
    pub max_scale: f64,
}

impl Support {
    /// A target with nothing but a default scale.
    pub fn new() -> Self {
        Self {
            sizes: Vec::new(),
            orientations: Vec::new(),
            honours_safe_area: true,
            max_scale: 1.0,
        }
    }

    /// The same support with one more accepted size.
    pub fn with_size(mut self, size: Size) -> Self {
        self.sizes.push(size);
        self
    }

    /// The same support with one more accepted orientation.
    pub fn with_orientation(mut self, orientation: Orientation) -> Self {
        self.orientations.push(orientation);
        self
    }

    /// The same support, saying whether it honours a safe area.
    pub fn honouring(mut self, honours: bool) -> Self {
        self.honours_safe_area = honours;
        self
    }

    /// The same support at a maximum scale.
    pub fn up_to_scale(mut self, max_scale: f64) -> Self {
        self.max_scale = max_scale;
        self
    }
}

impl Default for Support {
    fn default() -> Self {
        Self::new()
    }
}

/// One way the simulation differs from what the target supports.
#[derive(Clone, PartialEq, Debug)]
pub struct Difference {
    /// The frame's label.
    pub frame: String,
    /// What differs.
    pub kind: DifferenceKind,
    /// What to say about it.
    pub message: String,
}

/// The kinds of difference the check can find.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DifferenceKind {
    /// The frame's size is not one the target accepts.
    Size,
    /// The frame's orientation is not one the target accepts.
    Orientation,
    /// The frame has a safe area the target does not honour.
    SafeArea,
    /// The frame's scale is above what the target renders.
    Scale,
}

impl DifferenceKind {
    /// The kind's name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Size => "size",
            Self::Orientation => "orientation",
            Self::SafeArea => "safe-area",
            Self::Scale => "scale",
        }
    }

    /// Whether this kind stops the target from building the design as shown.
    pub const fn blocks_generation(self) -> bool {
        match self {
            Self::Size | Self::Scale => true,
            Self::Orientation | Self::SafeArea => false,
        }
    }
}

impl Difference {
    /// Whether this difference stops generation.
    pub const fn blocks_generation(&self) -> bool {
        self.kind.blocks_generation()
    }

    /// The difference in one line.
    pub fn render(&self) -> String {
        let marker = if self.blocks_generation() {
            "blocker"
        } else {
            "note"
        };
        format!(
            "{marker} [{}] {}: {}",
            self.kind.name(),
            self.frame,
            self.message
        )
    }
}

/// Every way the frames differ from what the target supports.
pub fn check(frames: &[Frame], support: &Support) -> Vec<Difference> {
    let mut differences = Vec::new();
    for frame in frames {
        if !support.sizes.is_empty() && !support.sizes.contains(&frame.size) {
            differences.push(Difference {
                frame: frame.label.clone(),
                kind: DifferenceKind::Size,
                message: format!(
                    "the target accepts no {}x{} viewport",
                    frame.size.width, frame.size.height
                ),
            });
        }
        if !support.orientations.is_empty() && !support.orientations.contains(&frame.orientation) {
            differences.push(Difference {
                frame: frame.label.clone(),
                kind: DifferenceKind::Orientation,
                message: format!("the target does not accept {}", frame.orientation.name()),
            });
        }
        if !frame.insets.is_empty() && !support.honours_safe_area {
            differences.push(Difference {
                frame: frame.label.clone(),
                kind: DifferenceKind::SafeArea,
                message: "the frame keeps a safe area the target does not honour".to_owned(),
            });
        }
        if frame.scale > support.max_scale {
            differences.push(Difference {
                frame: frame.label.clone(),
                kind: DifferenceKind::Scale,
                message: format!(
                    "the frame is at {} but the target renders up to {}",
                    frame.scale, support.max_scale
                ),
            });
        }
    }
    differences
}

#[cfg(test)]
mod tests {
    use super::{DifferenceKind, Frame, Insets, Orientation, Support, check};
    use crate::app::layout_plan::Size;

    fn phone() -> Frame {
        Frame::new("phone", Size::new(390.0, 844.0), Orientation::Portrait)
    }

    #[test]
    fn a_frame_the_target_supports_has_no_differences() {
        let support = Support::new()
            .with_size(Size::new(390.0, 844.0))
            .with_orientation(Orientation::Portrait);
        assert!(check(&[phone()], &support).is_empty());
    }

    #[test]
    fn an_unsupported_size_is_a_blocker() {
        let support = Support::new().with_size(Size::new(800.0, 600.0));
        let found = check(&[phone()], &support);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, DifferenceKind::Size);
        assert!(found[0].blocks_generation());
    }

    #[test]
    fn an_unsupported_orientation_is_a_note() {
        let support = Support::new()
            .with_size(Size::new(390.0, 844.0))
            .with_orientation(Orientation::Landscape);
        let found = check(&[phone()], &support);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, DifferenceKind::Orientation);
        assert!(!found[0].blocks_generation());
    }

    #[test]
    fn a_safe_area_the_target_ignores_is_a_note() {
        let support = Support::new()
            .with_size(Size::new(390.0, 844.0))
            .with_orientation(Orientation::Portrait)
            .honouring(false);
        let frame = phone().with_insets(Insets::new(44.0, 0.0, 34.0, 0.0));
        let found = check(&[frame], &support);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, DifferenceKind::SafeArea);
    }

    #[test]
    fn a_scale_above_the_targets_is_a_blocker() {
        let support = Support::new()
            .with_size(Size::new(390.0, 844.0))
            .with_orientation(Orientation::Portrait)
            .up_to_scale(2.0);
        let frame = phone().at_scale(3.0);
        let found = check(&[frame], &support);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, DifferenceKind::Scale);
        assert!(found[0].blocks_generation());
    }

    #[test]
    fn every_difference_is_reported_not_just_the_first() {
        let support = Support::new()
            .with_size(Size::new(800.0, 600.0))
            .with_orientation(Orientation::Landscape)
            .honouring(false)
            .up_to_scale(1.0);
        let frame = phone()
            .at_scale(2.0)
            .with_insets(Insets::new(10.0, 0.0, 0.0, 0.0));
        let found = check(&[frame], &support);
        assert_eq!(found.len(), 4, "size, orientation, safe area and scale");
        assert_eq!(
            found.iter().filter(|d| d.blocks_generation()).count(),
            2,
            "size and scale block; orientation and safe area are notes"
        );
    }

    #[test]
    fn an_empty_support_list_accepts_every_size_and_orientation() {
        let found = check(&[phone()], &Support::new());
        assert!(found.is_empty(), "an unstated support constrains nothing");
    }

    #[test]
    fn a_difference_names_its_frame_and_kind() {
        let support = Support::new().with_size(Size::new(1.0, 1.0));
        let found = check(&[phone()], &support);
        assert!(found[0].render().contains("phone"));
        assert!(found[0].render().contains("blocker"));
        assert!(found[0].render().contains("size"));
    }

    #[test]
    fn orientation_names_round_trip() {
        for orientation in Orientation::ALL {
            assert_eq!(
                Orientation::from_name(orientation.name()),
                Some(orientation)
            );
        }
        assert!(Orientation::from_name("square").is_none());
    }

    #[test]
    fn empty_insets_are_empty() {
        assert!(Insets::default().is_empty());
        assert!(!Insets::new(1.0, 0.0, 0.0, 0.0).is_empty());
    }
}
