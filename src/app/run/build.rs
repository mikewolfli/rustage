// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! F16: the build, its problems, the run, terminals, tasks, external tools, the end-to-end walk, the
//! task graph, reproducibility and the headless walk.
//!
//! One command handler per parsed command of this domain; the dispatcher lives in [`super`](self).
//! Where a command can run a real program (`--compile execute`), it does; where it cannot, it prints
//! the exact command it would run rather than a success shape.

use super::*;
use crate::app::build::build_diagnostics::Report;
use crate::app::quality::{Center, Filter};
use crate::contracts::identity::NodeId;
use crate::contracts::revision::{GeneratorVersion, Revision};
use crate::contracts::source_map::{SourceMap, SourceSpan};
use std::time::Duration;

/// How `--compile` reads a real build's `--message-format=json` stream into problems (task T16.03).
///
/// The ops are optional: every `execute` run already captures the JSON stream, and naming these
/// adds the map and the source text needed to link a position back to a node, plus where to record
/// the resulting rows.
#[derive(Default)]
struct BuildDiagnostics {
    /// The path `diag-source` read, used to match the compiler's relative file name.
    source_path: Option<String>,
    /// The text of that file, for turning line/column into a byte offset.
    source_text: Option<String>,
    /// The model revision the build was started against.
    model: Revision,
    /// The revision now on screen, when the caller wants a staleness verdict.
    current: Option<Revision>,
    /// Node spans from the source map.
    spans: Vec<(NodeId, SourceSpan)>,
    /// Where the parsed problem rows are written, when the caller asked for a record.
    record: Option<String>,
    /// Whether any diagnostic op was named, so a stream that cannot exist is refused.
    explicit: bool,
}

impl BuildDiagnostics {
    /// The source map the spans describe, stamped with the model revision.
    fn map(&self) -> SourceMap {
        let mut map = SourceMap::new(GeneratorVersion::CURRENT, self.model);
        for (node, span) in &self.spans {
            map.insert(*node, span.clone());
        }
        map
    }

    /// The text of `file`: the supplied source when it is that file, else what is on disk.
    fn text_of(&self, file: &str) -> Option<String> {
        if let (Some(path), Some(text)) = (&self.source_path, &self.source_text)
            && (std::path::Path::new(path).ends_with(file)
                || std::path::Path::new(file).ends_with(path))
        {
            return Some(text.clone());
        }
        read(file).ok()
    }
}

/// Parses a captured JSON stream and places each message against the map.
fn build_report(output: &str, diagnostics: &BuildDiagnostics) -> Report {
    use crate::app::build::build_diagnostics::parse;

    let batch = parse(output, diagnostics.model);
    let map = diagnostics.map();
    let mut files: std::collections::HashMap<String, Option<String>> =
        std::collections::HashMap::new();
    for message in &batch.messages {
        if let Some(span) = &message.span {
            files
                .entry(span.file.clone())
                .or_insert_with(|| diagnostics.text_of(&span.file));
        }
    }
    batch.place(&map, |file| files.get(file).cloned().flatten())
}

/// Prints the problems a real build reported and records them for the problem centre.
///
/// This runs after the exit was measured and never replaces it: the verdict is the process's, and
/// a diagnostic stream is what the process said on the way there.
fn print_build_diagnostics(output: &str, diagnostics: &BuildDiagnostics, target: &str) {
    let report = build_report(output, diagnostics);
    let mut center = Center::new();
    report.record(&mut center, target);
    for line in report.render(target) {
        println!("{line}");
    }
    if let Some(current) = diagnostics.current {
        println!(
            "{}",
            crate::contracts::revision::Staleness::between(diagnostics.model, current)
        );
    }
    if let Some(path) = &diagnostics.record {
        let mut text = center.render(&Filter::default()).join("\n");
        if !text.is_empty() {
            text.push('\n');
        }
        if let Err(error) = std::fs::write(path, text) {
            println!("the build problems could not be written to {path}: {error}");
        }
    }
}

/// Reads a file, or reports why it could not be.
fn read(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|error| format!("{path} could not be read: {error}"))
}

/// Splits `name=value`.
fn pair<'a>(rest: &'a str, what: &str) -> Result<(&'a str, &'a str), String> {
    rest.split_once('=')
        .ok_or_else(|| format!("{what} needs <name>=<value>, got {rest:?}"))
}

/// Parses `yes`/`no`.
fn yes_no(value: &str) -> Result<bool, String> {
    match value {
        "yes" | "true" => Ok(true),
        "no" | "false" => Ok(false),
        other => Err(format!("{other:?} is not yes or no")),
    }
}

/// Probes a toolchain and chooses a feature set (task T16.01).
///
/// The probe is the real machine first: `cargo --version`, `rustc --version` and
/// `rustup target list --installed` each run as their own argv, never through a shell. The CLI
/// operations then override or extend what it found, which is how a test drives this path without
/// the machine having to match, and how a person corrects a probe that looked at the wrong toolchain.
pub(super) fn toolchain(options: &crate::cli::ToolchainOptions) -> bool {
    toolchain_with(options, crate::app::build::toolchain::Environment::probe())
}

/// The same, with the probe supplied, so a test can judge a machine it describes itself.
fn toolchain_with(
    options: &crate::cli::ToolchainOptions,
    mut environment: crate::app::build::toolchain::Environment,
) -> bool {
    use crate::app::build::toolchain::{Finding, diagnose, selection};

    let mut profile = String::new();
    let mut extra: Vec<String> = Vec::new();
    let mut triple: Option<String> = None;
    let mut check = false;
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<(), String> = match verb {
            "profile" => {
                profile = argument.to_owned();
                Ok(())
            }
            "feature" => {
                extra.push(argument.to_owned());
                Ok(())
            }
            "target" => {
                triple = Some(argument.to_owned());
                Ok(())
            }
            "cargo" => {
                environment.cargo = Some(argument.to_owned());
                Ok(())
            }
            "rustc" => {
                environment.rustc = Some(argument.to_owned());
                Ok(())
            }
            "installed" => {
                environment.targets.push(argument.to_owned());
                Ok(())
            }
            "sdk" => {
                environment.sdk = Some(argument.to_owned());
                Ok(())
            }
            "check" => {
                check = true;
                Ok(())
            }
            "show" => Ok(()),
            other => Err(format!("{other:?} is not a toolchain operation")),
        };
        if let Err(reason) = step {
            return report(reason);
        }
    }
    let chosen = match selection(&profile, &extra) {
        Ok(chosen) => chosen,
        Err(findings) => {
            for finding in &findings {
                println!("{}", finding.render());
            }
            return false;
        }
    };
    println!("{}", chosen.render());
    println!("features: {}", chosen.feature_arguments().join(" "));
    for line in environment.render() {
        println!("{line}");
    }
    let findings = diagnose(&chosen, triple.as_deref(), &environment);
    for finding in &findings {
        println!("{}", finding.render());
    }
    let unsupported = findings
        .iter()
        .any(|finding| matches!(finding, Finding::Unsupported { .. }));
    if !findings.is_empty() {
        println!("toolchain: not ready ({} finding(s))", findings.len());
    } else if check {
        println!("toolchain: ready");
    }
    if unsupported {
        return false;
    }
    // `check` is the request to judge readiness: a missing tool or target is not ready, while
    // without it this is a report of what the probe found.
    !check || findings.is_empty()
}

