// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! F08: search, replace, navigation and diff.
//!
//! One command handler per parsed command of this domain; the dispatcher lives in
//! [`super`](self).

use super::*;

/// Compares two texts or trees, and shows a read-only revision (task T08.07).
pub(super) fn diff_command(options: &crate::cli::DiffOptions) -> bool {
    use crate::app::diff::{Options, Revision, Row, TreeEntry, changed_rows, compare_trees, diff};

    let compare = Options {
        ignore_line_endings: options.ignore_endings,
        ignore_trailing_whitespace: options.ignore_trailing,
    };
    let mut left = String::new();
    let mut right = String::new();
    let mut left_tree: Vec<TreeEntry> = Vec::new();
    let mut right_tree: Vec<TreeEntry> = Vec::new();
    let mut actions: Vec<&String> = Vec::new();
    for op in &options.ops {
        if let Some(text) = op.strip_prefix("left:") {
            left = unescape(text);
        } else if let Some(text) = op.strip_prefix("right:") {
            right = unescape(text);
        } else if let Some(entry) = op.strip_prefix("tree-left:") {
            match tree_entry(entry) {
                Some(entry) => left_tree.push(entry),
                None => {
                    println!("rustage: {entry:?} is not <path>=<text>");
                    return false;
                }
            }
        } else if let Some(entry) = op.strip_prefix("tree-right:") {
            match tree_entry(entry) {
                Some(entry) => right_tree.push(entry),
                None => {
                    println!("rustage: {entry:?} is not <path>=<text>");
                    return false;
                }
            }
        } else {
            actions.push(op);
        }
    }
    if let Some(path) = &options.left {
        match std::fs::read_to_string(path) {
            Ok(text) => left = text,
            Err(error) => {
                println!("rustage: cannot read {}: {error}", path.display());
                return false;
            }
        }
    }
    if let Some(path) = &options.right {
        match std::fs::read_to_string(path) {
            Ok(text) => right = text,
            Err(error) => {
                println!("rustage: cannot read {}: {error}", path.display());
                return false;
            }
        }
    }

    for op in actions {
        let (verb, _argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        match verb {
            "show" => {
                for row in diff(&left, &right, compare) {
                    println!("{}", row.render());
                    if let Row::Change { inline, .. } = &row {
                        for segment in inline {
                            println!("{}", segment.render());
                        }
                    }
                }
                println!(
                    "diff: {} changed row(s)",
                    changed_rows(&diff(&left, &right, compare))
                );
            }
            "trees" => {
                let mut changed = 0;
                for file in compare_trees(&left_tree, &right_tree, compare) {
                    println!("{}", file.render());
                    if file.kind != crate::app::diff::FileKind::Identical {
                        changed += 1;
                    }
                }
                println!("trees: {changed} file(s) differ");
            }
            "read-only" => {
                let revision = Revision::new("HEAD", right.clone());
                println!("{}", revision.render());
                match revision.edit(0, 1) {
                    Ok(()) => {
                        println!("rustage: the read-only revision accepted an edit");
                        return false;
                    }
                    Err(reason) => println!("edit: refused: {reason}"),
                }
            }
            other => {
                println!("rustage: {other:?} is not a diff operation");
                return false;
            }
        }
    }
    true
}

/// A tree entry from `<path>=<text>`.
pub(super) fn tree_entry(text: &str) -> Option<crate::app::diff::TreeEntry> {
    let (path, body) = text.split_once('=')?;
    Some(crate::app::diff::TreeEntry::new(path, unescape(body)))
}

/// Builds the project file index (task T08.01).
pub(super) fn index_command(options: &crate::cli::IndexOptions) -> bool {
    use crate::project::index::{Event, Index, Node, Rules, Tree};

    let mut rules = Rules::new();
    let mut tree = Tree::default();
    let mut built: Option<Index> = None;
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "node" => match parse_node(argument) {
                Ok(node) => {
                    let line = format!("{} {:?}", node.path, node.kind);
                    tree.nodes.push(node);
                    Ok(vec![format!("node: {line}")])
                }
                Err(reason) => Err(reason),
            },
            "ignore" => {
                rules.ignore = argument
                    .split(',')
                    .filter(|pattern| !pattern.is_empty())
                    .map(str::to_owned)
                    .collect();
                Ok(vec![format!("ignore: {argument}")])
            }
            "binary" => {
                rules.binary = argument
                    .split(',')
                    .filter(|extension| !extension.is_empty())
                    .map(str::to_owned)
                    .collect();
                Ok(vec![format!("binary: {argument}")])
            }
            "max-bytes" | "max-depth" | "max-entries" => match number(argument) {
                Ok(value) => {
                    match verb {
                        "max-bytes" => rules.max_bytes = value,
                        "max-depth" => rules.max_depth = value,
                        _ => rules.max_entries = value,
                    }
                    Ok(vec![format!("{verb}: {value}")])
                }
                Err(reason) => Err(reason),
            },
            "build" => {
                let index = Index::build(&tree, &rules);
                let summary = format!("{} entry(s)", index.entries().len());
                built = Some(index);
                Ok(vec![format!("build: {summary}")])
            }
            "create" => match parse_path_size(argument) {
                Ok((path, size)) => {
                    if let Some(index) = built.as_mut() {
                        index.apply(Event::Created(Node::file(path.clone(), size)), &rules);
                    }
                    Ok(vec![format!("create: {path} ({size})")])
                }
                Err(reason) => Err(reason),
            },
            "remove" => {
                if let Some(index) = built.as_mut() {
                    index.apply(
                        Event::Removed {
                            path: argument.to_owned(),
                        },
                        &rules,
                    );
                }
                Ok(vec![format!("remove: {argument}")])
            }
            "modify" => match parse_path_size(argument) {
                Ok((path, size)) => {
                    if let Some(index) = built.as_mut() {
                        index.apply(
                            Event::Modified {
                                path: path.clone(),
                                size,
                            },
                            &rules,
                        );
                    }
                    Ok(vec![format!("modify: {path} ({size})")])
                }
                Err(reason) => Err(reason),
            },
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not an index operation")),
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

    let index = built.unwrap_or_else(|| Index::build(&tree, &rules));
    for line in index.render() {
        println!("{line}");
    }
    true
}

/// A node from `<path>:<file|dir|symlink>:<size|target>`.
pub(super) fn parse_node(argument: &str) -> Result<crate::project::index::Node, String> {
    use crate::project::index::Node;
    let fields: Vec<&str> = argument.rsplitn(3, ':').collect();
    let [value, kind, path] = fields.as_slice() else {
        return Err(format!("{argument:?} is not <path>:<kind>:<size|target>"));
    };
    match *kind {
        "dir" => Ok(Node::dir(*path)),
        "symlink" => Ok(Node::symlink(*path, *value)),
        "file" => Ok(Node::file(*path, number(value)?)),
        other => Err(format!("{other:?} is not a node kind")),
    }
}

/// A `(path, size)` from `<path>:<size>`.
pub(super) fn parse_path_size(argument: &str) -> Result<(String, usize), String> {
    let (path, size) = argument
        .rsplit_once(':')
        .ok_or_else(|| format!("{argument:?} is not <path>:<size>"))?;
    Ok((path.to_owned(), number(size)?))
}

/// Searches file and project text (task T08.02).
pub(super) fn search(options: &crate::cli::SearchOptions) -> bool {
    use crate::app::scale::Cancellation;
    use crate::app::search::{Limits, Query, search};

    let mut query = Query::literal(String::new());
    let mut files: Vec<(String, String)> = Vec::new();
    let mut limits = Limits::new();
    let mut cancel = false;
    let mut results: Option<crate::app::search::Results> = None;
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "file" => match argument.split_once('=') {
                Some((path, text)) => {
                    files.push((path.to_owned(), unescape(text)));
                    Ok(vec![format!("file: {path}")])
                }
                None => Err(format!("{argument:?} is not <path>=<text>")),
            },
            "pattern" => {
                query.pattern = unescape(argument);
                Ok(vec![format!("pattern: {:?}", query.pattern)])
            }
            "regex" => {
                query.regex = true;
                Ok(vec!["regex".to_owned()])
            }
            "literal" => {
                query.regex = false;
                Ok(vec!["literal".to_owned()])
            }
            "no-case" => {
                query.case_sensitive = false;
                Ok(vec!["no-case".to_owned()])
            }
            "whole-word" => {
                query.whole_word = true;
                Ok(vec!["whole-word".to_owned()])
            }
            "range" => match two_numbers(argument) {
                Ok((start, end)) => {
                    query.range = Some((start, end));
                    Ok(vec![format!("range: {start}..{end}")])
                }
                Err(reason) => Err(reason),
            },
            "max" => match argument.parse::<usize>() {
                Ok(max) => {
                    limits.max_results = max;
                    Ok(vec![format!("max: {max}")])
                }
                Err(_) => Err(format!("{argument:?} is not a number")),
            },
            "cancel" => {
                cancel = true;
                Ok(vec!["cancel".to_owned()])
            }
            "search" => {
                let token = Cancellation::new();
                if cancel {
                    token.cancel();
                }
                match search(&files, &query, limits, &token) {
                    Ok(found) => {
                        let notes = found.render();
                        results = Some(found);
                        Ok(notes)
                    }
                    Err(error) => Err(error.message()),
                }
            }
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a search operation")),
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
    if let Some(found) = &results {
        println!("query: {}", query.render());
        println!("complete: {}", found.is_complete());
    }
    true
}

/// Opens files and symbols by a query (task T08.03).
pub(super) fn quick_open(options: &crate::cli::QuickOpenOptions) -> bool {
    use crate::app::language::{Capabilities, Capability};
    use crate::app::quick_open::{Candidate, QuickOpen, SymbolEntry, symbols};

    let mut open = QuickOpen::new();
    let mut entries: Vec<SymbolEntry> = Vec::new();
    let mut capabilities = Capabilities::all();
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "file" => {
                open.add(Candidate::new(argument));
                Ok(vec![format!("file: {argument}")])
            }
            "touch" => {
                open.touch(argument);
                Ok(vec![format!("touch: {argument}")])
            }
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
            "symbol" => match argument.rsplit_once(':') {
                Some((name, path)) => {
                    entries.push(SymbolEntry {
                        name: name.to_owned(),
                        path: path.to_owned(),
                    });
                    Ok(vec![format!("symbol: {name} in {path}")])
                }
                None => Err(format!("{argument:?} is not <name>:<path>")),
            },
            "open" => {
                let hits = open.open(argument, 50);
                let mut notes = vec![format!("open {argument:?}: {} hit(s)", hits.len())];
                notes.extend(hits.iter().map(|hit| format!("  {}", hit.render())));
                Ok(notes)
            }
            "file-symbols" | "workspace-symbols" => {
                match symbols(&entries, &capabilities, argument, 50) {
                    Ok(hits) => {
                        let mut notes = vec![format!("{verb} {argument:?}: {} hit(s)", hits.len())];
                        notes.extend(hits.iter().map(|hit| format!("  {}", hit.render())));
                        Ok(notes)
                    }
                    Err(refusal) => Err(refusal.message()),
                }
            }
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a quick-open operation")),
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
    println!("files known: {}", open.len());
    true
}

/// Builds replace candidates with a diff and a protection filter (task T08.04).
pub(super) fn replace(options: &crate::cli::ReplaceOptions) -> bool {
    use std::collections::BTreeMap;

    use crate::app::replace_candidates::{Candidates, parse_template};
    use crate::app::search::Query;
    use crate::project::conflict::Writability;
    use crate::project::protection::{ProtectedRegion, WriteGuard};

    let mut files: Vec<(String, String)> = Vec::new();
    let mut query = Query::literal(String::new());
    let mut template_src = String::new();
    let mut guards: BTreeMap<String, WriteGuard> = BTreeMap::new();
    let mut candidates: Option<Candidates> = None;
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "file" => match argument.split_once('=') {
                Some((path, text)) => {
                    files.push((path.to_owned(), unescape(text)));
                    Ok(vec![format!("file: {path}")])
                }
                None => Err(format!("{argument:?} is not <path>=<text>")),
            },
            "pattern" => {
                query.pattern = unescape(argument);
                Ok(vec![format!("pattern: {:?}", query.pattern)])
            }
            "regex" => {
                query.regex = true;
                Ok(vec!["regex".to_owned()])
            }
            "no-case" => {
                query.case_sensitive = false;
                Ok(vec!["no-case".to_owned()])
            }
            "whole-word" => {
                query.whole_word = true;
                Ok(vec!["whole-word".to_owned()])
            }
            "template" => {
                template_src = unescape(argument);
                Ok(vec![format!("template: {:?}", template_src)])
            }
            "protect" => match parse_uri_range(argument) {
                Ok((uri, start, end)) => {
                    let mut regions: Vec<ProtectedRegion> = guards
                        .get(&uri)
                        .map(|guard| guard.regions().to_vec())
                        .unwrap_or_default();
                    regions.push(ProtectedRegion::new(start, end));
                    guards.insert(uri.clone(), WriteGuard::new(regions, Writability::Writable));
                    Ok(vec![format!("protect {uri} {start}..{end}")])
                }
                Err(reason) => Err(reason),
            },
            "read-only" => {
                guards.insert(
                    argument.to_owned(),
                    WriteGuard::new(
                        Vec::new(),
                        Writability::ReadOnly {
                            reason: "declared read-only".to_owned(),
                        },
                    ),
                );
                Ok(vec![format!("read-only: {argument}")])
            }
            "build" => match parse_template(&template_src) {
                Ok(template) => match Candidates::build(&files, &query, &template, &guards) {
                    Ok(built) => {
                        let notes = built.render();
                        candidates = Some(built);
                        Ok(notes)
                    }
                    Err(error) => Err(error.message()),
                },
                Err(error) => Err(error.message()),
            },
            "check" | "uncheck" => {
                let on = verb == "check";
                match argument.parse::<usize>() {
                    Ok(index) => match candidates.as_mut() {
                        Some(built) => match built.check(index, on) {
                            Ok(()) => Ok(vec![format!("{verb} {index}")]),
                            Err(reason) => Err(reason),
                        },
                        None => Err("no candidates yet".to_owned()),
                    },
                    Err(_) => Err(format!("{argument:?} is not a candidate index")),
                }
            }
            "check-all" | "uncheck-all" => match candidates.as_mut() {
                Some(built) => {
                    built.check_all(verb == "check-all");
                    Ok(vec![verb.to_owned()])
                }
                None => Err("no candidates yet".to_owned()),
            },
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a replace operation")),
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
    if let Some(built) = &candidates {
        println!(
            "would apply {} candidate(s), {} blocked",
            built.checked().len(),
            built.blocked().len()
        );
    }
    true
}

/// Runs a batch replace that stops midway, resumes and undoes (task T08.05).
pub(super) fn batch(options: &crate::cli::BatchOptions) -> bool {
    use std::collections::BTreeMap;

    use crate::app::batch_replace::{Batch, BatchEdit};
    use crate::app::workspace_edit::OpenDocument;
    use crate::project::conflict::Writability;
    use crate::project::protection::{ProtectedRegion, WriteGuard};

    let mut plan: Vec<BatchEdit> = Vec::new();
    let mut documents: BTreeMap<String, OpenDocument> = BTreeMap::new();
    let mut guards: BTreeMap<String, WriteGuard> = BTreeMap::new();
    let mut batch: Option<Batch> = None;
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
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
                    let mut regions: Vec<ProtectedRegion> = guards
                        .get(&uri)
                        .map(|guard| guard.regions().to_vec())
                        .unwrap_or_default();
                    regions.push(ProtectedRegion::new(start, end));
                    guards.insert(uri.clone(), WriteGuard::new(regions, Writability::Writable));
                    Ok(vec![format!("protect {uri} {start}..{end}")])
                }
                Err(reason) => Err(reason),
            },
            "edit" => match parse_file_edit(argument) {
                Ok(file) => {
                    let line = format!("{} baseline {}", file.uri, file.baseline);
                    plan.push(BatchEdit::new(file.uri, file.baseline, file.edits));
                    Ok(vec![format!("edit: {line}")])
                }
                Err(reason) => Err(reason),
            },
            "run" => {
                let mut running = batch.take().unwrap_or_else(|| Batch::new(plan.clone()));
                running.run(&mut documents, &guards);
                let notes = running.render();
                batch = Some(running);
                Ok(notes)
            }
            "undo" => match batch.as_mut() {
                Some(running) => {
                    let restored = running.undo(&mut documents);
                    Ok(vec![format!("undo: {restored} file(s) restored")])
                }
                None => Err("no batch to undo".to_owned()),
            },
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a batch operation")),
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

