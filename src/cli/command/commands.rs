// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The command surface's commands: the registry, the keymap, the menu, the palette,
//! the clipboard, batch operations and the audit.

// Every branch of the flag table answers with `return`, including the ones that could be a
// trailing expression: the branches are independent flags, and writing them alike is worth more
// than saving a keyword. The returns that are *not* needless — the ones inside a loop that refuses
// a bad value — are why the table reads the way it does.
#![allow(clippy::needless_return)]

use super::*;

/// How to resolve a command through the registry (task T05.01).
///
/// The id and arguments are what a menu item or a shortcut would produce; the rest describes the
/// context the call happens in — where the keyboard is, whether the project is trusted, what effects
/// are authorized and what tasks are already running — so a disabled command can say which of those
/// it was, and an unauthorized one is a different answer from a disabled one.
#[derive(Debug, PartialEq, Eq)]
pub struct CommandOptions {
    /// The command to resolve; absent lists every registered command.
    pub id: Option<String>,
    /// The raw arguments, as a menu or a command line would supply them.
    pub arguments: Vec<(String, String)>,
    /// Where the keyboard is.
    pub focus: String,
    /// Whether the project is trusted.
    pub trust: bool,
    /// The effects that are authorized (names of [`EffectClass`](crate::contracts::permission::EffectClass)).
    pub authorize: Vec<String>,
    /// The tasks already running.
    pub running: Vec<String>,
}

impl CommandOptions {
    /// Options that list every command.
    pub fn listing() -> Self {
        Self {
            id: None,
            arguments: Vec::new(),
            focus: "design".to_owned(),
            trust: false,
            authorize: Vec::new(),
            running: Vec::new(),
        }
    }
}

/// How to resolve a key chord through the keymap (task T05.03).
///
/// The scene is what decides whether a bare key is a character or a command: with `--text-field` a
/// bare character goes to the field, and with `--composing` the keys belong to the input method.
#[derive(Debug, PartialEq, Eq)]
pub struct KeysOptions {
    /// The chord to press, as `Ctrl+K Ctrl+S`; absent lists the bindings.
    pub press: Option<String>,
    /// The mode the designer is in.
    pub mode: String,
    /// The focused panel, when one is.
    pub panel: Option<String>,
    /// Whether a text field has the keyboard.
    pub text_field: bool,
    /// Whether an input method is composing.
    pub composing: bool,
}

impl KeysOptions {
    /// Options that list the bindings.
    pub fn listing() -> Self {
        Self {
            press: None,
            mode: "design".to_owned(),
            panel: None,
            text_field: false,
            composing: false,
        }
    }
}

/// How to search the command palette (task T05.04).
#[derive(Debug, PartialEq, Eq)]
pub struct PaletteOptions {
    /// The query; an empty query lists every command.
    pub query: String,
    /// A command to show the help and parameters for, instead of searching.
    pub help_for: Option<String>,
    /// Where the keyboard is.
    pub focus: String,
    /// Whether the project is trusted.
    pub trust: bool,
    /// The authorized effects.
    pub authorize: Vec<String>,
    /// The tasks already running.
    pub running: Vec<String>,
    /// The recently run commands, most recent first.
    pub recent: Vec<String>,
}

impl PaletteOptions {
    /// Options that list every command.
    pub fn listing() -> Self {
        Self {
            query: String::new(),
            help_for: None,
            focus: "design".to_owned(),
            trust: false,
            authorize: Vec::new(),
            running: Vec::new(),
            recent: Vec::new(),
        }
    }
}

/// How to preview and carry out a batch operation (task T05.07).
///
/// The selection is given as the items the operation applies to (`--select`) and the ones it does not
/// (`--inapplicable id=reason`), so the preview a person sees is the classification itself. No
/// `--choice` prints the preview and changes nothing.
#[derive(Debug, PartialEq, Eq)]
pub struct OperationOptions {
    /// The operation's name.
    pub name: String,
    /// A note for the person.
    pub note: Option<String>,
    /// The external effects the operation has.
    pub effects: Vec<String>,
    /// The selected items the operation applies to.
    pub select: Vec<String>,
    /// The selected items it does not apply to, as `(id, reason)`.
    pub inapplicable: Vec<(String, String)>,
    /// The authorized effects.
    pub authorize: Vec<String>,
    /// What to do: `apply-all`, `apply-none` or `cancel`.
    pub choice: Option<String>,
}

impl OperationOptions {
    /// Options for an operation, changing nothing.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            note: None,
            effects: Vec::new(),
            select: Vec::new(),
            inapplicable: Vec::new(),
            authorize: Vec::new(),
            choice: None,
        }
    }
}

