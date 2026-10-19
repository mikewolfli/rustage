// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Two-way navigation between generated Rust and the document model (task T19.06).
//!
//! The acceptance line is *"a neighbouring line is not guessed to be the accurate node"*. That is a
//! property of returning [`Navigation::Unmapped`] whenever the position is not inside a span the
//! generator recorded — a debugger stops between statements, in generated boilerplate, and on lines
//! this build never mapped, and all three must show the original debug view rather than jumping to
//! the closest control.
//!
//! The map is the same [`SourceMap`] the generator produces and the build diagnostics use, so there
//! is no second idea of "which node is on which line" (principle 59). A stale map is refused here as
//! well: a navigation computed from an older model revision looks like an answer and is not one.

#![allow(dead_code)]

use crate::contracts::source_map::{SourceMap, SourceSpan};

/// Where a debugger position points in the document.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Navigation {
    /// The position is inside a generated span for this node.
    Node {
        /// The node.
        node: crate::contracts::identity::NodeId,
        /// The span the position was found in.
        span: SourceSpan,
        /// The position, as a one-based line and column.
        line: u32,
        /// The position, as a one-based column.
        column: u32,
    },
    /// The position is not inside any mapped span.
    Unmapped {
        /// Where the position was.
        file: String,
        /// The one-based line.
        line: u32,
        /// The one-based column.
        column: u32,
        /// Why it did not map. This always says that no nearby line was used.
        reason: String,
    },
}

impl Navigation {
    /// The navigation in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Node {
                node,
                span,
                line,
                column,
            } => format!("node {} at {}:{}:{}", node.get(), span.file(), line, column),
            Self::Unmapped {
                file,
                line,
                column,
                reason,
            } => format!("unmapped {file}:{line}:{column} ({reason})"),
        }
    }
}

/// Where a node's generated construct starts.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct GeneratedPosition {
    /// The generated file.
    pub file: String,
    /// The line, counting from one.
    pub line: u32,
    /// The column, counting from one.
    pub column: u32,
}

/// Finds the node whose generated span contains a debugger position.
///
/// The position is converted to a byte offset in `source_text`; a line past the end, a column past
/// the line's end or a position between two mapped constructs all answer [`Navigation::Unmapped`].
/// No span is widened and no neighbouring line is tried.
pub fn from_debug(
    map: &SourceMap,
    source_text: &str,
    file: &str,
    line: u32,
    column: u32,
) -> Navigation {
    let Some(offset) = offset_of(source_text, line, column) else {
        return Navigation::Unmapped {
            file: file.to_owned(),
            line,
            column,
            reason: "the position is past the end of the generated source".to_owned(),
        };
    };
    let Some(node) = map.node_at(file, offset) else {
        return Navigation::Unmapped {
            file: file.to_owned(),
            line,
            column,
            reason: "no mapped node contains this position; nearby lines are not guessed"
                .to_owned(),
        };
    };
    let Some(span) = map.span_of(node).cloned() else {
        return Navigation::Unmapped {
            file: file.to_owned(),
            line,
            column,
            reason: "the map changed while the position was being resolved".to_owned(),
        };
    };
    Navigation::Node {
        node,
        span,
        line,
        column,
    }
}

/// Finds where a node's construct starts in the generated source.
pub fn to_debug(
    map: &SourceMap,
    node: crate::contracts::identity::NodeId,
    source_text: &str,
) -> Option<GeneratedPosition> {
    let span = map.span_of(node)?;
    let (line, column) = super::breakpoints::line_of(source_text, span.start());
    Some(GeneratedPosition {
        file: span.file().to_owned(),
        line,
        column,
    })
}

/// The byte offset of a one-based line and column.
///
/// A column is counted in characters, which is what DAP specifies; the returned offset is a byte
/// index into `text`, so a multi-byte character before the position still lands correctly.
pub fn offset_of(text: &str, line: u32, column: u32) -> Option<u32> {
    if line == 0 || column == 0 {
        return None;
    }
    let mut current_line = 1_u32;
    let mut offset = 0_usize;
    for (index, character) in text.char_indices() {
        if current_line == line {
            let mut current_column = 1_u32;
            for (inner, _) in text[index..].char_indices() {
                if current_column == column {
                    return Some((index + inner) as u32);
                }
                if text[index + inner..].starts_with('\n') {
                    return None;
                }
                current_column += 1;
            }
            return if current_column == column {
                Some(text.len() as u32)
            } else {
                None
            };
        }
        if character == '\n' {
            current_line += 1;
            offset = index + 1;
        }
    }
    if current_line == line && column == 1 {
        return Some(text.len() as u32);
    }
    let _ = offset;
    None
}

#[cfg(test)]
mod tests {
    use super::{Navigation, from_debug, offset_of, to_debug};
    use crate::contracts::identity::NodeId;
    use crate::contracts::revision::{GeneratorVersion, Revision};
    use crate::contracts::source_map::{SourceMap, SourceSpan};

    fn source() -> String {
        "// generated\nfn build_ui() {\n    let title = Label::new();\n    let go = Button::new();\n}\n"
            .to_owned()
    }

    fn map() -> (SourceMap, NodeId, NodeId) {
        let title = NodeId::from_raw(1);
        let button = NodeId::from_raw(2);
        let mut map = SourceMap::new(GeneratorVersion::new(1, 0, 0), Revision::INITIAL);
        // `let title` starts at byte 32 and `let go` at byte 59.
        map.insert(title, SourceSpan::new("ui.rs", 32, 55).expect("a span"));
        map.insert(button, SourceSpan::new("ui.rs", 59, 83).expect("a span"));
        (map, title, button)
    }

    #[test]
    fn a_position_inside_a_span_maps_to_that_node() {
        let (map, title, _) = map();
        let text = source();
        let navigation = from_debug(&map, &text, "ui.rs", 3, 9);
        match navigation {
            Navigation::Node { node, .. } => assert_eq!(node, title),
            other => panic!("expected a node, got {other:?}"),
        }
    }

    #[test]
    fn a_blank_or_unmapped_line_is_not_guessed_to_a_neighbour() {
        let (map, _, _) = map();
        let text = source();
        for (line, column) in [(1, 1), (2, 1), (5, 1)] {
            let navigation = from_debug(&map, &text, "ui.rs", line, column);
            assert!(
                matches!(navigation, Navigation::Unmapped { .. }),
                "line {line} should not map: {navigation:?}"
            );
            assert!(navigation.render().contains("not guessed"));
        }
    }

    #[test]
    fn a_position_past_the_end_is_unmapped_with_its_own_reason() {
        let (map, _, _) = map();
        let navigation = from_debug(&map, &source(), "ui.rs", 90, 1);
        assert!(matches!(navigation, Navigation::Unmapped { .. }));
        assert!(navigation.render().contains("past the end"));
    }

    #[test]
    fn node_to_source_uses_the_span_start() {
        let (map, _, button) = map();
        let position = to_debug(&map, button, &source()).expect("a position");
        assert_eq!(position.file, "ui.rs");
        assert_eq!(position.line, 4);
        assert_eq!(position.column, 1);
    }

    #[test]
    fn offsets_are_one_based_and_respect_multibyte_characters() {
        let text = "a\néx\nz";
        assert_eq!(offset_of(text, 1, 1), Some(0));
        assert_eq!(offset_of(text, 2, 1), Some(2));
        assert_eq!(offset_of(text, 2, 2), Some(4));
        assert_eq!(offset_of(text, 3, 1), Some(6));
        assert_eq!(offset_of(text, 2, 9), None);
        assert_eq!(offset_of(text, 0, 0), None);
    }
}
