// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! F06: the text editor.
//!
//! One command handler per parsed command of this domain; the dispatcher lives in
//! [`super`](self).

use super::*;

/// Replays in-place text edits over a piece table (task T06.01).
///
/// A keystroke is a splice, and the report makes the acceptance readable: after a one-character
/// `splice`, `reassembled` is still 0 and `additions` grew by one byte, so "the whole text was not
/// cloned" is a number on the line rather than a claim.
pub(super) fn text(options: &crate::cli::TextOptions) -> bool {
    use crate::project::text::piece_table::TextStore;

    let initial = match &options.file {
        Some(file) => match std::fs::read_to_string(file) {
            Ok(text) => text,
            Err(error) => {
                println!("rustage: cannot read {}: {error}", file.display());
                return false;
            }
        },
        None => String::new(),
    };
    let mut store = TextStore::new(initial);
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<String, String> = match verb {
            "splice" => {
                let mut parts = argument.splitn(3, ':');
                match (parts.next(), parts.next(), parts.next()) {
                    (Some(at), Some(remove), Some(insert)) => {
                        match (at.parse::<usize>(), remove.parse::<usize>()) {
                            (Ok(at), Ok(remove)) => store
                                .splice(at, remove, insert)
                                .map(|edit| edit.render())
                                .map_err(|error| error.message()),
                            _ => Err(format!("{argument:?} is not <at>:<remove>:<insert>")),
                        }
                    }
                    _ => Err(format!("{argument:?} is not <at>:<remove>:<insert>")),
                }
            }
            "undo" => store
                .undo()
                .map(|did| {
                    if did {
                        "undone".to_owned()
                    } else {
                        "nothing to undo".to_owned()
                    }
                })
                .map_err(|error| error.message()),
            "redo" => store
                .redo()
                .map(|did| {
                    if did {
                        "redone".to_owned()
                    } else {
                        "nothing to redo".to_owned()
                    }
                })
                .map_err(|error| error.message()),
            "show" => Ok(String::new()),
            other => Err(format!("{other:?} is not a text operation")),
        };
        match step {
            Ok(note) if note.is_empty() => {}
            Ok(note) => println!("{note}"),
            Err(reason) => {
                println!("rustage: {reason}");
                return text_report(&store);
            }
        }
    }
    text_report(&store)
}

/// Shows syntax highlighting: cross-line state, the incremental cache and the theme (task T06.02).
///
/// The script is a story about the cache: lines added, a `step` that lexes some of them, a `set` that
/// makes one line unreliable again, and a `show` that reads the cache without lexing. An unlexed
/// line reads `not lexed yet`, never `clean`, and the tail after an edit that changes an end state is
/// re-lexed by the next `step`.
pub(super) fn highlight(options: &crate::cli::HighlightOptions) -> bool {
    use crate::app::editor::highlight::{Highlights, Language, Theme};

    let language = options
        .language
        .as_deref()
        .and_then(Language::from_name)
        .or_else(|| {
            options
                .file
                .as_deref()
                .and_then(|file| file.to_str())
                .map(Language::of_path)
        })
        .unwrap_or(Language::Plain);
    let theme = options
        .theme
        .as_deref()
        .and_then(Theme::named)
        .unwrap_or_else(Theme::dark);

    let mut highlights = Highlights::new(language);
    if let Some(file) = &options.file {
        match std::fs::read_to_string(file) {
            Ok(text) => highlights.set_text(&text),
            Err(error) => {
                println!("rustage: cannot read {}: {error}", file.display());
                return false;
            }
        }
    }

    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<String, String> = match verb {
            "line" => {
                highlights.push_line(argument);
                Ok(format!("line {} added", highlights.line_count() - 1))
            }
            "step" => match argument.parse::<usize>() {
                Ok(budget) => Ok(format!("lexed {} line(s)", highlights.step(budget))),
                Err(_) => Err(format!("{argument:?} is not a budget")),
            },
            "set" => {
                let mut parts = argument.splitn(2, ':');
                match (parts.next(), parts.next()) {
                    (Some(index), Some(text)) => match index.parse::<usize>() {
                        Ok(index) => {
                            highlights.set_line(index, text);
                            Ok(format!(
                                "line {index} changed; {} line(s) unresolved",
                                highlights.line_count() - highlights.resolved()
                            ))
                        }
                        Err(_) => Err(format!("{index:?} is not a line number")),
                    },
                    _ => Err(format!("{argument:?} is not <line>:<text>")),
                }
            }
            "language" => match Language::from_name(argument) {
                Some(language) => {
                    highlights.set_language(language);
                    Ok(format!("language {}", language.name()))
                }
                None => Err(format!("{argument:?} is not a language")),
            },
            "show" => Ok(String::new()),
            other => Err(format!("{other:?} is not a highlight operation")),
        };
        match step {
            Ok(note) if note.is_empty() => {}
            Ok(note) => println!("{note}"),
            Err(reason) => {
                println!("rustage: {reason}");
                return highlight_report(&highlights, &theme);
            }
        }
    }
    highlight_report(&highlights, &theme)
}

