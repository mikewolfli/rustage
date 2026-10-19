// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! F18: the target configuration, the capability snapshot and switch, the desktop matrix, the mobile
//! target, the restricted targets, local delivery, the queue and the certification report.
//!
//! One command handler per parsed command of this domain; the dispatcher lives in [`super`](self). A
//! refusal prints the reason and answers false, so a command that found a problem exits through the
//! host's `Reported` code rather than looking like a success. Nothing here upgrades evidence: a plan
//! is printed as a plan, a simulator is not a device, and a check is not a build.

use super::*;
use crate::app::component::targets::ResourceKind;
use crate::app::targets::certification::{self, Report, Stage};
use crate::app::targets::configuration::{Configuration, Mode, Os, Runner};
use crate::app::targets::devices::{
    Asset, Evidence, Input, InputKind, Plan as MobilePlan, Sdk, Support, Where,
};
use crate::app::targets::local::{
    self, Device, Discovery, Journal, MobilePlatform, Package, Probe, Recovery, ToolState, Toolbox,
    Transport,
};
use crate::app::targets::matrix::{Matrix, Proof};
use crate::app::targets::queue::{
    Artifact, CredentialRef, CredentialSource, Queue, Signing, redact,
};
use crate::app::targets::restricted::{self, Feature, Gate, Restricted};
use crate::app::targets::snapshot::{self, Node, Snapshot};

/// Splits `text` into `count` fields on `:`, the last field absorbing any remaining colons.
fn split_fields(text: &str, count: usize) -> Option<Vec<&str>> {
    let mut parts = Vec::with_capacity(count);
    let mut rest = text;
    for _ in 1..count {
        let (head, tail) = rest.split_once(':')?;
        parts.push(head);
        rest = tail;
    }
    parts.push(rest);
    Some(parts)
}

/// Parses `yes`/`true` and `no`/`false`.
fn flag(value: &str) -> Option<bool> {
    match value {
        "yes" | "true" => Some(true),
        "no" | "false" => Some(false),
        _ => None,
    }
}

/// Parses a whole number.
fn count(value: &str) -> Result<u32, String> {
    value
        .parse()
        .map_err(|_| format!("{value:?} is not a whole number"))
}

/// Parses `<width>x<height>`.
fn size(text: &str) -> Result<(u32, u32), String> {
    let (width, height) = text
        .split_once('x')
        .ok_or_else(|| format!("{text:?} is not <width>x<height>"))?;
    Ok((count(width)?, count(height)?))
}

/// Configures one output target (task T18.01).
pub(super) fn target_config(options: &crate::cli::TargetConfigOptions) -> bool {
    let mut profile = String::from("desktop");
    let mut triple: Option<String> = None;
    let mut toolchain: Option<String> = None;
    let mut sdk: Option<String> = None;
    let mut mode: Option<Mode> = None;
    let mut runner: Option<Runner> = None;
    let mut host = Os::host();
    let mut check = false;

    for op in &options.ops {
        let (verb, rest) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<(), String> = match verb {
            "profile" => {
                profile = rest.to_owned();
                Ok(())
            }
            "triple" => {
                triple = Some(rest.to_owned());
                Ok(())
            }
            "toolchain" => {
                toolchain = Some(rest.to_owned());
                Ok(())
            }
            "sdk" => {
                sdk = Some(rest.to_owned());
                Ok(())
            }
            "mode" => Mode::parse(rest)
                .map(|parsed| mode = Some(parsed))
                .ok_or_else(|| format!("{rest:?} is not one or two")),
            "runner" => Runner::parse(rest)
                .map(|parsed| runner = Some(parsed))
                .ok_or_else(|| format!("{rest:?} is not host, simulator, device or none")),
            "host" => Os::parse(rest)
                .map(|parsed| host = parsed)
                .ok_or_else(|| format!("{rest:?} is not macos, linux or windows")),
            "check" => {
                check = true;
                Ok(())
            }
            "show" => Ok(()),
            other => Err(format!("{other:?} is not a target-configuration operation")),
        };
        if let Err(reason) = step {
            return report(reason);
        }
    }

    let mut configuration = Configuration::new(profile);
    if let Some(triple) = triple {
        configuration = configuration.with_triple(triple);
    }
    if let Some(toolchain) = toolchain {
        configuration = configuration.with_toolchain(toolchain);
    }
    if let Some(sdk) = sdk {
        configuration = configuration.with_sdk(sdk);
    }
    if let Some(mode) = mode {
        configuration = configuration.with_mode(mode);
    }
    if let Some(runner) = runner {
        configuration = configuration.with_runner(runner);
    }

    println!("{}", configuration.render());
    let findings = configuration.check(host);
    for finding in &findings {
        println!("{}", finding.render());
    }
    let unsupported = findings.iter().any(|finding| finding.is_unsupported());
    if !findings.is_empty() {
        println!("target: not ready ({} finding(s))", findings.len());
    } else if check {
        println!("target: ready");
    }
    if unsupported {
        return false;
    }
    // `check` is the request to judge readiness; without it this is a report of what was chosen.
    !check || findings.is_empty()
}

