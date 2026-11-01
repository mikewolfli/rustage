// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Installing and uninstalling a staged release offline (task T25.04).
//!
//! The acceptance line is *"uninstalling deletes no user project, and the compatibility limits are
//! visible"*. That is a boundary, not a feature, so every rule here forbids rather than repairs:
//!
//! * **Only what the stage lists is installed.** `stage.json` names every file; a file the stage
//!   does not list is not copied, and a listed file that is missing or whose digest differs is a
//!   refusal. An install therefore cannot introduce a file the release did not freeze.
//! * **A release is never installed over a project.** A root that already holds `project.json` is
//!   refused before anything is written, so the common mistake of pointing at a workspace cannot
//!   overwrite it.
//! * **Nothing existing is overwritten.** An install refuses a destination file that already
//!   exists; the repair is an uninstall first, which removes exactly the recorded files.
//! * **Uninstall removes exactly the install record.** It reads the `rustage-install.json` written
//!   at install time and removes those paths only, so settings, projects and anything else the user
//!   put beside the release are untouched by construction.
//! * **No network.** Every check is a local file read. The stage's digests are integrity evidence,
//!   not a signature; [`boundaries`] says so rather than implying a verified publisher.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::project::record::format::PROJECT_FILE_NAME;
use crate::project::record::json::{Json, JsonObject, parse};

/// The manifest a stage writes.
pub const STAGE_MANIFEST: &str = "stage.json";
/// The record an install writes, and the only thing an uninstall reads.
pub const INSTALL_RECORD: &str = "rustage-install.json";

/// One file an install would copy.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Entry {
    /// Path relative to the stage, with `/` separators.
    pub path: String,
    /// Its size in bytes.
    pub bytes: u64,
    /// Its lower-case SHA-256.
    pub digest: String,
}

/// A checked plan: what would be installed, and where.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Plan {
    /// The stage directory.
    pub stage: PathBuf,
    /// The install root.
    pub root: PathBuf,
    /// The files the stage lists, checked against the stage itself.
    pub entries: Vec<Entry>,
}

impl Plan {
    /// The plan in lines.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "install {} file(s) from {} into {}",
            self.entries.len(),
            self.stage.display(),
            self.root.display()
        )];
        for entry in &self.entries {
            lines.push(format!(
                "  {} ({} byte(s)) {}",
                entry.path, entry.bytes, entry.digest
            ));
        }
        lines.push(
            "no network is used; settings and user projects are outside this record".to_owned(),
        );
        lines
    }
}

/// Checks a stage and an install root without writing anything.
pub fn plan(stage_directory: &Path, root: &Path) -> Result<Plan, String> {
    let manifest_path = stage_directory.join(STAGE_MANIFEST);
    let text = std::fs::read_to_string(&manifest_path)
        .map_err(|error| format!("cannot read {}: {error}", manifest_path.display()))?;
    let parsed = parse(&text)
        .map_err(|error| format!("{} is not valid JSON: {error}", manifest_path.display()))?;
    let object = parsed
        .as_object()
        .ok_or_else(|| format!("{} is not an object", manifest_path.display()))?;
    let files = object
        .get("files")
        .and_then(Json::as_array)
        .ok_or_else(|| format!("{} has no files array", manifest_path.display()))?;

    let mut entries = Vec::new();
    for file in files {
        let file = file
            .as_object()
            .ok_or_else(|| "a stage file entry is not an object".to_owned())?;
        let path = file
            .get("path")
            .and_then(Json::as_str)
            .ok_or_else(|| "a stage file entry has no path".to_owned())?;
        safe_relative(path)?;
        let expected = file
            .get("digest")
            .and_then(Json::as_str)
            .ok_or_else(|| format!("{path} has no digest"))?;
        let bytes = std::fs::read(stage_directory.join(path))
            .map_err(|error| format!("cannot read staged {path}: {error}"))?;
        let digest = digest_of(&bytes);
        if !digest.eq_ignore_ascii_case(expected) {
            return Err(format!(
                "staged {path} does not match its digest: {} != {expected}",
                digest
            ));
        }
        entries.push(Entry {
            path: path.to_owned(),
            bytes: bytes.len() as u64,
            digest,
        });
    }
    if entries.is_empty() {
        return Err("the stage lists no file".to_owned());
    }
    if !entries
        .iter()
        .any(|entry| entry.path.ends_with(".json") && entry.path.contains("language/"))
    {
        return Err(
            "the stage holds no language pack; a release without its runtime packs would open \
             with keys instead of labels"
                .to_owned(),
        );
    }
    if root.join(PROJECT_FILE_NAME).is_file() {
        return Err(format!(
            "{} is a project; a release is installed beside a project, never over it",
            root.display()
        ));
    }
    if root.join(INSTALL_RECORD).is_file() {
        return Err(format!(
            "{} is already an install; uninstall it before installing again",
            root.display()
        ));
    }

    Ok(Plan {
        stage: stage_directory.to_path_buf(),
        root: root.to_path_buf(),
        entries,
    })
}

