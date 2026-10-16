// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! F04/F05: the workbench, commands, menus, keys and the clipboard.
//!
//! One command handler per parsed command of this domain; the dispatcher lives in
//! [`super`](self).

use super::*;

/// Resolves a command through the typed registry (task T05.01).
///
/// The registry is the only door: a [`Call`](crate::app::command::Call) becomes an
/// [`Invocation`](crate::app::command::Invocation) only through `Registry::resolve`, which checks that
/// the command is known, available in this context, authorized, and given valid arguments — in that
/// order, because "not available" is a more useful answer than "one argument would have been wrong"
/// for a command that was never going to run. With no id, every registered command is listed.
pub(super) fn resolve_command(options: &crate::cli::CommandOptions) -> bool {
    use crate::app::command::{Call, Context};
    use crate::contracts::permission::EffectClass;

    let registry = crate::app::command::built_in();

    let Some(id) = &options.id else {
        println!("{} command(s):", registry.len());
        for command in registry.commands() {
            let params = command
                .params()
                .iter()
                .map(|param| format!("{}: {}", param.name(), param.kind().name()))
                .collect::<Vec<_>>()
                .join(", ");
            println!(
                "  {:<18} {:<10} {:<14} undo {}{}",
                command.id().to_string(),
                command.category(),
                command.title(),
                command.undo().name(),
                if params.is_empty() {
                    String::new()
                } else {
                    format!("  arguments: {params}")
                }
            );
            if !command.help().is_empty() {
                println!("      {}", command.help());
            }
        }
        return true;
    };

    let effects: Vec<EffectClass> = options
        .authorize
        .iter()
        .filter_map(|name| {
            EffectClass::ALL
                .iter()
                .find(|effect| effect.name() == name)
                .copied()
        })
        .collect();
    let mut context = Context::in_area(&options.focus);
    if options.trust {
        context = context.trusted();
    }
    for effect in effects {
        context = context.authorizing(effect);
    }
    for task in &options.running {
        context = context.running(task.clone());
    }

    let mut call = Call::to(id.clone());
    for (name, value) in &options.arguments {
        call = call.with(name.clone(), value.clone());
    }

    println!(
        "context: focus {}, trusted {}",
        options.focus,
        yes_no(options.trust)
    );
    match registry.resolve(&call, &context) {
        Ok(invocation) => {
            println!("resolved: {}", invocation.describe());
            println!("title: {}", invocation.title());
            if let Some(task) = invocation.task() {
                println!("runs as task: {task}");
            }
            true
        }
        Err(refusal) => {
            println!("refused: {}", refusal.message());
            false
        }
    }
}

/// Resolves a key chord through the keymap (task T05.03).
///
/// The scene decides whether a bare key is a character or a command: with `--text-field` a bare
/// character goes to the field, and with `--composing` the keys belong to the input method. A chord
/// resolves to a command, waits for more keys, is suppressed, or is unbound — and each is reported as
/// itself rather than as a success.
pub(super) fn keys(options: &crate::cli::KeysOptions) -> bool {
    use crate::app::shortcuts::{Chord, Key, Resolution, Scene};

    let keymap = crate::app::shortcuts::built_in();
    let Some(press) = &options.press else {
        println!("{} binding(s):", keymap.len());
        for binding in keymap.bindings() {
            println!(
                "  {:<14} {:<18} {}",
                binding.chord().to_string(),
                binding.command(),
                binding.scope().render()
            );
        }
        return true;
    };

    let chord = match Chord::parse(press) {
        Ok(chord) => chord,
        Err(reason) => {
            println!("rustage: {reason}");
            return false;
        }
    };
    let mut scene = Scene::in_mode(&options.mode, options.panel.clone());
    if options.text_field {
        scene = Scene::text_field(&options.mode);
    }
    if options.composing {
        scene = scene.composing();
    }

    let pressed: Vec<Key> = chord.keys().to_vec();
    println!(
        "scene: mode {}, panel {}, text field {}, composing {}",
        options.mode,
        options.panel.as_deref().unwrap_or("(none)"),
        yes_no(options.text_field),
        yes_no(options.composing)
    );
    match keymap.resolve(&pressed, &scene) {
        Resolution::Triggered { command } => {
            println!("pressed {chord}: runs {command}");
            true
        }
        Resolution::Pending {
            pressed,
            candidates,
        } => {
            let so_far: Vec<String> = pressed.iter().map(Key::to_string).collect();
            println!(
                "pressed {}: waiting for more keys (could be {})",
                so_far.join(" "),
                candidates.join(", ")
            );
            true
        }
        Resolution::Suppressed { reason } => {
            println!("pressed {chord}: no command ({reason})");
            true
        }
        Resolution::Nothing => {
            println!("pressed {chord}: nothing is bound to it");
            false
        }
    }
}

/// Shows the menu tree and checks every implemented leaf against the registry (task T05.02).
///
/// The check is the acceptance line: an implemented leaf must bind a command the registry knows, no
/// two leaves may share an id, and a leaf marked planned must not name one that works. A problem is
/// printed with the menu path, so a report points at the item rather than at the file.
pub(super) fn menu() -> bool {
    let registry = crate::app::command::built_in();
    let menus = crate::app::menu::built_in();
    let mut problems = Vec::new();
    for menu in &menus {
        for line in menu.render() {
            println!("{line}");
        }
        problems.extend(menu.problems(&registry));
    }
    if problems.is_empty() {
        println!("every implemented menu leaf binds one registered command");
        true
    } else {
        for problem in problems {
            println!("problem: {problem}");
        }
        false
    }
}

