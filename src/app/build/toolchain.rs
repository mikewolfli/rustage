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
//!
//! [`Environment::probe`] fills the machine half by asking the machine itself — `cargo --version`,
//! `rustc --version` and `rustup target list --installed`, each as its own argv and never through a
//! shell. The command runner is a trait so a test can answer those three programs without one being
//! installed, and CLI overrides are applied on top of a finished probe rather than replacing it.

#![allow(dead_code)]

use std::path::Path;
use std::time::Duration;

use crate::contracts::permission::{EffectClass, Principal};
use crate::host::permission::{PermissionService, Scope};
use crate::host::subprocess::{DEFAULT_GRACE, OwnedProcess, ProcessSpec};

/// How long each probe program is given before it is treated as hanging.
///
/// The same order of magnitude as the host's own grace: these programs answer in milliseconds on a
/// working machine, and the limit exists to bound one that never answers.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// How much of a probe program's output is kept.
///
/// A version is one line and a target list is a few dozen; 64 KiB leaves generous room while
/// refusing to grow this process with a program that prints without end.
pub const PROBE_CAPTURE_LIMIT: usize = 64 * 1024;

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

/// What one probe program said.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ProbeOutput {
    /// Whether the program exited successfully.
    pub success: bool,
    /// What it wrote to standard output.
    pub stdout: String,
    /// What it wrote to standard error.
    pub stderr: String,
}

/// Runs one probe program, without a shell.
///
/// A trait rather than a free function so the probe is injectable: a test answers `cargo`,
/// `rustc` and `rustup` with fixed text and never depends on what is installed on the machine
/// running it.
pub trait CommandRunner {
    /// Runs `program` with `arguments`, each passed as its own argv entry.
    fn run(&self, program: &str, arguments: &[&str]) -> Result<ProbeOutput, String>;
}

/// The runner that really starts the programs, through the host's owned-process helper.
pub struct SystemCommandRunner;

impl CommandRunner for SystemCommandRunner {
    fn run(&self, program: &str, arguments: &[&str]) -> Result<ProbeOutput, String> {
        let label = if arguments.is_empty() {
            program.to_owned()
        } else {
            format!("{program} {}", arguments.join(" "))
        };
        let spec = ProcessSpec::new(label.clone(), program)
            .args(arguments.iter().copied())
            .capture_limit(PROBE_CAPTURE_LIMIT);
        // The session's own probe is the human's work, not code that arrived with a project, so it
        // is decided on the session scope. No decision table row is bypassed.
        let permit = PermissionService::new()
            .permit(EffectClass::Command, Scope::Session, Principal::Human)
            .map_err(|refusal| refusal.message())?;
        let mut process = OwnedProcess::spawn(spec, &permit)
            .map_err(|error| format!("`{label}` could not be started: {error}"))?;
        let Some(exit) = process.wait_for(PROBE_TIMEOUT) else {
            let termination = process.terminate(DEFAULT_GRACE);
            return Err(format!(
                "`{label}` did not finish within {PROBE_TIMEOUT:?} ({})",
                termination.message()
            ));
        };
        let output = process.output_so_far();
        if output.is_truncated() {
            // A truncated answer is not a complete one: claiming a target list from half of it
            // would be exactly the kind of invented fact this module exists to avoid.
            return Err(format!(
                "`{label}` wrote more than {PROBE_CAPTURE_LIMIT} bytes; kept {} bytes and dropped {}",
                output.stdout.len() + output.stderr.len(),
                output.dropped_bytes
            ));
        }
        Ok(ProbeOutput {
            success: exit.is_success(),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }
}

/// What this machine has, as a build probe reports it.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Environment {
    /// The cargo version line, or `None` when cargo is not on the path.
    pub cargo: Option<String>,
    /// The rustc version line, or `None` when rustc is not on the path.
    pub rustc: Option<String>,
    /// The target triples whose standard library is installed.
    pub targets: Vec<String>,
    /// The host SDK an explicit platform check could see, if any.
    ///
    /// This is deliberately narrow: a name appears here only when a checked signal (an environment
    /// variable the platform sets) actually named it. A cross target whose SDK this cannot see is
    /// reported as missing, never assumed present.
    pub sdk: Option<String>,
}

impl Environment {
    /// Probes this machine.
    pub fn probe() -> Self {
        Self::probe_with(&SystemCommandRunner)
    }

