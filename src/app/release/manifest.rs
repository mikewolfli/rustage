// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Release freeze, artifacts, SBOM and updates (task F25).
//!
//! A release is a set of names, revisions and digests, not a directory that happens to build. The
//! freeze names the revision, generator, formats and support-matrix revision; an artifact carries
//! its SHA-256 and signature state; the SBOM lists every component with its source and licence. An
//! update verifies the artifact before touching a backup, and a failed verification leaves the old
//! installation in place.
//!
//! [`Freeze::verify_against`] is the part that makes the freeze real: every fact it names is
//! compared with the checkout (`Cargo.toml`, `Cargo.lock`, `docs/plan/support-matrix.json` and the
//! commit), not merely checked for being non-empty. A support-matrix cell the matrix itself marks
//! unsupported or unverified stays blocked; it can never move into the verified list.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use sha2::{Digest, Sha256};

use crate::contracts::revision::{FormatVersion, GeneratorVersion};

/// The release channel.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Channel {
    /// A stable release.
    Stable,
    /// A release candidate.
    Candidate,
}

impl Channel {
    /// The channel's name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Candidate => "candidate",
        }
    }

    /// Reads a channel name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "stable" => Some(Self::Stable),
            "candidate" => Some(Self::Candidate),
            _ => None,
        }
    }
}

/// Whether an artifact is signed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Signature {
    /// No signature was made.
    Unsigned,
    /// A signature was made and verified.
    Verified,
}

impl Signature {
    /// The state's name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Unsigned => "unsigned",
            Self::Verified => "verified",
        }
    }
}

/// One frozen input of a release.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Freeze {
    /// The product revision (the commit the release is cut from).
    pub revision: String,
    /// The `rustage` package version.
    pub package_version: String,
    /// The generator version.
    pub generator: String,
    /// The artifact formats.
    pub formats: Vec<String>,
    /// The document format this build reads and writes.
    pub document_format: FormatVersion,
    /// The logic protocol this build speaks.
    pub protocol_version: u32,
    /// The pinned `rust_widgets` commit.
    pub widget_revision: String,
    /// The support-matrix revision.
    pub support_matrix: String,
    /// The SHA-256 of the support-matrix file.
    pub support_matrix_digest: String,
    /// Component versions by name.
    pub components: BTreeMap<String, String>,
}

impl Freeze {
    /// An empty freeze, except for the format and protocol this build contains.
    pub fn new() -> Self {
        Self {
            revision: String::new(),
            package_version: String::new(),
            generator: String::new(),
            formats: Vec::new(),
            document_format: FormatVersion::CURRENT,
            protocol_version: crate::app::logic::ipc::Protocol::VERSION,
            widget_revision: String::new(),
            support_matrix: String::new(),
            support_matrix_digest: String::new(),
            components: BTreeMap::new(),
        }
    }

    /// Checks that every frozen fact is present.
    pub fn validate(&self) -> Result<(), String> {
        if self.revision.trim().is_empty() {
            return Err("the freeze has no revision".to_owned());
        }
        if self.generator.trim().is_empty() {
            return Err("the freeze has no generator version".to_owned());
        }
        if self.formats.is_empty() {
            return Err("the freeze names no artifact format".to_owned());
        }
        if self.support_matrix.trim().is_empty() {
            return Err("the freeze names no support-matrix revision".to_owned());
        }
        if self.components.is_empty() {
            return Err("the freeze names no component versions".to_owned());
        }
        Ok(())
    }

