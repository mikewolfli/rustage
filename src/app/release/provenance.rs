// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Release provenance: the artifact, the source facts and the lockfile, bound in one record
//! (task T25.07).
//!
//! The acceptance line is *"a download can be verified, installed and reproduced"*. A digest alone
//! proves the bytes are the ones someone published; it does not say which source revision or which
//! dependency lock produced them. [`Provenance`] is that binding:
//!
//! * **The artifact facts are the declared ones.** Name, SHA-256, size and signature state come
//!   from the artifact the release already declared, not from a second reading of the file.
//! * **The source facts are frozen.** Revision, package version, generator and the pinned widget
//!   revision are copied from [`Freeze`], so a provenance report cannot describe a different build.
//! * **The lockfile is identified by its digest.** The path is recorded, but the digest is what
//!   [`Provenance::verify`] checks: a lockfile edited after the report was written is a failure.
//! * **Verification re-reads both files.** `verify` recomputes the artifact digest and size and the
//!   lockfile digest; it does not trust the report's own arithmetic.

use std::path::Path;

use sha2::{Digest, Sha256};

use crate::project::record::json::{Json, JsonObject, parse};

use super::{Artifact, Freeze, lockfile};

/// The artifact a report describes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ArtifactFacts {
    /// Its file name.
    pub name: String,
    /// Its SHA-256.
    pub digest: String,
    /// Its size in bytes.
    pub size: u64,
    /// `unsigned` or `verified`.
    pub signature: String,
}

/// The source facts a report describes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SourceFacts {
    /// The product revision.
    pub revision: String,
    /// The package version.
    pub package_version: String,
    /// The generator version.
    pub generator: String,
    /// The pinned widget revision.
    pub widget_revision: String,
    /// The support-matrix revision.
    pub support_matrix: String,
}

/// The lockfile a report describes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LockfileFacts {
    /// The path as recorded when the report was written.
    pub path: String,
    /// The lockfile's SHA-256.
    pub digest: String,
    /// How many packages it locks.
    pub packages: usize,
}

/// One release's provenance record.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Provenance {
    /// The artifact.
    pub artifact: ArtifactFacts,
    /// The source facts.
    pub source: SourceFacts,
    /// The lockfile.
    pub lockfile: LockfileFacts,
}

impl Provenance {
    /// Builds the record from the declared artifact, the frozen inputs and a lockfile.
    pub fn assemble(
        artifact: &Artifact,
        freeze: &Freeze,
        lockfile_path: &Path,
    ) -> Result<Self, String> {
        let text = std::fs::read_to_string(lockfile_path)
            .map_err(|error| format!("cannot read {}: {error}", lockfile_path.display()))?;
        let packages = lockfile::parse(&text).map_err(|error| {
            format!(
                "{} is not a Cargo lockfile: {error}",
                lockfile_path.display()
            )
        })?;
        let digest = digest_of(
            &std::fs::read(lockfile_path)
                .map_err(|error| format!("cannot read {}: {error}", lockfile_path.display()))?,
        );
        Ok(Self {
            artifact: ArtifactFacts {
                name: artifact.name.clone(),
                digest: artifact.digest.clone(),
                size: artifact.size,
                signature: artifact.signature.name().to_owned(),
            },
            source: SourceFacts {
                revision: freeze.revision.clone(),
                package_version: freeze.package_version.clone(),
                generator: freeze.generator.clone(),
                widget_revision: freeze.widget_revision.clone(),
                support_matrix: freeze.support_matrix.clone(),
            },
            lockfile: LockfileFacts {
                path: lockfile_path.to_string_lossy().into_owned(),
                digest,
                packages: packages.len(),
            },
        })
    }

