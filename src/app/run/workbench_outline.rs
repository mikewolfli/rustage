// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The source-outline panel's rows (roadmap F07, `panel.outline`).
//!
//! The document is the one the focused editor group shows. Its symbols come from the language
//! server's `textDocument/documentSymbol` through [`crate::app::language::navigation`], and its
//! generated/handwritten regions come from the same markers the merge path reads
//! ([`crate::app::generate::artifact`]). Activating a row moves the editor's caret to the symbol:
//! the outline and the editor read one buffer, so the offset the server reported is the offset the
//! editor jumps to.
//!
//! **Node association.** The panel does not start a generation run; the caller passes one in when it
//! has it. When the editor buffer is exactly the text that run produced, each symbol is labelled with
//! the design node whose emitted expression contains it (upstream `GenerationReport::node_anchors`,
//! RW-42). A buffer that is not that text gets a row saying so rather than an association inferred
//! from a neighbouring line.

use crate::app::generate::artifact::{GENERATED_OPEN, HANDWRITTEN_OPEN, Region, RegionKind};
use crate::app::language::navigation::{Navigation, OutlineNode, Symbol};
use crate::app::language::session::{Capabilities, Capability, capabilities_from_lsp};
use crate::host::lsp::Client;
use crate::project::record::json::{Json, JsonObject};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long one request to the server may take.
const REQUEST_TIMEOUT: Duration = crate::host::lsp::DEFAULT_TIMEOUT;

/// A generation run whose text and per-node anchors the panel may use.
///
/// `source` is the text the anchors' offsets are relative to; the association is only used when the
/// editor buffer is byte-for-byte that text.
pub(super) struct Generated<'a> {
    /// The page the run read, so a row can name where the anchors came from.
    pub page: &'a str,
    /// The generated text.
    pub source: &'a str,
    /// One entry per emitted node, from `GenerationReport::node_anchors`.
    pub anchors: &'a [rust_widgets::designer::NodeAnchor],
}

/// The outline's state: the server it may start, and the document it last read.
pub(super) struct SourceOutline {
    program: Option<String>,
    arguments: Vec<String>,
    client: Option<Client>,
    capabilities: Capabilities,
    root: Option<PathBuf>,
    /// The document the server holds open, so a refresh is a change rather than a second open.
    opened: Option<String>,
    version: u64,
    /// The last failure, in the server's own words; cleared by a refresh that reaches the server.
    failure: Option<String>,
    document: Option<Loaded>,
    /// The rows `render` produced, so an activation can find its target again.
    rendered: Vec<Row>,
}

/// What one refresh read from the server and the buffer.
struct Loaded {
    name: String,
    text: String,
    outline: Vec<OutlineNode>,
    symbols: Vec<Symbol>,
    regions: Result<Vec<Region>, String>,
    /// Whether the buffer declares generated/handwritten markers at all.
    ///
    /// [`crate::app::generate::artifact::regions`] treats a marker-free file as one generated
    /// region. That is right for a merge, and wrong as an outline label: a hand-written file is not
    /// "generated", so the panel only labels symbols when a marker is actually present.
    has_markers: bool,
    note: Option<String>,
    /// The design node whose emitted expression contains a symbol's name, by name offset.
    nodes: BTreeMap<usize, String>,
    /// One row per emitted node, in generation order, each jumping to its expression.
    node_rows: Vec<Row>,
    /// The row that says whether the association is active, and why not when it is not.
    mapping: String,
}

/// One panel row and where activating it moves the caret.
pub(super) struct Row {
    text: String,
    /// The zero-based line and character column of the symbol's name.
    target: Option<(usize, usize)>,
}

impl Row {
    fn message(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            target: None,
        }
    }
}

/// One workspace symbol the server reported, with the position it named.
///
/// The position is kept in the protocol's own units (a zero-based line and a UTF-16 character) so
/// it can be resolved against the file the caller reads; a symbol whose document is outside the
/// caller's project is refused there rather than opened.
pub(super) struct WorkspaceSymbol {
    /// The symbol's name.
    pub(super) name: String,
    /// The document it was declared in, as the server named it.
    pub(super) uri: String,
    /// The zero-based line of the name.
    pub(super) line: u32,
    /// The zero-based character of the name, in UTF-16 code units.
    pub(super) character: u32,
}

/// One code-navigation query the editor can ask the server (roadmap `nav.code.*`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum CodeQuery {
    /// Where the symbol is defined.
    Definition,
    /// Where the symbol is declared.
    Declaration,
    /// The type's definition.
    TypeDefinition,
    /// The implementation of a trait or interface member.
    Implementation,
    /// Every reference to the symbol.
    References,
}

impl CodeQuery {
    /// The protocol method the query sends.
    pub(super) fn method(self) -> &'static str {
        match self {
            Self::Definition => "textDocument/definition",
            Self::Declaration => "textDocument/declaration",
            Self::TypeDefinition => "textDocument/typeDefinition",
            Self::Implementation => "textDocument/implementation",
            Self::References => "textDocument/references",
        }
    }

    /// The capability the query needs.
    pub(super) fn capability(self) -> Capability {
        match self {
            Self::Definition | Self::Declaration | Self::TypeDefinition | Self::Implementation => {
                Capability::Definition
            }
            Self::References => Capability::References,
        }
    }
}

