// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The restricted targets: static storage, capacity, `no_std` applicability and the link audit
//! (task T18.05).
//!
//! The acceptance line is *"development dependencies do not leak, and an actual target build meets
//! the gate"*. The module separates the three things that can go wrong:
//!
//! * **A construction the target cannot express** is [`Finding::FeatureUnavailable`] — the
//!   JSON/CSS runtime, the filesystem and the network are not there; `mini` is the library's
//!   `no_std` profile (`alloc_frugal`) and `embedded` keeps the paired `embedded_surface` contract
//!   with no allocator. `embedded` is **not** marked `no_std` by the library, so this check does not
//!   invent that restriction — the actual target build remains the authority (principle 12/24).
//! * **A dependency graph that reaches the artifact** is [`Finding::DevelopmentDependency`] or
//!   [`Finding::StandardLibrary`]. [`audit`] reads a real `cargo tree` closure; a build dependency
//!   is not a leak because it runs before the artifact exists, and the distinction is stated rather
//!   than folded together.
//! * **A gate that was not run is not a pass.** [`Gate::verdict`] answers `Err` for `None` — the
//!   honest "unverified" — and `Err` for a check that stopped before a build. Only a proof at
//!   [`Proof::Built`] or better passes.
//!
//! Capacity is the library's own bound for the stripped profile
//! ([`crate::app::targets::snapshot::capacity`]), so this build does not keep a second number that
//! could disagree with what the generated program can hold.

#![allow(dead_code)]

use super::matrix::Proof;
use super::snapshot::capacity as profile_capacity;

/// The two restricted profiles.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Restricted {
    /// `mini`: static storage, `alloc` but no `std`.
    Mini,
    /// `embedded`: no `std` and no allocator.
    Embedded,
}

impl Restricted {
    /// The profile's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Mini => "mini",
            Self::Embedded => "embedded",
        }
    }

    /// Reads a restricted profile's name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "mini" => Some(Self::Mini),
            "embedded" => Some(Self::Embedded),
            _ => None,
        }
    }

    /// The profiles, in roadmap order.
    pub fn all() -> [Self; 2] {
        [Self::Mini, Self::Embedded]
    }

    /// How many children a container holds on this profile.
    pub fn capacity(&self) -> usize {
        profile_capacity(self.name()).unwrap_or(0)
    }
}

/// A capability a design may ask the restricted target for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Feature {
    /// The standard library.
    Std,
    /// An allocator.
    Alloc,
    /// Heap allocation, which needs [`Feature::Alloc`].
    Heap,
    /// The mode 1 JSON runtime.
    JsonRuntime,
    /// The mode 1 CSS engine.
    CssRuntime,
    /// Files.
    Filesystem,
    /// Sockets.
    Network,
}

impl Feature {
    /// The feature's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Std => "std",
            Self::Alloc => "alloc",
            Self::Heap => "heap",
            Self::JsonRuntime => "json-runtime",
            Self::CssRuntime => "css-runtime",
            Self::Filesystem => "filesystem",
            Self::Network => "network",
        }
    }

    /// Reads a feature name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "std" => Some(Self::Std),
            "alloc" => Some(Self::Alloc),
            "heap" => Some(Self::Heap),
            "json-runtime" => Some(Self::JsonRuntime),
            "css-runtime" => Some(Self::CssRuntime),
            "filesystem" => Some(Self::Filesystem),
            "network" => Some(Self::Network),
            _ => None,
        }
    }

    /// Whether the profile has this feature.
    ///
    /// The source is the library's own profile contract: `mini` is `#![no_std]` and compiles against
    /// `core` + `alloc`; `embedded` selects `embedded_surface` (static tables, no allocator) but the
    /// library does not put it in `no_std`, so `std` is not reported as missing there.
    pub fn available_on(&self, profile: Restricted) -> bool {
        matches!(
            (self, profile),
            (Self::Std, Restricted::Embedded) | (Self::Alloc | Self::Heap, Restricted::Mini)
        )
    }
}

/// How a container's child count compares with the target's bound.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Relation {
    /// Fewer children than the storage holds.
    Below,
    /// Exactly as many as it holds.
    At,
    /// More than it holds.
    Over,
}

/// The relation between a child count and a bound.
///
/// The three cases are named rather than reduced to a boolean because the acceptance for this task
/// measures all three: at the bound must behave like below, and only over may block.
pub fn relation(children: usize, bound: usize) -> Relation {
    match children.cmp(&bound) {
        std::cmp::Ordering::Less => Relation::Below,
        std::cmp::Ordering::Equal => Relation::At,
        std::cmp::Ordering::Greater => Relation::Over,
    }
}

