// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Tablet and mobile targets: the platform SDK, sizes, input, resources and the evidence a
//! simulator and a device each produce (task T18.04).
//!
//! The acceptance line is *"simulated and device results are recorded separately, and all-device
//! support is not claimed automatically"*. Three properties keep that true:
//!
//! * **A simulator is not a device.** [`Where`] has two variants, and [`Support::record`] refuses a
//!   simulator claiming [`Proof::Device`]. The evidence lists can be read apart
//!   ([`Support::simulated`], [`Support::devices`]), so a report cannot average them.
//! * **No type here can claim every device.** [`Support::claims_all_devices`] answers `false` by
//!   construction — there is no "all" field to set — and the report lists exactly the models that
//!   were exercised.
//! * **The platform SDK is a real prerequisite.** [`Plan::check`] reports a missing or unknown
//!   platform as [`Finding::SdkMissing`]/[`Finding::UnsupportedSdk`]; a size or touch-target
//!   problem is its own finding, because "the SDK is missing" and "the button is too small" send a
//!   person to different places.
//!
//! The size and capacity rows come from the profile and from
//! [`crate::app::component::targets::capacity`], so this module does not keep a second resource
//! table (principle 59).

#![allow(dead_code)]

use crate::app::component::targets::{ResourceKind, capacity};
use crate::app::preview::viewport_matrix::Orientation;

use super::matrix::Proof;

/// The mobile profiles this module covers.
pub const PROFILES: [&str; 2] = ["tablet", "mobile"];

/// One mobile target's size, orientation and touch-target minimum.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Plan {
    /// `tablet` or `mobile`.
    pub profile: String,
    /// The client size in logical pixels.
    pub size: (u32, u32),
    /// Which way round it is.
    pub orientation: Orientation,
    /// The smallest touch target the platform's guidance allows.
    pub minimum_target: u32,
}

impl Plan {
    /// The plan for a profile, or `None` when it is not a mobile one.
    pub fn of(profile: &str) -> Option<Self> {
        match profile {
            // Tablet guidance (Material) asks for 48 logical pixels; the Apple human interface
            // guidance asks for 44 on phones. They are stated per profile rather than averaged.
            "tablet" => Some(Self {
                profile: profile.to_owned(),
                size: (1024, 768),
                orientation: Orientation::Landscape,
                minimum_target: 48,
            }),
            "mobile" => Some(Self {
                profile: profile.to_owned(),
                size: (390, 844),
                orientation: Orientation::Portrait,
                minimum_target: 44,
            }),
            _ => None,
        }
    }

    /// The same plan at another size.
    pub fn with_size(mut self, size: (u32, u32)) -> Self {
        self.size = size;
        self
    }

    /// Checks the inputs, the resources and the platform SDK for this plan.
    pub fn check(&self, inputs: &[Input], assets: &[Asset], sdk: Option<&Sdk>) -> Vec<Finding> {
        let mut findings = Vec::new();
        for (index, input) in inputs.iter().enumerate() {
            if input.kind == InputKind::Pointer {
                findings.push(Finding::PointerOnTouchProfile {
                    input: index,
                    profile: self.profile.clone(),
                });
            }
            if input.kind == InputKind::Touch
                && (input.size.0 < self.minimum_target || input.size.1 < self.minimum_target)
            {
                findings.push(Finding::TargetTooSmall {
                    input: index,
                    size: input.size,
                    minimum: self.minimum_target,
                });
            }
        }
        for asset in assets {
            if let Some(bound) = capacity(asset.kind, &self.profile)
                && asset.bytes > bound
            {
                findings.push(Finding::OverCapacity {
                    asset: asset.name.clone(),
                    bytes: asset.bytes,
                    capacity: bound,
                });
            }
        }
        match sdk {
            None => findings.push(Finding::SdkMissing {
                platform: "ios or android".to_owned(),
                remedy: "name the platform SDK the package is built with".to_owned(),
            }),
            Some(sdk) if !matches!(sdk.platform.as_str(), "ios" | "android") => {
                findings.push(Finding::UnsupportedSdk {
                    platform: sdk.platform.clone(),
                });
            }
            Some(sdk) if !sdk.installed => findings.push(Finding::SdkMissing {
                platform: sdk.platform.clone(),
                remedy: format!(
                    "install the {} SDK and put it on the path this build probes",
                    sdk.platform
                ),
            }),
            Some(_) => {}
        }
        findings
    }