/// Captures, compares and applies a capability snapshot (task T18.02).
pub(super) fn target_capability(options: &crate::cli::TargetCapabilityOptions) -> bool {
    let mut generator = env!("CARGO_PKG_VERSION").to_owned();
    let mut built_for = String::from("desktop");
    let mut names: Vec<String> = Vec::new();
    let mut read_path: Option<String> = None;
    let mut save_path: Option<String> = None;
    let mut compare_path: Option<String> = None;
    let mut nodes: Vec<Node> = Vec::new();
    let mut switches: Vec<(String, String)> = Vec::new();
    let mut pinned: Option<String> = None;
    let mut check = false;
    let mut ok = true;

    for op in &options.ops {
        let (verb, rest) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<(), String> = match verb {
            "generator" => {
                generator = rest.to_owned();
                Ok(())
            }
            "built-for" => {
                built_for = rest.to_owned();
                Ok(())
            }
            "name" => {
                names.push(rest.to_owned());
                Ok(())
            }
            "read" => {
                read_path = Some(rest.to_owned());
                Ok(())
            }
            "save" => {
                save_path = Some(rest.to_owned());
                Ok(())
            }
            "compare" => {
                compare_path = Some(rest.to_owned());
                Ok(())
            }
            "node" => split_fields(rest, 2)
                .ok_or_else(|| format!("node needs <kind>:<children>, got {rest:?}"))
                .and_then(|fields| {
                    let children = count(fields[1])?;
                    nodes.push(Node::new(fields[0], children as usize));
                    Ok(())
                }),
            "switch" => split_fields(rest, 2)
                .ok_or_else(|| format!("switch needs <from>:<to>, got {rest:?}"))
                .map(|fields| switches.push((fields[0].to_owned(), fields[1].to_owned()))),
            "stale" => {
                pinned = Some(rest.to_owned());
                Ok(())
            }
            "check" => {
                check = true;
                Ok(())
            }
            "show" => Ok(()),
            other => Err(format!("{other:?} is not a capability operation")),
        };
        if let Err(reason) = step {
            return report(reason);
        }
    }

    if !crate::app::targets::configuration::is_profile(&built_for) {
        return report(format!(
            "built-for {built_for:?} is not a profile; a snapshot names which build answered"
        ));
    }
    if generator.trim().is_empty() {
        return report("a snapshot needs a generator version".to_owned());
    }

    let snapshot = match read_path {
        Some(path) => match std::fs::read_to_string(&path) {
            Ok(text) => match Snapshot::decode(&text) {
                Ok(snapshot) => snapshot,
                Err(fault) => return report(format!("{path}: {}", fault.message())),
            },
            Err(error) => return report(format!("{path} could not be read: {error}")),
        },
        None => {
            let owned = if names.is_empty() {
                Snapshot::widget_names()
            } else {
                names
            };
            let borrowed: Vec<&str> = owned.iter().map(String::as_str).collect();
            println!(
                "capturing {} widget name(s) under the {} build",
                borrowed.len(),
                built_for
            );
            Snapshot::capture(generator.clone(), built_for.clone(), &borrowed)
        }
    };

    println!(
        "snapshot: generator {} built for {}; {} profile(s)",
        snapshot.generator(),
        snapshot.built_for(),
        crate::app::targets::configuration::PROFILES.len()
    );
    for profile in crate::app::targets::configuration::PROFILES {
        let answers = snapshot.answers(profile);
        let unverified = snapshot.unverified(profile);
        if answers == 0 {
            println!("{profile}: no answers");
            ok = false;
        } else if unverified > 0 {
            println!("{profile}: {answers} answer(s), {unverified} not verified under the target");
        } else {
            println!("{profile}: {answers} answer(s), all measured");
        }
    }

    if let Some(pinned) = &pinned {
        match snapshot::stale(pinned, snapshot.generator()) {
            None => println!("pinned capabilities: current"),
            Some(message) => println!("diagnostic: {message}"),
        }
    }

    if let Some(path) = save_path {
        let text = snapshot.encode();
        let result = std::fs::write(&path, &text)
            .map_err(|error| error.to_string())
            .and_then(|()| std::fs::read_to_string(&path).map_err(|error| error.to_string()))
            .and_then(|read_back| {
                Snapshot::decode(&read_back)
                    .map(|decoded| decoded == snapshot)
                    .map_err(|fault| fault.message())
            });
        match result {
            Ok(true) => println!("wrote {path}"),
            Ok(false) => return report(format!("{path} read back differently")),
            Err(reason) => return report(format!("{path} could not be written: {reason}")),
        }
    }

    if let Some(path) = compare_path {
        match std::fs::read_to_string(&path)
            .map_err(|error| error.to_string())
            .and_then(|text| Snapshot::decode(&text).map_err(|fault| fault.message()))
        {
            Ok(older) => {
                let changes = older.changes(&snapshot);
                if changes.is_empty() {
                    println!("compared with {path}: no capability changed");
                } else {
                    println!("compared with {path}: {} change(s)", changes.len());
                    for change in &changes {
                        println!("{}", change.render());
                    }
                }
            }
            Err(reason) => return report(format!("{path} could not be compared: {reason}")),
        }
    }

    for (from, to) in &switches {
        match snapshot::switch(&snapshot, &nodes, from, to) {
            Ok(switched) => {
                for line in switched.render() {
                    println!("{line}");
                }
                if switched.blocks_generation() {
                    println!(
                        "generation for {to} is blocked; the {} node(s) are kept",
                        nodes.len()
                    );
                    ok = false;
                }
            }
            Err(reason) => return report(reason),
        }
    }

    if check && ok {
        println!("capability: ready");
    }
    ok
}

