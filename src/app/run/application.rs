// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! F26: the target application's shell, actions, menu, tool bar, context menus, dialogs, dock and the
//! end-to-end walk.
//!
//! One command handler per parsed command of this domain; the dispatcher lives in [`super`](self).
//! Each handler builds the domain value from `verb:argument` operations and prints what it got, and a
//! refusal from the domain is printed through [`report`] rather than presented as a success.

use super::*;

/// Parses `yes`/`no`.
fn yes_no(value: &str) -> Result<bool, String> {
    match value {
        "yes" | "true" => Ok(true),
        "no" | "false" => Ok(false),
        other => Err(format!("{other:?} is not yes or no")),
    }
}

/// Parses `WxH`.
fn size(text: &str) -> Result<(u32, u32), String> {
    let (width, height) = text
        .split_once('x')
        .ok_or_else(|| format!("{text:?} is not <width>x<height>"))?;
    let width = width
        .parse::<u32>()
        .map_err(|_| format!("{width:?} is not a width"))?;
    let height = height
        .parse::<u32>()
        .map_err(|_| format!("{height:?} is not a height"))?;
    Ok((width, height))
}

/// Assembles the target application's window shell (task T26.01).
pub(super) fn app_shell(options: &crate::cli::AppShellOptions) -> bool {
    use crate::app::application::shell::{
        Capability, Library, Region, Regions, Target, Window, assemble,
    };

    let mut name = String::new();
    let mut title = "Application".to_owned();
    let mut dimensions = (800_u32, 600_u32);
    let mut regions = Regions::none();
    let mut menu: Option<Capability> = None;
    let mut dock: Option<Capability> = None;
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<(), String> = match verb {
            "target" => {
                name = argument.to_owned();
                Ok(())
            }
            "title" => {
                title = argument.to_owned();
                Ok(())
            }
            "size" => match size(argument) {
                Ok(parsed) => {
                    dimensions = parsed;
                    Ok(())
                }
                Err(reason) => Err(reason),
            },
            "region" => match Region::parse(argument) {
                Some(region) => {
                    regions = regions.with(region);
                    Ok(())
                }
                None => Err(format!("{argument:?} is not a region")),
            },
            "provide" => match Region::parse(argument) {
                Some(Region::MenuBar) => {
                    menu = Some(Capability::provided());
                    Ok(())
                }
                Some(Region::Dock) => {
                    dock = Some(Capability::provided());
                    Ok(())
                }
                Some(other) => Err(format!("{} has no native seam to provide", other.name())),
                None => Err(format!("{argument:?} is not a region")),
            },
            "gap" => match argument.split_once(':') {
                Some((region, reason)) => match Region::parse(region) {
                    Some(Region::MenuBar) => {
                        menu = Some(Capability::gap("native menu", reason));
                        Ok(())
                    }
                    Some(Region::Dock) => {
                        dock = Some(Capability::gap("docking", reason));
                        Ok(())
                    }
                    Some(other) => Err(format!("{} has no native seam to gap", other.name())),
                    None => Err(format!("{region:?} is not a region")),
                },
                None => Err(format!("gap needs <region>:<reason>, got {argument:?}")),
            },
            "show" => Ok(()),
            other => Err(format!("{other:?} is not a shell operation")),
        };
        if let Err(reason) = step {
            return report(reason);
        }
    }
    let Some(target) = Target::new(&name) else {
        return report("a shell needs a target".to_owned());
    };
    let window = match Window::new(&title, dimensions.0, dimensions.1) {
        Ok(window) => window,
        Err(reason) => return report(reason),
    };
    let mut library = Library::provided(target);
    if let Some(menu) = menu {
        library = library.with_menu(menu);
    }
    if let Some(dock) = dock {
        library = library.with_dock(dock);
    }
    match assemble(window, regions, &library) {
        Ok(shell) => {
            for line in shell.render() {
                println!("{line}");
            }
            true
        }
        Err(reason) => report(reason),
    }
}

