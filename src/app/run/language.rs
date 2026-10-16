// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! F07: the language service.
//!
//! One command handler per parsed command of this domain; the dispatcher lives in
//! [`super`](self).

use super::*;

/// Drives the language-service lifecycle (task T07.01).
pub(super) fn language(options: &crate::cli::LanguageOptions) -> bool {
    use std::cell::RefCell;
    use std::rc::Rc;

    use crate::app::language::{Capabilities, Capability, Launcher, Services, StartupFailure};

    /// What the command line has told the launcher to do.
    #[derive(Default)]
    struct Script {
        capabilities: Option<Capabilities>,
        failure: Option<StartupFailure>,
    }

    /// A launcher whose behaviour the command line scripts through a shared handle.
    struct Scripted {
        script: Rc<RefCell<Script>>,
    }

    impl Launcher for Scripted {
        fn launch(&mut self, _project: &str) -> Result<Capabilities, StartupFailure> {
            let script = self.script.borrow();
            match &script.failure {
                Some(failure) => Err(failure.clone()),
                None => Ok(script
                    .capabilities
                    .clone()
                    .unwrap_or_else(Capabilities::all)),
            }
        }

        fn stop(&mut self, _project: &str) {}
    }

    let script = Rc::new(RefCell::new(Script::default()));
    let mut services = Services::new(Scripted {
        script: Rc::clone(&script),
    });

    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "caps" => {
                let mut capabilities = Vec::new();
                for name in argument.split(',').filter(|name| !name.is_empty()) {
                    match Capability::from_name(name.trim()) {
                        Some(capability) => capabilities.push(capability),
                        None => return report(format!("{name:?} is not a capability")),
                    }
                }
                script.borrow_mut().failure = None;
                let capabilities = Capabilities::of(capabilities);
                let line = capabilities.render();
                script.borrow_mut().capabilities = Some(capabilities);
                Ok(vec![format!("caps: {line}")])
            }
            "fail" => {
                let failure = match argument {
                    "toolchain" => StartupFailure::NoToolchain,
                    "spawn" => StartupFailure::SpawnFailed("executable not found".to_owned()),
                    "handshake" => {
                        StartupFailure::HandshakeFailed("initialize timed out".to_owned())
                    }
                    other => return report(format!("{other:?} is not a failure kind")),
                };
                script.borrow_mut().failure = Some(failure);
                Ok(vec![format!("fail: {argument}")])
            }
            "view" => {
                let session = services.open_view(argument);
                let line = session.render();
                Ok(vec![format!("view: {line}")])
            }
            "restart" => {
                let session = services.restart(argument);
                let line = session.render();
                let restarts = session.restarts();
                Ok(vec![format!("restart: {line} (restarts {restarts})")])
            }
            "close" => {
                let released = services.close(argument);
                Ok(vec![format!("close {argument}: released={released}")])
            }
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a language operation")),
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

    for line in services.render() {
        println!("{line}");
    }
    true
}

