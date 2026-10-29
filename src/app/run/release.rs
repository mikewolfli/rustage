// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The `--release` command: freeze, artifacts, SBOM, updates and maintenance (task F25).

use std::collections::BTreeMap;
use std::path::Path;

use crate::app::release::update_journal::{Journal, Record};
use crate::app::release::{
    Artifact, Channel, Component, Freeze, Maintenance, Migration, Repository, Sbom, Signature,
    Update, lockfile,
};
use crate::contracts::revision::FormatVersion;

/// Runs the release operations, printing each answer as it is read.
pub(super) fn release(options: &crate::cli::ReleaseOptions) -> bool {
    let mut freeze = Freeze::new();
    let mut sbom = Sbom::new();
    let mut licences: Vec<String> = Vec::new();
    // Licences supplied for lockfile packages, keyed by `name@version`.
    let mut package_licences: BTreeMap<String, String> = BTreeMap::new();
    let mut artifact: Option<Artifact> = None;
    let mut update: Option<Update> = None;
    let mut channel = Channel::Candidate;
    let mut maintenance: Option<Maintenance> = None;
    // The journal a declared `journal:<file>` writes every transition to, with the last record it
    // wrote, so an op that changes nothing does not rewrite the file.
    let mut journal: Option<Journal> = None;
    let mut journalled: Option<Record> = None;
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
                            licenses: licences.clone(),
                        });
                        Ok(vec![format!("artifact {name} ({signature:?})")])
                    }
                    _ => Err(format!(
                        "artifact needs <name>:<digest>:<size>:<signature>, got {argument:?}"
                    )),
                }
            }
            "license" => {
                licences.push(argument.to_owned());
                Ok(vec![format!("license {argument}")])
            }
            "package-license" => match argument.split_once(':') {
                Some((identity, licence)) if !identity.is_empty() => {
                    package_licences.insert(identity.to_owned(), licence.to_owned());
                    Ok(vec![format!("package licence {identity}: {licence}")])
                }
                _ => Err(format!(
                    "package-license needs <name>@<version>:<licence>, got {argument:?}"
                )),
            },
            "sbom-lock" => match argument.split_once(':') {
                Some((lock_path, output)) => {
                    let text = match std::fs::read_to_string(lock_path) {
                        Ok(text) => text,
                        Err(error) => {
                            return super::report(format!("cannot read {lock_path}: {error}"));
                        }
                    };
                    let packages = match lockfile::parse(&text) {
                        Ok(packages) => packages,
                        Err(reason) => return super::report(reason),
                    };
                    let (mut derived, coverage) = lockfile::sbom(&packages, &package_licences);
                    // The artifact this release produced belongs in the bill of materials too, with
                    // the digest the release actually declared — that is the artifact↔SBOM link.
                    if let Some(artifact) = &artifact {
                        let version = if freeze.package_version.is_empty() {
                            env!("CARGO_PKG_VERSION").to_owned()
                        } else {
                            freeze.package_version.clone()
                        };
                        let licence = if artifact.licenses.is_empty() {
                            "MIT".to_owned()
                        } else {
                            artifact.licenses.join(" AND ")
                        };
                        if let Err(reason) = derived.add(Component {
                            name: artifact.name.clone(),
                            version,
                            license: licence,
                            source: "release artifact".to_owned(),
                            digest: Some(artifact.digest.clone()),
                        }) {
                            return super::report(reason);
                        }
                        let linked = derived.components.values().any(|component| {
                            component.digest.as_deref() == Some(artifact.digest.as_str())
                        });
                        if !linked {
                            return super::report(format!(
                                "the SBOM does not carry the artifact digest {}",
                                artifact.digest
                            ));
                        }
                    }
                    if let Err(error) = std::fs::write(output, derived.json()) {
                        return super::report(format!("cannot write {output}: {error}"));
                    }
                    let mut lines = vec![coverage.render()];
                    if coverage.is_complete() {
                        lines.push("every package has a digest and a recorded licence".to_owned());
                    }
                    lines.push(format!(
                        "wrote {} component(s) to {output}",
                        derived.components.len()
                    ));
                    Ok(lines)
                }
                None => Err(format!(
                    "sbom-lock needs <Cargo.lock>:<output>, got {argument:?}"
                )),
            },
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
                    license: if licences.is_empty() {
                        "MIT".to_owned()
                    } else {
                        licences.join(" AND ")
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
            "confirm" => match &mut update {
                Some(update) => update.confirm().map(|()| vec![update.render()]),
                None => Err("plan an update first".to_owned()),
            },
            "interrupt" => match &mut update {
                Some(update) => update.interrupt().map(|()| vec![update.render()]),
                None => Err("plan an update first".to_owned()),
            },
            "begin-install" => match &mut update {
                Some(update) => update
                    .begin_install()
                    .map(|backup| vec![format!("backup {backup}"), update.render()]),
                None => Err("plan an update first".to_owned()),
            },
            "complete-install" => match &mut update {
                Some(update) => update.complete_install().map(|()| vec![update.render()]),
                None => Err("plan an update first".to_owned()),
            },
            "resume" => match &mut update {
                Some(update) => update.resume().map(|()| vec![update.render()]),
                None => Err("plan an update first".to_owned()),
            },
            "recover" => match &mut update {
                Some(update) => update.recover().map(|()| vec![update.render()]),
                None => Err("plan an update first".to_owned()),
            },
            "migration" => match &mut update {
                Some(update) => match argument.split_once(':') {
                    Some((from, rest)) => match rest.split_once(':') {
                        Some((to, steps)) => match (from.parse::<u32>(), to.parse::<u32>()) {
                            (Ok(from), Ok(to)) => {
                                let steps: Vec<String> = steps
                                    .split('|')
                                    .filter(|step| !step.trim().is_empty())
                                    .map(str::to_owned)
                                    .collect();
                                match Migration::plan(from, to, steps) {
                                    Ok(migration) => {
                                        let line = migration.render();
                                        match update.set_migration(migration) {
                                            Ok(()) => Ok(vec![line]),
                                            Err(reason) => Err(reason),
                                        }
                                    }
                                    Err(reason) => Err(reason),
                                }
                            }
                            _ => Err(format!(
                                "migration needs <from-format>:<to-format>:<step>[|<step>...], got {argument:?}"
                            )),
                        },
                        None => Err(format!(
                            "migration needs <from-format>:<to-format>:<step>[|<step>...], got {argument:?}"
                        )),
                    },
                    None => Err(format!(
                        "migration needs <from-format>:<to-format>:<step>[|<step>...], got {argument:?}"
                    )),
                },
                None => Err("plan an update first".to_owned()),
            },
            "ack-migration" => match &mut update {
                Some(update) => update
                    .acknowledge_migration()
                    .map(|()| vec![update.render()]),
                None => Err("plan an update first".to_owned()),
            },
            "rollback" => match &mut update {
                Some(update) => update
                    .rollback()
                    .map(|()| vec!["update rolled back".to_owned()]),
                None => Err("plan an update first".to_owned()),
            },
            "journal" => {
                if argument.is_empty() {
                    return super::report("journal needs <file>".to_owned());
                }
                let declared = Journal::at(argument);
                let line = format!("journal {}", declared.path().display());
                journal = Some(declared);
                journalled = None;
                Ok(vec![line])
            }
            "journal-recover" => {
                if argument.is_empty() {
                    return super::report("journal-recover needs <file>".to_owned());
                }
                match Journal::at(argument).recover_interrupted() {
                    Ok(Some(record)) => Ok(record.render()),
                    Ok(None) => Ok(vec!["no interrupted install".to_owned()]),
                    Err(reason) => Err(reason),
                }
            }
            "journal-clear" => {
                if argument.is_empty() {
                    return super::report("journal-clear needs <file>".to_owned());
                }
                match Journal::at(argument).clear() {
                    Ok(()) => Ok(vec![format!("journal {argument} cleared")]),
                    Err(reason) => Err(reason),
                }
            }
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
                // A declared journal records every transition, so a process that dies mid-install
                // leaves an `Installing` record for the next one to recover.
                if let (Some(journal), Some(update)) = (&journal, &update) {
                    let record = Record::of(update);
                    if journalled.as_ref() != Some(&record) {
                        if let Err(reason) = journal.write(&record) {
                            return super::report(reason);
                        }
                        journalled = Some(record);
                    }
                }
            }
            Err(reason) => return super::report(reason),
        }
    }
    true
}
