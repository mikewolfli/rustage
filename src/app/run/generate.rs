// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! F15: the generator — the plan, the pipeline, the mapping, the merge, the reverse path, the
//! position recalculator, the export and the golden comparisons.
//!
//! One command handler per parsed command of this domain; the dispatcher lives in
//! [`super`](self). Nothing here is written unless a command asks to write, and every handler reports
//! what it could not do rather than printing a success shape.

use super::*;

/// Reads a file, or reports why it could not be.
fn read(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|error| format!("{path} could not be read: {error}"))
}

/// Splits `name=value`, or reports the shape it did not have.
fn pair<'a>(rest: &'a str, what: &str) -> Result<(&'a str, &'a str), String> {
    rest.split_once('=')
        .ok_or_else(|| format!("{what} needs <name>=<value>, got {rest:?}"))
}

/// Plans a project's generated files (task T15.01).
pub(super) fn artifact(options: &crate::cli::ArtifactOptions) -> bool {
    use crate::app::generate::artifact::plan;

    let mut entries: Vec<(String, String)> = Vec::new();
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        match verb {
            "entry" => match pair(argument, "entry") {
                Ok((source, output)) => entries.push((source.to_owned(), output.to_owned())),
                Err(reason) => return report(reason),
            },
            "output" => match plan(&entries) {
                Ok(planned) => match planned.output_for(argument) {
                    Some(output) => println!("{argument} -> {output}"),
                    None => return report(format!("{argument} has no planned output")),
                },
                Err(reason) => return report(reason),
            },
            "show" => {}
            other => return report(format!("{other:?} is not an artifact operation")),
        }
    }
    match plan(&entries) {
        Ok(planned) => {
            for line in planned.render() {
                println!("{line}");
            }
            true
        }
        Err(reason) => report(reason),
    }
}

/// Runs the generation pipeline (task T15.02).
pub(super) fn generate(options: &crate::cli::GenerateOptions) -> bool {
    use crate::app::generate::pipeline::{self, Gap, Input, Node};
    use rust_widgets::designer::TargetProfile;

    let mut input = Input {
        json: String::new(),
        target: TargetProfile::Default,
        width: 320,
        height: 200,
        function_name: "build_ui".to_owned(),
        nodes: Vec::new(),
        style: Vec::new(),
        layout: Vec::new(),
    };
    let mut once = true;
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<(), String> = match verb {
            "document" => read(argument).map(|text| input.json = text),
            "node" => match argument.split_once(':') {
                Some((kind, children)) => match children.parse::<usize>() {
                    Ok(children) => {
                        input.nodes.push(Node {
                            kind: kind.to_owned(),
                            children,
                        });
                        Ok(())
                    }
                    Err(_) => Err(format!("{children:?} is not a child count")),
                },
                None => Err(format!("node needs <kind>:<children>, got {argument:?}")),
            },
            "target" => match argument {
                "default" => {
                    input.target = TargetProfile::Default;
                    Ok(())
                }
                "stripped" => {
                    input.target = TargetProfile::Stripped;
                    Ok(())
                }
                other => Err(format!("{other:?} is not a target (default or stripped)")),
            },
            "size" => match argument.split_once('x') {
                Some((w, h)) => match (w.parse::<u32>(), h.parse::<u32>()) {
                    (Ok(width), Ok(height)) => {
                        input.width = width;
                        input.height = height;
                        Ok(())
                    }
                    _ => Err(format!("{argument:?} is not <w>x<h>")),
                },
                None => Err(format!("size needs <w>x<h>, got {argument:?}")),
            },
            "fn" => {
                input.function_name = argument.to_owned();
                Ok(())
            }
            "style" | "layout" => match argument.split_once(':') {
                Some((node, reason)) => {
                    let gap = Gap {
                        node: node.to_owned(),
                        reason: reason.to_owned(),
                    };
                    if verb == "style" {
                        input.style.push(gap);
                    } else {
                        input.layout.push(gap);
                    }
                    Ok(())
                }
                None => Err(format!("{verb} needs <node>:<reason>, got {argument:?}")),
            },
            "check" => {
                once = true;
                Ok(())
            }
            "run" => {
                once = false;
                Ok(())
            }
            "twice" => {
                once = false;
                Ok(())
            }
            "show" => Ok(()),
            other => Err(format!("{other:?} is not a generate operation")),
        };
        if let Err(reason) = step {
            return report(reason);
        }
    }

    let outcome = if once {
        pipeline::run(&input)
    } else {
        pipeline::run_twice(&input)
    };
    match outcome {
        Ok(outcome) => {
            for line in outcome.render() {
                println!("{line}");
            }
            if !outcome.is_clean() {
                print!("{}", outcome.source);
            }
            true
        }
        Err(gaps) => report(
            gaps.iter()
                .map(Gap::render)
                .collect::<Vec<_>>()
                .join("\nrustage: "),
        ),
    }
}

