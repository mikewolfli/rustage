// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Definitions, references and symbols, with the outline and breadcrumbs they imply (task T07.04).
//!
//! The acceptance line is *"an unsupported capability is disabled, and results carry multiple target
//! locations"*. Two things must not be faked, so the module is built so neither can be:
//!
//! * **A query is answered only if the service negotiated its capability.** Every [`Query`] names the
//!   [`Capability`] it needs, and [`Navigation::query`] answers [`QueryRefusal::Unsupported`] when it
//!   was not negotiated — a disabled feature reads as disabled, not as an empty result that looks
//!   like "nothing found".
//! * **A result is a list of locations.** [`Location`] is a uri and a [`Span`], and the answer to
//!   definition or references is a `Vec<Location>`, so an overload or a reference in another file is
//!   naturally several targets rather than one.
//! * **The outline nests by span, and breadcrumbs are its path.** [`Navigation::outline`] builds the
//!   tree from the spans the service gave (a symbol strictly inside another is its child), and
//!   [`Navigation::breadcrumbs`] is the chain of symbols containing an offset. Neither invents a
//!   structure the service did not describe.
//!
//! Spans here are **byte offsets** in a document, the same unit the editor edits in
//! ([`editing`](crate::app::editor::editing)); the protocol positions a server speaks are
//! [T07.02](crate::app::language::protocol)'s conversion, so this module never mixes the two.
//!
//! The symbols are the **service's** answer; where the parser is (F07's later tasks and the
//! toolchain) is not this module's subject, which is why it takes them as data.

#![allow(dead_code)]

use crate::app::language::protocol::{Position, to_offset};
use crate::app::language::session::{Capabilities, Capability};
use crate::project::record::json::Json;

/// A byte range in a document.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Span {
    /// Where it starts.
    pub start: usize,
    /// Where it ends.
    pub end: usize,
}

impl Span {
    /// A span over `[start, end)`.
    pub fn new(start: usize, end: usize) -> Self {
        Self {
            start: start.min(end),
            end: start.max(end),
        }
    }

    /// How many bytes it covers.
    pub fn len(&self) -> usize {
        self.end - self.start
    }

    /// Whether it is empty.
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// Whether it strictly contains another span.
    pub fn contains(&self, other: &Span) -> bool {
        self != other && self.start <= other.start && other.end <= self.end
    }

    /// Whether it contains an offset.
    pub fn contains_offset(&self, offset: usize) -> bool {
        self.start <= offset && offset <= self.end
    }

    /// The span in one line.
    pub fn render(&self) -> String {
        format!("{}..{}", self.start, self.end)
    }
}

/// What a navigation request asks for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Query {
    /// Where a symbol is defined.
    Definition,
    /// Where a symbol is declared.
    Declaration,
    /// The implementations of a symbol.
    Implementation,
    /// Every reference to a symbol.
    References,
    /// The symbols of one document.
    DocumentSymbol,
    /// The symbols of the whole project.
    WorkspaceSymbol,
}

impl Query {
    /// Every query, in a stable order.
    pub const ALL: [Self; 6] = [
        Self::Definition,
        Self::Declaration,
        Self::Implementation,
        Self::References,
        Self::DocumentSymbol,
        Self::WorkspaceSymbol,
    ];

    /// The query's name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Definition => "definition",
            Self::Declaration => "declaration",
            Self::Implementation => "implementation",
            Self::References => "references",
            Self::DocumentSymbol => "document-symbol",
            Self::WorkspaceSymbol => "workspace-symbol",
        }
    }

    /// The query a name picks.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|query| query.name() == name)
    }

    /// The capability this query needs. Definitions, declarations and implementations share one.
    pub fn capability(self) -> Capability {
        match self {
            Self::Definition | Self::Declaration | Self::Implementation => Capability::Definition,
            Self::References => Capability::References,
            Self::DocumentSymbol | Self::WorkspaceSymbol => Capability::Symbols,
        }
    }
}

