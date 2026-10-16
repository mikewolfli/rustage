// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! F02: the project model — creation, import, configuration and files.
//!
//! One command handler per parsed command of this domain; the dispatcher lives in
//! [`super`](self).

use super::*;

/// Opens a project: prints what it is, asks the question, and does nothing else unless asked.
///
/// Returns whether the project was opened.
pub(super) fn open_project(options: &crate::cli::OpenProjectOptions) -> bool {
    let permissions = PermissionService::new();
    let mut session = match ProjectSession::open(&options.path) {
        Ok(session) => session,
        Err(error) => {
            println!("rustage: {}", error.message());
            return false;
        }
    };
    println!("project: {}", session.path().display());
    println!("root: {}", session.root().as_path().display());
    println!("format: {}", session.file().verdict().message());
    println!(
        "identity: {} ({})",
        session.name().unwrap_or("<not readable>"),
        session.id().unwrap_or("<not readable>")
    );
    println!("state: {}", session.state().message());
    println!(
        "trust: {}",
        if session.is_trusted(&permissions) {
            "trusted"
        } else {
            "not decided yet"
        }
    );
    println!("writable now: {}", yes_no(session.may_write(&permissions)));
    println!("{}", session.question().ask());

    // Opening is recorded as a place, never as a permission: the list holds a path, a name and an
    // identity, and nothing in it can run anything.
    match recent_list() {
        Ok(mut recent) => {
            let entry = session.recent_entry(std::time::SystemTime::now());
            recent.record(entry);
            match recent.save() {
                Ok(path) => println!("remembered: {}", path.display()),
                Err(error) => println!(
                    "rustage: the recent list was not saved: {}",
                    error.message()
                ),
            }
        }
        Err(reason) => println!("remembered: no — {reason}"),
    }

    if options.trust {
        let outcome = session.decide(&permissions, TrustDecision::Trust);
        println!("answer: {}", outcome.message());
        if !outcome.trusted() {
            return false;
        }
    }
    if options.diagnose {
        let state = session.diagnose_toolchain(&permissions);
        println!("toolchain: {}", state.message());
        if options.trust && state.report().is_none() {
            return false;
        }
    } else {
        println!("toolchain: {}", session.toolchain().message());
    }
    match session.state() {
        SessionState::Readable => true,
        // Read-only is a state this build reports and honours, not a failure: the acceptance is that
        // the project *can* be opened read-only.
        SessionState::ReadOnly { .. } => true,
    }
}

/// Removes an entry, fixing what points at it or refusing to remove it.
///
/// The removal goes through the same decisions every other effect goes through: the project has to
/// be trusted (a person's answer, `--trust`) and writing a file needs the permission the decision
/// table gives it. Nothing is written when either says no.
pub(super) fn drop_entry(options: &crate::cli::DropEntryOptions) -> bool {
    let permissions = PermissionService::new();
    let session = match ProjectSession::open(&options.path) {
        Ok(session) => session,
        Err(error) => {
            println!("rustage: {}", error.message());
            return false;
        }
    };
    println!("{}", session.question().ask());
    let outcome = session.decide(
        &permissions,
        if options.trust {
            TrustDecision::Trust
        } else {
            TrustDecision::DoNotTrust
        },
    );
    println!("answer: {}", outcome.message());
    if !outcome.trusted() {
        return false;
    }
    if let Err(refusal) = permissions.permit(
        crate::contracts::permission::EffectClass::FileWrite,
        crate::host::permission::Scope::Project(session.root()),
        crate::contracts::permission::Principal::Human,
    ) {
        println!("rustage: {}", refusal.message());
        return false;
    }

    let Some(document) = session.file().document() else {
        println!("rustage: the document cannot be interpreted, so nothing was removed");
        return false;
    };
    let Some(index) = document
        .pages()
        .iter()
        .position(|page| page.document().written() == options.document)
    else {
        println!(
            "rustage: no entry names `{}`; nothing was removed",
            options.document
        );
        return false;
    };

    let resolution = if options.fix {
        crate::project::format::Resolution::FixReferences
    } else {
        crate::project::format::Resolution::Block
    };
    let (removed, fixed) = match document.remove_entry(index, resolution) {
        Ok(outcome) => outcome,
        Err(refusal) => {
            println!("rustage: {}", refusal.message());
            return false;
        }
    };
    println!("removed {}", options.document);
    for line in &fixed {
        println!("  fixed: {line}");
    }
    if fixed.is_empty() {
        println!("  nothing pointed at it");
    }
    match session.file().write_document(&removed) {
        Ok(written) => {
            println!("{}", written.message());
            match ProjectFile::open(session.path()) {
                Ok(again) => {
                    println!("read back: {}", again.verdict().message());
                    true
                }
                Err(error) => {
                    println!(
                        "rustage: what was written cannot be read back: {}",
                        error.message()
                    );
                    false
                }
            }
        }
        Err(error) => {
            println!("rustage: {}", error.message());
            false
        }
    }
}

