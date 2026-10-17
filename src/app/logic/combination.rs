// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! What each kind of change needs, which interactions are ready, and keeping a late result out of the
//! version it does not belong to (task T17.07).
//!
//! The acceptance line is *"a pure style change does not wait for an unrelated build, and an old result
//! does not pollute the new version"*:
//!
//! * **A change says what it needs.** [`Change::requires_logic_build`] is false for a style change, so a
//!   style edit is not made to wait for a logic build it does not depend on (principle 7 and T14.02,
//!   carried across the process boundary).
//! * **An interaction that is not ready is disabled with a reason.** [`readiness`] answers
//!   [`Readiness::Disabled`] while a needed build is running or the protocol does not match, so a
//!   control is not offered as working before it is.
//! * **A result is stamped with its generation.** [`deliver`] applies a result only to the generation it
//!   was computed for; an older one is [`Delivery::Isolated`] and reported, never applied — the echo rule
//!   the preview service uses, applied to a swapped-in result.

#![allow(dead_code)]

/// One generation of the running logic.
pub type Generation = u64;

/// A kind of change to the design, and what the logic process must do about it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Change {
    /// An interaction style: absorbed where it is drawn.
    Style,
    /// The document structure: changes the generated logic.
    Document,
    /// A binding: the generated logic must carry it.
    Binding,
    /// Hand-written logic text.
    Logic,
    /// The protocol between the two processes.
    Protocol,
}

impl Change {
    /// The change's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Style => "style",
            Self::Document => "document",
            Self::Binding => "binding",
            Self::Logic => "logic",
            Self::Protocol => "protocol",
        }
    }

    /// Reads a change's name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "style" => Some(Self::Style),
            "document" => Some(Self::Document),
            "binding" => Some(Self::Binding),
            "logic" => Some(Self::Logic),
            "protocol" => Some(Self::Protocol),
            _ => None,
        }
    }

    /// Whether the change needs the logic process rebuilt.
    ///
    /// A style change needs nothing built, which is the whole point of the acceptance line; everything
    /// that can reach the generated logic does.
    pub fn requires_logic_build(&self) -> bool {
        !matches!(self, Self::Style)
    }
}

/// A batch of changes.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Plan {
    /// The changes, in the order they arrived.
    pub changes: Vec<Change>,
}

impl Plan {
    /// A plan with nothing in it.
    pub fn new() -> Self {
        Self::default()
    }

    /// The same plan with one more change.
    pub fn with(mut self, change: Change) -> Self {
        self.changes.push(change);
        self
    }

    /// Whether anything in the plan needs the logic process rebuilt.
    pub fn requires_logic_build(&self) -> bool {
        self.changes.iter().any(Change::requires_logic_build)
    }
}

/// Whether an interaction may be used right now.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Readiness {
    /// It may be used.
    Ready,
    /// It must be disabled, and this says why.
    Disabled {
        /// Why.
        reason: String,
    },
}

impl Readiness {
    /// Whether the interaction may be used.
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready)
    }

    /// The readiness in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Ready => "ready".to_owned(),
            Self::Disabled { reason } => format!("disabled: {reason}"),
        }
    }
}

/// An interaction and the changes it depends on.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Interaction {
    /// The interaction's name.
    pub name: String,
    /// What it depends on.
    pub needs: Vec<Change>,
}

impl Interaction {
    /// An interaction with no dependencies.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            needs: Vec::new(),
        }
    }

    /// The same interaction depending on one more change.
    pub fn needs(mut self, change: Change) -> Self {
        self.needs.push(change);
        self
    }
}

/// The state of the world an interaction's readiness is judged against.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Conditions {
    /// Whether a logic build is running.
    pub building: bool,
    /// Whether the business process speaks this build's protocol.
    pub protocol_matches: bool,
}

/// Whether an interaction may be used under the current conditions.
pub fn readiness(interaction: &Interaction, conditions: &Conditions) -> Readiness {
    if conditions.building && interaction.needs.iter().any(Change::requires_logic_build) {
        return Readiness::Disabled {
            reason: "a logic build is running".to_owned(),
        };
    }
    if !conditions.protocol_matches && interaction.needs.contains(&Change::Protocol) {
        return Readiness::Disabled {
            reason: "the business process speaks another protocol".to_owned(),
        };
    }
    Readiness::Ready
}

