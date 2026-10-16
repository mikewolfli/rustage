// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! F09: the canvas, the widget catalog and the object tree.
//!
//! One command handler per parsed command of this domain; the dispatcher lives in
//! [`super`](self).

use super::*;

/// Shows the widget catalog (task T09.01).
pub(super) fn catalog(options: &crate::cli::CatalogOptions) -> bool {
    use crate::app::widget_catalog::{Catalog, WidgetDescriptor};

    let mut descriptors: Vec<WidgetDescriptor> = Vec::new();
    let mut favorites: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "widget" => match parse_widget(argument) {
                Some((id, name, category)) => {
                    descriptors.push(WidgetDescriptor::new(id, name, category));
                    Ok(vec![format!(
                        "widget: {} ({})",
                        descriptors.last().expect("just pushed").name,
                        descriptors.last().expect("just pushed").category
                    )])
                }
                None => Err(format!("{argument:?} is not <id>:<name>:<category>")),
            },
            "unsupported" => match argument.split_once(':') {
                Some((id, reason)) => match descriptors.iter_mut().find(|d| d.id == id) {
                    Some(descriptor) => {
                        *descriptor = descriptor.clone().unsupported(unescape(reason));
                        Ok(vec![format!("unsupported: {id}")])
                    }
                    None => Err(format!("no widget {id:?}")),
                },
                None => Err(format!("{argument:?} is not <id>:<reason>")),
            },
            "property" => match parse_property(argument) {
                Ok((id, property)) => match descriptors.iter_mut().find(|d| d.id == id) {
                    Some(descriptor) => {
                        let line = property.render();
                        descriptor.properties.push(property);
                        Ok(vec![format!("property: {id} {line}")])
                    }
                    None => Err(format!("no widget {id:?}")),
                },
                Err(reason) => Err(reason),
            },
            "limit" => match argument.split_once(':') {
                Some((id, note)) => match descriptors.iter_mut().find(|d| d.id == id) {
                    Some(descriptor) => {
                        descriptor.limits.push(unescape(note));
                        Ok(vec![format!("limit: {id}")])
                    }
                    None => Err(format!("no widget {id:?}")),
                },
                None => Err(format!("{argument:?} is not <id>:<note>")),
            },
            "search" => {
                let catalog = Catalog::of(descriptors.clone());
                let hits = catalog.search(argument);
                let mut notes = vec![format!("search {argument:?}: {} hit(s)", hits.len())];
                notes.extend(hits.iter().map(|hit| format!("  {}", hit.render())));
                Ok(notes)
            }
            "category" => {
                let catalog = Catalog::of(descriptors.clone());
                let hits = catalog.in_category(argument);
                let mut notes = vec![format!("category {argument}: {} kind(s)", hits.len())];
                notes.extend(hits.iter().map(|hit| format!("  {}", hit.render())));
                Ok(notes)
            }
            "favorite" | "unfavorite" => {
                if verb == "favorite" {
                    favorites.insert(argument.to_owned());
                } else {
                    favorites.remove(argument);
                }
                Ok(vec![format!("{verb}: {argument}")])
            }
            "schema" => {
                let catalog = Catalog::of(descriptors.clone());
                match catalog.schema(argument) {
                    Some(schema) => {
                        let mut notes = vec![format!("schema {argument}:")];
                        notes.extend(
                            schema
                                .iter()
                                .map(|property| format!("  {}", property.render())),
                        );
                        for note in catalog
                            .get(argument)
                            .map(|d| d.limits.clone())
                            .unwrap_or_default()
                        {
                            notes.push(format!("  limit: {note}"));
                        }
                        Ok(notes)
                    }
                    None => Err(format!("no widget {argument:?}")),
                }
            }
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a catalog operation")),
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
    let mut catalog = Catalog::of(descriptors);
    for id in &favorites {
        catalog.favorite(id, true);
    }
    for line in catalog.render() {
        println!("{line}");
    }
    println!("unsupported:");
    for descriptor in catalog.unsupported() {
        println!(
            "  {}: {}",
            descriptor.id,
            catalog.reason(&descriptor.id).unwrap_or("")
        );
    }
    true
}