/// The highlight cache's own report.
pub(super) fn highlight_report(
    highlights: &crate::app::editor::highlight::Highlights,
    theme: &crate::app::editor::highlight::Theme,
) -> bool {
    println!("theme: {}", theme.name());
    for line in highlights.render(theme) {
        println!("{line}");
    }
    true
}

/// Converts between document offsets and visual positions (task T06.03).
///
/// A `mark` is placed by the same conversion a `visual` lookup uses, which is the acceptance: the
/// cursor, a diagnostic and a protected region cannot disagree about where a character is.
pub(super) fn coordinates(options: &crate::cli::CoordinatesOptions) -> bool {
    use crate::app::editor::coordinates::{Coordinates, Mark, Metrics};

    // Lines given as `line:` build the document; a file replaces it; everything else is an action.
    let mut lines: Vec<String> = Vec::new();
    let mut actions: Vec<&String> = Vec::new();
    for op in &options.ops {
        match op.strip_prefix("line:") {
            Some(text) => lines.push(text.to_owned()),
            None => actions.push(op),
        }
    }
    let text = match &options.file {
        Some(file) => match std::fs::read_to_string(file) {
            Ok(text) => text,
            Err(error) => {
                println!("rustage: cannot read {}: {error}", file.display());
                return false;
            }
        },
        None => lines.join("\n"),
    };
    let metrics = Metrics::new(options.tab.unwrap_or(4), options.wrap);
    let mut coordinates = Coordinates::new(&text, metrics);

    for op in actions {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<String, String> = match verb {
            "tab" => match argument.parse::<u32>() {
                Ok(width) => {
                    coordinates.set_metrics(Metrics::new(width, coordinates.metrics().wrap));
                    Ok(format!("tab {width}"))
                }
                Err(_) => Err(format!("{argument:?} is not a width")),
            },
            "wrap" => match argument.parse::<u32>() {
                Ok(width) => {
                    coordinates
                        .set_metrics(Metrics::new(coordinates.metrics().tab_width, Some(width)));
                    Ok(format!("wrap {width}"))
                }
                Err(_) => Err(format!("{argument:?} is not a width")),
            },
            "fold" => match argument.split_once(':') {
                Some((start, end)) => match (start.parse::<usize>(), end.parse::<usize>()) {
                    (Ok(start), Ok(end)) => coordinates
                        .fold(start, end)
                        .map(|()| format!("folded {start}..{end}")),
                    _ => Err(format!("{argument:?} is not <start>:<end>")),
                },
                None => Err(format!("{argument:?} is not <start>:<end>")),
            },
            "unfold" => match argument.parse::<usize>() {
                Ok(start) => {
                    coordinates.unfold(start);
                    Ok(format!("unfolded {start}"))
                }
                Err(_) => Err(format!("{argument:?} is not a line")),
            },
            "visual" => match argument.parse::<usize>() {
                Ok(offset) => Ok(format!(
                    "offset {offset} → {}",
                    coordinates.visual_of(offset).render()
                )),
                Err(_) => Err(format!("{argument:?} is not an offset")),
            },
            "offset" => match argument.split_once(':') {
                Some((row, column)) => match (row.parse::<usize>(), column.parse::<u32>()) {
                    (Ok(row), Ok(column)) => {
                        let offset = coordinates
                            .offset_at(crate::app::editor::coordinates::Visual::new(row, column));
                        Ok(format!("row {row} col {column} → offset {offset}"))
                    }
                    _ => Err(format!("{argument:?} is not <row>:<column>")),
                },
                None => Err(format!("{argument:?} is not <row>:<column>")),
            },
            "mark" => {
                let parts: Vec<&str> = argument.split(':').collect();
                match parts.as_slice() {
                    [kind, start, end] => match (start.parse::<usize>(), end.parse::<usize>()) {
                        (Ok(start), Ok(end)) => {
                            let mark = match *kind {
                                "cursor" => Mark::cursor(start),
                                "diagnostic" => Mark::diagnostic(start, end),
                                "protected" => Mark::protected(start, end),
                                _ => {
                                    return coordinates_report(
                                        &coordinates,
                                        &format!("{kind:?} is not a kind"),
                                    );
                                }
                            };
                            Ok(format!("mark → {}", coordinates.project(&mark).render()))
                        }
                        _ => Err(format!("{argument:?} is not <kind>:<start>:<end>")),
                    },
                    _ => Err(format!("{argument:?} is not <kind>:<start>:<end>")),
                }
            }
            "show" => Ok(String::new()),
            other => Err(format!("{other:?} is not a coordinates operation")),
        };
        match step {
            Ok(note) if note.is_empty() => {}
            Ok(note) => println!("{note}"),
            Err(reason) => return coordinates_report(&coordinates, &reason),
        }
    }
    coordinates_report(&coordinates, "")
}

