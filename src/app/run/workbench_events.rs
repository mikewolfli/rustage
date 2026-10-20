// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The workbench's Events panel (tasks T13.03/T13.04).
//!
//! The panel keeps three facts apart, because conflating them is what the acceptance line forbids:
//!
//! * **declared** — the event has a typed payload contract in [`Registry`];
//! * **wired** — this node has a handler attached in [`Wiring`];
//! * **fired** — a trigger was checked against the declaration, whether or not a handler ran.
//!
//! A name that exists therefore never reads as "connected", and a trigger with no handler is
//! reported as unwired rather than as a delivery. Binding is a real model edit: attaching twice
//! replaces the one attachment instead of adding a second, a refused bind changes nothing, and
//! unbinding removes exactly the attachment it names.

use crate::app::data::data_model::{Data, DataField, FieldKind};
use crate::app::data::events::{self, Firing, Registry, Status, Trigger, Wiring};
use crate::contracts::identity::NodeId;
use crate::contracts::model::PropertyValue;

/// The events this demo project declares. The payload is part of the declaration, so a trigger
/// cannot be "valid" by name alone.
fn demo_registry() -> Registry {
    Registry::new()
        .with(
            "pressed",
            vec![DataField {
                name: "count".to_owned(),
                kind: FieldKind::Integer,
                required: true,
            }],
        )
        .with(
            "changed",
            vec![DataField {
                name: "text".to_owned(),
                kind: FieldKind::Text,
                required: false,
            }],
        )
        .with(
            "submitted",
            vec![DataField {
                name: "form".to_owned(),
                kind: FieldKind::Text,
                required: true,
            }],
        )
}

/// The Events panel's model.
pub(super) struct EventsState {
    registry: Registry,
    wiring: Wiring,
    node: NodeId,
    /// The event whose last trigger the panel reports.
    last: Option<(String, Firing)>,
    /// One per accepted bind/unbind, so the panel can show that a model edit happened.
    revision: u64,
}

impl EventsState {
    /// The demo declarations plus two wires, one on the button and one on the label.
    ///
    /// The two wires are what makes "declared but not wired" observable: selecting the panel
    /// (node 1) and the button (node 2) gives different statuses for the same declared event.
    pub(super) fn new() -> Self {
        let registry = demo_registry();
        let mut wiring = Wiring::new();
        let _ = wiring.attach(&registry, NodeId::from_raw(2), "pressed", "count");
        let _ = wiring.attach(&registry, NodeId::from_raw(3), "changed", "edit");
        Self {
            registry,
            wiring,
            node: NodeId::from_raw(1),
            last: None,
            revision: 0,
        }
    }

    /// Points the panel at the selected node. Nothing is rewritten; the status rows follow.
    pub(super) fn retarget(&mut self, node: NodeId) {
        self.node = node;
    }

    /// The selected node.
    pub(super) fn node(&self) -> NodeId {
        self.node
    }

    /// The declared event names, in declaration order.
    pub(super) fn events(&self) -> Vec<String> {
        self.registry
            .events
            .iter()
            .map(|event| event.name.clone())
            .collect()
    }

    /// One row per declaration: the payload contract and what is true of it on the selected node.
    pub(super) fn rows(&self) -> Vec<String> {
        self.registry
            .events
            .iter()
            .map(|event| {
                let payload: Vec<String> = event
                    .payload
                    .iter()
                    .map(|field| {
                        format!(
                            "{}: {}{}",
                            field.name,
                            field.kind.render(),
                            if field.required { "!" } else { "" }
                        )
                    })
                    .collect();
                let status = events::status(&self.registry, &self.wiring, self.node, &event.name);
                format!(
                    "{} [{}] — {}",
                    event.name,
                    payload.join(", "),
                    status.render(&event.name)
                )
            })
            .collect()
    }

    /// The handler wired to the selected node and an event, if any.
    pub(super) fn handler_of(&self, event: &str) -> Option<&str> {
        self.wiring.handler_of(self.node, event)
    }

    /// The distinct handlers, in first-appearance order.
    pub(super) fn handlers(&self) -> Vec<String> {
        self.wiring
            .handlers()
            .into_iter()
            .map(str::to_owned)
            .collect()
    }

    /// The selected node's wiring, one line per attachment.
    pub(super) fn wiring_rows(&self) -> Vec<String> {
        self.wiring
            .render()
            .into_iter()
            .filter(|line| line.starts_with(&format!("node {}:", self.node.get())))
            .collect()
    }

    /// The model revision: one per accepted bind or unbind.
    pub(super) fn revision(&self) -> u64 {
        self.revision
    }

    /// Binds `event` to `handler` on the selected node.
    ///
    /// An undeclared event and a blank handler are refused, leaving both the registry and the
    /// wiring untouched. Re-binding an event replaces its handler.
    pub(super) fn bind(&mut self, event: &str, handler: &str) -> Result<u64, String> {
        let handler = handler.trim();
        if handler.is_empty() {
            return Err("a handler name is required".to_owned());
        }
        self.wiring
            .attach(&self.registry, self.node, event, handler)?;
        self.revision += 1;
        Ok(self.revision)
    }

