// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! F16: the build, its problems, the run, terminals, tasks, external tools, the end-to-end walk, the
//! task graph, reproducibility and the headless walk.
//!
//! One command handler per parsed command of this domain; the dispatcher lives in [`super`](self).
//! Where a command can run a real program (`--compile execute`), it does; where it cannot, it prints
//! the exact command it would run rather than a success shape.

use super::*;
use std::process::Command as ProcessCommand;

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
pub(super) fn toolchain(options: &crate::cli::ToolchainOptions) -> bool {
    use crate::app::build::toolchain::{Environment, Finding, diagnose, selection};

    let mut environment = Environment::default();
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
    let findings = diagnose(&chosen, triple.as_deref(), &environment);
    for finding in &findings {
        println!("{}", finding.render());
    }
    if check && findings.is_empty() {
        println!("toolchain: ready");
    }
    findings
        .iter()
        .all(|finding| !matches!(finding, Finding::Unsupported { .. }))
}

/// Checks, builds or rebuilds (task T16.02).
pub(super) fn compile(options: &crate::cli::CompileOptions) -> bool {
    use crate::app::build::compile::{Action, Job, Outcome};
    use crate::app::build::toolchain::Selection;

    let mut action = Action::Check;
    let mut selection = Selection {
        profile: "desktop".to_owned(),
        extra: Vec::new(),
    };
    let mut job_target: Option<String> = None;
    let mut package: Option<String> = None;
    let mut manifest: Option<String> = None;
    let mut exit: Option<Option<i32>> = None;
    let mut execute = false;
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
            "exit" => match argument.parse::<i32>() {
                Ok(code) => {
                    exit = Some(Some(code));
                    Ok(())
                }
                Err(_) => Err(format!("{argument:?} is not an exit code")),
            },
            "cancel" => {
                exit = Some(None);
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
    let job = Job {
        action,
        selection,
        target: job_target,
        package,
        manifest,
    };
    if let Some(clean) = job.clean_command() {
        println!("clean: cargo {}", clean.join(" "));
    }
    println!("cargo {}", job.command().join(" "));
    if execute {
        // The real command, with the real exit code: this is what turns "it built" from a claim into
        // a measurement (principle 25). Nothing here uses a shell, so the argv is the one printed.
        let outcome = ProcessCommand::new("cargo").args(job.command()).output();
        match outcome {
            Ok(output) => {
                let code = output.status.code();
                let outcome = Outcome::from_exit(
                    job,
                    code,
                    String::from_utf8_lossy(&output.stdout).into_owned(),
                    String::from_utf8_lossy(&output.stderr).into_owned(),
                );
                for line in outcome.render() {
                    println!("{line}");
                }
                return outcome.verdict.is_success();
            }
            Err(error) => return report(format!("cargo could not be started: {error}")),
        }
    }
    match exit {
        Some(code) => {
            let outcome = Outcome::from_exit(job, code, String::new(), String::new());
            for line in outcome.render() {
                println!("{line}");
            }
            outcome.verdict.is_success()
        }
        None => true,
    }
}

/// Reads a build's problems and links them to nodes (task T16.03).
pub(super) fn build_diag(options: &crate::cli::BuildDiagOptions) -> bool {
    use crate::app::build::build_diagnostics::{locate, parse};
    use crate::contracts::identity::NodeId;
    use crate::contracts::revision::{GeneratorVersion, Revision};
    use crate::contracts::source_map::{SourceMap, SourceSpan};

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
    for line in batch.render() {
        println!("{line}");
    }
    for message in &batch.messages {
        if let Some(node) = locate(message, &source, &map) {
            println!("  -> node {} in {file}", node.get());
        }
    }
    if let Some(current) = current {
        println!("{}", batch.staleness(current));
    }
    batch.errors() == 0
}

/// Runs the delivered program on its own (task T16.04).
pub(super) fn run_program(options: &crate::cli::RunOptions) -> bool {
    use crate::app::build::runner::{Config, Reason, Run, State};

    let mut config = Config {
        program: String::new(),
        args: Vec::new(),
        environment: Vec::new(),
        directory: None,
        standalone: false,
    };
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
            "start" => {
                run = Some(Run::new(config.clone()));
                match run.as_mut().expect("set above").start() {
                    Ok(()) => Ok(vec![format!("started {}", config.render())]),
                    Err(reason) => Err(reason),
                }
            }
            "exit" => match argument.parse::<i32>() {
                Ok(code) => match run.as_mut() {
                    Some(run) => {
                        run.ended(Reason::NonZero { code });
                        Ok(vec![format!(
                            "ended: {}",
                            run.reason.clone().unwrap().render()
                        )])
                    }
                    None => Err("no run has started".to_owned()),
                },
                Err(_) => Err(format!("{argument:?} is not an exit code")),
            },
            "signal" => match argument.parse::<i32>() {
                Ok(signal) => match run.as_mut() {
                    Some(run) => {
                        run.ended(Reason::Signalled { signal });
                        Ok(vec![format!(
                            "ended: {}",
                            run.reason.clone().unwrap().render()
                        )])
                    }
                    None => Err("no run has started".to_owned()),
                },
                Err(_) => Err(format!("{argument:?} is not a signal")),
            },
            "stop" => match run.as_mut() {
                Some(run) => match run.stop() {
                    Ok(()) => Ok(vec![run.reason.clone().unwrap().render().to_string()]),
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
            Err(reason) => return report(reason),
        }
    }
    if let Some(run) = &run {
        println!(
            "state: {}",
            match run.state {
                State::Idle => "idle",
                State::Running => "running",
                State::Ended => "ended",
            }
        );
    }
    true
}

/// Orders configured tasks (task T16.05).
pub(super) fn tasks(options: &crate::cli::TasksOptions) -> bool {
    use crate::app::build::tasks::{Task, cache_key, cleanup, default_task, order};

    let mut tasks: Vec<Task> = Vec::new();
    let mut fingerprints: Vec<(String, String)> = Vec::new();
    let mut declared_default: Option<String> = None;
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<(), String> = match verb {
            "task" => match argument.split_once(':') {
                Some((name, command)) => {
                    tasks.push(Task {
                        name: name.to_owned(),
                        command: vec![command.to_owned()],
                        depends_on: Vec::new(),
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
    let default = match default_task(&tasks, declared_default.as_deref()) {
        Ok(default) => default,
        Err(reason) => return report(reason),
    };
    for line in crate::app::build::tasks::render(&tasks, &ordered, &default) {
        println!("{line}");
    }
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
                        Ok(()) => Ok(vec![format!("size {columns}x{rows}")]),
                        Err(reason) => Err(reason),
                    },
                    _ => Err(format!("{argument:?} is not <columns>x<rows>")),
                },
                None => Err(format!("size needs <columns>x<rows>, got {argument:?}")),
            },
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
            "start" => match session.start() {
                Ok(()) => Ok(vec!["started".to_owned()]),
                Err(reason) => Err(reason),
            },
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

/// Builds a typed task graph and matches its output (task T16.09).
pub(super) fn dag(options: &crate::cli::DagOptions) -> bool {
    use crate::app::build::dag::{Dag, Node, Order, Ready, match_line, matcher};

    let mut dag = Dag::default();
    let mut matchers = Vec::new();
    let mut ready: Vec<Ready> = Vec::new();
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
                        found = true;
                    }
                }
                if !found {
                    println!("no matcher read {argument:?}");
                }
                Ok(())
            }
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
            github_workflow(
                &profile,
                "1.98.1",
                "8178d04865731ad06d11d6d15129d889bf0ae327"
            )
        );
    }
    exit_code(&outcomes) == 0
}
