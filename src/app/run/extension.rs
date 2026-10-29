// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The `--extension` command: local packages, contracts and templates (task F22).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use crate::app::extension::adapters::{Adapter, Adapters, Kind};
use crate::app::extension::manager::{Panel, Usage};
use crate::app::extension::startup::{Finding, bisect};
use crate::app::extension::supervisor::{
    Access, ChildHandler, HandlerFailure, Limits, LocalBehavior, LocalHarness, Supervisor,
};
use crate::app::extension::{
    DiagnosticBundle, Host, Manifest, Package, Param, ParamKind, Registry, ServiceLog, Template,
    Version,
};
use crate::contracts::permission::{EffectClass, Principal};
use crate::host::log::Level;
use crate::host::permission::{PermissionService, Scope};
use crate::host::subprocess::{OwnedProcess, ProcessSpec};

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
    let mut provides: Vec<String> = Vec::new();
    // The tool adapters registered in this invocation, and the one being declared (T22.03).
    let mut adapters = Adapters::new();
    let mut adapter: Option<Adapter> = None;
    // What the open project reports using; the panel matches it against what an extension provides.
    let mut usage = Usage::new();
    let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut registry = Registry::new();
    let mut template: Option<Template> = None;
    let harness = LocalHarness::new();
    let mut supervisor: Option<Supervisor> = None;
    let mut supervised_failed = false;
    // How many real child processes the supervision path started. A local harness counts its own.
    let child_starts = Arc::new(AtomicUsize::new(0));
    // One decision service for the invocation, so an install's permission check and an explicit
    // `authorize` are recorded by the same host service (T22.08).
    let permission_service = PermissionService::new();
    let mut service_log = ServiceLog::new();
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
            &child_starts,
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
                provides = manifest.provides.clone();
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
            "provides" => {
                let names: Vec<&str> = argument
                    .split(',')
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .collect();
                if names.is_empty() {
                    return super::report(format!(
                        "provides needs <name>[,<name>...], got {argument:?}"
                    ));
                }
                provides.extend(names.iter().map(|name| (*name).to_owned()));
                Ok(vec![format!("provides {}", names.join(", "))])
            }
            "usage" => {
                let names: Vec<&str> = argument
                    .split(',')
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .collect();
                if names.is_empty() {
                    return super::report(format!(
                        "usage needs <name>[,<name>...], got {argument:?}"
                    ));
                }
                for name in &names {
                    usage.declare(name);
                }
                Ok(vec![format!("project uses {}", names.join(", "))])
            }
            "panel" => {
                let panel = Panel::new(&registry, &usage);
                let mut lines = if registry.ids().is_empty() {
                    vec!["no extensions installed".to_owned()]
                } else {
                    panel.render()
                };
                if usage.is_empty() {
                    lines.push("project uses no extension entry point".to_owned());
                } else {
                    lines.push(format!(
                        "project uses {} name(s): {}",
                        usage.len(),
                        usage.names().join(", ")
                    ));
                }
                let dependents = panel.dependents();
                if dependents.is_empty() {
                    lines.push("project depends on no installed extension".to_owned());
                } else {
                    for dependency in dependents {
                        lines.push(format!("dependency: {}", dependency.render()));
                    }
                }
                Ok(lines)
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
                    &provides,
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
                    &provides,
                    &files,
                );
                if let Err(reason) = check_permissions(&permission_service, &package.manifest) {
                    return super::report(reason);
                }
                let digest = package.digest();
                registry
                    .install(package, &host)
                    .map(|_| vec![format!("installed {id} {} ({digest})", version.render())])
            }
            "install-dir" => {
                if argument.is_empty() {
                    return super::report("install-dir needs <directory>".to_owned());
                }
                let path = std::path::Path::new(argument);
                let package = match Package::from_directory(path) {
                    Ok(package) => package,
                    Err(reason) => return super::report(reason),
                };
                let id = package.manifest.id.clone();
                let version = package.manifest.version.render();
                let digest = package.digest();
                let source = package.source.clone();
                let checks = match check_permissions(&permission_service, &package.manifest) {
                    Ok(checks) => checks,
                    Err(reason) => return super::report(reason),
                };
                registry.install(package, &host).map(|_| {
                    let mut lines =
                        vec![format!("installed {id} {version} ({digest}) from {source}")];
                    lines.extend(checks);
                    lines
                })
            }
            "authorize" => match permission_effect(argument) {
                Some(effect) => {
                    match permission_service.permit(effect, Scope::Session, Principal::Human) {
                        Ok(permit) => Ok(vec![format!(
                            "permission {argument}: allowed ({})",
                            permit.evidence()
                        )]),
                        Err(refusal) => Err(format!(
                            "permission {argument} is not authorized: {}",
                            refusal.message()
                        )),
                    }
                }
                None => Err(format!(
                    "{argument:?} is not an extension permission (fs or network)"
                )),
            },
            "service-log" => match argument.splitn(3, ':').collect::<Vec<_>>().as_slice() {
                [service, level, text] => match Level::parse(level) {
                    Some(level) => {
                        service_log.push(*service, level, *text);
                        Ok(vec![format!("log {service} {}: {text}", level.name())])
                    }
                    None => Err(format!("{level:?} is not a log level")),
                },
                _ => Err(format!(
                    "service-log needs <service>:<level>:<text>, got {argument:?}"
                )),
            },
            "bisect" => match argument.split_once(':') {
                Some((program, ids)) => {
                    let ids: Vec<String> = ids
                        .split(',')
                        .map(str::trim)
                        .filter(|id| !id.is_empty())
                        .map(str::to_owned)
                        .collect();
                    if ids.is_empty() {
                        return super::report("bisect needs at least one extension id".to_owned());
                    }
                    let outcome = bisect(&ids, startup_probe(program));
                    let lines = outcome.render();
                    if matches!(&outcome.finding, Finding::Unresolved { .. }) {
                        for line in &lines {
                            println!("{line}");
                        }
                        return super::report(outcome.finding.render());
                    }
                    Ok(lines)
                }
                None => Err(format!(
                    "bisect needs <program>:<id>[,<id>...], got {argument:?}"
                )),
            },
            "bundle-preview" => Ok(diagnostic_bundle(&registry, &service_log).preview()),
            "bundle" => {
                if argument.is_empty() {
                    return super::report("bundle needs <file>".to_owned());
                }
                let bundle = diagnostic_bundle(&registry, &service_log);
                let permit = match bundle.authorize_local(&permission_service) {
                    Ok(permit) => permit,
                    Err(reason) => return super::report(reason),
                };
                match bundle.write(&permit, std::path::Path::new(argument)) {
                    Ok(bytes) => Ok(vec![format!(
                        "wrote {bytes} byte(s) to {argument} ({})",
                        permit.evidence()
                    )]),
                    Err(reason) => Err(reason),
                }
            }
            "bundle-send" => {
                if argument.is_empty() {
                    return super::report("bundle-send needs a destination".to_owned());
                }
                let bundle = diagnostic_bundle(&registry, &service_log);
                match bundle.authorize_external(&permission_service, argument) {
                    Ok(permit) => Ok(vec![format!(
                        "authorized {} ({})",
                        permit.target().unwrap_or("(local)"),
                        permit.evidence()
                    )]),
                    Err(reason) => Err(reason),
                }
            }
            "pin" => registry
                .pin(argument)
                .map(|_| vec![format!("pinned {argument}")]),
            "unpin" => registry
                .unpin(argument)
                .map(|_| vec![format!("unpinned {argument}")]),
            "rollback" => registry.rollback(argument).map(|line| vec![line]),
            "status" => Ok(registry.render()),
            "lookup" => match registry.version(argument) {
                Some(version) => Ok(vec![format!(
                    "{argument} {version} [{}] from {}",
                    if registry.is_enabled(argument) {
                        "enabled"
                    } else {
                        "disabled"
                    },
                    registry.source(argument).unwrap_or("unknown")
                )]),
                None => Err(format!("no extension {argument:?} is installed")),
            },
            "disable" => registry
                .disable(argument)
                .map(|_| vec![format!("disabled {argument}")]),
            "enable" => registry
                .enable(argument)
                .map(|_| vec![format!("enabled {argument}")]),
            "remove" => {
                // The removal first shows the project's dependencies, and is refused while any exist:
                // a warning that removed anyway would leave exactly the dangling entry point that
                // T22.05 forbids.
                let removal = match Panel::new(&registry, &usage).removal(argument) {
                    Ok(removal) => removal,
                    Err(reason) => return super::report(reason),
                };
                if !removal.allowed() {
                    for line in removal.render() {
                        println!("{line}");
                    }
                    return super::report(format!(
                        "refusing to remove {}: the project still uses {} entry point(s) it provides",
                        removal.id,
                        removal.dangling()
                    ));
                }
                registry.remove(argument).map(|line| vec![line])
            }
            "remove-preflight" => match Panel::new(&registry, &usage).removal(argument) {
                Ok(removal) => {
                    let lines = removal.render();
                    if removal.allowed() {
                        Ok(lines)
                    } else {
                        for line in lines {
                            println!("{line}");
                        }
                        return super::report(format!(
                            "{} would leave {} dangling entry point(s)",
                            removal.id,
                            removal.dangling()
                        ));
                    }
                }
                Err(reason) => Err(reason),
            },
            "update-preflight" => {
                let package = build_package(
                    &id,
                    version,
                    host_min,
                    host_max,
                    &license,
                    &capabilities,
                    &permissions,
                    &provides,
                    &files,
                );
                // An upgrade is re-checked too: a new version may declare a permission the
                // installed one did not, and that must not ride in on the old authorization.
                if let Err(reason) = check_permissions(&permission_service, &package.manifest) {
                    return super::report(reason);
                }
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
            "adapter" => match split_fields::<5>(argument) {
                Some([id, kind, version, host_min, host_max]) => match Kind::parse(kind) {
                    Some(kind) => match (
                        Version::parse(version),
                        Version::parse(host_min),
                        Version::parse(host_max),
                    ) {
                        (Ok(version), Ok(host_min), Ok(host_max)) => {
                            adapter = Some(Adapter::new(id, kind, version, host_min, host_max));
                            Ok(vec![format!(
                                "adapter {id} {} {} host {}..={}",
                                kind.name(),
                                version.render(),
                                host_min.render(),
                                host_max.render()
                            )])
                        }
                        _ => Err(format!(
                            "adapter needs <id>:<kind>:<x.y.z>:<x.y.z>:<x.y.z>, got {argument:?}"
                        )),
                    },
                    None => Err(format!("{kind:?} is not language, debug, build or deploy")),
                },
                None => Err(format!(
                    "adapter needs <id>:<kind>:<version>:<host-min>:<host-max>, got {argument:?}"
                )),
            },
            "adapter-needs" => match adapter.take() {
                Some(declared) => {
                    let declared = declared.needing(argument);
                    let line = format!("adapter {} needs {argument}", declared.id);
                    adapter = Some(declared);
                    Ok(vec![line])
                }
                None => Err(
                    "declare adapter:<id>:<kind>:<version>:<host-min>:<host-max> first".to_owned(),
                ),
            },
            "adapter-register" => match &adapter {
                Some(declared) => {
                    let id = declared.id.clone();
                    match adapters.register(declared.clone(), &host) {
                        Ok(status) => Ok(vec![format!("adapter {id} [{}]", status.render())]),
                        Err(reason) => Err(reason),
                    }
                }
                None => Err(
                    "declare adapter:<id>:<kind>:<version>:<host-min>:<host-max> first".to_owned(),
                ),
            },
            "adapter-probe" => {
                let changes = adapters.probe(&host);
                if changes.is_empty() {
                    Ok(vec!["no adapter changed".to_owned()])
                } else {
                    Ok(changes)
                }
            }
            "adapter-fail" => match argument.split_once(':') {
                Some((id, reason)) => adapters.fail(id, reason).map(|()| {
                    let status = adapters
                        .status(id)
                        .map(|status| status.render())
                        .unwrap_or_default();
                    vec![format!("adapter {id} [{status}]")]
                }),
                None => Err(format!(
                    "adapter-fail needs <id>:<reason>, got {argument:?}"
                )),
            },
            "adapter-ok" => adapters
                .ok(argument, &host)
                .map(|status| vec![format!("adapter {argument} [{}]", status.render())]),
            "adapters" => {
                let mut lines = adapters.render();
                let missing: Vec<&str> = adapters
                    .missing_kinds()
                    .into_iter()
                    .map(|kind| kind.name())
                    .collect();
                lines.push(format!(
                    "adapters: {} registered, no usable adapter for {}",
                    adapters.ids().len(),
                    if missing.is_empty() {
                        "no kind".to_owned()
                    } else {
                        missing.join(", ")
                    }
                ));
                Ok(lines)
            }
            "show" => Ok(vec![
                format!("id: {id}"),
                format!("version: {}", version.render()),
                format!("host: {}..={}", host_min.render(), host_max.render()),
                format!("license: {license}"),
                format!("capabilities: {}", capabilities.join(", ")),
                format!("permissions: {}", permissions.join(", ")),
                format!("provides: {}", provides.join(", ")),
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
                            let pin = if registry.is_pinned(&id) {
                                ",pinned"
                            } else {
                                ""
                            };
                            format!("{id} ({state}{pin})")
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                format!("pinned: {}", registry.pinned_ids().join(", ")),
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
    child_starts: &Arc<AtomicUsize>,
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
        "supervise-child" => {
            if argument.is_empty() {
                return Some(Err("supervise-child needs <program>".to_owned()));
            }
            let program = argument.to_owned();
            let starts = Arc::clone(child_starts);
            let spawned = program.clone();
            // The child's own read deadline is longer than the supervisor's, so a slow answer is the
            // supervisor's `Timeout` rather than a handler-side "crashed".
            let deadline = Limits::default().call_timeout + Duration::from_secs(1);
            match Supervisor::new(
                supervised_id("", package_id),
                Limits::default(),
                supervise_access(),
                move || {
                    use crate::contracts::permission::{EffectClass, Principal};
                    use crate::host::permission::{PermissionService, Scope};
                    let permit = PermissionService::new()
                        .permit(EffectClass::Command, Scope::Session, Principal::Human)
                        .map_err(|refusal| HandlerFailure::Crashed {
                            reason: refusal.message(),
                        })?;
                    starts.fetch_add(1, Ordering::SeqCst);
                    ChildHandler::spawn(
                        crate::host::subprocess::ProcessSpec::new(
                            "extension process",
                            spawned.clone(),
                        ),
                        &permit,
                        deadline,
                    )
                    .map(|handler| {
                        Box::new(handler) as Box<dyn crate::app::extension::supervisor::Handler>
                    })
                },
            ) {
                Ok(value) => {
                    *supervisor = Some(value);
                    Some(Ok(vec![format!("supervising child process {program}")]))
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
            let starts = harness.starts() + child_starts.load(Ordering::SeqCst);
            Some(Ok(vec![
                value.render(),
                value.health().render(),
                format!("handler starts: {starts}"),
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
    match PermissionService::new().permit(EffectClass::Command, Scope::Session, Principal::Human) {
        Ok(_) => Access::Granted,
        Err(refusal) => Access::Denied {
            reason: refusal.message(),
        },
    }
}

/// Splits `a:b:c` into exactly `N` non-empty fields; the last one keeps any colons it holds.
fn split_fields<const N: usize>(text: &str) -> Option<[&str; N]> {
    let parts: Vec<&str> = text.splitn(N, ':').collect();
    if parts.len() != N || parts.iter().any(|part| part.trim().is_empty()) {
        return None;
    }
    let mut fields = [""; N];
    for (slot, part) in fields.iter_mut().zip(parts) {
        *slot = part;
    }
    Some(fields)
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
    provides: &[String],
    files: &BTreeMap<String, Vec<u8>>,
) -> Package {
    let manifest = Manifest {
        id: id.to_owned(),
        version,
        host_min,
        host_max,
        capabilities: capabilities.to_vec(),
        permissions: permissions.to_vec(),
        provides: provides.to_vec(),
        license: license.to_owned(),
        digest: String::new(),
    };
    let mut package = Package::new(manifest, files.clone());
    if package.manifest.digest.is_empty() {
        package.manifest.digest = package.digest();
    }
    package
}

/// The effect an extension permission asks the host to perform.
///
/// The two names are the ones the host publishes (`Host.permissions`); an extension cannot declare a
/// third, because [`Manifest::validate`] refuses a permission the host does not publish.
fn permission_effect(name: &str) -> Option<EffectClass> {
    match name {
        "fs" => Some(EffectClass::FileWrite),
        "network" => Some(EffectClass::Network),
        _ => None,
    }
}

/// Asks the permission service for every permission a package declares.
///
/// This is the gate T22.08 asks for. A package that declares `network` is refused with the service's
/// own reason: the session has no network policy, and a policy is a project-configuration fact that
/// this command cannot invent. Saying so is the honest answer — the alternative would be installing
/// code whose network use was never authorized.
fn check_permissions(
    service: &PermissionService,
    manifest: &Manifest,
) -> Result<Vec<String>, String> {
    let mut lines = Vec::new();
    for name in &manifest.permissions {
        let Some(effect) = permission_effect(name) else {
            return Err(format!(
                "the host has no decision for the declared permission {name:?}"
            ));
        };
        match service.permit(effect, Scope::Session, Principal::Human) {
            Ok(permit) => lines.push(format!(
                "permission {name}: allowed ({})",
                permit.evidence()
            )),
            Err(refusal) => {
                return Err(format!(
                    "{} declares {name}, which is not authorized: {}",
                    manifest.id,
                    refusal.message()
                ));
            }
        }
    }
    Ok(lines)
}

/// Runs a program as one startup probe, with the enabled extensions in its environment.
///
/// The program is the caller's: this build has no extension entrypoint, so the closest honest
/// probe is a real process that reads `RUSTAGE_EXTENSIONS` and exits non-zero when it cannot
/// start. The process is started through a command permit and bounded by a ten-second wait.
fn startup_probe(program: &str) -> impl FnMut(&[String]) -> Result<(), String> + '_ {
    move |enabled: &[String]| {
        let permit = PermissionService::new()
            .permit(EffectClass::Command, Scope::Session, Principal::Human)
            .map_err(|refusal| refusal.message())?;
        let spec = ProcessSpec::new("extension startup", program)
            .env("RUSTAGE_EXTENSIONS", enabled.join(","))
            .capture_limit(4096);
        let mut process = OwnedProcess::spawn(spec, &permit).map_err(|error| error.message())?;
        match process.wait_for(std::time::Duration::from_secs(10)) {
            Some(exit) if exit.is_success() => Ok(()),
            Some(exit) => {
                let output = process.output_so_far();
                let detail = output.stderr.trim();
                Err(if detail.is_empty() {
                    exit.message()
                } else {
                    format!("{}: {detail}", exit.message())
                })
            }
            None => {
                let stopped = process.terminate(std::time::Duration::from_millis(500));
                Err(match stopped {
                    crate::host::subprocess::Termination::SignalFailed { reason } => format!(
                        "the probe did not finish within 10 s; stopping it also failed: {reason}"
                    ),
                    _ => "the probe did not finish within 10 s".to_owned(),
                })
            }
        }
    }
}

/// The bundle as this invocation knows it: host facts, the registry and the service log.
fn diagnostic_bundle(registry: &Registry, service_log: &ServiceLog) -> DiagnosticBundle {
    let mut bundle = DiagnosticBundle::new();
    bundle.section(
        "host",
        [
            format!("rustage {}", env!("CARGO_PKG_VERSION")),
            format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        ],
    );
    bundle.section("extensions", registry.render());
    bundle.section("service log", service_log.render());
    bundle
}

#[cfg(test)]
mod tests {
    use super::supervise_op;
    use crate::app::extension::supervisor::LocalHarness;

    fn manifest(permissions: &[&str]) -> crate::app::extension::Manifest {
        use crate::app::extension::Version;
        crate::app::extension::Manifest {
            id: "acme.chart".to_owned(),
            version: Version::parse("1.2.0").expect("a version"),
            host_min: Version::parse("0.1.0").expect("a version"),
            host_max: Version::parse("0.2.0").expect("a version"),
            capabilities: vec!["widget".to_owned()],
            permissions: permissions.iter().map(|name| (*name).to_owned()).collect(),
            provides: vec!["acme.chart".to_owned()],
            license: "MIT".to_owned(),
            digest: "00".repeat(32),
        }
    }

    #[test]
    fn a_declared_permission_goes_through_the_host_decision_table() {
        let service = crate::host::permission::PermissionService::new();
        // `fs` maps to writing a file, which a human may do; the check is recorded, not assumed.
        let checks = super::check_permissions(&service, &manifest(&["fs"]))
            .expect("a filesystem permission is allowed for a human");
        assert!(checks[0].contains("allowed"), "{checks:?}");

        // `network` needs a project network policy that the session does not have, so the install
        // is refused with the service's own reason rather than installing unauthorized code.
        let error = super::check_permissions(&service, &manifest(&["network"]))
            .expect_err("no network policy exists yet");
        assert!(error.contains("declares network"), "{error}");
        assert!(error.contains("not authorized"), "{error}");
    }

    #[test]
    fn a_permission_the_host_has_no_decision_for_is_refused() {
        let service = crate::host::permission::PermissionService::new();
        let error = super::check_permissions(&service, &manifest(&["teleport"]))
            .expect_err("no decision exists");
        assert!(error.contains("no decision"), "{error}");
    }

    #[test]
    fn the_supervise_path_starts_calls_reports_and_stops() {
        let harness = LocalHarness::new();
        let mut supervisor = None;
        let mut failed = false;
        let child_starts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let lines = supervise_op(
            &harness,
            &mut supervisor,
            &mut failed,
            &child_starts,
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
            &child_starts,
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
            &child_starts,
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
            &child_starts,
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
        let child_starts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let lines = supervise_op(
            &harness,
            &mut supervisor,
            &mut failed,
            &child_starts,
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
            &child_starts,
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
        let child_starts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

        supervise_op(
            &harness,
            &mut supervisor,
            &mut failed,
            &child_starts,
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
            &child_starts,
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
            &child_starts,
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
            &child_starts,
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