    /// The plan in one line.
    pub fn render(&self) -> String {
        format!(
            "{}: {}x{} {} touch-target<{}",
            self.profile,
            self.size.0,
            self.size.1,
            self.orientation.name(),
            self.minimum_target
        )
    }
}

/// What an input is made for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InputKind {
    /// A finger.
    Touch,
    /// A mouse or trackpad.
    Pointer,
}

impl InputKind {
    /// The kind's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Touch => "touch",
            Self::Pointer => "pointer",
        }
    }

    /// Reads an input kind.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "touch" => Some(Self::Touch),
            "pointer" => Some(Self::Pointer),
            _ => None,
        }
    }
}

/// One interactive target on the screen.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Input {
    /// How it is operated.
    pub kind: InputKind,
    /// Its size in logical pixels.
    pub size: (u32, u32),
}

impl Input {
    /// An input of a kind and a size.
    pub fn new(kind: InputKind, size: (u32, u32)) -> Self {
        Self { kind, size }
    }
}

/// One resource the target carries.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Asset {
    /// The resource's name.
    pub name: String,
    /// Its kind, which decides the capacity row.
    pub kind: ResourceKind,
    /// Its size in bytes.
    pub bytes: u64,
}

impl Asset {
    /// An asset of a kind and a size.
    pub fn new(name: impl Into<String>, kind: ResourceKind, bytes: u64) -> Self {
        Self {
            name: name.into(),
            kind,
            bytes,
        }
    }
}

/// The platform SDK a mobile package is built with.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Sdk {
    /// `ios` or `android`.
    pub platform: String,
    /// Whether this machine has it.
    pub installed: bool,
    /// The version the probe found, when it names one.
    pub version: Option<String>,
}

impl Sdk {
    /// An SDK with its availability.
    pub fn new(platform: impl Into<String>, installed: bool) -> Self {
        Self {
            platform: platform.into(),
            installed,
            version: None,
        }
    }

    /// The same SDK with a version.
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }
}

/// Why a mobile target cannot be built as asked.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Finding {
    /// A touch target is smaller than the platform's guidance.
    TargetTooSmall {
        /// Which input, by document order.
        input: usize,
        /// The size it was given.
        size: (u32, u32),
        /// The minimum.
        minimum: u32,
    },
    /// A pointer-only input on a touch-first profile.
    PointerOnTouchProfile {
        /// Which input.
        input: usize,
        /// The profile.
        profile: String,
    },
    /// A resource is larger than the profile's capacity.
    OverCapacity {
        /// The asset.
        asset: String,
        /// Its size.
        bytes: u64,
        /// The capacity.
        capacity: u64,
    },
    /// The platform SDK is not available on this machine.
    SdkMissing {
        /// The platform.
        platform: String,
        /// What to do.
        remedy: String,
    },
    /// The platform names a system this build has no package path for.
    UnsupportedSdk {
        /// The platform it named.
        platform: String,
    },
}

impl Finding {
    /// The finding in one line.
    pub fn render(&self) -> String {
        match self {
            Self::TargetTooSmall {
                input,
                size,
                minimum,
            } => format!(
                "input {input}: {}x{} is below the {minimum} pixel touch minimum",
                size.0, size.1
            ),
            Self::PointerOnTouchProfile { input, profile } => {
                format!("input {input}: a pointer target does not belong on {profile}")
            }
            Self::OverCapacity {
                asset,
                bytes,
                capacity,
            } => format!("{asset}: {bytes} bytes over the {capacity} byte capacity"),
            Self::SdkMissing { platform, remedy } => {
                format!("missing: {platform} SDK (remedy: {remedy})")
            }
            Self::UnsupportedSdk { platform } => format!(
                "unsupported: {platform:?} is not a mobile platform this build packages for"
            ),
        }
    }
}

