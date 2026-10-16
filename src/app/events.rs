// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Event declarations, their typed payloads, the handlers wired to them, and keeping *declared*,
//! *wired* and *fired* apart (task T13.03).
//!
//! The acceptance line is *"the name existing is verified apart from the actual wiring and
//! triggering"*. Three facts that are easy to conflate are therefore three separate answers:
//!
//! * **A declaration is a typed contract, not a name.** [`EventDecl`] carries the payload fields, so
//!   a declared event also says what a trigger must carry (principle 17).
//! * **Declared is not wired.** [`Registry::has`] answers only whether the name is known;
//!   [`Wiring::is_wired`] answers whether a node actually has a handler; [`status`] answers which of
//!   the two holds, so "the event exists" never reads as "it is connected".
//! * **Wired is not fired.** [`fire`] checks the payload against the declaration and reports whether a
//!   handler was wired, so a trigger whose payload is valid but has no handler is reported as
//!   *unwired*, not as a success.
//! * **The payload is checked with the one data validator.** A trigger's payload is validated through
//!   [`data_model::validate`](crate::app::data_model::validate), so an event's contract and a
//!   dataset's contract are checked by the same rule rather than two.

#![allow(dead_code)]

use crate::app::data_model::{Data, DataField, Dataset, Diagnostic, Schema, validate};
use crate::contracts::identity::NodeId;

/// A declared event: its name and the payload a trigger must carry.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct EventDecl {
    /// The event's name.
    pub name: String,
    /// The payload fields a trigger must carry.
    pub payload: Vec<DataField>,
}

/// The events a document declares.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Registry {
    /// The declarations.
    pub events: Vec<EventDecl>,
}

impl Registry {
    /// A registry with no declarations.
    pub fn new() -> Self {
        Self::default()
    }

    /// The same registry with one more declaration.
    pub fn with(mut self, name: impl Into<String>, payload: Vec<DataField>) -> Self {
        self.events.push(EventDecl {
            name: name.into(),
            payload,
        });
        self
    }

    /// The declaration for a name, if it is declared.
    pub fn get(&self, name: &str) -> Option<&EventDecl> {
        self.events.iter().find(|event| event.name == name)
    }

    /// Whether the name is declared. This says nothing about wiring.
    pub fn has(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// The registry in one line per declaration.
    pub fn render(&self) -> Vec<String> {
        self.events
            .iter()
            .map(|event| {
                let fields: Vec<String> = event
                    .payload
                    .iter()
                    .map(|field| format!("{}: {}", field.name, field.kind.render()))
                    .collect();
                format!("event {} ({})", event.name, fields.join(", "))
            })
            .collect()
    }
}

/// One handler wired to one node's event.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Attachment {
    /// The node the event belongs to.
    pub node: NodeId,
    /// The event.
    pub event: String,
    /// The handler that runs.
    pub handler: String,
}

/// The handlers wired in a document.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Wiring {
    /// The attachments.
    pub attachments: Vec<Attachment>,
}

impl Wiring {
    /// Nothing wired.
    pub fn new() -> Self {
        Self::default()
    }

    /// Attaches a handler, refusing an event the registry does not declare.
    ///
    /// Attaching the same event on the same node again *replaces* the handler rather than adding a
    /// second one, so a re-wire cannot leave two handlers running.
    pub fn attach(
        &mut self,
        registry: &Registry,
        node: NodeId,
        event: &str,
        handler: impl Into<String>,
    ) -> Result<(), String> {
        if !registry.has(event) {
            return Err(format!("no event {event:?} is declared"));
        }
        let attachment = Attachment {
            node,
            event: event.to_owned(),
            handler: handler.into(),
        };
        match self
            .attachments
            .iter_mut()
            .find(|existing| existing.node == node && existing.event == event)
        {
            Some(existing) => *existing = attachment,
            None => self.attachments.push(attachment),
        }
        Ok(())
    }

    /// Whether a handler for this node's event is wired.
    pub fn is_wired(&self, node: NodeId, event: &str) -> bool {
        self.attachments
            .iter()
            .any(|attachment| attachment.node == node && attachment.event == event)
    }

    /// Whether any node has a handler for this event.
    pub fn any_wired(&self, event: &str) -> bool {
        self.attachments
            .iter()
            .any(|attachment| attachment.event == event)
    }

    /// The distinct handler names wired, in order of first appearance.
    pub fn handlers(&self) -> Vec<&str> {
        let mut names: Vec<&str> = Vec::new();
        for attachment in &self.attachments {
            if !names.contains(&attachment.handler.as_str()) {
                names.push(&attachment.handler);
            }
        }
        names
    }

    /// The handler wired to a node's event, if there is one.
    pub fn handler_of(&self, node: NodeId, event: &str) -> Option<&str> {
        self.attachments
            .iter()
            .find(|attachment| attachment.node == node && attachment.event == event)
            .map(|attachment| attachment.handler.as_str())
    }

