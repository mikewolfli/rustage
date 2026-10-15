// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Editing probes: the text surface the code view and the JSON/CSS editors stand on.
//!
//! T00.02 asks for five things to be settled **before** the designer depends on them: highlight
//! state, threading, caching, coordinates and editing primitives. Each is exercised here by calling
//! the real `CodeEditor` API, and the findings are split by layer, because the two halves have
//! different owners:
//!
//! | Concern | Library layer (`rust_widgets`) | Host layer (rustage) |
//! |---|---|---|
//! | Highlight state | token spans, line state, the pluggable [`SyntaxHighlighter`] seam | choosing a language per document and supplying a real grammar when the built-in lexer is not enough |
//! | Primitives | insert/delete/undo/redo, guarded `replace_range` | deciding *which* edits a generated region forbids and mapping them onto locked spans |
//! | Coordinates | point ⇄ text position, viewport snapshot | hit-testing with its own overlay geometry, DPI policy |
//! | Caching | `DocumentScale` graded degradation chosen when text is set | keeping per-frame work viewport-sized at each level |
//! | Threading | the editor is thread-affine (`Rc`/`RefCell`/`Cell`); the highlighter seam is `Send + Sync` | owning the editor on the UI thread and marshalling background analysis results back |
//!
//! The threading row is the one finding that is a *boundary* rather than a capability: the editor
//! cannot cross threads, so the host must not try. The seam that *can* cross threads is the
//! highlighter, and probe 5 proves that by compiling a real cross-thread move.

use rust_widgets::core::{Point, Rect};
use rust_widgets::widget::special_widgets::code_editor::{
    BuiltinHighlighter, CodeEditor, DocumentScale, EditRejection, LanguageId, LineState,
    ReadOnlySpan, SCALE_REDUCED_LINES, SCALE_VIEW_ONLY_LINES, SyntaxHighlighter, TextPosition,
    TokenKind,
};

use super::{ApiFamily, ProbeResult, require};

/// The family these probes belong to.
const FAMILY: ApiFamily = ApiFamily::Editing;

/// The geometry every probe builds its editor with.
fn geometry() -> Rect {
    Rect::new(0, 0, 400, 300)
}

/// Runs every editing probe.
pub fn probes() -> Vec<ProbeResult> {
    vec![
        edit_primitives(),
        history_and_revision(),
        guarded_edits(),
        highlight_state(),
        multiline_highlight_state(),
        highlighter_crosses_threads(),
        graded_scale(),
        coordinates_and_viewport(),
    ]
}

/// `CodeEditor::insert` / `backspace` / `delete_forward`: the typing primitives.
fn edit_primitives() -> ProbeResult {
    const API: &str = "code_editor::CodeEditor::insert/backspace/delete_forward";
    let mut editor = CodeEditor::new(geometry());
    editor.set_text("ab");

    editor.set_caret(TextPosition::new(0, 1), false);
    editor.insert("X");
    require!(
        editor.text() == "aXb",
        FAMILY,
        API,
        format!("insert produced {:?}, expected \"aXb\"", editor.text())
    );

    editor.backspace();
    require!(
        editor.text() == "ab",
        FAMILY,
        API,
        format!("backspace produced {:?}, expected \"ab\"", editor.text())
    );

    editor.set_caret(TextPosition::new(0, 1), false);
    editor.delete_forward();
    require!(
        editor.text() == "a",
        FAMILY,
        API,
        format!(
            "delete_forward produced {:?}, expected \"a\"",
            editor.text()
        )
    );

    ProbeResult::verified(
        FAMILY,
        API,
        vec![
            "insert at (0,1) -> \"aXb\"".to_string(),
            "backspace -> \"ab\"".to_string(),
            "delete_forward at (0,1) -> \"a\"".to_string(),
        ],
    )
}

/// `CodeEditor::undo` / `redo` / `revision`: history and the revision the edit guard uses.
fn history_and_revision() -> ProbeResult {
    const API: &str = "code_editor::CodeEditor::undo/redo/revision";
    let mut editor = CodeEditor::new(geometry());
    editor.set_text("ab");
    let before = editor.revision();

    editor.set_caret(TextPosition::new(0, 1), false);
    editor.insert("X");
    let after = editor.revision();
    require!(
        after != before,
        FAMILY,
        API,
        format!("revision did not move across an edit: {before} -> {after}")
    );

    require!(
        editor.can_undo(),
        FAMILY,
        API,
        "can_undo reported nothing to undo after an edit"
    );
    require!(
        editor.undo() && editor.text() == "ab",
        FAMILY,
        API,
        format!(
            "undo did not restore \"ab\"; the editor holds {:?}",
            editor.text()
        )
    );
    require!(
        editor.can_redo(),
        FAMILY,
        API,
        "can_redo reported nothing to redo after an undo"
    );
    require!(
        editor.redo() && editor.text() == "aXb",
        FAMILY,
        API,
        format!(
            "redo did not restore \"aXb\"; the editor holds {:?}",
            editor.text()
        )
    );

    ProbeResult::verified(
        FAMILY,
        API,
        vec![
            format!("revision moved {before} -> {after}"),
            "undo restored \"ab\", redo restored \"aXb\"".to_string(),
        ],
    )
}

