// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Local extension packages, their contracts and templates (task F22).
//!
//! An extension is a manifest plus files, not a name. The manifest states the host range, the
//! capabilities it registers and the permissions it asks for; [`Package::verify_digest`] proves the
//! files are the ones the author published. A package that is incompatible, asks for a capability
//! the host does not publish, carries an unknown permission, or fails its digest is refused as a
//! whole — nothing is half-installed. Templates use the same parameter schema rule: a missing
//! required parameter is a refusal, not an empty string.

use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

use crate::project::record::json::Json;
#[cfg(test)]
use crate::project::record::json::JsonObject;

/// A three-part version.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Version {
    /// Major.
    pub major: u64,
    /// Minor.
    pub minor: u64,
    /// Patch.
    pub patch: u64,
}

impl Version {
    /// Reads `major.minor.patch`.
    pub fn parse(text: &str) -> Result<Self, String> {
        let parts: Vec<&str> = text.split('.').collect();
        if parts.len() != 3 {
            return Err(format!("{text:?} is not a major.minor.patch version"));
        }
        let number = |part: &str| {
            part.parse::<u64>()
                .map_err(|_| format!("{text:?} is not a major.minor.patch version"))
        };
        Ok(Self {
            major: number(parts[0])?,
            minor: number(parts[1])?,
            patch: number(parts[2])?,
        })
    }

    /// Whether this version is inside `minimum..=maximum`.
    pub fn within(self, minimum: Self, maximum: Self) -> bool {
        minimum <= self && self <= maximum
    }

    /// The version as text.
    pub fn render(self) -> String {
        format!("{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// What the host offers an extension.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Host {
    /// The host version.
    pub version: Version,
    /// The capabilities the host publishes.
    pub capabilities: BTreeSet<String>,
    /// The permissions the host can grant.
    pub permissions: BTreeSet<String>,
}

/// An extension's declared contract.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Manifest {
    /// Its stable id.
    pub id: String,
    /// Its version.
    pub version: Version,
    /// The oldest host it works with.
    pub host_min: Version,
    /// The newest host it works with.
    pub host_max: Version,
    /// The capabilities it registers.
    pub capabilities: Vec<String>,
    /// The permissions it asks for.
    pub permissions: Vec<String>,
    /// Its licence.
    pub license: String,
    /// The SHA-256 of its files, as the author published it.
    pub digest: String,
}

impl Manifest {
    /// Reads a manifest from JSON.
    pub fn parse(json: &Json) -> Result<Self, String> {
        let object = json
            .as_object()
            .ok_or_else(|| "the manifest is not an object".to_owned())?;
        let text = |name: &str| {
            object
                .get(name)
                .and_then(Json::as_str)
                .map(str::to_owned)
                .ok_or_else(|| format!("the manifest needs a string {name:?}"))
        };
        let list = |name: &str| -> Result<Vec<String>, String> {
            match object.get(name) {
                Some(Json::Array(items)) => items
                    .iter()
                    .map(|item| {
                        item.as_str()
                            .map(str::to_owned)
                            .ok_or_else(|| format!("{name:?} holds a non-string item"))
                    })
                    .collect(),
                None => Ok(Vec::new()),
                _ => Err(format!("{name:?} is not an array")),
            }
        };
        Ok(Self {
            id: text("id")?,
            version: Version::parse(&text("version")?)?,
            host_min: Version::parse(&text("host_min")?)?,
            host_max: Version::parse(&text("host_max")?)?,
            capabilities: list("capabilities")?,
            permissions: list("permissions")?,
            license: text("license")?,
            digest: text("digest")?,
        })
    }

