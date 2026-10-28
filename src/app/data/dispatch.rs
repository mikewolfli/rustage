// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Event handlers as actions: writing a property, invoking a command or calling a business function,
//! with the permission each needs and the subscription each holds (task T13.04).
//!
//! The acceptance line is *"replacing or removing a handler does not leave duplicate subscriptions,
//! and an illegal action is refused"*. Both are about a handler that never quite goes away, so:
//!
//! * **The target is checked against the library's own rule table.** [`WIRE_RULES`] is consulted
//!   through [`compatibility`](rust_widgets::widget::capability::compatibility), not re-expressed
//!   here. A command is a [`WireTarget::PayloadFreeCommand`], so only a payload-free event may reach
//!   it; a property is a [`WireTarget::Property`] of the slot's own kind, so a payload-free event
//!   cannot assign it and a payload of the wrong kind is refused rather than converted by guess.
//!   An event whose declaration is not known is not silently accepted: a table-covered target
//!   (`set`, `command`) is refused until the event's [`WireSource`] is declared.
//! * **One handler per event, and a replacement retires the last handle.** A binding is keyed by
//!   node and event; installing over an existing one takes the old subscription out and records it
//!   as [`released`](Dispatcher::released), so a re-wire leaves exactly one live handle rather than
//!   two. [`Dispatcher::dispatch_by_handle`] refuses a retired handle, which is what makes "no stale
//!   handle" observable rather than implied.
//! * **A scope releases what it installed.** [`BindingScope`] mirrors
//!   [`ConnectionScope`](rust_widgets::signal::ConnectionScope): dropping it unbinds every handle it
//!   tracks. The library scope can only track disconnectors registered through
//!   `Signal::connect_scoped`, so with no mounted control and no signal hub here there is nothing to
//!   register on; this host-side scope keeps the drop semantics while the handles stay the library's
//!   opaque type.
//! * **Permission is the contract's, not this module's.** An action names the [`Capability`] it needs
//!   and the decision comes from the project's [`PermissionGate`], so the designer cannot invent a
//!   grant the host does not have (principle 12/21).
//! * **An illegal action is refused before it is bound.** A write to a slot that does not exist, a
//!   command that is not registered, a function the seam does not expose, a wire the table rejects,
//!   or a denied capability answers `Err` — and the existing binding, if any, is left exactly as it
//!   was.
//!
//! # The runtime gap this module does not paper over
//!
//! The library's live subscription path is [`EventSignalRef`](rust_widgets::signal::EventSignalRef)
//! (`subscribe`/`disconnect`) and [`EventSignalBinder`](rust_widgets::signal::EventSignalBinder), and
//! both need a mounted control (`Widget::event_signal_dyn`) plus, for the binder, a
//! [`CustomSignalHub`](rust_widgets::signal::CustomSignalHub). This module has neither. Its
//! [`ConnectionHandle`]s therefore identify records here, not slots on a control's signal: no slot is
//! connected and no live subscription is claimed. Installing one against a mounted control remains
//! the runtime mount path's job (T14.02).

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use rust_widgets::signal::ConnectionHandle;
use rust_widgets::widget::capability::{
    PropertyValueKind, WireCompatibility, WireTarget, compatibility,
};

use crate::app::data::binding::Environment;
use crate::app::data::events::{Registry, WireSource};
use crate::app::style::property_editor::PropertyType;
use crate::contracts::identity::NodeId;
use crate::contracts::model::{PropertyName, PropertyValue};
use crate::contracts::permission::{Capability, Decision, PermissionGate, Principal};

/// What a handler does when its event fires.
#[derive(Clone, PartialEq, Debug)]
pub enum Action {
    /// Write a value to a node property.
    Set {
        /// The node.
        node: NodeId,
        /// The property.
        property: PropertyName,
        /// The value to write.
        value: PropertyValue,
    },
    /// Invoke one of the designer's own commands, already gated at its own entry point.
    Command {
        /// The command's id.
        id: String,
    },
    /// Call a business function through the Rust seam.
    Call {
        /// The function the seam exposes.
        function: String,
        /// The capability the call needs.
        needs: Capability,
    },
}

impl Action {
    /// The capability the action needs, if any.
    ///
    /// A property write needs [`Capability::EditWidgetProperty`]; a command needs nothing here
    /// because its own entry point already decided; a business call needs what it declared.
    pub fn capability(&self) -> Option<Capability> {
        match self {
            Self::Set { .. } => Some(Capability::EditWidgetProperty),
            Self::Command { .. } => None,
            Self::Call { needs, .. } => Some(*needs),
        }
    }

    /// What kind of action this is.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Set { .. } => "set",
            Self::Command { .. } => "command",
            Self::Call { .. } => "call",
        }
    }

    /// The action in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Set { node, property, .. } => {
                format!("set {}.{}", node.get(), property.as_str())
            }
            Self::Command { id } => format!("command {id}"),
            Self::Call { function, .. } => format!("call {function}"),
        }
    }
}

