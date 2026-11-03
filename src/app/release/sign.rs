// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Signing and notarising a release artifact with the platform's own tools (task T25.02).
//!
//! The acceptance line is *"the three desktops can be packaged, signed and notarised"*. This module
//! owns the signing half; packaging remains the platform packager's job and is documented as such.
//!
//! Rules that keep the seam honest:
//!
//! * **The tool is the platform's.** macOS uses `/usr/bin/codesign`, Windows uses `signtool`, and
//!   Linux uses `gpg`'s detached signature. The host does not implement a signature format.
//! * **No shell.** Each tool runs through [`OwnedProcess`] with an argument array, never a shell
//!   string, so a path or identity containing a space or a quote is one argument rather than
//!   syntax. Each invocation takes a [`PermissionService`] permit first, the same gate every other
//!   command goes through.
//! * **Signing changes the artifact.** macOS and Windows embed the signature in the file, so the
//!   digest and size are re-read after signing; a caller that declared the artifact before signing
//!   must declare it again with the returned facts. A Linux detached signature leaves the artifact
//!   untouched and names the sidecar.
//! * **Verification is part of signing.** The platform verifier runs immediately after, and a
//!   signature that does not verify is an error, not a reported success.
//! * **Notarisation needs credentials.** The command is real, but a missing keychain profile is
//!   reported by `notarytool` rather than invented here.

use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::contracts::permission::{EffectClass, Principal};
use crate::host::permission::{PermissionService, Scope};
use crate::host::subprocess::{OwnedProcess, ProcessSpec};

/// How long a signing tool may take.
const SIGN_TIMEOUT: Duration = Duration::from_secs(120);
/// How long a notarisation may take; the service can queue for minutes.
///
/// macOS-only because [`notarize`] is the only reader: on another target this constant would be
/// dead code that looks like a capability the build does not have.
#[cfg(target_os = "macos")]
const NOTARIZE_TIMEOUT: Duration = Duration::from_secs(600);

/// What signing produced.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Signed {
    /// The tool that signed it.
    pub tool: String,
    /// The identity the tool was given.
    pub identity: String,
    /// The artifact's SHA-256 after signing (unchanged for a detached signature).
    pub digest: String,
    /// The artifact's size after signing.
    pub bytes: u64,
    /// The detached signature, when the platform writes one.
    pub sidecar: Option<PathBuf>,
}

/// Signs `artifact` with `identity` using the platform's tool, then verifies it.
pub fn sign(artifact: &Path, identity: &str) -> Result<Signed, String> {
    if !artifact.is_file() {
        return Err(format!("{} is not a file", artifact.display()));
    }
    if identity.trim().is_empty() {
        return Err("a signing identity is required".to_owned());
    }
    let permit = PermissionService::new()
        .permit(EffectClass::Command, Scope::Session, Principal::Human)
        .map_err(|refusal| refusal.message())?;

    let (tool, sidecar) = if cfg!(target_os = "macos") {
        run(
            &permit,
            "/usr/bin/codesign",
            &["--force", "--sign", identity, &artifact.to_string_lossy()],
            SIGN_TIMEOUT,
        )?;
        run(
            &permit,
            "/usr/bin/codesign",
            &["--verify", "--strict", &artifact.to_string_lossy()],
            SIGN_TIMEOUT,
        )?;
        ("codesign".to_owned(), None)
    } else if cfg!(target_os = "windows") {
        run(
            &permit,
            "signtool",
            &["sign", "/fd", "SHA256", "/a", &artifact.to_string_lossy()],
            SIGN_TIMEOUT,
        )?;
        run(
            &permit,
            "signtool",
            &["verify", "/pa", &artifact.to_string_lossy()],
            SIGN_TIMEOUT,
        )?;
        ("signtool".to_owned(), None)
    } else {
        let sidecar = path_with_suffix(artifact, ".asc");
        run(
            &permit,
            "gpg",
            &[
                "--batch",
                "--yes",
                "--armor",
                "--detach-sign",
                "--local-user",
                identity,
                "--output",
                &sidecar.to_string_lossy(),
                &artifact.to_string_lossy(),
            ],
            SIGN_TIMEOUT,
        )?;
        run(
            &permit,
            "gpg",
            &[
                "--verify",
                &sidecar.to_string_lossy(),
                &artifact.to_string_lossy(),
            ],
            SIGN_TIMEOUT,
        )?;
        ("gpg".to_owned(), Some(sidecar))
    };

    let bytes = std::fs::read(artifact)
        .map_err(|error| format!("cannot read {} after signing: {error}", artifact.display()))?;
    Ok(Signed {
        tool,
        identity: identity.to_owned(),
        digest: digest_of(&bytes),
        bytes: bytes.len() as u64,
        sidecar,
    })
}

