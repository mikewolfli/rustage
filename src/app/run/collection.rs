// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! F27: collection content, table columns, tree items, cell delegates, navigation containers, rich
//! content, specialised views and the content regression.
//!
//! One command handler per parsed command of this domain; the dispatcher lives in [`super`](self).
//! Where a command can edit real content it does; where it cannot — a view the library does not
//! support, a link the permission gate denies, a source row that does not map — it prints the refusal
//! rather than a success shape.

use super::*;

/// Splits `<a>:<b>` into two numbers.
fn split_two(rest: &str, what: &str) -> Result<(u64, u64), String> {
    let (first, second) = rest
        .split_once(':')
        .ok_or_else(|| format!("{what} needs <a>:<b>, got {rest:?}"))?;
    match (first.parse::<u64>(), second.parse::<u64>()) {
        (Ok(first), Ok(second)) => Ok((first, second)),
        _ => Err(format!("{what} needs two numbers, got {rest:?}")),
    }
}

/// Splits `<a>:<b>:<value>` into two numbers and the rest.
fn split_three<'a>(rest: &'a str, what: &str) -> Result<(u64, u64, &'a str), String> {
    let mut parts = rest.splitn(3, ':');
    let first = parts.next().unwrap_or_default();
    let second = parts
        .next()
        .ok_or_else(|| format!("{what} needs <a>:<b>:<value>, got {rest:?}"))?;
    let value = parts
        .next()
        .ok_or_else(|| format!("{what} needs <a>:<b>:<value>, got {rest:?}"))?;
    match (first.parse::<u64>(), second.parse::<u64>()) {
        (Ok(first), Ok(second)) => Ok((first, second, value)),
        _ => Err(format!("{what} needs two numbers first, got {rest:?}")),
    }
}

/// Reads an item identity.
fn id_of(text: &str) -> Result<u64, String> {
    text.parse::<u64>()
        .map_err(|_| format!("{text:?} is not an item id"))
}

/// Reads a parent identity, where `-` means "no parent".
fn parent_id(text: &str) -> Result<Option<u64>, String> {
    if text == "-" {
        return Ok(None);
    }
    id_of(text).map(Some)
}

/// Edits a collection's content model (task T27.01).
pub(super) fn collection(options: &crate::cli::CollectionOptions) -> bool {
    use crate::app::collection::content::{CellRole, Collection, ColumnId, Origin, RowId};
    use crate::contracts::identity::NodeId;

    let mut content = Collection::new(NodeId::from_raw(0), Origin::Sample);
    let mut business = false;
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<(), String> = match verb {
            "control" => match argument.parse::<u64>() {
                Ok(value) => {
                    content.control = NodeId::from_raw(value);
                    Ok(())
                }
                Err(_) => Err(format!("{argument:?} is not a control id")),
            },
            "origin" => match Origin::parse(argument) {
                Some(origin) => {
                    content.origin = origin;
                    Ok(())
                }
                None => Err(format!("{argument:?} is not sample or source")),
            },
            "column" => match argument.split_once(':') {
                Some((role, header)) => match role {
                    "text" => {
                        content.add_column(header, CellRole::Text);
                        Ok(())
                    }
                    "check" => {
                        content.add_column(header, CellRole::Check);
                        Ok(())
                    }
                    "badge" => {
                        content.add_column(header, CellRole::Badge(header.to_owned()));
                        Ok(())
                    }
                    other => Err(format!("{other:?} is not a column role")),
                },
                None => Err(format!("column needs <role>:<header>, got {argument:?}")),
            },
            "row" => {
                content.add_row();
                Ok(())
            }
            "set" => match split_three(argument, "set") {
                Ok((row, column, value)) => {
                    content.set(RowId::from_raw(row), ColumnId::from_raw(column), value)
                }
                Err(reason) => Err(reason),
            },
            "move-row" => match split_two(argument, "move-row") {
                Ok((from, to)) => content.move_row(from as usize, to as usize),
                Err(reason) => Err(reason),
            },
            "move-column" => match split_two(argument, "move-column") {
                Ok((from, to)) => content.move_column(from as usize, to as usize),
                Err(reason) => Err(reason),
            },
            "business" => {
                business = true;
                Ok(())
            }
            "show" => Ok(()),
            other => Err(format!("{other:?} is not a collection operation")),
        };
        if let Err(reason) = step {
            return report(reason);
        }
    }
    for line in content.render() {
        println!("{line}");
    }
    if business {
        match content.business_rows() {
            Ok(rows) => println!("business rows: {}", rows.len()),
            Err(reason) => return report(reason),
        }
    }
    match content.validate() {
        Ok(()) => true,
        Err(reason) => report(reason),
    }
}