/// Defines the shared actions and their entry points (task T26.02).
pub(super) fn app_action(options: &crate::cli::AppActionOptions) -> bool {
    use crate::app::application::action::{
        Action, ActionId, Catalog, EntryPoint, EventTarget, Scope, Shortcut,
    };

    let mut catalog = Catalog::new();
    let mut pending: Option<Action> = None;
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let modifies = matches!(
            verb,
            "title" | "icon" | "shortcut" | "enabled" | "checked" | "group" | "target"
        );
        if !modifies
            && let Some(action) = pending.take()
            && let Err(reason) = catalog.add(action)
        {
            return report(reason);
        }
        let step: Result<(), String> = match verb {
            "action" => match ActionId::new(argument) {
                Some(id) => {
                    let target = EventTarget::Handler(id.as_str().to_owned());
                    match Action::new(id, argument, target) {
                        Ok(action) => {
                            pending = Some(action);
                            Ok(())
                        }
                        Err(reason) => Err(reason),
                    }
                }
                None => Err(format!("{argument:?} is not an action id")),
            },
            "title" | "icon" | "shortcut" | "enabled" | "checked" | "group" | "target" => {
                let Some(action) = pending.as_mut() else {
                    return report(format!("{verb} needs a current action"));
                };
                let applied: Result<(), String> = match verb {
                    "title" => {
                        action.title = argument.to_owned();
                        Ok(())
                    }
                    "icon" => {
                        action.icon = Some(argument.to_owned());
                        Ok(())
                    }
                    "shortcut" => match Shortcut::parse(argument) {
                        Some(shortcut) => {
                            action.shortcut = Some(shortcut);
                            Ok(())
                        }
                        None => Err(format!("{argument:?} is not a shortcut")),
                    },
                    "enabled" => match yes_no(argument) {
                        Ok(value) => {
                            action.enabled = value;
                            Ok(())
                        }
                        Err(reason) => Err(reason),
                    },
                    "checked" => match yes_no(argument) {
                        Ok(value) => {
                            action.checked = value;
                            Ok(())
                        }
                        Err(reason) => Err(reason),
                    },
                    "group" => {
                        action.group = Some(argument.to_owned());
                        Ok(())
                    }
                    "target" => match argument.split_once(':') {
                        Some((kind, value)) => match EventTarget::parse(kind, value) {
                            Some(target) => {
                                action.target = target;
                                Ok(())
                            }
                            None => Err(format!("{:?} is not an event target", argument)),
                        },
                        None => Err(format!("{:?} is not <kind>:<value>", argument)),
                    },
                    _ => Ok(()),
                };
                applied
            }
            "reference" => match argument.split_once(':') {
                Some((entry, id)) => match (EntryPoint::parse(entry), ActionId::new(id)) {
                    (Some(entry), Some(id)) => {
                        catalog.reference(entry, id);
                        Ok(())
                    }
                    _ => Err(format!("{:?} is not <entry>:<action>", argument)),
                },
                None => Err(format!("{:?} is not <entry>:<action>", argument)),
            },
            "claim" => match argument.split_once(':') {
                Some((scope, id)) => match (Scope::parse(scope), ActionId::new(id)) {
                    (Some(scope), Some(id)) => {
                        catalog.claim(scope, id);
                        Ok(())
                    }
                    _ => Err(format!("{:?} is not <scope>:<action>", argument)),
                },
                None => Err(format!("{:?} is not <scope>:<action>", argument)),
            },
            "select" => match ActionId::new(argument) {
                Some(id) => catalog.select(&id),
                None => Err(format!("{argument:?} is not an action id")),
            },
            "show" => Ok(()),
            other => Err(format!("{other:?} is not an action operation")),
        };
        if let Err(reason) = step {
            return report(reason);
        }
    }
    if let Some(action) = pending.take()
        && let Err(reason) = catalog.add(action)
    {
        return report(reason);
    }
    for line in catalog.render() {
        println!("{line}");
    }
    let conflicts = catalog.conflicts();
    for conflict in &conflicts {
        println!("{}", conflict.render());
    }
    conflicts.is_empty()
}

