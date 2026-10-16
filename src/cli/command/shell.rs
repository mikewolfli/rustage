// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The window shell's commands: the workbench, panels, tabs, editor groups, window layout
//! and the chrome.

// Every branch of the flag table answers with `return`, including the ones that could be a
// trailing expression: the branches are independent flags, and writing them alike is worth more
// than saving a keyword. The returns that are *not* needless — the ones inside a loop that refuses
// a bad value — are why the table reads the way it does.
#![allow(clippy::needless_return)]

use super::*;

/// How to show the workbench and its panels (tasks T04.01/T04.02).
#[derive(Debug, PartialEq, Eq, Default)]
pub struct WorkbenchOptions {
    /// The mode to switch to.
    pub mode: Option<String>,
    /// The active project's name.
    pub project: Option<String>,
    /// The target the project would build for.
    pub target: Option<String>,
    /// The active document.
    pub document: Option<String>,
    /// Whether the document has unsaved changes.
    pub dirty: bool,
    /// A one-line status message.
    pub message: Option<String>,
    /// The services this host has, which decide which panels can be shown.
    pub services: Vec<String>,
    /// Panels to show.
    pub show: Vec<String>,
    /// Panels to hide.
    pub hide: Vec<String>,
    /// A panel to give the keyboard.
    pub focus: Option<String>,
    /// Panels to resize, as `(id, size)`.
    pub resize: Vec<(String, u32)>,
}

impl WorkbenchOptions {
    /// Options that show the workbench as it stands.
    pub fn new() -> Self {
        Self::default()
    }
}

/// A script of document-tab operations (task T04.03).
///
/// Each operation is `verb` or `verb:argument` (`move:from:to` takes two), in the order they happen,
/// so a run is a story about one tab strip: a document previewed, another pinned, one view closed and
/// the document itself closed.
#[derive(Debug, PartialEq, Eq)]
pub struct TabsOptions {
    /// How many tabs the strip shows before overflowing.
    pub limit: usize,
    /// The operations, in order.
    pub ops: Vec<String>,
}

impl TabsOptions {
    /// Options for an empty strip.
    pub fn new() -> Self {
        Self {
            limit: 8,
            ops: Vec::new(),
        }
    }
}

/// How to replay a script of editor-group operations (task T04.04).
///
/// Each operation is `verb` or `verb:argument` (`move:from:index:to:index` takes four, `select` takes
/// two), in the order they happen, so a run is a story about one editor area: two documents opened,
/// the area split, one view moved across, and the same buffer typed into from either side.
#[derive(Debug, PartialEq, Eq)]
pub struct GroupsOptions {
    /// How many tabs each group's strip shows before overflowing.
    pub limit: usize,
    /// The operations, in order.
    pub ops: Vec<String>,
}

impl GroupsOptions {
    /// Options for an editor area with one empty group.
    pub fn new() -> Self {
        Self {
            limit: 8,
            ops: Vec::new(),
        }
    }
}

/// How to replay a script of layout operations (task T04.05).
///
/// Each operation is `verb` or `verb:argument`, in the order they happen, so a run is a story about
/// one editor area: a document opened, the area split, the layout saved, a preset applied, the
/// layout locked, and a bad configuration refused.
#[derive(Debug, PartialEq, Eq)]
pub struct LayoutOptions {
    /// How many tabs each group's strip shows before overflowing.
    pub limit: usize,
    /// The services this host has, which decide which panels may be shown.
    pub services: Vec<String>,
    /// The operations, in order.
    pub ops: Vec<String>,
}

impl LayoutOptions {
    /// Options for a default layout and no services.
    pub fn new() -> Self {
        Self {
            limit: 8,
            services: Vec::new(),
            ops: Vec::new(),
        }
    }
}

/// How to replay a script of window and display operations (task T04.06).
#[derive(Debug, PartialEq, Eq)]
pub struct WindowsOptions {
    /// The displays this host has, in order (the first is primary).
    pub displays: Vec<DisplayArg>,
    /// With `--main <w>x<h>`, the main window's size.
    pub main_width: Option<u32>,
    /// With `--main <w>x<h>`, the main window's height.
    pub main_height: Option<u32>,
    /// The operations, in order.
    pub ops: Vec<String>,
}

impl WindowsOptions {
    /// Options for a session with no displays named yet and one main window.
    pub fn new() -> Self {
        Self {
            displays: Vec::new(),
            main_width: None,
            main_height: None,
            ops: Vec::new(),
        }
    }
}

/// How to replay a script of window-chrome operations (task T04.07).
#[derive(Debug, PartialEq, Eq)]
pub struct ChromeOptions {
    /// The operations, in order: `focus:<area>`, `type:<text>`, `backspace:<n>`,
    /// `project:<name>`, `document:<path>`, `node:<label>`, `notice:<severity>:<source>:<summary>`,
    /// `toolbar`, `status`, `clear`.
    pub ops: Vec<String>,
}