    /// Checks the manifest against the host contract.
    pub fn validate(&self, host: &Host) -> Result<(), String> {
        if self.id.trim().is_empty() {
            return Err("the manifest has an empty id".to_owned());
        }
        if self.host_min > self.host_max {
            return Err(format!(
                "the host range is reversed: {} > {}",
                self.host_min.render(),
                self.host_max.render()
            ));
        }
        if !host.version.within(self.host_min, self.host_max) {
            return Err(format!(
                "host {} is outside {}..={}",
                host.version.render(),
                self.host_min.render(),
                self.host_max.render()
            ));
        }
        for capability in &self.capabilities {
            if !host.capabilities.contains(capability) {
                return Err(format!(
                    "the host does not publish capability {capability:?}"
                ));
            }
        }
        for permission in &self.permissions {
            if !host.permissions.contains(permission) {
                return Err(format!("the host does not grant permission {permission:?}"));
            }
        }
        if self.license.trim().is_empty() {
            return Err("the manifest has no licence".to_owned());
        }
        Version::parse(&self.version.render())?;
        Ok(())
    }
}

/// A package's files plus its manifest.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Package {
    /// The declared contract.
    pub manifest: Manifest,
    /// Project-relative file names and their bytes.
    pub files: BTreeMap<String, Vec<u8>>,
}

impl Package {
    /// The SHA-256 of the files, in name order.
    ///
    /// Each name and its bytes are fed with their length prefixes, so `ab` + `c` cannot collide with
    /// `a` + `bc`.
    pub fn digest(&self) -> String {
        let mut hasher = Sha256::new();
        for (name, bytes) in &self.files {
            hasher.update((name.len() as u64).to_le_bytes());
            hasher.update(name.as_bytes());
            hasher.update((bytes.len() as u64).to_le_bytes());
            hasher.update(bytes);
        }
        let digest = hasher.finalize();
        digest.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    /// Whether the files match the digest in the manifest.
    pub fn verify_digest(&self) -> Result<(), String> {
        let actual = self.digest();
        if actual != self.manifest.digest {
            return Err(format!(
                "the package digest is {actual}, but the manifest says {}",
                self.manifest.digest
            ));
        }
        Ok(())
    }
}

/// The installed extensions.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Registry {
    packages: BTreeMap<String, Package>,
    disabled: BTreeSet<String>,
}

impl Registry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Installs a package after every contract check, replacing an earlier version only when the
    /// new version is higher.
    pub fn install(&mut self, package: Package, host: &Host) -> Result<(), String> {
        package.manifest.validate(host)?;
        package.verify_digest()?;
        if let Some(existing) = self.packages.get(&package.manifest.id) {
            if package.manifest.version < existing.manifest.version {
                return Err(format!(
                    "refusing to downgrade {} from {} to {}",
                    package.manifest.id,
                    existing.manifest.version.render(),
                    package.manifest.version.render()
                ));
            }
            if package.manifest.version == existing.manifest.version
                && package.digest() != existing.digest()
            {
                return Err(format!(
                    "{} {} has a different digest than the installed copy",
                    package.manifest.id,
                    package.manifest.version.render()
                ));
            }
        }
        let id = package.manifest.id.clone();
        self.packages.insert(id.clone(), package);
        self.disabled.remove(&id);
        Ok(())
    }

    /// The installed ids.
    pub fn ids(&self) -> Vec<String> {
        self.packages.keys().cloned().collect()
    }

    /// Disables an extension without removing its files.
    pub fn disable(&mut self, id: &str) -> Result<(), String> {
        if !self.packages.contains_key(id) {
            return Err(format!("no extension {id:?} is installed"));
        }
        self.disabled.insert(id.to_owned());
        Ok(())
    }

    /// Enables an installed extension.
    pub fn enable(&mut self, id: &str) -> Result<(), String> {
        if !self.packages.contains_key(id) {
            return Err(format!("no extension {id:?} is installed"));
        }
        self.disabled.remove(id);
        Ok(())
    }

    /// Removes an extension and its disabled state.
    pub fn remove(&mut self, id: &str) -> Result<String, String> {
        let package = self
            .packages
            .remove(id)
            .ok_or_else(|| format!("no extension {id:?} is installed"))?;
        self.disabled.remove(id);
        Ok(format!(
            "removed {} {}",
            package.manifest.id,
            package.manifest.version.render()
        ))
    }

    /// Whether an extension is installed and enabled.
    pub fn is_enabled(&self, id: &str) -> bool {
        self.packages.contains_key(id) && !self.disabled.contains(id)
    }

    /// The registry in one line per extension.
    pub fn render(&self) -> Vec<String> {
        self.packages
            .values()
            .map(|package| {
                format!(
                    "{} {} {} [{}] {}",
                    package.manifest.id,
                    package.manifest.version.render(),
                    package.manifest.license,
                    if self.disabled.contains(&package.manifest.id) {
                        "disabled"
                    } else {
                        "enabled"
                    },
                    package.digest()
                )
            })
            .collect()
    }

    /// Runs an update preflight: the new package must be valid, higher, and not a different build
    /// of the same version.
    pub fn preflight_update(&self, package: &Package, host: &Host) -> Result<(), String> {
        package.manifest.validate(host)?;
        package.verify_digest()?;
        let existing = self
            .packages
            .get(&package.manifest.id)
            .ok_or_else(|| format!("{} is not installed", package.manifest.id))?;
        if package.manifest.version <= existing.manifest.version {
            return Err(format!(
                "{} {} is not newer than {}",
                package.manifest.id,
                package.manifest.version.render(),
                existing.manifest.version.render()
            ));
        }
        Ok(())
    }
}