/// Fills the desktop build/behaviour matrix (task T18.03).
pub(super) fn target_matrix(options: &crate::cli::TargetMatrixOptions) -> bool {
    let mut matrix = Matrix::plan();
    let mut check = false;

    for op in &options.ops {
        let (verb, rest) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<(), String> = match verb {
            "start" => matrix.start(rest),
            "pass" => {
                let Some((triple, rest)) = rest.split_once(':') else {
                    return report(format!(
                        "pass needs <triple>:<proof>[:<note>], got {rest:?}"
                    ));
                };
                let (proof, note) = rest.split_once(':').unwrap_or((rest, ""));
                let Some(proof) = Proof::parse(proof) else {
                    return report(format!("{proof:?} is not a proof level"));
                };
                matrix.pass(triple, proof, note)
            }
            "fail" => {
                let Some((triple, reason)) = rest.split_once(':') else {
                    return report(format!("fail needs <triple>:<reason>, got {rest:?}"));
                };
                matrix.fail(triple, reason)
            }
            "cancel" => matrix.cancel(rest),
            "check" => {
                check = true;
                Ok(())
            }
            "show" => Ok(()),
            other => Err(format!("{other:?} is not a matrix operation")),
        };
        if let Err(reason) = step {
            return report(reason);
        }
    }

    for line in matrix.render() {
        println!("{line}");
    }
    let unverified = matrix.unverified();
    if unverified.is_empty() {
        println!("matrix: every committed combination ran");
    } else {
        println!("unverified: {} combination(s)", unverified.len());
        for line in &unverified {
            println!("  {line}");
        }
    }
    for (combination, proof) in matrix.artifacts() {
        println!(
            "artifact {}: {} ({})",
            combination.triple,
            combination.artifact,
            proof.name()
        );
    }
    !check || matrix.is_success()
}