/// The subscription one binding holds.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Subscription {
    /// The node.
    pub node: NodeId,
    /// The event.
    pub event: String,
    /// The record's handle, the library's opaque connection identity.
    ///
    /// No slot is connected (see the module docs); the handle identifies this record so a retired
    /// one can be told from a live one.
    pub handle: ConnectionHandle,
}

impl Subscription {
    /// The subscription in one line.
    pub fn render(&self) -> String {
        format!("node {}: {}", self.node.get(), self.event)
    }
}

/// One handler bound to a node's event.
#[derive(Clone, PartialEq, Debug)]
pub struct Binding {
    /// The node.
    pub node: NodeId,
    /// The event.
    pub event: String,
    /// What it does.
    pub action: Action,
    /// The subscription it holds.
    pub subscription: Subscription,
}

/// What installing a handler did.
#[derive(Clone, PartialEq, Debug)]
pub enum Install {
    /// A new handler was bound.
    Installed,
    /// An existing handler was replaced, and this subscription was released.
    Replaced {
        /// What was released.
        released: Subscription,
    },
}

impl Install {
    /// Whether an existing handler was replaced.
    pub fn is_replacement(&self) -> bool {
        matches!(self, Self::Replaced { .. })
    }

    /// The install in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Installed => "installed".to_owned(),
            Self::Replaced { released } => {
                format!("replaced, released {}", released.render())
            }
        }
    }
}

/// What dispatching an event did.
#[derive(Clone, PartialEq, Debug)]
pub struct Dispatch {
    /// The action that ran.
    pub action: Action,
    /// The subscription it ran under.
    pub subscription: Subscription,
}

impl Dispatch {
    /// The handle this dispatch ran under.
    pub fn handle(&self) -> ConnectionHandle {
        self.subscription.handle
    }

    /// The dispatch in one line.
    pub fn render(&self) -> String {
        format!("{} -> {}", self.subscription.render(), self.action.render())
    }
}

/// The handlers bound in a document, and the subscriptions they hold.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Dispatcher {
    /// Where slot existence comes from.
    pub environment: Environment,
    /// The command ids that exist.
    pub commands: BTreeSet<String>,
    /// The business functions the seam exposes.
    pub functions: BTreeSet<String>,
    /// The bindings, at most one per node and event.
    pub bindings: Vec<Binding>,
    /// The wire source each declared event carries, when it is known.
    event_payloads: BTreeMap<String, WireSource>,
    /// Every subscription that has been released, in order.
    released: Vec<Subscription>,
    /// The next handle to issue. The first binding holds handle 1.
    next_handle: u64,
}

impl Dispatcher {
    /// A dispatcher with nothing bound.
    pub fn new(environment: Environment) -> Self {
        Self {
            environment,
            commands: BTreeSet::new(),
            functions: BTreeSet::new(),
            bindings: Vec::new(),
            event_payloads: BTreeMap::new(),
            released: Vec::new(),
            next_handle: 0,
        }
    }

    /// The same dispatcher with a command registered.
    pub fn with_command(mut self, id: impl Into<String>) -> Self {
        self.commands.insert(id.into());
        self
    }

    /// The same dispatcher with a business function exposed.
    pub fn with_function(mut self, name: impl Into<String>) -> Self {
        self.functions.insert(name.into());
        self
    }

    /// The same dispatcher with every event in `registry` declared for the wire table.
    ///
    /// Only the event's payload shape is kept: the declaration itself stays in the registry. A
    /// control's `EventSchema` rows are the normal source, so the answer here is the library's and
    /// not a caller's guess.
    pub fn with_registry(mut self, registry: &Registry) -> Self {
        for declaration in &registry.events {
            self.event_payloads
                .insert(declaration.name.clone(), declaration.wire_source());
        }
        self
    }

    /// Declares one event's wire source (the command-line path, and a test's explicit fixture).
    pub fn declare_event(&mut self, event: impl Into<String>, source: WireSource) {
        self.event_payloads.insert(event.into(), source);
    }

    /// The wire source declared for an event, or [`WireSource::Undeclared`].
    pub fn event_source(&self, event: &str) -> WireSource {
        self.event_payloads
            .get(event)
            .copied()
            .unwrap_or(WireSource::Undeclared)
    }

    /// The next opaque handle this dispatcher will issue.
    fn issue_handle(&mut self) -> ConnectionHandle {
        self.next_handle += 1;
        ConnectionHandle(self.next_handle)
    }

    /// Binds a handler, replacing and releasing any handler already bound to the same node and event.
    ///
    /// The event's wire source is taken from this dispatcher's declarations when one exists. When
    /// none does, the caller has supplied only a name; that path is kept for the smoke harness and
    /// is not used by the command line or the panel, both of which declare their events first.
    pub fn install(
        &mut self,
        gate: &PermissionGate,
        principal: Principal,
        node: NodeId,
        event: &str,
        action: Action,
    ) -> Result<Install, String> {
        let source = self.event_payloads.get(event).copied();
        self.install_checked(gate, principal, node, event, source, action)
    }