/// Synchronises documents and converts positions (task T07.02).
pub(super) fn protocol(options: &crate::cli::ProtocolOptions) -> bool {
    use crate::app::protocol::{Sync, to_offset, to_units};

    let mut sync = Sync::new();
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "open" => match argument.split_once('=') {
                Some((uri, text)) => {
                    let version = sync.open(uri, unescape(text));
                    Ok(vec![format!("open {uri}: version {version}")])
                }
                None => Err(format!("{argument:?} is not <uri>=<text>")),
            },
            "replace" => match argument.split_once('=') {
                Some((uri, text)) => match sync.replace(uri, unescape(text)) {
                    Ok(version) => Ok(vec![format!("replace {uri}: version {version}")]),
                    Err(error) => Err(error.message()),
                },
                None => Err(format!("{argument:?} is not <uri>=<text>")),
            },
            "edit" => match parse_change(argument) {
                Ok((uri, version, change)) => match sync.change(&uri, version, &[change]) {
                    Ok(version) => Ok(vec![format!("edit {uri}: version {version}")]),
                    Err(error) => Err(error.message()),
                },
                Err(reason) => Err(reason),
            },
            "save" => match sync.save(argument) {
                Ok(version) => Ok(vec![format!("save {argument}: version {version}")]),
                Err(error) => Err(error.message()),
            },
            "close" => match sync.close(argument) {
                Ok(()) => Ok(vec![format!("close {argument}")]),
                Err(error) => Err(error.message()),
            },
            "units" => match argument.rsplit_once(':') {
                Some((uri, byte)) => match byte.parse::<usize>() {
                    Ok(byte) => match sync.document(uri) {
                        Some(document) => {
                            let position = to_units(document.text(), byte);
                            Ok(vec![format!("units {byte} -> {}", position.render())])
                        }
                        None => Err(format!("{uri} is not open")),
                    },
                    Err(_) => Err(format!("{byte:?} is not a byte offset")),
                },
                None => Err(format!("{argument:?} is not <uri>:<byte>")),
            },
            "offset" => match argument.rsplit_once(':') {
                Some((uri, position)) => match parse_position(position) {
                    Ok(position) => match sync.document(uri) {
                        Some(document) => match to_offset(document.text(), position) {
                            Some(offset) => {
                                Ok(vec![format!("offset {} -> {offset}", position.render())])
                            }
                            None => Ok(vec![format!(
                                "offset {} -> no such line",
                                position.render()
                            )]),
                        },
                        None => Err(format!("{uri} is not open")),
                    },
                    Err(reason) => Err(reason),
                },
                None => Err(format!("{argument:?} is not <uri>:<line>,<char>")),
            },
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a protocol operation")),
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

    for line in sync.render() {
        println!("{line}");
    }
    for uri in sync.open_documents() {
        if let Some(document) = sync.document(uri) {
            println!("  text {uri}: {:?}", document.text());
        }
    }
    true
}

/// A change from `<uri>:<version>:<sl>,<sc>:<el>,<ec>=<text>`.
pub(super) fn parse_change(
    argument: &str,
) -> Result<(String, u64, crate::app::protocol::Change), String> {
    use crate::app::protocol::{Change, Range};
    let (spec, text) = argument
        .split_once('=')
        .ok_or_else(|| format!("{argument:?} has no =<text>"))?;
    // A uri contains colons, so the last three colon-separated fields are version, start and end.
    let fields: Vec<&str> = spec.rsplitn(3, ':').collect();
    let [end, start, version_and_uri] = fields.as_slice() else {
        return Err(format!(
            "{spec:?} is not <uri>:<version>:<sl>,<sc>:<el>,<ec>"
        ));
    };
    let (uri, version) = version_and_uri
        .rsplit_once(':')
        .ok_or_else(|| format!("{version_and_uri:?} has no version"))?;
    let version: u64 = version
        .parse()
        .map_err(|_| format!("{version:?} is not a version"))?;
    let start = parse_position(start)?;
    let end = parse_position(end)?;
    Ok((
        uri.to_owned(),
        version,
        Change::new(Range::new(start, end), unescape(text)),
    ))
}

/// A position from `<line>,<char>`.
pub(super) fn parse_position(text: &str) -> Result<crate::app::protocol::Position, String> {
    let (line, character) = text
        .split_once(',')
        .ok_or_else(|| format!("{text:?} is not <line>,<char>"))?;
    let line: u32 = line
        .parse()
        .map_err(|_| format!("{line:?} is not a line"))?;
    let character: u32 = character
        .parse()
        .map_err(|_| format!("{character:?} is not a character"))?;
    Ok(crate::app::protocol::Position::new(line, character))
}