/// The conversion's own report.
pub(super) fn coordinates_report(
    coordinates: &crate::app::editor::coordinates::Coordinates,
    reason: &str,
) -> bool {
    if !reason.is_empty() {
        println!("rustage: {reason}");
    }
    for line in coordinates.render() {
        println!("{line}");
    }
    reason.is_empty()
}

/// Runs multi-cursor editing: carets, insertions, line operations, comments and brackets (task
/// T06.04).
///
/// The report is the acceptance on the line. After a multi-caret `insert`, a single `undo` takes the
/// whole keystroke back rather than one caret's share of it, and an edit that would land inside a
/// character is refused whole, so `text:` never shows half a character.
pub(super) fn edit_ops(options: &crate::cli::EditOpsOptions) -> bool {
    use crate::app::editor::editing::{Carets, Editor, PAIRS, Selection, matching_bracket};
    use crate::project::text::piece_table::TextStore;

    // Lines given as `line:` build the document; a file replaces it; everything else is an action.
    let mut lines: Vec<String> = Vec::new();
    let mut actions: Vec<&String> = Vec::new();
    for op in &options.ops {
        match op.strip_prefix("line:") {
            Some(text) => lines.push(text.to_owned()),
            None => actions.push(op),
        }
    }
    let text = match &options.file {
        Some(file) => match std::fs::read_to_string(file) {
            Ok(text) => text,
            Err(error) => {
                println!("rustage: cannot read {}: {error}", file.display());
                return false;
            }
        },
        None => lines.join("\n"),
    };
    let mut store = TextStore::new(text);
    let mut editor = Editor::new();

    for op in actions {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<String, String> = match verb {
            "carets" => match carets_of(argument) {
                Ok(carets) => match editor.set_carets(&store.text(), carets) {
                    Ok(()) => Ok(format!("carets: {}", editor.carets().len())),
                    Err(reason) => Err(reason),
                },
                Err(reason) => Err(reason),
            },
            "select" => match argument.split_once(':') {
                Some((anchor, cursor)) => {
                    match (anchor.parse::<usize>(), cursor.parse::<usize>()) {
                        (Ok(anchor), Ok(cursor)) => {
                            match Carets::of(vec![Selection::range(anchor, cursor)]) {
                                Ok(carets) => match editor.set_carets(&store.text(), carets) {
                                    Ok(()) => Ok(format!(
                                        "selection {}..{}",
                                        anchor.min(cursor),
                                        anchor.max(cursor)
                                    )),
                                    Err(reason) => Err(reason),
                                },
                                Err(reason) => Err(reason),
                            }
                        }
                        _ => Err(format!("{argument:?} is not <anchor>:<cursor>")),
                    }
                }
                None => Err(format!("{argument:?} is not <anchor>:<cursor>")),
            },
            "insert" => editor
                .insert(&mut store, argument)
                .map(|()| format!("inserted {argument:?}"))
                .map_err(|error| error.message()),
            "type" => match single_char(argument) {
                Some(character) => editor
                    .type_char(&mut store, character, &PAIRS)
                    .map(|()| format!("typed {character:?}"))
                    .map_err(|error| error.message()),
                None => Err(format!("{argument:?} is not one character")),
            },
            "backspace" => editor
                .backspace(&mut store)
                .map(|()| "backspaced".to_owned())
                .map_err(|error| error.message()),
            "delete" => editor
                .delete_forward(&mut store)
                .map(|()| "deleted forward".to_owned())
                .map_err(|error| error.message()),
            "up" => editor
                .move_lines(&mut store, -1)
                .map(|()| "moved up".to_owned())
                .map_err(|error| error.message()),
            "down" => editor
                .move_lines(&mut store, 1)
                .map(|()| "moved down".to_owned())
                .map_err(|error| error.message()),
            "duplicate" => editor
                .duplicate_lines(&mut store)
                .map(|()| "duplicated".to_owned())
                .map_err(|error| error.message()),
            "delete-lines" => editor
                .delete_lines(&mut store)
                .map(|()| "deleted lines".to_owned())
                .map_err(|error| error.message()),
            "comment" => editor
                .toggle_comment(&mut store, argument)
                .map(|()| format!("toggled comment {argument:?}"))
                .map_err(|error| error.message()),
            "matching" => match argument.parse::<usize>() {
                Ok(offset) => match matching_bracket(&store.text(), offset) {
                    Some(at) => Ok(format!("bracket at {offset} matches {at}")),
                    None => Ok(format!("bracket at {offset} has no match")),
                },
                Err(_) => Err(format!("{argument:?} is not an offset")),
            },
            "undo" => store
                .undo()
                .map(|did| {
                    if did {
                        "undone".to_owned()
                    } else {
                        "nothing to undo".to_owned()
                    }
                })
                .map_err(|error| error.message()),
            "redo" => store
                .redo()
                .map(|did| {
                    if did {
                        "redone".to_owned()
                    } else {
                        "nothing to redo".to_owned()
                    }
                })
                .map_err(|error| error.message()),
            "show" => Ok(String::new()),
            other => Err(format!("{other:?} is not an editing operation")),
        };
        match step {
            Ok(note) if note.is_empty() => {}
            Ok(note) => println!("{note}"),
            Err(reason) => return edit_report(&store, &editor, &reason),
        }
    }
    edit_report(&store, &editor, "")
}

