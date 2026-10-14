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

use crate::cli::{self, Command};
use crate::host::paths::{HostDirectories, HostDirectory};
use crate::host::permission::PermissionService;
use crate::host::startup::StartupRequest;
use crate::probe::report::ProbeReport;
use crate::project::create::{Creation, NewProjectRequest, SystemProjectIds};
use crate::project::format::{ProjectContent, ProjectFile};
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
