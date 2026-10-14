// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Opening a URL or a file with the operating system's own handler. Contract **HOST**.
//!
//! T01.02 asks for "URL 打开". The designer needs it for two things: a documentation or help link,
//! and "reveal this file" style handoffs to the user's desktop. The dependency offers no such API,
//! so the host owns the handoff and the gap is recorded upstream
//! (`docs/rw-feedback/feedback-20261013.md`, T01.02 section).
//!
//! **Two judgements this module makes explicitly**, because both are places where a plausible
//! implementation is a wrong one:
//!
//! * **"Started" and "finished" are different answers.** The OS handler is a program; a program
//!   that was spawned has not necessarily opened anything. So the outcome keeps
//!   [`OpenOutcome::HandedOver`] (the handler exited 0), [`OpenOutcome::HandlerFailed`] (it ran and
//!   failed, with its own exit code and stderr) and [`OpenOutcome::HandlerMissing`] (it could not
//!   be started at all) apart, instead of collapsing them into "opened" (principle #45).
//! * **A target that would be read as an option is refused, not escaped.** `open` and `xdg-open`
//!   parse their own arguments, so a target beginning with `-` is an option to them, not a URL —
//!   `xdg-open -- <path>` is itself rejected with "unexpected option" (verified on the pinned
//!   revision's host). Refusing an option-shaped target with a named reason keeps the caller's
//!   mistake in front of them instead of handing a flag to the desktop (principle #29).
//!
//! **Ownership.** This module starts a process and reaps it; it does not own a process tree, log a
//! child's output to a panel, or kill anything. That is T01.04's contract, which depends on the
//! task ownership of T01.03. A handler that outlives [`ProgramOpener::timeout`] is left running and
//! reported as still running — the host does not kill an application the user asked to open.

use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long the default opener waits for the OS handler to finish before giving up on it.
pub const DEFAULT_HANDOVER_TIMEOUT: Duration = Duration::from_secs(10);

/// What the caller wants the OS to open.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum OpenTarget {
    /// A URL. The scheme is checked against [`ALLOWED_SCHEMES`] before anything is started.
    Url(String),
    /// A file or directory that must already exist.
    Path(PathBuf),
}

/// The schemes this host hands to the OS.
///
/// Deliberately short. A designer opens documentation (`http`, `https`) and hands the desktop a
/// local file (`file`); everything else — `javascript:`, `data:`, an application's private
/// handler — is a URL a designer has no business launching, and an unknown scheme is exactly the
/// kind of thing that becomes a security finding later (principle #37: refuse, do not guess).
pub const ALLOWED_SCHEMES: &[&str] = &["http", "https", "file"];

/// Why a target was refused before anything was started.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum OpenRefusal {
    /// Nothing was named.
    Empty,
    /// The target begins with `-`, so the handler would read it as an option.
    OptionLike { target: String },
    /// The target contains a character that cannot be passed to a program as an argument.
    ControlCharacter { target: String },
    /// A URL without a scheme (`example.com/docs`), which the OS would not know how to open.
    MissingScheme { target: String },
    /// A scheme outside [`ALLOWED_SCHEMES`].
    UnsupportedScheme { scheme: String },
    /// A path that does not exist, so there is nothing to reveal.
    MissingPath { path: PathBuf },
    /// This operating system has no handler this host knows how to start.
    UnsupportedPlatform { os: String },
}

impl OpenRefusal {
    /// A sentence naming what was found.
    pub fn message(&self) -> String {
        match self {
            Self::Empty => "no target was named".to_owned(),
            Self::OptionLike { target } => format!(
                "{target} begins with `-`, so the system handler would read it as an option"
            ),
            Self::ControlCharacter { target } => {
                format!("{target} contains a control character and cannot be passed as an argument")
            }
            Self::MissingScheme { target } => format!(
                "{target} has no `scheme:` prefix, so the system handler cannot tell what to open"
            ),
            Self::UnsupportedScheme { scheme } => format!(
                "the `{scheme}:` scheme is not one this host hands to the system \
                 (allowed: {})",
                ALLOWED_SCHEMES.join(", "),
            ),
            Self::MissingPath { path } => {
                format!(
                    "{} does not exist, so there is nothing to open",
                    path.display()
                )
            }
            Self::UnsupportedPlatform { os } => {
                format!("no system URL handler is known for `{os}`")
            }
        }
    }
}