/// Drives the design scene: instances, hit tests and selection (task T09.02).
pub(super) fn scene(options: &crate::cli::SceneOptions) -> bool {
    use crate::app::scene::Scene;

    let mut scene = Scene::new();
    let mut selection = crate::app::selection::Selection::new();
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "parent" => match argument.split_once(':') {
                Some((node, parent)) => {
                    let node = match parse_scene_node(node) {
                        Ok(node) => node,
                        Err(reason) => return report(reason),
                    };
                    let target = if parent == "-" {
                        None
                    } else {
                        match parse_scene_node(parent) {
                            Ok(parent) => Some(parent),
                            Err(reason) => return report(reason),
                        }
                    };
                    match scene.set_parent(node, target) {
                        Ok(()) => Ok(vec![format!("parent {}: {parent}", node.get())]),
                        Err(reason) => Err(reason),
                    }
                }
                None => Err(format!("{argument:?} is not <node>:<parent>-")),
            },
            "hide" | "unhide" => match parse_scene_node(argument) {
                Ok(node) => {
                    scene.set_hidden(node, verb == "hide");
                    Ok(vec![format!("{verb} {}", node.get())])
                }
                Err(reason) => Err(reason),
            },
            "lock" | "unlock" => match parse_scene_node(argument) {
                Ok(node) => {
                    scene.set_locked(node, verb == "lock");
                    Ok(vec![format!("{verb} {}", node.get())])
                }
                Err(reason) => Err(reason),
            },
            "click" | "cycle" => match parse_point(argument) {
                Ok((x, y)) => {
                    let picked = if verb == "click" {
                        scene
                            .hit(x, y)
                            .filter(|node| selection.click(&scene, *node))
                    } else {
                        selection.cycle(&scene, x, y)
                    };
                    Ok(vec![match picked {
                        Some(node) => format!("{verb} {x},{y}: node {}", node.get()),
                        None => format!("{verb} {x},{y}: nothing"),
                    }])
                }
                Err(reason) => Err(reason),
            },
            "marquee" => match parse_rect(argument) {
                Ok(rect) => {
                    let picked = selection.marquee(&scene, rect);
                    Ok(vec![format!("marquee: {} node(s)", picked.len())])
                }
                Err(reason) => Err(reason),
            },
            "select-all" => {
                selection.select_all(&scene);
                Ok(vec![format!("select-all: {} node(s)", selection.len())])
            }
            "prune" => {
                let dropped = selection.prune(&scene);
                Ok(vec![format!("prune: dropped {}", dropped.len())])
            }
            "add" | "rebuild" => match parse_node_rect(argument) {
                Ok((node, rect)) => {
                    let handle = if verb == "add" {
                        scene.insert(node, rect)
                    } else {
                        match scene.rebuild(node, rect) {
                            Some(handle) => handle,
                            None => return report(format!("no instance for node {}", node.get())),
                        }
                    };
                    Ok(vec![format!("{verb}: {}", handle.render())])
                }
                Err(reason) => Err(reason),
            },
            "remove" => match parse_scene_node(argument) {
                Ok(node) => {
                    let removed = scene.remove(node);
                    Ok(vec![format!("remove {}: {removed}", node.get())])
                }
                Err(reason) => Err(reason),
            },
            "hit" => match parse_point(argument) {
                Ok((x, y)) => Ok(vec![match scene.hit(x, y) {
                    Some(node) => format!("hit {x},{y}: node {}", node.get()),
                    None => format!("hit {x},{y}: nothing"),
                }]),
                Err(reason) => Err(reason),
            },
            "resolve" => match parse_node_generation(argument) {
                Ok((node, generation)) => {
                    match scene.resolve(crate::app::scene::Handle { node, generation }) {
                        Ok(rect) => Ok(vec![format!(
                            "resolve node {} gen {generation}: {}",
                            node.get(),
                            rect.render()
                        )]),
                        Err(stale) => Err(stale.message()),
                    }
                }
                Err(reason) => Err(reason),
            },
            "raise" => match parse_scene_node(argument) {
                Ok(node) => {
                    let raised = scene.raise(node);
                    Ok(vec![format!("raise {}: {raised}", node.get())])
                }
                Err(reason) => Err(reason),
            },
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a scene operation")),
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
    for line in scene.render() {
        println!("{line}");
    }
    for line in selection.render() {
        println!("{line}");
    }
    true
}

/// A `(node, rect)` from `<node>:<x>,<y>,<w>,<h>`.
pub(super) fn parse_node_rect(
    argument: &str,
) -> Result<(crate::contracts::identity::NodeId, crate::app::scene::Rect), String> {
    let (node, rect) = argument
        .split_once(':')
        .ok_or_else(|| format!("{argument:?} is not <node>:<rect>"))?;
    Ok((parse_scene_node(node)?, parse_rect(rect)?))
}

/// A rect from `<x>,<y>,<w>,<h>`.
pub(super) fn parse_rect(text: &str) -> Result<crate::app::scene::Rect, String> {
    let parts: Vec<&str> = text.split(',').collect();
    let [x, y, width, height] = parts.as_slice() else {
        return Err(format!("{text:?} is not <x>,<y>,<w>,<h>"));
    };
    let number = |value: &str| -> Result<i32, String> {
        value
            .parse()
            .map_err(|_| format!("{value:?} is not a number"))
    };
    Ok(crate::app::scene::Rect::new(
        number(x)?,
        number(y)?,
        number(width)?.max(0) as u32,
        number(height)?.max(0) as u32,
    ))
}

/// A point from `<x>,<y>`.
pub(super) fn parse_point(text: &str) -> Result<(i32, i32), String> {
    let (x, y) = text
        .split_once(',')
        .ok_or_else(|| format!("{text:?} is not <x>,<y>"))?;
    let number = |value: &str| -> Result<i32, String> {
        value
            .parse()
            .map_err(|_| format!("{value:?} is not a coordinate"))
    };
    Ok((number(x)?, number(y)?))
}

