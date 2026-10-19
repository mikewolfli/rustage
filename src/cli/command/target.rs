// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The multi-target family's commands: one profile's configuration, the capability snapshot and
//! switch markers, the desktop matrix, the mobile target, the restricted targets, local delivery,
//! the queue and the certification report.

// Every branch of the flag table answers with `return`, including the ones that could be a trailing
// expression: the branches are independent flags, and writing them alike is worth more than saving a
// keyword.
#![allow(clippy::needless_return)]

use super::*;

/// How to configure one output target (task T18.01).
#[derive(Debug, PartialEq, Eq)]
pub struct TargetConfigOptions {
    /// The operations: `profile:<name>`, `triple:<triple>`, `toolchain:<version>`, `sdk:<name>`,
    /// `mode:<one|two>`, `runner:<host|simulator|device|none>`, `host:<system>`, `check`, `show`.
    pub ops: Vec<String>,
}

impl TargetConfigOptions {
    /// Options with no overrides.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to capture, compare and apply a capability snapshot (task T18.02).
#[derive(Debug, PartialEq, Eq)]
pub struct TargetCapabilityOptions {
    /// The operations: `generator:<version>`, `built-for:<profile>`, `name:<kind>` (repeatable),
    /// `read:<path>`, `save:<path>`, `compare:<path>`, `node:<kind>:<children>`,
    /// `switch:<from>:<to>`, `stale:<version>`, `check`, `show`.
    pub ops: Vec<String>,
}

impl TargetCapabilityOptions {
    /// Options with no names and no switch.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to fill the desktop build/behaviour matrix (task T18.03).
#[derive(Debug, PartialEq, Eq)]
pub struct TargetMatrixOptions {
    /// The operations: `start:<triple>`, `pass:<triple>:<proof>:<note>`, `fail:<triple>:<reason>`,
    /// `cancel:<triple>`, `check`, `show`.
    pub ops: Vec<String>,
}

impl TargetMatrixOptions {
    /// Options with the committed matrix and nothing recorded.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to check a tablet/mobile target (task T18.04).
#[derive(Debug, PartialEq, Eq)]
pub struct TargetMobileOptions {
    /// The operations: `profile:<tablet|mobile>`, `size:<width>x<height>`,
    /// `input:<touch|pointer>:<width>x<height>` (repeatable), `asset:<name>:<kind>:<bytes>`,
    /// `sdk:<ios|android>:<yes|no>[:<version>]` or `sdk:<ios|android>:auto`, `host:<system>`,
    /// `evidence:<simulator|device>:<name>:<proof>:<note>`, `check`, `show`.
    pub ops: Vec<String>,
}

impl TargetMobileOptions {
    /// Options with no inputs and no evidence.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to check a mini/embedded target and audit its links (task T18.05).
#[derive(Debug, PartialEq, Eq)]
pub struct TargetRestrictedOptions {
    /// The operations: `profile:<mini|embedded>`, `children:<n>`, `mode:<one|two>`,
    /// `feature:<name>` (repeatable), `tree:<path>`, `dev:<crate>` (repeatable),
    /// `build:<yes|no>`, `check`, `show`.
    pub ops: Vec<String>,
}

impl TargetRestrictedOptions {
    /// Options with no design and no build.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to discover a local device, export and plan an install (task T18.06).
#[derive(Debug, PartialEq, Eq)]
pub struct TargetLocalOptions {
    /// The operations: `profile:<name>`, `host:<system>`, `platform:<ios|android>`,
    /// `program:<name>` (repeatable), `device:<id>:<model>:<yes|no>`, `select:<id>`,
    /// `authorize:<id>`, `discover[:<ios|android>]`, `package:<identifier>:<version>:<artifact>`,
    /// `export:<directory>`, `install`, `remote:<host>`, `fail:<step>:<reason>`,
    /// `previous:<version>`, `check`, `show`.
    pub ops: Vec<String>,
}

impl TargetLocalOptions {
    /// Options with no devices and no package.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to queue builds and deliveries and track their artifacts (task T18.07).
#[derive(Debug, PartialEq, Eq)]
pub struct TargetQueueOptions {
    /// The operations: `job:<id>:<target>:<build|deploy|sign>`,
    /// `signing:<identity>:<credential>:<environment|keychain|file>`, `start:<id>`,
    /// `sign:<id>:<identity>:<artifact>:<credential>:<source>`, `host:<system>`,
    /// `pass:<id>:<proof>[:<artifact>]`, `fail:<id>:<reason>`, `cancel:<id>`, `secret:<value>`,
    /// `log:<text>`, `check`, `show`.
    pub ops: Vec<String>,
}

impl TargetQueueOptions {
    /// Options with an empty queue.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to fill the five-profile certification report (task T18.08).
#[derive(Debug, PartialEq, Eq)]
pub struct TargetCertificationOptions {
    /// The operations: `profile:<name>`, `stage:<stage>:<proof>:<command>:<note>`,
    /// `consistency`, `check`, `show`.
    pub ops: Vec<String>,
}

impl TargetCertificationOptions {
    /// Options with every cell unverified.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// Parses this family's flags, answering `None` when the flag is another family's.
pub(super) fn parse_target(
    text: &str,
    _argument: OsString,
    arguments: &mut std::vec::IntoIter<OsString>,
) -> Option<Result<Command, ParseError>> {
    macro_rules! ops {
        ($options:expr, $variant:path) => {{
            let mut options = $options;
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok($variant(options)));
        }};
    }
    match text {
        "--target-config" => ops!(TargetConfigOptions::new(), Command::TargetConfig),
        "--target-capability" => {
            ops!(TargetCapabilityOptions::new(), Command::TargetCapability)
        }
        "--target-build-matrix" => ops!(TargetMatrixOptions::new(), Command::TargetMatrix),
        "--target-mobile" => ops!(TargetMobileOptions::new(), Command::TargetMobile),
        "--target-restricted" => ops!(TargetRestrictedOptions::new(), Command::TargetRestricted),
        "--target-local" => ops!(TargetLocalOptions::new(), Command::TargetLocal),
        "--target-queue" => ops!(TargetQueueOptions::new(), Command::TargetQueue),
        "--target-certification" => {
            ops!(
                TargetCertificationOptions::new(),
                Command::TargetCertification
            )
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;

    #[test]
    fn parses_the_target_config_operations() {
        assert_eq!(
            parse(["--target-config"]),
            Ok(Command::TargetConfig(TargetConfigOptions::new())),
        );
        let mut expected = TargetConfigOptions::new();
        expected.ops = vec![
            "profile:desktop".to_owned(),
            "triple:aarch64-apple-darwin".to_owned(),
            "check".to_owned(),
        ];
        assert_eq!(
            parse([
                "--target-config",
                "profile:desktop",
                "triple:aarch64-apple-darwin",
                "check",
            ]),
            Ok(Command::TargetConfig(expected)),
        );
        assert!(parse(["--target-config", "--oops"]).is_err());
    }

    #[test]
    fn parses_each_target_flag() {
        assert_eq!(
            parse(["--target-capability"]),
            Ok(Command::TargetCapability(TargetCapabilityOptions::new())),
        );
        assert_eq!(
            parse(["--target-build-matrix"]),
            Ok(Command::TargetMatrix(TargetMatrixOptions::new())),
        );
        assert_eq!(
            parse(["--target-mobile"]),
            Ok(Command::TargetMobile(TargetMobileOptions::new())),
        );
        assert_eq!(
            parse(["--target-restricted"]),
            Ok(Command::TargetRestricted(TargetRestrictedOptions::new())),
        );
        assert_eq!(
            parse(["--target-local"]),
            Ok(Command::TargetLocal(TargetLocalOptions::new())),
        );
        assert_eq!(
            parse(["--target-queue"]),
            Ok(Command::TargetQueue(TargetQueueOptions::new())),
        );
        assert_eq!(
            parse(["--target-certification"]),
            Ok(Command::TargetCertification(
                TargetCertificationOptions::new()
            )),
        );
    }

    #[test]
    fn the_target_flags_do_not_capture_another_familys_flag() {
        // `--target` is the project family's single option; the new flags must not claim it.
        assert_ne!(parse(["--target", "mobile"]), Ok(Command::Status));
        assert!(parse(["--target-config", "profile:desktop", "--help"]).is_err());
    }
}
