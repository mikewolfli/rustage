// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Event handlers as actions: writing a property, invoking a command or calling a business function,
//! with the permission each needs and the subscription each holds (task T13.04).
//!
//! The acceptance line is *"replacing or removing a handler does not leave duplicate subscriptions,
//! and an illegal action is refused"*. Both are about a handler that never quite goes away, so:
//!
//! * **One handler per event, and a replacement releases the last.** A binding is keyed by node and
//!   event; installing over an existing one takes the old subscription out and records it as
//!   [`released`](Dispatcher::released), so a re-wire leaves exactly one subscription rather than
//!   two.
//! * **Permission is the contract's, not this module's.** An action names the [`Capability`] it needs
//!   and the decision comes from the project's [`PermissionGate`], so the designer cannot invent a
//!   grant the host does not have (principle 12/21).
//! * **An illegal action is refused before it is bound.** A write to a slot that does not exist, a
//!   command that is not registered, a function the seam does not expose, or a denied capability
//!   answers `Err` — and the existing binding, if any, is left exactly as it was.

#![allow(dead_code)]

use std::collections::BTreeSet;

use crate::app::data::binding::Environment;
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
    /// Every subscription that has been released, in order.
    released: Vec<Subscription>,
}

impl Dispatcher {
    /// A dispatcher with nothing bound.
    pub fn new(environment: Environment) -> Self {
        Self {
            environment,
            commands: BTreeSet::new(),
            functions: BTreeSet::new(),
            bindings: Vec::new(),
            released: Vec::new(),
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

    /// Binds a handler, replacing and releasing any handler already bound to the same node and event.
    pub fn install(
        &mut self,
        gate: &PermissionGate,
        principal: Principal,
        node: NodeId,
        event: &str,
        action: Action,
    ) -> Result<Install, String> {
        self.check(&action)?;
        if let Some(capability) = action.capability()
            && let Decision::Denied { reason, .. } = gate.decide(principal, capability)
        {
            return Err(format!(
                "the {capability:?} capability is denied: {reason:?}"
            ));
        }
        let subscription = Subscription {
            node,
            event: event.to_owned(),
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
}

#[cfg(test)]
mod tests {
    use super::{Action, Dispatcher, Install};
    use crate::app::data::binding::Environment;
    use crate::app::data::data_model::Schema;
    use crate::app::style::property_editor::PropertyType;
    use crate::contracts::identity::NodeId;
    use crate::contracts::model::{PropertyName, PropertyValue};
    use crate::contracts::permission::{Capability, PermissionGate, Principal};

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
}
