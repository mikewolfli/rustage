// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Opening a URL or a file with the operating system's own handler. Contract **HOST**.
//!
//! The designer needs this for a documentation or help link and for "reveal this file" handoffs.
//! The *platform* half is the library's: [`rust_widgets::platform::Platform::open_external`] owns
//! the `open` / `xdg-open` / `cmd` mapping and the argument-shape refusal, and a backend that has
//! no handler reports that through its capability flags (blue22 H-15, principle #59). This module
//! adds only the host policy the designer needs on top:
//!
//! * **A target the designer must never launch is refused.** `javascript:`, `data:` and private
//!   application schemes are not URLs a designer has business opening; a URL without a scheme and
//!   a missing path are refused for the same reason (principle #29).
//! * **"Handed off" and "finished" are different answers.** The library returns once the target
//!   has been handed to the host handler, not when that handler exits. [`OpenOutcome::HandedOff`]
//!   says exactly that, while a controlled handler the tests start still reports its own exit code
//!   through [`OpenOutcome::HandedOver`] (principle #45).

use std::fmt;
#[cfg(test)]
use std::io::Read;
use std::path::PathBuf;
#[cfg(test)]
use std::process::{Command, Stdio};
use std::time::Duration;
#[cfg(test)]
use std::time::Instant;

/// How long a controlled test handler may take before the test opener reports it still running.
#[cfg(test)]
pub const DEFAULT_HANDOVER_TIMEOUT: Duration = Duration::from_secs(10);

/// How many times a start is retried when the kernel says the file is still open for writing.
#[cfg(test)]
const EXECUTABLE_BUSY_ATTEMPTS: usize = 50;

