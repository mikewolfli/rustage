// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The `--extension` command: local packages, contracts and templates (task F22).

use std::collections::{BTreeMap, BTreeSet};

use crate::app::extension::supervisor::{Access, Limits, LocalBehavior, LocalHarness, Supervisor};
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
    let harness = LocalHarness::new();
    let mut supervisor: Option<Supervisor> = None;
    let mut supervised_failed = false;
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
        if let Some(step) = supervise_op(
            &harness,
            &mut supervisor,
            &mut supervised_failed,
            verb,
            argument,
            &id,
        ) {
            match step {
                Ok(lines) => {
                    for line in lines {
                        println!("{line}");
                    }
                }
                Err(reason) => return super::report(reason),
            }
            continue;
        }
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
    if supervised_failed {
        return false;
    }
    true
}

/// Routes the `supervise` operations to the typed supervisor.
///
/// The returned `Option` is `None` when `verb` is not a supervision operation, so the caller can
/// fall through to the package/template operations unchanged.
fn supervise_op(
    harness: &LocalHarness,
    supervisor: &mut Option<Supervisor>,
    failed: &mut bool,
    verb: &str,
    argument: &str,
    package_id: &str,
) -> Option<Result<Vec<String>, String>> {
    match verb {
        "supervise" | "supervise-denied" => {
            let name = if verb == "supervise" {
                supervised_id(argument, package_id)
            } else {
                supervised_id("", package_id)
            };
            let access = if verb == "supervise-denied" {
                let reason = if argument.is_empty() {
                    crate::contracts::permission::DenialReason::ScopeNotAuthorized
                        .explanation()
                        .to_owned()
                } else {
                    argument.to_owned()
                };
                Access::Denied { reason }
            } else {
                supervise_access()
            };
            match harness.supervisor(name, Limits::default(), access) {
                Ok(value) => {
                    let line = value.render();
                    *supervisor = Some(value);
                    Some(Ok(vec![line]))
                }
                Err(error) => Some(Err(error.render())),
            }
        }
        "supervise-call" => {
            let Some(value) = supervisor.as_mut() else {
                return Some(Err("declare supervise[:<id>] first".to_owned()));
            };
            match value.call(argument.as_bytes()) {
                Ok(answer) => Some(Ok(vec![format!(
                    "answer ({} byte(s)): {}",
                    answer.len(),
                    String::from_utf8_lossy(&answer)
                )])),
                Err(error) => {
                    *failed = true;
                    Some(Ok(vec![format!("supervisor error: {}", error.render())]))
                }
            }
        }
        "supervise-status" => {
            let Some(value) = supervisor.as_ref() else {
                return Some(Err("declare supervise[:<id>] first".to_owned()));
            };
            Some(Ok(vec![
                value.render(),
                value.health().render(),
                format!("handler starts: {}", harness.starts()),
            ]))
        }
        "supervise-stop" => {
            let Some(value) = supervisor.as_mut() else {
                return Some(Err("declare supervise[:<id>] first".to_owned()));
            };
            value.stop();
            Some(Ok(vec![value.render()]))
        }
        "supervise-echo" => {
            harness.set_behavior(LocalBehavior::Echo);
            Some(Ok(vec![behavior_line(harness)]))
        }
        "supervise-crash" => {
            let reason = if argument.is_empty() {
                "the extension process stopped".to_owned()
            } else {
                argument.to_owned()
            };
            harness.set_behavior(LocalBehavior::crash(reason));
            Some(Ok(vec![behavior_line(harness)]))
        }
        "supervise-protocol" => {
            let reason = if argument.is_empty() {
                "the answer is not a frame".to_owned()
            } else {
                argument.to_owned()
            };
            harness.set_behavior(LocalBehavior::protocol(reason));
            Some(Ok(vec![behavior_line(harness)]))
        }
        "supervise-empty" => {
            harness.set_behavior(LocalBehavior::Empty);
            Some(Ok(vec![behavior_line(harness)]))
        }
        "supervise-timeout" => {
            let millis = if argument.is_empty() {
                3_000
            } else {
                match argument.parse::<u64>() {
                    Ok(value) => value,
                    Err(_) => {
                        return Some(Err(format!("{argument:?} is not a number of milliseconds")));
                    }
                }
            };
            harness.set_behavior(LocalBehavior::sleep(millis));
            Some(Ok(vec![format!(
                "supervisor behaviour: sleep ({millis} ms)"
            )]))
        }
        "supervise-oversize" => {
            let bytes = match argument.parse::<usize>() {
                Ok(value) => value,
                Err(_) => {
                    return Some(Err(format!("{argument:?} is not a number of bytes")));
                }
            };
            harness.set_behavior(LocalBehavior::oversize(bytes));
            Some(Ok(vec![format!(
                "supervisor behaviour: oversize ({bytes} byte(s))"
            )]))
        }
        _ => None,
    }
}