/// Why a restricted target cannot build a design as asked.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Finding {
    /// Mode 1 has no entry on a restricted target.
    ModeOne {
        /// The profile.
        profile: Restricted,
    },
    /// A capability the profile does not have.
    FeatureUnavailable {
        /// The capability.
        feature: Feature,
        /// The profile.
        profile: Restricted,
        /// Why it is not there.
        reason: String,
    },
    /// A container holds more children than the storage has room for.
    OverCapacity {
        /// How many it holds.
        children: usize,
        /// The bound.
        capacity: usize,
        /// The profile.
        profile: Restricted,
    },
    /// A development dependency reached the artifact's dependency closure.
    DevelopmentDependency {
        /// The crate.
        name: String,
        /// What pulled it in, when the graph says.
        via: Option<String>,
    },
    /// The standard library is in the artifact's closure on a target that has none.
    StandardLibrary {
        /// The crate that pulled it in, when the graph says.
        name: String,
        /// What pulled it in, when the graph says.
        via: Option<String>,
    },
}

impl Finding {
    /// The finding in one line.
    pub fn render(&self) -> String {
        match self {
            Self::ModeOne { profile } => format!(
                "unsupported: mode one on {}; use mode two, which needs no JSON/CSS runtime",
                profile.name()
            ),
            Self::FeatureUnavailable {
                feature,
                profile,
                reason,
            } => format!(
                "unsupported: {} on {} ({reason})",
                feature.name(),
                profile.name()
            ),
            Self::OverCapacity {
                children,
                capacity,
                profile,
            } => format!(
                "{children} children exceed the {capacity} the {} profile holds",
                profile.name()
            ),
            Self::DevelopmentDependency { name, via } => match via {
                Some(parent) => format!(
                    "{name} reaches the artifact through {parent}; it is a development dependency"
                ),
                None => format!("{name} reaches the artifact; it is a development dependency"),
            },
            Self::StandardLibrary { name, via } => match via {
                Some(parent) => format!("{name} pulls the standard library in through {parent}"),
                None => format!("{name} pulls the standard library into a no_std target"),
            },
        }
    }
}

/// Checks a restricted target's design against its profile.
pub fn check(
    profile: &str,
    children: usize,
    mode_one: bool,
    features: &[Feature],
) -> Result<Vec<Finding>, String> {
    let Some(profile) = Restricted::parse(profile) else {
        return Err(format!(
            "{profile:?} is not a restricted profile (use {} or {})",
            Restricted::Mini.name(),
            Restricted::Embedded.name()
        ));
    };
    let mut findings = Vec::new();
    if mode_one {
        findings.push(Finding::ModeOne { profile });
    }
    for feature in features {
        if !feature.available_on(profile) {
            findings.push(Finding::FeatureUnavailable {
                feature: *feature,
                profile,
                reason: feature_reason(*feature, profile),
            });
        }
    }
    let bound = profile.capacity();
    if relation(children, bound) == Relation::Over {
        findings.push(Finding::OverCapacity {
            children,
            capacity: bound,
            profile,
        });
    }
    Ok(findings)
}

/// Why a feature is absent from a profile, in the terms that apply to it.
fn feature_reason(feature: Feature, profile: Restricted) -> String {
    match feature {
        Feature::Std => "the mini profile is no_std".to_owned(),
        Feature::Alloc | Feature::Heap => {
            format!("the {} profile has no allocator", profile.name())
        }
        Feature::JsonRuntime | Feature::CssRuntime => {
            "mode 1 interprets at run time, and the restricted profiles do not link it".to_owned()
        }
        Feature::Filesystem => "the profile has no filesystem".to_owned(),
        Feature::Network => "the profile has no network stack".to_owned(),
    }
}

/// What a dependency is for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DependencyKind {
    /// Linked into the artifact.
    Normal,
    /// Runs at build time; it is not in the artifact.
    Build,
    /// Used by tests and benches; it must never reach the artifact.
    Dev,
}

impl DependencyKind {
    /// The kind's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Build => "build",
            Self::Dev => "dev",
        }
    }
}

/// One dependency in a closure.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Dependency {
    /// The crate.
    pub name: String,
    /// What it is for.
    pub kind: DependencyKind,
    /// What pulled it in, when known.
    pub via: Option<String>,
}

impl Dependency {
    /// A dependency of a kind.
    pub fn new(name: impl Into<String>, kind: DependencyKind) -> Self {
        Self {
            name: name.into(),
            kind,
            via: None,
        }
    }

    /// The same dependency with the crate that pulled it in.
    pub fn through(mut self, via: impl Into<String>) -> Self {
        self.via = Some(via.into());
        self
    }
}

