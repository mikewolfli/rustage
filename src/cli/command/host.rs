// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The host-facing commands: the desktop session, and the producers of icon and report
//! artefacts.

// Every branch of the flag table answers with `return`, including the ones that could be a
// trailing expression: the branches are independent flags, and writing them alike is worth more
// than saving a keyword. The returns that are *not* needless — the ones inside a loop that refuses
// a bad value — are why the table reads the way it does.
#![allow(clippy::needless_return)]

use super::*;

/// How long the baseline session should live.
///
/// A start that cannot end is not evidence and not usable by a person, so the duration is part of
/// the command rather than a hidden constant; the default matches
/// [`crate::host::platform::BASELINE_SESSION`].
#[derive(Debug, PartialEq, Eq)]
pub struct LaunchOptions {
    pub exit_after: Duration,
    /// Mount one button in the window and require at least one real click before the session is
    /// called successful. This is the GUI evidence path: a window that only exists cannot show
    /// that pointer input reached a control.
    pub smoke: bool,
    /// Also forward each smoke click to a real `--logic-worker` process over the typed channel.
    pub smoke_worker: bool,
    /// Wire a real GTK input-method context to the smoke field (Linux X11 only).
    pub ime: bool,
    /// Seed the smoke field with this text before the session (visual evidence).
    pub smoke_text: Option<String>,
}

impl Default for LaunchOptions {
    fn default() -> Self {
        Self {
            exit_after: crate::host::platform::BASELINE_SESSION,
            smoke: false,
            smoke_worker: false,
            ime: false,
            smoke_text: None,
        }
    }
}

/// Parses this family's flags, answering `None` when the flag is another family's.
pub(super) fn parse_host(
    text: &str,
    argument: OsString,
    arguments: &mut std::vec::IntoIter<OsString>,
) -> Option<Result<Command, ParseError>> {
    match text {
        "--launch" => {
            let mut options = LaunchOptions::default();
            let rest = arguments;
            while let Some(argument) = rest.next() {
                if argument == OsStr::new("--exit-after-ms") {
                    let Some(value) = rest.next() else {
                        return Some(Err(ParseError { argument }));
                    };
                    let Some(milliseconds) =
                        value.to_str().and_then(|value| value.parse::<u64>().ok())
                    else {
                        return Some(Err(ParseError { argument: value }));
                    };
                    options.exit_after = Duration::from_millis(milliseconds);
                    continue;
                }
                if argument == OsStr::new("--smoke") {
                    options.smoke = true;
                    continue;
                }
                if argument == OsStr::new("--smoke-worker") {
                    options.smoke = true;
                    options.smoke_worker = true;
                    continue;
                }
                if argument == OsStr::new("--ime") {
                    options.smoke = true;
                    options.ime = true;
                    continue;
                }
                if argument == OsStr::new("--smoke-text") {
                    let Some(value) = rest.next() else {
                        return Some(Err(ParseError { argument }));
                    };
                    options.smoke = true;
                    options.smoke_text = Some(value.to_string_lossy().into_owned());
                    continue;
                }
                return Some(Err(ParseError { argument }));
            }
            return Some(Ok(Command::Launch(options)));
        }
        "--render-icons" | "--probe-apis" | "--probe-host" | "--project-check"
        | "--locale-report" => {
            let Some(value) = arguments.next() else {
                return Some(Err(ParseError { argument }));
            };
            let rest = arguments;
            if let Some(argument) = rest.next() {
                return Some(Err(ParseError { argument }));
            }
            let path = PathBuf::from(value);
            return Some(Ok(match text {
                "--render-icons" => Command::RenderIcons(path),
                "--probe-apis" => Command::ProbeApis(path),
                "--project-check" => Command::ProjectCheck(path),
                "--locale-report" => Command::LocaleReport(path),
                _ => Command::ProbeHost(path),
            }));
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;

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
        assert_eq!(
            parse(["--project-check", "demo/project.json"]),
            Ok(Command::ProjectCheck(PathBuf::from("demo/project.json")))
        );
        assert_eq!(
            parse(["--locale-report", "out/locale"]),
            Ok(Command::LocaleReport(PathBuf::from("out/locale")))
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
                exit_after: Duration::from_millis(10),
                smoke: false,
                smoke_worker: false,
                ime: false,
                smoke_text: None,
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
                exit_after: Duration::from_millis(1500),
                smoke: false,
                smoke_worker: false,
                ime: false,
                smoke_text: None,
            })),
        );
        assert_eq!(
            parse(["--launch", "--smoke", "--exit-after-ms", "1500"]),
            Ok(Command::Launch(LaunchOptions {
                exit_after: Duration::from_millis(1500),
                smoke: true,
                smoke_worker: false,
                ime: false,
                smoke_text: None,
            })),
        );
        assert_eq!(
            parse(["--launch", "--smoke-worker", "--exit-after-ms", "1500"]),
            Ok(Command::Launch(LaunchOptions {
                exit_after: Duration::from_millis(1500),
                smoke: true,
                smoke_worker: true,
                ime: false,
                smoke_text: None,
            })),
        );
        assert_eq!(
            parse(["--launch", "--ime", "--exit-after-ms", "1500"]),
            Ok(Command::Launch(LaunchOptions {
                exit_after: Duration::from_millis(1500),
                smoke: true,
                smoke_worker: false,
                ime: true,
                smoke_text: None,
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
}
