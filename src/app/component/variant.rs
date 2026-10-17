// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Variants, local overrides, detach and reset, and the target-compatibility check (task T12.03).
//!
//! The acceptance line is *"detach and reset have complete inverses"*. The design keeps that true
//! by making each operation hand back exactly the state its inverse needs and nothing more:
//!
//! * [`detach`] turns an instance into a plain copy and returns the [`Variant`] it replaced, so
//!   [`Detach::invert`] rebuilds the original exactly — the operation is pure and carries no hidden
//!   state, so the round trip is the identity rather than an approximation.
//! * [`reset`] drops an instance's local overrides so it inherits the definition again, and returns
//!   the overrides it dropped; [`Reset::invert`] reapplies precisely those overrides. Nothing else
//!   about the variant moves, so the inversion is exact.
//! * **A variant states which targets it covers.** [`target_compatibility`] refuses a target the
//!   project does not know and reports a variant that does not cover a known one, so a variant is
//!   never silently applied where it was not meant to go.
//!
//! Overrides win over the definition in [`resolve`], and [`detach`] bakes that same merge into the
//! plain copy: one merge function means the copy and the resolved instance cannot drift apart.

#![allow(dead_code)]

use std::collections::BTreeMap;

use crate::contracts::identity::NodeId;
use crate::contracts::model::{NodeKind, PropertyName, PropertyValue};

/// The output profiles a variant may be aimed at, in the order the roadmap names them.
pub const TARGETS: [&str; 5] = ["desktop", "tablet", "mobile", "mini", "embedded"];

/// Whether `target` is one of the five output profiles.
pub fn is_target(target: &str) -> bool {
    TARGETS.contains(&target)
}

/// An instance's local overrides and the targets it covers.
#[derive(Clone, PartialEq, Debug)]
pub struct Variant {
    /// The instance this variant is of.
    pub instance: NodeId,
    /// The definition the instance inherits from.
    pub definition: String,
    /// The root control kind.
    pub kind: NodeKind,
    /// The targets this variant is aimed at.
    pub targets: Vec<String>,
    /// The instance's local overrides.
    pub overrides: BTreeMap<PropertyName, PropertyValue>,
}

impl Variant {
    /// A variant with no overrides and no targets.
    pub fn new(instance: NodeId, definition: &str, kind: NodeKind) -> Self {
        Self {
            instance,
            definition: definition.to_owned(),
            kind,
            targets: Vec::new(),
            overrides: BTreeMap::new(),
        }
    }
}

/// The plain copy a detach produces: no definition link, just resolved values.
#[derive(Clone, PartialEq, Debug)]
pub struct PlainCopy {
    /// The copy's control kind.
    pub kind: NodeKind,
    /// The copy's resolved properties.
    pub properties: BTreeMap<PropertyName, PropertyValue>,
}

/// The result of a detach, carrying the state needed to invert it.
#[derive(Clone, PartialEq, Debug)]
pub struct Detach {
    /// The plain copy left behind.
    pub copy: PlainCopy,
    /// The variant the detach replaced.
    pub original: Variant,
}

impl Detach {
    /// The exact inverse of [`detach`]: the variant that was there before.
    pub fn invert(&self) -> Variant {
        self.original.clone()
    }
}

/// The result of a reset, carrying the state needed to invert it.
#[derive(Clone, PartialEq, Debug)]
pub struct Reset {
    /// The variant with its local overrides dropped.
    pub reset: Variant,
    /// The overrides the reset dropped.
    pub dropped: BTreeMap<PropertyName, PropertyValue>,
}

impl Reset {
    /// The exact inverse of [`reset`]: the overrides reapplied.
    pub fn invert(&self) -> Variant {
        let mut variant = self.reset.clone();
        variant.overrides = self.dropped.clone();
        variant
    }
}

/// The values a variant resolves to: the definition's values, then the variant's overrides.
pub fn resolve(
    variant: &Variant,
    base: &BTreeMap<PropertyName, PropertyValue>,
) -> BTreeMap<PropertyName, PropertyValue> {
    let mut properties = base.clone();
    for (name, value) in &variant.overrides {
        properties.insert(name.clone(), value.clone());
    }
    properties
}