/// Parses an `o1:o2:...` argument into carets, one per offset.
pub(super) fn carets_of(argument: &str) -> Result<crate::app::editor::editing::Carets, String> {
    use crate::app::editor::editing::{Carets, Selection};

    let mut selections = Vec::new();
    for number in argument.split(':') {
        match number.parse::<usize>() {
            Ok(offset) => selections.push(Selection::caret(offset)),
            Err(_) => return Err(format!("{number:?} is not an offset")),
        }
    }
    Carets::of(selections)
}

/// The one character an argument is, or `None` when it is not exactly one character.
pub(super) fn single_char(argument: &str) -> Option<char> {
    let mut characters = argument.chars();
    let character = characters.next()?;
    if characters.next().is_none() {
        Some(character)
    } else {
        None
    }
}

/// The text store's and editor's own report.
pub(super) fn edit_report(
    store: &crate::project::text::piece_table::TextStore,
    editor: &crate::app::editor::editing::Editor,
    reason: &str,
) -> bool {
    if !reason.is_empty() {
        println!("rustage: {reason}");
    }
    for line in store.render() {
        println!("{line}");
    }
    for line in editor.render() {
        println!("{line}");
    }
    println!("text: {:?}", store.text());
    reason.is_empty()
}

/// Reads a file's encoding, line endings and indentation, and shows the editor's decoration (task
/// T06.05).
///
/// The policy is the report: a file that is not UTF-8 is refused by name and the command ends
/// non-zero, and a file that mixed its line endings is reported as mixed and cannot be encoded by
/// default. The decoration — visible whitespace, the gutter, fold regions and a minimap — is printed
/// from the same text, so the view cannot disagree with the file it was read from.
pub(super) fn editor_view(options: &crate::cli::EditorViewOptions) -> bool {
    use crate::app::editor::editor_view::{FoldKind, View, fold_regions, minimap};
    use crate::project::text::text_file::{Ending, TextFile};

    // `line:` operations build an in-memory file; `--file` reads bytes; the rest are actions.
    let mut lines: Vec<String> = Vec::new();
    let mut actions: Vec<&String> = Vec::new();
    for op in &options.ops {
        match op.strip_prefix("line:") {
            Some(text) => lines.push(text.to_owned()),
            None => actions.push(op),
        }
    }
    let file = match &options.file {
        Some(path) => match std::fs::read(path) {
            Ok(bytes) => match TextFile::open(&bytes) {
                Ok(file) => file,
                Err(error) => {
                    println!("rustage: {}: {}", path.display(), error.message());
                    return false;
                }
            },
            Err(error) => {
                println!("rustage: cannot read {}: {error}", path.display());
                return false;
            }
        },
        None => TextFile::from_text(&lines.join("\n")),
    };

    for line in file.render() {
        println!("{line}");
    }

    let mut view = View::new().with_tab_width(options.tab.unwrap_or(4));
    for op in actions {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "whitespace" => {
                view = view.with_whitespace();
                Ok(vec!["whitespace: shown".to_owned()])
            }
            "numbers" => {
                view = if argument == "relative" {
                    view.with_relative_numbers()
                } else {
                    view
                };
                Ok(vec![format!(
                    "numbers: {}",
                    if view.relative_numbers {
                        "relative"
                    } else {
                        "absolute"
                    }
                )])
            }
            "window" => match window_of(argument) {
                Ok((first, count, current)) => Ok(view.lines(&file.text(), first, count, current)),
                Err(reason) => Err(reason),
            },
            "fold" => match FoldKind::from_name(argument) {
                Some(kind) => Ok(fold_regions(&file.text(), kind)
                    .iter()
                    .map(|region| region.render())
                    .collect()),
                None => Err(format!("{argument:?} is not a fold kind")),
            },
            "minimap" => match argument.parse::<usize>() {
                Ok(height) => Ok(minimap(&file.text(), height)
                    .iter()
                    .map(|row| row.render())
                    .collect()),
                Err(_) => Err(format!("{argument:?} is not a height")),
            },
            "encode" => match file.encode() {
                Ok(bytes) => Ok(vec![format!("encode: {} byte(s)", bytes.len())]),
                Err(error) => Err(error.message()),
            },
            "encode-with" => match Ending::from_name(argument) {
                Some(ending) => Ok(vec![format!(
                    "encode-with {}: {} byte(s)",
                    ending.render(),
                    file.encode_with(ending).len()
                )]),
                None => Err(format!("{argument:?} is not an ending")),
            },
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not an editor operation")),
        };
        match step {
            Ok(notes) => {
                for note in notes {
                    println!("{note}");
                }
            }
            Err(reason) => {
                println!("rustage: {reason}");
                return false;
            }
        }
    }
    true
}

