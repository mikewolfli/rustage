// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Visual-regression evidence for the quality centre (roadmap task T21.06).
//!
//! The quality report used to hold a [`VisualCondition`] — the platform, the scale and the font
//! set — and compare those three fields with each other. That answers "were these two captures
//! taken under the same conditions", not "did the pixels change", and it had no way to say that no
//! capture existed at all. This module connects the quality centre to
//! [`crate::app::preview::visual_baseline`], which does the real comparison:
//!
//! * **A baseline and a candidate are shots**, not conditions: each carries its pixels and the
//!   conditions it was taken under.
//! * **Conditions are checked before pixels.** A different platform, DPI, resource version or font
//!   set is [`Verdict::NotComparable`] with the field named, never a pixel verdict.
//! * **Not collecting is a state of its own.** [`VisualEvidence::NotCollected`] says no capture
//!   exists; it is neither a pass nor a failure, and [`VisualEvidence::passed`] answers `None` for
//!   it. A candidate without a baseline is [`VisualEvidence::MissingBaseline`], which is also not a
//!   pass. Only [`VisualEvidence::Compared`] can pass or fail.
//! * **A resized capture is not a pixel verdict.** [`Verdict::Resized`] keeps the two sizes so a
//!   layout change is read as one.

#![allow(dead_code)]

use crate::app::preview::visual_baseline::{
    Baseline, Conditions, Report, Shot, Tolerance, Verdict, compare,
};
use crate::project::record::json::{Json, JsonObject};

/// The environment a capture was taken in, as the quality command records it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct VisualCondition {
    /// The platform or host name.
    pub platform: String,
    /// The display scale, as a percentage (recorded as the baseline's DPI value).
    pub scale: u32,
    /// A stable font-set identifier (a digest, not a font name that can drift).
    pub font_digest: String,
    /// The resource-bundle version the capture was rendered with; 0 is the default bundle.
    pub resources: u32,
}

impl VisualCondition {
    /// Whether two captures are comparable.
    ///
    /// This is the condition-only check the quality command has always had. A pixel comparison goes
    /// through [`VisualRun`] and [`crate::app::preview::visual_baseline::compare`], which also
    /// checks the resource version.
    pub fn compare(&self, other: &Self) -> Result<(), String> {
        if self.platform != other.platform {
            return Err(format!(
                "platform differs: {} vs {}",
                self.platform, other.platform
            ));
        }
        if self.scale != other.scale {
            return Err(format!("scale differs: {} vs {}", self.scale, other.scale));
        }
        if self.font_digest != other.font_digest {
            return Err("font set differs".to_owned());
        }
        if self.resources != other.resources {
            return Err(format!(
                "resource version differs: {} vs {}",
                self.resources, other.resources
            ));
        }
        Ok(())
    }

    /// The baseline conditions these facts describe.
    pub fn conditions(&self) -> Conditions {
        Conditions::new(&self.platform, self.scale)
            .with_resources(self.resources)
            .with_fonts(&self.font_digest)
    }
}

/// What the run knows about visual evidence.
#[derive(Clone, PartialEq, Debug, Default)]
pub enum VisualEvidence {
    /// No capture was taken, so nothing was compared.
    #[default]
    NotCollected,
    /// A candidate was captured and a baseline of that name exists, but no comparison was run.
    Captured {
        /// The candidate's name.
        name: String,
    },
    /// A candidate was captured and there is no baseline of that name.
    MissingBaseline {
        /// The candidate's name.
        name: String,
    },
    /// A real comparison happened.
    Compared(Report),
}

impl VisualEvidence {
    /// Whether anything was captured at all.
    pub fn is_collected(&self) -> bool {
        !matches!(self, Self::NotCollected)
    }

    /// Whether the evidence passes.
    ///
    /// `None` means "not evidence either way" — nothing was captured, a baseline is missing, or a
    /// comparison was not run. Treating any of those as a pass is exactly the failure this module
    /// exists to avoid.
    pub fn passed(&self) -> Option<bool> {
        match self {
            Self::Compared(report) => Some(report.verdict.passed()),
            Self::NotCollected | Self::Captured { .. } | Self::MissingBaseline { .. } => None,
        }
    }

    /// The evidence in one line per fact.
    pub fn render(&self) -> Vec<String> {
        match self {
            Self::NotCollected => vec!["visual: not collected".to_owned()],
            Self::Captured { name } => {
                vec![format!("visual: {name}: candidate captured, not compared")]
            }
            Self::MissingBaseline { name } => vec![format!(
                "visual: {name}: no baseline is recorded, so nothing was compared"
            )],
            Self::Compared(report) => {
                let mut lines = vec![format!(
                    "visual: {}: {}",
                    report.name,
                    report.verdict.name()
                )];
                lines.extend(report.render());
                lines
            }
        }
    }