/// Plans one file operation, and carries it out when it was asked for.
///
/// The preview reads the project and decides nothing, so it needs no permission: it is what the
/// design surface shows before anyone agrees to anything. Carrying it out moves files and writes
/// the document, so it asks the project's question first and then takes the write permission.
pub(super) fn file_op(options: &crate::cli::FileOpOptions) -> bool {
    let permissions = PermissionService::new();
    let session = match ProjectSession::open(&options.path) {
        Ok(session) => session,
        Err(error) => {
            println!("rustage: {}", error.message());
            return false;
        }
    };

    if !options.apply {
        let Some(document) = session.file().document() else {
            println!("rustage: the document cannot be interpreted, so no operation can be planned");
            return false;
        };
        let plan = FilePlan::plan(document, session.root(), options.operation.clone());
        println!("{}", session.question().ask());
        println!("answer: not asked; this is a preview and nothing is written");
        for line in plan.render() {
            println!("{line}");
        }
        if !plan.is_executable() {
            println!("rustage: the operation is refused while those blockers hold");
            return false;
        }
        return true;
    }

    println!("{}", session.question().ask());
    let outcome = session.decide(
        &permissions,
        if options.trust {
            TrustDecision::Trust
        } else {
            TrustDecision::DoNotTrust
        },
    );
    println!("answer: {}", outcome.message());
    if !outcome.trusted() {
        return false;
    }
    if let Err(refusal) = permissions.permit(
        crate::contracts::permission::EffectClass::FileWrite,
        crate::host::permission::Scope::Project(session.root()),
        crate::contracts::permission::Principal::Human,
    ) {
        println!("rustage: {}", refusal.message());
        return false;
    }

    let Some(document) = session.file().document() else {
        println!("rustage: the document cannot be interpreted, so nothing was done");
        return false;
    };
    let plan = FilePlan::plan(document, session.root(), options.operation.clone());
    // The plan is printed either way: a refusal has to say what it refused, and a success has to
    // say what it did.
    for line in plan.render() {
        println!("{line}");
    }
    match plan.execute(session.file()) {
        Ok(executed) => {
            for line in executed.render() {
                println!("{line}");
            }
            match ProjectFile::open(session.path()) {
                Ok(again) => {
                    println!("read back: {}", again.verdict().message());
                    true
                }
                Err(error) => {
                    println!(
                        "rustage: what was written cannot be read back: {}",
                        error.message()
                    );
                    false
                }
            }
        }
        Err(failure) => {
            // Every line of the failure is printed, including what could not be put back: a
            // half-done operation that says so is recoverable, one that does not is not.
            println!("rustage: {}", failure.message());
            println!("recovered: {}", failure.is_recovered());
            // The operation did not happen, so this is not a success even when everything was put
            // back; the recovery is reported because it decides whether anyone has to look.
            false
        }
    }
}

/// Shows which tools a kit is made of, and what this machine has of them (task T02.10).
///
/// Showing runs nothing. `--diagnose` asks this machine about each member — which runs the programs
/// the kit names, so it asks the kit's directory question first and takes the permission the decision
/// table gives a command — and `--triple` is the output the kit is asked about. The exit code says
/// whether anything was *decided* to be wrong: "nobody looked" leaves the kit incomplete and is not
/// reported as a failure, because a script that treats it as one would be told to fix what may be
/// fine.
/// Plans and applies a batch of edits to a page document's model (task T03.02).
///
/// Nothing is written to disk and nothing is run, so there is no question to ask and no permission to
/// take — the batch is planned against the model the file describes, and applied to it in memory. The
/// report is the point: what each operation would do, what cannot land and why (a blocked batch is
/// refused as a whole), which capability the store will ask about, and — when it lands — the batch
/// that would undo it. `--undo` then applies that inverse, so "all committed or all refused" and "the
/// document is back where it was" can both be seen in one run. Writing the model back to the file is
/// T03.04's subject, and the report says so.
pub(super) fn edit(options: &crate::cli::EditOptions) -> bool {
    use crate::contracts::permission::PermissionGate;
    use crate::project::batch::{Batch, Fingerprint, Plan, apply, store_of};
    use crate::project::document::root_of;
    use crate::project::page::PageDocument;

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
    let page = match PageDocument::read(&root, relative_to(&root, &options.file)) {
        Ok(page) => page,
        Err(error) => {
            println!("rustage: {}", error.message());
            return false;
        }
    };
    let mut store = match store_of(&page, PermissionGate::new()) {
        Ok(store) => store,
        Err(refusal) => {
            println!("rustage: {}", refusal.message());
            return false;
        }
    };
    println!("edit: in memory; nothing is written to any file, and no question is asked");
    println!(
        "model of {}: {} node(s), revision {}",
        page.path().display(),
        store.nodes().count(),
        store.revision()
    );

    let Some(path) = options.batch.as_ref() else {
        for line in Fingerprint::of(&store).as_str().lines() {
            println!("  {line}");
        }
        println!("no batch was named, so nothing was planned or applied");
        return true;
    };
    let batch = match Batch::read(path) {
        Ok(batch) => batch,
        Err(refusal) => {
            println!("rustage: {}", refusal.message());
            return false;
        }
    };
    for line in batch.describe() {
        println!("{line}");
    }
    let plan = match Plan::of(&batch, &store, &root) {
        Ok(plan) => plan,
        Err(refusal) => {
            println!("rustage: {}", refusal.message());
            return false;
        }
    };
    for line in plan.render() {
        println!("{line}");
    }
    if !options.apply {
        println!("planned and not applied: --apply commits it to the model in memory");
        println!("either way the file is not written: that is T03.04's subject");
        return !plan.is_blocked();
    }
    let before = Fingerprint::of(&store);
    let outcome = apply(&plan, &mut store);
    for line in outcome.render() {
        println!("{line}");
    }
    if !outcome.is_applied() {
        // A refused batch is a finding, not a crash: the reason was printed above, and the document
        // is where it was.
        return false;
    }
    for line in Fingerprint::of(&store).as_str().lines() {
        println!("  {line}");
    }
    if let Some(inverse) = outcome.inverse() {
        match inverse.batch.render() {
            Ok(text) => println!("inverse: {text}"),
            Err(error) => {
                println!(
                    "rustage: the inverse could not be written: {}",
                    error.message()
                );
                return false;
            }
        }
    }
    if !options.undo {
        println!("nothing was written: the file is unchanged (writing it is T03.04)");
        return true;
    }
    let inverse = match outcome.inverse() {
        Some(inverse) => inverse,
        None => {
            println!("rustage: there is nothing to undo");
            return false;
        }
    };
    let undo_plan = match Plan::of(&inverse.batch, &store, &root) {
        Ok(plan) => plan,
        Err(refusal) => {
            println!(
                "rustage: the inverse cannot be planned: {}",
                refusal.message()
            );
            return false;
        }
    };
    for line in undo_plan.render() {
        println!("{line}");
    }
    let undone = apply(&undo_plan, &mut store);
    for line in undone.render() {
        println!("{line}");
    }
    if !undone.is_applied() {
        return false;
    }
    let after = Fingerprint::of(&store);
    println!(
        "the model is {} after undoing the batch",
        if after == before {
            "back where it started"
        } else {
            "not where it started"
        }
    );
    after == before
}