impl fmt::Display for OpenRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message())
    }
}

impl std::error::Error for OpenRefusal {}

/// What happened after a target was accepted and the handler was started.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum OpenOutcome {
    /// The handler ran and exited with success.
    HandedOver {
        /// The handler program the host started.
        program: String,
        /// Its exit code.
        exit: i32,
    },
    /// The handler ran and failed. Its own exit code and output are kept, because "the browser
    /// exited 4 and said why" is the diagnostic a caller needs.
    HandlerFailed {
        /// The handler program the host started.
        program: String,
        /// Its exit code, when the platform reported one.
        exit: Option<i32>,
        /// What it wrote to stderr, trimmed. Empty when it said nothing.
        detail: String,
    },
    /// The handler could not be started at all.
    HandlerMissing {
        /// The handler program the host tried to start.
        program: String,
        /// The OS's own reason.
        reason: String,
    },
    /// The handler was still running when the timeout passed.
    ///
    /// Not a failure: a desktop handler may hand off to an already-running application and wait.
    /// The process is left alone and reaped in the background, so it cannot become a zombie.
    StillRunning {
        /// The handler program the host started.
        program: String,
        /// How long the host waited before reporting this.
        after: Duration,
    },
}

impl OpenOutcome {
    /// Whether the handler completed successfully.
    pub fn is_handed_over(&self) -> bool {
        matches!(self, Self::HandedOver { .. })
    }

    /// Whether the OS actually started something. [`Self::HandlerMissing`] is the only answer that
    /// says the program never ran.
    pub fn started_the_handler(&self) -> bool {
        !matches!(self, Self::HandlerMissing { .. })
    }

    /// A sentence for a log line or a diagnostic.
    pub fn message(&self) -> String {
        match self {
            Self::HandedOver { program, exit } => {
                format!("{program} opened the target and exited with {exit}")
            }
            Self::HandlerFailed {
                program,
                exit,
                detail,
            } => {
                let code = exit.map_or_else(|| "no code".to_owned(), |code| code.to_string());
                if detail.is_empty() {
                    format!("{program} failed to open the target (exit {code})")
                } else {
                    format!("{program} failed to open the target (exit {code}): {detail}")
                }
            }
            Self::HandlerMissing { program, reason } => {
                format!("{program} could not be started: {reason}")
            }
            Self::StillRunning { program, after } => {
                format!("{program} is still running after {after:?}; it was left alone")
            }
        }
    }
}

/// The system's URL handler, as far as the host needs it.
pub trait UrlOpener {
    /// Hands `target` to the system handler.
    fn open(&self, target: &OpenTarget) -> Result<OpenOutcome, OpenRefusal>;
}

/// Checks a target before any process is started, returning the argument the handler should take.
///
/// The order of the checks is deliberate: the argument's *shape* is judged before its *meaning,
/// because a target that would be read as an option must be refused for that reason even when it
/// also happens to be malformed. Naming the dangerous defect first is what makes the message
/// actionable.
pub fn accept(target: &OpenTarget) -> Result<String, OpenRefusal> {
    let argument = match target {
        OpenTarget::Url(url) => {
            if url.trim().is_empty() {
                return Err(OpenRefusal::Empty);
            }
            url.clone()
        }
        OpenTarget::Path(path) => {
            if path.as_os_str().is_empty() {
                return Err(OpenRefusal::Empty);
            }
            path.to_string_lossy().into_owned()
        }
    };

    if argument.starts_with('-') {
        return Err(OpenRefusal::OptionLike { target: argument });
    }
    if argument.chars().any(char::is_control) {
        return Err(OpenRefusal::ControlCharacter { target: argument });
    }

    match target {
        OpenTarget::Url(url) => {
            let Some((scheme, _)) = url.split_once(':') else {
                return Err(OpenRefusal::MissingScheme {
                    target: url.clone(),
                });
            };
            let scheme = scheme.to_ascii_lowercase();
            if !ALLOWED_SCHEMES.contains(&scheme.as_str()) {
                return Err(OpenRefusal::UnsupportedScheme { scheme });
            }
        }
        OpenTarget::Path(path) => {
            if !path.exists() {
                return Err(OpenRefusal::MissingPath { path: path.clone() });
            }
        }
    }
    Ok(argument)
}