/// A `(node, generation)` from `<node>:<generation>`.
pub(super) fn parse_node_generation(
    argument: &str,
) -> Result<(crate::contracts::identity::NodeId, u64), String> {
    let (node, generation) = argument
        .split_once(':')
        .ok_or_else(|| format!("{argument:?} is not <node>:<generation>"))?;
    let generation: u64 = generation
        .parse()
        .map_err(|_| format!("{generation:?} is not a generation"))?;
    Ok((parse_scene_node(node)?, generation))
}

/// Runs structural edits on the object tree (task T09.04).
pub(super) fn structure(options: &crate::cli::StructureOptions) -> bool {
    use crate::app::structure::Tree;

    let mut tree = Tree::new();
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "root" => match argument.split_once(':') {
                Some((kind, name)) => match parse_kind(kind) {
                    Ok(kind) => match tree.insert(None, kind, name) {
                        Ok(node) => Ok(vec![format!("root {name}: node {}", node.get())]),
                        Err(error) => Err(error.message()),
                    },
                    Err(reason) => Err(reason),
                },
                None => Err(format!("{argument:?} is not <kind>:<name>")),
            },
            "insert" => match parse_insert(argument) {
                Ok((parent, kind, name)) => match tree.insert(Some(parent), kind, name.clone()) {
                    Ok(node) => Ok(vec![format!("insert {name}: node {}", node.get())]),
                    Err(error) => Err(error.message()),
                },
                Err(reason) => Err(reason),
            },
            "delete" => match parse_scene_node(argument) {
                Ok(node) => match tree.delete(node) {
                    Ok(report) => {
                        let mut notes = vec![report.render()];
                        notes.extend(report.references.iter().map(|(from, target)| {
                            format!("  node {} referenced node {}", from.get(), target.get())
                        }));
                        Ok(notes)
                    }
                    Err(error) => Err(error.message()),
                },
                Err(reason) => Err(reason),
            },
            "copy" => match parse_scene_node(argument) {
                Ok(node) => match tree.copy(node) {
                    Ok(copy) => Ok(vec![format!("copy: node {}", copy.get())]),
                    Err(error) => Err(error.message()),
                },
                Err(reason) => Err(reason),
            },
            "rename" => match argument.split_once(':') {
                Some((node, name)) => match parse_scene_node(node) {
                    Ok(node) => match tree.rename(node, name) {
                        Ok(()) => Ok(vec![format!("rename {}: {name}", node.get())]),
                        Err(error) => Err(error.message()),
                    },
                    Err(reason) => Err(reason),
                },
                None => Err(format!("{argument:?} is not <node>:<name>")),
            },
            "reorder" => match argument.split_once(':') {
                Some((node, index)) => match (parse_scene_node(node), index.parse::<usize>()) {
                    (Ok(node), Ok(index)) => match tree.reorder(node, index) {
                        Ok(()) => Ok(vec![format!("reorder {}: {index}", node.get())]),
                        Err(error) => Err(error.message()),
                    },
                    _ => Err(format!("{argument:?} is not <node>:<index>")),
                },
                None => Err(format!("{argument:?} is not <node>:<index>")),
            },
            "reparent" => match argument.split_once(':') {
                Some((node, parent)) => match parse_scene_node(node) {
                    Ok(node) => {
                        let target = if parent == "-" {
                            Ok(None)
                        } else {
                            parse_scene_node(parent).map(Some)
                        };
                        match target {
                            Ok(target) => match tree.reparent(node, target) {
                                Ok(()) => Ok(vec![format!("reparent {}: {parent}", node.get())]),
                                Err(error) => Err(error.message()),
                            },
                            Err(reason) => Err(reason),
                        }
                    }
                    Err(reason) => Err(reason),
                },
                None => Err(format!("{argument:?} is not <node>:<parent>-")),
            },
            "reference" => match argument.split_once(':') {
                Some((from, to)) => match (parse_scene_node(from), parse_scene_node(to)) {
                    (Ok(from), Ok(to)) => {
                        tree.reference(from, to);
                        Ok(vec![format!("reference {} -> {}", from.get(), to.get())])
                    }
                    _ => Err(format!("{argument:?} is not <from>:<to>")),
                },
                None => Err(format!("{argument:?} is not <from>:<to>")),
            },
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a structure operation")),
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
    for line in tree.render() {
        println!("{line}");
    }
    println!("acyclic: {}", tree.is_acyclic());
    true
}

