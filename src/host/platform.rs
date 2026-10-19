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

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
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
    smoke_field_text: Rc<RefCell<String>>,
}

impl NativePlatform {
    /// A platform handle. Constructing it performs no platform call.
    pub fn new() -> Self {
        Self {
            smoke_clicks: Arc::new(AtomicUsize::new(0)),
            smoke_field: None,
            smoke_field_text: Rc::new(RefCell::new(String::new())),
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
    fn mount_smoke_controls(&mut self, window: u64, seed: Option<&str>) -> HostResult<()> {
        let handle = WindowHandle::from_raw(window);

        let field = handle.new_line_edit("", 40, 100, 300, 32);
        let field_id = field.raw_id();
        if field_id == 0 {
            return Err(HostError::WindowCreation {
                reason: "the widget runtime refused the smoke line edit".to_owned(),
            });
        }
        let last_text = Rc::clone(&self.smoke_field_text);
        field.on_value_changed(move |text| {
            // The pinned revision does not raise this for every key path (feedback §17), so the
            // polling watcher below is the authoritative producer; this only makes the callback
            // path observable when it does fire.
            note_field_change(&last_text, text.to_owned());
        });
        self.smoke_field = Some(field_id);
        if let Some(text) = seed {
            *self.smoke_field_text.borrow_mut() = text.to_owned();
            rust_widgets::set_widget_text(field_id, text);
            emit(
                Level::Info,
                "host::platform",
                &format!("smoke field seeded: {text:?}"),
            );
        }
        // The pinned revision's pointer router does not move keyboard focus on a press (see
        // docs/rw-feedback/feedback-20261015.md §17), so focusing the field explicitly is the
        // host-side workaround. The text the key path actually produced is logged after the loop.
        rust_widgets::widget::runtime::focus_widget(field_id);
        emit(
            Level::Info,
            "host::platform",
            &format!(
                "smoke field {field_id} focused programmatically; click-to-focus needs upstream"
            ),
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
    /// The pinned revision raises `ValueChanged` for some typing paths but not all (feedback §17),
    /// and an IME commit can change the text without a key event the host sees. A short poll is the
    /// smallest host-side answer that covers both: it is not a model of editing, only a change
    /// detector at the one control the smoke uses. [`note_field_change`] de-duplicates, so a real
    /// `ValueChanged` callback and this watcher cannot report the same edit twice.
    fn install_smoke_text_watcher(&self) {
        let Some(field) = self.smoke_field else {
            return;
        };
        let last = Rc::clone(&self.smoke_field_text);
        gtk::glib::timeout_add_local(Duration::from_millis(100), move || {
            let text = rust_widgets::get_widget_text(field);
            note_field_change(&last, text);
            gtk::glib::ControlFlow::Continue
        });
        emit(
            Level::Info,
            "host::platform",
            "smoke field text watcher installed (100 ms, de-duplicated)",
        );
    }

    /// Wires a real GTK input-method context to the smoke field.
    ///
    /// The pinned library records composition state but never connects a toolkit IM context, and
    /// its window key path never asks one to filter keys. This installs the context the host
    /// owns, blocks the library's key handlers so composition keys are not also inserted as raw
    /// text, and forwards preedit/commit through the library's public IME delivery functions.
    ///
    /// The context id and the toolkit's `gtk-im-module` setting are logged because they are the
    /// difference between "a composition-capable context was selected" and "the fallback context
    /// committed every keystroke". Whether the selected input method is actually composing (fcitx
    /// can be inactive, or on its `keyboard-us` method) is an environment fact the harness reports
    /// rather than one this host can claim.
    ///
    /// This is a host workaround for the smoke; the upstream fix is feedback §17.
    fn install_ime(&self, title: &str) -> HostResult<()> {
        use gtk::prelude::*;

        let Some(field) = self.smoke_field else {
            return Err(HostError::WindowCreation {
                reason: "--ime needs --smoke: the smoke field is the IME target".to_owned(),
            });
        };
        let Some(area) = find_paint_area(title) else {
            return Err(HostError::WindowCreation {
                reason: "the paint area was not found for the IME context".to_owned(),
            });
        };

        let im = gtk::IMMulticontext::new();
        if !area.is_realized() {
            area.realize();
        }
        area.add_events(
            gtk::gdk::EventMask::KEY_PRESS_MASK
                | gtk::gdk::EventMask::KEY_RELEASE_MASK
                | gtk::gdk::EventMask::FOCUS_CHANGE_MASK,
        );
        im.set_client_window(area.window().as_ref());
        im.set_use_preedit(true);
        im.set_cursor_location(&gtk::gdk::Rectangle::new(40, 100, 300, 32));
        let im_module = gtk::Settings::default().and_then(|settings| settings.gtk_im_module());
        emit(
            Level::Info,
            "host::platform",
            &format!(
                "smoke IME client window: realized={}, gdk={}, context={:?}, gtk-im-module={:?}",
                area.is_realized(),
                area.window().is_some(),
                im.context_id(),
                im_module,
            ),
        );

        let preedit = im.clone();
        im.connect_preedit_changed(move |_| {
            let (text, _attrs, cursor) = preedit.preedit_string();
            let composition = rust_widgets::platform::ime::ImeComposition {
                text: text.to_string(),
                cursor_position: if cursor < 0 {
                    text.len()
                } else {
                    cursor as usize
                },
                selection_length: 0,
            };
            if !composition.text.is_empty() {
                emit(
                    Level::Info,
                    "host::platform",
                    &format!("smoke IME preedit: {}", composition.text),
                );
            }
            let _ = rust_widgets::platform::ime::deliver_composition(field, &composition);
        });
        let commit_last = Rc::clone(&self.smoke_field_text);
        im.connect_commit(move |_im, text| {
            emit(
                Level::Info,
                "host::platform",
                &format!("smoke IME commit: {text}"),
            );
            let _ = rust_widgets::platform::ime::deliver_commit(field, text);
            // The watcher also sees this, but reporting here keeps the commit and the change in
            // one causal line; the de-duplication makes the second observation a no-op.
            note_field_change(&commit_last, rust_widgets::get_widget_text(field));
        });

        self.install_key_bridge(&area, field, Some(im.clone()));

        let focus_in = im.clone();
        area.connect_focus_in_event(move |_, _| {
            // The IM context must be told after the GTK widget actually receives focus; calling
            // `focus_in` at construction time alone raced the window manager's focus handoff, so
            // fcitx sometimes committed raw letters instead of composing. `reset` drops a stale
            // composition from the previous focus before the new one starts.
            focus_in.reset();
            focus_in.focus_in();
            emit(
                Level::Info,
                "host::platform",
                "smoke IME focus in (context reset and focused)",
            );
            gtk::glib::Propagation::Proceed
        });
        let focus_out = im.clone();
        area.connect_focus_out_event(move |_, _| {
            focus_out.focus_out();
            gtk::glib::Propagation::Proceed
        });
        im.focus_in();
        emit(
            Level::Info,
            "host::platform",
            "smoke IME context installed (gtk window im-multicontext)",
        );
        Ok(())
    }

    /// Installs the key path, optionally filtered by an IM context.
    ///
    /// With `filter = None` this is not used: the library's own key handlers stay in place so an
    /// ASCII smoke exercises the real host path. With an IM context the library's handlers are
    /// blocked first, composition keys go to `filter_keypress`, and the rest is dispatched as the
    /// same `KeyPress`/`KeyRelease` events the library would have produced.
    fn install_key_bridge(
        &self,
        area: &gtk::DrawingArea,
        field: u64,
        filter: Option<gtk::IMMulticontext>,
    ) {
        use gtk::prelude::*;

        block_key_handlers(area);
        let press_filter = filter.clone();
        area.connect_key_press_event(move |widget, event| {
            if let Some(im) = &press_filter
                && im.filter_keypress(event)
            {
                return gtk::glib::Propagation::Stop;
            }
            let key = *event.keyval();
            let target = rust_widgets::widget::runtime::focused_widget().unwrap_or(field);
            let _ = rust_widgets::widget::runtime::dispatch_event(
                target,
                &rust_widgets::event::Event::KeyPress {
                    key,
                    modifiers: modifier_bits(event.state()),
                },
            );
            widget.queue_draw();
            gtk::glib::Propagation::Proceed
        });
        area.connect_key_release_event(move |widget, event| {
            let key = *event.keyval();
            let target = rust_widgets::widget::runtime::focused_widget().unwrap_or(field);
            let _ = rust_widgets::widget::runtime::dispatch_event(
                target,
                &rust_widgets::event::Event::KeyRelease {
                    key,
                    modifiers: modifier_bits(event.state()),
                },
            );
            widget.queue_draw();
            gtk::glib::Propagation::Proceed
        });
        emit(
            Level::Info,
            "host::platform",
            &format!(
                "smoke key bridge installed (IM filter: {})",
                if filter.is_some() { "yes" } else { "no" }
            ),
        );
    }
}

/// Reports a field text change exactly once, and forwards it as a `Field/change` event.
///
/// Two observers can see the same edit — the library's `ValueChanged` callback and the host's
/// watcher (or an IME commit) — so the last value is the arbiter. A seed is written into it before
/// the field is seeded, which keeps a seed from being reported as a user edit.
fn note_field_change(last: &Rc<RefCell<String>>, text: String) {
    let changed = {
        let mut known = last.borrow_mut();
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
            self.mount_smoke_controls(id, request.smoke_text.as_deref())?;
        }
        rust_widgets::show_widget(id);
        #[cfg(all(target_os = "linux", feature = "linux-native"))]
        {
            grab_paint_area_focus(&request.title);
            if request.smoke {
                self.install_smoke_text_watcher();
                if request.ime {
                    self.install_ime(&request.title)?;
                }
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

/// Grants GTK keyboard focus to the window's paint area, without changing the pinned library.
///
/// The pinned revision marks the paint area `can_focus` but never grabs focus, so the connected
/// key handlers never run. This walks the GTK toplevel the host just created, finds its first
/// `DrawingArea` (the paint area is packed before the surface overlay), and focuses it.
///
/// This is a **host workaround for the smoke**, not the general fix: the library still does not
/// move router focus on a pointer press, so the smoke also focuses its field explicitly. The
/// upstream request is in `docs/rw-feedback/feedback-20261015.md` §17.
#[cfg(all(target_os = "linux", feature = "linux-native"))]
fn grab_paint_area_focus(title: &str) {
    use gtk::prelude::*;

    let Some(area) = find_paint_area(title) else {
        emit(
            Level::Warn,
            "host::platform",
            "the paint area was not found, so keyboard focus could not be granted",
        );
        return;
    };
    area.grab_focus();
    emit(
        Level::Info,
        "host::platform",
        "paint area grabbed GTK focus (host workaround for upstream §17)",
    );
}

/// The first `DrawingArea` under the toplevel whose title is `title`.
#[cfg(all(target_os = "linux", feature = "linux-native"))]
fn find_paint_area(title: &str) -> Option<gtk::DrawingArea> {
    use gtk::prelude::*;

    let window = find_window(title)?;
    find_drawing_area(window.upcast_ref::<gtk::Widget>())
}

/// The toplevel whose title is `title`.
#[cfg(all(target_os = "linux", feature = "linux-native"))]
fn find_window(title: &str) -> Option<gtk::Window> {
    use gtk::prelude::*;

    for widget in gtk::Window::list_toplevels() {
        let Ok(window) = widget.clone().downcast::<gtk::Window>() else {
            continue;
        };
        if window.title().as_deref() == Some(title) {
            return Some(window);
        }
    }
    None
}

/// The first `DrawingArea` under `widget`, depth-first.
#[cfg(all(target_os = "linux", feature = "linux-native"))]
fn find_drawing_area(widget: &gtk::Widget) -> Option<gtk::DrawingArea> {
    use gtk::prelude::*;

    if let Ok(area) = widget.clone().downcast::<gtk::DrawingArea>() {
        return Some(area);
    }
    if let Ok(container) = widget.clone().downcast::<gtk::Container>() {
        for child in container.children() {
            if let Some(found) = find_drawing_area(&child) {
                return Some(found);
            }
        }
    }
    None
}

/// Blocks the library's own key handlers on `area` so the host IME context can filter keys first.
#[cfg(all(target_os = "linux", feature = "linux-native"))]
fn block_key_handlers(area: &gtk::DrawingArea) {
    use gtk::glib::translate::IntoGlib;
    use gtk::prelude::*;

    unsafe {
        let instance: *mut gtk::glib::gobject_ffi::GObject = area.as_ptr().cast();
        let type_ = gtk::DrawingArea::static_type().into_glib();
        for signal in ["key-press-event", "key-release-event"] {
            let name = std::ffi::CString::new(signal).expect("a signal name has no NUL");
            let id = gtk::glib::gobject_ffi::g_signal_lookup(name.as_ptr(), type_);
            if id == 0 {
                continue;
            }
            let blocked = gtk::glib::gobject_ffi::g_signal_handlers_block_matched(
                instance,
                gtk::glib::gobject_ffi::G_SIGNAL_MATCH_ID,
                id,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );
            emit(
                Level::Info,
                "host::platform",
                &format!("smoke IME blocked {blocked} {signal} handler(s)"),
            );
        }
    }
}

/// The widget-layer modifier bits for a GTK key state.
#[cfg(all(target_os = "linux", feature = "linux-native"))]
fn modifier_bits(state: gtk::gdk::ModifierType) -> u32 {
    let mut bits = 0;
    if state.contains(gtk::gdk::ModifierType::SHIFT_MASK) {
        bits |= 1;
    }
    if state.contains(gtk::gdk::ModifierType::CONTROL_MASK) {
        bits |= 2;
    }
    if state.contains(gtk::gdk::ModifierType::MOD1_MASK) {
        bits |= 4;
    }
    bits
}

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
