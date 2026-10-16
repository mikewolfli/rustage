// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! F03: the document — transactions, saving, history and recovery.
//!
//! One command handler per parsed command of this domain; the dispatcher lives in
//! [`super`](self).

use super::*;

/// Shows what a buffer over a page document is holding (task T03.01).
///
/// Nothing is written and nothing is run, so there is no question to ask and no permission to take:
/// the command reads the file, types the edits in the order they were given, and reports. What it
/// reports is the whole point of the task — the text as it was typed (an invalid text is kept), the
/// last model that read (still previewable, and saying which text it came from), and what happened
/// to identity. The exit code says whether the newest text reads; a text that does not read is a
/// finding, not a crash, and the whole report is still printed.
pub(super) fn document(options: &crate::cli::DocumentOptions) -> bool {
    use crate::project::document::buffer::{Buffer, root_of};

    let root = match options.root.as_ref() {
        Some(directory) => match crate::host::paths::ProjectRoot::new(directory) {
            Ok(root) => root,
            Err(error) => {
                println!("rustage: {}", error.message());
                return false;
            }
        },
        None => match root_of(&options.file) {
            Ok(root) => root,
            Err(refusal) => {
                println!("rustage: {}", refusal.message());
                return false;
            }
        },
    };
    let relative = relative_to(&root, &options.file);

    let mut buffer = match Buffer::open(&root, &relative) {
        Ok(buffer) => buffer,
        Err(refusal) => {
            println!("rustage: {}", refusal.message());
            return false;
        }
    };
    println!("document: read only; nothing is written and no question is asked");
    for edit in &options.edits {
        let text = match edit {
            crate::cli::DocumentEdit::Text(text) => text.clone(),
            crate::cli::DocumentEdit::File(path) => match std::fs::read_to_string(path) {
                Ok(text) => text,
                Err(error) => {
                    println!("rustage: {} could not be read: {error}", path.display());
                    return false;
                }
            },
        };
        buffer.set_text(&root, text);
    }
    for line in buffer.render() {
        println!("{line}");
    }
    buffer.state().is_current()
}