    /// The evidence as JSON, with its state spelled out.
    pub fn json(&self) -> Json {
        let mut object = JsonObject::new();
        match self {
            Self::NotCollected => {
                object.insert("state", Json::string("not-collected"));
            }
            Self::Captured { name } => {
                object.insert("state", Json::string("captured"));
                object.insert("name", Json::string(name));
            }
            Self::MissingBaseline { name } => {
                object.insert("state", Json::string("missing-baseline"));
                object.insert("name", Json::string(name));
            }
            Self::Compared(report) => {
                object.insert("state", Json::string("compared"));
                object.insert("name", Json::string(&report.name));
                object.insert("verdict", Json::string(report.verdict.name()));
                object.insert("baseline", conditions_json(&report.baseline));
                object.insert("candidate", conditions_json(&report.candidate));
                if let Verdict::Within { comparison, .. } | Verdict::Differs { comparison, .. } =
                    &report.verdict
                {
                    object.insert("pixels", Json::string(comparison.render()));
                }
            }
        }
        Json::Object(object)
    }
}

/// One conditions record as JSON.
fn conditions_json(conditions: &Conditions) -> Json {
    let mut object = JsonObject::new();
    object.insert("platform", Json::string(&conditions.platform));
    object.insert("dpi", Json::number(u64::from(conditions.dpi)));
    object.insert("resources", Json::number(u64::from(conditions.resources)));
    object.insert("fonts", Json::string(&conditions.fonts));
    Json::Object(object)
}

/// One run's baselines, candidate and (when asked) comparison.
#[derive(Clone, PartialEq, Debug)]
pub struct VisualRun {
    baseline: Baseline,
    candidate: Option<Shot>,
    /// The last comparison, when one was run.
    comparison: Option<Report>,
    tolerance: Tolerance,
}

impl Default for VisualRun {
    fn default() -> Self {
        Self::new()
    }
}

impl VisualRun {
    /// A run with no baselines and an exact tolerance.
    pub fn new() -> Self {
        Self {
            baseline: Baseline::new(),
            candidate: None,
            comparison: None,
            tolerance: Tolerance::exact(),
        }
    }

    /// The tolerance in force.
    pub fn tolerance(&self) -> Tolerance {
        self.tolerance
    }

    /// Replaces the tolerance.
    pub fn set_tolerance(&mut self, tolerance: Tolerance) {
        self.tolerance = tolerance;
    }

    /// The baselines recorded so far.
    pub fn baselines(&self) -> &Baseline {
        &self.baseline
    }

    /// The candidate captured for this run.
    pub fn candidate(&self) -> Option<&Shot> {
        self.candidate.as_ref()
    }

    /// Records a baseline shot.
    ///
    /// A baseline recorded under different conditions is refused by
    /// [`Baseline::record`](crate::app::preview::visual_baseline::Baseline::record); the reference
    /// may only move when someone drops it first.
    pub fn record_baseline(&mut self, shot: Shot) -> Result<(), String> {
        self.baseline.record(shot)
    }

    /// Captures the candidate shot, clearing any earlier comparison.
    pub fn capture(&mut self, shot: Shot) {
        self.candidate = Some(shot);
        self.comparison = None;
    }

    /// What the run currently knows, without changing anything.
    pub fn evidence(&self) -> VisualEvidence {
        if let Some(report) = &self.comparison {
            return VisualEvidence::Compared(report.clone());
        }
        let Some(candidate) = &self.candidate else {
            return VisualEvidence::NotCollected;
        };
        if self.baseline.get(&candidate.name).is_some() {
            VisualEvidence::Captured {
                name: candidate.name.clone(),
            }
        } else {
            VisualEvidence::MissingBaseline {
                name: candidate.name.clone(),
            }
        }
    }

    /// Compares the candidate with its baseline and records the answer.
    ///
    /// A missing candidate leaves the evidence [`VisualEvidence::NotCollected`]; a missing baseline
    /// is [`VisualEvidence::MissingBaseline`]. Neither is turned into a pass.
    pub fn check(&mut self) -> VisualEvidence {
        let Some(candidate) = &self.candidate else {
            return VisualEvidence::NotCollected;
        };
        let Some(baseline) = self.baseline.get(&candidate.name) else {
            return VisualEvidence::MissingBaseline {
                name: candidate.name.clone(),
            };
        };
        let report = compare(baseline, candidate, self.tolerance);
        self.comparison = Some(report.clone());
        VisualEvidence::Compared(report)
    }