/// Edits a table's columns and maps its rows (task T27.02).
pub(super) fn table(options: &crate::cli::TableOptions) -> bool {
    use crate::app::collection::table::{ColumnType, Sort, SourceRow, Table, map_all};

    let mut table = Table::new();
    let mut sources: Vec<SourceRow> = Vec::new();
    let mut map_now = false;
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<(), String> = match verb {
            "column" => match argument.split_once(':') {
                Some((name, kind)) => match ColumnType::parse(kind) {
                    Some(kind) => table.add_column(name, kind, 120),
                    None => Err(format!("{kind:?} is not a column type")),
                },
                None => Err(format!("column needs <name>:<type>, got {argument:?}")),
            },
            "width" => match argument.split_once(':') {
                Some((name, width)) => match width.parse::<u32>() {
                    Ok(width) => table.set_width(name, width),
                    Err(_) => Err(format!("{width:?} is not a width")),
                },
                None => Err(format!("width needs <name>:<n>, got {argument:?}")),
            },
            "sort" => match argument.split_once(':') {
                Some((name, direction)) => match Sort::parse(direction) {
                    Some(direction) => table.sort(name, direction),
                    None => Err(format!("{direction:?} is not asc, desc or none")),
                },
                None => Err(format!(
                    "sort needs <name>:<asc|desc|none>, got {argument:?}"
                )),
            },
            "filter" => match argument.split_once(':') {
                Some((name, contains)) => table.filter(name, contains),
                None => Err(format!("filter needs <name>:<text>, got {argument:?}")),
            },
            "select" => match argument.parse::<usize>() {
                Ok(index) => {
                    table.selection.toggle(index);
                    Ok(())
                }
                Err(_) => Err(format!("{argument:?} is not a row index")),
            },
            "empty" => {
                table.empty_message = argument.to_owned();
                Ok(())
            }
            "source" => {
                sources.push(SourceRow::new(argument));
                Ok(())
            }
            "field" => match (sources.last_mut(), argument.split_once(':')) {
                (Some(row), Some((name, raw))) => {
                    row.set(name, raw);
                    Ok(())
                }
                (None, _) => Err("field needs a source row first".to_owned()),
                (_, None) => Err(format!("field needs <name>:<raw>, got {argument:?}")),
            },
            "map" => {
                map_now = true;
                Ok(())
            }
            "show" => Ok(()),
            other => Err(format!("{other:?} is not a table operation")),
        };
        if let Err(reason) = step {
            return report(reason);
        }
    }
    for line in table.render() {
        println!("{line}");
    }
    if map_now {
        let reports = map_all(&table.columns, &sources);
        let mut ok = true;
        for summary in &reports {
            match &summary.row {
                Ok(row) => println!("{}: {} cell(s)", row.key, row.cells.len()),
                Err(diagnoses) => {
                    ok = false;
                    for diagnosis in diagnoses {
                        println!("{}", diagnosis.render());
                    }
                }
            }
        }
        if !ok {
            return false;
        }
    }
    true
}