/// Replays a session of edits against a page document, then undoes or redoes them (task T03.03).
///
/// Nothing is written to any file and nothing is run, so there is no question to ask and no
/// permission to take: the document is read once, the edits happen to the buffer and to the model
/// **in memory**, and the history records what happened. The point is the undo rule, which is a rule
/// about domains rather than about time:
///
/// * One document has one history, and every entry in it is in one of two domains — the text or the
///   model. The entries are in the order things happened whatever proposed them, so a panel edit, a
///   canvas edit and a paste are one sequence. A view is an attribute of an entry, never a separate
///   history, because undoing "the canvas's last edit" could then skip the edit that happened after
///   it.
/// * A step is undone in its own domain, and the two never convert: text is restored to the text
///   that was there, and the model is moved by a change set. So an undo cannot turn into the other
///   kind of edit and cannot loop.
/// * `--only` asks for a domain explicitly, and a step in the other domain is **refused** with the
///   newest step named rather than skipped, because quietly undoing the change *behind* somebody's
///   typing would leave the text as nobody typed it.
/// * `--merge` makes every batch in the run one step: that is what a drag is — several change sets
///   applied as one gesture, undone as one step and redone as the whole gesture again.
///
/// Writing the model back to the file is T03.04's subject, which is why the model and the text are
/// reported separately here: the batch landed on the model and the text of the buffer was not
/// rewritten, and saying so is better than pretending the two are one thing before the writing path
/// exists.
pub(super) fn history(options: &crate::cli::HistoryOptions) -> bool {
    use crate::cli::HistoryEdit;
    use crate::contracts::permission::PermissionGate;
    use crate::project::document::batch::{Batch, Fingerprint, Plan, apply, store_of};
    use crate::project::document::buffer::Buffer;
    use crate::project::document::history::{Domain, History, View, batch_edit_of, text_edit_of};

    let root = match options.root.as_ref() {
        Some(directory) => match crate::host::paths::ProjectRoot::new(directory) {
            Ok(root) => root,
            Err(error) => {
                println!("rustage: {}", error.message());
                return false;
            }
        },
        None => match crate::project::document::buffer::root_of(&options.file) {
            Ok(root) => root,
            Err(refusal) => {
                println!("rustage: {}", refusal.message());
                return false;
            }
        },
    };
    let relative = relative_to(&root, &options.file);

    let mut buffer = match Buffer::open(&root, &relative) {
        Ok(buffer) => buffer,
        Err(refusal) => {
            println!("rustage: {}", refusal.message());
            return false;
        }
    };
    // The model the text read, as the editable store: the same seam `--edit` uses, taken from the
    // read the buffer already did rather than from a second read of the file, so the model being
    // edited is the one this text produced.
    let Some(model) = buffer.model() else {
        println!(
            "rustage: the text of {} does not read, so there is no model to edit",
            options.file.display()
        );
        return false;
    };
    let mut store = match store_of(model.document(), PermissionGate::new()) {
        Ok(store) => store,
        Err(refusal) => {
            println!("rustage: {}", refusal.message());
            return false;
        }
    };
    let mut history = History::for_document(&store);
    println!("history: in memory; nothing is written to any file, and no question is asked");
    println!(
        "document {}: page {}, revision {}, {} node(s)",
        relative.display(),
        history.page().get(),
        store.revision(),
        store.nodes().count()
    );

    let mut moved = true;
    let mut edits = 0_usize;
    let mut text_edits = 0_usize;
    let mut batches = 0_usize;
    let mut stopped = None;
    for edit in &options.edits {
        edits += 1;
        match edit {
            HistoryEdit::Text(_) | HistoryEdit::File(_) => {
                let text = match edit {
                    HistoryEdit::Text(text) => text.clone(),
                    HistoryEdit::File(path) => match std::fs::read_to_string(path) {
                        Ok(text) => text,
                        Err(error) => {
                            println!("rustage: {} could not be read: {error}", path.display());
                            return false;
                        }
                    },
                    _ => unreachable!("the arm above matched"),
                };
                text_edits += 1;
                let intent = format!("text edit {text_edits}");
                let before = buffer.text().to_owned();
                let was = buffer.typed();
                buffer.set_text(&root, text);
                let Some(entry) = text_edit_of(View::Text, &intent, &before, &buffer, was) else {
                    // The version only stays put when nothing was typed at all, which cannot happen
                    // here: `set_text` always moves it. Refusing is better than recording an edit
                    // that did not happen.
                    println!("rustage: `{intent}` changed nothing, so nothing was recorded");
                    continue;
                };
                let state = buffer.state().render();
                println!(
                    "edit {edits}: {} — {}",
                    entry.describe(),
                    state.first().map(String::as_str).unwrap_or("read")
                );
                report_recording(&history.record(entry));
            }
            HistoryEdit::Batch(path) => {
                if options.merge && batches == 0 {
                    history.begin_merge(View::Canvas, Domain::Model, "one drag, applied at once");
                    println!(
                        "a merge window is open: every batch in this run is one step, and an edit in \
                         the other domain ends it"
                    );
                }
                batches += 1;
                let batch = match Batch::read(path) {
                    Ok(batch) => batch,
                    Err(refusal) => {
                        println!("rustage: {}", refusal.message());
                        return false;
                    }
                };
                let plan = match Plan::of(&batch, &store, &root) {
                    Ok(plan) => plan,
                    Err(refusal) => {
                        println!("rustage: {}", refusal.message());
                        return false;
                    }
                };
                if plan.is_blocked() {
                    // A blocked batch is a finding, not a crash: the plan says what cannot land, and
                    // the session stops here rather than going on to undo something else.
                    for line in plan.render() {
                        println!("{line}");
                    }
                    println!("rustage: the batch cannot land, so the session stopped");
                    return false;
                }
                let outcome = apply(&plan, &mut store);
                let Some(edit) = batch_edit_of(View::Canvas, batch.intent(), &plan, &outcome)
                else {
                    for line in outcome.render() {
                        println!("{line}");
                    }
                    println!("rustage: the batch was not applied, so nothing was recorded");
                    return false;
                };
                let entry = edit.entry();
                let applied = outcome
                    .render()
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "applied".to_owned());
                println!("edit {edits}: {} — {applied}", entry.describe());
                report_recording(&history.record(entry));
            }
            HistoryEdit::Undo(count) | HistoryEdit::Redo(count) => {
                let redo = matches!(edit, HistoryEdit::Redo(_));
                let times = *count;
                let many = if times == 1 { "step" } else { "steps" };
                println!(
                    "move {edits}: {} {times} {many}{}",
                    if redo { "redo" } else { "undo" },
                    match options.only {
                        None => String::new(),
                        Some(domain) => format!(", in the {} domain only", domain.name()),
                    }
                );
                moved = move_steps(
                    &mut history,
                    &mut store,
                    &mut buffer,
                    &root,
                    redo,
                    times,
                    options.only,
                ) && moved;
            }
        }
        if history.merge().is_none() && options.merge && batches > 0 {
            stopped = Some("the merge window ended: an edit in the other domain was recorded");
        }
    }
    if let Some(reason) = stopped {
        println!("{reason}");
    }
    if options.merge && history.merge().is_some() {
        println!(
            "the merge window closes: {}",
            close_merge(history.end_merge())
        );
    }

    for line in history.render() {
        println!("{line}");
    }
    println!("the model, after every move:");
    for line in Fingerprint::of(&store).as_str().lines() {
        println!("  {line}");
    }
    println!("the buffer, after every move:");
    for line in buffer.render() {
        println!("    {line}");
    }
    println!("nothing was written: the file is unchanged (writing it is T03.04)");
    if !moved {
        println!("rustage: a step that was asked for did not happen; the report above says why");
    }
    moved && buffer.state().is_current()
}

