// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! One target's configuration: profile, triple, toolchain, SDK, mode and runner (task T18.01).
//!
//! The acceptance line is *"a profile is not an operating system, and `mini`/`embedded` have no
//! mode 1 entry"*. Both are properties of this type rather than rules a caller has to remember:
//!
//! * **The profile and the operating system are separate fields.** [`Configuration::profile`] is one
//!   of the five output profiles — a statement about capability and size — while [`Os`] is read from
//!   the triple. [`Configuration::check`] refuses an operating system's name as a profile with a reason that names
//!   the triple, so the two cannot be used interchangeably.
//! * **Mode 1 has no entry on a restricted target.** [`Mode::One`] loads JSON and CSS at run time,
//!   which `mini`/`embedded` do not link; [`Configuration::check`] refuses it there instead of letting a preview
//!   configuration become a build that cannot exist. Mode 2 is allowed everywhere.
//! * **A missing fact and an impossible request stay apart.** A missing toolchain or SDK is
//!   [`Finding::Missing`] with a remedy; an unknown profile, a mode the profile cannot have, or a
//!   runner the profile does not have is [`Finding::Unsupported`] with a reason. They are two
//!   variants for the same reason [`crate::app::build::toolchain`] has two: telling somebody to
//!   install an SDK when the real mistake is a profile name wastes their time.

#![allow(dead_code)]

/// The five output profiles, in the order the roadmap names them.
pub const PROFILES: [&str; 5] = ["desktop", "tablet", "mobile", "mini", "embedded"];

/// Whether `profile` is one of the five.
pub fn is_profile(profile: &str) -> bool {
    PROFILES.contains(&profile)
}

/// Whether a profile is one of the sized targets that has no mode 1 entry.
pub fn is_restricted(profile: &str) -> bool {
    matches!(profile, "mini" | "embedded")
}

/// An operating system, read from a target triple.
///
/// This is deliberately *not* the profile: `desktop` says what the machine can do, `Macos` says
/// which system it runs. A build can be `desktop` on any of the three, and the same system can host
/// several profiles.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Os {
    /// macOS.
    Macos,
    /// Linux.
    Linux,
    /// Windows.
    Windows,
}

impl Os {
    /// The system's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Macos => "macos",
            Self::Linux => "linux",
            Self::Windows => "windows",
        }
    }

    /// Reads a system name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "macos" | "darwin" => Some(Self::Macos),
            "linux" => Some(Self::Linux),
            "windows" | "win" => Some(Self::Windows),
            _ => None,
        }
    }

    /// The system this build is running on.
    pub fn host() -> Self {
        if cfg!(target_os = "macos") {
            Self::Macos
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else {
            Self::Linux
        }
    }

    /// The system a triple names.
    pub fn from_triple(triple: &str) -> Option<Self> {
        let triple = triple.to_ascii_lowercase();
        if triple.contains("windows") {
            Some(Self::Windows)
        } else if triple.contains("darwin") || triple.contains("apple") {
            Some(Self::Macos)
        } else if triple.contains("linux") || triple.contains("android") {
            Some(Self::Linux)
        } else {
            None
        }
    }
}

/// How a target is delivered.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// JSON and CSS interpreted at run time; a desktop/tablet/mobile preview.
    One,
    /// Rust source compiled for the target; every profile.
    Two,
}

impl Mode {
    /// The mode's number, as the roadmap writes it.
    pub fn name(&self) -> &'static str {
        match self {
            Self::One => "one",
            Self::Two => "two",
        }
    }

    /// Reads a mode number.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "one" | "1" => Some(Self::One),
            "two" | "2" => Some(Self::Two),
            _ => None,
        }
    }

    /// Whether a profile has an entry for this mode.
    ///
    /// Mode 1 is a runtime interpreter; `mini`/`embedded` build without it, which is why they must
    /// use mode 2 (principle 11).
    pub fn allows(&self, profile: &str) -> bool {
        match self {
            Self::Two => true,
            Self::One => is_profile(profile) && !is_restricted(profile),
        }
    }
}

/// How the built program is run.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Runner {
    /// On the host it was built on.
    Host,
    /// On a simulator or emulator.
    Simulator,
    /// On a physically attached device.
    Device,
    /// No local runner: the restricted targets are flashed by local tooling, not run here.
    None,
}

