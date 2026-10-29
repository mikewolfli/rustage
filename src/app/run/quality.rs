// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The `--quality` command: the problem centre, design checks and evidence (task F21).

use crate::app::logic::core::Value;
use crate::app::preview::load::Load;
use crate::app::preview::preview_service::{Document, DocumentNode};
use crate::app::preview::visual_baseline::{Image, Shot, Tolerance};
use crate::app::quality::{
    BindingFact, BindingTarget, BindingType, Center, DesignFacts, Evidence, Extent, Filter, Frame,
    LayoutFact, Overlap, Problem, Provenance, RunReport, Selection, Severity, Source, TestConfig,
    TestRunner, TestStatus, Tests, VisualCondition, Wizard, contrast,
};
use crate::contracts::identity::NodeId;
use crate::host::tasks::CancelToken;
use std::time::Duration;

/// Runs the quality operations, printing each answer as it is read.
pub(super) fn quality(options: &crate::cli::QualityOptions) -> bool {
    let mut center = Center::new();
    let mut tests = Tests::new();
    let mut facts = DesignFacts::default();
    let mut filter = Filter::default();
    let mut target = "desktop".to_owned();
    let mut revision = 0;
    let mut current = Problem {
        source: Source::Design,
        target: target.clone(),
        revision,
        node: None,
        location: None,
        severity: Severity::Error,
        message: String::new(),
    };
    let mut visual_condition: Option<VisualCondition> = None;
    let mut visual = crate::app::quality::VisualRun::new();
    let mut a11y = crate::app::quality::A11yCheck::new("desktop");
    let mut test_tree: Option<crate::app::quality::TestTree> = None;
    // The test wizard's declared plan, until `wizard-run` or `wizard-save` uses it (T21.05).
    let mut wizard: Option<Wizard> = None;
    // The stop policy later test operations use, and whether every test operation so far was a
    // complete pass. A run that leaves a test not run is not a pass, so it must not exit like one.
    let mut stop = Stop::default();
    let mut passed = true;
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "source" => match Source::parse(argument) {
                Some(source) => {
                    current.source = source;
                    Ok(vec![format!("source {}", source.name())])
                }
                None => Err(format!("{argument:?} is not a problem source")),
            },
            "target" => {
                target = argument.to_owned();
                current.target = target.clone();
                Ok(vec![format!("target {target}")])
            }
            "revision" => match argument.parse() {
                Ok(value) => {
                    revision = value;
                    current.revision = value;
                    center.advance(value);
                    Ok(vec![format!("revision {revision}")])
                }
                Err(_) => Err(format!("{argument:?} is not a revision")),
            },
            "node" => match argument.parse() {
                Ok(value) => {
                    current.node = Some(value);
                    Ok(vec![format!("node {value}")])
                }
                Err(_) => Err(format!("{argument:?} is not a node id")),
            },
            "location" => {
                current.location = Some(argument.to_owned());
                Ok(vec![format!("location {argument}")])
            }
            "severity" => match Severity::parse(argument) {
                Some(severity) => {
                    current.severity = severity;
                    Ok(vec![format!("severity {}", severity.name())])
                }
                None => Err(format!("{argument:?} is not a severity")),
            },
            "problem" => {
                if argument.trim().is_empty() {
                    return super::report("a problem message is required".to_owned());
                }
                current.message = argument.to_owned();
                center.add(current.clone());
                Ok(vec![
                    format!("problem: {}", current.render()),
                    format!(
                        "navigate: {}",
                        Center::navigate(&current).unwrap_or_else(|| "(no location)".to_owned())
                    ),
                ])
            }
            "filter" => match Source::parse(argument) {
                Some(source) => {
                    filter.source = Some(source);
                    Ok(vec![format!("filter source {}", source.name())])
                }
                None => Err(format!("{argument:?} is not a problem source")),
            },
            "filter-target" => {
                filter.target = Some(argument.to_owned());
                Ok(vec![format!("filter target {argument}")])
            }
            "filter-revision" => match argument.parse() {
                Ok(value) => {
                    filter.revision = Some(value);
                    Ok(vec![format!("filter revision >= {value}")])
                }
                Err(_) => Err(format!("{argument:?} is not a revision")),
            },
            "duplicate" => match argument.parse() {
                Ok(node) => {
                    facts.nodes.push(node);
                    Ok(vec![format!("node {node}")])
                }
                Err(_) => Err(format!("{argument:?} is not a node id")),
            },
            "parent" => match argument.split_once(':') {
                Some((node, parent)) => match (node.parse::<u64>(), parent.parse::<u64>()) {
                    (Ok(node), Ok(parent)) => {
                        facts.parents.insert(node, Some(parent));
                        facts.nodes.push(node);
                        Ok(vec![format!("parent {node} -> {parent}")])
                    }
                    _ => Err(format!("{argument:?} is not <node>:<parent>")),
                },
                None => match argument.parse::<u64>() {
                    Ok(node) => {
                        facts.parents.insert(node, None);
                        facts.nodes.push(node);
                        Ok(vec![format!("root {node}")])
                    }
                    Err(_) => Err(format!("{argument:?} is not a node id")),
                },
            },
            "capability" => match argument.rsplit_once(':') {
                Some((name, available)) => match parse_bool(available) {
                    Ok(available) => {
                        facts.capabilities.insert(name.to_owned(), available);
                        Ok(vec![format!("capability {name} {available}")])
                    }
                    Err(reason) => Err(reason),
                },
                None => Err(format!("{argument:?} is not <name>:<true|false>")),
            },
            "capacity" => match argument.split_once(':') {
                Some((used, limit)) => match (used.parse::<usize>(), limit.parse::<usize>()) {
                    (Ok(used), Ok(limit)) => {
                        facts.capacity = (used, limit);
                        facts.capacity_provenance = Provenance::Supplied;
                        facts.capacity_limits = None;
                        Ok(vec![format!("capacity {used}/{limit}")])
                    }
                    _ => Err(format!("{argument:?} is not <used>:<limit>")),
                },
                None => Err(format!("{argument:?} is not <used>:<limit>")),
            },
            "contract-control" => match parse_contract_control(argument) {
                Ok((profile, names)) => {
                    let borrowed: Vec<&str> = names.iter().map(String::as_str).collect();
                    // The host factory, so a suite control (`form`/`form_set`) is answerable.
                    match facts.capability_from(
                        crate::app::contract::factory(),
                        &profile,
                        &borrowed,
                    ) {
                        Ok(answers) => Ok(answers
                            .iter()
                            .map(|answer| {
                                let provenance = facts
                                    .capability_provenance(&answer.name)
                                    .unwrap_or(Provenance::Contract);
                                format!(
                                    "capability {} {} ({} {profile})",
                                    answer.name,
                                    answer.level.name(),
                                    provenance.name()
                                )
                            })
                            .collect()),
                        Err(reason) => Err(reason),
                    }
                }
                Err(reason) => Err(reason),
            },
            "contract-capacity" => {
                let profile = argument.trim();
                if profile.is_empty() {
                    Err("contract-capacity needs <profile>".to_owned())
                } else {
                    match facts.capacity_from(profile) {
                        Ok(limits) => Ok(vec![format!(
                            "capacity {}/{} from surface policy {profile} (textures {}, font cache {} bytes, event queue {})",
                            facts.capacity.0,
                            facts.capacity.1,
                            limits.max_texture,
                            limits.font_cache_bytes,
                            limits.event_queue
                        )]),
                        Err(reason) => Err(reason),
                    }
                }
            }
            "load" => {
                let cycles: u32 = if argument.is_empty() {
                    100
                } else {
                    match argument.parse() {
                        Ok(cycles) => cycles,
                        Err(_) => {
                            return super::report(format!("{argument:?} is not a cycle count"));
                        }
                    }
                };
                let document = Document::new("group_box").with(DocumentNode::new(
                    NodeId::from_raw(1),
                    "label",
                    "load",
                ));
                let mut load = Load::new().with_budget(5_000);
                if let Err(reason) = load.drive("desktop", cycles, &document) {
                    return super::report(reason);
                }
                let mut lines = load.render();
                if !load.passed() || load.resources_steady() == Some(false) {
                    return super::report(load.verdict());
                }
                lines.push(format!("load passed: {cycles} real cycle(s)"));
                Ok(lines)
            }
            "resource" => match argument.rsplit_once(':') {
                Some((name, present)) => match parse_bool(present) {
                    Ok(present) => {
                        facts.resources.insert(name.to_owned(), present);
                        Ok(vec![format!("resource {name} {present}")])
                    }
                    Err(reason) => Err(reason),
                },
                None => Err(format!("{argument:?} is not <name>:<true|false>")),
            },
            "focus" => match argument.parse() {
                Ok(node) => {
                    facts.focus_order.push(node);
                    Ok(vec![format!("focus {node}")])
                }
                Err(_) => Err(format!("{argument:?} is not a node id")),
            },
            "min" => match parse_node_extent(argument) {
                Ok((node, extent)) => {
                    layout_fact(&mut facts, node).minimum = Some(extent);
                    Ok(vec![format!(
                        "minimum {node} {}x{}",
                        extent.width, extent.height
                    )])
                }
                Err(reason) => Err(reason),
            },
            "max" => match parse_node_extent(argument) {
                Ok((node, extent)) => {
                    layout_fact(&mut facts, node).maximum = Some(extent);
                    Ok(vec![format!(
                        "maximum {node} {}x{}",
                        extent.width, extent.height
                    )])
                }
                Err(reason) => Err(reason),
            },
            "box" => match parse_node_frame(argument) {
                Ok((node, frame)) => {
                    layout_fact(&mut facts, node).frame = Some(frame);
                    Ok(vec![format!(
                        "box {node} {},{} {}x{}",
                        frame.x, frame.y, frame.width, frame.height
                    )])
                }
                Err(reason) => Err(reason),
            },
            "overlap" => match parse_node_overlap(argument) {
                Ok((node, policy)) => {
                    layout_fact(&mut facts, node).overlap = policy;
                    Ok(vec![format!("overlap {node} {}", policy.name())])
                }
                Err(reason) => Err(reason),
            },
            "bind" => match parse_binding(argument) {
                Ok(binding) => {
                    let line = format!("binding {} -> {}", binding.source, binding.target.render());
                    facts.bindings.push(binding);
                    Ok(vec![line])
                }
                Err(reason) => Err(reason),
            },
            "check" => {
                let problems = facts.check(&target, revision);
                let lines = problems.iter().map(Problem::render).collect();
                for problem in problems {
                    center.add(problem);
                }
                Ok(lines)
            }
            "contrast" => {
                let parts: Vec<&str> = argument.split(':').collect();
                match parts.as_slice() {
                    [foreground, background, minimum] => {
                        let foreground = parse_colour(foreground);
                        let background = parse_colour(background);
                        let minimum_text = *minimum;
                        let minimum = minimum_text.parse::<f64>();
                        match (foreground, background, minimum) {
                            (Ok(foreground), Ok(background), Ok(minimum)) => {
                                let ratio = contrast(foreground, background);
                                Ok(vec![format!(
                                    "contrast {ratio:.2}:1 (minimum {minimum:.2}:1) {}",
                                    if ratio >= minimum { "pass" } else { "fail" }
                                )])
                            }
                            (Err(reason), _, _) => Err(reason),
                            (_, Err(reason), _) => Err(reason),
                            (_, _, Err(_)) => {
                                Err(format!("{minimum_text:?} is not a contrast ratio"))
                            }
                        }
                    }
                    _ => Err(format!("contrast needs <fg>:<bg>:<min>, got {argument:?}")),
                }
            }
            "test" => {
                let parts: Vec<&str> = argument.split(':').collect();
                let (name, status, seconds) = match parts.as_slice() {
                    [name, status] => (*name, *status, None),
                    [name, status, seconds] => (
                        *name,
                        *status,
                        match seconds.parse::<f64>() {
                            Ok(value) => Some(value),
                            Err(_) => {
                                return super::report(format!("{seconds:?} is not seconds"));
                            }
                        },
                    ),
                    _ => return super::report("test needs <name>:<status>[:<seconds>]".to_owned()),
                };
                let status = match TestStatus::parse(status) {
                    Some(status) => status,
                    None => return super::report(format!("{status:?} is not a test status")),
                };
                tests.record(name, status, seconds);
                Ok(vec![format!("test {name}: {}", status.name())])
            }
            "tests" => {
                let mut lines: Vec<String> = tests
                    .tally()
                    .into_iter()
                    .map(|(status, count)| format!("{status}: {count}"))
                    .collect();
                lines.push(match tests.revision() {
                    Some(recorded) if tests.is_stale(revision) => {
                        format!("results: revision {recorded} (current {revision}) — stale")
                    }
                    Some(recorded) => format!("results: revision {recorded} (current)"),
                    None => "results: no run recorded".to_owned(),
                });
                Ok(lines)
            }
            "test-discover" => {
                if argument.trim().is_empty() {
                    Err("test-discover needs a project directory".to_owned())
                } else {
                    let runner = TestRunner::new(TestConfig::in_directory(argument));
                    runner.discover_into(&mut tests).map(|discovery| {
                        test_tree = Some(crate::app::quality::TestTree::from_discovery(&discovery));
                        let mut line = format!(
                            "discovered {} test(s), {} ignored, {} benchmark(s)",
                            discovery.tests.len(),
                            discovery.ignored(),
                            discovery.benchmarks
                        );
                        if discovery.truncated {
                            line.push_str(" (output truncated: the list may be incomplete)");
                        }
                        if discovery.unparsed > 0 {
                            line.push_str(&format!(
                                "; {} line(s) were not understood, so the list is incomplete",
                                discovery.unparsed
                            ));
                        }
                        if let Some(reason) = &discovery.incomplete {
                            line.push_str(&format!("; {reason}"));
                        }
                        vec![line]
                    })
                }
            }
            "test-run" => {
                if argument.trim().is_empty() {
                    Err("test-run needs a project directory".to_owned())
                } else {
                    // Once discovery has run, the automatic selection is the tree's non-ignored
                    // tests by exact path. An empty answer runs nothing rather than falling back to
                    // "everything", which is how a side-effecting ignored test would be started.
                    let selection = match &test_tree {
                        Some(tree) => {
                            let names = tree.auto_run();
                            if names.is_empty() {
                                return super::report(
                                    "no test would run: every discovered test is ignored"
                                        .to_owned(),
                                );
                            }
                            Selection::exact(names)
                        }
                        None => Selection::all(),
                    };
                    let runner = TestRunner::new(TestConfig::in_directory(argument));
                    record_step(
                        run_tests(
                            &runner,
                            &selection,
                            revision,
                            &target,
                            &mut tests,
                            &mut center,
                            stop,
                        ),
                        &mut passed,
                    )
                }
            }
            "test-tree" => match &test_tree {
                Some(tree) => {
                    let mut lines = tree.render();
                    if !argument.trim().is_empty() {
                        let names = tree.matching(argument);
                        lines.push(format!(
                            "matching {argument:?}: {}",
                            if names.is_empty() {
                                "(none)".to_owned()
                            } else {
                                names.join(", ")
                            }
                        ));
                    }
                    Ok(lines)
                }
                None => Err("run test-discover first".to_owned()),
            },
            "test-filter" => match argument.split_once(':') {
                Some((directory, filter))
                    if !directory.trim().is_empty() && !filter.trim().is_empty() =>
                {
                    let runner = TestRunner::new(TestConfig::in_directory(directory));
                    record_step(
                        run_tests(
                            &runner,
                            &Selection::filter(filter),
                            revision,
                            &target,
                            &mut tests,
                            &mut center,
                            stop,
                        ),
                        &mut passed,
                    )
                }
                _ => Err("test-filter needs <directory>:<filter>".to_owned()),
            },
            "test-select" => match argument.split_once(':') {
                Some((directory, names)) if !directory.trim().is_empty() => {
                    let names: Vec<&str> = names
                        .split(',')
                        .map(str::trim)
                        .filter(|name| !name.is_empty())
                        .collect();
                    if names.is_empty() {
                        return super::report(
                            "test-select needs at least one test name".to_owned(),
                        );
                    }
                    let runner = TestRunner::new(TestConfig::in_directory(directory));
                    record_step(
                        run_tests(
                            &runner,
                            &Selection::exact(names),
                            revision,
                            &target,
                            &mut tests,
                            &mut center,
                            stop,
                        ),
                        &mut passed,
                    )
                }
                _ => Err("test-select needs <directory>:<name>[,<name>…]".to_owned()),
            },
            "test-cancel" => match Stop::parse(argument) {
                Ok(chosen) => {
                    stop = chosen;
                    Ok(vec![chosen.describe()])
                }
                Err(reason) => Err(reason),
            },
            "test-rerun" => {
                if argument.trim().is_empty() {
                    return super::report("test-rerun needs a project directory".to_owned());
                }
                let runner = TestRunner::new(TestConfig::in_directory(argument));
                // With a discovered tree, the rerun names the failed tests by exact path and skips
                // ignored ones; without it, the runner's own failed selection is used.
                match &test_tree {
                    Some(tree) => {
                        let names = tree.failed(&tests);
                        if names.is_empty() {
                            Ok(vec!["no failed test to rerun".to_owned()])
                        } else {
                            record_step(
                                run_tests(
                                    &runner,
                                    &Selection::exact(names),
                                    revision,
                                    &target,
                                    &mut tests,
                                    &mut center,
                                    stop,
                                ),
                                &mut passed,
                            )
                        }
                    }
                    None => {
                        let token = stop.token();
                        match runner.rerun_failed_cancellable(
                            revision,
                            &target,
                            &mut tests,
                            &mut center,
                            &token,
                        ) {
                            Ok((selection, report)) => {
                                tests.set_revision(revision);
                                record_step(
                                    Ok((report_lines(&report), report.complete_for(&selection))),
                                    &mut passed,
                                )
                            }
                            Err(reason) => Err(reason),
                        }
                    }
                }
            }
            "junit" => {
                if argument.is_empty() {
                    Err("junit needs a file".to_owned())
                } else {
                    if let Err(error) = std::fs::write(argument, tests.junit()) {
                        return super::report(format!("cannot write {argument}: {error}"));
                    }
                    Ok(vec![format!("wrote JUnit XML to {argument}")])
                }
            }
            "junit-import" => {
                if argument.is_empty() {
                    return super::report("junit-import needs a file".to_owned());
                }
                let text = match std::fs::read_to_string(argument) {
                    Ok(text) => text,
                    Err(error) => {
                        return super::report(format!("cannot read {argument}: {error}"));
                    }
                };
                let imported = match crate::app::quality::JUnitImport::from_xml(&text) {
                    Ok(imported) => imported,
                    Err(reason) => return super::report(reason),
                };
                if let Err(reason) = imported.matches(revision, &target) {
                    return super::report(reason);
                }
                let count = imported.tests.results().len();
                if imported.properties.contains_key("revision") {
                    tests.set_revision(revision);
                }
                for result in imported.tests.results() {
                    tests.record(result.name.clone(), result.status, result.seconds);
                }
                Ok(vec![format!(
                    "imported {count} test result(s) from {argument} ({})",
                    imported.environment()
                )])
            }
            "a11y-profile" => {
                if argument.trim().is_empty() {
                    Err("a11y-profile needs a profile".to_owned())
                } else {
                    a11y.set_profile(argument);
                    Ok(vec![format!("a11y profile {argument}")])
                }
            }
            "a11y-node" => match argument.splitn(5, ':').collect::<Vec<_>>().as_slice() {
                [id, role, focusable, focused, label] => {
                    let id: u64 = match id.parse() {
                        Ok(id) => id,
                        Err(_) => return super::report(format!("{id:?} is not a node id")),
                    };
                    let Some(role) = crate::app::quality::parse_role(role) else {
                        return super::report(format!("{role:?} is not an accessibility role"));
                    };
                    let focusable = match parse_bool(focusable) {
                        Ok(value) => value,
                        Err(reason) => return super::report(reason),
                    };
                    let focused = match parse_bool(focused) {
                        Ok(value) => value,
                        Err(reason) => return super::report(reason),
                    };
                    let mut element = crate::app::quality::Element::new(id, role, *label);
                    element.focusable = focusable;
                    element.focused = focused;
                    a11y.add(element);
                    Ok(vec![format!(
                        "a11y node {id} ({})",
                        crate::app::quality::role_name(role)
                    )])
                }
                _ => Err(format!(
                    "a11y-node needs <id>:<role>:<focusable>:<focused>:<label>, got {argument:?}"
                )),
            },
            "a11y-child" => match argument.split_once(':') {
                Some((parent, child)) => match (parent.parse::<u64>(), child.parse::<u64>()) {
                    (Ok(parent), Ok(child)) => match a11y.add_child(parent, child) {
                        Ok(()) => Ok(vec![format!("a11y child {parent} -> {child}")]),
                        Err(reason) => Err(reason),
                    },
                    _ => Err(format!("{argument:?} is not <parent>:<child>")),
                },
                None => Err(format!("{argument:?} is not <parent>:<child>")),
            },
            "a11y-label-for" => match argument.split_once(':') {
                Some((node, label)) => match (node.parse::<u64>(), label.parse::<u64>()) {
                    (Ok(node), Ok(label)) => match a11y.label_for(node, label) {
                        Ok(()) => Ok(vec![format!("a11y label {label} names {node}")]),
                        Err(reason) => Err(reason),
                    },
                    _ => Err(format!("{argument:?} is not <node>:<label>")),
                },
                None => Err(format!("{argument:?} is not <node>:<label>")),
            },
            "a11y-control-for" => match argument.split_once(':') {
                Some((label, node)) => match (label.parse::<u64>(), node.parse::<u64>()) {
                    (Ok(label), Ok(node)) => match a11y.control_for(label, node) {
                        Ok(()) => Ok(vec![format!("a11y label {label} names control {node}")]),
                        Err(reason) => Err(reason),
                    },
                    _ => Err(format!("{argument:?} is not <label>:<node>")),
                },
                None => Err(format!("{argument:?} is not <label>:<node>")),
            },
            "a11y-manual" => match argument.splitn(3, ':').collect::<Vec<_>>().as_slice() {
                [tool, result, note] => match crate::app::quality::ManualResult::parse(result) {
                    Some(result) => {
                        a11y.set_manual(crate::app::quality::ManualCheck {
                            tool: (*tool).to_owned(),
                            result,
                            note: (*note).to_owned(),
                        });
                        Ok(vec![format!("a11y manual {tool}: {result:?}")])
                    }
                    None => Err(format!("{result:?} is not not-performed, passed or failed")),
                },
                _ => Err(format!(
                    "a11y-manual needs <tool>:<result>:<note>, got {argument:?}"
                )),
            },
            "a11y" => {
                let report = a11y.run();
                if !report.automated_passed() {
                    passed = false;
                }
                Ok(report.render())
            }
            "a11y-accept" => {
                let report = a11y.run();
                if !report.accepted() {
                    passed = false;
                }
                Ok(report.render())
            }
            "visual" => match parse_visual(argument) {
                Ok(condition) => {
                    visual_condition = Some(condition.clone());
                    Ok(vec![format!(
                        "visual {} at {}% font {} resources {}",
                        condition.platform,
                        condition.scale,
                        condition.font_digest,
                        condition.resources
                    )])
                }
                Err(reason) => Err(reason),
            },
            "visual-compare" => match parse_visual(argument) {
                Ok(other) => match &visual_condition {
                    Some(baseline) => match baseline.compare(&other) {
                        Ok(()) => Ok(vec!["visual conditions match".to_owned()]),
                        Err(reason) => Ok(vec![format!("visual conditions differ: {reason}")]),
                    },
                    None => Err("no visual baseline was set".to_owned()),
                },
                Err(reason) => Err(reason),
            },
            "visual-tolerance" => match argument.parse::<f64>() {
                Ok(percent) if (0.0..=100.0).contains(&percent) => {
                    visual.set_tolerance(Tolerance::percentage(percent, 0));
                    Ok(vec![format!(
                        "visual tolerance: {}",
                        visual.tolerance().render()
                    )])
                }
                _ => Err(format!("{argument:?} is not a percentage")),
            },
            "visual-baseline" => match parse_shot(argument, &visual_condition) {
                Ok(shot) => {
                    let name = shot.name.clone();
                    match visual.record_baseline(shot) {
                        Ok(()) => Ok(vec![format!("baseline {name} recorded")]),
                        Err(reason) => Err(reason),
                    }
                }
                Err(reason) => Err(reason),
            },
            "visual-candidate" => match parse_shot(argument, &visual_condition) {
                Ok(shot) => {
                    let line = shot.render();
                    visual.capture(shot);
                    Ok(vec![format!("candidate: {line}")])
                }
                Err(reason) => Err(reason),
            },
            "visual-check" => {
                let evidence = visual.check();
                let lines = evidence.render();
                // A check that produced no comparison is not evidence, so it is not a pass either.
                if evidence.passed() != Some(true) {
                    passed = false;
                }
                Ok(lines)
            }
            "evidence" => {
                if argument.is_empty() {
                    Err("evidence needs a file".to_owned())
                } else {
                    center.advance(revision);
                    let evidence = Evidence {
                        revision,
                        target: target.clone(),
                        center: center.clone(),
                        tests: tests.clone(),
                        visual: visual.evidence(),
                    };
                    if let Err(error) = std::fs::write(argument, evidence.json()) {
                        return super::report(format!("cannot write {argument}: {error}"));
                    }
                    Ok(vec![format!("wrote evidence to {argument}")])
                }
            }
            "state" => {
                center.advance(revision);
                Ok(center.render(&filter))
            }
            "show" => {
                center.advance(revision);
                Ok(center.render(&filter))
            }
            "wizard" => {
                if argument.trim().is_empty() {
                    return super::report("wizard needs <name>".to_owned());
                }
                wizard = Some(Wizard::new(argument));
                Ok(vec![format!("wizard {argument}")])
            }
            "w-state" => wizard_declare(&mut wizard, |plan| {
                plan.state(argument).map(|()| format!("state {argument}"))
            }),
            "w-initial" => wizard_declare(&mut wizard, |plan| {
                plan.start_at(argument)
                    .map(|()| format!("initial {argument}"))
            }),
            "w-event" => match argument.split_once(':') {
                Some((name, token)) => match Value::parse(token) {
                    Some(payload) => {
                        let rendered = payload.render();
                        wizard_declare(&mut wizard, |plan| {
                            plan.event(name, payload)
                                .map(|()| format!("event {name} ({rendered})"))
                        })
                    }
                    None => Err(format!(
                        "{token:?} is not flag:<yes|no>, number:<n> or text:<value>"
                    )),
                },
                None => Err(format!(
                    "w-event needs <name>:<kind>:<value>, got {argument:?}"
                )),
            },
            "w-transition" => match split_fields::<3>(argument) {
                Some([from, event, to]) => wizard_declare(&mut wizard, |plan| {
                    plan.transition(from, event, to)
                        .map(|()| format!("transition {from} --{event}--> {to}"))
                }),
                None => Err(format!(
                    "w-transition needs <from>:<event>:<to>, got {argument:?}"
                )),
            },
            "w-transition-if" => match split_fields::<5>(argument) {
                Some([from, event, to, field, token]) => match Value::parse(token) {
                    Some(equals) => {
                        let rendered = equals.render();
                        wizard_declare(&mut wizard, |plan| {
                            plan.transition_if(from, event, to, field, equals).map(|()| {
                                format!("transition {from} --{event}--> {to} if {field} == {rendered}")
                            })
                        })
                    }
                    None => Err(format!(
                        "{token:?} is not flag:<yes|no>, number:<n> or text:<value>"
                    )),
                },
                None => Err(format!(
                    "w-transition-if needs <from>:<event>:<to>:<field>:<kind>:<value>, got {argument:?}"
                )),
            },
            "w-page" => wizard_declare(&mut wizard, |plan| {
                plan.page(argument).map(|()| format!("page {argument}"))
            }),
            "w-entry" => wizard_declare(&mut wizard, |plan| {
                plan.entry(argument).map(|()| format!("entry {argument}"))
            }),
            "w-edge" => match argument.split_once(':') {
                Some((from, to)) => wizard_declare(&mut wizard, |plan| {
                    plan.edge(from, to).map(|()| format!("edge {from} -> {to}"))
                }),
                None => Err(format!("w-edge needs <from>:<to>, got {argument:?}")),
            },
            "w-seam" => match argument.split_once(':') {
                Some((function, token)) => match Value::parse(token) {
                    Some(answer) => {
                        let rendered = answer.render();
                        wizard_declare(&mut wizard, |plan| {
                            plan.seam_answer(function, answer);
                            Ok(format!("seam {function} answers {rendered}"))
                        })
                    }
                    None => Err(format!(
                        "{token:?} is not flag:<yes|no>, number:<n> or text:<value>"
                    )),
                },
                None => Err(format!(
                    "w-seam needs <function>:<kind>:<value>, got {argument:?}"
                )),
            },
            "wizard-vectors" => match &wizard {
                Some(plan) => match plan.generate() {
                    Ok(vectors) => {
                        let mut lines = Vec::new();
                        for vector in &vectors.events {
                            let steps: Vec<String> = vector
                                .events
                                .iter()
                                .map(crate::app::logic::core::Event::render)
                                .collect();
                            lines.push(format!("vector {}: {}", vector.name, steps.join(", ")));
                        }
                        for vector in &vectors.navigation {
                            lines.push(format!(
                                "navigation {}: {} -> {}",
                                vector.name, vector.from, vector.to
                            ));
                        }
                        for state in &vectors.unreachable {
                            lines.push(format!("unreachable: {state} has no declared event walk"));
                        }
                        for guarded in &vectors.guarded {
                            lines.push(format!(
                                "not generated: {guarded} has a guard that reads runtime state"
                            ));
                        }
                        lines.push(format!(
                            "wizard {}: {} event vector(s), {} navigation vector(s) from {}",
                            plan.name(),
                            vectors.events.len(),
                            vectors.navigation.len(),
                            plan.initial().unwrap_or("(no initial state)")
                        ));
                        Ok(lines)
                    }
                    Err(reason) => Err(reason),
                },
                None => Err("start the wizard with wizard:<name> first".to_owned()),
            },
            "wizard-run" => match &wizard {
                Some(plan) => match plan.run() {
                    Ok(report) => {
                        if !report.passed() {
                            passed = false;
                        }
                        Ok(report.render())
                    }
                    Err(reason) => Err(reason),
                },
                None => Err("start the wizard with wizard:<name> first".to_owned()),
            },
            "wizard-save" => match &wizard {
                Some(plan) => {
                    if argument.is_empty() {
                        return super::report("wizard-save needs <file>".to_owned());
                    }
                    match plan.save(std::path::Path::new(argument)) {
                        Ok(bytes) => Ok(vec![format!("wrote {bytes} byte(s) to {argument}")]),
                        Err(reason) => Err(reason),
                    }
                }
                None => Err("start the wizard with wizard:<name> first".to_owned()),
            },
            "wizard-load" => {
                if argument.is_empty() {
                    return super::report("wizard-load needs <file>".to_owned());
                }
                match Wizard::load(std::path::Path::new(argument)) {
                    Ok(loaded) => {
                        let line = format!("loaded wizard {}", loaded.name());
                        wizard = Some(loaded);
                        Ok(vec![line])
                    }
                    Err(reason) => Err(reason),
                }
            }
            other => Err(format!("{other:?} is not a quality operation")),
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
    passed
}

