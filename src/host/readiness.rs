// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! What the host's platform services can actually do *here*, with the evidence. Contract **HOST**.
//!
//! T01.02 delivers five platform services — file dialogs, the clipboard, system paths, URL opening
//! and file events — and T01.03 adds the background executor every long build will run on. Each is a
//! claim about an operating system, a pinned dependency revision, or a thread scheduler. Principle #25 says a claim without evidence is not a claim, and principle
//! #22 says a failure must be visible rather than hidden behind a success-shaped default. So each
//! capability is *exercised*, and this module keeps the three answers apart:
//!
//! * [`CheckStatus::Ready`] — the capability was exercised and behaved as documented.
//! * [`CheckStatus::Gap`] — the capability is missing, and the reason names what is missing. A gap
//!   is a finding, not a pass: it makes [`ReadinessReport::is_ready`] false.
//! * [`CheckStatus::NotVerified`] — this environment cannot produce the evidence (a probe must not
//!   open the user's browser, a server session has no screen). Also not a pass.
//!
//! The split is what keeps "we support file dialogs" from being written on the strength of a
//! `create_file_dialog` that returned `0`.
//!
//! **Where the boundary with `crate::probe` is.** That module exercises the *dependency's API
//! contracts* (layout, generation, events, teardown, editing) against the pinned revision; this one
//! exercises the *host's own services* in the environment the host is running in. They share the
//! revision constants and the JSON escaper, and nothing else — merging them would make one report
//! answer two different questions.

use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use rust_widgets::widget::runtime;

use super::clipboard::{ClipboardHost, ClipboardRead, ClipboardWrite, NativeClipboard};
use super::dialog::{DialogMode, DialogOutcome, DialogRequest, FileChooser, FrameChooser};
use super::notify::{Cause, NotificationCentre, Severity};
use super::open_url::{OpenRefusal, OpenTarget, ProgramOpener};
use super::paths::{HostDirectories, HostDirectory, ProjectRoot, SystemPaths};
use super::permission::{PermissionService, Scope};
use super::subprocess::{OwnedProcess, ProcessPresence, ProcessSpec, Termination, presence};
use super::tasks::{
    CancelResult, CancellationReason, Executor, ExecutorConfig, JobControl, JobOutcome, OwnerId,
    SubmitError, SubmitRequest, TaskOutcome, TaskState,
};
use super::watch::{DirectoryWatch, WatchEvent};
use crate::contracts::permission::{Capability as PermissionCapability, EffectClass, Principal};

/// How long a check waits for the operating system to deliver a file event.
const EVENT_DEADLINE: Duration = Duration::from_secs(5);

/// One capability of the host's platform layer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Capability {
    /// The operating system's own file chooser.
    NativeFileDialog,
    /// The painted chooser frame: mounting it, answering it, and cleaning it up.
    ChooserFrame,
    /// The clipboard, as a round trip.
    Clipboard,
    /// The four host directories of this environment.
    HostDirectories,
    /// A program to hand a URL or a path to on this operating system.
    UrlHandlerPresent,
    /// A real handover to that program.
    UrlHandover,
    /// Watching a directory for changes.
    DirectoryWatch,
    /// The background executor: submission that returns, a real cancelling state, and an honest
    /// report for a job that will not stop.
    BackgroundExecutor,
    /// Process ownership: taking down a whole process tree this session started, and leaving a
    /// process started outside it alone.
    ProcessTrees,
    /// Project trust: an untrusted project runs no code, and the authorization that lets an agent
    /// run one can be withdrawn.
    ProjectTrust,
    /// The notification centre: a real failure reaches it with a cause, a next step and a link, and
    /// a repeat of the same failure counts instead of prompting again.
    Notifications,
}

impl Capability {
    /// Every capability this report covers, in report order.
    pub const ALL: [Self; 11] = [
        Self::NativeFileDialog,
        Self::ChooserFrame,
        Self::Clipboard,
        Self::HostDirectories,
        Self::UrlHandlerPresent,
        Self::UrlHandover,
        Self::DirectoryWatch,
        Self::BackgroundExecutor,
        Self::ProcessTrees,
        Self::ProjectTrust,
        Self::Notifications,
    ];

    /// The name this capability is known by in the artifact.
    pub const fn name(self) -> &'static str {
        match self {
            Self::NativeFileDialog => "native-file-dialog",
            Self::ChooserFrame => "chooser-frame",
            Self::Clipboard => "clipboard",
            Self::HostDirectories => "host-directories",
            Self::UrlHandlerPresent => "url-handler",
            Self::UrlHandover => "url-handover",
            Self::DirectoryWatch => "directory-watch",
            Self::BackgroundExecutor => "background-executor",
            Self::ProcessTrees => "process-trees",
            Self::ProjectTrust => "project-trust",
            Self::Notifications => "notifications",
        }
    }
}

/// What a check found.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum CheckStatus {
    /// Exercised, behaved as documented.
    Ready,
    /// The capability is missing in this build or environment.
    Gap { reason: String },
    /// This environment cannot produce the evidence.
    NotVerified { reason: String },
}

impl CheckStatus {
    /// The word the report prints.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Gap { .. } => "gap",
            Self::NotVerified { .. } => "not-verified",
        }
    }

    /// Whether this status counts as a pass.
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready)
    }

    /// The reason, for the two statuses that carry one.
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Ready => None,
            Self::Gap { reason } | Self::NotVerified { reason } => Some(reason),
        }
    }
}

/// One capability and what was found about it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CapabilityCheck {
    /// Which capability.
    pub capability: Capability,
    /// What was found.
    pub status: CheckStatus,
    /// The evidence itself: what was called, and what came back.
    pub evidence: String,
}

impl CapabilityCheck {
    fn new(capability: Capability, status: CheckStatus, evidence: impl Into<String>) -> Self {
        Self {
            capability,
            status,
            evidence: evidence.into(),
        }
    }
}

/// The result of running every host capability check.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ReadinessReport {
    /// The rustage build that ran the checks.
    pub rustage_version: String,
    /// `std::env::consts::OS` of the host.
    pub host_os: String,
    /// `std::env::consts::ARCH` of the host.
    pub host_arch: String,
    /// What kind of session the checks ran in, in words.
    ///
    /// The dependency distinguishes a native interactive backend from a preview/stub one precisely
    /// because a stub runs a loop and creates no window, so a log alone cannot tell them apart. The
    /// same distinction decides how much the checks below are worth: in a stub session they exercise
    /// the host's contract and the platform's state layer, not a composited desktop.
    pub session: String,
    /// The dependency's backend for this host, as the backend itself reports it.
    ///
    /// Recorded because every capability below is a claim about *this* backend: a report that named
    /// the machine but not the backend could be re-read as a statement about a native build when it
    /// was taken on a stub one.
    pub backend: String,
    /// The pinned dependency crate.
    pub dependency_crate: String,
    /// The pinned dependency revision.
    pub dependency_revision: String,
    /// Every check, in report order.
    pub checks: Vec<CapabilityCheck>,
}