/// Resolves a workspace's dependencies, and says exactly what it did (task T02.11).
///
/// The order here *is* the guarantee: the resolution is decided (a command and its effects), it is
/// shown, every effect is mapped to the class the permission table uses, each class is authorized,
/// and only then is anything started. Nothing is inferred from "it is only cargo": the effects come
/// from the resolution that was asked for, and a caller who did not authorize one does not get it.
pub(super) fn dependencies(options: &crate::cli::DependenciesOptions) -> bool {
    use crate::contracts::permission::{EffectClass, Principal};
    use crate::host::paths::ProjectRoot;
    use crate::host::permission::Scope;
    use crate::host::subprocess::{OwnedProcess, ProcessSpec};
    use crate::project::dependencies::{
        Digest, LOCKFILE_FILE_NAME, Lockfile, Resolution, Review, failure_reason,
        is_a_lockfile_disagreement, manifest_of, may_record_at,
    };
    use crate::project::session::{TrustDecision, TrustQuestion};

    if let Err(refusal) = manifest_of(&options.directory) {
        println!("rustage: {}", refusal.message());
        return false;
    }
    let root = match ProjectRoot::new(&options.directory) {
        Ok(root) => root,
        Err(error) => {
            println!("rustage: {}", error.message());
            return false;
        }
    };

    // The resolution is a value, and the flags are its effects: what would be run, and what running
    // it could do, are decided once, here, and reported before anything is asked for.
    let resolution = Resolution::with_effects(
        root.as_path(),
        options.allow_lockfile_change,
        options.allow_network,
    );
    println!("{}", resolution.render());

    let permissions = PermissionService::new();
    let question = TrustQuestion::read(&root);
    println!("{}", question.ask());
    let outcome = crate::project::session::answer(
        &question,
        &permissions,
        if options.trust {
            TrustDecision::Trust
        } else {
            TrustDecision::DoNotTrust
        },
    );
    println!("answer: {}", outcome.message());

    let mut authorized = Vec::new();
    let mut command_permit = None;
    if outcome.trusted() {
        for class in [
            EffectClass::Command,
            EffectClass::FileWrite,
            EffectClass::Network,
        ] {
            if !resolution
                .effects()
                .iter()
                .any(|effect| effect.class() == class)
            {
                continue;
            }
            match permissions.permit(class, Scope::Project(&root), Principal::Human) {
                Ok(permit) => {
                    if class == EffectClass::Command {
                        command_permit = Some(permit);
                    }
                    authorized.push(class);
                }
                Err(refusal) => {
                    println!("rustage: {}", refusal.message());
                    return false;
                }
            }
        }
    }
    let unauthorized = resolution.unauthorized(&authorized);
    if !unauthorized.is_empty() {
        println!(
            "rustage: {} effect(s) are not authorized here: {}; nothing was run",
            unauthorized.len(),
            unauthorized
                .iter()
                .map(|effect| effect.name())
                .collect::<Vec<_>>()
                .join(", ")
        );
        return false;
    }
    let Some(command_permit) = command_permit else {
        // Unreachable while `RunsAProgram` is part of every resolution, and a branch rather than an
        // unwrap so that a future resolution without it still cannot run an unauthorized program.
        println!("rustage: no command was authorized; nothing was run");
        return false;
    };

    let before = Lockfile::read(root.as_path());
    let mut spec = ProcessSpec::new(resolution.command(), "cargo");
    for argument in resolution.arguments() {
        spec = spec.arg(argument);
    }
    let mut process = match OwnedProcess::spawn(spec, &command_permit) {
        Ok(process) => process,
        Err(error) => {
            println!("rustage: `cargo` could not be started: {error}");
            return false;
        }
    };
    let exit = process.wait();
    let output = process.output_so_far();

    if !exit.is_success() {
        // A resolution that failed is reported as a failure, with cargo's own reason, and the one
        // reason that is not a defect is named as what it is: a dependency change is pending, and
        // this resolution does not make it.
        match failure_reason(&output.stderr) {
            Some(reason) if is_a_lockfile_disagreement(&reason) => {
                println!("rustage: `{}` failed: {reason}", resolution.command());
                println!(
                    "a dependency change is pending: {} is not what the manifest resolves to, and \
                     this resolution does not change it (--allow-lockfile-change resolves it \
                     offline; --allow-network additionally lets it reach the registry)",
                    LOCKFILE_FILE_NAME
                );
            }
            Some(reason) => println!("rustage: `{}` failed: {reason}", resolution.command()),
            None => println!(
                "rustage: `{}` ended with {}: {}",
                resolution.command(),
                exit.message(),
                output.stderr.trim()
            ),
        }
        return false;
    }

    let digest = match Digest::of_cargo_metadata(&output.stdout, root.as_path()) {
        Ok(digest) => digest,
        Err(error) => {
            println!(
                "rustage: cargo reported something this build could not read: {}",
                error.message()
            );
            return false;
        }
    };
    for line in digest.render() {
        println!("{line}");
    }
    // The lockfile is read where cargo keeps it, and where that is not the directory that was named,
    // saying so is the honest answer: a member directory's lockfile is the workspace's.
    let workspace_root = PathBuf::from(digest.root());
    let after = digest.lockfile().clone();
    if workspace_root != root.as_path() {
        println!(
            "this manifest is a member of the workspace at {}: {LOCKFILE_FILE_NAME} is there, {}",
            workspace_root.display(),
            after.render()
        );
    } else if before != after {
        println!(
            "this resolution changed {LOCKFILE_FILE_NAME}: {} before, {} after",
            packages_of(&before),
            packages_of(&after)
        );
    }

    if let Some(path) = options.review.as_ref() {
        let recorded = match Digest::load(path) {
            Ok(recorded) => recorded,
            Err(error) => {
                println!("rustage: the record could not be read: {}", error.message());
                return false;
            }
        };
        match Review::between(&recorded, &digest) {
            Ok(review) => {
                for line in review.render() {
                    println!("{line}");
                }
            }
            Err(refusal) => {
                println!("rustage: {}", refusal.message());
                return false;
            }
        }
    }
    if let Some(path) = options.record.as_ref() {
        if let Err(refusal) = may_record_at(root.as_path(), path) {
            println!("rustage: {}", refusal.message());
            return false;
        }
        match digest.save(path) {
            Ok(written) => println!("recorded: {}", written.display()),
            Err(error) => {
                println!(
                    "rustage: the record could not be written: {}",
                    error.message()
                );
                return false;
            }
        }
    }
    true
}