/// `CodeEditor::replace_range` + `EditRejection`: a locked span and a stale revision are refused,
/// and a legal edit still lands.
///
/// This is the primitive behind principle #19: a generated region is read-only, and a request
/// computed against an older revision is not applied silently.
fn guarded_edits() -> ProbeResult {
    const API: &str = "code_editor::CodeEditor::replace_range";
    let mut editor = CodeEditor::new(geometry());
    editor.set_text("keep\nedit\nkeep\n");
    let revision = editor.revision();

    editor.add_read_only_span(ReadOnlySpan::lines(0, 0));
    require!(
        !editor.is_position_editable(TextPosition::new(0, 0)),
        FAMILY,
        API,
        "a position inside the locked line was reported editable"
    );
    require!(
        editor.is_position_editable(TextPosition::new(1, 0)),
        FAMILY,
        API,
        "an unlocked line was reported not editable"
    );

    let locked = editor.replace_range(
        revision,
        TextPosition::new(0, 0),
        TextPosition::new(0, 4),
        "XXXX",
    );
    require!(
        matches!(locked, Err(EditRejection::ReadOnly)),
        FAMILY,
        API,
        format!("editing the locked line returned {locked:?}, expected Err(ReadOnly)")
    );
    require!(
        editor.text() == "keep\nedit\nkeep\n",
        FAMILY,
        API,
        format!("a refused edit changed the text: {:?}", editor.text())
    );

    let stale = editor.replace_range(
        revision + 1000,
        TextPosition::new(1, 0),
        TextPosition::new(1, 4),
        "EDIT",
    );
    require!(
        matches!(stale, Err(EditRejection::StaleRevision { .. })),
        FAMILY,
        API,
        format!("a stale edit returned {stale:?}, expected Err(StaleRevision)")
    );

    let applied = editor.replace_range(
        revision,
        TextPosition::new(1, 0),
        TextPosition::new(1, 4),
        "EDIT",
    );
    require!(
        applied.is_ok(),
        FAMILY,
        API,
        format!("a legal edit was refused: {applied:?}")
    );
    require!(
        editor.text() == "keep\nEDIT\nkeep\n",
        FAMILY,
        API,
        format!("the legal edit produced {:?}", editor.text())
    );

    ProbeResult::verified(
        FAMILY,
        API,
        vec![
            "locked line -> Err(ReadOnly), text unchanged".to_string(),
            "stale revision -> Err(StaleRevision), nothing written".to_string(),
            "current revision on an unlocked line -> applied".to_string(),
        ],
    )
}

/// `CodeEditor::tokens_on_line` / `line_state` / `set_language`: the highlight state of lines
/// whose entry state is `Clean`, which is what a lexer can answer without walking the document.
fn highlight_state() -> ProbeResult {
    const API: &str = "code_editor::CodeEditor::tokens_on_line/line_state";
    let mut editor = CodeEditor::new(geometry());
    editor.set_language(LanguageId::Rust);
    editor.set_text("fn main() {\n    // note\n}\n");

    let tokens = editor.tokens_on_line(0);
    let keyword = tokens.iter().any(|span| span.kind == TokenKind::Keyword);
    require!(
        keyword,
        FAMILY,
        API,
        format!("no keyword token on `fn main() {{`; got {tokens:?}")
    );
    let comment_kind = editor.token_kind_at(TextPosition::new(1, 6));
    require!(
        comment_kind == TokenKind::Comment,
        FAMILY,
        API,
        format!("a position inside a line comment reported {comment_kind:?}, expected Comment")
    );
    require!(
        editor.line_state(0) == LineState::Clean,
        FAMILY,
        API,
        format!(
            "a line that starts outside any construct reported {:?}, expected Clean",
            editor.line_state(0)
        )
    );

    ProbeResult::verified(
        FAMILY,
        API,
        vec![
            format!("`fn` tokenized as {:?}", TokenKind::Keyword),
            "a position inside `// note` tokenized as Comment".to_string(),
            "a line that opens no construct is reported Clean".to_string(),
        ],
    )
}