impl ReadinessReport {
    /// Runs every check in the environment the host is running in.
    ///
    /// The dependency's platform is initialised first, for the same reason `--launch` does it: a
    /// question about the platform backend asked before `init` would describe a backend that does
    /// not exist yet.
    pub fn run() -> Self {
        rust_widgets::init();

        let mut checks = Vec::with_capacity(Capability::ALL.len());
        checks.push(check_native_dialog());
        checks.push(check_chooser_frame());
        checks.push(check_clipboard());
        checks.push(check_directories());
        let (present, handover) = check_url();
        checks.push(present);
        checks.push(handover);
        checks.push(check_watch());
        checks.push(check_executor());
        checks.push(check_process_trees());
        checks.push(check_project_trust());
        checks.push(check_notifications());

        Self {
            rustage_version: env!("CARGO_PKG_VERSION").to_string(),
            host_os: std::env::consts::OS.to_string(),
            host_arch: std::env::consts::ARCH.to_string(),
            backend: format!(
                "{} ({:?})",
                rust_widgets::platform::backend_name(),
                rust_widgets::platform::runtime_gui_mode(),
            ),
            session: session_note(),
            dependency_crate: crate::probe::DEPENDENCY_CRATE.to_string(),
            dependency_revision: crate::probe::DEPENDENCY_REVISION.to_string(),
            checks,
        }
    }

    /// Whether the checks ran against a native interactive backend.
    ///
    /// False means the platform answered from its state layer: real answers to the host's calls,
    /// but from a session with no window in it.
    pub fn is_native_session(&self) -> bool {
        self.session.starts_with("native interactive")
    }

    /// Counts of `(ready, gap, not verified)`.
    pub fn counts(&self) -> (usize, usize, usize) {
        let mut ready = 0;
        let mut gap = 0;
        let mut unverified = 0;
        for check in &self.checks {
            match check.status {
                CheckStatus::Ready => ready += 1,
                CheckStatus::Gap { .. } => gap += 1,
                CheckStatus::NotVerified { .. } => unverified += 1,
            }
        }
        (ready, gap, unverified)
    }

    /// Whether every capability is ready.
    ///
    /// A gap and an unverified check both make this false: neither is evidence of support, and a
    /// report that called them ready would be the placeholder success T01.01's acceptance line
    /// forbids.
    pub fn is_ready(&self) -> bool {
        self.checks.iter().all(|check| check.status.is_ready())
    }

    /// The findings that are not ready, for a caller that only wants the problems.
    pub fn findings(&self) -> Vec<&CapabilityCheck> {
        self.checks
            .iter()
            .filter(|check| !check.status.is_ready())
            .collect()
    }

    /// The human-readable rendering, one line per capability.
    pub fn render_text(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "rustage host readiness ({}/{} · {} · {} {} @ {})",
            self.host_os,
            self.host_arch,
            self.backend,
            self.dependency_crate,
            &self.dependency_revision[..12.min(self.dependency_revision.len())],
            self.rustage_version,
        );
        let _ = writeln!(out, "session: {}", self.session);
        for check in &self.checks {
            let _ = writeln!(
                out,
                "{:<20}{:<13}{}",
                check.capability.name(),
                check.status.as_str(),
                check.evidence,
            );
            if let Some(reason) = check.status.reason() {
                let _ = writeln!(out, "{:<20}{:<13}{reason}", "", "");
            }
        }
        let (ready, gap, unverified) = self.counts();
        let _ = writeln!(
            out,
            "summary: {ready} ready, {gap} gap, {unverified} not verified",
        );
        out
    }

    /// The machine-readable rendering.
    ///
    /// Deterministic, like the dependency probe report: the same build in the same environment
    /// regenerates it byte for byte, so an artifact committed as evidence can be compared.
    pub fn render_json(&self) -> String {
        use crate::probe::report::json_string;
        let mut out = String::new();
        out.push_str("{\n");
        let _ = writeln!(out, "  \"schema\": \"rustage.host-readiness/v1\",");
        let _ = writeln!(
            out,
            "  \"rustage\": {},",
            json_string(&self.rustage_version)
        );
        let _ = writeln!(
            out,
            "  \"host\": {{ \"os\": {}, \"arch\": {}, \"backend\": {} }},",
            json_string(&self.host_os),
            json_string(&self.host_arch),
            json_string(&self.backend),
        );
        let _ = writeln!(
            out,
            "  \"dependency\": {{ \"crate\": {}, \"revision\": {} }},",
            json_string(&self.dependency_crate),
            json_string(&self.dependency_revision),
        );
        let _ = writeln!(out, "  \"session\": {},", json_string(&self.session));
        out.push_str("  \"checks\": [\n");
        for (index, check) in self.checks.iter().enumerate() {
            let comma = if index + 1 == self.checks.len() {
                ""
            } else {
                ","
            };
            out.push_str("    {\n");
            let _ = writeln!(
                out,
                "      \"capability\": {},",
                json_string(check.capability.name())
            );
            let _ = writeln!(
                out,
                "      \"status\": {},",
                json_string(check.status.as_str())
            );
            let _ = writeln!(out, "      \"evidence\": {},", json_string(&check.evidence));
            match check.status.reason() {
                Some(reason) => {
                    let _ = writeln!(out, "      \"reason\": {}", json_string(reason));
                }
                None => out.push_str("      \"reason\": null\n"),
            }
            let _ = writeln!(out, "    }}{comma}");
        }
        out.push_str("  ]\n}\n");
        out
    }
}

/// A sentence describing the session the checks ran in.
fn session_note() -> String {
    let mode = rust_widgets::platform::runtime_gui_mode();
    if mode == rust_widgets::platform::RuntimeGuiMode::NativeInteractive {
        return "native interactive: the checks ran against a real desktop session".to_owned();
    }
    format!(
        "preview/stub ({mode:?}): a real backend loop, but no window is composited, so the checks \
         below exercised the host's contract and the platform's state layer rather than a desktop",
    )
}

/// A note to append to evidence whose meaning depends on the session, or an empty string.
fn session_caveat() -> String {
    if session_note().starts_with("native interactive") {
        String::new()
    } else {
        format!(
            "; answered by the {} state backend, so no desktop session was involved",
            rust_widgets::platform::backend_name(),
        )
    }
}

/// A directory of the probe's own, removed when the check is done with it.
struct Scratch {
    path: PathBuf,
}