/// Searches the command palette, or shows one command's help (task T05.04).
///
/// Every hit is a registry command with the registry's own verdict, so a disabled command shows its
/// reason rather than a grey row. An empty query lists every command, which is what makes a new
/// command discoverable from the keyboard; a non-empty query that matches nothing is a finding.
pub(super) fn palette(options: &crate::cli::PaletteOptions) -> bool {
    use crate::app::command::Context;
    use crate::app::palette::{Recents, help_for, search};
    use crate::contracts::permission::EffectClass;

    let registry = crate::app::command::built_in();

    if let Some(id) = &options.help_for {
        return match help_for(&registry, id) {
            Ok(help) => {
                for line in help.render() {
                    println!("{line}");
                }
                true
            }
            Err(reason) => {
                println!("rustage: {reason}");
                false
            }
        };
    }

    let mut context = Context::in_area(&options.focus);
    if options.trust {
        context = context.trusted();
    }
    for name in &options.authorize {
        if let Some(effect) = EffectClass::ALL.iter().find(|effect| effect.name() == name) {
            context = context.authorizing(*effect);
        }
    }
    for task in &options.running {
        context = context.running(task.clone());
    }

    let mut recents = Recents::new(20);
    // The list is given most-recent-first, so recording in reverse yields the same order.
    for id in options.recent.iter().rev() {
        recents.record(id.clone());
    }

    let hits = search(&registry, &context, &options.query, &recents);
    println!(
        "{} hit(s) for {:?} (focus {}, trusted {})",
        hits.len(),
        options.query,
        options.focus,
        yes_no(options.trust)
    );
    for entry in &hits {
        println!("  {}", entry.render());
    }
    !(hits.is_empty() && !options.query.is_empty())
}

/// Shows the workbench and its panels (tasks T04.01/T04.02).
///
/// The workbench's title, regions and status are derived from the identities given here, so the report
/// cannot say one thing while the state says another. The panels are the registered set with the
/// layout changes applied: a panel whose service this host does not have is never shown, and hiding
/// and showing a panel restores the size and focus it had.
pub(super) fn workbench(options: &crate::cli::WorkbenchOptions) -> bool {
    use crate::app::panels::{PanelError, built_in as panels_built_in};
    use crate::app::workbench::{Mode, Workbench};
    use std::collections::BTreeSet;

    let mut workbench = Workbench::new();
    if let Some(name) = &options.mode {
        match Mode::from_name(name) {
            Some(mode) => workbench.set_mode(mode),
            None => {
                println!("rustage: {name:?} is not a mode");
                return false;
            }
        }
    }
    if let Some(project) = &options.project {
        workbench.open_project(project.clone());
    }
    if let Some(target) = &options.target {
        workbench.set_target(target.clone());
    }
    if let Some(document) = &options.document {
        workbench.open_document(document.clone());
    }
    if options.dirty {
        workbench.mark_dirty();
    }
    if let Some(message) = &options.message {
        workbench.set_message(message.clone());
    }
    for line in workbench.render() {
        println!("{line}");
    }

    let services: BTreeSet<String> = options.services.iter().cloned().collect();
    let mut panels = panels_built_in();
    let mut problem: Option<PanelError> = None;
    for id in &options.show {
        problem = problem.or_else(|| panels.show(id).err());
    }
    for id in &options.hide {
        problem = problem.or_else(|| panels.hide(id).err());
    }
    for (id, size) in &options.resize {
        problem = problem.or_else(|| panels.resize(id, *size).err());
    }
    if let Some(id) = &options.focus {
        problem = problem.or_else(|| panels.focus(id).err());
    }
    println!();
    for line in panels.render(&services) {
        println!("{line}");
    }
    if let Some(problem) = problem {
        println!("rustage: {}", problem.message());
        return false;
    }
    true
}

