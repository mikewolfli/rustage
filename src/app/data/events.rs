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
//!   [`data_model::validate`](crate::app::data::data_model::validate), so an event's contract and a
//!   dataset's contract are checked by the same rule rather than two.
//! * **The declaration comes from the library, not from the caller.** [`Registry::from_control`]
//!   reads a control's events from the capability the library registers, whose payload types are
//!   [`EventSchema`]'s; [`Registry::from_manifest`] reads the same rows back from a designer
//!   manifest. A name the control does not publish is absent, and a payload field the library does
//!   not declare is never created here; [`Registry::with`] remains for a caller that already holds
//!   a typed field list (the smoke document), not as a way to declare events.
//!
//! # Runtime subscription: the library gap this module does not paper over
//!
//! The library's name-addressed subscription path is
//! [`EventSignalRef`](rust_widgets::signal::EventSignalRef) (`subscribe`/`disconnect`, with
//! `slot_count`/`is_connected` answering whether a wire is live), reached through
//! `Widget::event_signal_dyn` on a mounted control.
//! [`EventSignalBinder`](rust_widgets::signal::EventSignalBinder) is what makes those signals reach a
//! [`CustomSignalHub`](rust_widgets::signal::CustomSignalHub), so
//! `WidgetFactory::connect_event` can bind a name. Both need a live control instance, and the binder
//! also needs the hub the host wired it into.
//!
//! This module has neither: it edits the *design-time* record of which node is wired to which
//! event. [`Wiring`] is therefore a wiring record, **not** a subscription — no `EventSignalRef` is
//! constructed and no slot is connected, so the panel never reports a live subscription it did not
//! make. Turning Bind into a real subscribe belongs to the runtime mount path (T14.02), where
//! `event_signal_dyn` exists and a contract to a real signal can be resolved.
//!
//! # What the record does own: a handle, so an unbind really drops it
//!
//! [`Attachment`] carries a [`ConnectionHandle`], the library's opaque subscription identity. It is
//! allocated by this module rather than by a `Signal` — the one honest part of the gap above is that
//! no signal is connected. Everything downstream treats it the same way: [`Wiring::detach`] retires
//! exactly that handle, [`Wiring::is_live`] answers whether the record still exists, and a handler
//! looked up by node and event can never find a retired record. A re-bind allocates a new handle and
//! retires the old one, so there is never a stale record that still answers.
//!
//! The library's [`ConnectionScope`](rust_widgets::signal::ConnectionScope) is the same idea one
//! level up (drop the owner, release what it tracks), but it can only track disconnectors registered
//! through `Signal::connect_scoped`. With no mounted control there is no `Signal` to register one on;
//! the host-side record lifecycle here is the closest honest equivalent, and the dispatcher's
//! `BindingScope` mirrors the drop semantics where actions are installed (T13.04).

#![allow(dead_code)]

use rust_widgets::signal::ConnectionHandle;
use rust_widgets::widget::capability::{
    DesignerManifest, EventPayloadShape, EventSchema, PropertyValueKind, shape_token,
    value_kind_token,
};

use crate::app::data::data_model::{
    Data, DataField, Dataset, Diagnostic, FieldKind, Schema, validate,
};
use crate::contracts::identity::NodeId;

/// The library's declaration of a payload: its carrier kind and its arrangement.
///
/// Kept beside [`EventDecl::payload`] rather than folded into it, because the two answer different
/// questions. `payload` is what the one data validator can check; this is what the library actually
/// declared. When the shape is one the data vocabulary cannot express, the declaration is still
/// shown exactly as the library states it, and firing is refused instead of fabricating fields.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DeclaredPayload {
    /// The declared carrier type, or `None` for an event that carries no value.
    pub kind: Option<PropertyValueKind>,
    /// How the payload is arranged, or `None` when there is no payload.
    pub shape: Option<EventPayloadShape>,
}