/// What to copy, round-trip or paste on the clipboard (task T05.06).
///
/// One payload at a time, because the clipboard holds one thing: copy text, a resource reference or
/// a node subtree, optionally round-trip it, and optionally paste it into a project.
#[derive(Debug, PartialEq, Eq)]
pub struct ClipboardOptions {
    /// The payload size ceiling, in bytes.
    pub limit: usize,
    /// Copy this text.
    pub text: Option<String>,
    /// With a resource copy, the project it came from.
    pub resource_project: Option<String>,
    /// With a resource copy, the project-relative path.
    pub resource_path: Option<String>,
    /// With a node copy, the page document the subtree is read from.
    pub node_file: Option<PathBuf>,
    /// With a node copy, the id of the subtree's root.
    pub node_id: Option<u64>,
    /// Paste the payload into this project.
    pub paste_into: Option<String>,
    /// Encode the payload and decode it again, reporting whether the two agree.
    pub roundtrip: bool,
}

impl ClipboardOptions {
    /// Options carrying nothing yet.
    pub fn new() -> Self {
        Self {
            limit: 1 << 20,
            text: None,
            resource_project: None,
            resource_path: None,
            node_file: None,
            node_id: None,
            paste_into: None,
            roundtrip: false,
        }
    }
}

/// How to route a keypress and show the entry points that share one state (task T05.05).
#[derive(Debug, PartialEq, Eq)]
pub struct FocusOptions {
    /// Where the keyboard is: a target name (canvas, outline, inspector, editor, tabs, status, welcome).
    pub target: String,
    /// The mode the designer is in.
    pub mode: String,
    /// The chord to press; absent shows the toolbar and the menu instead.
    pub press: Option<String>,
    /// A menu to show as a context menu, by title.
    pub menu: Option<String>,
    /// Whether the project is trusted.
    pub trust: bool,
    /// The authorized effects.
    pub authorize: Vec<String>,
    /// The tasks already running.
    pub running: Vec<String>,
}

impl FocusOptions {
    /// Options that show the toolbar for the canvas.
    pub fn new(target: impl Into<String>) -> Self {
        Self {
            target: target.into(),
            mode: "design".to_owned(),
            press: None,
            menu: None,
            trust: false,
            authorize: Vec::new(),
            running: Vec::new(),
        }
    }
}

