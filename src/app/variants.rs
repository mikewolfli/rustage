// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Size conditions and layout variants, and the block when the target cannot express a condition
//! (task T10.07).
//!
//! The acceptance line is *"no responsive rule invented that is incompatible with the library"*. The
//! rule is therefore not the designer's to invent: a condition must be one the *target* declares it
//! supports, and anything else is refused rather than approximated:
//!
//! * **The vocabulary is the capability's.** [`Condition`] variants are exactly the kinds a target
//!   states it supports ([`Capability`]); a condition outside that set is refused, so no rule the
//!   library cannot evaluate can be written.
//! * **A missing capability blocks, and says which.** [`add`] answers a [`Block`] naming the variant
//!   and the missing [`ConditionKind`]s, so "the target cannot do responsive layout" is a reason, not
//!   a silently dropped variant.
//! * **A variant is conditions and the values they select.** A variant with no conditions is the base
//!   case, so "always" needs no special spelling.
//! * **Adding is all or nothing.** A refused variant is not half-added, so the set never holds a rule
//!   the target cannot evaluate.

#![allow(dead_code)]

use std::collections::BTreeMap;

/// The kinds of size condition a target may support.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum ConditionKind {
    /// The viewport is at least this wide.
    MinWidth,
    /// The viewport is at most this wide.
    MaxWidth,
    /// The viewport is at least this tall.
    MinHeight,
    /// The viewport is at most this tall.
    MaxHeight,
    /// The viewport is held this way.
    Orientation,
}

impl ConditionKind {
    /// Every kind, in a stable order.
    pub const ALL: [Self; 5] = [
        Self::MinWidth,
        Self::MaxWidth,
        Self::MinHeight,
        Self::MaxHeight,
        Self::Orientation,
    ];

    /// The kind's name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::MinWidth => "min-width",
            Self::MaxWidth => "max-width",
            Self::MinHeight => "min-height",
            Self::MaxHeight => "max-height",
            Self::Orientation => "orientation",
        }
    }

    /// The kind a name picks.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|kind| kind.name() == name)
    }
}

/// One size condition.
#[derive(Clone, PartialEq, Debug)]
pub enum Condition {
    /// At least this wide.
    MinWidth(f64),
    /// At most this wide.
    MaxWidth(f64),
    /// At least this tall.
    MinHeight(f64),
    /// At most this tall.
    MaxHeight(f64),
    /// Held this way.
    Orientation(crate::app::frames::Orientation),
}

impl Condition {
    /// Which kind of condition this is.
    pub const fn kind(&self) -> ConditionKind {
        match self {
            Self::MinWidth(_) => ConditionKind::MinWidth,
            Self::MaxWidth(_) => ConditionKind::MaxWidth,
            Self::MinHeight(_) => ConditionKind::MinHeight,
            Self::MaxHeight(_) => ConditionKind::MaxHeight,
            Self::Orientation(_) => ConditionKind::Orientation,
        }
    }

    /// The condition in one line.
    pub fn render(&self) -> String {
        match self {
            Self::MinWidth(value) => format!("min-width {value}"),
            Self::MaxWidth(value) => format!("max-width {value}"),
            Self::MinHeight(value) => format!("min-height {value}"),
            Self::MaxHeight(value) => format!("max-height {value}"),
            Self::Orientation(orientation) => format!("orientation {}", orientation.name()),
        }
    }
}

/// What a target declares about responsive layout.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Capability {
    /// The condition kinds it can evaluate.
    pub conditions: Vec<ConditionKind>,
}

impl Capability {
    /// A capability that supports nothing.
    pub fn new() -> Self {
        Self::default()
    }

    /// The same capability supporting one more kind.
    pub fn supporting(mut self, kind: ConditionKind) -> Self {
        if !self.conditions.contains(&kind) {
            self.conditions.push(kind);
        }
        self
    }

    /// Whether the target can evaluate a condition kind.
    pub fn supports(&self, kind: ConditionKind) -> bool {
        self.conditions.contains(&kind)
    }
}

/// One layout variant: the conditions it applies under, and the values it selects.
#[derive(Clone, PartialEq, Debug)]
pub struct Variant {
    /// Its name.
    pub name: String,
    /// The conditions that select it. An empty list is the base case.
    pub conditions: Vec<Condition>,
    /// The values it sets, by name.
    pub values: BTreeMap<String, String>,
}

impl Variant {
    /// A base variant with no conditions.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            conditions: Vec::new(),
            values: BTreeMap::new(),
        }
    }

    /// The same variant with one more condition.
    pub fn when(mut self, condition: Condition) -> Self {
        self.conditions.push(condition);
        self
    }

    /// The same variant with one more value.
    pub fn with_value(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.values.insert(key.into(), value.into());
        self
    }

    /// The variant in a report.
    pub fn render(&self) -> Vec<String> {
        let when = if self.conditions.is_empty() {
            "always".to_owned()
        } else {
            self.conditions
                .iter()
                .map(Condition::render)
                .collect::<Vec<_>>()
                .join(" and ")
        };
        let mut lines = vec![format!("variant {}: {when}", self.name)];
        for (key, value) in &self.values {
            lines.push(format!("  {key} = {value}"));
        }
        lines
    }
}