    /// Checks every frozen fact against the repository that is about to build the release.
    ///
    /// A fact is verified only when the frozen value equals the checkout's value. A support-matrix
    /// cell whose own status is `unsupported` or `unverified` is reported as blocked and is never
    /// placed among the verified items; a supported cell without a reason is a failure because the
    /// evidence the roadmap asks for is missing.
    pub fn verify_against(&self, repository: &Repository) -> FreezeReport {
        let mut report = FreezeReport::default();

        if self.revision == repository.revision {
            report.verified.push(format!("revision {}", self.revision));
        } else {
            report.failed.push(format!(
                "revision is {:?} but the repository is at {:?}",
                self.revision, repository.revision
            ));
        }

        if self.package_version.is_empty() {
            report
                .failed
                .push("the freeze names no package version".to_owned());
        } else if self.package_version == repository.package_version {
            report
                .verified
                .push(format!("package {}", self.package_version));
        } else {
            report.failed.push(format!(
                "package is {:?} but Cargo.toml says {:?}",
                self.package_version, repository.package_version
            ));
        }

        let generator = GeneratorVersion::CURRENT;
        let current_generator = format!(
            "{}.{}.{}",
            generator.major, generator.minor, generator.patch
        );
        if self.generator.is_empty() {
            report
                .failed
                .push("the freeze names no generator version".to_owned());
        } else if self.generator == current_generator {
            report
                .verified
                .push(format!("generator {}", self.generator));
        } else {
            report.failed.push(format!(
                "generator is {:?} but this build contains {:?}",
                self.generator, current_generator
            ));
        }

        if self.document_format.get() == repository.document_format {
            report
                .verified
                .push(format!("document format v{}", self.document_format.get()));
        } else {
            report.failed.push(format!(
                "document format is v{} but this build writes v{}",
                self.document_format.get(),
                repository.document_format
            ));
        }

        if self.protocol_version == repository.protocol_version {
            report
                .verified
                .push(format!("protocol {}", self.protocol_version));
        } else {
            report.failed.push(format!(
                "protocol is {} but this build speaks {}",
                self.protocol_version, repository.protocol_version
            ));
        }

        if self.widget_revision.is_empty() {
            report
                .failed
                .push("the freeze names no rust_widgets revision".to_owned());
        } else if self.widget_revision == repository.widget_revision {
            report
                .verified
                .push(format!("rust_widgets revision {}", self.widget_revision));
        } else {
            report.failed.push(format!(
                "rust_widgets revision is {:?} but Cargo.lock resolves {:?}",
                self.widget_revision, repository.widget_revision
            ));
        }

        if !self.components.contains_key("rust_widgets") {
            report
                .failed
                .push("the freeze names no rust_widgets component version".to_owned());
        }
        for (name, version) in &self.components {
            match repository.packages.get(name) {
                Some(locked) if locked == version => {
                    report
                        .verified
                        .push(format!("component {name} {version}"));
                }
                Some(locked) => report.failed.push(format!(
                    "component {name} is {version} but Cargo.lock resolves {locked}"
                )),
                None => report.failed.push(format!(
                    "component {name} {version} is not in Cargo.lock, so there is no evidence for it"
                )),
            }
        }

        if self.support_matrix.is_empty() {
            report
                .failed
                .push("the freeze names no support-matrix revision".to_owned());
        } else if self.support_matrix == repository.support_matrix_revision {
            report
                .verified
                .push(format!("support matrix revision {}", self.support_matrix));
        } else {
            report.failed.push(format!(
                "support matrix revision is {:?} but the matrix names {:?}",
                self.support_matrix, repository.support_matrix_revision
            ));
        }

        if self.support_matrix_digest.is_empty() {
            report
                .failed
                .push("the freeze names no support-matrix digest".to_owned());
        } else if self.support_matrix_digest == repository.support_matrix_digest {
            report.verified.push(format!(
                "support matrix digest {}",
                self.support_matrix_digest
            ));
        } else {
            report.failed.push(format!(
                "support matrix digest is {:?} but the file has {:?}",
                self.support_matrix_digest, repository.support_matrix_digest
            ));
        }

        for cell in &repository.cells {
            let label = format!("support {}/{}", cell.host, cell.profile);
            match cell.status.as_str() {
                "certified" | "build-verified" => {
                    if cell.reason.trim().is_empty() {
                        report.failed.push(format!(
                            "{label} is {} but carries no evidence",
                            cell.status
                        ));
                    } else {
                        report
                            .verified
                            .push(format!("{label} {}: {}", cell.status, cell.reason));
                    }
                }
                "unsupported" | "unverified" => report
                    .blocked
                    .push(format!("{label} {}: {}", cell.status, cell.reason)),
                other => report
                    .failed
                    .push(format!("{label} has unknown status {other:?}")),
            }
        }

        // The per-fact checks above cover the repository-backed facts. The artifact formats are
        // not a repository fact until T25.02 builds them, but a freeze that otherwise matches
        // still has to name one; `validate` is the remaining completeness answer.
        if report.failed.is_empty()
            && let Err(reason) = self.validate()
        {
            report.failed.push(reason);
        }

        report
    }

    /// The freeze in one line per fact.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![
            format!("revision {}", self.revision),
            format!("package {}", self.package_version),
            format!("generator {}", self.generator),
            format!("document format v{}", self.document_format.get()),
            format!("protocol {}", self.protocol_version),
            format!("rust_widgets revision {}", self.widget_revision),
            format!("support matrix {}", self.support_matrix),
            format!("support matrix digest {}", self.support_matrix_digest),
            format!("formats {}", self.formats.join(", ")),
        ];
        for (name, version) in &self.components {
            lines.push(format!("component {name} {version}"));
        }
        lines
    }
}