impl Runner {
    /// The runner's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Simulator => "simulator",
            Self::Device => "device",
            Self::None => "none",
        }
    }

    /// Reads a runner name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "host" => Some(Self::Host),
            "simulator" => Some(Self::Simulator),
            "device" => Some(Self::Device),
            "none" => Some(Self::None),
            _ => None,
        }
    }

    /// Whether a profile can be run this way.
    pub fn allows(&self, profile: &str) -> bool {
        match profile {
            "desktop" => matches!(self, Self::Host),
            "tablet" | "mobile" => matches!(self, Self::Simulator | Self::Device),
            "mini" | "embedded" => matches!(self, Self::None),
            _ => false,
        }
    }

    /// The runners a profile can use, for a message that says what to choose.
    pub fn allowed_for(profile: &str) -> &'static [&'static str] {
        match profile {
            "desktop" => &["host"],
            "tablet" | "mobile" => &["simulator", "device"],
            "mini" | "embedded" => &["none"],
            _ => &[],
        }
    }
}

/// One target's configuration.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Configuration {
    /// The output profile.
    pub profile: String,
    /// The target triple, or `None` when it has not been chosen.
    pub triple: Option<String>,
    /// The pinned toolchain, or `None` when it has not been chosen.
    pub toolchain: Option<String>,
    /// The SDK the target needs, or `None` when it has not been chosen.
    pub sdk: Option<String>,
    /// How the target is delivered.
    pub mode: Mode,
    /// How the target is run.
    pub runner: Runner,
}

impl Configuration {
    /// A configuration with the defaults a profile implies.
    pub fn new(profile: impl Into<String>) -> Self {
        let profile = profile.into();
        let restricted = is_restricted(&profile);
        Self {
            mode: if restricted { Mode::Two } else { Mode::One },
            runner: match profile.as_str() {
                "desktop" => Runner::Host,
                "tablet" | "mobile" => Runner::Simulator,
                _ => Runner::None,
            },
            profile,
            triple: None,
            toolchain: None,
            sdk: None,
        }
    }

    /// The same configuration with a triple.
    pub fn with_triple(mut self, triple: impl Into<String>) -> Self {
        self.triple = Some(triple.into());
        self
    }

    /// The same configuration with a toolchain.
    pub fn with_toolchain(mut self, toolchain: impl Into<String>) -> Self {
        self.toolchain = Some(toolchain.into());
        self
    }

    /// The same configuration with an SDK.
    pub fn with_sdk(mut self, sdk: impl Into<String>) -> Self {
        self.sdk = Some(sdk.into());
        self
    }

    /// The same configuration in a mode.
    pub fn with_mode(mut self, mode: Mode) -> Self {
        self.mode = mode;
        self
    }

    /// The same configuration with a runner.
    pub fn with_runner(mut self, runner: Runner) -> Self {
        self.runner = runner;
        self
    }

    /// The operating system the triple names, if it names one.
    pub fn os(&self) -> Option<Os> {
        self.triple.as_deref().and_then(Os::from_triple)
    }

