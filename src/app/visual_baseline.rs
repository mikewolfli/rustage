// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Comparing a preview screenshot with its recorded baseline, and refusing to guess (task T14.06).
//!
//! The acceptance line is *"a difference states the platform, the DPI and the resource version, and
//! nothing is judged from pixels alone"*. The type makes that the only thing it can say:
//!
//! * **A shot carries the conditions it was taken under.** [`Shot`] is pixels *and* a [`Conditions`]
//!   record — platform, DPI, resource version and font set — so the two cannot be separated and the
//!   conditions travel with the picture into every answer.
//! * **Pixels are only compared when the conditions agree.** [`compare`] checks them first and
//!   answers [`Verdict::NotComparable`] naming the field that differs, so a picture from another
//!   platform, DPI, resource version or font set can never come back as a pixel verdict.
//! * **A pixel verdict carries its numbers and its tolerance.** Nothing answers "looks the same":
//!   the answer is how many pixels differ, out of how many, at which declared tolerance, and where.
//! * **A baseline is not silently re-recorded somewhere else.** [`Baseline::record`] refuses to
//!   replace a baseline taken under different conditions, because that would quietly move the
//!   reference everyone else compares against.

#![allow(dead_code)]

use std::collections::BTreeMap;

/// The platform, DPI, resource version and font set a shot was taken under.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Conditions {
    /// The platform, as `uname`-like machine and system.
    pub platform: String,
    /// The dots per inch the shot was rendered at.
    pub dpi: u32,
    /// The version of the resource bundle the shot was rendered with.
    pub resources: u32,
    /// The font set the shot was rendered with.
    pub fonts: String,
}

impl Conditions {
    /// Conditions on a platform at a DPI, with the default resource bundle and font set.
    pub fn new(platform: impl Into<String>, dpi: u32) -> Self {
        Self {
            platform: platform.into(),
            dpi,
            resources: 0,
            fonts: "default".to_owned(),
        }
    }

    /// The same conditions with another resource bundle version.
    pub fn with_resources(mut self, resources: u32) -> Self {
        self.resources = resources;
        self
    }

    /// The same conditions with another font set.
    pub fn with_fonts(mut self, fonts: impl Into<String>) -> Self {
        self.fonts = fonts.into();
        self
    }

    /// The first field a candidate differs in, said as a sentence.
    ///
    /// `self` is the baseline's record. A difference here means no pixel comparison may be made, so
    /// it always names the field rather than leaving the reader to compare two records by eye.
    pub fn mismatch(&self, candidate: &Self) -> Option<String> {
        if self.platform != candidate.platform {
            return Some(format!(
                "platform is {:?} and the baseline is {:?}",
                candidate.platform, self.platform
            ));
        }
        if self.dpi != candidate.dpi {
            return Some(format!(
                "dpi is {} and the baseline is {}",
                candidate.dpi, self.dpi
            ));
        }
        if self.resources != candidate.resources {
            return Some(format!(
                "resource version is {} and the baseline is {}",
                candidate.resources, self.resources
            ));
        }
        if self.fonts != candidate.fonts {
            return Some(format!(
                "font set is {:?} and the baseline is {:?}",
                candidate.fonts, self.fonts
            ));
        }
        None
    }

    /// The record as one phrase.
    pub fn render(&self) -> String {
        format!(
            "platform {}, dpi {}, resources {}, fonts {}",
            self.platform, self.dpi, self.resources, self.fonts
        )
    }
}

/// A rectangular region, in pixels.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Bounds {
    /// The left edge.
    pub left: u32,
    /// The top edge.
    pub top: u32,
    /// The right edge.
    pub right: u32,
    /// The bottom edge.
    pub bottom: u32,
}

impl Bounds {
    /// Grows the region to include a pixel.
    fn including(self, x: u32, y: u32) -> Self {
        Self {
            left: self.left.min(x),
            top: self.top.min(y),
            right: self.right.max(x),
            bottom: self.bottom.max(y),
        }
    }