/// Previews and carries out a batch operation on a selection (task T05.07).
///
/// The preview classifies every selected item before anything happens, and the permission is explained
/// before it is checked. `apply-all` over a selection with an inapplicable item is **refused**, naming
/// them: skipping one is `apply-none`, a choice, and it is recorded in the history so a later "why is
/// this one missing" has an answer.
pub(super) fn operation(options: &crate::cli::OperationOptions) -> bool {
    use crate::app::operations::{BatchPreview, Choice, History, Operation, Verdict};
    use crate::contracts::permission::EffectClass;
    use std::collections::BTreeSet;
    use std::time::{SystemTime, UNIX_EPOCH};

    let mut operation = Operation::new(options.name.clone());
    if let Some(note) = &options.note {
        operation = operation.noted(note.clone());
    }
    for name in &options.effects {
        match EffectClass::ALL.iter().find(|effect| effect.name() == name) {
            Some(effect) => operation = operation.with_effect(*effect),
            None => {
                println!("rustage: {name:?} is not an effect");
                return false;
            }
        }
    }
    let authorized: BTreeSet<EffectClass> = options
        .authorize
        .iter()
        .filter_map(|name| {
            EffectClass::ALL
                .iter()
                .find(|effect| effect.name() == name)
                .copied()
        })
        .collect();

    let mut items: Vec<(String, Verdict)> = options
        .select
        .iter()
        .map(|id| (id.clone(), Verdict::Applies))
        .collect();
    for (id, reason) in &options.inapplicable {
        items.push((id.clone(), Verdict::inapplicable(reason.clone())));
    }
    let preview = BatchPreview::of(operation, items);
    for line in preview.render(&authorized) {
        println!("{line}");
    }

    let Some(choice) = &options.choice else {
        println!("nothing was changed; pass --choice to carry it out");
        return true;
    };
    let Some(choice) = Choice::from_name(choice) else {
        println!("rustage: {choice:?} is not a choice (apply-all, apply-none, cancel)");
        return false;
    };
    match preview.apply(choice, &authorized) {
        Ok(outcome) => {
            println!("{}: {}", choice.name(), outcome.message());
            let mut history = History::new(20);
            let at = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos() as u64)
                .unwrap_or(0);
            if history.record(preview.operation().name(), &outcome, at) {
                for entry in history.entries() {
                    println!("history: {}", entry.render());
                }
            }
            true
        }
        Err(refusal) => {
            println!("refused: {}", refusal.message());
            false
        }
    }
}

/// Audits the command inventory (task T05.08).
///
/// The registry, the menu tree and the keymap are walked against each other: every menu leaf and every
/// key binding must name a registered command, and a registered command that nothing reaches is a note
/// (the palette searches the registry itself). A new area re-verifies by being added to one of the
/// three tables — there is no per-area checklist to remember.
pub(super) fn audit() -> bool {
    let registry = crate::app::command::built_in();
    let menus = crate::app::menu::built_in();
    let keymap = crate::app::shortcuts::built_in();
    let report = crate::app::audit::audit(&registry, &menus, &keymap);
    for line in report.render() {
        println!("{line}");
    }

    // T04.08: every offer (toolbar or key) has both a way to see it and a way to reach it.
    let sets = ["design", "code", "debug", "welcome"]
        .into_iter()
        .map(crate::app::chrome::toolbar_for)
        .collect::<Vec<_>>();
    let slices: Vec<&[String]> = sets.iter().map(Vec::as_slice).collect();
    let reach = crate::app::audit::discoverability(&registry, &menus, &slices, &keymap);
    for line in reach.render() {
        println!("{line}");
    }

    println!(
        "{} command(s), {} menu(s), {} binding(s)",
        registry.len(),
        menus.len(),
        keymap.len()
    );
    report.is_clean() && reach.is_clean()
}

/// Replays a script of document-tab operations (task T04.03).
///
/// The script is a story about one tab strip: a document previewed, another pinned, a view closed and
/// the document itself closed. Every operation is applied in order; an operation that names a tab
/// that is not there is reported and stops the run, so a script cannot silently do nothing.
pub(super) fn tabs(options: &crate::cli::TabsOptions) -> bool {
    use crate::app::tabs::Tabs;

    let mut tabs = Tabs::new(options.limit);
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let result: Result<String, String> = match verb {
            "preview" => {
                tabs.preview(argument);
                Ok(format!("preview {argument}"))
            }
            "pin" => {
                tabs.pin(argument);
                Ok(format!("pin {argument}"))
            }
            "view" => {
                tabs.another_view(argument);
                Ok(format!("another view of {argument}"))
            }
            "dirty" => {
                tabs.mark_dirty(argument, true);
                Ok(format!("{argument} has unsaved changes"))
            }
            "activate" | "close-view" | "close-others" | "close-right" => {
                match argument.parse::<usize>() {
                    Ok(index) => match verb {
                        "activate" => tabs.activate(index).map(|()| format!("activate {index}")),
                        "close-view" => tabs
                            .close_view(index)
                            .map(|tab| format!("closed the view of {}", tab.document())),
                        "close-others" => tabs
                            .close_others(index)
                            .map(|closed| format!("closed {closed} other(s)")),
                        _ => tabs
                            .close_to_the_right(index)
                            .map(|closed| format!("closed {closed} to the right")),
                    },
                    Err(_) => Err(format!("{op:?}: {argument:?} is not a tab index")),
                }
            }
            "close-doc" => {
                let closed = tabs.close_document(argument);
                Ok(format!("closed {closed} view(s) of {argument}"))
            }
            "move" => {
                let Some((from, to)) = argument.split_once(':') else {
                    return tabs_fail(&tabs, &format!("{op:?}: move takes `from:to`"));
                };
                match (from.parse::<usize>(), to.parse::<usize>()) {
                    (Ok(from), Ok(to)) => tabs
                        .reorder(from, to)
                        .map(|()| format!("moved {from} to {to}")),
                    _ => Err(format!("{op:?}: {argument:?} is not a move")),
                }
            }
            other => Err(format!("{other:?} is not a tab operation")),
        };
        match result {
            Ok(note) => println!("{note}"),
            Err(reason) => return tabs_fail(&tabs, &reason),
        }
    }
    for line in tabs.render() {
        println!("{line}");
    }
    true
}