/// Builds a node-to-source map (task T15.03).
pub(super) fn mapping(options: &crate::cli::MappingOptions) -> bool {
    use crate::app::generate::mapping as source_mapping;
    use crate::contracts::identity::NodeId;
    use crate::contracts::revision::{GeneratorVersion, Revision};

    let mut source = String::new();
    let mut file = "src/generated/ui.rs".to_owned();
    let mut keys: Vec<(NodeId, String)> = Vec::new();
    let mut model = Revision::INITIAL;
    let mut generator = GeneratorVersion::CURRENT;
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<(), String> = match verb {
            "source" => read(argument).map(|text| source = text),
            "file" => {
                file = argument.to_owned();
                Ok(())
            }
            "key" => match argument.split_once(':') {
                Some((node, key)) => match node.parse::<u64>() {
                    Ok(node) => {
                        keys.push((NodeId::from_raw(node), key.to_owned()));
                        Ok(())
                    }
                    Err(_) => Err(format!("{node:?} is not a node id")),
                },
                None => Err(format!("key needs <node>:<key>, got {argument:?}")),
            },
            "model" => match argument.parse::<u64>() {
                Ok(value) => {
                    model = Revision::from_raw(value);
                    Ok(())
                }
                Err(_) => Err(format!("{argument:?} is not a revision")),
            },
            "generator" => {
                let parts: Vec<&str> = argument.split('.').collect();
                match parts.as_slice() {
                    [major, minor, patch] => match (
                        major.parse::<u32>(),
                        minor.parse::<u32>(),
                        patch.parse::<u32>(),
                    ) {
                        (Ok(major), Ok(minor), Ok(patch)) => {
                            generator = GeneratorVersion::new(major, minor, patch);
                            Ok(())
                        }
                        _ => Err(format!("{argument:?} is not a version")),
                    },
                    _ => Err(format!(
                        "generator needs <major>.<minor>.<patch>, got {argument:?}"
                    )),
                }
            }
            "locate" => Ok(()),
            "at" => Ok(()),
            "staleness" => Ok(()),
            "show" => Ok(()),
            other => Err(format!("{other:?} is not a mapping operation")),
        };
        if let Err(reason) = step {
            return report(reason);
        }
    }

    let map = source_mapping::build(&file, &source, generator, model, &keys);
    println!("mapped {} node(s) in {file}", map.len());
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        match verb {
            "locate" => match argument.parse::<u64>() {
                Ok(node) => match map.span_of(NodeId::from_raw(node)) {
                    Some(span) => println!(
                        "node {node} -> {}:{}..{}",
                        span.file(),
                        span.start(),
                        span.end()
                    ),
                    None => println!("node {node} -> no span"),
                },
                Err(_) => return report(format!("{argument:?} is not a node id")),
            },
            "at" => match argument.parse::<u32>() {
                Ok(offset) => match map.node_at(&file, offset) {
                    Some(node) => println!("{file}:{offset} -> node {}", node.get()),
                    None => println!("{file}:{offset} -> no node"),
                },
                Err(_) => return report(format!("{argument:?} is not an offset")),
            },
            "staleness" => match argument.parse::<u64>() {
                Ok(current) => println!("{}", map.staleness(Revision::from_raw(current))),
                Err(_) => return report(format!("{argument:?} is not a revision")),
            },
            _ => {}
        }
    }
    true
}

/// Merges a regenerated body into the committed file (task T15.04).
pub(super) fn gen_merge(options: &crate::cli::GenMergeOptions) -> bool {
    use crate::app::generate::merge::merge;

    let mut old = String::new();
    let mut new = String::new();
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        match verb {
            "old" => match read(argument) {
                Ok(text) => old = text,
                Err(reason) => return report(reason),
            },
            "new" => match read(argument) {
                Ok(text) => new = text,
                Err(reason) => return report(reason),
            },
            "show" => {}
            other => return report(format!("{other:?} is not a gen-merge operation")),
        }
    }
    match merge(&old, &new) {
        Ok(merged) => {
            for line in merged.review.render() {
                println!("{line}");
            }
            print!("{}", merged.text);
            true
        }
        Err(reason) => report(reason),
    }
}

