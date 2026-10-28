// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! F14: the preview, its viewports, hot updates and inspection.
//!
//! One command handler per parsed command of this domain; the dispatcher lives in
//! [`super`](self).

use super::*;

pub(super) fn preview(options: &crate::cli::PreviewOptions) -> bool {
    use rust_widgets::core::{ObjectId, Rect};
    use rust_widgets::widget::{Widget, WidgetFactory, runtime};

    use crate::app::preview::hot_update::Updater;
    use crate::app::preview::preview_service::{Document, DocumentNode, Preview};
    use crate::app::preview::state_keep::Capture;
    use crate::contracts::identity::NodeId;

    /// Builds real, registered controls, so a mounted preview is a live session rather than ids on
    /// paper. The library's factory is headless; it needs no window.
    fn creator(
        factory: &WidgetFactory,
    ) -> impl Fn(&rust_widgets::view::Node) -> Option<ObjectId> + '_ {
        move |node| {
            let widget: Box<dyn Widget> = factory.create(
                &node.widget,
                Rect::new(0, 0, 200, 28),
                node.key_str().unwrap_or("anon"),
            )?;
            runtime::register(widget)
        }
    }

    /// `<node>:<kind>:<text>`.
    fn document_node(text: &str) -> Result<DocumentNode, String> {
        let mut parts = text.splitn(3, ':');
        let node = parts
            .next()
            .and_then(|raw| raw.parse::<u64>().ok())
            .map(NodeId::from_raw)
            .ok_or_else(|| format!("{text:?} needs <node>:<kind>:<text>"))?;
        let kind = parts
            .next()
            .filter(|kind| !kind.is_empty())
            .ok_or_else(|| format!("{text:?} needs <node>:<kind>:<text>"))?;
        let body = parts.next().unwrap_or("");
        Ok(DocumentNode::new(node, kind, body))
    }

    /// `<root>:<node>:<kind>:<text>`.
    fn parse_document(text: &str) -> Result<Document, String> {
        let mut parts = text.splitn(2, ':');
        let root = parts
            .next()
            .filter(|root| !root.is_empty())
            .ok_or_else(|| format!("{text:?} needs <root>:<node>:<kind>:<text>"))?;
        let rest = parts
            .next()
            .ok_or_else(|| format!("{text:?} needs <root>:<node>:<kind>:<text>"))?;
        Ok(Document::new(root).with(document_node(rest)?))
    }

    let mut preview = Preview::new();
    let mut document: Option<Document> = None;
    let mut capture = Capture::new();
    // The classified-edit path is reachable from the same session, so an edit's classification and
    // its effect cannot be demonstrated on two different previews: `edit` records what the designer
    // changed, `apply` flushes the burst into one classified batch and applies it.
    let mut updater = Updater::new("preview");
    let mut edit_source = "editor".to_owned();
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let node = |text: &str| -> Result<NodeId, String> {
            text.parse::<u64>()
                .map(NodeId::from_raw)
                .map_err(|_| format!("{text:?} is not a node id"))
        };
        let step: Result<Vec<String>, String> = match verb {
            "start" => match preview.start(argument) {
                Ok(()) => Ok(vec![format!("started {argument}")]),
                Err(reason) => Err(reason),
            },
            "ready" => match preview.ready() {
                Ok(()) => Ok(vec![format!("state: {}", preview.state.name())]),
                Err(reason) => Err(reason),
            },
            "spawn" => match argument.parse::<u32>() {
                Ok(process) => match preview.spawn(process) {
                    Ok(()) => Ok(vec![format!("spawned {process}")]),
                    Err(reason) => Err(reason),
                },
                Err(_) => Err(format!("{argument:?} is not a process id")),
            },
            "mount" => match parse_document(argument) {
                Ok(next) => {
                    let factory = WidgetFactory::new_with_defaults();
                    let make = creator(&factory);
                    match preview.mount_view(&next, &make) {
                        Ok(report) => {
                            document = Some(next);
                            Ok(vec![format!(
                                "mounted {} control(s), {} issue(s), generation {}",
                                report.widgets_created, report.issues, report.generation
                            )])
                        }
                        Err(reason) => Err(reason),
                    }
                }
                Err(reason) => Err(reason),
            },
            "update" => match document.as_mut() {
                None => Err("no document is mounted, so there is nothing to update".to_owned()),
                Some(current) => match document_node(argument) {
                    Ok(next) => {
                        match current.nodes.iter_mut().find(|held| held.node == next.node) {
                            Some(held) => *held = next,
                            None => current.nodes.push(next),
                        }
                        let factory = WidgetFactory::new_with_defaults();
                        let make = creator(&factory);
                        match preview.update_view(current, &make) {
                            Ok(report) => Ok(vec![format!(
                                "updated: {} patch(es), generation {}, {} update(s)",
                                report.patches, report.generation, report.updates
                            )]),
                            Err(reason) => Err(reason),
                        }
                    }
                    Err(reason) => Err(reason),
                },
            },
            "from" => {
                edit_source = argument.to_owned();
                Ok(vec![format!("edit source: {argument}")])
            }
            "edit" => match argument.split_once(':') {
                Some((kind, subject)) => match edit_from(kind, subject) {
                    Ok(edit) => {
                        let edit_kind = edit.kind();
                        let taken = updater.push(edit_source.clone(), edit);
                        Ok(vec![format!(
                            "edit {subject} -> {}{}",
                            edit_kind.name(),
                            if taken {
                                String::new()
                            } else {
                                " (ignored: our own write)".to_owned()
                            }
                        )])
                    }
                    Err(reason) => Err(reason),
                },
                None => Err(format!(
                    "edit needs <kind>:<node>:<property>=<value>, got {argument:?}"
                )),
            },
            "apply" => match updater.flush() {
                None => Ok(vec!["nothing pending".to_owned()]),
                Some(update) => match document.as_mut() {
                    None => Err("no document is mounted, so no edit can be applied".to_owned()),
                    Some(current) => {
                        let factory = WidgetFactory::new_with_defaults();
                        let make = creator(&factory);
                        match preview.apply_update(&update, current, &make) {
                            Ok(decision) => Ok(vec![decision.render()]),
                            Err(reason) => Err(reason),
                        }
                    }
                },
            },
            "edits" => Ok(vec![
                format!("pending: {}", updater.pending()),
                format!("updates: {}", updater.history().len()),
                format!("edit source: {edit_source}"),
            ]),
            "capture" => {
                capture = preview.capture_state();
                let lines = capture.render();
                if lines.is_empty() {
                    Ok(vec!["nothing focused".to_owned()])
                } else {
                    Ok(lines)
                }
            }
            "restore" => match preview.restore_state(&capture) {
                Ok(outcomes) => Ok(outcomes.iter().map(|outcome| outcome.render()).collect()),
                Err(reason) => Err(reason),
            },
            "gap" => Ok(vec![Preview::scroll_gap().to_owned()]),
            "install" => match node(argument) {
                Ok(node) => match preview.install(node) {
                    Ok(instance) => Ok(vec![format!(
                        "node {} (generation {})",
                        instance.node.get(),
                        instance.generation
                    )]),
                    Err(reason) => Err(reason),
                },
                Err(reason) => Err(reason),
            },
            "rebuild" => match node(argument) {
                Ok(node) => match preview.rebuild(node) {
                    Ok(instance) => Ok(vec![format!(
                        "node {} (generation {})",
                        instance.node.get(),
                        instance.generation
                    )]),
                    Err(reason) => Err(reason),
                },
                Err(reason) => Err(reason),
            },
            "stop" => match preview.stop() {
                Ok(released) => Ok(vec![released.render()]),
                Err(reason) => Err(reason),
            },
            "finish-stop" => match preview.finish_stop() {
                Ok(()) => Ok(vec![format!("state: {}", preview.state.name())]),
                Err(reason) => Err(reason),
            },
            "clean" => Ok(vec![format!(
                "clean: {}",
                if preview.is_clean() { "yes" } else { "no" }
            )]),
            "state" => {
                let mut lines = preview.render();
                lines.push(format!(
                    "document: {}",
                    document
                        .as_ref()
                        .map(|document| {
                            format!("{} with {} node(s)", document.root, document.nodes.len())
                        })
                        .unwrap_or_else(|| "(none)".to_owned())
                ));
                Ok(lines)
            }
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a preview operation")),
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

/// One edit from `<kind>:<subject>`, as both the preview and hot-update commands spell it.
///
/// The subject is passed through verbatim: a subject that does not address a document property is
/// a fact the live-session path reports, not something this parser guesses at.
fn edit_from(kind: &str, subject: &str) -> Result<crate::app::preview::hot_update::Edit, String> {
    use crate::app::preview::hot_update::Edit;

    Ok(match kind {
        "property" => Edit::Property(subject.to_owned()),
        "style" => Edit::Style(subject.to_owned()),
        "existing" => Edit::ExistingBinding(subject.to_owned()),
        "add-binding" => Edit::AddBinding(subject.to_owned()),
        "remove-binding" => Edit::RemoveBinding(subject.to_owned()),
        "add-node" => Edit::AddNode(subject.to_owned()),
        "remove-node" => Edit::RemoveNode(subject.to_owned()),
        "text" => Edit::Text(subject.to_owned()),
        other => return Err(format!("{other:?} is not an edit kind")),
    })
}

pub(super) fn hot_update(options: &crate::cli::HotUpdateOptions) -> bool {
    use crate::app::preview::hot_update::Updater;

    let mut updater = Updater::new("preview");
    let mut source = "editor".to_owned();
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "self" => {
                updater.self_source = argument.to_owned();
                Ok(vec![format!("self source: {argument}")])
            }
            "from" => {
                source = argument.to_owned();
                Ok(vec![format!("from: {argument}")])
            }
            "property" | "style" | "existing" | "add-binding" | "remove-binding" | "add-node"
            | "remove-node" | "text" => match edit_from(verb, argument) {
                Ok(edit) => {
                    let kind = edit.kind();
                    let taken = updater.push(source.clone(), edit);
                    Ok(vec![format!(
                        "{} {argument} -> {}{}",
                        verb,
                        kind.name(),
                        if taken {
                            String::new()
                        } else {
                            " (ignored: our own write)".to_owned()
                        }
                    )])
                }
                Err(reason) => Err(reason),
            },
            "flush" => match updater.flush() {
                Some(update) => Ok(vec![update.render()]),
                None => Ok(vec!["nothing pending".to_owned()]),
            },
            "check" | "state" => Ok(vec![
                format!("pending: {}", updater.pending()),
                format!("updates: {}", updater.history().len()),
                format!("self source: {}", updater.self_source),
            ]),
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a hot-update operation")),
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

pub(super) fn state_keep(options: &crate::cli::StateKeepOptions) -> bool {
    use crate::app::preview::state_keep::{Capture, Kept, Present, restore};
    use crate::contracts::identity::NodeId;

    let mut capture = Capture::new();
    let mut present: Vec<Present> = Vec::new();
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let node_of = |text: &str| -> Result<NodeId, String> {
            text.parse::<u64>()
                .map(NodeId::from_raw)
                .map_err(|_| format!("{text:?} is not a node id"))
        };
        let step: Result<Vec<String>, String> = match verb {
            "keep" => match argument.split_once(':') {
                Some((node, kind)) => match node_of(node) {
                    Ok(node) => {
                        let line = format!("captured node {} ({kind})", node.get());
                        capture.entries.push(Kept {
                            node,
                            kind: kind.to_owned(),
                            focus: false,
                            input: String::new(),
                            scroll: 0,
                            selection: None,
                        });
                        Ok(vec![line])
                    }
                    Err(reason) => Err(reason),
                },
                None => Err(format!("keep needs <node>:<kind>, got {argument:?}")),
            },
            "focus" => match node_of(argument) {
                Ok(node) => match capture.entries.iter_mut().find(|kept| kept.node == node) {
                    Some(kept) => {
                        kept.focus = true;
                        Ok(vec![format!("focus node {}", node.get())])
                    }
                    None => Err(format!("node {} was not captured", node.get())),
                },
                Err(reason) => Err(reason),
            },
            "scroll" => match argument.split_once(':') {
                Some((node, amount)) => {
                    let amount = match amount.parse::<u32>() {
                        Ok(amount) => amount,
                        Err(_) => return report(format!("{amount:?} is not a number")),
                    };
                    let node = match node_of(node) {
                        Ok(node) => node,
                        Err(reason) => return report(reason),
                    };
                    match capture.entries.iter_mut().find(|kept| kept.node == node) {
                        Some(kept) => {
                            kept.scroll = amount;
                            Ok(vec![format!("scroll node {}: {amount}", node.get())])
                        }
                        None => Err(format!("node {} was not captured", node.get())),
                    }
                }
                None => Err(format!("scroll needs <node>:<n>, got {argument:?}")),
            },
            "input" => match argument.split_once(':') {
                Some((node, text)) => match node_of(node) {
                    Ok(node) => match capture.entries.iter_mut().find(|kept| kept.node == node) {
                        Some(kept) => {
                            kept.input = text.to_owned();
                            Ok(vec![format!("input node {}: {text}", node.get())])
                        }
                        None => Err(format!("node {} was not captured", node.get())),
                    },
                    Err(reason) => Err(reason),
                },
                None => Err(format!("input needs <node>:<text>, got {argument:?}")),
            },
            "selection" => match argument.split_once(':') {
                Some((node, rest)) => match rest.split_once(':') {
                    Some((start, end)) => {
                        let start = match start.parse::<usize>() {
                            Ok(value) => value,
                            Err(_) => return report(format!("{start:?} is not a number")),
                        };
                        let end = match end.parse::<usize>() {
                            Ok(value) => value,
                            Err(_) => return report(format!("{end:?} is not a number")),
                        };
                        let node = match node_of(node) {
                            Ok(node) => node,
                            Err(reason) => return report(reason),
                        };
                        match capture.entries.iter_mut().find(|kept| kept.node == node) {
                            Some(kept) => {
                                kept.selection = Some((start, end));
                                Ok(vec![format!(
                                    "selection node {}: {start}..{end}",
                                    node.get()
                                )])
                            }
                            None => Err(format!("node {} was not captured", node.get())),
                        }
                    }
                    None => Err(format!(
                        "selection needs <node>:<start>:<end>, got {argument:?}"
                    )),
                },
                None => Err(format!(
                    "selection needs <node>:<start>:<end>, got {argument:?}"
                )),
            },
            "present" => match argument.split_once(':') {
                Some((node, kind)) => match node_of(node) {
                    Ok(node) => {
                        present.push(Present::new(node, kind));
                        Ok(vec![format!("present node {} ({kind})", node.get())])
                    }
                    Err(reason) => Err(reason),
                },
                None => Err(format!("present needs <node>:<kind>, got {argument:?}")),
            },
            "restore" => Ok(restore(&capture, &present)
                .iter()
                .map(|outcome| outcome.render())
                .collect()),
            "check" | "state" => {
                let mut lines = capture.render();
                lines.push(format!("present: {}", present.len()));
                Ok(lines)
            }
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a state-keep operation")),
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

pub(super) fn matrix(options: &crate::cli::MatrixOptions) -> bool {
    use crate::app::preview::viewport_matrix::{Matrix, Orientation, Viewport};

    let mut matrix = Matrix::new("(none)");
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "document" => {
                matrix = Matrix::new(argument);
                Ok(vec![format!("document: {argument}")])
            }
            "viewport" => match argument.split_once(':') {
                Some((name, rest)) => match rest.split_once(':') {
                    Some((size, orientation)) => {
                        let Some((width, height)) = size.split_once('x') else {
                            return report(format!("{size:?} is not <w>x<h>"));
                        };
                        let width = match width.parse::<u32>() {
                            Ok(width) => width,
                            Err(_) => return report(format!("{width:?} is not a width")),
                        };
                        let height = match height.parse::<u32>() {
                            Ok(height) => height,
                            Err(_) => return report(format!("{height:?} is not a height")),
                        };
                        let Some(orientation) = Orientation::parse(orientation) else {
                            return report(format!("{orientation:?} is not portrait or landscape"));
                        };
                        let viewport = Viewport::new(name, (width, height), orientation);
                        let line = viewport.render();
                        match matrix.push(viewport) {
                            Ok(()) => Ok(vec![line]),
                            Err(reason) => Err(reason),
                        }
                    }
                    None => Err(format!(
                        "viewport needs <name>:<w>x<h>:<orientation>, got {argument:?}"
                    )),
                },
                None => Err(format!(
                    "viewport needs <name>:<w>x<h>:<orientation>, got {argument:?}"
                )),
            },
            "theme" | "language" | "sample" => match argument.split_once(':') {
                Some((name, value)) => match matrix.viewport_mut(name) {
                    Some(viewport) => {
                        match verb {
                            "theme" => viewport.theme = Some(value.to_owned()),
                            "language" => viewport.language = value.to_owned(),
                            _ => viewport.sample = Some(value.to_owned()),
                        }
                        Ok(vec![format!("{verb} {name}: {value}")])
                    }
                    None => Err(format!("no viewport named {name:?}")),
                },
                None => Err(format!("{verb} needs <name>:<value>, got {argument:?}")),
            },
            "commit" => {
                matrix.commit();
                Ok(vec![format!("revision: {}", matrix.revision())])
            }
            "start" => match matrix.start(argument) {
                Ok(()) => Ok(vec![format!("started {argument}")]),
                Err(reason) => Err(reason),
            },
            "stop" => match matrix.stop(argument) {
                Ok(()) => Ok(vec![format!("stopped {argument}")]),
                Err(reason) => Err(reason),
            },
            "restart" => match matrix.restart(argument) {
                Ok(restarts) => Ok(vec![format!("{argument}: {restarts} restart(s)")]),
                Err(reason) => Err(reason),
            },
            "remove" => match matrix.remove(argument) {
                Ok(removed) => Ok(vec![format!("removed {}", removed.name)]),
                Err(reason) => Err(reason),
            },
            "check" | "state" => Ok(matrix.render()),
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a matrix operation")),
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

/// A node identity from a decimal argument.
fn inspect_node(argument: &str) -> Result<crate::contracts::identity::NodeId, String> {
    argument
        .parse::<u64>()
        .map(crate::contracts::identity::NodeId::from_raw)
        .map_err(|_| format!("{argument:?} is not a node id"))
}

/// A version from a decimal argument.
fn inspect_version(argument: &str) -> Result<u32, String> {
    argument
        .parse::<u32>()
        .map_err(|_| format!("{argument:?} is not a version"))
}

pub(super) fn inspect(options: &crate::cli::InspectOptions) -> bool {
    use crate::app::preview::inspection::{Adapter, Inspection, Mode, Source};

    let mut inspection = Inspection::new();
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "mode" => match Mode::parse(argument) {
                Some(mode) => {
                    inspection.set_mode(mode);
                    Ok(vec![format!("mode: {}", mode.name())])
                }
                None => Err(format!("{argument:?} is not design or interact")),
            },
            "adapter" => match Adapter::parse(argument) {
                Some(adapter) => {
                    inspection.set_adapter(adapter);
                    Ok(vec![format!("adapter: {}", adapter.name())])
                }
                None => Err(format!("{argument:?} is not real, sample or absent")),
            },
            "edit" => match inspect_version(argument) {
                Ok(version) => {
                    inspection.set_edit_version(version);
                    Ok(vec![format!("edit version: {version}")])
                }
                Err(reason) => Err(reason),
            },
            "instance" => match argument.split_once(':') {
                Some((name, version)) => match inspect_version(version) {
                    Ok(version) => {
                        inspection.start_instance(name, version);
                        Ok(vec![format!("instance: {name} (v {version})")])
                    }
                    Err(reason) => Err(reason),
                },
                None => Err(format!("instance needs <name>:<version>, got {argument:?}")),
            },
            "stop" => {
                inspection.stop_instance();
                Ok(vec!["instance: stopped".to_owned()])
            }
            "kind" => match argument.split_once(':') {
                Some((node, kind)) => match inspect_node(node) {
                    Ok(node) => {
                        inspection.kind(node, kind);
                        Ok(vec![format!("node {}: {kind}", node.get())])
                    }
                    Err(reason) => Err(reason),
                },
                None => Err(format!("kind needs <node>:<kind>, got {argument:?}")),
            },
            "design" | "computed" => match argument.splitn(3, ':').collect::<Vec<_>>().as_slice() {
                [node, property, text] => match inspect_node(node) {
                    Ok(node) => {
                        if verb == "design" {
                            inspection.declare_design(node, property, *text);
                        } else {
                            inspection.declare_computed(node, property, *text);
                        }
                        Ok(vec![format!(
                            "{verb} node {} {property}: {text}",
                            node.get()
                        )])
                    }
                    Err(reason) => Err(reason),
                },
                _ => Err(format!(
                    "{verb} needs <node>:<property>:<text>, got {argument:?}"
                )),
            },
            "runtime" => match argument.splitn(4, ':').collect::<Vec<_>>().as_slice() {
                [node, property, text, version] => match inspect_node(node) {
                    Ok(node) => match inspect_version(version) {
                        Ok(version) => {
                            match inspection.declare_runtime(node, property, *text, version) {
                                Ok(()) => Ok(vec![format!(
                                    "runtime node {} {property}: {text} (v {version})",
                                    node.get()
                                )]),
                                Err(reason) => Err(reason),
                            }
                        }
                        Err(reason) => Err(reason),
                    },
                    Err(reason) => Err(reason),
                },
                _ => Err(format!(
                    "runtime needs <node>:<property>:<text>:<version>, got {argument:?}"
                )),
            },
            "source" => match argument.splitn(4, ':').collect::<Vec<_>>().as_slice() {
                [node, file, line, column] => match inspect_node(node) {
                    Ok(node) => match (line.parse::<u32>(), column.parse::<u32>()) {
                        (Ok(line), Ok(column)) => {
                            inspection.source(node, Source::new(*file, line, column));
                            Ok(vec![format!(
                                "source node {}: {file}:{line}:{column}",
                                node.get()
                            )])
                        }
                        _ => Err(format!("{line:?}:{column:?} is not <line>:<column>")),
                    },
                    Err(reason) => Err(reason),
                },
                _ => Err(format!(
                    "source needs <node>:<file>:<line>:<column>, got {argument:?}"
                )),
            },
            "child" => match argument.split_once(':') {
                Some((node, parent)) => match (inspect_node(node), inspect_node(parent)) {
                    (Ok(node), Ok(parent)) => match inspection.child(node, parent) {
                        Ok(()) => Ok(vec![format!(
                            "child: node {} under {}",
                            node.get(),
                            parent.get()
                        )]),
                        Err(reason) => Err(reason),
                    },
                    (Err(reason), _) | (_, Err(reason)) => Err(reason),
                },
                None => Err(format!("child needs <node>:<parent>, got {argument:?}")),
            },
            "event" => match argument.split_once(':') {
                Some((name, payload)) => {
                    inspection.event(name, unescape(payload));
                    Ok(vec![format!("event {name}: {payload}")])
                }
                None => Err(format!("event needs <name>:<payload>, got {argument:?}")),
            },
            "data" => match argument.split_once(':') {
                Some((field, value)) => {
                    inspection.data(field, value);
                    Ok(vec![format!("data {field}={value}")])
                }
                None => Err(format!("data needs <field>:<value>, got {argument:?}")),
            },
            "values" => match argument.split_once(':') {
                Some((node, property)) => match inspect_node(node) {
                    Ok(node) => inspection.values(node, property),
                    Err(reason) => Err(reason),
                },
                None => Err(format!("values needs <node>:<property>, got {argument:?}")),
            },
            "locate" => match inspect_node(argument) {
                Ok(node) => inspection.locate(node),
                Err(reason) => Err(reason),
            },
            "trigger" => match inspection.trigger(argument) {
                Ok(line) => Ok(vec![line]),
                Err(reason) => Err(reason),
            },
            "check" | "state" => Ok(inspection.render()),
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not an inspection operation")),
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

/// A greyscale image from `<w>x<h>` and its digits.
fn visual_image(
    size: &str,
    digits: &str,
) -> Result<crate::app::preview::visual_baseline::Image, String> {
    use crate::app::preview::visual_baseline::Image;

    let Some((width, height)) = size.split_once('x') else {
        return Err(format!("{size:?} is not <w>x<h>"));
    };
    let width = width
        .parse::<u32>()
        .map_err(|_| format!("{width:?} is not a width"))?;
    let height = height
        .parse::<u32>()
        .map_err(|_| format!("{height:?} is not a height"))?;
    Image::from_digits(width, height, digits)
}

pub(super) fn visual(options: &crate::cli::VisualOptions) -> bool {
    use crate::app::preview::visual_baseline::{Baseline, Conditions, Shot, Tolerance, compare};

    let mut baseline = Baseline::new();
    let mut conditions = Conditions::new("unknown", 0);
    let mut tolerance = Tolerance::exact();
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "platform" => {
                conditions.platform = argument.to_owned();
                Ok(vec![format!("platform: {argument}")])
            }
            "dpi" => match argument.parse::<u32>() {
                Ok(dpi) => {
                    conditions.dpi = dpi;
                    Ok(vec![format!("dpi: {dpi}")])
                }
                Err(_) => Err(format!("{argument:?} is not a dpi")),
            },
            "resources" => match argument.parse::<u32>() {
                Ok(resources) => {
                    conditions.resources = resources;
                    Ok(vec![format!("resources: {resources}")])
                }
                Err(_) => Err(format!("{argument:?} is not a resource version")),
            },
            "fonts" => {
                conditions.fonts = argument.to_owned();
                Ok(vec![format!("fonts: {argument}")])
            }
            "tolerance" => match argument.split_once(':') {
                Some((percent, channel)) => match (percent.parse::<f64>(), channel.parse::<u8>()) {
                    (Ok(percent), Ok(channel)) if (0.0..=100.0).contains(&percent) => {
                        tolerance = Tolerance::percentage(percent, channel);
                        Ok(vec![format!("tolerance: {}", tolerance.render())])
                    }
                    _ => Err(format!("tolerance needs <0-100>:<0-15>, got {argument:?}")),
                },
                None => Err(format!(
                    "tolerance needs <percent>:<channel-step>, got {argument:?}"
                )),
            },
            "record" | "shot" => match argument.splitn(3, ':').collect::<Vec<_>>().as_slice() {
                [name, size, digits] => match visual_image(size, digits) {
                    Ok(image) => {
                        let shot = Shot::new(*name, conditions.clone(), image);
                        if verb == "record" {
                            let line = shot.render();
                            match baseline.record(shot) {
                                Ok(()) => Ok(vec![format!("recorded {line}")]),
                                Err(reason) => Err(reason),
                            }
                        } else {
                            match baseline.get(name) {
                                Some(base) => Ok(compare(base, &shot, tolerance).render()),
                                None => Err(format!("no baseline named {name:?}")),
                            }
                        }
                    }
                    Err(reason) => Err(reason),
                },
                _ => Err(format!(
                    "{verb} needs <name>:<w>x<h>:<digits>, got {argument:?}"
                )),
            },
            "drop" => match baseline.drop_shot(argument) {
                Ok(()) => Ok(vec![format!("dropped {argument}")]),
                Err(reason) => Err(reason),
            },
            "baseline" | "shots" | "check" | "state" => Ok(baseline.render()),
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a visual comparison operation")),
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

pub(super) fn preview_recovery(options: &crate::cli::PreviewRecoveryOptions) -> bool {
    use crate::app::preview::preview_recovery::Preview;

    let mut preview = Preview::new();
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "edit" => match argument.parse::<u32>() {
                Ok(version) => {
                    preview.edit(version);
                    Ok(vec![format!("edit version: {version}")])
                }
                Err(_) => Err(format!("{argument:?} is not a version")),
            },
            "fault" => {
                preview.declare_fault(unescape(argument));
                Ok(vec![format!("fault: {argument}")])
            }
            "clear" => {
                preview.clear_fault();
                Ok(vec!["fault: cleared".to_owned()])
            }
            "exit" => match argument.parse::<i32>() {
                Ok(code) => {
                    preview.exit(code);
                    Ok(vec![format!("the app exited with code {code}")])
                }
                Err(_) => Err(format!("{argument:?} is not an exit code")),
            },
            "crash" => match argument.parse::<i32>() {
                Ok(signal) => {
                    preview.crash(signal);
                    Ok(vec![format!("the app crashed on signal {signal}")])
                }
                Err(_) => Err(format!("{argument:?} is not a signal")),
            },
            "launch" | "reload" | "rebuild" | "reset" => {
                let recovery = match verb {
                    "launch" => preview.launch(),
                    "rebuild" => preview.rebuild(),
                    "reset" => preview.reset(),
                    _ => preview.reload(),
                };
                let succeeded = recovery.succeeded();
                let reason = recovery.reason();
                for line in recovery.render() {
                    println!("{line}");
                }
                if succeeded {
                    Ok(Vec::new())
                } else {
                    Err(reason)
                }
            }
            "check" | "state" => Ok(preview.render()),
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a recovery operation")),
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

pub(super) fn load(options: &crate::cli::LoadOptions) -> bool {
    use crate::app::preview::load::Load;
    use crate::app::preview::preview_service::{Document, DocumentNode};
    use crate::contracts::identity::NodeId;

    let mut load = Load::new();
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "budget" => match argument.parse::<u32>() {
                Ok(budget) => {
                    load = load.with_budget(budget);
                    Ok(vec![format!("budget: {budget} ms")])
                }
                Err(_) => Err(format!("{argument:?} is not a millisecond budget")),
            },
            // `cycle`, `repeat` and `leak` carry counts the caller declares. They model a driver so
            // the checker can be exercised; they are not measurements, and a run made only of them
            // cannot pass. `drive` is the run that reads a real session.
            "cycle" => match argument.splitn(4, ':').collect::<Vec<_>>().as_slice() {
                [viewport, controls, nodes] => match load_counts(controls, nodes, None) {
                    Ok((controls, nodes, latency)) => {
                        load.declared_cycle(viewport, controls, nodes, latency);
                        Ok(vec![format!(
                            "declared cycle {viewport}: {controls} control(s) (not measured)"
                        )])
                    }
                    Err(reason) => Err(reason),
                },
                [viewport, controls, nodes, latency] => {
                    match load_counts(controls, nodes, Some(latency)) {
                        Ok((controls, nodes, latency)) => {
                            load.declared_cycle(viewport, controls, nodes, latency);
                            Ok(vec![format!(
                                "declared cycle {viewport}: {controls} control(s) (not measured)"
                            )])
                        }
                        Err(reason) => Err(reason),
                    }
                }
                _ => Err(format!(
                    "cycle needs <viewport>:<controls>:<nodes>[:<latency>], got {argument:?}"
                )),
            },
            "repeat" => match argument.splitn(5, ':').collect::<Vec<_>>().as_slice() {
                [viewport, cycles, controls, nodes] => match load_cycles(cycles) {
                    Ok(cycles) => match load_counts(controls, nodes, None) {
                        Ok((controls, nodes, latency)) => {
                            load.repeat(viewport, cycles, controls, nodes, latency);
                            Ok(vec![format!(
                                "declared {cycles} cycle(s) in {viewport} (not measured)"
                            )])
                        }
                        Err(reason) => Err(reason),
                    },
                    Err(reason) => Err(reason),
                },
                [viewport, cycles, controls, nodes, latency] => match load_cycles(cycles) {
                    Ok(cycles) => match load_counts(controls, nodes, Some(latency)) {
                        Ok((controls, nodes, latency)) => {
                            load.repeat(viewport, cycles, controls, nodes, latency);
                            Ok(vec![format!(
                                "declared {cycles} cycle(s) in {viewport} (not measured)"
                            )])
                        }
                        Err(reason) => Err(reason),
                    },
                    Err(reason) => Err(reason),
                },
                _ => Err(format!(
                    "repeat needs <viewport>:<cycles>:<controls>:<nodes>[:<latency>], got \
                     {argument:?}"
                )),
            },
            "leak" => match argument.splitn(4, ':').collect::<Vec<_>>().as_slice() {
                [viewport, cycles, controls, nodes] => match load_cycles(cycles) {
                    Ok(cycles) => match (controls.parse::<u32>(), nodes.parse::<u32>()) {
                        (Ok(controls), Ok(nodes)) => {
                            load.leak(viewport, cycles, controls, nodes);
                            Ok(vec![format!(
                                "declared leak of {cycles} cycle(s) in {viewport}"
                            )])
                        }
                        _ => Err(format!("{controls:?}:{nodes:?} is not <controls>:<nodes>")),
                    },
                    Err(reason) => Err(reason),
                },
                _ => Err(format!(
                    "leak needs <viewport>:<cycles>:<controls>:<nodes>, got {argument:?}"
                )),
            },
            "drive" => match argument.splitn(6, ':').collect::<Vec<_>>().as_slice() {
                [viewport, cycles, root, node, kind, text] => {
                    match (load_cycles(cycles), node.parse::<u64>()) {
                        (Ok(cycles), Ok(node)) if !root.is_empty() && !kind.is_empty() => {
                            let document = Document::new(*root).with(DocumentNode::new(
                                NodeId::from_raw(node),
                                *kind,
                                *text,
                            ));
                            match load.drive(viewport, cycles, &document) {
                                Ok(()) => {
                                    Ok(vec![format!("drove {cycles} real cycle(s) in {viewport}")])
                                }
                                Err(reason) => Err(reason),
                            }
                        }
                        (Err(reason), _) => Err(reason),
                        (_, Err(_)) => Err(format!("{node:?} is not a node id")),
                        _ => Err(format!(
                            "drive needs <viewport>:<cycles>:<root>:<node>:<kind>:<text>, got \
                         {argument:?}"
                        )),
                    }
                }
                _ => Err(format!(
                    "drive needs <viewport>:<cycles>:<root>:<node>:<kind>:<text>, got \
                     {argument:?}"
                )),
            },
            "invalid" => {
                load.invalid(unescape(argument));
                Ok(vec![format!("invalid input kept: {argument}")])
            }
            "check" | "state" => Ok(load.render()),
            "verdict" => {
                if load.passed() {
                    Ok(vec!["passed".to_owned()])
                } else {
                    for line in load.render() {
                        println!("{line}");
                    }
                    Err(load.verdict())
                }
            }
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a load operation")),
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

/// A span of cycles from a decimal argument.
fn load_cycles(argument: &str) -> Result<u32, String> {
    argument
        .parse::<u32>()
        .map_err(|_| format!("{argument:?} is not a cycle count"))
}

/// A cycle's counts, and the latency the driver observed if it gave one.
fn load_counts(
    controls: &str,
    nodes: &str,
    latency: Option<&str>,
) -> Result<(u32, u32, Option<u32>), String> {
    let controls = controls
        .parse::<u32>()
        .map_err(|_| format!("{controls:?} is not a control count"))?;
    let nodes = nodes
        .parse::<u32>()
        .map_err(|_| format!("{nodes:?} is not a node count"))?;
    let latency = match latency {
        None => None,
        Some(latency) => Some(
            latency
                .parse::<u32>()
                .map_err(|_| format!("{latency:?} is not a millisecond latency"))?,
        ),
    };
    Ok((controls, nodes, latency))
}