impl Default for Freeze {
    fn default() -> Self {
        Self::new()
    }
}

/// The result of checking a freeze against the repository.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct FreezeReport {
    /// Facts whose frozen value equals the repository's value.
    pub verified: Vec<String>,
    /// Facts that the repository contradicts or that cannot be evidenced.
    pub failed: Vec<String>,
    /// Matrix cells the repository itself marks unsupported or unverified.
    pub blocked: Vec<String>,
}

impl FreezeReport {
    /// Whether every frozen fact matches the repository.
    pub fn passed(&self) -> bool {
        self.failed.is_empty()
    }

    /// One line per item, labels first so a reader can scan the failures.
    pub fn render(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for item in &self.verified {
            lines.push(format!("verified: {item}"));
        }
        for item in &self.blocked {
            lines.push(format!("blocked: {item}"));
        }
        for item in &self.failed {
            lines.push(format!("failed: {item}"));
        }
        lines
    }
}

/// What the checkout says about the release inputs.
///
/// Every field is read from a file or a build constant; nothing here is supplied by the freeze, so
/// comparing the two is a real check rather than a value compared with itself.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Repository {
    /// The commit `HEAD` resolves to.
    pub revision: String,
    /// The `rustage` package version in `Cargo.toml`.
    pub package_version: String,
    /// The document format this build writes.
    pub document_format: u32,
    /// The logic protocol this build speaks.
    pub protocol_version: u32,
    /// The `rust_widgets` version in `Cargo.lock`.
    pub widget_version: String,
    /// The `rust_widgets` git revision in `Cargo.lock`.
    pub widget_revision: String,
    /// Every package version `Cargo.lock` resolves.
    pub packages: BTreeMap<String, String>,
    /// The dependency revision the support matrix names.
    pub support_matrix_revision: String,
    /// The SHA-256 of the support-matrix file.
    pub support_matrix_digest: String,
    /// The support matrix's cells.
    pub cells: Vec<SupportCell>,
}

impl Repository {
    /// Reads the release facts from the checkout at `root`.
    pub fn read(root: &Path) -> Result<Self, String> {
        let manifest_path = root.join("Cargo.toml");
        let manifest = fs::read_to_string(&manifest_path)
            .map_err(|error| format!("cannot read {}: {error}", manifest_path.display()))?;
        let package_version = manifest_version(&manifest)?;
        if package_version != env!("CARGO_PKG_VERSION") {
            return Err(format!(
                "Cargo.toml names package version {package_version} but this binary was built as {}; run freeze-check from the release checkout",
                env!("CARGO_PKG_VERSION")
            ));
        }

        let lock_path = root.join("Cargo.lock");
        let lock = fs::read_to_string(&lock_path)
            .map_err(|error| format!("cannot read {}: {error}", lock_path.display()))?;
        let locked = locked_packages(&lock)?;
        let widget = locked.get("rust_widgets").ok_or_else(|| {
            "Cargo.lock has no rust_widgets package, so its revision cannot be verified".to_owned()
        })?;
        let source = widget.source.as_deref().ok_or_else(|| {
            "Cargo.lock records no source for rust_widgets, so its revision cannot be verified"
                .to_owned()
        })?;
        let widget_revision = source_revision(source)
            .ok_or_else(|| format!("the rust_widgets source {source:?} names no git revision"))?;
        let widget_version = widget.version.clone();

        let matrix_path = root.join("docs/plan/support-matrix.json");
        let matrix_bytes = fs::read(&matrix_path)
            .map_err(|error| format!("cannot read {}: {error}", matrix_path.display()))?;
        let matrix_text = std::str::from_utf8(&matrix_bytes)
            .map_err(|error| format!("{} is not text: {error}", matrix_path.display()))?;
        let matrix = crate::project::record::json::parse(matrix_text)
            .map_err(|error| format!("{}: {error}", matrix_path.display()))?;
        let matrix_object = matrix
            .as_object()
            .ok_or_else(|| format!("{} is not a JSON object", matrix_path.display()))?;
        let dependency = matrix_object
            .get("dependency")
            .and_then(crate::project::record::json::Json::as_object)
            .ok_or_else(|| "the support matrix has no dependency object".to_owned())?;
        let crate_name = dependency
            .get("crate")
            .and_then(crate::project::record::json::Json::as_str)
            .ok_or_else(|| "the support matrix names no dependency crate".to_owned())?;
        if crate_name != "rust_widgets" {
            return Err(format!(
                "the support matrix describes {crate_name:?}, not rust_widgets"
            ));
        }
        let support_matrix_revision = dependency
            .get("revision")
            .and_then(crate::project::record::json::Json::as_str)
            .ok_or_else(|| "the support matrix names no dependency revision".to_owned())?
            .to_owned();
        if support_matrix_revision != widget_revision {
            return Err(format!(
                "the support matrix names rust_widgets revision {support_matrix_revision} but Cargo.lock resolves {widget_revision}"
            ));
        }
        let cells = matrix_object
            .get("cells")
            .and_then(crate::project::record::json::Json::as_array)
            .ok_or_else(|| "the support matrix has no cells".to_owned())?;
        let mut parsed = Vec::with_capacity(cells.len());
        for cell in cells {
            parsed.push(SupportCell::read(cell)?);
        }
        let support_matrix_digest = sha256_hex(&matrix_bytes);

        Ok(Self {
            revision: head_revision(root)?,
            package_version,
            document_format: FormatVersion::CURRENT.get(),
            protocol_version: crate::app::logic::ipc::Protocol::VERSION,
            widget_version,
            widget_revision,
            packages: locked
                .into_iter()
                .map(|(name, package)| (name, package.version))
                .collect(),
            support_matrix_revision,
            support_matrix_digest,
            cells: parsed,
        })
    }
}