    /// Binds a handler after checking the event's declared [`WireSource`] against `WIRE_RULES`.
    ///
    /// This is the entry a policy-bearing caller uses: an undeclared event is refused for a `set` or
    /// `command` target rather than accepted on faith, because the table cannot prove the wire is
    /// legal. A business `call` is outside the table (a function's parameter contract is the seam's,
    /// not a property kind), so it is checked by the command/function registries instead.
    pub fn install_wired(
        &mut self,
        gate: &PermissionGate,
        principal: Principal,
        node: NodeId,
        event: &str,
        source: WireSource,
        action: Action,
    ) -> Result<Install, String> {
        self.install_checked(gate, principal, node, event, Some(source), action)
    }

    /// The shared body of [`Self::install`] and [`Self::install_wired`].
    fn install_checked(
        &mut self,
        gate: &PermissionGate,
        principal: Principal,
        node: NodeId,
        event: &str,
        source: Option<WireSource>,
        action: Action,
    ) -> Result<Install, String> {
        self.check(&action)?;
        if let Some(source) = source {
            self.check_wire(source, &action)?;
        }
        if let Some(capability) = action.capability()
            && let Decision::Denied { reason, .. } = gate.decide(principal, capability)
        {
            return Err(format!(
                "the {capability:?} capability is denied: {reason:?}"
            ));
        }
        let handle = self.issue_handle();
        let subscription = Subscription {
            node,
            event: event.to_owned(),
            handle,
        };
        if let Some(index) = self
            .bindings
            .iter()
            .position(|binding| binding.node == node && binding.event == event)
        {
            let previous = self.bindings.remove(index);
            self.released.push(previous.subscription.clone());
            self.bindings.push(Binding {
                node,
                event: event.to_owned(),
                action,
                subscription: subscription.clone(),
            });
            return Ok(Install::Replaced {
                released: previous.subscription,
            });
        }
        self.bindings.push(Binding {
            node,
            event: event.to_owned(),
            action,
            subscription,
        });
        Ok(Install::Installed)
    }

    /// Removes a handler and releases its subscription.
    pub fn unbind(&mut self, node: NodeId, event: &str) -> Option<Subscription> {
        let index = self
            .bindings
            .iter()
            .position(|binding| binding.node == node && binding.event == event)?;
        let removed = self.bindings.remove(index);
        self.released.push(removed.subscription.clone());
        Some(removed.subscription)
    }

    /// Removes the handler exactly `handle` names, if it is still the live one.
    ///
    /// This is the detach-by-identity form: a handle retired by a replacement or an earlier unbind
    /// answers `None` and cannot remove the binding that replaced it.
    pub fn unbind_handle(&mut self, handle: ConnectionHandle) -> Option<Subscription> {
        let index = self
            .bindings
            .iter()
            .position(|binding| binding.subscription.handle == handle)?;
        let removed = self.bindings.remove(index);
        self.released.push(removed.subscription.clone());
        Some(removed.subscription)
    }

    /// The subscriptions currently held, one per binding.
    pub fn subscriptions(&self) -> Vec<Subscription> {
        self.bindings
            .iter()
            .map(|binding| binding.subscription.clone())
            .collect()
    }

    /// Every subscription that has been released, in order.
    pub fn released(&self) -> &[Subscription] {
        &self.released
    }

    /// The live handle bound to a node's event, if any.
    pub fn handle_of(&self, node: NodeId, event: &str) -> Option<ConnectionHandle> {
        self.bindings
            .iter()
            .find(|binding| binding.node == node && binding.event == event)
            .map(|binding| binding.subscription.handle)
    }

    /// Whether a handle still names a live binding.
    pub fn is_live(&self, handle: ConnectionHandle) -> bool {
        self.bindings
            .iter()
            .any(|binding| binding.subscription.handle == handle)
    }

    /// The action bound to a node's event, if any.
    pub fn action_of(&self, node: NodeId, event: &str) -> Option<&Action> {
        self.bindings
            .iter()
            .find(|binding| binding.node == node && binding.event == event)
            .map(|binding| &binding.action)
    }

    /// Dispatches a node's event, running the bound action.
    pub fn dispatch(&self, node: NodeId, event: &str) -> Result<Dispatch, String> {
        let binding = self
            .bindings
            .iter()
            .find(|binding| binding.node == node && binding.event == event)
            .ok_or_else(|| format!("nothing is bound to node {}'s {event}", node.get()))?;
        Ok(Dispatch {
            action: binding.action.clone(),
            subscription: binding.subscription.clone(),
        })
    }