/// Drives completion freshness (task T07.03).
pub(super) fn features(options: &crate::cli::FeaturesOptions) -> bool {
    use crate::app::features::{CompletionItem, Completions, Delivery, Response};

    let mut completions = Completions::new(1);
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "version" => match argument.parse::<u64>() {
                Ok(version) => {
                    completions.advance(version);
                    Ok(vec![format!("version: {version}")])
                }
                Err(_) => Err(format!("{argument:?} is not a version")),
            },
            "request" => {
                let request = completions.request();
                Ok(vec![format!(
                    "request {} against version {}",
                    request.id, request.version
                )])
            }
            "cancel" => match argument.parse::<u64>() {
                Ok(id) => {
                    let cancelled = completions.cancel(id);
                    Ok(vec![format!("cancel {id}: {cancelled}")])
                }
                Err(_) => Err(format!("{argument:?} is not a request id")),
            },
            "respond" => match parse_response(argument) {
                Ok((request, labels)) => {
                    let items: Vec<CompletionItem> =
                        labels.iter().map(CompletionItem::text).collect();
                    match completions.accept(Response::new(request, items)) {
                        Delivery::Fresh(count) => {
                            Ok(vec![format!("respond: fresh, {count} item(s)")])
                        }
                        Delivery::Stale { computed, current } => Ok(vec![format!(
                            "respond: stale (computed {computed}, current {current})"
                        )]),
                        Delivery::Cancelled => Ok(vec!["respond: cancelled".to_owned()]),
                    }
                }
                Err(reason) => Err(reason),
            },
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a feature operation")),
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

    for line in completions.render() {
        println!("{line}");
    }
    true
}

/// A response from `<id>:<version>:<label>,<label>`.
pub(super) fn parse_response(
    argument: &str,
) -> Result<(crate::app::features::Request, Vec<String>), String> {
    use crate::app::features::Request;
    let (id, rest) = argument
        .split_once(':')
        .ok_or_else(|| format!("{argument:?} has no version"))?;
    let (version, labels) = rest
        .split_once(':')
        .ok_or_else(|| format!("{rest:?} has no labels"))?;
    let id: u64 = id
        .parse()
        .map_err(|_| format!("{id:?} is not a request id"))?;
    let version: u64 = version
        .parse()
        .map_err(|_| format!("{version:?} is not a version"))?;
    let labels = labels
        .split(',')
        .filter(|label| !label.is_empty())
        .map(str::to_owned)
        .collect();
    Ok((Request { id, version }, labels))
}

/// Answers navigation queries and shows the outline (task T07.04).
pub(super) fn navigation(options: &crate::cli::NavigationOptions) -> bool {
    use crate::app::language::{Capabilities, Capability};
    use crate::app::navigation::{Navigation, Query, Target};

    let mut capabilities = Capabilities::all();
    let mut nav = Navigation::new(capabilities.clone());
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "caps" => {
                let mut chosen = Vec::new();
                for name in argument.split(',').filter(|name| !name.is_empty()) {
                    match Capability::from_name(name.trim()) {
                        Some(capability) => chosen.push(capability),
                        None => return report(format!("{name:?} is not a capability")),
                    }
                }
                capabilities = Capabilities::of(chosen);
                nav = Navigation::new(capabilities.clone());
                Ok(vec![format!("caps: {}", capabilities.render())])
            }
            "symbol" => match parse_symbol(argument) {
                Ok(symbol) => {
                    let line = format!(
                        "{} {} [{}] {}",
                        symbol.uri,
                        symbol.name,
                        symbol.kind.name(),
                        symbol.span.render()
                    );
                    nav.add(symbol);
                    Ok(vec![format!("symbol: {line}")])
                }
                Err(reason) => Err(reason),
            },
            "definition" | "references" | "declaration" | "implementation" => {
                let Some(query) = Query::from_name(verb) else {
                    return report(format!("{verb:?} is not a query"));
                };
                match nav.query(query, &Target::Symbol(argument.to_owned())) {
                    Ok(locations) => {
                        let mut notes = vec![format!(
                            "{verb} {argument}: {} location(s)",
                            locations.len()
                        )];
                        notes.extend(
                            locations
                                .iter()
                                .map(|location| format!("  {}", location.render())),
                        );
                        Ok(notes)
                    }
                    Err(refusal) => Err(refusal.message()),
                }
            }
            "document-symbol" | "workspace-symbol" => {
                let Some(query) = Query::from_name(verb) else {
                    return report(format!("{verb:?} is not a query"));
                };
                match nav.query(query, &Target::Symbol(argument.to_owned())) {
                    Ok(locations) => Ok(vec![format!("{verb}: {} location(s)", locations.len())]),
                    Err(refusal) => Err(refusal.message()),
                }
            }
            "outline" => {
                let mut notes = vec![format!("outline {argument}:")];
                for node in nav.outline(argument) {
                    notes.extend(node.render(1));
                }
                Ok(notes)
            }
            "breadcrumbs" => match argument.rsplit_once(':') {
                Some((uri, offset)) => match offset.parse::<usize>() {
                    Ok(offset) => {
                        let crumbs = nav.breadcrumbs(uri, offset);
                        Ok(vec![format!(
                            "breadcrumbs {uri} at {offset}: {}",
                            crumbs.join(" > ")
                        )])
                    }
                    Err(_) => Err(format!("{offset:?} is not an offset")),
                },
                None => Err(format!("{argument:?} is not <uri>:<offset>")),
            },
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a navigation operation")),
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
    true
}