/// Copies a checked plan and writes the install record.
pub fn install(plan: &Plan) -> Result<Vec<String>, String> {
    for entry in &plan.entries {
        let target = plan.root.join(&entry.path);
        if target.exists() {
            return Err(format!(
                "{} already exists; an install does not overwrite, so uninstall first",
                target.display()
            ));
        }
    }
    std::fs::create_dir_all(&plan.root)
        .map_err(|error| format!("cannot create {}: {error}", plan.root.display()))?;
    let mut written = Vec::new();
    for entry in &plan.entries {
        let source = plan.stage.join(&entry.path);
        let target = plan.root.join(&entry.path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
        }
        std::fs::copy(&source, &target)
            .map_err(|error| format!("cannot install {}: {error}", entry.path))?;
        written.push(entry.path.clone());
    }
    let mut files = Vec::new();
    for entry in &plan.entries {
        let mut object = JsonObject::new();
        object.insert("path", Json::string(entry.path.clone()));
        object.insert("bytes", Json::number(entry.bytes));
        object.insert("digest", Json::string(entry.digest.clone()));
        files.push(Json::Object(object));
    }
    let mut record = JsonObject::new();
    record.insert("files", Json::array(files));
    let record_path = plan.root.join(INSTALL_RECORD);
    std::fs::write(&record_path, Json::Object(record).render()).map_err(|error| {
        format!(
            "the install record {} could not be written: {error}",
            record_path.display()
        )
    })?;
    let mut lines = vec![format!(
        "installed {} file(s) into {}",
        written.len(),
        plan.root.display()
    )];
    lines.push(
        "settings and user projects are outside the install record and were not touched".to_owned(),
    );
    Ok(lines)
}

/// Removes exactly the files an install recorded, and nothing else.
pub fn uninstall(root: &Path) -> Result<Vec<String>, String> {
    let record_path = root.join(INSTALL_RECORD);
    let text = std::fs::read_to_string(&record_path)
        .map_err(|error| format!("cannot read {}: {error}", record_path.display()))?;
    let parsed = parse(&text)
        .map_err(|error| format!("{} is not valid JSON: {error}", record_path.display()))?;
    let files = parsed
        .as_object()
        .and_then(|object| object.get("files"))
        .and_then(Json::as_array)
        .ok_or_else(|| format!("{} has no files array", record_path.display()))?;
    let mut lines = Vec::new();
    for file in files {
        let path = file
            .as_object()
            .and_then(|object| object.get("path"))
            .and_then(Json::as_str)
            .ok_or_else(|| "an install record entry has no path".to_owned())?;
        safe_relative(path)?;
        if path == PROJECT_FILE_NAME {
            return Err(format!(
                "the install record names {PROJECT_FILE_NAME}; refusing to delete a project"
            ));
        }
        let target = root.join(path);
        match std::fs::remove_file(&target) {
            Ok(()) => lines.push(format!("removed {path}")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                lines.push(format!("already absent ({path})"));
            }
            Err(error) => return Err(format!("cannot remove {path}: {error}")),
        }
    }
    std::fs::remove_file(&record_path)
        .map_err(|error| format!("cannot remove {}: {error}", record_path.display()))?;
    // A language directory left empty by the removals is removed; a non-empty one is kept.
    let language = root.join("language");
    if language.is_dir()
        && std::fs::read_dir(&language)
            .map(|mut entries| entries.next().is_none())
            .unwrap_or(false)
    {
        std::fs::remove_dir(&language)
            .map_err(|error| format!("cannot remove {}: {error}", language.display()))?;
    }
    lines
        .push("kept: user projects, settings and every path the install did not record".to_owned());
    Ok(lines)
}

/// The compatibility limits an offline install has, for the release notes to copy verbatim.
pub fn boundaries() -> [&'static str; 5] {
    [
        "installs from a staged directory only; no network is used or needed",
        "the stage's SHA-256 digests prove integrity, not a publisher signature",
        "no operating-system uninstaller is registered; uninstall removes the install record only",
        "the runtime language packs ship in the stage; a stage without them is refused",
        "settings and user projects live outside the install root and are never removed",
    ]
}