/// Edits a tree's items and loads them (task T27.03).
pub(super) fn tree(options: &crate::cli::TreeOptions) -> bool {
    use crate::app::collection::tree::{IconRole, ItemId, Tree};
    use crate::app::data::adapters::{Adapter, Cancel, Outcome};

    let mut tree = Tree::new();
    let mut answers: Vec<Outcome> = Vec::new();
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<(), String> = match verb {
            "node" => match argument.split_once(':') {
                Some((id, rest)) => match rest.split_once(':') {
                    Some((parent, label)) => match (id_of(id), parent_id(parent)) {
                        (Ok(id), Ok(parent)) => tree.add(
                            ItemId::from_raw(id),
                            parent.map(ItemId::from_raw),
                            label,
                            IconRole::Leaf,
                        ),
                        (Err(reason), _) => Err(reason),
                        (_, Err(reason)) => Err(reason),
                    },
                    None => Err(format!(
                        "node needs <id>:<parent|->:<label>, got {argument:?}"
                    )),
                },
                None => Err(format!(
                    "node needs <id>:<parent|->:<label>, got {argument:?}"
                )),
            },
            "icon" => match argument.split_once(':') {
                Some((id, role)) => match (id_of(id), IconRole::parse(role)) {
                    (Ok(id), Some(role)) => match tree.node_mut(ItemId::from_raw(id)) {
                        Some(node) => {
                            node.icon = role;
                            Ok(())
                        }
                        None => Err(format!("item {id} does not exist")),
                    },
                    (Err(reason), _) => Err(reason),
                    (_, None) => Err(format!("{role:?} is not an icon role")),
                },
                None => Err(format!("icon needs <id>:<role>, got {argument:?}")),
            },
            "expand" => id_of(argument).and_then(|id| tree.expand(ItemId::from_raw(id))),
            "collapse" => id_of(argument).and_then(|id| tree.collapse(ItemId::from_raw(id))),
            "select" => id_of(argument).and_then(|id| tree.select(ItemId::from_raw(id))),
            "lazy" => id_of(argument).and_then(|id| tree.make_lazy(ItemId::from_raw(id))),
            "reparent" => match argument.split_once(':') {
                Some((id, parent)) => match (id_of(id), parent_id(parent)) {
                    (Ok(id), Ok(parent)) => {
                        tree.reparent(ItemId::from_raw(id), parent.map(ItemId::from_raw))
                    }
                    (Err(reason), _) => Err(reason),
                    (_, Err(reason)) => Err(reason),
                },
                None => Err(format!("reparent needs <id>:<parent|->, got {argument:?}")),
            },
            "answer" => match argument.split_once(':') {
                Some((kind, text)) => {
                    let value = match kind {
                        "ok" => Some(Outcome::Ok(text.to_owned())),
                        "fail" => Some(Outcome::Failed(text.to_owned())),
                        "cancel" => Some(Outcome::Cancelled),
                        "unknown" => Some(Outcome::Unknown(text.to_owned())),
                        _ => None,
                    };
                    match value {
                        Some(value) => {
                            answers.push(value);
                            Ok(())
                        }
                        None => Err(format!("{kind:?} is not ok, fail, cancel or unknown")),
                    }
                }
                None => Err(format!("answer needs <kind>:<text>, got {argument:?}")),
            },
            "load" => match argument.split_once(':') {
                Some((id, target)) => match id_of(id) {
                    Ok(id) => {
                        let mut adapter = Adapter::sample(&answers);
                        let outcome =
                            tree.load(ItemId::from_raw(id), target, &mut adapter, &Cancel::new());
                        answers.clear();
                        match outcome {
                            Ok(outcome) => {
                                println!("load: {}", outcome.render());
                                Ok(())
                            }
                            Err(reason) => Err(reason),
                        }
                    }
                    Err(reason) => Err(reason),
                },
                None => Err(format!("load needs <id>:<target>, got {argument:?}")),
            },
            "show" => Ok(()),
            other => Err(format!("{other:?} is not a tree operation")),
        };
        if let Err(reason) = step {
            return report(reason);
        }
    }
    for line in tree.render() {
        println!("{line}");
    }
    match tree.validate() {
        Ok(()) => true,
        Err(reason) => report(reason),
    }
}

/// Edits a cell and commits or cancels it (task T27.04).
pub(super) fn cell_edit(options: &crate::cli::CellEditOptions) -> bool {
    use crate::app::collection::delegate::{DelegateKind, Edit, Saves};

    let mut row = 0usize;
    let mut column = String::new();
    let mut current = String::new();
    let mut candidate = String::new();
    let mut kind = DelegateKind::Text;
    let mut saves = Saves::new();
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<(), String> = match verb {
            "row" => match argument.parse::<usize>() {
                Ok(value) => {
                    row = value;
                    Ok(())
                }
                Err(_) => Err(format!("{argument:?} is not a row index")),
            },
            "column" => {
                column = argument.to_owned();
                Ok(())
            }
            "current" => {
                current = argument.to_owned();
                Ok(())
            }
            "candidate" => {
                candidate = argument.to_owned();
                Ok(())
            }
            "delegate" => match DelegateKind::parse(argument) {
                Some(value) => {
                    kind = value;
                    Ok(())
                }
                None => Err(format!("{argument:?} is not a delegate")),
            },
            "commit" => {
                let mut edit = Edit::begin(row, &column, &current);
                edit.set(&candidate);
                let committed = edit.commit(&kind, &mut saves);
                println!("{}", edit.render());
                if committed {
                    Ok(())
                } else {
                    Err(format!("the edit at {row}:{column} was refused"))
                }
            }
            "cancel" => {
                let mut edit = Edit::begin(row, &column, &current);
                edit.set(&candidate);
                edit.cancel();
                println!("{}", edit.render());
                Ok(())
            }
            "show" => Ok(()),
            other => Err(format!("{other:?} is not a cell-edit operation")),
        };
        if let Err(reason) = step {
            return report(reason);
        }
    }
    println!("saves recorded: {}", saves.len());
    true
}