/// Writes a document's text back to disk, or a set of them together (task T03.04).
///
/// One file is the whole of T03.04 in one run: the text is read (with any `--set-text` edits typed
/// into a buffer first, so the buffer and the save are one loop), written into a temporary beside
/// the target and renamed over it. More than one file is a set, staged whole before anything is
/// renamed, so the set is all committed or all left as it was. `--plan` prints what would be written
/// and writes nothing.
///
/// The exit code is the report's own: a save that did not happen (a refusal, or a write the disk
/// refused) is a finding, and the report says whether the original is still exactly what it was.
pub(super) fn save(options: &crate::cli::SaveOptions) -> bool {
    use crate::host::paths::ProjectRoot;
    use crate::project::document::buffer::{Buffer, root_of};
    use crate::project::document::save::{Commit, Saving, Verb};

    let root = match options.root.as_ref() {
        Some(directory) => match ProjectRoot::new(directory) {
            Ok(root) => root,
            Err(error) => {
                println!("rustage: {}", error.message());
                return false;
            }
        },
        None => match root_of(&options.files[0]) {
            Ok(root) => root,
            Err(refusal) => {
                println!("rustage: {}", refusal.message());
                return false;
            }
        },
    };

    let mut documents: Vec<String> = Vec::new();
    for file in &options.files {
        let relative = relative_to(&root, file);
        match relative.to_str() {
            Some(name) => documents.push(name.to_owned()),
            None => {
                println!(
                    "rustage: {} is not a path this build can name",
                    file.display()
                );
                return false;
            }
        }
    }

    // A single file goes through a buffer, so the text that is saved is the text somebody typed, not
    // the file re-read. A set is the files as they now stand, and takes no edits — the point of a set
    // is that the commit is one transaction, not that it edits several files at once.
    if documents.len() != 1 && !matches!(options.verb, Verb::Save) {
        println!(
            "rustage: {} names one destination, so it writes one file; {} files were given",
            options.verb.name(),
            documents.len()
        );
        return false;
    }
    if documents.len() == 1 {
        let mut buffer = match Buffer::open(&root, &documents[0]) {
            Ok(buffer) => buffer,
            Err(refusal) => {
                println!("rustage: {}", refusal.message());
                return false;
            }
        };
        for edit in &options.edits {
            let text = match edit {
                crate::cli::DocumentEdit::Text(text) => text.clone(),
                crate::cli::DocumentEdit::File(path) => match std::fs::read_to_string(path) {
                    Ok(text) => text,
                    Err(error) => {
                        println!("rustage: {} could not be read: {error}", path.display());
                        return false;
                    }
                },
            };
            buffer.set_text(&root, text);
        }
        // The external-modification guard (T03.05) only applies to writing back over the same file:
        // for save-as and save-a-copy the destination is a different file, and the document's own
        // bytes say nothing about it.
        let agreed = match options.verb {
            Verb::Save => buffer.on_disk(),
            _ => None,
        };
        let saving = match Saving::plan_guarded(
            &root,
            &documents[0],
            buffer.text(),
            options.verb.clone(),
            agreed,
        ) {
            Ok(saving) => saving,
            Err(problem) => {
                println!("rustage: {}", problem.message());
                return false;
            }
        };
        println!("{}: {}", saving.target().display(), options.verb.name());
        if saving.is_unchanged() {
            println!("- the target already holds exactly this text, so there is nothing to write");
            return true;
        }
        println!(
            "- {} byte(s) would go through {}",
            saving.text().len(),
            saving.temporary().display()
        );
        if options.plan_only {
            println!("planned only: nothing was written");
            return true;
        }
        return match saving.execute() {
            Ok(committed) => {
                println!(
                    "- wrote {} ({} byte(s){})",
                    committed.target().display(),
                    committed.bytes(),
                    if committed.created() {
                        ", it did not exist before"
                    } else {
                        ""
                    }
                );
                true
            }
            Err(failure) => {
                println!("rustage: {}", failure.message());
                false
            }
        };
    }

    if !options.edits.is_empty() {
        println!(
            "rustage: --set-text is one file's business; a set of {} file(s) is written back as it \
             stands",
            documents.len()
        );
        return false;
    }
    if !matches!(options.verb, Verb::Save) {
        println!(
            "rustage: {} names one destination, so it is one file's business",
            options.verb.name()
        );
        return false;
    }

    // The set is written back from each file's own current bytes, so nothing changes: the point of
    // this path is the transaction, and the test suite injects the failure that exercises it.
    let entries = match read_each(&root, &documents) {
        Ok(entries) => entries,
        Err(reason) => {
            println!("rustage: {reason}");
            return false;
        }
    };
    let commit = match Commit::plan(&root, entries) {
        Ok(commit) => commit,
        Err(problem) => {
            println!("rustage: {}", problem.message());
            return false;
        }
    };
    for target in commit.targets() {
        println!("would write {}", target.display());
    }
    if options.plan_only || commit.is_unchanged() {
        println!("nothing was written");
        return true;
    }
    match commit.execute() {
        Ok(committed) => {
            println!("committed {} file(s) as one set", committed.targets().len());
            true
        }
        Err(failure) => {
            println!("rustage: {}", failure.message());
            false
        }
    }
}