/// Parses a `first:count[:current]` argument into a window of lines.
pub(super) fn window_of(argument: &str) -> Result<(usize, usize, Option<usize>), String> {
    let parts: Vec<&str> = argument.split(':').collect();
    let parse = |value: &str| {
        value
            .parse::<usize>()
            .map_err(|_| format!("{value:?} is not a number"))
    };
    match parts.as_slice() {
        [first, count] => Ok((parse(first)?, parse(count)?, None)),
        [first, count, current] => Ok((parse(first)?, parse(count)?, Some(parse(current)?))),
        _ => Err(format!("{argument:?} is not <first>:<count>[:<current>]")),
    }
}

/// Checks writes against protected regions through every entry point (task T06.06).
///
/// Each write operation prints `allowed` or `refused: <reason>`, so one run shows a positive and a
/// negative case for every door. A refused write is the guard doing its job, not a command failure:
/// the exit code is non-zero only when an operation itself is malformed.
pub(super) fn write_guard(options: &crate::cli::WriteGuardOptions) -> bool {
    use crate::project::document::conflict::Writability;
    use crate::project::text::protection::{ProtectedRegion, Write, WriteGuard, WriteSource};

    let mut regions: Vec<ProtectedRegion> = Vec::new();
    let mut writability = Writability::Writable;

    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "protect" => match two_numbers(argument) {
                Ok((start, end)) => {
                    regions.push(ProtectedRegion::new(start, end));
                    Ok(vec![format!("protected {start}..{end}")])
                }
                Err(reason) => Err(reason),
            },
            "writable" => {
                writability = Writability::Writable;
                Ok(vec!["writable".to_owned()])
            }
            "read-only" => {
                let reason = if argument.is_empty() {
                    "declared read-only".to_owned()
                } else {
                    argument.to_owned()
                };
                writability = Writability::ReadOnly {
                    reason: reason.clone(),
                };
                Ok(vec![format!("read-only: {reason}")])
            }
            "input" | "paste" => match source_and_write(verb, argument, false) {
                Ok((source, write)) => Ok(vec![verdict(&regions, &writability, source, write)]),
                Err(reason) => Err(reason),
            },
            "replace" | "format" => match source_and_write(verb, argument, true) {
                Ok((source, write)) => Ok(vec![verdict(&regions, &writability, source, write)]),
                Err(reason) => Err(reason),
            },
            "external" => match two_numbers(argument) {
                Ok((at, remove)) => Ok(vec![verdict(
                    &regions,
                    &writability,
                    WriteSource::External,
                    Write::delete(at, remove),
                )]),
                Err(reason) => Err(reason),
            },
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a write-guard operation")),
        };
        match step {
            Ok(notes) => {
                for note in notes {
                    println!("{note}");
                }
            }
            Err(reason) => {
                println!("rustage: {reason}");
                return false;
            }
        }
    }

    for line in WriteGuard::new(regions, writability).render() {
        println!("{line}");
    }
    true
}

/// The number of a write's entry point and its shape: an insertion, or a replacement.
pub(super) fn source_and_write(
    verb: &str,
    argument: &str,
    replacement: bool,
) -> Result<
    (
        crate::project::text::protection::WriteSource,
        crate::project::text::protection::Write,
    ),
    String,
> {
    use crate::project::text::protection::{Write, WriteSource};

    let source = WriteSource::from_name(verb).expect("the caller passes a write source name");
    if replacement {
        match three_numbers(argument) {
            Ok((at, remove, insert)) => Ok((source, Write::replace(at, remove, insert))),
            Err(reason) => Err(reason),
        }
    } else {
        match two_numbers(argument) {
            Ok((at, insert)) => Ok((source, Write::insert(at, insert))),
            Err(reason) => Err(reason),
        }
    }
}

/// Parses a `first:second:third` triple of numbers.
pub(super) fn three_numbers(argument: &str) -> Result<(usize, usize, usize), String> {
    match numbers(argument).as_deref() {
        Some([first, second, third]) => Ok((*first, *second, *third)),
        _ => Err(format!("{argument:?} is not <first>:<second>:<third>")),
    }
}