/// One support-matrix cell as the matrix states it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SupportCell {
    /// The host id.
    pub host: String,
    /// The profile id.
    pub profile: String,
    /// `certified`, `build-verified`, `unverified` or `unsupported`.
    pub status: String,
    /// The recorded evidence, or the recorded reason there is none.
    pub reason: String,
}

impl SupportCell {
    fn read(value: &crate::project::record::json::Json) -> Result<Self, String> {
        let object = value
            .as_object()
            .ok_or_else(|| "a support-matrix cell is not an object".to_owned())?;
        let text = |name: &str| {
            object
                .get(name)
                .and_then(crate::project::record::json::Json::as_str)
                .map(str::to_owned)
                .ok_or_else(|| format!("a support-matrix cell has no {name}"))
        };
        Ok(Self {
            host: text("host")?,
            profile: text("profile")?,
            status: text("status")?,
            reason: text("reason")?,
        })
    }
}

/// A package as `Cargo.lock` records it.
struct LockedPackage {
    version: String,
    source: Option<String>,
}

fn manifest_version(text: &str) -> Result<String, String> {
    let mut in_package = false;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        if !in_package {
            continue;
        }
        let Some(rest) = line.strip_prefix("version") else {
            continue;
        };
        let Some(rest) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        let version = unquote(rest);
        if !version.is_empty() {
            return Ok(version);
        }
    }
    Err("Cargo.toml has no package version".to_owned())
}

fn locked_packages(text: &str) -> Result<BTreeMap<String, LockedPackage>, String> {
    let mut packages = BTreeMap::new();
    for block in text.split("[[package]]").skip(1) {
        let mut name = None;
        let mut version = None;
        let mut source = None;
        for line in block.lines() {
            let line = line.trim();
            if let Some(value) = line.strip_prefix("name = ") {
                name = Some(unquote(value));
            } else if let Some(value) = line.strip_prefix("version = ") {
                version = Some(unquote(value));
            } else if let Some(value) = line.strip_prefix("source = ") {
                source = Some(unquote(value));
            }
        }
        if let (Some(name), Some(version)) = (name, version) {
            packages.insert(name, LockedPackage { version, source });
        }
    }
    if packages.is_empty() {
        return Err("Cargo.lock lists no package".to_owned());
    }
    Ok(packages)
}

fn source_revision(source: &str) -> Option<String> {
    if let Some((_, fragment)) = source.split_once('#')
        && !fragment.is_empty()
    {
        return Some(fragment.to_owned());
    }
    let (_, query) = source.split_once('?')?;
    query
        .split('&')
        .find_map(|pair| pair.strip_prefix("rev=").map(str::to_owned))
}

fn head_revision(root: &Path) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "HEAD"])
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("cannot run git: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git rev-parse HEAD failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let revision = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if revision.is_empty() {
        return Err("git named no commit for HEAD".to_owned());
    }
    Ok(revision)
}