/// A variant the target cannot evaluate, and what it is missing.
#[derive(Clone, PartialEq, Debug)]
pub struct Block {
    /// The variant that was refused.
    pub variant: String,
    /// The capability kinds the target is missing.
    pub missing: Vec<ConditionKind>,
}

impl Block {
    /// The block in a report.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "variant {}: blocked, the target lacks {}",
            self.variant,
            self.missing
                .iter()
                .map(|kind| kind.name())
                .collect::<Vec<_>>()
                .join(", ")
        )];
        for kind in &self.missing {
            lines.push(format!(
                "  {} is not a supported size condition",
                kind.name()
            ));
        }
        lines
    }
}

/// Adds a variant, refusing one that needs a capability the target does not have.
pub fn add(
    capability: &Capability,
    variants: &mut Vec<Variant>,
    variant: Variant,
) -> Result<(), Block> {
    let mut missing: Vec<ConditionKind> = Vec::new();
    for condition in &variant.conditions {
        let kind = condition.kind();
        if !capability.supports(kind) && !missing.contains(&kind) {
            missing.push(kind);
        }
    }
    if !missing.is_empty() {
        return Err(Block {
            variant: variant.name.clone(),
            missing,
        });
    }
    variants.push(variant);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Block, Capability, Condition, ConditionKind, Variant, add};
    use crate::app::frames::Orientation;

    fn full() -> Capability {
        ConditionKind::ALL
            .into_iter()
            .fold(Capability::new(), Capability::supporting)
    }

    #[test]
    fn a_variant_whose_conditions_are_supported_is_added() {
        let mut variants = Vec::new();
        let variant = Variant::new("wide").when(Condition::MinWidth(600.0));
        assert_eq!(add(&full(), &mut variants, variant), Ok(()));
        assert_eq!(variants.len(), 1);
    }

    #[test]
    fn a_variant_that_needs_a_missing_capability_is_blocked() {
        let capability = Capability::new().supporting(ConditionKind::MinWidth);
        let mut variants = Vec::new();
        let variant = Variant::new("tall").when(Condition::MinHeight(800.0));
        let block = add(&capability, &mut variants, variant).expect_err("min-height is missing");
        assert_eq!(block.variant, "tall");
        assert_eq!(block.missing, vec![ConditionKind::MinHeight]);
        assert!(variants.is_empty(), "a refused variant is not half-added");
    }

    #[test]
    fn a_block_reports_every_missing_kind_once() {
        let capability = Capability::new();
        let mut variants = Vec::new();
        let variant = Variant::new("both")
            .when(Condition::Orientation(Orientation::Landscape))
            .when(Condition::MinHeight(400.0))
            .when(Condition::MaxHeight(900.0));
        let block = add(&capability, &mut variants, variant).expect_err("nothing is supported");
        assert_eq!(block.missing.len(), 3);
        assert!(block.render()[0].contains("blocked"));
    }

    #[test]
    fn the_block_type_names_the_variant_and_the_missing_capability() {
        let block = Block {
            variant: "narrow".to_owned(),
            missing: vec![ConditionKind::MaxWidth],
        };
        let rendered = block.render();
        assert!(rendered[0].contains("narrow"));
        assert!(rendered[0].contains("max-width"));
    }

    #[test]
    fn a_variant_with_no_conditions_is_the_base_case() {
        let mut variants = Vec::new();
        assert_eq!(
            add(&Capability::new(), &mut variants, Variant::new("base")),
            Ok(())
        );
        assert!(variants[0].render()[0].contains("always"));
    }

    #[test]
    fn adding_two_variants_keeps_both() {
        let mut variants = Vec::new();
        add(&full(), &mut variants, Variant::new("base")).expect("base");
        add(
            &full(),
            &mut variants,
            Variant::new("wide").when(Condition::MinWidth(600.0)),
        )
        .expect("wide");
        assert_eq!(variants.len(), 2);
    }

    #[test]
    fn every_condition_kind_name_round_trips() {
        for kind in ConditionKind::ALL {
            assert_eq!(ConditionKind::from_name(kind.name()), Some(kind));
        }
        assert!(ConditionKind::from_name("aspect").is_none());
    }

    #[test]
    fn a_condition_reports_its_own_kind() {
        assert_eq!(Condition::MinWidth(1.0).kind(), ConditionKind::MinWidth);
        assert_eq!(
            Condition::Orientation(Orientation::Portrait).kind(),
            ConditionKind::Orientation
        );
        assert!(Condition::MaxHeight(10.0).render().contains("max-height"));
    }

    #[test]
    fn a_value_renders_under_its_variant() {
        let variant = Variant::new("wide")
            .when(Condition::MinWidth(600.0))
            .with_value("gap", "16");
        let rendered = variant.render();
        assert!(rendered[0].contains("min-width 600"));
        assert!(rendered[1].contains("gap = 16"));
    }
}