/// One location a code query reported, in the protocol's own units.
///
/// The position is resolved against the target file when it is opened, so the value here is the
/// server's answer rather than a column this build guessed from another document's text.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct CodeLocation {
    /// The document it names.
    pub(super) uri: String,
    /// The zero-based line.
    pub(super) line: u32,
    /// The zero-based character, in UTF-16 code units.
    pub(super) character: u32,
}

impl SourceOutline {
    /// A panel that will start `program` with `arguments` the first time it reads a document.
    pub(super) fn new(program: Option<String>, arguments: Vec<String>) -> Self {
        Self {
            program,
            arguments,
            client: None,
            capabilities: Capabilities::none(),
            root: None,
            opened: None,
            version: 0,
            failure: None,
            document: None,
            rendered: Vec::new(),
        }
    }

    /// Whether a language server was named on the command line.
    pub(super) fn is_configured(&self) -> bool {
        self.program.is_some()
    }

    /// The document the rows are about, when one was read.
    pub(super) fn document(&self) -> Option<&str> {
        self.document
            .as_ref()
            .map(|document| document.name.as_str())
    }

    /// The text the rows were read from.
    pub(super) fn document_text(&self) -> Option<&str> {
        self.document
            .as_ref()
            .map(|document| document.text.as_str())
    }