/// Pans, zooms and shows the design aids without touching the model (task T09.05).
pub(super) fn viewport(options: &crate::cli::ViewportOptions) -> bool {
    use crate::app::viewport::{Point, Size, Transform, Viewport, align};

    let mut viewport = Viewport::new(10.0);
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "transform" => match parse_floats(argument).as_deref() {
                Ok([pan_x, pan_y, zoom]) => {
                    *viewport.transform_mut() = Transform::at(Point::new(*pan_x, *pan_y), *zoom);
                    Ok(vec![format!("transform: pan {pan_x},{pan_y} zoom {zoom}")])
                }
                _ => Err(format!("{argument:?} is not <panx>,<pany>,<zoom>")),
            },
            "pan" => match parse_floats(argument).as_deref() {
                Ok([dx, dy]) => {
                    viewport.transform_mut().pan_by(*dx, *dy);
                    Ok(vec![format!("pan: {dx},{dy}")])
                }
                _ => Err(format!("{argument:?} is not <dx>,<dy>")),
            },
            "zoom" => match argument.parse::<f64>() {
                Ok(factor) => {
                    viewport.transform_mut().zoom_by(factor);
                    Ok(vec![format!(
                        "zoom: x{factor} -> {}",
                        viewport.transform().zoom()
                    )])
                }
                Err(_) => Err(format!("{argument:?} is not a factor")),
            },
            "zoom-at" => match argument.split_once(':') {
                Some((factor, anchor)) => {
                    match (factor.parse::<f64>(), parse_floats(anchor).as_deref()) {
                        (Ok(factor), Ok([x, y])) => {
                            viewport.transform_mut().zoom_at(factor, Point::new(*x, *y));
                            Ok(vec![format!("zoom-at: x{factor} about {x},{y}")])
                        }
                        _ => Err(format!("{argument:?} is not <factor>:<x>,<y>")),
                    }
                }
                None => Err(format!("{argument:?} is not <factor>:<x>,<y>")),
            },
            "fit" => match parse_fit(argument) {
                Ok((bounds, view, margin)) => {
                    *viewport.transform_mut() = Transform::fit(bounds, view, margin);
                    Ok(vec![format!(
                        "fit: {} in {},{}",
                        bounds.render(),
                        view.width,
                        view.height
                    )])
                }
                Err(reason) => Err(reason),
            },
            "actual-size" => match parse_floats(argument).as_deref() {
                Ok([width, height]) => {
                    viewport
                        .transform_mut()
                        .actual_size(Size::new(*width, *height));
                    Ok(vec![format!("actual-size: {width},{height}")])
                }
                _ => Err(format!("{argument:?} is not <vw>,<vh>")),
            },
            "spacing" => match argument.parse::<f64>() {
                Ok(spacing) => {
                    viewport.grid_mut().set_spacing(spacing);
                    Ok(vec![format!("spacing: {spacing}")])
                }
                Err(_) => Err(format!("{argument:?} is not a spacing")),
            },
            "ruler" => match argument.parse::<f64>() {
                Ok(step) => {
                    viewport.rulers_mut().set_step(step);
                    Ok(vec![format!("ruler: {step}")])
                }
                Err(_) => Err(format!("{argument:?} is not a step")),
            },
            "grid" => match argument {
                "on" => {
                    viewport.grid_mut().set_visible(true);
                    Ok(vec!["grid: on".to_owned()])
                }
                "off" => {
                    viewport.grid_mut().set_visible(false);
                    Ok(vec!["grid: off".to_owned()])
                }
                other => Err(format!("{other:?} is not on or off")),
            },
            "snap" => {
                if argument.contains(',') {
                    match parse_floats(argument).as_deref() {
                        Ok([x, y]) => {
                            let snapped = viewport.grid().snap(Point::new(*x, *y));
                            Ok(vec![format!("snap {x},{y} -> {},{}", snapped.x, snapped.y)])
                        }
                        _ => Err(format!("{argument:?} is not <x>,<y>")),
                    }
                } else {
                    match argument {
                        "on" => {
                            viewport.grid_mut().set_snapping(true);
                            Ok(vec!["snap: on".to_owned()])
                        }
                        "off" => {
                            viewport.grid_mut().set_snapping(false);
                            Ok(vec!["snap: off".to_owned()])
                        }
                        other => Err(format!("{other:?} is not on, off or <x>,<y>")),
                    }
                }
            }
            "align" => match parse_align(argument) {
                Ok((moving, others, tolerance)) => {
                    let guides = align(moving, &others, tolerance);
                    let report = guides.len();
                    viewport.set_guides(guides);
                    Ok(vec![format!("align: {report} guide(s)")])
                }
                Err(reason) => Err(reason),
            },
            "to-screen" => match parse_rect_point(argument) {
                Ok(point) => {
                    let screen = viewport.transform().to_screen(point);
                    Ok(vec![format!(
                        "to-screen {argument}: {},{}",
                        screen.x, screen.y
                    )])
                }
                Err(reason) => Err(reason),
            },
            "to-model" => match parse_rect_point(argument) {
                Ok(point) => {
                    let model = viewport.transform().to_model(point);
                    Ok(vec![format!(
                        "to-model {argument}: {},{}",
                        model.x, model.y
                    )])
                }
                Err(reason) => Err(reason),
            },
            "overlay" => match parse_floats(argument).as_deref() {
                Ok([width, height]) => Ok(viewport.overlay(Size::new(*width, *height)).render()),
                _ => Err(format!("{argument:?} is not <vw>,<vh>")),
            },
            "show" => Ok(Vec::new()),
            "state" => Ok(viewport.render()),
            other => Err(format!("{other:?} is not a viewport operation")),
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
    for line in viewport.render() {
        println!("{line}");
    }
    true
}