    /// The record as stable JSON.
    pub fn json(&self) -> String {
        let mut artifact = JsonObject::new();
        artifact.insert("name", Json::string(self.artifact.name.clone()));
        artifact.insert("digest", Json::string(self.artifact.digest.clone()));
        artifact.insert("bytes", Json::number(self.artifact.size));
        artifact.insert("signature", Json::string(self.artifact.signature.clone()));

        let mut source = JsonObject::new();
        source.insert("revision", Json::string(self.source.revision.clone()));
        source.insert(
            "package_version",
            Json::string(self.source.package_version.clone()),
        );
        source.insert("generator", Json::string(self.source.generator.clone()));
        source.insert(
            "widget_revision",
            Json::string(self.source.widget_revision.clone()),
        );
        source.insert(
            "support_matrix",
            Json::string(self.source.support_matrix.clone()),
        );

        let mut lock = JsonObject::new();
        lock.insert("path", Json::string(self.lockfile.path.clone()));
        lock.insert("digest", Json::string(self.lockfile.digest.clone()));
        lock.insert("packages", Json::number(self.lockfile.packages as u64));

        let mut root = JsonObject::new();
        root.insert("artifact", Json::Object(artifact));
        root.insert("source", Json::Object(source));
        root.insert("lockfile", Json::Object(lock));
        Json::Object(root).render()
    }

    /// Re-reads a report from its JSON.
    pub fn parse(text: &str) -> Result<Self, String> {
        let parsed = parse(text).map_err(|error| format!("the report is not JSON: {error}"))?;
        let root = parsed
            .as_object()
            .ok_or_else(|| "the report is not an object".to_owned())?;
        let artifact = object(root.get("artifact"), "artifact")?;
        let source = object(root.get("source"), "source")?;
        let lock = object(root.get("lockfile"), "lockfile")?;
        Ok(Self {
            artifact: ArtifactFacts {
                name: string(artifact.get("name"), "artifact.name")?,
                digest: string(artifact.get("digest"), "artifact.digest")?,
                size: number(artifact.get("bytes"), "artifact.bytes")?,
                signature: string(artifact.get("signature"), "artifact.signature")?,
            },
            source: SourceFacts {
                revision: string(source.get("revision"), "source.revision")?,
                package_version: string(source.get("package_version"), "source.package_version")?,
                generator: string(source.get("generator"), "source.generator")?,
                widget_revision: string(source.get("widget_revision"), "source.widget_revision")?,
                support_matrix: string(source.get("support_matrix"), "source.support_matrix")?,
            },
            lockfile: LockfileFacts {
                path: string(lock.get("path"), "lockfile.path")?,
                digest: string(lock.get("digest"), "lockfile.digest")?,
                packages: number(lock.get("packages"), "lockfile.packages")? as usize,
            },
        })
    }

    /// Re-reads the artifact and the lockfile and answers what matched.
    pub fn verify(&self, artifact_path: &Path) -> Result<Vec<String>, String> {
        let bytes = std::fs::read(artifact_path)
            .map_err(|error| format!("cannot read {}: {error}", artifact_path.display()))?;
        let digest = digest_of(&bytes);
        if digest != self.artifact.digest {
            return Err(format!(
                "{} has digest {digest}, not {}",
                artifact_path.display(),
                self.artifact.digest
            ));
        }
        if bytes.len() as u64 != self.artifact.size {
            return Err(format!(
                "{} has {} byte(s), not {}",
                artifact_path.display(),
                bytes.len(),
                self.artifact.size
            ));
        }
        let lock_bytes = std::fs::read(&self.lockfile.path)
            .map_err(|error| format!("cannot read {}: {error}", self.lockfile.path))?;
        let lock_digest = digest_of(&lock_bytes);
        if lock_digest != self.lockfile.digest {
            return Err(format!(
                "{} has digest {lock_digest}, not {}",
                self.lockfile.path, self.lockfile.digest
            ));
        }
        Ok(vec![
            format!(
                "{} verified ({} byte(s), {})",
                self.artifact.name,
                bytes.len(),
                digest
            ),
            format!(
                "lockfile {} verified ({} package(s), {})",
                self.lockfile.path, self.lockfile.packages, lock_digest
            ),
            format!(
                "source revision {} (package {}, widgets {}, support matrix {})",
                self.source.revision,
                self.source.package_version,
                self.source.widget_revision,
                self.source.support_matrix
            ),
        ])
    }