/// Manages a container's pages (task T27.05).
pub(super) fn container(options: &crate::cli::ContainerOptions) -> bool {
    use crate::app::collection::pages::{ContainerKind, Page, Pages};
    use crate::contracts::identity::{NodeId, PageId};

    let mut pages = Pages::new(ContainerKind::Tabs);
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<(), String> = match verb {
            "kind" => match ContainerKind::parse(argument) {
                Some(kind) => {
                    pages.kind = kind;
                    Ok(())
                }
                None => Err(format!("{argument:?} is not a container kind")),
            },
            "page" => match argument.split_once(':') {
                Some((id, rest)) => match rest.split_once(':') {
                    Some((title, node)) => match (id.parse::<u64>(), node.parse::<u64>()) {
                        (Ok(id), Ok(node)) => pages.add(Page {
                            id: PageId::from_raw(id),
                            title: title.to_owned(),
                            node: NodeId::from_raw(node),
                        }),
                        _ => Err(format!(
                            "page needs numeric <id> and <node>, got {argument:?}"
                        )),
                    },
                    None => Err(format!("page needs <id>:<title>:<node>, got {argument:?}")),
                },
                None => Err(format!("page needs <id>:<title>:<node>, got {argument:?}")),
            },
            "activate" => match argument.parse::<u64>() {
                Ok(id) => pages.activate(PageId::from_raw(id)),
                Err(_) => Err(format!("{argument:?} is not a page id")),
            },
            "remove" => match argument.parse::<u64>() {
                Ok(id) => pages.remove(PageId::from_raw(id)),
                Err(_) => Err(format!("{argument:?} is not a page id")),
            },
            "reorder" => match argument.split_once(':') {
                Some((id, to)) => match (id.parse::<u64>(), to.parse::<usize>()) {
                    (Ok(id), Ok(to)) => pages.reorder(PageId::from_raw(id), to),
                    _ => Err(format!("reorder needs <id>:<to>, got {argument:?}")),
                },
                None => Err(format!("reorder needs <id>:<to>, got {argument:?}")),
            },
            "next" => pages.next().map(|_| ()),
            "previous" => pages.previous().map(|_| ()),
            "show" => Ok(()),
            other => Err(format!("{other:?} is not a container operation")),
        };
        if let Err(reason) = step {
            return report(reason);
        }
    }
    for line in pages.render() {
        println!("{line}");
    }
    true
}

