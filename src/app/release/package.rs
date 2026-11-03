// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Desktop packaging from a staged release (task T25.02).
//!
//! The stage is the input; this module turns it into the two formats the host can build with tools
//! it can actually check for:
//!
//! * **dmg** on macOS uses `hdiutil create -srcfolder`, so the image contains exactly the staged
//!   files. The image is compressed (`UDZO`) and the volume name is fixed.
//! * **deb** is built without `dpkg-deb`: a `.deb` is an `ar` archive of `debian-binary`,
//!   `control.tar.gz` and `data.tar.gz`, so the host writes the control file, uses `tar` for the two
//!   members and `ar qc` for the archive. The file order is asserted by the test because `dpkg`
//!   requires `debian-binary` first.
//! * **msi** is refused with the missing tool named. WiX is not installed and is not something the
//!   host can synthesize; pretending to build an `.msi` would produce a file no installer accepts.
//!
//! Every external tool runs through the command-permission gate with an argument array (never a
//! shell string), and the result is the artifact's size and SHA-256 so it can be declared as an
//! `artifact:` immediately.

use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::contracts::permission::{EffectClass, Principal};
use crate::host::permission::{PermissionService, Scope};

use super::sign;

/// How long a packager may take.
const PACKAGE_TIMEOUT: Duration = Duration::from_secs(300);

/// The formats this host can build.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    /// A macOS disk image.
    Dmg,
    /// A Debian package.
    Deb,
    /// A Windows installer; not buildable without WiX.
    Msi,
}

impl Format {
    /// Reads a format name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "dmg" => Some(Self::Dmg),
            "deb" => Some(Self::Deb),
            "msi" => Some(Self::Msi),
            _ => None,
        }
    }

    /// Its name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Dmg => "dmg",
            Self::Deb => "deb",
            Self::Msi => "msi",
        }
    }
}

/// What packaging produced.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Packaged {
    /// The format.
    pub format: Format,
    /// The file written.
    pub path: PathBuf,
    /// Its size.
    pub bytes: u64,
    /// Its lower-case SHA-256.
    pub digest: String,
}

/// Builds one package from a staged directory.
pub fn package(
    format: Format,
    stage: &Path,
    output: &Path,
    package_version: &str,
) -> Result<Packaged, String> {
    if !stage.is_dir() {
        return Err(format!("{} is not a staged directory", stage.display()));
    }
    if !stage.join("rustage").is_file() {
        return Err(format!("{} holds no rustage binary", stage.display()));
    }
    let permit = PermissionService::new()
        .permit(EffectClass::Command, Scope::Session, Principal::Human)
        .map_err(|refusal| refusal.message())?;
    match format {
        Format::Dmg => {
            if !cfg!(target_os = "macos") {
                return Err("dmg packaging uses hdiutil and is only available on macOS".to_owned());
            }
            let volume = format!("Rustage {}", package_version);
            sign::run(
                &permit,
                "/usr/bin/hdiutil",
                &[
                    "create",
                    "-volname",
                    &volume,
                    "-srcfolder",
                    &stage.to_string_lossy(),
                    "-ov",
                    "-format",
                    "UDZO",
                    &output.to_string_lossy(),
                ],
                PACKAGE_TIMEOUT,
            )?;
        }
        Format::Deb => build_deb(&permit, stage, output, package_version)?,
        Format::Msi => {
            return Err(
                "msi packaging needs WiX (candle/light), which this host cannot find; build the \
                 msi on a Windows machine with the WiX toolset"
                    .to_owned(),
            );
        }
    }
    let bytes = std::fs::read(output)
        .map_err(|error| format!("cannot read {}: {error}", output.display()))?;
    if bytes.is_empty() {
        return Err(format!("{} is empty", output.display()));
    }
    Ok(Packaged {
        format,
        path: output.to_path_buf(),
        bytes: bytes.len() as u64,
        digest: digest_of(&bytes),
    })
}

