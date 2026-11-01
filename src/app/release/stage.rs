// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Staging a release directory: the binary, the runtime language packs, and a digest manifest
//! (task T25.02, the layout every desktop installer packages).
//!
//! A stage is not an installer, a signature or a notarisation. It is the directory those steps take
//! as input, produced deterministically and with the same SHA-256 the release artifacts use, so an
//! installer can be checked before and after signing. The manifest names every copied file, its
//! byte size and its digest; a file added to `language/` later is not silently part of the release.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::Freeze;
use crate::project::record::json::{Json, JsonObject};

/// One file copied into the stage.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StagedFile {
    /// Path relative to the stage directory, with `/` separators.
    pub path: String,
    /// Size in bytes.
    pub bytes: u64,
    /// Lower-case SHA-256 of the file's bytes.
    pub digest: String,
}

/// A completed stage.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Stage {
    /// Where the stage was written.
    pub directory: PathBuf,
    /// Every file in the stage, in path order.
    pub files: Vec<StagedFile>,
}

impl Stage {
    /// The stage manifest as stable JSON.
    ///
    /// The shape is `{"files": [{"path","bytes","digest"}], "freeze": {...}}`; the file list is
    /// sorted by path so two stages of the same inputs produce the same bytes.
    pub fn manifest(&self, freeze: &Freeze) -> String {
        let mut files = Vec::with_capacity(self.files.len());
        for file in &self.files {
            let mut object = JsonObject::new();
            object.insert("path", Json::string(file.path.clone()));
            object.insert("bytes", Json::number(file.bytes));
            object.insert("digest", Json::string(file.digest.clone()));
            files.push(Json::Object(object));
        }
        let mut frozen = JsonObject::new();
        frozen.insert("revision", Json::string(freeze.revision.clone()));
        frozen.insert(
            "package_version",
            Json::string(freeze.package_version.clone()),
        );
        frozen.insert("generator", Json::string(freeze.generator.clone()));
        frozen.insert(
            "widget_revision",
            Json::string(freeze.widget_revision.clone()),
        );
        let mut root = JsonObject::new();
        root.insert("files", Json::array(files));
        root.insert("freeze", Json::Object(frozen));
        Json::Object(root).render()
    }
}

/// Copies `binary` and the runtime language packs into `output`.
///
/// `language/` is runtime data (blue21 L-1), so a stage without it would launch with keys instead
/// of labels. Pseudo-localisation files are development artifacts and are not staged. A missing
/// language directory is a refusal, not an empty directory: a release that claims a language and
/// ships no pack is exactly the failure this stage exists to catch.
pub fn stage(
    binary: &Path,
    language_directory: &Path,
    output: &Path,
    freeze: &Freeze,
) -> Result<Stage, String> {
    if !binary.is_file() {
        return Err(format!("{} is not a file", binary.display()));
    }
    if !language_directory.is_dir() {
        return Err(format!(
            "{} is not a language directory; a release needs its JSON packs",
            language_directory.display()
        ));
    }
    std::fs::create_dir_all(output)
        .map_err(|error| format!("cannot create {}: {error}", output.display()))?;

    let binary_name = binary
        .file_name()
        .ok_or_else(|| format!("{} has no file name", binary.display()))?
        .to_string_lossy()
        .into_owned();
    let mut files = Vec::new();
    copy(binary, &output.join(&binary_name), &binary_name, &mut files)?;

    let mut packs = Vec::new();
    let entries = std::fs::read_dir(language_directory)
        .map_err(|error| format!("cannot read {}: {error}", language_directory.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot read a language entry: {error}"))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".json") || name.contains(".pseudo.") {
            continue;
        }
        packs.push(name);
    }
    if packs.is_empty() {
        return Err(format!(
            "{} holds no language pack to stage",
            language_directory.display()
        ));
    }
    packs.sort();
    for name in packs {
        let source = language_directory.join(&name);
        copy(
            &source,
            &output.join("language").join(&name),
            &format!("language/{name}"),
            &mut files,
        )?;
    }

    files.sort_by(|left, right| left.path.cmp(&right.path));
    let staged = Stage {
        directory: output.to_path_buf(),
        files,
    };
    std::fs::write(output.join("stage.json"), staged.manifest(freeze))
        .map_err(|error| format!("cannot write stage.json: {error}"))?;
    Ok(staged)
}

/// Copies one file into `target` and records its place relative to the stage root.
fn copy(
    source: &Path,
    target: &Path,
    relative: &str,
    files: &mut Vec<StagedFile>,
) -> Result<(), String> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    std::fs::copy(source, target)
        .map_err(|error| format!("cannot copy {}: {error}", source.display()))?;
    let bytes = std::fs::read(target)
        .map_err(|error| format!("cannot read back {}: {error}", target.display()))?;
    let digest = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    files.push(StagedFile {
        path: relative.to_owned(),
        bytes: bytes.len() as u64,
        digest,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::stage;
    use crate::app::release::Freeze;
    use std::path::PathBuf;

    fn scratch(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("a clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("rustage-stage-{tag}-{nanos}"));
        std::fs::create_dir_all(&path).expect("the scratch directory");
        path
    }

    #[test]
    fn a_stage_copies_the_binary_and_packs_and_records_their_digests() {
        let root = scratch("ok");
        let binary = root.join("rustage");
        std::fs::write(&binary, b"binary").expect("the binary");
        let language = root.join("language");
        std::fs::create_dir_all(&language).expect("the language directory");
        std::fs::write(language.join("en.json"), b"{\"language\":\"en\"}").expect("en");
        std::fs::write(language.join("zh-Hans.json"), b"{\"language\":\"zh-Hans\"}").expect("zh");
        std::fs::write(language.join("en.pseudo.json"), b"{}").expect("pseudo");
        let output = root.join("stage");
        let mut freeze = Freeze::new();
        freeze.revision = "abc".to_owned();

        let staged = stage(&binary, &language, &output, &freeze).expect("a stage");

        assert_eq!(staged.files.len(), 3, "binary plus two real packs");
        assert!(staged.files.iter().all(|file| file.digest.len() == 64));
        assert!(output.join("rustage").is_file());
        assert!(output.join("language/en.json").is_file());
        assert!(
            !output.join("language/en.pseudo.json").exists(),
            "a pseudo pack is a development artifact"
        );
        let manifest = std::fs::read_to_string(output.join("stage.json")).expect("stage.json");
        assert!(manifest.contains("\"revision\": \"abc\""), "{manifest}");
        assert!(manifest.contains("zh-Hans.json"), "{manifest}");
        assert!(
            manifest.contains("\"path\": \"language/en.json\""),
            "a pack's manifest path is relative to the stage root: {manifest}"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_release_without_language_packs_is_refused() {
        let root = scratch("empty");
        let binary = root.join("rustage");
        std::fs::write(&binary, b"binary").expect("the binary");
        let language = root.join("language");
        std::fs::create_dir_all(&language).expect("the language directory");
        let error = stage(&binary, &language, &root.join("stage"), &Freeze::new())
            .expect_err("no packs is a refusal");
        assert!(error.contains("no language pack"), "{error}");
        let _ = std::fs::remove_dir_all(root);
    }
}