/// What a symbol is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SymbolKind {
    /// A module or namespace.
    Module,
    /// A type.
    Type,
    /// A function.
    Function,
    /// A method.
    Method,
    /// A field.
    Field,
    /// A variable.
    Variable,
    /// A constant.
    Constant,
}

impl SymbolKind {
    /// Every kind, in a stable order.
    pub const ALL: [Self; 7] = [
        Self::Module,
        Self::Type,
        Self::Function,
        Self::Method,
        Self::Field,
        Self::Variable,
        Self::Constant,
    ];

    /// The kind's name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Module => "module",
            Self::Type => "type",
            Self::Function => "function",
            Self::Method => "method",
            Self::Field => "field",
            Self::Variable => "variable",
            Self::Constant => "constant",
        }
    }

    /// The kind a name picks.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|kind| kind.name() == name)
    }

    /// The kind an LSP `SymbolKind` number names.
    ///
    /// The protocol has twenty-six kinds and this model has seven, so the mapping collapses the
    /// ones that mean the same thing to a reader of an outline (a class, an enum, a struct and an
    /// interface are all [`Self::Type`]) and keeps the rest from being dropped: a number this model
    /// has no closer word for is a [`Self::Variable`], not an error.
    pub fn from_lsp(number: u64) -> Self {
        match number {
            // File, module, namespace, package.
            1..=4 => Self::Module,
            // Class, enum, interface, struct, type parameter.
            5 | 10 | 11 | 23 | 26 => Self::Type,
            // Constructor, function.
            9 | 12 => Self::Function,
            // Method.
            6 => Self::Method,
            // Property, field, event.
            7 | 8 | 24 => Self::Field,
            // Enum member, constant.
            14 | 22 => Self::Constant,
            _ => Self::Variable,
        }
    }
}

/// A symbol the service reported.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Symbol {
    /// The document it is in.
    pub uri: String,
    /// Its name.
    pub name: String,
    /// What it is.
    pub kind: SymbolKind,
    /// The whole span of the definition.
    pub span: Span,
    /// The span of its name, which is where a jump lands.
    pub name_span: Span,
}

impl Symbol {
    /// A symbol.
    pub fn new(
        uri: impl Into<String>,
        name: impl Into<String>,
        kind: SymbolKind,
        span: Span,
        name_span: Span,
    ) -> Self {
        Self {
            uri: uri.into(),
            name: name.into(),
            kind,
            span,
            name_span,
        }
    }

    /// The location a jump to it lands on.
    pub fn location(&self) -> Location {
        Location {
            uri: self.uri.clone(),
            span: self.name_span,
        }
    }
}

/// A place a navigation result points at.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Location {
    /// The document.
    pub uri: String,
    /// The span within it.
    pub span: Span,
}

impl Location {
    /// The location in one line.
    pub fn render(&self) -> String {
        format!("{} {}", self.uri, self.span.render())
    }
}

/// An outline node: a symbol and the symbols nested inside it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct OutlineNode {
    /// The symbol's name.
    pub name: String,
    /// What it is.
    pub kind: SymbolKind,
    /// Its span.
    pub span: Span,
    /// The symbols strictly inside it.
    pub children: Vec<OutlineNode>,
}

impl OutlineNode {
    /// The node in lines, indented by depth.
    pub fn render(&self, depth: usize) -> Vec<String> {
        let mut lines = vec![format!(
            "{}{} [{}]",
            "  ".repeat(depth),
            self.name,
            self.kind.name()
        )];
        for child in &self.children {
            lines.extend(child.render(depth + 1));
        }
        lines
    }
}

/// Why a query was not answered.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum QueryRefusal {
    /// The service did not negotiate the capability the query needs.
    Unsupported {
        /// The query.
        query: Query,
        /// The capability it needs.
        capability: Capability,
    },
    /// The target named nothing to look for.
    EmptyTarget,
}

impl QueryRefusal {
    /// The refusal in one sentence.
    pub fn message(&self) -> String {
        match self {
            Self::Unsupported { query, capability } => format!(
                "the service did not negotiate {}, so {} is disabled",
                capability.name(),
                query.name()
            ),
            Self::EmptyTarget => "the query names no symbol".to_owned(),
        }
    }
}