/// A symbol from `<uri>:<name>:<kind>:<start>:<end>:<name_start>:<name_end>`.
pub(super) fn parse_symbol(argument: &str) -> Result<crate::app::navigation::Symbol, String> {
    use crate::app::navigation::{Span, Symbol, SymbolKind};
    // The uri has colons, so the last six fields are name, kind, start, end, name_start, name_end.
    let fields: Vec<&str> = argument.rsplitn(6, ':').collect();
    let [name_end, name_start, end, start, kind, name_and_uri] = fields.as_slice() else {
        return Err(format!(
            "{argument:?} is not <uri>:<name>:<kind>:<s>:<e>:<ns>:<ne>"
        ));
    };
    let (uri, name) = name_and_uri
        .rsplit_once(':')
        .ok_or_else(|| format!("{name_and_uri:?} has no name"))?;
    let kind =
        SymbolKind::from_name(kind).ok_or_else(|| format!("{kind:?} is not a symbol kind"))?;
    let number = |text: &str| -> Result<usize, String> {
        text.parse()
            .map_err(|_| format!("{text:?} is not an offset"))
    };
    Ok(Symbol::new(
        uri,
        name,
        kind,
        Span::new(number(start)?, number(end)?),
        Span::new(number(name_start)?, number(name_end)?),
    ))
}

