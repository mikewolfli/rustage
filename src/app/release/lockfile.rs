// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Deriving an SBOM from `Cargo.lock` (roadmap task T25.09).
//!
//! The release SBOM was a single hand-filled `rustage` component. That answers "what was
//! released" but not "what is inside it", so this module reads the lockfile that the release was
//! actually built from and turns every locked package into a component.
//!
//! Three facts are kept separate, because a bill of materials that blurs them is worse than none:
//!
//! * **What the lockfile says** — name, version, source and the registry checksum. These are read,
//!   not guessed; a `[[package]]` without a name or version is a refusal with the line number.
//! * **What it does not say** — a licence is not in `Cargo.lock`, and neither is an advisory
//!   database. A package whose licence was not supplied is recorded as `unrecorded`, and the report
//!   says no vulnerability source is configured. Neither is invented.
//! * **How complete the result is** — [`Coverage`] counts packages, digests, recorded licences and
//!   packages with no source (the workspace member), so "derived from the lockfile" is a measured
//!   claim rather than an assertion.
//!
//! A package's identity is its `name@version`, and the component name is the conventional
//! `pkg:cargo/<name>@<version>` form. `Cargo.lock` legitimately contains several versions of one
//! crate (`bitflags` 1 and 2, for instance); keying by the bare name would either lose one of them
//! or refuse the lockfile outright.

use std::collections::BTreeMap;

use super::manifest::{Component, Sbom};

/// One package as the lockfile records it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Locked {
    /// The crate name.
    pub name: String,
    /// The locked version.
    pub version: String,
    /// The lockfile's `source` line, when it has one; `None` for a workspace/path package.
    pub source: Option<String>,
    /// The registry checksum, when the lockfile records one.
    pub checksum: Option<String>,
}

impl Locked {
    /// The package's identity: `name@version`.
    pub fn identity(&self) -> String {
        format!("{}@{}", self.name, self.version)
    }

    /// The SBOM component name: the conventional `pkg:cargo/<name>@<version>` form.
    pub fn component_name(&self) -> String {
        format!("pkg:cargo/{}@{}", self.name, self.version)
    }
}

/// What a derived SBOM covers, and what it could not.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Coverage {
    /// How many packages the lockfile lists.
    pub packages: usize,
    /// How many carry a registry checksum.
    pub with_digest: usize,
    /// How many have a supplied licence.
    pub with_license: usize,
    /// How many have no `source` line (workspace/path packages).
    pub without_source: usize,
}

impl Coverage {
    /// Whether every package has a digest and a recorded licence.
    pub fn is_complete(&self) -> bool {
        self.packages == self.with_digest && self.packages == self.with_license
    }

    /// Packages whose licence was not supplied.
    pub fn unlicensed(&self) -> usize {
        self.packages - self.with_license
    }

    /// The coverage in one line.
    pub fn render(&self) -> String {
        format!(
            "{} package(s): {} with digest, {} with licence ({} unrecorded), {} without source; \
             vulnerabilities: no advisory source configured",
            self.packages,
            self.with_digest,
            self.with_license,
            self.unlicensed(),
            self.without_source
        )
    }
}

/// Reads the `[[package]]` tables out of a `Cargo.lock`.
///
/// Only the fields this module needs are read; `dependencies`, `[metadata]` and `[[patch.unused]]`
/// are skipped because they do not describe a bundled package. A package missing its name or
/// version is refused with the line it starts on: a component with an empty version would sort into
/// the SBOM as if it were real.
pub fn parse(text: &str) -> Result<Vec<Locked>, String> {
    let mut packages: Vec<Locked> = Vec::new();
    let mut current: Option<(usize, Locked)> = None;

    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim();
        let number = index + 1;
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with("[[") {
            if let Some((start, package)) = current.take() {
                packages.push(finish(start, package)?);
            }
            if line == "[[package]]" {
                current = Some((
                    number,
                    Locked {
                        name: String::new(),
                        version: String::new(),
                        source: None,
                        checksum: None,
                    },
                ));
            }
            continue;
        }
        if line.starts_with('[') {
            // A single-bracket table (`[metadata]`) ends the package table it followed.
            if let Some((start, package)) = current.take() {
                packages.push(finish(start, package)?);
            }
            continue;
        }
        let Some((_, package)) = current.as_mut() else {
            continue;
        };
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let Some(value) = quoted(value.trim()) else {
            // `dependencies = [` opens a multi-line array; its contents are not needed and its
            // closing bracket is not a key, so an unquoted value is simply skipped.
            continue;
        };
        match key.trim() {
            "name" => package.name = value,
            "version" => package.version = value,
            "source" => package.source = Some(value),
            "checksum" => package.checksum = Some(value),
            _ => {}
        }
    }
    if let Some((start, package)) = current.take() {
        packages.push(finish(start, package)?);
    }
    Ok(packages)
}

/// Returns a quoted TOML string's contents, or `None` for anything else.
fn quoted(value: &str) -> Option<String> {
    let value = value.split('#').next().unwrap_or(value).trim();
    let inner = value.strip_prefix('"')?.strip_suffix('"')?;
    if inner.contains('"') {
        return None;
    }
    Some(inner.to_owned())
}

/// Checks one package is complete, or reports where it is not.
fn finish(start: usize, package: Locked) -> Result<Locked, String> {
    if package.name.trim().is_empty() {
        return Err(format!("Cargo.lock line {start}: the package has no name"));
    }
    if package.version.trim().is_empty() {
        return Err(format!(
            "Cargo.lock line {start}: package {} has no version",
            package.name
        ));
    }
    Ok(package)
}