    /// The last failure, in the server's own words.
    pub(super) fn failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }

    /// How many symbols the server reported for the document last read.
    pub(super) fn symbol_count(&self) -> usize {
        self.document
            .as_ref()
            .map(|document| document.symbols.len())
            .unwrap_or(0)
    }

    /// The row that says whether the node association is active.
    pub(super) fn mapping(&self) -> Option<&str> {
        self.document
            .as_ref()
            .map(|document| document.mapping.as_str())
    }

    /// How many symbols the panel labelled with a design node.
    pub(super) fn associated_symbols(&self) -> usize {
        self.document
            .as_ref()
            .map(|document| document.nodes.len())
            .unwrap_or(0)
    }

    /// How many emitted nodes the panel can jump to.
    pub(super) fn node_count(&self) -> usize {
        self.document
            .as_ref()
            .map(|document| document.node_rows.len())
            .unwrap_or(0)
    }

    /// Reads `document` (the editor's buffer text) through the server.
    ///
    /// The server is started once, against `root`, and kept for the session; a later refresh sends
    /// the new text as a full change rather than reopening the document.
    ///
    /// `generated` is the caller's generation run, when it has one. The per-node anchors are only
    /// used for a buffer that is exactly the text that run produced; any other buffer is reported
    /// as unassociated rather than mapped through offsets that belong to another document.
    pub(super) fn refresh(
        &mut self,
        root: &Path,
        document: &str,
        text: &str,
        generated: Option<Generated<'_>>,
    ) {
        let Some(program) = self.program.clone() else {
            return;
        };
        self.failure = None;
        self.ensure_started(program.as_str(), root);
        let uri = match self.ensure_open(root, document, text) {
            Ok(uri) => uri,
            Err(reason) => {
                self.failure = Some(reason);
                self.document = None;
                return;
            }
        };
        let Some(client) = self.client.as_mut() else {
            self.document = None;
            return;
        };

        let mut note = None;
        let symbols = if self.capabilities.supports(Capability::Symbols) {
            let mut text_document = JsonObject::new();
            text_document.insert("uri", Json::string(&uri));
            let mut params = JsonObject::new();
            params.insert("textDocument", Json::Object(text_document));
            match client.request(
                "textDocument/documentSymbol",
                Json::Object(params),
                REQUEST_TIMEOUT,
            ) {
                Ok(result) => {
                    crate::app::language::navigation::symbols_from_lsp(&uri, text, &result)
                }
                Err(reason) => {
                    self.failure = Some(reason);
                    Vec::new()
                }
            }
        } else {
            note = Some(
                "symbols: the service did not negotiate documentSymbol, so the outline is disabled"
                    .to_owned(),
            );
            Vec::new()
        };

        let mut navigation = Navigation::new(Capabilities::none());
        for symbol in &symbols {
            navigation.add(symbol.clone());
        }
        let (nodes, node_rows, mapping) = associate(&symbols, text, generated);
        self.document = Some(Loaded {
            name: document.to_owned(),
            text: text.to_owned(),
            outline: navigation.outline(&uri),
            symbols,
            regions: crate::app::generate::artifact::regions(text),
            has_markers: declares_regions(text),
            note,
            nodes,
            node_rows,
            mapping,
        });
    }

    /// Starts the server against `root` when one is configured and none is running.
    ///
    /// A session started against another root cannot answer for this one, so the old client is
    /// dropped and its negotiated state forgotten rather than reused across projects.
    fn ensure_started(&mut self, program: &str, root: &Path) {
        if self.root.as_deref() != Some(root) {
            self.client = None;
            self.capabilities = Capabilities::none();
            self.opened = None;
            self.version = 0;
            self.root = Some(root.to_path_buf());
        }
        if self.client.is_none() {
            self.start(program, root);
        }
    }

    /// Opens `document` (or sends a full change when it is already open) and answers its URI.
    fn ensure_open(&mut self, root: &Path, document: &str, text: &str) -> Result<String, String> {
        let Some(program) = self.program.clone() else {
            return Err("no language server is configured".to_owned());
        };
        self.ensure_started(&program, root);
        let Some(client) = self.client.as_mut() else {
            return Err(self
                .failure
                .clone()
                .unwrap_or_else(|| "the language server is not running".to_owned()));
        };
        let uri = document_uri(root, document);
        if self.opened.as_deref() != Some(uri.as_str()) {
            if let Some(previous) = self.opened.take()
                && let Err(reason) = client.did_close(&previous)
            {
                self.failure = Some(reason);
            }
            self.version = 1;
            let language = crate::app::run::language::language_id(document);
            client.did_open(&uri, language, self.version, text)?;
            self.opened = Some(uri.clone());
        } else {
            self.version += 1;
            client.did_change_full(&uri, self.version, text)?;
        }
        Ok(uri)
    }

    /// The locations a code-navigation query reports at `line`/`column` in `document`.
    ///
    /// The caret position is converted to the protocol's UTF-16 units first. A query whose
    /// capability was not negotiated is refused with the capability's name, so the caller can say
    /// why it is unavailable rather than pretending nothing matched.
    pub(super) fn code_locations(
        &mut self,
        root: &Path,
        document: &str,
        text: &str,
        line: usize,
        column: usize,
        query: CodeQuery,
    ) -> Result<Vec<CodeLocation>, String> {
        let (line, character) = units(text, line, column)
            .ok_or_else(|| format!("line {} has no column {}", line + 1, column + 1))?;
        let uri = self.ensure_open(root, document, text)?;
        if !self.capabilities.supports(query.capability()) {
            return Err(format!(
                "the service did not negotiate {}",
                query.capability().name()
            ));
        }
        let Some(client) = self.client.as_mut() else {
            return Err("the language server is not running".to_owned());
        };
        let mut text_document = JsonObject::new();
        text_document.insert("uri", Json::string(&uri));
        let mut position = JsonObject::new();
        position.insert("line", Json::number(u64::from(line)));
        position.insert("character", Json::number(u64::from(character)));
        let mut params = JsonObject::new();
        params.insert("textDocument", Json::Object(text_document));
        params.insert("position", Json::Object(position));
        if query == CodeQuery::References {
            let mut context = JsonObject::new();
            context.insert("includeDeclaration", Json::bool(false));
            params.insert("context", Json::Object(context));
        }
        let result = client
            .request(query.method(), Json::Object(params), REQUEST_TIMEOUT)
            .inspect_err(|reason| {
                self.failure = Some(reason.clone());
            })?;
        Ok(code_locations_from_lsp(&result))
    }

    /// Starts the server and negotiates its capabilities, or records why it did not start.
    fn start(&mut self, program: &str, root: &Path) {
        let mut client = match Client::start(program, &self.arguments) {
            Ok(client) => client,
            Err(reason) => {
                self.failure = Some(format!("{program}: {reason}"));
                return;
            }
        };
        match client.initialize(root, REQUEST_TIMEOUT) {
            Ok(result) => {
                self.capabilities = result
                    .as_object()
                    .and_then(|object| object.get("capabilities"))
                    .map(capabilities_from_lsp)
                    .unwrap_or_else(Capabilities::none);
                self.client = Some(client);
            }
            Err(reason) => self.failure = Some(format!("{program}: {reason}")),
        }
    }

    /// The rows for the document last read, narrowed by `filter`.
    pub(super) fn render(&mut self, filter: &str) -> Vec<String> {
        self.rendered = self.rows(filter);
        self.rendered.iter().map(|row| row.text.clone()).collect()
    }

    /// Where activating row `index` of the last render moves the caret.
    pub(super) fn target(&self, index: usize) -> Option<(usize, usize)> {
        self.rendered.get(index).and_then(|row| row.target)
    }

    /// The first row of the last render that names a symbol.
    pub(super) fn first_target(&self) -> Option<usize> {
        self.rendered.iter().position(|row| row.target.is_some())
    }

    /// The symbols containing `offset`, outermost first.
    ///
    /// Nothing is invented when no symbol contains the offset: an empty list is the honest answer,
    /// and the caller says so rather than showing the last symbol that happened to be nearby.
    pub(super) fn breadcrumbs(&self, offset: usize) -> Vec<String> {
        let Some(document) = &self.document else {
            return Vec::new();
        };
        let mut containing: Vec<&Symbol> = document
            .symbols
            .iter()
            .filter(|symbol| symbol.span.contains_offset(offset))
            .collect();
        // Outermost first: an earlier start encloses a later one, and for the same start the
        // longer span is the outer symbol.
        containing.sort_by(|left, right| {
            left.span
                .start
                .cmp(&right.span.start)
                .then(right.span.end.cmp(&left.span.end))
        });
        containing
            .into_iter()
            .map(|symbol| symbol.name.clone())
            .collect()
    }

    /// The best file-symbol match for `query`: its name and zero-based line and column.
    ///
    /// An exact name wins; otherwise the best fuzzy score wins, so a query is a name rather than a
    /// substring. Symbols carrying no name span cannot be placed and are not offered.
    pub(super) fn file_symbol(&self, query: &str) -> Option<(String, usize, usize)> {
        use crate::app::search::quick_open::fuzzy_score;

        let query = query.trim();
        if query.is_empty() {
            return None;
        }
        let document = self.document.as_ref()?;
        let symbol = document
            .symbols
            .iter()
            .find(|symbol| symbol.name == query)
            .or_else(|| {
                document
                    .symbols
                    .iter()
                    .filter_map(|symbol| {
                        fuzzy_score(query, &symbol.name).map(|score| (score, symbol))
                    })
                    .max_by_key(|(score, _)| *score)
                    .map(|(_, symbol)| symbol)
            })?;
        let (line, column) = line_col(&document.text, symbol.name_span.start);
        Some((symbol.name.clone(), line, column))
    }

    /// Queries the server for workspace symbols matching `query`.
    ///
    /// The server is started against `root` first when it is configured but not yet running. The
    /// request is the protocol's own `workspace/symbol`, so the result is the server's answer rather
    /// than a scan of the panel's rows.
    pub(super) fn workspace_symbols(
        &mut self,
        root: &Path,
        query: &str,
    ) -> Result<Vec<WorkspaceSymbol>, String> {
        let query = query.trim();
        if query.is_empty() {
            return Err("the symbol query is empty".to_owned());
        }
        if let Some(program) = self.program.clone() {
            self.ensure_started(&program, root);
        }
        if !self.capabilities.supports(Capability::Symbols) {
            return Err(
                "the service did not negotiate symbols, so a workspace symbol search is disabled"
                    .to_owned(),
            );
        }
        let Some(client) = self.client.as_mut() else {
            return Err(self
                .failure
                .clone()
                .unwrap_or_else(|| "the language server is not running".to_owned()));
        };
        let mut params = JsonObject::new();
        params.insert("query", Json::string(query));
        let result = client
            .request("workspace/symbol", Json::Object(params), REQUEST_TIMEOUT)
            .inspect_err(|reason| {
                self.failure = Some(reason.clone());
            })?;
        Ok(workspace_symbols_from_lsp(&result))
    }

    /// The zero-based line and character column an LSP position names in `text`.
    ///
    /// The protocol counts characters in UTF-16 code units; the editor counts characters, so the
    /// conversion is explicit rather than assuming a byte or a scalar value.
    pub(super) fn position(text: &str, line: u32, character: u32) -> Option<(usize, usize)> {
        let offset = crate::app::language::protocol::to_offset(
            text,
            crate::app::language::protocol::Position::new(line, character),
        )?;
        Some(line_col(text, offset))
    }

    fn rows(&self, filter: &str) -> Vec<Row> {
        let mut rows = Vec::new();
        let Some(program) = self.program.as_deref() else {
            rows.push(Row::message(
                "no language server is configured; start the workbench with --lsp <program>",
            ));
            return rows;
        };
        rows.push(Row::message(format!("language server: {program}")));
        if let Some(failure) = &self.failure {
            rows.push(Row::message(format!("language server failed: {failure}")));
        }
        let Some(document) = &self.document else {
            rows.push(Row::message("no editor document to outline"));
            return rows;
        };
        rows.push(Row::message(format!("source: {}", document.name)));
        if document.has_markers {
            match &document.regions {
                Ok(regions) => {
                    let generated = regions
                        .iter()
                        .filter(|region| region.kind == RegionKind::Generated)
                        .count();
                    rows.push(Row::message(format!(
                        "regions: {generated} generated, {} handwritten",
                        regions.len() - generated
                    )));
                }
                Err(reason) => rows.push(Row::message(format!("regions: {reason}"))),
            }
        } else {
            rows.push(Row::message(
                "regions: this document declares no generated/handwritten markers",
            ));
        }
        rows.push(Row::message(document.mapping.clone()));
        if let Some(note) = &document.note {
            rows.push(Row::message(note.clone()));
        }
        if !document.node_rows.is_empty() {
            rows.push(Row::message(format!("nodes: {}", document.node_rows.len())));
            rows.extend(document.node_rows.iter().map(|row| Row {
                text: row.text.clone(),
                target: row.target,
            }));
        }

        let filter = filter.trim().to_lowercase();
        if filter.is_empty() {
            for node in &document.outline {
                push_node(&mut rows, node, 0, document);
            }
        } else {
            let matching: Vec<&Symbol> = document
                .symbols
                .iter()
                .filter(|symbol| symbol.name.to_lowercase().contains(&filter))
                .collect();
            rows.push(Row::message(format!(
                "filter: {} of {} symbol(s)",
                matching.len(),
                document.symbols.len()
            )));
            for symbol in matching {
                push_symbol(&mut rows, symbol, 0, document);
            }
        }
        rows
    }
}