/// Where evidence was produced.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Where {
    /// On a simulator or emulator.
    Simulator(String),
    /// On a physical device.
    Device(String),
}

impl Where {
    /// The place's name.
    pub fn name(&self) -> &str {
        match self {
            Self::Simulator(name) | Self::Device(name) => name,
        }
    }

    /// Whether this is a physical device.
    pub fn is_device(&self) -> bool {
        matches!(self, Self::Device(_))
    }
}

/// One observation of a mobile target.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Evidence {
    /// The profile it was observed on.
    pub profile: String,
    /// Where.
    pub place: Where,
    /// How far it got.
    pub proof: Proof,
    /// What was observed.
    pub note: String,
}

impl Evidence {
    /// An observation.
    pub fn new(
        profile: impl Into<String>,
        place: Where,
        proof: Proof,
        note: impl Into<String>,
    ) -> Self {
        Self {
            profile: profile.into(),
            place,
            proof,
            note: note.into(),
        }
    }
}

/// The evidence collected for the mobile profiles.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Support {
    evidences: Vec<Evidence>,
}

impl Support {
    /// No evidence yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records one observation, refusing one that a simulator cannot produce.
    pub fn record(&mut self, evidence: Evidence) -> Result<(), String> {
        if !PROFILES.contains(&evidence.profile.as_str()) {
            return Err(format!(
                "{} is not a mobile profile (use {})",
                evidence.profile,
                PROFILES.join(" or ")
            ));
        }
        if !evidence.place.is_device() && evidence.proof == Proof::Device {
            return Err(format!(
                "{} is a simulator; it cannot produce device evidence",
                evidence.place.name()
            ));
        }
        self.evidences.push(evidence);
        Ok(())
    }

    /// Evidence from simulators, in the order it was recorded.
    pub fn simulated(&self) -> Vec<&Evidence> {
        self.evidences
            .iter()
            .filter(|evidence| !evidence.place.is_device())
            .collect()
    }

    /// Evidence from physical devices, in the order it was recorded.
    pub fn devices(&self) -> Vec<&Evidence> {
        self.evidences
            .iter()
            .filter(|evidence| evidence.place.is_device())
            .collect()
    }

    /// The strongest proof each physical device reached.
    pub fn device_levels(&self) -> Vec<(&str, Proof)> {
        self.devices()
            .into_iter()
            .map(|evidence| (evidence.place.name(), evidence.proof))
            .collect()
    }

    /// Whether this support claims every device.
    ///
    /// It never does, by construction: the type records the models that were exercised and has no
    /// field for a broader claim.
    pub fn claims_all_devices(&self) -> bool {
        false
    }