    /// Dispatches the handler `handle` names, refusing a retired handle.
    ///
    /// The runtime's `Signal::disconnect` answers `false` for a handle it no longer holds; this is
    /// the same question one level up, so a caller that kept a handle across a replacement or an
    /// unbind cannot make the old action run.
    pub fn dispatch_by_handle(&self, handle: ConnectionHandle) -> Result<Dispatch, String> {
        let binding = self
            .bindings
            .iter()
            .find(|binding| binding.subscription.handle == handle)
            .ok_or_else(|| format!("connection {} is no longer live", handle.0))?;
        Ok(Dispatch {
            action: binding.action.clone(),
            subscription: binding.subscription.clone(),
        })
    }

    /// A drop-scoped owner over this dispatcher's bindings.
    pub fn scope(&mut self) -> BindingScope<'_> {
        BindingScope::new(self)
    }

    /// The dispatcher in one line per binding.
    pub fn render(&self) -> Vec<String> {
        let mut lines: Vec<String> = self
            .bindings
            .iter()
            .map(|binding| {
                format!(
                    "{} -> {}",
                    binding.subscription.render(),
                    binding.action.render()
                )
            })
            .collect();
        lines.push(format!(
            "bindings: {}, released: {}",
            self.bindings.len(),
            self.released.len()
        ));
        lines
    }

    /// Whether an action is legal at all, before permission is considered.
    fn check(&self, action: &Action) -> Result<(), String> {
        match action {
            Action::Set { node, property, .. } => {
                if self.environment.slot_kind(*node, property).is_some() {
                    Ok(())
                } else {
                    Err(format!(
                        "node {} declares no property {:?}",
                        node.get(),
                        property.as_str()
                    ))
                }
            }
            Action::Command { id } => {
                if self.commands.contains(id) {
                    Ok(())
                } else {
                    Err(format!("no command {id:?} is registered"))
                }
            }
            Action::Call { function, .. } => {
                if self.functions.contains(function) {
                    Ok(())
                } else {
                    Err(format!("the seam does not expose {function:?}"))
                }
            }
        }
    }

    /// Whether `WIRE_RULES` admits an event carrying `source` into `action`'s target.
    ///
    /// The decision is the library's: [`compatibility`] is called with the source kind and the
    /// target kind, and its rejection reason is passed through. A business `call` has no entry in
    /// the table (its parameter contract belongs to the seam), so it is not judged here.
    fn check_wire(&self, source: WireSource, action: &Action) -> Result<WireCompatibility, String> {
        let target = match action {
            Action::Command { .. } => WireTarget::PayloadFreeCommand,
            Action::Set { node, property, .. } => {
                let kind = self.environment.slot_kind(*node, property).ok_or_else(|| {
                    format!(
                        "node {} declares no property {:?}",
                        node.get(),
                        property.as_str()
                    )
                })?;
                let Some(kind) = wire_kind(kind) else {
                    return Err(format!(
                        "node {}'s property {:?} has a type with no library wire target; \
                         WIRE_RULES cannot check this wire",
                        node.get(),
                        property.as_str()
                    ));
                };
                WireTarget::Property(kind)
            }
            Action::Call { .. } => return Ok(WireCompatibility::Direct),
        };
        let source_kind = match source {
            WireSource::PayloadFree => None,
            WireSource::Value(kind) => Some(kind),
            WireSource::Payload => {
                return Err(
                    "the event carries a payload WIRE_RULES cannot address as one value; \
                     the wire is refused"
                        .to_owned(),
                );
            }
            WireSource::Undeclared => {
                return Err(
                    "the event's payload is not declared, so WIRE_RULES cannot prove this wire \
                     is legal; declare the event from its control's EventSchema"
                        .to_owned(),
                );
            }
        };
        match compatibility(source_kind, target) {
            verdict @ (WireCompatibility::Direct | WireCompatibility::Converted) => Ok(verdict),
            WireCompatibility::Rejected(reason) => Err(format!(
                "the library's WIRE_RULES refuses this wire: {reason}"
            )),
        }
    }
}

/// The library kind a slot type is checked as, or `None` when the table has no target for it.
fn wire_kind(kind: &PropertyType) -> Option<PropertyValueKind> {
    Some(match kind {
        PropertyType::Number { .. } | PropertyType::Size => PropertyValueKind::Number,
        PropertyType::Integer { .. } => PropertyValueKind::Int,
        PropertyType::Enum { .. } => PropertyValueKind::Enum,
        PropertyType::Text { .. } => PropertyValueKind::String,
        PropertyType::Colour => PropertyValueKind::Color,
        // An asset reference is not a colour, a rectangle or an icon spec; guessing would let a
        // wire the table never stated be reported as checked.
        PropertyType::Asset => return None,
    })
}

/// A drop-scoped owner over the bindings a caller installs through it.
///
/// This is the host-side counterpart of
/// [`ConnectionScope`](rust_widgets::signal::ConnectionScope). The library scope owns
/// disconnectors registered by `Signal::connect_scoped`; with no mounted control and no signal hub
/// there is no signal to register one on, so this scope owns handles instead and releases each by
/// identity when it is dropped or [`release`](Self::release)d. The drop semantics are the same: a
/// caller cannot forget to unbind what it installed through the scope.
pub struct BindingScope<'a> {
    dispatcher: &'a mut Dispatcher,
    handles: Vec<ConnectionHandle>,
}

