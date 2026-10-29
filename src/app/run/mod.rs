// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! What rustage does when it runs: one arm per parsed command.
//!
//! The designer itself is not implemented. What *is* implemented and reachable here is the icon
//! strip in [`toolbar`](super::shell::toolbar) — which exists because the host's icons need a real
//! consumer before anything downstream can claim to be verified: the resource tooling proves a
//! glyph is a glyph, and only a control can prove the glyph reaches a screen — and the dependency
//! contract probes under `crate::probe`, and the host's own platform services under
//! `crate::host::readiness`.
//!
//! * The command handlers live in one sibling module per domain — `document`, `project`,
//!   `workbench`, `editor`, `language`, `search`, `design`, `layout`, `style`, `data_events`,
//!   `preview` — and this file keeps only the dispatcher and the helpers more than one domain
//!   needs. One 13,000-line file cannot be reviewed, and the domains do not share state, so the
//!   split is the domain boundary rather than an arbitrary line count.
//! * A handler is `pub(super)` and re-exported by a glob import, so a domain that needs another
//!   domain's parser can name it without the two modules having to be merged back together.

use super::shell::toolbar;
use crate::cli::{self, Command};
use crate::host::paths::{HostDirectories, HostDirectory};
use crate::host::permission::PermissionService;
use crate::host::startup::StartupRequest;
use crate::probe::report::ProbeReport;
use crate::project::build::configuration::{
    ActiveVerdict, MachineConfiguration, Override, ResolvedConfiguration, SettingKey,
};
use crate::project::lifecycle::create::{Creation, NewProjectRequest, SystemProjectIds};
use crate::project::lifecycle::import::{Backup, Import, backups_of};
use crate::project::lifecycle::recent::RecentProjects;
use crate::project::lifecycle::session::{ProjectSession, SessionState, TrustDecision};
use crate::project::record::entries::{ProjectEntries, ProjectTree};
use crate::project::record::files::FilePlan;
use crate::project::record::format::{ProjectContent, ProjectFile};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

mod ai;
mod application;
mod build;
mod collection;
mod component;
mod data_events;
mod debug;
mod design;
mod document;
mod editor;
mod extension;
mod form;
mod generate;
mod gui_smoke;
mod language;
mod layout;
mod locale;
mod logic;
mod preview;
mod project;
mod quality;
mod release;
mod search;
mod settings;
mod style;
mod target;
mod target_i18n;
mod vcs;
mod workbench;
mod workbench_debug;
mod workbench_events;
mod workbench_gui;
mod workbench_inspect;
mod workbench_layout;
mod workbench_pages;
mod workbench_palette;
mod workbench_property;
mod workbench_scene;
use application::*;
use build::*;
use collection::*;
use component::*;
use data_events::*;
use design::*;
use document::*;
use editor::*;
use form::*;
use generate::*;
use language::*;
use layout::*;
use locale::*;
use logic::*;
use preview::*;
use project::*;
use search::*;
use style::*;
use target::*;
use workbench::*;