/// What a later test operation may be asked to do about a stop request.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum Stop {
    /// Nothing is stopped; the run ends when cargo does.
    #[default]
    None,
    /// A stop is already requested, so no process is started and every selected test stays not run.
    BeforeStart,
    /// A stop is requested after this long, so a run that outlives it is terminated for real.
    After(Duration),
}

impl Stop {
    /// Parses `test-cancel` (nothing starts) and `test-cancel:<seconds>` (a run is stopped after
    /// that long).
    fn parse(argument: &str) -> Result<Self, String> {
        let argument = argument.trim();
        if argument.is_empty() {
            return Ok(Self::BeforeStart);
        }
        let seconds: f64 = argument
            .parse()
            .map_err(|_| format!("{argument:?} is not a number of seconds"))?;
        if !seconds.is_finite() || seconds <= 0.0 {
            return Err(format!(
                "{argument:?} is not a number of seconds greater than zero"
            ));
        }
        Ok(Self::After(Duration::from_secs_f64(seconds)))
    }

    /// What this policy does, in the caller's words.
    fn describe(self) -> String {
        match self {
            Self::None => "cancel: a run ends when cargo does".to_owned(),
            Self::BeforeStart => {
                "cancel: every later test run is stopped before it starts; its tests stay not run"
                    .to_owned()
            }
            Self::After(limit) => format!(
                "cancel: a later test run still working after {}s is stopped; tests without a \
                 result stay not run",
                limit.as_secs_f64()
            ),
        }
    }