/// The multi-line half of the highlight state: the distinction the public surface used to merge.
///
/// # What is observed
///
/// `CodeEditor` keeps a per-line entry-state cache and fills it from the **paint** path
/// (`prepare_visible_line_cache`). [`CodeEditor::line_state`] reads that cache, and a line the walk
/// has not reached reads back as [`LineState::Clean`] — the `Default` — which is the same value a
/// genuinely clean line gives.
///
/// The pinned revision restores the missing distinction without changing the value type:
/// [`CodeEditor::line_state_is_known`] reports whether the walk has **proven** the line, and
/// [`CodeEditor::resolve_line_state`] advances the chain so the answer is real (paying a walk
/// bounded by the distance from the last proven line, not by the document).
///
/// # Why this is verified rather than a gap
///
/// The two facts a host must tell apart — "this line starts outside any construct" and "this line
/// has not been lexed" — used to share one representation, the success-shaped default principle #22
/// forbids. With the predicate and the resolver both present and answering correctly, a host can ask
/// about any line, on screen or off, and get a dependable state; the designer's workaround (confine
/// highlight queries to lines the paint path covered) is no longer needed.
///
/// Filed as `feedback-20261013.md` §3 and closed by the revision this probe now runs against.
fn multiline_highlight_state() -> ProbeResult {
    const API: &str = "code_editor::CodeEditor::line_state/line_state_is_known/resolve_line_state";
    let mut editor = CodeEditor::new(geometry());
    editor.set_language(LanguageId::Rust);
    editor.set_text("/* open\nstill\n*/\nfn x() {}\n");

    // An un-walked line still reads back the `Default`, but the predicate now says so rather than
    // letting the two facts share one answer.
    let unproven = editor.line_state(1);
    let known_before = editor.line_state_is_known(1);
    require!(
        unproven == LineState::Clean && !known_before,
        FAMILY,
        API,
        format!(
            "line 1 (inside `/* … */`) has not been walked; expected the Clean default with an \
             unknown predicate, got state {unproven:?} with line_state_is_known = {known_before}"
        )
    );

    // The dependable answer is reachable without the paint path reaching the line first.
    let resolved = editor.resolve_line_state(1);
    require!(
        resolved == LineState::BlockComment && editor.line_state_is_known(1),
        FAMILY,
        API,
        format!(
            "resolving line 1 inside `/* … */` should report BlockComment and prove the state, got \
             {resolved:?} with line_state_is_known = {}",
            editor.line_state_is_known(1)
        )
    );

    ProbeResult::verified(
        FAMILY,
        API,
        vec![
            format!(
                "an un-walked line reads {unproven:?} with line_state_is_known = {known_before}"
            ),
            "resolve_line_state(1) proves the state and reports BlockComment".to_string(),
            "\"not computed\" and \"genuinely clean\" are now distinguishable".to_string(),
        ],
    )
}

/// `SyntaxHighlighter: Send + Sync`: the plugin seam, and the only part of the editor that may
/// cross a thread boundary.
///
/// # Why this is the threading probe
///
/// `CodeEditor` itself is thread-affine by construction — it stores its model in
/// `Rc<RefCell<..>>` and a measured width in a `Cell`, so it is neither `Send` nor `Sync`. A probe
/// cannot assert a *negative* `Send` bound at run time, and pretending to would be a claim without
/// evidence. What *can* be proved by compiling is the positive half the host actually needs: the
/// highlighter trait requires `Send + Sync`, so a highlighter — the part that does per-line work —
/// can be moved to a worker thread. The editor's own affinity is recorded in the notes as the
/// boundary the host must respect.
fn highlighter_crosses_threads() -> ProbeResult {
    const API: &str = "code_editor::SyntaxHighlighter (Send + Sync)";
    let highlighter: Box<dyn SyntaxHighlighter> =
        Box::new(BuiltinHighlighter::new(LanguageId::Rust));

    let worker = std::thread::spawn(move || {
        let (_, state) = highlighter.highlight_line("fn main() {", LineState::Clean);
        state
    });
    let state = match worker.join() {
        Ok(state) => state,
        Err(_) => {
            return ProbeResult::gap(FAMILY, API, "the highlighter thread panicked");
        }
    };

    ProbeResult::verified(
        FAMILY,
        API,
        vec![
            format!("a Box<dyn SyntaxHighlighter> was moved to a worker thread and returned {state:?}"),
            "CodeEditor itself is thread-affine (Rc<RefCell<..>> model, Cell-measured width): the host owns it on the UI thread".to_string(),
        ],
    )
}

