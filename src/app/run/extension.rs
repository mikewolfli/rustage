// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The `--extension` command: local packages, contracts and templates (task F22).

use std::collections::{BTreeMap, BTreeSet};

use crate::app::extension::{
    Host, Manifest, Package, Param, ParamKind, Registry, Template, Version,
};

/// Runs the extension/template operations, printing each answer as it is read.
pub(super) fn extension(options: &crate::cli::ExtensionOptions) -> bool {
    let mut id = String::new();
    let mut version = Version {
        major: 0,
        minor: 0,
        patch: 0,
    };
    let mut host_min = version;
    let mut host_max = Version {
        major: 0,
        minor: 2,
        patch: 0,
    };
    let mut license = String::new();
    let mut capabilities = Vec::new();
    let mut permissions = Vec::new();
    let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut registry = Registry::new();
    let mut template: Option<Template> = None;
    let host = Host {
        version: Version {
            major: 0,
            minor: 1,
            patch: 0,
        },
        capabilities: ["widget", "template"]
            .into_iter()
            .map(str::to_owned)
            .collect::<BTreeSet<_>>(),
        permissions: ["network", "fs"]
            .into_iter()
            .map(str::to_owned)
            .collect::<BTreeSet<_>>(),
    };

    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "id" => {
                id = argument.to_owned();
                Ok(vec![format!("id {id}")])
            }
            "version" => match Version::parse(argument) {
                Ok(value) => {
                    version = value;
                    Ok(vec![format!("version {}", version.render())])
                }
                Err(reason) => Err(reason),
            },
            "host-min" => match Version::parse(argument) {
                Ok(value) => {
                    host_min = value;
                    Ok(vec![format!("host-min {}", host_min.render())])
                }
                Err(reason) => Err(reason),
            },
            "host-max" => match Version::parse(argument) {
                Ok(value) => {
                    host_max = value;
                    Ok(vec![format!("host-max {}", host_max.render())])
                }
                Err(reason) => Err(reason),
            },
            "license" => {
                license = argument.to_owned();
                Ok(vec![format!("license {license}")])
            }
            "manifest-file" => {
                let text = match std::fs::read_to_string(argument) {
                    Ok(text) => text,
                    Err(error) => {
                        return super::report(format!("cannot read {argument}: {error}"));
                    }
                };
                let json = match crate::project::record::json::parse(&text) {
                    Ok(json) => json,
                    Err(error) => {
                        return super::report(format!("cannot parse {argument}: {error}"));
                    }
                };
                let manifest = match Manifest::parse(&json) {
                    Ok(manifest) => manifest,
                    Err(reason) => return super::report(reason),
                };
                id = manifest.id.clone();
                version = manifest.version;
                host_min = manifest.host_min;
                host_max = manifest.host_max;
                license = manifest.license.clone();
                capabilities = manifest.capabilities.clone();
                permissions = manifest.permissions.clone();
                Ok(vec![format!(
                    "manifest {} {}",
                    manifest.id,
                    manifest.version.render()
                )])
            }
            "capability" => {
                capabilities.push(argument.to_owned());
                Ok(vec![format!("capability {argument}")])
            }
            "permission" => {
                permissions.push(argument.to_owned());
                Ok(vec![format!("permission {argument}")])
            }
            "file" => match argument.split_once(':') {
                Some((name, text)) => {
                    files.insert(name.to_owned(), text.as_bytes().to_vec());
                    Ok(vec![format!("file {name} ({} byte(s))", text.len())])
                }
                None => Err(format!("file needs <name>:<text>, got {argument:?}")),
            },
            "digest" => {
                let package = build_package(
                    &id,
                    version,
                    host_min,
                    host_max,
                    &license,
                    &capabilities,
                    &permissions,
                    &files,
                );
                Ok(vec![format!("digest {}", package.digest())])
            }
            "install" => {
                let package = build_package(
                    &id,
                    version,
                    host_min,
                    host_max,
                    &license,
                    &capabilities,
                    &permissions,
                    &files,
                );
                let digest = package.digest();
                registry
                    .install(package, &host)
                    .map(|_| vec![format!("installed {id} {} ({digest})", version.render())])
            }
            "status" => Ok(registry.render()),
            "disable" => registry
                .disable(argument)
                .map(|_| vec![format!("disabled {argument}")]),
            "enable" => registry
                .enable(argument)
                .map(|_| vec![format!("enabled {argument}")]),
            "remove" => registry.remove(argument).map(|line| vec![line]),
            "update-preflight" => {
                let package = build_package(
                    &id,
                    version,
                    host_min,
                    host_max,
                    &license,
                    &capabilities,
                    &permissions,
                    &files,
                );
                registry
                    .preflight_update(&package, &host)
                    .map(|_| vec![format!("update {} {} is compatible", id, version.render())])
            }
            "template" => {
                template = Some(Template {
                    name: argument.to_owned(),
                    params: BTreeMap::new(),
                    files: BTreeMap::new(),
                });
                Ok(vec![format!("template {argument}")])
            }
            "param" => match argument.split_once(':') {
                Some((name, rest)) => match rest.split_once(':') {
                    Some((kind, required)) => {
                        let Some(kind) = ParamKind::parse(kind) else {
                            return super::report(format!("{kind:?} is not a parameter kind"));
                        };
                        let required = match required {
                            "required" => true,
                            "optional" => false,
                            other => {
                                return super::report(format!(
                                    "{other:?} is not required or optional"
                                ));
                            }
                        };
                        match &mut template {
                            Some(template) => {
                                template
                                    .params
                                    .insert(name.to_owned(), Param { kind, required });
                                Ok(vec![format!("param {name}")])
                            }
                            None => Err("declare template:<name> first".to_owned()),
                        }
                    }
                    None => Err(format!(
                        "param needs <name>:<kind>:<required>, got {argument:?}"
                    )),
                },
                None => Err(format!(
                    "param needs <name>:<kind>:<required>, got {argument:?}"
                )),
            },
            "template-file" => match argument.split_once(':') {
                Some((name, text)) => match &mut template {
                    Some(template) => {
                        template.files.insert(name.to_owned(), text.to_owned());
                        Ok(vec![format!("template file {name}")])
                    }
                    None => Err("declare template:<name> first".to_owned()),
                },
                None => Err(format!(
                    "template-file needs <name>:<text>, got {argument:?}"
                )),
            },
            "render" => {
                let Some(template) = &template else {
                    return super::report("declare template:<name> first".to_owned());
                };
                let mut values = BTreeMap::new();
                if !argument.is_empty() {
                    for pair in argument.split(',') {
                        match pair.split_once('=') {
                            Some((key, value)) => {
                                values.insert(key.to_owned(), value.to_owned());
                            }
                            None => return super::report(format!("{pair:?} is not key=value")),
                        }
                    }
                }
                template.render(&values).map(|rendered| {
                    rendered
                        .into_iter()
                        .flat_map(|(name, text)| {
                            let mut lines = vec![format!("--- {name}")];
                            lines.extend(text.lines().map(str::to_owned));
                            lines
                        })
                        .collect()
                })
            }
            "show" => Ok(vec![
                format!("id: {id}"),
                format!("version: {}", version.render()),
                format!("host: {}..={}", host_min.render(), host_max.render()),
                format!("license: {license}"),
                format!("capabilities: {}", capabilities.join(", ")),
                format!("permissions: {}", permissions.join(", ")),
                format!("files: {}", files.len()),
                format!(
                    "installed: {}",
                    registry
                        .ids()
                        .into_iter()
                        .map(|id| {
                            let state = if registry.is_enabled(&id) {
                                "enabled"
                            } else {
                                "disabled"
                            };
                            format!("{id} ({state})")
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ]),
            other => Err(format!("{other:?} is not an extension operation")),
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

#[allow(clippy::too_many_arguments)]
fn build_package(
    id: &str,
    version: Version,
    host_min: Version,
    host_max: Version,
    license: &str,
    capabilities: &[String],
    permissions: &[String],
    files: &BTreeMap<String, Vec<u8>>,
) -> Package {
    let manifest = Manifest {
        id: id.to_owned(),
        version,
        host_min,
        host_max,
        capabilities: capabilities.to_vec(),
        permissions: permissions.to_vec(),
        license: license.to_owned(),
        digest: String::new(),
    };
    let mut package = Package {
        manifest,
        files: files.clone(),
    };
    if package.manifest.digest.is_empty() {
        package.manifest.digest = package.digest();
    }
    package
}