/// A `(bounds, view, margin)` from `<x>,<y>,<w>,<h>:<vw>,<vh>[:<margin>]`.
pub(super) fn parse_fit(
    argument: &str,
) -> Result<(crate::app::scene::Rect, crate::app::viewport::Size, f64), String> {
    let mut parts = argument.split(':');
    let bounds = parts
        .next()
        .ok_or_else(|| format!("{argument:?} is not <bounds>:<view>"))?;
    let view = parts
        .next()
        .ok_or_else(|| format!("{argument:?} is not <bounds>:<view>"))?;
    let margin = match parts.next() {
        Some(margin) => margin
            .parse::<f64>()
            .map_err(|_| format!("{margin:?} is not a margin"))?,
        None => 0.0,
    };
    Ok((parse_rect(bounds)?, parse_size(view)?, margin))
}

/// A `(x, y)` size in float form.
pub(super) fn parse_size(text: &str) -> Result<crate::app::viewport::Size, String> {
    match parse_floats(text).as_deref() {
        Ok([width, height]) => Ok(crate::app::viewport::Size::new(*width, *height)),
        _ => Err(format!("{text:?} is not <w>,<h>")),
    }
}

/// A model or screen point from `<x>,<y>`.
pub(super) fn parse_rect_point(text: &str) -> Result<crate::app::viewport::Point, String> {
    match parse_floats(text).as_deref() {
        Ok([x, y]) => Ok(crate::app::viewport::Point::new(*x, *y)),
        _ => Err(format!("{text:?} is not <x>,<y>")),
    }
}

/// A `(moving, others, tolerance)` from `<rect>:<rect>[:<tol>]`.
pub(super) fn parse_align(
    argument: &str,
) -> Result<(crate::app::scene::Rect, Vec<crate::app::scene::Rect>, f64), String> {
    let mut parts = argument.split(':');
    let moving = parts
        .next()
        .ok_or_else(|| format!("{argument:?} is not <rect>:<rect>"))?;
    let other = parts
        .next()
        .ok_or_else(|| format!("{argument:?} is not <rect>:<rect>"))?;
    let tolerance = match parts.next() {
        Some(tolerance) => tolerance
            .parse::<f64>()
            .map_err(|_| format!("{tolerance:?} is not a tolerance"))?,
        None => 1.0,
    };
    Ok((parse_rect(moving)?, vec![parse_rect(other)?], tolerance))
}

/// Drags a node, previews the drop, and commits a batch as one transaction (task T09.06).
pub(super) fn drag(options: &crate::cli::DragOptions) -> bool {
    use crate::app::drag::{Design, Drag, Drop, Edit};

    let mut design = Design::new();
    let mut in_progress: Option<Drag> = None;
    let mut pending: Vec<crate::app::drag::Move> = Vec::new();
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "root" => match argument.split_once(':') {
                Some((kind, name)) => match parse_kind(kind) {
                    Ok(kind) => insert_edit(&mut design, None, kind, name),
                    Err(reason) => Err(reason),
                },
                None => Err(format!("{argument:?} is not <kind>:<name>")),
            },
            "insert" => match parse_insert(argument) {
                Ok((parent, kind, name)) => insert_edit(&mut design, Some(parent), kind, &name),
                Err(reason) => Err(reason),
            },
            "begin" => match parse_scene_node(argument) {
                Ok(node) => match Drag::begin(design.tree(), node) {
                    Ok(started) => {
                        in_progress = Some(started);
                        Ok(vec![format!("begin: node {}", node.get())])
                    }
                    Err(error) => Err(error.message()),
                },
                Err(reason) => Err(reason),
            },
            "candidates" => match &in_progress {
                Some(started) => Ok(started.candidates().render()),
                None => Err("no drag is in progress".to_owned()),
            },
            "aim" => match parse_parent_index(argument) {
                Ok((container, index)) => match &mut in_progress {
                    Some(started) => {
                        started.aim(container, index);
                        let target = match container {
                            Some(container) => format!("node {}", container.get()),
                            None => "the root".to_owned(),
                        };
                        Ok(vec![format!("aim: {target} at {index}")])
                    }
                    None => Err("no drag is in progress".to_owned()),
                },
                Err(reason) => Err(reason),
            },
            "preview" => match &in_progress {
                Some(started) => Ok(vec![started.preview(design.tree()).render()]),
                None => Err("no drag is in progress".to_owned()),
            },
            "release" => match &in_progress {
                Some(started) => {
                    let outcome = started.release(design.tree());
                    let rendered = outcome.render();
                    if let Drop::Move(moving) = outcome {
                        pending.push(moving);
                    }
                    Ok(vec![rendered])
                }
                None => Err("no drag is in progress".to_owned()),
            },
            "cancel" => {
                in_progress = None;
                Ok(vec!["drop: cancelled".to_owned()])
            }
            "stage" => match parse_move(argument) {
                Ok(moving) => {
                    pending.push(moving);
                    Ok(vec![format!("stage: {}", moving.render())])
                }
                Err(reason) => Err(reason),
            },
            "commit" => {
                let edits: Vec<Edit> = pending.iter().copied().map(Edit::Move).collect();
                let staged = pending.len();
                pending.clear();
                match design.apply(&edits) {
                    Ok(outcomes) => {
                        let mut lines = vec![format!("commit: {staged} move(s)")];
                        lines.extend(outcomes.iter().map(|outcome| outcome.render()));
                        Ok(lines)
                    }
                    Err(error) => Err(error.message()),
                }
            }
            "discard" => {
                let staged = pending.len();
                pending.clear();
                Ok(vec![format!("discard: {staged} move(s)")])
            }
            "undo" => Ok(vec![format!("undo: {}", design.undo())]),
            "redo" => Ok(vec![format!("redo: {}", design.redo())]),
            "state" => Ok(design.render()),
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a drag operation")),
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
    for line in design.render() {
        println!("{line}");
    }
    true
}