/// Edits the target application's menu hierarchy (task T26.03).
pub(super) fn app_menu(options: &crate::cli::AppMenuOptions) -> bool {
    use crate::app::application::action::ActionId;
    use crate::app::application::menu::{MenuTree, Node, Platform, Role};
    use std::collections::{BTreeMap, BTreeSet};

    let mut roots: Vec<(String, String)> = Vec::new();
    let mut defined: BTreeSet<String> = BTreeSet::new();
    let mut nodes: BTreeMap<String, Vec<Node>> = BTreeMap::new();
    let mut platform = Platform::MacOs;
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<(), String> = match verb {
            "root" => match argument.split_once(':') {
                Some((id, title)) => {
                    roots.push((id.to_owned(), title.to_owned()));
                    defined.insert(id.to_owned());
                    nodes.entry(id.to_owned()).or_default();
                    Ok(())
                }
                None => Err(format!("{:?} is not <id>:<title>", argument)),
            },
            "define" => {
                defined.insert(argument.to_owned());
                nodes.entry(argument.to_owned()).or_default();
                Ok(())
            }
            "item" => match argument.split_once(':') {
                Some((menu, rest)) => match rest.split_once(':') {
                    Some((label, action)) => match ActionId::new(action) {
                        Some(action) => {
                            nodes
                                .entry(menu.to_owned())
                                .or_default()
                                .push(Node::action(label, action));
                            Ok(())
                        }
                        None => Err(format!("{action:?} is not an action id")),
                    },
                    None => Err(format!("{:?} is not <menu>:<label>:<action>", argument)),
                },
                None => Err(format!("{:?} is not <menu>:<label>:<action>", argument)),
            },
            "separator" => {
                nodes
                    .entry(argument.to_owned())
                    .or_default()
                    .push(Node::separator());
                Ok(())
            }
            "role" => match argument.split_once(':') {
                Some((menu, role)) => match Role::parse(role) {
                    Some(role) => {
                        nodes
                            .entry(menu.to_owned())
                            .or_default()
                            .push(Node::role(role));
                        Ok(())
                    }
                    None => Err(format!("{role:?} is not a menu role")),
                },
                None => Err(format!("{:?} is not <menu>:<role>", argument)),
            },
            "submenu" => match argument.split_once(':') {
                Some((parent, rest)) => match rest.split_once(':') {
                    Some((child, label)) => {
                        nodes
                            .entry(parent.to_owned())
                            .or_default()
                            .push(Node::submenu(child, label));
                        Ok(())
                    }
                    None => Err(format!("{:?} is not <parent>:<child>:<label>", argument)),
                },
                None => Err(format!("{:?} is not <parent>:<child>:<label>", argument)),
            },
            "platform" => match Platform::parse(argument) {
                Some(parsed) => {
                    platform = parsed;
                    Ok(())
                }
                None => Err(format!("{argument:?} is not a platform")),
            },
            "show" => Ok(()),
            other => Err(format!("{other:?} is not a menu operation")),
        };
        if let Err(reason) = step {
            return report(reason);
        }
    }
    let mut tree = MenuTree::new();
    for (id, title) in &roots {
        let items = nodes.remove(id).unwrap_or_default();
        if let Err(reason) = tree.add_root(id, title, items) {
            return report(reason);
        }
    }
    for id in &defined {
        if roots.iter().any(|(root, _)| root == id) {
            continue;
        }
        let items = nodes.remove(id).unwrap_or_default();
        if let Err(reason) = tree.place(id, items) {
            return report(reason);
        }
    }
    for line in tree.render() {
        println!("{line}");
    }
    let (native, drawn) = tree.native_roles(platform);
    println!("native on {}: {}", platform.name(), render_roles(&native));
    println!("drawn by the app: {}", render_roles(&drawn));
    let problems = tree.problems();
    for problem in &problems {
        println!("{problem}");
    }
    problems.is_empty()
}