/// `DocumentScale`: graded degradation chosen from the line count, and reported by the editor.
fn graded_scale() -> ProbeResult {
    const API: &str = "code_editor::DocumentScale";
    require!(
        DocumentScale::for_lines(10) == DocumentScale::Full,
        FAMILY,
        API,
        "a small document was not Full"
    );
    require!(
        DocumentScale::for_lines(SCALE_REDUCED_LINES) == DocumentScale::Reduced,
        FAMILY,
        API,
        format!("{SCALE_REDUCED_LINES} lines did not select Reduced")
    );
    require!(
        DocumentScale::for_lines(SCALE_VIEW_ONLY_LINES) == DocumentScale::ViewOnly,
        FAMILY,
        API,
        format!("{SCALE_VIEW_ONLY_LINES} lines did not select ViewOnly")
    );
    require!(
        !DocumentScale::ViewOnly.allows_editing() && DocumentScale::Full.allows_editing(),
        FAMILY,
        API,
        "the scale no longer says which levels accept edits"
    );

    // End to end: a real document large enough for Reduced is reported that way by the editor,
    // which is the level it cached when the text was set.
    let mut editor = CodeEditor::new(geometry());
    editor.set_text("line\n".repeat(SCALE_REDUCED_LINES));
    let scale = editor.document_scale();
    require!(
        scale == DocumentScale::Reduced,
        FAMILY,
        API,
        format!("a {SCALE_REDUCED_LINES}-line document reported {scale:?}, expected Reduced")
    );

    ProbeResult::verified(
        FAMILY,
        API,
        vec![
            format!(
                "{SCALE_REDUCED_LINES} lines -> Reduced, {SCALE_VIEW_ONLY_LINES} lines -> ViewOnly"
            ),
            "a real 50 000-line document reported Reduced".to_string(),
        ],
    )
}

/// `CodeEditor::viewport_snapshot` / `apply_viewport` / `visible_row_range` / `select_word_at_point`:
/// the coordinate surface the canvas hit-testing will use.
fn coordinates_and_viewport() -> ProbeResult {
    const API: &str = "code_editor::CodeEditor::viewport_snapshot/select_word_at_point";
    let mut editor = CodeEditor::new(geometry());
    // Every line is one long run of a single word character, so a click anywhere in the text
    // area lands on a word regardless of the cell width the backend measures or of the width the
    // gutter and status bar take.
    let mut text = String::new();
    for _ in 0..200 {
        text.push_str(&"a".repeat(120));
        text.push('\n');
    }
    editor.set_text(text);

    let (first, last) = editor.visible_row_range();
    require!(
        last >= first,
        FAMILY,
        API,
        format!("visible_row_range returned ({first}, {last})")
    );
    require!(
        editor.visible_rows() > 0,
        FAMILY,
        API,
        "no visible rows in a 300 px editor"
    );

    let snapshot = editor.viewport_snapshot();
    editor.scroll_by(20);
    require!(
        editor.viewport_snapshot() != snapshot,
        FAMILY,
        API,
        "scrolling did not move the viewport"
    );
    editor.apply_viewport(snapshot);
    require!(
        editor.viewport_snapshot() == snapshot,
        FAMILY,
        API,
        format!(
            "apply_viewport did not round-trip: {:?} vs {:?}",
            editor.viewport_snapshot(),
            snapshot
        )
    );

    // A point in the text area maps to a position and selects the word under it. Several rows are
    // tried because the chrome above the text area (tabs, gutter) is the editor's to lay out and
    // this probe must not encode its height.
    let mut selected = None;
    for y in [40, 60, 80, 120, 160, 200] {
        if editor.select_word_at_point(Point::new(100, y))
            && let Some(word) = editor.selected_text()
            && !word.is_empty()
            && word.chars().all(|c| c == 'a')
        {
            selected = Some(word.len());
            break;
        }
    }
    let Some(width) = selected else {
        return ProbeResult::gap(
            FAMILY,
            API,
            "no point in the text area mapped to a word: the point-to-position mapping was not \
             reachable from the public API",
        );
    };

    ProbeResult::verified(
        FAMILY,
        API,
        vec![
            format!(
                "visible_row_range -> ({first}, {last}), visible_rows -> {}",
                editor.visible_rows()
            ),
            "a viewport snapshot round-trips through apply_viewport".to_string(),
            format!("a point in the text area selected {width} characters of the word run"),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::probes;
    use crate::probe::ProbeOutcome;

    /// Every editing probe is verified against the pinned revision.
    ///
    /// This family used to carry one library-layer gap: `line_state`'s multi-line state merged "not
    /// lexed" with "genuinely clean". The revision this project now pins answers it
    /// (`line_state_is_known` + `resolve_line_state`), so the probe asserts the distinction instead
    /// of recording its absence. A regression that re-merged the two facts would fail here.
    #[test]
    fn every_editing_probe_is_verified() {
        for probe in probes() {
            assert!(
                matches!(probe.outcome, ProbeOutcome::Verified),
                "editing probe {} was {:?}",
                probe.api,
                probe.outcome
            );
        }
    }
}