/// A declared event: its name and the payload a trigger must carry.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct EventDecl {
    /// The event's name.
    pub name: String,
    /// The payload fields a trigger must carry.
    pub payload: Vec<DataField>,
    /// The library's own declaration, when the name came from an `EventSchema`.
    pub declared: Option<DeclaredPayload>,
}

/// What the library's wire table sees on the source side of an event.
///
/// [`WIRE_RULES`](rust_widgets::widget::capability::WIRE_RULES) distinguishes only two sources:
/// no value at all, and one value of a stated kind. A payload this module cannot reduce to that
/// shape (a pair, a list, mixed kinds) is [`Payload`](Self::Payload) rather than a guessed kind, so
/// a wire into a single property or a payload-free command is refused instead of being accepted on
/// the strength of the first field.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WireSource {
    /// The event carries nothing (`clicked`).
    PayloadFree,
    /// The event carries exactly one value of this kind.
    Value(PropertyValueKind),
    /// The event carries a payload with no single-kind shape `WIRE_RULES` can address.
    Payload,
    /// No declaration was supplied, so the wire table cannot be consulted at all.
    Undeclared,
}

impl WireSource {
    /// Whether the source carries no value.
    pub fn is_payload_free(&self) -> bool {
        matches!(self, Self::PayloadFree)
    }

    /// Reads the spelling a caller can declare: `none`, `payload`, or a library kind name.
    ///
    /// This is deliberately a host-side spelling of the library's kinds rather than a second type
    /// vocabulary: the value stored is still [`PropertyValueKind`].
    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "none" => Self::PayloadFree,
            "payload" => Self::Payload,
            "bool" => Self::Value(PropertyValueKind::Bool),
            "int" => Self::Value(PropertyValueKind::Int),
            "uint" => Self::Value(PropertyValueKind::UInt),
            "float" => Self::Value(PropertyValueKind::Float),
            "number" => Self::Value(PropertyValueKind::Number),
            "string" | "text" => Self::Value(PropertyValueKind::String),
            "enum" => Self::Value(PropertyValueKind::Enum),
            "color" | "colour" => Self::Value(PropertyValueKind::Color),
            "rect" => Self::Value(PropertyValueKind::Rect),
            "icon" => Self::Value(PropertyValueKind::IconSpec),
            "records" => Self::Value(PropertyValueKind::Records),
            "string-list" => Self::Value(PropertyValueKind::StringList),
            "number-list" => Self::Value(PropertyValueKind::NumberList),
            _ => return None,
        })
    }

    /// The source in one line.
    pub fn render(&self) -> String {
        match self {
            Self::PayloadFree => "no payload".to_owned(),
            Self::Value(kind) => format!("payload: {}", value_kind_token(*kind)),
            Self::Payload => "payload (no single-kind shape)".to_owned(),
            Self::Undeclared => "undeclared".to_owned(),
        }
    }
}

impl EventDecl {
    /// What the library's wire table sees when this event is the source of a wire.
    ///
    /// A library declaration is read exactly as stated: no kind and no shape is payload-free, a
    /// scalar (or optional scalar) is one value, and every other arrangement is a payload the table
    /// cannot express. A declaration built from an explicit field list (the smoke document) is
    /// payload-free when it has no fields, a value when it has one, and a payload otherwise.
    pub fn wire_source(&self) -> WireSource {
        if let Some(declared) = &self.declared {
            return match (declared.kind, declared.shape) {
                (None, None) => WireSource::PayloadFree,
                (
                    Some(kind),
                    Some(EventPayloadShape::Scalar | EventPayloadShape::OptionalScalar),
                ) => WireSource::Value(kind),
                _ => WireSource::Payload,
            };
        }
        match self.payload.as_slice() {
            [] => WireSource::PayloadFree,
            [field] => WireSource::Value(wire_kind(&field.kind)),
            _ => WireSource::Payload,
        }
    }