/// Renders a list of roles as names.
fn render_roles(roles: &[crate::app::application::menu::Role]) -> String {
    if roles.is_empty() {
        return "none".to_owned();
    }
    roles
        .iter()
        .map(|role| role.name())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Edits the target application's tool bar and status bar (task T26.04).
pub(super) fn app_toolbar(options: &crate::cli::AppToolbarOptions) -> bool {
    use crate::app::application::action::ActionId;
    use crate::app::application::toolbar::{Item, Overflow, Presentation, StatusBar, ToolBar};

    let mut bar = ToolBar::new(Overflow::Collapse);
    let mut status = StatusBar::new();
    let mut current: Option<Item> = None;
    let mut capacity: Option<usize> = None;
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let modifies = matches!(verb, "label" | "present" | "visible");
        if !modifies && let Some(item) = current.take() {
            bar.place(item);
        }
        let step: Result<(), String> = match verb {
            "item" => match ActionId::new(argument) {
                Some(action) => match Item::new(action, argument) {
                    Ok(item) => {
                        current = Some(item);
                        Ok(())
                    }
                    Err(reason) => Err(reason),
                },
                None => Err(format!("{argument:?} is not an action id")),
            },
            "label" | "present" | "visible" => {
                let Some(item) = current.as_mut() else {
                    return report(format!("{verb} needs a current item"));
                };
                match verb {
                    "label" => {
                        item.label = argument.to_owned();
                        Ok(())
                    }
                    "present" => match Presentation::parse(argument) {
                        Some(presentation) => {
                            item.presentation = presentation;
                            Ok(())
                        }
                        None => Err(format!("{argument:?} is not a presentation")),
                    },
                    "visible" => match yes_no(argument) {
                        Ok(value) => {
                            item.visible = value;
                            Ok(())
                        }
                        Err(reason) => Err(reason),
                    },
                    _ => Ok(()),
                }
            }
            "overflow" => match Overflow::parse(argument) {
                Some(overflow) => {
                    bar.overflow = overflow;
                    Ok(())
                }
                None => Err(format!("{argument:?} is not an overflow behaviour")),
            },
            "capacity" => match argument.parse::<usize>() {
                Ok(value) => {
                    capacity = Some(value);
                    Ok(())
                }
                Err(_) => Err(format!("{argument:?} is not a capacity")),
            },
            "status" => match argument.split_once(':') {
                Some((label, value)) => {
                    status.set(label, value);
                    Ok(())
                }
                None => Err(format!("{:?} is not <label>:<value>", argument)),
            },
            "show" => Ok(()),
            other => Err(format!("{other:?} is not a tool bar operation")),
        };
        if let Err(reason) = step {
            return report(reason);
        }
    }
    if let Some(item) = current.take() {
        bar.place(item);
    }
    for line in bar.render() {
        println!("{line}");
    }
    let layout = bar.resolve(capacity.unwrap_or(usize::MAX));
    println!(
        "shown: {}",
        layout
            .shown
            .iter()
            .map(|action| action.as_str().to_owned())
            .collect::<Vec<_>>()
            .join(", "),
    );
    println!(
        "overflowed: {}",
        layout
            .overflowed
            .iter()
            .map(|action| action.as_str().to_owned())
            .collect::<Vec<_>>()
            .join(", "),
    );
    println!("chevron: {}", yes_no_render(layout.chevron));
    for line in status.render() {
        println!("status {line}");
    }
    true
}

/// Renders a flag as `yes` or `no`.
fn yes_no_render(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

/// Edits the target application's context menus (task T26.05).
pub(super) fn app_context(options: &crate::cli::AppContextOptions) -> bool {
    use crate::app::application::action::ActionId;
    use crate::app::application::context::{ContextKind, Registry, Request, ScopeParams};

    let mut registry = Registry::new();
    let mut params = ScopeParams::new();
    let mut last: Option<Request> = None;
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<(), String> = match verb {
            "kind" => match ContextKind::parse(argument) {
                Some(kind) => {
                    // Registering an empty menu makes the kind known even before it has items.
                    registry.register(crate::app::application::context::ContextMenu::new(kind));
                    Ok(())
                }
                None => Err(format!("{argument:?} is not a context kind")),
            },
            "item" => match argument.split_once(':') {
                Some((kind, action)) => match (ContextKind::parse(kind), ActionId::new(action)) {
                    (Some(kind), Some(action)) => {
                        registry.add_item(kind, action);
                        Ok(())
                    }
                    _ => Err(format!("{:?} is not <kind>:<action>", argument)),
                },
                None => Err(format!("{:?} is not <kind>:<action>", argument)),
            },
            "enable" => match argument.split_once(':') {
                Some((action, value)) => match (ActionId::new(action), yes_no(value)) {
                    (Some(action), Ok(value)) => {
                        registry.set_enabled(action, value);
                        Ok(())
                    }
                    _ => Err(format!("{:?} is not <action>:<yes|no>", argument)),
                },
                None => Err(format!("{:?} is not <action>:<yes|no>", argument)),
            },
            "param" => match argument.split_once(':') {
                Some((name, value)) => {
                    params.set(name, value);
                    Ok(())
                }
                None => Err(format!("{:?} is not <name>:<value>", argument)),
            },
            "click" => match registry.request_named(argument, params.clone()) {
                Ok(request) => {
                    println!(
                        "right-click on {}: {} item(s)",
                        request.kind.name(),
                        request.items.len(),
                    );
                    for action in &request.items {
                        println!("  {}", action.as_str());
                    }
                    last = Some(request);
                    Ok(())
                }
                Err(reason) => Err(reason),
            },
            "invoke" => match ActionId::new(argument) {
                Some(action) => {
                    let Some(request) = last.as_ref() else {
                        return report("invoke needs a preceding click".to_owned());
                    };
                    let invocation = request.invoke(&action, |id, scope| {
                        println!("  handler ran for {} with {}", id.as_str(), scope.render());
                    });
                    println!("{}", invocation.render());
                    Ok(())
                }
                None => Err(format!("{argument:?} is not an action id")),
            },
            "show" => {
                for kind in ContextKind::ALL {
                    if let Some(menu) = registry.menu(kind) {
                        println!(
                            "{}: {}",
                            kind.name(),
                            menu.items
                                .iter()
                                .map(|action| action.as_str().to_owned())
                                .collect::<Vec<_>>()
                                .join(", "),
                        );
                    }
                }
                Ok(())
            }
            other => Err(format!("{other:?} is not a context operation")),
        };
        if let Err(reason) = step {
            return report(reason);
        }
    }
    true
}