/// What a query is asked about.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Target {
    /// A symbol by name.
    Symbol(String),
    /// A byte offset in a document.
    Position {
        /// The document.
        uri: String,
        /// The offset.
        offset: usize,
    },
}

/// The navigation a project's service offers, over the symbols it reported.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Navigation {
    capabilities: Capabilities,
    symbols: Vec<Symbol>,
}

impl Navigation {
    /// Navigation over a set of negotiated capabilities.
    pub fn new(capabilities: Capabilities) -> Self {
        Self {
            capabilities,
            symbols: Vec::new(),
        }
    }

    /// Records a symbol the service reported.
    pub fn add(&mut self, symbol: Symbol) {
        self.symbols.push(symbol);
    }

    /// Whether the service negotiated the capability a query needs.
    pub fn supports(&self, query: Query) -> bool {
        self.capabilities.supports(query.capability())
    }

    /// Answers a query, or says why it is disabled.
    pub fn query(&self, query: Query, target: &Target) -> Result<Vec<Location>, QueryRefusal> {
        if !self.supports(query) {
            return Err(QueryRefusal::Unsupported {
                query,
                capability: query.capability(),
            });
        }
        let name = match target {
            Target::Symbol(name) => name.clone(),
            Target::Position { uri, offset } => match self.symbol_at(uri, *offset) {
                Some(symbol) => symbol.name.clone(),
                None => return Ok(Vec::new()),
            },
        };
        if name.is_empty() {
            return Err(QueryRefusal::EmptyTarget);
        }
        let mut locations: Vec<Location> = self
            .symbols
            .iter()
            .filter(|symbol| symbol.name == name)
            .map(Symbol::location)
            .collect();
        locations.sort_by(|a, b| {
            (&a.uri, a.span.start, a.span.end).cmp(&(&b.uri, b.span.start, b.span.end))
        });
        Ok(locations)
    }

    /// The outline of one document.
    pub fn outline(&self, uri: &str) -> Vec<OutlineNode> {
        let symbols: Vec<Symbol> = self
            .symbols
            .iter()
            .filter(|symbol| symbol.uri == uri)
            .cloned()
            .collect();
        build_outline(&symbols)
    }

    /// The breadcrumb path to an offset: the symbols containing it, outermost first.
    pub fn breadcrumbs(&self, uri: &str, offset: usize) -> Vec<String> {
        let mut containing: Vec<&Symbol> = self
            .symbols
            .iter()
            .filter(|symbol| symbol.uri == uri && symbol.span.contains_offset(offset))
            .collect();
        containing.sort_by_key(|symbol| (symbol.span.start, std::cmp::Reverse(symbol.span.end)));
        containing
            .into_iter()
            .map(|symbol| symbol.name.clone())
            .collect()
    }

    /// The documents with at least one symbol.
    pub fn documents(&self) -> Vec<&str> {
        let mut uris: Vec<&str> = self
            .symbols
            .iter()
            .map(|symbol| symbol.uri.as_str())
            .collect();
        uris.sort_unstable();
        uris.dedup();
        uris
    }

    /// The innermost symbol whose span contains an offset.
    fn symbol_at(&self, uri: &str, offset: usize) -> Option<&Symbol> {
        self.symbols
            .iter()
            .filter(|symbol| symbol.uri == uri && symbol.span.contains_offset(offset))
            .min_by_key(|symbol| symbol.span.len())
    }
}

/// The symbols an LSP `textDocument/documentSymbol` result reports, with byte spans.
///
/// Both shapes the protocol allows are accepted: `DocumentSymbol` (which carries `range`,
/// `selectionRange` and nested `children`) and `SymbolInformation` (whose range lives under
/// `location` and which is flat). Positions are UTF-16 pairs, so they are converted against `text`
/// — the same buffer the request was made for — and a symbol the conversion cannot place is dropped
/// rather than guessed at.
///
/// The name span is the jump target: `selectionRange` when the server sent one, else the whole
/// range, which is the closest thing `SymbolInformation` can say.
pub fn symbols_from_lsp(uri: &str, text: &str, result: &Json) -> Vec<Symbol> {
    let mut out = Vec::new();
    if let Some(items) = result.as_array() {
        for item in items {
            collect_symbol(uri, text, item, &mut out);
        }
    }
    out.sort_by_key(|symbol| (symbol.span.start, symbol.span.end));
    out
}