/// How long to wait between those retries.
#[cfg(test)]
const EXECUTABLE_BUSY_RETRY: Duration = Duration::from_millis(10);

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
    /// The platform handler refused a target the host's own checks accepted.
    HandlerRefused { target: String },
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
            Self::HandlerRefused { target } => {
                format!("the platform handler refused {target}")
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
    /// The target was handed to the platform's handler, which owns its own lifetime.
    ///
    /// This is the upstream `open_external` answer: the request left the process, but the handler
    /// has not necessarily opened anything yet. It is deliberately distinct from
    /// [`Self::HandedOver`], which is the exit code a handler the tests start reports.
    HandedOff {
        /// The target handed over.
        target: String,
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
    /// The handler could not be started: it is not installed, or the operating system refused to
    /// start it. The reason is the OS's own, so a refusal caused by something else — a file still
    /// open for writing, a script with no interpreter — stays distinguishable from "not installed"
    /// instead of everything landing under that one word.
    HandlerNotStarted {
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
        matches!(self, Self::HandedOver { .. } | Self::HandedOff { .. })
    }

    /// Whether the OS actually started something. [`Self::HandlerNotStarted`] is the only answer
    /// that says the program never ran.
    pub fn started_the_handler(&self) -> bool {
        !matches!(self, Self::HandlerNotStarted { .. })
    }

    /// A sentence for a log line or a diagnostic.
    pub fn message(&self) -> String {
        match self {
            Self::HandedOver { program, exit } => {
                format!("{program} opened the target and exited with {exit}")
            }
            Self::HandedOff { target } => {
                format!("the platform accepted {target}; its handler owns the rest")
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
            Self::HandlerNotStarted { program, reason } => {
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

/// The platform's own handler, as the library reports it.
///
/// The mapping (`open` on macOS, `xdg-open` on Linux, `cmd /C start` on Windows) and the
/// argument-shape refusal live in the library's platform backend. This type adds the host's own
/// target policy ([`accept`]) and reports the handoff honestly (blue22 H-15).
pub struct PlatformOpener;

impl PlatformOpener {
    /// The opener for the machine this host is running on.
    ///
    /// Returns `Err` when the backend reports no external-opening capability, rather than guessing
    /// a handler program nobody tested.
    pub fn for_this_platform() -> Result<Self, OpenRefusal> {
        if rust_widgets::platform::platform_facts()
            .capabilities()
            .open_external
        {
            Ok(Self)
        } else {
            Err(OpenRefusal::UnsupportedPlatform {
                os: std::env::consts::OS.to_owned(),
            })
        }
    }
}

impl UrlOpener for PlatformOpener {
    fn open(&self, target: &OpenTarget) -> Result<OpenOutcome, OpenRefusal> {
        let argument = accept(target)?;
        match rust_widgets::platform::platform_facts().open_external(&argument) {
            Ok(()) => Ok(OpenOutcome::HandedOff { target: argument }),
            Err(rust_widgets::platform::OpenError::Unsupported) => {
                Err(OpenRefusal::UnsupportedPlatform {
                    os: std::env::consts::OS.to_owned(),
                })
            }
            Err(rust_widgets::platform::OpenError::NoHandler) => {
                Ok(OpenOutcome::HandlerNotStarted {
                    program: "the platform handler".to_owned(),
                    reason: "no default handler is installed for this target".to_owned(),
                })
            }
            Err(rust_widgets::platform::OpenError::InvalidTarget) => {
                Err(OpenRefusal::HandlerRefused { target: argument })
            }
        }
    }
}

/// A test-only opener around an explicit program, so a handler can be started and its exit code
/// observed without asking the platform to open anything.
#[cfg(test)]
pub struct ProgramOpener {
    program: String,
    prefix_arguments: Vec<String>,
    timeout: Duration,
}

#[cfg(test)]
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

    /// The program this opener starts.
    pub fn program(&self) -> &str {
        &self.program
    }

    /// Starts the handler, retrying the one transient refusal the kernel can produce here.
    ///
    /// `ETXTBSY` — "text file busy" — is what the operating system answers when a file that some
    /// process still holds open for writing is executed. In a multi-threaded host it is genuinely
    /// transient: a thread that forked while another was writing a file inherits that descriptor
    /// until it execs, so the refusal lasts milliseconds and then goes away. Reporting it as "the
    /// handler is not installed" would send a user to look for a program that is right there, so the
    /// start is retried a bounded number of times and the reason is carried out if it persists.
    fn spawn(&self, argument: &str) -> std::io::Result<std::process::Child> {
        let mut attempt = 0;
        loop {
            let mut command = Command::new(&self.program);
            command.args(&self.prefix_arguments);
            command.arg(argument);
            command.stdin(Stdio::null());
            command.stdout(Stdio::null());
            command.stderr(Stdio::piped());
            match command.spawn() {
                Ok(child) => return Ok(child),
                Err(error)
                    if error.kind() == std::io::ErrorKind::ExecutableFileBusy
                        && attempt < EXECUTABLE_BUSY_ATTEMPTS =>
                {
                    attempt += 1;
                    std::thread::sleep(EXECUTABLE_BUSY_RETRY);
                }
                Err(error) => return Err(error),
            }
        }
    }
}

#[cfg(test)]
impl UrlOpener for ProgramOpener {
    fn open(&self, target: &OpenTarget) -> Result<OpenOutcome, OpenRefusal> {
        let argument = accept(target)?;
        let mut child = match self.spawn(&argument) {
            Ok(child) => child,
            Err(error) => {
                return Ok(OpenOutcome::HandlerNotStarted {
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
        ALLOWED_SCHEMES, OpenOutcome, OpenRefusal, OpenTarget, PlatformOpener, ProgramOpener,
        UrlOpener,
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

    #[cfg(unix)]
    #[test]
    fn a_handler_file_still_open_for_writing_is_retried_rather_than_reported_as_missing() {
        // Reproduces `ETXTBSY` on purpose: another process holds the script open for writing, which
        // is exactly what a forked thread's inherited descriptor does for a few milliseconds. The
        // host must wait it out rather than telling the user the program is not installed.
        let handler = FakeHandler::new("exit 0");
        let held = std::fs::OpenOptions::new()
            .write(true)
            .open(&handler.path)
            .expect("the script can be opened for writing");
        let mut holder = std::process::Command::new("sh")
            .arg("-c")
            .arg("sleep 0.2")
            .stdout(held)
            .spawn()
            .expect("the holder process starts");

        let outcome = opener(&handler, Duration::from_secs(5))
            .open(&OpenTarget::Url("https://example.invalid/docs".to_owned()))
            .expect("the URL is acceptable");

        assert!(
            matches!(outcome, OpenOutcome::HandedOver { .. }),
            "{outcome:?}",
        );
        let _ = holder.wait();
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
            OpenOutcome::HandlerNotStarted { program, reason } => {
                assert_eq!(program, "rustage-no-such-handler-program");
                assert!(!reason.is_empty());
                assert!(!outcome.started_the_handler());
            }
            other => panic!("expected HandlerNotStarted, got {other:?}"),
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
    fn the_platform_opener_follows_the_library_capability() {
        let capability = rust_widgets::platform::platform_facts()
            .capabilities()
            .open_external;
        assert_eq!(PlatformOpener::for_this_platform().is_ok(), capability);
    }
}