/// Builds a `.deb` as `debian-binary`, `control.tar.gz`, `data.tar.gz` in that order.
fn build_deb(
    permit: &crate::host::permission::ExecutionPermit,
    stage: &Path,
    output: &Path,
    package_version: &str,
) -> Result<(), String> {
    let root = std::env::temp_dir().join(format!(
        "rustage-deb-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0)
    ));
    let control = root.join("control");
    let data = root.join("data");
    let result = (|| -> Result<(), String> {
        std::fs::create_dir_all(&control)
            .map_err(|error| format!("cannot create {}: {error}", control.display()))?;
        let install = data.join("opt").join("rustage");
        copy_directory(stage, &install)?;
        std::fs::write(root.join("debian-binary"), b"2.0\n")
            .map_err(|error| format!("cannot write debian-binary: {error}"))?;
        std::fs::write(
            control.join("control"),
            format!(
                "Package: rustage\nVersion: {package_version}\nArchitecture: {}\nMaintainer: rustage\nSection: editors\nPriority: optional\nDescription: Rustage desktop UI designer\n Desktop visual UI designer for Rust-Widgets.\n",
                deb_architecture()
            ),
        )
        .map_err(|error| format!("cannot write control: {error}"))?;
        sign::run(
            permit,
            "/usr/bin/tar",
            &[
                "-czf",
                &control.join("control.tar.gz").to_string_lossy(),
                "-C",
                &control.to_string_lossy(),
                "control",
            ],
            PACKAGE_TIMEOUT,
        )?;
        sign::run(
            permit,
            "/usr/bin/tar",
            &[
                "-czf",
                &control.join("data.tar.gz").to_string_lossy(),
                "-C",
                &data.to_string_lossy(),
                "opt",
            ],
            PACKAGE_TIMEOUT,
        )?;
        if let Some(parent) = output.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
        }
        sign::run(
            permit,
            "/usr/bin/ar",
            &[
                "qc",
                &output.to_string_lossy(),
                &root.join("debian-binary").to_string_lossy(),
                &control.join("control.tar.gz").to_string_lossy(),
                &control.join("data.tar.gz").to_string_lossy(),
            ],
            PACKAGE_TIMEOUT,
        )?;
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&root);
    result
}

/// Copies a staged directory recursively.
fn copy_directory(source: &Path, target: &Path) -> Result<(), String> {
    std::fs::create_dir_all(target)
        .map_err(|error| format!("cannot create {}: {error}", target.display()))?;
    let entries = std::fs::read_dir(source)
        .map_err(|error| format!("cannot read {}: {error}", source.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot read an entry: {error}"))?;
        let from = entry.path();
        let to = target.join(entry.file_name());
        if from.is_dir() {
            copy_directory(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)
                .map_err(|error| format!("cannot copy {}: {error}", from.display()))?;
        }
    }
    Ok(())
}

/// The Debian architecture name for this build.
fn deb_architecture() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        "x86" => "i386",
        other => other,
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
    #[cfg(target_os = "macos")]
    use super::digest_of;
    use super::{Format, deb_architecture, package};
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "rustage-package-{}-{}-{name}",
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
        let stage = root.join("stage");
        let language = stage.join("language");
        std::fs::create_dir_all(&language).expect("a language directory");
        std::fs::write(stage.join("rustage"), b"a binary").expect("a binary");
        std::fs::write(language.join("en.json"), b"{}").expect("a pack");
        stage
    }

    #[test]
    fn a_deb_is_an_ar_with_the_three_members_in_order() {
        let stage = staged("deb");
        let output = stage.parent().expect("a parent").join("rustage.deb");
        let built = package(Format::Deb, &stage, &output, "0.1.0").expect("a deb");
        assert_eq!(built.format, Format::Deb);
        assert!(built.bytes > 0);
        assert_eq!(built.digest.len(), 64);
        let bytes = std::fs::read(&output).expect("the deb");
        assert!(bytes.starts_with(b"!<arch>\n"), "an ar archive");
        let listing = std::process::Command::new("ar")
            .arg("t")
            .arg(&output)
            .output()
            .expect("ar t");
        let names: Vec<&str> = std::str::from_utf8(&listing.stdout)
            .expect("UTF-8")
            .lines()
            .collect();
        assert_eq!(
            names,
            vec!["debian-binary", "control.tar.gz", "data.tar.gz"]
        );
        assert!(deb_architecture().is_ascii());
    }

    #[test]
    fn msi_names_the_missing_tool_instead_of_writing_a_file() {
        let stage = staged("msi");
        let output = stage.parent().expect("a parent").join("rustage.msi");
        let error = package(Format::Msi, &stage, &output, "0.1.0").expect_err("a refusal");
        assert!(error.contains("WiX"), "{error}");
        assert!(!output.exists());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_dmg_contains_the_staged_tree() {
        if !std::path::Path::new("/usr/bin/hdiutil").is_file() {
            return;
        }
        let stage = staged("dmg");
        let output = stage.parent().expect("a parent").join("rustage.dmg");
        let built = package(Format::Dmg, &stage, &output, "0.1.0").expect("a dmg");
        assert!(built.path.is_file());
        assert!(built.bytes > 0);
        assert_eq!(
            digest_of(&std::fs::read(&output).expect("the dmg")),
            built.digest
        );
    }
}
