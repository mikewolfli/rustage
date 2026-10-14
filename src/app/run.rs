// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! What rustage does when it runs: one arm per parsed command.
//!
//! The designer itself is not implemented. What *is* implemented and reachable here is the icon
//! strip in [`toolbar`](super::toolbar) — which exists because the host's icons need a real
//! consumer before anything downstream can claim to be verified: the resource tooling proves a
//! glyph is a glyph, and only a control can prove the glyph reaches a screen — and the dependency
//! contract probes under `crate::probe`, and the host's own platform services under
//! `crate::host::readiness`.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::cli::{self, Command};
use crate::host::paths::{HostDirectories, HostDirectory};
use crate::host::permission::PermissionService;
use crate::host::startup::StartupRequest;
use crate::probe::report::ProbeReport;
use crate::project::configuration::{
    ActiveVerdict, MachineConfiguration, Override, ResolvedConfiguration, SettingKey,
};
use crate::project::create::{Creation, NewProjectRequest, SystemProjectIds};
use crate::project::entries::{ProjectEntries, ProjectTree};
use crate::project::files::FilePlan;
use crate::project::format::{ProjectContent, ProjectFile};
use crate::project::import::{Backup, Import, backups_of};
use crate::project::recent::RecentProjects;
use crate::project::session::{ProjectSession, SessionState, TrustDecision};

use super::toolbar;