/// Prints the strip as it stands and reports why the script stopped.
pub(super) fn tabs_fail(tabs: &crate::app::tabs::Tabs, reason: &str) -> bool {
    println!("rustage: {reason}");
    for line in tabs.render() {
        println!("{line}");
    }
    false
}

/// Replays a script of editor-group operations (task T04.04).
///
/// One editor area's story, one operation per call: a document opened, the area split, the same
/// document shown in both groups, a view scrolled, a tab moved across. The first refusal stops the
/// script and prints the area as it stands, so a story that goes wrong says where it went wrong
/// instead of a success shape around a half-run script (principle #22).
pub(super) fn groups(options: &crate::cli::GroupsOptions) -> bool {
    use crate::app::groups::{Axis, Direction, GroupId, Groups};

    let mut area = Groups::new(options.limit);
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let result: Result<String, String> = match verb {
            "split" => match Axis::from_name(argument) {
                Some(axis) => Ok(format!(
                    "split {}: group {}",
                    axis.name(),
                    area.split(axis).get()
                )),
                None => Err(format!(
                    "{argument:?} is not an axis (left-right, top-bottom)"
                )),
            },
            "next" => {
                area.focus_next();
                Ok(format!("focus group {}", area.focus().get()))
            }
            "prev" => {
                area.focus_previous();
                Ok(format!("focus group {}", area.focus().get()))
            }
            "left" | "right" | "up" | "down" => {
                let direction = Direction::from_name(verb).expect("the arm names a direction");
                if area.focus_direction(direction) {
                    Ok(format!(
                        "focus {}: group {}",
                        direction.name(),
                        area.focus().get()
                    ))
                } else {
                    Err(format!("there is no group {}", direction.name()))
                }
            }
            "focus" => match argument.parse::<u64>() {
                Ok(id) => area
                    .focus_group(GroupId::from_raw(id))
                    .map(|()| format!("focus group {id}")),
                Err(_) => Err(format!("{argument:?} is not a group id")),
            },
            "open" => {
                area.open(argument);
                Ok(format!("open {argument} in group {}", area.focus().get()))
            }
            "view" => {
                area.another_view(argument);
                Ok(format!(
                    "another view of {argument} in group {}",
                    area.focus().get()
                ))
            }
            "pin" => {
                area.pin(argument);
                Ok(format!("pin {argument} in group {}", area.focus().get()))
            }
            "type" => area
                .type_into(argument)
                .map(|()| format!("typed into {argument:?}")),
            "scroll" => match argument.parse::<u32>() {
                Ok(offset) => area
                    .scroll(offset)
                    .map(|()| format!("scrolled to {offset}")),
                Err(_) => Err(format!("{argument:?} is not a scroll offset")),
            },
            "select" => {
                let parts: Vec<&str> = argument.split(':').collect();
                match (parts.first(), parts.get(1)) {
                    (Some(anchor), Some(cursor)) => {
                        match (anchor.parse::<u32>(), cursor.parse::<u32>()) {
                            (Ok(anchor), Ok(cursor)) => area
                                .select(anchor, cursor)
                                .map(|()| format!("selected {anchor}..{cursor}")),
                            _ => Err(format!("{argument:?} is not a selection")),
                        }
                    }
                    _ => Err(format!("{argument:?} is not a selection (anchor:cursor)")),
                }
            }
            "move" => {
                let parts: Vec<&str> = argument.split(':').collect();
                let numbers: Option<Vec<u64>> =
                    parts.iter().map(|part| part.parse::<u64>().ok()).collect();
                match numbers.as_deref() {
                    Some([from, from_index, to, to_index]) => area
                        .move_tab(
                            GroupId::from_raw(*from),
                            *from_index as usize,
                            GroupId::from_raw(*to),
                            *to_index as usize,
                        )
                        .map(|()| {
                            format!("moved tab {from_index} of group {from} to {to}:{to_index}")
                        }),
                    _ => Err(format!("{argument:?} is not a move (from:index:to:index)")),
                }
            }
            "close" => {
                if argument != "group" {
                    Err(format!("{argument:?} is not close:group"))
                } else {
                    area.close_focus_group()
                        .map(|moved| format!("closed the group; {moved} view(s) moved"))
                }
            }
            other => Err(format!("{other:?} is not a group operation")),
        };
        match result {
            Ok(note) => println!("{note}"),
            Err(reason) => return groups_fail(&area, &reason),
        }
    }
    for line in area.render() {
        println!("{line}");
    }
    true
}

/// Prints the editor area as it stands and reports why the script stopped.
pub(super) fn groups_fail(area: &crate::app::groups::Groups, reason: &str) -> bool {
    println!("rustage: {reason}");
    for line in area.render() {
        println!("{line}");
    }
    false
}

