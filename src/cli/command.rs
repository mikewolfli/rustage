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
    time::Duration,
};

use crate::host::log::Level;

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Help,
    Version,
    Status,
    /// Writes the icon strip's light and dark renders into the given directory.
    RenderIcons(PathBuf),
    /// Runs the dependency contract probes and writes the report into the given directory.
    ProbeApis(PathBuf),
    /// Runs the host capability checks and writes the report into the given directory.
    ProbeHost(PathBuf),
    /// Starts the baseline desktop session and leaves it again.
    Launch(LaunchOptions),
}

/// How long the baseline session should live.
///
/// A start that cannot end is not evidence and not usable by a person, so the duration is part of
/// the command rather than a hidden constant; the default matches
/// [`crate::host::platform::BASELINE_SESSION`].
#[derive(Debug, PartialEq, Eq)]
pub struct LaunchOptions {
    pub exit_after: Duration,
}

impl Default for LaunchOptions {
    fn default() -> Self {
        Self {
            exit_after: crate::host::platform::BASELINE_SESSION,
        }
    }
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

/// Reads the global `--log-level <level>` option wherever it appears.
///
/// It is global rather than per-command because the logger is installed once for the process: a
/// `--launch` that logs and a `--probe-apis` that logs must not each invent their own switch.
pub fn log_level_of<I, S>(arguments: I) -> Result<Option<Level>, ParseError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let arguments: Vec<OsString> = arguments
        .into_iter()
        .map(|value| value.as_ref().to_os_string())
        .collect();
    let mut level = None;
    let mut index = 0;
    while index < arguments.len() {
        if arguments[index] == OsStr::new("--log-level") {
            let Some(value) = arguments.get(index + 1) else {
                return Err(ParseError {
                    argument: arguments[index].clone(),
                });
            };
            let Some(parsed) = value.to_str().and_then(Level::parse) else {
                return Err(ParseError {
                    argument: value.clone(),
                });
            };
            level = Some(parsed);
            index += 2;
            continue;
        }
        index += 1;
    }
    Ok(level)
}

pub fn parse<I, S>(arguments: I) -> Result<Command, ParseError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    // The global option is removed here so every arm below can assume it is looking at the command;
    // `log_level_of` reads it separately, and both agree because they share this token handling.
    let collected: Vec<OsString> = arguments
        .into_iter()
        .map(|value| value.as_ref().to_os_string())
        .collect();
    let mut stripped: Vec<OsString> = Vec::with_capacity(collected.len());
    let mut index = 0;
    while index < collected.len() {
        if collected[index] == OsStr::new("--log-level") {
            let Some(value) = collected.get(index + 1) else {
                return Err(ParseError {
                    argument: collected[index].clone(),
                });
            };
            if value.to_str().and_then(Level::parse).is_none() {
                return Err(ParseError {
                    argument: value.clone(),
                });
            }
            index += 2;
            continue;
        }
        stripped.push(collected[index].clone());
        index += 1;
    }

    let mut arguments = stripped.into_iter();
    let Some(argument) = arguments.next() else {
        return Ok(Command::Status);
    };
    let text = argument.to_str().unwrap_or_default();

    let command = match text {
        "--help" | "-h" => Command::Help,
        "--version" | "-V" => Command::Version,
        "--launch" => {
            let mut options = LaunchOptions::default();
            let mut rest = arguments;
            while let Some(argument) = rest.next() {
                if argument == OsStr::new("--exit-after-ms") {
                    let Some(value) = rest.next() else {
                        return Err(ParseError { argument });
                    };
                    let Some(milliseconds) =
                        value.to_str().and_then(|value| value.parse::<u64>().ok())
                    else {
                        return Err(ParseError { argument: value });
                    };
                    options.exit_after = Duration::from_millis(milliseconds);
                    continue;
                }
                return Err(ParseError { argument });
            }
            return Ok(Command::Launch(options));
        }
        "--render-icons" | "--probe-apis" | "--probe-host" => {
            let Some(directory) = arguments.next() else {
                return Err(ParseError { argument });
            };
            let mut rest = arguments;
            if let Some(argument) = rest.next() {
                return Err(ParseError { argument });
            }
            let path = PathBuf::from(directory);
            return Ok(match text {
                "--render-icons" => Command::RenderIcons(path),
                "--probe-apis" => Command::ProbeApis(path),
                _ => Command::ProbeHost(path),
            });
        }
        _ => return Err(ParseError { argument }),
    };

    if let Some(argument) = arguments.next() {
        return Err(ParseError { argument });
    }

    Ok(command)
}