/// How many packages a lockfile holds, for a line about a change.
pub(super) fn packages_of(lockfile: &crate::project::dependencies::Lockfile) -> String {
    match lockfile.packages() {
        Some(packages) => format!("{packages} package(s)"),
        None => "absent".to_owned(),
    }
}

pub(super) fn kit(options: &crate::cli::KitOptions) -> bool {
    use crate::project::kit::{Found, Kit, Observed, Probe};
    use crate::project::session::{TrustDecision, TrustQuestion};

    let permissions = PermissionService::new();
    let kit = match Kit::read(&options.path) {
        Ok(kit) => kit,
        Err(error) => {
            println!("rustage: {}", error.message());
            return false;
        }
    };
    for line in kit.render() {
        println!("{line}");
    }

    let mut observed = Observed::nothing();
    if options.diagnose {
        let question = TrustQuestion::read(kit.root());
        println!("{}", question.ask());
        let outcome = crate::project::session::answer(
            &question,
            &permissions,
            if options.trust {
                TrustDecision::Trust
            } else {
                TrustDecision::DoNotTrust
            },
        );
        println!("answer: {}", outcome.message());
        if !outcome.trusted() {
            return false;
        }
        if let Err(refusal) = permissions.permit(
            crate::contracts::permission::EffectClass::Command,
            crate::host::permission::Scope::Project(kit.root()),
            crate::contracts::permission::Principal::Human,
        ) {
            println!("rustage: {}", refusal.message());
            return false;
        }
        for member in kit.members() {
            let found = match member.probe() {
                Probe::Program { command, argument } => {
                    ask_the_program(&permissions, kit.root(), command, argument.as_deref())
                }
                Probe::Path { written } => {
                    let path = kit.root().resolve_for_creation(written);
                    match path {
                        Ok(path) if path.exists() => Found::Value(path.display().to_string()),
                        Ok(path) => Found::Nothing {
                            reason: format!("there is nothing at {}", path.display()),
                        },
                        Err(error) => Found::NotChecked {
                            reason: error.message(),
                        },
                    }
                }
                Probe::Variable { name } => match std::env::var(name) {
                    Ok(value) if value.is_empty() => Found::Nothing {
                        reason: format!("the {name} variable is set to nothing"),
                    },
                    Ok(value) => Found::Value(value),
                    Err(_) => Found::Nothing {
                        reason: format!("the {name} variable is not set"),
                    },
                },
                Probe::Undetermined { reason } => Found::NotChecked {
                    reason: reason.clone(),
                },
            };
            observed = observed.with(member.name(), found);
        }
    } else {
        println!(
            "answer: not asked; nothing was run, so every member is reported as not checked \
             (use --diagnose --trust to ask this machine)"
        );
    }

    let diagnosis = kit.diagnose(&observed, options.triple.as_deref());
    for line in diagnosis.render() {
        println!("{line}");
    }
    !diagnosis.has_decided_problems()
}