/// Drives diagnostics, tokens and guarded code actions (task T07.05).
pub(super) fn diagnostics(options: &crate::cli::DiagnosticsOptions) -> bool {
    use crate::app::diagnostics::{
        CodeAction, Diagnostic, DiagnosticSet, Diagnostics, SemanticToken, SemanticTokens, Tokens,
        apply_action,
    };
    use crate::app::features::{Delivery, Response};
    use crate::project::conflict::Writability;
    use crate::project::protection::{ProtectedRegion, WriteGuard};

    let mut diagnostics = Diagnostics::new(1);
    let mut tokens = Tokens::new(1);
    let mut regions: Vec<ProtectedRegion> = Vec::new();
    let mut buffer: Vec<Diagnostic> = Vec::new();
    let mut token_buffer: Vec<SemanticToken> = Vec::new();
    let mut action: Option<CodeAction> = None;

    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "version" => match argument.parse::<u64>() {
                Ok(version) => {
                    diagnostics.advance(version);
                    tokens.advance(version);
                    Ok(vec![format!("version: {version}")])
                }
                Err(_) => Err(format!("{argument:?} is not a version")),
            },
            "diag" => match parse_diagnostic(argument) {
                Ok(diagnostic) => {
                    buffer.push(diagnostic);
                    Ok(vec![format!("diag: buffered {}", buffer.len())])
                }
                Err(reason) => Err(reason),
            },
            "publish" => match argument.parse::<u64>() {
                Ok(version) => {
                    let request = diagnostics.request_at(version);
                    let items = std::mem::take(&mut buffer);
                    match diagnostics.accept(Response::new(request, DiagnosticSet::new(items))) {
                        Delivery::Fresh(count) => Ok(vec![format!("publish: fresh, {count}")]),
                        Delivery::Stale { computed, current } => Ok(vec![format!(
                            "publish: stale (computed {computed}, current {current})"
                        )]),
                        Delivery::Cancelled => Ok(vec!["publish: cancelled".to_owned()]),
                    }
                }
                Err(_) => Err(format!("{argument:?} is not a version")),
            },
            "protect" => match two_numbers(argument) {
                Ok((start, end)) => {
                    regions.push(ProtectedRegion::new(start, end));
                    Ok(vec![format!("protect: {start}..{end}")])
                }
                Err(reason) => Err(reason),
            },
            "mark" => match diagnostics.shown() {
                Some(set) => {
                    let mut set = set.clone();
                    set.mark_generated(&regions);
                    let generated = set.generated().len();
                    let handwritten = set.handwritten().len();
                    Ok(vec![format!(
                        "mark: {generated} generated, {handwritten} handwritten"
                    )])
                }
                None => Err("no diagnostics to mark".to_owned()),
            },
            "token" => match parse_token(argument) {
                Ok(token) => {
                    token_buffer.push(token);
                    Ok(vec![format!("token: buffered {}", token_buffer.len())])
                }
                Err(reason) => Err(reason),
            },
            "tokens" => match argument.parse::<u64>() {
                Ok(version) => {
                    let request = tokens.request_at(version);
                    let layer = SemanticTokens::new(std::mem::take(&mut token_buffer));
                    match tokens.accept(Response::new(request, layer)) {
                        Delivery::Fresh(count) => Ok(vec![format!("tokens: fresh, {count}")]),
                        Delivery::Stale { computed, current } => Ok(vec![format!(
                            "tokens: stale (computed {computed}, current {current})"
                        )]),
                        Delivery::Cancelled => Ok(vec!["tokens: cancelled".to_owned()]),
                    }
                }
                Err(_) => Err(format!("{argument:?} is not a version")),
            },
            "action" => match parse_action(argument) {
                Ok(parsed) => {
                    let line = parsed.render();
                    action = Some(parsed);
                    Ok(vec![format!("action: {line}")])
                }
                Err(reason) => Err(reason),
            },
            "apply" => match &action {
                Some(action) => {
                    let guard = WriteGuard::new(regions.clone(), Writability::Writable);
                    match apply_action(action, &guard) {
                        Ok(edits) => Ok(vec![format!("apply: {} edit(s) allowed", edits.len())]),
                        Err(refusal) => Err(refusal.message()),
                    }
                }
                None => Err("no action to apply".to_owned()),
            },
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a diagnostics operation")),
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

    for line in diagnostics.render() {
        println!("{line}");
    }
    true
}