    /// The region as `(left,top)-(right,bottom)`.
    pub fn render(&self) -> String {
        format!(
            "({},{})-({},{})",
            self.left, self.top, self.right, self.bottom
        )
    }
}

/// How far a pixel may move and still count as the same.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Tolerance {
    /// The largest fraction of pixels that may differ, as `0.0`–`1.0`.
    pub fraction: f64,
    /// The largest per-pixel channel step that is not counted as a difference.
    pub channel_delta: u8,
}

impl Tolerance {
    /// Nothing may differ.
    pub fn exact() -> Self {
        Self {
            fraction: 0.0,
            channel_delta: 0,
        }
    }

    /// A percentage of pixels, with a channel step.
    pub fn percentage(percent: f64, channel_delta: u8) -> Self {
        Self {
            fraction: percent / 100.0,
            channel_delta,
        }
    }

    /// The tolerance as one phrase.
    pub fn render(&self) -> String {
        format!(
            "{:.1}% of pixels (channel step {})",
            self.fraction * 100.0,
            self.channel_delta
        )
    }
}

/// A greyscale image: one nibble per pixel, row by row.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Image {
    /// The width in pixels.
    pub width: u32,
    /// The height in pixels.
    pub height: u32,
    /// The pixels, row by row.
    pub pixels: Vec<u8>,
}

impl Image {
    /// An image from exactly `width * height` hexadecimal digits.
    pub fn from_digits(width: u32, height: u32, digits: &str) -> Result<Self, String> {
        let expected = (width as usize) * (height as usize);
        let mut pixels = Vec::with_capacity(expected);
        for character in digits.chars() {
            match character.to_digit(16) {
                Some(value) => pixels.push(value as u8),
                None => return Err(format!("{digits:?} has {character:?}, which is not hex")),
            }
        }
        if pixels.len() != expected {
            return Err(format!(
                "{digits:?} is {} pixel(s), and a {width}x{height} image has {expected}",
                pixels.len()
            ));
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    /// The size as `<width>x<height>`.
    pub fn size(&self) -> String {
        format!("{}x{}", self.width, self.height)
    }

    /// Compares two images of the same size, pixel by pixel.
    ///
    /// A pixel counts as differing only when its channel step is above the tolerance's, so the
    /// answer is "how many pixels moved, and where" rather than "the bytes are not equal".
    pub fn compare(&self, other: &Self, tolerance: Tolerance) -> Result<Comparison, String> {
        if self.width != other.width || self.height != other.height {
            return Err(format!(
                "{} cannot be compared with {}",
                self.size(),
                other.size()
            ));
        }
        let mut differing = 0;
        let mut worst = 0;
        let mut bounds: Option<Bounds> = None;
        for (index, (left, right)) in self.pixels.iter().zip(&other.pixels).enumerate() {
            let step = left.abs_diff(*right);
            if step > tolerance.channel_delta {
                differing += 1;
                worst = worst.max(step);
                let x = index as u32 % self.width;
                let y = index as u32 / self.width;
                bounds = Some(match bounds {
                    None => Bounds {
                        left: x,
                        top: y,
                        right: x,
                        bottom: y,
                    },
                    Some(region) => region.including(x, y),
                });
            }
        }
        Ok(Comparison {
            differing,
            total: self.pixels.len() as u32,
            worst: worst as u32,
            bounds,
        })
    }
}

/// What a pixel-by-pixel comparison measured.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Comparison {
    /// How many pixels differ.
    pub differing: u32,
    /// How many pixels there are.
    pub total: u32,
    /// The largest channel step seen.
    pub worst: u32,
    /// The region containing every differing pixel.
    pub bounds: Option<Bounds>,
}

impl Comparison {
    /// The fraction of pixels that differ.
    pub fn fraction(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            f64::from(self.differing) / f64::from(self.total)
        }
    }