/// Copies, round-trips or pastes one clipboard payload (task T05.06).
pub(super) fn clipboard(options: &crate::cli::ClipboardOptions) -> bool {
    use crate::app::clipboard::{CopySubtree, Pasted, Payload};
    use crate::contracts::identity::{IdAllocator, NodeId};
    use crate::project::document::root_of;
    use crate::project::page::PageDocument;

    let payload = if let Some(text) = &options.text {
        Payload::Text(text.clone())
    } else if let (Some(project), Some(path)) = (&options.resource_project, &options.resource_path)
    {
        Payload::Resource {
            project: project.clone(),
            path: path.clone(),
        }
    } else if let (Some(file), Some(id)) = (&options.node_file, options.node_id) {
        let root = match root_of(file) {
            Ok(root) => root,
            Err(refusal) => {
                println!("rustage: {}", refusal.message());
                return false;
            }
        };
        let page = match PageDocument::read(&root, relative_to(&root, file)) {
            Ok(page) => page,
            Err(error) => {
                println!("rustage: {error}");
                return false;
            }
        };
        let id = NodeId::from_raw(id);
        let Some(nodes) = subtree_of(&page, id) else {
            println!("rustage: the document has no node {}", id.get());
            return false;
        };
        Payload::Nodes(CopySubtree::new(directory_name(root.as_path()), id, nodes))
    } else {
        println!("rustage: nothing to copy: name text, a resource or a node");
        return false;
    };

    let encoded = match payload.encode(options.limit) {
        Ok(text) => text,
        Err(error) => {
            println!("rustage: {}", error.message());
            return false;
        }
    };
    println!(
        "copied {} as {} ({} byte(s))",
        describe(&payload),
        payload.mime(),
        encoded.len()
    );

    if options.roundtrip {
        match crate::app::clipboard::decode(&encoded, options.limit) {
            Ok(back) if back == payload => {
                println!("round-trip: the payload decoded to itself");
            }
            Ok(_) => {
                println!("round-trip: the payload decoded to something else");
                return false;
            }
            Err(error) => {
                println!("rustage: {}", error.message());
                return false;
            }
        }
    }

    if let Some(target) = &options.paste_into {
        let mut allocator = IdAllocator::new();
        match payload.paste(target, &mut allocator) {
            Ok(Pasted::Text(text)) => println!("paste: {} char(s) of text", text.chars().count()),
            Ok(Pasted::Resource { path }) => println!("paste: resource {path}"),
            Ok(Pasted::Nodes(placement)) => {
                let old_root = match &payload {
                    Payload::Nodes(subtree) => subtree.root(),
                    _ => placement.root(),
                };
                let mapping = placement
                    .ids()
                    .iter()
                    .map(|(old, new)| format!("{}->{}", old.get(), new.get()))
                    .collect::<Vec<_>>()
                    .join(", ");
                println!(
                    "paste: {} node(s); root {} -> {}; {mapping}",
                    placement.nodes().len(),
                    old_root.get(),
                    placement.root().get()
                );
                if !placement.assets_to_reconcile().is_empty() {
                    println!(
                        "reconcile: {} no longer points at the same file here",
                        placement.assets_to_reconcile().join(", ")
                    );
                }
            }
            Err(error) => {
                println!("rustage: {}", error.message());
                return false;
            }
        }
    }
    true
}

/// Applies, saves, resets and locks a layout (task T04.05).
///
/// A refusal from an operation stops the script, *except* a bad layout file: that one is printed and
/// the run carries on, because the point of "recoverable" is that the area and every open document
/// are still there afterwards. The run still exits non-zero so the refusal is not mistaken for a
/// success.
pub(super) fn layout(options: &crate::cli::LayoutOptions) -> bool {
    use crate::app::groups::{Axis, Groups};
    use crate::app::layout::Layouts;
    use crate::app::panels::built_in;
    use std::collections::BTreeSet;

    let services: BTreeSet<String> = options.services.iter().cloned().collect();
    let mut groups = Groups::new(options.limit);
    let mut panels = built_in();
    let mut layouts = Layouts::new();
    let mut refused = false;

    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<String, String> = match verb {
            "open" => {
                groups.open(argument);
                Ok(format!("open {argument}"))
            }
            "split" => match Axis::from_name(argument) {
                Some(axis) => Ok(format!(
                    "split {}: group {}",
                    axis.name(),
                    groups.split(axis).get()
                )),
                None => Err(format!(
                    "{argument:?} is not an axis (left-right, top-bottom)"
                )),
            },
            "preset" => layout_outcome(
                layouts.use_preset(argument, &mut groups, &mut panels),
                || format!("applied preset {argument}"),
            ),
            "save" => {
                layouts.save(argument, &groups, &panels);
                Ok(format!("saved layout {argument}"))
            }
            "recall" => layout_outcome(layouts.recall(argument, &mut groups, &mut panels), || {
                format!("recalled layout {argument}")
            }),
            "zen" => match on_off(argument) {
                Some(on) => layouts
                    .set_zen(on)
                    .map(|()| format!("zen {}", if on { "on" } else { "off" }))
                    .map_err(|error| error.message()),
                None => Err(format!("{argument:?} is not on or off")),
            },
            "fullscreen" => match on_off(argument) {
                Some(on) => layouts
                    .set_fullscreen(on)
                    .map(|()| format!("fullscreen {}", if on { "on" } else { "off" }))
                    .map_err(|error| error.message()),
                None => Err(format!("{argument:?} is not on or off")),
            },
            "lock" => {
                layouts.lock();
                Ok("locked".to_owned())
            }
            "unlock" => {
                layouts.unlock();
                Ok("unlocked".to_owned())
            }
            "reset" => layout_outcome(layouts.reset(&mut groups, &mut panels), || {
                "reset to the default layout".to_owned()
            }),
            "load" => match std::fs::read_to_string(argument) {
                Ok(text) => {
                    let outcome = layouts.load(&text, &mut groups, &mut panels);
                    if let Err(error) = &outcome {
                        // Recoverable: say why, carry on, and leave the area as it was.
                        println!("rustage: {}", error.message());
                        refused = true;
                        Ok(String::new())
                    } else {
                        layout_outcome(outcome, || format!("loaded {argument}"))
                    }
                }
                Err(error) => Err(format!("cannot read {argument}: {error}")),
            },
            "write" => match std::fs::write(argument, layouts.to_json()) {
                Ok(()) => Ok(format!("wrote {argument}")),
                Err(error) => Err(format!("cannot write {argument}: {error}")),
            },
            "panels" => Ok(String::new()),
            other => Err(format!("{other:?} is not a layout operation")),
        };
        match step {
            Ok(note) if note.is_empty() => {}
            Ok(note) => println!("{note}"),
            Err(reason) => {
                println!("rustage: {reason}");
                return layout_report(&groups, &panels, &services, &layouts);
            }
        }
    }

    let report_ok = layout_report(&groups, &panels, &services, &layouts);
    report_ok && !refused
}

