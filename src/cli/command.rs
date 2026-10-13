// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The argument parser and the usage text.
//!
//! The surface is deliberately tiny. The scaffold answers `--help`, `--version` and reports its
//! own status; the other two options exist because the host needs reproducible producers for
//! artifacts it commits or inspects: `--render-icons` renders the icon strip in
//! [`crate::app::toolbar`] into a directory (the committed baselines under `tests/baselines/icons/`
//! would otherwise be pictures of whatever the machine last rendered, which is exactly what a
//! baseline must not be), and `--probe-apis` runs the dependency contract probes in
//! [`crate::probe`] and writes their report.

use std::{
    ffi::{OsStr, OsString},
    fmt,
    path::PathBuf,
};

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Help,
    Version,
    Status,
    /// Writes the icon strip's light and dark renders into the given directory.
    RenderIcons(PathBuf),
    /// Runs the dependency contract probes and writes the report into the given directory.
    ProbeApis(PathBuf),
}

#[derive(Debug, PartialEq, Eq)]
pub struct ParseError {
    argument: OsString,
}

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "unexpected argument `{}`",
            self.argument.to_string_lossy()
        )
    }
}

impl std::error::Error for ParseError {}

pub fn parse<I, S>(arguments: I) -> Result<Command, ParseError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut arguments = arguments.into_iter();
    let Some(argument) = arguments.next() else {
        return Ok(Command::Status);
    };

    let command = match argument.as_ref() {
        value if value == "--help" || value == "-h" => Command::Help,
        value if value == "--version" || value == "-V" => Command::Version,
        value if value == "--render-icons" || value == "--probe-apis" => {
            let Some(directory) = arguments.next() else {
                return Err(ParseError {
                    argument: argument.as_ref().to_os_string(),
                });
            };
            let mut rest = arguments;
            if let Some(argument) = rest.next() {
                return Err(ParseError {
                    argument: argument.as_ref().to_os_string(),
                });
            }
            let path = PathBuf::from(directory.as_ref());
            if value == "--render-icons" {
                return Ok(Command::RenderIcons(path));
            }
            return Ok(Command::ProbeApis(path));
        }
        _ => {
            return Err(ParseError {
                argument: argument.as_ref().to_os_string(),
            });
        }
    };

    if let Some(argument) = arguments.next() {
        return Err(ParseError {
            argument: argument.as_ref().to_os_string(),
        });
    }

    Ok(command)
}

pub fn help_text() -> &'static str {
    "rustage - Rust desktop UI designer\n\n\
     Usage: rustage [--help | --version | --render-icons <directory> | --probe-apis <directory>]\n\n\
     With no options, prints the current project status.\n\n\
     Options:\n\
       -h, --help                    Print this help message\n\
       -V, --version                 Print the application version\n\
       --render-icons <directory>    Render the icon strip, light and dark, into <directory>\n\
       --probe-apis <directory>      Run the dependency contract probes and write api-probes.json\n"
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{Command, parse};

    #[test]
    fn no_arguments_show_status() {
        assert_eq!(parse([] as [&str; 0]), Ok(Command::Status));
    }

    #[test]
    fn parses_help_and_version_options() {
        assert_eq!(parse(["--help"]), Ok(Command::Help));
        assert_eq!(parse(["-h"]), Ok(Command::Help));
        assert_eq!(parse(["--version"]), Ok(Command::Version));
        assert_eq!(parse(["-V"]), Ok(Command::Version));
    }

    #[test]
    fn parses_the_icon_export_directory() {
        assert_eq!(
            parse(["--render-icons", "out/icons"]),
            Ok(Command::RenderIcons(PathBuf::from("out/icons")))
        );
    }

    #[test]
    fn parses_the_probe_report_directory() {
        assert_eq!(
            parse(["--probe-apis", "out/probe"]),
            Ok(Command::ProbeApis(PathBuf::from("out/probe")))
        );
    }

    #[test]
    fn rejects_unknown_and_extra_arguments() {
        assert!(parse(["--launch"]).is_err());
        assert!(parse(["--help", "--version"]).is_err());
        // A missing or repeated directory is a usage error rather than an empty path: exporting
        // into "" would write into the process's working directory.
        assert!(parse(["--render-icons"]).is_err());
        assert!(parse(["--render-icons", "a", "b"]).is_err());
        assert!(parse(["--probe-apis"]).is_err());
        assert!(parse(["--probe-apis", "a", "b"]).is_err());
    }
}