/// Checks, builds or rebuilds (task T16.02).
pub(super) fn compile(options: &crate::cli::CompileOptions) -> bool {
    use crate::app::build::compile::{Action, Job};
    use crate::app::build::reproducible::Reproducibility;
    use crate::app::build::toolchain::Selection;

    let mut action = Action::Check;
    let mut selection = Selection {
        profile: "desktop".to_owned(),
        extra: Vec::new(),
    };
    let mut job_target: Option<String> = None;
    let mut package: Option<String> = None;
    let mut manifest: Option<String> = None;
    let mut reproducible = Reproducibility::default();
    let mut expected_exit: Option<i32> = None;
    let mut cancel = false;
    let mut execute = false;
    let mut diagnostics = BuildDiagnostics::default();
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<(), String> = match verb {
            "action" => match argument {
                "check" => {
                    action = Action::Check;
                    Ok(())
                }
                "build" => {
                    action = Action::Build;
                    Ok(())
                }
                "rebuild" => {
                    action = Action::Rebuild;
                    Ok(())
                }
                other => Err(format!("{other:?} is not an action")),
            },
            "profile" => {
                selection.profile = argument.to_owned();
                Ok(())
            }
            "feature" => {
                selection.extra.push(argument.to_owned());
                Ok(())
            }
            "target" => {
                job_target = Some(argument.to_owned());
                Ok(())
            }
            "package" => {
                package = Some(argument.to_owned());
                Ok(())
            }
            "manifest" => {
                manifest = Some(argument.to_owned());
                Ok(())
            }
            "locked" => yes_no(argument).map(|value| reproducible.locked = value),
            "offline" => yes_no(argument).map(|value| reproducible.offline = value),
            "vendor" => {
                reproducible.vendor = Some(argument.to_owned());
                Ok(())
            }
            "cache" => {
                reproducible.cache = Some(argument.to_owned());
                Ok(())
            }
            "source" => {
                reproducible.source = Some(argument.to_owned());
                Ok(())
            }
            "diag-source" => match read(argument) {
                Ok(text) => {
                    diagnostics.source_path = Some(argument.to_owned());
                    diagnostics.source_text = Some(text);
                    diagnostics.explicit = true;
                    Ok(())
                }
                Err(reason) => Err(reason),
            },
            "diag-model" => match argument.parse::<u64>() {
                Ok(value) => {
                    diagnostics.model = Revision::from_raw(value);
                    diagnostics.explicit = true;
                    Ok(())
                }
                Err(_) => Err(format!("{argument:?} is not a revision")),
            },
            "diag-current" => match argument.parse::<u64>() {
                Ok(value) => {
                    diagnostics.current = Some(Revision::from_raw(value));
                    diagnostics.explicit = true;
                    Ok(())
                }
                Err(_) => Err(format!("{argument:?} is not a revision")),
            },
            "diag-span" => {
                let parts: Vec<&str> = argument.split(':').collect();
                match parts.as_slice() {
                    [node, path, start, end] => match (
                        node.parse::<u64>(),
                        start.parse::<u32>(),
                        end.parse::<u32>(),
                    ) {
                        (Ok(node), Ok(start), Ok(end)) => match SourceSpan::new(path, start, end) {
                            Some(span) => {
                                diagnostics.spans.push((NodeId::from_raw(node), span));
                                diagnostics.explicit = true;
                                Ok(())
                            }
                            None => Err(format!("diag-span {argument:?} is empty or reversed")),
                        },
                        _ => Err(format!(
                            "diag-span {argument:?} is not <node>:<file>:<start>:<end>"
                        )),
                    },
                    _ => Err(format!(
                        "diag-span needs <node>:<file>:<start>:<end>, got {argument:?}"
                    )),
                }
            }
            "diag-record" => {
                diagnostics.record = Some(argument.to_owned());
                diagnostics.explicit = true;
                Ok(())
            }
            "exit" => match argument.parse::<i32>() {
                Ok(code) => {
                    expected_exit = Some(code);
                    Ok(())
                }
                Err(_) => Err(format!("{argument:?} is not an exit code")),
            },
            "cancel" => {
                cancel = true;
                Ok(())
            }
            "execute" => {
                execute = true;
                Ok(())
            }
            "show" => Ok(()),
            other => Err(format!("{other:?} is not a compile operation")),
        };
        if let Err(reason) = step {
            return report(reason);
        }
    }
    let diagnostic_target = job_target
        .clone()
        .unwrap_or_else(|| selection.profile.clone());
    let job = Job {
        action,
        selection,
        target: job_target,
        package,
        manifest,
        reproducible,
    };
    if !job.reproducible.cache_is_outside_source() {
        return report(
            "the build cache is inside the committed source; choose a cache outside it".to_owned(),
        );
    }
    if let Some(clean) = job.clean_command() {
        println!("clean: cargo {}", clean.join(" "));
    }
    println!("cargo {}", job.command().join(" "));
    if execute {
        return execute_compile(
            &job,
            cancel,
            expected_exit,
            &diagnostics,
            &diagnostic_target,
        );
    }
    if diagnostics.explicit {
        return report(
            "build diagnostics read a real build's JSON stream; add `execute` so there is one"
                .to_owned(),
        );
    }
    if cancel {
        return report(
            "cancel is a request to a running build; add `execute` so there is one".to_owned(),
        );
    }
    if let Some(code) = expected_exit {
        return report(format!(
            "an exit code is a fact about a real run; add `execute` to measure it (expected {code})"
        ));
    }
    true
}

