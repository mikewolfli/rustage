// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Platform startup: what "the desktop session started" means, and what happens when it cannot.
//!
//! The dependency publishes [`RuntimeGuiMode`] and documents why it exists: a stub backend runs an
//! event loop, prints progress and creates no window, so its log looks exactly like a working one.
//! A host that skipped that question could therefore report success for a session with no window in
//! it — exactly the placeholder success T01.01's acceptance line rules out. [`launch`] asks first
//! and returns [`StartupOutcome::Refused`] with the backend and mode named when the answer is not
//! interactive.
//!
//! **Why the platform calls sit behind a trait.** Everything except the dependency calls can be
//! exercised without a display: a [`StartupPlatform`] fake lets the tests prove that a stub backend
//! refuses before creating anything, that a native backend creates a window and runs the loop in
//! that order, and that a loop failure is reported as a failure rather than as a completed session.
//! Without that seam, the only testable path would be the one that needs a GUI session — which is
//! the path that cannot run in CI.
//!
//! **What this module deliberately does not do.** It does not create controls, register services,
//! own a task queue or decide anything about windows beyond opening the baseline one. Those are
//! T01.02–T01.04 and T04.x; a startup path that grew into a service registry would be the
//! monolithic file the roadmap's module boundaries exist to prevent.

use std::time::{Duration, Instant};

use rust_widgets::platform::RuntimeGuiMode;

use super::error::{ExitCode, HostError, HostResult};
use super::log::{Level, emit};
use super::platform::NativePlatform;

/// What the caller asked the host to start.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StartupRequest {
    /// The window title.
    pub title: String,
    /// Window width in logical pixels.
    pub width: u32,
    /// Window height in logical pixels.
    pub height: u32,
    /// Leave the event loop after this long.
    ///
    /// The baseline start must be able to **end**: a smoke test that hangs is not evidence, and a
    /// person running it wants their terminal back. Zero means "run until something else quits".
    pub exit_after: Duration,
}

impl Default for StartupRequest {
    fn default() -> Self {
        Self {
            title: "rustage".to_owned(),
            width: 1280,
            height: 800,
            exit_after: Duration::from_millis(750),
        }
    }
}

impl StartupRequest {
    /// Rejects a request that cannot produce a window, before any platform call happens.
    pub fn validate(&self) -> HostResult<()> {
        if self.title.trim().is_empty() {
            return Err(HostError::WindowCreation {
                reason: "the title is empty".into(),
            });
        }
        if self.width == 0 || self.height == 0 {
            return Err(HostError::WindowCreation {
                reason: format!("geometry {}x{} has no area", self.width, self.height),
            });
        }
        Ok(())
    }
}

/// What a platform can tell the host and do for it.
pub trait StartupPlatform {
    /// The backend's name, for the report and the log.
    fn backend_name(&self) -> String;
    /// Whether the backend creates native windows and runs an interactive loop.
    fn gui_mode(&self) -> RuntimeGuiMode;
    /// Prepares the runtime. Called once, before anything else.
    fn init(&mut self) -> HostResult<()>;
    /// Creates the baseline window and returns its identity.
    fn create_window(&mut self, request: &StartupRequest) -> HostResult<u64>;
    /// Runs the event loop and says how it ended.
    fn run_loop(&mut self, request: &StartupRequest) -> HostResult<LoopEnd>;
    /// Closes the window created by [`StartupPlatform::create_window`].
    fn close_window(&mut self, window: u64) -> HostResult<()>;
}

/// How the event loop ended.
///
/// The distinction is not cosmetic. The pinned revision documents that `quit()` only works on the
/// thread that owns the GUI toolkit: called from anywhere else it logs that it skipped and does
/// nothing. So a host that wants a bounded session cannot ask the loop to stop — it can only end
/// its own process, which is [`LoopEnd::HostEnded`], and saying so is more honest than reporting a
/// clean shutdown the toolkit never performed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LoopEnd {
    /// The loop returned because the window was closed: a clean end.
    WindowClosed,
    /// The host ended the session itself at its deadline, because it cannot ask the loop to stop.
    HostEnded,
}

/// How a startup attempt ended. There is no "started" without a window.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum StartupOutcome {
    /// A window was created and the loop ran. `end` says how the session finished.
    Started { window: u64, end: LoopEnd },
    /// Nothing was started, and this says why. Not a crash and not a success.
    Refused { reason: String, exit_code: ExitCode },
}

/// What happened, with the facts a reviewer needs to check it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StartupReport {
    pub outcome: StartupOutcome,
    /// The backend that answered, as reported by the platform.
    pub backend: String,
    /// The GUI mode the backend reported.
    pub gui_mode: RuntimeGuiMode,
    /// How long the whole attempt took.
    pub elapsed: Duration,
}

impl StartupReport {
    /// Whether a window was created and the loop ran.
    pub fn is_started(&self) -> bool {
        matches!(self.outcome, StartupOutcome::Started { .. })
    }