/// Models the target application's dialogs and modality (task T26.06).
pub(super) fn app_dialog(options: &crate::cli::AppDialogOptions) -> bool {
    use crate::app::application::action::ActionId;
    use crate::app::application::dialog::{Dialog, Kind, Modality, Stack, WindowId};
    use std::collections::BTreeMap;

    let mut stack: Option<Stack> = None;
    let mut defined: BTreeMap<String, Dialog> = BTreeMap::new();
    let mut fields: BTreeMap<String, String> = BTreeMap::new();
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<(), String> = match verb {
            "main" => match WindowId::new(argument) {
                Some(main) => {
                    stack = Some(Stack::new(main));
                    Ok(())
                }
                None => Err(format!("{argument:?} is not a window id")),
            },
            "dialog" => {
                let parts: Vec<&str> = argument.split(':').collect();
                if let [id, owner, kind, modality] = parts.as_slice() {
                    match (
                        WindowId::new(id),
                        WindowId::new(owner),
                        Kind::parse(kind),
                        Modality::parse(modality),
                    ) {
                        (Some(id), Some(owner), Some(kind), Some(modality)) => {
                            defined.insert(
                                id.as_str().to_owned(),
                                Dialog::new(id, owner, kind, modality),
                            );
                            Ok(())
                        }
                        _ => Err(format!("{:?} is not a well-formed dialog", argument)),
                    }
                } else {
                    Err(format!(
                        "{:?} is not <id>:<owner>:<kind>:<modality>",
                        argument
                    ))
                }
            }
            "default" | "cancel" => match argument.split_once(':') {
                Some((id, action)) => match (defined.get_mut(id), ActionId::new(action)) {
                    (Some(dialog), Some(action)) => {
                        if verb == "default" {
                            dialog.default = Some(action);
                        } else {
                            dialog.cancel = Some(action);
                        }
                        Ok(())
                    }
                    (None, _) => Err(format!("dialog {id:?} is not defined")),
                    (_, None) => Err(format!("{action:?} is not an action id")),
                },
                None => Err(format!("{:?} is not <id>:<action>", argument)),
            },
            "field" => match argument.split_once(':') {
                Some((name, value)) => {
                    fields.insert(name.to_owned(), value.to_owned());
                    Ok(())
                }
                None => Err(format!("{:?} is not <name>:<value>", argument)),
            },
            "open" => {
                let Some(stack) = stack.as_mut() else {
                    return report("open needs a main window".to_owned());
                };
                let Some(dialog) = defined.get(argument).cloned() else {
                    return report(format!("dialog {argument:?} is not defined"));
                };
                match stack.open(dialog) {
                    Ok(()) => {
                        println!("opened {} (focus {})", argument, stack.focus().as_str());
                        Ok(())
                    }
                    Err(reason) => Err(reason),
                }
            }
            "close" => {
                let Some(stack) = stack.as_mut() else {
                    return report("close needs a main window".to_owned());
                };
                let Some(id) = WindowId::new(argument) else {
                    return report(format!("{argument:?} is not a window id"));
                };
                match stack.close(&id) {
                    Ok(outcome) => {
                        println!("{}: focus {}", outcome.render(), stack.focus().as_str());
                        Ok(())
                    }
                    Err(reason) => Err(reason),
                }
            }
            "accept" => {
                let Some(stack) = stack.as_mut() else {
                    return report("accept needs a main window".to_owned());
                };
                let Some(id) = WindowId::new(argument) else {
                    return report(format!("{argument:?} is not a window id"));
                };
                match stack.accept(&id, fields.clone()) {
                    Ok(outcome) => {
                        println!("{}: focus {}", outcome.render(), stack.focus().as_str());
                        Ok(())
                    }
                    Err(reason) => Err(reason),
                }
            }
            "show" => {
                match stack.as_ref() {
                    Some(stack) => println!(
                        "focus {}, open: {}",
                        stack.focus().as_str(),
                        stack
                            .open_ids()
                            .iter()
                            .map(|id| id.as_str().to_owned())
                            .collect::<Vec<_>>()
                            .join(", "),
                    ),
                    None => println!("no main window"),
                }
                Ok(())
            }
            other => Err(format!("{other:?} is not a dialog operation")),
        };
        if let Err(reason) = step {
            return report(reason);
        }
    }
    true
}