/// Reviews and commits a cross-file edit as one transaction (task T07.06).
pub(super) fn workspace_edit(options: &crate::cli::WorkspaceEditOptions) -> bool {
    use std::collections::BTreeMap;

    use crate::app::workspace_edit::{OpenDocument, WorkspaceEdit, commit, review};
    use crate::project::conflict::Writability;
    use crate::project::protection::{ProtectedRegion, WriteGuard};

    let mut edit = WorkspaceEdit::new("edit");
    let mut documents: BTreeMap<String, OpenDocument> = BTreeMap::new();
    let mut guards: BTreeMap<String, WriteGuard> = BTreeMap::new();

    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "label" => {
                edit.label = argument.to_owned();
                Ok(vec![format!("label: {argument}")])
            }
            "open" => match argument.split_once('=') {
                Some((spec, text)) => match spec.rsplit_once(':') {
                    Some((uri, version)) => match version.parse::<u64>() {
                        Ok(version) => {
                            documents
                                .insert(uri.to_owned(), OpenDocument::new(unescape(text), version));
                            Ok(vec![format!("open {uri}: version {version}")])
                        }
                        Err(_) => Err(format!("{version:?} is not a version")),
                    },
                    None => Err(format!("{spec:?} is not <uri>:<version>")),
                },
                None => Err(format!("{argument:?} is not <uri>:<version>=<text>")),
            },
            "protect" => match parse_uri_range(argument) {
                Ok((uri, start, end)) => {
                    let guard = guards
                        .entry(uri.clone())
                        .or_insert_with(|| WriteGuard::new(Vec::new(), Writability::Writable));
                    let mut regions: Vec<ProtectedRegion> = guard.regions().to_vec();
                    regions.push(ProtectedRegion::new(start, end));
                    guards.insert(uri.clone(), WriteGuard::new(regions, Writability::Writable));
                    Ok(vec![format!("protect {uri} {start}..{end}")])
                }
                Err(reason) => Err(reason),
            },
            "edit" => match parse_file_edit(argument) {
                Ok(file) => {
                    let line = format!("{} baseline {}", file.uri, file.baseline);
                    edit.add(file);
                    Ok(vec![format!("edit: {line}")])
                }
                Err(reason) => Err(reason),
            },
            "review" => {
                let outcome = review(&edit, &documents, &guards);
                Ok(outcome.render())
            }
            "commit" => match commit(&edit, &mut documents, &guards) {
                Ok(applied) => {
                    let mut notes = vec![format!("commit: {} file(s) changed", applied.len())];
                    notes.extend(applied.iter().map(|file| format!("  {}", file.render())));
                    Ok(notes)
                }
                Err(blocks) => {
                    let mut notes = vec![format!(
                        "commit: blocked, nothing applied ({} block(s))",
                        blocks.len()
                    )];
                    notes.extend(blocks.iter().map(|block| format!("  {}", block.message())));
                    Ok(notes)
                }
            },
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a workspace-edit operation")),
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

    for (uri, document) in &documents {
        println!("  {uri}: {:?} (v{})", document.text, document.version);
    }
    true
}

/// Shows schema documentation and target-relative completion (task T07.07).
pub(super) fn schema(options: &crate::cli::SchemaOptions) -> bool {
    use crate::app::highlight::Language;
    use crate::app::schema_docs::{Catalog, Target};

    let mut catalog = Catalog::new(Language::Css);
    let mut target = Target::new("default", Vec::new());
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "lang" => match Language::from_name(argument) {
                Some(language) => {
                    catalog.set_language(language);
                    Ok(vec![format!("lang: {}", language.name())])
                }
                None => Err(format!("{argument:?} is not a language")),
            },
            "entry" => match parse_entry(argument) {
                Ok(entry) => {
                    let line = entry.render();
                    catalog.add(entry);
                    Ok(vec![format!("entry: {line}")])
                }
                Err(reason) => Err(reason),
            },
            "target" => match argument.split_once(':') {
                Some((name, caps)) => {
                    let available: Vec<String> = caps
                        .split(',')
                        .filter(|cap| !cap.is_empty())
                        .map(str::to_owned)
                        .collect();
                    target = Target::new(name, available);
                    Ok(vec![format!("target: {}", target.render())])
                }
                None => Err(format!("{argument:?} is not <name>:<caps>")),
            },
            "complete" => {
                let items = catalog.completions(argument, &target);
                let mut notes = vec![format!(
                    "complete {argument:?}: {} offered, {} forbidden",
                    items.len(),
                    catalog.filtered(argument, &target).len()
                )];
                notes.extend(items.iter().map(|item| format!("  {}", item.render())));
                for entry in catalog.filtered(argument, &target) {
                    notes.push(format!("  forbidden: {}", entry.render()));
                }
                Ok(notes)
            }
            "doc" => match catalog.documentation(argument) {
                Some(entry) => Ok(vec![format!("doc {}: {}", entry.key, entry.documentation)]),
                None => Err(format!("no documentation for {argument:?}")),
            },
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a schema operation")),
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

    for line in catalog.render("", &target) {
        println!("{line}");
    }
    true
}

