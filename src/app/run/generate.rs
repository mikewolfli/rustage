// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! F15: the generator — the plan, the pipeline, the mapping, the merge, the reverse path, the
//! position recalculator, the export and the golden comparisons.
//!
//! One command handler per parsed command of this domain; the dispatcher lives in
//! [`super`](self). Nothing here is written unless a command asks to write, and every handler reports
//! what it could not do rather than printing a success shape.

use super::*;

/// Reads a generation document, translating a rustage page document first.
///
/// `--generate document:` takes the library's widget document (one single-key object per node).
/// A project page is a different file — a flat node list with `type`, `properties` and `children` —
/// and [`crate::app::generate::document::to_library_document`] is the one translation between them.
/// Without this branch the design model could never reach the generator, which is the J01/J06 seam
/// (principle #26: a translation with no consumer is not an integration).
fn read_document(path: &str) -> Result<String, String> {
    use crate::project::record::json;

    let text = read(path)?;
    let value = match json::parse(&text) {
        Ok(value) => value,
        // The generator reports a malformed document with its own diagnosis; leave it to do so.
        Err(_) => return Ok(text),
    };
    let is_page = value
        .as_object()
        .map(|object| object.get("page").is_some() && object.get("nodes").is_some())
        .unwrap_or(false);
    if !is_page {
        return Ok(text);
    }

    use crate::project::document::buffer::root_of;
    use crate::project::record::page::PageDocument;

    // The project root is a file-system fact here: a page in a project resolves its assets against
    // that root, and a page outside one is refused rather than resolved against an invented
    // directory.
    let root = root_of(std::path::Path::new(path)).map_err(|refusal| refusal.message())?;
    let page = PageDocument::parse(&root, path, &text).map_err(|error| error.message())?;
    let root_id = page.page().root();
    crate::app::generate::document::to_library_document(root_id, page.nodes())
}

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
    // Read before the loop: a malformed op before `dual` still belongs to a dual invocation, and
    // its exit code has to be the roadmap's 2 rather than the dispatcher's generic failure.
    let dual = options.ops.iter().any(|op| op == "dual");
    let mut once = true;
    let mut check = false;
    let mut root = std::path::PathBuf::from(".");
    let mut write: Option<std::path::PathBuf> = None;
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<(), String> = match verb {
            "document" | "project" => read_document(argument).map(|text| input.json = text),
            "dual" => Ok(()),
            "root" => {
                if argument.is_empty() {
                    Err("root needs <directory>".to_owned())
                } else {
                    root = std::path::PathBuf::from(argument);
                    Ok(())
                }
            }
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
                // In a single-target run this stays the existing "generate once" spelling. A dual
                // run reads it as the roadmap's compare-only mode, decided after all ops are read.
                check = true;
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
            "write" => {
                // Generation is in memory until a caller asks for a file. T15.07's export step
                // reads generated files, and an independent project needs one to build, so the
                // pipeline gets one explicit write point rather than making export guess a path.
                if argument.is_empty() {
                    Err("write needs <path>".to_owned())
                } else {
                    write = Some(std::path::PathBuf::from(argument));
                    Ok(())
                }
            }
            "show" => Ok(()),
            other => Err(format!("{other:?} is not a generate operation")),
        };
        if let Err(reason) = step {
            if dual {
                let _ = report(reason);
                stop(crate::host::error::ExitCode::Usage.code());
            }
            return report(reason);
        }
    }

    if dual {
        return dual_generate(&input, &root, check);
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
            if let Some(path) = write {
                if let Some(parent) = path.parent()
                    && !parent.as_os_str().is_empty()
                    && let Err(error) = std::fs::create_dir_all(parent)
                {
                    return report(format!(
                        "{} could not be created: {error}",
                        parent.display()
                    ));
                }
                match write_generated(&path, &outcome.source) {
                    Ok(WriteOutcome::Unchanged) => println!("unchanged {}", path.display()),
                    Ok(WriteOutcome::Wrote(bytes)) => {
                        println!("wrote {} ({bytes} bytes)", path.display());
                    }
                    Err(reason) => return report(reason),
                }
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

/// What a dual run should exit with (task T15.02).
///
/// The numbers are the roadmap's contract: `0` unchanged (or written), `1` a committed product is
/// stale, `2` the run was refused because a gap blocks. They are expressed through
/// [`crate::host::error::ExitCode`] rather than as bare literals, like every other exit in this
/// binary.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum DualStatus {
    /// Every product matches, or was written.
    Clean,
    /// `--check` found a committed product that does not match.
    Stale,
    /// A gap blocks generation, so nothing was compared or written.
    Refused,
}

impl DualStatus {
    /// The process exit number for this status.
    pub(super) fn code(self) -> i32 {
        use crate::host::error::ExitCode;
        match self {
            Self::Clean => ExitCode::Ok.code(),
            Self::Stale => ExitCode::Failed.code(),
            Self::Refused => ExitCode::Usage.code(),
        }
    }
}

/// Generates both products, then writes them or compares them with what is committed.
///
/// The dispatcher maps a handler's `false` to [`ExitCode::Reported`](crate::host::error::ExitCode),
/// which cannot express the roadmap's 1/2 split. A non-zero dual status therefore ends the process
/// here, after the report has been printed; the success path still returns `true` like every other
/// handler.
fn dual_generate(
    input: &crate::app::generate::pipeline::Input,
    root: &std::path::Path,
    check: bool,
) -> bool {
    use crate::app::generate::pipeline::{self, DualInput};
    use crate::host::error::ExitCode;

    let outcome = match pipeline::run_dual(&DualInput::from_single(input)) {
        Ok(outcome) => outcome,
        Err(gaps) => {
            for gap in &gaps {
                println!("refused {}", gap.render());
            }
            stop(ExitCode::Usage.code());
        }
    };
    for line in outcome.render() {
        println!("{line}");
    }
    match dual_status(&outcome, root, check) {
        Ok(DualStatus::Clean) => true,
        Ok(status) => stop(status.code()),
        Err(reason) => {
            let _ = report(reason);
            stop(ExitCode::Usage.code());
        }
    }
}

/// The decision a dual run makes, without ending the process so a test can assert it.
pub(super) fn dual_status(
    outcome: &crate::app::generate::pipeline::DualOutcome,
    root: &std::path::Path,
    check: bool,
) -> Result<DualStatus, String> {
    if outcome.is_blocked() {
        return Ok(DualStatus::Refused);
    }
    if check {
        if compare_products(outcome, root)? {
            Ok(DualStatus::Clean)
        } else {
            Ok(DualStatus::Stale)
        }
    } else {
        write_products(outcome, root)?;
        Ok(DualStatus::Clean)
    }
}

/// Compares each product with the file committed under `root`, writing nothing.
///
/// A missing file is stale rather than an error: `--check` is run on a checkout that may not have
/// generated yet, and "missing" is a finding a CI job must be able to report without the tool
/// failing to run. A file without the marker is not one this tool wrote, so it is reported instead
/// of compared — overwriting it later would destroy hand-written code (T15.04). A marked file is
/// compared against the *merged* result, so a preserved hand-written region is not mistaken for a
/// stale generated half.
fn compare_products(
    outcome: &crate::app::generate::pipeline::DualOutcome,
    root: &std::path::Path,
) -> Result<bool, String> {
    use crate::app::generate::artifact;
    use crate::app::generate::merge::{self, Prepared};

    let mut up_to_date = true;
    for product in &outcome.products {
        let path = root.join(&product.path);
        match std::fs::read_to_string(&path) {
            Ok(committed) if !artifact::is_generated(&committed) => {
                println!(
                    "not-generated {} (first line is not the generated marker)",
                    path.display()
                );
                up_to_date = false;
            }
            Ok(committed) => match merge::prepare(&committed, &product.source) {
                Ok(Prepared::Unchanged) => println!("unchanged {}", path.display()),
                Ok(Prepared::Write(_)) => {
                    println!("stale {}", path.display());
                    up_to_date = false;
                }
                Err(reason) => {
                    return Err(format!("{}: {reason}", path.display()));
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                println!("missing {}", path.display());
                up_to_date = false;
            }
            Err(error) => {
                return Err(format!("{} could not be read: {error}", path.display()));
            }
        }
    }
    Ok(up_to_date)
}

/// What a guarded generated write did.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum WriteOutcome {
    /// The merged result already equals the committed file.
    Unchanged,
    /// Bytes were written.
    Wrote(usize),
}

/// Writes generated text only when the committed file may be replaced.
///
/// A missing file is created. An existing file must start with the generated marker and its
/// hand-written boundaries must be provable; [`merge::prepare`] answers both questions before a
/// single byte is written. A file that is not ours, or whose boundaries cannot be proven, is
/// refused with the path in the reason rather than overwritten.
fn write_generated(path: &std::path::Path, regenerated: &str) -> Result<WriteOutcome, String> {
    use crate::app::generate::merge::{self, Prepared};

    match std::fs::read_to_string(path) {
        Ok(existing) => match merge::prepare(&existing, regenerated)
            .map_err(|reason| format!("{}: {reason}", path.display()))?
        {
            Prepared::Unchanged => Ok(WriteOutcome::Unchanged),
            Prepared::Write(text) => {
                std::fs::write(path, &text)
                    .map_err(|error| format!("{} could not be written: {error}", path.display()))?;
                Ok(WriteOutcome::Wrote(text.len()))
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::write(path, regenerated)
                .map_err(|error| format!("{} could not be written: {error}", path.display()))?;
            Ok(WriteOutcome::Wrote(regenerated.len()))
        }
        Err(error) => Err(format!("{} could not be read: {error}", path.display())),
    }
}

/// Writes both products under `root`, through the generated-file guard and merge path.
fn write_products(
    outcome: &crate::app::generate::pipeline::DualOutcome,
    root: &std::path::Path,
) -> Result<(), String> {
    use crate::app::generate::mapping;
    use crate::contracts::revision::{GeneratorVersion, Revision};

    for product in &outcome.products {
        let path = root.join(&product.path);
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("{} could not be created: {error}", parent.display()))?;
        }
        // The generated document is the authority for which nodes exist, so the map is built from
        // it rather than from a caller's `key:` list. Counting here proves the dual path's output
        // still yields per-node spans; a changed output shape surfaces as `mapped 0 node(s)`.
        let map = mapping::mapping_from_source(
            &product.path,
            &product.source,
            GeneratorVersion::CURRENT,
            Revision::INITIAL,
            &[],
        );
        println!("mapped {} node(s) in {}", map.len(), product.path);
        match write_generated(&path, &product.source)? {
            WriteOutcome::Unchanged => println!("unchanged {}", path.display()),
            WriteOutcome::Wrote(bytes) => println!("wrote {} ({bytes} bytes)", path.display()),
        }
    }
    Ok(())
}

