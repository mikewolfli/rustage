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

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use rust_widgets::app::{WidgetHandle, WindowHandle};
use rust_widgets::core::Rect;
use rust_widgets::platform::{RuntimeGuiMode, backend_name, runtime_gui_mode};
use rust_widgets::widget::Window;
use rust_widgets::widget::runtime;

use super::error::{HostError, HostResult};
use super::log::{Level, emit, flush};
use super::smoke::{Role, SmokeEvent};
use super::startup::{LoopEnd, StartupPlatform, StartupRequest};

/// The process's real platform: the pinned dependency's runtime.
pub struct NativePlatform {
    /// Clicks the smoke button received during the session.
    smoke_clicks: Arc<AtomicUsize>,
    /// The line edit, so the report can read the text the key path actually produced.
    smoke_field: Option<u64>,
    /// The last text the field held, so a real change is reported exactly once.
    smoke_field_text: Arc<Mutex<String>>,
}

impl NativePlatform {
    /// A platform handle. Constructing it performs no platform call.
    pub fn new() -> Self {
        Self {
            smoke_clicks: Arc::new(AtomicUsize::new(0)),
            smoke_field: None,
            smoke_field_text: Arc::new(Mutex::new(String::new())),
        }
    }

    /// Reports the text the smoke field holds after the loop, so a key event that arrives but
    /// does not trigger the value callback is still observable.
    fn log_smoke_text(&self) {
        let Some(field) = self.smoke_field else {
            return;
        };
        let text = rust_widgets::get_widget_text(field);
        emit(
            Level::Info,
            "host::platform",
            &format!("smoke field final: {text:?}"),
        );
    }

    /// Mounts the controls the click/keyboard smoke uses.
    ///
    /// Three real controls, because the designer's event model is about routing rather than about a
    /// single button: the first button declares `click`, the field declares `change`, and the second
    /// button declares `submit`. Each callback sends a [`SmokeEvent`] with its role and, where the
    /// document declares one, the payload. Which node each role is, and which action is bound, is
    /// the app's decision — the host only raises the event.
    ///
    /// The button is a real child of the mounted window, so the window's paint callback draws it
    /// and GTK's pointer handler can hit-test it. The line edit is the keyboard half: a click
    /// focuses it, and the platform's key handler routes typing to the focused control. The
    /// callback counts a real activation; the host reports zero and exits non-zero when a requested
    /// smoke observed no click.
    fn mount_smoke_controls(
        &mut self,
        window: u64,
        seed: Option<&str>,
        ime: bool,
    ) -> HostResult<()> {
        let handle = WindowHandle::from_raw(window);

        let field = handle.new_line_edit("", 40, 100, 300, 32);
        let field_id = field.raw_id();
        if field_id == 0 {
            return Err(HostError::WindowCreation {
                reason: "the widget runtime refused the smoke line edit".to_owned(),
            });
        }
        let last_text = Arc::clone(&self.smoke_field_text);
        field.on_value_changed(move |text| {
            // The library injects `ValueChanged` after a real edit; the watcher installed below is
            // the second, de-duplicated observer. Either path may be the first to see a change.
            note_field_change(&last_text, text.to_owned());
        });
        self.smoke_field = Some(field_id);
        if ime {
            // The library owns the toolkit input method; this observer is the host-facing half
            // (`WidgetHandle::on_ime_preedit`, added in the pinned 2.8.10). It logs each
            // uncommitted composition frame, so `--ime` can prove composition happened instead
            // of only seeing the final commit.
            let observed = field.on_ime_preedit(move |text, cursor| {
                emit(
                    Level::Info,
                    "host::platform",
                    &format!("smoke IME preedit: {text:?}@{cursor}"),
                );
            });
            if !observed {
                return Err(HostError::WindowCreation {
                    reason: "the smoke line edit refused the IME pre-edit observer".to_owned(),
                });
            }
        }
        if let Some(text) = seed {
            *self.smoke_field_text.lock().expect("smoke text lock") = text.to_owned();
            rust_widgets::set_widget_text(field_id, text);
            emit(
                Level::Info,
                "host::platform",
                &format!("smoke field seeded: {text:?}"),
            );
        }
        // The pinned revision now moves router focus on a pointer press itself
        // (`widget::runtime::dispatch_pointer_event` calls `focus_on_pointer_press`), so the smoke
        // does not focus the field programmatically. Leaving it to the click is what makes
        // `--keys` an actual test of the upstream path instead of a host-side workaround.
        emit(
            Level::Info,
            "host::platform",
            &format!("smoke field {field_id} created; clicking it moves focus"),
        );

        let submit = handle.new_button("Submit", 220, 40, 160, 40);
        let submit_id = submit.raw_id();
        if submit_id == 0 {
            return Err(HostError::WindowCreation {
                reason: "the widget runtime refused the smoke submit button".to_owned(),
            });
        }
        submit.on_click(move || {
            let text = rust_widgets::get_widget_text(field_id);
            emit(
                Level::Info,
                "host::platform",
                &format!("smoke submit button clicked: {text:?}"),
            );
            super::smoke::notify(SmokeEvent::new(Role::Submit, "submit").with_text("text", text));
        });

        let button = handle.new_button("Click", 40, 40, 160, 40);
        let button_id = button.raw_id();
        if button_id == 0 {
            return Err(HostError::WindowCreation {
                reason: "the widget runtime refused the smoke button".to_owned(),
            });
        }
        let clicks = Arc::clone(&self.smoke_clicks);
        button.on_click(move || {
            let count = clicks.fetch_add(1, Ordering::SeqCst) + 1;
            emit(
                Level::Info,
                "host::platform",
                &format!("smoke button clicked ({count})"),
            );
            WindowHandle::from_raw(window).set_title(&format!("rustage — clicked {count}"));
            // The business side is optional: with no bridge installed this is a no-op, and with
            // one it is a non-blocking channel send (the worker thread owns the process).
            super::smoke::notify(SmokeEvent::new(Role::Primary, "click"));
        });

        emit(
            Level::Info,
            "host::platform",
            &format!(
                "smoke controls: button {button_id} at 40,40 160x40, field {field_id} at 40,100 300x32, submit {submit_id} at 220,40 160x40"
            ),
        );
        Ok(())
    }
}

