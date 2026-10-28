// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The `--release` command: freeze, artifacts, SBOM, updates and maintenance (task F25).

use std::path::Path;

use crate::app::release::{
    Artifact, Channel, Component, Freeze, Maintenance, Repository, Sbom, Signature, Update,
};
use crate::contracts::revision::FormatVersion;

/// Runs the release operations, printing each answer as it is read.
pub(super) fn release(options: &crate::cli::ReleaseOptions) -> bool {
    let mut freeze = Freeze::new();
    let mut sbom = Sbom::new();
    let mut artifact: Option<Artifact> = None;
    let mut licenses: Vec<String> = Vec::new();
    let mut update: Option<Update> = None;
    let mut channel = Channel::Candidate;
    let mut maintenance: Option<Maintenance> = None;
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "freeze" => match argument.split_once(':') {
                Some((revision, generator)) => {
                    freeze.revision = revision.to_owned();
                    freeze.generator = generator.to_owned();
                    Ok(vec![format!("freeze {revision} generator {generator}")])
                }
                None => Err(format!(
                    "freeze needs <revision>:<generator>, got {argument:?}"
                )),
            },
            "format" => {
                freeze.formats.push(argument.to_owned());
                Ok(vec![format!("format {argument}")])
            }
            "component" => match argument.split_once(':') {
                Some((name, version)) => {
                    freeze
                        .components
                        .insert(name.to_owned(), version.to_owned());
                    Ok(vec![format!("component {name} {version}")])
                }
                None => Err(format!(
                    "component needs <name>:<version>, got {argument:?}"
                )),
            },
            "support" => {
                freeze.support_matrix = argument.to_owned();
                Ok(vec![format!("support matrix {argument}")])
            }
            "support-digest" => {
                freeze.support_matrix_digest = argument.to_owned();
                Ok(vec![format!("support matrix digest {argument}")])
            }
            "package" => {
                freeze.package_version = argument.to_owned();
                Ok(vec![format!("package {argument}")])
            }
            "widget-revision" => {
                freeze.widget_revision = argument.to_owned();
                Ok(vec![format!("rust_widgets revision {argument}")])
            }
            "document-format" => match argument.parse::<u32>() {
                Ok(value) => {
                    freeze.document_format = FormatVersion::from_raw(value);
                    Ok(vec![format!("document format v{value}")])
                }
                Err(_) => Err(format!("{argument:?} is not a document format version")),
            },
            "protocol" => match argument.parse::<u32>() {
                Ok(value) => {
                    freeze.protocol_version = value;
                    Ok(vec![format!("protocol {value}")])
                }
                Err(_) => Err(format!("{argument:?} is not a protocol version")),
            },
            "freeze-check" => {
                let root = if argument.is_empty() { "." } else { argument };
                match Repository::read(Path::new(root)) {
                    Ok(repository) => {
                        let report = freeze.verify_against(&repository);
                        for line in report.render() {
                            println!("{line}");
                        }
                        if report.passed() {
                            Ok(vec![format!(
                                "freeze verified: {} verified, {} blocked, 0 failed",
                                report.verified.len(),
                                report.blocked.len()
                            )])
                        } else {
                            return super::report(format!(
                                "freeze check failed: {} item(s) do not match the repository",
                                report.failed.len()
                            ));
                        }
                    }
                    Err(reason) => Err(reason),
                }
            }
            "artifact" => {
                let parts: Vec<&str> = argument.splitn(4, ':').collect();
                match parts.as_slice() {
                    [name, digest, size, signature] => {
                        let signature = match *signature {
                            "signed" | "verified" => Signature::Verified,
                            "unsigned" => Signature::Unsigned,
                            other => {
                                return super::report(format!(
                                    "{other:?} is not signed or unsigned"
                                ));
                            }
                        };
                        let size: u64 = match size.parse() {
                            Ok(size) => size,
                            Err(_) => {
                                return super::report(format!("{size:?} is not a byte size"));
                            }
                        };
                        artifact = Some(Artifact {
                            name: (*name).to_owned(),
                            digest: (*digest).to_owned(),
                            size,
                            signature,
                            licenses: licenses.clone(),
                        });
                        Ok(vec![format!("artifact {name} ({signature:?})")])
                    }
                    _ => Err(format!(
                        "artifact needs <name>:<digest>:<size>:<signature>, got {argument:?}"
                    )),
                }
            }
            "license" => {
                licenses.push(argument.to_owned());
                Ok(vec![format!("license {argument}")])
            }
            "verify-file" => {
                let bytes = match std::fs::read(argument) {
                    Ok(bytes) => bytes,
                    Err(error) => return super::report(format!("cannot read {argument}: {error}")),
                };
                if let Some(artifact) = &artifact {
                    match artifact.verify(&bytes) {
                        Ok(()) => Ok(vec![format!(
                            "{} verified ({} byte(s))",
                            artifact.name,
                            bytes.len()
                        )]),
                        Err(reason) => Err(reason),
                    }
                } else if let Some(update) = &mut update {
                    match update.verify(&bytes) {
                        Ok(()) => Ok(vec![format!("update {} verified", update.to)]),
                        Err(reason) => Err(reason),
                    }
                } else {
                    Err("declare an artifact or an update first".to_owned())
                }
            }
            "sbom" => {
                let digest = artifact
                    .as_ref()
                    .map(|artifact| artifact.digest.clone())
                    .unwrap_or_default();
                let component = Component {
                    name: "rustage".to_owned(),
                    version: freeze.generator.clone(),
                    license: if licenses.is_empty() {
                        "MIT".to_owned()
                    } else {
                        licenses.join(" AND ")
                    },
                    source: freeze.revision.clone(),
                    digest: if digest.is_empty() {
                        None
                    } else {
                        Some(digest)
                    },
                };
                if let Err(reason) = sbom.add(component) {
                    return super::report(reason);
                }
                match std::fs::write(argument, sbom.json()) {
                    Ok(()) => Ok(vec![format!("wrote SBOM to {argument}")]),
                    Err(error) => Err(format!("cannot write {argument}: {error}")),
                }
            }
            "update" => match argument.split_once(':') {
                Some((from, to)) => match &artifact {
                    Some(artifact) => match Update::plan(from, to, artifact.clone()) {
                        Ok(planned) => {
                            let line = planned.render();
                            update = Some(planned);
                            Ok(vec![line])
                        }
                        Err(reason) => Err(reason),
                    },
                    None => {
                        Err("declare artifact:<name>:<digest>:<size>:<signature> first".to_owned())
                    }
                },
                None => Err(format!("update needs <from>:<to>, got {argument:?}")),
            },
            "verify-update" => {
                let bytes = match std::fs::read(argument) {
                    Ok(bytes) => bytes,
                    Err(error) => return super::report(format!("cannot read {argument}: {error}")),
                };
                match &mut update {
                    Some(update) => match update.verify(&bytes) {
                        Ok(()) => Ok(vec![format!("update {} verified", update.to)]),
                        Err(reason) => Err(reason),
                    },
                    None => Err("plan an update first".to_owned()),
                }
            }
            "install" => match &mut update {
                Some(update) => update.install().map(|backup| vec![backup]),
                None => Err("plan an update first".to_owned()),
            },
            "rollback" => match &mut update {
                Some(update) => update
                    .rollback()
                    .map(|()| vec!["update rolled back".to_owned()]),
                None => Err("plan an update first".to_owned()),
            },
            "channel" => match Channel::parse(argument) {
                Some(parsed) => {
                    channel = parsed;
                    Ok(vec![format!("channel {}", channel.name())])
                }
                None => Err(format!("{argument:?} is not a release channel")),
            },
            "maintenance" => match argument.split_once(':') {
                Some((branch, matrix)) => {
                    let candidate = Maintenance {
                        branch: branch.to_owned(),
                        support_matrix: matrix.to_owned(),
                        limits: Default::default(),
                    };
                    if let Err(reason) = candidate.validate() {
                        return super::report(reason);
                    }
                    maintenance = Some(candidate);
                    Ok(vec![format!("maintenance {branch} ({matrix})")])
                }
                None => Err(format!(
                    "maintenance needs <branch>:<matrix>, got {argument:?}"
                )),
            },
            "limit" => match &mut maintenance {
                Some(maintenance) => {
                    maintenance.limits.insert(argument.to_owned());
                    Ok(vec![format!("limit {argument}")])
                }
                None => Err("declare maintenance:<branch>:<matrix> first".to_owned()),
            },
            "show" => {
                let mut lines = freeze.render();
                lines.push(format!("channel: {}", channel.name()));
                lines.push(format!("sbom components: {}", sbom.names().join(", ")));
                if let Some(artifact) = &artifact {
                    lines.push(format!(
                        "artifact: {} {} byte(s) {}",
                        artifact.name,
                        artifact.size,
                        artifact.signature.name()
                    ));
                }
                if let Some(update) = &update {
                    lines.push(format!("update: {}", update.render()));
                }
                if let Some(maintenance) = &maintenance {
                    lines.push(format!(
                        "maintenance: {} ({}) limits {}",
                        maintenance.branch,
                        maintenance.support_matrix,
                        maintenance.limits.len()
                    ));
                }
                Ok(lines)
            }
            other => Err(format!("{other:?} is not a release operation")),
        };
        match step {
            Ok(lines) => {
                for line in lines {
                    println!("{line}");
                }
            }
            Err(reason) => return super::report(reason),
        }
    }
    true
}
