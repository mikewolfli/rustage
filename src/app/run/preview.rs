// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! F14: the preview, its viewports, hot updates and inspection.
//!
//! One command handler per parsed command of this domain; the dispatcher lives in
//! [`super`](self).

use super::*;

use rust_widgets::core::{ObjectId, Rect, Size};
use rust_widgets::view::Breakpoint;
use rust_widgets::widget::{Widget, WidgetFactory, runtime};

/// Builds real, registered controls, so a mounted preview is a live session rather than ids on
/// paper. The library's factory is headless; it needs no window.
pub(super) fn control_creator(
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

pub(super) fn preview(options: &crate::cli::PreviewOptions) -> bool {
    use crate::app::preview::hot_update::Updater;
    use crate::app::preview::preview_service::{Document, DocumentNode, Preview};
    use crate::app::preview::state_keep::Capture;
    use crate::contracts::identity::NodeId;

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
                    let make = control_creator(&factory);
                    match preview.mount_view(&next, &make) {
                        Ok(report) => {
                            document = Some(next);
                            let mut lines = vec![format!(
                                "mounted {} control(s), {} issue(s), generation {}",
                                report.widgets_created, report.issues, report.generation
                            )];
                            lines.extend(
                                preview
                                    .unwired_events()
                                    .into_iter()
                                    .map(|line| format!("unwired: {line}")),
                            );
                            Ok(lines)
                        }
                        Err(reason) => Err(reason),
                    }
                }
                Err(reason) => Err(reason),
            },
            "mount-page" => match crate::app::run::generate::read_document(argument) {
                Ok(json) => {
                    let factory = WidgetFactory::new_with_defaults();
                    let make = control_creator(&factory);
                    match preview.mount_json(&json, &make) {
                        Ok(report) => Ok(vec![format!(
                            "mounted page {argument}: {} control(s), {} issue(s), generation {}",
                            report.widgets_created, report.issues, report.generation
                        )]),
                        Err(reason) => Err(reason),
                    }
                }
                Err(reason) => Err(reason),
            },
            "update-page" => match crate::app::run::generate::read_document(argument) {
                Ok(json) => {
                    let factory = WidgetFactory::new_with_defaults();
                    let make = control_creator(&factory);
                    match preview.update_json(&json, &make) {
                        Ok(report) => Ok(vec![format!(
                            "updated page {argument}: {} patch(es), generation {}, {} update(s)",
                            report.patches, report.generation, report.updates
                        )]),
                        Err(reason) => Err(reason),
                    }
                }
                Err(reason) => Err(reason),
            },
            "update" => match document.as_mut() {
                None => Err("no document is mounted, so there is nothing to update".to_owned()),
                Some(current) => match document_node(argument) {
                    Ok(next) => {
                        let nodes = std::sync::Arc::make_mut(&mut current.nodes);
                        match nodes.iter_mut().find(|held| held.node == next.node) {
                            Some(held) => *held = next,
                            None => nodes.push(next),
                        }
                        let factory = WidgetFactory::new_with_defaults();
                        let make = control_creator(&factory);
                        match preview.update_view(current, &make) {
                            Ok(report) => {
                                let mut lines = vec![format!(
                                    "updated: {} patch(es), generation {}, {} update(s)",
                                    report.patches, report.generation, report.updates
                                )];
                                lines.extend(
                                    preview
                                        .unwired_events()
                                        .into_iter()
                                        .map(|line| format!("unwired: {line}")),
                                );
                                Ok(lines)
                            }
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
                        let make = control_creator(&factory);
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
                // Only what a restore would actually move: the focused control, any row container
                // that is scrolled away from its first row, and any control whose text differs from
                // the text its declaration gives it.
                let live = Capture {
                    entries: capture
                        .entries
                        .iter()
                        .filter(|kept| kept.focus || kept.scroll > 0 || !kept.input.is_empty())
                        .cloned()
                        .collect(),
                };
                let lines = live.render();
                if lines.is_empty() {
                    Ok(vec!["nothing focused, scrolled or typed into".to_owned()])
                } else {
                    Ok(lines)
                }
            }
            "restore" => match preview.restore_state(&capture) {
                Ok(outcomes) => {
                    let mut lines: Vec<String> =
                        outcomes.iter().map(|outcome| outcome.render()).collect();
                    match preview.restore_input(&capture) {
                        Ok(inputs) => {
                            lines.extend(inputs.iter().map(|input| input.render()));
                            Ok(lines)
                        }
                        Err(reason) => Err(reason),
                    }
                }
                Err(reason) => Err(reason),
            },
            "gap" => Ok(vec![Preview::scroll_gap().to_owned()]),
            // H-34: paint the mounted tree into an SVG frame. The library's real paint traversal
            // produces the picture, so this is a headless thumbnail of the same controls a window
            // would show, not a second host-side renderer.
            "paint" => match argument.rsplit_once(':') {
                Some((path, size)) => match size.split_once('x') {
                    Some((width, height)) => match (width.parse::<u32>(), height.parse::<u32>()) {
                        (Ok(width), Ok(height)) => {
                            match preview.paint_svg(rust_widgets::core::Size::new(width, height)) {
                                Ok(svg) => match std::fs::write(path, svg) {
                                    Ok(()) => Ok(vec![format!(
                                        "painted {width}x{height} frame -> {path}"
                                    )]),
                                    Err(error) => Err(format!("cannot write {path}: {error}")),
                                },
                                Err(reason) => Err(reason),
                            }
                        }
                        _ => Err(format!("{size:?} is not <width>x<height> in pixels")),
                    },
                    None => Err(format!("{size:?} is not <width>x<height>")),
                },
                None => Err(format!("{argument:?} is not <path>:<width>x<height>")),
            },
            // H-29: one document, three breakpoint tiers, solved by the library's own generator at
            // the width each tier accepts.
            "tiers" => match argument.split_once(':') {
                Some((page, directory)) if !page.is_empty() && !directory.is_empty() => {
                    tier_frames(page, Some(directory))
                }
                _ => tier_frames(argument, None),
            },
            "unwired" => {
                let lines = preview.unwired_events();
                if lines.is_empty() {
                    Ok(vec!["unwired: none reported".to_owned()])
                } else {
                    Ok(lines
                        .into_iter()
                        .map(|line| format!("unwired: {line}"))
                        .collect())
                }
            }
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

/// The height every breakpoint tier is solved at (H-29).
///
/// A tier is classified by width alone (the library's `Breakpoint::of` says why), so the height is
/// one constant shared by the three tiers; differing heights would make the results incomparable
/// without testing anything a tier decides.
const TIER_HEIGHT: u32 = 800;

/// The three tiers and the widest width each one accepts, from the library's own tokens.
///
/// Compact and Medium use the boundaries themselves; Expanded is the first width past the Medium
/// boundary, because the classifier has no upper bound. All three numbers come from
/// [`rust_widgets::style::breakpoint_tokens`], so a theme that moves a boundary moves the preview.
fn tier_widths() -> [(Breakpoint, u32); 3] {
    let tokens = rust_widgets::style::breakpoint_tokens();
    [
        (Breakpoint::Compact, tokens.compact_max),
        (Breakpoint::Medium, tokens.medium_max),
        (Breakpoint::Expanded, tokens.medium_max.saturating_add(1)),
    ]
}

/// The tier's stable name.
fn tier_name(tier: Breakpoint) -> &'static str {
    match tier {
        Breakpoint::Compact => "compact",
        Breakpoint::Medium => "medium",
        Breakpoint::Expanded => "expanded",
    }
}

/// Solves one document at every breakpoint tier (H-29).
///
/// The numbers are the library's generation-time solve (`GenerationReport::node_rects`, RW-116), so
/// the three tiers show what the generator would place, not a host-side arithmetic pass. The
/// classifier is the library's too, and a width that does not classify as the tier it was chosen
/// for is an error rather than a report with a misleading name.
///
/// `frames` names a directory to paint into. When it is given, the mounted preview's widgets receive
/// the generator's rectangles through the library's `widget::runtime::set_geometry` and the library's
/// own paint traversal writes one SVG per tier: the picture and the numbers come from one solve.
/// (The C-ABI `set_widget_geometry` only records a rectangle that paint never reads, so it is not the
/// setter used here.) When it is absent the command only reports the numbers.
///
/// **Boundary.** A tier that selected a different *subtree* needs a breakpoint member in the
/// library's declarative document, which it does not have (RW-117); this command therefore solves
/// one unchanged tree at three widths and says so. It does not invent a host-only breakpoint field
/// that generation could not consume (principle #13/#26).
fn tier_frames(page: &str, frames: Option<&str>) -> Result<Vec<String>, String> {
    use rust_widgets::designer::{GenerationRequest, TargetProfile, generate};

    let json = crate::app::run::generate::read_document(page)?;
    let mut lines = vec![format!("page: {page}")];
    // The full declarative loader registers parent/child links and applies a layout, which is what
    // a paint pass walks; the incremental view session keeps its own tree and paints only the root.
    // The solved rectangles below then replace the loader's geometry, so the picture is the
    // generator's solve rather than a second layout.
    let bound = match frames {
        Some(_) => Some(rust_widgets::json::JsonLoader::load(&json)?),
        None => None,
    };
    if let Some(directory) = frames {
        std::fs::create_dir_all(directory)
            .map_err(|error| format!("cannot create {directory}: {error}"))?;
    }
    let root = match (&bound, frames) {
        (Some(bound), Some(_)) => Some(
            bound
                .root()
                .ok_or_else(|| "the document has no root widget to paint".to_owned())?,
        ),
        _ => None,
    };
    for (tier, width) in tier_widths() {
        let size = Size::new(width, TIER_HEIGHT);
        if Breakpoint::of(size) != tier {
            return Err(format!(
                "{width}x{TIER_HEIGHT} classifies as {} rather than {}",
                tier_name(Breakpoint::of(size)),
                tier_name(tier)
            ));
        }
        let generated = generate(&GenerationRequest {
            json: json.clone(),
            target: TargetProfile::Default,
            width,
            height: TIER_HEIGHT,
            function_name: "build_ui".to_owned(),
        })
        .map_err(|reason| format!("{} at {width}x{TIER_HEIGHT}: {reason}", tier_name(tier)))?;
        lines.push(format!(
            "{}: {width}x{TIER_HEIGHT} -> {} node rect(s), {} layout issue(s)",
            tier_name(tier),
            generated.report.node_rects.len(),
            generated.report.layout_issues.len()
        ));
        for rect in &generated.report.node_rects {
            lines.push(format!(
                "  node {} (path {:?}): x={} y={} {}x{}",
                rect.id.as_deref().unwrap_or("-"),
                rect.path,
                rect.x,
                rect.y,
                rect.width,
                rect.height
            ));
        }
        for issue in &generated.report.layout_issues {
            lines.push(format!("  issue: {}", issue.describe()));
        }
        if let (Some(bound), Some(root), Some(directory)) = (bound.as_ref(), root, frames) {
            // The rectangles are the generator's solve (RW-116); applying them to the loaded
            // widgets is geometry, not a second layout. The library's paint traversal then writes
            // the frame, so the picture and the numbers above are one answer.
            for rect in &generated.report.node_rects {
                if let Some(name) = &rect.id
                    && let Some(widget) = bound.id(name)
                {
                    // `runtime::set_geometry` moves the widget itself; the C-ABI/state setter only
                    // records a rectangle the paint pass never reads.
                    rust_widgets::widget::runtime::set_geometry(
                        widget,
                        Rect::new(rect.x, rect.y, rect.width, rect.height),
                    );
                }
            }
            let mut backend = rust_widgets::render::SvgPaintBackend::new(size);
            let painted = rust_widgets::widget::runtime::paint_tree_into(
                root,
                size,
                rust_widgets::core::Color::WHITE,
                &mut backend,
            );
            if !painted {
                return Err(format!(
                    "{} frame: the root widget has no drawable",
                    tier_name(tier)
                ));
            }
            let path = format!("{directory}/{}.svg", tier_name(tier));
            std::fs::write(&path, backend.finish())
                .map_err(|error| format!("cannot write {path}: {error}"))?;
            lines.push(format!("  frame: {path}"));
        }
    }
    if let Some(bound) = &bound {
        // The full loader registers real widgets and stores declarative layouts keyed by parent;
        // a headless check must not leave either behind.
        for (id, _, _) in bound.walk() {
            rust_widgets::layout::declarative::forget_layout(id);
            rust_widgets::widget::runtime::unregister(id);
        }
    }
    lines.push(
        "per-tier subtree declarations: not expressible in the library's JSON document (RW-117); \
         these tiers solve one unchanged tree at three widths"
            .to_owned(),
    );
    Ok(lines)
}

#[cfg(test)]
mod tiers_tests {
    use super::{Breakpoint, Size, TIER_HEIGHT, tier_name, tier_widths};

    #[test]
    fn each_tier_width_classifies_as_the_tier_it_names() {
        for (tier, width) in tier_widths() {
            assert_eq!(
                Breakpoint::of(Size::new(width, TIER_HEIGHT)),
                tier,
                "{} at {width}",
                tier_name(tier)
            );
        }
        let widths: Vec<u32> = tier_widths().iter().map(|(_, width)| *width).collect();
        assert!(
            widths.windows(2).all(|pair| pair[0] < pair[1]),
            "{widths:?}"
        );
    }

    #[test]
    fn one_page_is_solved_at_all_three_tiers() {
        let root = std::env::temp_dir().join(format!("rustage-tiers-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("pages")).expect("a directory");
        std::fs::write(root.join("project.json"), "{}").expect("a project file");
        let page = root.join("pages/main.json");
        std::fs::write(
            &page,
            "{\"format\": 7, \"page\": {\"id\": 1, \"name\": \"P\", \"root\": 1}, \
             \"nodes\": [{\"id\": 1, \"type\": \"window\", \"children\": [2], \
               \"layout\": {\"type\": \"vbox\", \"spacing\": 8, \"margin\": 8}}, \
               {\"id\": 2, \"type\": \"label\", \"constraints\": {\"min_height\": 24}}], \
             \"states\": null, \"collections\": [], \
             \"components\": {\"definitions\": [], \"instances\": [], \"variants\": []}, \
             \"motion\": {\"motions\": []}}",
        )
        .expect("a page");
        let lines = super::tier_frames(page.to_str().expect("a path"), None).expect("three tiers");
        let _ = std::fs::remove_dir_all(&root);
        for tier in ["compact", "medium", "expanded"] {
            assert!(lines.iter().any(|line| line.starts_with(tier)), "{lines:?}");
        }
        assert_eq!(
            lines
                .iter()
                .filter(|line| line.contains("node rect(s)"))
                .count(),
            3,
            "{lines:?}"
        );
        assert!(
            lines.iter().any(|line| line.contains("RW-117")),
            "the boundary is named: {lines:?}"
        );
        // The root rect is the tier width, so the three solves differ.
        let roots: Vec<&String> = lines
            .iter()
            .filter(|line| line.starts_with("  node 1 (path [])"))
            .collect();
        assert_eq!(roots.len(), 3, "{lines:?}");
        assert!(roots[0] != roots[1] && roots[1] != roots[2], "{roots:?}");
    }

    #[test]
    fn a_frame_directory_gets_one_svg_per_tier_from_the_same_solve() {
        let root = std::env::temp_dir().join(format!("rustage-tier-frames-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("pages")).expect("a directory");
        std::fs::write(root.join("project.json"), "{}").expect("a project file");
        let page = root.join("pages/main.json");
        std::fs::write(
            &page,
            "{\"format\": 7, \"page\": {\"id\": 1, \"name\": \"P\", \"root\": 1}, \
             \"nodes\": [{\"id\": 1, \"type\": \"window\", \"children\": [2], \
               \"layout\": {\"type\": \"vbox\", \"spacing\": 8, \"margin\": 8}}, \
               {\"id\": 2, \"type\": \"label\"}], \
             \"states\": null, \"collections\": [], \
             \"components\": {\"definitions\": [], \"instances\": [], \"variants\": []}, \
             \"motion\": {\"motions\": []}}",
        )
        .expect("a page");
        let frames = root.join("frames");
        let lines = super::tier_frames(
            page.to_str().expect("a path"),
            Some(frames.to_str().expect("a frames path")),
        )
        .expect("three frames");
        let mut svgs = Vec::new();
        for tier in ["compact", "medium", "expanded"] {
            let path = frames.join(format!("{tier}.svg"));
            let svg = std::fs::read_to_string(&path).expect("a frame");
            assert!(svg.starts_with("<svg"), "{tier}: {svg}");
            assert!(svg.contains("width=\""), "{tier}: {svg}");
            svgs.push(svg);
        }
        let _ = std::fs::remove_dir_all(&root);
        assert!(
            svgs[0] != svgs[1] && svgs[1] != svgs[2],
            "the three frames are three solves"
        );
        assert!(
            lines.iter().filter(|line| line.contains("frame: ")).count() == 3,
            "{lines:?}"
        );
    }
}