    /// Checks the configuration against the host and what it provides.
    ///
    /// `host` is passed rather than read here so a test can ask about another machine and a caller
    /// can report what it checked against.
    pub fn check(&self, host: Os) -> Vec<Finding> {
        let mut findings = Vec::new();

        if !is_profile(&self.profile) {
            findings.push(Finding::Unsupported {
                subject: format!("profile {:?}", self.profile),
                reason: if Os::parse(&self.profile).is_some() {
                    "a profile is not an operating system; choose the OS with the triple".to_owned()
                } else {
                    format!("not one of {}", PROFILES.join(", "))
                },
            });
            // Nothing below can be judged against an unknown profile without guessing, so the
            // answer stops here rather than stacking consequences of the first mistake.
            return findings;
        }

        if !self.mode.allows(&self.profile) {
            findings.push(Finding::Unsupported {
                subject: format!("mode {} on {}", self.mode.name(), self.profile),
                reason: "mode 1 needs the JSON/CSS runtime, which mini and embedded do not link; \
                         use mode two"
                    .to_owned(),
            });
        }
        if !self.runner.allows(&self.profile) {
            findings.push(Finding::Unsupported {
                subject: format!("runner {} on {}", self.runner.name(), self.profile),
                reason: format!(
                    "this profile runs with {}",
                    Runner::allowed_for(&self.profile).join(" or ")
                ),
            });
        }

        match self.triple.as_deref() {
            None => findings.push(Finding::Missing {
                subject: "target triple".to_owned(),
                remedy: "name the triple the output is compiled for, such as \
                         x86_64-unknown-linux-gnu"
                    .to_owned(),
            }),
            Some(triple) => match Os::from_triple(triple) {
                None => findings.push(Finding::Unsupported {
                    subject: format!("triple {triple:?}"),
                    reason: "its operating system cannot be read from the triple".to_owned(),
                }),
                Some(os) => {
                    if os != host && self.sdk.is_none() {
                        // A Windows target on a macOS host is the case this project actually
                        // exercises, and xwin is the seam that makes it possible.
                        let remedy = if os == Os::Windows {
                            "run `xwin` to fetch the Windows SDK, or point the build at an \
                             installed one"
                                .to_owned()
                        } else {
                            format!("install the {} SDK and name it here", os.name())
                        };
                        findings.push(Finding::Missing {
                            subject: format!("SDK for {}", os.name()),
                            remedy,
                        });
                    }
                }
            },
        }

        if matches!(self.profile.as_str(), "tablet" | "mobile") && self.sdk.is_none() {
            // The triple says iOS or Android through its system, but the platform SDK is what
            // produces a package that can be installed at all.
            let platform = self.package_platform();
            findings.push(Finding::Missing {
                subject: format!("{platform} SDK"),
                remedy: format!(
                    "install the {platform} SDK and name it here; a device build needs it"
                ),
            });
        }

        if self.toolchain.is_none() {
            findings.push(Finding::Missing {
                subject: "toolchain".to_owned(),
                remedy: "pin the toolchain the target is built with".to_owned(),
            });
        }

        findings
    }

    /// The mobile platform this configuration packages for.
    ///
    /// Android triples end in `-linux-android`, so the triple is what decides; a configuration with
    /// no triple answers `android` because that is the platform this build has a local tool seam for
    /// (T18.06), and the answer is only used in a message.
    pub fn package_platform(&self) -> &'static str {
        if self
            .triple
            .as_deref()
            .is_some_and(|triple| triple.to_ascii_lowercase().contains("ios"))
        {
            "ios"
        } else {
            "android"
        }
    }

    /// The configuration in one line.
    pub fn render(&self) -> String {
        format!(
            "profile={} triple={} toolchain={} sdk={} mode={} runner={}",
            self.profile,
            self.triple.as_deref().unwrap_or("-"),
            self.toolchain.as_deref().unwrap_or("-"),
            self.sdk.as_deref().unwrap_or("-"),
            self.mode.name(),
            self.runner.name(),
        )
    }
}

/// One thing standing between this configuration and a build.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Finding {
    /// This machine lacks something; providing it fixes the configuration.
    Missing {
        /// What is missing.
        subject: String,
        /// What to do about it.
        remedy: String,
    },
    /// The request cannot be honoured by any machine in this build.
    Unsupported {
        /// What cannot be honoured.
        subject: String,
        /// Why not.
        reason: String,
    },
}

impl Finding {
    /// The finding in one line, naming which side it is on.
    pub fn render(&self) -> String {
        match self {
            Self::Missing { subject, remedy } => {
                format!("missing: {subject} (remedy: {remedy})")
            }
            Self::Unsupported { subject, reason } => {
                format!("unsupported: {subject} ({reason})")
            }
        }
    }