/// Checks a tablet/mobile target (task T18.04).
pub(super) fn target_mobile(options: &crate::cli::TargetMobileOptions) -> bool {
    let mut profile = String::from("mobile");
    let mut chosen_size: Option<(u32, u32)> = None;
    let mut inputs: Vec<Input> = Vec::new();
    let mut assets: Vec<Asset> = Vec::new();
    let mut sdk: Option<Sdk> = None;
    let mut support = Support::new();
    let mut host = Os::host();
    let mut check = false;

    for op in &options.ops {
        let (verb, rest) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<(), String> = match verb {
            "profile" => {
                profile = rest.to_owned();
                Ok(())
            }
            "size" => size(rest).map(|parsed| chosen_size = Some(parsed)),
            "input" => split_fields(rest, 2)
                .ok_or_else(|| format!("input needs <kind>:<w>x<h>, got {rest:?}"))
                .and_then(|fields| {
                    let kind = InputKind::parse(fields[0])
                        .ok_or_else(|| format!("{:?} is not touch or pointer", fields[0]))?;
                    let size = size(fields[1])?;
                    inputs.push(Input::new(kind, size));
                    Ok(())
                }),
            "asset" => split_fields(rest, 3)
                .ok_or_else(|| format!("asset needs <name>:<kind>:<bytes>, got {rest:?}"))
                .and_then(|fields| {
                    let kind = ResourceKind::parse(fields[1])
                        .ok_or_else(|| format!("{:?} is not a resource kind", fields[1]))?;
                    let bytes = count(fields[2])?;
                    assets.push(Asset::new(fields[0], kind, u64::from(bytes)));
                    Ok(())
                }),
            "sdk" => {
                if let Some((platform, "auto")) = rest.split_once(':') {
                    sdk = Some(Sdk::detect(platform, host));
                    Ok(())
                } else {
                    split_fields(rest, 3)
                        .ok_or_else(|| {
                            format!("sdk needs <platform>:<yes|no>[:<version>], got {rest:?}")
                        })
                        .and_then(|fields| {
                            let installed = flag(fields[1])
                                .ok_or_else(|| format!("{:?} is not yes or no", fields[1]))?;
                            let mut parsed = Sdk::new(fields[0], installed);
                            if !fields[2].is_empty() {
                                parsed = parsed.with_version(fields[2]);
                            }
                            sdk = Some(parsed);
                            Ok(())
                        })
                }
            }
            "host" => Os::parse(rest)
                .map(|parsed| host = parsed)
                .ok_or_else(|| format!("{rest:?} is not macos, linux or windows")),
            "evidence" => split_fields(rest, 4)
                .ok_or_else(|| {
                    format!("evidence needs <place>:<name>:<proof>:<note>, got {rest:?}")
                })
                .and_then(|fields| {
                    let place = match fields[0] {
                        "simulator" => Where::Simulator(fields[1].to_owned()),
                        "device" => Where::Device(fields[1].to_owned()),
                        other => return Err(format!("{other:?} is not simulator or device")),
                    };
                    let proof = Proof::parse(fields[2])
                        .ok_or_else(|| format!("{:?} is not a proof level", fields[2]))?;
                    support
                        .record(Evidence::new(profile.clone(), place, proof, fields[3]))
                        .map_err(|reason| reason.to_string())
                }),
            "check" => {
                check = true;
                Ok(())
            }
            "show" => Ok(()),
            other => Err(format!("{other:?} is not a mobile-target operation")),
        };
        if let Err(reason) = step {
            return report(reason);
        }
    }

    let Some(mut plan) = MobilePlan::of(&profile) else {
        return report(format!(
            "{profile:?} is not tablet or mobile; this command checks a mobile target"
        ));
    };
    if let Some(size) = chosen_size {
        plan = plan.with_size(size);
    }
    println!("{}", plan.render());
    if let Some(sdk) = &sdk {
        println!(
            "sdk {}: {}{}",
            sdk.platform,
            if sdk.installed {
                "installed"
            } else {
                "missing"
            },
            sdk.version
                .as_ref()
                .map(|version| format!(" {version}"))
                .unwrap_or_default()
        );
    }
    let findings = plan.check(&inputs, &assets, sdk.as_ref());
    for finding in &findings {
        println!("{}", finding.render());
    }
    for line in support.render() {
        println!("{line}");
    }
    let devices = support.device_levels().len();
    println!("device support: {devices} device(s) exercised; not a claim about every device");
    if check && findings.is_empty() {
        println!("mobile target: ready");
    }
    !check || findings.is_empty()
}