impl<'a> BindingScope<'a> {
    /// A scope over a dispatcher, holding nothing yet.
    pub fn new(dispatcher: &'a mut Dispatcher) -> Self {
        Self {
            dispatcher,
            handles: Vec::new(),
        }
    }

    /// Installs a binding checked against `WIRE_RULES` and tracks its handle.
    pub fn install(
        &mut self,
        gate: &PermissionGate,
        principal: Principal,
        node: NodeId,
        event: &str,
        source: WireSource,
        action: Action,
    ) -> Result<Install, String> {
        let install = self
            .dispatcher
            .install_wired(gate, principal, node, event, source, action)?;
        if let Install::Replaced { released } = &install {
            // The replacement retired a handle; if this scope owned it, stop tracking it so a
            // later release cannot try to unbind the binding that replaced it.
            self.handles.retain(|handle| *handle != released.handle);
        }
        if let Some(handle) = self.dispatcher.handle_of(node, event) {
            self.handles.push(handle);
        }
        Ok(install)
    }

    /// The handles this scope currently tracks.
    pub fn handles(&self) -> &[ConnectionHandle] {
        &self.handles
    }

    /// Releases every binding this scope still owns, in installation order.
    pub fn release(&mut self) -> Vec<Subscription> {
        let handles = core::mem::take(&mut self.handles);
        handles
            .into_iter()
            .filter_map(|handle| self.dispatcher.unbind_handle(handle))
            .collect()
    }
}

impl Drop for BindingScope<'_> {
    fn drop(&mut self) {
        self.release();
    }
}

#[cfg(test)]
mod tests {
    use super::{Action, BindingScope, Dispatcher, Install};
    use crate::app::data::binding::Environment;
    use crate::app::data::data_model::Schema;
    use crate::app::data::events::{Registry, WireSource};
    use crate::app::style::property_editor::PropertyType;
    use crate::contracts::identity::NodeId;
    use crate::contracts::model::{PropertyName, PropertyValue};
    use crate::contracts::permission::{Capability, PermissionGate, Principal};
    use rust_widgets::widget::capability::{EventSchema, PropertyValueKind};

    fn node(value: u64) -> NodeId {
        NodeId::from_raw(value)
    }

    fn name(text: &str) -> PropertyName {
        PropertyName::new(text).expect("a name")
    }

    fn environment() -> Environment {
        Environment::new(Schema::new("user", 1)).with_slot(
            node(1),
            name("label"),
            PropertyType::Text { max_len: 100 },
        )
    }

    fn dispatcher() -> Dispatcher {
        Dispatcher::new(environment())
            .with_command("save")
            .with_function("fetch_user")
    }

    fn set_action(node_id: u64) -> Action {
        Action::Set {
            node: node(node_id),
            property: name("label"),
            value: PropertyValue::Text("hi".to_owned()),
        }
    }

    #[test]
    fn an_authorized_action_installs_one_subscription() {
        let gate = PermissionGate::new();
        let mut dispatcher = dispatcher();
        let install = dispatcher
            .install(&gate, Principal::Human, node(1), "pressed", set_action(1))
            .expect("an install");
        assert_eq!(install, Install::Installed);
        assert_eq!(dispatcher.subscriptions().len(), 1);
        assert!(dispatcher.released().is_empty());
    }

    #[test]
    fn installing_the_same_event_again_replaces_and_releases() {
        let gate = PermissionGate::new();
        let mut dispatcher = dispatcher();
        dispatcher
            .install(&gate, Principal::Human, node(1), "pressed", set_action(1))
            .expect("an install");
        let install = dispatcher
            .install(&gate, Principal::Human, node(1), "pressed", set_action(1))
            .expect("an install");
        assert!(install.is_replacement());
        assert_eq!(dispatcher.subscriptions().len(), 1);
        assert_eq!(dispatcher.released().len(), 1);
    }

    #[test]
    fn a_hundred_reinstalls_leave_one_subscription_and_ninety_nine_releases() {
        let gate = PermissionGate::new();
        let mut dispatcher = dispatcher();
        for _ in 0..100 {
            dispatcher
                .install(&gate, Principal::Human, node(1), "pressed", set_action(1))
                .expect("an install");
        }
        assert_eq!(dispatcher.subscriptions().len(), 1);
        assert_eq!(dispatcher.released().len(), 99);
    }

    #[test]
    fn unbinding_releases_and_removes() {
        let gate = PermissionGate::new();
        let mut dispatcher = dispatcher();
        dispatcher
            .install(&gate, Principal::Human, node(1), "pressed", set_action(1))
            .expect("an install");
        let released = dispatcher
            .unbind(node(1), "pressed")
            .expect("a subscription");
        assert_eq!(released.render(), "node 1: pressed");
        assert!(dispatcher.subscriptions().is_empty());
        assert_eq!(dispatcher.released().len(), 1);
        assert!(dispatcher.unbind(node(1), "pressed").is_none());
    }