/// Classifies a document's size and shows which features are degraded, and why (task T06.07).
///
/// The report names the tier, the size boundaries it used, how much of the document the highlight
/// cache keeps, and every feature with `on` or `off — <reason>`; a degradation always has a reason.
pub(super) fn scale(options: &crate::cli::ScaleOptions) -> bool {
    use crate::app::editor::scale::{Cancellation, Limits, Scale, read_cancellable};

    // `line:` builds a document; `fill:`/`long-line:` synthesise one; `--file` reads one.
    let mut lines: Vec<String> = Vec::new();
    let mut synthetic: Option<String> = None;
    let mut actions: Vec<&String> = Vec::new();
    for op in &options.ops {
        if let Some(text) = op.strip_prefix("line:") {
            lines.push(text.to_owned());
        } else if let Some(size) = op.strip_prefix("fill:") {
            match size.parse::<usize>() {
                Ok(bytes) => synthetic = Some(fill(bytes)),
                Err(_) => {
                    println!("rustage: {size:?} is not a size");
                    return false;
                }
            }
        } else if let Some(columns) = op.strip_prefix("long-line:") {
            match columns.parse::<usize>() {
                Ok(columns) => synthetic = Some("x".repeat(columns)),
                Err(_) => {
                    println!("rustage: {columns:?} is not a width");
                    return false;
                }
            }
        } else {
            actions.push(op);
        }
    }
    let text = match &options.file {
        Some(path) => match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) => {
                println!("rustage: cannot read {}: {error}", path.display());
                return false;
            }
        },
        None => synthetic.unwrap_or_else(|| lines.join("\n")),
    };

    for op in actions {
        let (verb, _argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        match verb {
            "read" | "read-cancel" => {
                let Some(path) = &options.file else {
                    println!("rustage: {verb} needs --file");
                    return false;
                };
                let token = Cancellation::new();
                if verb == "read-cancel" {
                    token.cancel();
                }
                match read_cancellable(path, &token, options.chunk.unwrap_or(1 << 16)) {
                    Ok(bytes) if verb == "read" => println!("read: {} byte(s)", bytes.len()),
                    Ok(_) => {
                        println!("rustage: read-cancel read the file anyway");
                        return false;
                    }
                    Err(error) => println!("{}: {}", verb, error.message()),
                }
            }
            "show" => {}
            other => {
                println!("rustage: {other:?} is not a scale operation");
                return false;
            }
        }
    }

    for line in Scale::of(&text, Limits::new()).render() {
        println!("{line}");
    }
    true
}

/// Expands a language snippet, walks its placeholders, and surrounds a selection (task T06.09).
pub(super) fn snippet(options: &crate::cli::SnippetOptions) -> bool {
    use crate::app::editor::editing::Editor;
    use crate::app::editor::snippets::{Session, Snippet, surround_selection};
    use crate::project::document::conflict::Writability;
    use crate::project::text::piece_table::TextStore;
    use crate::project::text::protection::{ProtectedRegion, WriteGuard};

    let mut store = TextStore::new(String::new());
    let mut editor = Editor::new();
    let mut regions: Vec<ProtectedRegion> = Vec::new();
    let mut writability = Writability::Writable;
    let mut snippets: Vec<Snippet> = Vec::new();
    let mut session: Option<Session> = None;

    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "text" => {
                store = TextStore::new(unescape(argument));
                session = None;
                editor = Editor::new();
                Ok(vec![format!("text: {} byte(s)", store.len())])
            }
            "snippet" => match argument.split_once(':') {
                Some((prefix, body)) => match Snippet::parse(prefix, &unescape(body)) {
                    Ok(parsed) => {
                        let prefix = parsed.prefix().to_owned();
                        snippets.retain(|snippet| snippet.prefix() != prefix);
                        snippets.push(parsed);
                        Ok(vec![format!("snippet: {prefix}")])
                    }
                    Err(error) => Err(error.message()),
                },
                None => Err(format!("{argument:?} is not <prefix>:<body>")),
            },
            "protect" => match two_numbers(argument) {
                Ok((start, end)) => {
                    regions.push(ProtectedRegion::new(start, end));
                    Ok(vec![format!("protected {start}..{end}")])
                }
                Err(reason) => Err(reason),
            },
            "writable" => {
                writability = Writability::Writable;
                Ok(vec!["writable".to_owned()])
            }
            "read-only" => {
                let reason = if argument.is_empty() {
                    "declared read-only".to_owned()
                } else {
                    argument.to_owned()
                };
                writability = Writability::ReadOnly {
                    reason: reason.clone(),
                };
                Ok(vec![format!("read-only: {reason}")])
            }
            "cursor" | "select" | "add-caret" | "add-select" => {
                if let Err(reason) = place_carets(&mut editor, &store.text(), verb, argument) {
                    return report(reason);
                }
                session = None;
                Ok(vec![format!("carets: {}", editor.carets().len())])
            }
            "expand" => {
                let Some(found) = snippets.iter().find(|snippet| snippet.prefix() == argument)
                else {
                    return report(format!("{argument:?} is not a known snippet"));
                };
                let guard = WriteGuard::new(regions.clone(), writability.clone());
                match Session::start(&mut store, editor.carets(), found, &guard) {
                    Ok(started) => {
                        let carets = started.carets();
                        if let Err(reason) = editor.set_carets(&store.text(), carets) {
                            return report(reason);
                        }
                        session = Some(started);
                        Ok(vec![format!(
                            "expanded {argument} at {} caret(s)",
                            editor.carets().len()
                        )])
                    }
                    Err(error) => Err(error.message()),
                }
            }
            "tab" | "shift-tab" => {
                let Some(active) = session.as_mut() else {
                    return report(format!("{verb} without an expand"));
                };
                let moved = if verb == "tab" {
                    active.jump_next()
                } else {
                    active.jump_previous()
                };
                let carets = active.carets();
                let label = format!("stop {} of {}", active.step(), active.stops());
                if let Err(reason) = editor.set_carets(&store.text(), carets) {
                    return report(reason);
                }
                Ok(vec![format!("{verb}: {label}, moved={moved}")])
            }
            "type" => {
                let text = unescape(argument);
                let guard = WriteGuard::new(regions.clone(), writability.clone());
                let handled = if let Some(active) = session.as_mut() {
                    match active.type_text(&mut store, &guard, &text) {
                        Ok(true) => Some(format!("stop {} of {}", active.step(), active.stops())),
                        Ok(false) => None,
                        Err(error) => return report(error.message()),
                    }
                } else {
                    None
                };
                match handled {
                    Some(label) => {
                        let carets = session.as_ref().expect("a live session").carets();
                        if let Err(reason) = editor.set_carets(&store.text(), carets) {
                            return report(reason);
                        }
                        Ok(vec![format!("type: {label}")])
                    }
                    None => match editor.insert(&mut store, &text) {
                        Ok(()) => Ok(vec![format!("insert: {:?}", text)]),
                        Err(error) => Err(error.message()),
                    },
                }
            }
            "surround" => match argument.split_once(':') {
                Some((open, close)) => {
                    let (Some(open), Some(close)) = (single_char(open), single_char(close)) else {
                        return report(format!("{argument:?} is not <open>:<close>"));
                    };
                    let guard = WriteGuard::new(regions.clone(), writability.clone());
                    match surround_selection(&mut store, &mut editor, open, close, &guard) {
                        Ok(()) => {
                            session = None;
                            Ok(vec![format!("surrounded with {open}{close}")])
                        }
                        Err(error) => Err(error.message()),
                    }
                }
                None => Err(format!("{argument:?} is not <open>:<close>")),
            },
            "cancel" => {
                if let Some(active) = session.as_mut() {
                    active.cancel();
                    Ok(vec!["cancelled".to_owned()])
                } else {
                    Err("cancel without an expand".to_owned())
                }
            }
            "undo" => match store.undo() {
                Ok(true) => Ok(vec!["undo: taken".to_owned()]),
                Ok(false) => Ok(vec!["undo: nothing to take back".to_owned()]),
                Err(error) => Err(error.message()),
            },
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a snippet operation")),
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

    println!("text: {:?}", store.text());
    if let Some(active) = &session {
        for line in active.render() {
            println!("{line}");
        }
    }
    true
}