/// Decides what a document should do when the file changed outside the designer (task T03.05).
///
/// The three versions are read here and named in the report: the base is what the buffer read, `mine`
/// is the text after any `--set-text` edits, and `theirs` is the file as it is now. Showing the
/// decision writes nothing; `--recreate` answers a deleted file by writing the document's text back,
/// and `--accept` writes a merge that landed. A conflict is reported and never resolved by writing:
/// picking a side silently is the data loss this task exists to prevent.
pub(super) fn conflict(options: &crate::cli::ConflictOptions) -> bool {
    use crate::host::paths::ProjectRoot;
    use crate::project::document::buffer::{Buffer, root_of};
    use crate::project::document::conflict::{OnDisk, Verdict, decide, writability};
    use crate::project::document::save::Verb;

    let root = match options.root.as_ref() {
        Some(directory) => match ProjectRoot::new(directory) {
            Ok(root) => root,
            Err(error) => {
                println!("rustage: {}", error.message());
                return false;
            }
        },
        None => match root_of(&options.file) {
            Ok(root) => root,
            Err(refusal) => {
                println!("rustage: {}", refusal.message());
                return false;
            }
        },
    };
    let relative = relative_to(&root, &options.file);
    let Some(document) = relative.to_str().map(str::to_owned) else {
        println!(
            "rustage: {} is not a path this build can name",
            options.file.display()
        );
        return false;
    };

    // A deleted file is exactly one of the states this command decides about, so a buffer that
    // cannot be opened is not a failure here: the document's text comes from the edits instead.
    let mut buffer = Buffer::open(&root, &document).ok();
    if let Some(buffer) = buffer.as_mut() {
        for edit in &options.edits {
            let text = match edit {
                crate::cli::DocumentEdit::Text(text) => text.clone(),
                crate::cli::DocumentEdit::File(path) => match std::fs::read_to_string(path) {
                    Ok(text) => text,
                    Err(error) => {
                        println!("rustage: {} could not be read: {error}", path.display());
                        return false;
                    }
                },
            };
            buffer.set_text(&root, text);
        }
    }

    let path = match root.resolve_for_creation(&document) {
        Ok(path) => path,
        Err(error) => {
            println!("rustage: {}", error.message());
            return false;
        }
    };
    // `mine` is the text the document holds: the buffer's, when a file could be read, and otherwise
    // the last `--set-text`/`--set-text-file` edit, so a deleted document can still be answered.
    let mine = match buffer.as_ref() {
        Some(buffer) => buffer.text().to_owned(),
        None => match options.edits.last() {
            Some(crate::cli::DocumentEdit::Text(text)) => text.clone(),
            Some(crate::cli::DocumentEdit::File(path)) => match std::fs::read_to_string(path) {
                Ok(text) => text,
                Err(error) => {
                    println!("rustage: {} could not be read: {error}", path.display());
                    return false;
                }
            },
            None => String::new(),
        },
    };
    let base = match &options.base {
        Some(base_path) => match std::fs::read(base_path) {
            Ok(bytes) => Some(bytes),
            Err(error) => {
                println!(
                    "rustage: {} could not be read: {error}",
                    base_path.display()
                );
                return false;
            }
        },
        None => std::fs::read(&path).ok(),
    };
    let on_disk = OnDisk::inspect(&path, base.as_deref());
    let verdict = decide(base.as_deref(), &mine, &on_disk);

    println!("document: {}", path.display());
    println!("base: {} byte(s)", base.as_ref().map(Vec::len).unwrap_or(0));
    println!("mine: {} byte(s)", mine.len());
    println!(
        "on disk: {}",
        match &on_disk {
            OnDisk::Same { bytes } => format!("unchanged ({} byte(s))", bytes.len()),
            OnDisk::Changed { bytes } => format!("changed ({} byte(s))", bytes.len()),
            OnDisk::Gone => "gone".to_owned(),
            OnDisk::NotAFile { reason } => format!("not a readable file: {reason}"),
        }
    );
    println!("verdict: {}", verdict.message());
    if !matches!(on_disk, OnDisk::Gone) {
        println!("{}", writability(&path).message());
    }

    match &verdict {
        Verdict::Conflicted { regions } => {
            for region in regions {
                println!("- {}", region.describe());
                println!(
                    "    the document has: {}",
                    region.mine().concat().trim_end()
                );
                println!(
                    "    the file has:     {}",
                    region.theirs().concat().trim_end()
                );
            }
            println!("a conflict is reported, not resolved: choose which side to keep");
            false
        }
        Verdict::Merged { text } => {
            println!("merged text ({} byte(s)):", text.len());
            print!("{text}");
            if !text.ends_with('\n') {
                println!();
            }
            if options.accept {
                // The merge was computed against what is on disk *now*, so that is the base the
                // write has to agree with — not the older bytes the document read.
                let agreed = match &on_disk {
                    OnDisk::Same { bytes } | OnDisk::Changed { bytes } => Some(bytes.as_slice()),
                    _ => None,
                };
                write_text(&root, &document, text, Verb::Save, agreed)
            } else {
                println!("nothing was written; pass --accept to write the merged text");
                true
            }
        }
        Verdict::Gone => {
            if options.recreate {
                // There is no file to write back over, so re-creating is `SaveAs`: the same atomic
                // write, with the path resolved as one that is going to be created.
                let verb = Verb::SaveAs {
                    to: document.clone(),
                };
                write_text(&root, &document, &mine, verb, None)
            } else {
                println!(
                    "nothing was written; pass --recreate to write the document's text back, or let \
                     the file go"
                );
                true
            }
        }
        Verdict::DiskOnly => {
            println!("the file changed and the document has no edit of its own");
            true
        }
        _ => true,
    }
}