/// Reads a generated file back into a model candidate (task T15.05).
pub(super) fn reverse(options: &crate::cli::ReverseOptions) -> bool {
    use crate::app::generate::reverse as read_back;

    let mut base = String::new();
    let mut edited = String::new();
    let mut mine = String::new();
    let mut theirs = String::new();
    let mut support = false;
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<(), String> = match verb {
            "base" => read(argument).map(|text| base = text),
            "edited" => read(argument).map(|text| edited = text),
            "mine" => read(argument).map(|text| mine = text),
            "theirs" => read(argument).map(|text| theirs = text),
            "support" => {
                support = true;
                Ok(())
            }
            "show" => Ok(()),
            other => Err(format!("{other:?} is not a reverse operation")),
        };
        if let Err(reason) = step {
            return report(reason);
        }
    }
    if support {
        for entry in read_back::SUPPORT {
            println!(
                "{:<32} {:<10} {}",
                entry.construct,
                if entry.readable { "readable" } else { "kept" },
                entry.meaning
            );
        }
        return true;
    }
    let candidate = read_back::read(&base, &edited);
    for line in candidate.render() {
        println!("{line}");
    }
    if !mine.is_empty() || !theirs.is_empty() {
        for conflict in read_back::conflicts(&base, &mine, &theirs) {
            println!("conflict {}", conflict.render());
        }
    }
    candidate.is_clean()
}

/// Recalculates positions, guards a loop and undoes across views (task T15.06).
pub(super) fn position(options: &crate::cli::PositionOptions) -> bool {
    use crate::app::generate::position::{
        Edit, Journal, LoopGuard, Position, Step, offset_of, position_of,
    };

    let mut text = String::new();
    let mut guard = LoopGuard::new(4);
    let mut journal = Journal::new();
    let mut last_edit: Option<Edit> = None;
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "text" => match read(argument) {
                Ok(value) => {
                    text = value;
                    Ok(Vec::new())
                }
                Err(reason) => Err(reason),
            },
            "at" => match argument.parse::<u32>() {
                Ok(byte) => {
                    let found = position_of(&text, byte);
                    Ok(vec![format!("byte {byte} -> {}", found.render())])
                }
                Err(_) => Err(format!("{argument:?} is not a byte offset")),
            },
            "line" => match argument.split_once(':') {
                Some((line, column)) => match (line.parse::<u32>(), column.parse::<u32>()) {
                    (Ok(line), Ok(column)) => {
                        let position = Position { line, column };
                        match offset_of(&text, position) {
                            Some(byte) => Ok(vec![format!("{} -> byte {byte}", position.render())]),
                            None => Ok(vec![format!("{} -> past the text", position.render())]),
                        }
                    }
                    _ => Err(format!("{argument:?} is not <line>:<column>")),
                },
                None => Err(format!("line needs <line>:<column>, got {argument:?}")),
            },
            "edit" => parse_edit(argument).map(|edit| {
                let line = format!(
                    "edit {}..{} (+{}) shifts by {}",
                    edit.start,
                    edit.end,
                    edit.inserted,
                    edit.delta()
                );
                journal.record(Step {
                    view: "code".to_owned(),
                    label: line.clone(),
                    before: Vec::new(),
                    after: Vec::new(),
                });
                last_edit = Some(edit);
                vec![line]
            }),
            "apply" => match argument.parse::<u32>() {
                Ok(offset) => match &last_edit {
                    Some(edit) => Ok(vec![format!(
                        "byte {offset} -> {}",
                        edit.apply(offset)
                            .map(|moved| format!("byte {moved}"))
                            .unwrap_or_else(|| "deleted".to_owned()),
                    )]),
                    None => Ok(vec![format!("byte {offset} (no edit to apply)")]),
                },
                Err(_) => Err(format!("{argument:?} is not a byte offset")),
            },
            "guard" => match argument.parse::<u32>() {
                Ok(limit) => {
                    guard = LoopGuard::new(limit);
                    Ok(vec![format!("guard limit {limit}")])
                }
                Err(_) => Err(format!("{argument:?} is not a loop limit")),
            },
            "enter" => Ok(vec![match guard.enter() {
                Ok(depth) => format!("entered pass {depth}"),
                Err(reason) => format!("refused: {reason}"),
            }]),
            "leave" => {
                guard.leave();
                Ok(vec![format!("depth {}", guard.depth())])
            }
            "undo" => Ok(vec![match journal.undo() {
                Some(step) => format!("undid {} ({})", step.label, step.view),
                None => "nothing to undo".to_owned(),
            }]),
            "redo" => Ok(vec![match journal.redo() {
                Some(step) => format!("redid {} ({})", step.label, step.view),
                None => "nothing to redo".to_owned(),
            }]),
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a position operation")),
        };
        match step {
            Ok(notes) => {
                for note in notes {
                    println!("{note}");
                }
            }
            Err(reason) => return report(reason),
        }
    }
    true
}

