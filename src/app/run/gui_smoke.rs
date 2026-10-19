// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The GUI event bridge: a real control event enters the designer's event model before it reaches
//! the business process.
//!
//! Three layers are involved, and this module keeps them from collapsing into one:
//!
//! * **Declaration** ([`Registry`], [`Wiring`]) says an event exists, what payload it carries, and
//!   which node has a handler. A name that exists is not the same fact as a handler that is wired.
//! * **Binding** ([`Dispatcher`]) says what the handler does — a command, a business call, a
//!   property write — and which permission it needs. Binding is per node and event, so two controls
//!   can answer the same event name with different actions, and re-wiring replaces rather than
//!   accumulates.
//! * **Delivery** ([`Bridge`]) forwards an accepted event to the real `--logic-worker` process. A
//!   refusal is logged as a refusal and is never sent as if it had succeeded.
//!
//! The smoke document below is a real, if small, document: `click` is declared by the primary
//! button, `change` by the field, `submit` by the second button. It exists to prove the routing is
//! the app's model rather than a direct GTK-to-channel shortcut; the MS09 process boundary itself is
//! unchanged.

use std::sync::mpsc::{Sender, channel, sync_channel};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::app::data::binding::Environment;
use crate::app::data::data_model::{Data, DataField, FieldKind, Schema};
use crate::app::data::dispatch::{Action, Dispatcher};
use crate::app::data::events::{Registry, Trigger, Wiring, fire};
use crate::app::style::property_editor::PropertyType;
use crate::contracts::identity::NodeId;
use crate::contracts::model::{PropertyName, PropertyValue};
use crate::contracts::permission::{Capability, PermissionGate, Principal};
use crate::host::log::{Level, emit};
use crate::host::smoke::{Role, SmokeEvent, SmokeValue};

/// The nodes the smoke document declares, one per mounted control.
const PRIMARY: u64 = 1;
const FIELD: u64 = 2;
const SUBMIT: u64 = 3;

/// Why an event did not reach the business process.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) enum Refusal {
    /// The document does not declare the event name.
    Undeclared(String),
    /// The payload does not satisfy the declaration.
    Payload(Vec<String>),
    /// The name is declared, but this node has no handler.
    Unwired { node: NodeId, event: String },
    /// A handler is wired, but no action is bound (the two registries disagree).
    Unbound { node: NodeId, event: String },
}

impl Refusal {
    fn render(&self) -> String {
        match self {
            Self::Undeclared(event) => format!("event {event:?} is not declared"),
            Self::Payload(problems) => format!("payload refused: {}", problems.join("; ")),
            Self::Unwired { node, event } => {
                format!("node {} declares {event} but nothing is wired", node.get())
            }
            Self::Unbound { node, event } => {
                format!("node {} wires {event} but no action is bound", node.get())
            }
        }
    }
}

/// An accepted event and the action its binding names.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct Route {
    /// The document node the control was mapped to.
    pub node: NodeId,
    /// The bound action, in one line.
    pub action: String,
    /// The action's kind (`command`, `call`, `set`).
    pub kind: &'static str,
    /// The event name the worker receives.
    pub event: String,
}

/// The smoke document: three controls, three declared events, three bindings.
#[derive(Clone, PartialEq, Debug)]
pub(super) struct Document {
    registry: Registry,
    wiring: Wiring,
    dispatcher: Dispatcher,
}

impl Document {
    /// Builds the smoke document, refusing to exist half-bound.
    pub(super) fn new() -> Result<Self, String> {
        let field = NodeId::from_raw(FIELD);
        let registry = Registry::new()
            .with("click", Vec::new())
            .with(
                "change",
                vec![DataField {
                    name: "text".to_owned(),
                    kind: FieldKind::Text,
                    required: true,
                }],
            )
            .with(
                "submit",
                vec![DataField {
                    name: "text".to_owned(),
                    kind: FieldKind::Text,
                    required: true,
                }],
            );

        let mut wiring = Wiring::new();
        wiring.attach(&registry, NodeId::from_raw(PRIMARY), "click", "count")?;
        wiring.attach(&registry, field, "change", "edit")?;
        wiring.attach(&registry, NodeId::from_raw(SUBMIT), "submit", "submit")?;

        let text = PropertyName::new("text")
            .ok_or_else(|| "the smoke document needs a text property".to_owned())?;
        let environment = Environment::new(Schema::new("smoke", 0)).with_slot(
            field,
            text,
            PropertyType::Text { max_len: 1024 },
        );
        let mut dispatcher = Dispatcher::new(environment)
            .with_command("count")
            .with_function("edit")
            .with_function("submit");
        let gate = PermissionGate::new();
        for (node, event, action) in [
            (
                NodeId::from_raw(PRIMARY),
                "click",
                Action::Command {
                    id: "count".to_owned(),
                },
            ),
            (
                field,
                "change",
                Action::Call {
                    function: "edit".to_owned(),
                    needs: Capability::EditWidgetProperty,
                },
            ),
            (
                NodeId::from_raw(SUBMIT),
                "submit",
                Action::Call {
                    function: "submit".to_owned(),
                    needs: Capability::EditWidgetProperty,
                },
            ),
        ] {
            dispatcher.install(&gate, Principal::Human, node, event, action)?;
        }

        Ok(Self {
            registry,
            wiring,
            dispatcher,
        })
    }

