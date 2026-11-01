// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Maintaining a released line: branches, defect severity, upgrade drills and review triggers
//! (task T25.08).
//!
//! The acceptance line is *"an upstream or toolchain upgrade triggers the matching regression, and
//! support changes are recorded"*. The rules are a table, not prose in a document that drifts:
//!
//! * **A branch has one job.** [`branches`] names the main line, the release branch and the
//!   maintenance branch, and what may land on each — a fix that lands everywhere is stated as such
//!   rather than left to judgement.
//! * **Severity decides whether a release may ship.** [`Severity::blocks_release`] is true for a
//!   blocker or a critical defect and false for major/minor ones, so the classification has a
//!   consequence instead of being a label.
//! * **A drill is a re-check of a real stage.** [`drill`] verifies the stage manifest, every
//!   digest, the language packs and the frozen facts. It reports every failure rather than the
//!   first, because an upgrade drill is where several things are expected to move at once.
//! * **An upgrade names the regression it triggers.** [`triggers`] compares the previous and new
//!   facts and answers the suites that must run; an unchanged input triggers nothing.

use std::path::Path;

use sha2::{Digest, Sha256};

use crate::project::record::json::{Json, parse};

/// How serious a defect is.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Severity {
    /// Data loss, wrong generated output, a privilege escape or a release blocker.
    Blocker,
    /// A regression in a supported journey with no workaround.
    Critical,
    /// A defect with a workaround in a supported journey.
    Major,
    /// A cosmetic or low-frequency defect.
    Minor,
}

impl Severity {
    /// Every severity, most serious first.
    pub const ALL: [Self; 4] = [Self::Blocker, Self::Critical, Self::Major, Self::Minor];

    /// Its name, as a report writes it.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Blocker => "blocker",
            Self::Critical => "critical",
            Self::Major => "major",
            Self::Minor => "minor",
        }
    }

    /// Reads a severity's name.
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|severity| severity.name() == text)
    }

    /// Whether a release carrying it may ship.
    pub const fn blocks_release(self) -> bool {
        matches!(self, Self::Blocker | Self::Critical)
    }
}

/// The branch policy, one line per branch.
pub fn branches() -> [&'static str; 3] {
    [
        "main: the next line; every fix lands here first",
        "release/x.y: the release candidate line; fixes cherry-pick from main, features do not land",
        "maintenance/x.y: a shipped line; only blocker and critical fixes backport, each with the \
         regression that proved it",
    ]
}

/// When a line is reviewed: on every upgrade and before every maintenance release.
pub fn review_cadence() -> &'static str {
    "review on every upstream or toolchain upgrade and before every maintenance release"
}

/// The facts an upgrade compares.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Upgrade {
    /// The pinned widget revision.
    pub widget_revision: String,
    /// The Rust toolchain the release was built with.
    pub toolchain: String,
    /// The product package version.
    pub package_version: String,
}

/// The regression suites an upgrade triggers, by the fact that changed.
pub fn triggers(previous: &Upgrade, next: &Upgrade) -> Vec<&'static str> {
    let mut suites = Vec::new();
    if previous.widget_revision != next.widget_revision {
        suites.push("widget contract, style, preview and i18n regression");
    }
    if previous.toolchain != next.toolchain {
        suites.push("build, packaging and release journey regression");
    }
    if previous.package_version != next.package_version {
        suites.push("release freeze, stage, verify and install journey");
    }
    suites
}

/// What an upgrade drill checked.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Drill {
    /// The stage's frozen revision.
    pub revision: String,
    /// The stage's frozen package version.
    pub package_version: String,
    /// The stage's frozen widget revision.
    pub widget_revision: String,
    /// Every file the stage lists.
    pub files: Vec<String>,
    /// The language packs in the stage.
    pub packs: Vec<String>,
}

impl Drill {
    /// The drill in lines.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "drill: {} file(s), {} language pack(s), revision {}, package {}, widgets {}",
            self.files.len(),
            self.packs.len(),
            self.revision,
            self.package_version,
            self.widget_revision
        )];
        lines.extend(self.files.iter().map(|file| format!("  ok {file}")));
        lines
    }
}

