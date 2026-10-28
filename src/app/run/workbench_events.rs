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

use crate::app::data::data_model::{Data, FieldKind};
use crate::app::data::events::{self, Firing, Registry, Status, Trigger, Wiring};
use crate::contracts::identity::NodeId;
use crate::contracts::model::PropertyValue;
use rust_widgets::signal::ConnectionHandle;

/// The control whose published events this demo panel declares.
///
/// The panel shows the button the scene mounts at node 2, so its event list is the button's own
/// `EventSchema` rows — read from the library rather than typed in here. A name the button does not
/// publish is not offered, and its payload types are the library's, not the panel's.
const DEMO_CONTROL: &str = "button";

/// The events this demo project declares.
///
/// Derived from [`DEMO_CONTROL`]'s published `EventSchema` rows. When the library cannot resolve
/// the control, the panel says so in the host log and shows no declarations: inventing a plausible
/// event list would make "declared" mean "the panel guessed" and defeat the point of the three
/// separate facts.
fn demo_registry() -> Registry {
    match Registry::from_control(DEMO_CONTROL) {
        Ok(registry) => registry,
        Err(reason) => {
            crate::host::log::emit(
                crate::host::log::Level::Warn,
                "app::run::workbench-events",
                &reason,
            );
            Registry::new()
        }
    }
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
    /// The button's declarations plus two wires, one on each of the two demo nodes.
    ///
    /// The two wires are what makes "declared but not wired" observable: selecting the panel
    /// (node 1) and the button (node 2) gives different statuses for the same declared event.
    pub(super) fn new() -> Self {
        let registry = demo_registry();
        let mut wiring = Wiring::new();
        let _ = wiring.attach(&registry, NodeId::from_raw(2), "clicked", "count");
        let _ = wiring.attach(&registry, NodeId::from_raw(3), "state_changed", "edit");
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

    /// The declared event names, in the library's published order.
    pub(super) fn events(&self) -> Vec<String> {
        self.registry
            .events
            .iter()
            .map(|event| event.name.clone())
            .collect()
    }

    /// One row per declaration: the library's payload contract and what is true of it on the
    /// selected node.
    pub(super) fn rows(&self) -> Vec<String> {
        self.registry
            .events
            .iter()
            .map(|event| {
                let status = events::status(&self.registry, &self.wiring, self.node, &event.name);
                format!(
                    "{} [{}] — {}",
                    event.name,
                    event.payload_summary(),
                    status.render(&event.name)
                )
            })
            .collect()
    }

    /// The handler wired to the selected node and an event, if any.
    pub(super) fn handler_of(&self, event: &str) -> Option<&str> {
        self.wiring.handler_of(self.node, event)
    }

    /// The handle of the selected node's wire for an event, when one is live.
    pub(super) fn handle_of(&self, event: &str) -> Option<ConnectionHandle> {
        self.wiring.handle_of(self.node, event)
    }

    /// Every handle retired by a replacement or an unbind, in order.
    pub(super) fn released(&self) -> &[ConnectionHandle] {
        self.wiring.released()
    }

    /// The distinct handlers, in first-appearance order.
    pub(super) fn handlers(&self) -> Vec<String> {
        self.wiring
            .handlers()
            .into_iter()
            .map(str::to_owned)
            .collect()
    }

    /// The selected node's wiring, one line per attachment, with its live connection handle.
    pub(super) fn wiring_rows(&self) -> Vec<String> {
        let mut rows: Vec<String> = self
            .wiring
            .attachments
            .iter()
            .filter(|attachment| attachment.node == self.node)
            .map(|attachment| {
                let handle = self
                    .handle_of(&attachment.event)
                    .map_or(0, |handle| handle.0);
                format!("{} [connection {handle}]", attachment.render())
            })
            .collect();
        let released = self.released();
        if !released.is_empty() {
            rows.push(format!("released connections: {}", released.len()));
        }
        rows
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
        if self.wiring.detach(self.node, event).is_none() {
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
    use rust_widgets::widget::WidgetFactory;

    #[test]
    fn the_panel_declares_the_controls_published_events_with_their_payloads() {
        let state = EventsState::new();
        let capability = WidgetFactory::shared()
            .capability("button")
            .expect("the button capability");
        let published: Vec<String> = capability
            .events
            .iter()
            .map(|event| event.name.to_owned())
            .collect();
        assert_eq!(state.events(), published);
        // The row's payload text is the library's declaration, not a panel-supplied one.
        let rows = state.rows();
        let changed = rows
            .iter()
            .find(|row| row.starts_with("state_changed"))
            .expect("state_changed is published");
        assert!(changed.contains("string scalar"), "{changed}");
        assert!(changed.contains("value: text"), "{changed}");
        // A payload-free event stays payload-free rather than gaining a placeholder field.
        let clicked = rows
            .iter()
            .find(|row| row.starts_with("clicked"))
            .expect("clicked is published");
        assert!(clicked.contains("no payload"), "{clicked}");
    }

    #[test]
    fn an_event_the_control_does_not_publish_is_not_offered() {
        let state = EventsState::new();
        assert!(!state.events().iter().any(|event| event == "submitted"));
        assert_eq!(state.status_of("submitted"), Status::NotDeclared);
    }

    #[test]
    fn declared_and_wired_are_reported_separately() {
        let mut state = EventsState::new();
        // The panel node has no handler for `clicked`, so the name exists but nothing runs.
        assert_eq!(state.status_of("clicked"), Status::DeclaredOnly);
        state.retarget(NodeId::from_raw(2));
        assert_eq!(state.status_of("clicked"), Status::Wired);
        assert_eq!(state.handler_of("clicked"), Some("count"));
    }

    #[test]
    fn binding_an_undeclared_event_is_refused_and_changes_nothing() {
        let mut state = EventsState::new();
        let before = state.revision();
        let reason = state
            .bind("submitted", "on_submit")
            .expect_err("an undeclared event");
        assert!(reason.contains("declared"), "{reason}");
        assert_eq!(state.revision(), before);
        assert_eq!(state.status_of("submitted"), Status::NotDeclared);
    }

    #[test]
    fn binding_twice_replaces_the_handler_without_adding_a_second() {
        let mut state = EventsState::new();
        state.bind("clicked", "first").expect("a bind");
        state.bind("clicked", "second").expect("a bind");
        assert_eq!(state.handler_of("clicked"), Some("second"));
        let rows = state.wiring_rows();
        assert_eq!(
            rows.iter()
                .filter(|row| row.contains("clicked -> second"))
                .count(),
            1,
            "{rows:?}"
        );
        assert!(!rows.iter().any(|row| row.contains("clicked -> first")));
        assert_eq!(state.revision(), 2);
    }

    #[test]
    fn unbinding_removes_the_one_attachment_and_a_second_unbind_is_refused() {
        let mut state = EventsState::new();
        state.retarget(NodeId::from_raw(2));
        assert_eq!(state.status_of("clicked"), Status::Wired);
        state.unbind("clicked").expect("an unbind");
        assert_eq!(state.status_of("clicked"), Status::DeclaredOnly);
        assert!(state.unbind("clicked").is_err());
        // Node 3's wire for a different event is untouched by node 2's unbind.
        state.retarget(NodeId::from_raw(3));
        assert_eq!(state.handler_of("state_changed"), Some("edit"));
    }

    #[test]
    fn bind_and_unbind_hold_and_drop_exactly_one_handle() {
        let mut state = EventsState::new();
        state.retarget(NodeId::from_raw(1));
        assert_eq!(state.handle_of("clicked"), None);
        state.bind("clicked", "count").expect("a bind");
        let handle = state.handle_of("clicked").expect("the live handle");

        state.unbind("clicked").expect("an unbind");
        assert_eq!(state.handle_of("clicked"), None);
        assert_eq!(state.released(), &[handle]);
        // The record is gone, so a valid trigger is unwired rather than delivered.
        let lines = state.fire("clicked").expect("a trigger");
        assert!(lines[0].contains("no handler is wired"), "{lines:?}");
    }

    #[test]
    fn a_repeated_unbind_does_not_touch_another_nodes_handle() {
        let mut state = EventsState::new();
        state.retarget(NodeId::from_raw(2));
        let node_two = state.handle_of("clicked").expect("node 2's handle");
        state.retarget(NodeId::from_raw(3));
        let node_three = state.handle_of("state_changed").expect("node 3's handle");
        assert_ne!(node_two, node_three);

        state.retarget(NodeId::from_raw(2));
        state.unbind("clicked").expect("an unbind");
        assert!(state.unbind("clicked").is_err());

        state.retarget(NodeId::from_raw(3));
        assert_eq!(state.handle_of("state_changed"), Some(node_three));
        assert_eq!(state.handler_of("state_changed"), Some("edit"));
    }

    #[test]
    fn re_binding_retires_the_previous_handle() {
        let mut state = EventsState::new();
        state.retarget(NodeId::from_raw(2));
        let first = state.handle_of("clicked").expect("the first handle");
        state.bind("clicked", "other").expect("a bind");
        let second = state.handle_of("clicked").expect("the second handle");
        assert_ne!(first, second);
        assert_eq!(state.released(), &[first]);
        assert_eq!(state.handler_of("clicked"), Some("other"));
    }

    #[test]
    fn a_valid_trigger_with_no_handler_is_unwired_not_a_delivery() {
        let mut state = EventsState::new();
        // Node 1 (the panel) has no `clicked` handler.
        let lines = state.fire("clicked").expect("a trigger");
        assert!(lines[0].contains("no handler is wired"), "{lines:?}");
        assert_eq!(state.last_firing()[1], lines[0]);
    }

    #[test]
    fn a_wired_trigger_is_reported_as_wired() {
        let mut state = EventsState::new();
        state.retarget(NodeId::from_raw(2));
        let lines = state.fire("clicked").expect("a trigger");
        assert!(lines[0].contains("a handler is wired"), "{lines:?}");
    }

    #[test]
    fn a_trigger_payload_is_built_from_the_declared_shape() {
        let mut state = EventsState::new();
        state.retarget(NodeId::from_raw(3));
        let lines = state.fire("state_changed").expect("a trigger");
        // The library declares one text value, so the panel's trigger carries exactly that.
        assert!(lines[0].contains("a handler is wired"), "{lines:?}");
        assert_eq!(
            lines.len(),
            1,
            "a well-shaped payload has no problems: {lines:?}"
        );
    }

    #[test]
    fn an_undeclared_trigger_is_refused() {
        let mut state = EventsState::new();
        assert!(state.fire("submitted").is_err());
        assert_eq!(state.last_firing(), vec!["nothing has been triggered"]);
    }
}