/// Checks a mini/embedded target and audits its links (task T18.05).
pub(super) fn target_restricted(options: &crate::cli::TargetRestrictedOptions) -> bool {
    let mut profile = String::from("mini");
    let mut children = 0_u32;
    let mut mode_one = false;
    let mut features: Vec<Feature> = Vec::new();
    let mut tree_path: Option<String> = None;
    let mut dev: Vec<String> = Vec::new();
    let mut build: Option<bool> = None;
    let mut check = false;

    for op in &options.ops {
        let (verb, rest) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<(), String> = match verb {
            "profile" => {
                profile = rest.to_owned();
                Ok(())
            }
            "children" => count(rest).map(|parsed| children = parsed),
            "mode" => Mode::parse(rest)
                .map(|parsed| mode_one = parsed == Mode::One)
                .ok_or_else(|| format!("{rest:?} is not one or two")),
            "feature" => match Feature::parse(rest) {
                Some(feature) => {
                    features.push(feature);
                    Ok(())
                }
                None => Err(format!("{rest:?} is not a restricted-target feature")),
            },
            "tree" => {
                tree_path = Some(rest.to_owned());
                Ok(())
            }
            "dev" => {
                dev.push(rest.to_owned());
                Ok(())
            }
            "build" => flag(rest)
                .map(|parsed| build = Some(parsed))
                .ok_or_else(|| format!("{rest:?} is not yes or no")),
            "check" => {
                check = true;
                Ok(())
            }
            "show" => Ok(()),
            other => Err(format!("{other:?} is not a restricted-target operation")),
        };
        if let Err(reason) = step {
            return report(reason);
        }
    }

    let mut findings = match restricted::check(&profile, children as usize, mode_one, &features) {
        Ok(findings) => findings,
        Err(reason) => return report(reason),
    };
    let restricted_profile = Restricted::parse(&profile).expect("a checked restricted profile");
    println!(
        "{}: {} children, capacity {}",
        restricted_profile.name(),
        children,
        restricted_profile.capacity()
    );
    if let Some(path) = tree_path {
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let mut dependencies = restricted::parse_tree(&text);
                println!("tree {path}: {} crate(s)", dependencies.len());
                for name in &dev {
                    if dependencies
                        .iter()
                        .any(|dependency| &dependency.name == name)
                    {
                        // A crate the caller declares as development-only is audited as such even
                        // when the tree text cannot say so; the declaration is the evidence.
                        dependencies.retain(|dependency| &dependency.name != name);
                        dependencies.push(restricted::Dependency::new(
                            name,
                            restricted::DependencyKind::Dev,
                        ));
                    } else {
                        println!("dev dependency {name}: not in this tree");
                    }
                }
                findings.extend(restricted::audit(&dependencies, restricted_profile));
            }
            Err(error) => return report(format!("{path} could not be read: {error}")),
        }
    }
    for finding in &findings {
        println!("{}", finding.render());
    }

    let mut gate = Gate::new(restricted_profile);
    match build {
        Some(true) => gate.record(Proof::Built),
        Some(false) => {}
        None => {}
    }
    match gate.verdict() {
        Ok(proof) => println!("target build: {}", proof.name()),
        Err(reason) => println!("unverified: {reason}"),
    }
    let gate_ok = gate.verdict().is_ok();
    if check && findings.is_empty() && gate_ok {
        println!("restricted target: ready");
    }
    !check || (findings.is_empty() && gate_ok)
}