/// Submits `artifact` to Apple's notary service and staples the ticket.
///
/// `profile` is a `notarytool` keychain profile the user created with their Apple credentials; the
/// host does not store or ask for them. A missing profile is reported by `notarytool`.
#[cfg(target_os = "macos")]
pub fn notarize(artifact: &Path, profile: &str) -> Result<String, String> {
    if !artifact.is_file() {
        return Err(format!("{} is not a file", artifact.display()));
    }
    if profile.trim().is_empty() {
        return Err("a notarytool keychain profile is required".to_owned());
    }
    let permit = PermissionService::new()
        .permit(EffectClass::Command, Scope::Session, Principal::Human)
        .map_err(|refusal| refusal.message())?;
    run(
        &permit,
        "/usr/bin/xcrun",
        &[
            "notarytool",
            "submit",
            "--wait",
            "--keychain-profile",
            profile,
            &artifact.to_string_lossy(),
        ],
        NOTARIZE_TIMEOUT,
    )?;
    run(
        &permit,
        "/usr/bin/xcrun",
        &["stapler", "staple", &artifact.to_string_lossy()],
        NOTARIZE_TIMEOUT,
    )?;
    Ok(format!("{} was notarised and stapled", artifact.display()))
}

/// Notarisation is Apple's service; other platforms refuse rather than pretend.
#[cfg(not(target_os = "macos"))]
pub fn notarize(_artifact: &Path, _profile: &str) -> Result<String, String> {
    Err("notarisation is only available on macOS (notarytool)".to_owned())
}

/// Runs one tool, refusing a non-zero exit with its stderr.
pub(super) fn run(
    permit: &crate::host::permission::ExecutionPermit,
    program: &str,
    args: &[&str],
    timeout: Duration,
) -> Result<(), String> {
    let spec = ProcessSpec::new("release signing", program)
        .args(args.iter().copied())
        .capture_limit(16_384);
    let mut process = OwnedProcess::spawn(spec, permit)
        .map_err(|error| format!("{program} could not be started: {}", error.message()))?;
    let Some(exit) = process.wait_for(timeout) else {
        return Err(format!("{program} did not finish within {timeout:?}"));
    };
    if exit.is_success() {
        return Ok(());
    }
    let output = process.output_so_far();
    let detail = output.stderr.trim();
    Err(if detail.is_empty() {
        format!("{program} failed: {}", exit.message())
    } else {
        format!("{program} failed: {}: {detail}", exit.message())
    })
}

/// `artifact` with `suffix` appended to its whole name.
fn path_with_suffix(artifact: &Path, suffix: &str) -> PathBuf {
    let mut name = artifact.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
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
    use super::{digest_of, path_with_suffix, sign};
    use std::path::PathBuf;

    #[cfg(target_os = "macos")]
    fn scratch(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "rustage-sign-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&path).expect("a scratch directory");
        path
    }

    #[test]
    fn a_detached_signature_suffix_is_appended_to_the_whole_file_name() {
        assert_eq!(
            path_with_suffix(std::path::Path::new("/tmp/rustage.tar.gz"), ".asc"),
            PathBuf::from("/tmp/rustage.tar.gz.asc")
        );
        assert_eq!(digest_of(b"abc").len(), 64);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn an_adhoc_signature_signs_and_verifies_a_real_file() {
        if !std::path::Path::new("/usr/bin/codesign").is_file() {
            return;
        }
        let root = scratch("adhoc");
        let artifact = root.join("artifact");
        std::fs::write(&artifact, b"a release artifact").expect("an artifact");
        let signed = sign(&artifact, "-").expect("an ad-hoc signature");
        assert_eq!(signed.tool, "codesign");
        assert_eq!(signed.bytes, 18);
        assert_eq!(signed.digest.len(), 64);
        assert_eq!(signed.sidecar, None);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_missing_artifact_is_refused_before_any_tool_runs() {
        let error = sign(std::path::Path::new("/definitely/not/here"), "-").expect_err("a refusal");
        assert!(error.contains("is not a file"), "{error}");
        let error =
            sign(std::path::Path::new("/definitely/not/here"), "  ").expect_err("a refusal");
        assert!(
            error.contains("is not a file") || error.contains("identity"),
            "{error}"
        );
    }
}