/// Runs one program a kit names, and reads a version out of what it printed.
///
/// The permit is the caller's: this does not decide whether a program may be started. A program that
/// cannot be started, or that ends badly, is a finding about the machine rather than a failure of
/// the report, so it is returned as `nothing` with what happened.
pub(super) fn ask_the_program(
    permissions: &PermissionService,
    root: &crate::host::paths::ProjectRoot,
    command: &str,
    argument: Option<&str>,
) -> crate::project::kit::Found {
    use crate::project::kit::{Found, version_like};

    let mut spec = crate::host::subprocess::ProcessSpec::new(
        format!(
            "{command}{}",
            argument
                .map(|argument| format!(" {argument}"))
                .unwrap_or_default()
        ),
        command,
    );
    if let Some(argument) = argument {
        spec = spec.arg(argument);
    }
    let permit = match permissions.permit(
        crate::contracts::permission::EffectClass::Command,
        crate::host::permission::Scope::Project(root),
        crate::contracts::permission::Principal::Human,
    ) {
        Ok(permit) => permit,
        Err(refusal) => {
            return Found::NotChecked {
                reason: refusal.message(),
            };
        }
    };
    let mut process = match crate::host::subprocess::OwnedProcess::spawn(spec, &permit) {
        Ok(process) => process,
        Err(error) => {
            return Found::Nothing {
                reason: format!("`{command}` could not be started: {error}"),
            };
        }
    };
    let exit = process.wait();
    let output = process.output_so_far();
    let text = format!("{}\n{}", output.stdout, output.stderr);
    match (exit.is_success(), version_like(&text)) {
        (true, Some(version)) => Found::Value(version),
        (true, None) => Found::Nothing {
            reason: format!("`{command}` printed no version this build could read"),
        },
        (false, _) => Found::Nothing {
            reason: format!("`{command}` ended with {}", exit.message()),
        },
    }
}

/// Shows a workspace, and plans or carries out one change to its members (task T02.09).
///
/// Showing writes nothing at all. A change is planned first, and `--apply` is the only path that
/// writes: it asks the directory's question, takes the `write-file` permission, and then writes the
/// workspace document — no directory is created and none is removed, which is the acceptance and is
/// said out loud in the output.
pub(super) fn workspace(options: &crate::cli::WorkspaceOptions) -> bool {
    use crate::cli::WorkspaceAction;

    let permissions = PermissionService::new();
    let workspace = match crate::project::workspace::Workspace::read(&options.path) {
        Ok(workspace) => workspace,
        Err(error) => {
            println!("rustage: {}", error.message());
            return false;
        }
    };
    for line in workspace.render() {
        println!("{line}");
    }
    let mut state = match workspace_state() {
        Ok(state) => state,
        Err(reason) => {
            println!("rustage: {reason}");
            return false;
        }
    };
    // Which member is active is this machine's state and not the document's, and it is printed
    // before anything is done with it: a choice that is not usable here says so rather than being
    // silently replaced.
    println!("active member: {}", state.active_for(&workspace).render());

    match &options.action {
        WorkspaceAction::Report => true,
        WorkspaceAction::Activate { name } => {
            let choice = match workspace.activate(&mut state, name, SystemTime::now()) {
                Ok(choice) => choice,
                Err(refusal) => {
                    println!("rustage: {}", refusal.message());
                    return false;
                }
            };
            println!(
                "active member: `{}` for {} ({})",
                choice.member(),
                choice.name(),
                choice.workspace()
            );
            println!(
                "the workspace document was not written: which member is active belongs to this \
                 machine"
            );
            match state.save() {
                Ok(path) => {
                    println!("wrote {}", path.display());
                    true
                }
                Err(error) => {
                    println!("rustage: {}", error.message());
                    false
                }
            }
        }
        WorkspaceAction::File { name } => match workspace.file(name) {
            Ok(file) => {
                println!("file: {}", file.render());
                println!("  named by member `{}`", file.member());
                println!("  where it lands: {}", file.cross().render());
                true
            }
            Err(lookup) => {
                println!("rustage: {}", lookup.message());
                false
            }
        },
        WorkspaceAction::Add { name, root } => change_members(
            &workspace,
            &permissions,
            options,
            crate::project::workspace::WorkspaceChange::Add {
                name: name.clone(),
                root: root.clone(),
            },
        ),
        WorkspaceAction::Remove { name } => change_members(
            &workspace,
            &permissions,
            options,
            crate::project::workspace::WorkspaceChange::Remove { name: name.clone() },
        ),
    }
}