    /// What was measured, as one phrase.
    pub fn render(&self) -> String {
        let region = match self.bounds {
            Some(bounds) => format!(", worst region {}", bounds.render()),
            None => String::new(),
        };
        format!(
            "{}/{} pixel(s) differ ({:.1}%), largest step {}{region}",
            self.differing,
            self.total,
            self.fraction() * 100.0,
            self.worst
        )
    }
}

/// A screenshot: its pixels and the conditions it was taken under.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Shot {
    /// What it is called.
    pub name: String,
    /// What it was taken under.
    pub conditions: Conditions,
    /// What it shows.
    pub image: Image,
}

impl Shot {
    /// A shot of an image under conditions.
    pub fn new(name: impl Into<String>, conditions: Conditions, image: Image) -> Self {
        Self {
            name: name.into(),
            conditions,
            image,
        }
    }

    /// The shot as one line.
    pub fn render(&self) -> String {
        format!(
            "{}: {} ({})",
            self.name,
            self.image.size(),
            self.conditions.render()
        )
    }
}

/// What comparing one shot with its baseline found.
#[derive(Clone, PartialEq, Debug)]
pub enum Verdict {
    /// The conditions agree and nothing differed.
    Same,
    /// The conditions agree, something differed, and it is within the declared tolerance.
    Within {
        /// What the comparison measured.
        comparison: Comparison,
        /// The declared tolerance.
        tolerance: Tolerance,
    },
    /// The conditions agree and it differed above the declared tolerance.
    Differs {
        /// What the comparison measured.
        comparison: Comparison,
        /// The declared tolerance.
        tolerance: Tolerance,
    },
    /// The conditions differ, so no pixel comparison was made.
    NotComparable {
        /// Which condition differs, and how.
        reason: String,
    },
    /// The sizes differ, so no pixel comparison is possible.
    Resized {
        /// The baseline's size.
        baseline: String,
        /// The candidate's size.
        candidate: String,
    },
}

impl Verdict {
    /// The verdict's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Same => "same",
            Self::Within { .. } => "within tolerance",
            Self::Differs { .. } => "differs",
            Self::NotComparable { .. } => "not comparable",
            Self::Resized { .. } => "resized",
        }
    }

    /// Whether the candidate passed: identical or within the declared tolerance.
    ///
    /// "Not comparable" is not a pass: nothing was checked, and calling that a success is exactly
    /// the kind of silent pass the acceptance line forbids.
    pub fn passed(&self) -> bool {
        matches!(self, Self::Same | Self::Within { .. })
    }
}

/// One comparison's answer, with the conditions of both sides.
#[derive(Clone, PartialEq, Debug)]
pub struct Report {
    /// The shot's name.
    pub name: String,
    /// The baseline's conditions.
    pub baseline: Conditions,
    /// The candidate's conditions.
    pub candidate: Conditions,
    /// What the comparison found.
    pub verdict: Verdict,
}

impl Report {
    /// The answer as lines: the verdict, then both records.
    pub fn render(&self) -> Vec<String> {
        let detail = match &self.verdict {
            Verdict::Same => "same".to_owned(),
            Verdict::Within {
                comparison,
                tolerance,
            } => format!(
                "within tolerance: {} at tolerance {}",
                comparison.render(),
                tolerance.render()
            ),
            Verdict::Differs {
                comparison,
                tolerance,
            } => format!(
                "differs: {} above tolerance {}",
                comparison.render(),
                tolerance.render()
            ),
            Verdict::NotComparable { reason } => format!("not comparable: {reason}"),
            Verdict::Resized {
                baseline,
                candidate,
            } => format!("resized: the candidate is {candidate} and the baseline is {baseline}"),
        };
        vec![
            format!("{}: {detail}", self.name),
            format!("  baseline:  {}", self.baseline.render()),
            format!("  candidate: {}", self.candidate.render()),
        ]
    }
}