    /// Probes through `runner`, which a test uses to answer without a toolchain installed.
    pub fn probe_with(runner: &dyn CommandRunner) -> Self {
        let mut environment = Self::default();
        if let Ok(output) = runner.run("cargo", &["--version"]) {
            environment.cargo = version_from(&output);
        }
        if let Ok(output) = runner.run("rustc", &["--version"]) {
            environment.rustc = version_from(&output);
        }
        if let Ok(output) = runner.run("rustup", &["target", "list", "--installed"]) {
            environment.targets = parse_installed_targets(&output.stdout);
        }
        environment.sdk = platform_sdk();
        environment
    }

    /// Whether a target triple's standard library is installed.
    pub fn has_target(&self, triple: &str) -> bool {
        self.targets.iter().any(|present| present == triple)
    }

    /// What the probe found, one line per fact, for a person reading `--toolchain check`.
    pub fn render(&self) -> Vec<String> {
        let targets = if self.targets.is_empty() {
            "none detected".to_owned()
        } else {
            self.targets.join(", ")
        };
        vec![
            format!(
                "cargo: {}",
                self.cargo.as_deref().unwrap_or("not found on the path")
            ),
            format!(
                "rustc: {}",
                self.rustc.as_deref().unwrap_or("not found on the path")
            ),
            format!("targets: {targets}"),
            format!(
                "sdk: {}",
                self.sdk.as_deref().unwrap_or("not detected on this host")
            ),
        ]
    }
}

/// The first non-empty line of a version program's output.
pub fn parse_version(output: &str) -> Option<String> {
    output
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_owned)
}

/// Reads a version from a successful program, preferring standard output.
fn version_from(output: &ProbeOutput) -> Option<String> {
    if !output.success {
        return None;
    }
    parse_version(&output.stdout).or_else(|| parse_version(&output.stderr))
}

/// Reads the triples from `rustup target list --installed`.
///
/// Rustup prints one triple per line. A line with whitespace is not a triple — a warning or an
/// `info:` notice that reached standard output — and neither is a component name such as
/// `rust-std`; a triple is at least `arch-vendor-os` or `arch-os-env`, so it has two hyphens.
pub fn parse_installed_targets(output: &str) -> Vec<String> {
    let mut targets: Vec<String> = output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| !line.chars().any(char::is_whitespace))
        .filter(|line| line.matches('-').count() >= 2)
        .map(str::to_owned)
        .collect();
    targets.sort();
    targets.dedup();
    targets
}