/// Executes a parsed command.
pub fn run(command: Command) {
    match command {
        Command::Help => print!("{}", cli::help_text()),
        Command::Version => println!("rustage {}", env!("CARGO_PKG_VERSION")),
        Command::Status => {
            println!(
                "rustage {}\n\
                 The desktop designer is not implemented yet.\n\
                 The icon strip is: run `rustage --render-icons <directory>` to render it.\n\
                 The dependency contract probes are: run `rustage --probe-apis <directory>`.",
                env!("CARGO_PKG_VERSION")
            );
            // The contract vocabulary is derived from the code the binary was built with, so this
            // section cannot claim a policy the build does not implement.
            println!("{}", crate::contracts::vocabulary::version_line());
            println!("Permission policy for an untrusted project with no approvals:");
            for line in crate::contracts::vocabulary::permission_lines() {
                println!("{line}");
            }
            println!("External effects, and what decides each of them:");
            for line in crate::contracts::vocabulary::effect_lines() {
                println!("{line}");
            }
        }
        Command::Launch(options) => {
            let request = StartupRequest {
                exit_after: options.exit_after,
                ..StartupRequest::default()
            };
            if !options.exit_after.is_zero() {
                // Say what will happen before it does: a bounded session ends the process from a
                // watcher thread, because the pinned revision cannot be asked to stop its loop.
                println!(
                    "launching the baseline session; it will end itself after {} ms",
                    options.exit_after.as_millis(),
                );
            }
            let report = crate::host::startup::launch(&request);
            // The report names the backend and the GUI mode in both directions, because "it
            // started" without a window is the failure this command exists to avoid claiming.
            print!("{}", report.summary());
            println!();
            crate::host::log::flush();
            if !report.is_started() {
                std::process::exit(report.exit_code().code());
            }
        }
        Command::RenderIcons(directory) => match render_icons(&directory) {
            Ok(written) => {
                for path in written {
                    println!("{}", path.display());
                }
            }
            Err(error) => {
                eprintln!("rustage: could not render the icon strip: {error}");
                std::process::exit(1);
            }
        },
        Command::ProbeApis(directory) => match write_probe_report(&directory) {
            Ok((path, report)) => {
                print!("{}", report.render_text());
                println!("{}", path.display());
                if !report.is_clean() {
                    // A gap is a finding, not a pass; say so in the exit status so a caller that
                    // only checks the code cannot mistake one for success.
                    std::process::exit(1);
                }
            }
            Err(error) => {
                eprintln!("rustage: could not write the probe report: {error}");
                std::process::exit(1);
            }
        },
        Command::NewProject(options) => {
            let request = match options.name {
                Some(name) => {
                    NewProjectRequest::new(options.directory, options.template).with_name(name)
                }
                None => NewProjectRequest::new(options.directory, options.template),
            };
            if !create_project(request) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::OpenProject(options) => {
            if !open_project(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::ProjectTree(path) => {
            if !show_tree(&path) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::DropEntry(options) => {
            if !drop_entry(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::FileOp(options) => {
            if !file_op(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Configuration(options) => {
            if !configuration(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Edit(options) => {
            if !edit(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Document(options) => {
            if !document(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::History(options) => {
            if !history(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Dependencies(options) => {
            if !dependencies(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Kit(options) => {
            if !kit(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Workspace(options) => {
            if !workspace(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::NewWorkspace(options) => {
            if !new_workspace(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Validate(options) => {
            if !validate(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Import(options) => {
            if !import(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Recent => {
            if !list_recent() {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::ProjectCheck(path) => {
            if !check_project(&path) {
                // The project is not something this build can use as it stands, and the reason was
                // reported above. `Reported` says a real finding was found and printed.
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::ProbeHost(directory) => match write_host_report(&directory) {
            Ok((path, report)) => {
                print!("{}", report.render_text());
                println!("{}", path.display());
                if !report.is_ready() {
                    // A missing capability and a check this environment cannot run are both "not a
                    // pass". `Reported` says exactly that: the command found a real finding and
                    // reported it, which is a different number from a defect in the host.
                    crate::host::log::flush();
                    std::process::exit(crate::host::error::ExitCode::Reported.code());
                }
            }
            Err(error) => {
                eprintln!("rustage: could not write the host readiness report: {error}");
                std::process::exit(1);
            }
        },
    }
}

/// Where this machine keeps the list of projects that were opened before.
fn recent_list() -> Result<RecentProjects, String> {
    let directories = HostDirectories::from_environment()
        .map_err(|error| format!("this machine's configuration directory is unknown: {error}"))?;
    RecentProjects::load(
        directories
            .path(HostDirectory::Config)
            .join("recent-projects.json"),
    )
    .map_err(|error| format!("the recent list could not be read: {}", error.message()))
}

/// Opens a project: prints what it is, asks the question, and does nothing else unless asked.
///
/// Returns whether the project was opened.
fn open_project(options: &crate::cli::OpenProjectOptions) -> bool {
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

/// Shows what a project has, classified, and which entry it starts with.
fn show_tree(path: &Path) -> bool {
    let session = match ProjectSession::open(path) {
        Ok(session) => session,
        Err(error) => {
            println!("rustage: {}", error.message());
            return false;
        }
    };
    let Some(document) = session.file().document() else {
        println!(
            "rustage: {} is {}, so what it holds cannot be classified",
            session.path().display(),
            session.file().verdict().message()
        );
        return false;
    };
    let root = session.root().as_path().to_string_lossy().into_owned();
    let tree = ProjectTree::of(document, Some(&root));
    for line in tree.render() {
        println!("{line}");
    }
    println!("entries: {}", ProjectEntries::of(document).len());
    for line in ProjectEntries::of(document).render() {
        println!("  {line}");
    }
    println!("format: {}", session.file().verdict().message());
    if let Some(startup) = document.startup_written() {
        println!("startup: {startup}");
    } else {
        println!("startup: none chosen");
    }
    true
}

/// Removes an entry, fixing what points at it or refusing to remove it.
///
/// The removal goes through the same decisions every other effect goes through: the project has to
/// be trusted (a person's answer, `--trust`) and writing a file needs the permission the decision
/// table gives it. Nothing is written when either says no.
fn drop_entry(options: &crate::cli::DropEntryOptions) -> bool {
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
fn file_op(options: &crate::cli::FileOpOptions) -> bool {
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

/// Ends a check that refused something.
///
/// The last line is printed either way, and the answer is *not* that line: a check that could not do
/// what it was asked returns a failure even though the document is untouched, because "nothing was
/// written" describes the file and not what the command achieved. Reporting the two as one answer is
/// how a refusal comes out as a success with a reassuring last line.
fn stop_without_writing(session: &ProjectSession, before: &[u8]) -> bool {
    if !nothing_written(session, before) {
        println!("rustage: and the document is not what it was, which it must be");
    }
    false
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
fn edit(options: &crate::cli::EditOptions) -> bool {
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

/// The path of a file inside a project root, as the root's own relative path rules want it.
fn relative_to(
    root: &crate::host::paths::ProjectRoot,
    file: &std::path::Path,
) -> std::path::PathBuf {
    if let Ok(relative) = file.strip_prefix(root.as_path()) {
        return relative.to_path_buf();
    }
    file.canonicalize()
        .ok()
        .and_then(|absolute| {
            absolute
                .strip_prefix(root.as_path())
                .ok()
                .map(std::path::Path::to_path_buf)
        })
        .unwrap_or_else(|| file.to_path_buf())
}

/// Shows what a buffer over a page document is holding (task T03.01).
///
/// Nothing is written and nothing is run, so there is no question to ask and no permission to take:
/// the command reads the file, types the edits in the order they were given, and reports. What it
/// reports is the whole point of the task — the text as it was typed (an invalid text is kept), the
/// last model that read (still previewable, and saying which text it came from), and what happened
/// to identity. The exit code says whether the newest text reads; a text that does not read is a
/// finding, not a crash, and the whole report is still printed.
fn document(options: &crate::cli::DocumentOptions) -> bool {
    use crate::project::document::{Buffer, root_of};

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
fn history(options: &crate::cli::HistoryOptions) -> bool {
    use crate::cli::HistoryEdit;
    use crate::contracts::permission::PermissionGate;
    use crate::project::batch::{Batch, Fingerprint, Plan, apply, store_of};
    use crate::project::document::Buffer;
    use crate::project::history::{Domain, History, View, batch_edit_of, text_edit_of};

    let root = match options.root.as_ref() {
        Some(directory) => match crate::host::paths::ProjectRoot::new(directory) {
            Ok(root) => root,
            Err(error) => {
                println!("rustage: {}", error.message());
                return false;
            }
        },
        None => match crate::project::document::root_of(&options.file) {
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

/// Says what recording an entry did beyond recording it.
///
/// Both things it can do are worth saying out loud: an entry absorbed into a merge window is not a
/// step of its own, and an entry recorded after an undo *drops* the steps that were undone, which is
/// the one part of a history a person cannot see from the document itself.
fn report_recording(recording: &crate::project::history::Recording) {
    if recording.merged {
        println!("  absorbed into the step the merge window opened");
    }
    if recording.dropped > 0 {
        println!(
            "  {} undone step(s) were dropped: an edit after an undo is a new branch",
            recording.dropped
        );
    }
}

/// Takes back or puts back up to `times` steps, and says whether every one of them happened.
///
/// The move is asked for in the domain the caller named, and the history's own answer is printed
/// either way: a refusal names the step in the other domain that would have been skipped, and a
/// refusal is why the answer is false — a caller that asked for three steps and got two has not been
/// obeyed, and saying "two" while exiting zero would be reporting the wrong thing.
fn move_steps(
    history: &mut crate::project::history::History,
    store: &mut crate::contracts::store::DocumentStore,
    buffer: &mut crate::project::document::Buffer,
    root: &crate::host::paths::ProjectRoot,
    redo: bool,
    times: usize,
    only: Option<crate::cli::HistoryDomain>,
) -> bool {
    use crate::cli::HistoryDomain;
    let mut all = true;
    for _ in 0..times {
        let step = match (redo, only) {
            (false, None) => history.undo(store, buffer, root),
            (true, None) => history.redo(store, buffer, root),
            (false, Some(HistoryDomain::Model)) => history.undo_model(store),
            (true, Some(HistoryDomain::Model)) => history.redo_model(store),
            (false, Some(HistoryDomain::Text)) => history.undo_text(buffer, root),
            (true, Some(HistoryDomain::Text)) => history.redo_text(buffer, root),
        };
        for line in step.render() {
            println!("{line}");
        }
        if !step.did_move() {
            all = false;
            break;
        }
    }
    all
}

/// The merge window's closing, in one sentence.
fn close_merge(state: crate::project::history::MergeState) -> String {
    use crate::project::history::MergeState;

    match state {
        MergeState::NothingOpen => "there was no window open".to_owned(),
        MergeState::Open(merge) => format!(
            "the window is still open after {} recording(s)",
            merge.recorded()
        ),
        MergeState::Closed { entries, domain } => format!(
            "the {entries} {} recording(s) in it are one step, so one undo takes them all back",
            domain.name()
        ),
    }
}

/// Resolves a workspace's dependencies, and says exactly what it did (task T02.11).
///
/// The order here *is* the guarantee: the resolution is decided (a command and its effects), it is
/// shown, every effect is mapped to the class the permission table uses, each class is authorized,
/// and only then is anything started. Nothing is inferred from "it is only cargo": the effects come
/// from the resolution that was asked for, and a caller who did not authorize one does not get it.
fn dependencies(options: &crate::cli::DependenciesOptions) -> bool {
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
fn packages_of(lockfile: &crate::project::dependencies::Lockfile) -> String {
    match lockfile.packages() {
        Some(packages) => format!("{packages} package(s)"),
        None => "absent".to_owned(),
    }
}

fn kit(options: &crate::cli::KitOptions) -> bool {
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
fn ask_the_program(
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
fn workspace(options: &crate::cli::WorkspaceOptions) -> bool {
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
fn change_members(
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
fn new_workspace(options: &crate::cli::NewWorkspaceOptions) -> bool {
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

/// A directory's own name, when it has one.
fn directory_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// This machine's workspace state: which member is active, per workspace.
fn workspace_state() -> Result<crate::project::workspace::WorkspaceState, String> {
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
fn validate(options: &crate::cli::ValidateOptions) -> bool {
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

/// The last line of a check: the document is byte for byte what it was when the check began.
///
/// It is printed on every path out, including the ones that refused something, because "the file was
/// not touched" is exactly what a person needs to know before they trust the report.
fn nothing_written(session: &ProjectSession, before: &[u8]) -> bool {
    match std::fs::read(session.path()) {
        Ok(after) if after == before => {
            println!(
                "unchanged: {} is byte for byte what it was ({} byte(s))",
                session.path().display(),
                after.len()
            );
            true
        }
        Ok(after) => {
            println!(
                "rustage: {} changed while it was being checked ({} byte(s) before, {} now)",
                session.path().display(),
                before.len(),
                after.len()
            );
            false
        }
        Err(error) => {
            println!(
                "rustage: {} could not be read back: {error}",
                session.path().display()
            );
            false
        }
    }
}

/// Imports an existing project: what it is, what an import would change, and the migration itself
/// (task T02.07).
///
/// Reading and planning decide nothing and need no permission: the report is what a person looks at
/// before agreeing. `--apply` is the only path that writes, so it is the only one that asks the
/// project's question and takes the write permission.
fn import(options: &crate::cli::ImportOptions) -> bool {
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
fn configuration(options: &crate::cli::ConfigurationOptions) -> bool {
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

/// Lists the projects opened before, and says which of them are still there.
fn list_recent() -> bool {
    let mut recent = match recent_list() {
        Ok(recent) => recent,
        Err(reason) => {
            println!("rustage: {reason}");
            return false;
        }
    };
    println!("recent projects: {}", recent.path().display());
    if recent.is_empty() {
        println!("  none have been opened yet");
        return true;
    }
    // Looking at the list notices which entries are gone, and remembers it: a person who comes back
    // later should see "gone" rather than a silently missing line.
    let entries: Vec<_> = recent.entries().to_vec();
    let mut missing = 0usize;
    for entry in &entries {
        let state = recent.state_of(entry);
        if state == crate::project::recent::RecentState::Missing {
            missing += 1;
        }
    }
    for line in recent.render() {
        println!("  {line}");
    }
    if missing > 0 {
        println!(
            "  {missing} entry/entries are not there any more; they can be repaired or forgotten"
        );
    }
    match recent.save() {
        Ok(_) => true,
        Err(error) => {
            println!(
                "rustage: the recent list was not saved: {}",
                error.message()
            );
            false
        }
    }
}

/// Creates a project, printing the last step's plan before it does anything.
///
/// Returns whether a project was created.
fn create_project(request: NewProjectRequest) -> bool {
    let mut creation = match Creation::plan(request, &SystemProjectIds) {
        Ok(creation) => creation,
        Err(refusal) => {
            println!("rustage: {}", refusal.message());
            return false;
        }
    };
    let plan = creation.summary();
    println!("project: {}", plan.root().display());
    println!(
        "template: {}, name: {}, identity: {}",
        plan.template().name(),
        plan.name(),
        plan.id()
    );
    println!("files: {}", plan.files().len());
    for file in plan.files() {
        println!("  {:<24} {}", file.path(), file.what());
    }
    println!("if it fails:");
    for line in plan.impact() {
        println!("  {line}");
    }

    let staged = match creation.stage() {
        Ok(_) => true,
        Err(failure) => {
            println!("rustage: {}", failure.message());
            false
        }
    };
    if !staged {
        return false;
    }
    match creation.commit() {
        Ok(created) => {
            println!(
                "created {} ({} file(s)){}",
                created.root().display(),
                created.files().len(),
                if created.target_preexisted() {
                    ", in the directory that was already there"
                } else {
                    ""
                }
            );
            // The created project is read back with the readers the designer uses, so "a project was
            // created" and "a project can be opened" are the same statement.
            match created.open() {
                Ok(file) => {
                    println!("read back: {}", file.verdict().message());
                    true
                }
                Err(error) => {
                    println!(
                        "rustage: what was created cannot be read back: {}",
                        error.message()
                    );
                    false
                }
            }
        }
        Err(failure) => {
            println!("rustage: {}", failure.message());
            false
        }
    }
}

/// Reads a project file and says what this build makes of it.
///
/// The point of the command is the acceptance line of the project format: a version this build does
/// not know is never written, and nothing in a file this build does know is dropped. So the command
/// does not stop at reading: it writes the document back through this build's own writer and reads
/// that file again, which turns "nothing was dropped" into something a person can check rather than
/// something a comment claims.
///
/// Returns whether the project came through cleanly.
fn check_project(path: &Path) -> bool {
    let file = match ProjectFile::open(path) {
        Ok(file) => file,
        Err(error) => {
            println!("rustage: {}", error.message());
            return false;
        }
    };
    println!("project: {}", file.path().display());
    println!("root: {}", file.root().as_path().display());
    println!("format: {}", file.verdict().message());
    match file.content() {
        ProjectContent::Interpreted(document) => {
            println!(
                "identity: {} ({})",
                document.identity().name(),
                document.identity().id()
            );
            println!(
                "pages: {}, resources: {}, targets: {}, generated: {}, services: {}",
                document.pages().len(),
                document.resources().len(),
                document.targets().len(),
                document.generated().len(),
                document.services().len()
            );
            let unknown = document.unknown_fields();
            println!(
                "members this build keeps without reading: {}",
                unknown.len()
            );
            for field in unknown {
                println!("  {field}");
            }
            let references = document.references();
            println!("references: {}", references.len());
            for reference in references {
                println!("  {}", reference.render());
            }
        }
        ProjectContent::Foreign(foreign) => {
            match foreign.identity() {
                Some(identity) => println!(
                    "identity: {} ({}) — read best effort from a layout this build does not know",
                    identity.name(),
                    identity.id()
                ),
                None => println!("identity: not readable without interpreting the document"),
            }
            println!("members: {}", foreign.members().join(", "));
            match file.writer() {
                Err(refusal) => println!("round trip: refused — {}", refusal.message()),
                Ok(_) => println!(
                    "round trip: nothing written — the version is not one this build writes"
                ),
            }
            return false;
        }
    }

    let roundtrip = path.with_extension("roundtrip.json");
    let writer = match file.writer() {
        Ok(writer) => writer,
        Err(refusal) => {
            println!("round trip: refused — {}", refusal.message());
            return false;
        }
    };
    let written = match writer.save_to(&roundtrip) {
        Ok(written) => written,
        Err(error) => {
            println!("rustage: {}", error.message());
            return false;
        }
    };
    let before = file
        .content()
        .document()
        .expect("the document was interpreted above");
    match ProjectFile::open(&written) {
        Ok(again) => {
            let after = again
                .content()
                .document()
                .expect("a version this build wrote");
            let same_document = after == before;
            let same_unknown = again.unknown_fields() == file.unknown_fields();
            println!(
                "round trip: wrote {}, read back: document unchanged: {}, members kept: {}",
                written.display(),
                yes_no(same_document),
                yes_no(same_unknown)
            );
            same_document && same_unknown
        }
        Err(error) => {
            println!(
                "rustage: the round trip could not be read back: {}",
                error.message()
            );
            false
        }
    }
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

/// Writes the icon strip's baseline renders, reporting the files written.
fn render_icons(directory: &Path) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    std::fs::create_dir_all(directory)?;
    let mut written = Vec::new();
    for render in toolbar::BASELINE_RENDERS {
        let svg = toolbar::render_baseline(render)?;
        let path = directory.join(format!("{}.svg", render.name));
        std::fs::write(&path, svg)?;
        written.push(path);
    }
    Ok(written)
}

/// Runs the host capability checks and writes the report, returning where it went.
///
/// Separate from [`write_probe_report`] because the two answer different questions: that one is
/// about the pinned dependency's API contracts, this one about the host's own services in the
/// environment the host is running in. The artifact is written even when a check is not ready, for
/// the same reason: the finding is the output.
fn write_host_report(
    directory: &Path,
) -> Result<(PathBuf, crate::host::readiness::ReadinessReport), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(directory)?;
    let report = crate::host::readiness::ReadinessReport::run();
    let path = directory.join("host-readiness.json");
    std::fs::write(&path, report.render_json())?;
    Ok((path, report))
}

/// Runs the dependency contract probes and writes the report, returning where it went.
///
/// The JSON artifact is always written, even when a probe reports a gap: the finding is the output,
/// and suppressing the artifact on failure would destroy the evidence a reviewer needs to read it.
fn write_probe_report(
    directory: &Path,
) -> Result<(PathBuf, ProbeReport), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(directory)?;
    let report = ProbeReport::run();
    let path = directory.join("api-probes.json");
    std::fs::write(&path, report.render_json())?;
    Ok((path, report))
}
