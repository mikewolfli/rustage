// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Diagnostics, semantic tokens and code actions with a fix preview (task T07.05).
//!
//! The acceptance line is *"the fix does not cross a generated region, and diagnostic sources are
//! distinguishable"*. Both are properties of what the module is willing to hold, so it holds them:
//!
//! * **A diagnostic names its source.** [`Source`] is the compiler, the linter or the language
//!   server, and [`Diagnostics::by_source`]/[`Diagnostics::sources`] keep them apart, so "the compiler
//!   said" and "clippy said" are not one undifferentiated list.
//! * **A fix is a write, and writes are guarded.** [`apply_action`] offers the action's edits to the
//!   [T06.06](crate::project::protection) guard through [`WriteSource::Format`], so a fix that would
//!   change a region a generator owns is refused **whole** rather than half-applied.
//! * **A diagnostic on generated code says so.** [`mark_generated`] flags every diagnostic whose span
//!   falls inside a protected region, so a report can distinguish it from a person's own code.
//! * **Freshness is the one rule of [T07.03](crate::app::features).** This module reuses
//!   [`Pending`], so a diagnostic set or a token layer for an old version is
//!   [`Delivery::Stale`] exactly as a completion is; there is no second freshness rule.
//!
//! **Boundary.** [`TokenKind`] is the *service's* vocabulary — richer than the built-in lexer's
//! [`Role`](crate::app::highlight::Role); mapping these onto theme colours is a rendering concern and
//! is not done here, so the two are deliberately not the same type yet.

#![allow(dead_code)]

use std::collections::BTreeSet;

use crate::app::features::{Delivery, Pending, Request, Response};
use crate::app::navigation::Span;
use crate::project::protection::{ProtectedRegion, Write, WriteGuard, WriteRefusal, WriteSource};

/// Who reported a diagnostic.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Source {
    /// The compiler.
    Compiler,
    /// The linter.
    Clippy,
    /// The language server's own analysis.
    LanguageServer,
}

impl Source {
    /// Every source, in a stable order.
    pub const ALL: [Self; 3] = [Self::Compiler, Self::Clippy, Self::LanguageServer];

    /// The source's name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Compiler => "compiler",
            Self::Clippy => "clippy",
            Self::LanguageServer => "language-server",
        }
    }

    /// The source a name picks.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|source| source.name() == name)
    }
}

/// How serious a diagnostic is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Severity {
    /// An error.
    Error,
    /// A warning.
    Warning,
    /// Information.
    Information,
    /// A hint.
    Hint,
}

impl Severity {
    /// Every severity, in a stable order.
    pub const ALL: [Self; 4] = [Self::Error, Self::Warning, Self::Information, Self::Hint];

    /// The severity's name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Information => "information",
            Self::Hint => "hint",
        }
    }

    /// The severity a name picks.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|severity| severity.name() == name)
    }
}

/// One diagnostic.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Diagnostic {
    /// Where it is.
    pub span: Span,
    /// How serious it is.
    pub severity: Severity,
    /// Who reported it.
    pub source: Source,
    /// The rule or error code, when there is one.
    pub code: Option<String>,
    /// The message.
    pub message: String,
    /// Whether it falls on a region a generator owns.
    pub generated: bool,
}

impl Diagnostic {
    /// A diagnostic.
    pub fn new(span: Span, severity: Severity, source: Source, message: impl Into<String>) -> Self {
        Self {
            span,
            severity,
            source,
            code: None,
            message: message.into(),
            generated: false,
        }
    }

    /// The same diagnostic with a code.
    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }

    /// The diagnostic in one line.
    pub fn render(&self) -> String {
        let code = self
            .code
            .as_ref()
            .map(|code| format!(" [{code}]"))
            .unwrap_or_default();
        let generated = if self.generated { " (generated)" } else { "" };
        format!(
            "{}:{} {} {}{code}{generated}: {}",
            self.span.start,
            self.span.end,
            self.source.name(),
            self.severity.name(),
            self.message
        )
    }
}

/// A set of diagnostics the service computed for one version.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct DiagnosticSet {
    /// The diagnostics.
    pub items: Vec<Diagnostic>,
}

impl DiagnosticSet {
    /// A set.
    pub fn new(items: Vec<Diagnostic>) -> Self {
        Self { items }
    }

    /// How many there are.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether it is empty.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Flags every diagnostic whose span is inside a protected region.
    pub fn mark_generated(&mut self, regions: &[ProtectedRegion]) {
        for item in &mut self.items {
            item.generated = regions
                .iter()
                .any(|region| item.span.start < region.end() && item.span.end > region.start());
        }
    }