/// One outline node and its children, innermost last.
fn push_node(rows: &mut Vec<Row>, node: &OutlineNode, depth: usize, document: &Loaded) {
    let name_offset = document
        .symbols
        .iter()
        .find(|symbol| symbol.span == node.span)
        .map(|symbol| symbol.name_span.start)
        .unwrap_or(node.span.start);
    rows.push(symbol_row(
        &node.name,
        node.kind,
        name_offset,
        depth,
        document,
    ));
    for child in &node.children {
        push_node(rows, child, depth + 1, document);
    }
}

/// One leaf symbol, for a filtered list.
fn push_symbol(rows: &mut Vec<Row>, symbol: &Symbol, depth: usize, document: &Loaded) {
    rows.push(symbol_row(
        &symbol.name,
        symbol.kind,
        symbol.name_span.start,
        depth,
        document,
    ));
}

/// A symbol row: its place in the tree, what it is, and where it starts.
fn symbol_row(
    name: &str,
    kind: crate::app::language::navigation::SymbolKind,
    offset: usize,
    depth: usize,
    document: &Loaded,
) -> Row {
    let (line, column) = line_col(&document.text, offset);
    let node = document
        .nodes
        .get(&offset)
        .map(|label| format!(" node {label}"))
        .unwrap_or_default();
    Row {
        text: format!(
            "{}{} [{}] {}:{}{}{}",
            "  ".repeat(depth),
            name,
            kind.name(),
            line + 1,
            column + 1,
            region_tag(document, offset),
            node
        ),
        target: Some((line, column)),
    }
}