/// Runs the job through the host's task executor, which owns it and gives it a real token.
///
/// `cancel` is a request to the executor, not a value written into the result: the task's process
/// group is what stops. `expected_exit` turns `exit:<n>` into an assertion about the measured code
/// instead of a fabricated verdict. The captured stdout is cargo's JSON stream: it is parsed into
/// problems after the exit was measured, and the measured verdict is what this returns.
fn execute_compile(
    job: &crate::app::build::compile::Job,
    cancel: bool,
    expected_exit: Option<i32>,
    diagnostics: &BuildDiagnostics,
    target: &str,
) -> bool {
    use crate::app::build::compile::{BuildTask, Verdict};
    use crate::host::tasks::{Executor, ExecutorConfig, OwnerId, TaskState};

    if cancel && expected_exit.is_some() {
        return report(
            "a build cannot be both cancelled and expected to exit with a code".to_owned(),
        );
    }
    let executor = match Executor::new(ExecutorConfig::responsive()) {
        Ok(executor) => executor,
        Err(error) => return report(format!("the task executor could not start: {error}")),
    };
    let owner = OwnerId::new();
    let submitted = match BuildTask::submit(&executor, owner, job.clone(), "cargo") {
        Ok(submitted) => submitted,
        Err(error) => return report(format!("the build could not be submitted: {error}")),
    };

    if cancel {
        // A cancel is meaningful once the job is running; wait for that, but never past the point
        // where it has already finished (in which case the real exit stands).
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            match executor.record(submitted.task()) {
                Some(record) if record.state == TaskState::Running => break,
                Some(record) if record.is_final() => break,
                _ if std::time::Instant::now() >= deadline => break,
                _ => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        println!("cancel: {:?}", executor.cancel(submitted.task()));
    }

    // Wait for the task to be final; a build may take as long as it takes.
    let record = loop {
        let record = executor.wait(submitted.task(), Duration::from_secs(1));
        match record {
            Some(record) if record.is_final() => break record,
            Some(_) => continue,
            None => {
                let shutdown = executor.shutdown();
                if !shutdown.is_clean() {
                    crate::host::log::emit(
                        crate::host::log::Level::Warn,
                        "app::run::build",
                        &format!("build executor shutdown was not clean: {shutdown}"),
                    );
                }
                return report("the submitted build disappeared from the executor".to_owned());
            }
        }
    };

    let result = submitted.result();
    let shutdown = executor.shutdown();
    if !shutdown.is_clean() {
        crate::host::log::emit(
            crate::host::log::Level::Warn,
            "app::run::build",
            &format!("build executor shutdown was not clean: {shutdown}"),
        );
    }
    match result {
        Some(Ok(outcome)) => {
            for line in outcome.render() {
                println!("{line}");
            }
            print_build_diagnostics(&outcome.stdout, diagnostics, target);
            if let Some(expected) = expected_exit {
                let measured = match outcome.verdict {
                    Verdict::Succeeded => Some(0),
                    Verdict::Failed { code } => Some(code),
                    _ => None,
                };
                if measured != Some(expected) {
                    return report(format!(
                        "the build was expected to exit {expected} but {}: {}",
                        outcome.verdict.render(),
                        record.summary(),
                    ));
                }
            }
            outcome.verdict.is_success()
        }
        Some(Err(reason)) => report(reason),
        // The job never ran (cancelled while queued) or was abandoned; the record is the fact.
        None => {
            println!("{}", record.summary());
            false
        }
    }
}

/// Reads a build's problems and links them to nodes (task T16.03).
pub(super) fn build_diag(options: &crate::cli::BuildDiagOptions) -> bool {
    use crate::app::build::build_diagnostics::parse;

    let mut output = String::new();
    let mut source = String::new();
    let mut file = "src/main.rs".to_owned();
    let mut model = Revision::INITIAL;
    let mut current: Option<Revision> = None;
    let mut spans: Vec<(NodeId, SourceSpan)> = Vec::new();

    // First pass: the file the spans and the map live in, and the spans themselves.
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<(), String> = match verb {
            "output" => read(argument).map(|text| output = text),
            "source" => read(argument).map(|text| source = text),
            "model" => match argument.parse::<u64>() {
                Ok(value) => {
                    model = Revision::from_raw(value);
                    Ok(())
                }
                Err(_) => Err(format!("{argument:?} is not a revision")),
            },
            "current" => match argument.parse::<u64>() {
                Ok(value) => {
                    current = Some(Revision::from_raw(value));
                    Ok(())
                }
                Err(_) => Err(format!("{argument:?} is not a revision")),
            },
            "span" => {
                let parts: Vec<&str> = argument.split(':').collect();
                match parts.as_slice() {
                    [node, path, start, end] => match (
                        node.parse::<u64>(),
                        start.parse::<u32>(),
                        end.parse::<u32>(),
                    ) {
                        (Ok(node), Ok(start), Ok(end)) => match SourceSpan::new(path, start, end) {
                            Some(span) => {
                                file = (*path).to_owned();
                                spans.push((NodeId::from_raw(node), span));
                                Ok(())
                            }
                            None => Err(format!("span {argument:?} is empty or reversed")),
                        },
                        _ => Err(format!(
                            "span {argument:?} is not <node>:<file>:<start>:<end>"
                        )),
                    },
                    _ => Err(format!(
                        "span needs <node>:<file>:<start>:<end>, got {argument:?}"
                    )),
                }
            }
            "key" | "show" => Ok(()),
            other => Err(format!("{other:?} is not a build-diag operation")),
        };
        if let Err(reason) = step {
            return report(reason);
        }
    }

    let mut map = SourceMap::new(GeneratorVersion::CURRENT, model);
    for (node, span) in spans {
        map.insert(node, span);
    }
    let batch = parse(&output, model);
    let report = batch.place(&map, |span_file| {
        if source.is_empty() {
            read(span_file).ok()
        } else {
            Some(source.clone())
        }
    });
    for line in batch.render() {
        println!("{line}");
    }
    for diagnostic in &report.diagnostics {
        if let Some(node) = diagnostic.node {
            let at = diagnostic
                .message
                .span
                .as_ref()
                .map(|span| span.file.as_str())
                .unwrap_or(&file);
            println!("  -> node {} in {at}", node.get());
        } else if diagnostic.unmapped {
            println!("  -> unmapped (position kept)");
        }
    }
    if let Some(current) = current {
        println!("{}", batch.staleness(current));
    }
    batch.errors() == 0
}

/// How long `exit:<code>` and `signal:<n>` wait for the real process before reporting it.
const PROGRAM_WAIT: Duration = Duration::from_secs(30);