/// Places carets from a `cursor`/`select`/`add-caret`/`add-select` operation.
pub(super) fn place_carets(
    editor: &mut crate::app::editor::editing::Editor,
    text: &str,
    verb: &str,
    argument: &str,
) -> Result<(), String> {
    use crate::app::editor::editing::{Carets, Selection};
    let selection = match verb {
        "cursor" | "add-caret" => match argument.parse::<usize>() {
            Ok(at) => Selection::caret(at),
            Err(_) => return Err(format!("{argument:?} is not an offset")),
        },
        _ => match two_numbers(argument) {
            Ok((start, end)) => Selection::range(start, end),
            Err(reason) => return Err(reason),
        },
    };
    let carets = if verb.starts_with("add-") {
        let mut carets = editor.carets().clone();
        carets.add(selection)?;
        carets
    } else {
        Carets::of(vec![selection])?
    };
    editor.set_carets(text, carets)
}

/// Replays the editing regression scenarios and rechecks the size boundaries (task T06.08).
pub(super) fn regression(options: &crate::cli::RegressionOptions) -> bool {
    use crate::app::editor::regression::{Scenario, boundaries, run};

    let mut ok = true;
    println!("size boundaries:");
    for check in boundaries() {
        println!("{}", check.render());
        ok &= check.passed;
    }

    let scenarios = if options.scenarios.is_empty() {
        Scenario::ALL.to_vec()
    } else {
        let mut chosen = Vec::new();
        for name in &options.scenarios {
            match Scenario::from_name(name) {
                Some(scenario) => chosen.push(scenario),
                None => {
                    println!("rustage: {name:?} is not a regression scenario");
                    return false;
                }
            }
        }
        chosen
    };

    for scenario in scenarios {
        let lines = options.lines.unwrap_or_else(|| scenario.default_lines());
        println!();
        let measurement = run(scenario, lines);
        for line in measurement.render() {
            println!("{line}");
        }
        ok &= measurement.passed();
    }
    ok
}