    /// Whether [`Self::payload`] expresses the whole declared payload.
    ///
    /// `false` means the library declared a shape this module's field vocabulary cannot describe
    /// (a list, or a tuple of mixed kinds). Firing such an event is refused rather than validated
    /// against fewer fields than it really carries; a declaration built from an explicit field list
    /// is always whole.
    pub fn payload_is_whole(&self) -> bool {
        match &self.declared {
            None => true,
            Some(declared) => fields_for(declared).is_some(),
        }
    }

    /// The payload contract in one parenthesised fragment.
    pub fn payload_summary(&self) -> String {
        let Some(declared) = &self.declared else {
            return render_fields(&self.payload);
        };
        if declared.kind.is_none() && declared.shape.is_none() {
            return "no payload".to_owned();
        }
        let kind = declared.kind.map(value_kind_token).unwrap_or("no kind");
        let shape = declared.shape.map(shape_token).unwrap_or("no shape");
        match fields_for(declared) {
            None => format!("{kind} {shape}; not expressible as one data field"),
            Some(fields) if fields.is_empty() => "no payload".to_owned(),
            Some(_) => format!("{kind} {shape}: {}", render_fields(&self.payload)),
        }
    }
}

/// The events a document declares.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Registry {
    /// The control these declarations came from, when they were read from the library.
    pub control: Option<String>,
    /// The declarations.
    pub events: Vec<EventDecl>,
}

impl Registry {
    /// A registry with no declarations.
    pub fn new() -> Self {
        Self::default()
    }

    /// The same registry with one more declaration from an explicit field list.
    ///
    /// For a caller that already holds typed fields. A designer's declarations come from
    /// [`Self::from_control`], so a payload the library does not publish cannot be introduced by
    /// this route in the panel or the CLI.
    pub fn with(mut self, name: impl Into<String>, payload: Vec<DataField>) -> Self {
        self.events.push(EventDecl {
            name: name.into(),
            payload,
            declared: None,
        });
        self
    }

    /// Declarations derived from a control's own [`EventSchema`] list.
    pub fn from_event_schemas(schemas: &[EventSchema]) -> Self {
        let mut registry = Self::new();
        for schema in schemas {
            registry.declare(schema.name, schema.payload, schema.shape);
        }
        registry
    }

    /// Declarations derived from a control's designer manifest.
    ///
    /// The manifest's events are the same `EventSchema` rows, copied out; reading them rather than
    /// walking the capability table is what lets a saved document be loaded without a live control.
    pub fn from_manifest(manifest: &DesignerManifest) -> Self {
        let mut registry = Self::new();
        registry.control = Some(manifest.control.clone());
        for event in &manifest.events {
            registry.declare(&event.name, event.payload, event.shape);
        }
        registry
    }

    /// Declarations read from the control the library registers under `control`.
    ///
    /// The source is the capability's own [`EventSchema`] rows, so the names and payload types are
    /// the library's and nothing else is offered. An unregistered control is an error naming the
    /// library's answer, not an empty registry: an empty registry would read as "this control
    /// publishes no events".
    pub fn from_control(control: &str) -> Result<Self, String> {
        // The host factory: suite controls are installed into it, and `shared()` cannot own them.
        let factory = crate::app::contract::factory();
        let capability = factory
            .capability(control)
            .ok_or_else(|| format!("the library registers no control {control:?}"))?;
        let mut registry = Self::from_event_schemas(capability.events);
        registry.control = Some(capability.canonical_name.to_owned());
        Ok(registry)
    }

    /// Adds one declaration, deriving its fields from the library's kind and shape.
    fn declare(
        &mut self,
        name: &str,
        kind: Option<PropertyValueKind>,
        shape: Option<EventPayloadShape>,
    ) {
        let declared = DeclaredPayload { kind, shape };
        let payload = fields_for(&declared).unwrap_or_default();
        self.events.push(EventDecl {
            name: name.to_owned(),
            payload,
            declared: Some(declared),
        });
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
            .map(|event| format!("event {} ({})", event.name, event.payload_summary()))
            .collect()
    }
}