/// Discovers a local device, exports a package and plans an install (task T18.06).
pub(super) fn target_local(options: &crate::cli::TargetLocalOptions) -> bool {
    let mut profile = String::from("desktop");
    let mut host = Os::host();
    let mut platform = MobilePlatform::Android;
    let mut programs: Vec<String> = Vec::new();
    let mut declared_programs = false;
    let mut discovery = Discovery::new();
    let mut selected: Option<String> = None;
    let mut package: Option<Package> = None;
    let mut export_directory: Option<String> = None;
    let mut install = false;
    let mut remote: Option<String> = None;
    let mut failed_step: Option<(String, String)> = None;
    let mut previous: Option<String> = None;
    let mut discover: Option<MobilePlatform> = None;
    let mut check = false;
    let mut failed = false;

    for op in &options.ops {
        let (verb, rest) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<(), String> = match verb {
            "profile" => {
                profile = rest.to_owned();
                Ok(())
            }
            "host" => Os::parse(rest)
                .map(|parsed| host = parsed)
                .ok_or_else(|| format!("{rest:?} is not macos, linux or windows")),
            "platform" => MobilePlatform::parse(rest)
                .map(|parsed| platform = parsed)
                .ok_or_else(|| format!("{rest:?} is not ios or android")),
            "program" => {
                declared_programs = true;
                programs.push(rest.to_owned());
                Ok(())
            }
            "device" => split_fields(rest, 3)
                .ok_or_else(|| format!("device needs <id>:<model>:<yes|no>, got {rest:?}"))
                .and_then(|fields| {
                    let authorized = flag(fields[2])
                        .ok_or_else(|| format!("{:?} is not yes or no", fields[2]))?;
                    discovery
                        .add(Device::new(fields[0], fields[1], authorized))
                        .map_err(|refusal| refusal.render())
                }),
            "select" => discovery
                .select(rest)
                .map(|device| {
                    println!("selected {}", device.render());
                    selected = Some(rest.to_owned());
                })
                .map_err(|refusal| refusal.render()),
            "discover" => match rest {
                "" => {
                    discover = Some(platform);
                    Ok(())
                }
                "ios" | "android" => {
                    discover = MobilePlatform::parse(rest);
                    Ok(())
                }
                other => Err(format!(
                    "{other:?} is not ios, android or empty; discovery asks a real local tool"
                )),
            },
            "authorize" => discovery
                .authorize(rest)
                .map(|()| println!("authorized {rest}"))
                .map_err(|refusal| refusal.render()),
            "package" => split_fields(rest, 3)
                .ok_or_else(|| {
                    format!("package needs <identifier>:<version>:<artifact>, got {rest:?}")
                })
                .map(|fields| {
                    package = Some(Package::new(
                        fields[0],
                        fields[1],
                        profile.clone(),
                        fields[2],
                    ));
                }),
            "export" => {
                export_directory = Some(rest.to_owned());
                Ok(())
            }
            "install" => {
                install = true;
                Ok(())
            }
            "remote" => {
                remote = Some(rest.to_owned());
                Ok(())
            }
            "fail" => split_fields(rest, 2)
                .ok_or_else(|| format!("fail needs <step>:<reason>, got {rest:?}"))
                .map(|fields| failed_step = Some((fields[0].to_owned(), fields[1].to_owned()))),
            "previous" => {
                previous = Some(rest.to_owned());
                Ok(())
            }
            "check" => {
                check = true;
                Ok(())
            }
            "show" => Ok(()),
            other => Err(format!("{other:?} is not a local-delivery operation")),
        };
        if let Err(reason) = step {
            failed = true;
            println!(
                "{}",
                line_with("shell.error.prefix", &[("message", reason.as_str())])
            );
        }
    }

    let probe = if declared_programs {
        Probe::of(programs)
    } else {
        Probe::from_path()
    };
    if !crate::app::targets::configuration::is_profile(&profile) {
        return report(format!(
            "{profile:?} is not one of the five target profiles"
        ));
    }
    let toolbox = Toolbox::for_target(&profile, host, platform, &probe);
    for line in toolbox.render() {
        println!("{line}");
    }

    if let Some(platform) = discover {
        match local::discover(platform, &probe) {
            Ok(found) => {
                println!(
                    "discovered {} {} device(s) from the platform's own tool",
                    found.devices().len(),
                    platform.name()
                );
                for device in found.devices() {
                    println!("  {}", device.render());
                    if let Err(refusal) = discovery.add(device.clone()) {
                        return report(refusal.render());
                    }
                }
            }
            Err(refusal) => return report(refusal.render()),
        }
    }

    if let Some(directory) = export_directory {
        match &package {
            None => {
                return report("export needs package:<identifier>:<version>:<artifact>".to_owned());
            }
            Some(package) => {
                if !package.path.exists() {
                    return report(format!(
                        "there is no artifact at {} to export",
                        package.path.display()
                    ));
                }
                match local::export(package, std::path::Path::new(&directory)) {
                    Ok(path) => println!("exported {}", path.display()),
                    Err(refusal) => return report(refusal.render()),
                }
            }
        }
    }

    if install {
        let Some(package) = &package else {
            return report("install needs package:<identifier>:<version>:<artifact>".to_owned());
        };
        let Some(device) = &selected else {
            return report(
                "install needs a device: add select:<id> after authorizing one".to_owned(),
            );
        };
        let transport = match &remote {
            Some(host) => Transport::Remote { host: host.clone() },
            None => Transport::Local,
        };
        match local::plan_install(&toolbox, &discovery, device, package, &transport) {
            Ok(plan) => {
                println!("{}", plan.render());
                println!("plan only: no program was started");
            }
            Err(refusal) => return report(refusal.render()),
        }
    }

    if let Some((step, reason)) = failed_step {
        let mut journal = Journal::new();
        journal.succeeded("export package");
        journal.failed(step, reason);
        let recovery = Recovery {
            device: selected.clone().unwrap_or_else(|| "the device".to_owned()),
            previous,
            journal,
        };
        match recovery.rollback() {
            Ok(message) => println!("recovery: {message}"),
            Err(reason) => {
                println!("recovery: {reason}");
                failed = true;
            }
        }
    }

    let missing = toolbox
        .tools()
        .iter()
        .filter(|tool| matches!(tool.state, ToolState::Missing { .. }))
        .count();
    if check {
        if missing == 0 {
            println!("local tools: ready");
        } else {
            println!("local tools: {missing} missing");
        }
    }
    !failed && (!check || missing == 0)
}