/// The design node each symbol belongs to, and the row that explains the association.
///
/// The anchors are only used when the buffer is exactly the generated text: an offset from another
/// document would point at an unrelated line, and a wrong association reads like an answer.
fn associate(
    symbols: &[Symbol],
    text: &str,
    generated: Option<Generated<'_>>,
) -> (BTreeMap<usize, String>, Vec<Row>, String) {
    let mut nodes = BTreeMap::new();
    let Some(generated) = generated else {
        return (
            nodes,
            Vec::new(),
            "node mapping: no generation run is available for this project".to_owned(),
        );
    };
    if generated.source != text {
        return (
            nodes,
            Vec::new(),
            format!(
                "node mapping: {} generated {} byte(s), but the editor buffer is a different text",
                generated.page,
                generated.source.len()
            ),
        );
    }
    for symbol in symbols {
        if let Some(anchor) = innermost(generated.anchors, symbol.name_span.start) {
            nodes.insert(
                symbol.name_span.start,
                anchor.id.clone().unwrap_or_else(|| anchor.key.clone()),
            );
        }
    }
    let rows = generated
        .anchors
        .iter()
        .map(|anchor| {
            let (line, column) = line_col(text, anchor.offset);
            Row {
                text: format!(
                    "node {} [{}] {}:{}",
                    anchor.id.clone().unwrap_or_else(|| anchor.key.clone()),
                    anchor.widget,
                    line + 1,
                    column + 1
                ),
                target: Some((line, column)),
            }
        })
        .collect();
    (
        nodes,
        rows,
        format!(
            "node mapping: {} anchor(s) from generating {}",
            generated.anchors.len(),
            generated.page
        ),
    )
}

/// The innermost emitted node expression containing `offset`.
fn innermost(
    anchors: &[rust_widgets::designer::NodeAnchor],
    offset: usize,
) -> Option<&rust_widgets::designer::NodeAnchor> {
    anchors
        .iter()
        .filter(|anchor| {
            anchor.offset <= offset && offset < anchor.offset.saturating_add(anchor.length.max(1))
        })
        .min_by_key(|anchor| anchor.length)
}

/// The region label of a symbol, when the buffer declares regions and the symbol is inside one.
fn region_tag(document: &Loaded, offset: usize) -> &'static str {
    if !document.has_markers {
        return "";
    }
    let Ok(regions) = &document.regions else {
        return "";
    };
    let offset = offset as u32;
    match regions
        .iter()
        .find(|region| region.start <= offset && offset < region.end)
    {
        Some(region) => match region.kind {
            RegionKind::Generated => " [generated]",
            RegionKind::Handwritten => " [handwritten]",
        },
        None => "",
    }
}

/// Whether the buffer opens or closes a generated/handwritten region.
fn declares_regions(text: &str) -> bool {
    text.lines().any(|line| {
        let line = line.trim();
        line.starts_with(GENERATED_OPEN) || line.starts_with(HANDWRITTEN_OPEN)
    })
}

/// The `file://` URI of `document`, resolved against `root` when it is relative.
fn document_uri(root: &Path, document: &str) -> String {
    let path = if Path::new(document).is_absolute() {
        PathBuf::from(document)
    } else {
        root.join(document)
    };
    crate::host::lsp::file_uri(&path)
}

/// The byte offset of a zero-based line and character column.
///
/// A column past the line's end has no offset, the same rule [`units`] applies for the protocol.
pub(super) fn offset_of(text: &str, line: usize, column: usize) -> Option<usize> {
    let line_start = if line == 0 {
        0
    } else {
        text.match_indices('\n')
            .nth(line - 1)
            .map(|(index, _)| index + 1)?
    };
    let mut seen = 0usize;
    for (index, character) in text[line_start..].char_indices() {
        if character == '\n' {
            break;
        }
        if seen == column {
            return Some(line_start + index);
        }
        seen += 1;
    }
    if seen == column {
        let end = text[line_start..]
            .find('\n')
            .map_or(text.len(), |index| line_start + index);
        Some(end)
    } else {
        None
    }
}

/// The protocol position of a zero-based line and character column.
///
/// The editor counts characters; the protocol counts UTF-16 code units, so a character outside the
/// basic plane contributes two units. A column past the line's end has no position.
pub(super) fn units(text: &str, line: usize, column: usize) -> Option<(u32, u32)> {
    let line_start = if line == 0 {
        0
    } else {
        text.match_indices('\n')
            .nth(line - 1)
            .map(|(index, _)| index + 1)?
    };
    let mut units = 0u32;
    let mut seen = 0usize;
    for character in text[line_start..].chars() {
        if character == '\n' {
            break;
        }
        if seen == column {
            return Some((line as u32, units));
        }
        units += character.len_utf16() as u32;
        seen += 1;
    }
    if seen == column {
        Some((line as u32, units))
    } else {
        None
    }
}