/// A value computed by one generation of the logic.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Computed {
    /// The generation it was computed at.
    pub generation: Generation,
    /// The value.
    pub value: String,
}

impl Computed {
    /// A computed value at a generation.
    pub fn new(generation: Generation, value: impl Into<String>) -> Self {
        Self {
            generation,
            value: value.into(),
        }
    }
}

/// What became of a computed value offered to a generation.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Delivery {
    /// It belongs to the current generation and is applied.
    Applied(Computed),
    /// It is from an older generation and is isolated, never applied.
    Isolated {
        /// The generation it came from.
        generation: Generation,
        /// The current generation.
        current: Generation,
    },
}

impl Delivery {
    /// Whether the value was applied.
    pub fn is_applied(&self) -> bool {
        matches!(self, Self::Applied(_))
    }

    /// The delivery in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Applied(computed) => format!(
                "generation {}: applied {}",
                computed.generation, computed.value
            ),
            Self::Isolated {
                generation,
                current,
            } => format!(
                "generation {generation}: isolated (the running logic is generation {current})"
            ),
        }
    }
}

/// Applies a computed value only to the generation it belongs to.
pub fn deliver(computed: Computed, current: Generation) -> Delivery {
    if computed.generation == current {
        Delivery::Applied(computed)
    } else {
        Delivery::Isolated {
            generation: computed.generation,
            current,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Change, Computed, Conditions, Delivery, Interaction, Plan, Readiness, deliver, readiness,
    };

    #[test]
    fn a_style_change_does_not_require_a_logic_build() {
        assert!(!Change::Style.requires_logic_build());
        assert!(!Plan::new().with(Change::Style).requires_logic_build());
    }

    #[test]
    fn changes_that_reach_the_generated_logic_do() {
        for change in [
            Change::Document,
            Change::Binding,
            Change::Logic,
            Change::Protocol,
        ] {
            assert!(change.requires_logic_build(), "{}", change.name());
        }
    }

    #[test]
    fn a_style_only_interaction_stays_ready_while_a_build_runs() {
        let interaction = Interaction::new("hover-highlight").needs(Change::Style);
        let conditions = Conditions {
            building: true,
            protocol_matches: true,
        };
        assert_eq!(readiness(&interaction, &conditions), Readiness::Ready);
        assert!(readiness(&interaction, &conditions).is_ready());
    }

    #[test]
    fn an_interaction_that_needs_the_logic_is_disabled_while_building() {
        let interaction = Interaction::new("place-order").needs(Change::Logic);
        let conditions = Conditions {
            building: true,
            protocol_matches: true,
        };
        let disabled = readiness(&interaction, &conditions);
        assert!(!disabled.is_ready());
        assert!(disabled.render().contains("logic build is running"));
    }

    #[test]
    fn a_protocol_mismatch_disables_a_protocol_dependent_interaction() {
        let interaction = Interaction::new("stream-feed").needs(Change::Protocol);
        let conditions = Conditions {
            building: false,
            protocol_matches: false,
        };
        let disabled = readiness(&interaction, &conditions);
        assert!(disabled.render().contains("another protocol"));
    }

    #[test]
    fn a_result_from_the_current_generation_is_applied() {
        let delivery = deliver(Computed::new(4, "42"), 4);
        assert!(delivery.is_applied());
        assert_eq!(delivery, Delivery::Applied(Computed::new(4, "42")));
    }

    #[test]
    fn a_result_from_an_older_generation_is_isolated() {
        let delivery = deliver(Computed::new(3, "42"), 4);
        assert_eq!(
            delivery,
            Delivery::Isolated {
                generation: 3,
                current: 4,
            }
        );
        assert!(!delivery.is_applied());
        assert!(delivery.render().contains("isolated"));
    }

    #[test]
    fn change_names_parse() {
        for change in [
            Change::Style,
            Change::Document,
            Change::Binding,
            Change::Logic,
            Change::Protocol,
        ] {
            assert_eq!(Change::parse(change.name()), Some(change));
        }
        assert_eq!(Change::parse("other"), None);
    }
}