    /// The diagnostics on generated code.
    pub fn generated(&self) -> Vec<&Diagnostic> {
        self.items.iter().filter(|item| item.generated).collect()
    }

    /// The diagnostics on a person's own code.
    pub fn handwritten(&self) -> Vec<&Diagnostic> {
        self.items.iter().filter(|item| !item.generated).collect()
    }
}

/// The diagnostics shown, kept fresh by the one request rule.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Diagnostics {
    pending: Pending,
    version: u64,
    shown: Option<DiagnosticSet>,
}

impl Diagnostics {
    /// A set over a document at a version.
    pub fn new(version: u64) -> Self {
        Self {
            pending: Pending::new(),
            version,
            shown: None,
        }
    }

    /// Records that the document moved to a new version.
    pub fn advance(&mut self, version: u64) {
        self.version = version;
    }

    /// Asks for diagnostics against the current version.
    pub fn request(&mut self) -> Request {
        self.pending.request(self.version)
    }

    /// Asks for diagnostics against a named version (a delayed request).
    pub fn request_at(&mut self, version: u64) -> Request {
        self.pending.request(version)
    }

    /// Takes a diagnostic reply. Only a fresh one becomes the shown set.
    pub fn accept(&mut self, response: Response<DiagnosticSet>) -> Delivery<usize> {
        let current = self.version;
        match self.pending.deliver(response, current) {
            Delivery::Fresh(set) => {
                let count = set.len();
                self.shown = Some(set);
                Delivery::Fresh(count)
            }
            Delivery::Stale { computed, current } => Delivery::Stale { computed, current },
            Delivery::Cancelled => Delivery::Cancelled,
        }
    }

    /// The shown set, when there is one.
    pub fn shown(&self) -> Option<&DiagnosticSet> {
        self.shown.as_ref()
    }

    /// The sources that reported at least one shown diagnostic.
    pub fn sources(&self) -> BTreeSet<Source> {
        match &self.shown {
            Some(set) => set.items.iter().map(|item| item.source).collect(),
            None => BTreeSet::new(),
        }
    }

    /// The shown diagnostics from one source.
    pub fn by_source(&self, source: Source) -> Vec<&Diagnostic> {
        match &self.shown {
            Some(set) => set
                .items
                .iter()
                .filter(|item| item.source == source)
                .collect(),
            None => Vec::new(),
        }
    }

    /// The shown set in one report.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!("diagnostics at version {}", self.version)];
        match &self.shown {
            Some(set) => {
                lines.push(format!("  shown: {} diagnostic(s)", set.len()));
                for item in &set.items {
                    lines.push(format!("    {}", item.render()));
                }
            }
            None => lines.push("  shown: none".to_owned()),
        }
        lines
    }
}

/// The kind of a semantic token.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TokenKind {
    /// A keyword.
    Keyword,
    /// A type or trait.
    Type,
    /// A function or method.
    Function,
    /// A variable or parameter.
    Variable,
    /// A string literal.
    String,
    /// A number literal.
    Number,
    /// A comment.
    Comment,
    /// An operator.
    Operator,
}

impl TokenKind {
    /// Every kind, in a stable order.
    pub const ALL: [Self; 8] = [
        Self::Keyword,
        Self::Type,
        Self::Function,
        Self::Variable,
        Self::String,
        Self::Number,
        Self::Comment,
        Self::Operator,
    ];

    /// The kind's name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Keyword => "keyword",
            Self::Type => "type",
            Self::Function => "function",
            Self::Variable => "variable",
            Self::String => "string",
            Self::Number => "number",
            Self::Comment => "comment",
            Self::Operator => "operator",
        }
    }

    /// The kind a name picks.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|kind| kind.name() == name)
    }
}

/// One semantic token.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SemanticToken {
    /// Where it is.
    pub span: Span,
    /// What it is.
    pub kind: TokenKind,
}

/// A semantic-token layer the service computed for one version.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct SemanticTokens {
    /// The tokens.
    pub tokens: Vec<SemanticToken>,
}

impl SemanticTokens {
    /// A layer.
    pub fn new(tokens: Vec<SemanticToken>) -> Self {
        Self { tokens }
    }

    /// How many tokens there are.
    pub fn len(&self) -> usize {
        self.tokens.len()
    }

    /// Whether it is empty.
    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }
}

/// The token layer shown, kept fresh by the same rule.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Tokens {
    pending: Pending,
    version: u64,
    shown: Option<SemanticTokens>,
}