    /// The baselines and what the run knows, in one line per fact.
    pub fn render(&self) -> Vec<String> {
        let mut lines = self.baseline.render();
        if let Some(candidate) = &self.candidate {
            lines.push(format!("candidate: {}", candidate.render()));
        }
        lines.extend(self.evidence().render());
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{VisualCondition, VisualEvidence, VisualRun};
    use crate::app::preview::visual_baseline::{Image, Shot, Tolerance};

    fn condition() -> VisualCondition {
        VisualCondition {
            platform: "macos".to_owned(),
            scale: 200,
            font_digest: "fonts-a".to_owned(),
            resources: 3,
        }
    }

    fn shot(name: &str, digits: &str) -> Shot {
        Shot::new(
            name,
            condition().conditions(),
            Image::from_digits(2, 2, digits).expect("a 2x2 image"),
        )
    }

    #[test]
    fn a_run_with_no_capture_says_not_collected() {
        let run = VisualRun::new();
        assert_eq!(run.evidence(), VisualEvidence::NotCollected);
        assert_eq!(run.evidence().passed(), None);
        assert_eq!(
            run.render(),
            vec!["baselines: none", "visual: not collected"]
        );
    }

    #[test]
    fn a_candidate_without_a_baseline_is_not_a_pass() {
        let mut run = VisualRun::new();
        run.capture(shot("home", "0123"));
        assert_eq!(
            run.evidence(),
            VisualEvidence::MissingBaseline {
                name: "home".to_owned()
            }
        );
        assert_eq!(run.check().passed(), None);
        assert!(run.render().iter().any(|line| line.contains("no baseline")));
    }

    #[test]
    fn a_captured_candidate_with_a_baseline_is_not_compared_until_check_runs() {
        let mut run = VisualRun::new();
        run.record_baseline(shot("home", "0123"))
            .expect("a baseline");
        run.capture(shot("home", "0123"));
        assert_eq!(
            run.evidence(),
            VisualEvidence::Captured {
                name: "home".to_owned()
            }
        );
        assert_eq!(run.evidence().passed(), None);
    }

    #[test]
    fn identical_pixels_compare_as_same() {
        let mut run = VisualRun::new();
        run.record_baseline(shot("home", "0123"))
            .expect("a baseline");
        run.capture(shot("home", "0123"));
        let evidence = run.check();
        assert_eq!(evidence.passed(), Some(true));
        assert!(
            evidence.render()[0].contains("same"),
            "{:?}",
            evidence.render()
        );
        assert!(evidence.json().render().contains("\"state\": \"compared\""));
        assert!(evidence.json().render().contains("\"verdict\": \"same\""));
    }

    #[test]
    fn a_different_condition_is_not_comparable_rather_than_a_pixel_verdict() {
        let mut run = VisualRun::new();
        run.record_baseline(shot("home", "0123"))
            .expect("a baseline");
        let mut other = condition();
        other.resources = 4;
        run.capture(Shot::new(
            "home",
            other.conditions(),
            Image::from_digits(2, 2, "0123").expect("an image"),
        ));
        let evidence = run.check();
        assert_eq!(evidence.passed(), Some(false));
        assert!(
            evidence
                .render()
                .iter()
                .any(|line| line.contains("not comparable")),
            "{:?}",
            evidence.render()
        );
        assert!(evidence.json().render().contains("not comparable"));
    }

    #[test]
    fn a_tolerance_can_admit_a_small_difference() {
        let mut run = VisualRun::new();
        run.set_tolerance(Tolerance::percentage(50.0, 0));
        assert_eq!(run.tolerance().render(), "50.0% of pixels (channel step 0)");
        run.record_baseline(shot("home", "0123"))
            .expect("a baseline");
        // Three of the four pixels are unchanged, so 25% differ and the tolerance admits it.
        run.capture(shot("home", "0124"));
        let evidence = run.check();
        assert_eq!(evidence.passed(), Some(true));
        assert!(
            evidence.render().iter().any(|line| line.contains("within")),
            "{:?}",
            evidence.render()
        );
    }

    #[test]
    fn a_baseline_cannot_be_re_recorded_under_other_conditions() {
        let mut run = VisualRun::new();
        run.record_baseline(shot("home", "0123"))
            .expect("a baseline");
        let mut other = condition();
        other.platform = "linux".to_owned();
        let error = run
            .record_baseline(Shot::new(
                "home",
                other.conditions(),
                Image::from_digits(2, 2, "0123").expect("an image"),
            ))
            .expect_err("the reference moved without a drop");
        assert!(error.contains("other conditions"), "{error}");
    }
}