/// Places windows and floating panels across displays, and reconciles them (task T04.06).
///
/// The display set is an input because the pinned platform exposes no display enumeration (see
/// [`crate::app::windows`] and `feedback-20261015.md` §十二): the caller plays the platform's part.
/// With no `--display` named, one documented stand-in display is used so the command is usable from
/// a terminal, and that is said out loud rather than left to look like a real reading.
pub(super) fn windows(options: &crate::cli::WindowsOptions) -> bool {
    use crate::app::windows::{Display, DisplayId, Displays, Rect, WindowId, Windows};

    let mut displays = if options.displays.is_empty() {
        println!(
            "note: no --display given; this build cannot ask the platform for its displays, so one \
             stand-in display 1280x800 @1.0 is used"
        );
        Displays::new(vec![
            Display::new(DisplayId::from_raw(0), Rect::new(0, 0, 1280, 800), 1.0)
                .expect("a valid display"),
        ])
    } else {
        let mut list = Vec::with_capacity(options.displays.len());
        for argument in &options.displays {
            let bounds = Rect::new(argument.x, argument.y, argument.width, argument.height);
            match Display::new(
                DisplayId::from_raw(argument.id),
                bounds,
                f64::from(argument.scale_milli) / 1000.0,
            ) {
                Some(display) => list.push(display),
                None => {
                    println!(
                        "rustage: display {} has a scale that is not a scale",
                        argument.id
                    );
                    return false;
                }
            }
        }
        Displays::new(list)
    };

    let mut windows = Windows::new(&displays);
    let main = windows.main().expect("a new session has a main window");
    if let (Some(width), Some(height)) = (options.main_width, options.main_height)
        && let Err(error) = windows.resize(main, width, height)
    {
        println!("rustage: {error}");
        return false;
    }

    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<String, String> = match verb {
            "view" => windows
                .open_view(argument, &displays)
                .map(|id| format!("opened view window {} for {argument}", id.get())),
            "float" => windows
                .float_panel(argument, &displays)
                .map(|id| format!("floated panel {argument} as window {}", id.get())),
            "close" => match argument.parse::<u64>() {
                Ok(id) => windows.close(WindowId::from_raw(id)).map(|closed| {
                    let ids = closed
                        .iter()
                        .map(|window| window.get().to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("closed {} window(s): {ids}", closed.len())
                }),
                Err(_) => Err(format!("{argument:?} is not a window id")),
            },
            "move" => match argument.split_once(':') {
                Some((window, display)) => match (window.parse::<u64>(), display.parse::<u64>()) {
                    (Ok(window), Ok(display)) => windows
                        .move_to(
                            WindowId::from_raw(window),
                            DisplayId::from_raw(display),
                            &displays,
                        )
                        .map(|()| format!("moved window {window} to display {display}")),
                    _ => Err(format!("{argument:?} is not <window>:<display>")),
                },
                None => Err(format!("{argument:?} is not <window>:<display>")),
            },
            "unplug" => match argument.parse::<u64>() {
                Ok(id) => {
                    if displays.unplug(DisplayId::from_raw(id)) {
                        Ok(format!("unplugged display {id}"))
                    } else {
                        Err(format!("there is no display {id}"))
                    }
                }
                Err(_) => Err(format!("{argument:?} is not a display id")),
            },
            "plug" => match crate::cli::parse_display(argument) {
                Some(arg) => {
                    let display = Display::new(
                        DisplayId::from_raw(arg.id),
                        Rect::new(arg.x, arg.y, arg.width, arg.height),
                        f64::from(arg.scale_milli) / 1000.0,
                    )
                    .expect("a parsed display has a scale");
                    displays.plug(display);
                    Ok(format!("plugged display {}", arg.id))
                }
                None => Err(format!("{argument:?} is not <id>:<x>,<y>,<w>,<h>,<scale>")),
            },
            "rescale" => match argument.split_once(':') {
                Some((display, scale)) => match (display.parse::<u64>(), scale.parse::<f64>()) {
                    (Ok(id), Ok(scale)) if scale.is_finite() && scale > 0.0 => {
                        match displays.get(DisplayId::from_raw(id)) {
                            Some(held) => {
                                let updated = Display::new(held.id(), held.bounds(), scale)
                                    .expect("the scale is positive");
                                displays.plug(updated);
                                Ok(format!("display {id} is now at scale {scale}"))
                            }
                            None => Err(format!("there is no display {id}")),
                        }
                    }
                    _ => Err(format!("{argument:?} is not <display>:<scale>")),
                },
                None => Err(format!("{argument:?} is not <display>:<scale>")),
            },
            "reconcile" => {
                let adjustments = windows.reconcile(&displays);
                for adjustment in &adjustments {
                    println!("{}", adjustment.render());
                }
                Ok(String::new())
            }
            other => Err(format!("{other:?} is not a window operation")),
        };
        match step {
            Ok(note) if note.is_empty() => {}
            Ok(note) => println!("{note}"),
            Err(reason) => {
                println!("rustage: {reason}");
                for line in windows.render(&displays) {
                    println!("{line}");
                }
                return false;
            }
        }
    }

    for line in windows.render(&displays) {
        println!("{line}");
    }
    true
}