/// Inserts a node as its own one-edit transaction.
pub(super) fn insert_edit(
    design: &mut crate::app::drag::Design,
    parent: Option<crate::contracts::identity::NodeId>,
    kind: crate::app::structure::Kind,
    name: &str,
) -> Result<Vec<String>, String> {
    let edit = crate::app::drag::Edit::Insert {
        parent,
        kind,
        name: name.to_owned(),
    };
    match design.apply(std::slice::from_ref(&edit)) {
        Ok(outcomes) => Ok(outcomes.iter().map(|outcome| outcome.render()).collect()),
        Err(error) => Err(error.message()),
    }
}

/// The comma-separated numbers in an argument.
pub(super) fn parse_move(argument: &str) -> Result<crate::app::drag::Move, String> {
    let (node, rest) = argument
        .split_once(':')
        .ok_or_else(|| format!("{argument:?} is not <node>:<parent|->:<index>"))?;
    let (parent, index) = parse_parent_index(rest)?;
    Ok(crate::app::drag::Move::new(
        parse_scene_node(node)?,
        parent,
        index,
    ))
}

/// A `(parent, index)` from `<parent|->:<index>`.
pub(super) fn parse_parent_index(
    argument: &str,
) -> Result<(Option<crate::contracts::identity::NodeId>, usize), String> {
    let (parent, index) = argument
        .split_once(':')
        .ok_or_else(|| format!("{argument:?} is not <parent|->:<index>"))?;
    let parent = if parent == "-" {
        None
    } else {
        Some(parse_scene_node(parent)?)
    };
    let index = index
        .parse::<usize>()
        .map_err(|_| format!("{index:?} is not an index"))?;
    Ok((parent, index))
}

/// Shows design-time object state, filtering and pages (task T09.07).
pub(super) fn objects(options: &crate::cli::ObjectsOptions) -> bool {
    use crate::app::objects::{Filter, Objects, Pages, published};
    use crate::contracts::identity::NodeId;
    use crate::contracts::model::Node;
    use std::collections::BTreeMap;

    let mut nodes: BTreeMap<NodeId, Node> = BTreeMap::new();
    let mut objects = Objects::new();
    let mut pages = Pages::new();
    let mut filter = Filter::new();
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "node" => match parse_document_node(argument) {
                Ok((id, node)) => {
                    objects.add(id);
                    nodes.insert(id, node);
                    Ok(vec![format!("node {}: {}", id.get(), argument)])
                }
                Err(reason) => Err(reason),
            },
            "hide" | "unhide" => with_node(argument, |node| {
                let hidden = verb == "hide";
                objects.set_hidden(node, hidden);
                format!("node {}: hidden {hidden}", node.get())
            }),
            "lock" | "unlock" => with_node(argument, |node| {
                let locked = verb == "lock";
                objects.set_locked(node, locked);
                format!("node {}: locked {locked}", node.get())
            }),
            "raise" | "lower" => with_node(argument, |node| {
                let moved = if verb == "raise" {
                    objects.raise(node)
                } else {
                    objects.lower(node)
                };
                format!("node {}: {verb}ed {moved}", node.get())
            }),
            "published" => match parse_scene_node(argument) {
                Ok(node) => match nodes.get(&node) {
                    Some(document) => Ok(vec![format!(
                        "node {}: published {}",
                        node.get(),
                        published(document)
                    )]),
                    None => Err(format!("node {} is not in the document", node.get())),
                },
                Err(reason) => Err(reason),
            },
            "visible" => with_node(argument, |node| {
                format!(
                    "node {}: design visible {}",
                    node.get(),
                    !objects.is_hidden(node)
                )
            }),
            "filter-text" => {
                filter.text = Some(argument.to_owned());
                Ok(vec![format!("filter-text: {argument:?}")])
            }
            "filter-kind" => match crate::contracts::model::NodeKind::new(argument) {
                Some(kind) => {
                    filter.kind = Some(kind);
                    Ok(vec![format!("filter-kind: {argument}")])
                }
                None => Err(format!("{argument:?} is not a kind")),
            },
            "filter-state" => match parse_state_filter(argument) {
                Ok(state) => {
                    filter.state = Some(state);
                    Ok(vec![format!("filter-state: {argument}")])
                }
                Err(reason) => Err(reason),
            },
            "filter-clear" => {
                filter = Filter::new();
                Ok(vec!["filter-clear".to_owned()])
            }
            "filter" => {
                let matched = objects.filter(nodes.values(), &filter);
                let mut lines = vec![format!("filter: {} match(es)", matched.len())];
                lines.extend(matched.iter().map(|id| format!("  node {}", id.get())));
                Ok(lines)
            }
            "locate" => {
                let matched = objects.locate(nodes.values(), argument);
                let mut lines = vec![format!("locate {argument:?}: {} match(es)", matched.len())];
                lines.extend(matched.iter().map(|id| format!("  node {}", id.get())));
                Ok(lines)
            }
            "page" => match parse_page(argument) {
                Ok(page) => match pages.add(page.clone()) {
                    Ok(()) => Ok(vec![format!(
                        "page {}: {:?} root {}",
                        page.id().get(),
                        page.name(),
                        page.root().get()
                    )]),
                    Err(reason) => Err(reason),
                },
                Err(reason) => Err(reason),
            },
            "activate" => match parse_page_id(argument) {
                Ok(id) => match pages.activate(id) {
                    Ok(()) => Ok(vec![format!("activate: page {}", id.get())]),
                    Err(reason) => Err(reason),
                },
                Err(reason) => Err(reason),
            },
            "close-page" => match parse_page_id(argument) {
                Ok(id) => match pages.close(id) {
                    Ok(()) => Ok(vec![format!("close-page: {}", id.get())]),
                    Err(reason) => Err(reason),
                },
                Err(reason) => Err(reason),
            },
            "rename-page" => match argument.split_once(':') {
                Some((id, name)) => match parse_page_id(id) {
                    Ok(id) => match pages.rename(id, name) {
                        Ok(()) => Ok(vec![format!("rename-page {}: {name:?}", id.get())]),
                        Err(reason) => Err(reason),
                    },
                    Err(reason) => Err(reason),
                },
                None => Err(format!("{argument:?} is not <id>:<name>")),
            },
            "state" => {
                let mut lines = objects.render();
                lines.extend(pages.render());
                Ok(lines)
            }
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not an objects operation")),
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
    for line in objects.render() {
        println!("{line}");
    }
    for line in pages.render() {
        println!("{line}");
    }
    true
}