/// Plans a member change, and carries it out when it was asked for.
pub(super) fn change_members(
    workspace: &crate::project::workspace::Workspace,
    permissions: &PermissionService,
    options: &crate::cli::WorkspaceOptions,
    change: crate::project::workspace::WorkspaceChange,
) -> bool {
    use crate::project::session::{TrustDecision, TrustQuestion};

    let plan = workspace.plan(change);
    for line in plan.render() {
        println!("{line}");
    }
    if !options.apply {
        println!("answer: not asked; this is a preview and nothing is written");
        return plan.is_executable();
    }

    let question = TrustQuestion::read(workspace.root());
    println!("{}", question.ask());
    let outcome = crate::project::session::answer(
        &question,
        permissions,
        if options.trust {
            TrustDecision::Trust
        } else {
            TrustDecision::DoNotTrust
        },
    );
    println!("answer: {}", outcome.message());
    if !outcome.trusted() {
        return false;
    }
    if let Err(refusal) = permissions.permit(
        crate::contracts::permission::EffectClass::FileWrite,
        crate::host::permission::Scope::Project(workspace.root()),
        crate::contracts::permission::Principal::Human,
    ) {
        println!("rustage: {}", refusal.message());
        return false;
    }
    match plan.execute() {
        Ok(written) => {
            for line in written.render() {
                println!("{line}");
            }
            println!(
                "no directory was created and none was removed: a member is a line in this \
                 document, and the roots are the person's own"
            );
            true
        }
        Err(failure) => {
            println!("rustage: {}", failure.message());
            false
        }
    }
}

/// Creates a workspace document in a directory, and reads it back.
///
/// It takes no question and no permission, the same way creating a project does not: it is a
/// person's own action, and what it writes is one document that did not exist. The directory has to
/// be there, because a workspace is made by naming roots that are already there.
pub(super) fn new_workspace(options: &crate::cli::NewWorkspaceOptions) -> bool {
    use crate::project::create::{ProjectIds, SystemProjectIds};
    use crate::project::workspace::Workspace;

    let name = options
        .name
        .clone()
        .unwrap_or_else(|| directory_name(&options.directory));
    if name.trim().is_empty() {
        println!(
            "rustage: {} has no name of its own, and this build does not invent one",
            options.directory.display()
        );
        return false;
    }
    let id = SystemProjectIds.next_id();
    println!(
        "workspace: {}",
        options
            .directory
            .join(crate::project::workspace::WORKSPACE_FILE_NAME)
            .display()
    );
    println!("name: {name}, identity: {id}");
    let draft = match Workspace::draft(&options.directory, &name, &id) {
        Ok(draft) => draft,
        Err(error) => {
            println!("rustage: {}", error.message());
            return false;
        }
    };
    match draft.create() {
        Ok(workspace) => {
            println!(
                "created {} ({} member(s))",
                workspace.path().display(),
                workspace.members().len()
            );
            for line in workspace.render() {
                println!("{line}");
            }
            true
        }
        Err(error) => {
            println!("rustage: {}", error.message());
            false
        }
    }
}

/// This machine's workspace state: which member is active, per workspace.
pub(super) fn workspace_state() -> Result<crate::project::workspace::WorkspaceState, String> {
    use crate::project::workspace::WorkspaceState;

    let directories = HostDirectories::from_environment()
        .map_err(|error| format!("this machine's configuration directory is unknown: {error}"))?;
    WorkspaceState::load(
        directories
            .path(HostDirectory::Config)
            .join("workspace-state.json"),
    )
    .map_err(|error| {
        format!(
            "this machine's workspace state could not be read: {}",
            error.message()
        )
    })
}

