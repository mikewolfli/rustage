// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Multi-selection: the properties several nodes share, the values they agree on, and resetting an
//! override back to the source it came from (task T11.02).
//!
//! The acceptance line is *"a multi-selection is the capability intersection, and a reset restores
//! the right source"*. Both are answers about a *set* of nodes, so the module keeps the per-node
//! facts and derives the set answers from them:
//!
//! * **The common properties are the intersection.** [`common`] answers the names every selected node
//!   can be given, so a property only one of them has is not offered as if it were shared.
//! * **A shared property is agreed or mixed, never guessed.** [`value_of`] answers [`Common::Agreed`]
//!   only when every node holds the same value, and [`Common::Mixed`] otherwise, so the panel shows a
//!   blank rather than picking one node's value to stand for the rest.
//! * **A value remembers its origin.** [`Setting`] carries the value and its [`Origin`], so resetting
//!   can tell an override from an inherited or default value instead of erasing everything.
//! * **A reset reverts to the right source.** [`reset`] takes an explicit value away and answers what
//!   is left: the inherited fallback when there is one, otherwise the default — and says so when there
//!   was nothing to reset.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use crate::app::layout_plan::Origin;
use crate::contracts::identity::NodeId;
use crate::contracts::model::{PropertyName, PropertyValue};

/// One property of one node: its value and where that value came from.
#[derive(Clone, PartialEq, Debug)]
pub struct Setting {
    /// The value.
    pub value: PropertyValue,
    /// Where it came from.
    pub origin: Origin,
}

/// One node's properties.
#[derive(Clone, PartialEq, Debug)]
pub struct NodeProperties {
    /// The node.
    pub node: NodeId,
    /// Its properties, by name.
    pub settings: BTreeMap<PropertyName, Setting>,
}

impl NodeProperties {
    /// A node with no properties.
    pub fn new(node: NodeId) -> Self {
        Self {
            node,
            settings: BTreeMap::new(),
        }
    }

    /// The same node with one more property set.
    pub fn with(mut self, name: PropertyName, value: PropertyValue, origin: Origin) -> Self {
        self.settings.insert(name, Setting { value, origin });
        self
    }

    /// What a property is set to, if it is set at all.
    pub fn setting(&self, name: &PropertyName) -> Option<&Setting> {
        self.settings.get(name)
    }
}

/// What a property is across a selection.
#[derive(Clone, PartialEq, Debug)]
pub enum Common {
    /// Every node holds the same value.
    Agreed(PropertyValue),
    /// The nodes disagree.
    Mixed,
}

/// What resetting a property did.
#[derive(Clone, PartialEq, Debug)]
pub enum Reset {
    /// The override was taken away and the value reverted to this source.
    Reverted {
        /// The value it reverted to, if any.
        value: Option<PropertyValue>,
        /// Where that value comes from.
        origin: Origin,
    },
    /// There was no override to take away.
    Nothing(&'static str),
}

impl Reset {
    /// The reset in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Reverted { value, origin } => match value {
                Some(value) => format!("reverted to {} ({})", value.family(), origin.name()),
                None => format!("cleared (no value; source {})", origin.name()),
            },
            Self::Nothing(reason) => format!("nothing to reset — {reason}"),
        }
    }
}

/// The property names every node in the selection can be given, in name order.
pub fn common(nodes: &[NodeProperties]) -> Vec<PropertyName> {
    let mut shared: Option<BTreeSet<PropertyName>> = None;
    for node in nodes {
        let names: BTreeSet<PropertyName> = node.settings.keys().cloned().collect();
        shared = Some(match shared {
            Some(existing) => existing.intersection(&names).cloned().collect(),
            None => names,
        });
    }
    shared
        .map(|set| set.into_iter().collect())
        .unwrap_or_default()
}

/// What a property is across the selection: agreed, or mixed.
///
/// A node that does not set the property at all makes the answer mixed, because "some nodes have it
/// and some do not" is not agreement.
pub fn value_of(nodes: &[NodeProperties], name: &PropertyName) -> Common {
    let mut agreed: Option<PropertyValue> = None;
    for node in nodes {
        match node.setting(name) {
            Some(setting) => match &agreed {
                Some(value) if value != &setting.value => return Common::Mixed,
                Some(_) => {}
                None => agreed = Some(setting.value.clone()),
            },
            None => return Common::Mixed,
        }
    }
    match agreed {
        Some(value) => Common::Agreed(value),
        None => Common::Mixed,
    }
}

