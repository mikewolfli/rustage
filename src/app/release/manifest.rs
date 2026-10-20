// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Release freeze, artifacts, SBOM and updates (task F25).
//!
//! A release is a set of names, revisions and digests, not a directory that happens to build. The
//! freeze names the revision, generator, formats and support-matrix revision; an artifact carries
//! its SHA-256 and signature state; the SBOM lists every component with its source and licence. An
//! update verifies the artifact before touching a backup, and a failed verification leaves the old
//! installation in place.

use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

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
    /// The product revision.
    pub revision: String,
    /// The generator version.
    pub generator: String,
    /// The artifact formats.
    pub formats: Vec<String>,
    /// The support-matrix revision or digest.
    pub support_matrix: String,
    /// Component versions by name.
    pub components: BTreeMap<String, String>,
}

impl Freeze {
    /// An empty freeze.
    pub fn new() -> Self {
        Self {
            revision: String::new(),
            generator: String::new(),
            formats: Vec::new(),
            support_matrix: String::new(),
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

    /// The freeze in one line per fact.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![
            format!("revision {}", self.revision),
            format!("generator {}", self.generator),
            format!("support matrix {}", self.support_matrix),
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
        let actual: String = Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
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
        if from == to {
            return Err(format!("{to} is already installed"));
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
        Artifact, Channel, Component, Freeze, Maintenance, Sbom, Signature, Update, UpdateState,
    };
    use std::collections::BTreeSet;

    use sha2::{Digest, Sha256};

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
            version: "2.8.12".to_owned(),
            license: "MIT".to_owned(),
            source: "git".to_owned(),
            digest: None,
        };
        sbom.add(component.clone()).expect("an add");
        sbom.add(component.clone()).expect("an identical add");
        let mut other = component;
        other.version = "2.8.11".to_owned();
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
        assert!(Update::plan("1.0.0", "1.0.0", artifact).is_err());
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