/// Grows a selection, keeps edit locations, and toggles display elements (task T06.10).
pub(super) fn structural(options: &crate::cli::StructuralOptions) -> bool {
    use crate::app::editor::editing::Selection;
    use crate::app::editor::structural::{Bookmarks, Capability, Toggle, Toggles, expand};

    let mut text = String::new();
    let mut selection = Selection::caret(0);
    let mut bookmarks = Bookmarks::new();
    let mut toggles = Toggles::new();

    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "text" => {
                text = unescape(argument);
                selection = Selection::caret(0);
                Ok(vec![format!("text: {} byte(s)", text.len())])
            }
            "cursor" | "select" => match place_selection(&text, verb, argument) {
                Ok(placed) => {
                    selection = placed;
                    Ok(vec![format!("selection: {}", selection.render())])
                }
                Err(reason) => Err(reason),
            },
            "expand" => {
                let count = argument.parse::<usize>().unwrap_or(1);
                let mut notes = Vec::new();
                for _ in 0..count {
                    // No syntax capability is wired yet, so the ladder is the text one.
                    let grown = expand(&text, selection, &Capability::TextOnly);
                    selection = grown.selection;
                    notes.push(format!("expand: {}", grown.render()));
                }
                Ok(notes)
            }
            "syntax" => {
                // A syntax capability that knows no spans: every rung falls back to text, and says so.
                let count = argument.parse::<usize>().unwrap_or(1);
                let mut notes = Vec::new();
                for _ in 0..count {
                    let grown = expand(&text, selection, &Capability::Syntax(Vec::new()));
                    selection = grown.selection;
                    notes.push(format!("expand (syntax): {}", grown.render()));
                }
                Ok(notes)
            }
            "bookmark" => match argument.split_once(':') {
                Some((name, offset)) => match offset.parse::<usize>() {
                    Ok(offset) => {
                        bookmarks.add(name, offset);
                        Ok(vec![format!("bookmark: {name} at {offset}")])
                    }
                    Err(_) => Err(format!("{offset:?} is not an offset")),
                },
                None => Err(format!("{argument:?} is not <name>:<offset>")),
            },
            "drop" => {
                let removed = bookmarks.remove(argument);
                Ok(vec![format!("drop: {argument}, removed={removed}")])
            }
            "edit" => {
                let numbers: Vec<usize> = argument
                    .split(':')
                    .filter_map(|piece| piece.parse::<usize>().ok())
                    .collect();
                let [at, remove, insert_len] = numbers.as_slice() else {
                    return report(format!("{argument:?} is not <at>:<remove>:<insert>"));
                };
                // Keep the text, the caret and the bookmarks consistent with one another.
                if text.is_char_boundary(*at) {
                    let end = at + remove;
                    if text.is_char_boundary(end.min(text.len())) && end <= text.len() {
                        let replacement = "x".repeat(*insert_len);
                        text.replace_range(*at..end, &replacement);
                        bookmarks.translate(*at, *remove, *insert_len);
                        Ok(vec![format!("edit: {text:?}")])
                    } else {
                        Err(format!("the edit at {at} is not on a character boundary"))
                    }
                } else {
                    Err(format!("the edit at {at} is not on a character boundary"))
                }
            }
            "toggle" => match Toggle::from_name(argument) {
                Some(toggle) => {
                    let target = !toggles.is_on(toggle);
                    match toggles.set(toggle, target) {
                        Ok(()) => Ok(vec![format!("toggle {}: {}", toggle.name(), target)]),
                        Err(reason) => {
                            Ok(vec![format!("toggle {}: refused: {reason}", toggle.name())])
                        }
                    }
                }
                None => Err(format!("{argument:?} is not a display toggle")),
            },
            "baseline" => {
                toggles.set_baseline(crate::app::search::diff::Revision::new(
                    "baseline",
                    unescape(argument),
                ));
                Ok(vec![format!("baseline: {} byte(s)", argument.len())])
            }
            "inline" => {
                let rows = toggles.inline_rows(&text);
                let mut notes = vec![format!("inline: {} row(s)", rows.len())];
                for row in rows {
                    notes.push(format!("  {}", row.render()));
                }
                Ok(notes)
            }
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a structural operation")),
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

    println!("text: {:?}", text);
    println!("selection: {}", selection.render());
    for line in bookmarks.render() {
        println!("{line}");
    }
    for line in toggles.render() {
        println!("{line}");
    }
    true
}

/// A selection from a `cursor`/`select` operation.
pub(super) fn place_selection(
    text: &str,
    verb: &str,
    argument: &str,
) -> Result<crate::app::editor::editing::Selection, String> {
    use crate::app::editor::editing::Selection;
    if verb == "cursor" {
        let at = argument
            .parse::<usize>()
            .map_err(|_| format!("{argument:?} is not an offset"))?;
        return Ok(Selection::caret(at.min(text.len())));
    }
    match two_numbers(argument) {
        Ok((start, end)) => Ok(Selection::range(start.min(text.len()), end.min(text.len()))),
        Err(reason) => Err(reason),
    }
}

/// A synthetic document of at least `bytes` bytes: whole lines of 79 columns and a newline.
pub(super) fn fill(bytes: usize) -> String {
    let line = "a".repeat(79);
    let mut text = String::with_capacity(bytes + 80);
    while text.len() < bytes {
        text.push_str(&line);
        text.push('\n');
    }
    text
}