/// The library kind a single declared data field carries.
///
/// This is the inverse of [`field_kind`] for the subset both vocabularies share. The data
/// vocabulary's single `Integer` becomes `Int` (the library's signed carrier); an enumeration is
/// `Enum`, not `String`, because the declaration said so.
fn wire_kind(kind: &FieldKind) -> PropertyValueKind {
    match kind {
        FieldKind::Bool => PropertyValueKind::Bool,
        FieldKind::Integer => PropertyValueKind::Int,
        FieldKind::Number => PropertyValueKind::Float,
        FieldKind::Text => PropertyValueKind::String,
        FieldKind::Enum(_) => PropertyValueKind::Enum,
    }
}

/// The payload fields a declared shape needs, or `None` when one data field cannot express it.
fn fields_for(declared: &DeclaredPayload) -> Option<Vec<DataField>> {
    // No kind and no shape is the honest unit declaration: an event that happened carries nothing.
    if declared.kind.is_none() && declared.shape.is_none() {
        return Some(Vec::new());
    }
    let kind = field_kind(declared.kind?)?;
    // A payload with no stated arrangement is half a declaration; guessing "scalar" here would
    // turn the library's silence into a claim this module made up.
    let shape = declared.shape?;
    let (names, required): (&[&str], bool) = match shape {
        EventPayloadShape::Scalar => (&["value"], true),
        EventPayloadShape::OptionalScalar => (&["value"], false),
        EventPayloadShape::Tuple2 => (&["value.0", "value.1"], true),
        EventPayloadShape::Tuple3 => (&["value.0", "value.1", "value.2"], true),
        EventPayloadShape::Tuple4 => (&["value.0", "value.1", "value.2", "value.3"], true),
        EventPayloadShape::OptionalTuple2 => (&["value.0", "value.1"], false),
        // A variable-length list has no arity to declare, and `Mixed` is explicitly "the parts are
        // not of one type". Both are real library declarations this module cannot reduce to data
        // fields without lying about the payload.
        EventPayloadShape::ListScalar | EventPayloadShape::Mixed => return None,
    };
    Some(
        names
            .iter()
            .map(|name| DataField {
                name: (*name).to_owned(),
                kind: kind.clone(),
                required,
            })
            .collect(),
    )
}

/// The data-field kind that carries a declared payload kind, or `None` when this module has none.
fn field_kind(kind: PropertyValueKind) -> Option<FieldKind> {
    Some(match kind {
        PropertyValueKind::Bool => FieldKind::Bool,
        // The data vocabulary has one integer kind, so the signedness the library distinguishes is
        // not re-checked here; the declaration still records which one the library stated.
        PropertyValueKind::Int | PropertyValueKind::UInt => FieldKind::Integer,
        PropertyValueKind::Float | PropertyValueKind::Number => FieldKind::Number,
        // `EventSchema` carries no accepted-token list (unlike `PropertySchema`), so an enum's
        // choices are unknown at the event side. Its carrier is a string, and free text is the
        // honest field kind rather than an empty variant set that would reject every value.
        PropertyValueKind::String | PropertyValueKind::Enum => FieldKind::Text,
        PropertyValueKind::Color
        | PropertyValueKind::Rect
        | PropertyValueKind::IconSpec
        | PropertyValueKind::Records
        | PropertyValueKind::StringList
        | PropertyValueKind::NumberList => return None,
    })
}

/// A field list in one comma-separated fragment.
fn render_fields(fields: &[DataField]) -> String {
    fields
        .iter()
        .map(|field| format!("{}: {}", field.name, field.kind.render()))
        .collect::<Vec<String>>()
        .join(", ")
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
    /// The record's subscription identity.
    ///
    /// Allocated by [`Wiring`], not by a `Signal`: the library's `ConnectionHandle` is used as the
    /// opaque identity of the record, and [`Wiring::is_live`] answers whether it is still held.
    pub handle: ConnectionHandle,
}

impl Attachment {
    /// The attachment in one line.
    pub fn render(&self) -> String {
        format!(
            "node {}: {} -> {}",
            self.node.get(),
            self.event,
            self.handler
        )
    }
}