    /// The token one test operation uses.
    fn token(self) -> CancelToken {
        let token = CancelToken::default();
        match self {
            Self::None => {}
            Self::BeforeStart => token.request(),
            Self::After(limit) => {
                let watcher = token.clone();
                // Detached on purpose: the run it stops is the only thing that waits for it, and a
                // run that finishes first leaves nothing to stop.
                std::thread::spawn(move || {
                    std::thread::sleep(limit);
                    watcher.request();
                });
            }
        }
        token
    }
}

/// Folds whether one test operation was a complete pass into the command's result.
///
/// The lines are still returned, so the report is printed before the exit code says it was not a
/// pass.
fn record_step(
    step: Result<(Vec<String>, bool), String>,
    passed: &mut bool,
) -> Result<Vec<String>, String> {
    let (lines, complete) = step?;
    *passed &= complete;
    Ok(lines)
}

/// Runs a test selection and turns the report into printable lines, with whether it was a complete
/// pass.
fn run_tests(
    runner: &TestRunner,
    selection: &Selection,
    revision: u64,
    target: &str,
    tests: &mut Tests,
    center: &mut Center,
    stop: Stop,
) -> Result<(Vec<String>, bool), String> {
    let report =
        runner.run_cancellable(selection, revision, target, tests, center, &stop.token())?;
    // The results that exist belong to this revision; a later revision makes them stale rather than
    // silently current.
    tests.set_revision(revision);
    Ok((report_lines(&report), report.complete_for(selection)))
}