/// States the target's docking and persisted layout support (task T26.07).
pub(super) fn app_dock(options: &crate::cli::AppDockOptions) -> bool {
    use crate::app::application::dock::{Layout, Orientation, Panel, Support, generate};

    let mut target = "desktop".to_owned();
    let mut shape = "single".to_owned();
    let mut panels: Vec<Panel> = Vec::new();
    let mut orientation = Orientation::Horizontal;
    let mut persist = false;
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<(), String> = match verb {
            "target" => {
                target = argument.to_owned();
                Ok(())
            }
            "layout" => match argument {
                "single" | "split" | "docked" => {
                    shape = argument.to_owned();
                    Ok(())
                }
                other => Err(format!("{other:?} is not a layout")),
            },
            "panel" => match Panel::new(argument) {
                Ok(panel) => {
                    panels.push(panel);
                    Ok(())
                }
                Err(reason) => Err(reason),
            },
            "orientation" => match Orientation::parse(argument) {
                Some(parsed) => {
                    orientation = parsed;
                    Ok(())
                }
                None => Err(format!("{argument:?} is not an orientation")),
            },
            "persist" => match yes_no(argument) {
                Ok(value) => {
                    persist = value;
                    Ok(())
                }
                Err(reason) => Err(reason),
            },
            "show" => Ok(()),
            other => Err(format!("{other:?} is not a dock operation")),
        };
        if let Err(reason) = step {
            return report(reason);
        }
    }
    let layout = match shape.as_str() {
        "docked" => Layout::Docked { panels },
        "split" => Layout::Split {
            orientation,
            panes: vec![Layout::Single, Layout::Single],
        },
        _ => Layout::Single,
    };
    let support = Support::of(&target);
    println!(
        "target {target}: docking {}, persisted layout {}",
        yes_no_render(support.docking),
        yes_no_render(support.persisted_layout),
    );
    match generate(&support, &layout, persist) {
        Ok(plan) => {
            for line in plan.render() {
                println!("{line}");
            }
            true
        }
        Err(refusal) => report(refusal.render()),
    }
}

/// Runs the end-to-end shell walk over both generation modes (task T26.08).
pub(super) fn app_regression(options: &crate::cli::AppRegressionOptions) -> bool {
    use crate::app::application::action::ActionId;
    use crate::app::application::regression::{Entry, Mode, Walk};

    // The mode decides the walk, so it is read before any activation is recorded.
    let mode = options
        .ops
        .iter()
        .find_map(|op| op.strip_prefix("mode:").and_then(Mode::parse))
        .unwrap_or(Mode::Runtime);
    let mut walk = Walk::new(mode);
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<(), String> = match verb {
            "mode" => match Mode::parse(argument) {
                Some(_) => Ok(()),
                None => Err(format!("{argument:?} is not a mode")),
            },
            "bind" | "activate" => match argument.split_once(':') {
                Some((entry, action)) => match (Entry::parse(entry), ActionId::new(action)) {
                    (Some(entry), Some(action)) => {
                        if verb == "bind" {
                            walk.bind(entry, action);
                            Ok(())
                        } else {
                            walk.activate(entry, &action)
                        }
                    }
                    _ => Err(format!("{:?} is not <entry>:<action>", argument)),
                },
                None => Err(format!("{:?} is not <entry>:<action>", argument)),
            },
            "preview" => {
                walk.preview();
                Ok(())
            }
            "generate" => {
                walk.generate();
                Ok(())
            }
            "show" => Ok(()),
            other => Err(format!("{other:?} is not a regression operation")),
        };
        if let Err(reason) = step {
            return report(reason);
        }
    }
    for event in walk.events() {
        println!("{event}");
    }
    let duplicates = walk.duplicates();
    for duplicate in &duplicates {
        println!("duplicate binding: {duplicate}");
    }
    duplicates.is_empty()
}
