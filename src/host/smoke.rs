// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The one sink a GUI smoke control can notify, and the reason it is a host module.
//!
//! The platform adapter owns the real GTK controls, but it must not depend on `app::data` or
//! `app::logic` just to say that a control raised an event. This module is the boundary: the app
//! installs a sender before the GUI session starts, the platform's control callbacks send a
//! [`SmokeEvent`], and the app's bridge — not the platform — turns that into the designer's declared
//! event, its wiring, its bound action and a request to the business process.
//!
//! **Roles, not widget ids.** A widget id belongs to the widget runtime and changes between builds;
//! the app's event model is keyed by document node ids. The smoke therefore names the three controls
//! it mounts ([`Role`]) and the app maps each role to the node id it declared. That keeps the host
//! from inventing document identity and keeps the app from reaching into GTK.
//!
//! **Payloads are small and typed enough.** Text is the only payload the smoke controls produce, so
//! [`SmokeValue`] has one variant rather than a copy of the designer's property types. A control
//! that later needs a number can add a variant here without teaching the platform about
//! `PropertyValue`.
//!
//! `None` is the normal state. A smoke event with no bridge installed is still a real control event:
//! the platform counts and logs it, and this sink silently drops the notification. A sender that has
//! been dropped by its receiver is treated the same way; the event already happened and the GUI must
//! not block or panic because the business side went away.

use std::sync::mpsc::Sender;
use std::sync::{Mutex, OnceLock};

/// Which smoke control raised an event.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    /// The primary button.
    Primary,
    /// The text field.
    Field,
    /// The second button, which submits the field's text.
    Submit,
}

impl Role {
    /// The role's name, for a log line and the routing table.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Primary => "Primary",
            Self::Field => "Field",
            Self::Submit => "Submit",
        }
    }
}

/// A payload value a smoke control carries.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SmokeValue {
    /// Free text, as typed into or read from the field.
    Text(String),
}

/// One real control event on its way to the designer's event model.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SmokeEvent {
    /// Which control raised it.
    pub role: Role,
    /// The event name the document declares (`click`, `change`, `submit`).
    pub event: &'static str,
    /// The payload fields, in declaration order.
    pub payload: Vec<(&'static str, SmokeValue)>,
}

impl SmokeEvent {
    /// An event with no payload.
    pub fn new(role: Role, event: &'static str) -> Self {
        Self {
            role,
            event,
            payload: Vec::new(),
        }
    }

    /// The same event with one text field.
    pub fn with_text(mut self, field: &'static str, text: impl Into<String>) -> Self {
        self.payload.push((field, SmokeValue::Text(text.into())));
        self
    }

    /// The text of a payload field, if it is present and textual.
    pub fn text(&self, field: &str) -> Option<&str> {
        self.payload.iter().find_map(|(name, value)| {
            (*name == field).then_some(match value {
                SmokeValue::Text(text) => text.as_str(),
            })
        })
    }

    /// The event in one line, for a log.
    pub fn render(&self) -> String {
        let fields: Vec<String> = self
            .payload
            .iter()
            .map(|(name, value)| match value {
                SmokeValue::Text(text) => format!("{name}={text:?}"),
            })
            .collect();
        if fields.is_empty() {
            format!("{}/{}", self.role.name(), self.event)
        } else {
            format!(
                "{}/{} ({})",
                self.role.name(),
                self.event,
                fields.join(", ")
            )
        }
    }
}

/// The process-wide sink. One smoke session exists per process, so one slot is the whole model.
static SINK: OnceLock<Mutex<Option<Sender<SmokeEvent>>>> = OnceLock::new();

fn cell() -> &'static Mutex<Option<Sender<SmokeEvent>>> {
    SINK.get_or_init(|| Mutex::new(None))
}

/// Installs the sender a smoke event notifies.
pub fn install(sender: Sender<SmokeEvent>) {
    *cell()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(sender);
}

/// Removes the sender, if any. An event after this is counted and logged, but not forwarded.
pub fn clear() {
    *cell()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
}

/// Notifies the installed bridge, if one is present.
///
/// A closed receiver is not an error here: the event is a fact of the GUI, and the business side
/// going away must not turn that fact into a panic or a blocked loop.
pub fn notify(event: SmokeEvent) {
    let guard = cell()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(sender) = guard.as_ref() {
        let _ = sender.send(event);
    }
}

#[cfg(test)]
mod tests {
    use super::{Role, SmokeEvent, SmokeValue, clear, install, notify};
    use std::sync::mpsc::channel;
    use std::sync::{Mutex, MutexGuard};
    use std::time::Duration;

    /// The sink is process-wide, so the tests that install and clear it must not overlap.
    static SINK_LOCK: Mutex<()> = Mutex::new(());

    fn exclusive() -> MutexGuard<'static, ()> {
        SINK_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[test]
    fn an_installed_sink_receives_the_event_and_clearing_stops_forwarding() {
        let _guard = exclusive();
        let (sender, receiver) = channel();
        install(sender);
        notify(SmokeEvent::new(Role::Primary, "click"));
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(1)),
            Ok(SmokeEvent::new(Role::Primary, "click"))
        );

        clear();
        notify(SmokeEvent::new(Role::Primary, "click"));
        assert!(receiver.recv_timeout(Duration::from_millis(50)).is_err());
    }

    #[test]
    fn an_event_with_no_bridge_is_dropped_without_panicking() {
        let _guard = exclusive();
        clear();
        notify(SmokeEvent::new(Role::Submit, "submit"));
        clear();
        notify(SmokeEvent::new(Role::Field, "change").with_text("text", "abc"));
    }

    #[test]
    fn a_text_payload_is_read_back_and_rendered() {
        let event = SmokeEvent::new(Role::Field, "change").with_text("text", "abc");
        assert_eq!(event.text("text"), Some("abc"));
        assert_eq!(event.text("missing"), None);
        assert_eq!(
            event.payload,
            vec![("text", SmokeValue::Text("abc".into()))]
        );
        assert_eq!(event.render(), "Field/change (text=\"abc\")");
    }
}