/// Reads a `cargo tree` closure's crate names.
///
/// Handles both `--prefix none` (`serde v1.0.200`) and the default tree glyphs
/// (`│   ├── serde v1.0.200`). Everything on the line after the name is discarded: the audit asks
/// which crates are present, not which versions, because a version change is not a leak.
pub fn parse_tree(text: &str) -> Vec<Dependency> {
    let mut dependencies: Vec<Dependency> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start_matches(|character: char| {
            matches!(
                character,
                ' ' | '\t' | '│' | '├' | '└' | '─' | '┬' | '┐' | '┘' | '┌'
            )
        });
        let Some(name) = trimmed.split_whitespace().next() else {
            continue;
        };
        if name.is_empty() || name.starts_with('(') || name == "[" {
            continue;
        }
        if !dependencies
            .iter()
            .any(|dependency| dependency.name == name)
        {
            dependencies.push(Dependency::new(name, DependencyKind::Normal));
        }
    }
    dependencies
}

/// Audits a dependency closure for a restricted target.
pub fn audit(dependencies: &[Dependency], profile: Restricted) -> Vec<Finding> {
    let mut findings = Vec::new();
    for dependency in dependencies {
        match dependency.kind {
            // A build dependency runs before the artifact exists, so it is not a leak. It is named
            // here rather than silently skipped so the rule is visible to a reader.
            DependencyKind::Build => {}
            DependencyKind::Dev => findings.push(Finding::DevelopmentDependency {
                name: dependency.name.clone(),
                via: dependency.via.clone(),
            }),
            DependencyKind::Normal => {
                if dependency.name == "std" {
                    // `mini` is the library's no_std profile; `embedded` is not, so a `std` edge is
                    // only a leak where the contract actually says there is no standard library.
                    if profile == Restricted::Mini {
                        findings.push(Finding::StandardLibrary {
                            name: dependency.name.clone(),
                            via: dependency.via.clone(),
                        });
                    }
                } else if dependency.name == "alloc" && profile == Restricted::Embedded {
                    findings.push(Finding::FeatureUnavailable {
                        feature: Feature::Alloc,
                        profile,
                        reason:
                            "the artifact's closure reaches alloc, which embedded does not have"
                                .to_owned(),
                    });
                }
            }
        }
    }
    findings
}

/// The real target build a restricted profile needs.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Gate {
    /// The profile.
    pub profile: Restricted,
    /// What the last target build reached, or `None` when it was not run.
    pub proof: Option<Proof>,
}

impl Gate {
    /// A gate with no build yet.
    pub fn new(profile: Restricted) -> Self {
        Self {
            profile,
            proof: None,
        }
    }

    /// Records what a build run reached.
    pub fn record(&mut self, proof: Proof) {
        self.proof = Some(proof);
    }