/// Runs the delivered program on its own (task T16.04).
///
/// `start` really starts the program; `stop` signals the process group; `exit:<code>` and
/// `signal:<n>` are assertions about the measured exit, never a value written into the reason.
pub(super) fn run_program(options: &crate::cli::RunOptions) -> bool {
    use crate::app::build::runner::{Artifact, Config, Reason, Run, State};

    let mut config = Config {
        program: String::new(),
        args: Vec::new(),
        environment: Vec::new(),
        directory: None,
        standalone: false,
        artifact: Artifact::Preview,
    };
    let mut artifact: Option<Artifact> = None;
    let mut run: Option<Run> = None;
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "program" => {
                config.program = argument.to_owned();
                Ok(Vec::new())
            }
            "arg" => {
                config.args.push(argument.to_owned());
                Ok(Vec::new())
            }
            "env" => match pair(argument, "env") {
                Ok((name, value)) => {
                    config.environment.push((name.to_owned(), value.to_owned()));
                    Ok(Vec::new())
                }
                Err(reason) => Err(reason),
            },
            "directory" => {
                config.directory = Some(argument.to_owned());
                Ok(Vec::new())
            }
            "standalone" => match yes_no(argument) {
                Ok(value) => {
                    config.standalone = value;
                    Ok(Vec::new())
                }
                Err(reason) => Err(reason),
            },
            "artifact" => match argument {
                "preview" => {
                    artifact = Some(Artifact::Preview);
                    Ok(Vec::new())
                }
                "compiled" => {
                    artifact = Some(Artifact::Compiled);
                    Ok(Vec::new())
                }
                other => Err(format!("{other:?} is not preview or compiled")),
            },
            "start" => {
                config.artifact = artifact.unwrap_or(if config.standalone {
                    Artifact::Compiled
                } else {
                    Artifact::Preview
                });
                run = Some(Run::new(config.clone()));
                let started = run.as_mut().expect("set above").start();
                match started {
                    Ok(()) => {
                        let pid = run
                            .as_ref()
                            .and_then(Run::pid)
                            .map(|pid| pid.to_string())
                            .unwrap_or_else(|| "unknown".to_owned());
                        Ok(vec![format!(
                            "started {} ({} artifact, pid {pid})",
                            config.render(),
                            config.artifact.name(),
                        )])
                    }
                    Err(reason) => {
                        // A failed start leaves no run behind, so a later stop cannot claim one.
                        run = None;
                        Err(reason)
                    }
                }
            }
            "exit" => match argument.parse::<i32>() {
                Ok(code) => match run.as_mut() {
                    Some(run) => match run.wait(PROGRAM_WAIT) {
                        Some(reason) => match (reason, code) {
                            (Reason::Clean, 0) => {
                                Ok(vec![format!("ended: {}", Reason::Clean.render())])
                            }
                            (Reason::NonZero { code: actual }, expected) if actual == expected => {
                                Ok(vec![format!("ended: exited with code {actual}")])
                            }
                            (other, expected) => Err(format!(
                                "the program did not exit {expected}; it {}",
                                other.render()
                            )),
                        },
                        None => Err(format!(
                            "the program is still running after {PROGRAM_WAIT:?}; no exit was measured"
                        )),
                    },
                    None => Err("no run has started".to_owned()),
                },
                Err(_) => Err(format!("{argument:?} is not an exit code")),
            },
            "signal" => match argument.parse::<i32>() {
                Ok(expected) => match run.as_mut() {
                    Some(run) => match run.wait(PROGRAM_WAIT) {
                        Some(Reason::Signalled { signal }) if signal == expected => {
                            Ok(vec![format!("ended: killed by signal {signal}")])
                        }
                        Some(other) => Err(format!(
                            "the program was not killed by signal {expected}; it {}",
                            other.render()
                        )),
                        None => Err(format!(
                            "the program is still running after {PROGRAM_WAIT:?}; no signal was measured"
                        )),
                    },
                    None => Err("no run has started".to_owned()),
                },
                Err(_) => Err(format!("{argument:?} is not a signal")),
            },
            "stop" => match run.as_mut() {
                Some(run) => match run.stop() {
                    Ok(()) => Ok(vec![
                        run.reason
                            .as_ref()
                            .map(Reason::render)
                            .unwrap_or_else(|| "stopped".to_owned()),
                    ]),
                    Err(reason) => Err(reason),
                },
                None => Err("no run has started".to_owned()),
            },
            "restart" => match run.as_mut() {
                Some(run) => match run.restart() {
                    Ok(()) => Ok(vec!["restarted".to_owned()]),
                    Err(reason) => Err(reason),
                },
                None => Err("no run has started".to_owned()),
            },
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a run operation")),
        };
        match step {
            Ok(notes) => {
                for note in notes {
                    println!("{note}");
                }
            }
            Err(reason) => {
                // A failed start or a mismatch must not leave the program running behind us. If the
                // stop itself fails, say so instead of promising a cleanup that did not happen.
                if let Some(run) = run.as_mut()
                    && run.is_running()
                    && let Err(stop_reason) = run.stop()
                {
                    return report(format!(
                        "{reason}; the program could not be stopped and may still be running: \
                         {stop_reason}"
                    ));
                }
                return report(reason);
            }
        }
    }
    if let Some(run) = run.as_mut() {
        // This host owns the process; returning while it still runs would leave an orphan. A
        // one-shot CLI command stops what it started, and says so.
        if run.is_running() {
            match run.stop() {
                Ok(()) => println!(
                    "stopped: {}",
                    run.reason
                        .as_ref()
                        .map(Reason::render)
                        .unwrap_or_else(|| "stopped on request".to_owned())
                ),
                Err(reason) => return report(reason),
            }
        }
        println!(
            "state: {}",
            match run.state {
                State::Idle => "idle",
                State::Running => "running",
                State::Ended => "ended",
            }
        );
        if let Some(reason) = &run.reason {
            println!("reason: {}", reason.render());
        }
    }
    true
}

/// Orders configured tasks (task T16.05).
pub(super) fn tasks(options: &crate::cli::TasksOptions) -> bool {
    use crate::app::build::tasks::{
        Task, cache_key, cleanup, cleanup_plan, default_task, order, schedule,
    };

    let mut tasks: Vec<Task> = Vec::new();
    let mut fingerprints: Vec<(String, String)> = Vec::new();
    let mut declared_default: Option<String> = None;
    let mut root = ".".to_owned();
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<(), String> = match verb {
            "task" => match argument.split_once(':') {
                Some((name, command)) => {
                    tasks.push(Task {
                        name: name.to_owned(),
                        command: vec![command.to_owned()],
                        depends_on: Vec::new(),
                        pre: Vec::new(),
                        post: Vec::new(),
                        directory: None,
                        inputs: Vec::new(),
                        outputs: Vec::new(),
                    });
                    Ok(())
                }
                None => Err(format!("task needs <name>:<command>, got {argument:?}")),
            },
            "after" => match argument.split_once(':') {
                Some((name, dependency)) => match tasks.iter_mut().find(|task| task.name == name) {
                    Some(task) => {
                        task.depends_on.push(dependency.to_owned());
                        Ok(())
                    }
                    None => Err(format!("task {name:?} is not defined yet")),
                },
                None => Err(format!("after needs <name>:<dependency>, got {argument:?}")),
            },
            "pre" => match argument.split_once(':') {
                Some((name, task)) => match tasks.iter_mut().find(|defined| defined.name == name) {
                    Some(defined) => {
                        defined.pre.push(task.to_owned());
                        Ok(())
                    }
                    None => Err(format!("task {name:?} is not defined yet")),
                },
                None => Err(format!("pre needs <name>:<task>, got {argument:?}")),
            },
            "post" => match argument.split_once(':') {
                Some((name, task)) => match tasks.iter_mut().find(|defined| defined.name == name) {
                    Some(defined) => {
                        defined.post.push(task.to_owned());
                        Ok(())
                    }
                    None => Err(format!("task {name:?} is not defined yet")),
                },
                None => Err(format!("post needs <name>:<task>, got {argument:?}")),
            },
            "root" => {
                if argument.trim().is_empty() {
                    Err("root needs a path".to_owned())
                } else {
                    root = argument.to_owned();
                    Ok(())
                }
            }
            "input" => match argument.split_once(':') {
                Some((name, path)) => match tasks.iter_mut().find(|task| task.name == name) {
                    Some(task) => {
                        task.inputs.push(path.to_owned());
                        Ok(())
                    }
                    None => Err(format!("task {name:?} is not defined yet")),
                },
                None => Err(format!("input needs <name>:<path>, got {argument:?}")),
            },
            "fingerprint" => match argument.split_once(':') {
                Some((path, value)) => {
                    fingerprints.push((path.to_owned(), value.to_owned()));
                    Ok(())
                }
                None => Err(format!(
                    "fingerprint needs <path>:<value>, got {argument:?}"
                )),
            },
            "output" => match argument.split_once(':') {
                Some((name, path)) => match tasks.iter_mut().find(|task| task.name == name) {
                    Some(task) => {
                        task.outputs.push(path.to_owned());
                        Ok(())
                    }
                    None => Err(format!("task {name:?} is not defined yet")),
                },
                None => Err(format!("output needs <name>:<path>, got {argument:?}")),
            },
            "default" => {
                declared_default = Some(argument.to_owned());
                Ok(())
            }
            "order" | "show" => Ok(()),
            "key" => Ok(()),
            "cleanup" => Ok(()),
            "schedule" => Ok(()),
            "clean-preview" => Ok(()),
            "clean" => Ok(()),
            other => Err(format!("{other:?} is not a tasks operation")),
        };
        if let Err(reason) = step {
            return report(reason);
        }
    }
    let ordered = match order(&tasks) {
        Ok(ordered) => ordered,
        Err(reason) => return report(reason),
    };
    let waves = match schedule(&tasks) {
        Ok(waves) => waves,
        Err(reason) => return report(reason),
    };
    let default = match default_task(&tasks, declared_default.as_deref()) {
        Ok(default) => default,
        Err(reason) => return report(reason),
    };
    for line in crate::app::build::tasks::render(&tasks, &ordered, &default) {
        println!("{line}");
    }
    // A wave is what may run at once; the order above is what runs.
    for (index, wave) in waves.iter().enumerate() {
        println!("wave {}: {}", index + 1, wave.join(", "));
    }
    // Only a task whose scope was shown in this invocation may be cleaned.
    let mut previewed: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        match verb {
            "key" => match tasks.iter().find(|task| task.name == argument) {
                Some(task) => println!("key {argument}: {}", cache_key(task, &[], &fingerprints)),
                None => return report(format!("no task {argument:?}")),
            },
            "cleanup" => match tasks.iter().find(|task| task.name == argument) {
                Some(task) => println!("cleanup {argument}: {:?}", cleanup(task)),
                None => return report(format!("no task {argument:?}")),
            },
            "clean-preview" => {
                if argument.is_empty() {
                    return report("clean-preview needs a task".to_owned());
                }
                let plan = cleanup_plan(std::path::Path::new(&root), &tasks, &[argument]);
                for line in plan.render() {
                    println!("{line}");
                }
                if !plan.problems.is_empty() {
                    return report(plan.problems.join("; "));
                }
                previewed.insert(argument.to_owned());
            }
            "clean" => {
                if argument.is_empty() {
                    return report("clean needs a task".to_owned());
                }
                if !previewed.contains(argument) {
                    return report(format!(
                        "clean {argument} needs clean-preview:{argument} first, so the scope is shown \
                         before anything is removed"
                    ));
                }
                let plan = cleanup_plan(std::path::Path::new(&root), &tasks, &[argument]);
                let (removed, failed) = plan.apply();
                for path in &removed {
                    println!("removed {path}");
                }
                for failure in &failed {
                    println!("failed {failure}");
                }
                if !failed.is_empty() {
                    return report(format!("{} path(s) could not be removed", failed.len()));
                }
            }
            _ => {}
        }
    }
    true
}