/// Writes `text` over a document through the save path, with a guard when there is a base.
pub(super) fn write_text(
    root: &crate::host::paths::ProjectRoot,
    document: &str,
    text: &str,
    verb: crate::project::document::save::Verb,
    agreed: Option<&[u8]>,
) -> bool {
    use crate::project::document::save::Saving;

    match Saving::plan_guarded(root, document, text, verb, agreed) {
        Ok(saving) => match saving.execute() {
            Ok(committed) => {
                println!(
                    "wrote {} ({} byte(s))",
                    committed.target().display(),
                    committed.bytes()
                );
                true
            }
            Err(failure) => {
                println!("rustage: {}", failure.message());
                false
            }
        },
        Err(problem) => {
            println!("rustage: {}", problem.message());
            false
        }
    }
}

/// Keeps, offers and refuses recovery copies (task T03.06).
///
/// `--record` writes a copy of a document's text into a store that lives outside any project; with no
/// action the command is the **startup recovery** report — every copy for a project, newest first,
/// each saying how it stands next to the file now. `--restore` prints the newest copy so a person can
/// take it, and `--discard` is the refusal. Nothing adopts a copy on its own: a recovery that
/// silently replaced a file saved afterwards would be worse than none.
pub(super) fn recovery(options: &crate::cli::RecoveryOptions) -> bool {
    use crate::host::paths::ProjectRoot;
    use crate::project::document::recovery::RecoveryStore;
    use std::time::{SystemTime, UNIX_EPOCH};

    let store = RecoveryStore::new(&options.store);
    let target = options
        .record
        .as_ref()
        .or(options.restore.as_ref())
        .or(options.discard.as_ref());

    // The project root: named, or found above the document this action names.
    let root = match (&options.root, target) {
        (Some(directory), _) => match ProjectRoot::new(directory) {
            Ok(root) => root,
            Err(error) => {
                println!("rustage: {}", error.message());
                return false;
            }
        },
        (None, Some(file)) => match crate::project::document::buffer::root_of(file) {
            Ok(root) => root,
            Err(refusal) => {
                println!("rustage: {}", refusal.message());
                return false;
            }
        },
        (None, None) => {
            println!(
                "rustage: --root names the project whose copies to list (there is no document to read it from)"
            );
            return false;
        }
    };

    let session = options.session.clone().unwrap_or_else(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs().to_string())
            .unwrap_or_else(|_| "session".to_owned())
    });

    if let Some(file) = &options.record {
        let relative = relative_to(&root, file);
        let Some(document) = relative.to_str().map(str::to_owned) else {
            println!(
                "rustage: {} is not a path this build can name",
                file.display()
            );
            return false;
        };
        let path = match root.resolve(&document) {
            Ok(path) => path,
            Err(error) => {
                println!("rustage: {}", error.message());
                return false;
            }
        };
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) => {
                println!("rustage: {} could not be read: {error}", file.display());
                return false;
            }
        };
        return match store.record(
            &session,
            root.as_path(),
            &document,
            &text,
            SystemTime::now(),
        ) {
            Ok(recorded) => {
                println!("{}", recorded.message());
                true
            }
            Err(error) => {
                println!("rustage: {}", error.message());
                false
            }
        };
    }

    if let Some(file) = &options.restore {
        let relative = relative_to(&root, file);
        let Some(document) = relative.to_str().map(str::to_owned) else {
            println!(
                "rustage: {} is not a path this build can name",
                file.display()
            );
            return false;
        };
        return match store.newest(root.as_path(), &document) {
            Some(copy) => {
                for line in copy.render() {
                    println!("{line}");
                }
                print!("{}", copy.text());
                if !copy.text().ends_with('\n') {
                    println!();
                }
                println!("nothing was written: taking the copy is the caller's decision");
                true
            }
            None => {
                println!("rustage: there is no recovery copy for {document}");
                false
            }
        };
    }

    if let Some(file) = &options.discard {
        let relative = relative_to(&root, file);
        let Some(document) = relative.to_str().map(str::to_owned) else {
            println!(
                "rustage: {} is not a path this build can name",
                file.display()
            );
            return false;
        };
        let removed = store.discard(root.as_path(), &document);
        println!(
            "refused {removed} recovery copy/copies of {document}; the document itself was not touched"
        );
        return true;
    }

    if options.prune {
        let policy = crate::project::document::retention::Policy::new(options.keep.unwrap_or(5));
        let plan = crate::project::document::retention::plan(&store, policy);
        for line in plan.render() {
            println!("{line}");
        }
        if !options.apply {
            println!("nothing was removed; pass --apply to carry the prune out");
            return true;
        }
        let pruned = crate::project::document::retention::apply(&plan);
        println!("{}", pruned.message());
        return true;
    }

    let copies = store.for_project(root.as_path());
    if copies.is_empty() {
        println!("no recovery copies for {}", root.as_path().display());
        return true;
    }
    println!(
        "{} recovery copy/copies for {} (newest first):",
        copies.len(),
        root.as_path().display()
    );
    for copy in &copies {
        for line in copy.render() {
            println!("{line}");
        }
    }
    println!(
        "--restore <document> prints one so a person can take it; --discard <document> refuses it"
    );
    true
}