/// Refuses an absolute path or one that leaves the root.
fn safe_relative(path: &str) -> Result<(), String> {
    if path.is_empty() || path.starts_with('/') || path.contains('\\') {
        return Err(format!("{path:?} is not a relative install path"));
    }
    if path
        .split('/')
        .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(format!("{path:?} leaves the install root"));
    }
    Ok(())
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
    use super::{INSTALL_RECORD, boundaries, install, plan, uninstall};
    use crate::app::release::Freeze;
    use crate::app::release::stage::stage;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "rustage-install-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&path).expect("a scratch directory");
        path
    }

    fn freeze() -> Freeze {
        let mut freeze = Freeze::new();
        freeze.revision = "rev".to_owned();
        freeze.package_version = "0.1.0".to_owned();
        freeze.generator = "0.1.0".to_owned();
        freeze.widget_revision = "widgets".to_owned();
        freeze
    }

    /// A stage with a binary and two language packs.
    fn staged(name: &str) -> (PathBuf, PathBuf) {
        let root = scratch(name);
        let source = root.join("source");
        let language = source.join("language");
        std::fs::create_dir_all(&language).expect("a language directory");
        std::fs::write(source.join("rustage"), b"a binary").expect("a binary");
        std::fs::write(language.join("en.json"), b"{}").expect("a pack");
        std::fs::write(language.join("zh-Hans.json"), b"{}").expect("a pack");
        let stage_directory = root.join("stage");
        stage(
            &source.join("rustage"),
            &language,
            &stage_directory,
            &freeze(),
        )
        .expect("a stage");
        (root, stage_directory)
    }

    #[test]
    fn an_install_copies_the_stage_and_an_uninstall_keeps_the_user_data() {
        let (root, stage_directory) = staged("round-trip");
        let install_root = root.join("install");
        let plan = plan(&stage_directory, &install_root).expect("a plan");
        assert_eq!(plan.entries.len(), 3);
        install(&plan).expect("an install");
        assert!(install_root.join("rustage").is_file());
        assert!(install_root.join("language/en.json").is_file());
        assert!(install_root.join(INSTALL_RECORD).is_file());

        // A user project and settings sit beside the install.
        std::fs::write(install_root.join("project.json"), b"{}").expect("a project");
        std::fs::write(install_root.join("settings.json"), b"{}").expect("settings");

        let lines = uninstall(&install_root).expect("an uninstall");
        assert!(lines.iter().any(|line| line.contains("kept")));
        assert!(!install_root.join("rustage").exists());
        assert!(
            !install_root.join("language").exists(),
            "the empty directory goes"
        );
        assert!(
            install_root.join("project.json").is_file(),
            "a project is not deleted"
        );
        assert!(
            install_root.join("settings.json").is_file(),
            "settings are kept"
        );
        assert!(!install_root.join(INSTALL_RECORD).exists());
    }

    #[test]
    fn an_install_over_a_project_is_refused_before_anything_is_written() {
        let (root, stage_directory) = staged("over-project");
        let install_root = root.join("project");
        std::fs::create_dir_all(&install_root).expect("a project directory");
        std::fs::write(install_root.join("project.json"), b"{}").expect("a project");
        let error = plan(&stage_directory, &install_root).expect_err("a refusal");
        assert!(error.contains("is a project"), "{error}");
        assert!(!install_root.join("rustage").exists());
    }

    #[test]
    fn a_stage_whose_digest_does_not_match_is_refused() {
        let (root, stage_directory) = staged("digest");
        std::fs::write(stage_directory.join("rustage"), b"tampered").expect("a change");
        let error = plan(&stage_directory, &root.join("install")).expect_err("a refusal");
        assert!(error.contains("does not match its digest"), "{error}");
    }

    #[test]
    fn a_path_that_leaves_the_root_is_refused() {
        let (root, stage_directory) = staged("escape");
        let manifest =
            std::fs::read_to_string(stage_directory.join("stage.json")).expect("a manifest");
        let escaped = manifest.replace("\"rustage\"", "\"../escape\"");
        std::fs::write(stage_directory.join("stage.json"), escaped).expect("a changed manifest");
        let error = plan(&stage_directory, &root.join("install")).expect_err("a refusal");
        assert!(error.contains("leaves the install root"), "{error}");
    }

    #[test]
    fn the_boundaries_name_the_missing_signature_and_network() {
        let lines = boundaries();
        assert_eq!(lines.len(), 5);
        assert!(
            lines
                .iter()
                .any(|line| line.contains("not a publisher signature"))
        );
        assert!(lines.iter().any(|line| line.contains("no network")));
    }
}
