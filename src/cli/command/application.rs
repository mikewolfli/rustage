// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The application family's commands: the target application's shell, its actions, menu, tool bar,
//! context menus, dialogs, dock and the end-to-end regression walk.

// Every branch of the flag table answers with `return`; writing the independent flags alike is worth
// more than saving a keyword.
#![allow(clippy::needless_return)]

use super::*;

/// How to assemble the target application's window shell (task T26.01).
#[derive(Debug, PartialEq, Eq)]
pub struct AppShellOptions {
    /// The operations: `target:<name>`, `title:<text>`, `size:<w>x<h>`, `region:<menu|toolbar|status|dock>`
    /// (repeatable), `provide:<menu|dock>`, `gap:<menu|dock>:<reason>`, `show`.
    pub ops: Vec<String>,
}

impl AppShellOptions {
    /// Options with no window.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to define the shared actions and their entry points (task T26.02).
#[derive(Debug, PartialEq, Eq)]
pub struct AppActionOptions {
    /// The operations: `action:<id>`, `title:<text>`, `icon:<name>`, `shortcut:<keys>`,
    /// `enabled:<yes|no>`, `checked:<yes|no>`, `group:<name>`, `target:<handler|route|builtin>:<value>`,
    /// `reference:<menu|toolbar|context>:<id>`, `claim:<global|menu|toolbar|context>:<id>`,
    /// `select:<id>`, `show`.
    pub ops: Vec<String>,
}

impl AppActionOptions {
    /// Options with no actions.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to edit the target application's menu hierarchy (task T26.03).
#[derive(Debug, PartialEq, Eq)]
pub struct AppMenuOptions {
    /// The operations: `root:<id>:<title>`, `define:<id>`, `item:<menu>:<label>:<action>`,
    /// `separator:<menu>`, `role:<menu>:<role>`, `submenu:<parent>:<child>:<label>`,
    /// `platform:<macos|windows|linux>`, `show`.
    pub ops: Vec<String>,
}

impl AppMenuOptions {
    /// Options with no menus.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to edit the target application's tool bar and status bar (task T26.04).
#[derive(Debug, PartialEq, Eq)]
pub struct AppToolbarOptions {
    /// The operations: `item:<id>`, `label:<text>`, `present:<icon|text|both>`, `visible:<yes|no>`,
    /// `overflow:<collapse|wrap>`, `capacity:<n>`, `status:<label>:<value>`, `show`.
    pub ops: Vec<String>,
}

impl AppToolbarOptions {
    /// Options with no items.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to edit the target application's context menus (task T26.05).
#[derive(Debug, PartialEq, Eq)]
pub struct AppContextOptions {
    /// The operations: `kind:<item|canvas|tree-node>`, `item:<kind>:<action>`,
    /// `enable:<action>:<yes|no>`, `param:<key>:<value>`, `click:<kind>`, `invoke:<action>`, `show`.
    pub ops: Vec<String>,
}

impl AppContextOptions {
    /// Options with no menus.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to model the target application's dialogs and modality (task T26.06).
#[derive(Debug, PartialEq, Eq)]
pub struct AppDialogOptions {
    /// The operations: `main:<id>`, `dialog:<id>:<owner>:<kind>:<modality>`, `default:<id>:<action>`,
    /// `cancel:<id>:<action>`, `field:<name>:<value>`, `open:<id>`, `close:<id>`, `accept:<id>`, `show`.
    pub ops: Vec<String>,
}

impl AppDialogOptions {
    /// Options with no dialogs.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to state the target's docking and persisted layout support (task T26.07).
#[derive(Debug, PartialEq, Eq)]
pub struct AppDockOptions {
    /// The operations: `target:<name>`, `layout:<single|split|docked>`, `panel:<name>`,
    /// `orientation:<horizontal|vertical>`, `persist:<yes|no>`, `show`.
    pub ops: Vec<String>,
}

impl AppDockOptions {
    /// Options with no layout.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to run the end-to-end shell walk over both generation modes (task T26.08).
#[derive(Debug, PartialEq, Eq)]
pub struct AppRegressionOptions {
    /// The operations: `mode:<runtime|static>`, `bind:<menu|toolbar|context|shortcut>:<action>`,
    /// `preview`, `generate`, `activate:<menu|toolbar|context|shortcut>:<action>`, `show`.
    pub ops: Vec<String>,
}

impl AppRegressionOptions {
    /// Options with no walk.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// Parses this family's flags, answering `None` when the flag is another family's.
pub(super) fn parse_application(
    text: &str,
    _argument: OsString,
    arguments: &mut std::vec::IntoIter<OsString>,
) -> Option<Result<Command, ParseError>> {
    macro_rules! ops {
        ($options:expr, $variant:path) => {{
            let mut options = $options;
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok($variant(options)));
        }};
    }
    match text {
        "--app-shell" => ops!(AppShellOptions::new(), Command::AppShell),
        "--app-action" => ops!(AppActionOptions::new(), Command::AppAction),
        "--app-menu" => ops!(AppMenuOptions::new(), Command::AppMenu),
        "--app-toolbar" => ops!(AppToolbarOptions::new(), Command::AppToolbar),
        "--app-context" => ops!(AppContextOptions::new(), Command::AppContext),
        "--app-dialog" => ops!(AppDialogOptions::new(), Command::AppDialog),
        "--app-dock" => ops!(AppDockOptions::new(), Command::AppDock),
        "--app-regression" => ops!(AppRegressionOptions::new(), Command::AppRegression),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;

    #[test]
    fn parses_the_application_operations() {
        assert_eq!(
            parse(["--app-shell"]),
            Ok(Command::AppShell(AppShellOptions::new()))
        );
        let mut expected = AppShellOptions::new();
        expected.ops = vec!["target:desktop".to_owned(), "title:Notes".to_owned()];
        assert_eq!(
            parse(["--app-shell", "target:desktop", "title:Notes"]),
            Ok(Command::AppShell(expected)),
        );
        assert!(parse(["--app-shell", "--oops"]).is_err());
    }

    #[test]
    fn parses_each_application_flag() {
        assert_eq!(
            parse(["--app-action"]),
            Ok(Command::AppAction(AppActionOptions::new()))
        );
        assert_eq!(
            parse(["--app-menu"]),
            Ok(Command::AppMenu(AppMenuOptions::new()))
        );
        assert_eq!(
            parse(["--app-toolbar"]),
            Ok(Command::AppToolbar(AppToolbarOptions::new()))
        );
        assert_eq!(
            parse(["--app-context"]),
            Ok(Command::AppContext(AppContextOptions::new()))
        );
        assert_eq!(
            parse(["--app-dialog"]),
            Ok(Command::AppDialog(AppDialogOptions::new()))
        );
        assert_eq!(
            parse(["--app-dock"]),
            Ok(Command::AppDock(AppDockOptions::new()))
        );
        assert_eq!(
            parse(["--app-regression"]),
            Ok(Command::AppRegression(AppRegressionOptions::new()))
        );
    }
}
