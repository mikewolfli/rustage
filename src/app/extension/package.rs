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
//!
//! **Provenance, pinning and rollback (T22.09).** Every package records where this copy came from
//! ([`Package::source`]): a local directory for an offline install, or `local` for one declared on
//! the command line. The digest is verified before any state changes. An extension can be pinned at
//! its installed version, in which case an update is refused until it is unpinned, and a successful
//! update keeps the previous package so [`Registry::rollback`] can restore it. None of this makes an
//! extension trusted: provenance is a fact a person can read, not a permission.
//!
//! **What an extension contributes (T22.05).** A manifest may name the entry points it registers
//! (`provides`): the control or command names an open project can reference. That list is what
//! [`crate::app::extension::manager`] matches a project's used names against, so a removal can say
//! which of its entry points the project would leave dangling. A manifest that names nothing is
//! still a valid extension; it simply has no project-visible entry point to check.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::project::record::json::Json;
#[cfg(test)]
use crate::project::record::json::JsonObject;

/// The source recorded for a package declared on the command line rather than read from a directory.
pub const LOCAL_SOURCE: &str = "local";

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
    /// The entry points it registers (control or command names a project can reference).
    pub provides: Vec<String>,
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
            provides: list("provides")?,
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
    /// Where this copy came from: a local directory, or [`LOCAL_SOURCE`].
    pub source: String,
}

impl Package {
    /// A package with no declared provenance.
    pub fn new(manifest: Manifest, files: BTreeMap<String, Vec<u8>>) -> Self {
        Self {
            manifest,
            files,
            source: LOCAL_SOURCE.to_owned(),
        }
    }