/// Plans and carries out closing documents with a dirty-content decision (task T03.07).
///
/// The plan is always printed: which documents have unsaved changes and how much work would stop. A
/// decision then carries it out, and the outcome names what happened — a cancel and a failing write
/// both leave every document and task as they were, because the state is taken and given back rather
/// than acted on in place.
pub(super) fn close(options: &crate::cli::CloseOptions) -> bool {
    use crate::host::paths::ProjectRoot;
    use crate::project::document::closing::{Closing, Document, Task};

    let first = match options.files.first().or(options.root.as_ref()) {
        Some(path) => path,
        None => {
            println!("rustage: --close names at least one document");
            return false;
        }
    };
    let root = match options.root.as_ref() {
        Some(directory) => match ProjectRoot::new(directory) {
            Ok(root) => root,
            Err(error) => {
                println!("rustage: {}", error.message());
                return false;
            }
        },
        None => match crate::project::document::buffer::root_of(first) {
            Ok(root) => root,
            Err(refusal) => {
                println!("rustage: {}", refusal.message());
                return false;
            }
        },
    };

    let mut documents = Vec::new();
    for file in &options.files {
        let relative = relative_to(&root, file);
        let Some(name) = relative.to_str() else {
            println!(
                "rustage: {} is not a path this build can name",
                file.display()
            );
            return false;
        };
        match Document::clean(&root, name) {
            Ok(document) => documents.push(document),
            Err(reason) => {
                println!("rustage: {reason}");
                return false;
            }
        }
    }
    for (document, buffer) in &options.unsaved {
        let text = match std::fs::read_to_string(buffer) {
            Ok(text) => text,
            Err(error) => {
                println!("rustage: {} could not be read: {error}", buffer.display());
                return false;
            }
        };
        match Document::unsaved(&root, document, text) {
            Ok(document) => documents.push(document),
            Err(reason) => {
                println!("rustage: {reason}");
                return false;
            }
        }
    }
    let tasks: Vec<Task> = options.tasks.iter().cloned().map(Task::running).collect();

    let closing = Closing::new(root, documents, tasks);
    println!("close plan: {}", closing.plan().message());
    let Some(decision) = options.decision else {
        println!("nothing was closed; pass --decision to carry it out");
        return true;
    };
    let (after, outcome) = closing.close(decision);
    println!(
        "{} ({}): {}",
        decision.name(),
        outcome.message(),
        after.tasks().len()
    );
    match outcome {
        crate::project::document::closing::Outcome::Closed { .. } => true,
        crate::project::document::closing::Outcome::Cancelled => true,
        crate::project::document::closing::Outcome::Blocked { .. } => false,
    }
}