    /// The record in lines, for a report that is not JSON.
    pub fn render(&self) -> Vec<String> {
        vec![
            format!(
                "artifact {} ({} byte(s)) {} {}",
                self.artifact.name,
                self.artifact.size,
                self.artifact.digest,
                self.artifact.signature
            ),
            format!(
                "source revision {}, package {}, generator {}, widgets {}",
                self.source.revision,
                self.source.package_version,
                self.source.generator,
                self.source.widget_revision
            ),
            format!(
                "lockfile {} ({} package(s)) {}",
                self.lockfile.path, self.lockfile.packages, self.lockfile.digest
            ),
        ]
    }
}

fn object<'a>(value: Option<&'a Json>, name: &str) -> Result<&'a JsonObject, String> {
    value
        .and_then(Json::as_object)
        .ok_or_else(|| format!("the report has no {name} object"))
}

fn string(value: Option<&Json>, name: &str) -> Result<String, String> {
    value
        .and_then(Json::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("the report has no {name}"))
}

fn number(value: Option<&Json>, name: &str) -> Result<u64, String> {
    value
        .and_then(Json::as_u64)
        .ok_or_else(|| format!("the report has no {name} number"))
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
    use super::Provenance;
    use crate::app::release::{Artifact, Freeze, Signature};
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "rustage-provenance-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&path).expect("a scratch directory");
        path
    }

    fn facts() -> (Artifact, Freeze) {
        let bytes = b"a release binary";
        let digest = super::digest_of(bytes);
        let mut freeze = Freeze::new();
        freeze.revision = "abc".to_owned();
        freeze.package_version = "0.1.0".to_owned();
        freeze.generator = "0.1.0".to_owned();
        freeze.widget_revision = "widgets".to_owned();
        freeze.support_matrix = "matrix".to_owned();
        (
            Artifact {
                name: "rustage".to_owned(),
                digest,
                size: bytes.len() as u64,
                signature: Signature::Unsigned,
                licenses: Vec::new(),
            },
            freeze,
        )
    }

    #[test]
    fn a_report_binds_artifact_source_and_lockfile_and_round_trips() {
        let root = scratch("round-trip");
        let binary = root.join("rustage");
        std::fs::write(&binary, b"a release binary").expect("a binary");
        let lock = root.join("Cargo.lock");
        std::fs::write(
            &lock,
            "version = 3\n\n[[package]]\nname = \"serde\"\nversion = \"1.0.0\"\nsource = \"registry\"\nchecksum = \"aa\"\n",
        )
        .expect("a lockfile");
        let (artifact, freeze) = facts();
        let report = Provenance::assemble(&artifact, &freeze, &lock).expect("a report");
        assert_eq!(report.lockfile.packages, 1);
        assert_eq!(report.source.revision, "abc");

        let parsed = Provenance::parse(&report.json()).expect("a parsed report");
        assert_eq!(parsed, report);
        let lines = parsed.verify(&binary).expect("a verification");
        assert!(lines[0].contains("verified"), "{lines:?}");
        assert!(lines[2].contains("source revision abc"), "{lines:?}");
    }

    #[test]
    fn a_changed_artifact_or_lockfile_fails_verification() {
        let root = scratch("tamper");
        let binary = root.join("rustage");
        std::fs::write(&binary, b"a release binary").expect("a binary");
        let lock = root.join("Cargo.lock");
        std::fs::write(&lock, "version = 3\n").expect("a lockfile");
        let (artifact, freeze) = facts();
        let report = Provenance::assemble(&artifact, &freeze, &lock).expect("a report");

        std::fs::write(&binary, b"tampered").expect("a change");
        let error = report.verify(&binary).expect_err("a refusal");
        assert!(error.contains("has digest"), "{error}");

        std::fs::write(&binary, b"a release binary").expect("restore");
        std::fs::write(&lock, "version = 3\n# changed\n").expect("a change");
        let error = report.verify(&binary).expect_err("a refusal");
        assert!(
            error.contains("lockfile") || error.contains("Cargo.lock"),
            "{error}"
        );
    }
}
