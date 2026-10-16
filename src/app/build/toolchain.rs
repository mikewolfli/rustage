// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The toolchain a build needs, and the feature selection that decides it (task T16.01).
//!
//! The acceptance line is *"a missing environment and an unsupported capability are diagnosed
//! separately"*. That split is the whole design:
//!
//! * [`Finding::Missing`] is a fact about **this machine** — cargo is not on the path, a target's
//!   standard library is not installed. It has a remedy the person can carry out, and this build
//!   would work once they did.
//! * [`Finding::Unsupported`] is a fact about **this build or the target** — a profile name that is
//!   not one of the five, a feature selection that would build a configuration that does not exist
//!   (principle 51). No remedy on the machine helps; the request itself is wrong.
//!
//! Conflating them is how a person is told to install a missing SDK when the real problem is a typo
//! in a profile name, so they are two variants of one enum rather than one message with a prefix.

#![allow(dead_code)]

/// The five output profiles, in the order the roadmap names them.
pub const PROFILES: [&str; 5] = ["desktop", "tablet", "mobile", "mini", "embedded"];

/// Whether `profile` is one of the five.
pub fn is_profile(profile: &str) -> bool {
    PROFILES.contains(&profile)
}

/// One thing standing between this request and a build.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Finding {
    /// This machine lacks something; installing or adding it fixes the build.
    Missing {
        /// What is missing, such as `cargo` or the target's standard library.
        capability: String,
        /// What to do about it.
        remedy: String,
    },
    /// The request cannot be honoured by any machine in this build.
    Unsupported {
        /// What cannot be honoured.
        capability: String,
        /// Why not.
        reason: String,
    },
}

impl Finding {
    /// The finding in one line, naming which side it is on.
    pub fn render(&self) -> String {
        match self {
            Self::Missing { capability, remedy } => {
                format!("missing environment: {capability} (remedy: {remedy})")
            }
            Self::Unsupported { capability, reason } => {
                format!("unsupported: {capability} ({reason})")
            }
        }
    }
}

/// What this machine has, as a build probe reports it.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Environment {
    /// The cargo version, or `None` when cargo is not on the path.
    pub cargo: Option<String>,
    /// The rustc version, or `None` when rustc is not on the path.
    pub rustc: Option<String>,
    /// The target triples whose standard library is installed.
    pub targets: Vec<String>,
    /// The cross-compilation SDK in use, if any (a Windows SDK reached through `xwin` names itself
    /// here).
    pub sdk: Option<String>,
}

impl Environment {
    /// Whether a target triple's standard library is installed.
    pub fn has_target(&self, triple: &str) -> bool {
        self.targets.iter().any(|present| present == triple)
    }
}

/// Which profile and features a build is compiled with.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Selection {
    /// The output profile.
    pub profile: String,
    /// Extra features beyond the profile, in order.
    pub extra: Vec<String>,
}

impl Selection {
    /// The cargo feature arguments for this selection.
    ///
    /// Profile features are exclusive: the project's rule (principle 51) is
    /// `--no-default-features --features <profile>`, and a default feature must not be added on top
    /// of a target profile or the result is a configuration no CI builds.
    pub fn feature_arguments(&self) -> Vec<String> {
        let mut features = vec![self.profile.clone()];
        features.extend(self.extra.iter().cloned());
        vec![
            "--no-default-features".to_owned(),
            "--features".to_owned(),
            features.join(","),
        ]
    }

    /// The selection in one line.
    pub fn render(&self) -> String {
        let mut line = format!("profile {} with --no-default-features", self.profile);
        if !self.extra.is_empty() {
            line.push_str(&format!(" plus {}", self.extra.join(", ")));
        }
        line
    }
}

/// Validates a profile and its extra features.
pub fn selection(profile: &str, extra: &[String]) -> Result<Selection, Vec<Finding>> {
    let mut findings = Vec::new();
    if !is_profile(profile) {
        findings.push(Finding::Unsupported {
            capability: format!("profile {profile:?}"),
            reason: format!("not one of {}", PROFILES.join(", ")),
        });
    }
    for feature in extra {
        if is_profile(feature) {
            findings.push(Finding::Unsupported {
                capability: format!("feature {feature:?}"),
                reason: "another profile's feature cannot be combined with this one; a build has \
                         exactly one output profile"
                    .to_owned(),
            });
        }
    }
    if !findings.is_empty() {
        return Err(findings);
    }
    Ok(Selection {
        profile: profile.to_owned(),
        extra: extra.to_vec(),
    })
}