/// Queues builds and deliveries and tracks their artifacts (task T18.07).
pub(super) fn target_queue(options: &crate::cli::TargetQueueOptions) -> bool {
    let mut queue = Queue::new();
    let mut secrets: Vec<String> = Vec::new();
    let mut last: Option<String> = None;
    let mut host = Os::host();
    let mut check = false;

    for op in &options.ops {
        let (verb, rest) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<(), String> = match verb {
            "job" => split_fields(rest, 3)
                .ok_or_else(|| format!("job needs <id>:<target>:<action>, got {rest:?}"))
                .and_then(|fields| {
                    let action = crate::app::targets::queue::Action::parse(fields[2])
                        .ok_or_else(|| format!("{:?} is not build, deploy or sign", fields[2]))?;
                    queue
                        .enqueue(fields[0], fields[1], action)
                        .map_err(|refusal| refusal.render())?;
                    last = Some(fields[0].to_owned());
                    Ok(())
                }),
            "signing" => split_fields(rest, 3)
                .ok_or_else(|| {
                    format!("signing needs <identity>:<credential>:<source>, got {rest:?}")
                })
                .and_then(|fields| {
                    let source = CredentialSource::parse(fields[2]).ok_or_else(|| {
                        format!("{:?} is not environment, keychain or file", fields[2])
                    })?;
                    let Some(id) = &last else {
                        return Err("signing needs a job before it".to_owned());
                    };
                    queue
                        .attach_signing(
                            id,
                            Signing::new(fields[0], CredentialRef::new(fields[1], source)),
                        )
                        .map_err(|refusal| refusal.render())
                }),
            "start" => queue.start(rest).map_err(|refusal| refusal.render()),
            "host" => Os::parse(rest)
                .map(|parsed| host = parsed)
                .ok_or_else(|| format!("{rest:?} is not macos, linux or windows")),
            "sign" => split_fields(rest, 5)
                .ok_or_else(|| {
                    format!(
                        "sign needs <id>:<identity>:<artifact>:<credential>:<source>, got {rest:?}"
                    )
                })
                .and_then(|fields| {
                    let source = CredentialSource::parse(fields[4]).ok_or_else(|| {
                        format!("{:?} is not environment, keychain or file", fields[4])
                    })?;
                    queue
                        .attach_signing(
                            fields[0],
                            Signing::new(fields[1], CredentialRef::new(fields[3], source)),
                        )
                        .map_err(|refusal| refusal.render())?;
                    let signature = queue
                        .sign(fields[0], std::path::Path::new(fields[2]), host)
                        .map_err(|refusal| refusal.render())?;
                    println!("signature {}", signature.render());
                    Ok(())
                }),
            "pass" => {
                let Some((id, rest)) = rest.split_once(':') else {
                    return report(format!(
                        "pass needs <id>:<proof>[:<artifact>], got {rest:?}"
                    ));
                };
                let (proof, artifact) = rest.split_once(':').unwrap_or((rest, ""));
                let Some(proof) = Proof::parse(proof) else {
                    return report(format!("{proof:?} is not a proof level"));
                };
                let measured = if artifact.is_empty() {
                    None
                } else {
                    match Artifact::of_file(id, artifact) {
                        Ok(artifact) => Some(artifact),
                        Err(reason) => return report(reason),
                    }
                };
                queue
                    .pass(id, proof, measured)
                    .map_err(|refusal| refusal.render())
            }
            "fail" => split_fields(rest, 2)
                .ok_or_else(|| format!("fail needs <id>:<reason>, got {rest:?}"))
                .and_then(|fields| {
                    queue
                        .fail(fields[0], fields[1])
                        .map_err(|refusal| refusal.render())
                }),
            "cancel" => queue.cancel(rest).map_err(|refusal| refusal.render()),
            "secret" => {
                secrets.push(rest.to_owned());
                Ok(())
            }
            "log" => {
                let borrowed: Vec<&str> = secrets.iter().map(String::as_str).collect();
                println!("{}", redact(rest, &borrowed));
                Ok(())
            }
            "check" => {
                check = true;
                Ok(())
            }
            "show" => Ok(()),
            other => Err(format!("{other:?} is not a queue operation")),
        };
        if let Err(reason) = step {
            return report(reason);
        }
    }

    for line in queue.render() {
        println!("{line}");
    }
    for job in queue.jobs() {
        if let Some(line) = job.signature_line() {
            println!("signature {}: {line}", job.id);
        }
    }
    println!("{}", queue.summary().render());
    for artifact in queue.artifacts() {
        println!("artifact {}", artifact.render());
    }
    !check || queue.is_success()
}