/// Edits rich content and executes its links (task T27.06).
pub(super) fn rich_content(options: &crate::cli::RichContentOptions) -> bool {
    use crate::app::collection::rich::{
        Font, Format, Image, Limits, Link, RichText, Run, execute_link,
    };
    use crate::contracts::permission::{Capability, PermissionGate, Principal};

    let mut gate = PermissionGate::new();
    let mut runs: Vec<Run> = Vec::new();
    let mut font = Font {
        family: "Inter".to_owned(),
        size: 14,
        weight: 400,
    };
    let mut limits = Limits::permissive(4096);
    let mut formats_given = false;
    let mut link: Option<Link> = None;
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<(), String> = match verb {
            "font" => {
                let mut parts = argument.splitn(3, ':');
                let family = parts.next().unwrap_or_default();
                let size = parts.next();
                let weight = parts.next();
                match (size, weight) {
                    (Some(size), Some(weight)) => {
                        match (size.parse::<u32>(), weight.parse::<u32>()) {
                            (Ok(size), Ok(weight)) => match Font::new(family, size, weight) {
                                Ok(chosen) => {
                                    font = chosen;
                                    Ok(())
                                }
                                Err(reason) => Err(reason),
                            },
                            _ => Err(format!(
                                "font needs numeric <size> and <weight>, got {argument:?}"
                            )),
                        }
                    }
                    _ => Err(format!(
                        "font needs <family>:<size>:<weight>, got {argument:?}"
                    )),
                }
            }
            "run" => match argument.split_once(':') {
                Some((format, text)) => match Format::parse(format) {
                    Some(format) => Run::new(text, &[format]).map(|run| {
                        runs.push(run);
                    }),
                    None => Err(format!("{format:?} is not a format")),
                },
                None => Err(format!("run needs <format>:<text>, got {argument:?}")),
            },
            "link" => match argument.split_once(':') {
                Some((target, alt)) => match Link::new(target, alt) {
                    Ok(value) => {
                        link = Some(value);
                        Ok(())
                    }
                    Err(reason) => Err(reason),
                },
                None => Err(format!("link needs <target>:<alt>, got {argument:?}")),
            },
            "image" => match argument.split_once(':') {
                Some((asset, alt)) => Image::new(asset, alt).map(|_| ()),
                None => Err(format!("image needs <asset>:<alt>, got {argument:?}")),
            },
            "limit-length" => match argument.parse::<usize>() {
                Ok(value) => {
                    limits.max_length = value;
                    Ok(())
                }
                Err(_) => Err(format!("{argument:?} is not a length")),
            },
            "limit-format" => match Format::parse(argument) {
                Some(format) => {
                    if !formats_given {
                        limits.formats.clear();
                        formats_given = true;
                    }
                    if !limits.formats.contains(&format) {
                        limits.formats.push(format);
                    }
                    Ok(())
                }
                None => Err(format!("{argument:?} is not a format")),
            },
            "trust" => {
                gate.trust_project();
                Ok(())
            }
            "network" => match argument {
                "yes" | "true" => {
                    gate.set_network_policy(true);
                    Ok(())
                }
                "no" | "false" => {
                    gate.set_network_policy(false);
                    Ok(())
                }
                other => Err(format!("{other:?} is not yes or no")),
            },
            "authorize" => {
                gate.authorize(Capability::RequestNetworkService);
                Ok(())
            }
            "execute-link" => match &link {
                Some(link) => match execute_link(&gate, Principal::Human, link) {
                    Ok(line) => {
                        println!("{line}");
                        Ok(())
                    }
                    Err(reason) => Err(reason),
                },
                None => Err("execute-link needs a link first".to_owned()),
            },
            "show" => Ok(()),
            other => Err(format!("{other:?} is not a rich-content operation")),
        };
        if let Err(reason) = step {
            return report(reason);
        }
    }
    let body = RichText { runs, font };
    for line in body.render() {
        println!("{line}");
    }
    for line in limits.report() {
        println!("{line}");
    }
    match body.validate(&limits) {
        Ok(()) => true,
        Err(violations) => {
            for violation in violations {
                println!("{violation}");
            }
            false
        }
    }
}

/// Describes a specialised view and its support (task T27.07).
pub(super) fn view(options: &crate::cli::ViewOptions) -> bool {
    use crate::app::collection::view::{View, ViewKind, capability};

    let mut kind: Option<ViewKind> = None;
    let mut data: Vec<String> = Vec::new();
    let mut ask_capability = false;
    let mut ask_editor = false;
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<(), String> = match verb {
            "kind" => match ViewKind::parse(argument) {
                Some(value) => {
                    kind = Some(value);
                    Ok(())
                }
                None => Err(format!("{argument:?} is not a view kind")),
            },
            "data" => {
                data.push(argument.to_owned());
                Ok(())
            }
            "capability" => {
                ask_capability = true;
                Ok(())
            }
            "editor" => {
                ask_editor = true;
                Ok(())
            }
            "show" => Ok(()),
            other => Err(format!("{other:?} is not a view operation")),
        };
        if let Err(reason) = step {
            return report(reason);
        }
    }
    let Some(kind) = kind else {
        return report("no view kind was given".to_owned());
    };
    let support = capability(kind);
    if ask_capability {
        println!("capability: {}", support.render());
    }
    if ask_editor {
        match View::editor(kind) {
            Some(editor) => println!("editor fields: {}", editor.fields.join(", ")),
            None => {
                return report(format!(
                    "no editor is offered for {}: {}",
                    kind.name(),
                    support.reason().unwrap_or("unsupported")
                ));
            }
        }
    }
    let chosen = View { kind, data };
    for line in chosen.render() {
        println!("{line}");
    }
    support.is_supported()
}