/// The handlers wired in a document.
///
/// A wiring **record**, not a runtime subscription: nothing here calls
/// [`EventSignalRef`](rust_widgets::signal::EventSignalRef) or connects a slot, because the panel
/// holds no mounted control to resolve a signal against (see the module docs). `attach` is still
/// the one place a name is refused, so the record can never claim an event the library does not
/// declare.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Wiring {
    /// The attachments.
    pub attachments: Vec<Attachment>,
    /// Handles retired by a replacement or an unbind, in order.
    released: Vec<ConnectionHandle>,
    /// The next handle to allocate. The first attachment holds handle 1.
    next_handle: u64,
}

impl Wiring {
    /// Nothing wired.
    pub fn new() -> Self {
        Self::default()
    }

    /// The next opaque handle this wiring will issue.
    fn issue_handle(&mut self) -> ConnectionHandle {
        self.next_handle += 1;
        ConnectionHandle(self.next_handle)
    }

    /// Attaches a handler, refusing an event the registry does not declare.
    ///
    /// Attaching the same event on the same node again *replaces* the handler rather than adding a
    /// second one, so a re-wire cannot leave two handlers running. The replaced record's handle is
    /// retired and the attachment gets a new one: a caller holding the old handle cannot use it to
    /// reach the replacement.
    pub fn attach(
        &mut self,
        registry: &Registry,
        node: NodeId,
        event: &str,
        handler: impl Into<String>,
    ) -> Result<ConnectionHandle, String> {
        if !registry.has(event) {
            return Err(format!("no event {event:?} is declared"));
        }
        let handle = self.issue_handle();
        let attachment = Attachment {
            node,
            event: event.to_owned(),
            handler: handler.into(),
            handle,
        };
        match self
            .attachments
            .iter_mut()
            .find(|existing| existing.node == node && existing.event == event)
        {
            Some(existing) => {
                self.released.push(existing.handle);
                *existing = attachment;
            }
            None => self.attachments.push(attachment),
        }
        Ok(handle)
    }

    /// Takes a handler away from a node's event, answering with the record that was removed.
    ///
    /// Removing the one attachment is what keeps a re-bind from leaving a second subscription
    /// behind; an event that was not wired is reported (`None`) rather than treated as success. The
    /// removed record's handle is retired, so [`Self::is_live`] answers `false` for it afterwards.
    pub fn detach(&mut self, node: NodeId, event: &str) -> Option<Attachment> {
        let index = self
            .attachments
            .iter()
            .position(|attachment| attachment.node == node && attachment.event == event)?;
        let removed = self.attachments.remove(index);
        self.released.push(removed.handle);
        Some(removed)
    }

    /// The handle of the record for a node's event, if one is wired.
    pub fn handle_of(&self, node: NodeId, event: &str) -> Option<ConnectionHandle> {
        self.attachments
            .iter()
            .find(|attachment| attachment.node == node && attachment.event == event)
            .map(|attachment| attachment.handle)
    }

    /// The record a handle names, if it is still held.
    pub fn attachment_of_handle(&self, handle: ConnectionHandle) -> Option<&Attachment> {
        self.attachments
            .iter()
            .find(|attachment| attachment.handle == handle)
    }

    /// Whether a handle still names a live record.
    pub fn is_live(&self, handle: ConnectionHandle) -> bool {
        self.attachment_of_handle(handle).is_some()
    }