/// Drives a terminal session (task T16.06).
pub(super) fn terminal(options: &crate::cli::TerminalOptions) -> bool {
    use crate::app::build::terminal::{Platform, Session, mechanism};

    let mut session = Session::new("terminal");
    let mut platform = Platform::Macos;
    #[cfg(any(unix, windows))]
    let mut real: Option<crate::host::pty::PtySession> = None;
    #[cfg(any(unix, windows))]
    let mut program: Option<String> = None;
    let mut arguments: Vec<String> = Vec::new();
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "session" => {
                session.name = argument.to_owned();
                Ok(Vec::new())
            }
            "directory" => {
                session.directory = Some(argument.to_owned());
                Ok(Vec::new())
            }
            "env" => match pair(argument, "env") {
                Ok((name, value)) => {
                    session
                        .environment
                        .push((name.to_owned(), value.to_owned()));
                    Ok(Vec::new())
                }
                Err(reason) => Err(reason),
            },
            "size" | "resize" => match argument.split_once('x') {
                Some((columns, rows)) => match (columns.parse::<u16>(), rows.parse::<u16>()) {
                    (Ok(columns), Ok(rows)) => match session.resize(columns, rows) {
                        Ok(()) => {
                            #[cfg(any(unix, windows))]
                            if let Some(real) = real.as_mut()
                                && let Err(reason) = real.resize(columns, rows)
                            {
                                return report(reason);
                            }
                            Ok(vec![format!("size {columns}x{rows}")])
                        }
                        Err(reason) => Err(reason),
                    },
                    _ => Err(format!("{argument:?} is not <columns>x<rows>")),
                },
                None => Err(format!("size needs <columns>x<rows>, got {argument:?}")),
            },
            "arg" => {
                arguments.push(argument.to_owned());
                Ok(Vec::new())
            }
            "exec" => {
                // `exec` only names the program; `start` is what runs it, after any `arg:` ops.
                // Spawning here would make `exec:/bin/sh arg:-c arg:...` impossible, because the
                // shell word order is exactly the program-then-arguments order users type.
                #[cfg(any(unix, windows))]
                {
                    if real.is_some() {
                        return report("a real terminal is already running".to_owned());
                    }
                    if argument.is_empty() {
                        return report("exec needs <program>".to_owned());
                    }
                    program = Some(argument.to_owned());
                    Ok(vec![format!("exec {argument}")])
                }
                #[cfg(not(any(unix, windows)))]
                {
                    Err("a real terminal is provided by the POSIX host; this host has no byte pump yet"
                        .to_owned())
                }
            }
            "input" => {
                #[cfg(any(unix, windows))]
                {
                    let Some(real) = real.as_mut() else {
                        return report("input needs a running exec: terminal".to_owned());
                    };
                    let text = unescape(argument);
                    match real.write(&text) {
                        Ok(()) => Ok(vec![format!("input: {} byte(s)", text.len())]),
                        Err(reason) => Err(reason),
                    }
                }
                #[cfg(not(any(unix, windows)))]
                {
                    let _ = argument;
                    Err("no real terminal on this host".to_owned())
                }
            }
            "read" => {
                #[cfg(any(unix, windows))]
                {
                    let Some(real) = real.as_ref() else {
                        return report("read needs a running exec: terminal".to_owned());
                    };
                    Ok(vec![
                        format!("output {:?}", real.output()),
                        format!("dropped {}", real.dropped()),
                    ])
                }
                #[cfg(not(any(unix, windows)))]
                {
                    Err("no real terminal on this host".to_owned())
                }
            }
            "wait" => {
                #[cfg(any(unix, windows))]
                {
                    let Some(real) = real.as_mut() else {
                        return report("wait needs a running exec: terminal".to_owned());
                    };
                    let limit = match argument.parse::<u64>() {
                        Ok(limit) => Duration::from_millis(limit),
                        Err(_) => {
                            return report(format!("{argument:?} is not a wait in milliseconds"));
                        }
                    };
                    match real.wait(limit) {
                        Ok(Some(status)) => Ok(vec![format!(
                            "exit {}",
                            status.code.map_or_else(
                                || format!("signal {}", status.signal.unwrap_or_default()),
                                |code| code.to_string()
                            )
                        )]),
                        Ok(None) => Ok(vec!["still running".to_owned()]),
                        Err(reason) => Err(reason),
                    }
                }
                #[cfg(not(any(unix, windows)))]
                {
                    Err("no real terminal on this host".to_owned())
                }
            }
            "terminate" => {
                #[cfg(any(unix, windows))]
                {
                    let Some(real) = real.as_mut() else {
                        return report("terminate needs a running exec: terminal".to_owned());
                    };
                    real.terminate();
                    session.running = false;
                    Ok(vec!["terminated".to_owned()])
                }
                #[cfg(not(any(unix, windows)))]
                {
                    Err("no real terminal on this host".to_owned())
                }
            }
            "platform" => match argument {
                "macos" => {
                    platform = Platform::Macos;
                    Ok(Vec::new())
                }
                "linux" => {
                    platform = Platform::Linux;
                    Ok(Vec::new())
                }
                "windows" => {
                    platform = Platform::Windows;
                    Ok(Vec::new())
                }
                other => Err(format!("{other:?} is not a platform")),
            },
            "authorize" => {
                session.authorize();
                Ok(vec!["authorized".to_owned()])
            }
            "start" => {
                #[cfg(any(unix, windows))]
                {
                    if let Some(program) = program.take() {
                        if !session.authorized {
                            return report(format!(
                                "session {:?} has not been authorized to run processes",
                                session.name
                            ));
                        }
                        match crate::host::pty::PtySession::spawn(
                            &program,
                            &arguments,
                            session.directory.as_deref(),
                            &session.environment,
                            session.columns,
                            session.rows,
                        ) {
                            Ok(started) => {
                                let pid = started.pid();
                                session.running = true;
                                real = Some(started);
                                arguments.clear();
                                Ok(vec![format!("started {program} pid {pid}")])
                            }
                            Err(reason) => Err(reason),
                        }
                    } else {
                        match session.start() {
                            Ok(()) => Ok(vec!["started".to_owned()]),
                            Err(reason) => Err(reason),
                        }
                    }
                }
                #[cfg(not(any(unix, windows)))]
                {
                    match session.start() {
                        Ok(()) => Ok(vec!["started".to_owned()]),
                        Err(reason) => Err(reason),
                    }
                }
            }
            "write" => {
                session.write(argument.as_bytes());
                Ok(Vec::new())
            }
            "copy" => match argument.split_once(':') {
                Some((start, end)) => match (start.parse::<usize>(), end.parse::<usize>()) {
                    (Ok(start), Ok(end)) => match session.copy(start, end) {
                        Ok(text) => Ok(vec![text]),
                        Err(reason) => Err(reason),
                    },
                    _ => Err(format!("{argument:?} is not <start>:<end>")),
                },
                None => Err(format!("copy needs <start>:<end>, got {argument:?}")),
            },
            "show" => Ok(vec![session.render()]),
            other => Err(format!("{other:?} is not a terminal operation")),
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
    println!("mechanism: {}", mechanism(platform).render());
    if session.dropped() > 0 {
        println!("dropped {} byte(s)", session.dropped());
    }
    true
}