/// Runs the collection content regression (task T27.08).
pub(super) fn collection_regression(options: &crate::cli::CollectionRegressionOptions) -> bool {
    use crate::app::collection::regression::{
        GridState, Mode, Viewport, keyboard, render_window, window,
    };

    let mut total = 0usize;
    let mut height = 10usize;
    let mut first = 0usize;
    let mut mode = Mode::Static;
    let mut locale = "en".to_owned();
    let mut empty: Option<String> = None;
    let mut error: Option<String> = None;
    let mut selection: Vec<usize> = Vec::new();
    let mut edits: Vec<(usize, String)> = Vec::new();
    let mut key: Option<String> = None;
    let mut check = false;
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<(), String> = match verb {
            "total" => match argument.parse::<usize>() {
                Ok(value) => {
                    total = value;
                    Ok(())
                }
                Err(_) => Err(format!("{argument:?} is not a count")),
            },
            "viewport" => match argument.parse::<usize>() {
                Ok(value) => {
                    height = value;
                    Ok(())
                }
                Err(_) => Err(format!("{argument:?} is not a height")),
            },
            "first" => match argument.parse::<usize>() {
                Ok(value) => {
                    first = value;
                    Ok(())
                }
                Err(_) => Err(format!("{argument:?} is not a row index")),
            },
            "mode" => match Mode::parse(argument) {
                Some(value) => {
                    mode = value;
                    Ok(())
                }
                None => Err(format!("{argument:?} is not static or live")),
            },
            "locale" => {
                locale = argument.to_owned();
                Ok(())
            }
            "empty" => {
                empty = Some(argument.to_owned());
                Ok(())
            }
            "error" => {
                error = Some(argument.to_owned());
                Ok(())
            }
            "select" => match argument.parse::<usize>() {
                Ok(index) => {
                    selection.push(index);
                    Ok(())
                }
                Err(_) => Err(format!("{argument:?} is not a row index")),
            },
            "edit" => match argument.split_once(':') {
                Some((index, value)) => match index.parse::<usize>() {
                    Ok(index) => {
                        edits.push((index, value.to_owned()));
                        Ok(())
                    }
                    Err(_) => Err(format!("{index:?} is not a row index")),
                },
                None => Err(format!("edit needs <index>:<value>, got {argument:?}")),
            },
            "key" => {
                key = Some(argument.to_owned());
                Ok(())
            }
            "check" => {
                check = true;
                Ok(())
            }
            "show" => Ok(()),
            other => Err(format!(
                "{other:?} is not a collection-regression operation"
            )),
        };
        if let Err(reason) = step {
            return report(reason);
        }
    }
    let state = match (&error, &empty) {
        (Some(reason), _) => GridState::Error(reason.clone()),
        (None, Some(message)) => GridState::Empty(message.clone()),
        (None, None) => GridState::Ready,
    };
    let mut viewport = match Viewport::new(height) {
        Ok(viewport) => viewport.scrolled_to(first),
        Err(reason) => return report(reason),
    };
    if let Some(command) = &key {
        viewport = match keyboard(command, total, &viewport) {
            Ok(next) => next,
            Err(reason) => return report(reason),
        };
    }
    let visible = window(total, &viewport);
    let lines = render_window(total, &viewport, &state, |index| {
        match edits.iter().rev().find(|(row, _)| *row == index) {
            Some((_, value)) => value.clone(),
            None => format!("{locale} row {index}"),
        }
    });
    for line in &lines {
        println!("{line}");
    }
    println!(
        "shown {}..{} of {total} in {} mode, locale {locale}",
        visible.first,
        visible.end(),
        mode.name()
    );
    if !selection.is_empty() {
        println!(
            "selected: {}",
            selection
                .iter()
                .map(|index| index.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    if check && lines.len() > height + 1 {
        return report("the window rendered more rows than the viewport holds".to_owned());
    }
    true
}
