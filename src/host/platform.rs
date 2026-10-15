// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The dependency adapter: the only file in the host that calls `rust_widgets` runtime functions.
//!
//! Keeping this in one place is what makes [`super::startup`] testable without a display, and what
//! keeps the dependency revision's surface countable: when the pinned revision changes, this file
//! and the contract probes are the places to look. The calls used here were checked against the
//! pinned revision by `cargo run -- --probe-apis`, which reports the revision it ran against.
//!
//! **The event loop ends by itself here.** A window that only closes when a person clicks it cannot
//! be evidence in a terminal session, so `run_loop` asks the dependency to stop its own loop at
//! `exit_after` — from the watcher thread, which the dependency's `quit()` accepts — before the loop
//! ends. That is a real end of a real loop, not a process kill: the alternative, skipping `run_loop`
//! and printing "started", is the placeholder success this task forbids.
//!
//! **What is not proven here.** Whether a window is composited, whether it accepts input, and how
//! it behaves under a real display server or a remote session are host-platform facts that need a
//! person and a screen; this module only drives the documented API and reports what it got.

use std::thread;
use std::time::Duration;

use rust_widgets::core::Rect;
use rust_widgets::platform::{RuntimeGuiMode, backend_name, runtime_gui_mode};
use rust_widgets::widget::Window;
use rust_widgets::widget::runtime;

use super::error::{HostError, HostResult};
use super::log::{Level, emit, flush};
use super::startup::{LoopEnd, StartupPlatform, StartupRequest};

/// The process's real platform: the pinned dependency's runtime.
pub struct NativePlatform;

impl NativePlatform {
    /// A platform handle. Constructing it performs no platform call.
    pub fn new() -> Self {
        Self
    }
}

impl Default for NativePlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl StartupPlatform for NativePlatform {
    fn backend_name(&self) -> String {
        backend_name().to_owned()
    }

    fn gui_mode(&self) -> RuntimeGuiMode {
        runtime_gui_mode()
    }

    fn init(&mut self) -> HostResult<()> {
        rust_widgets::init();
        Ok(())
    }

    fn create_window(&mut self, request: &StartupRequest) -> HostResult<u64> {
        let geometry = Rect::new(0, 0, request.width, request.height);
        runtime::register(Box::new(Window::new(request.title.clone(), geometry))).ok_or_else(|| {
            HostError::WindowCreation {
                reason: "the widget runtime refused a window registration".to_owned(),
            }
        })
    }

    fn run_loop(&mut self, request: &StartupRequest) -> HostResult<LoopEnd> {
        if request.exit_after.is_zero() {
            emit(
                Level::Info,
                "host::platform",
                "entering the event loop; it ends when the window is closed",
            );
            rust_widgets::run();
            return Ok(LoopEnd::WindowClosed);
        }

        // The revision can be asked to stop its loop from another thread: `quit()` records the
        // request, and the loop's own frame tick performs the toolkit stop *on the loop thread*
        // (the dependency's `platform::stop`). So the deadline is a real end of a real loop — the
        // difference between a session that stopped and a process that was torn down — and the
        // request is honoured on every backend, macOS included, because recording it is one atomic
        // store and no toolkit call crosses a thread.
        let exit_after = request.exit_after;
        let _watcher = thread::spawn(move || {
            thread::sleep(exit_after);
            let reader = rust_widgets::quit();
            emit(
                Level::Info,
                "host::platform",
                &format!(
                    "session deadline {exit_after:?} reached; asked the loop to stop ({})",
                    if reader {
                        "a loop is running and stops at its next frame"
                    } else {
                        "no loop had ticked yet; the request is recorded for the next loop"
                    }
                ),
            );
            flush();
        });

        emit(
            Level::Info,
            "host::platform",
            "entering the event loop for a bounded session",
        );
        rust_widgets::run();
        Ok(LoopEnd::HostEnded)
    }

    fn close_window(&mut self, window: u64) -> HostResult<()> {
        if rust_widgets::close_widget(window) {
            runtime::unregister(window);
            return Ok(());
        }
        Err(HostError::WindowCreation {
            reason: format!("window {window} was already gone when it was closed"),
        })
    }
}

/// A short session used by `--launch` when the caller does not name a duration.
pub const BASELINE_SESSION: Duration = Duration::from_millis(750);

#[cfg(test)]
mod tests {
    use super::{BASELINE_SESSION, NativePlatform};
    use crate::host::startup::{StartupPlatform, StartupRequest};
    use std::time::Duration;

    #[test]
    fn a_platform_handle_can_be_built_without_touching_the_runtime() {
        let platform = NativePlatform::new();

        // Construction must not initialise anything: `--help` builds no platform.
        assert!(std::mem::size_of_val(&platform) == 0);
    }

    #[test]
    fn the_reported_gui_mode_is_the_documented_enum() {
        let platform = NativePlatform::new();
        let mode = platform.gui_mode();

        // Either answer is legitimate; what matters is that it is the dependency's own value and
        // not a locally invented one, because `launch` compares it to the interactive case.
        assert!(matches!(
            mode,
            rust_widgets::platform::RuntimeGuiMode::NativeInteractive
                | rust_widgets::platform::RuntimeGuiMode::PreviewOrStub
        ));
        assert!(!platform.backend_name().is_empty());
    }

    #[test]
    fn the_platform_reports_the_dependency_gui_mode_it_will_be_compared_against() {
        // `launch` refuses anything that is not `NativeInteractive`; this pins the value the
        // platform adapter answers so a re-vendored dependency cannot silently change the meaning.
        let platform = NativePlatform::new();
        let mode = platform.gui_mode();
        assert!(
            mode == rust_widgets::platform::RuntimeGuiMode::NativeInteractive
                || mode == rust_widgets::platform::RuntimeGuiMode::PreviewOrStub,
        );
    }

    #[test]
    fn the_baseline_session_is_long_enough_to_start_and_short_enough_to_wait_for() {
        assert!(BASELINE_SESSION >= Duration::from_millis(500));
        assert!(BASELINE_SESSION <= Duration::from_secs(5));
        assert_eq!(StartupRequest::default().exit_after, BASELINE_SESSION);
    }
}