/// Takes an explicit override away, reverting to `fallback` if there is one.
pub fn reset(origin: Origin, fallback: Option<&PropertyValue>) -> Reset {
    match origin {
        Origin::Explicit => Reset::Reverted {
            value: fallback.cloned(),
            origin: match fallback {
                Some(_) => Origin::Inherited,
                None => Origin::Default,
            },
        },
        Origin::Inherited => Reset::Nothing("the value is inherited, not overridden"),
        Origin::Default => Reset::Nothing("the value is the default"),
    }
}

#[cfg(test)]
mod tests {
    use super::{Common, NodeProperties, Reset, common, reset, value_of};
    use crate::app::layout_plan::Origin;
    use crate::contracts::identity::NodeId;
    use crate::contracts::model::{PropertyName, PropertyValue};

    fn node(value: u64) -> NodeId {
        NodeId::from_raw(value)
    }

    fn name(text: &str) -> PropertyName {
        PropertyName::new(text).expect("a name")
    }

    fn text(value: &str) -> PropertyValue {
        PropertyValue::Text(value.to_owned())
    }

    fn names(props: &[PropertyName]) -> Vec<String> {
        props.iter().map(|name| name.as_str().to_owned()).collect()
    }

    #[test]
    fn the_common_properties_are_the_intersection() {
        let a = NodeProperties::new(node(1))
            .with(name("width"), text("1"), Origin::Explicit)
            .with(name("gap"), text("2"), Origin::Explicit);
        let b = NodeProperties::new(node(2))
            .with(name("width"), text("1"), Origin::Explicit)
            .with(name("tint"), text("#000"), Origin::Explicit);
        assert_eq!(names(&common(&[a, b])), vec!["width"]);
    }

    #[test]
    fn one_node_has_all_of_its_own_properties_common() {
        let a = NodeProperties::new(node(1))
            .with(name("width"), text("1"), Origin::Explicit)
            .with(name("gap"), text("2"), Origin::Explicit);
        assert_eq!(names(&common(&[a])), vec!["gap", "width"]);
    }

    #[test]
    fn nodes_with_nothing_shared_have_no_common_properties() {
        let a = NodeProperties::new(node(1)).with(name("width"), text("1"), Origin::Explicit);
        let b = NodeProperties::new(node(2)).with(name("gap"), text("2"), Origin::Explicit);
        assert!(common(&[a, b]).is_empty());
    }

    #[test]
    fn a_shared_property_all_agree_on_is_agreed() {
        let a = NodeProperties::new(node(1)).with(name("width"), text("1"), Origin::Explicit);
        let b = NodeProperties::new(node(2)).with(name("width"), text("1"), Origin::Default);
        assert_eq!(value_of(&[a, b], &name("width")), Common::Agreed(text("1")));
    }

    #[test]
    fn a_shared_property_the_nodes_disagree_on_is_mixed() {
        let a = NodeProperties::new(node(1)).with(name("width"), text("1"), Origin::Explicit);
        let b = NodeProperties::new(node(2)).with(name("width"), text("2"), Origin::Explicit);
        assert_eq!(value_of(&[a, b], &name("width")), Common::Mixed);
    }

    #[test]
    fn a_property_only_some_nodes_set_is_mixed() {
        let a = NodeProperties::new(node(1)).with(name("width"), text("1"), Origin::Explicit);
        let b = NodeProperties::new(node(2));
        assert_eq!(value_of(&[a, b], &name("width")), Common::Mixed);
    }

    #[test]
    fn resetting_an_override_reverts_to_the_inherited_fallback() {
        assert_eq!(
            reset(Origin::Explicit, Some(&text("inherited"))),
            Reset::Reverted {
                value: Some(text("inherited")),
                origin: Origin::Inherited,
            }
        );
    }

    #[test]
    fn resetting_an_override_with_no_fallback_reverts_to_the_default() {
        assert_eq!(
            reset(Origin::Explicit, None),
            Reset::Reverted {
                value: None,
                origin: Origin::Default,
            }
        );
    }

    #[test]
    fn resetting_a_value_that_is_not_an_override_does_nothing() {
        assert!(matches!(
            reset(Origin::Inherited, Some(&text("x"))),
            Reset::Nothing(_)
        ));
        assert!(matches!(reset(Origin::Default, None), Reset::Nothing(_)));
    }

    #[test]
    fn the_reset_render_names_the_new_source() {
        let reverted = reset(Origin::Explicit, Some(&text("inherited")));
        assert!(reverted.render().contains("inherited"));
        assert!(
            reset(Origin::Default, None)
                .render()
                .contains("nothing to reset")
        );
    }
}