/// One template parameter's kind.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ParamKind {
    /// Free text.
    Text,
    /// A whole number.
    Integer,
    /// A flag.
    Flag,
}

impl ParamKind {
    /// Reads a parameter kind.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "text" => Some(Self::Text),
            "integer" => Some(Self::Integer),
            "flag" => Some(Self::Flag),
            _ => None,
        }
    }
}

/// A template parameter.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Param {
    /// Its kind.
    pub kind: ParamKind,
    /// Whether it must be supplied.
    pub required: bool,
}

/// A project/page/test template.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Template {
    /// Its name.
    pub name: String,
    /// Its parameters.
    pub params: BTreeMap<String, Param>,
    /// Its files, with `{{parameter}}` placeholders.
    pub files: BTreeMap<String, String>,
}

impl Template {
    /// Renders the files with `values`.
    ///
    /// Every required parameter must be present and every value must match its kind; a missing or
    /// malformed one is refused before a file is produced.
    pub fn render(
        &self,
        values: &BTreeMap<String, String>,
    ) -> Result<BTreeMap<String, String>, String> {
        for (name, param) in &self.params {
            match values.get(name) {
                None if param.required => {
                    return Err(format!("the template needs parameter {name:?}"));
                }
                None => {}
                Some(value) => match param.kind {
                    ParamKind::Text => {}
                    ParamKind::Integer => {
                        value
                            .parse::<i64>()
                            .map_err(|_| format!("{name:?} must be an integer, got {value:?}"))?;
                    }
                    ParamKind::Flag => {
                        if value != "true" && value != "false" {
                            return Err(format!("{name:?} must be true or false, got {value:?}"));
                        }
                    }
                },
            }
        }
        let mut out = BTreeMap::new();
        for (name, text) in &self.files {
            let mut rendered = text.clone();
            for (key, value) in values {
                rendered = rendered.replace(&format!("{{{{{key}}}}}"), value);
            }
            if rendered.contains("{{") {
                return Err(format!("file {name:?} has an unresolved placeholder"));
            }
            out.insert(name.clone(), rendered);
        }
        Ok(out)
    }
}

/// Builds a manifest object, for tests and the command line.
#[cfg(test)]
pub fn manifest_json(manifest: &Manifest) -> Json {
    let mut object = JsonObject::new();
    object.insert("id", Json::string(&manifest.id));
    object.insert("version", Json::string(manifest.version.render()));
    object.insert("host_min", Json::string(manifest.host_min.render()));
    object.insert("host_max", Json::string(manifest.host_max.render()));
    object.insert(
        "capabilities",
        Json::Array(manifest.capabilities.iter().map(Json::string).collect()),
    );
    object.insert(
        "permissions",
        Json::Array(manifest.permissions.iter().map(Json::string).collect()),
    );
    object.insert("license", Json::string(&manifest.license));
    object.insert("digest", Json::string(&manifest.digest));
    Json::Object(object)
}

#[cfg(test)]
mod tests {
    use super::{
        Host, Manifest, Package, Param, ParamKind, Registry, Template, Version, manifest_json,
    };
    use std::collections::{BTreeMap, BTreeSet};

    fn host() -> Host {
        Host {
            version: Version::parse("0.1.0").expect("a version"),
            capabilities: ["widget", "template"]
                .into_iter()
                .map(str::to_owned)
                .collect::<BTreeSet<_>>(),
            permissions: ["network"].into_iter().map(str::to_owned).collect(),
        }
    }