/// Configures and invokes an external tool (task T16.07).
pub(super) fn external(options: &crate::cli::ExternalOptions) -> bool {
    use crate::app::build::external::{Tool, invoke};

    let mut tool = Tool {
        name: "tool".to_owned(),
        program: String::new(),
        args: Vec::new(),
        directory: None,
        inputs: Vec::new(),
        outputs: Vec::new(),
        confirm: false,
    };
    let mut placeholders = std::collections::BTreeMap::new();
    let mut confirmed = false;
    let mut do_invoke = false;
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<(), String> = match verb {
            "tool" => {
                tool.name = argument.to_owned();
                Ok(())
            }
            "program" => {
                tool.program = argument.to_owned();
                Ok(())
            }
            "arg" => {
                tool.args.push(argument.to_owned());
                Ok(())
            }
            "input" => {
                tool.inputs.push(argument.to_owned());
                Ok(())
            }
            "output" => {
                tool.outputs.push(argument.to_owned());
                Ok(())
            }
            "confirm" => yes_no(argument).map(|value| tool.confirm = value),
            "placeholder" => match pair(argument, "placeholder") {
                Ok((name, value)) => {
                    placeholders.insert(name.to_owned(), value.to_owned());
                    Ok(())
                }
                Err(reason) => Err(reason),
            },
            "invoke" => {
                do_invoke = true;
                Ok(())
            }
            "confirmed" => {
                confirmed = true;
                Ok(())
            }
            "show" => Ok(()),
            other => Err(format!("{other:?} is not an external operation")),
        };
        if let Err(reason) = step {
            return report(reason);
        }
    }
    if !do_invoke {
        println!("tool {}", tool.name);
        return true;
    }
    match invoke(&tool, &placeholders, confirmed) {
        Ok(invocation) => {
            println!("{}", invocation.render());
            if let Some(directory) = &invocation.directory {
                println!("in {directory}");
            }
            true
        }
        Err(reason) => report(reason),
    }
}

/// Records the generate → check → build → run → feedback walk (task T16.08).
pub(super) fn e2e(options: &crate::cli::E2eOptions) -> bool {
    use crate::app::build::e2e::{Outcome, Report, Stage};

    let mut walk = Report::new();
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let stage = match verb {
            "generate" => Some(Stage::Generate),
            "check" => Some(Stage::Check),
            "build" => Some(Stage::Build),
            "run" => Some(Stage::Run),
            "feedback" => Some(Stage::Feedback),
            _ => None,
        };
        if let Some(stage) = stage {
            let (verdict, note) = argument.split_once(':').unwrap_or((argument, ""));
            let outcome = match verdict {
                "ok" => Outcome::Ok,
                "fail" => Outcome::Failed(note.to_owned()),
                other => return report(format!("{other:?} is not ok or fail")),
            };
            walk.record(stage, outcome, note.to_owned());
            continue;
        }
        match verb {
            "skip" => walk.skip_missing(),
            "walk" => {
                if let Err(reason) = walk_stages(&mut walk, argument) {
                    return report(reason);
                }
            }
            "show" => {}
            other => return report(format!("{other:?} is not an e2e operation")),
        }
    }
    for line in walk.render() {
        println!("{line}");
    }
    if let Some(stage) = walk.blocked_at() {
        println!("blocked at {}", stage.name());
    }
    walk.is_success()
}

/// Runs the walk against a real project directory: check, build, then run.
///
/// `argument` is `<directory>:<program>`. Every stage is the real `cargo` invocation and the real
/// binary; a failing check records its first error line as the feedback, so the loop ends at the
/// compiler's own words rather than at a claimed success.
fn walk_stages(walk: &mut crate::app::build::e2e::Report, argument: &str) -> Result<(), String> {
    use crate::app::build::e2e::{Outcome, Stage};
    let (directory, program) = argument
        .split_once(':')
        .ok_or_else(|| format!("walk needs <directory>:<program>, got {argument:?}"))?;
    if directory.trim().is_empty() || program.trim().is_empty() {
        return Err("walk needs <directory>:<program>, got an empty field".to_owned());
    }
    if !std::path::Path::new(directory).join("Cargo.toml").is_file() {
        return Err(format!(
            "{} has no Cargo.toml, so it is not a project to walk",
            directory
        ));
    }

    let check = cargo(directory, &["check", "--quiet"]);
    match &check {
        Ok(()) => walk.record(Stage::Check, Outcome::Ok, "cargo check"),
        Err(line) => {
            walk.record(Stage::Check, Outcome::Failed(line.clone()), "cargo check");
            // The compiler's own line is the feedback: it names the file and the line number.
            walk.record(Stage::Feedback, Outcome::Failed(line.clone()), line.clone());
            return Ok(());
        }
    }

    match cargo(directory, &["build", "--quiet"]) {
        Ok(()) => walk.record(Stage::Build, Outcome::Ok, "cargo build"),
        Err(line) => {
            walk.record(Stage::Build, Outcome::Failed(line.clone()), "cargo build");
            walk.record(Stage::Feedback, Outcome::Failed(line.clone()), line.clone());
            return Ok(());
        }
    }

    let binary = std::path::Path::new(directory)
        .join("target")
        .join("debug")
        .join(program);
    match std::process::Command::new(&binary)
        .stdin(std::process::Stdio::null())
        .output()
    {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            walk.record(
                Stage::Run,
                Outcome::Ok,
                format!("{} exited 0 ({})", binary.display(), stdout),
            );
            walk.record(
                Stage::Feedback,
                Outcome::Ok,
                "the build reported no problem".to_owned(),
            );
            Ok(())
        }
        Ok(output) => {
            let reason = format!(
                "{} exited {}: {}",
                binary.display(),
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            );
            walk.record(Stage::Run, Outcome::Failed(reason.clone()), reason);
            Ok(())
        }
        Err(error) => {
            let reason = format!("cannot run {}: {error}", binary.display());
            walk.record(Stage::Run, Outcome::Failed(reason.clone()), reason);
            Ok(())
        }
    }
}