/// Checks what a project needs, re-scans it when asked, and proves on the way out that nothing was
/// written (task T02.08).
///
/// The check reads the document, the filesystem, the toolchain (only when asked, because reading the
/// toolchain runs two programs) and this machine's credential store. That is the whole of it: there
/// is no writer in this path except `--record`, which writes the scan to the path the caller names,
/// and the last line of the output is the project's own bytes compared with the ones read at the
/// start — so "this did not clear my document" is shown rather than promised.
pub(super) fn validate(options: &crate::cli::ValidateOptions) -> bool {
    use crate::project::validate::{Reach, Scan, Validation};

    let permissions = PermissionService::new();
    let mut session = match ProjectSession::open(&options.path) {
        Ok(session) => session,
        Err(error) => {
            println!("rustage: {}", error.message());
            return false;
        }
    };
    let before = match std::fs::read(session.path()) {
        Ok(bytes) => bytes,
        Err(error) => {
            println!(
                "rustage: {} could not be read: {error}",
                session.path().display()
            );
            return false;
        }
    };
    println!("project: {}", session.path().display());
    println!("root: {}", session.root().as_path().display());
    println!("format: {}", session.file().verdict().message());

    let toolchain = match session.file().document() {
        Some(_) => {
            if options.diagnose {
                println!("{}", session.question().ask());
                if options.trust {
                    let outcome = session.decide(&permissions, TrustDecision::Trust);
                    println!("answer: {}", outcome.message());
                }
                session.diagnose_toolchain(&permissions).clone()
            } else {
                session.toolchain().clone()
            }
        }
        None => {
            // A document this build does not interpret cannot be checked, and this is not an empty
            // report pretending otherwise.
            println!(
                "rustage: {} cannot be checked — {}; the file is untouched and stays readable",
                session.path().display(),
                session.file().verdict().message()
            );
            return stop_without_writing(&session, &before);
        }
    };
    println!("toolchain: {}", toolchain.message());
    println!(
        "identity: {} ({})",
        session.name().unwrap_or("<not readable>"),
        session.id().unwrap_or("<not readable>")
    );

    let store = crate::host::network::default_store();
    let document = session
        .file()
        .document()
        .expect("the document was interpreted above");
    let reach = Reach::of(document, store.as_ref(), SystemTime::now());
    let validation = Validation::of(document, &toolchain, &reach);
    for line in validation.render() {
        println!("{line}");
    }
    // A missing dependency is a finding, not a failure: the project is still open and editable, and
    // saying so is the difference between a report and a repair.
    if !validation.missing().is_empty() {
        println!(
            "missing: {} — the project stays open and editable, and nothing was cleared to make \
             this report simpler",
            validation.missing().len()
        );
    }

    if let Some(record) = options.record.as_deref() {
        if let Err(refusal) = crate::project::validate::may_record_at(session.file(), record) {
            println!("rustage: {}", refusal.message());
            return stop_without_writing(&session, &before);
        }
        // The record is taken from the file as it is now, the same way a re-scan is: a record of
        // what a document was when it was opened is not a record of the project.
        match Scan::read(session.path()) {
            Ok(scan) => match scan.save(record) {
                Ok(path) => println!("recorded scan: {}", path.display()),
                Err(error) => {
                    println!("rustage: {}", error.message());
                    return stop_without_writing(&session, &before);
                }
            },
            Err(error) => {
                println!("rustage: {}", error.message());
                return stop_without_writing(&session, &before);
            }
        }
    }

    if let Some(baseline) = options.rescan.as_deref() {
        let recorded = match Scan::load(baseline) {
            Ok(recorded) => recorded,
            Err(error) => {
                println!("rustage: {}", error.message());
                return stop_without_writing(&session, &before);
            }
        };
        // A re-scan reads the file again rather than reusing the document that was read at the
        // start: a remembered state reported as the current one is the failure this is here to
        // avoid.
        let now = match Scan::read(session.path()) {
            Ok(now) => now,
            Err(error) => {
                println!("rustage: {}", error.message());
                return stop_without_writing(&session, &before);
            }
        };
        match recorded.between(&now) {
            Ok(rescan) => {
                println!("re-scan against {}:", baseline.display());
                for line in rescan.render() {
                    println!("{line}");
                }
            }
            Err(refusal) => {
                println!("rustage: {}", refusal.message());
                return stop_without_writing(&session, &before);
            }
        }
    }

    nothing_written(&session, &before)
}

/// Imports an existing project: what it is, what an import would change, and the migration itself
/// (task T02.07).
///
/// Reading and planning decide nothing and need no permission: the report is what a person looks at
/// before agreeing. `--apply` is the only path that writes, so it is the only one that asks the
/// project's question and takes the write permission.
pub(super) fn import(options: &crate::cli::ImportOptions) -> bool {
    let import = match Import::inspect(&options.path) {
        Ok(import) => import,
        Err(refusal) => {
            println!("rustage: {}", refusal.message());
            return false;
        }
    };
    for line in import.render() {
        println!("{line}");
    }

    if options.check_backup {
        let backups = backups_of(import.path());
        if backups.is_empty() {
            println!("kept files: none; no migration of this project has been done here");
            return true;
        }
        println!("kept files:");
        let mut unreadable = 0usize;
        for path in &backups {
            match Backup::read(path) {
                Ok(backup) => {
                    if !backup.is_readable() {
                        unreadable += 1;
                    }
                    println!("  {}", backup.render());
                }
                Err(reason) => {
                    unreadable += 1;
                    println!("  {reason}");
                }
            }
        }
        if unreadable > 0 {
            println!(
                "rustage: {unreadable} kept file(s) cannot be read back as a document, so this \
                 build cannot go back to them"
            );
            return false;
        }
        return true;
    }

    let candidate = match import.candidate() {
        Ok(candidate) => candidate,
        Err(refusal) => {
            println!("rustage: {}", refusal.message());
            return false;
        }
    };
    for line in candidate.render() {
        println!("{line}");
    }

    if !options.apply {
        println!(
            "nothing was written: this is the plan, and the project on disk is exactly what it was"
        );
        return true;
    }

    let permissions = PermissionService::new();
    let session = match ProjectSession::open(import.path()) {
        Ok(session) => session,
        Err(error) => {
            println!("rustage: {}", error.message());
            return false;
        }
    };
    println!("{}", session.question().ask());
    let outcome = session.decide(
        &permissions,
        if options.trust {
            TrustDecision::Trust
        } else {
            TrustDecision::DoNotTrust
        },
    );
    println!("answer: {}", outcome.message());
    if !outcome.trusted() {
        return false;
    }
    if let Err(refusal) = permissions.permit(
        crate::contracts::permission::EffectClass::FileWrite,
        crate::host::permission::Scope::Project(session.root()),
        crate::contracts::permission::Principal::Human,
    ) {
        println!("rustage: {}", refusal.message());
        return false;
    }

    match import.commit() {
        Ok(imported) => {
            for line in imported.render() {
                println!("{line}");
            }
            true
        }
        Err(failure) => {
            // Every line is printed, including what could not be put back: a migrated project that
            // says it is fine is the one outcome this command must not produce.
            println!("rustage: {}", failure.message());
            println!("recovered: {}", failure.is_recovered());
            false
        }
    }
}