/// Parses `<start>:<end>:<inserted>` into an edit.
fn parse_edit(argument: &str) -> Result<crate::app::generate::position::Edit, String> {
    use crate::app::generate::position::Edit;

    match argument.split(':').collect::<Vec<_>>().as_slice() {
        [start, end, inserted] => {
            match (
                start.parse::<u32>(),
                end.parse::<u32>(),
                inserted.parse::<u32>(),
            ) {
                (Ok(start), Ok(end), Ok(inserted)) => Ok(Edit {
                    start,
                    end,
                    inserted,
                }),
                _ => Err(format!("{argument:?} is not <start>:<end>:<inserted>")),
            }
        }
        _ => Err(format!(
            "edit needs <start>:<end>:<inserted>, got {argument:?}"
        )),
    }
}

/// Commits generated files and states the standalone contract (task T15.07).
pub(super) fn export(options: &crate::cli::ExportOptions) -> bool {
    use crate::app::generate::export::{Artifact, commit, standalone_contract};

    let mut root = std::path::PathBuf::from(".");
    let mut artifacts: Vec<Artifact> = Vec::new();
    let mut do_commit = false;
    let mut contract = false;
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<(), String> = match verb {
            "root" => {
                root = std::path::PathBuf::from(argument);
                Ok(())
            }
            "artifact" => match pair(argument, "artifact") {
                Ok((path, file)) => match read(file) {
                    Ok(source) => {
                        artifacts.push(Artifact {
                            path: path.to_owned(),
                            source,
                        });
                        Ok(())
                    }
                    Err(reason) => Err(reason),
                },
                Err(reason) => Err(reason),
            },
            "commit" => {
                do_commit = true;
                Ok(())
            }
            "contract" => {
                contract = true;
                Ok(())
            }
            "show" => Ok(()),
            other => Err(format!("{other:?} is not an export operation")),
        };
        if let Err(reason) = step {
            return report(reason);
        }
    }
    if contract {
        for line in standalone_contract() {
            println!("{line}");
        }
    }
    if !do_commit {
        for artifact in &artifacts {
            println!("would write {}", artifact.path);
        }
        return true;
    }
    match commit(&root, &artifacts) {
        Ok(results) => {
            for result in results {
                println!("{}", result.render());
            }
            true
        }
        Err(reason) => report(reason),
    }
}

/// Compares generation against recorded goldens (task T15.08).
pub(super) fn golden(options: &crate::cli::GoldenOptions) -> bool {
    use crate::app::generate::golden::{self, Golden};

    let mut goldens: Vec<Golden> = Vec::new();
    let mut produced: Vec<(String, String)> = Vec::new();
    let mut semantics = false;
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<(), String> = match verb {
            "golden" => match pair(argument, "golden") {
                Ok((name, file)) => match read(file) {
                    Ok(expected) => {
                        goldens.push(Golden {
                            name: name.to_owned(),
                            expected,
                        });
                        Ok(())
                    }
                    Err(reason) => Err(reason),
                },
                Err(reason) => Err(reason),
            },
            "produced" => match pair(argument, "produced") {
                Ok((name, file)) => match read(file) {
                    Ok(content) => {
                        produced.push((name.to_owned(), content));
                        Ok(())
                    }
                    Err(reason) => Err(reason),
                },
                Err(reason) => Err(reason),
            },
            "semantics" => {
                semantics = true;
                match read(argument) {
                    Ok(source) => {
                        for line in golden::semantics(&source) {
                            println!("{line}");
                        }
                        Ok(())
                    }
                    Err(reason) => Err(reason),
                }
            }
            "check" => Ok(()),
            "show" => Ok(()),
            other => Err(format!("{other:?} is not a golden operation")),
        };
        if let Err(reason) = step {
            return report(reason);
        }
    }
    if semantics && goldens.is_empty() {
        return true;
    }
    let results = golden::check(&goldens, &produced);
    let mut ok = true;
    for comparison in &results {
        println!("{}", comparison.render());
        ok &= comparison.is_match();
    }
    ok
}