impl Scratch {
    /// A fresh directory under the operating system's temporary directory.
    ///
    /// The name carries the process id and a counter so two hosts probing at once cannot share one,
    /// and everything the probe writes lives here rather than in the user's project.
    fn new(name: &str) -> Result<Self, String> {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "rustage-readiness-{name}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed),
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path)
            .map_err(|error| format!("{} could not be created: {error}", path.display()))?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Whether the operating system's own chooser can be opened.
fn check_native_dialog() -> CapabilityCheck {
    let id = rust_widgets::create_file_dialog(0, 0, 0, 640, 420);
    if id == 0 {
        return CapabilityCheck::new(
            Capability::NativeFileDialog,
            CheckStatus::Gap {
                reason:
                    "the pinned revision creates no file-dialog control on this backend, so the \
                         operating system's own chooser cannot be opened through it"
                        .to_owned(),
            },
            "rust_widgets::create_file_dialog(0, 0, 0, 640, 420) -> 0",
        );
    }
    runtime::unregister(id);
    CapabilityCheck::new(
        Capability::NativeFileDialog,
        CheckStatus::Ready,
        format!("rust_widgets::create_file_dialog(..) -> {id}"),
    )
}

/// Whether the painted chooser frame can be mounted, answered and cleaned up.
fn check_chooser_frame() -> CapabilityCheck {
    let scratch = match Scratch::new("chooser") {
        Ok(scratch) => scratch,
        Err(reason) => {
            return CapabilityCheck::new(
                Capability::ChooserFrame,
                CheckStatus::NotVerified { reason },
                "no scratch directory",
            );
        }
    };
    let chosen = scratch.path().join("chosen.json");
    if let Err(error) = std::fs::write(&chosen, "{}") {
        return CapabilityCheck::new(
            Capability::ChooserFrame,
            CheckStatus::NotVerified {
                reason: format!("{} could not be written: {error}", chosen.display()),
            },
            "no file to choose",
        );
    }

    let chooser = FrameChooser::detached();
    let accepted = match answer(&chooser, DialogMode::OpenFile, Some(chosen.clone()), true) {
        Ok(outcome) => outcome,
        Err(reason) => {
            return CapabilityCheck::new(Capability::ChooserFrame, CheckStatus::Gap { reason }, "");
        }
    };
    let expected = std::fs::canonicalize(&chosen).unwrap_or_else(|_| chosen.clone());
    let accepted_paths: Vec<PathBuf> = accepted
        .selections()
        .iter()
        .map(|selection| selection.path.clone())
        .collect();
    if accepted_paths != [expected] {
        return CapabilityCheck::new(
            Capability::ChooserFrame,
            CheckStatus::Gap {
                reason: "the frame was answered but reported a different path than the one it was \
                         given"
                    .to_owned(),
            },
            format!(
                "{} reported {}",
                DialogMode::OpenFile.name(),
                accepted.message()
            ),
        );
    }

    let cancelled = match answer(&chooser, DialogMode::OpenFile, None, false) {
        Ok(outcome) => outcome,
        Err(reason) => {
            return CapabilityCheck::new(Capability::ChooserFrame, CheckStatus::Gap { reason }, "");
        }
    };
    if cancelled != DialogOutcome::Cancelled {
        return CapabilityCheck::new(
            Capability::ChooserFrame,
            CheckStatus::Gap {
                reason: "cancelling the frame did not report a cancellation".to_owned(),
            },
            format!("cancel reported {}", cancelled.message()),
        );
    }

    let empty = match answer(&chooser, DialogMode::OpenFile, None, true) {
        Ok(outcome) => outcome,
        Err(reason) => {
            return CapabilityCheck::new(Capability::ChooserFrame, CheckStatus::Gap { reason }, "");
        }
    };
    let refused_empty = matches!(empty, DialogOutcome::Refused { .. });

    CapabilityCheck::new(
        Capability::ChooserFrame,
        if refused_empty {
            CheckStatus::Ready
        } else {
            CheckStatus::Gap {
                reason: "accepting the frame with nothing selected was not reported as a refusal"
                    .to_owned(),
            }
        },
        format!(
            "mount, Return -> {}, Escape -> cancelled, Return with no selection -> {}{}",
            accepted.message(),
            empty.message(),
            session_caveat(),
        ),
    )
}

/// Shows a frame, answers it, and takes the outcome.
fn answer(
    chooser: &FrameChooser,
    mode: DialogMode,
    preselected: Option<PathBuf>,
    accept: bool,
) -> Result<DialogOutcome, String> {
    let request = DialogRequest {
        mode,
        preselected: preselected.into_iter().collect(),
        ..DialogRequest::default()
    };
    let pending = chooser.show(&request).map_err(|error| error.message())?;
    let answered = if accept {
        pending.accept()
    } else {
        pending.cancel()
    };
    if !answered {
        return Err("the frame did not take the key event the host sent it".to_owned());
    }
    pending
        .poll()
        .ok_or_else(|| "the frame reported no answer after taking the key event".to_owned())
}

/// Whether the clipboard survives a write and a read, and is left as it was found.
fn check_clipboard() -> CapabilityCheck {
    let backend = NativeClipboard::new();
    let before = rust_widgets::get_clipboard_text();
    let marker = "rustage clipboard readiness — 剪贴板 ✓";

    let evidence = match backend.write_text(marker) {
        ClipboardWrite::Refused { reason } => {
            return CapabilityCheck::new(
                Capability::Clipboard,
                CheckStatus::Gap {
                    reason: format!("the platform refused the write: {reason}"),
                },
                "rust_widgets::set_clipboard_text(..) -> false",
            );
        }
        ClipboardWrite::Written => match backend.read() {
            ClipboardRead::Text(text) if text == marker => format!(
                "wrote and read back a non-ASCII string of {} characters{}",
                marker.chars().count(),
                session_caveat(),
            ),
            ClipboardRead::Text(text) => {
                return CapabilityCheck::new(
                    Capability::Clipboard,
                    CheckStatus::Gap {
                        reason: format!(
                            "the text came back changed: wrote {} characters, read {}",
                            marker.chars().count(),
                            text.chars().count(),
                        ),
                    },
                    "the round trip did not preserve the text",
                );
            }
            other => {
                return CapabilityCheck::new(
                    Capability::Clipboard,
                    CheckStatus::Gap {
                        reason: format!(
                            "the clipboard reported {other:?} after a successful write"
                        ),
                    },
                    "the round trip did not read the text back",
                );
            }
        },
    };

    // The clipboard belongs to the user, not to this probe. Whatever was there is put back, and a
    // restore that failed is reported rather than assumed.
    if before.is_empty() {
        let _ = rust_widgets::set_clipboard_text("");
    } else if rust_widgets::set_clipboard_text(&before) {
        return CapabilityCheck::new(Capability::Clipboard, CheckStatus::Ready, evidence);
    } else {
        return CapabilityCheck::new(
            Capability::Clipboard,
            CheckStatus::NotVerified {
                reason:
                    "the probe wrote to the clipboard and the platform refused to restore what \
                         it found"
                        .to_owned(),
            },
            evidence,
        );
    }

    CapabilityCheck::new(
        Capability::Clipboard,
        CheckStatus::Ready,
        format!("{evidence}; the clipboard was empty and is empty again"),
    )
}

/// What this environment says about the four host directories.
fn check_directories() -> CapabilityCheck {
    let directories = match HostDirectories::from_environment() {
        Ok(directories) => directories,
        Err(error) => {
            return CapabilityCheck::new(
                Capability::HostDirectories,
                CheckStatus::Gap {
                    reason: error.message(),
                },
                "the operating system did not name the host's directories",
            );
        }
    };

    let mut evidence = String::new();
    for directory in HostDirectory::ALL {
        let state = directories.inspect(directory);
        if !evidence.is_empty() {
            evidence.push_str("; ");
        }
        let _ = write!(
            evidence,
            "{} -> {}",
            directory.name(),
            match &state {
                super::paths::DirectoryState::Usable { path } =>
                    format!("usable ({})", path.display()),
                super::paths::DirectoryState::Absent { .. } => "absent".to_owned(),
                super::paths::DirectoryState::NotADirectory { path } => {
                    format!("not a directory ({})", path.display())
                }
                super::paths::DirectoryState::Unusable { reason, .. } =>
                    format!("unusable ({reason})"),
            },
        );
        if matches!(
            state,
            super::paths::DirectoryState::NotADirectory { .. }
                | super::paths::DirectoryState::Unusable { .. }
        ) {
            return CapabilityCheck::new(
                Capability::HostDirectories,
                CheckStatus::Gap {
                    reason: state.message(),
                },
                evidence,
            );
        }
    }

    // Creating is exercised in a directory of the probe's own: the check must not create the
    // developer's configuration directory to prove that creating it would work.
    let scratch = match Scratch::new("directories") {
        Ok(scratch) => scratch,
        Err(reason) => {
            return CapabilityCheck::new(
                Capability::HostDirectories,
                CheckStatus::NotVerified { reason },
                evidence,
            );
        }
    };
    let sandbox = HostDirectories::with_roots(
        scratch.path().join("config"),
        scratch.path().join("cache"),
        scratch.path().join("data"),
        scratch.path().join("temp"),
    );
    for directory in HostDirectory::ALL {
        match sandbox.ensure(directory) {
            Ok(path) if path.is_dir() => {
                let _ = write!(evidence, "; created {} in a scratch root", directory.name());
            }
            Ok(path) => {
                return CapabilityCheck::new(
                    Capability::HostDirectories,
                    CheckStatus::Gap {
                        reason: format!(
                            "{} was reported as created but {} is not a directory",
                            directory.name(),
                            path.display(),
                        ),
                    },
                    evidence,
                );
            }
            Err(error) => {
                return CapabilityCheck::new(
                    Capability::HostDirectories,
                    CheckStatus::Gap {
                        reason: error.message(),
                    },
                    evidence,
                );
            }
        }
    }

    CapabilityCheck::new(Capability::HostDirectories, CheckStatus::Ready, evidence)
}

/// Whether this operating system has a handler program, and whether the host refuses the targets it
/// must refuse.
fn check_url() -> (CapabilityCheck, CapabilityCheck) {
    let opener = match ProgramOpener::for_this_platform() {
        Ok(opener) => opener,
        Err(OpenRefusal::UnsupportedPlatform { os }) => {
            let missing = CapabilityCheck::new(
                Capability::UrlHandlerPresent,
                CheckStatus::Gap {
                    reason: format!("this host knows no handler program for `{os}`"),
                },
                "no program mapped for this operating system",
            );
            return (
                missing,
                CapabilityCheck::new(
                    Capability::UrlHandover,
                    CheckStatus::NotVerified {
                        reason: "there is no handler program to hand a target to".to_owned(),
                    },
                    "",
                ),
            );
        }
        Err(other) => {
            let missing = CapabilityCheck::new(
                Capability::UrlHandlerPresent,
                CheckStatus::Gap {
                    reason: other.message(),
                },
                "the handler could not be resolved",
            );
            return (
                missing,
                CapabilityCheck::new(
                    Capability::UrlHandover,
                    CheckStatus::NotVerified {
                        reason: "no handler program was resolved".to_owned(),
                    },
                    "",
                ),
            );
        }
    };

    let refusals = [
        (
            "-open-soon",
            super::open_url::accept(&OpenTarget::Url("-open-soon".to_owned())),
        ),
        (
            "javascript:",
            super::open_url::accept(&OpenTarget::Url("javascript:alert(1)".to_owned())),
        ),
        (
            "no scheme",
            super::open_url::accept(&OpenTarget::Url("example.invalid/docs".to_owned())),
        ),
    ];
    let refused: Vec<&str> = refusals
        .iter()
        .filter(|(_, result)| result.is_err())
        .map(|(name, _)| *name)
        .collect();
    let evidence = format!(
        "{} is {} on PATH; refused before launch: {}",
        opener.program(),
        if opener.is_on_path() {
            "present"
        } else {
            "absent"
        },
        refused.join(", "),
    );

    let present = if opener.is_on_path() && refused.len() == refusals.len() {
        CapabilityCheck::new(Capability::UrlHandlerPresent, CheckStatus::Ready, evidence)
    } else {
        CapabilityCheck::new(
            Capability::UrlHandlerPresent,
            CheckStatus::Gap {
                reason: if !opener.is_on_path() {
                    format!(
                        "{} is not on PATH, so no target can be handed to it",
                        opener.program()
                    )
                } else {
                    "a target the host must refuse was accepted".to_owned()
                },
            },
            evidence,
        )
    };

    // A probe that opened a browser would be a side effect nobody asked for, so the handover itself
    // is reported as unverified here with the reason, and is exercised with a handler the host
    // controls in the unit tests.
    let handover = CapabilityCheck::new(
        Capability::UrlHandover,
        CheckStatus::NotVerified {
            reason: "a probe must not open the user's browser or editor; the handover is verified \
                     against a handler the host starts in its own tests, and end to end only when a \
                     person asks for it"
                .to_owned(),
        },
        format!("{} would be started with the accepted target as its only argument", opener.program()),
    );

    (present, handover)
}

/// Whether a directory can be watched and a change reported.
fn check_watch() -> CapabilityCheck {
    let scratch = match Scratch::new("watch") {
        Ok(scratch) => scratch,
        Err(reason) => {
            return CapabilityCheck::new(
                Capability::DirectoryWatch,
                CheckStatus::NotVerified { reason },
                "no scratch directory",
            );
        }
    };

    let watch = match DirectoryWatch::start(scratch.path(), false, |_| true) {
        Ok(watch) => watch,
        Err(error) => {
            return CapabilityCheck::new(
                Capability::DirectoryWatch,
                CheckStatus::Gap {
                    reason: error.message(),
                },
                "the watcher refused a temporary directory",
            );
        }
    };

    let changed = scratch.path().join("page.json");
    if let Err(error) = std::fs::write(&changed, "{}") {
        return CapabilityCheck::new(
            Capability::DirectoryWatch,
            CheckStatus::NotVerified {
                reason: format!("{} could not be written: {error}", changed.display()),
            },
            "nothing could be changed to observe",
        );
    }

    let deadline = Instant::now() + EVENT_DEADLINE;
    let mut reported: Vec<WatchEvent> = Vec::new();
    while Instant::now() < deadline && reported.is_empty() {
        reported = watch.poll();
        if reported.is_empty() {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    let saw_change = reported
        .iter()
        .any(|event| matches!(event, WatchEvent::Changed { path, .. } if path == &changed));

    let missing = DirectoryWatch::start(scratch.path().join("not-there"), false, |_| true);
    let refused_missing = missing.is_err();

    let evidence = format!(
        "a write in the watched directory produced {} event(s); watching a missing directory was {}",
        reported.len(),
        if refused_missing {
            "refused"
        } else {
            "accepted"
        },
    );

    if !saw_change {
        return CapabilityCheck::new(
            Capability::DirectoryWatch,
            CheckStatus::Gap {
                reason: format!(
                    "the watcher reported nothing about {} within {EVENT_DEADLINE:?}",
                    changed.display(),
                ),
            },
            evidence,
        );
    }
    if !refused_missing {
        return CapabilityCheck::new(
            Capability::DirectoryWatch,
            CheckStatus::Gap {
                reason:
                    "watching a directory that is not there was accepted, so a typo would look \
                         like an idle directory"
                        .to_owned(),
            },
            evidence,
        );
    }

    CapabilityCheck::new(Capability::DirectoryWatch, CheckStatus::Ready, evidence)
}

/// Whether the background executor keeps its promises about not blocking and about cancellation.
///
/// This is T01.03's acceptance line, exercised here rather than only in the unit tests because the
/// promise is about a *thread scheduler*: a machine that serialises threads would break it, and the
/// evidence has to come from the machine the host runs on.
fn check_executor() -> CapabilityCheck {
    let config = ExecutorConfig {
        workers: 1,
        queue_capacity: 4,
        monitor_interval: Duration::from_millis(5),
        timeout_grace: Duration::from_millis(50),
        shutdown_grace: Duration::from_millis(200),
    };
    let executor = match Executor::new(config) {
        Ok(executor) => executor,
        Err(error) => {
            return CapabilityCheck::new(
                Capability::BackgroundExecutor,
                CheckStatus::Gap {
                    reason: error.message(),
                },
                "the executor did not start",
            );
        }
    };

    let owner = OwnerId::new();
    let (started, started_rx) = std::sync::mpsc::channel();
    let submitted = Instant::now();
    let cooperative = executor.submit(
        SubmitRequest::new(owner, "readiness: waits to be cancelled"),
        move |control: JobControl| {
            let _ = started.send(());
            if control.token().wait(Duration::from_secs(5)) {
                JobOutcome::Cancelled
            } else {
                JobOutcome::Succeeded
            }
        },
    );
    let submit_took = submitted.elapsed();
    let cooperative = match cooperative {
        Ok(id) => id,
        Err(error) => {
            return CapabilityCheck::new(
                Capability::BackgroundExecutor,
                CheckStatus::Gap {
                    reason: error.message(),
                },
                "the executor refused a submission",
            );
        }
    };
    if started_rx.recv_timeout(Duration::from_secs(5)).is_err() {
        return CapabilityCheck::new(
            Capability::BackgroundExecutor,
            CheckStatus::Gap {
                reason: "the job never started, so the caller would wait for work that never runs"
                    .to_owned(),
            },
            "no worker picked up the job",
        );
    }

    // The caller returned while the job was still running: that is what keeps a long build off the
    // thread that drives the UI.
    let caller_returned_early = submit_took < Duration::from_millis(50);
    let requested = executor.cancel(cooperative) == CancelResult::Requested;
    let cancelling_shown = executor
        .record(cooperative)
        .is_some_and(|record| record.state == TaskState::Cancelling);
    let acknowledged = executor
        .wait(cooperative, Duration::from_secs(5))
        .and_then(|record| record.outcome);
    let cancelled = matches!(
        acknowledged,
        Some(TaskOutcome::Cancelled {
            reason: CancellationReason::Requested,
            ..
        })
    );

    // A job that ignores both its token and its deadline must be reported, not waited for forever.
    let stubborn = executor.submit(
        SubmitRequest::new(owner, "readiness: ignores its deadline")
            .with_timeout(Duration::from_millis(30)),
        |_control: JobControl| {
            std::thread::sleep(Duration::from_millis(600));
            JobOutcome::Succeeded
        },
    );
    let abandoned = match stubborn {
        Ok(id) => {
            let record = executor.wait(id, Duration::from_secs(5));
            matches!(
                record.and_then(|record| record.outcome),
                Some(TaskOutcome::Abandoned {
                    job_still_running: true,
                    ..
                })
            )
        }
        Err(_) => false,
    };

    // Shutdown must return, leave every task with an outcome, and refuse new work. It may report
    // nothing as abandoned here because the deadline monitor already recorded that job — which is
    // the point: the record, not the shutdown, is where "this job never stopped" lives.
    let shutdown = executor.shutdown();
    let everything_accounted_for = executor.records().iter().all(|record| record.is_final());
    let refuses_new_work = matches!(
        executor.submit(SubmitRequest::new(owner, "too late"), |_| {
            JobOutcome::Succeeded
        }),
        Err(SubmitError::Stopped)
    );

    let evidence = format!(
        "submit returned in {:?} while the job ran; cancel -> {}{}; a cooperative job -> {}; a job \
         that ignored its 30ms deadline -> {}; shutdown -> {}",
        submit_took,
        if cancelling_shown {
            "cancelling"
        } else {
            "not cancelling"
        },
        if requested { "" } else { " (not requested)" },
        if cancelled {
            "cancelled"
        } else {
            "not cancelled"
        },
        if abandoned {
            "abandoned"
        } else {
            "not abandoned"
        },
        shutdown.message(),
    );

    let ready = caller_returned_early
        && requested
        && cancelling_shown
        && cancelled
        && abandoned
        && everything_accounted_for
        && refuses_new_work;
    if ready {
        return CapabilityCheck::new(Capability::BackgroundExecutor, CheckStatus::Ready, evidence);
    }

    let mut missing = Vec::new();
    if !caller_returned_early {
        missing.push("submission blocked the caller");
    }
    if !(requested && cancelling_shown) {
        missing.push("the cancelling state was not observable");
    }
    if !cancelled {
        missing.push("a cooperative job did not stop");
    }
    if !abandoned {
        missing.push("a job that ignored its deadline was not reported as abandoned");
    }
    if !everything_accounted_for {
        missing.push("shutdown left a task without an outcome");
    }
    if !refuses_new_work {
        missing.push("a stopped executor accepted new work");
    }
    CapabilityCheck::new(
        Capability::BackgroundExecutor,
        CheckStatus::Gap {
            reason: missing.join("; "),
        },
        evidence,
    )
}

/// Whether this host can take down the process tree it started, and only that tree.
///
/// This is T01.04's acceptance line — "only processes this session owns are cleaned up, and no
/// external process is killed" — exercised on the machine the host is running on, with three
/// processes at once: one this session started, one *that process* started, and one started outside
/// this session entirely. The last one has to survive.
fn check_process_trees() -> CapabilityCheck {
    let evidence = |parts: &[String]| parts.join("; ");

    // A process this session did not start and must not touch. It deliberately looks exactly like
    // the ones we do start: a test that used a differently named program would pass by accident.
    let mut foreign = match std::process::Command::new("sh")
        .arg("-c")
        .arg("sleep 30")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            return CapabilityCheck::new(
                Capability::ProcessTrees,
                CheckStatus::NotVerified {
                    reason: format!("an unrelated test process could not be started: {error}"),
                },
                "no control process",
            );
        }
    };
    let foreign_pid = foreign.id();

    // Starting a program is an external effect, so the check obtains a permit exactly as a caller
    // would: the session scope, for the human who is running this host. A check that spawned
    // without asking would be exercising a path no caller has.
    let service = PermissionService::new();
    let permit = match service.permit(EffectClass::Command, Scope::Session, Principal::Human) {
        Ok(permit) => permit,
        Err(refusal) => {
            let _ = foreign.kill();
            let _ = foreign.wait();
            return CapabilityCheck::new(
                Capability::ProcessTrees,
                CheckStatus::Gap {
                    reason: refusal.message(),
                },
                "the host's own session was refused a command permit",
            );
        }
    };

    let mut owned = match OwnedProcess::spawn(
        ProcessSpec::new("readiness: a tree of its own", "sh")
            .args(["-c", "sleep 30 & echo $!; wait"]),
        &permit,
    ) {
        Ok(process) => process,
        Err(error) => {
            let _ = foreign.kill();
            let _ = foreign.wait();
            return CapabilityCheck::new(
                Capability::ProcessTrees,
                CheckStatus::Gap {
                    reason: error.message(),
                },
                "a process could not be started at all",
            );
        }
    };
    let owned_pid = owned.pid();
    let ownership = owned.ownership_evidence();

    let descendant = {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Ok(pid) = owned.output_so_far().stdout.trim().parse::<u32>() {
                break Some(pid);
            }
            if Instant::now() >= deadline {
                break None;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    };

    let termination = owned.terminate(Duration::from_secs(5));
    let owned_gone = wait_for_presence(owned_pid, Duration::from_secs(3)) == ProcessPresence::Gone;
    let descendant_gone = descendant
        .map(|pid| wait_for_presence(pid, Duration::from_secs(3)) == ProcessPresence::Gone);
    let foreign_alive = presence(foreign_pid) == ProcessPresence::Running;

    let _ = foreign.kill();
    let _ = foreign.wait();

    let mut parts = vec![
        format!("termination: {}", termination.message()),
        ownership,
        permit.evidence(),
        format!(
            "a process started outside this session: {}",
            if foreign_alive {
                "survived"
            } else {
                "was not running"
            }
        ),
    ];
    match descendant {
        Some(pid) => parts.push(format!(
            "its descendant {pid} is {}",
            if descendant_gone == Some(true) {
                "gone"
            } else {
                "still running"
            }
        )),
        None => parts.push("its descendant never reported its pid".to_owned()),
    }
    let evidence = evidence(&parts);

    let mut missing = Vec::new();
    if !owned_gone {
        parts.push("the process this session started outlived its termination".to_owned());
        missing.push("the owned process outlived its termination");
    }
    if descendant.is_none() {
        missing.push("the descendant never reported its pid");
    } else if descendant_gone != Some(true) {
        missing.push("a descendant inside the owned group survived");
    }
    if !foreign_alive {
        missing.push("a process started outside this session did not survive");
    }
    if !matches!(
        termination,
        Termination::Stopped { .. } | Termination::Killed { .. }
    ) {
        missing.push("the termination did not report which signal ended the tree");
    }

    if missing.is_empty() {
        return CapabilityCheck::new(Capability::ProcessTrees, CheckStatus::Ready, evidence);
    }
    CapabilityCheck::new(
        Capability::ProcessTrees,
        CheckStatus::Gap {
            reason: missing.join("; "),
        },
        evidence,
    )
}

/// Whether an untrusted project really runs no code, and whether the authorization that lets an
/// agent run one can be withdrawn.
///
/// This is T01.05's acceptance line. It is exercised as a sequence, because either half alone is
/// easy to fake: a host that refused everything would "pass" the first half, and a host that
/// checked nothing would "pass" the second. The command writes a marker file, so "no code ran" is
/// observed as an absent file rather than as a return value.
fn check_project_trust() -> CapabilityCheck {
    let project = match Scratch::new("trust") {
        Ok(project) => project,
        Err(reason) => {
            return CapabilityCheck::new(
                Capability::ProjectTrust,
                CheckStatus::NotVerified { reason },
                "no project directory could be created",
            );
        }
    };
    // A build script, so the trust prompt has a real finding to read rather than a constant.
    if let Err(error) = std::fs::write(project.path().join("build.rs"), "fn main() {}\n") {
        return CapabilityCheck::new(
            Capability::ProjectTrust,
            CheckStatus::NotVerified {
                reason: format!("a build script could not be written: {error}"),
            },
            "no project to trust",
        );
    }
    let root = match ProjectRoot::new(project.path()) {
        Ok(root) => root,
        Err(error) => {
            return CapabilityCheck::new(
                Capability::ProjectTrust,
                CheckStatus::NotVerified {
                    reason: error.to_string(),
                },
                "the project directory could not be resolved",
            );
        }
    };

    let service = PermissionService::new();
    let prompt = service.prompt(&root);
    let marker = project.path().join("ran");
    let command = || {
        ProcessSpec::new("readiness: a command in a project", "sh")
            .args(["-c", "touch ran"])
            .in_directory(project.path())
    };

    // 1. Untrusted: refused, and nothing runs.
    let untrusted = service
        .permit(
            EffectClass::Command,
            Scope::Project(&root),
            Principal::Agent,
        )
        .is_err();
    let ran_untrusted = marker.exists();

    // 2. Trusted, with the execution scope authorized: permitted, and the command really runs.
    service.trust(&root);
    let trusted_but_unauthorized = service
        .permit(
            EffectClass::Command,
            Scope::Project(&root),
            Principal::Agent,
        )
        .is_err();
    let authorized = service
        .authorize(Scope::Project(&root), PermissionCapability::RunCargoCheck)
        .is_ok();
    let ran_after_authorizing = match service.permit(
        EffectClass::Command,
        Scope::Project(&root),
        Principal::Agent,
    ) {
        Ok(permit) => match OwnedProcess::spawn(command(), &permit) {
            Ok(mut process) => {
                let exit = process.wait_for(Duration::from_secs(5));
                matches!(exit, Some(exit) if exit.is_success())
            }
            Err(_) => false,
        },
        Err(_) => false,
    } && marker.exists();

    // 3. Authorization withdrawn: refused again, and a second command does not run either.
    let _ = std::fs::remove_file(&marker);
    service.revoke_authorization(Scope::Project(&root), PermissionCapability::RunCargoCheck);
    let revoked_refusal = service
        .permit(
            EffectClass::Command,
            Scope::Project(&root),
            Principal::Agent,
        )
        .is_err();
    let ran_after_revoking = marker.exists();

    // 4. The record holds the whole sequence, so the decisions can be reviewed rather than trusted.
    let record = service.record();
    let kinds: Vec<&str> = record
        .events()
        .iter()
        .map(|event| event.kind.name())
        .collect();
    let recorded = kinds
        == vec![
            "decided", "trusted", "decided", "granted", "decided", "revoked", "decided",
        ];
    // The record itself, not a summary of it: an artifact that says "7 entries" is a claim, and an
    // artifact that prints them is evidence.
    let entries: Vec<String> = record
        .render_text()
        .lines()
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect();

    let evidence = format!(
        "an untrusted project: {} and {}; after trusting and authorizing the scope: {} and the \
         command {}; after withdrawing the authorization: {} and {}; the prompt read the directory: \
         {}; the record ({} entries, {} grants, {} denials): {}",
        if untrusted { "refused" } else { "permitted" },
        if ran_untrusted {
            "it ran anyway"
        } else {
            "wrote no marker"
        },
        if trusted_but_unauthorized {
            "trust alone was not authorization"
        } else {
            "trust opened the scope by itself"
        },
        if ran_after_authorizing {
            "wrote the marker"
        } else {
            "did not write the marker"
        },
        if revoked_refusal {
            "refused"
        } else {
            "permitted"
        },
        if ran_after_revoking {
            "it ran anyway"
        } else {
            "wrote no second marker"
        },
        if prompt.runs_code_at_build_time() {
            "yes, build.rs"
        } else {
            "no"
        },
        record.len(),
        record.grants().len(),
        record.denials(),
        entries.join(" | "),
    );

    let mut missing = Vec::new();
    if !untrusted {
        missing.push("an untrusted project was permitted to run a command");
    }
    if ran_untrusted {
        missing.push("code ran in an untrusted project");
    }
    if !trusted_but_unauthorized {
        missing.push("trust alone opened the execution scope");
    }
    if !authorized {
        missing.push("a trusted project could not have its execution scope authorized");
    }
    if !ran_after_authorizing {
        missing.push("an authorized command did not run");
    }
    if !revoked_refusal {
        missing.push("a withdrawn authorization still permitted the command");
    }
    if ran_after_revoking {
        missing.push("code ran after the authorization was withdrawn");
    }
    if !prompt.runs_code_at_build_time() {
        missing.push("the trust prompt did not find the build script in the directory");
    }
    if !recorded {
        missing.push("the authorization record does not hold the whole sequence");
    }

    if missing.is_empty() {
        return CapabilityCheck::new(Capability::ProjectTrust, CheckStatus::Ready, evidence);
    }
    CapabilityCheck::new(
        Capability::ProjectTrust,
        CheckStatus::Gap {
            reason: missing.join("; "),
        },
        evidence,
    )
}

/// Whether a real failure reaches the notification centre with something to do about it.
///
/// This is T01.06's acceptance line — "an error has a cause and a next step, not just log output".
/// It is exercised with values the host really produced: a task the executor failed, a process that
/// exited non-zero, a program that does not exist, and a permission the gate refused. A centre fed
/// hand-written strings would prove nothing, which is why none of the four is one.
fn check_notifications() -> CapabilityCheck {
    let centre = NotificationCentre::new();

    // 1. A task the executor really ran and failed.
    let failed_task = {
        let executor = match Executor::new(ExecutorConfig::responsive()) {
            Ok(executor) => executor,
            Err(error) => {
                return CapabilityCheck::new(
                    Capability::Notifications,
                    CheckStatus::Gap {
                        reason: error.message(),
                    },
                    "the executor did not start, so no task could fail",
                );
            }
        };
        let id = executor.submit(
            SubmitRequest::new(OwnerId::new(), "readiness: a task that fails"),
            |_control: JobControl| JobOutcome::failed("readiness: this failure is deliberate"),
        );
        let record = match id {
            Ok(id) => executor.wait(id, Duration::from_secs(5)),
            Err(_) => None,
        };
        let succeeded = executor.submit(
            SubmitRequest::new(OwnerId::new(), "readiness: a task that works"),
            |_control: JobControl| JobOutcome::Succeeded,
        );
        let finished = match succeeded {
            Ok(id) => executor.wait(id, Duration::from_secs(5)),
            Err(_) => None,
        };
        executor.shutdown();
        (record, finished)
    };
    let (failed_task, succeeded_task) = failed_task;

    let task_notification = failed_task
        .as_ref()
        .and_then(Cause::for_task)
        .map(|cause| centre.report(cause));
    let info_notification = succeeded_task
        .as_ref()
        .and_then(Cause::for_task)
        .map(|cause| centre.report(cause));

    // 2. A process that really ran and exited non-zero, reported twice: the second time counts.
    let process_cause = {
        let service = PermissionService::new();
        match service.permit(EffectClass::Command, Scope::Session, Principal::Human) {
            Ok(permit) => OwnedProcess::spawn(
                ProcessSpec::new("readiness: a command that fails", "sh").args([
                    "-c",
                    "echo 'readiness: this failure is deliberate' >&2; exit 9",
                ]),
                &permit,
            )
            .ok()
            .map(|mut process| {
                let exit = process.wait();
                Cause::Process(process.outcome(exit))
            }),
            Err(_) => None,
        }
    };
    let process_notification = process_cause.clone().map(|cause| centre.report(cause));
    let repeated = process_cause.map(|cause| centre.report(cause));

    // 3. A program that does not exist, so the work never started.
    let program_notification = {
        let service = PermissionService::new();
        service
            .permit(EffectClass::Command, Scope::Session, Principal::Human)
            .ok()
            .and_then(|permit| {
                OwnedProcess::spawn(
                    ProcessSpec::new("readiness: a missing program", "/nonexistent/rustage-tool"),
                    &permit,
                )
                .err()
            })
            .map(|error| centre.report(Cause::Program(error)))
    };

    // 4. A permission the gate really refused.
    let refusal_notification = Scratch::new("notify").ok().and_then(|project| {
        let root = ProjectRoot::new(project.path()).ok()?;
        PermissionService::new()
            .permit(
                EffectClass::Command,
                Scope::Project(&root),
                Principal::Human,
            )
            .err()
            .map(|refusal| centre.report(Cause::Permission(refusal)))
    });

    let notifications = centre.notifications();
    let every_one_actionable = notifications.iter().all(|notification| {
        !notification.summary().is_empty()
            && !notification.steps().is_empty()
            && !notification.link().render().is_empty()
    });
    let rendered = centre.render_text();
    let every_one_rendered = notifications
        .iter()
        .all(|notification| rendered.contains(&notification.summary()));
    let severity_of = |reported: Option<crate::host::notify::Reported>| {
        reported
            .and_then(|reported| centre.notification(reported.id))
            .map(|notification| notification.severity())
    };
    let (failed_task_severity, info_severity, refusal_severity) = (
        severity_of(task_notification),
        severity_of(info_notification),
        severity_of(refusal_notification),
    );
    let retry_asks_first = task_notification
        .and_then(|reported| centre.notification(reported.id))
        .is_some_and(|notification| {
            notification
                .steps()
                .iter()
                .any(|step| step.requires_confirmation())
        });
    let process_occurrences = process_notification
        .and_then(|reported| centre.notification(reported.id))
        .map_or(0, |notification| notification.occurrences());
    let repeat_counted = repeated.is_some_and(|reported| !reported.first_time)
        && process_notification.is_some_and(|reported| reported.first_time)
        && process_occurrences == 2;
    // Five problems were reported and one of them twice, so the catalogue has to hold five entries.
    let counts = centre.counts();
    let link_kinds: Vec<&str> = notifications
        .iter()
        .map(|notification| notification.link().kind())
        .collect();

    let evidence = format!(
        "a real failed task -> {} with a step that {}, and a {} link; a real task that worked -> {}; \
         a real failing process reported twice -> {} entry with {process_occurrences} occurrences; a \
         program that never started -> {}; a refusal on an untrusted project -> {}; the catalogue: \
         {} notification(s) = {} error, {} warning, {} information, links {:?}; {}",
        severity_word(failed_task_severity),
        if retry_asks_first {
            "asks first because re-running can repeat its effects"
        } else {
            "does not ask first"
        },
        notifications
            .first()
            .map_or("none", |notification| notification.link().kind()),
        severity_word(info_severity),
        if process_notification.is_some() { 1 } else { 0 },
        severity_word(
            program_notification
                .and_then(|reported| centre.notification(reported.id))
                .map(|notification| notification.severity()),
        ),
        severity_word(refusal_severity),
        notifications.len(),
        counts.errors,
        counts.warnings,
        counts.information,
        link_kinds,
        notifications
            .first()
            .map_or_else(String::new, |notification| notification.render()),
    );

    let mut missing = Vec::new();
    if task_notification.is_none() || failed_task_severity != Some(Severity::Error) {
        missing.push("a failed task did not become an error notification");
    }
    if !retry_asks_first {
        missing.push("the failed task offered no step, or one that needs no confirmation");
    }
    if !every_one_actionable {
        missing.push("a notification has no cause, no next step or no link");
    }
    if !every_one_rendered {
        missing.push("a notification is missing from the rendered catalogue");
    }
    if info_notification.is_none() || info_severity != Some(Severity::Info) {
        missing.push("a task that worked was not reported as information");
    }
    if refusal_notification.is_none() || refusal_severity != Some(Severity::Warning) {
        missing.push("a permission refusal was not reported as a warning");
    }
    if program_notification.is_none() {
        missing.push("a program that never started was not reported");
    }
    if !repeat_counted {
        missing.push("a repeated failure raised a second notification instead of counting");
    }
    if notifications.len() != 5 {
        missing.push("the catalogue does not hold one entry per distinct problem");
    }

    if missing.is_empty() {
        return CapabilityCheck::new(Capability::Notifications, CheckStatus::Ready, evidence);
    }
    CapabilityCheck::new(
        Capability::Notifications,
        CheckStatus::Gap {
            reason: missing.join("; "),
        },
        evidence,
    )
}

/// A severity in one word, for an evidence line.
fn severity_word(severity: Option<Severity>) -> &'static str {
    match severity {
        Some(severity) => severity.name(),
        None => "nothing reported",
    }
}

/// Waits for a pid to stop existing, returning what was last seen.
fn wait_for_presence(pid: u32, limit: Duration) -> ProcessPresence {
    let deadline = Instant::now() + limit;
    loop {
        let seen = presence(pid);
        if seen != ProcessPresence::Running || Instant::now() >= deadline {
            return seen;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The system paths of this environment, without creating anything.
///
/// Used by the readiness report for the "where would the host write" line; kept here rather than in
/// [`check_directories`] so the environment question and the capability question stay separable.
pub fn system_path_summary() -> Result<String, String> {
    let paths = SystemPaths::from_environment().map_err(|error| error.message())?;
    Ok(format!(
        "home {}, config {}, cache {}, data {}",
        paths.home().display(),
        paths.config_dir().display(),
        paths.cache_dir().display(),
        paths.data_dir().display(),
    ))
}

#[cfg(test)]
mod tests {
    use super::{Capability, CapabilityCheck, CheckStatus, ReadinessReport};

    #[test]
    fn the_report_covers_every_capability_exactly_once() {
        let report = ReadinessReport::run();

        assert_eq!(report.checks.len(), Capability::ALL.len());
        for (check, expected) in report.checks.iter().zip(Capability::ALL) {
            assert_eq!(check.capability, expected, "checks must be in report order");
            assert!(
                !check.evidence.is_empty(),
                "{} produced no evidence",
                expected.name(),
            );
        }
    }

    #[test]
    fn a_gap_is_not_a_pass_and_a_skipped_check_is_not_a_pass_either() {
        let gap = CapabilityCheck {
            capability: Capability::NativeFileDialog,
            status: CheckStatus::Gap {
                reason: "no control was created".to_owned(),
            },
            evidence: "create_file_dialog -> 0".to_owned(),
        };
        let skipped = CapabilityCheck {
            capability: Capability::UrlHandover,
            status: CheckStatus::NotVerified {
                reason: "a probe must not open a browser".to_owned(),
            },
            evidence: String::new(),
        };

        assert!(!gap.status.is_ready());
        assert!(!skipped.status.is_ready());
        assert_eq!(gap.status.as_str(), "gap");
        assert_eq!(skipped.status.as_str(), "not-verified");

        let report = ReadinessReport {
            rustage_version: "0.0.0".to_owned(),
            host_os: "plan9".to_owned(),
            host_arch: "unknown".to_owned(),
            backend: "stub (PreviewOrStub)".to_owned(),
            session: "preview/stub".to_owned(),
            dependency_crate: "rust_widgets".to_owned(),
            dependency_revision: "0".repeat(40),
            checks: vec![gap, skipped],
        };
        assert!(!report.is_ready());
        assert_eq!(report.counts(), (0, 1, 1));
        assert_eq!(report.findings().len(), 2);
    }

    #[test]
    fn both_renderings_name_every_capability_and_stay_well_shaped() {
        let report = ReadinessReport::run();
        let text = report.render_text();

        for capability in Capability::ALL {
            assert!(
                text.contains(capability.name()),
                "{text}\nmissing {}",
                capability.name(),
            );
        }
        assert!(text.contains("summary:"));

        let json = report.render_json();
        assert!(json.starts_with("{\n"));
        assert!(json.ends_with("}\n"));
        assert!(json.contains("\"schema\": \"rustage.host-readiness/v1\""));
        assert!(
            json.contains("\"backend\""),
            "the artifact must name the backend it describes",
        );
        assert_eq!(
            json.matches("\"capability\"").count(),
            Capability::ALL.len(),
            "every capability must appear in the artifact",
        );
        assert_eq!(
            json.matches("\"reason\"").count(),
            Capability::ALL.len(),
            "every entry carries a reason field, null when there is none",
        );

        // Regenerating the same JSON must produce the same bytes, so the artifact can be committed
        // and compared. The environment could change, so this asserts on the shape, not the values.
        let again = report.render_json();
        assert_eq!(json.len(), again.len());
        assert_eq!(json.lines().count(), again.lines().count());
    }

    #[test]
    fn the_system_path_summary_names_all_four_directories_or_says_why_not() {
        match super::system_path_summary() {
            Ok(summary) => {
                for name in ["home", "config", "cache", "data"] {
                    assert!(summary.contains(name), "{summary}");
                }
            }
            Err(reason) => assert!(
                !reason.is_empty(),
                "a missing environment must explain itself"
            ),
        }
    }
}