    /// The exit code the process should use.
    pub fn exit_code(&self) -> ExitCode {
        match &self.outcome {
            StartupOutcome::Started { .. } => ExitCode::Ok,
            StartupOutcome::Refused { exit_code, .. } => *exit_code,
        }
    }

    /// One line for the terminal, naming the mode and the backend either way.
    pub fn summary(&self) -> String {
        match &self.outcome {
            StartupOutcome::Started { window, end } => format!(
                "desktop session started: backend {backend}, gui mode {mode:?}, window {window}, \
                 ended {end:?}, {elapsed} ms",
                backend = self.backend,
                mode = self.gui_mode,
                elapsed = self.elapsed.as_millis(),
            ),
            StartupOutcome::Refused { reason, .. } => format!(
                "desktop session refused: backend {backend}, gui mode {mode:?}: {reason}",
                backend = self.backend,
                mode = self.gui_mode,
            ),
        }
    }
}

/// Starts the baseline desktop session.
#[must_use = "a refused startup is a result the caller has to report"]
pub fn launch(request: &StartupRequest) -> StartupReport {
    launch_with(request, &mut NativePlatform::new())
}

/// Starts a session against any platform, which is how the tests drive the ordered steps.
pub fn launch_with(request: &StartupRequest, platform: &mut dyn StartupPlatform) -> StartupReport {
    let started = Instant::now();
    let backend = platform.backend_name();
    let gui_mode = platform.gui_mode();

    let refuse = |reason: String, exit_code: ExitCode, elapsed: Duration| StartupReport {
        outcome: StartupOutcome::Refused { reason, exit_code },
        backend: backend.clone(),
        gui_mode,
        elapsed,
    };

    if let Err(error) = request.validate() {
        emit(Level::Error, "host::startup", &error.message());
        return refuse(error.message(), error.exit_code(), started.elapsed());
    }

    if gui_mode != RuntimeGuiMode::NativeInteractive {
        let error = HostError::NoGuiSession {
            backend: backend.clone(),
            mode: format!("{gui_mode:?}"),
        };
        emit(Level::Warn, "host::startup", &error.message());
        return refuse(error.message(), error.exit_code(), started.elapsed());
    }

    if let Err(error) = platform.init() {
        emit(Level::Error, "host::startup", &error.message());
        return refuse(error.message(), error.exit_code(), started.elapsed());
    }
    emit(
        Level::Info,
        "host::startup",
        &format!("backend {backend} initialised, gui mode {gui_mode:?}"),
    );

    let window = match platform.create_window(request) {
        Ok(window) => window,
        Err(error) => {
            emit(Level::Error, "host::startup", &error.message());
            return refuse(error.message(), error.exit_code(), started.elapsed());
        }
    };
    emit(
        Level::Info,
        "host::startup",
        &format!(
            "window {window} created {}x{}",
            request.width, request.height
        ),
    );

    let loop_result = platform.run_loop(request);
    let close_result = platform.close_window(window);

    match (loop_result, close_result) {
        (Ok(end), Ok(())) => {
            emit(
                Level::Info,
                "host::startup",
                &format!("window {window} closed, event loop returned ({end:?})"),
            );
            StartupReport {
                outcome: StartupOutcome::Started { window, end },
                backend,
                gui_mode,
                elapsed: started.elapsed(),
            }
        }
        (Err(error), _) | (_, Err(error)) => {
            emit(Level::Error, "host::startup", &error.message());
            refuse(error.message(), error.exit_code(), started.elapsed())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LoopEnd, StartupOutcome, StartupPlatform, StartupRequest, launch_with};
    use crate::host::error::{ExitCode, HostError, HostResult};
    use crate::host::log::{FixedClock, Level, Logger};
    use rust_widgets::platform::RuntimeGuiMode;
    use std::sync::Mutex;
    use std::time::Duration;

    /// A platform that records what was asked of it, so the order is asserted, not assumed.
    struct FakePlatform {
        mode: RuntimeGuiMode,
        init_fails: bool,
        loop_fails: bool,
        loop_end: LoopEnd,
        calls: Vec<String>,
    }

    impl FakePlatform {
        fn interactive() -> Self {
            Self {
                mode: RuntimeGuiMode::NativeInteractive,
                init_fails: false,
                loop_fails: false,
                loop_end: LoopEnd::WindowClosed,
                calls: Vec::new(),
            }
        }

        fn stub() -> Self {
            Self {
                mode: RuntimeGuiMode::PreviewOrStub,
                ..Self::interactive()
            }
        }
    }

    impl StartupPlatform for FakePlatform {
        fn backend_name(&self) -> String {
            "fake".to_owned()
        }

        fn gui_mode(&self) -> RuntimeGuiMode {
            self.mode
        }

        fn init(&mut self) -> HostResult<()> {
            self.calls.push("init".to_owned());
            if self.init_fails {
                return Err(HostError::PlatformInit {
                    reason: "no display session".into(),
                });
            }
            Ok(())
        }

        fn create_window(&mut self, request: &StartupRequest) -> HostResult<u64> {
            self.calls.push(format!(
                "create_window {}x{}",
                request.width, request.height
            ));
            Ok(7)
        }

        fn run_loop(&mut self, request: &StartupRequest) -> HostResult<LoopEnd> {
            self.calls
                .push(format!("run_loop {} ms", request.exit_after.as_millis()));
            if self.loop_fails {
                return Err(HostError::EventLoop {
                    reason: "loop returned 1".into(),
                });
            }
            Ok(self.loop_end)
        }

        fn close_window(&mut self, window: u64) -> HostResult<()> {
            self.calls.push(format!("close_window {window}"));
            Ok(())
        }
    }

    fn capture_log() -> &'static Mutex<Vec<u8>> {
        Box::leak(Box::new(Mutex::new(Vec::new())))
    }

    #[test]
    fn a_native_backend_initialises_creates_the_window_then_runs_the_loop() {
        let buffer = capture_log();
        let _ = Logger::to_buffer(Level::Trace, Box::new(FixedClock::new()), buffer);
        let mut platform = FakePlatform::interactive();
        let request = StartupRequest::default();

        let report = launch_with(&request, &mut platform);

        assert!(report.is_started());
        assert_eq!(report.exit_code(), ExitCode::Ok);
        assert_eq!(
            platform.calls,
            [
                "init".to_owned(),
                "create_window 1280x800".to_owned(),
                "run_loop 750 ms".to_owned(),
                "close_window 7".to_owned(),
            ],
        );
        assert_eq!(
            report.outcome,
            StartupOutcome::Started {
                window: 7,
                end: LoopEnd::WindowClosed
            }
        );
        assert!(report.summary().contains("gui mode NativeInteractive"));
    }

    #[test]
    fn a_stub_backend_is_refused_before_anything_is_created() {
        let mut platform = FakePlatform::stub();

        let report = launch_with(&StartupRequest::default(), &mut platform);

        assert!(
            !report.is_started(),
            "a stub backend must never report a session"
        );
        assert_eq!(report.exit_code(), ExitCode::EnvironmentUnavailable);
        assert!(
            platform.calls.is_empty(),
            "nothing may be created: {:?}",
            platform.calls
        );
        assert!(report.summary().contains("refused"));
        assert!(report.summary().contains("PreviewOrStub"));
    }

    #[test]
    fn a_failed_init_is_reported_as_a_failure_with_its_reason() {
        let mut platform = FakePlatform {
            init_fails: true,
            ..FakePlatform::interactive()
        };

        let report = launch_with(&StartupRequest::default(), &mut platform);

        assert!(!report.is_started());
        assert_eq!(report.exit_code(), ExitCode::Failed);
        assert_eq!(platform.calls, ["init".to_owned()]);
        match report.outcome {
            StartupOutcome::Refused { reason, .. } => {
                assert!(reason.contains("no display session"))
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn a_loop_failure_closes_the_window_and_reports_the_failure() {
        let mut platform = FakePlatform {
            loop_fails: true,
            ..FakePlatform::interactive()
        };

        let report = launch_with(&StartupRequest::default(), &mut platform);

        assert!(!report.is_started());
        assert_eq!(report.exit_code(), ExitCode::Failed);
        assert_eq!(
            platform.calls.last().map(String::as_str),
            Some("close_window 7")
        );
        assert!(report.summary().contains("loop returned 1"));
    }

    #[test]
    fn an_impossible_request_is_refused_without_touching_the_platform() {
        let mut platform = FakePlatform::interactive();

        let empty_title = StartupRequest {
            title: "  ".into(),
            ..StartupRequest::default()
        };
        let report = launch_with(&empty_title, &mut platform);
        assert_eq!(report.exit_code(), ExitCode::Failed);
        assert!(platform.calls.is_empty());

        let no_area = StartupRequest {
            width: 0,
            ..StartupRequest::default()
        };
        assert!(no_area.validate().is_err());
        assert!(StartupRequest::default().validate().is_ok());
    }

    #[test]
    fn a_session_the_host_had_to_end_reports_that_instead_of_a_clean_shutdown() {
        let mut platform = FakePlatform {
            loop_end: LoopEnd::HostEnded,
            ..FakePlatform::interactive()
        };

        let report = launch_with(&StartupRequest::default(), &mut platform);

        assert!(report.is_started());
        assert_eq!(
            report.outcome,
            StartupOutcome::Started {
                window: 7,
                end: LoopEnd::HostEnded
            }
        );
        assert!(
            report.summary().contains("ended HostEnded"),
            "{}",
            report.summary()
        );
    }

    #[test]
    fn the_request_default_is_a_bounded_session() {
        let request = StartupRequest::default();

        assert!(
            request.exit_after > Duration::ZERO,
            "the smoke test must be able to end"
        );
        assert_eq!(request.width, 1280);
        assert_eq!(request.height, 800);
    }
}