impl ChromeOptions {
    /// Options for a fresh chrome.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// Parses this family's flags, answering `None` when the flag is another family's.
pub(super) fn parse_shell(
    text: &str,
    _argument: OsString,
    arguments: &mut std::vec::IntoIter<OsString>,
) -> Option<Result<Command, ParseError>> {
    match text {
        "--workbench" => {
            let mut options = WorkbenchOptions::new();
            let rest = arguments;
            while let Some(argument) = rest.next() {
                let flag = argument.to_str().unwrap_or_default();
                match flag {
                    "--dirty" => options.dirty = true,
                    "--mode" | "--project" | "--target" | "--document" | "--message"
                    | "--focus" => {
                        let Some(value) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(value) = value.to_str() else {
                            return Some(Err(ParseError { argument: value }));
                        };
                        match flag {
                            "--mode" => options.mode = Some(value.to_owned()),
                            "--project" => options.project = Some(value.to_owned()),
                            "--target" => options.target = Some(value.to_owned()),
                            "--document" => options.document = Some(value.to_owned()),
                            "--focus" => options.focus = Some(value.to_owned()),
                            _ => options.message = Some(value.to_owned()),
                        }
                    }
                    "--service" | "--show" | "--hide" => {
                        let Some(value) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(name) = value.to_str() else {
                            return Some(Err(ParseError { argument: value }));
                        };
                        match flag {
                            "--service" => options.services.push(name.to_owned()),
                            "--show" => options.show.push(name.to_owned()),
                            _ => options.hide.push(name.to_owned()),
                        }
                    }
                    "--resize" => {
                        let Some(value) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(value) = value.to_str() else {
                            return Some(Err(ParseError { argument: value }));
                        };
                        let Some((id, size)) = value.split_once('=') else {
                            return Some(Err(ParseError {
                                argument: value.into(),
                            }));
                        };
                        let Some(size) = size.parse::<u32>().ok() else {
                            return Some(Err(ParseError {
                                argument: value.into(),
                            }));
                        };
                        options.resize.push((id.to_owned(), size));
                    }
                    _ => return Some(Err(ParseError { argument })),
                }
            }
            return Some(Ok(Command::Workbench(options)));
        }
        "--tabs" => {
            let mut options = TabsOptions::new();
            let rest = arguments;
            while let Some(argument) = rest.next() {
                let value = argument.to_str().unwrap_or_default();
                if value == "--limit" {
                    let Some(count) = rest.next() else {
                        return Some(Err(ParseError { argument }));
                    };
                    let Some(count) = count.to_str().and_then(|value| value.parse::<usize>().ok())
                    else {
                        return Some(Err(ParseError { argument: count }));
                    };
                    options.limit = count;
                    continue;
                }
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Tabs(options)));
        }
        "--groups" => {
            let mut options = GroupsOptions::new();
            let rest = arguments;
            while let Some(argument) = rest.next() {
                let value = argument.to_str().unwrap_or_default();
                if value == "--limit" {
                    let Some(count) = rest.next() else {
                        return Some(Err(ParseError { argument }));
                    };
                    let Some(count) = count.to_str().and_then(|value| value.parse::<usize>().ok())
                    else {
                        return Some(Err(ParseError { argument: count }));
                    };
                    options.limit = count;
                    continue;
                }
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Groups(options)));
        }
        "--layout" => {
            let mut options = LayoutOptions::new();
            let rest = arguments;
            while let Some(argument) = rest.next() {
                let value = argument.to_str().unwrap_or_default();
                if value == "--limit" || value == "--service" {
                    let Some(entry) = rest.next() else {
                        return Some(Err(ParseError { argument }));
                    };
                    if value == "--limit" {
                        let Some(count) =
                            entry.to_str().and_then(|value| value.parse::<usize>().ok())
                        else {
                            return Some(Err(ParseError { argument: entry }));
                        };
                        options.limit = count;
                    } else {
                        let Some(service) = entry.to_str() else {
                            return Some(Err(ParseError { argument: entry }));
                        };
                        options.services.push(service.to_owned());
                    }
                    continue;
                }
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Layout(options)));
        }
        "--windows" => {
            let mut options = WindowsOptions::new();
            let rest = arguments;
            while let Some(argument) = rest.next() {
                let flag = argument.to_str().unwrap_or_default().to_owned();
                match flag.as_str() {
                    "--display" => {
                        let Some(raw) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(text) = raw.to_str() else {
                            return Some(Err(ParseError { argument: raw }));
                        };
                        match parse_display(text) {
                            Some(display) => options.displays.push(display),
                            None => return Some(Err(ParseError { argument: raw })),
                        }
                    }
                    "--main" => {
                        let Some(raw) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(text) = raw.to_str() else {
                            return Some(Err(ParseError { argument: raw }));
                        };
                        match parse_size(text) {
                            Some((width, height)) => {
                                options.main_width = Some(width);
                                options.main_height = Some(height);
                            }
                            None => return Some(Err(ParseError { argument: raw })),
                        }
                    }
                    _ if flag.starts_with("--") => return Some(Err(ParseError { argument })),
                    _ => options.ops.push(flag),
                }
            }
            return Some(Ok(Command::Windows(options)));
        }
        "--chrome" => {
            let mut options = ChromeOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Chrome(options)));
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;

    #[test]
    fn parses_the_editor_group_operations() {
        assert_eq!(
            parse(["--groups"]),
            Ok(Command::Groups(GroupsOptions::new())),
        );
        assert_eq!(
            parse([
                "--groups",
                "--limit",
                "3",
                "split:left-right",
                "view:pages/a.json"
            ]),
            Ok(Command::Groups(GroupsOptions {
                limit: 3,
                ops: vec![
                    "split:left-right".to_owned(),
                    "view:pages/a.json".to_owned()
                ],
            })),
        );

        // A limit that is missing or unparsable is a usage error, and an unknown flag is not an op.
        assert!(parse(["--groups", "--limit"]).is_err());
        assert!(parse(["--groups", "--limit", "many"]).is_err());
        assert!(parse(["--groups", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_layout_operations() {
        assert_eq!(
            parse(["--layout"]),
            Ok(Command::Layout(LayoutOptions::new()))
        );
        assert_eq!(
            parse([
                "--layout",
                "--limit",
                "3",
                "--service",
                "document-tree",
                "preset:columns",
                "zen:on"
            ]),
            Ok(Command::Layout(LayoutOptions {
                limit: 3,
                services: vec!["document-tree".to_owned()],
                ops: vec!["preset:columns".to_owned(), "zen:on".to_owned()],
            })),
        );

        assert!(parse(["--layout", "--limit"]).is_err());
        assert!(parse(["--layout", "--limit", "many"]).is_err());
        assert!(parse(["--layout", "--service"]).is_err());
        assert!(parse(["--layout", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_window_operations() {
        assert_eq!(
            parse(["--windows"]),
            Ok(Command::Windows(WindowsOptions::new())),
        );

        let mut expected = WindowsOptions::new();
        expected.displays.push(DisplayArg {
            id: 0,
            x: 0,
            y: 0,
            width: 1280,
            height: 800,
            scale_milli: 1000,
        });
        expected.main_width = Some(900);
        expected.main_height = Some(700);
        expected.ops = vec![
            "view:pages/a.json".to_owned(),
            "unplug:1".to_owned(),
            "reconcile".to_owned(),
        ];
        assert_eq!(
            parse([
                "--windows",
                "--display",
                "0:0,0,1280,800,1.0",
                "--main",
                "900x700",
                "view:pages/a.json",
                "unplug:1",
                "reconcile"
            ]),
            Ok(Command::Windows(expected)),
        );

        // A scale is kept in thousandths, so a fractional scale survives.
        let Ok(Command::Windows(scaled)) = parse(["--windows", "--display", "2:0,0,800,600,1.5"])
        else {
            panic!("expected a windows command");
        };
        assert_eq!(scaled.displays[0].scale_milli, 1500);
        assert_eq!(scaled.displays[0].width, 800);

        // A missing, malformed or zero-scaled display, and a malformed size, are usage errors.
        assert!(parse(["--windows", "--display"]).is_err());
        assert!(parse(["--windows", "--display", "bad"]).is_err());
        assert!(parse(["--windows", "--display", "0:0,0,1280,800,0"]).is_err());
        assert!(parse(["--windows", "--display", "0:0,0,1280,800"]).is_err());
        assert!(parse(["--windows", "--main", "900"]).is_err());
        assert!(parse(["--windows", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_chrome_operations() {
        assert_eq!(
            parse(["--chrome"]),
            Ok(Command::Chrome(ChromeOptions::new()))
        );
        assert_eq!(
            parse([
                "--chrome",
                "focus:editor",
                "notice:error:build.run:the linker failed",
                "status"
            ]),
            Ok(Command::Chrome(ChromeOptions {
                ops: vec![
                    "focus:editor".to_owned(),
                    "notice:error:build.run:the linker failed".to_owned(),
                    "status".to_owned(),
                ],
            })),
        );

        assert!(parse(["--chrome", "--unknown"]).is_err());
    }
}
