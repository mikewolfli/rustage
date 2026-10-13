// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! What rustage does when it runs: one arm per parsed command.
//!
//! The designer itself is not implemented. What *is* implemented and reachable here is the icon
//! strip in [`toolbar`](super::toolbar) — which exists because the host's icons need a real
//! consumer before anything downstream can claim to be verified: the resource tooling proves a
//! glyph is a glyph, and only a control can prove the glyph reaches a screen — and the dependency
//! contract probes under `crate::probe`.

use std::path::{Path, PathBuf};

use crate::cli::{self, Command};
use crate::probe::report::ProbeReport;

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
    }
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