impl Tokens {
    /// A layer view over a document at a version.
    pub fn new(version: u64) -> Self {
        Self {
            pending: Pending::new(),
            version,
            shown: None,
        }
    }

    /// Records that the document moved to a new version.
    pub fn advance(&mut self, version: u64) {
        self.version = version;
    }

    /// Asks for tokens against the current version.
    pub fn request(&mut self) -> Request {
        self.pending.request(self.version)
    }

    /// Asks for tokens against a named version (a delayed request).
    pub fn request_at(&mut self, version: u64) -> Request {
        self.pending.request(version)
    }

    /// Takes a token reply; only a fresh one becomes the shown layer.
    pub fn accept(&mut self, response: Response<SemanticTokens>) -> Delivery<usize> {
        let current = self.version;
        match self.pending.deliver(response, current) {
            Delivery::Fresh(tokens) => {
                let count = tokens.len();
                self.shown = Some(tokens);
                Delivery::Fresh(count)
            }
            Delivery::Stale { computed, current } => Delivery::Stale { computed, current },
            Delivery::Cancelled => Delivery::Cancelled,
        }
    }

    /// The shown layer, when there is one.
    pub fn shown(&self) -> Option<&SemanticTokens> {
        self.shown.as_ref()
    }
}

/// What a code action does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ActionKind {
    /// A quick fix.
    QuickFix,
    /// A refactor.
    Refactor,
}

impl ActionKind {
    /// The kind's name.
    pub fn name(self) -> &'static str {
        match self {
            Self::QuickFix => "quick-fix",
            Self::Refactor => "refactor",
        }
    }

    /// The kind a name picks.
    pub fn from_name(name: &str) -> Option<Self> {
        [Self::QuickFix, Self::Refactor]
            .into_iter()
            .find(|kind| kind.name() == name)
    }
}

/// One edit of an action.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TextEdit {
    /// Where it starts.
    pub at: usize,
    /// How many bytes it removes.
    pub remove: usize,
    /// What it puts there.
    pub insert: String,
}

impl TextEdit {
    /// A text edit.
    pub fn new(at: usize, remove: usize, insert: impl Into<String>) -> Self {
        Self {
            at,
            remove,
            insert: insert.into(),
        }
    }

    /// The edit as a write, for the guard.
    pub fn write(&self) -> Write {
        Write::replace(self.at, self.remove, self.insert.len())
    }

    /// The edit in one line.
    pub fn render(&self) -> String {
        format!("at {}: {:?}", self.at, self.insert)
    }
}

/// A code action: a titled set of edits.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CodeAction {
    /// What a person sees.
    pub title: String,
    /// What kind of action it is.
    pub kind: ActionKind,
    /// The edits it makes.
    pub edits: Vec<TextEdit>,
}

impl CodeAction {
    /// An action.
    pub fn new(title: impl Into<String>, kind: ActionKind, edits: Vec<TextEdit>) -> Self {
        Self {
            title: title.into(),
            kind,
            edits,
        }
    }

    /// The action in one line.
    pub fn render(&self) -> String {
        format!(
            "{} [{}] ({} edit(s))",
            self.title,
            self.kind.name(),
            self.edits.len()
        )
    }
}