fn unquote(value: &str) -> String {
    value.trim().trim_matches('"').to_owned()
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// One deliverable.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Artifact {
    /// Its file name.
    pub name: String,
    /// Its SHA-256, as published.
    pub digest: String,
    /// Its size in bytes.
    pub size: u64,
    /// Whether it is signed.
    pub signature: Signature,
    /// The licences it carries.
    pub licenses: Vec<String>,
}

impl Artifact {
    /// Checks the artifact's digest and size against actual bytes.
    pub fn verify(&self, bytes: &[u8]) -> Result<(), String> {
        let actual = sha256_hex(bytes);
        if actual != self.digest {
            return Err(format!(
                "{} has digest {actual}, not {}",
                self.name, self.digest
            ));
        }
        if bytes.len() as u64 != self.size {
            return Err(format!(
                "{} has {} byte(s), not {}",
                self.name,
                bytes.len(),
                self.size
            ));
        }
        Ok(())
    }
}

/// One SBOM component.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Component {
    /// Its name.
    pub name: String,
    /// Its version.
    pub version: String,
    /// Its licence expression.
    pub license: String,
    /// Where it came from (registry, git URL, path).
    pub source: String,
    /// Its digest, when one is known.
    pub digest: Option<String>,
}

/// A software bill of materials.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Sbom {
    /// The components, by name.
    pub components: BTreeMap<String, Component>,
}

impl Sbom {
    /// An empty SBOM.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a component, refusing a name already present with different facts.
    pub fn add(&mut self, component: Component) -> Result<(), String> {
        if component.name.trim().is_empty()
            || component.version.trim().is_empty()
            || component.license.trim().is_empty()
            || component.source.trim().is_empty()
        {
            return Err("an SBOM component needs a name, version, licence and source".to_owned());
        }
        if let Some(existing) = self.components.get(&component.name)
            && existing != &component
        {
            return Err(format!(
                "component {} is listed twice with different facts",
                component.name
            ));
        }
        self.components.insert(component.name.clone(), component);
        Ok(())
    }

    /// The SBOM as JSON.
    pub fn json(&self) -> String {
        let components: Vec<crate::project::record::json::Json> = self
            .components
            .values()
            .map(|component| {
                let mut object = crate::project::record::json::JsonObject::new();
                object.insert(
                    "name",
                    crate::project::record::json::Json::string(&component.name),
                );
                object.insert(
                    "version",
                    crate::project::record::json::Json::string(&component.version),
                );
                object.insert(
                    "license",
                    crate::project::record::json::Json::string(&component.license),
                );
                object.insert(
                    "source",
                    crate::project::record::json::Json::string(&component.source),
                );
                if let Some(digest) = &component.digest {
                    object.insert("digest", crate::project::record::json::Json::string(digest));
                }
                crate::project::record::json::Json::Object(object)
            })
            .collect();
        let mut root = crate::project::record::json::JsonObject::new();
        root.insert(
            "components",
            crate::project::record::json::Json::Array(components),
        );
        crate::project::record::json::Json::Object(root).render()
    }

    /// The names in order.
    pub fn names(&self) -> Vec<&str> {
        self.components.keys().map(String::as_str).collect()
    }
}

/// An update's state.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UpdateState {
    /// Nothing has been done.
    Planned,
    /// The artifact was verified.
    Verified,
    /// The backup was made and the new files installed.
    Installed,
    /// The installation was rolled back.
    RolledBack,
}

/// An update plan for one artifact.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Update {
    /// The version installed now.
    pub from: String,
    /// The version offered.
    pub to: String,
    /// The artifact.
    pub artifact: Artifact,
    /// The state.
    pub state: UpdateState,
}

impl Update {
    /// Plans an update; a same-or-older version is refused.
    pub fn plan(
        from: impl Into<String>,
        to: impl Into<String>,
        artifact: Artifact,
    ) -> Result<Self, String> {
        let from = from.into();
        let to = to.into();
        match compare_versions(&to, &from)? {
            Ordering::Greater => {}
            Ordering::Equal => return Err(format!("{to} is already installed")),
            Ordering::Less => {
                return Err(format!(
                    "refusing to downgrade from {from} to {to}: updates must move forward"
                ));
            }
        }
        if artifact.digest.len() != 64 {
            return Err("the update artifact has no SHA-256 digest".to_owned());
        }
        Ok(Self {
            from,
            to,
            artifact,
            state: UpdateState::Planned,
        })
    }