/// Runs the language-service failure regression (task T07.08).
pub(super) fn health(options: &crate::cli::HealthOptions) -> bool {
    use crate::app::service_health::{Scenario, run};

    let scenarios = if options.scenarios.is_empty() {
        Scenario::ALL.to_vec()
    } else {
        let mut chosen = Vec::new();
        for name in &options.scenarios {
            match Scenario::from_name(name) {
                Some(scenario) => chosen.push(scenario),
                None => {
                    println!("rustage: {name:?} is not a health scenario");
                    return false;
                }
            }
        }
        chosen
    };

    let mut ok = true;
    for scenario in scenarios {
        let report = run(scenario);
        for line in report.render() {
            println!("{line}");
        }
        ok &= report.passed();
    }
    ok
}

/// Shows hints, lenses and hierarchies (task T07.09).
pub(super) fn hints(options: &crate::cli::HintsOptions) -> bool {
    use crate::app::features::{Delivery, Response};
    use crate::app::hints::{HierarchyKind, HierarchyNode, Hint, HintKind, Hints, hierarchy};
    use crate::app::language::{Capabilities, Capability};

    let mut capabilities = Capabilities::all();
    let mut layer = Hints::new(1);
    let mut buffer: Vec<Hint> = Vec::new();
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "caps" => {
                let mut chosen = Vec::new();
                for name in argument.split(',').filter(|name| !name.is_empty()) {
                    match Capability::from_name(name.trim()) {
                        Some(capability) => chosen.push(capability),
                        None => return report(format!("{name:?} is not a capability")),
                    }
                }
                capabilities = Capabilities::of(chosen);
                Ok(vec![format!("caps: {}", capabilities.render())])
            }
            "version" => match argument.parse::<u64>() {
                Ok(version) => {
                    layer.advance(version);
                    Ok(vec![format!("version: {version}")])
                }
                Err(_) => Err(format!("{argument:?} is not a version")),
            },
            "inlay" | "lens" => match parse_hint(verb, argument) {
                Ok(hint) => {
                    buffer.push(hint);
                    Ok(vec![format!("{verb}: buffered {}", buffer.len())])
                }
                Err(reason) => Err(reason),
            },
            "publish" => match argument.parse::<u64>() {
                Ok(version) => {
                    let request = layer.request_at(version);
                    match layer.accept(Response::new(request, std::mem::take(&mut buffer))) {
                        Delivery::Fresh(count) => Ok(vec![format!("publish: fresh, {count}")]),
                        Delivery::Stale { computed, current } => Ok(vec![format!(
                            "publish: stale (computed {computed}, current {current})"
                        )]),
                        Delivery::Cancelled => Ok(vec!["publish: cancelled".to_owned()]),
                    }
                }
                Err(_) => Err(format!("{argument:?} is not a version")),
            },
            "dismiss" => match HintKind::from_name(argument) {
                Some(kind) => {
                    let on = !layer.switches().is_on(kind);
                    layer.dismiss(kind, on);
                    Ok(vec![format!("dismiss {}: on={on}", kind.name())])
                }
                None => Err(format!("{argument:?} is not a hint kind")),
            },
            "invoke" => match layer.invoke(argument) {
                Ok(command) => Ok(vec![format!("invoke: ran {}", command.render())]),
                Err(reason) => Err(reason),
            },
            "hierarchy" => match argument.split_once(':') {
                Some((kind, name)) => match HierarchyKind::from_name(kind) {
                    Some(kind) => {
                        let root = HierarchyNode::new(name);
                        match hierarchy(kind, &capabilities, root) {
                            Ok(root) => {
                                let mut notes = vec![format!("hierarchy {}: {name}", kind.name())];
                                notes.extend(root.render(1));
                                Ok(notes)
                            }
                            Err(refusal) => Err(refusal.message()),
                        }
                    }
                    None => Err(format!("{kind:?} is not a hierarchy kind")),
                },
                None => Err(format!("{argument:?} is not <kind>:<name>")),
            },
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a hints operation")),
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

    for line in layer.render() {
        println!("{line}");
    }
    true
}