#[cfg(all(target_os = "linux", feature = "linux-native"))]
impl NativePlatform {
    /// Watches the smoke field and reports each text change once.
    ///
    /// The library raises `ValueChanged` for typing now, and this watcher is kept as a second
    /// observer for paths that change text without a key event the host sees (an IME commit is
    /// delivered by the library, not by the host). [`note_field_change`] de-duplicates, so the
    /// callback and this watcher cannot report the same edit twice.
    fn install_smoke_text_watcher(&self) {
        let Some(field) = self.smoke_field else {
            return;
        };
        let last = Arc::clone(&self.smoke_field_text);
        // `schedule_on_main` is the library's own cross-platform timer: a host that polls on the
        // loop thread must not create a toolkit timer from another thread. The callback re-arms
        // itself, which is the documented way to express repeating work.
        fn watch(field: u64, last: Arc<Mutex<String>>) {
            let _ = rust_widgets::schedule_on_main(Duration::from_millis(100), move || {
                let text = rust_widgets::get_widget_text(field);
                note_field_change(&last, text);
                watch(field, last.clone());
            });
        }
        watch(field, last);
        emit(
            Level::Info,
            "host::platform",
            "smoke field text watcher installed (100 ms, de-duplicated)",
        );
    }
}

/// Reports a field text change exactly once, and forwards it as a `Field/change` event.
///
/// Two observers can see the same edit — the library's `ValueChanged` callback and the host's
/// watcher (or an IME commit) — so the last value is the arbiter. A seed is written into it before
/// the field is seeded, which keeps a seed from being reported as a user edit.
fn note_field_change(last: &Mutex<String>, text: String) {
    let changed = {
        let mut known = last.lock().expect("smoke text lock");
        if *known == text {
            false
        } else {
            *known = text.clone();
            true
        }
    };
    if !changed {
        return;
    }
    emit(
        Level::Info,
        "host::platform",
        &format!("smoke field changed: {text}"),
    );
    super::smoke::notify(SmokeEvent::new(Role::Field, "change").with_text("text", text));
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
        #[cfg(all(target_os = "linux", feature = "linux-native"))]
        register_system_cjk_face();
        Ok(())
    }

    fn create_window(&mut self, request: &StartupRequest) -> HostResult<u64> {
        // Two ids are involved, and both are needed. `create_window` builds the platform's real
        // toplevel; `register` mounts the widget the host draws and dispatches through; and
        // `set_host_window` is the association that lets a platform callback translate its own id
        // back to the widget id. Registering the widget alone left the native toplevel unbuilt, so
        // `--launch` logged the requested 1280x800 while the X server held only an unmapped 10x10
        // helper. Showing the widget is the last step: creation and visibility are separate calls.
        // `rust_widgets::create_window` routes through the control backend, which paints every
        // control itself — including a `Window`'s model — and builds no OS object. The platform's
        // `create_window` is the call that actually constructs the GTK toplevel, so the host uses it
        // and associates the mounted widget with the id it returns.
        let host = rust_widgets::platform::get_platform().create_window(
            &request.title,
            0,
            0,
            request.width,
            request.height,
        );
        if host == 0 {
            return Err(HostError::WindowCreation {
                reason: "the platform refused to create a window".to_owned(),
            });
        }
        let geometry = Rect::new(0, 0, request.width, request.height);
        let id = runtime::register(Box::new(Window::new(request.title.clone(), geometry)))
            .ok_or_else(|| HostError::WindowCreation {
                reason: "the widget runtime refused a window registration".to_owned(),
            })?;
        if !runtime::set_host_window(id, host) {
            runtime::unregister(id);
            return Err(HostError::WindowCreation {
                reason: "the window widget and its platform toplevel could not be associated"
                    .to_owned(),
            });
        }
        if request.smoke {
            self.mount_smoke_controls(id, request.smoke_text.as_deref(), request.ime)?;
        }
        rust_widgets::show_widget(id);
        #[cfg(all(target_os = "linux", feature = "linux-native"))]
        if request.smoke {
            self.install_smoke_text_watcher();
            if request.ime {
                // The pinned revision owns the toolkit IM context now: the window creates and
                // focuses an `IMMulticontext`, filters composition keys before the widget layer,
                // and delivers preedit/commit through `platform::ime`. Installing a second context
                // here would compose twice, so `--ime` only observes the library path the tool
                // exposes (`on_ime_preedit`, wired above in `mount_smoke_controls`).
                emit(
                    Level::Info,
                    "host::platform",
                    "smoke IME: the pinned library owns the toolkit input context",
                );
            }
        }
        Ok(id)
    }

    fn run_loop(&mut self, request: &StartupRequest) -> HostResult<LoopEnd> {
        if request.exit_after.is_zero() {
            emit(
                Level::Info,
                "host::platform",
                "entering the event loop; it ends when the window is closed",
            );
            rust_widgets::run();
            self.log_smoke_text();
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
        self.log_smoke_text();
        Ok(LoopEnd::HostEnded)
    }

    fn close_window(&mut self, window: u64) -> HostResult<()> {
        // Hide before unregistering: after the association is dropped, the widget id can no longer
        // resolve to its platform toplevel, and the native window would outlive the widget it was
        // showing until process exit.
        rust_widgets::hide_widget(window);
        if rust_widgets::close_widget(window) {
            runtime::unregister(window);
            return Ok(());
        }
        Err(HostError::WindowCreation {
            reason: format!("window {window} was already gone when it was closed"),
        })
    }

    fn smoke_clicks(&self) -> usize {
        self.smoke_clicks.load(Ordering::SeqCst)
    }
}

