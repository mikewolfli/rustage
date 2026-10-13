// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The command line: what the executable accepts.
//!
//! The surface is deliberately tiny. The scaffold answers `--help`, `--version` and reports its
//! own status; the one other option, `--render-icons`, exists because the icon strip in
//! [`crate::app::toolbar`] needs a reproducible producer for the committed baselines under
//! `tests/baselines/icons/`. Without a producer the baselines would be pictures of whatever the
//! machine last rendered, which is exactly what a baseline must not be.

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
        value if value == "--render-icons" => {
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
            return Ok(Command::RenderIcons(PathBuf::from(directory.as_ref())));
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
     Usage: rustage [--help | --version | --render-icons <directory>]\n\n\
     With no options, prints the current project status.\n\n\
     Options:\n\
       -h, --help                    Print this help message\n\
       -V, --version                 Print the application version\n\
       --render-icons <directory>    Render the icon strip, light and dark, into <directory>\n"
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
    fn rejects_unknown_and_extra_arguments() {
        assert!(parse(["--launch"]).is_err());
        assert!(parse(["--help", "--version"]).is_err());
        // A missing or repeated directory is a usage error rather than an empty path: exporting
        // into "" would write into the process's working directory.
        assert!(parse(["--render-icons"]).is_err());
        assert!(parse(["--render-icons", "a", "b"]).is_err());
    }
}