/// The handler program and fixed arguments a platform uses, if this host knows one.
///
/// `cfg!` is used rather than `#[cfg]` so the mapping is a value a test can read on any host: the
/// Windows and macOS entries are then checked by the same test that checks the running platform's,
/// instead of the non-matching arms being invisible on the machine that runs the tests.
pub fn program_for(os: &str) -> Option<(String, Vec<String>)> {
    match os {
        "macos" => Some(("open".to_owned(), Vec::new())),
        "linux" | "freebsd" | "netbsd" | "openbsd" | "dragonfly" | "solaris" | "illumos" => {
            Some(("xdg-open".to_owned(), Vec::new()))
        }
        // `cmd /C start "" <target>`: the empty title is required, because `start` treats the first
        // quoted argument as the window title — a target with spaces would otherwise be consumed as
        // one. The target is passed as its own argv entry, so the shell never parses it.
        "windows" => Some((
            "cmd".to_owned(),
            vec!["/C".to_owned(), "start".to_owned(), String::new()],
        )),
        _ => None,
    }
}

/// A URL opener that starts a program and reports what that program did.
pub struct ProgramOpener {
    program: String,
    prefix_arguments: Vec<String>,
    timeout: Duration,
}

impl ProgramOpener {
    /// An opener around an explicit program, which tests use to run a handler they control.
    pub fn with_program(
        program: impl Into<String>,
        prefix_arguments: Vec<String>,
        timeout: Duration,
    ) -> Self {
        Self {
            program: program.into(),
            prefix_arguments,
            timeout,
        }
    }

    /// The opener for the machine this host is running on.
    ///
    /// Returns `Err` rather than guessing when the platform has no known handler: a host that got a
    /// default program here would start the wrong thing on a platform nobody tested.
    pub fn for_this_platform() -> Result<Self, OpenRefusal> {
        let os = std::env::consts::OS;
        let (program, prefix_arguments) = program_for(os)
            .ok_or_else(|| OpenRefusal::UnsupportedPlatform { os: os.to_owned() })?;
        Ok(Self::with_program(
            program,
            prefix_arguments,
            DEFAULT_HANDOVER_TIMEOUT,
        ))
    }

    /// The program this opener starts.
    pub fn program(&self) -> &str {
        &self.program
    }

    /// Whether the handler program can be found on `PATH`.
    ///
    /// Used by the readiness report, which must say "the handler is not installed" before anyone
    /// blames the URL. This is *not* a claim that a launch would succeed — the report keeps those
    /// apart (principle #45).
    pub fn is_on_path(&self) -> bool {
        let Some(path) = std::env::var_os("PATH") else {
            return false;
        };
        std::env::split_paths(&path).any(|directory| {
            let candidate = directory.join(&self.program);
            candidate.is_file() && is_executable(&candidate)
        })
    }
}

/// Whether a path is executable on this host.
///
/// On Unix that is the owner/group/other execute bit; elsewhere the file's presence is all the
/// platform lets this host ask for without opening it, so a `.exe` on `PATH` counts.
fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        true
    }
}