/// The report in printable lines. A cancelled or incomplete run says so instead of reading as a
/// pass.
fn report_lines(report: &RunReport) -> Vec<String> {
    let mut lines = vec![format!(
        "tests: {} passed, {} failed, {} skipped, {} not run{}",
        report.passed(),
        report.failed(),
        report.skipped(),
        report.not_run.len(),
        if report.cancelled {
            " (cancelled: tests without a result stay not run)"
        } else {
            ""
        }
    )];
    for result in &report.observed {
        lines.push(format!("test {}: {}", result.name, result.status.name()));
    }
    for failure in &report.failures {
        let place = failure
            .location
            .as_deref()
            .map(|location| format!(" at {location}"))
            .unwrap_or_default();
        lines.push(format!(
            "failed {}{place}: {}",
            failure.name, failure.reason
        ));
        for detail in failure.detail.lines().take(12) {
            if !detail.trim().is_empty() {
                lines.push(format!("  {detail}"));
            }
        }
    }
    if !report.not_run.is_empty() {
        lines.push(format!("not run: {}", report.not_run.join(", ")));
    }
    if let Some(note) = &report.incomplete {
        lines.push(format!("note: {note}"));
    }
    lines
}

fn parse_bool(text: &str) -> Result<bool, String> {
    match text {
        "true" => Ok(true),
        "false" => Ok(false),
        other => Err(format!("{other:?} is not true or false")),
    }
}