/// Offers a code action's edits to the guard. A refusal refuses the whole action.
///
/// Returns the edits for the caller to apply as one transaction, so the preview and the application
/// are the same list.
pub fn apply_action(
    action: &CodeAction,
    guard: &WriteGuard,
) -> Result<Vec<(usize, usize, String)>, WriteRefusal> {
    let writes: Vec<Write> = action.edits.iter().map(TextEdit::write).collect();
    guard.check(WriteSource::Format, &writes)?;
    Ok(action
        .edits
        .iter()
        .map(|edit| (edit.at, edit.remove, edit.insert.clone()))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::{
        ActionKind, CodeAction, Diagnostic, DiagnosticSet, Diagnostics, SemanticToken,
        SemanticTokens, Severity, Source, TextEdit, TokenKind, Tokens, apply_action,
    };
    use crate::app::features::{Delivery, Response};
    use crate::app::navigation::Span;
    use crate::project::conflict::Writability;
    use crate::project::protection::{ProtectedRegion, WriteGuard, WriteRefusal};

    fn set(items: Vec<Diagnostic>) -> DiagnosticSet {
        DiagnosticSet::new(items)
    }

    #[test]
    fn a_diagnostic_set_for_an_old_version_is_stale() {
        let mut diagnostics = Diagnostics::new(1);
        let request = diagnostics.request();
        diagnostics.advance(2);
        let items = set(vec![Diagnostic::new(
            Span::new(0, 3),
            Severity::Error,
            Source::Compiler,
            "old",
        )]);
        assert_eq!(
            diagnostics.accept(Response::new(request, items)),
            Delivery::Stale {
                computed: 1,
                current: 2
            }
        );
        assert!(diagnostics.shown().is_none());
    }

    #[test]
    fn diagnostic_sources_are_distinguishable() {
        let mut diagnostics = Diagnostics::new(1);
        let request = diagnostics.request();
        let items = set(vec![
            Diagnostic::new(Span::new(0, 3), Severity::Error, Source::Compiler, "E0308")
                .with_code("E0308"),
            Diagnostic::new(
                Span::new(5, 8),
                Severity::Warning,
                Source::Clippy,
                "needless",
            ),
            Diagnostic::new(
                Span::new(5, 8),
                Severity::Information,
                Source::LanguageServer,
                "note",
            ),
        ]);
        diagnostics.accept(Response::new(request, items));
        // Three sources, and each one's list is its own.
        assert_eq!(diagnostics.sources().len(), 3);
        assert_eq!(diagnostics.by_source(Source::Compiler).len(), 1);
        assert_eq!(diagnostics.by_source(Source::Clippy).len(), 1);
        assert_eq!(
            diagnostics.by_source(Source::Compiler)[0].code.as_deref(),
            Some("E0308")
        );
    }

    #[test]
    fn a_diagnostic_on_generated_code_says_so() {
        let mut set = set(vec![
            Diagnostic::new(
                Span::new(2, 5),
                Severity::Warning,
                Source::Compiler,
                "in gen",
            ),
            Diagnostic::new(
                Span::new(20, 25),
                Severity::Warning,
                Source::Compiler,
                "mine",
            ),
        ]);
        set.mark_generated(&[ProtectedRegion::new(0, 10)]);
        assert_eq!(set.generated().len(), 1);
        assert_eq!(set.handwritten().len(), 1);
        assert!(set.generated()[0].render().contains("(generated)"));
    }

    #[test]
    fn a_fix_that_crosses_a_generated_region_is_refused_whole() {
        let guard = WriteGuard::new(vec![ProtectedRegion::new(4, 10)], Writability::Writable);
        // A fix that replaces across the region's edge is refused.
        let across = CodeAction::new("fix", ActionKind::QuickFix, vec![TextEdit::new(3, 4, "x")]);
        match apply_action(&across, &guard) {
            Err(WriteRefusal::Protected { .. }) => {}
            other => panic!("expected a protected refusal, got {other:?}"),
        }
        // A fix entirely outside is allowed, and returns the edits to apply.
        let outside = CodeAction::new("fix", ActionKind::QuickFix, vec![TextEdit::new(20, 2, "y")]);
        let edits = apply_action(&outside, &guard).expect("allowed");
        assert_eq!(edits, vec![(20, 2, "y".to_owned())]);
    }

    #[test]
    fn a_token_layer_for_an_old_version_is_stale() {
        let mut tokens = Tokens::new(1);
        let request = tokens.request();
        tokens.advance(2);
        let layer = SemanticTokens::new(vec![SemanticToken {
            span: Span::new(0, 2),
            kind: TokenKind::Keyword,
        }]);
        assert_eq!(
            tokens.accept(Response::new(request, layer)),
            Delivery::Stale {
                computed: 1,
                current: 2
            }
        );
        assert!(tokens.shown().is_none());
        // A fresh layer is shown.
        let request = tokens.request();
        let layer = SemanticTokens::new(vec![
            SemanticToken {
                span: Span::new(0, 2),
                kind: TokenKind::Keyword,
            },
            SemanticToken {
                span: Span::new(3, 6),
                kind: TokenKind::Function,
            },
        ]);
        assert_eq!(
            tokens.accept(Response::new(request, layer)),
            Delivery::Fresh(2)
        );
        assert_eq!(tokens.shown().expect("shown").len(), 2);
    }

    #[test]
    fn names_round_trip() {
        for source in Source::ALL {
            assert_eq!(Source::from_name(source.name()), Some(source));
        }
        for severity in Severity::ALL {
            assert_eq!(Severity::from_name(severity.name()), Some(severity));
        }
        for kind in TokenKind::ALL {
            assert_eq!(TokenKind::from_name(kind.name()), Some(kind));
        }
        for kind in [ActionKind::QuickFix, ActionKind::Refactor] {
            assert_eq!(ActionKind::from_name(kind.name()), Some(kind));
        }
        assert_eq!(Source::from_name("nowhere"), None);
    }
}