/// Compares a candidate shot with its recorded baseline.
///
/// The conditions are checked before any pixel is: a difference there is named and the answer says
/// no comparison was made, so a shot from another platform, DPI, resource version or font set can
/// never come back as a pixel verdict.
pub fn compare(baseline: &Shot, candidate: &Shot, tolerance: Tolerance) -> Report {
    let verdict = if let Some(reason) = baseline.conditions.mismatch(&candidate.conditions) {
        Verdict::NotComparable { reason }
    } else if baseline.image.width != candidate.image.width
        || baseline.image.height != candidate.image.height
    {
        Verdict::Resized {
            baseline: baseline.image.size(),
            candidate: candidate.image.size(),
        }
    } else {
        let comparison = baseline
            .image
            .compare(&candidate.image, tolerance)
            .expect("the sizes were just checked");
        if comparison.differing == 0 {
            Verdict::Same
        } else if comparison.fraction() <= tolerance.fraction {
            Verdict::Within {
                comparison,
                tolerance,
            }
        } else {
            Verdict::Differs {
                comparison,
                tolerance,
            }
        }
    };
    Report {
        name: candidate.name.clone(),
        baseline: baseline.conditions.clone(),
        candidate: candidate.conditions.clone(),
        verdict,
    }
}

/// The recorded baselines, by name.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Baseline {
    shots: BTreeMap<String, Shot>,
}

impl Baseline {
    /// No baselines yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a baseline.
    ///
    /// Refused when a baseline of that name was recorded under different conditions: silently
    /// replacing it would move the reference for every later comparison, on a machine nobody
    /// agreed on. Drop it first when the reference really moved.
    pub fn record(&mut self, shot: Shot) -> Result<(), String> {
        if let Some(existing) = self.shots.get(&shot.name)
            && let Some(difference) = existing.conditions.mismatch(&shot.conditions)
        {
            return Err(format!(
                "baseline {:?} was recorded under other conditions ({difference}); \
                 drop it first if the reference really moved",
                shot.name
            ));
        }
        self.shots.insert(shot.name.clone(), shot);
        Ok(())
    }

    /// Removes a baseline.
    pub fn drop_shot(&mut self, name: &str) -> Result<(), String> {
        self.shots
            .remove(name)
            .map(|_| ())
            .ok_or_else(|| format!("no baseline named {name:?}"))
    }

    /// The baseline of a name.
    pub fn get(&self, name: &str) -> Option<&Shot> {
        self.shots.get(name)
    }

    /// How many baselines are recorded.
    pub fn len(&self) -> usize {
        self.shots.len()
    }

    /// Whether nothing is recorded.
    pub fn is_empty(&self) -> bool {
        self.shots.is_empty()
    }