/// Builds an SBOM from the locked packages.
///
/// `licences` is keyed by [`Locked::identity`] (`name@version`), because that is the package's
/// identity in a lockfile that may carry two versions of one crate. A package with no entry is
/// recorded as `unrecorded`; the [`Coverage`] it returns counts both, so the gap is visible in the
/// report instead of being mistaken for a recorded licence.
pub fn sbom(packages: &[Locked], licences: &BTreeMap<String, String>) -> (Sbom, Coverage) {
    let mut sbom = Sbom::new();
    let mut coverage = Coverage {
        packages: packages.len(),
        ..Coverage::default()
    };
    for package in packages {
        let licence = licences
            .get(&package.identity())
            .cloned()
            .unwrap_or_else(|| "unrecorded".to_owned());
        if licence != "unrecorded" {
            coverage.with_license += 1;
        }
        if package.checksum.is_some() {
            coverage.with_digest += 1;
        }
        let source = package
            .source
            .clone()
            .unwrap_or_else(|| "path (workspace member)".to_owned());
        if package.source.is_none() {
            coverage.without_source += 1;
        }
        let _ = sbom.add(Component {
            name: package.component_name(),
            version: package.version.clone(),
            license: licence,
            source,
            digest: package.checksum.clone(),
        });
    }
    (sbom, coverage)
}

#[cfg(test)]
mod tests {
    use super::{Coverage, Locked, parse, sbom};
    use std::collections::BTreeMap;

    const LOCK: &str = r#"# This file is automatically @generated by Cargo.
version = 4

[[package]]
name = "adler2"
version = "2.0.1"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "320119579fcad9c21884f5c4861d16174d0e06250625266f50fe6898340abefa"

[[package]]
name = "bitflags"
version = "1.3.2"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "bef38d45163c2f1dde094a7dfd33ccf595c92905c8f8f4fdc18d06fb1037718a"
dependencies = [
 "other",
]

[[package]]
name = "bitflags"
version = "2.9.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "5c8214115b7da9133081b800a42c6d57a0e46ffa5e0bb3d6a9a7d5a7c3c7b1e0"

[[package]]
name = "rustage"
version = "0.1.0"
dependencies = [
 "adler2",
]

[metadata]
"checksum adler2 2.0.1 (registry+https://github.com/rust-lang/crates.io-index)" = "320119579fcad9c21884f5c4861d16174d0e06250625266f50fe6898340abefa"
"#;

    #[test]
    fn every_package_is_read_with_its_source_and_digest() {
        let packages = parse(LOCK).expect("a lockfile");
        assert_eq!(packages.len(), 4);
        assert_eq!(packages[0].name, "adler2");
        assert_eq!(packages[0].version, "2.0.1");
        assert!(packages[0].source.is_some());
        assert!(packages[0].checksum.is_some());
        // A workspace member has no source and no checksum, and that is recorded as absent.
        let root = &packages[3];
        assert_eq!(root.name, "rustage");
        assert_eq!(root.source, None);
        assert_eq!(root.checksum, None);
    }

    #[test]
    fn two_versions_of_one_crate_keep_distinct_identities() {
        let packages = parse(LOCK).expect("a lockfile");
        let names: Vec<String> = packages.iter().map(Locked::component_name).collect();
        assert!(names.contains(&"pkg:cargo/bitflags@1.3.2".to_owned()));
        assert!(names.contains(&"pkg:cargo/bitflags@2.9.0".to_owned()));
        assert_eq!(packages[2].identity(), "bitflags@2.9.0");
    }

    #[test]
    fn a_package_without_a_version_is_refused_with_its_line() {
        let text = "version = 4\n\n[[package]]\nname = \"broken\"\n";
        let reason = parse(text).expect_err("no version");
        assert!(reason.contains("line 3"), "{reason}");
        assert!(reason.contains("broken"), "{reason}");
    }

    #[test]
    fn the_sbom_records_a_missing_licence_as_unrecorded_and_counts_coverage() {
        let packages = parse(LOCK).expect("a lockfile");
        let mut licences = BTreeMap::new();
        licences.insert(
            "adler2@2.0.1".to_owned(),
            "0BSD OR MIT OR Apache-2.0".to_owned(),
        );
        licences.insert("rustage@0.1.0".to_owned(), "MIT".to_owned());
        let (sbom, coverage) = sbom(&packages, &licences);

        assert_eq!(
            sbom.components["pkg:cargo/adler2@2.0.1"].license,
            "0BSD OR MIT OR Apache-2.0"
        );
        assert_eq!(
            sbom.components["pkg:cargo/bitflags@1.3.2"].license,
            "unrecorded"
        );
        assert_eq!(
            sbom.components["pkg:cargo/rustage@0.1.0"].source,
            "path (workspace member)"
        );
        assert_eq!(
            coverage,
            Coverage {
                packages: 4,
                with_digest: 3,
                with_license: 2,
                without_source: 1,
            }
        );
        assert_eq!(coverage.unlicensed(), 2);
        assert!(!coverage.is_complete());
        assert!(coverage.render().contains("no advisory source configured"));
    }

    #[test]
    fn an_empty_lockfile_derives_an_empty_sbom() {
        let packages = parse("version = 4\n").expect("a lockfile");
        assert!(packages.is_empty());
        let (sbom, coverage) = sbom(&packages, &BTreeMap::new());
        assert!(sbom.names().is_empty());
        assert_eq!(coverage.packages, 0);
        assert!(coverage.is_complete());
    }
}