/// Runs `action` on the node a decimal argument names, as its own one-line report.
pub(super) fn with_node(
    argument: &str,
    action: impl FnOnce(crate::contracts::identity::NodeId) -> String,
) -> Result<Vec<String>, String> {
    Ok(vec![action(parse_scene_node(argument)?)])
}

/// A `(id, node)` from `<id>:<kind>[:visible=<bool>][:name=<text>]`.
pub(super) fn parse_document_node(
    argument: &str,
) -> Result<
    (
        crate::contracts::identity::NodeId,
        crate::contracts::model::Node,
    ),
    String,
> {
    use crate::contracts::identity::PageId;
    use crate::contracts::model::{Node, NodeKind, PropertyName, PropertyValue};
    use std::collections::BTreeMap;

    let mut fields = argument.split(':');
    let id = fields
        .next()
        .ok_or_else(|| format!("{argument:?} is not <id>:<kind>"))?;
    let kind = fields
        .next()
        .ok_or_else(|| format!("{argument:?} is not <id>:<kind>"))?;
    let id = parse_scene_node(id)?;
    let kind = NodeKind::new(kind).ok_or_else(|| format!("{kind:?} is not a kind"))?;
    let mut properties = BTreeMap::new();
    for field in fields {
        let (key, value) = field
            .split_once('=')
            .ok_or_else(|| format!("{field:?} is not <key>=<value>"))?;
        match key {
            "visible" => {
                properties.insert(
                    PropertyName::new("visible").expect("a fixed name"),
                    PropertyValue::Bool(value == "true"),
                );
            }
            "name" => {
                properties.insert(
                    PropertyName::new("name").expect("a fixed name"),
                    PropertyValue::Text(value.to_owned()),
                );
            }
            other => return Err(format!("{other:?} is not visible or name")),
        }
    }
    Ok((id, Node::record(PageId::from_raw(0), id, kind, properties)))
}

/// A page from `<id>:<name>:<root>`.
pub(super) fn parse_page(argument: &str) -> Result<crate::contracts::model::Page, String> {
    let fields: Vec<&str> = argument.splitn(3, ':').collect();
    let [id, name, root] = fields.as_slice() else {
        return Err(format!("{argument:?} is not <id>:<name>:<root>"));
    };
    let id = parse_page_id(id)?;
    let root = parse_scene_node(root)?;
    crate::contracts::model::Page::new(id, name, root)
        .ok_or_else(|| format!("{name:?} is not a page name"))
}

/// A page identity from a decimal argument.
pub(super) fn parse_page_id(argument: &str) -> Result<crate::contracts::identity::PageId, String> {
    argument
        .parse::<u64>()
        .map(crate::contracts::identity::PageId::from_raw)
        .map_err(|_| format!("{argument:?} is not a page id"))
}