    /// The baselines as one line each.
    pub fn render(&self) -> Vec<String> {
        if self.shots.is_empty() {
            return vec!["baselines: none".to_owned()];
        }
        let mut lines = vec![format!("baselines: {}", self.shots.len())];
        lines.extend(
            self.shots
                .values()
                .map(|shot| format!("  {}", shot.render())),
        );
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{Baseline, Conditions, Image, Shot, Tolerance, Verdict, compare};

    fn linux() -> Conditions {
        Conditions::new("linux/x86_64", 96).with_resources(12)
    }

    fn shot(name: &str, conditions: Conditions, digits: &str) -> Shot {
        Shot::new(
            name,
            conditions,
            Image::from_digits(3, 2, digits).expect("an image"),
        )
    }

    #[test]
    fn the_same_shot_matches_its_baseline() {
        let baseline = shot("home", linux(), "012345");
        let candidate = shot("home", linux(), "012345");
        let report = compare(&baseline, &candidate, Tolerance::exact());
        assert_eq!(report.verdict, Verdict::Same);
        assert!(report.verdict.passed());
    }

    #[test]
    fn the_report_names_the_conditions_of_both_sides() {
        let baseline = shot("home", linux().with_fonts("noto"), "012345");
        let candidate = shot("home", linux().with_fonts("noto"), "012345");
        let lines = compare(&baseline, &candidate, Tolerance::exact()).render();
        assert_eq!(lines[0], "home: same");
        assert_eq!(
            lines[1],
            "  baseline:  platform linux/x86_64, dpi 96, resources 12, fonts noto"
        );
        assert_eq!(
            lines[2],
            "  candidate: platform linux/x86_64, dpi 96, resources 12, fonts noto"
        );
    }

    #[test]
    fn a_different_platform_is_not_comparable_and_names_the_field() {
        let baseline = shot("home", linux(), "012345");
        let candidate = shot(
            "home",
            Conditions::new("darwin/arm64", 96).with_resources(12),
            "987654",
        );
        let report = compare(&baseline, &candidate, Tolerance::exact());
        assert!(!report.verdict.passed());
        assert_eq!(report.verdict.name(), "not comparable");
        match report.verdict {
            Verdict::NotComparable { reason } => {
                assert_eq!(
                    reason,
                    "platform is \"darwin/arm64\" and the baseline is \"linux/x86_64\""
                );
            }
            other => panic!("expected not comparable, got {other:?}"),
        }
    }

    #[test]
    fn a_different_dpi_is_not_comparable() {
        let baseline = shot("home", linux(), "012345");
        let candidate = shot(
            "home",
            Conditions::new("linux/x86_64", 192).with_resources(12),
            "012345",
        );
        match compare(&baseline, &candidate, Tolerance::exact()).verdict {
            Verdict::NotComparable { reason } => {
                assert_eq!(reason, "dpi is 192 and the baseline is 96")
            }
            other => panic!("expected not comparable, got {other:?}"),
        }
    }

    #[test]
    fn a_different_resource_version_is_not_comparable() {
        let baseline = shot("home", linux(), "012345");
        let candidate = shot(
            "home",
            Conditions::new("linux/x86_64", 96).with_resources(13),
            "012345",
        );
        match compare(&baseline, &candidate, Tolerance::exact()).verdict {
            Verdict::NotComparable { reason } => {
                assert_eq!(reason, "resource version is 13 and the baseline is 12");
            }
            other => panic!("expected not comparable, got {other:?}"),
        }
    }

    #[test]
    fn a_different_font_set_is_not_comparable() {
        let baseline = shot("home", linux().with_fonts("noto"), "012345");
        let candidate = shot("home", linux().with_fonts("inter"), "012345");
        match compare(&baseline, &candidate, Tolerance::exact()).verdict {
            Verdict::NotComparable { reason } => {
                assert_eq!(reason, "font set is \"inter\" and the baseline is \"noto\"");
            }
            other => panic!("expected not comparable, got {other:?}"),
        }
    }

    #[test]
    fn a_different_size_is_reported_as_resized_not_as_a_pixel_verdict() {
        let baseline = shot("home", linux(), "012345");
        let candidate = Shot::new(
            "home",
            linux(),
            Image::from_digits(2, 2, "0123").expect("an image"),
        );
        let report = compare(&baseline, &candidate, Tolerance::exact());
        assert_eq!(report.verdict.name(), "resized");
        assert_eq!(
            report.render()[0],
            "home: resized: the candidate is 2x2 and the baseline is 3x2"
        );
        assert!(!report.verdict.passed());
    }

    #[test]
    fn a_difference_names_the_count_the_tolerance_and_the_region() {
        let baseline = shot("home", linux(), "000000");
        let candidate = shot("home", linux(), "100001");
        let report = compare(&baseline, &candidate, Tolerance::exact());
        match report.verdict {
            Verdict::Differs {
                comparison,
                tolerance,
            } => {
                assert_eq!(comparison.differing, 2);
                assert_eq!(comparison.total, 6);
                assert_eq!(comparison.worst, 1);
                assert_eq!(comparison.bounds.expect("a region").render(), "(0,0)-(2,1)");
                assert_eq!(tolerance.render(), "0.0% of pixels (channel step 0)");
            }
            other => panic!("expected a difference, got {other:?}"),
        }
        assert!(report.render()[0].starts_with("home: differs: 2/6 pixel(s) differ"));
    }

    #[test]
    fn a_difference_within_the_declared_tolerance_passes() {
        let baseline = shot("home", linux(), "000000");
        let candidate = shot("home", linux(), "100000");
        let report = compare(&baseline, &candidate, Tolerance::percentage(50.0, 0));
        assert!(report.verdict.passed());
        assert_eq!(report.verdict.name(), "within tolerance");
    }

    #[test]
    fn a_channel_step_below_the_tolerance_is_not_a_difference() {
        let baseline = shot("home", linux(), "000000");
        let candidate = shot("home", linux(), "111111");
        let report = compare(&baseline, &candidate, Tolerance::percentage(0.0, 1));
        assert_eq!(report.verdict, Verdict::Same);
    }

    #[test]
    fn an_image_needs_exactly_width_times_height_digits() {
        assert!(Image::from_digits(3, 2, "01234").is_err());
        assert!(Image::from_digits(3, 2, "0123456").is_err());
        assert!(Image::from_digits(3, 2, "012345").is_ok());
    }

    #[test]
    fn an_image_needs_hexadecimal_digits() {
        let refused = Image::from_digits(2, 2, "01z3");
        assert!(refused.is_err());
        assert!(refused.unwrap_err().contains("not hex"));
    }

    #[test]
    fn a_baseline_is_not_silently_re_recorded_under_other_conditions() {
        let mut baseline = Baseline::new();
        baseline
            .record(shot("home", linux(), "012345"))
            .expect("a record");
        let refused = baseline.record(shot("home", Conditions::new("darwin/arm64", 96), "012345"));
        assert!(refused.is_err());
        assert!(refused.unwrap_err().contains("other conditions"));
        assert_eq!(baseline.len(), 1);
        assert_eq!(
            baseline
                .get("home")
                .expect("a baseline")
                .conditions
                .platform,
            "linux/x86_64"
        );
    }

    #[test]
    fn a_baseline_can_be_dropped_and_then_re_recorded() {
        let mut baseline = Baseline::new();
        baseline
            .record(shot("home", linux(), "012345"))
            .expect("a record");
        baseline.drop_shot("home").expect("a drop");
        assert!(baseline.drop_shot("home").is_err());
        baseline
            .record(shot("home", Conditions::new("darwin/arm64", 96), "012345"))
            .expect("a re-record");
        assert_eq!(
            baseline
                .get("home")
                .expect("a baseline")
                .conditions
                .platform,
            "darwin/arm64"
        );
    }

    #[test]
    fn a_baseline_records_the_same_name_twice_when_the_conditions_agree() {
        let mut baseline = Baseline::new();
        baseline
            .record(shot("home", linux(), "012345"))
            .expect("a record");
        baseline
            .record(shot("home", linux(), "987654"))
            .expect("a re-record");
        assert_eq!(baseline.len(), 1);
        assert_eq!(baseline.get("home").expect("a baseline").image.pixels[0], 9);
    }

    #[test]
    fn the_baseline_renders_its_shots() {
        let mut baseline = Baseline::new();
        assert_eq!(baseline.render(), vec!["baselines: none".to_owned()]);
        assert!(baseline.is_empty());
        baseline
            .record(shot("home", linux(), "012345"))
            .expect("a record");
        assert_eq!(
            baseline.render(),
            vec![
                "baselines: 1".to_owned(),
                "  home: 3x2 (platform linux/x86_64, dpi 96, resources 12, fonts default)"
                    .to_owned(),
            ]
        );
    }

    #[test]
    fn the_tolerance_parses_a_percentage() {
        let tolerance = Tolerance::percentage(25.0, 2);
        assert_eq!(tolerance.fraction, 0.25);
        assert_eq!(tolerance.render(), "25.0% of pixels (channel step 2)");
        assert_eq!(Tolerance::exact().channel_delta, 0);
    }
}