/// Runs one cargo command in a directory, answering the first error line it printed.
fn cargo(directory: &str, arguments: &[&str]) -> Result<(), String> {
    let output = std::process::Command::new("cargo")
        .args(arguments)
        .args(["--color", "never", "--manifest-path"])
        .arg(std::path::Path::new(directory).join("Cargo.toml"))
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|error| format!("cannot run cargo: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    // The compiler's own words: its first `error…` line, plus the `--> file:line` line under it.
    let stderr = String::from_utf8_lossy(&output.stderr);
    let lines: Vec<&str> = stderr.lines().collect();
    let start = lines
        .iter()
        .position(|line| line.trim_start().starts_with("error"))
        .unwrap_or(0);
    let mut message = lines
        .get(start)
        .map_or("cargo failed".to_owned(), |line| line.trim().to_owned());
    if let Some(location) = lines
        .iter()
        .skip(start + 1)
        .find(|line| line.trim_start().starts_with("-->"))
    {
        message.push(' ');
        message.push_str(location.trim());
    }
    Err(message)
}

/// Builds a typed task graph and matches its output (task T16.09).
pub(super) fn dag(options: &crate::cli::DagOptions) -> bool {
    use crate::app::build::dag::{Dag, Node, Order, Ready, match_line, matcher};

    let mut dag = Dag::default();
    let mut matchers = Vec::new();
    let mut ready: Vec<Ready> = Vec::new();
    // The problems the matchers read, and the target/revision they are recorded against.
    let mut found_problems: Vec<crate::app::build::dag::Problem> = Vec::new();
    let mut target = "desktop".to_owned();
    let mut revision: u64 = 0;
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<(), String> = match verb {
            "node" => {
                dag.nodes.push(Node {
                    name: argument.to_owned(),
                    depends_on: Vec::new(),
                    background: false,
                    terminal: None,
                });
                Ok(())
            }
            "after" | "together" => match argument.split_once(':') {
                Some((name, dependency)) => {
                    match dag.nodes.iter_mut().find(|node| node.name == name) {
                        Some(node) => {
                            let order = if verb == "after" {
                                Order::After
                            } else {
                                Order::Together
                            };
                            node.depends_on.push((dependency.to_owned(), order));
                            Ok(())
                        }
                        None => Err(format!("node {name:?} is not defined yet")),
                    }
                }
                None => Err(format!(
                    "{verb} needs <name>:<dependency>, got {argument:?}"
                )),
            },
            "background" => match argument.split_once(':') {
                Some((name, marker)) => match dag.nodes.iter_mut().find(|node| node.name == name) {
                    Some(node) => {
                        node.background = true;
                        ready.push(Ready {
                            task: name.to_owned(),
                            marker: marker.to_owned(),
                        });
                        Ok(())
                    }
                    None => Err(format!("node {name:?} is not defined yet")),
                },
                None => Err(format!(
                    "background needs <name>:<marker>, got {argument:?}"
                )),
            },
            "terminal" => match argument.split_once(':') {
                Some((name, terminal)) => match dag.nodes.iter_mut().find(|node| node.name == name)
                {
                    Some(node) => {
                        node.terminal = Some(terminal.to_owned());
                        Ok(())
                    }
                    None => Err(format!("node {name:?} is not defined yet")),
                },
                None => Err(format!(
                    "terminal needs <name>:<terminal>, got {argument:?}"
                )),
            },
            "default" => {
                dag.default = Some(argument.to_owned());
                Ok(())
            }
            "pattern" => {
                // pattern:<owner>:<pattern>:<file>:<line>:<message>. The pattern may itself hold
                // colons only inside its parentheses, which none of the supported groups do.
                let parts: Vec<&str> = argument.split(':').collect();
                if parts.len() < 5 {
                    return report(format!(
                        "pattern needs <owner>:<pattern>:<file>:<line>:<message>, got {argument:?}"
                    ));
                }
                let message = parts[parts.len() - 1];
                let line = parts[parts.len() - 2];
                let file = parts[parts.len() - 3];
                let owner = parts[0];
                let pattern = parts[1..parts.len() - 3].join(":");
                match (line.parse::<usize>(), message.parse::<usize>()) {
                    (Ok(line), Ok(message)) => {
                        match matcher(
                            owner,
                            &pattern,
                            file.parse::<usize>().unwrap_or(0),
                            line,
                            message,
                        ) {
                            Ok(m) => {
                                matchers.push(m);
                                Ok(())
                            }
                            Err(reason) => Err(reason),
                        }
                    }
                    _ => Err(format!(
                        "pattern {argument:?} has a non-numeric group index"
                    )),
                }
            }
            "match" => {
                let mut found = false;
                for m in &matchers {
                    if let Some(problem) = match_line(m, argument) {
                        println!("{}", problem.render());
                        found_problems.push(problem);
                        found = true;
                    }
                }
                if !found {
                    println!("no matcher read {argument:?}");
                }
                Ok(())
            }
            "target" => {
                if argument.trim().is_empty() {
                    Err("target needs a name".to_owned())
                } else {
                    target = argument.to_owned();
                    Ok(())
                }
            }
            "revision" => match argument.parse() {
                Ok(value) => {
                    revision = value;
                    Ok(())
                }
                Err(_) => Err(format!("{argument:?} is not a revision")),
            },
            "waves" | "show" => Ok(()),
            "ready" => match argument.split_once(':') {
                Some((name, text)) => match ready.iter().find(|r| r.task == name) {
                    Some(r) => {
                        println!("{name} ready: {}", r.is_ready(text));
                        Ok(())
                    }
                    None => Err(format!("no readiness marker for {name:?}")),
                },
                None => Err(format!("ready needs <name>:<text>, got {argument:?}")),
            },
            other => Err(format!("{other:?} is not a dag operation")),
        };
        if let Err(reason) = step {
            return report(reason);
        }
    }
    match dag.waves() {
        Ok(waves) => {
            for (index, wave) in waves.iter().enumerate() {
                println!("wave {index}: {}", wave.join(", "));
            }
            // A matched problem is a build problem with a source, a target and a revision, so it
            // goes into the same centre the compiler's own diagnostics use — a log line with no
            // owner or version is the thing T16.09 exists to stop.
            let mut center = crate::app::quality::Center::new();
            for problem in &found_problems {
                center.add(crate::app::quality::Problem {
                    source: crate::app::quality::Source::Build,
                    target: target.clone(),
                    revision,
                    node: None,
                    location: Some(format!("{}:{}", problem.file, problem.line)),
                    severity: crate::app::quality::Severity::Error,
                    message: format!("{}: {}", problem.owner, problem.message),
                });
            }
            for line in center.render(&crate::app::quality::Filter::default()) {
                println!("{line}");
            }
            println!(
                "matched problems: {} (source build, target {target}, revision {revision})",
                center.render(&crate::app::quality::Filter::default()).len()
            );
            true
        }
        Err(reason) => report(reason),
    }
}