/// Shows the toolbar, breadcrumb, status items and notices (task T04.07).
///
/// The point of the script is the acceptance's second half: a notice can arrive while a person is
/// typing, and neither the keyboard nor the text moves. `Chrome::notices` is a borrowed sub-view
/// with no way to reach either, so the property holds by construction; the script prints the state
/// after the notices so it can be read off.
pub(super) fn chrome(options: &crate::cli::ChromeOptions) -> bool {
    use crate::app::chrome::{Breadcrumb, Chrome};
    use crate::app::command::{Context, built_in as registry};
    use crate::contracts::permission::EffectClass;

    let registry = registry();
    let context = Context::in_area("design")
        .trusted()
        .authorizing(EffectClass::Command);
    let mut chrome = Chrome::new();
    let mut project: Option<String> = None;
    let mut document: Option<String> = None;
    let mut node: Option<String> = None;
    let mut width: Option<u32> = None;

    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<String, String> = match verb {
            "focus" => match target_of(argument) {
                Some(target) => {
                    let area = target.area().to_owned();
                    chrome.set_focus(target);
                    Ok(format!("the keyboard is in {area}"))
                }
                None => Err(format!("{argument:?} is not an area")),
            },
            "type" => {
                let mut accepted = 0;
                for character in argument.chars() {
                    if chrome.type_char(character) {
                        accepted += 1;
                    }
                }
                if accepted == 0 && !argument.is_empty() {
                    Err("the keyboard is not in a text field, so typing goes nowhere".to_owned())
                } else {
                    Ok(format!("typed {accepted} character(s)"))
                }
            }
            "backspace" => match argument.parse::<u32>() {
                Ok(count) => {
                    let mut removed = 0;
                    for _ in 0..count {
                        if chrome.backspace() {
                            removed += 1;
                        }
                    }
                    Ok(format!("removed {removed} character(s)"))
                }
                Err(_) => Err(format!("{argument:?} is not a count")),
            },
            "project" => {
                project = Some(argument.to_owned());
                Ok(format!("project {argument}"))
            }
            "document" => {
                document = Some(argument.to_owned());
                Ok(format!("document {argument}"))
            }
            "node" => {
                node = Some(argument.to_owned());
                Ok(format!("node {argument}"))
            }
            "notice" => {
                let mut parts = argument.splitn(3, ':');
                match (parts.next(), parts.next(), parts.next()) {
                    (Some(severity), Some(source), Some(summary)) => match severity_of(severity) {
                        Some(severity) => {
                            chrome.notices().show(severity, summary, source);
                            Ok(format!("notice from {source}: {summary}"))
                        }
                        None => Err(format!(
                            "{severity:?} is not a severity (error, warning, info)"
                        )),
                    },
                    _ => Err(format!("{argument:?} is not <severity>:<source>:<summary>")),
                }
            }
            "clear" => {
                chrome.notices().clear();
                Ok("cleared the notices".to_owned())
            }
            "width" => match argument.parse::<u32>() {
                Ok(value) => {
                    width = Some(value);
                    Ok(format!("the window is {value} wide"))
                }
                Err(_) => Err(format!("{argument:?} is not a width")),
            },
            "toolbar" | "status" => Ok(String::new()),
            other => Err(format!("{other:?} is not a chrome operation")),
        };
        match step {
            Ok(note) if note.is_empty() => {}
            Ok(note) => println!("{note}"),
            Err(reason) => {
                println!("rustage: {reason}");
                return chrome_report(&chrome, &registry, &context, width);
            }
        }
    }

    let mut breadcrumb = Breadcrumb::of(project.as_deref(), document.as_deref());
    if let Some(node) = &node {
        breadcrumb = breadcrumb.into_node(node.clone());
    }
    chrome.set_breadcrumb(breadcrumb);
    chrome_report(&chrome, &registry, &context, width)
}