/// A hint from `inlay`'s `<s>:<e>:<label>` or `lens`'s `<s>:<e>:<label>:<id>:<title>`.
pub(super) fn parse_hint(verb: &str, argument: &str) -> Result<crate::app::hints::Hint, String> {
    use crate::app::hints::{Hint, LensCommand};
    use crate::app::navigation::Span;
    if verb == "inlay" {
        let fields: Vec<&str> = argument.splitn(3, ':').collect();
        let [start, end, label] = fields.as_slice() else {
            return Err(format!("{argument:?} is not <s>:<e>:<label>"));
        };
        let span = Span::new(number(start)?, number(end)?);
        return Ok(Hint::inlay(span, unescape(label)));
    }
    let fields: Vec<&str> = argument.splitn(5, ':').collect();
    let [start, end, label, id, title] = fields.as_slice() else {
        return Err(format!("{argument:?} is not <s>:<e>:<label>:<id>:<title>"));
    };
    Ok(Hint::lens(
        Span::new(number(start)?, number(end)?),
        unescape(label),
        LensCommand::new(*id, unescape(title)),
    ))
}

/// Rebuilds the analysis configuration after a member/feature/target/Kit switch (task T07.10).
pub(super) fn analysis(options: &crate::cli::AnalysisOptions) -> bool {
    use crate::app::analysis_config::{Analysis, Selection, Trust};
    use crate::app::diagnostics::DiagnosticSet;
    use crate::app::features::{Delivery, Response};

    let mut analysis = Analysis::new();
    let mut selection = Selection::new("app", "x86_64-unknown-linux-gnu");
    let mut trust = Trust::Trusted;
    let mut known: Vec<String> = vec!["x86_64-unknown-linux-gnu".to_owned()];
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "target" => {
                selection.target = argument.to_owned();
                Ok(vec![format!("target: {argument}")])
            }
            "known" => {
                known.push(argument.to_owned());
                Ok(vec![format!("known: {argument}")])
            }
            "member" => {
                selection.member = argument.to_owned();
                Ok(vec![format!("member: {argument}")])
            }
            "feature" => {
                selection.features.insert(argument.to_owned());
                Ok(vec![format!("feature: {argument}")])
            }
            "kit" => {
                selection.kit = Some(argument.to_owned());
                Ok(vec![format!("kit: {argument}")])
            }
            "trust" => match Trust::from_name(argument) {
                Some(chosen) => {
                    trust = chosen;
                    Ok(vec![format!("trust: {}", trust.name())])
                }
                None => Err(format!("{argument:?} is not a trust")),
            },
            "configure" => {
                let config = analysis.configure(selection.clone(), trust, &known);
                Ok(vec![format!("configure: {}", config.render())])
            }
            "publish" => match argument.parse::<u64>() {
                Ok(generation) => {
                    let request = analysis.request_at(generation);
                    match analysis.accept(Response::new(request, DiagnosticSet::default())) {
                        Delivery::Fresh(count) => Ok(vec![format!("publish: fresh, {count}")]),
                        Delivery::Stale { computed, current } => Ok(vec![format!(
                            "publish: stale (computed {computed}, current {current})"
                        )]),
                        Delivery::Cancelled => Ok(vec!["publish: cancelled".to_owned()]),
                    }
                }
                Err(_) => Err(format!("{argument:?} is not a generation")),
            },
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not an analysis operation")),
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

    for line in analysis.render() {
        println!("{line}");
    }
    true
}