/// One `DocumentSymbol`/`SymbolInformation` object, and its `children` when it has any.
fn collect_symbol(uri: &str, text: &str, item: &Json, out: &mut Vec<Symbol>) {
    let Some(object) = item.as_object() else {
        return;
    };
    let Some(name) = object.get("name").and_then(Json::as_str) else {
        return;
    };
    if name.is_empty() {
        return;
    }
    let location = object.get("location").and_then(Json::as_object);
    let span = range_span(location.and_then(|location| location.get("range")), text)
        .or_else(|| range_span(object.get("range"), text));
    let Some(span) = span else {
        return;
    };
    let name_span = range_span(object.get("selectionRange"), text).unwrap_or(span);
    let kind = object
        .get("kind")
        .and_then(Json::as_u64)
        .map(SymbolKind::from_lsp)
        .unwrap_or(SymbolKind::Variable);
    out.push(Symbol::new(uri, name, kind, span, name_span));
    if let Some(children) = object.get("children").and_then(Json::as_array) {
        for child in children {
            collect_symbol(uri, text, child, out);
        }
    }
}

/// The byte span an LSP `Range` names, when both of its positions exist in `text`.
fn range_span(range: Option<&Json>, text: &str) -> Option<Span> {
    let range = range.and_then(Json::as_object)?;
    let start = position_offset(range.get("start"), text)?;
    let end = position_offset(range.get("end"), text)?;
    Some(Span::new(start, end))
}

/// The byte offset an LSP `Position` names.
fn position_offset(position: Option<&Json>, text: &str) -> Option<usize> {
    let position = position.and_then(Json::as_object)?;
    let line = position.get("line").and_then(Json::as_u64)? as u32;
    let character = position
        .get("character")
        .and_then(Json::as_u64)
        .unwrap_or(0) as u32;
    to_offset(text, Position::new(line, character))
}

/// Builds the outline tree from spans: a symbol strictly inside another is its child.
fn build_outline(symbols: &[Symbol]) -> Vec<OutlineNode> {
    let mut roots = Vec::new();
    for (index, symbol) in symbols.iter().enumerate() {
        let inside = symbols
            .iter()
            .enumerate()
            .any(|(other, candidate)| other != index && candidate.span.contains(&symbol.span));
        if inside {
            continue;
        }
        roots.push(node_of(symbol, symbols, index));
    }
    sort_nodes(&mut roots);
    roots
}

/// One outline node: the symbol, with the symbols directly inside it as children.
fn node_of(symbol: &Symbol, symbols: &[Symbol], index: usize) -> OutlineNode {
    let mut children = Vec::new();
    for (other, candidate) in symbols.iter().enumerate() {
        if other == index || !symbol.span.contains(&candidate.span) {
            continue;
        }
        // A direct child is one not contained in another candidate inside the same symbol.
        let nested_deeper = symbols.iter().enumerate().any(|(third, middle)| {
            third != index
                && third != other
                && symbol.span.contains(&middle.span)
                && middle.span.contains(&candidate.span)
        });
        if !nested_deeper {
            children.push(node_of(candidate, symbols, other));
        }
    }
    sort_nodes(&mut children);
    OutlineNode {
        name: symbol.name.clone(),
        kind: symbol.kind,
        span: symbol.span,
        children,
    }
}

/// Orders nodes by where they start.
fn sort_nodes(nodes: &mut [OutlineNode]) {
    nodes.sort_by_key(|node| node.span.start);
}

#[cfg(test)]
mod tests {
    use super::{
        Location, Navigation, OutlineNode, Query, QueryRefusal, Span, Symbol, SymbolKind, Target,
    };
    use crate::app::language::session::{Capabilities, Capability};