/// The chrome's own report: the trail, the toolbar and the status items.
///
/// With a width, the toolbar is split into what fits and what overflows — the narrow-window case
/// (T04.08): a cramped window moves buttons into a "more" menu rather than clipping one.
pub(super) fn chrome_report(
    chrome: &crate::app::chrome::Chrome,
    registry: &crate::app::command::Registry,
    context: &crate::app::command::Context,
    width: Option<u32>,
) -> bool {
    for line in chrome.render() {
        println!("{line}");
    }
    println!("toolbar:");
    let entries = chrome.toolbar(registry, context);
    match width {
        None => {
            for entry in &entries {
                println!("  {}", entry.render());
            }
        }
        Some(width) => {
            let (shown, overflowed) = crate::app::chrome::fit_toolbar(&entries, width);
            for entry in shown {
                println!("  {}", entry.render());
            }
            if !overflowed.is_empty() {
                println!("  (+{} more in the overflow menu)", overflowed.len());
            }
        }
    }
    println!("status:");
    for item in chrome.status() {
        println!("  {}", item.render());
    }
    true
}

/// The focus target an area name asks for. The design and code modes name the canvas and the editor.
pub(super) fn target_of(name: &str) -> Option<crate::app::focus::Target> {
    use crate::app::focus::Target::{Canvas, Editor, Inspector, Outline, Status, Tabs, Welcome};
    Some(match name {
        "welcome" => Welcome,
        "canvas" | "design" => Canvas,
        "outline" => Outline,
        "inspector" => Inspector,
        "editor" | "code" => Editor,
        "tabs" => Tabs,
        "status" => Status,
        _ => return None,
    })
}

/// The severity an argument names.
pub(super) fn severity_of(name: &str) -> Option<crate::host::notify::Severity> {
    use crate::host::notify::Severity::{Error, Info, Warning};
    Some(match name {
        "error" => Error,
        "warning" => Warning,
        "info" => Info,
        _ => return None,
    })
}

/// The node and every node under it, in the order the page document holds them.
///
/// A repeated id stops the walk instead of looping, so a document that somehow names one node twice
/// cannot make a copy run forever.
pub(super) fn subtree_of(
    page: &crate::project::page::PageDocument,
    root: crate::contracts::identity::NodeId,
) -> Option<Vec<crate::project::page::PageNode>> {
    use std::collections::BTreeSet;

    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        let node = page.node(id)?;
        for child in node.children().iter().rev() {
            stack.push(*child);
        }
        out.push(node.clone());
    }
    Some(out)
}

/// Routes a keypress, or shows a context menu and the toolbar (task T05.05).
///
/// One keypress produces one route, and it is claimed unless nothing was bound: a refused command does
/// not fall through, which is what keeps one press from running two things. The context menu and the
/// toolbar both ask the registry, so they grey the same items with the same reasons.
pub(super) fn focus(options: &crate::cli::FocusOptions) -> bool {
    use crate::app::command::Context;
    use crate::app::focus::{Target, menu_entries, press, toolbar_entries};
    use crate::app::shortcuts::Chord;
    use crate::contracts::permission::EffectClass;

    let registry = crate::app::command::built_in();
    let keymap = crate::app::shortcuts::built_in();
    let target = match options.target.as_str() {
        "welcome" => Target::Welcome,
        "canvas" => Target::Canvas,
        "outline" => Target::Outline,
        "inspector" => Target::Inspector,
        "editor" => Target::Editor,
        "tabs" => Target::Tabs,
        "status" => Target::Status,
        other => {
            println!("rustage: {other:?} is not a focus target");
            return false;
        }
    };

    let mut context = Context::in_area(target.area());
    if options.trust {
        context = context.trusted();
    }
    for name in &options.authorize {
        if let Some(effect) = EffectClass::ALL.iter().find(|effect| effect.name() == name) {
            context = context.authorizing(*effect);
        }
    }
    for task in &options.running {
        context = context.running(task.clone());
    }
    println!(
        "focus {} (area {}), mode {}, trusted {}",
        options.target,
        target.area(),
        options.mode,
        yes_no(options.trust)
    );

    if let Some(chord) = &options.press {
        let Ok(chord) = Chord::parse(chord) else {
            println!("rustage: {chord:?} is not a chord");
            return false;
        };
        let route = press(
            &keymap,
            &registry,
            &context,
            &target,
            &options.mode,
            chord.keys(),
        );
        println!(
            "{} [{}]",
            route.message(),
            if route.is_claimed() {
                "the key is claimed"
            } else {
                "the key is not claimed"
            }
        );
        // A refusal is a finding (the convention `--command` also uses); a consumed key is not.
        return matches!(
            route,
            crate::app::focus::Route::Ran { .. }
                | crate::app::focus::Route::Suppressed { .. }
                | crate::app::focus::Route::Pending { .. }
        );
    }

    let entries = match &options.menu {
        Some(title) => match crate::app::menu::built_in()
            .into_iter()
            .find(|menu| menu.title() == title)
        {
            Some(menu) => {
                println!("context menu {title}:");
                menu_entries(&registry, &menu, &context)
            }
            None => {
                println!("rustage: there is no {title:?} menu");
                return false;
            }
        },
        None => {
            println!("toolbar:");
            toolbar_entries(&registry, &crate::app::focus::built_in_toolbar(), &context)
        }
    };
    let ok = entries.iter().all(|entry| entry.is_enabled()) || entries.is_empty();
    for entry in &entries {
        println!("  {}", entry.render());
    }
    ok
}