    /// Every handle retired so far, in order.
    pub fn released(&self) -> &[ConnectionHandle] {
        &self.released
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
        self.attachments.iter().map(Attachment::render).collect()
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
/// reported as unwired and never as a success. An event whose declared shape this module cannot
/// express as data fields is refused before validation, because checking an empty field list would
/// claim a payload matched a declaration that was never checked.
pub fn fire(
    registry: &Registry,
    wiring: &Wiring,
    node: NodeId,
    trigger: &Trigger,
) -> Result<Firing, String> {
    let declaration = registry
        .get(&trigger.event)
        .ok_or_else(|| format!("no event {:?} is declared", trigger.event))?;
    if !declaration.payload_is_whole() {
        return Err(format!(
            "event {:?} declares a payload this module cannot express as data fields; no trigger was constructed",
            trigger.event
        ));
    }
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
    use super::{
        DeclaredPayload, EventDecl, Registry, Status, Trigger, WireSource, Wiring, fire, status,
    };
    use crate::app::data::data_model::{Data, DataField, FieldKind};
    use crate::contracts::identity::NodeId;
    use crate::contracts::model::PropertyValue;
    use rust_widgets::widget::WidgetFactory;
    use rust_widgets::widget::capability::{EventPayloadShape, EventSchema, PropertyValueKind};

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
    fn payload_types_are_derived_from_the_event_schema() {
        let schemas = [
            EventSchema::unit("clicked"),
            EventSchema::scalar("state_changed", PropertyValueKind::String),
            EventSchema::pair("range_changed", PropertyValueKind::UInt),
            EventSchema::list("samples_changed", PropertyValueKind::Float),
        ];
        let registry = Registry::from_event_schemas(&schemas);

        // A payload-free declaration stays payload-free: no field is invented for it.
        let clicked = registry.get("clicked").expect("a declaration");
        assert!(clicked.payload.is_empty());
        assert_eq!(
            clicked.declared,
            Some(DeclaredPayload {
                kind: None,
                shape: None,
            })
        );
        assert!(clicked.payload_is_whole());
        assert_eq!(clicked.payload_summary(), "no payload");

        // A scalar payload is exactly the one field the library's kind implies.
        let changed = registry.get("state_changed").expect("a declaration");
        assert_eq!(changed.payload, vec![field("value", FieldKind::Text, true)]);
        assert_eq!(
            changed.declared,
            Some(DeclaredPayload {
                kind: Some(PropertyValueKind::String),
                shape: Some(EventPayloadShape::Scalar),
            })
        );

        // The arrangement decides the arity and the names, not a caller's opinion.
        let range = registry.get("range_changed").expect("a declaration");
        assert_eq!(
            range.payload,
            vec![
                field("value.0", FieldKind::Integer, true),
                field("value.1", FieldKind::Integer, true),
            ]
        );

        // A list has no arity, so the module keeps the library's declaration and refuses to model it.
        let samples = registry.get("samples_changed").expect("a declaration");
        assert!(samples.payload.is_empty());
        assert!(!samples.payload_is_whole());
        assert!(samples.payload_summary().contains("not expressible"));

        // No name outside the schema exists.
        assert!(!registry.has("pressed"));
        assert_eq!(registry.events.len(), schemas.len());
    }

    #[test]
    fn a_controls_declarations_are_the_librarys_own() {
        let factory = WidgetFactory::shared();
        let capability = factory.capability("button").expect("the button capability");
        let registry = Registry::from_control("button").expect("a registry");

        assert_eq!(registry.control.as_deref(), Some("button"));
        let derived: Vec<&str> = registry
            .events
            .iter()
            .map(|event| event.name.as_str())
            .collect();
        let published: Vec<&str> = capability.events.iter().map(|event| event.name).collect();
        assert_eq!(
            derived, published,
            "one declaration per published event, no more"
        );

        for schema in capability.events {
            let declaration = registry.get(schema.name).expect("a declaration");
            let declared = declaration.declared;
            assert_eq!(
                declared.and_then(|declared| declared.kind),
                schema.payload,
                "{}'s payload kind",
                schema.name
            );
            assert_eq!(
                declared.and_then(|declared| declared.shape),
                schema.shape,
                "{}'s payload shape",
                schema.name
            );
        }

        // `submitted` is not a name any of this control's events publishes.
        assert!(!registry.has("submitted"));
        assert!(!registry.has("changed"));
    }

    #[test]
    fn an_unregistered_control_is_an_error_not_an_empty_registry() {
        let reason =
            Registry::from_control("no_such_control").expect_err("the library has no such control");
        assert!(reason.contains("registers no control"), "{reason}");
    }

    #[test]
    fn binding_succeeds_and_unbinding_removes_only_that_nodes_event() {
        let registry = Registry::from_control("button").expect("a registry");
        let mut wiring = Wiring::new();
        wiring
            .attach(&registry, node(1), "clicked", "first")
            .expect("a declared event binds");
        wiring
            .attach(&registry, node(2), "clicked", "second")
            .expect("a declared event binds");

        assert!(wiring.detach(node(1), "clicked").is_some());
        assert!(!wiring.is_wired(node(1), "clicked"));
        assert!(wiring.is_wired(node(2), "clicked"));
        assert_eq!(wiring.handler_of(node(2), "clicked"), Some("second"));
        assert_eq!(wiring.attachments.len(), 1);
    }

    #[test]
    fn an_unwired_schema_declared_event_is_reported_unwired_not_as_success() {
        let registry = Registry::from_control("button").expect("a registry");
        let firing = fire(
            &registry,
            &Wiring::new(),
            node(1),
            &Trigger {
                event: "clicked".to_owned(),
                payload: Data::new(),
            },
        )
        .expect("a firing");
        assert!(!firing.wired);
        assert!(firing.problems.is_empty());
        assert!(firing.render(node(1))[0].contains("no handler is wired"));
    }

    #[test]
    fn firing_an_unexpressible_payload_is_refused_rather_than_checked_against_nothing() {
        let registry = Registry::from_event_schemas(&[EventSchema::list(
            "samples_changed",
            PropertyValueKind::Float,
        )]);
        let reason = fire(
            &registry,
            &Wiring::new(),
            node(1),
            &Trigger {
                event: "samples_changed".to_owned(),
                payload: Data::new(),
            },
        )
        .expect_err("an unexpressible payload");
        assert!(reason.contains("cannot express"), "{reason}");
    }

    fn field(name: &str, kind: FieldKind, required: bool) -> DataField {
        DataField {
            name: name.to_owned(),
            kind,
            required,
        }
    }

    #[test]
    fn the_registry_renders_the_payload_contract() {
        let lines = registry().render();
        assert_eq!(lines, vec!["event pressed (count: integer, label: text)"]);
    }

    #[test]
    fn the_wiring_renders_its_attachments() {
        let mut wiring = Wiring::new();
        wiring
            .attach(&registry(), node(1), "pressed", "on_press")
            .expect("an attach");
        assert_eq!(wiring.render(), vec!["node 1: pressed -> on_press"]);
    }

    #[test]
    fn attaching_a_declared_event_issues_exactly_one_handle() {
        let mut wiring = Wiring::new();
        let handle = wiring
            .attach(&registry(), node(1), "pressed", "on_press")
            .expect("an attach");
        assert_eq!(wiring.attachments.len(), 1);
        assert_eq!(wiring.attachments[0].handle, handle);
        assert_eq!(wiring.handle_of(node(1), "pressed"), Some(handle));
        assert!(wiring.is_live(handle));
        assert!(wiring.released().is_empty());
    }

    #[test]
    fn unbinding_drops_exactly_the_one_handle_and_leaves_another_nodes_alone() {
        let mut wiring = Wiring::new();
        let first = wiring
            .attach(&registry(), node(1), "pressed", "a")
            .expect("an attach");
        let second = wiring
            .attach(&registry(), node(2), "pressed", "b")
            .expect("an attach");

        let removed = wiring.detach(node(1), "pressed").expect("the record");
        assert_eq!(removed.handle, first);
        assert_eq!(wiring.released(), &[first]);
        assert!(!wiring.is_live(first));
        assert!(wiring.is_live(second));
        assert!(wiring.attachment_of_handle(first).is_none());
        assert_eq!(wiring.handler_of(node(2), "pressed"), Some("b"));

        // A second unbind removes nothing, and cannot reach node 2's attachment.
        assert!(wiring.detach(node(1), "pressed").is_none());
        assert_eq!(wiring.attachments.len(), 1);
        assert_eq!(wiring.released(), &[first]);
        assert!(wiring.is_live(second));
    }

    #[test]
    fn re_attaching_retires_the_previous_handle_instead_of_reusing_it() {
        let mut wiring = Wiring::new();
        let first = wiring
            .attach(&registry(), node(1), "pressed", "first")
            .expect("an attach");
        let second = wiring
            .attach(&registry(), node(1), "pressed", "second")
            .expect("an attach");
        assert_ne!(first, second);
        assert!(!wiring.is_live(first));
        assert!(wiring.is_live(second));
        assert_eq!(wiring.released(), &[first]);
        assert_eq!(wiring.attachments.len(), 1);
    }

    #[test]
    fn a_retired_record_cannot_be_reported_as_wired_or_fired() {
        let registry = Registry::from_control("button").expect("a registry");
        let mut wiring = Wiring::new();
        let handle = wiring
            .attach(&registry, node(1), "clicked", "count")
            .expect("an attach");
        wiring.detach(node(1), "clicked").expect("the record");

        assert_eq!(wiring.handle_of(node(1), "clicked"), None);
        assert!(!wiring.is_live(handle));
        let firing = fire(
            &registry,
            &wiring,
            node(1),
            &Trigger {
                event: "clicked".to_owned(),
                payload: Data::new(),
            },
        )
        .expect("a firing");
        assert!(!firing.wired);
        assert!(firing.render(node(1))[0].contains("no handler is wired"));
    }

    #[test]
    fn the_wire_source_is_derived_from_the_librarys_declaration() {
        let schemas = [
            EventSchema::unit("clicked"),
            EventSchema::scalar("state_changed", PropertyValueKind::String),
            EventSchema::pair("range_changed", PropertyValueKind::UInt),
            EventSchema::list("samples_changed", PropertyValueKind::Float),
        ];
        let registry = Registry::from_event_schemas(&schemas);
        assert_eq!(
            registry.get("clicked").map(EventDecl::wire_source),
            Some(WireSource::PayloadFree)
        );
        assert_eq!(
            registry.get("state_changed").map(EventDecl::wire_source),
            Some(WireSource::Value(PropertyValueKind::String))
        );
        // A pair is a payload, not the first of its two kinds: a scalar target cannot take it.
        assert_eq!(
            registry.get("range_changed").map(EventDecl::wire_source),
            Some(WireSource::Payload)
        );
        assert_eq!(
            registry.get("samples_changed").map(EventDecl::wire_source),
            Some(WireSource::Payload)
        );
        assert_eq!(registry.get("submitted"), None);
    }

    #[test]
    fn an_explicit_field_list_is_read_as_no_value_one_value_or_a_payload() {
        let registry = registry();
        assert_eq!(
            registry.get("pressed").map(EventDecl::wire_source),
            Some(WireSource::Payload),
            "two fields are a payload, not a single value"
        );
        let one = Registry::new().with(
            "changed",
            vec![DataField {
                name: "value".to_owned(),
                kind: FieldKind::Integer,
                required: true,
            }],
        );
        assert_eq!(
            one.get("changed").map(EventDecl::wire_source),
            Some(WireSource::Value(PropertyValueKind::Int))
        );
        let none = Registry::new().with("clicked", Vec::new());
        assert_eq!(
            none.get("clicked").map(EventDecl::wire_source),
            Some(WireSource::PayloadFree)
        );
    }

    #[test]
    fn the_wire_source_spelling_round_trips_through_parse_and_render() {
        for text in ["none", "payload", "int", "string", "enum", "color"] {
            let source = WireSource::parse(text).expect("a source spelling");
            assert!(
                !source.render().is_empty(),
                "{text} must render its contract"
            );
        }
        assert!(WireSource::parse("no_such_kind").is_none());
    }
}