impl UrlOpener for ProgramOpener {
    fn open(&self, target: &OpenTarget) -> Result<OpenOutcome, OpenRefusal> {
        let argument = accept(target)?;
        let mut command = Command::new(&self.program);
        command.args(&self.prefix_arguments);
        command.arg(&argument);
        command.stdin(Stdio::null());
        command.stdout(Stdio::null());
        command.stderr(Stdio::piped());

        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                return Ok(OpenOutcome::HandlerMissing {
                    program: self.program.clone(),
                    reason: error.to_string(),
                });
            }
        };

        let deadline = Instant::now() + self.timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) => {
                    if Instant::now() >= deadline {
                        break None;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => {
                    return Ok(OpenOutcome::HandlerFailed {
                        program: self.program.clone(),
                        exit: None,
                        detail: format!("the handler's status could not be read: {error}"),
                    });
                }
            }
        };

        let Some(status) = status else {
            // Reap in the background so a long-lived handler cannot become a zombie. The thread
            // ends with the child; nothing else is done to a process the user asked to open.
            let program = self.program.clone();
            std::thread::spawn(move || {
                if let Err(error) = child.wait() {
                    crate::host::log::emit(
                        crate::host::log::Level::Debug,
                        "host::open_url",
                        &format!("{program} could not be reaped: {error}"),
                    );
                }
            });
            return Ok(OpenOutcome::StillRunning {
                program: self.program.clone(),
                after: self.timeout,
            });
        };

        if status.success() {
            return Ok(OpenOutcome::HandedOver {
                program: self.program.clone(),
                exit: status.code().unwrap_or(0),
            });
        }

        let mut detail = String::new();
        if let Some(mut stderr) = child.stderr.take() {
            let _ = stderr.read_to_string(&mut detail);
        }
        Ok(OpenOutcome::HandlerFailed {
            program: self.program.clone(),
            exit: status.code(),
            detail: detail.trim().to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ALLOWED_SCHEMES, OpenOutcome, OpenRefusal, OpenTarget, ProgramOpener, UrlOpener,
        program_for,
    };
    use std::path::PathBuf;
    use std::time::Duration;

    /// A fake handler on disk, so a launch and its outcome are exercised without opening anything.
    struct FakeHandler {
        directory: PathBuf,
        path: PathBuf,
    }

    impl FakeHandler {
        fn new(body: &str) -> Self {
            let directory = std::env::temp_dir().join(format!(
                "rustage-handler-{}-{:?}",
                std::process::id(),
                std::thread::current().id(),
            ));
            let _ = std::fs::remove_dir_all(&directory);
            std::fs::create_dir_all(&directory).expect("the sandbox is creatable");
            let path = directory.join("handler.sh");
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("the handler is writable");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                    .expect("the handler is executable");
            }
            Self { directory, path }
        }

        fn program(&self) -> String {
            self.path.to_string_lossy().into_owned()
        }
    }

    impl Drop for FakeHandler {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }

    fn opener(handler: &FakeHandler, timeout: Duration) -> ProgramOpener {
        ProgramOpener::with_program(handler.program(), Vec::new(), timeout)
    }

    #[cfg(unix)]
    #[test]
    fn a_handler_that_succeeds_is_reported_as_handed_over() {
        let handler = FakeHandler::new("exit 0");
        let outcome = opener(&handler, Duration::from_secs(5))
            .open(&OpenTarget::Url("https://example.invalid/docs".to_owned()))
            .expect("the URL is acceptable");

        match outcome {
            OpenOutcome::HandedOver { exit, .. } => assert_eq!(exit, 0),
            other => panic!("expected HandedOver, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_handler_that_fails_is_reported_with_its_own_code_and_output() {
        let handler = FakeHandler::new("echo 'no browser is registered' >&2; exit 3");
        let outcome = opener(&handler, Duration::from_secs(5))
            .open(&OpenTarget::Url("https://example.invalid/docs".to_owned()))
            .expect("the URL is acceptable");

        match outcome {
            OpenOutcome::HandlerFailed { exit, detail, .. } => {
                assert_eq!(exit, Some(3));
                assert!(detail.contains("no browser is registered"), "{detail}");
            }
            other => panic!("expected HandlerFailed, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_handler_that_outlives_the_timeout_is_reported_as_still_running() {
        let handler = FakeHandler::new("sleep 30");
        let started = std::time::Instant::now();
        let outcome = opener(&handler, Duration::from_millis(200))
            .open(&OpenTarget::Url("https://example.invalid/slow".to_owned()))
            .expect("the URL is acceptable");

        match outcome {
            OpenOutcome::StillRunning { after, .. } => {
                assert_eq!(after, Duration::from_millis(200));
                assert!(
                    started.elapsed() >= after,
                    "the host reported a still-running handler before the timeout passed"
                );
            }
            other => panic!("expected StillRunning, got {other:?}"),
        }
    }

    #[test]
    fn a_handler_that_is_not_installed_is_reported_as_missing() {
        let opener = ProgramOpener::with_program(
            "rustage-no-such-handler-program",
            Vec::new(),
            Duration::from_secs(1),
        );
        let outcome = opener
            .open(&OpenTarget::Url("https://example.invalid".to_owned()))
            .expect("the URL is acceptable");

        match &outcome {
            OpenOutcome::HandlerMissing { program, reason } => {
                assert_eq!(program, "rustage-no-such-handler-program");
                assert!(!reason.is_empty());
                assert!(!outcome.started_the_handler());
            }
            other => panic!("expected HandlerMissing, got {other:?}"),
        }
    }

    #[test]
    fn an_option_shaped_target_is_refused_before_a_program_is_started() {
        // `xdg-open -- <path>` is itself an "unexpected option" error, so escaping is not an
        // option: the refusal is the only honest answer.
        let refusal = super::accept(&OpenTarget::Url("-open-soon".to_owned()))
            .expect_err("an option-shaped target is refused");
        assert!(matches!(refusal, OpenRefusal::OptionLike { .. }));

        let path_refusal = super::accept(&OpenTarget::Path(PathBuf::from("-rf")));
        assert!(matches!(path_refusal, Err(OpenRefusal::OptionLike { .. })));
    }

    #[test]
    fn a_url_without_an_allowed_scheme_is_refused() {
        assert!(matches!(
            super::accept(&OpenTarget::Url("example.com/docs".to_owned())),
            Err(OpenRefusal::MissingScheme { .. })
        ));
        assert!(matches!(
            super::accept(&OpenTarget::Url("javascript:alert(1)".to_owned())),
            Err(OpenRefusal::UnsupportedScheme { scheme }) if scheme == "javascript"
        ));
        assert!(matches!(
            super::accept(&OpenTarget::Url("  ".to_owned())),
            Err(OpenRefusal::Empty)
        ));
        for scheme in ALLOWED_SCHEMES {
            assert!(
                super::accept(&OpenTarget::Url(format!("{scheme}://example.invalid"))).is_ok(),
                "{scheme} should be allowed"
            );
        }
    }

    #[test]
    fn a_target_with_a_control_character_is_refused() {
        let refusal = super::accept(&OpenTarget::Url("https://example.invalid/a\nb".to_owned()))
            .expect_err("a newline cannot be an argument");
        assert!(matches!(refusal, OpenRefusal::ControlCharacter { .. }));
    }

    #[test]
    fn a_path_that_is_not_there_is_refused_rather_than_handed_over() {
        let missing = std::env::temp_dir().join("rustage-definitely-not-here");
        assert!(matches!(
            super::accept(&OpenTarget::Path(missing)),
            Err(OpenRefusal::MissingPath { .. })
        ));
    }

    #[test]
    fn every_platform_the_host_claims_names_its_handler() {
        assert_eq!(
            program_for("macos").map(|(program, _)| program).as_deref(),
            Some("open")
        );
        assert_eq!(
            program_for("linux").map(|(program, _)| program).as_deref(),
            Some("xdg-open")
        );
        assert_eq!(
            program_for("windows")
                .map(|(program, _)| program)
                .as_deref(),
            Some("cmd")
        );
        // An unknown platform gets no guess, which is what keeps a wrong program off an untested
        // system.
        assert!(program_for("plan9").is_none());
        assert!(matches!(
            ProgramOpener::for_this_platform(),
            Ok(_) | Err(OpenRefusal::UnsupportedPlatform { .. })
        ));
    }
}