/// Turns an instance into a plain copy, keeping the state needed to restore it.
pub fn detach(variant: &Variant, base: &BTreeMap<PropertyName, PropertyValue>) -> Detach {
    Detach {
        copy: PlainCopy {
            kind: variant.kind.clone(),
            properties: resolve(variant, base),
        },
        original: variant.clone(),
    }
}

/// Drops a variant's local overrides, keeping the state needed to restore them.
pub fn reset(variant: &Variant) -> Reset {
    let mut cleared = variant.clone();
    cleared.overrides = BTreeMap::new();
    Reset {
        reset: cleared,
        dropped: variant.overrides.clone(),
    }
}

/// Whether a variant may be applied to a target.
///
/// An unknown target and a variant that does not cover a known one are refused separately: the
/// first is a request the project cannot answer, the second is a variant that was not aimed here.
pub fn target_compatibility(variant: &Variant, target: &str) -> Result<(), String> {
    if !is_target(target) {
        return Err(format!("{target:?} is not a target"));
    }
    if !variant.targets.iter().any(|declared| declared == target) {
        return Err(format!(
            "the variant of {:?} does not cover {target}",
            variant.definition
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{TARGETS, Variant, detach, is_target, reset, resolve, target_compatibility};
    use crate::contracts::identity::NodeId;
    use crate::contracts::model::{NodeKind, PropertyName, PropertyValue};
    use std::collections::BTreeMap;

    fn name(text: &str) -> PropertyName {
        PropertyName::new(text).expect("a name")
    }

    fn base() -> BTreeMap<PropertyName, PropertyValue> {
        let mut base = BTreeMap::new();
        base.insert(name("text"), PropertyValue::Text("Ok".to_owned()));
        base.insert(name("size"), PropertyValue::Integer(3));
        base
    }

    fn variant() -> Variant {
        let mut variant = Variant::new(
            NodeId::from_raw(1),
            "button",
            NodeKind::new("button").expect("a kind"),
        );
        variant
            .overrides
            .insert(name("text"), PropertyValue::Text("Cancel".to_owned()));
        variant.targets.push("mobile".to_owned());
        variant
    }

    #[test]
    fn detaching_then_reattaching_restores_the_same_variant() {
        let variant = variant();
        let detached = detach(&variant, &base());
        assert_eq!(detached.invert(), variant);
    }

    #[test]
    fn a_detached_copy_resolves_its_overrides_into_plain_properties() {
        let variant = variant();
        let detached = detach(&variant, &base());
        assert_eq!(
            detached.copy.properties.get(&name("text")),
            Some(&PropertyValue::Text("Cancel".to_owned())),
        );
        assert_eq!(
            detached.copy.properties.get(&name("size")),
            Some(&PropertyValue::Integer(3)),
        );
        assert_eq!(detached.copy.kind.as_str(), "button");
    }

    #[test]
    fn resetting_then_reapplying_restores_the_same_overrides() {
        let variant = variant();
        let undone = reset(&variant);
        assert!(undone.reset.overrides.is_empty());
        assert_eq!(undone.invert(), variant);
    }

    #[test]
    fn a_reset_variant_inherits_the_definitions_values() {
        let variant = variant();
        let undone = reset(&variant);
        assert_eq!(resolve(&undone.reset, &base()), base());
    }

    #[test]
    fn a_variant_that_does_not_cover_a_target_is_reported() {
        let variant = variant();
        assert!(target_compatibility(&variant, "mobile").is_ok());
        let reason =
            target_compatibility(&variant, "desktop").expect_err("a target it does not cover");
        assert!(reason.contains("does not cover"), "{reason}");
    }

    #[test]
    fn an_unknown_target_is_not_a_target() {
        assert!(is_target("desktop"));
        assert!(!is_target("watch"));
        for target in TARGETS {
            assert!(is_target(target));
        }
        assert!(target_compatibility(&variant(), "watch").is_err());
    }
}