fn supervised_id(argument: &str, package_id: &str) -> String {
    if !argument.is_empty() {
        argument.to_owned()
    } else if package_id.is_empty() {
        "extension".to_owned()
    } else {
        package_id.to_owned()
    }
}

fn behavior_line(harness: &LocalHarness) -> String {
    format!("supervisor behaviour: {}", harness.behavior().name())
}

/// The permission check an extension call is created under.
///
/// The host's own session may run the extension; a refusal is carried into the supervisor as a
/// typed denial rather than being flattened into an empty answer.
fn supervise_access() -> Access {
    use crate::contracts::permission::{EffectClass, Principal};
    use crate::host::permission::{PermissionService, Scope};

    match PermissionService::new().permit(EffectClass::Command, Scope::Session, Principal::Human) {
        Ok(_) => Access::Granted,
        Err(refusal) => Access::Denied {
            reason: refusal.message(),
        },
    }
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

#[cfg(test)]
mod tests {
    use super::supervise_op;
    use crate::app::extension::supervisor::LocalHarness;

    #[test]
    fn the_supervise_path_starts_calls_reports_and_stops() {
        let harness = LocalHarness::new();
        let mut supervisor = None;
        let mut failed = false;

        let lines = supervise_op(
            &harness,
            &mut supervisor,
            &mut failed,
            "supervise",
            "",
            "demo",
        )
        .expect("a supervise operation")
        .expect("a supervisor");
        assert!(lines[0].contains("stopped"), "{lines:?}");
        assert!(!failed);

        let lines = supervise_op(
            &harness,
            &mut supervisor,
            &mut failed,
            "supervise-call",
            "ping",
            "demo",
        )
        .expect("a supervise operation")
        .expect("a call");
        assert!(lines[0].contains("ping"), "{lines:?}");
        assert!(!failed);

        let lines = supervise_op(
            &harness,
            &mut supervisor,
            &mut failed,
            "supervise-status",
            "",
            "demo",
        )
        .expect("a supervise operation")
        .expect("a status");
        assert!(lines[0].contains("running"), "{lines:?}");
        assert!(lines[1].contains("extension: on"), "{lines:?}");

        let lines = supervise_op(
            &harness,
            &mut supervisor,
            &mut failed,
            "supervise-stop",
            "",
            "demo",
        )
        .expect("a supervise operation")
        .expect("a stop");
        assert!(lines[0].contains("stopped"), "{lines:?}");
    }

    #[test]
    fn a_denied_supervisor_reports_the_refusal_instead_of_success() {
        let harness = LocalHarness::new();
        let mut supervisor = None;
        let mut failed = false;

        let lines = supervise_op(
            &harness,
            &mut supervisor,
            &mut failed,
            "supervise-denied",
            "the extension is not permitted",
            "demo",
        )
        .expect("a supervise operation")
        .expect("a supervisor");
        assert!(lines[0].contains("denied"), "{lines:?}");

        let lines = supervise_op(
            &harness,
            &mut supervisor,
            &mut failed,
            "supervise-call",
            "ping",
            "demo",
        )
        .expect("a supervise operation")
        .expect("a reported refusal");
        assert!(lines[0].contains("denied"), "{lines:?}");
        assert!(failed, "the denial must not be reported as success");
    }

    #[test]
    fn a_supervised_crash_is_visible_and_the_host_stays_available() {
        let harness = LocalHarness::new();
        let mut supervisor = None;
        let mut failed = false;

        supervise_op(
            &harness,
            &mut supervisor,
            &mut failed,
            "supervise",
            "",
            "demo",
        )
        .expect("a supervise operation")
        .expect("a supervisor");
        supervise_op(
            &harness,
            &mut supervisor,
            &mut failed,
            "supervise-crash",
            "the extension process exited",
            "demo",
        )
        .expect("a supervise operation")
        .expect("a behaviour");

        let lines = supervise_op(
            &harness,
            &mut supervisor,
            &mut failed,
            "supervise-call",
            "one",
            "demo",
        )
        .expect("a supervise operation")
        .expect("a reported crash");
        assert!(lines[0].contains("stopped"), "{lines:?}");
        assert!(failed);

        let lines = supervise_op(
            &harness,
            &mut supervisor,
            &mut failed,
            "supervise-status",
            "",
            "demo",
        )
        .expect("a supervise operation")
        .expect("a status");
        assert!(lines[0].contains("failed"), "{lines:?}");
        assert!(lines[1].contains("design: on"), "{lines:?}");
        assert!(lines[1].contains("preview: on"), "{lines:?}");
        assert!(lines[1].contains("extension: off"), "{lines:?}");
    }
}