pub fn help_text() -> &'static str {
    "rustage - Rust desktop UI designer\n\n\
     Usage: rustage [--help | --version | --launch [--exit-after-ms <ms>]]\n\
            rustage [--render-icons <directory> | --probe-apis <directory>]\n\
            rustage [--probe-host <directory>]\n\n\
     With no options, prints the current project status.\n\n\
     Options:\n\
       -h, --help                    Print this help message\n\
       -V, --version                 Print the application version\n\
       --launch                      Start the baseline desktop session, then leave it again\n\
       --exit-after-ms <ms>          With --launch, leave the event loop after <ms>\n\
       --render-icons <directory>    Render the icon strip, light and dark, into <directory>\n\
       --probe-apis <directory>      Run the dependency contract probes and write api-probes.json\n\
       --probe-host <directory>      Check the host platform services and write host-readiness.json\n\n\
     Global options:\n\
       --log-level <level>           error | warn | info | debug | trace (default: info)\n"
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{Command, LaunchOptions, log_level_of, parse};
    use crate::host::log::Level;
    use std::time::Duration;

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
        assert_eq!(
            parse(["--probe-host", "out/host"]),
            Ok(Command::ProbeHost(PathBuf::from("out/host")))
        );
    }

    #[test]
    fn the_log_level_option_is_read_and_removed_from_the_command() {
        assert_eq!(log_level_of(["--help"]).unwrap(), None);
        assert_eq!(
            log_level_of(["--log-level", "debug", "--launch"]).unwrap(),
            Some(Level::Debug)
        );
        assert_eq!(
            parse(["--log-level", "warn", "--launch"]),
            Ok(Command::Launch(LaunchOptions::default()))
        );
        assert_eq!(
            parse(["--launch", "--log-level", "trace", "--exit-after-ms", "10"]),
            Ok(Command::Launch(LaunchOptions {
                exit_after: Duration::from_millis(10)
            })),
        );

        // A missing or unknown level is a usage error, not a silent fallback to the default.
        assert!(log_level_of(["--log-level"]).is_err());
        assert!(log_level_of(["--log-level", "shout"]).is_err());
        assert!(parse(["--log-level", "shout", "--help"]).is_err());
    }

    #[test]
    fn parses_the_baseline_session_and_its_duration() {
        // `--launch` used to be an unknown argument; it is a command now, and its default session
        // is bounded so a smoke test can end on its own.
        assert_eq!(
            parse(["--launch"]),
            Ok(Command::Launch(LaunchOptions::default()))
        );
        assert_eq!(
            parse(["--launch", "--exit-after-ms", "1500"]),
            Ok(Command::Launch(LaunchOptions {
                exit_after: Duration::from_millis(1500)
            })),
        );
        assert_ne!(
            LaunchOptions::default().exit_after,
            Duration::from_millis(1500)
        );

        // A duration that is missing, unparsable or followed by anything else is a usage error.
        assert!(parse(["--launch", "--exit-after-ms"]).is_err());
        assert!(parse(["--launch", "--exit-after-ms", "soon"]).is_err());
        assert!(parse(["--launch", "--exit-after-ms", "10", "extra"]).is_err());
        assert!(parse(["--launch", "--unknown"]).is_err());
    }

    #[test]
    fn rejects_unknown_and_extra_arguments() {
        assert!(parse(["--help", "--version"]).is_err());
        // A missing or repeated directory is a usage error rather than an empty path: exporting
        // into "" would write into the process's working directory.
        assert!(parse(["--render-icons"]).is_err());
        assert!(parse(["--render-icons", "a", "b"]).is_err());
        assert!(parse(["--probe-apis"]).is_err());
        assert!(parse(["--probe-apis", "a", "b"]).is_err());
        assert!(parse(["--probe-host"]).is_err());
        assert!(parse(["--probe-host", "a", "b"]).is_err());
    }
}