    fn navigation() -> Navigation {
        let mut nav = Navigation::new(Capabilities::all());
        nav.add(Symbol::new(
            "file:///a.rs",
            "module",
            SymbolKind::Module,
            Span::new(0, 200),
            Span::new(7, 13),
        ));
        nav.add(Symbol::new(
            "file:///a.rs",
            "foo",
            SymbolKind::Function,
            Span::new(20, 80),
            Span::new(23, 26),
        ));
        nav.add(Symbol::new(
            "file:///a.rs",
            "bar",
            SymbolKind::Function,
            Span::new(100, 160),
            Span::new(103, 106),
        ));
        nav.add(Symbol::new(
            "file:///b.rs",
            "foo",
            SymbolKind::Function,
            Span::new(10, 50),
            Span::new(13, 16),
        ));
        nav
    }

    #[test]
    fn an_unsupported_capability_disables_its_queries() {
        let mut nav = Navigation::new(Capabilities::of([Capability::Completion]));
        nav.add(Symbol::new(
            "file:///a.rs",
            "foo",
            SymbolKind::Function,
            Span::new(0, 10),
            Span::new(0, 3),
        ));
        match nav.query(Query::References, &Target::Symbol("foo".to_owned())) {
            Err(QueryRefusal::Unsupported { query, capability }) => {
                assert_eq!(query, Query::References);
                assert_eq!(capability, Capability::References);
            }
            other => panic!("expected a disabled query, got {other:?}"),
        }
        assert!(!nav.supports(Query::DocumentSymbol));
        assert!(!nav.supports(Query::Definition));
    }

    #[test]
    fn a_supported_query_returns_every_target() {
        let nav = navigation();
        let locations = nav
            .query(Query::Definition, &Target::Symbol("foo".to_owned()))
            .expect("supported");
        assert_eq!(
            locations,
            vec![
                Location {
                    uri: "file:///a.rs".to_owned(),
                    span: Span::new(23, 26)
                },
                Location {
                    uri: "file:///b.rs".to_owned(),
                    span: Span::new(13, 16)
                },
            ]
        );
    }

    #[test]
    fn the_outline_nests_a_function_inside_its_module() {
        let nav = navigation();
        let outline = nav.outline("file:///a.rs");
        assert_eq!(outline.len(), 1);
        let module = &outline[0];
        assert_eq!(module.name, "module");
        let names: Vec<&str> = module
            .children
            .iter()
            .map(|child| child.name.as_str())
            .collect();
        assert_eq!(names, vec!["foo", "bar"]);
    }

    #[test]
    fn breadcrumbs_name_the_containing_symbols_outermost_first() {
        let nav = navigation();
        // Offset 25 is inside "foo", which is inside "module".
        assert_eq!(
            nav.breadcrumbs("file:///a.rs", 25),
            vec!["module".to_owned(), "foo".to_owned()]
        );
        // Offset 90 is inside "module" but in no function (between foo and bar).
        assert_eq!(
            nav.breadcrumbs("file:///a.rs", 90),
            vec!["module".to_owned()]
        );
    }

    #[test]
    fn a_position_target_resolves_to_the_symbol_containing_it() {
        let nav = navigation();
        let locations = nav
            .query(
                Query::References,
                &Target::Position {
                    uri: "file:///a.rs".to_owned(),
                    offset: 25,
                },
            )
            .expect("supported");
        // The symbol at offset 25 is "foo", so every "foo" is a target.
        assert_eq!(locations.len(), 2);
    }

    #[test]
    fn the_outline_renders_with_indentation() {
        let nav = navigation();
        let outline: Vec<OutlineNode> = nav.outline("file:///a.rs");
        let rendered: Vec<String> = outline.iter().flat_map(|node| node.render(0)).collect();
        assert_eq!(rendered[0], "module [module]");
        assert_eq!(rendered[1], "  foo [function]");
        assert_eq!(rendered[2], "  bar [function]");
    }