/// A design-state filter from a name.
pub(super) fn parse_state_filter(
    argument: &str,
) -> Result<crate::app::objects::StateFilter, String> {
    match argument {
        "hidden" => Ok(crate::app::objects::StateFilter::Hidden),
        "shown" => Ok(crate::app::objects::StateFilter::Shown),
        "locked" => Ok(crate::app::objects::StateFilter::Locked),
        "unlocked" => Ok(crate::app::objects::StateFilter::Unlocked),
        other => Err(format!("{other:?} is not a state")),
    }
}

/// Drives the design surface's keyboard and its stress harness (task T09.08).
pub(super) fn design(options: &crate::cli::DesignOptions) -> bool {
    use crate::app::design::{Action, Surface};
    use crate::app::shortcuts::{Chord, Scene as Keys};

    let mut surface = Surface::bounded(64);
    let context = Keys::in_mode("design", None);
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "node" => match parse_design_node(argument) {
                Ok((parent, kind, name)) => match surface.add(parent, kind, &name) {
                    Ok(id) => Ok(vec![format!("node {}: {name}", id.get())]),
                    Err(reason) => Err(reason),
                },
                Err(reason) => Err(reason),
            },
            "select" => match parse_scene_node(argument) {
                Ok(node) => Ok(vec![format!(
                    "select {}: {}",
                    node.get(),
                    surface.select(node)
                )]),
                Err(reason) => Err(reason),
            },
            "all" | "deselect" | "delete" | "undo" | "redo" | "next" | "previous" | "parent"
            | "child" => {
                let action = match verb {
                    "all" => Action::SelectAll,
                    "deselect" => Action::Deselect,
                    "delete" => Action::Delete,
                    "undo" => Action::Undo,
                    "redo" => Action::Redo,
                    "next" => Action::Next,
                    "previous" => Action::Previous,
                    "parent" => Action::Parent,
                    _ => Action::Child,
                };
                match surface.apply(action) {
                    Ok(outcome) => Ok(vec![outcome.render()]),
                    Err(reason) => Err(reason),
                }
            }
            "press" => match Chord::parse(argument) {
                Ok(chord) => match surface.press_and_apply(chord.keys(), &context) {
                    Ok(outcome) => Ok(vec![format!("{argument}: {}", outcome.render())]),
                    Err(reason) => Err(reason),
                },
                Err(reason) => Err(reason),
            },
            "stress" => match parse_pair(argument) {
                Ok((nodes, rounds)) => match run_stress(nodes, rounds) {
                    Ok(line) => Ok(vec![line]),
                    Err(reason) => Err(reason),
                },
                Err(reason) => Err(reason),
            },
            "state" => Ok(surface.render()),
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a design operation")),
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
    for line in surface.render() {
        println!("{line}");
    }
    true
}

/// Builds `nodes` nodes, then runs `rounds` delete/undo cycles and reports the footprint.
pub(super) fn run_stress(nodes: usize, rounds: usize) -> Result<String, String> {
    use crate::app::design::{Action, Surface};

    let mut surface = Surface::bounded(64);
    let panel = surface.add(None, crate::app::structure::Kind::Container, "panel")?;
    for index in 0..nodes {
        surface.add(
            Some(panel),
            crate::app::structure::Kind::Widget,
            &format!("n{index}"),
        )?;
    }
    let baseline = surface.footprint();
    for _ in 0..rounds {
        surface.apply(Action::SelectAll)?;
        surface.apply(Action::Delete)?;
        if surface.footprint().nodes != 0 {
            return Err("a delete left nodes behind".to_owned());
        }
        surface.apply(Action::Undo)?;
        if surface.footprint().nodes != baseline.nodes {
            return Err("an undo did not restore the surface".to_owned());
        }
    }
    let footprint = surface.footprint();
    Ok(format!(
        "stress: {} node(s), {} round(s), history {} past {}, agrees {}",
        footprint.nodes,
        rounds,
        footprint.past,
        footprint.future,
        surface.agrees()
    ))
}

/// A `(parent, kind, name)` from `<parent|->:<kind>:<name>`.
pub(super) fn parse_design_node(
    argument: &str,
) -> Result<
    (
        Option<crate::contracts::identity::NodeId>,
        crate::app::structure::Kind,
        String,
    ),
    String,
> {
    let fields: Vec<&str> = argument.splitn(3, ':').collect();
    let [parent, kind, name] = fields.as_slice() else {
        return Err(format!("{argument:?} is not <parent|->:<kind>:<name>"));
    };
    let parent = if *parent == "-" {
        None
    } else {
        Some(parse_scene_node(parent)?)
    };
    Ok((parent, parse_kind(kind)?, (*name).to_owned()))
}

/// A pair of whole numbers from `<a>:<b>`.
pub(super) fn parse_pair(argument: &str) -> Result<(usize, usize), String> {
    let (first, second) = argument
        .split_once(':')
        .ok_or_else(|| format!("{argument:?} is not <a>:<b>"))?;
    let number = |value: &str| -> Result<usize, String> {
        value
            .parse()
            .map_err(|_| format!("{value:?} is not a whole number"))
    };
    Ok((number(first)?, number(second)?))
}