/// Executes a parsed command.
pub fn run(command: Command) {
    match command {
        Command::Help => print!("{}", cli::help_text()),
        Command::Version => println!("{}", version_line()),
        Command::Status => {
            println!("{}", version_line());
            println!("{}", line("shell.status.not-implemented"));
            println!("{}", line("shell.status.icon-strip"));
            println!("{}", line("shell.status.probes"));
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
                smoke: options.smoke,
                ime: options.ime,
                smoke_text: options.smoke_text,
                ..StartupRequest::default()
            };
            let bridge = if options.smoke_worker {
                match gui_smoke::Bridge::start() {
                    Ok(bridge) => {
                        crate::host::smoke::install(bridge.sender());
                        Some(bridge)
                    }
                    Err(reason) => {
                        crate::host::log::flush();
                        eprintln!("rustage: gui smoke: {reason}");
                        std::process::exit(crate::host::error::ExitCode::Reported.code());
                    }
                }
            } else {
                None
            };
            if !options.exit_after.is_zero() {
                // Say what will happen before it does: a bounded session asks the dependency to stop
                // its own loop at the deadline (from a watcher thread; the dependency's `quit()`
                // records the request and the loop stops at its next frame).
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
            crate::host::smoke::clear();
            if let Some(bridge) = bridge {
                bridge.finish();
            }
            crate::host::log::flush();
            // The exit code is the report's, not just "did it start": a requested smoke that saw no
            // click is a reported failure even though a window existed.
            let code = report.exit_code();
            if code.code() != 0 {
                std::process::exit(code.code());
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
                    // A gap is a finding, not a pass; say so in the exit status so a caller that only
                    // checks the code cannot mistake one for success. The code is the shared one for
                    // "a real finding was reported", which is what `--probe-host` also exits with —
                    // two probes whose findings meant two different numbers was a distinction nobody
                    // could act on (recorded in the fifth development record §10.9).
                    std::process::exit(crate::host::error::ExitCode::Reported.code());
                }
            }
            Err(error) => {
                eprintln!("rustage: could not write the probe report: {error}");
                // Not `Reported`: the report could not be produced at all, so there is no finding
                // for a caller to read.
                std::process::exit(crate::host::error::ExitCode::Failed.code());
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
        Command::Save(options) => {
            if !save(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Conflict(options) => {
            if !conflict(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Recovery(options) => {
            if !recovery(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Close(options) => {
            if !close(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Invoke(options) => {
            if !resolve_command(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Keys(options) => {
            if !keys(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Menu => {
            if !menu() {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Palette(options) => {
            if !palette(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::WorkbenchGui(options) => {
            if !workbench_gui::workbench_gui(
                options.layout.clone(),
                options.exit_after,
                options.smoke,
            ) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Workbench(options) => {
            if !workbench(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Operation(options) => {
            if !operation(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Audit => {
            if !audit() {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Tabs(options) => {
            if !tabs(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Groups(options) => {
            if !groups(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Clipboard(options) => {
            if !clipboard(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Layout(options) => {
            if !layout(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Windows(options) => {
            if !windows(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Chrome(options) => {
            if !chrome(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Text(options) => {
            if !text(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Highlight(options) => {
            if !highlight(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Coordinates(options) => {
            if !coordinates(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::EditOps(options) => {
            if !edit_ops(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::EditorView(options) => {
            if !editor_view(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::WriteGuard(options) => {
            if !write_guard(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Scale(options) => {
            if !scale(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Snippet(options) => {
            if !snippet(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Regression(options) => {
            if !regression(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Diff(options) => {
            if !diff_command(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Structural(options) => {
            if !structural(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Language(options) => {
            if !language(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Protocol(options) => {
            if !protocol(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Features(options) => {
            if !features(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Navigation(options) => {
            if !navigation(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Diagnostics(options) => {
            if !diagnostics(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::WorkspaceEdit(options) => {
            if !workspace_edit(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Schema(options) => {
            if !schema(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Health(options) => {
            if !health(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Hints(options) => {
            if !hints(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Analysis(options) => {
            if !analysis(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Index(options) => {
            if !index_command(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Search(options) => {
            if !search(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::QuickOpen(options) => {
            if !quick_open(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Replace(options) => {
            if !replace(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Batch(options) => {
            if !batch(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::HistoryNav(options) => {
            if !history_nav(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::SearchHealth(options) => {
            if !search_health(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Catalog(options) => {
            if !catalog(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Scene(options) => {
            if !scene(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Structure(options) => {
            if !structure(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Viewport(options) => {
            if !viewport(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Drag(options) => {
            if !drag(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Objects(options) => {
            if !objects(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Design(options) => {
            if !design(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::LayoutPlan(options) => {
            if !layout_plan(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::LayoutSwitch(options) => {
            if !layout_switch(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Constraints(options) => {
            if !constraints(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Arrange(options) => {
            if !arrange(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Frames(options) => {
            if !frames(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::FocusOrder(options) => {
            if !focus_order(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Variants(options) => {
            if !variants(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::LayoutProbe(options) => {
            if !layout_probe(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::PropertyEditor(options) => {
            if !property_editor(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::MultiProperty(options) => {
            if !multi_property(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Css(options) => {
            if !css(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Tokens(options) => {
            if !tokens(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::States(options) => {
            if !states(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Motion(options) => {
            if !motion(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Style(options) => {
            if !style(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::StyleBoundary(options) => {
            if !style_boundary(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Data(options) => {
            if !data(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Binding(options) => {
            if !binding(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Events(options) => {
            if !events(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Dispatch(options) => {
            if !dispatch_actions(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Routes(options) => {
            if !routes(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Machine(options) => {
            if !machine(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Adapter(options) => {
            if !adapter(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::SessionLog(options) => {
            if !session_log(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Preview(options) => {
            if !preview(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::HotUpdate(options) => {
            if !hot_update(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::StateKeep(options) => {
            if !state_keep(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Matrix(options) => {
            if !matrix(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Inspect(options) => {
            if !inspect(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Visual(options) => {
            if !visual(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::PreviewRecovery(options) => {
            if !preview_recovery(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Load(options) => {
            if !load(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Artifact(options) => {
            if !artifact(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Generate(options) => {
            if !generate(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Mapping(options) => {
            if !mapping(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::GenMerge(options) => {
            if !gen_merge(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Reverse(options) => {
            if !reverse(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Position(options) => {
            if !position(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Export(options) => {
            if !export(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Golden(options) => {
            if !golden(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Toolchain(options) => {
            if !toolchain(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Compile(options) => {
            if !compile(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::BuildDiag(options) => {
            if !build_diag(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Run(options) => {
            if !run_program(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Tasks(options) => {
            if !tasks(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Terminal(options) => {
            if !terminal(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::External(options) => {
            if !external(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::E2e(options) => {
            if !e2e(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Dag(options) => {
            if !dag(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Reproducible(options) => {
            if !reproducible(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Headless(options) => {
            if !headless(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Component(options) => {
            if !component(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::ComponentAuthor(options) => {
            if !component_author(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::ComponentOverride(options) => {
            if !component_override(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::CustomWidget(options) => {
            if !custom_widget(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::ResourceImport(options) => {
            if !resource_import(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::ResourceTarget(options) => {
            if !resource_target(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::AssetPackage(options) => {
            if !asset_package(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::ComponentUpgrade(options) => {
            if !component_upgrade(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::AssetManifest(options) => {
            if !asset_manifest(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::AppShell(options) => {
            if !app_shell(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::AppAction(options) => {
            if !app_action(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::AppMenu(options) => {
            if !app_menu(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::AppToolbar(options) => {
            if !app_toolbar(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::AppContext(options) => {
            if !app_context(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::AppDialog(options) => {
            if !app_dialog(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::AppDock(options) => {
            if !app_dock(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::AppRegression(options) => {
            if !app_regression(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Collection(options) => {
            if !collection(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Table(options) => {
            if !table(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Tree(options) => {
            if !tree(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::CellEdit(options) => {
            if !cell_edit(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Container(options) => {
            if !container(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::RichContent(options) => {
            if !rich_content(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::View(options) => {
            if !view(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::CollectionRegression(options) => {
            if !collection_regression(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Form(options) => {
            if !form(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::LogicCore(options) => {
            if !logic_core(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::LogicIpc(options) => {
            if !logic_ipc(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::LogicActions(options) => {
            if !logic_actions(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::LogicCandidate(options) => {
            if !logic_candidate(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::LogicMigration(options) => {
            if !logic_migration(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::LogicSwap(options) => {
            if !logic_swap(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::LogicCombination(options) => {
            if !logic_combination(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::LogicRegression(options) => {
            if !logic_regression(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::LogicWorker(options) => {
            if !logic_worker(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::LogicProcess(options) => {
            if !logic_process(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::TargetConfig(options) => {
            if !target_config(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::TargetCapability(options) => {
            if !target_capability(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::TargetMatrix(options) => {
            if !target_matrix(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::TargetMobile(options) => {
            if !target_mobile(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::TargetRestricted(options) => {
            if !target_restricted(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::TargetLocal(options) => {
            if !target_local(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::TargetQueue(options) => {
            if !target_queue(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::TargetCertification(options) => {
            if !target_certification(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Debug(options) => {
            if !debug::debug(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Focus(options) => {
            if !focus(&options) {
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
        Command::Vcs(options) => {
            if !vcs::vcs(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Quality(options) => {
            if !quality::quality(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Extension(options) => {
            if !extension::extension(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Ai(options) => {
            if !ai::ai(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Settings(options) => {
            if !settings::settings(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::Release(options) => {
            if !release::release(&options) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
        Command::TargetI18n(options) => {
            if !target_i18n::target_i18n(&options) {
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
        Command::LocaleReport(directory) => {
            if !locale_report(&directory) {
                crate::host::log::flush();
                std::process::exit(crate::host::error::ExitCode::Reported.code());
            }
        }
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

/// The path of a file inside a project root, as the root's own relative path rules want it.
fn relative_to(
    root: &crate::host::paths::ProjectRoot,
    file: &std::path::Path,
) -> std::path::PathBuf {
    // A relative argument is resolved against the working directory first, because the root is
    // canonical and a relative path cannot be compared with it. The file need not exist: a deleted
    // document is exactly the case `--conflict` has to name, and canonicalizing would fail there.
    let absolute = if file.is_absolute() {
        file.to_path_buf()
    } else {
        std::path::absolute(file).unwrap_or_else(|_| file.to_path_buf())
    };
    if let Ok(relative) = absolute.strip_prefix(root.as_path()) {
        return relative.to_path_buf();
    }
    if let Ok(canonical) = absolute.canonicalize()
        && let Ok(relative) = canonical.strip_prefix(root.as_path())
    {
        return relative.to_path_buf();
    }
    file.to_path_buf()
}

/// Says what recording an entry did beyond recording it.
///
/// Both things it can do are worth saying out loud: an entry absorbed into a merge window is not a
/// step of its own, and an entry recorded after an undo *drops* the steps that were undone, which is
/// the one part of a history a person cannot see from the document itself.
fn report_recording(recording: &crate::project::document::history::Recording) {
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
    history: &mut crate::project::document::history::History,
    store: &mut crate::contracts::store::DocumentStore,
    buffer: &mut crate::project::document::buffer::Buffer,
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
fn close_merge(state: crate::project::document::history::MergeState) -> String {
    use crate::project::document::history::MergeState;

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

/// A short name for what a payload holds, for the copy line.
fn describe(payload: &crate::app::commands::clipboard::Payload) -> String {
    use crate::app::commands::clipboard::Payload;
    match payload {
        Payload::Text(text) => format!("{} char(s) of text", text.chars().count()),
        Payload::Nodes(subtree) => format!("a subtree of {} node(s)", subtree.nodes().len()),
        Payload::Resource { path, .. } => format!("the resource {path}"),
    }
}

/// The area, the panels and the layout's own state, as the closing report.
fn layout_report(
    groups: &crate::app::shell::groups::Groups,
    panels: &crate::app::shell::panels::Panels,
    services: &std::collections::BTreeSet<String>,
    layouts: &crate::app::shell::layout::Layouts,
) -> bool {
    for line in groups.render() {
        println!("{line}");
    }
    for line in panels.render(services) {
        println!("{line}");
    }
    let saved = if layouts.saved().is_empty() {
        String::new()
    } else {
        format!("; saved {}", layouts.saved().join(", "))
    };
    let note = match layouts.note() {
        Some(note) => format!("; last refusal: {note}"),
        None => String::new(),
    };
    println!(
        "layout: {} (zen {}, fullscreen {}, {}){saved}{note}",
        layouts.current().name(),
        layouts.zen(),
        layouts.fullscreen(),
        if layouts.locked() {
            "locked"
        } else {
            "unlocked"
        },
    );
    true
}

/// A layout operation's outcome as a line, naming any panel the build has no home for.
fn layout_outcome(
    outcome: Result<Vec<String>, crate::app::shell::layout::LayoutError>,
    note: impl FnOnce() -> String,
) -> Result<String, String> {
    match outcome {
        Ok(unknown) if unknown.is_empty() => Ok(note()),
        Ok(unknown) => Ok(format!("{} (no panel for: {})", note(), unknown.join(", "))),
        Err(error) => Err(error.message()),
    }
}

/// Reads `on`/`off`.
fn on_off(argument: &str) -> Option<bool> {
    match argument {
        "on" => Some(true),
        "off" => Some(false),
        _ => None,
    }
}

/// The text store's own report.
fn text_report(store: &crate::project::text::piece_table::TextStore) -> bool {
    for line in store.render() {
        println!("{line}");
    }
    println!("text: {:?}", store.text());
    true
}

/// The allowed/refused line for one write through one entry point.
fn verdict(
    regions: &[crate::project::text::protection::ProtectedRegion],
    writability: &crate::project::document::conflict::Writability,
    source: crate::project::text::protection::WriteSource,
    write: crate::project::text::protection::Write,
) -> String {
    let guard =
        crate::project::text::protection::WriteGuard::new(regions.to_vec(), writability.clone());
    match guard.check(source, &[write]) {
        Ok(()) => format!("{}: allowed", source.name()),
        Err(refusal) => format!("{}: refused: {}", source.name(), refusal.message()),
    }
}

/// Parses a `first:second` pair of numbers.
fn two_numbers(argument: &str) -> Result<(usize, usize), String> {
    match numbers(argument).as_deref() {
        Some([first, second]) => Ok((*first, *second)),
        _ => Err(format!("{argument:?} is not <first>:<second>")),
    }
}

/// Parses every colon-separated number of an argument, or nothing when one is not a number.
fn numbers(argument: &str) -> Option<Vec<usize>> {
    argument
        .split(':')
        .map(|part| part.parse::<usize>().ok())
        .collect()
}

/// Prints a refusal and answers false, so an operation loop can `return report(reason)`.
fn report(reason: String) -> bool {
    println!(
        "{}",
        line_with("shell.error.prefix", &[("message", reason.as_str())])
    );
    false
}

/// One character from a command-line argument, when it is exactly one.
/// A command-line argument with the common `\n`, `\r`, `\t` and `\\` escapes understood.
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut characters = text.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            out.push(character);
            continue;
        }
        match characters.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// A diagnostic from `<severity>:<source>:<start>:<end>:<message>`.
fn parse_diagnostic(
    argument: &str,
) -> Result<crate::app::language::diagnostics::Diagnostic, String> {
    use crate::app::language::diagnostics::{Diagnostic, Severity, Source};
    use crate::app::language::navigation::Span;
    let fields: Vec<&str> = argument.splitn(5, ':').collect();
    let [severity, source, start, end, message] = fields.as_slice() else {
        return Err(format!(
            "{argument:?} is not <severity>:<source>:<s>:<e>:<message>"
        ));
    };
    let severity =
        Severity::from_name(severity).ok_or_else(|| format!("{severity:?} is not a severity"))?;
    let source = Source::from_name(source).ok_or_else(|| format!("{source:?} is not a source"))?;
    let number = |text: &str| -> Result<usize, String> {
        text.parse()
            .map_err(|_| format!("{text:?} is not an offset"))
    };
    Ok(Diagnostic::new(
        Span::new(number(start)?, number(end)?),
        severity,
        source,
        *message,
    ))
}

/// A token from `<kind>:<start>:<end>`.
fn parse_token(argument: &str) -> Result<crate::app::language::diagnostics::SemanticToken, String> {
    use crate::app::language::diagnostics::{SemanticToken, TokenKind};
    use crate::app::language::navigation::Span;
    let fields: Vec<&str> = argument.splitn(3, ':').collect();
    let [kind, start, end] = fields.as_slice() else {
        return Err(format!("{argument:?} is not <kind>:<start>:<end>"));
    };
    let kind = TokenKind::from_name(kind).ok_or_else(|| format!("{kind:?} is not a token kind"))?;
    let number = |text: &str| -> Result<usize, String> {
        text.parse()
            .map_err(|_| format!("{text:?} is not an offset"))
    };
    Ok(SemanticToken {
        span: Span::new(number(start)?, number(end)?),
        kind,
    })
}

/// An action from `<title>:<at>:<remove>:<insert>`.
fn parse_action(argument: &str) -> Result<crate::app::language::diagnostics::CodeAction, String> {
    use crate::app::language::diagnostics::{ActionKind, CodeAction, TextEdit};
    let fields: Vec<&str> = argument.rsplitn(4, ':').collect();
    let [insert, remove, at, title] = fields.as_slice() else {
        return Err(format!(
            "{argument:?} is not <title>:<at>:<remove>:<insert>"
        ));
    };
    let at: usize = at.parse().map_err(|_| format!("{at:?} is not an offset"))?;
    let remove: usize = remove
        .parse()
        .map_err(|_| format!("{remove:?} is not a length"))?;
    Ok(CodeAction::new(
        *title,
        ActionKind::QuickFix,
        vec![TextEdit::new(at, remove, unescape(insert))],
    ))
}

/// A `(uri, start, end)` from `<uri>:<start>:<end>` (the uri may contain colons).
fn parse_uri_range(argument: &str) -> Result<(String, usize, usize), String> {
    let fields: Vec<&str> = argument.rsplitn(3, ':').collect();
    let [end, start, uri] = fields.as_slice() else {
        return Err(format!("{argument:?} is not <uri>:<start>:<end>"));
    };
    let number = |text: &str| -> Result<usize, String> {
        text.parse()
            .map_err(|_| format!("{text:?} is not an offset"))
    };
    Ok(((*uri).to_owned(), number(start)?, number(end)?))
}

/// A file edit from `<uri>:<baseline>:<at>:<remove>:<insert>` (the uri may contain colons).
fn parse_file_edit(
    argument: &str,
) -> Result<crate::app::language::workspace_edit::FileEdit, String> {
    use crate::app::language::diagnostics::TextEdit;
    use crate::app::language::workspace_edit::FileEdit;
    let fields: Vec<&str> = argument.rsplitn(5, ':').collect();
    let [insert, remove, at, baseline, uri] = fields.as_slice() else {
        return Err(format!(
            "{argument:?} is not <uri>:<baseline>:<at>:<remove>:<insert>"
        ));
    };
    let baseline: u64 = baseline
        .parse()
        .map_err(|_| format!("{baseline:?} is not a version"))?;
    let at: usize = at.parse().map_err(|_| format!("{at:?} is not an offset"))?;
    let remove: usize = remove
        .parse()
        .map_err(|_| format!("{remove:?} is not a length"))?;
    Ok(FileEdit::new(
        *uri,
        baseline,
        vec![TextEdit::new(at, remove, unescape(insert))],
    ))
}

/// An entry from `<key>:<requires|->:<detail>`.
fn parse_entry(argument: &str) -> Result<crate::app::language::schema_docs::Entry, String> {
    use crate::app::language::schema_docs::Entry;
    let (key, rest) = argument
        .split_once(':')
        .ok_or_else(|| format!("{argument:?} has no requirement"))?;
    let (requires, detail) = rest
        .split_once(':')
        .ok_or_else(|| format!("{rest:?} has no detail"))?;
    let entry = Entry::new(key, "schema", unescape(detail));
    if requires == "-" || requires.is_empty() {
        Ok(entry)
    } else {
        Ok(entry.requiring(requires))
    }
}

/// An offset from an argument.
fn number(text: &str) -> Result<usize, String> {
    text.parse()
        .map_err(|_| format!("{text:?} is not an offset"))
}

/// A widget from `<id>:<name>:<category>`.
fn parse_widget(argument: &str) -> Option<(String, String, String)> {
    let fields: Vec<&str> = argument.splitn(3, ':').collect();
    let [id, name, category] = fields.as_slice() else {
        return None;
    };
    Some(((*id).to_owned(), (*name).to_owned(), (*category).to_owned()))
}

/// A property from `<id>:<name>:<type>[:<default>]`.
fn parse_property(
    argument: &str,
) -> Result<(String, crate::app::canvas::widget_catalog::PropertySchema), String> {
    use crate::app::canvas::widget_catalog::{PropertySchema, ValueType};
    let fields: Vec<&str> = argument.splitn(4, ':').collect();
    let [id, name, type_name, rest @ ..] = fields.as_slice() else {
        return Err(format!(
            "{argument:?} is not <id>:<name>:<type>[:<default>]"
        ));
    };
    let value_type = match type_name.split_once('(') {
        Some((base, rest)) => {
            let inner = rest.trim_end_matches(')');
            match base {
                "enum" => ValueType::Enum(inner.split(',').map(str::to_owned).collect()),
                "resource" => ValueType::Resource {
                    kind: inner.to_owned(),
                },
                other => return Err(format!("{other:?} is not a value type")),
            }
        }
        None => match *type_name {
            "number" => ValueType::Number,
            "text" => ValueType::Text,
            "bool" => ValueType::Bool,
            "color" => ValueType::Color,
            "dimension" => ValueType::Dimension,
            other => return Err(format!("{other:?} is not a value type")),
        },
    };
    let mut property = PropertySchema::new(*name, value_type);
    if let Some(default) = rest.first() {
        property = property.with_default(unescape(default));
    }
    Ok(((*id).to_owned(), property))
}

/// A node from a decimal argument.
fn parse_scene_node(text: &str) -> Result<crate::contracts::identity::NodeId, String> {
    text.parse::<u64>()
        .map(crate::contracts::identity::NodeId::from_raw)
        .map_err(|_| format!("{text:?} is not a node id"))
}

/// A kind from a name.
fn parse_kind(text: &str) -> Result<crate::app::canvas::structure::Kind, String> {
    crate::app::canvas::structure::Kind::from_name(text)
        .ok_or_else(|| format!("{text:?} is not a node kind"))
}

/// A `(parent, kind, name)` from `<parent>:<kind>:<name>`.
fn parse_insert(
    argument: &str,
) -> Result<
    (
        crate::contracts::identity::NodeId,
        crate::app::canvas::structure::Kind,
        String,
    ),
    String,
> {
    let fields: Vec<&str> = argument.splitn(3, ':').collect();
    let [parent, kind, name] = fields.as_slice() else {
        return Err(format!("{argument:?} is not <parent>:<kind>:<name>"));
    };
    Ok((
        parse_scene_node(parent)?,
        parse_kind(kind)?,
        (*name).to_owned(),
    ))
}

/// The comma-separated numbers in an argument.
fn parse_floats(text: &str) -> Result<Vec<f64>, String> {
    text.split(',')
        .map(|part| {
            part.trim()
                .parse::<f64>()
                .map_err(|_| format!("{part:?} is not a number"))
        })
        .collect()
}

/// A property type from `<kind>[:<args>]`.
fn parse_property_type(
    text: &str,
) -> Result<crate::app::style::property_editor::PropertyType, String> {
    use crate::app::style::property_editor::PropertyType;

    let (kind, args) = text.split_once(':').unwrap_or((text, ""));
    match kind {
        "number" => {
            let (min, max) = parse_bounds(args)?;
            Ok(PropertyType::Number { min, max })
        }
        "integer" => {
            let (min, max) = parse_bounds(args)?;
            Ok(PropertyType::Integer {
                min: min.map(|value| value as i64),
                max: max.map(|value| value as i64),
            })
        }
        "enum" => Ok(PropertyType::Enum {
            variants: args.split(',').map(str::to_owned).collect(),
        }),
        "text" => match args.parse::<usize>() {
            Ok(max_len) => Ok(PropertyType::Text { max_len }),
            Err(_) => Err(format!("{args:?} is not a length")),
        },
        "colour" | "color" => Ok(PropertyType::Colour),
        "size" => Ok(PropertyType::Size),
        "asset" => Ok(PropertyType::Asset),
        other => Err(format!("{other:?} is not a property type")),
    }
}

/// A `(min, max)` from `<min>:<max>`, either side possibly empty.
fn parse_bounds(args: &str) -> Result<(Option<f64>, Option<f64>), String> {
    let (min, max) = match args.split_once(':') {
        Some((min, max)) => (min, max),
        None => (args, ""),
    };
    let number = |value: &str| -> Result<Option<f64>, String> {
        if value.is_empty() {
            Ok(None)
        } else {
            value
                .parse::<f64>()
                .map(Some)
                .map_err(|_| format!("{value:?} is not a bound"))
        }
    };
    Ok((number(min)?, number(max)?))
}

fn parse_origin(text: &str) -> Result<crate::app::data::binding::Origin, String> {
    use crate::app::data::binding::Origin;
    use crate::contracts::identity::NodeId;
    use crate::contracts::model::PropertyName;
    if let Some(name) = text.strip_prefix("f:") {
        return Ok(Origin::Field(name.to_owned()));
    }
    match text.split_once('.') {
        Some((node, property)) => {
            let node = node
                .parse::<u64>()
                .map_err(|_| format!("{node:?} is not a node id"))?;
            let property = PropertyName::new(property)
                .ok_or_else(|| format!("{property:?} is not a property name"))?;
            Ok(Origin::Slot {
                node: NodeId::from_raw(node),
                property,
            })
        }
        None => Err(format!(
            "{text:?} is neither f:<field> nor <node>.<property>"
        )),
    }
}

/// Reads each document's own text, so a set can be written back as it stands.
fn read_each(
    root: &crate::host::paths::ProjectRoot,
    documents: &[String],
) -> Result<Vec<(String, String)>, String> {
    let mut entries = Vec::new();
    for document in documents {
        let path = root.resolve(document).map_err(|error| error.message())?;
        let text = std::fs::read_to_string(&path)
            .map_err(|error| format!("{} could not be read: {error}", path.display()))?;
        entries.push((document.clone(), text));
    }
    Ok(entries)
}

/// A directory's own name, when it has one.
fn directory_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
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
        if state == crate::project::lifecycle::recent::RecentState::Missing {
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
