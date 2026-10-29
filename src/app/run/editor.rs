// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! F06: the text editor.
//!
//! One command handler per parsed command of this domain; the dispatcher lives in
//! [`super`](self).

use super::*;
use rust_widgets::widget::special_widgets::code_editor::{LanguageId, TextPosition};

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

/// Converts between document offsets and the widget's visual rows (task T06.03).
///
/// The conversion is the widget's: `CodeEditor::visual_lines` decides how a line wraps and which
/// rows a fold hides, and `TextPosition` is the one position the caret, a diagnostic and a
/// protected region use. The command line still speaks byte offsets, so [`position_at`] is the one
/// boundary conversion; the host keeps no second coordinate model.
pub(super) fn coordinates(options: &crate::cli::CoordinatesOptions) -> bool {
    use crate::app::editor::adapter::Adapter;

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
    let mut adapter = match Adapter::with_options(
        &text,
        options.wrap.is_some(),
        options.tab.unwrap_or(4) as usize,
    ) {
        Ok(adapter) => adapter,
        Err(reason) => {
            println!("rustage: the editor refused its configuration: {reason}");
            return false;
        }
    };

    for op in actions {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<String, String> = match verb {
            "tab" => match argument.parse::<usize>() {
                Ok(width) => {
                    adapter.set_tab_width(width);
                    Ok(format!("tab {width}"))
                }
                Err(_) => Err(format!("{argument:?} is not a width")),
            },
            "wrap" => match argument.parse::<u32>() {
                Ok(width) => Ok(format!(
                    "wrap {width}: the widget wraps at its own width ({} for this run)",
                    if options.wrap.is_some() {
                        "enabled"
                    } else {
                        "disabled"
                    }
                )),
                Err(_) => Err(format!("{argument:?} is not a width")),
            },
            "fold" => match argument.split_once(':') {
                Some((start, end)) => match (start.parse::<usize>(), end.parse::<usize>()) {
                    (Ok(start), Ok(end)) => {
                        adapter.fold(start, end);
                        Ok(format!("folded {start}..{end}"))
                    }
                    _ => Err(format!("{argument:?} is not <start>:<end>")),
                },
                None => Err(format!("{argument:?} is not <start>:<end>")),
            },
            "unfold" => match argument.parse::<usize>() {
                Ok(start) => {
                    adapter.unfold(start);
                    Ok(format!("unfolded {start}"))
                }
                Err(_) => Err(format!("{argument:?} is not a line")),
            },
            "visual" => match argument.parse::<usize>() {
                Ok(offset) => match position_at(&text, offset) {
                    Ok(position) => Ok(format!(
                        "offset {offset} → {}",
                        visual_of(&adapter, position)
                    )),
                    Err(reason) => Err(reason),
                },
                Err(_) => Err(format!("{argument:?} is not an offset")),
            },
            "offset" => match argument.split_once(':') {
                Some((row, column)) => match (row.parse::<usize>(), column.parse::<usize>()) {
                    (Ok(row), Ok(column)) => match visual_at(&adapter, &text, row, column) {
                        Some((position, offset)) => Ok(format!(
                            "row {row} col {column} → line {} char {} → offset {offset}",
                            position.line, position.column
                        )),
                        None => Err(format!(
                            "row {row} is past the {} visible visual row(s)",
                            adapter.visual_lines().len()
                        )),
                    },
                    _ => Err(format!("{argument:?} is not <row>:<column>")),
                },
                None => Err(format!("{argument:?} is not <row>:<column>")),
            },
            "mark" => {
                let parts: Vec<&str> = argument.split(':').collect();
                match parts.as_slice() {
                    [kind, start, end] => match (start.parse::<usize>(), end.parse::<usize>()) {
                        (Ok(start), Ok(end)) => {
                            match (position_at(&text, start), position_at(&text, end)) {
                                (Ok(start_at), Ok(end_at)) => Ok(format!(
                                    "mark {kind} → {} .. {}",
                                    visual_of(&adapter, start_at),
                                    visual_of(&adapter, end_at)
                                )),
                                (Err(reason), _) | (_, Err(reason)) => Err(reason),
                            }
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
            Err(reason) => return coordinates_report(&adapter, &reason),
        }
    }
    coordinates_report(&adapter, "")
}

/// A position as the widget's visual row, for a report.
fn visual_of(adapter: &crate::app::editor::adapter::Adapter, position: TextPosition) -> String {
    let mut found = None;
    for (row, line) in adapter.visual_lines().iter().enumerate() {
        if line.line == position.line && line.start_column <= position.column {
            found = Some((row, *line));
            if position.column <= line.end_column {
                break;
            }
        }
    }
    match found {
        Some((row, line)) => format!(
            "row {row} (line {}, characters {}..{})",
            line.line, line.start_column, line.end_column
        ),
        None => format!(
            "line {} character {} (no visible row: the line is folded or past the visible range)",
            position.line, position.column
        ),
    }
}

/// A visual row and a character column within it, as a document position and a byte offset.
fn visual_at(
    adapter: &crate::app::editor::adapter::Adapter,
    text: &str,
    row: usize,
    column: usize,
) -> Option<(TextPosition, usize)> {
    let lines = adapter.visual_lines();
    let line = lines.get(row)?;
    let character = line
        .start_column
        .saturating_add(column)
        .min(line.end_column);
    let position = TextPosition::new(line.line, character);
    Some((position, offset_of(text, position)))
}

/// The conversion's own report.
pub(super) fn coordinates_report(
    adapter: &crate::app::editor::adapter::Adapter,
    reason: &str,
) -> bool {
    if !reason.is_empty() {
        println!("rustage: {reason}");
    }
    println!(
        "lines: {}, visual rows: {}",
        adapter.line_count(),
        adapter.visual_row_count()
    );
    let (first, last) = adapter.visible_rows();
    println!("visible rows: {first}..{last}");
    for region in adapter.fold_regions() {
        println!(
            "fold {}..{} {}",
            region.start_line,
            region.end_line,
            if region.folded { "folded" } else { "open" }
        );
    }
    reason.is_empty()
}

/// Runs multi-cursor editing: carets, insertions, line operations, comments and brackets (task
/// T06.04).
///
/// Every operation is the library widget's own: [`Adapter`](crate::app::editor::adapter::Adapter)
/// forwards to `CodeEditor`, so a multi-caret `insert` is one widget transaction and a single `undo`
/// takes the whole keystroke back. The command line speaks byte offsets; [`position_at`] is the one
/// conversion into the widget's line/column positions, and it refuses an offset past the end or
/// inside a character.
pub(super) fn edit_ops(options: &crate::cli::EditOpsOptions) -> bool {
    use crate::app::editor::adapter::Adapter;
    use crate::app::editor::highlight::Language;

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
    let mut adapter = Adapter::new();
    adapter.set_text(&text);
    if let Some(file) = &options.file {
        adapter.set_language(Language::of_path(&file.to_string_lossy()).id());
    }

    for op in actions {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<String, String> = match verb {
            "carets" => match positions_of(&text, argument) {
                Ok(positions) => match positions.split_first() {
                    None => Err("carets needs at least one offset".to_owned()),
                    Some((first, rest)) => {
                        adapter.set_caret(*first);
                        for position in rest {
                            adapter.add_caret(*position);
                        }
                        Ok(format!("carets: {}", adapter.caret_count()))
                    }
                },
                Err(reason) => Err(reason),
            },
            "select" => match argument.split_once(':') {
                Some((anchor, cursor)) => {
                    match (anchor.parse::<usize>(), cursor.parse::<usize>()) {
                        (Ok(anchor), Ok(cursor)) => {
                            match (position_at(&text, anchor), position_at(&text, cursor)) {
                                (Ok(anchor_at), Ok(cursor_at)) => {
                                    adapter.select(anchor_at, cursor_at);
                                    Ok(format!(
                                        "selection {}..{}",
                                        anchor.min(cursor),
                                        anchor.max(cursor)
                                    ))
                                }
                                (Err(reason), _) | (_, Err(reason)) => Err(reason),
                            }
                        }
                        _ => Err(format!("{argument:?} is not <anchor>:<cursor>")),
                    }
                }
                None => Err(format!("{argument:?} is not <anchor>:<cursor>")),
            },
            "insert" => {
                adapter.insert(argument);
                Ok(format!("inserted {argument:?}"))
            }
            "type" => match single_char(argument) {
                Some(character) => {
                    adapter.type_text(&character.to_string());
                    Ok(format!("typed {character:?}"))
                }
                None => Err(format!("{argument:?} is not one character")),
            },
            "backspace" => {
                adapter.backspace();
                Ok("backspaced".to_owned())
            }
            "delete" => {
                adapter.delete_forward();
                Ok("deleted forward".to_owned())
            }
            "up" | "down" => {
                let direction = if verb == "up" { -1 } else { 1 };
                adapter.move_line(direction);
                Ok(format!("moved {verb}"))
            }
            "duplicate" => {
                adapter.duplicate_line();
                Ok("duplicated".to_owned())
            }
            "delete-lines" => {
                adapter.delete_line();
                Ok("deleted lines".to_owned())
            }
            "comment" => {
                if let Some(language) = comment_language(argument) {
                    adapter.set_language(language);
                }
                adapter.toggle_comment();
                Ok(format!("toggled comment {argument:?}"))
            }
            "matching" => match argument.parse::<usize>() {
                Ok(offset) => match position_at(&text, offset) {
                    Ok(position) => {
                        adapter.set_caret(position);
                        match adapter.matching_brackets() {
                            Some((open, close)) => {
                                let other = if position == open { close } else { open };
                                Ok(format!(
                                    "bracket at {offset} matches {}",
                                    offset_of(&text, other)
                                ))
                            }
                            None => Ok(format!("bracket at {offset} has no match")),
                        }
                    }
                    Err(reason) => Err(reason),
                },
                Err(_) => Err(format!("{argument:?} is not an offset")),
            },
            "undo" => Ok(if adapter.undo() {
                "undone".to_owned()
            } else {
                "nothing to undo".to_owned()
            }),
            "redo" => Ok(if adapter.redo() {
                "redone".to_owned()
            } else {
                "nothing to redo".to_owned()
            }),
            "show" => Ok(String::new()),
            other => Err(format!("{other:?} is not an editing operation")),
        };
        match step {
            Ok(note) if note.is_empty() => {}
            Ok(note) => println!("{note}"),
            Err(reason) => return edit_report(&adapter, &reason),
        }
    }
    edit_report(&adapter, "")
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

/// Parses `<offset>:<offset>:...` into widget positions.
fn positions_of(text: &str, argument: &str) -> Result<Vec<TextPosition>, String> {
    argument
        .split(':')
        .map(|number| match number.parse::<usize>() {
            Ok(offset) => position_at(text, offset),
            Err(_) => Err(format!("{number:?} is not an offset")),
        })
        .collect()
}

/// Converts a byte offset to the widget's line/column position.
fn position_at(text: &str, offset: usize) -> Result<TextPosition, String> {
    if offset > text.len() {
        return Err(format!(
            "offset {offset} is past the end of the document ({} bytes)",
            text.len()
        ));
    }
    if !text.is_char_boundary(offset) {
        return Err(format!("offset {offset} is inside a character"));
    }
    let before = &text[..offset];
    let line = before.matches('\n').count();
    let column = before.rsplit('\n').next().unwrap_or("").chars().count();
    Ok(TextPosition::new(line, column))
}

/// Converts a widget position back to a byte offset, for a report.
fn offset_of(text: &str, position: TextPosition) -> usize {
    let mut offset = 0;
    for (index, line) in text.split('\n').enumerate() {
        if index == position.line {
            return offset
                + line
                    .char_indices()
                    .nth(position.column)
                    .map(|(at, _)| at)
                    .unwrap_or(line.len());
        }
        offset += line.len() + 1;
    }
    text.len()
}

/// The language a command-line comment prefix names, when it names one.
fn comment_language(prefix: &str) -> Option<LanguageId> {
    match prefix {
        "//" => Some(LanguageId::Rust),
        "#" => Some(LanguageId::Python),
        "--" => Some(LanguageId::Sql),
        "<!--" => Some(LanguageId::Html),
        _ => None,
    }
}

/// The editor's own report.
pub(super) fn edit_report(adapter: &crate::app::editor::adapter::Adapter, reason: &str) -> bool {
    if !reason.is_empty() {
        println!("rustage: {reason}");
    }
    println!("lines: {}", adapter.line_count());
    for caret in adapter.carets() {
        println!("caret {}:{}", caret.line, caret.column);
    }
    if let Some((start, end)) = adapter.selection() {
        println!(
            "selection {}:{}..{}:{}",
            start.line, start.column, end.line, end.column
        );
    }
    println!("revision {}", adapter.revision());
    println!("text: {:?}", adapter.text());
    reason.is_empty()
}

/// Reads a file's encoding and shows the widget's folded/visible state (task T06.05).
///
/// The document, its folds and its rows are the widget's; the host decides only how a line is
/// printed (gutter width, whitespace glyphs) and keeps the file's encoding policy. The minimap is
/// drawn by the widget, so the report names the viewport it covers instead of computing a second
/// set of bands.
pub(super) fn editor_view(options: &crate::cli::EditorViewOptions) -> bool {
    use crate::app::editor::adapter::Adapter;
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

    let tab_width = options.tab.unwrap_or(4) as usize;
    let mut adapter = match Adapter::with_options(&file.text(), false, tab_width) {
        Ok(adapter) => adapter,
        Err(reason) => {
            println!("rustage: the editor refused its configuration: {reason}");
            return false;
        }
    };
    let mut whitespace = false;
    let mut relative = false;

    for op in actions {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "whitespace" => {
                whitespace = true;
                Ok(vec!["whitespace: shown".to_owned()])
            }
            "numbers" => {
                relative = argument == "relative";
                Ok(vec![format!(
                    "numbers: {}",
                    if relative { "relative" } else { "absolute" }
                )])
            }
            "window" => match window_of(argument) {
                Ok((first, count, current)) => Ok(view_lines(
                    &adapter, first, count, current, whitespace, relative, tab_width,
                )),
                Err(reason) => Err(reason),
            },
            "fold" => {
                let folded = adapter.fold_all();
                let mut out = vec![format!("folded {folded} region(s)")];
                out.extend(adapter.fold_regions().iter().map(|region| {
                    format!(
                        "fold {}..{} {}",
                        region.start_line,
                        region.end_line,
                        if region.folded { "folded" } else { "open" }
                    )
                }));
                Ok(out)
            }
            "minimap" => match argument.parse::<usize>() {
                Ok(height) => {
                    let (first, last) = adapter.visible_rows();
                    Ok(vec![format!(
                        "minimap {height}: drawn by the widget; {} visual row(s), viewport \
                         {first}..{last}",
                        adapter.visual_row_count()
                    )])
                }
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

/// Prints a window of the widget's lines with the gutter and whitespace the caller asked for.
fn view_lines(
    adapter: &crate::app::editor::adapter::Adapter,
    first: usize,
    count: usize,
    current: Option<usize>,
    whitespace: bool,
    relative: bool,
    tab_width: usize,
) -> Vec<String> {
    let total = adapter.line_count();
    let width = total.to_string().len();
    let mut out = Vec::new();
    for index in first..(first.saturating_add(count)).min(total) {
        let Some(text) = adapter.line_text(index) else {
            continue;
        };
        let shown = if whitespace {
            show_whitespace(&text, tab_width)
        } else {
            text
        };
        let number = if relative && Some(index) != current {
            index.abs_diff(current.unwrap_or(index))
        } else {
            index
        };
        out.push(format!("{number:>width$} {shown}"));
    }
    out
}

/// One line with a space as `·` and a tab as `→` padded to the next stop.
fn show_whitespace(text: &str, tab_width: usize) -> String {
    let mut out = String::new();
    let mut column = 0;
    for character in text.chars() {
        match character {
            ' ' => {
                out.push('·');
                column += 1;
            }
            '\t' => {
                let stop = tab_width.max(1) - (column % tab_width.max(1));
                out.push('→');
                for _ in 1..stop {
                    out.push(' ');
                }
                column += stop;
            }
            other => {
                out.push(other);
                column += 1;
            }
        }
    }
    out
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
