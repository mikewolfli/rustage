// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The `--quality` command: the problem centre, design checks and evidence (task F21).

use rust_widgets::widget::WidgetFactory;

use crate::app::quality::{
    BindingFact, BindingTarget, BindingType, Center, DesignFacts, Evidence, Extent, Filter, Frame,
    LayoutFact, Overlap, Problem, Provenance, RunReport, Selection, Severity, Source, TestConfig,
    TestRunner, TestStatus, Tests, VisualCondition, contrast,
};
use crate::host::tasks::CancelToken;

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
    let mut visual: Option<VisualCondition> = None;
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
                    match facts.capability_from(WidgetFactory::shared(), &profile, &borrowed) {
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
            "tests" => Ok(tests
                .tally()
                .into_iter()
                .map(|(status, count)| format!("{status}: {count}"))
                .collect()),
            "test-discover" => {
                if argument.trim().is_empty() {
                    Err("test-discover needs a project directory".to_owned())
                } else {
                    let runner = TestRunner::new(TestConfig::in_directory(argument));
                    runner.discover_into(&mut tests).map(|discovery| {
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
                        vec![line]
                    })
                }
            }
            "test-run" => {
                if argument.trim().is_empty() {
                    Err("test-run needs a project directory".to_owned())
                } else {
                    let runner = TestRunner::new(TestConfig::in_directory(argument));
                    run_tests(
                        &runner,
                        &Selection::all(),
                        revision,
                        &target,
                        &mut tests,
                        &mut center,
                    )
                }
            }
            "test-filter" => match argument.split_once(':') {
                Some((directory, filter))
                    if !directory.trim().is_empty() && !filter.trim().is_empty() =>
                {
                    let runner = TestRunner::new(TestConfig::in_directory(directory));
                    run_tests(
                        &runner,
                        &Selection::filter(filter),
                        revision,
                        &target,
                        &mut tests,
                        &mut center,
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
                    run_tests(
                        &runner,
                        &Selection::exact(names),
                        revision,
                        &target,
                        &mut tests,
                        &mut center,
                    )
                }
                _ => Err("test-select needs <directory>:<name>[,<name>…]".to_owned()),
            },
            "test-rerun" => {
                if argument.trim().is_empty() {
                    Err("test-rerun needs a project directory".to_owned())
                } else {
                    let runner = TestRunner::new(TestConfig::in_directory(argument));
                    match runner.rerun_failed(revision, &target, &mut tests, &mut center) {
                        Ok(report) => Ok(report_lines(&report)),
                        Err(reason) => Err(reason),
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
            "visual" => match parse_visual(argument) {
                Ok(condition) => {
                    visual = Some(condition.clone());
                    Ok(vec![format!(
                        "visual {} at {}% font {}",
                        condition.platform, condition.scale, condition.font_digest
                    )])
                }
                Err(reason) => Err(reason),
            },
            "visual-compare" => match parse_visual(argument) {
                Ok(other) => match &visual {
                    Some(baseline) => match baseline.compare(&other) {
                        Ok(()) => Ok(vec!["visual conditions match".to_owned()]),
                        Err(reason) => Ok(vec![format!("visual conditions differ: {reason}")]),
                    },
                    None => Err("no visual baseline was set".to_owned()),
                },
                Err(reason) => Err(reason),
            },
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
                        visual: visual.clone(),
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
    true
}

/// Runs a test selection and turns the report into printable lines.
fn run_tests(
    runner: &TestRunner,
    selection: &Selection,
    revision: u64,
    target: &str,
    tests: &mut Tests,
    center: &mut Center,
) -> Result<Vec<String>, String> {
    // The command-line has no cancellation gesture of its own yet; the typed runner takes a token
    // so the same path is used by the windowed caller when it gains one.
    let report = runner.run_cancellable(
        selection,
        revision,
        target,
        tests,
        center,
        &CancelToken::default(),
    )?;
    Ok(report_lines(&report))
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
    match parts.as_slice() {
        [platform, scale, font] => Ok(VisualCondition {
            platform: (*platform).to_owned(),
            scale: scale
                .parse()
                .map_err(|_| format!("{scale:?} is not a display scale"))?,
            font_digest: (*font).to_owned(),
        }),
        _ => Err(format!(
            "visual needs <platform>:<scale>:<font>, got {text:?}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        parse_binding, parse_bool, parse_colour, parse_contract_control, parse_node_extent,
        parse_node_frame, parse_node_overlap, parse_visual, quality,
    };
    use crate::app::quality::{BindingTarget, BindingType, Overlap};
    use crate::cli::QualityOptions;

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
        assert!(parse_visual("macos:200").is_err());
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