/// Re-checks a stage as an upgrade drill. Every failure is collected and returned together.
pub fn drill(stage_directory: &Path) -> Result<Drill, Vec<String>> {
    let manifest_path = stage_directory.join(super::install::STAGE_MANIFEST);
    let mut failures = Vec::new();
    let text = match std::fs::read_to_string(&manifest_path) {
        Ok(text) => text,
        Err(error) => {
            return Err(vec![format!(
                "cannot read {}: {error}",
                manifest_path.display()
            )]);
        }
    };
    let parsed = match parse(&text) {
        Ok(parsed) => parsed,
        Err(error) => {
            return Err(vec![format!(
                "{} is not valid JSON: {error}",
                manifest_path.display()
            )]);
        }
    };
    let Some(object) = parsed.as_object() else {
        return Err(vec![format!(
            "{} is not an object",
            manifest_path.display()
        )]);
    };
    let files = object
        .get("files")
        .and_then(Json::as_array)
        .ok_or_else(|| vec![format!("{} has no files array", manifest_path.display())])?;
    let mut checked = Vec::new();
    let mut packs = Vec::new();
    for file in files {
        let Some(entry) = file.as_object() else {
            failures.push("a stage file entry is not an object".to_owned());
            continue;
        };
        let Some(path) = entry.get("path").and_then(Json::as_str) else {
            failures.push("a stage file entry has no path".to_owned());
            continue;
        };
        let Some(expected) = entry.get("digest").and_then(Json::as_str) else {
            failures.push(format!("{path} has no digest"));
            continue;
        };
        match std::fs::read(stage_directory.join(path)) {
            Ok(bytes) => {
                let digest = digest_of(&bytes);
                if !digest.eq_ignore_ascii_case(expected) {
                    failures.push(format!("{path} does not match its digest"));
                } else {
                    checked.push(path.to_owned());
                }
            }
            Err(error) => failures.push(format!("cannot read {path}: {error}")),
        }
        if path.ends_with(".json") && path.contains("language/") {
            packs.push(path.to_owned());
        }
    }
    if packs.is_empty() {
        failures.push("the stage holds no language pack".to_owned());
    }
    let freeze = object.get("freeze").and_then(Json::as_object);
    let fact = |name: &str| {
        freeze
            .and_then(|freeze| freeze.get(name))
            .and_then(Json::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let revision = fact("revision");
    let package_version = fact("package_version");
    let widget_revision = fact("widget_revision");
    for (name, value) in [
        ("revision", &revision),
        ("package_version", &package_version),
        ("widget_revision", &widget_revision),
    ] {
        if value.is_empty() {
            failures.push(format!("the stage records no {name}"));
        }
    }
    if failures.is_empty() {
        Ok(Drill {
            revision,
            package_version,
            widget_revision,
            files: checked,
            packs,
        })
    } else {
        Err(failures)
    }
}

/// A file's lower-case SHA-256.
fn digest_of(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Severity, Upgrade, branches, drill, review_cadence, triggers};
    use crate::app::release::Freeze;
    use crate::app::release::stage::stage;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "rustage-maintenance-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&path).expect("a scratch directory");
        path
    }

    fn staged(name: &str) -> PathBuf {
        let root = scratch(name);
        let source = root.join("source");
        let language = source.join("language");
        std::fs::create_dir_all(&language).expect("a language directory");
        std::fs::write(source.join("rustage"), b"a binary").expect("a binary");
        std::fs::write(language.join("en.json"), b"{}").expect("a pack");
        let mut freeze = Freeze::new();
        freeze.revision = "rev".to_owned();
        freeze.package_version = "0.1.0".to_owned();
        freeze.generator = "0.1.0".to_owned();
        freeze.widget_revision = "widgets".to_owned();
        let directory = root.join("stage");
        stage(&source.join("rustage"), &language, &directory, &freeze).expect("a stage");
        directory
    }

    #[test]
    fn a_blocker_or_critical_defect_blocks_the_release() {
        assert!(Severity::Blocker.blocks_release());
        assert!(Severity::Critical.blocks_release());
        assert!(!Severity::Major.blocks_release());
        assert!(!Severity::Minor.blocks_release());
        for severity in Severity::ALL {
            assert_eq!(Severity::parse(severity.name()), Some(severity));
        }
        assert_eq!(Severity::parse("cosmetic"), None);
        assert_eq!(branches().len(), 3);
        assert!(review_cadence().contains("maintenance release"));
    }

    #[test]
    fn an_upgrade_triggers_only_the_suites_whose_fact_changed() {
        let base = Upgrade {
            widget_revision: "a".to_owned(),
            toolchain: "1.90".to_owned(),
            package_version: "0.1.0".to_owned(),
        };
        assert!(triggers(&base, &base).is_empty());

        let widgets = Upgrade {
            widget_revision: "b".to_owned(),
            ..base.clone()
        };
        assert_eq!(
            triggers(&base, &widgets),
            vec!["widget contract, style, preview and i18n regression"]
        );

        let toolchain = Upgrade {
            toolchain: "1.91".to_owned(),
            ..base.clone()
        };
        assert_eq!(
            triggers(&base, &toolchain),
            vec!["build, packaging and release journey regression"]
        );

        let version = Upgrade {
            package_version: "0.2.0".to_owned(),
            ..base.clone()
        };
        assert_eq!(
            triggers(&base, &version),
            vec!["release freeze, stage, verify and install journey"]
        );
    }

    #[test]
    fn a_drill_rechecks_every_staged_file_and_reports_a_tampered_one() {
        let directory = staged("drill");
        let report = drill(&directory).expect("a passing drill");
        assert_eq!(report.files.len(), 2);
        assert_eq!(report.packs.len(), 1);
        assert_eq!(report.revision, "rev");
        assert!(report.render()[0].contains("2 file(s)"));

        std::fs::write(directory.join("rustage"), b"tampered").expect("a change");
        let failures = drill(&directory).expect_err("a refusal");
        assert!(
            failures
                .iter()
                .any(|failure| failure.contains("does not match its digest")),
            "{failures:?}"
        );
    }
}