/// Probes the environment and the selection, keeping the two kinds of finding apart.
pub fn diagnose(
    selection: &Selection,
    triple: Option<&str>,
    environment: &Environment,
) -> Vec<Finding> {
    let mut findings = Vec::new();
    if environment.cargo.is_none() {
        findings.push(Finding::Missing {
            capability: "cargo".to_owned(),
            remedy: "install Rust (https://rustup.rs) and put cargo on the path".to_owned(),
        });
    }
    if environment.rustc.is_none() {
        findings.push(Finding::Missing {
            capability: "rustc".to_owned(),
            remedy: "install the pinned toolchain with `rustup toolchain install`".to_owned(),
        });
    }
    if let Some(triple) = triple
        && !environment.has_target(triple)
    {
        // A cross target reached through `xwin` still needs the standard library; the SDK being
        // present does not install it, so this stays a `Missing` with both remedies named.
        let remedy = if triple.contains("windows") {
            "`rustup target add <triple>` and, for the Windows SDK headers and libs, `xwin` (or the \
             SDK installed natively)"
                .to_owned()
        } else {
            format!("`rustup target add {triple}`")
        };
        findings.push(Finding::Missing {
            capability: format!("standard library for {triple}"),
            remedy,
        });
    }
    if let Some(triple) = triple
        && triple.contains("windows")
        && environment.sdk.is_none()
    {
        findings.push(Finding::Missing {
            capability: "Windows SDK".to_owned(),
            remedy: "run `xwin` to fetch the SDK, or point the build at an installed one"
                .to_owned(),
        });
    }
    // A stripped-profile target that has no `alloc` still builds, but the constraint is a project
    // fact rather than a machine one, so it is reported as unsupported when the profile and the
    // triple plainly disagree.
    if selection.profile == "embedded"
        && triple.is_some_and(|triple| triple.contains("apple-darwin"))
    {
        findings.push(Finding::Unsupported {
            capability: format!("profile embedded on {}", triple.unwrap_or_default()),
            reason: "the embedded profile is for a bare target, not a hosted desktop one"
                .to_owned(),
        });
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::{Environment, Finding, PROFILES, diagnose, is_profile, selection};

    fn intact() -> Environment {
        Environment {
            cargo: Some("cargo 1.98.1".to_owned()),
            rustc: Some("rustc 1.98.1".to_owned()),
            targets: vec!["x86_64-unknown-linux-gnu".to_owned()],
            sdk: None,
        }
    }

    #[test]
    fn a_profile_is_one_of_five() {
        for profile in PROFILES {
            assert!(is_profile(profile));
        }
        assert!(!is_profile("watch"));
    }

    #[test]
    fn a_selection_uses_one_profile_feature_and_no_defaults() {
        let chosen = selection("mini", &[]).expect("a selection");
        assert_eq!(
            chosen.feature_arguments(),
            vec!["--no-default-features", "--features", "mini"],
        );
    }

    #[test]
    fn an_unknown_profile_is_unsupported_not_missing() {
        let findings = selection("watch", &[]).expect_err("an unknown profile");
        assert!(matches!(findings[0], Finding::Unsupported { .. }));
        assert!(findings[0].render().starts_with("unsupported:"));
    }

    #[test]
    fn two_profiles_cannot_be_combined() {
        let findings = selection("desktop", &["mini".to_owned()]).expect_err("two profiles");
        assert!(
            findings
                .iter()
                .any(|finding| finding.render().contains("exactly one"))
        );
    }

    #[test]
    fn a_missing_toolchain_is_missing_with_a_remedy() {
        let environment = Environment::default();
        let chosen = selection("desktop", &[]).expect("a selection");
        let findings = diagnose(&chosen, Some("x86_64-unknown-linux-gnu"), &environment);
        assert!(findings.iter().any(|finding| matches!(
            finding,
            Finding::Missing { capability, .. } if capability == "cargo"
        )));
        assert!(
            findings
                .iter()
                .all(|finding| matches!(finding, Finding::Missing { .. }))
        );
    }

    #[test]
    fn a_missing_cross_target_names_the_sdk_on_windows() {
        let environment = intact();
        let chosen = selection("desktop", &[]).expect("a selection");
        let findings = diagnose(&chosen, Some("x86_64-pc-windows-msvc"), &environment);
        assert!(
            findings
                .iter()
                .any(|finding| finding.render().contains("xwin"))
        );
        assert!(
            findings
                .iter()
                .any(|finding| finding.render().contains("standard library"))
        );
    }

    #[test]
    fn an_intact_environment_for_an_installed_target_reports_nothing() {
        let environment = intact();
        let chosen = selection("desktop", &[]).expect("a selection");
        assert!(diagnose(&chosen, Some("x86_64-unknown-linux-gnu"), &environment).is_empty());
    }
}