    /// The node a role is mapped to.
    fn node(&self, role: Role) -> NodeId {
        NodeId::from_raw(match role {
            Role::Primary => PRIMARY,
            Role::Field => FIELD,
            Role::Submit => SUBMIT,
        })
    }

    /// Checks one real control event against the declaration, the wiring and the binding.
    ///
    /// The order is the point: a missing payload is reported before "there is no handler", because
    /// the payload is part of the event's contract and a handler would not have run either way.
    pub(super) fn route(&self, event: &SmokeEvent) -> Result<Route, Refusal> {
        let node = self.node(event.role);
        let mut payload = Data::new();
        for (field, value) in &event.payload {
            match value {
                SmokeValue::Text(text) => {
                    payload
                        .values
                        .insert((*field).to_owned(), PropertyValue::Text(text.clone()));
                }
            }
        }
        let trigger = Trigger {
            event: event.event.to_owned(),
            payload,
        };
        let firing = fire(&self.registry, &self.wiring, node, &trigger)
            .map_err(|_| Refusal::Undeclared(event.event.to_owned()))?;
        if !firing.problems.is_empty() {
            return Err(Refusal::Payload(
                firing
                    .problems
                    .iter()
                    .map(|problem| problem.render())
                    .collect(),
            ));
        }
        if !firing.wired {
            return Err(Refusal::Unwired {
                node,
                event: event.event.to_owned(),
            });
        }
        let dispatch =
            self.dispatcher
                .dispatch(node, event.event)
                .map_err(|_| Refusal::Unbound {
                    node,
                    event: event.event.to_owned(),
                })?;
        Ok(Route {
            node,
            action: dispatch.action.render(),
            kind: dispatch.action.kind(),
            event: event.event.to_owned(),
        })
    }

    /// The request the worker receives for an accepted event.
    ///
    /// The event name is what drives the business transition; the text payload is forwarded with
    /// the worker's own escape rules so a space, colon or newline in the field cannot split the
    /// frame. It is evidence that the payload crossed the process boundary, not a second model.
    pub(super) fn worker_request(&self, event: &SmokeEvent) -> String {
        match event.text("text") {
            Some(text) => format!(
                "event {} text:{}",
                event.event,
                crate::app::logic::worker::escape(text)
            ),
            None => format!("event {}", event.event),
        }
    }
}

/// A running worker thread and the channel that feeds it.
pub(super) struct Bridge {
    sender: Sender<SmokeEvent>,
    handle: Option<JoinHandle<()>>,
}