    #[test]
    fn a_denied_capability_is_refused_and_installs_nothing() {
        let gate = PermissionGate::new();
        let mut dispatcher = dispatcher();
        let action = Action::Call {
            function: "fetch_user".to_owned(),
            needs: Capability::RunCargoCheck,
        };
        let outcome = dispatcher.install(&gate, Principal::Human, node(1), "pressed", action);
        assert!(outcome.is_err());
        assert!(dispatcher.subscriptions().is_empty());
        assert!(dispatcher.released().is_empty());
    }

    #[test]
    fn a_trusted_and_authorized_capability_is_accepted() {
        let mut gate = PermissionGate::new();
        gate.trust_project();
        gate.authorize(Capability::RunCargoCheck);
        let mut dispatcher = dispatcher();
        let action = Action::Call {
            function: "fetch_user".to_owned(),
            needs: Capability::RunCargoCheck,
        };
        assert!(
            dispatcher
                .install(&gate, Principal::Human, node(1), "pressed", action)
                .is_ok()
        );
    }

    #[test]
    fn a_command_that_is_not_registered_is_refused() {
        let gate = PermissionGate::new();
        let mut dispatcher = dispatcher();
        let outcome = dispatcher.install(
            &gate,
            Principal::Human,
            node(1),
            "pressed",
            Action::Command {
                id: "gone".to_owned(),
            },
        );
        assert!(outcome.unwrap_err().contains("no command"));
    }

    #[test]
    fn a_function_the_seam_does_not_expose_is_refused() {
        let gate = PermissionGate::new();
        let mut dispatcher = dispatcher();
        let outcome = dispatcher.install(
            &gate,
            Principal::Human,
            node(1),
            "pressed",
            Action::Call {
                function: "gone".to_owned(),
                needs: Capability::EditWidgetProperty,
            },
        );
        assert!(outcome.unwrap_err().contains("does not expose"));
    }

    #[test]
    fn a_write_to_an_unknown_slot_is_refused() {
        let gate = PermissionGate::new();
        let mut dispatcher = dispatcher();
        let outcome =
            dispatcher.install(&gate, Principal::Human, node(9), "pressed", set_action(9));
        assert!(outcome.unwrap_err().contains("node 9"));
    }

    #[test]
    fn a_refused_replacement_leaves_the_original_binding_intact() {
        let gate = PermissionGate::new();
        let mut dispatcher = dispatcher();
        dispatcher
            .install(&gate, Principal::Human, node(1), "pressed", set_action(1))
            .expect("an install");
        let outcome = dispatcher.install(
            &gate,
            Principal::Human,
            node(1),
            "pressed",
            Action::Command {
                id: "gone".to_owned(),
            },
        );
        assert!(outcome.is_err());
        assert_eq!(dispatcher.subscriptions().len(), 1);
        assert_eq!(
            dispatcher.action_of(node(1), "pressed"),
            Some(&set_action(1))
        );
        assert!(dispatcher.released().is_empty());
    }

    #[test]
    fn a_command_needs_no_capability_of_its_own() {
        let gate = PermissionGate::new();
        let mut dispatcher = dispatcher();
        assert!(
            dispatcher
                .install(
                    &gate,
                    Principal::Human,
                    node(1),
                    "pressed",
                    Action::Command {
                        id: "save".to_owned()
                    }
                )
                .is_ok()
        );
    }

    #[test]
    fn dispatching_runs_the_bound_action() {
        let gate = PermissionGate::new();
        let mut dispatcher = dispatcher();
        dispatcher
            .install(&gate, Principal::Human, node(1), "pressed", set_action(1))
            .expect("an install");
        let dispatch = dispatcher.dispatch(node(1), "pressed").expect("a dispatch");
        assert_eq!(dispatch.action.kind(), "set");
        assert_eq!(dispatch.render(), "node 1: pressed -> set 1.label");
    }

    #[test]
    fn dispatching_an_unbound_event_is_refused() {
        let dispatcher = dispatcher();
        assert!(dispatcher.dispatch(node(1), "pressed").is_err());
    }

    #[test]
    fn two_events_on_one_node_hold_two_subscriptions() {
        let gate = PermissionGate::new();
        let mut dispatcher = dispatcher();
        dispatcher
            .install(&gate, Principal::Human, node(1), "pressed", set_action(1))
            .expect("an install");
        dispatcher
            .install(
                &gate,
                Principal::Human,
                node(1),
                "released",
                Action::Command {
                    id: "save".to_owned(),
                },
            )
            .expect("an install");
        assert_eq!(dispatcher.subscriptions().len(), 2);
    }