    /// The wiring in one line per attachment.
    pub fn render(&self) -> Vec<String> {
        self.attachments
            .iter()
            .map(|attachment| {
                format!(
                    "node {}: {} -> {}",
                    attachment.node.get(),
                    attachment.event,
                    attachment.handler
                )
            })
            .collect()
    }
}

/// What is true of an event on one node.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    /// The registry does not declare the name.
    NotDeclared,
    /// The name is declared, but nothing is wired.
    DeclaredOnly,
    /// The name is declared and a handler is wired.
    Wired,
}

impl Status {
    /// Whether a handler is wired.
    pub fn is_wired(&self) -> bool {
        matches!(self, Self::Wired)
    }

    /// The status in one line.
    pub fn render(&self, event: &str) -> String {
        match self {
            Self::NotDeclared => format!("{event}: not declared"),
            Self::DeclaredOnly => format!("{event}: declared, not wired"),
            Self::Wired => format!("{event}: declared and wired"),
        }
    }
}

/// Which of the three facts holds for a node's event.
pub fn status(registry: &Registry, wiring: &Wiring, node: NodeId, event: &str) -> Status {
    if !registry.has(event) {
        Status::NotDeclared
    } else if wiring.is_wired(node, event) {
        Status::Wired
    } else {
        Status::DeclaredOnly
    }
}

/// A runtime occurrence of an event.
#[derive(Clone, PartialEq, Debug)]
pub struct Trigger {
    /// The event that happened.
    pub event: String,
    /// What it carried.
    pub payload: Data,
}

/// What firing an event showed.
#[derive(Clone, PartialEq, Debug)]
pub struct Firing {
    /// Whether a handler was wired for that node's event.
    pub wired: bool,
    /// What is wrong with the payload, if anything.
    pub problems: Vec<Diagnostic>,
}

impl Firing {
    /// The firing in one line per fact.
    pub fn render(&self, node: NodeId) -> Vec<String> {
        let mut lines = vec![format!(
            "node {}: {}",
            node.get(),
            if self.wired {
                "a handler is wired"
            } else {
                "no handler is wired"
            }
        )];
        for problem in &self.problems {
            lines.push(format!("  {}", problem.render()));
        }
        lines
    }
}

/// Fires an event on a node: validates the payload and reports whether a handler was wired.
///
/// An undeclared event is refused outright; a declared event answers with the payload's problems
/// (which may be none) and whether a handler was wired — so a valid payload with no handler is
/// reported as unwired and never as a success.
pub fn fire(
    registry: &Registry,
    wiring: &Wiring,
    node: NodeId,
    trigger: &Trigger,
) -> Result<Firing, String> {
    let declaration = registry
        .get(&trigger.event)
        .ok_or_else(|| format!("no event {:?} is declared", trigger.event))?;
    let schema = Schema {
        name: declaration.name.clone(),
        version: 0,
        fields: declaration.payload.clone(),
    };
    let dataset = Dataset::runtime(0, trigger.payload.clone());
    Ok(Firing {
        wired: wiring.is_wired(node, &trigger.event),
        problems: validate(&schema, &dataset),
    })
}

#[cfg(test)]
mod tests {
    use super::{Attachment, Registry, Status, Trigger, Wiring, fire, status};
    use crate::app::data_model::{Data, DataField, FieldKind};
    use crate::contracts::identity::NodeId;
    use crate::contracts::model::PropertyValue;

    fn node(value: u64) -> NodeId {
        NodeId::from_raw(value)
    }

    fn registry() -> Registry {
        Registry::new().with(
            "pressed",
            vec![
                DataField {
                    name: "count".to_owned(),
                    kind: FieldKind::Integer,
                    required: true,
                },
                DataField {
                    name: "label".to_owned(),
                    kind: FieldKind::Text,
                    required: false,
                },
            ],
        )
    }

    #[test]
    fn a_declared_event_exists_even_with_nothing_wired() {
        let registry = registry();
        assert!(registry.has("pressed"));
        assert!(!Wiring::new().is_wired(node(1), "pressed"));
        assert_eq!(
            status(&registry, &Wiring::new(), node(1), "pressed"),
            Status::DeclaredOnly
        );
        assert!(!Status::DeclaredOnly.is_wired());
    }

    #[test]
    fn an_undeclared_event_is_absent_and_cannot_be_attached() {
        let mut wiring = Wiring::new();
        assert!(!registry().has("released"));
        assert!(
            wiring
                .attach(&registry(), node(1), "released", "on_release")
                .is_err()
        );
        assert!(wiring.attachments.is_empty());
        assert_eq!(
            status(&registry(), &wiring, node(1), "released"),
            Status::NotDeclared
        );
    }

    #[test]
    fn attaching_a_declared_event_wires_it() {
        let mut wiring = Wiring::new();
        wiring
            .attach(&registry(), node(1), "pressed", "on_press")
            .expect("an attach");
        assert!(wiring.is_wired(node(1), "pressed"));
        assert_eq!(
            status(&registry(), &wiring, node(1), "pressed"),
            Status::Wired
        );
        assert_eq!(wiring.handler_of(node(1), "pressed"), Some("on_press"));
    }