/// Walks the navigation history and corrects moved files (task T08.06).
pub(super) fn history_nav(options: &crate::cli::HistoryNavOptions) -> bool {
    use std::collections::BTreeSet;

    use crate::app::history_nav::{History, Origin, resolve};

    let mut history = History::new();
    let mut vanished: BTreeSet<String> = BTreeSet::new();
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "visit" | "edit" | "problem" => match argument.rsplit_once(':') {
                Some((uri, offset)) => match offset.parse::<usize>() {
                    Ok(offset) => {
                        let origin = Origin::from_name(verb).expect("a known origin");
                        history.record(uri, offset, origin);
                        Ok(vec![format!("{verb}: {uri}:{offset}")])
                    }
                    Err(_) => Err(format!("{offset:?} is not an offset")),
                },
                None => Err(format!("{argument:?} is not <uri>:<offset>")),
            },
            "back" => match history.back() {
                Some(location) => Ok(vec![format!("back: {}", location.render())]),
                None => Ok(vec!["back: nothing".to_owned()]),
            },
            "forward" => match history.forward() {
                Some(location) => Ok(vec![format!("forward: {}", location.render())]),
                None => Ok(vec!["forward: nothing".to_owned()]),
            },
            "rename" => match argument.rsplit_once(':') {
                Some((from, to)) => {
                    let moved = history.rename(from, to);
                    if moved > 0 {
                        vanished.remove(from);
                    }
                    Ok(vec![format!("rename: {moved} entr(ies)")])
                }
                None => Err(format!("{argument:?} is not <from>:<to>")),
            },
            "vanished" => {
                vanished.insert(argument.to_owned());
                Ok(vec![format!("vanished: {argument}")])
            }
            "resolve" => match history.current() {
                Some(location) => match resolve(location, |uri| !vanished.contains(uri)) {
                    Ok(resolved) => Ok(vec![format!("resolve: {}", resolved.render())]),
                    Err(error) => Err(error.message()),
                },
                None => Err("nothing to resolve".to_owned()),
            },
            "show" => Ok(Vec::new()),
            other => Err(format!("{other:?} is not a history operation")),
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
    for line in history.render() {
        println!("{line}");
    }
    true
}

/// Runs the search/replace acceptance regression (task T08.08).
pub(super) fn search_health(options: &crate::cli::SearchHealthOptions) -> bool {
    use crate::app::search_regression::{Scenario, run};

    let scenarios = if options.scenarios.is_empty() {
        Scenario::ALL.to_vec()
    } else {
        let mut chosen = Vec::new();
        for name in &options.scenarios {
            match Scenario::from_name(name) {
                Some(scenario) => chosen.push(scenario),
                None => {
                    println!("rustage: {name:?} is not a search-health scenario");
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