    /// Verifies the downloaded bytes; a failure leaves the state planned.
    pub fn verify(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.artifact.verify(bytes)?;
        self.state = UpdateState::Verified;
        Ok(())
    }

    /// Installs after verification, making a backup name first.
    pub fn install(&mut self) -> Result<String, String> {
        if self.state != UpdateState::Verified {
            return Err("the update was not verified".to_owned());
        }
        self.state = UpdateState::Installed;
        Ok(format!("backup rustage-{}.bak", self.from))
    }

    /// Rolls back an installed update.
    pub fn rollback(&mut self) -> Result<(), String> {
        if self.state != UpdateState::Installed {
            return Err("there is no installed update to roll back".to_owned());
        }
        self.state = UpdateState::RolledBack;
        Ok(())
    }

    /// The update in one line.
    pub fn render(&self) -> String {
        format!(
            "{} -> {} ({}, {}, {:?})",
            self.from,
            self.to,
            self.artifact.name,
            self.artifact.signature.name(),
            self.state
        )
    }
}

/// Compares two dotted numeric versions.
///
/// An update may not move backwards, and a string comparison would order `1.10.0` before `1.9.0`.
/// The parser is deliberately small: three or more numeric components, with an optional
/// `-prerelease` suffix. A version that does not parse is refused rather than silently treated as
/// newer, because guessing here could install an older build over a newer one.
fn compare_versions(left: &str, right: &str) -> Result<Ordering, String> {
    let (left_parts, left_pre) = parse_version(left)?;
    let (right_parts, right_pre) = parse_version(right)?;
    let order = left_parts.cmp(&right_parts);
    if order != Ordering::Equal {
        return Ok(order);
    }
    Ok(match (left_pre, right_pre) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(left), Some(right)) => left.cmp(&right),
    })
}

fn parse_version(version: &str) -> Result<(Vec<u64>, Option<String>), String> {
    let (core, prerelease) = match version.split_once('-') {
        Some((core, prerelease)) => (core, Some(prerelease.to_owned())),
        None => (version, None),
    };
    let mut parts = Vec::new();
    for part in core.split('.') {
        let value = part
            .parse::<u64>()
            .map_err(|_| format!("{version:?} is not a numeric version"))?;
        parts.push(value);
    }
    if parts.len() < 2 {
        return Err(format!("{version:?} needs at least major.minor"));
    }
    Ok((parts, prerelease))
}

/// A maintained branch's regression scope.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Maintenance {
    /// The branch name.
    pub branch: String,
    /// The support-matrix revision it serves.
    pub support_matrix: String,
    /// The known limits this branch keeps.
    pub limits: BTreeSet<String>,
}

