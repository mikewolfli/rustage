// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The application: what rustage does when it runs.
//!
//! The designer itself is not implemented. What *is* implemented and reachable here is the icon
//! strip in [`toolbar`], which exists because the host's icons need a real consumer before
//! anything downstream can claim to be verified: the resource tooling proves a glyph is a glyph,
//! and only a control can prove the glyph reaches a screen.

use std::path::Path;

use crate::cli::{self, Command};

pub mod toolbar;

pub fn run(command: Command) {
    match command {
        Command::Help => print!("{}", cli::help_text()),
        Command::Version => println!("rustage {}", env!("CARGO_PKG_VERSION")),
        Command::Status => println!(
            "rustage {}\n\
             The desktop designer is not implemented yet.\n\
             The icon strip is: run `rustage --render-icons <directory>` to render it.",
            env!("CARGO_PKG_VERSION")
        ),
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
    }
}

/// Writes the icon strip's baseline renders, reporting the files written.
fn render_icons(directory: &Path) -> Result<Vec<std::path::PathBuf>, Box<dyn std::error::Error>> {
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
