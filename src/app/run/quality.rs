// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The `--quality` command: the problem centre, design checks and evidence (task F21).

use crate::app::quality::{
    Center, DesignFacts, Evidence, Filter, Problem, Severity, Source, TestStatus, Tests,
    VisualCondition, contrast,
};

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
                        Ok(vec![format!("capacity {used}/{limit}")])
                    }
                    _ => Err(format!("{argument:?} is not <used>:<limit>")),
                },
                None => Err(format!("{argument:?} is not <used>:<limit>")),
            },
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
            "state" => Ok(center.render(&filter)),
            "show" => Ok(center.render(&filter)),
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

fn parse_bool(text: &str) -> Result<bool, String> {
    match text {
        "true" => Ok(true),
        "false" => Ok(false),
        other => Err(format!("{other:?} is not true or false")),
    }
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
    use super::{parse_bool, parse_colour, parse_visual};

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
}