    /// Whether the gate is met.
    ///
    /// `None` and a proof below a build are errors with different sentences: "nothing was run" and
    /// "a check is not a build" are different facts, and neither is a pass.
    pub fn verdict(&self) -> Result<Proof, String> {
        match self.proof {
            None => Err(format!(
                "no target build was run for {}; that is unverified, not a pass",
                self.profile.name()
            )),
            Some(proof @ (Proof::Built | Proof::Ran | Proof::Device)) => Ok(proof),
            Some(proof) => Err(format!(
                "the {} target reached {}, not a build",
                self.profile.name(),
                proof.name()
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Dependency, DependencyKind, Feature, Finding, Gate, Relation, Restricted, audit, check,
        parse_tree, relation,
    };
    use crate::app::targets::matrix::Proof;

    #[test]
    fn only_mini_and_embedded_are_restricted() {
        for profile in Restricted::all() {
            assert_eq!(Restricted::parse(profile.name()), Some(profile));
        }
        assert!(Restricted::parse("desktop").is_none());
        assert!(check("desktop", 0, false, &[]).is_err());
    }

    #[test]
    fn capacity_comes_from_the_library_bound() {
        let bound = Restricted::Mini.capacity();
        assert_eq!(bound, Restricted::Embedded.capacity());
        assert_eq!(relation(bound - 1, bound), Relation::Below);
        assert_eq!(relation(bound, bound), Relation::At);
        assert_eq!(relation(bound + 1, bound), Relation::Over);
    }

    #[test]
    fn mode_one_is_refused_on_both_restricted_profiles() {
        for profile in Restricted::all() {
            let findings = check(profile.name(), 0, true, &[]).expect("a profile");
            assert!(matches!(findings[0], Finding::ModeOne { .. }));
        }
    }

    #[test]
    fn at_the_capacity_is_allowed_and_over_is_not() {
        let bound = Restricted::Mini.capacity();
        assert!(
            check("mini", bound, false, &[])
                .expect("a profile")
                .is_empty()
        );
        let findings = check("mini", bound + 1, false, &[]).expect("a profile");
        assert!(matches!(
            findings[0],
            Finding::OverCapacity { children, capacity, .. } if children == bound + 1 && capacity == bound
        ));
    }

    #[test]
    fn the_runtime_and_the_os_services_are_unavailable_on_both_profiles() {
        for profile in Restricted::all() {
            for feature in [
                Feature::JsonRuntime,
                Feature::CssRuntime,
                Feature::Filesystem,
                Feature::Network,
            ] {
                let findings = check(profile.name(), 0, false, &[feature]).expect("a profile");
                assert!(
                    findings.iter().any(|finding| matches!(
                        finding,
                        Finding::FeatureUnavailable { feature: found, .. } if *found == feature
                    )),
                    "{} on {}",
                    feature.name(),
                    profile.name()
                );
            }
        }
    }

    #[test]
    fn the_standard_library_is_missing_only_where_the_library_says_no_std() {
        let findings = check("mini", 0, false, &[Feature::Std]).expect("a profile");
        assert!(matches!(
            findings[0],
            Finding::FeatureUnavailable {
                feature: Feature::Std,
                ..
            }
        ));
        // `embedded` selects `embedded_surface`, not `no_std`; this check must not invent more than
        // the corresponding library contract.
        assert!(
            check("embedded", 0, false, &[Feature::Std])
                .expect("a profile")
                .is_empty()
        );
    }

    #[test]
    fn mini_has_an_allocator_and_embedded_does_not() {
        assert!(
            check("mini", 0, false, &[Feature::Alloc, Feature::Heap])
                .expect("a profile")
                .is_empty()
        );
        let findings =
            check("embedded", 0, false, &[Feature::Alloc, Feature::Heap]).expect("a profile");
        assert_eq!(findings.len(), 2);
        assert!(findings.iter().all(|finding| matches!(
            finding,
            Finding::FeatureUnavailable {
                profile: Restricted::Embedded,
                ..
            }
        )));
    }

    #[test]
    fn a_development_dependency_reaching_the_artifact_is_a_leak() {
        let dependencies = vec![
            Dependency::new("serde", DependencyKind::Normal),
            Dependency::new("proptest", DependencyKind::Dev).through("rustage"),
            Dependency::new("cc", DependencyKind::Build),
        ];
        let findings = audit(&dependencies, Restricted::Mini);
        assert_eq!(findings.len(), 1);
        assert!(matches!(
            &findings[0],
            Finding::DevelopmentDependency { name, via } if name == "proptest" && via.as_deref() == Some("rustage")
        ));
        assert!(findings[0].render().contains("through rustage"));
    }

    #[test]
    fn the_standard_library_in_the_closure_is_reported_on_a_no_std_target() {
        let dependencies = vec![
            Dependency::new("std", DependencyKind::Normal).through("somecrate"),
            Dependency::new("alloc", DependencyKind::Normal),
        ];
        let findings = audit(&dependencies, Restricted::Mini);
        assert!(findings.iter().any(|finding| matches!(
            finding,
            Finding::StandardLibrary { name, .. } if name == "std"
        )));
        // `embedded` is not `no_std`, but it has no allocator: the same closure reports `alloc`
        // and does not report `std`.
        let findings = audit(&dependencies, Restricted::Embedded);
        assert!(
            !findings
                .iter()
                .any(|finding| matches!(finding, Finding::StandardLibrary { .. }))
        );
        assert!(findings.iter().any(|finding| matches!(
            finding,
            Finding::FeatureUnavailable {
                feature: Feature::Alloc,
                ..
            }
        )));
    }

    #[test]
    fn a_tree_can_be_read_with_or_without_glyphs() {
        let glyphs =
            "rustage v0.1.0\n├── serde v1.0.200\n│   └── serde_derive v1.0.200\n└── cc v1.0.0\n";
        let plain = "rustage v0.1.0\nserde v1.0.200\nserde_derive v1.0.200\ncc v1.0.0\n";
        let from_glyphs = parse_tree(glyphs);
        let from_plain = parse_tree(plain);
        assert_eq!(from_glyphs, from_plain);
        assert_eq!(
            from_glyphs
                .iter()
                .map(|dependency| dependency.name.as_str())
                .collect::<Vec<&str>>(),
            vec!["rustage", "serde", "serde_derive", "cc"]
        );
    }

    #[test]
    fn a_gate_that_was_not_run_is_not_a_pass() {
        let gate = Gate::new(Restricted::Mini);
        let verdict = gate.verdict().expect_err("no build");
        assert!(verdict.contains("unverified"));
    }

    #[test]
    fn a_check_is_not_a_build_and_a_build_passes() {
        let mut gate = Gate::new(Restricted::Embedded);
        gate.record(Proof::Checked);
        assert!(gate.verdict().is_err());
        gate.record(Proof::Built);
        assert_eq!(gate.verdict(), Ok(Proof::Built));
    }
}