/// Fills the five-profile certification report (task T18.08).
pub(super) fn target_certification(options: &crate::cli::TargetCertificationOptions) -> bool {
    let mut board = Report::new();
    let mut profile = String::from("desktop");
    let mut check = false;

    for op in &options.ops {
        let (verb, rest) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<(), String> = match verb {
            "profile" => {
                profile = rest.to_owned();
                Ok(())
            }
            "stage" => split_fields(rest, 4)
                .ok_or_else(|| {
                    format!("stage needs <stage>:<proof>:<command>:<note>, got {rest:?}")
                })
                .and_then(|fields| {
                    let stage = Stage::parse(fields[0])
                        .ok_or_else(|| format!("{:?} is not a stage", fields[0]))?;
                    let proof = Proof::parse(fields[1])
                        .ok_or_else(|| format!("{:?} is not a proof level", fields[1]))?;
                    board.record(&profile, stage, proof, fields[2], fields[3])
                }),
            "consistency" => match certification::measure_consistency() {
                Ok(reports) => {
                    for line in reports.iter().map(|report| report.render()) {
                        println!("consistency {}", line);
                    }
                    board.add_consistency(reports);
                    Ok(())
                }
                Err(reason) => Err(reason),
            },
            "check" => {
                check = true;
                Ok(())
            }
            "show" => Ok(()),
            other => Err(format!("{other:?} is not a certification operation")),
        };
        if let Err(reason) = step {
            return report(reason);
        }
    }

    for line in board.render() {
        println!("{line}");
    }
    let unverified = board.unverified();
    if !unverified.is_empty() {
        println!("unverified: {} cell(s)", unverified.len());
        for cell in &unverified {
            println!("  {cell}");
        }
    }
    if check && board.is_certified() {
        println!("certification: complete");
    }
    !check || board.is_certified()
}