/// Ends the process with `code`, flushing the log first like the dispatcher does.
fn stop(code: i32) -> ! {
    crate::host::log::flush();
    std::process::exit(code);
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

    let map = source_mapping::mapping_from_source(&file, &source, generator, model, &keys);
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
    let mut root_given = false;
    let mut project: Option<std::path::PathBuf> = None;
    let mut artifacts: Vec<Artifact> = Vec::new();
    let mut do_commit = false;
    let mut contract = false;
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<(), String> = match verb {
            "root" => {
                root = std::path::PathBuf::from(argument);
                root_given = true;
                Ok(())
            }
            "project" => {
                // A project export takes the whole project input and generates its files; it is not
                // a copy of files that already exist on disk (T15.07).
                if argument.is_empty() {
                    Err("project needs <file>".to_owned())
                } else {
                    project = Some(std::path::PathBuf::from(argument));
                    Ok(())
                }
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
    if let Some(project) = project {
        // A project's files are relative to the project's own root, so an explicit `root:` is the
        // only reason to write them anywhere else.
        let project = if project.is_absolute() {
            project
        } else {
            root.join(project)
        };
        if !root_given && let Some(parent) = project.parent() {
            root = parent.to_path_buf();
        }
        match project_artifacts(&root, &project) {
            Ok(generated) => artifacts.extend(generated),
            Err(reason) => return report(reason),
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

/// Generates every file a project plans, ready for [`export::commit`] (task T15.07).
///
/// The project document is the input: its `generated` entries are the plan, grouped so the library's
/// dual product pair is one generation ([`crate::app::generate::artifact::plan_project`]). Each
/// design source is read through the same project-page translation the single-page pipeline uses, so
/// the designer's model reaches the generator exactly as it does for `--generate`. A source that
/// names the stripped product gets the stripped template; every other output gets the default
/// template, which is the one the project model can name without guessing a profile.
pub(super) fn project_artifacts(
    root: &std::path::Path,
    project: &std::path::Path,
) -> Result<Vec<crate::app::generate::export::Artifact>, String> {
    use crate::app::generate::artifact;
    use crate::app::generate::export::Artifact;
    use crate::app::generate::pipeline::{self, DualInput};
    use crate::project::record::format::ProjectFile;
    use rust_widgets::designer::{ArtifactPaths, TargetProfile};

    let file = ProjectFile::open(project).map_err(|error| {
        format!(
            "{} could not be read: {}",
            project.display(),
            error.message()
        )
    })?;
    let Some(document) = file.content().document() else {
        return Err(format!(
            "{} is a format this build does not interpret; it cannot be exported",
            project.display()
        ));
    };
    let entries: Vec<(String, String)> = document
        .generated()
        .iter()
        .map(|target| {
            (
                target.from().written().to_owned(),
                target.output().written().to_owned(),
            )
        })
        .collect();
    let plan = artifact::plan_project(&entries)?;
    if plan.is_empty() {
        return Err(format!(
            "{} names no generated files to export",
            project.display()
        ));
    }

    let stripped = ArtifactPaths::path_for(TargetProfile::Stripped);
    let mut generated = Vec::with_capacity(plan.groups.len());
    for group in &plan.groups {
        let page = root.join(&group.source);
        let path = page
            .to_str()
            .ok_or_else(|| format!("{} is not a UTF-8 path this build can read", page.display()))?;
        let json = read_document(path)?;
        // A source names at most one output, or the dual pair. Only the pair needs both products;
        // running both for a single output would refuse a project because of a gap in a file the
        // project never asked for.
        if group.outputs.len() == 2 {
            let dual = pipeline::run_dual(&DualInput {
                json,
                width: 320,
                height: 200,
                function_name: "build_ui".to_owned(),
                nodes: Vec::new(),
                style: Vec::new(),
                layout: Vec::new(),
            })
            .map_err(gaps)?;
            if let Some(reason) = blocking_reason(dual.products.iter().flat_map(|p| p.findings())) {
                return Err(reason);
            }
            for output in &group.outputs {
                let target = project_target(output, &stripped);
                let product = dual.product(target).ok_or_else(|| {
                    format!(
                        "the generator produced no {} product for {output}",
                        target.feature_hint()
                    )
                })?;
                generated.push(Artifact {
                    path: output.clone(),
                    source: product.source.clone(),
                });
            }
        } else {
            let output = &group.outputs[0];
            let target = project_target(output, &stripped);
            let outcome = pipeline::run(&pipeline::Input {
                json,
                target,
                width: 320,
                height: 200,
                function_name: "build_ui".to_owned(),
                nodes: Vec::new(),
                style: Vec::new(),
                layout: Vec::new(),
            })
            .map_err(gaps)?;
            if let Some(reason) = blocking_reason(outcome.findings()) {
                return Err(reason);
            }
            generated.push(Artifact {
                path: output.clone(),
                source: outcome.source,
            });
        }
    }
    Ok(generated)
}

/// The profile an output path names.
///
/// The library's stripped path selects the stripped template; every other path gets the default
/// template, which is the one the project model can name without inventing a profile.
fn project_target(output: &str, stripped: &str) -> rust_widgets::designer::TargetProfile {
    if output == stripped {
        rust_widgets::designer::TargetProfile::Stripped
    } else {
        rust_widgets::designer::TargetProfile::Default
    }
}

/// The blocking findings as one report, or `None` when nothing blocks.
fn blocking_reason(
    findings: impl IntoIterator<Item = crate::app::generate::pipeline::Finding>,
) -> Option<String> {
    let blocked: Vec<String> = findings
        .into_iter()
        .filter(|finding| finding.blocks())
        .map(|finding| finding.render())
        .collect();
    (!blocked.is_empty()).then(|| blocked.join("\nrustage: "))
}

/// One refusal sentence for a pipeline that could not run.
fn gaps(gaps: Vec<crate::app::generate::pipeline::Gap>) -> String {
    gaps.iter()
        .map(|gap| gap.render())
        .collect::<Vec<_>>()
        .join("\nrustage: ")
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

#[cfg(test)]
mod tests {
    use super::{
        DualStatus, WriteOutcome, dual_status, project_artifacts, read_document, write_generated,
    };
    use crate::app::generate::artifact::FILE_MARKER;
    use crate::app::generate::export;
    use crate::app::generate::pipeline::{self, DualInput};

    const DOCUMENT: &str = r#"{"window":{"id":"root","title":"Probe","children":[
        {"label":{"id":"title","text":"Hello"}},
        {"button":{"id":"go","text":"Go"}}
    ]}}"#;

    /// A throwaway root, unique per test within this process.
    fn directory(name: &str) -> std::path::PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "rustage-generate-dual-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("a temporary root");
        directory
    }

    fn dual(text: &str) -> pipeline::DualOutcome {
        pipeline::run_dual(&DualInput {
            json: text.to_owned(),
            width: 320,
            height: 200,
            function_name: "build_ui".to_owned(),
            nodes: Vec::new(),
            style: Vec::new(),
            layout: Vec::new(),
        })
        .expect("both products")
    }

    #[test]
    fn a_dual_write_writes_both_files_and_a_check_then_reports_clean() {
        let root = directory("clean");
        let outcome = dual(DOCUMENT);

        assert_eq!(
            dual_status(&outcome, &root, false),
            Ok(DualStatus::Clean),
            "writing is the success path"
        );
        assert!(root.join("src/generated/ui_default.rs").is_file());
        assert!(root.join("src/generated/ui_stripped.rs").is_file());
        assert_eq!(DualStatus::Clean.code(), 0);

        assert_eq!(
            dual_status(&outcome, &root, true),
            Ok(DualStatus::Clean),
            "the files just written must compare clean"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_dual_check_reports_stale_for_an_edited_or_missing_product() {
        let root = directory("stale");
        let outcome = dual(DOCUMENT);
        assert_eq!(dual_status(&outcome, &root, false), Ok(DualStatus::Clean));

        let stripped = root.join("src/generated/ui_stripped.rs");
        let mut edited = std::fs::read_to_string(&stripped).expect("the written product");
        edited.push_str("\n// a hand edit\n");
        std::fs::write(&stripped, edited).expect("the edit");
        assert_eq!(
            dual_status(&outcome, &root, true),
            Ok(DualStatus::Stale),
            "an edited product is stale, not unchanged"
        );
        assert_eq!(DualStatus::Stale.code(), 1);

        std::fs::remove_file(&stripped).expect("the removal");
        assert_eq!(
            dual_status(&outcome, &root, true),
            Ok(DualStatus::Stale),
            "a missing product is stale, not an error"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_dual_check_is_refused_when_a_gap_blocks() {
        let root = directory("gap");
        let outcome = dual(r#"{"definitely_not_a_widget":{"id":"x"}}"#);
        assert!(outcome.is_blocked(), "{:?}", outcome.render());
        assert!(
            outcome.render().iter().any(|line| {
                line.contains("unsupported") && line.contains("definitely_not_a_widget")
            }),
            "the cause and node have to be named: {:?}",
            outcome.render()
        );

        assert_eq!(
            dual_status(&outcome, &root, true),
            Ok(DualStatus::Refused),
            "a blocking gap takes precedence over comparing files"
        );
        assert_eq!(
            dual_status(&outcome, &root, false),
            Ok(DualStatus::Refused),
            "a blocking gap must not write either"
        );
        assert_eq!(DualStatus::Refused.code(), 2);
        assert!(
            !root.join("src/generated").exists(),
            "a refused run must not create or write products"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_project_page_is_translated_into_the_generator_document() {
        let directory = std::env::temp_dir().join(format!(
            "rustage-generate-page-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(directory.join("pages")).expect("a temporary project");
        std::fs::write(directory.join("project.json"), "{}").expect("the project marker");
        let page = directory.join("pages/main.json");
        std::fs::write(
            &page,
            r#"{"format":1,"page":{"id":1,"name":"Main","root":1},"nodes":[
                {"id":1,"type":"window","children":[2],"properties":[]},
                {"id":2,"type":"label","children":[],"properties":[{"name":"text","value":{"text":"Hi"}}]}
            ]}"#,
        )
        .expect("the page document");

        let translated =
            read_document(page.to_str().expect("a UTF-8 path")).expect("the page translates");
        assert!(
            translated.contains("\"window\"") && translated.contains("\"label\""),
            "{translated}"
        );
        assert!(translated.contains("\"Hi\""), "{translated}");

        let _ = std::fs::remove_dir_all(&directory);
    }

    const MARKED: &str = "// Generated by the rust_widgets designer. Do not edit by hand.\n\
        pub fn build_ui() { let a = 1; }\n";
    const REGENERATED: &str = "// Generated by the rust_widgets designer. Do not edit by hand.\n\
        pub fn build_ui() { let a = 2; }\n";

    #[test]
    fn a_single_write_refuses_a_file_without_the_generated_marker() {
        let root = directory("single-unmarked");
        let path = root.join("ui.rs");
        std::fs::write(&path, "fn main() { println!(\"mine\"); }\n").expect("a hand-written file");

        let error = write_generated(&path, REGENERATED).expect_err("not a generated file");
        assert!(error.contains("generated marker"), "{error}");
        assert_eq!(
            std::fs::read_to_string(&path).expect("still there"),
            "fn main() { println!(\"mine\"); }\n",
            "a refused write must not touch the file"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_single_write_keeps_a_handwritten_region_and_updates_the_generated_half() {
        let root = directory("single-marked");
        let path = root.join("ui.rs");
        let committed = format!(
            "{MARKED}// region:handwritten save\nfn save() {{ /* mine */ }}\n// endregion:handwritten save\n"
        );
        std::fs::write(&path, committed).expect("a generated file");

        match write_generated(&path, REGENERATED).expect("a guarded write") {
            WriteOutcome::Unchanged => panic!("the generated half changed"),
            WriteOutcome::Wrote(_) => {}
        }
        let after = std::fs::read_to_string(&path).expect("the merged file");
        assert!(after.contains("let a = 2;"), "{after}");
        assert!(after.contains("fn save() { /* mine */ }"), "{after}");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_changed_handwritten_boundary_blocks_the_write() {
        let root = directory("single-boundary");
        let path = root.join("ui.rs");
        let committed = format!("{MARKED}// region:handwritten save\nfn save() {{ /* mine */ }}\n");
        std::fs::write(&path, &committed).expect("a generated file");

        let error = write_generated(&path, REGENERATED).expect_err("an unprovable boundary");
        assert!(error.contains("never closed"), "{error}");
        assert_eq!(
            std::fs::read_to_string(&path).expect("still there"),
            committed,
            "a refused write must not touch the file"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_dual_write_refuses_an_unmarked_committed_file() {
        let root = directory("dual-unmarked");
        let path = root.join("src/generated/ui_default.rs");
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("the directory");
        std::fs::write(&path, "fn main() {}\n").expect("a hand-written file");

        let outcome = dual(DOCUMENT);
        let error = dual_status(&outcome, &root, false).expect_err("not a generated file");
        assert!(error.contains("generated marker"), "{error}");
        assert_eq!(
            std::fs::read_to_string(&path).expect("still there"),
            "fn main() {}\n"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_dual_write_keeps_a_handwritten_region_and_updates_the_generated_half() {
        let root = directory("dual-preserve");
        let first = dual(DOCUMENT);
        assert_eq!(dual_status(&first, &root, false), Ok(DualStatus::Clean));

        let path = root.join("src/generated/ui_default.rs");
        let mut committed = std::fs::read_to_string(&path).expect("the product");
        committed.push_str(
            "// region:handwritten save\nfn save() { /* mine */ }\n// endregion:handwritten save\n",
        );
        std::fs::write(&path, &committed).expect("the hand-written region");

        let changed = DOCUMENT.replace("\"Hello\"", "\"Hello again\"");
        let next = dual(&changed);
        assert_eq!(
            dual_status(&next, &root, false),
            Ok(DualStatus::Clean),
            "a marked file with a provable boundary may be regenerated"
        );

        let after = std::fs::read_to_string(&path).expect("the merged product");
        assert!(
            after.contains("Hello again"),
            "the generated half updated: {after}"
        );
        assert!(
            after.contains("fn save() { /* mine */ }"),
            "the hand-written region survived: {after}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_dual_write_refuses_an_unprovable_handwritten_boundary() {
        let root = directory("dual-boundary");
        let first = dual(DOCUMENT);
        assert_eq!(dual_status(&first, &root, false), Ok(DualStatus::Clean));

        let path = root.join("src/generated/ui_default.rs");
        let mut committed = std::fs::read_to_string(&path).expect("the product");
        committed.push_str("// region:handwritten save\nfn save() { /* mine */ }\n");
        std::fs::write(&path, &committed).expect("the broken boundary");

        let changed = DOCUMENT.replace("\"Hello\"", "\"Hello again\"");
        let next = dual(&changed);
        let error = dual_status(&next, &root, false).expect_err("an unprovable boundary");
        assert!(error.contains("never closed"), "{error}");
        assert_eq!(
            std::fs::read_to_string(&path).expect("still there"),
            committed
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A project root whose one page is generated into `generated`'s entries.
    fn project(name: &str, generated: &str) -> std::path::PathBuf {
        let root = directory(name);
        std::fs::create_dir_all(root.join("pages")).expect("the pages directory");
        std::fs::write(
            root.join("project.json"),
            format!(
                "{{\"format\": 2, \"project\": {{\"id\": \"3f2b\", \"name\": \"Demo\"}}, \
                 \"pages\": [{{\"document\": \"pages/main.json\"}}], \
                 \"targets\": [{{\"name\": \"desktop\"}}], \
                 \"generated\": [{generated}]}}"
            ),
        )
        .expect("the project document");
        std::fs::write(
            root.join("pages/main.json"),
            r#"{"format":1,"page":{"id":1,"name":"Main","root":1},"nodes":[
                {"id":1,"type":"window","children":[2],"properties":[]},
                {"id":2,"type":"label","children":[],"properties":[{"name":"text","value":{"text":"Hi"}}]}
            ]}"#,
        )
        .expect("the page document");
        root
    }

    const ONE_OUTPUT: &str = r#"{"output": "src/generated/main.rs", "from": "pages/main.json"}"#;
    const DUAL_OUTPUTS: &str = r#"{"output": "src/generated/ui_default.rs", "from": "pages/main.json"},
        {"output": "src/generated/ui_stripped.rs", "from": "pages/main.json"}"#;

    #[test]
    fn a_whole_project_generates_and_commits_its_planned_files() {
        let root = project("project-whole", ONE_OUTPUT);
        let artifacts =
            project_artifacts(&root, &root.join("project.json")).expect("the project generates");
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].path, "src/generated/main.rs");
        assert!(
            artifacts[0].source.starts_with(FILE_MARKER),
            "the generated file carries the marker: {}",
            &artifacts[0].source[..artifacts[0].source.len().min(80)]
        );
        assert!(artifacts[0].source.contains("Hi"));

        let results = export::commit(&root, &artifacts).expect("a commit");
        assert_eq!(results[0].action, export::Action::Created);
        assert!(root.join("src/generated/main.rs").is_file());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_project_that_names_the_dual_pair_generates_both_products() {
        let root = project("project-dual", DUAL_OUTPUTS);
        let artifacts =
            project_artifacts(&root, &root.join("project.json")).expect("the project generates");
        assert_eq!(
            artifacts
                .iter()
                .map(|artifact| artifact.path.as_str())
                .collect::<Vec<_>>(),
            vec![
                "src/generated/ui_default.rs",
                "src/generated/ui_stripped.rs"
            ]
        );
        export::commit(&root, &artifacts).expect("a commit");

        let default = std::fs::read_to_string(root.join("src/generated/ui_default.rs"))
            .expect("the default product");
        let stripped = std::fs::read_to_string(root.join("src/generated/ui_stripped.rs"))
            .expect("the stripped product");
        assert!(default.starts_with(FILE_MARKER), "{default}");
        assert!(stripped.starts_with(FILE_MARKER), "{stripped}");
        assert_ne!(
            default, stripped,
            "the two templates are different products"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn exporting_the_same_project_twice_is_byte_identical() {
        let root = project("project-twice", DUAL_OUTPUTS);
        let project_document = root.join("project.json");

        let first = project_artifacts(&root, &project_document).expect("the first generation");
        export::commit(&root, &first).expect("the first commit");
        let bytes: Vec<Vec<u8>> = first
            .iter()
            .map(|artifact| std::fs::read(root.join(&artifact.path)).expect("the file"))
            .collect();

        let second = project_artifacts(&root, &project_document).expect("the second generation");
        let results = export::commit(&root, &second).expect("the second commit");
        assert!(
            results
                .iter()
                .all(|result| result.action == export::Action::Unchanged),
            "a second export writes nothing: {:?}",
            results.iter().map(|r| r.render()).collect::<Vec<_>>()
        );
        let again: Vec<Vec<u8>> = second
            .iter()
            .map(|artifact| std::fs::read(root.join(&artifact.path)).expect("the file"))
            .collect();
        assert_eq!(bytes, again, "the same input is byte-identical");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_project_export_refuses_an_unmarked_existing_file() {
        let root = project("project-unmarked", ONE_OUTPUT);
        let path = root.join("src/generated/main.rs");
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("the directory");
        std::fs::write(&path, "fn main() { println!(\"mine\"); }\n").expect("a hand-written file");

        let artifacts =
            project_artifacts(&root, &root.join("project.json")).expect("the project generates");
        let error = export::commit(&root, &artifacts).expect_err("not a generated file");
        assert!(error.contains("generated marker"), "{error}");
        assert_eq!(
            std::fs::read_to_string(&path).expect("still there"),
            "fn main() { println!(\"mine\"); }\n"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_project_export_merges_a_marked_file_and_keeps_its_handwritten_region() {
        let root = project("project-merge", ONE_OUTPUT);
        let project_document = root.join("project.json");
        let artifacts = project_artifacts(&root, &project_document).expect("the first generation");
        export::commit(&root, &artifacts).expect("the first commit");

        let path = root.join("src/generated/main.rs");
        let mut committed = std::fs::read_to_string(&path).expect("the product");
        committed.push_str(
            "// region:handwritten save\nfn save() { /* mine */ }\n// endregion:handwritten save\n",
        );
        std::fs::write(&path, &committed).expect("the hand-written region");

        let page = root.join("pages/main.json");
        let text = std::fs::read_to_string(&page).expect("the page");
        std::fs::write(&page, text.replace("\"Hi\"", "\"Hi again\"")).expect("the edit");

        let next = project_artifacts(&root, &project_document).expect("the second generation");
        let results = export::commit(&root, &next).expect("the merged commit");
        assert_eq!(results[0].action, export::Action::Updated);

        let after = std::fs::read_to_string(&path).expect("the merged product");
        assert!(after.contains("Hi again"), "{after}");
        assert!(after.contains("fn save() { /* mine */ }"), "{after}");

        let _ = std::fs::remove_dir_all(&root);
    }
}