/// Shows which target would build a project, what every value came from, and what this machine
/// overrode (task T02.06).
///
/// Reading a configuration decides nothing and needs no permission: it reads the project document,
/// this machine's configuration file, and whether each credential's material is where the reference
/// says. Recording what was shown is a write — of the host's own state, not the project's, which is
/// why it takes no permission and why every entry it writes carries the project's identity.
pub(super) fn configuration(options: &crate::cli::ConfigurationOptions) -> bool {
    let directories = match HostDirectories::from_environment() {
        Ok(directories) => directories,
        Err(error) => {
            println!("rustage: this machine's configuration directory is unknown: {error}");
            return false;
        }
    };
    let mut machine = match MachineConfiguration::load(
        directories
            .path(HostDirectory::Config)
            .join("configuration.json"),
    ) {
        Ok(machine) => machine,
        Err(error) => {
            println!("rustage: {}", error.message());
            return false;
        }
    };

    let file = match ProjectFile::open(&options.path) {
        Ok(file) => file,
        Err(error) => {
            println!("rustage: {}", error.message());
            return false;
        }
    };
    let Some(document) = file.document() else {
        println!("rustage: the document cannot be interpreted, so it has no configuration");
        return false;
    };
    let identity = document.identity();

    if options.forget {
        let forgotten = machine.forget(identity.id());
        println!(
            "forgot {forgotten} thing(s) recorded for {} ({})",
            identity.name(),
            identity.id()
        );
        return match machine.save() {
            Ok(path) => {
                println!("wrote {}", path.display());
                true
            }
            Err(error) => {
                println!("rustage: {}", error.message());
                false
            }
        };
    }

    // The overrides given on the command line are the ones this run resolves with; recording them is
    // a separate decision, so a person can see what they do before they persist.
    let mut remembered = Vec::new();
    for word in &options.overrides {
        let Some(key) = SettingKey::parse(&word.key) else {
            println!("rustage: `{}` is not a setting", word.key);
            return false;
        };
        let entry = match Override::new(
            identity.id(),
            options.target.clone(),
            key,
            word.value.clone(),
            SystemTime::now(),
        ) {
            Ok(entry) => entry,
            Err(refusal) => {
                println!("rustage: {}", refusal.message());
                return false;
            }
        };
        remembered.push(entry);
    }

    // What this run resolves with: the machine's state as it is, plus the words on the command line.
    let mut effective = machine.clone();
    for entry in &remembered {
        effective.remember(entry.clone());
    }
    let resolved = match ResolvedConfiguration::of(document, options.target.as_deref(), &effective)
    {
        Ok(resolved) => resolved,
        Err(refusal) => {
            println!("rustage: {}", refusal.message());
            return false;
        }
    };

    // Nothing in the project is written either way, so no question is asked: what `--remember`
    // writes is this machine's own state, which is the host's business and not the project's.
    if options.remember {
        println!(
            "project: read only; no question is asked, because what is recorded belongs to this \
             machine and not to the project"
        );
    } else {
        println!("project: read only; nothing is written and no question is asked");
    }

    // The choice recorded for another project is printed rather than used: that is the acceptance,
    // and saying it out loud is what makes it visible.
    match effective.active_for(identity.id()) {
        ActiveVerdict::Chosen(choice) => println!(
            "chosen target: `{}` (chosen for this project on this machine)",
            choice.target()
        ),
        ActiveVerdict::AnotherProject(choice) => println!(
            "chosen target: none for this project; `{}` is chosen for {} ({}), and a choice belongs \
             to one project",
            choice.target(),
            choice.name(),
            choice.project()
        ),
        ActiveVerdict::NeverChosen => {
            println!("chosen target: none has been chosen for this project on this machine")
        }
    }
    for line in resolved.render() {
        println!("{line}");
    }
    // The store is the environment, which is what this host has; a credential whose material lives in
    // the system keyring is reported as unavailable rather than as missing, and never as a value.
    let store = crate::host::network::default_store();
    let views =
        crate::project::configuration::credentials(document, store.as_ref(), SystemTime::now());
    if views.is_empty() {
        println!("credentials: no service of this project names one");
    } else {
        for view in views {
            println!("  credential: {}", view.render());
        }
    }

    if !options.remember {
        return true;
    }
    if options.target.is_none() && remembered.is_empty() {
        println!("rustage: --remember needs a --target or an --override to record");
        return false;
    }

    if let Some(target) = options.target.as_deref() {
        machine.set_active(identity.id(), identity.name(), target, SystemTime::now());
    }
    for entry in remembered {
        machine.remember(entry);
    }
    match machine.save() {
        Ok(path) => {
            println!("recorded for {} ({}):", identity.name(), identity.id());
            for line in machine.render() {
                println!("  {line}");
            }
            println!("wrote {}", path.display());
            true
        }
        Err(error) => {
            println!("rustage: {}", error.message());
            false
        }
    }
}