    #[test]
    fn a_query_name_and_symbol_kind_round_trip() {
        for query in Query::ALL {
            assert_eq!(Query::from_name(query.name()), Some(query));
        }
        assert_eq!(Query::from_name("nowhere"), None);
        for kind in SymbolKind::ALL {
            assert_eq!(SymbolKind::from_name(kind.name()), Some(kind));
        }
    }

    #[test]
    fn an_lsp_document_symbol_tree_becomes_symbols_with_byte_spans() {
        use crate::project::record::json::parse;

        // Two lines of Chinese shift every offset by three bytes per character: the conversion
        // has to count UTF-16 units, not bytes, or "main" lands in the comment.
        let text = "// 中文注释\nfn main() {\n    let run = 1;\n}\nconst LIMIT: u32 = 1;\n";
        let result = parse(
            r#"[
                {"name": "main", "kind": 12,
                 "range": {"start": {"line": 1, "character": 0},
                           "end": {"line": 3, "character": 1}},
                 "selectionRange": {"start": {"line": 1, "character": 3},
                                    "end": {"line": 1, "character": 7}},
                 "children": [
                   {"name": "run", "kind": 6,
                    "range": {"start": {"line": 2, "character": 4},
                              "end": {"line": 2, "character": 17}},
                    "selectionRange": {"start": {"line": 2, "character": 8},
                                       "end": {"line": 2, "character": 11}}}
                 ]},
                {"name": "LIMIT", "kind": 14,
                 "location": {"uri": "file:///a.rs",
                              "range": {"start": {"line": 4, "character": 6},
                                        "end": {"line": 4, "character": 11}}}}
            ]"#,
        )
        .expect("valid JSON");

        let symbols = super::symbols_from_lsp("file:///a.rs", text, &result);
        let names: Vec<&str> = symbols.iter().map(|symbol| symbol.name.as_str()).collect();
        assert_eq!(names, vec!["main", "run", "LIMIT"]);
        assert_eq!(symbols[0].kind, SymbolKind::Function);
        assert_eq!(symbols[1].kind, SymbolKind::Method);
        assert_eq!(symbols[2].kind, SymbolKind::Constant);
        // The name span is `main`, not the whole declaration.
        assert_eq!(
            &text[symbols[0].name_span.start..symbols[0].name_span.end],
            "main"
        );
        assert_eq!(
            &text[symbols[1].name_span.start..symbols[1].name_span.end],
            "run"
        );
        // A `SymbolInformation` has no selection range, so its whole range is the jump target.
        assert_eq!(symbols[2].name_span, symbols[2].span);

        let mut nav = Navigation::new(Capabilities::of([Capability::Symbols]));
        for symbol in symbols {
            nav.add(symbol);
        }
        let outline = nav.outline("file:///a.rs");
        assert_eq!(outline.len(), 2);
        assert_eq!(outline[0].name, "main");
        assert_eq!(outline[0].children.len(), 1);
        assert_eq!(outline[0].children[0].name, "run");
    }

    #[test]
    fn an_lsp_symbol_whose_range_is_off_the_document_is_dropped() {
        use crate::project::record::json::parse;

        let text = "fn main() {}\n";
        let result = parse(
            r#"[{"name": "ghost", "kind": 12,
                 "range": {"start": {"line": 7, "character": 0},
                           "end": {"line": 8, "character": 0}},
                 "selectionRange": {"start": {"line": 7, "character": 0},
                                    "end": {"line": 7, "character": 5}}}]"#,
        )
        .expect("valid JSON");
        assert!(super::symbols_from_lsp("file:///a.rs", text, &result).is_empty());
    }

    #[test]
    fn the_protocol_symbol_kinds_collapse_onto_the_models() {
        assert_eq!(SymbolKind::from_lsp(12), SymbolKind::Function);
        assert_eq!(SymbolKind::from_lsp(6), SymbolKind::Method);
        assert_eq!(SymbolKind::from_lsp(23), SymbolKind::Type);
        assert_eq!(SymbolKind::from_lsp(8), SymbolKind::Field);
        assert_eq!(SymbolKind::from_lsp(14), SymbolKind::Constant);
        assert_eq!(SymbolKind::from_lsp(13), SymbolKind::Variable);
    }
}