/// One wizard declaration, which needs a plan to have been started.
fn wizard_declare(
    wizard: &mut Option<Wizard>,
    declare: impl FnOnce(&mut Wizard) -> Result<String, String>,
) -> Result<Vec<String>, String> {
    match wizard {
        Some(plan) => declare(plan).map(|line| vec![line]),
        None => Err("start the wizard with wizard:<name> first".to_owned()),
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

// These operations record facts the caller read from the document or the generator. The library's
// layout and binding contracts are not read here yet (see `DesignFacts`); a malformed fact is
// refused, and a fact the check cannot verify is reported as "cannot verify" rather than passing.
fn layout_fact(facts: &mut DesignFacts, node: u64) -> &mut LayoutFact {
    let index = match facts.layout.iter().position(|fact| fact.node == node) {
        Some(index) => index,
        None => {
            facts.layout.push(LayoutFact::new(node));
            facts.layout.len() - 1
        }
    };
    &mut facts.layout[index]
}

fn parse_node(text: &str) -> Result<u64, String> {
    text.parse()
        .map_err(|_| format!("{text:?} is not a node id"))
}

fn parse_length(text: &str) -> Result<f64, String> {
    let value = text
        .parse::<f64>()
        .map_err(|_| format!("{text:?} is not a length"))?;
    if value.is_finite() && value >= 0.0 {
        Ok(value)
    } else {
        Err(format!("{text:?} is not a non-negative length"))
    }
}

fn parse_coordinate(text: &str) -> Result<f64, String> {
    let value = text
        .parse::<f64>()
        .map_err(|_| format!("{text:?} is not a coordinate"))?;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(format!("{text:?} is not a finite coordinate"))
    }
}

fn parse_extent(text: &str) -> Result<Extent, String> {
    let (width, height) = text
        .split_once(',')
        .ok_or_else(|| format!("{text:?} is not <width>,<height>"))?;
    Ok(Extent {
        width: parse_length(width)?,
        height: parse_length(height)?,
    })
}

fn parse_node_extent(argument: &str) -> Result<(u64, Extent), String> {
    let (node, extent) = argument
        .split_once(':')
        .ok_or_else(|| format!("{argument:?} is not <node>:<width>,<height>"))?;
    Ok((parse_node(node)?, parse_extent(extent)?))
}

fn parse_node_frame(argument: &str) -> Result<(u64, Frame), String> {
    let (node, values) = argument
        .split_once(':')
        .ok_or_else(|| format!("{argument:?} is not <node>:<x>,<y>,<width>,<height>"))?;
    let parts: Vec<&str> = values.split(',').collect();
    match parts.as_slice() {
        [x, y, width, height] => Ok((
            parse_node(node)?,
            Frame {
                x: parse_coordinate(x)?,
                y: parse_coordinate(y)?,
                width: parse_length(width)?,
                height: parse_length(height)?,
            },
        )),
        _ => Err(format!(
            "{argument:?} is not <node>:<x>,<y>,<width>,<height>"
        )),
    }
}

fn parse_node_overlap(argument: &str) -> Result<(u64, Overlap), String> {
    let (node, policy) = argument
        .split_once(':')
        .ok_or_else(|| format!("{argument:?} is not <node>:<allowed|forbidden>"))?;
    let policy =
        Overlap::parse(policy).ok_or_else(|| format!("{policy:?} is not allowed or forbidden"))?;
    Ok((parse_node(node)?, policy))
}

fn parse_binding_target(text: &str) -> Result<BindingTarget, String> {
    if let Some((node, event)) = text.split_once('@') {
        if event.is_empty() {
            return Err(format!("{text:?} has an empty event"));
        }
        return Ok(BindingTarget::Event {
            node: parse_node(node)?,
            event: event.to_owned(),
        });
    }
    match text.split_once('.') {
        Some((node, property)) => {
            if property.is_empty() {
                return Err(format!("{text:?} has an empty property"));
            }
            Ok(BindingTarget::Property {
                node: parse_node(node)?,
                property: property.to_owned(),
            })
        }
        None => Err(format!(
            "{text:?} is not <node>.<property> or <node>@<event>"
        )),
    }
}

fn parse_binding(argument: &str) -> Result<BindingFact, String> {
    let parts: Vec<&str> = argument.split(':').collect();
    let (ends, source_type, target_type, generated) = match parts.as_slice() {
        [ends, source_type, target_type] => (*ends, *source_type, *target_type, false),
        [ends, source_type, target_type, "generated"] => (*ends, *source_type, *target_type, true),
        _ => {
            return Err(format!(
                "bind needs <source>=<target>:<source-type>:<target-type>[:generated], got {argument:?}"
            ));
        }
    };
    let (source, target) = ends
        .split_once('=')
        .ok_or_else(|| format!("{ends:?} is not <source>=<target>"))?;
    if source.trim().is_empty() {
        return Err(format!("{ends:?} has an empty source"));
    }
    let source_type = BindingType::parse(source_type)
        .ok_or_else(|| format!("{source_type:?} is not a binding type"))?;
    let target_type = if target_type == "unknown" {
        None
    } else {
        Some(
            BindingType::parse(target_type)
                .ok_or_else(|| format!("{target_type:?} is not a binding type or unknown"))?,
        )
    };
    Ok(BindingFact {
        source: source.to_owned(),
        target: parse_binding_target(target)?,
        source_type,
        target_type,
        generated,
    })
}

fn parse_contract_control(argument: &str) -> Result<(String, Vec<String>), String> {
    let (profile, names) = argument
        .split_once(':')
        .ok_or_else(|| format!("{argument:?} is not <profile>:<name>[,<name>...]"))?;
    let profile = profile.trim();
    if profile.is_empty() {
        return Err(format!("{argument:?} has no profile"));
    }
    let names: Vec<String> = names
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect();
    if names.is_empty() {
        return Err(format!("{argument:?} names no control"));
    }
    Ok((profile.to_owned(), names))
}

fn parse_colour(text: &str) -> Result<(u8, u8, u8), String> {
    let hex = text.strip_prefix('#').unwrap_or(text);
    if hex.len() != 6 {
        return Err(format!("{text:?} is not a #rrggbb colour"));
    }
    let red =
        u8::from_str_radix(&hex[0..2], 16).map_err(|_| format!("{text:?} is not a colour"))?;
    let green =
        u8::from_str_radix(&hex[2..4], 16).map_err(|_| format!("{text:?} is not a colour"))?;
    let blue =
        u8::from_str_radix(&hex[4..6], 16).map_err(|_| format!("{text:?} is not a colour"))?;
    Ok((red, green, blue))
}

fn parse_visual(text: &str) -> Result<VisualCondition, String> {
    let parts: Vec<&str> = text.split(':').collect();
    let (platform, scale, font, resources) = match parts.as_slice() {
        [platform, scale, font] => (*platform, *scale, *font, "0"),
        [platform, scale, font, resources] => (*platform, *scale, *font, *resources),
        _ => {
            return Err(format!(
                "visual needs <platform>:<scale>:<font>[:<resources>], got {text:?}"
            ));
        }
    };
    Ok(VisualCondition {
        platform: platform.to_owned(),
        scale: scale
            .parse()
            .map_err(|_| format!("{scale:?} is not a display scale"))?,
        font_digest: font.to_owned(),
        resources: resources
            .parse()
            .map_err(|_| format!("{resources:?} is not a resource version"))?,
    })
}

/// A shot named on the command line, under the declared visual conditions.
///
/// The pixels are hexadecimal digits, one per pixel, matching
/// [`Image::from_digits`](crate::app::preview::visual_baseline::Image::from_digits): the quality
/// command has no image decoder, and inventing one would make the check a different product.
fn parse_shot(text: &str, condition: &Option<VisualCondition>) -> Result<Shot, String> {
    let parts: Vec<&str> = text.splitn(4, ':').collect();
    let [name, width, height, digits] = parts.as_slice() else {
        return Err(format!(
            "a visual shot needs <name>:<width>:<height>:<digits>, got {text:?}"
        ));
    };
    let Some(condition) = condition else {
        return Err("declare visual:<platform>:<scale>:<font> first".to_owned());
    };
    let width: u32 = width
        .parse()
        .map_err(|_| format!("{width:?} is not a pixel width"))?;
    let height: u32 = height
        .parse()
        .map_err(|_| format!("{height:?} is not a pixel height"))?;
    let image = Image::from_digits(width, height, digits)?;
    Ok(Shot::new(*name, condition.conditions(), image))
}

#[cfg(test)]
mod tests {
    use super::{
        Stop, parse_binding, parse_bool, parse_colour, parse_contract_control, parse_node_extent,
        parse_node_frame, parse_node_overlap, parse_shot, parse_visual, quality, record_step,
    };
    use crate::app::quality::{BindingTarget, BindingType, Overlap};
    use crate::cli::QualityOptions;
    use std::time::Duration;

    #[test]
    fn a_cancel_request_is_typed() {
        assert_eq!(Stop::parse(""), Ok(Stop::BeforeStart));
        assert_eq!(Stop::parse("2"), Ok(Stop::After(Duration::from_secs(2))));
        assert!(
            matches!(Stop::parse("0.5"), Ok(Stop::After(limit)) if limit.as_millis() == 500),
            "a partial second is a deadline like any other"
        );
        assert!(Stop::parse("soon").is_err());
        assert!(
            Stop::parse("0").is_err(),
            "a zero deadline would stop every run"
        );
        assert!(Stop::parse("-1").is_err());

        assert!(!quality(&QualityOptions {
            ops: vec!["test-cancel:soon".to_owned()],
        }));
        assert!(quality(&QualityOptions {
            ops: vec!["test-cancel".to_owned()],
        }));
    }

    #[test]
    fn a_run_that_is_not_a_complete_pass_is_not_reported_as_one() {
        let mut passed = true;
        let lines = record_step(
            Ok((
                vec!["tests: 0 passed, 1 failed, 0 skipped, 0 not run".to_owned()],
                false,
            )),
            &mut passed,
        )
        .expect("the report is still printed");
        assert_eq!(lines.len(), 1);
        assert!(!passed, "a run with a failure must not exit as a pass");

        let mut passed = true;
        record_step(Ok((Vec::new(), true)), &mut passed).expect("a complete pass");
        assert!(passed);
    }

    #[test]
    fn a_test_operation_needs_a_directory() {
        assert!(!quality(&QualityOptions {
            ops: vec!["test-discover:".to_owned()],
        }));
        assert!(!quality(&QualityOptions {
            ops: vec!["test-run:".to_owned()],
        }));
        assert!(!quality(&QualityOptions {
            ops: vec!["test-filter:demo".to_owned()],
        }));
        assert!(!quality(&QualityOptions {
            ops: vec!["test-rerun:".to_owned()],
        }));
    }

    #[test]
    fn contract_operations_are_wired_to_the_library() {
        let profile = rust_widgets::widget::capability::TargetSpec::current().profile;
        assert!(quality(&QualityOptions {
            ops: vec![
                format!("contract-control:{profile}:button"),
                format!("contract-capacity:{profile}"),
                "check".to_owned(),
            ],
        }));

        let other = if profile == "mini" { "desktop" } else { "mini" };
        assert!(!quality(&QualityOptions {
            ops: vec![format!("contract-capacity:{other}")],
        }));
    }

    #[test]
    fn a_contract_control_names_a_profile_and_controls() {
        let (profile, names) =
            parse_contract_control("desktop:button, label").expect("a contract control op");
        assert_eq!(profile, "desktop");
        assert_eq!(names, vec!["button".to_owned(), "label".to_owned()]);
        assert!(parse_contract_control("desktop").is_err());
        assert!(parse_contract_control("desktop:").is_err());
        assert!(parse_contract_control(":button").is_err());
    }

    #[test]
    fn colours_and_booleans_are_typed() {
        assert_eq!(parse_colour("#0a84ff").expect("a colour"), (10, 132, 255));
        assert!(parse_colour("blue").is_err());
        assert!(parse_bool("true").expect("a flag"));
        assert!(parse_bool("yes").is_err());
    }

    #[test]
    fn a_visual_condition_needs_three_parts() {
        let condition = parse_visual("macos:200:abc").expect("a condition");
        assert_eq!(condition.scale, 200);
        assert_eq!(condition.resources, 0, "the default bundle");
        assert!(parse_visual("macos:200").is_err());
        let versioned = parse_visual("macos:200:abc:3").expect("a versioned condition");
        assert_eq!(versioned.resources, 3);
        assert!(parse_visual("macos:200:abc:many").is_err());
    }

    #[test]
    fn a_visual_shot_needs_declared_conditions_and_exact_pixels() {
        let condition = Some(parse_visual("macos:200:abc:3").expect("a condition"));
        let shot = parse_shot("home:2:2:0123", &condition).expect("a shot");
        assert_eq!(shot.name, "home");
        assert_eq!(shot.image.size(), "2x2");
        assert_eq!(shot.conditions.resources, 3);

        // Without a declared condition there is no environment to record.
        assert!(parse_shot("home:2:2:0123", &None).is_err());
        // The digits must be exactly width x height.
        assert!(parse_shot("home:2:2:012", &condition).is_err());
        assert!(parse_shot("home:2:2:zzzz", &condition).is_err());
        assert!(parse_shot("home:2", &condition).is_err());
    }

    #[test]
    fn layout_ops_are_typed() {
        let (node, extent) = parse_node_extent("3:120,80").expect("an extent");
        assert_eq!((node, extent.width, extent.height), (3, 120.0, 80.0));
        let (node, frame) = parse_node_frame("3:10,20,30,40").expect("a frame");
        assert_eq!(
            (node, frame.x, frame.y, frame.width, frame.height),
            (3, 10.0, 20.0, 30.0, 40.0)
        );
        let (node, policy) = parse_node_overlap("3:forbidden").expect("a policy");
        assert_eq!((node, policy), (3, Overlap::Forbidden));
        assert!(parse_node_extent("3:120").is_err());
        assert!(parse_node_frame("3:1,2,3").is_err());
        assert!(parse_node_overlap("3:maybe").is_err());
    }

    #[test]
    fn a_binding_names_its_target_and_refuses_an_unknown_shape() {
        let binding = parse_binding("f.count=2.label:text:number").expect("a binding");
        assert_eq!(binding.source, "f.count");
        assert_eq!(
            binding.target,
            BindingTarget::Property {
                node: 2,
                property: "label".to_owned(),
            }
        );
        assert_eq!(binding.source_type, BindingType::Text);
        assert_eq!(binding.target_type, Some(BindingType::Number));

        let unknown = parse_binding("f.count=2.tone:text:unknown").expect("an unknown target");
        assert_eq!(unknown.target_type, None);

        let event = parse_binding("f.saved=2@finished:bool:bool").expect("an event target");
        assert_eq!(
            event.target,
            BindingTarget::Event {
                node: 2,
                event: "finished".to_owned(),
            }
        );

        let generated =
            parse_binding("f.total=2.width:number:number:generated").expect("a generated target");
        assert!(generated.generated);

        assert!(parse_binding("f.count").is_err());
        assert!(parse_binding("f.count=nowhere:text:text").is_err());
        assert!(parse_binding("f.count=2.label:text:wat").is_err());
    }
}