    #[test]
    fn the_dispatcher_renders_the_bindings_and_the_release_count() {
        let gate = PermissionGate::new();
        let mut dispatcher = dispatcher();
        dispatcher
            .install(&gate, Principal::Human, node(1), "pressed", set_action(1))
            .expect("an install");
        dispatcher
            .install(&gate, Principal::Human, node(1), "pressed", set_action(1))
            .expect("an install");
        let lines = dispatcher.render();
        assert_eq!(lines[0], "node 1: pressed -> set 1.label");
        assert_eq!(lines[1], "bindings: 1, released: 1");
    }

    #[test]
    fn the_action_names_the_capability_it_needs() {
        assert_eq!(
            set_action(1).capability(),
            Some(Capability::EditWidgetProperty)
        );
        assert_eq!(
            Action::Command {
                id: "save".to_owned()
            }
            .capability(),
            None
        );
        assert_eq!(
            Action::Call {
                function: "fetch".to_owned(),
                needs: Capability::RequestNetworkService,
            }
            .capability(),
            Some(Capability::RequestNetworkService)
        );
    }

    fn command(id: &str) -> Action {
        Action::Command { id: id.to_owned() }
    }

    #[test]
    fn a_command_is_refused_for_a_payload_carrying_event() {
        let gate = PermissionGate::new();
        let mut dispatcher = Dispatcher::new(environment()).with_command("save");
        let reason = dispatcher
            .install_wired(
                &gate,
                Principal::Human,
                node(1),
                "state_changed",
                WireSource::Value(PropertyValueKind::String),
                command("save"),
            )
            .expect_err("a payload cannot start a payload-free command");
        assert!(reason.contains("WIRE_RULES"), "{reason}");
        assert!(reason.contains("takes no payload"), "{reason}");
        assert!(dispatcher.subscriptions().is_empty());
        assert!(dispatcher.released().is_empty());
    }

    #[test]
    fn a_command_is_allowed_for_a_payload_free_event() {
        let gate = PermissionGate::new();
        let mut dispatcher = Dispatcher::new(environment()).with_command("save");
        let install = dispatcher
            .install_wired(
                &gate,
                Principal::Human,
                node(1),
                "clicked",
                WireSource::PayloadFree,
                command("save"),
            )
            .expect("a payload-free event reaches a command");
        assert_eq!(install, Install::Installed);
        assert_eq!(dispatcher.subscriptions().len(), 1);
    }

    #[test]
    fn a_property_write_is_judged_by_the_librarys_table() {
        let gate = PermissionGate::new();

        let mut from_nothing = Dispatcher::new(environment());
        let reason = from_nothing
            .install_wired(
                &gate,
                Principal::Human,
                node(1),
                "clicked",
                WireSource::PayloadFree,
                set_action(1),
            )
            .expect_err("a payload-free event cannot assign a property");
        assert!(reason.contains("carries no value"), "{reason}");

        // Int -> String is the one widening conversion the table states.
        let mut from_number = Dispatcher::new(environment());
        assert!(
            from_number
                .install_wired(
                    &gate,
                    Principal::Human,
                    node(1),
                    "count_changed",
                    WireSource::Value(PropertyValueKind::Int),
                    set_action(1),
                )
                .is_ok()
        );
    }

    #[test]
    fn an_undeclared_event_is_not_silently_accepted_by_a_table_target() {
        let gate = PermissionGate::new();
        let mut dispatcher = Dispatcher::new(environment()).with_command("save");
        let reason = dispatcher
            .install_wired(
                &gate,
                Principal::Human,
                node(1),
                "pressed",
                WireSource::Undeclared,
                command("save"),
            )
            .expect_err("an undeclared event cannot be proved payload-free");
        assert!(reason.contains("not declared"), "{reason}");
        assert!(dispatcher.subscriptions().is_empty());
    }

    #[test]
    fn a_declared_registry_is_consulted_by_the_plain_install() {
        let gate = PermissionGate::new();
        let registry = Registry::from_event_schemas(&[EventSchema::scalar(
            "state_changed",
            PropertyValueKind::String,
        )]);
        let mut dispatcher = Dispatcher::new(environment())
            .with_command("save")
            .with_registry(&registry);
        let reason = dispatcher
            .install(
                &gate,
                Principal::Human,
                node(1),
                "state_changed",
                command("save"),
            )
            .expect_err("the declared payload reaches the table");
        assert!(reason.contains("WIRE_RULES"), "{reason}");
        assert!(dispatcher.subscriptions().is_empty());
    }

    #[test]
    fn a_business_call_is_not_judged_by_the_wire_table() {
        // A function's parameter contract belongs to the seam, not to a property kind, so the
        // library's table has no target for it and does not pretend to answer.
        let gate = PermissionGate::new();
        let mut dispatcher = Dispatcher::new(environment()).with_function("fetch_user");
        assert!(
            dispatcher
                .install_wired(
                    &gate,
                    Principal::Human,
                    node(1),
                    "state_changed",
                    WireSource::Value(PropertyValueKind::String),
                    Action::Call {
                        function: "fetch_user".to_owned(),
                        needs: Capability::EditWidgetProperty,
                    },
                )
                .is_ok()
        );
    }