/// The locations in a `textDocument/definition`-family answer.
///
/// Three shapes are accepted: one `Location`, a list of them, and `LocationLink` (whose target is
/// under `targetUri`/`targetSelectionRange`). A null answer is an empty list; an item this build
/// cannot read is dropped rather than guessed at.
fn code_locations_from_lsp(result: &Json) -> Vec<CodeLocation> {
    let items: Vec<&Json> = match result {
        Json::Null => Vec::new(),
        Json::Array(items) => items.iter().collect(),
        other => vec![other],
    };
    let mut out = Vec::new();
    for item in items {
        let Some(object) = item.as_object() else {
            continue;
        };
        let (uri, range) = if let Some(uri) = object.get("targetUri").and_then(Json::as_str) {
            (
                uri,
                object
                    .get("targetSelectionRange")
                    .or_else(|| object.get("targetRange")),
            )
        } else if let Some(uri) = object.get("uri").and_then(Json::as_str) {
            (uri, object.get("range"))
        } else {
            continue;
        };
        let Some(start) = range
            .and_then(Json::as_object)
            .and_then(|range| range.get("start"))
            .and_then(Json::as_object)
        else {
            continue;
        };
        let line = start.get("line").and_then(Json::as_u64).unwrap_or(0) as u32;
        let character = start.get("character").and_then(Json::as_u64).unwrap_or(0) as u32;
        out.push(CodeLocation {
            uri: uri.to_owned(),
            line,
            character,
        });
    }
    out
}

/// The workspace symbols in an LSP `workspace/symbol` answer.
///
/// Both shapes the protocol allows are accepted: `SymbolInformation` (with `location.range`) and
/// `WorkspaceSymbol` (with `location` carrying a URI and a range). Symbols with no name or no
/// location are dropped rather than guessed at.
fn workspace_symbols_from_lsp(result: &Json) -> Vec<WorkspaceSymbol> {
    let mut out = Vec::new();
    let Some(items) = result.as_array() else {
        return out;
    };
    for item in items {
        let Some(object) = item.as_object() else {
            continue;
        };
        let Some(name) = object.get("name").and_then(Json::as_str) else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        let Some(location) = object.get("location").and_then(Json::as_object) else {
            continue;
        };
        let Some(uri) = location.get("uri").and_then(Json::as_str) else {
            continue;
        };
        let start = location
            .get("range")
            .and_then(Json::as_object)
            .and_then(|range| range.get("start"))
            .and_then(Json::as_object);
        let line = start
            .and_then(|start| start.get("line"))
            .and_then(Json::as_u64)
            .unwrap_or(0) as u32;
        let character = start
            .and_then(|start| start.get("character"))
            .and_then(Json::as_u64)
            .unwrap_or(0) as u32;
        out.push(WorkspaceSymbol {
            name: name.to_owned(),
            uri: uri.to_owned(),
            line,
            character,
        });
    }
    out
}

/// The zero-based line and character column of a byte offset.
fn line_col(text: &str, offset: usize) -> (usize, usize) {
    let mut offset = offset.min(text.len());
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }
    let line = text[..offset].matches('\n').count();
    let line_start = text[..offset].rfind('\n').map_or(0, |index| index + 1);
    let column = text[line_start..offset].chars().count();
    (line, column)
}

#[cfg(test)]
mod tests {
    use super::{SourceOutline, line_col};
    use crate::app::language::navigation::{Navigation, Span, Symbol, SymbolKind};
    use crate::app::language::session::Capabilities;

    fn sources(text: &str, symbols: Vec<Symbol>) -> SourceOutline {
        let mut outline = SourceOutline::new(Some("rust-analyzer".to_owned()), Vec::new());
        let mut navigation = Navigation::new(Capabilities::none());
        for symbol in &symbols {
            navigation.add(symbol.clone());
        }
        outline.document = Some(super::Loaded {
            name: "src/main.rs".to_owned(),
            text: text.to_owned(),
            outline: navigation.outline("file:///src/main.rs"),
            symbols,
            regions: crate::app::generate::artifact::regions(text),
            has_markers: super::declares_regions(text),
            note: None,
            nodes: std::collections::BTreeMap::new(),
            node_rows: Vec::new(),
            mapping: "node mapping: no generation run is available for this project".to_owned(),
        });
        outline
    }

    fn symbol(name: &str, span: Span, name_span: Span) -> Symbol {
        Symbol::new(
            "file:///src/main.rs",
            name,
            SymbolKind::Function,
            span,
            name_span,
        )
    }

    #[test]
    fn a_file_symbol_query_prefers_an_exact_name_then_the_best_fuzzy_match() {
        let text = "fn main() {\n    run();\n}\nfn run() {}\n";
        let outline = sources(
            text,
            vec![
                symbol("main", Span::new(0, 12), Span::new(3, 7)),
                symbol("run", Span::new(25, 36), Span::new(28, 31)),
            ],
        );
        assert_eq!(outline.file_symbol("run"), Some(("run".to_owned(), 3, 3)));
        assert_eq!(
            outline.file_symbol("  main "),
            Some(("main".to_owned(), 0, 3))
        );
        assert_eq!(outline.file_symbol("nope"), None);
        assert_eq!(outline.file_symbol("   "), None);
    }