impl Maintenance {
    /// Checks the branch names its support matrix.
    pub fn validate(&self) -> Result<(), String> {
        if self.branch.trim().is_empty() || self.support_matrix.trim().is_empty() {
            return Err(
                "a maintenance branch needs a name and a support-matrix revision".to_owned(),
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Artifact, Channel, Component, Freeze, Maintenance, Repository, Sbom, Signature,
        SupportCell, Update, UpdateState, locked_packages, manifest_version, source_revision,
    };
    use std::collections::{BTreeMap, BTreeSet};

    use sha2::{Digest, Sha256};

    use crate::contracts::revision::{FormatVersion, GeneratorVersion};

    fn repository() -> Repository {
        Repository {
            revision: "abc123".to_owned(),
            package_version: env!("CARGO_PKG_VERSION").to_owned(),
            document_format: FormatVersion::CURRENT.get(),
            protocol_version: crate::app::logic::ipc::Protocol::VERSION,
            widget_version: "3.0.0".to_owned(),
            widget_revision: "177716410bc714cb0b1131d0c8f7065076f10583".to_owned(),
            packages: BTreeMap::from([
                ("rust_widgets".to_owned(), "3.0.0".to_owned()),
                ("sha2".to_owned(), "0.10.9".to_owned()),
            ]),
            support_matrix_revision: "177716410bc714cb0b1131d0c8f7065076f10583".to_owned(),
            support_matrix_digest: "matrix-digest".to_owned(),
            cells: vec![
                SupportCell {
                    host: "macos".to_owned(),
                    profile: "desktop".to_owned(),
                    status: "build-verified".to_owned(),
                    reason: "docs/log/log-20261029-1.md".to_owned(),
                },
                SupportCell {
                    host: "linux".to_owned(),
                    profile: "desktop".to_owned(),
                    status: "unverified".to_owned(),
                    reason: "3.0.0 has not been rebuilt on Linux".to_owned(),
                },
            ],
        }
    }

    fn matching_freeze() -> Freeze {
        let generator = GeneratorVersion::CURRENT;
        let mut freeze = Freeze::new();
        freeze.revision = "abc123".to_owned();
        freeze.package_version = env!("CARGO_PKG_VERSION").to_owned();
        freeze.generator = format!(
            "{}.{}.{}",
            generator.major, generator.minor, generator.patch
        );
        freeze.formats = vec!["zip".to_owned()];
        freeze.widget_revision = "177716410bc714cb0b1131d0c8f7065076f10583".to_owned();
        freeze.support_matrix = "177716410bc714cb0b1131d0c8f7065076f10583".to_owned();
        freeze.support_matrix_digest = "matrix-digest".to_owned();
        freeze
            .components
            .insert("rust_widgets".to_owned(), "3.0.0".to_owned());
        freeze
            .components
            .insert("sha2".to_owned(), "0.10.9".to_owned());
        freeze
    }

    fn artifact(bytes: &[u8]) -> Artifact {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        let digest = hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Artifact {
            name: "rustage.zip".to_owned(),
            digest,
            size: bytes.len() as u64,
            signature: Signature::Verified,
            licenses: vec!["MIT".to_owned()],
        }
    }

    #[test]
    fn a_freeze_needs_every_frozen_fact() {
        let mut freeze = Freeze::new();
        assert!(freeze.validate().is_err());
        freeze.revision = "abc".to_owned();
        freeze.generator = "1.0.0".to_owned();
        freeze.formats = vec!["zip".to_owned()];
        freeze.support_matrix = "matrix-1".to_owned();
        freeze
            .components
            .insert("rust".to_owned(), "1.98.1".to_owned());
        freeze.validate().expect("a complete freeze");
    }

    #[test]
    fn a_matching_freeze_is_verified_against_the_repository() {
        let report = matching_freeze().verify_against(&repository());
        assert!(report.passed(), "failures: {:?}", report.failed);
        assert!(
            report
                .verified
                .iter()
                .any(|line| line.starts_with("rust_widgets revision ")),
            "the pinned library revision is compared with Cargo.lock: {:?}",
            report.verified
        );
        assert!(
            report
                .verified
                .iter()
                .any(|line| line.starts_with("support matrix digest ")),
            "the support-matrix content digest is compared: {:?}",
            report.verified
        );
    }

    #[test]
    fn a_matching_freeze_without_an_artifact_format_fails() {
        let mut freeze = matching_freeze();
        freeze.formats.clear();
        let report = freeze.verify_against(&repository());
        assert!(!report.passed());
        assert!(
            report
                .failed
                .iter()
                .any(|line| line.contains("no artifact format")),
            "failures: {:?}",
            report.failed
        );
    }

    #[test]
    fn a_wrong_library_revision_fails() {
        let mut freeze = matching_freeze();
        freeze.widget_revision = "0000000000000000000000000000000000000000".to_owned();
        let report = freeze.verify_against(&repository());
        assert!(!report.passed());
        assert!(
            report
                .failed
                .iter()
                .any(|line| line.contains("rust_widgets revision")),
            "failures: {:?}",
            report.failed
        );
    }

    #[test]
    fn a_wrong_support_matrix_digest_or_revision_fails() {
        let mut wrong_digest = matching_freeze();
        wrong_digest.support_matrix_digest = "some-other-digest".to_owned();
        let report = wrong_digest.verify_against(&repository());
        assert!(!report.passed());
        assert!(
            report
                .failed
                .iter()
                .any(|line| line.contains("support matrix digest")),
            "failures: {:?}",
            report.failed
        );

        let mut wrong_revision = matching_freeze();
        wrong_revision.support_matrix = "old-revision".to_owned();
        let report = wrong_revision.verify_against(&repository());
        assert!(!report.passed());
        assert!(
            report
                .failed
                .iter()
                .any(|line| line.contains("support matrix revision")),
            "failures: {:?}",
            report.failed
        );
    }

    #[test]
    fn a_blocked_support_cell_is_not_reported_as_passed() {
        let report = matching_freeze().verify_against(&repository());
        assert!(
            report.passed(),
            "blocked cells do not fail the freeze by themselves"
        );
        assert!(
            report
                .blocked
                .iter()
                .any(|line| line.starts_with("support linux/desktop unverified")),
            "blocked: {:?}",
            report.blocked
        );
        assert!(
            !report
                .verified
                .iter()
                .any(|line| line.starts_with("support linux/desktop")),
            "an unverified cell must never move into the verified list: {:?}",
            report.verified
        );
    }

    #[test]
    fn a_supported_cell_without_evidence_fails() {
        let mut repository = repository();
        repository.cells[0].reason.clear();
        let report = matching_freeze().verify_against(&repository);
        assert!(!report.passed());
        assert!(
            report
                .failed
                .iter()
                .any(|line| line.contains("carries no evidence")),
            "failures: {:?}",
            report.failed
        );
    }

    #[test]
    fn a_component_absent_from_the_lock_fails() {
        let mut freeze = matching_freeze();
        freeze
            .components
            .insert("not-a-locked-crate".to_owned(), "1.0.0".to_owned());
        let report = freeze.verify_against(&repository());
        assert!(!report.passed());
        assert!(
            report
                .failed
                .iter()
                .any(|line| line.contains("not-a-locked-crate")),
            "failures: {:?}",
            report.failed
        );
    }

    #[test]
    fn repository_files_are_read_without_guessing() {
        let manifest = "[package]\nname = \"rustage\"\nversion = \"0.1.0\"\n";
        assert_eq!(
            manifest_version(manifest).expect("a package version"),
            "0.1.0"
        );

        let lock = "[[package]]\nname = \"rust_widgets\"\nversion = \"3.0.0\"\nsource = \"git+https://github.com/mikewolfli/rust-widgets?rev=abc#def\"\n\n[[package]]\nname = \"sha2\"\nversion = \"0.10.9\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\n";
        let packages = locked_packages(lock).expect("locked packages");
        assert_eq!(packages["rust_widgets"].version, "3.0.0");
        let source = packages["rust_widgets"]
            .source
            .as_deref()
            .expect("a git source");
        assert_eq!(source_revision(source).as_deref(), Some("def"));
        assert_eq!(
            source_revision("git+https://example.invalid/repo?rev=abc123").as_deref(),
            Some("abc123")
        );
        assert_eq!(source_revision("registry+https://example.invalid"), None);
    }

    #[test]
    fn an_artifact_digest_and_size_are_checked() {
        let bytes = b"release bytes";
        let artifact = artifact(bytes);
        artifact.verify(bytes).expect("a match");
        assert!(artifact.verify(b"other").is_err());
    }

    #[test]
    fn an_sbom_refuses_a_duplicate_with_different_facts() {
        let mut sbom = Sbom::new();
        let component = Component {
            name: "rust-widgets".to_owned(),
            version: "3.0.0".to_owned(),
            license: "MIT".to_owned(),
            source: "git".to_owned(),
            digest: None,
        };
        sbom.add(component.clone()).expect("an add");
        sbom.add(component.clone()).expect("an identical add");
        let mut other = component;
        other.version = "2.9.4".to_owned();
        assert!(sbom.add(other).is_err());
        assert!(sbom.json().contains("rust-widgets"));
    }

    #[test]
    fn an_update_verifies_before_installing_and_can_roll_back() {
        let bytes = b"new release";
        let artifact = artifact(bytes);
        let mut update = Update::plan("1.0.0", "1.0.1", artifact).expect("a plan");
        assert!(update.install().is_err());
        update.verify(bytes).expect("a verify");
        assert_eq!(update.state, UpdateState::Verified);
        assert!(update.install().is_ok());
        update.rollback().expect("a rollback");
        assert_eq!(update.state, UpdateState::RolledBack);
    }

    #[test]
    fn the_same_version_cannot_be_planned_as_an_update() {
        let artifact = artifact(b"x");
        assert!(Update::plan("1.0.0", "1.0.0", artifact.clone()).is_err());
    }

    #[test]
    fn a_downgrade_is_refused_and_a_numeric_upgrade_is_accepted() {
        let bytes = artifact(b"x");
        assert!(Update::plan("1.10.0", "1.9.0", bytes.clone()).is_err());
        assert!(Update::plan("1.0.0", "1.10.0", bytes).is_ok());
        assert!(Update::plan("1.0.0-beta", "1.0.0", artifact(b"x")).is_ok());
    }

    #[test]
    fn channels_and_maintenance_branches_are_typed() {
        assert_eq!(Channel::parse("candidate"), Some(Channel::Candidate));
        let maintenance = Maintenance {
            branch: "release/1.0".to_owned(),
            support_matrix: "matrix-1".to_owned(),
            limits: BTreeSet::new(),
        };
        maintenance.validate().expect("a branch");
    }
}