    /// Reads a package laid out on disk as `manifest.json` plus a `files/` directory.
    ///
    /// This is the offline install path: nothing is fetched, and the source is the directory the
    /// caller named. A symlink inside `files/` is refused rather than followed, so a package cannot
    /// reach outside its own directory. The manifest's digest is not checked here —
    /// [`Registry::install`] verifies it before anything changes.
    pub fn from_directory(path: &Path) -> Result<Self, String> {
        let manifest_path = path.join("manifest.json");
        let text = std::fs::read_to_string(&manifest_path)
            .map_err(|error| format!("cannot read {}: {error}", manifest_path.display()))?;
        let json = crate::project::record::json::parse(&text)
            .map_err(|error| format!("cannot parse {}: {error}", manifest_path.display()))?;
        let manifest = Manifest::parse(&json)?;
        let root = path.join("files");
        let mut files = BTreeMap::new();
        if root.is_dir() {
            collect_files(&root, &root, &mut files)?;
        }
        Ok(Self {
            manifest,
            files,
            source: path.display().to_string(),
        })
    }
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

/// Reads every regular file under `directory` into `files`, keyed by its `/`-separated relative path.
fn collect_files(
    root: &Path,
    directory: &Path,
    files: &mut BTreeMap<String, Vec<u8>>,
) -> Result<(), String> {
    let entries = std::fs::read_dir(directory)
        .map_err(|error| format!("cannot read {}: {error}", directory.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot read an entry: {error}"))?;
        let path = entry.path();
        let kind = entry
            .file_type()
            .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
        if kind.is_symlink() {
            return Err(format!(
                "{} is a symlink; a package may not link outside its own directory",
                path.display()
            ));
        }
        if kind.is_dir() {
            collect_files(root, &path, files)?;
            continue;
        }
        if !kind.is_file() {
            return Err(format!("{} is not a regular file", path.display()));
        }
        let name = path
            .strip_prefix(root)
            .map_err(|error| format!("{} is outside the package: {error}", path.display()))?;
        let name = path_name(name)?;
        let bytes = std::fs::read(&path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        files.insert(name, bytes);
    }
    Ok(())
}

/// Joins a relative path's components with `/`, refusing a component that is not UTF-8.
fn path_name(path: &Path) -> Result<String, String> {
    let mut parts: Vec<String> = Vec::new();
    for component in path.components() {
        let part = component
            .as_os_str()
            .to_str()
            .ok_or_else(|| format!("{} is not valid UTF-8", path.display()))?;
        parts.push(part.to_owned());
    }
    Ok(parts.join("/"))
}

/// The installed extensions.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Registry {
    packages: BTreeMap<String, Package>,
    disabled: BTreeSet<String>,
    /// Extensions fixed at their installed version; an update is refused until they are unpinned.
    pinned: BTreeSet<String>,
    /// The package each extension replaced, so an update can be rolled back.
    previous: BTreeMap<String, Package>,
}

impl Registry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Installs a package after every contract check, replacing an earlier version only when the
    /// new version is higher.
    ///
    /// Every check runs before the registry changes: a bad digest, an incompatible host or a pinned
    /// extension leaves the installed copy exactly as it was. A successful replacement records the
    /// old package so [`Self::rollback`] can restore it.
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
            if self.pinned.contains(&package.manifest.id)
                && package.manifest.version != existing.manifest.version
            {
                return Err(format!(
                    "{} is pinned at {}; unpin it before updating",
                    package.manifest.id,
                    existing.manifest.version.render()
                ));
            }
        }
        let id = package.manifest.id.clone();
        if let Some(previous) = self.packages.insert(id.clone(), package) {
            self.previous.insert(id.clone(), previous);
        }
        self.disabled.remove(&id);
        Ok(())
    }

    /// Fixes an installed extension at its current version.
    pub fn pin(&mut self, id: &str) -> Result<(), String> {
        if !self.packages.contains_key(id) {
            return Err(format!("no extension {id:?} is installed"));
        }
        self.pinned.insert(id.to_owned());
        Ok(())
    }

    /// Releases a pin.
    pub fn unpin(&mut self, id: &str) -> Result<(), String> {
        if !self.pinned.remove(id) {
            return Err(format!("{id:?} is not pinned"));
        }
        Ok(())
    }

    /// Whether an extension is fixed at its installed version.
    pub fn is_pinned(&self, id: &str) -> bool {
        self.pinned.contains(id)
    }

    /// The pinned ids, in order.
    pub fn pinned_ids(&self) -> Vec<String> {
        self.pinned.iter().cloned().collect()
    }

    /// The recorded source of an installed package.
    pub fn source(&self, id: &str) -> Option<&str> {
        self.packages.get(id).map(|package| package.source.as_str())
    }

    /// The installed version of an extension, as written.
    pub fn version(&self, id: &str) -> Option<String> {
        self.packages
            .get(id)
            .map(|package| package.manifest.version.render())
    }

    /// The installed manifest of an extension.
    pub fn manifest(&self, id: &str) -> Option<&Manifest> {
        self.packages.get(id).map(|package| &package.manifest)
    }

    /// Restores the version an extension replaced, and keeps the replaced one so a second rollback
    /// returns to where it was. The pin survives: a rollback is recovery, not an update.
    pub fn rollback(&mut self, id: &str) -> Result<String, String> {
        let current = self
            .packages
            .get(id)
            .ok_or_else(|| format!("no extension {id:?} is installed"))?;
        let previous = self
            .previous
            .remove(id)
            .ok_or_else(|| format!("no previous version of {id:?} is recorded"))?;
        let current_version = current.manifest.version.render();
        let previous_version = previous.manifest.version.render();
        let replaced = self
            .packages
            .insert(id.to_owned(), previous)
            .expect("the package was checked above");
        self.previous.insert(id.to_owned(), replaced);
        Ok(format!(
            "rolled back {id} from {current_version} to {previous_version}"
        ))
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

    /// Removes an extension and its disabled state, pin and recorded previous version.
    pub fn remove(&mut self, id: &str) -> Result<String, String> {
        let package = self
            .packages
            .remove(id)
            .ok_or_else(|| format!("no extension {id:?} is installed"))?;
        self.disabled.remove(id);
        self.pinned.remove(id);
        self.previous.remove(id);
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
                let mut state = if self.disabled.contains(&package.manifest.id) {
                    "disabled".to_owned()
                } else {
                    "enabled".to_owned()
                };
                if self.pinned.contains(&package.manifest.id) {
                    state.push_str(",pinned");
                }
                format!(
                    "{} {} {} [{}] {} from {}",
                    package.manifest.id,
                    package.manifest.version.render(),
                    package.manifest.license,
                    state,
                    package.digest(),
                    package.source
                )
            })
            .collect()
    }

    /// Runs an update preflight: the new package must be valid, higher, and not a different build
    /// of the same version; a pinned extension refuses an update.
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
        if self.pinned.contains(&package.manifest.id) {
            return Err(format!(
                "{} is pinned at {}; unpin it before updating",
                package.manifest.id,
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
    object.insert(
        "provides",
        Json::Array(manifest.provides.iter().map(Json::string).collect()),
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
        let manifest = Manifest {
            id: "acme.chart".to_owned(),
            version: Version::parse(version).expect("a version"),
            host_min: Version::parse("0.1.0").expect("a version"),
            host_max: Version::parse("0.2.0").expect("a version"),
            capabilities: vec!["widget".to_owned()],
            permissions: vec!["network".to_owned()],
            provides: vec!["acme.chart".to_owned()],
            license: "MIT".to_owned(),
            digest: String::new(),
        };
        let mut package = Package::new(manifest, files);
        let actual = package.digest();
        package.manifest.digest = digest.unwrap_or(actual);
        package
    }

    fn scratch(tag: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("a clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("rustage-ext-{tag}-{nanos}"));
        std::fs::create_dir_all(&path).expect("a scratch directory");
        path
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
    fn an_offline_directory_install_reads_the_manifest_and_files() {
        let root = scratch("install");
        let mut expected = package("1.2.0", None);
        expected
            .files
            .insert("sub/data.txt".to_owned(), b"hi".to_vec());
        expected.manifest.digest = expected.digest();
        std::fs::write(
            root.join("manifest.json"),
            manifest_json(&expected.manifest).render(),
        )
        .expect("a manifest file");
        std::fs::create_dir_all(root.join("files/sub")).expect("a files directory");
        std::fs::write(root.join("files/main.rs"), b"fn main() {}").expect("a file");
        std::fs::write(root.join("files/sub/data.txt"), b"hi").expect("a nested file");

        let package = Package::from_directory(&root).expect("a package");
        package
            .verify_digest()
            .expect("the files match the manifest");
        assert_eq!(package.source, root.display().to_string());

        let mut registry = Registry::new();
        registry.install(package, &host()).expect("an install");
        assert_eq!(
            registry.source("acme.chart"),
            Some(root.display().to_string().as_str())
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_inside_a_package_is_refused_rather_than_followed() {
        let root = scratch("symlink");
        let package = package("1.2.0", None);
        std::fs::write(
            root.join("manifest.json"),
            manifest_json(&package.manifest).render(),
        )
        .expect("a manifest file");
        std::fs::create_dir_all(root.join("files")).expect("a files directory");
        std::os::unix::fs::symlink("/etc/passwd", root.join("files/passwd")).expect("a symlink");

        let error = Package::from_directory(&root).expect_err("a symlink is refused");
        assert!(error.contains("symlink"), "{error}");
    }

    #[test]
    fn a_pinned_extension_refuses_an_update_until_it_is_unpinned() {
        let mut registry = Registry::new();
        registry
            .install(package("1.2.0", None), &host())
            .expect("an install");
        registry.pin("acme.chart").expect("a pin");
        assert!(registry.is_pinned("acme.chart"));

        let newer = package("1.3.0", None);
        assert!(registry.install(newer.clone(), &host()).is_err());
        assert!(registry.preflight_update(&newer, &host()).is_err());
        assert_eq!(registry.version("acme.chart").as_deref(), Some("1.2.0"));

        registry.unpin("acme.chart").expect("an unpin");
        registry
            .install(newer, &host())
            .expect("an update after unpinning");
        assert_eq!(registry.version("acme.chart").as_deref(), Some("1.3.0"));
    }

    #[test]
    fn a_rollback_restores_the_version_an_update_replaced() {
        let mut registry = Registry::new();
        registry
            .install(package("1.2.0", None), &host())
            .expect("an install");
        registry
            .install(package("1.3.0", None), &host())
            .expect("an update");
        let line = registry.rollback("acme.chart").expect("a rollback");
        assert!(line.contains("1.3.0") && line.contains("1.2.0"), "{line}");
        assert_eq!(registry.version("acme.chart").as_deref(), Some("1.2.0"));
        // The replaced package is kept, so a second rollback returns to 1.3.0.
        registry.rollback("acme.chart").expect("a second rollback");
        assert_eq!(registry.version("acme.chart").as_deref(), Some("1.3.0"));
    }

    #[test]
    fn a_bad_digest_or_a_missing_previous_version_changes_nothing() {
        let mut registry = Registry::new();
        assert!(registry.rollback("acme.chart").is_err());
        registry
            .install(package("1.2.0", None), &host())
            .expect("an install");
        assert!(registry.rollback("acme.chart").is_err());
        let bad = package("1.3.0", Some("00".repeat(32)));
        assert!(registry.install(bad, &host()).is_err());
        assert_eq!(registry.version("acme.chart").as_deref(), Some("1.2.0"));
    }

    #[test]
    fn removing_an_extension_also_clears_its_pin_and_previous_version() {
        let mut registry = Registry::new();
        registry
            .install(package("1.2.0", None), &host())
            .expect("an install");
        registry
            .install(package("1.3.0", None), &host())
            .expect("an update");
        registry.pin("acme.chart").expect("a pin");
        registry.remove("acme.chart").expect("a remove");
        assert!(!registry.is_pinned("acme.chart"));
        assert!(registry.rollback("acme.chart").is_err());
        // Reinstalling the older version is a fresh install, not a downgrade.
        registry
            .install(package("1.2.0", None), &host())
            .expect("a fresh install");
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