impl Bridge {
    /// Starts the worker and waits until its handshake completed.
    ///
    /// A worker that cannot start is reported here rather than as a silent no-op on the first
    /// event: the whole point of the bridge is to know whether the boundary exists.
    pub(super) fn start() -> Result<Self, String> {
        let document = Document::new()?;
        let (sender, receiver) = channel::<SmokeEvent>();
        let (ready_sender, ready_receiver) = sync_channel::<Result<(), String>>(1);
        let handle = std::thread::spawn(move || {
            let mut link = match super::logic::start_smoke_link() {
                Ok(link) => {
                    let _ = ready_sender.send(Ok(()));
                    link
                }
                Err(reason) => {
                    let _ = ready_sender.send(Err(reason.clone()));
                    emit(
                        Level::Error,
                        "host::gui-smoke",
                        &format!("the worker could not start: {reason}"),
                    );
                    return;
                }
            };
            emit(
                Level::Info,
                "host::gui-smoke",
                "worker linked; declared events are routed through the designer's wiring and bindings",
            );
            for event in receiver {
                match document.route(&event) {
                    Ok(route) => {
                        emit(
                            Level::Info,
                            "host::gui-smoke",
                            &format!(
                                "event accepted: {} -> node {}: {} ({})",
                                event.render(),
                                route.node.get(),
                                route.action,
                                route.kind
                            ),
                        );
                        match link.request(&document.worker_request(&event)) {
                            Ok(answer) => {
                                let state = answer.lines().next().unwrap_or_default();
                                if state.starts_with("error=") {
                                    // The designer's model accepted the event; the business core
                                    // refused the reduction. That is a reported failure, not a
                                    // state to print as if the event had succeeded.
                                    emit(
                                        Level::Error,
                                        "host::gui-smoke",
                                        &format!("worker refused {}: {state}", event.render()),
                                    );
                                } else {
                                    emit(
                                        Level::Info,
                                        "host::gui-smoke",
                                        &format!("worker after {}: {state}", event.event),
                                    );
                                }
                            }
                            Err(reason) => {
                                emit(
                                    Level::Error,
                                    "host::gui-smoke",
                                    &format!(
                                        "worker request failed for {}: {reason}",
                                        event.render()
                                    ),
                                );
                                break;
                            }
                        }
                    }
                    Err(refusal) => {
                        // A refused event is not a crash and not a success: the GUI keeps running
                        // and the log says exactly which of declaration, payload or wiring failed.
                        emit(
                            Level::Error,
                            "host::gui-smoke",
                            &format!("event refused: {}: {}", event.render(), refusal.render()),
                        );
                    }
                }
            }
            match link.quit() {
                Ok(message) => emit(
                    Level::Info,
                    "host::gui-smoke",
                    &format!("worker stopped: {message}"),
                ),
                Err(reason) => emit(
                    Level::Error,
                    "host::gui-smoke",
                    &format!("worker did not stop cleanly: {reason}"),
                ),
            }
        });
        match ready_receiver.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => Ok(Self {
                sender,
                handle: Some(handle),
            }),
            Ok(Err(reason)) => {
                let _ = handle.join();
                Err(reason)
            }
            Err(error) => {
                let _ = handle.join();
                Err(format!("the worker did not answer its handshake: {error}"))
            }
        }
    }

    /// The sender the platform's smoke sink forwards control events to.
    pub(super) fn sender(&self) -> Sender<SmokeEvent> {
        self.sender.clone()
    }

    /// Closes the event stream, waits for the worker to quit, and joins its thread.
    pub(super) fn finish(mut self) {
        drop(self.sender);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Document, Refusal, Role};
    use crate::contracts::identity::NodeId;
    use crate::host::smoke::SmokeEvent;

    fn document() -> Document {
        Document::new().expect("the smoke document is valid")
    }

    #[test]
    fn a_real_click_on_the_primary_button_reaches_the_bound_command() {
        let route = document()
            .route(&SmokeEvent::new(Role::Primary, "click"))
            .expect("an accepted event");
        assert_eq!(route.node, NodeId::from_raw(super::PRIMARY));
        assert_eq!(route.action, "command count");
        assert_eq!(route.kind, "command");
    }

    #[test]
    fn two_controls_route_their_own_events_to_their_own_bindings() {
        let document = document();
        let submit = document
            .route(&SmokeEvent::new(Role::Submit, "submit").with_text("text", "abc"))
            .expect("an accepted submit");
        let change = document
            .route(&SmokeEvent::new(Role::Field, "change").with_text("text", "abc"))
            .expect("an accepted change");
        assert_eq!(submit.node, NodeId::from_raw(super::SUBMIT));
        assert_eq!(submit.action, "call submit");
        assert_eq!(change.node, NodeId::from_raw(super::FIELD));
        assert_eq!(change.action, "call edit");
        assert_ne!(
            submit.node, change.node,
            "routing is per control, not global"
        );
    }

    #[test]
    fn the_payload_crosses_the_channel_with_its_escape_rules() {
        let document = document();
        let event = SmokeEvent::new(Role::Field, "change").with_text("text", "a b\nc");
        assert_eq!(document.worker_request(&event), "event change text:a b\\nc");
    }

    #[test]
    fn an_undeclared_event_is_refused_rather_than_sent() {
        let event = SmokeEvent::new(Role::Primary, "unknown");
        assert_eq!(
            document().route(&event),
            Err(Refusal::Undeclared("unknown".to_owned()))
        );
    }

    #[test]
    fn a_declared_event_with_no_handler_on_that_node_is_unwired_rather_than_successful() {
        // `click` is declared and wired on the primary button; the field has no click handler.
        let event = SmokeEvent::new(Role::Field, "click");
        assert_eq!(
            document().route(&event),
            Err(Refusal::Unwired {
                node: NodeId::from_raw(super::FIELD),
                event: "click".to_owned(),
            })
        );
    }

    #[test]
    fn a_required_payload_that_is_missing_is_refused_by_the_one_validator() {
        let event = SmokeEvent::new(Role::Submit, "submit");
        match document().route(&event) {
            Err(Refusal::Payload(problems)) => {
                assert!(
                    problems.iter().any(|problem| problem.contains("text")),
                    "{problems:?}"
                );
            }
            other => panic!("expected a payload refusal, got {other:?}"),
        }
    }

    #[test]
    fn the_worker_receives_one_event_name_per_accepted_control_event() {
        let document = document();
        assert_eq!(
            document.worker_request(&SmokeEvent::new(Role::Primary, "click")),
            "event click"
        );
        assert_eq!(
            document
                .worker_request(&SmokeEvent::new(Role::Submit, "submit").with_text("text", "你好")),
            "event submit text:你好"
        );
    }
}