    fn package(version: &str, digest: Option<String>) -> Package {
        let files = [("main.rs".to_owned(), b"fn main() {}".to_vec())]
            .into_iter()
            .collect::<BTreeMap<_, _>>();
        let mut manifest = Manifest {
            id: "acme.chart".to_owned(),
            version: Version::parse(version).expect("a version"),
            host_min: Version::parse("0.1.0").expect("a version"),
            host_max: Version::parse("0.2.0").expect("a version"),
            capabilities: vec!["widget".to_owned()],
            permissions: vec!["network".to_owned()],
            license: "MIT".to_owned(),
            digest: String::new(),
        };
        let mut package = Package { manifest, files };
        let actual = package.digest();
        package.manifest.digest = digest.unwrap_or(actual);
        manifest = package.manifest.clone();
        let _ = manifest;
        package
    }

    #[test]
    fn a_manifest_round_trips_and_checks_the_host_range() {
        let package = package("1.2.0", None);
        let json = manifest_json(&package.manifest);
        let parsed = Manifest::parse(&json).expect("a manifest");
        assert_eq!(parsed, package.manifest);
        parsed.validate(&host()).expect("a compatible manifest");
        let mut other = host();
        other.version = Version::parse("0.3.0").expect("a version");
        assert!(parsed.validate(&other).is_err());
    }

    #[test]
    fn an_unknown_capability_or_permission_is_refused() {
        let mut first = package("1.2.0", None);
        first.manifest.capabilities = vec!["teleport".to_owned()];
        first.manifest.digest = first.digest();
        assert!(first.manifest.validate(&host()).is_err());
        let mut second = package("1.2.0", None);
        second.manifest.permissions = vec!["admin".to_owned()];
        second.manifest.digest = second.digest();
        assert!(second.manifest.validate(&host()).is_err());
    }

    #[test]
    fn a_digest_mismatch_is_refused() {
        let package = package("1.2.0", Some("00".repeat(32)));
        assert!(package.verify_digest().is_err());
    }

    #[test]
    fn installing_refuses_a_downgrade_and_a_same_version_rebuild() {
        let mut registry = Registry::new();
        registry
            .install(package("1.2.0", None), &host())
            .expect("an install");
        assert!(registry.install(package("1.1.0", None), &host()).is_err());
        let mut rebuild = package("1.2.0", None);
        rebuild.files.insert("other.rs".to_owned(), b"x".to_vec());
        rebuild.manifest.digest = rebuild.digest();
        assert!(registry.install(rebuild, &host()).is_err());
    }

    #[test]
    fn disable_enable_and_remove_are_separate_states() {
        let mut registry = Registry::new();
        registry
            .install(package("1.2.0", None), &host())
            .expect("an install");
        assert!(registry.is_enabled("acme.chart"));
        registry.disable("acme.chart").expect("a disable");
        assert!(!registry.is_enabled("acme.chart"));
        registry.enable("acme.chart").expect("an enable");
        assert!(registry.is_enabled("acme.chart"));
        assert!(registry.remove("acme.chart").is_ok());
        assert!(!registry.is_enabled("acme.chart"));
    }

    #[test]
    fn a_template_renders_required_parameters_and_refuses_missing_ones() {
        let mut params = BTreeMap::new();
        params.insert(
            "name".to_owned(),
            Param {
                kind: ParamKind::Text,
                required: true,
            },
        );
        params.insert(
            "count".to_owned(),
            Param {
                kind: ParamKind::Integer,
                required: false,
            },
        );
        let template = Template {
            name: "card".to_owned(),
            params,
            files: [("card.rs".to_owned(), "// {{name}}\n".to_owned())]
                .into_iter()
                .collect(),
        };
        let mut values = BTreeMap::new();
        assert!(template.render(&values).is_err());
        values.insert("name".to_owned(), "Hero".to_owned());
        let rendered = template.render(&values).expect("a render");
        assert_eq!(rendered["card.rs"], "// Hero\n");
        values.insert("count".to_owned(), "many".to_owned());
        assert!(template.render(&values).is_err());
    }
}