    #[test]
    fn a_workspace_symbol_answer_names_each_symbol_and_its_position() {
        let answer = crate::project::record::json::parse(
            r#"[
                {"name":"build","kind":12,"location":{"uri":"file:///src/main.rs","range":{"start":{"line":4,"character":7},"end":{"line":4,"character":12}}}},
                {"name":"","location":{"uri":"file:///src/lib.rs"}},
                {"location":{"uri":"file:///src/lib.rs"}},
                {"name":"orphan"}
            ]"#,
        )
        .expect("json");
        let symbols = super::workspace_symbols_from_lsp(&answer);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "build");
        assert_eq!(symbols[0].uri, "file:///src/main.rs");
        assert_eq!((symbols[0].line, symbols[0].character), (4, 7));
    }

    #[test]
    fn a_protocol_position_is_converted_from_utf16_units() {
        let text = "let 汉 = 1;\nfoo\n";
        // `汉` is one UTF-16 code unit, so character 4 is the byte after it.
        assert_eq!(SourceOutline::position(text, 0, 4), Some((0, 4)));
        assert_eq!(SourceOutline::position(text, 1, 0), Some((1, 0)));
        assert_eq!(SourceOutline::position(text, 9, 0), None);
    }

    #[test]
    fn a_caret_position_becomes_a_byte_offset() {
        let text = "fn main() {\n    run();\n}\n";
        assert_eq!(super::offset_of(text, 0, 3), Some(3));
        assert_eq!(super::offset_of(text, 1, 4), Some(16));
        assert_eq!(super::offset_of(text, 2, 0), Some(23));
        assert_eq!(super::offset_of(text, 2, 1), Some(24));
        assert_eq!(super::offset_of(text, 0, 99), None);
        assert_eq!(super::offset_of(text, 9, 0), None);
    }

    #[test]
    fn breadcrumbs_name_the_containing_symbols_outermost_first() {
        let text = "mod outer {\n    fn inner() {}\n}\n";
        let outline = sources(
            text,
            vec![
                symbol("outer", Span::new(0, 32), Span::new(4, 9)),
                symbol("inner", Span::new(15, 28), Span::new(18, 23)),
            ],
        );
        assert_eq!(
            outline.breadcrumbs(20),
            vec!["outer".to_owned(), "inner".to_owned()]
        );
        assert!(outline.breadcrumbs(40).is_empty());
    }

    #[test]
    fn a_caret_column_becomes_protocol_units() {
        let text = "a😀b\nnext\n";
        assert_eq!(super::units(text, 0, 0), Some((0, 0)));
        assert_eq!(super::units(text, 0, 1), Some((0, 1)));
        // The emoji is two UTF-16 code units, so the `b` starts at unit 3.
        assert_eq!(super::units(text, 0, 2), Some((0, 3)));
        assert_eq!(super::units(text, 1, 2), Some((1, 2)));
        assert_eq!(super::units(text, 0, 9), None);
    }

    #[test]
    fn a_code_query_answer_reads_locations_and_location_links() {
        let answer = crate::project::record::json::parse(
            r#"[
                {"uri":"file:///src/a.rs","range":{"start":{"line":2,"character":5},"end":{"line":2,"character":9}}},
                {"targetUri":"file:///src/b.rs","targetSelectionRange":{"start":{"line":7,"character":1},"end":{"line":7,"character":4}},"targetRange":{"start":{"line":7,"character":0},"end":{"line":7,"character":4}}},
                {"uri":"file:///src/c.rs"}
            ]"#,
        )
        .expect("json");
        let locations = super::code_locations_from_lsp(&answer);
        assert_eq!(locations.len(), 2);
        assert_eq!(locations[0].uri, "file:///src/a.rs");
        assert_eq!((locations[0].line, locations[0].character), (2, 5));
        assert_eq!(locations[1].uri, "file:///src/b.rs");
        assert_eq!((locations[1].line, locations[1].character), (7, 1));
    }

    #[test]
    fn a_single_location_or_null_answer_is_read_too() {
        let single = crate::project::record::json::parse(
            r#"{"uri":"file:///src/a.rs","range":{"start":{"line":1,"character":2},"end":{"line":1,"character":5}}}"#,
        )
        .expect("json");
        assert_eq!(super::code_locations_from_lsp(&single).len(), 1);
        let null = crate::project::record::json::parse("null").expect("json");
        assert!(super::code_locations_from_lsp(&null).is_empty());
    }

    #[test]
    fn a_document_without_a_server_says_so() {
        let outline = SourceOutline::new(None, Vec::new());
        assert!(!outline.is_configured());
        let mut outline = outline;
        let rows = outline.render("");
        assert!(rows[0].contains("--lsp"));
        assert!(outline.target(0).is_none());
    }

    #[test]
    fn the_rows_name_the_symbols_and_their_lines() {
        let text = "// 中文\nfn main() {\n    let run = 1;\n}\n";
        let main = symbol("main", Span::new(10, 43), Span::new(13, 17));
        let mut outline = sources(text, vec![main]);
        let rows = outline.render("");
        assert!(
            rows.iter().any(|row| row == "source: src/main.rs"),
            "{rows:?}"
        );
        assert!(
            rows.iter()
                .any(|row| row.contains("node mapping:") && row.contains("no generation run")),
            "{rows:?}"
        );
        assert!(
            rows.iter()
                .any(|row| row.starts_with("main [function] 2:4")),
            "{rows:?}"
        );
        // The first row is a message; the symbol row is the one with a target.
        let index = rows
            .iter()
            .position(|row| row.starts_with("main [function]"))
            .expect("the symbol row");
        assert_eq!(outline.target(index), Some((1, 3)));
    }

    #[test]
    fn the_filter_narrows_the_rows_without_losing_their_targets() {
        let text = "fn main() {}\nfn helper() {}\n";
        let main = symbol("main", Span::new(0, 12), Span::new(3, 7));
        let helper = symbol("helper", Span::new(13, 27), Span::new(16, 22));
        let mut outline = sources(text, vec![main, helper]);
        assert_eq!(outline.render("hel").len(), 6);
        let rendered = outline.render("hel");
        assert!(
            rendered
                .iter()
                .any(|row| row.starts_with("helper [function] 2:4"))
        );
        assert!(
            !rendered
                .iter()
                .any(|row| row.starts_with("main [function]"))
        );
        let index = rendered
            .iter()
            .position(|row| row.starts_with("helper [function]"))
            .expect("the filtered row");
        assert_eq!(outline.target(index), Some((1, 3)));
    }

    #[test]
    fn region_markers_are_labelled_only_when_the_buffer_declares_them() {
        let text = "// region:generated body\nfn main() {}\n// endregion:generated body\n";
        let main = symbol("main", Span::new(25, 37), Span::new(28, 32));
        let mut outline = sources(text, vec![main]);
        let rendered = outline.render("");
        assert!(
            rendered
                .iter()
                .any(|row| row.starts_with("regions: 1 generated")),
            "{rendered:?}"
        );
        assert!(
            rendered
                .iter()
                .any(|row| row.contains("main [function]") && row.ends_with("[generated]")),
            "{rendered:?}"
        );

        // A marker-free file is not labelled generated, even though a merge treats its body as one.
        let text = "fn main() {}\n";
        let main = symbol("main", Span::new(0, 12), Span::new(3, 7));
        let mut outline = sources(text, vec![main]);
        let rendered = outline.render("");
        assert!(
            rendered
                .iter()
                .any(|row| row.contains("declares no generated/handwritten markers")),
            "{rendered:?}"
        );
        assert!(
            !rendered.iter().any(|row| row.contains("[generated]")),
            "{rendered:?}"
        );
    }

    #[test]
    fn a_column_counts_characters_not_bytes() {
        assert_eq!(line_col("中文🙂x", 10), (0, 3));
        assert_eq!(line_col("a\nbcd", 4), (1, 2));
        // An offset inside a character snaps down instead of panicking.
        assert_eq!(line_col("中", 1), (0, 0));
    }

    fn anchor(
        id: Option<&str>,
        key: &str,
        offset: usize,
        length: usize,
    ) -> rust_widgets::designer::NodeAnchor {
        rust_widgets::designer::NodeAnchor {
            path: Vec::new(),
            key: key.to_owned(),
            id: id.map(str::to_owned),
            widget: "panel".to_owned(),
            kind: "panel".to_owned(),
            line: 1,
            column: 1,
            offset,
            length,
            key_offset: offset,
            key_length: key.len(),
        }
    }

    #[test]
    fn a_generated_buffer_names_the_innermost_node_of_each_symbol() {
        let text = "fn build_ui() { let tree = Node::new(\"window\").key(\"n_0\") \
                    .child(Node::new(\"panel\").key(\"n_0_0\")); }\n";
        let root_key = text.find("n_0").expect("the root key");
        let key = text.find("n_0_0").expect("the nested key");
        let root = text
            .find("Node::new(\"window\")")
            .expect("the root expression");
        let inner = text
            .find("Node::new(\"panel\")")
            .expect("the nested expression");
        let symbols = vec![
            symbol(
                "n_0",
                Span::new(root, root_key + 3),
                Span::new(root_key, root_key + 3),
            ),
            symbol("n_0_0", Span::new(inner, key + 5), Span::new(key, key + 5)),
        ];
        let anchors = vec![
            anchor(Some("1"), "n_0", root, text.len() - root),
            anchor(None, "n_0_0", inner, key + 5 - inner),
        ];
        let (nodes, rows, mapping) = super::associate(
            &symbols,
            text,
            Some(super::Generated {
                page: "pages/main.json",
                source: text,
                anchors: &anchors,
            }),
        );
        assert_eq!(nodes.get(&root_key).map(String::as_str), Some("1"));
        assert_eq!(nodes.get(&key).map(String::as_str), Some("n_0_0"));
        assert_eq!(
            mapping,
            "node mapping: 2 anchor(s) from generating pages/main.json"
        );
        assert_eq!(rows.len(), 2);
        assert!(
            rows[0].text.starts_with("node 1 [panel] 1:"),
            "{:?}",
            rows[0].text
        );
        assert!(rows[0].target.is_some());
    }

    #[test]
    fn a_buffer_that_is_not_the_generated_text_is_not_associated() {
        let text = "fn main() {}\n";
        let main = symbol("main", Span::new(0, 12), Span::new(3, 7));
        let anchors = vec![anchor(Some("1"), "n_0", 0, 12)];
        let (nodes, rows, mapping) = super::associate(
            &[main],
            text,
            Some(super::Generated {
                page: "pages/main.json",
                source: "other text",
                anchors: &anchors,
            }),
        );
        assert!(nodes.is_empty());
        assert!(rows.is_empty());
        assert!(mapping.contains("different text"), "{mapping}");
        let (nodes, rows, mapping) = super::associate(&[], text, None);
        assert!(nodes.is_empty());
        assert!(rows.is_empty());
        assert!(mapping.contains("no generation run"), "{mapping}");
    }
}