    /// Whether this finding is about the request rather than the machine.
    pub fn is_unsupported(&self) -> bool {
        matches!(self, Self::Unsupported { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::{Configuration, Finding, Mode, Os, PROFILES, Runner, is_profile, is_restricted};

    fn complete(profile: &str) -> Configuration {
        Configuration::new(profile)
            .with_triple("x86_64-unknown-linux-gnu")
            .with_toolchain("rust-1.98.1")
    }

    fn unsupported(findings: &[Finding], needle: &str) -> bool {
        findings.iter().any(|finding| match finding {
            Finding::Unsupported { subject, reason } => {
                (subject.contains(needle) || reason.contains(needle)) && finding.is_unsupported()
            }
            Finding::Missing { .. } => false,
        })
    }

    #[test]
    fn a_profile_is_one_of_five() {
        for profile in PROFILES {
            assert!(is_profile(profile));
        }
        assert!(!is_profile("macos"));
        assert!(is_restricted("mini"));
        assert!(!is_restricted("tablet"));
    }

    #[test]
    fn an_operating_system_is_not_a_profile() {
        let configuration = Configuration::new("macos");
        let findings = configuration.check(Os::Macos);
        assert!(unsupported(&findings, "profile"));
        assert!(
            findings[0]
                .render()
                .contains("a profile is not an operating system")
        );
    }

    #[test]
    fn mini_and_embedded_refuse_mode_one() {
        for profile in ["mini", "embedded"] {
            let configuration = complete(profile).with_mode(Mode::One);
            let findings = configuration.check(Os::Linux);
            assert!(
                unsupported(&findings, "mode one"),
                "{profile}: {findings:?}"
            );
        }
    }

    #[test]
    fn a_device_profile_has_both_modes() {
        for profile in ["desktop", "tablet", "mobile"] {
            for mode in [Mode::One, Mode::Two] {
                assert!(mode.allows(profile), "{profile} in {}", mode.name());
            }
        }
    }

    #[test]
    fn the_default_mode_is_two_only_on_the_restricted_profiles() {
        assert_eq!(Configuration::new("mini").mode, Mode::Two);
        assert_eq!(Configuration::new("embedded").mode, Mode::Two);
        assert_eq!(Configuration::new("desktop").mode, Mode::One);
    }

    #[test]
    fn a_runner_belongs_to_its_profile() {
        assert!(
            complete("desktop")
                .with_runner(Runner::Host)
                .check(Os::Linux)
                .iter()
                .all(|finding| !finding.render().contains("runner"))
        );
        assert!(unsupported(
            &complete("desktop")
                .with_runner(Runner::Simulator)
                .check(Os::Linux),
            "runner"
        ));
        assert!(unsupported(
            &complete("mini")
                .with_runner(Runner::Device)
                .check(Os::Linux),
            "runner"
        ));
        assert!(
            complete("tablet")
                .with_runner(Runner::Device)
                .with_sdk("android-sdk")
                .check(Os::Linux)
                .iter()
                .all(|finding| !finding.render().contains("runner"))
        );
    }

    #[test]
    fn a_cross_os_triple_without_an_sdk_is_missing_with_the_xwin_remedy() {
        let configuration = complete("desktop").with_triple("x86_64-pc-windows-msvc");
        let findings = configuration.check(Os::Macos);
        assert!(findings.iter().any(|finding| match finding {
            Finding::Missing { subject, remedy } => {
                subject.contains("windows") && remedy.contains("xwin")
            }
            Finding::Unsupported { .. } => false,
        }));
    }

    #[test]
    fn a_complete_host_configuration_reports_nothing() {
        let findings = complete("desktop").check(Os::Linux);
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn a_mobile_target_without_an_sdk_names_the_platform() {
        let configuration = complete("mobile").with_triple("aarch64-linux-android");
        let findings = configuration.check(Os::Macos);
        assert!(findings.iter().any(|finding| match finding {
            Finding::Missing { subject, remedy } => {
                subject == "android SDK" && remedy.contains("device build")
            }
            Finding::Unsupported { .. } => false,
        }));
    }

    #[test]
    fn a_missing_toolchain_is_missing_with_a_remedy() {
        let configuration = Configuration::new("desktop").with_triple("x86_64-unknown-linux-gnu");
        let findings = configuration.check(Os::Linux);
        assert!(findings.iter().any(|finding| matches!(
            finding,
            Finding::Missing { subject, .. } if subject == "toolchain"
        )));
        assert!(!unsupported(&findings, "toolchain"));
    }

    #[test]
    fn the_configuration_renders_every_field_it_holds() {
        let line = complete("desktop")
            .with_sdk("xwin")
            .with_mode(Mode::Two)
            .render();
        for part in [
            "profile=desktop",
            "triple=x86_64-unknown-linux-gnu",
            "toolchain=rust-1.98.1",
            "sdk=xwin",
            "mode=two",
            "runner=host",
        ] {
            assert!(line.contains(part), "{line} is missing {part}");
        }
    }

    #[test]
    fn a_triple_names_the_system_separately_from_the_profile() {
        let configuration = complete("desktop").with_triple("aarch64-apple-darwin");
        assert_eq!(configuration.os(), Some(Os::Macos));
        assert_eq!(configuration.package_platform(), "android");
        assert_eq!(
            complete("mobile")
                .with_triple("aarch64-apple-ios")
                .package_platform(),
            "ios"
        );
    }
}