/// States a reproducible, offline build (task T16.10).
pub(super) fn reproducible(options: &crate::cli::ReproducibleOptions) -> bool {
    use crate::app::build::reproducible::Reproducibility;

    let mut settings = Reproducibility::default();
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<(), String> = match verb {
            "locked" => yes_no(argument).map(|value| settings.locked = value),
            "offline" => yes_no(argument).map(|value| settings.offline = value),
            "vendor" => {
                settings.vendor = Some(argument.to_owned());
                Ok(())
            }
            "cache" => {
                settings.cache = Some(argument.to_owned());
                Ok(())
            }
            "source" => {
                settings.source = Some(argument.to_owned());
                Ok(())
            }
            "show" => Ok(()),
            other => Err(format!("{other:?} is not a reproducible operation")),
        };
        if let Err(reason) = step {
            return report(reason);
        }
    }
    let mut ok = true;
    for line in settings.render() {
        println!("{line}");
    }
    for line in settings.vendor_config() {
        println!("{line}");
    }
    if !settings.cache_is_outside_source() {
        ok = false;
    }
    ok
}

/// Runs the headless walk and exports its CI (task T16.11).
pub(super) fn headless(options: &crate::cli::HeadlessOptions) -> bool {
    use crate::app::build::headless::{Outcome, Step, exit_code, github_workflow, render_json};

    let mut outcomes: Vec<Outcome> = Vec::new();
    let mut profile = "desktop".to_owned();
    let mut workflow = false;
    let mut json = false;
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step = match verb {
            "validate" => Some(Step::Validate),
            "generate" => Some(Step::Generate),
            "check" => Some(Step::Check),
            _ => None,
        };
        if let Some(step) = step {
            outcomes.push(Outcome {
                step,
                ok: argument == "ok",
                detail: if argument == "ok" {
                    String::new()
                } else {
                    argument.to_owned()
                },
            });
            continue;
        }
        match verb {
            "profile" => profile = argument.to_owned(),
            "workflow" => workflow = true,
            "json" => json = true,
            "show" => {}
            other => return report(format!("{other:?} is not a headless operation")),
        }
    }
    if json {
        println!("{}", render_json(&outcomes));
    } else {
        for line in crate::app::build::headless::render_human(&outcomes) {
            println!("{line}");
        }
    }
    if workflow {
        print!(
            "{}",
            github_workflow(&profile, "1.98.1", crate::probe::DEPENDENCY_REVISION)
        );
    }
    exit_code(&outcomes) == 0
}

#[cfg(test)]
mod tests {
    use super::{BuildDiagnostics, build_report, run_program, toolchain_with};
    use crate::app::build::toolchain::Environment;
    use crate::cli::{CompileOptions, RunOptions, ToolchainOptions};
    use crate::contracts::identity::NodeId;
    use crate::contracts::revision::Revision;
    use crate::contracts::source_map::SourceSpan;

    fn options(ops: &[&str]) -> ToolchainOptions {
        ToolchainOptions {
            ops: ops.iter().map(|op| (*op).to_owned()).collect(),
        }
    }

    fn run_options(ops: &[&str]) -> RunOptions {
        RunOptions {
            ops: ops.iter().map(|op| (*op).to_owned()).collect(),
        }
    }

    fn intact() -> Environment {
        Environment {
            cargo: Some("cargo 1.98.1".to_owned()),
            rustc: Some("rustc 1.98.1".to_owned()),
            targets: vec!["aarch64-apple-darwin".to_owned()],
            sdk: None,
        }
    }

    #[test]
    fn an_intact_probe_is_ready_for_an_installed_target() {
        assert!(toolchain_with(
            &options(&["profile:desktop", "target:aarch64-apple-darwin", "check"]),
            intact(),
        ));
    }

    #[test]
    fn check_fails_when_the_target_is_not_installed() {
        assert!(!toolchain_with(
            &options(&[
                "profile:desktop",
                "target:riscv64gc-unknown-none-elf",
                "check",
            ]),
            intact(),
        ));
    }

    #[test]
    fn check_fails_when_the_tool_is_absent() {
        assert!(!toolchain_with(
            &options(&["profile:desktop", "check"]),
            Environment::default(),
        ));
    }

    #[test]
    fn a_missing_environment_is_a_report_not_a_failure_without_check() {
        assert!(toolchain_with(
            &options(&["profile:desktop"]),
            Environment::default(),
        ));
    }

    #[test]
    fn an_unsupported_profile_fails_even_without_check() {
        assert!(!toolchain_with(
            &options(&["profile:watch"]),
            Environment::default(),
        ));
    }

    #[test]
    fn cli_overrides_replace_or_extend_the_probe() {
        assert!(toolchain_with(
            &options(&[
                "profile:desktop",
                "cargo:cargo 9.0",
                "rustc:rustc 9.0",
                "installed:thumbv7em-none-eabihf",
                "target:thumbv7em-none-eabihf",
                "sdk:xwin",
                "check",
            ]),
            Environment::default(),
        ));
    }

    #[cfg(unix)]
    #[test]
    fn a_real_exit_is_measured_through_the_cli() {
        assert!(run_program(&run_options(&[
            "program:sh",
            "arg:-c",
            "arg:exit 0",
            "artifact:compiled",
            "start",
            "exit:0",
        ])));
        assert!(run_program(&run_options(&[
            "program:sh",
            "arg:-c",
            "arg:exit 3",
            "artifact:compiled",
            "start",
            "exit:3",
        ])));
        // A claim about the exit that the process contradicts is a failure, not a pass.
        assert!(!run_program(&run_options(&[
            "program:sh",
            "arg:-c",
            "arg:exit 3",
            "artifact:compiled",
            "start",
            "exit:7",
        ])));
    }

    #[cfg(unix)]
    #[test]
    fn a_preview_is_refused_as_a_standalone_artifact_through_the_cli() {
        assert!(!run_program(&run_options(&[
            "program:sh",
            "arg:-c",
            "arg:exit 0",
            "artifact:preview",
            "standalone:yes",
            "start",
        ])));
    }

    #[test]
    fn a_captured_json_stream_becomes_a_build_problem_with_a_node() {
        let source = "line one\nline two\npub fn x() { let a = 1; }\n";
        let diagnostics = BuildDiagnostics {
            source_path: Some("/tmp/project/src/main.rs".to_owned()),
            source_text: Some(source.to_owned()),
            model: Revision::from_raw(3),
            spans: vec![(
                NodeId::from_raw(9),
                SourceSpan::new("src/main.rs", 18, source.len() as u32).unwrap(),
            )],
            ..BuildDiagnostics::default()
        };
        let output = r#"{"reason":"compiler-message","message":{"message":"mismatched types","level":"error","code":{"code":"E0308"},"spans":[{"file_name":"src/main.rs","line_start":3,"line_end":3,"column_start":5,"column_end":9,"is_primary":true}]}}"#;

        let report = build_report(output, &diagnostics);
        assert_eq!(report.diagnostics.len(), 1);
        assert_eq!(report.diagnostics[0].node, Some(NodeId::from_raw(9)));
        assert!(!report.diagnostics[0].unmapped);

        let problems = report.problems("aarch64-apple-darwin");
        assert_eq!(problems[0].node, Some(9));
        assert_eq!(problems[0].revision, 3);
        assert_eq!(problems[0].target, "aarch64-apple-darwin");
        assert_eq!(problems[0].location.as_deref(), Some("src/main.rs:3:5"));
    }

    #[test]
    fn diagnostics_without_execute_have_no_stream_to_read() {
        assert!(!super::compile(&CompileOptions {
            ops: vec!["diag-model:2".to_owned()],
        }));
    }
}