    /// Removes the selected node's handler for `event`.
    pub(super) fn unbind(&mut self, event: &str) -> Result<u64, String> {
        if !self.wiring.detach(self.node, event) {
            return Err(format!(
                "no handler is wired to {event:?} on node {}",
                self.node.get()
            ));
        }
        self.revision += 1;
        Ok(self.revision)
    }

    /// Fires `event` on the selected node with a payload built from its own declaration.
    ///
    /// This is a model-level trigger: it validates the payload through the one data validator and
    /// reports whether a handler was wired. It never runs a business function, so the running-effect
    /// counter of an inspection session cannot move because of it.
    pub(super) fn fire(&mut self, event: &str) -> Result<Vec<String>, String> {
        let declaration = self
            .registry
            .get(event)
            .ok_or_else(|| format!("no event {event:?} is declared"))?;
        let mut payload = Data::new();
        for field in &declaration.payload {
            payload = payload.with(&field.name, sample_value(&field.kind));
        }
        let firing = events::fire(
            &self.registry,
            &self.wiring,
            self.node,
            &Trigger {
                event: event.to_owned(),
                payload,
            },
        )?;
        let lines = firing.render(self.node);
        self.last = Some((event.to_owned(), firing));
        Ok(lines)
    }

    /// The last trigger, as lines, or a line saying nothing was fired.
    pub(super) fn last_firing(&self) -> Vec<String> {
        match &self.last {
            Some((event, firing)) => {
                let mut lines = vec![format!("last trigger {event}:")];
                lines.extend(firing.render(self.node));
                lines
            }
            None => vec!["nothing has been triggered".to_owned()],
        }
    }

    /// The status of one event on the selected node.
    pub(super) fn status_of(&self, event: &str) -> Status {
        events::status(&self.registry, &self.wiring, self.node, event)
    }
}

/// A payload value that satisfies a declared field, used for the panel's own trigger.
fn sample_value(kind: &FieldKind) -> PropertyValue {
    match kind {
        FieldKind::Integer => PropertyValue::Integer(1),
        FieldKind::Number => PropertyValue::Number(1.0),
        FieldKind::Bool => PropertyValue::Bool(true),
        FieldKind::Enum(values) => PropertyValue::Variant(
            values
                .first()
                .cloned()
                .unwrap_or_else(|| "default".to_owned()),
        ),
        FieldKind::Text => PropertyValue::Text("demo".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::EventsState;
    use crate::app::data::events::Status;
    use crate::contracts::identity::NodeId;

    #[test]
    fn declared_and_wired_are_reported_separately() {
        let mut state = EventsState::new();
        // The panel node has no handler for `pressed`, so the name exists but nothing runs.
        assert_eq!(state.status_of("pressed"), Status::DeclaredOnly);
        state.retarget(NodeId::from_raw(2));
        assert_eq!(state.status_of("pressed"), Status::Wired);
        assert_eq!(state.handler_of("pressed"), Some("count"));
    }

    #[test]
    fn binding_an_undeclared_event_is_refused_and_changes_nothing() {
        let mut state = EventsState::new();
        let before = state.revision();
        let reason = state
            .bind("released", "on_release")
            .expect_err("an undeclared event");
        assert!(reason.contains("declared"), "{reason}");
        assert_eq!(state.revision(), before);
        assert_eq!(state.status_of("released"), Status::NotDeclared);
    }

    #[test]
    fn binding_twice_replaces_the_handler_without_adding_a_second() {
        let mut state = EventsState::new();
        state.bind("pressed", "first").expect("a bind");
        state.bind("pressed", "second").expect("a bind");
        assert_eq!(state.handler_of("pressed"), Some("second"));
        assert_eq!(state.wiring_rows().len(), 1);
        assert_eq!(state.revision(), 2);
    }

    #[test]
    fn unbinding_removes_the_one_attachment_and_a_second_unbind_is_refused() {
        let mut state = EventsState::new();
        state.retarget(NodeId::from_raw(2));
        assert_eq!(state.status_of("pressed"), Status::Wired);
        state.unbind("pressed").expect("an unbind");
        assert_eq!(state.status_of("pressed"), Status::DeclaredOnly);
        assert!(state.unbind("pressed").is_err());
    }

    #[test]
    fn a_valid_trigger_with_no_handler_is_unwired_not_a_delivery() {
        let mut state = EventsState::new();
        // Node 1 (the panel) has no `pressed` handler.
        let lines = state.fire("pressed").expect("a trigger");
        assert!(lines[0].contains("no handler is wired"), "{lines:?}");
        assert_eq!(state.last_firing()[1], lines[0]);
    }

    #[test]
    fn a_wired_trigger_is_reported_as_wired() {
        let mut state = EventsState::new();
        state.retarget(NodeId::from_raw(2));
        let lines = state.fire("pressed").expect("a trigger");
        assert!(lines[0].contains("a handler is wired"), "{lines:?}");
    }

    #[test]
    fn an_undeclared_trigger_is_refused() {
        let mut state = EventsState::new();
        assert!(state.fire("released").is_err());
        assert_eq!(state.last_firing(), vec!["nothing has been triggered"]);
    }
}