/// Parses this family's flags, answering `None` when the flag is another family's.
pub(super) fn parse_commands(
    text: &str,
    argument: OsString,
    arguments: &mut std::vec::IntoIter<OsString>,
) -> Option<Result<Command, ParseError>> {
    match text {
        "--command" => {
            let mut options = CommandOptions::listing();
            let rest = arguments;
            while let Some(argument) = rest.next() {
                let flag = argument.to_str().unwrap_or_default();
                if flag.starts_with("--") {
                    match flag {
                        "--arg" => {
                            let Some(value) = rest.next() else {
                                return Some(Err(ParseError { argument }));
                            };
                            let Some(value) = value.to_str() else {
                                return Some(Err(ParseError { argument: value }));
                            };
                            let Some((name, value)) = value.split_once('=') else {
                                return Some(Err(ParseError {
                                    argument: value.into(),
                                }));
                            };
                            options.arguments.push((name.to_owned(), value.to_owned()));
                        }
                        "--focus" => {
                            let Some(value) = rest.next() else {
                                return Some(Err(ParseError { argument }));
                            };
                            let Some(area) = value.to_str() else {
                                return Some(Err(ParseError { argument: value }));
                            };
                            options.focus = area.to_owned();
                        }
                        "--trust" => options.trust = true,
                        "--authorize" => {
                            let Some(value) = rest.next() else {
                                return Some(Err(ParseError { argument }));
                            };
                            let Some(name) = value.to_str() else {
                                return Some(Err(ParseError { argument: value }));
                            };
                            options.authorize.push(name.to_owned());
                        }
                        "--running" => {
                            let Some(value) = rest.next() else {
                                return Some(Err(ParseError { argument }));
                            };
                            let Some(name) = value.to_str() else {
                                return Some(Err(ParseError { argument: value }));
                            };
                            options.running.push(name.to_owned());
                        }
                        _ => return Some(Err(ParseError { argument })),
                    }
                    continue;
                }
                if options.id.is_some() {
                    return Some(Err(ParseError { argument }));
                }
                options.id = Some(flag.to_owned());
            }
            return Some(Ok(Command::Invoke(options)));
        }
        "--menu" => return Some(Ok(Command::Menu)),
        "--audit" => return Some(Ok(Command::Audit)),
        "--keys" => {
            let mut options = KeysOptions::listing();
            let rest = arguments;
            while let Some(argument) = rest.next() {
                let flag = argument.to_str().unwrap_or_default();
                match flag {
                    "--press" => {
                        let Some(value) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(chord) = value.to_str() else {
                            return Some(Err(ParseError { argument: value }));
                        };
                        options.press = Some(chord.to_owned());
                    }
                    "--mode" => {
                        let Some(value) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(mode) = value.to_str() else {
                            return Some(Err(ParseError { argument: value }));
                        };
                        options.mode = mode.to_owned();
                    }
                    "--panel" => {
                        let Some(value) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(panel) = value.to_str() else {
                            return Some(Err(ParseError { argument: value }));
                        };
                        options.panel = Some(panel.to_owned());
                    }
                    "--text-field" => options.text_field = true,
                    "--composing" => options.composing = true,
                    _ => return Some(Err(ParseError { argument })),
                }
            }
            return Some(Ok(Command::Keys(options)));
        }
        "--palette" => {
            let mut options = PaletteOptions::listing();
            let rest = arguments;
            while let Some(argument) = rest.next() {
                let flag = argument.to_str().unwrap_or_default();
                if flag.starts_with("--") {
                    match flag {
                        "--help-for" => {
                            let Some(value) = rest.next() else {
                                return Some(Err(ParseError { argument }));
                            };
                            let Some(id) = value.to_str() else {
                                return Some(Err(ParseError { argument: value }));
                            };
                            options.help_for = Some(id.to_owned());
                        }
                        "--focus" => {
                            let Some(value) = rest.next() else {
                                return Some(Err(ParseError { argument }));
                            };
                            let Some(area) = value.to_str() else {
                                return Some(Err(ParseError { argument: value }));
                            };
                            options.focus = area.to_owned();
                        }
                        "--trust" => options.trust = true,
                        "--authorize" | "--running" | "--recent" => {
                            let Some(value) = rest.next() else {
                                return Some(Err(ParseError { argument }));
                            };
                            let Some(name) = value.to_str() else {
                                return Some(Err(ParseError { argument: value }));
                            };
                            match flag {
                                "--authorize" => options.authorize.push(name.to_owned()),
                                "--running" => options.running.push(name.to_owned()),
                                _ => options.recent.push(name.to_owned()),
                            }
                        }
                        _ => return Some(Err(ParseError { argument })),
                    }
                    continue;
                }
                if options.query.is_empty() {
                    options.query = flag.to_owned();
                } else {
                    options.query.push(' ');
                    options.query.push_str(flag);
                }
            }
            return Some(Ok(Command::Palette(options)));
        }
        "--operation" => {
            let Some(name) = arguments.next() else {
                return Some(Err(ParseError { argument }));
            };
            let Some(name) = name.to_str() else {
                return Some(Err(ParseError { argument: name }));
            };
            let mut options = OperationOptions::new(name);
            let rest = arguments;
            while let Some(argument) = rest.next() {
                let flag = argument.to_str().unwrap_or_default();
                match flag {
                    "--note" | "--choice" => {
                        let Some(value) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(value) = value.to_str() else {
                            return Some(Err(ParseError { argument: value }));
                        };
                        if flag == "--note" {
                            options.note = Some(value.to_owned());
                        } else {
                            options.choice = Some(value.to_owned());
                        }
                    }
                    "--effect" | "--select" | "--authorize" => {
                        let Some(value) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(value) = value.to_str() else {
                            return Some(Err(ParseError { argument: value }));
                        };
                        match flag {
                            "--effect" => options.effects.push(value.to_owned()),
                            "--select" => options.select.push(value.to_owned()),
                            _ => options.authorize.push(value.to_owned()),
                        }
                    }
                    "--inapplicable" => {
                        let Some(value) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(value) = value.to_str() else {
                            return Some(Err(ParseError { argument: value }));
                        };
                        let Some((id, reason)) = value.split_once('=') else {
                            return Some(Err(ParseError {
                                argument: value.into(),
                            }));
                        };
                        options
                            .inapplicable
                            .push((id.to_owned(), reason.to_owned()));
                    }
                    _ => return Some(Err(ParseError { argument })),
                }
            }
            return Some(Ok(Command::Operation(options)));
        }
        "--clipboard" => {
            let mut options = ClipboardOptions::new();
            let rest = arguments;
            while let Some(argument) = rest.next() {
                // Owned, so the arm below can move `argument` into a `ParseError` without fighting
                // the borrow of a `&str` derived from it.
                let flag = argument.to_str().unwrap_or_default().to_owned();
                if flag == "--roundtrip" {
                    options.roundtrip = true;
                    continue;
                }
                match flag.as_str() {
                    "--limit" | "--copy-text" | "--copy-resource" | "--copy-node"
                    | "--paste-into" => {
                        let Some(raw) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(text) = raw.to_str() else {
                            return Some(Err(ParseError { argument: raw }));
                        };
                        match flag.as_str() {
                            "--limit" => match text.parse::<usize>() {
                                Ok(limit) => options.limit = limit,
                                Err(_) => return Some(Err(ParseError { argument: raw })),
                            },
                            "--copy-text" => options.text = Some(text.to_owned()),
                            "--copy-resource" => match text.split_once(':') {
                                Some((project, path)) => {
                                    options.resource_project = Some(project.to_owned());
                                    options.resource_path = Some(path.to_owned());
                                }
                                None => return Some(Err(ParseError { argument: raw })),
                            },
                            "--copy-node" => match text.split_once('#') {
                                Some((file, id)) => match id.parse::<u64>() {
                                    Ok(id) => {
                                        options.node_file = Some(PathBuf::from(file));
                                        options.node_id = Some(id);
                                    }
                                    Err(_) => return Some(Err(ParseError { argument: raw })),
                                },
                                None => return Some(Err(ParseError { argument: raw })),
                            },
                            _ => options.paste_into = Some(text.to_owned()),
                        }
                    }
                    _ => return Some(Err(ParseError { argument })),
                }
            }
            return Some(Ok(Command::Clipboard(options)));
        }
        "--focus" => {
            let Some(target) = arguments.next() else {
                return Some(Err(ParseError { argument }));
            };
            let Some(target) = target.to_str() else {
                return Some(Err(ParseError { argument: target }));
            };
            let mut options = FocusOptions::new(target);
            let rest = arguments;
            while let Some(argument) = rest.next() {
                let flag = argument.to_str().unwrap_or_default();
                match flag {
                    "--trust" => options.trust = true,
                    "--mode" | "--press" | "--menu" => {
                        let Some(value) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(value) = value.to_str() else {
                            return Some(Err(ParseError { argument: value }));
                        };
                        match flag {
                            "--mode" => options.mode = value.to_owned(),
                            "--press" => options.press = Some(value.to_owned()),
                            _ => options.menu = Some(value.to_owned()),
                        }
                    }
                    "--authorize" | "--running" => {
                        let Some(value) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(value) = value.to_str() else {
                            return Some(Err(ParseError { argument: value }));
                        };
                        if flag == "--authorize" {
                            options.authorize.push(value.to_owned());
                        } else {
                            options.running.push(value.to_owned());
                        }
                    }
                    _ => return Some(Err(ParseError { argument })),
                }
            }
            return Some(Ok(Command::Focus(options)));
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;

    #[test]
    fn parses_the_clipboard_options() {
        assert_eq!(
            parse(["--clipboard"]),
            Ok(Command::Clipboard(ClipboardOptions::new())),
        );

        let mut expected = ClipboardOptions::new();
        expected.limit = 32;
        expected.text = Some("hi".to_owned());
        expected.paste_into = Some("Beta".to_owned());
        expected.roundtrip = true;
        assert_eq!(
            parse([
                "--clipboard",
                "--limit",
                "32",
                "--copy-text",
                "hi",
                "--paste-into",
                "Beta",
                "--roundtrip"
            ]),
            Ok(Command::Clipboard(expected)),
        );

        let Ok(Command::Clipboard(resource)) =
            parse(["--clipboard", "--copy-resource", "Alpha:assets/logo.svg"])
        else {
            panic!("expected a clipboard resource copy");
        };
        assert_eq!(resource.resource_project.as_deref(), Some("Alpha"));
        assert_eq!(resource.resource_path.as_deref(), Some("assets/logo.svg"));

        let Ok(Command::Clipboard(node)) =
            parse(["--clipboard", "--copy-node", "pages/home.json#3"])
        else {
            panic!("expected a clipboard node copy");
        };
        assert_eq!(node.node_file, Some(PathBuf::from("pages/home.json")));
        assert_eq!(node.node_id, Some(3));

        // A limit that is missing or unparsable, and a malformed resource or node, are usage errors.
        assert!(parse(["--clipboard", "--limit", "x"]).is_err());
        assert!(parse(["--clipboard", "--limit"]).is_err());
        assert!(parse(["--clipboard", "--copy-resource", "nocolon"]).is_err());
        assert!(parse(["--clipboard", "--copy-node", "no-hash"]).is_err());
        assert!(parse(["--clipboard", "--huh"]).is_err());
    }
}