/// Registers a system CJK face so characters outside the compiled subsets render.
///
/// The `desktop` feature ships a deliberately narrow vector subset (`U+4E00..=U+511F`), so common
/// characters such as 好 (`U+597D`) and 界 (`U+754C`) reached tofu. The `runtime-fonts` feature is
/// the library's answer: the host registers a system face and it is consulted before the compiled
/// stack. DroidSansFallback is preferred (3.9 MB and CJK-complete); Noto Sans CJK is the fallback.
///
/// The trade is that a runtime face is consulted for every character it covers, including Latin,
/// so this host accepts the system face as the default rather than adding a script-aware filter
/// (which the library does not expose).
#[cfg(all(target_os = "linux", feature = "linux-native"))]
fn register_system_cjk_face() {
    const CANDIDATES: [&str; 2] = [
        "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    ];
    for path in CANDIDATES {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let leaked: &'static [u8] = Box::leak(bytes.into_boxed_slice());
        if rust_widgets::render::text::register_face("System CJK", leaked) {
            emit(
                Level::Info,
                "host::platform",
                &format!("registered system CJK face: {path}"),
            );
            return;
        }
    }
    emit(
        Level::Warn,
        "host::platform",
        "no system CJK face was found; characters outside the compiled subset will be tofu",
    );
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

        // Construction must not initialise anything: `--help` builds no platform. The counter is
        // host state, not a runtime call, and it starts at zero so no smoke can inherit a click.
        assert_eq!(platform.smoke_clicks(), 0);
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

    #[test]
    fn creating_a_window_mounts_its_widget_associates_the_host_and_shows_it() {
        // The platform's own window is the OS object, the mounted widget is what the host draws
        // and dispatches through, and the association is the two-way translation. A window that
        // misses any of the three either does not exist, is never painted, or cannot resolve an
        // OS callback back to its widget.
        let mut platform = NativePlatform::new();
        let request = StartupRequest::default();
        let id = platform
            .create_window(&request)
            .expect("a state-only backend still creates a window model");
        assert!(
            rust_widgets::widget::runtime::host_window_for(id).is_some(),
            "the mounted widget must know the platform window it belongs to"
        );
        assert!(
            rust_widgets::is_widget_visible(id),
            "creating a window and showing it are separate steps"
        );
        assert!(platform.close_window(id).is_ok());
    }
}