    /// Every observation in one line.
    pub fn render(&self) -> Vec<String> {
        self.evidences
            .iter()
            .map(|evidence| {
                let place = match &evidence.place {
                    Where::Simulator(name) => format!("simulator {name}"),
                    Where::Device(name) => format!("device {name}"),
                };
                format!(
                    "{} on {place}: {}{}",
                    evidence.profile,
                    evidence.proof.name(),
                    if evidence.note.is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", evidence.note)
                    }
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{Asset, Finding, Input, InputKind, PROFILES, Plan, Sdk, Support, Where};
    use crate::app::component::targets::{ResourceKind, capacity};
    use crate::app::targets::matrix::Proof;

    fn complete_sdk() -> Sdk {
        Sdk::new("android", true)
    }

    #[test]
    fn only_tablet_and_mobile_have_a_plan() {
        for profile in PROFILES {
            assert!(Plan::of(profile).is_some(), "{profile}");
        }
        assert!(Plan::of("desktop").is_none());
        assert!(Plan::of("mini").is_none());
    }

    #[test]
    fn each_profile_states_its_touch_minimum() {
        assert_eq!(Plan::of("tablet").expect("a tablet").minimum_target, 48);
        assert_eq!(Plan::of("mobile").expect("a mobile").minimum_target, 44);
        assert!(
            Plan::of("tablet")
                .expect("a tablet")
                .render()
                .contains("1024x768")
        );
    }

    #[test]
    fn a_small_touch_target_is_a_finding() {
        let plan = Plan::of("mobile").expect("a mobile");
        let findings = plan.check(
            &[Input::new(InputKind::Touch, (40, 40))],
            &[],
            Some(&complete_sdk()),
        );
        assert!(
            findings
                .iter()
                .any(|finding| matches!(finding, Finding::TargetTooSmall { minimum: 44, .. }))
        );
    }

    #[test]
    fn a_pointer_input_does_not_belong_on_a_touch_profile() {
        let plan = Plan::of("tablet").expect("a tablet");
        let findings = plan.check(
            &[Input::new(InputKind::Pointer, (100, 48))],
            &[],
            Some(&complete_sdk()),
        );
        assert!(
            findings
                .iter()
                .any(|finding| matches!(finding, Finding::PointerOnTouchProfile { .. }))
        );
    }

    #[test]
    fn a_missing_sdk_is_distinct_from_a_size_problem() {
        let plan = Plan::of("tablet").expect("a tablet");
        let findings = plan.check(&[], &[], None);
        assert_eq!(findings.len(), 1);
        assert!(matches!(findings[0], Finding::SdkMissing { .. }));
        assert!(findings[0].render().contains("remedy"));
    }

    #[test]
    fn an_unknown_platform_is_unsupported() {
        let plan = Plan::of("tablet").expect("a tablet");
        let findings = plan.check(&[], &[], Some(&Sdk::new("windows-phone", true)));
        assert!(matches!(findings[0], Finding::UnsupportedSdk { .. }));
    }

    #[test]
    fn a_resource_is_measured_against_its_profile() {
        let plan = Plan::of("mobile").expect("a mobile");
        let bound = capacity(ResourceKind::Icon, "mobile").expect("a capacity");
        let at = Asset::new("at-capacity", ResourceKind::Icon, bound);
        let over = Asset::new("over", ResourceKind::Icon, bound + 1);
        let findings = plan.check(&[], &[at, over], Some(&complete_sdk()));
        assert_eq!(findings.len(), 1, "at the bound is allowed");
        assert!(matches!(
            &findings[0],
            Finding::OverCapacity { asset, .. } if asset == "over"
        ));
    }

    #[test]
    fn a_simulator_cannot_produce_device_evidence() {
        let mut support = Support::new();
        let evidence = super::Evidence::new(
            "mobile",
            Where::Simulator("iphone-15".to_owned()),
            Proof::Device,
            "launched",
        );
        assert!(support.record(evidence).is_err());
        assert!(support.devices().is_empty());
    }

    #[test]
    fn simulated_and_device_evidence_are_recorded_apart() {
        let mut support = Support::new();
        support
            .record(super::Evidence::new(
                "mobile",
                Where::Simulator("pixel-8".to_owned()),
                Proof::Ran,
                "launched",
            ))
            .expect("simulator evidence");
        support
            .record(super::Evidence::new(
                "mobile",
                Where::Device("pixel-8".to_owned()),
                Proof::Device,
                "installed and launched",
            ))
            .expect("device evidence");
        assert_eq!(support.simulated().len(), 1);
        assert_eq!(support.devices().len(), 1);
        assert_eq!(support.device_levels(), vec![("pixel-8", Proof::Device)]);
        assert!(!support.claims_all_devices());
        assert_eq!(support.render().len(), 2);
    }

    #[test]
    fn evidence_for_a_desktop_profile_is_refused() {
        let mut support = Support::new();
        let evidence = super::Evidence::new(
            "desktop",
            Where::Device("this-mac".to_owned()),
            Proof::Device,
            "launched",
        );
        assert!(support.record(evidence).is_err());
    }

    #[test]
    fn a_complete_mobile_plan_reports_nothing() {
        let plan = Plan::of("mobile").expect("a mobile");
        assert!(
            plan.check(
                &[Input::new(InputKind::Touch, (48, 48))],
                &[],
                Some(&complete_sdk().with_version("34")),
            )
            .is_empty()
        );
    }
}