/// The host SDK an explicit, platform-specific signal can actually see.
///
/// Only two signals are checked, both environment variables the platform's own tooling sets: a
/// Windows SDK location and a macOS `SDKROOT`. Anything else is `None`, and `None` is reported as
/// "not detected" rather than guessed at.
fn platform_sdk() -> Option<String> {
    if cfg!(target_os = "windows") {
        return std::env::var_os("WindowsSdkDir")
            .filter(|value| !value.is_empty())
            .map(|value| format!("Windows SDK ({})", Path::new(&value).display()));
    }
    if cfg!(target_os = "macos") {
        return std::env::var_os("SDKROOT")
            .filter(|value| !value.is_empty())
            .map(|value| format!("SDKROOT ({})", Path::new(&value).display()));
    }
    None
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
    use super::{
        CommandRunner, Environment, Finding, PROFILES, ProbeOutput, diagnose, is_profile,
        parse_installed_targets, parse_version, selection,
    };
    use std::cell::RefCell;

    /// A runner that answers from a closure, so no test depends on an installed toolchain.
    struct Scripted<F>(F);

    impl<F> CommandRunner for Scripted<F>
    where
        F: Fn(&str, &[&str]) -> Result<ProbeOutput, String>,
    {
        fn run(&self, program: &str, arguments: &[&str]) -> Result<ProbeOutput, String> {
            (self.0)(program, arguments)
        }
    }

    fn output(stdout: &str) -> ProbeOutput {
        ProbeOutput {
            success: true,
            stdout: stdout.to_owned(),
            stderr: String::new(),
        }
    }

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

    #[test]
    fn a_version_is_the_first_non_empty_line() {
        assert_eq!(
            parse_version("\n  cargo 1.98.1 (abc 2026-01-01)\n"),
            Some("cargo 1.98.1 (abc 2026-01-01)".to_owned())
        );
        assert_eq!(parse_version("   \n"), None);
    }

    #[test]
    fn installed_targets_are_read_one_per_line_and_ignore_noise() {
        let parsed = parse_installed_targets(
            "aarch64-apple-darwin\n\nx86_64-unknown-linux-gnu\ninfo: syncing channel\nrust-std\n",
        );
        assert_eq!(
            parsed,
            vec![
                "aarch64-apple-darwin".to_owned(),
                "x86_64-unknown-linux-gnu".to_owned(),
            ]
        );
    }

    #[test]
    fn a_missing_target_is_missing_not_unsupported() {
        let environment = intact();
        let chosen = selection("desktop", &[]).expect("a selection");
        let findings = diagnose(&chosen, Some("aarch64-pc-windows-msvc"), &environment);
        let missing = findings
            .iter()
            .find(|finding| finding.render().contains("standard library"))
            .expect("the absent target is reported");
        assert!(matches!(missing, Finding::Missing { .. }));
        assert!(missing.render().starts_with("missing environment:"));
    }

    #[test]
    fn the_probe_asks_all_three_programs_and_records_what_they_said() {
        let seen = RefCell::new(Vec::new());
        let runner = Scripted(|program: &str, arguments: &[&str]| {
            seen.borrow_mut()
                .push(format!("{program} {}", arguments.join(" ")));
            match (program, arguments) {
                ("cargo", ["--version"]) => Ok(output("cargo 1.98.1 (abc 2026-01-01)\n")),
                ("rustc", ["--version"]) => Ok(ProbeOutput {
                    success: true,
                    stdout: String::new(),
                    stderr: "rustc 1.98.1 (abc 2026-01-01)\n".to_owned(),
                }),
                ("rustup", ["target", "list", "--installed"]) => {
                    Ok(output("aarch64-apple-darwin\nx86_64-unknown-linux-gnu\n"))
                }
                other => Err(format!("unexpected probe {other:?}")),
            }
        });

        let environment = Environment::probe_with(&runner);
        assert_eq!(
            seen.into_inner(),
            vec![
                "cargo --version".to_owned(),
                "rustc --version".to_owned(),
                "rustup target list --installed".to_owned(),
            ]
        );
        assert_eq!(
            environment.cargo.as_deref(),
            Some("cargo 1.98.1 (abc 2026-01-01)")
        );
        assert_eq!(
            environment.rustc.as_deref(),
            Some("rustc 1.98.1 (abc 2026-01-01)")
        );
        assert!(environment.has_target("aarch64-apple-darwin"));
        assert!(environment.has_target("x86_64-unknown-linux-gnu"));
    }

    #[test]
    fn a_probe_that_cannot_start_cargo_leaves_it_missing() {
        let runner =
            Scripted(|program: &str, _: &[&str]| Err(format!("`{program}` could not be started")));
        let environment = Environment::probe_with(&runner);
        assert!(environment.cargo.is_none());
        assert!(environment.rustc.is_none());
        assert!(environment.targets.is_empty());

        let chosen = selection("desktop", &[]).expect("a selection");
        let findings = diagnose(&chosen, None, &environment);
        assert!(findings.iter().any(|finding| matches!(
            finding,
            Finding::Missing { capability, .. } if capability == "cargo"
        )));
    }
}