    #[test]
    fn attaching_twice_replaces_the_handler_rather_than_duplicating_it() {
        let mut wiring = Wiring::new();
        wiring
            .attach(&registry(), node(1), "pressed", "first")
            .expect("an attach");
        wiring
            .attach(&registry(), node(1), "pressed", "second")
            .expect("an attach");
        assert_eq!(wiring.attachments.len(), 1);
        assert_eq!(wiring.handler_of(node(1), "pressed"), Some("second"));
    }

    #[test]
    fn two_nodes_can_each_wire_the_same_event() {
        let mut wiring = Wiring::new();
        wiring
            .attach(&registry(), node(1), "pressed", "a")
            .expect("an attach");
        wiring
            .attach(&registry(), node(2), "pressed", "b")
            .expect("an attach");
        assert_eq!(wiring.attachments.len(), 2);
        assert!(wiring.is_wired(node(1), "pressed"));
        assert!(wiring.is_wired(node(2), "pressed"));
    }

    #[test]
    fn the_handler_list_names_the_distinct_handlers() {
        let mut wiring = Wiring::new();
        wiring
            .attach(&registry(), node(1), "pressed", "shared")
            .expect("an attach");
        wiring
            .attach(&registry(), node(2), "pressed", "shared")
            .expect("an attach");
        wiring
            .attach(&registry(), node(3), "pressed", "other")
            .expect("an attach");
        assert_eq!(wiring.handlers(), vec!["shared", "other"]);
    }

    #[test]
    fn any_wired_is_true_when_some_node_is_wired() {
        let mut wiring = Wiring::new();
        assert!(!wiring.any_wired("pressed"));
        wiring
            .attach(&registry(), node(7), "pressed", "h")
            .expect("an attach");
        assert!(wiring.any_wired("pressed"));
        assert!(!Wiring::new().any_wired("pressed"));
    }

    #[test]
    fn firing_an_undeclared_event_is_refused() {
        let trigger = Trigger {
            event: "released".to_owned(),
            payload: Data::new(),
        };
        assert!(fire(&registry(), &Wiring::new(), node(1), &trigger).is_err());
    }

    #[test]
    fn a_valid_payload_with_no_handler_is_reported_as_unwired_not_as_success() {
        let trigger = Trigger {
            event: "pressed".to_owned(),
            payload: Data::new().with("count", PropertyValue::Integer(3)),
        };
        let firing = fire(&registry(), &Wiring::new(), node(1), &trigger).expect("a firing");
        assert!(!firing.wired);
        assert!(firing.problems.is_empty());
        assert!(firing.render(node(1))[0].contains("no handler is wired"));
    }

    #[test]
    fn an_invalid_payload_is_reported_by_the_one_validator() {
        let trigger = Trigger {
            event: "pressed".to_owned(),
            payload: Data::new().with("count", PropertyValue::Text("three".to_owned())),
        };
        let firing = fire(&registry(), &Wiring::new(), node(1), &trigger).expect("a firing");
        assert_eq!(firing.problems.len(), 1);
        assert_eq!(firing.problems[0].field, "count");
        assert!(firing.render(node(1))[1].contains("count"));
    }

    #[test]
    fn a_missing_required_payload_field_is_reported() {
        let trigger = Trigger {
            event: "pressed".to_owned(),
            payload: Data::new(),
        };
        let firing = fire(&registry(), &Wiring::new(), node(1), &trigger).expect("a firing");
        assert!(
            firing
                .problems
                .iter()
                .any(|problem| problem.field == "count" && problem.reason.contains("required"))
        );
    }

    #[test]
    fn firing_a_wired_event_reports_it_wired() {
        let mut wiring = Wiring::new();
        wiring
            .attach(&registry(), node(1), "pressed", "on_press")
            .expect("an attach");
        let trigger = Trigger {
            event: "pressed".to_owned(),
            payload: Data::new().with("count", PropertyValue::Integer(1)),
        };
        let firing = fire(&registry(), &wiring, node(1), &trigger).expect("a firing");
        assert!(firing.wired);
        assert!(firing.problems.is_empty());
        // Wired on node 1 only, so node 2's event is still unwired even though the name is declared.
        assert!(
            !fire(&registry(), &wiring, node(2), &trigger)
                .expect("a firing")
                .wired
        );
    }

    #[test]
    fn the_status_renders_the_three_facts_differently() {
        assert_eq!(Status::NotDeclared.render("x"), "x: not declared");
        assert_eq!(Status::DeclaredOnly.render("x"), "x: declared, not wired");
        assert_eq!(Status::Wired.render("x"), "x: declared and wired");
    }

    #[test]
    fn the_registry_renders_the_payload_contract() {
        let lines = registry().render();
        assert_eq!(lines, vec!["event pressed (count: integer, label: text)"]);
    }

    #[test]
    fn the_wiring_renders_its_attachments() {
        let wiring = Wiring {
            attachments: vec![Attachment {
                node: node(1),
                event: "pressed".to_owned(),
                handler: "on_press".to_owned(),
            }],
        };
        assert_eq!(wiring.render(), vec!["node 1: pressed -> on_press"]);
    }
}