    #[test]
    fn binding_and_unbinding_hold_and_drop_exactly_one_handle() {
        let gate = PermissionGate::new();
        let mut dispatcher = Dispatcher::new(environment()).with_command("save");
        dispatcher
            .install_wired(
                &gate,
                Principal::Human,
                node(1),
                "clicked",
                WireSource::PayloadFree,
                command("save"),
            )
            .expect("an install");
        let handle = dispatcher
            .handle_of(node(1), "clicked")
            .expect("the live handle");
        assert_eq!(dispatcher.subscriptions().len(), 1);
        assert_eq!(dispatcher.subscriptions()[0].handle, handle);
        assert!(dispatcher.is_live(handle));
        assert!(dispatcher.dispatch_by_handle(handle).is_ok());

        let released = dispatcher.unbind(node(1), "clicked").expect("the release");
        assert_eq!(released.handle, handle);
        assert_eq!(dispatcher.released().len(), 1);
        assert_eq!(dispatcher.released()[0].handle, handle);
        assert!(!dispatcher.is_live(handle));
        assert!(dispatcher.dispatch_by_handle(handle).is_err());
        assert!(dispatcher.handle_of(node(1), "clicked").is_none());
    }

    #[test]
    fn a_repeated_unbind_cannot_remove_another_nodes_attachment() {
        let gate = PermissionGate::new();
        let mut dispatcher = Dispatcher::new(environment()).with_command("save");
        for node_id in [1, 2] {
            dispatcher
                .install_wired(
                    &gate,
                    Principal::Human,
                    node(node_id),
                    "clicked",
                    WireSource::PayloadFree,
                    command("save"),
                )
                .expect("an install");
        }
        let first = dispatcher.handle_of(node(1), "clicked").expect("node 1");
        let second = dispatcher.handle_of(node(2), "clicked").expect("node 2");
        assert_ne!(first, second);

        assert!(dispatcher.unbind(node(1), "clicked").is_some());
        assert!(dispatcher.unbind(node(1), "clicked").is_none());
        assert!(dispatcher.unbind_handle(first).is_none());

        assert_eq!(dispatcher.handle_of(node(2), "clicked"), Some(second));
        assert!(dispatcher.is_live(second));
        assert!(!dispatcher.is_live(first));
        assert!(dispatcher.dispatch_by_handle(second).is_ok());
        assert!(dispatcher.dispatch_by_handle(first).is_err());
    }

    #[test]
    fn a_replaced_handle_is_never_live_again() {
        let gate = PermissionGate::new();
        let mut dispatcher = Dispatcher::new(environment()).with_command("save");
        dispatcher
            .install_wired(
                &gate,
                Principal::Human,
                node(1),
                "clicked",
                WireSource::PayloadFree,
                command("save"),
            )
            .expect("an install");
        let first = dispatcher.handle_of(node(1), "clicked").expect("the first");
        dispatcher
            .install_wired(
                &gate,
                Principal::Human,
                node(1),
                "clicked",
                WireSource::PayloadFree,
                command("save"),
            )
            .expect("a replacement");
        let second = dispatcher
            .handle_of(node(1), "clicked")
            .expect("the second");
        assert_ne!(first, second);
        assert!(!dispatcher.is_live(first));
        assert!(dispatcher.is_live(second));
        assert!(dispatcher.dispatch_by_handle(first).is_err());
        assert!(dispatcher.dispatch_by_handle(second).is_ok());
        assert_eq!(dispatcher.released()[0].handle, first);
    }

    #[test]
    fn a_dropped_scope_releases_its_binding_and_cannot_fire() {
        let gate = PermissionGate::new();
        let mut dispatcher = Dispatcher::new(environment()).with_command("save");
        let handle;
        {
            let mut scope = BindingScope::new(&mut dispatcher);
            scope
                .install(
                    &gate,
                    Principal::Human,
                    node(1),
                    "clicked",
                    WireSource::PayloadFree,
                    command("save"),
                )
                .expect("a scoped install");
            assert_eq!(scope.handles().len(), 1);
            handle = scope.handles()[0];
        }
        assert!(!dispatcher.is_live(handle));
        assert!(dispatcher.dispatch_by_handle(handle).is_err());
        assert!(dispatcher.subscriptions().is_empty());
        assert_eq!(dispatcher.released().len(), 1);
        assert_eq!(dispatcher.released()[0].handle, handle);
    }

    #[test]
    fn a_scope_release_is_idempotent() {
        let gate = PermissionGate::new();
        let mut dispatcher = Dispatcher::new(environment()).with_command("save");
        let mut scope = BindingScope::new(&mut dispatcher);
        scope
            .install(
                &gate,
                Principal::Human,
                node(1),
                "clicked",
                WireSource::PayloadFree,
                command("save"),
            )
            .expect("a scoped install");
        assert_eq!(scope.release().len(), 1);
        assert!(scope.release().is_empty());
        assert!(scope.handles().is_empty());
    }
}
