// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Per-target resource variants and the capacity check that blocks generation (task T12.06).
//!
//! The acceptance line is *"an unsupported encoding and an over-capacity asset block generation"*.
//! The design keeps that true by returning a named [`Finding`] rather than a boolean:
//!
//! * **An unsupported encoding is not the same as an over-capacity asset.** A font declared as
//!   `png`, an image declared as `ttf` and a four-hundred-kilobyte icon are three different facts
//!   about three different requests; each gets its own [`Finding`] so a person is told which one to
//!   fix.
//! * **A missing target and an unknown target differ too.** [`Finding::MissingTarget`] is a resource
//!   that has no variant for a profile the project knows; [`Finding::UnknownTarget`] is a profile
//!   the project does not have a capacity row for at all. Conflating them would tell someone to add
//!   a variant for a profile that does not exist.
//! * **The capacity table is per kind and per profile.** [`capacity`] names one number for one kind
//!   on one profile, so "over capacity" is measured against the profile actually being built.
//!
//! Every [`Finding`] here blocks generation: a resource that cannot be encoded or that does not fit
//! is not a warning to shrug past.

#![allow(dead_code)]

/// The output profiles a resource may be built for, in the order the roadmap names them.
pub const TARGETS: [&str; 5] = ["desktop", "tablet", "mobile", "mini", "embedded"];

/// Whether `target` is one of the five output profiles.
pub fn is_target(target: &str) -> bool {
    TARGETS.contains(&target)
}

/// The kind of a resource, which decides its allowed encodings and its capacity row.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ResourceKind {
    /// A font.
    Font,
    /// An image.
    Image,
    /// An icon.
    Icon,
    /// A language table.
    Language,
}

impl ResourceKind {
    /// The kind's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Font => "font",
            Self::Image => "image",
            Self::Icon => "icon",
            Self::Language => "language",
        }
    }

    /// Reads a kind name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "font" => Some(Self::Font),
            "image" => Some(Self::Image),
            "icon" => Some(Self::Icon),
            "language" => Some(Self::Language),
            _ => None,
        }
    }

    /// The encodings this kind may be written in.
    pub fn encodings(&self) -> &'static [&'static str] {
        match self {
            Self::Font => &["woff2", "woff", "ttf"],
            Self::Image => &["png", "webp", "avif"],
            Self::Icon => &["svg", "png"],
            Self::Language => &["json", "ftl"],
        }
    }

    /// Whether an encoding belongs to this kind.
    pub fn supports(&self, encoding: &str) -> bool {
        self.encodings().contains(&encoding)
    }
}

/// The largest size, in bytes, this kind may have on a profile.
pub fn capacity(kind: ResourceKind, target: &str) -> Option<u64> {
    let row: [u64; 5] = match kind {
        ResourceKind::Font => [120_000, 120_000, 90_000, 60_000, 40_000],
        ResourceKind::Image => [2_000_000, 1_200_000, 800_000, 400_000, 200_000],
        ResourceKind::Icon => [64_000, 64_000, 48_000, 32_000, 16_000],
        ResourceKind::Language => [512_000, 512_000, 256_000, 128_000, 64_000],
    };
    TARGETS
        .iter()
        .position(|known| *known == target)
        .map(|index| row[index])
}

/// A resource's variant for one profile: an encoding and a size.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TargetVariant {
    /// The profile this variant is for.
    pub target: String,
    /// The encoding it is written in.
    pub encoding: String,
    /// Its size in bytes.
    pub bytes: u64,
}

/// One resource, with a variant for each profile it covers.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Resource {
    /// The resource's name.
    pub name: String,
    /// The resource's kind.
    pub kind: ResourceKind,
    /// The per-profile variants.
    pub variants: Vec<TargetVariant>,
}

/// A reason a resource blocks generation.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Finding {
    /// The profile is not one the project has a capacity row for.
    UnknownTarget {
        /// The resource.
        resource: String,
        /// The profile.
        target: String,
    },
    /// The resource has no variant for a known profile.
    MissingTarget {
        /// The resource.
        resource: String,
        /// The profile.
        target: String,
    },
    /// The variant's encoding is not one the kind allows.
    UnsupportedEncoding {
        /// The resource.
        resource: String,
        /// The profile.
        target: String,
        /// The encoding found.
        encoding: String,
        /// The encodings the kind allows.
        supported: Vec<String>,
    },
    /// The variant is larger than the profile's capacity.
    OverCapacity {
        /// The resource.
        resource: String,
        /// The profile.
        target: String,
        /// The variant's size.
        bytes: u64,
        /// The profile's capacity.
        capacity: u64,
    },
}

impl Finding {
    /// The finding in one line.
    pub fn render(&self) -> String {
        match self {
            Self::UnknownTarget { resource, target } => {
                format!("{resource}: {target} is not a target profile")
            }
            Self::MissingTarget { resource, target } => {
                format!("{resource}: no variant for {target}")
            }
            Self::UnsupportedEncoding {
                resource,
                target,
                encoding,
                supported,
            } => format!(
                "{resource}: encoding {encoding} is not supported for {target} (use {})",
                supported.join(", ")
            ),
            Self::OverCapacity {
                resource,
                target,
                bytes,
                capacity,
            } => {
                format!("{resource}: {bytes} bytes over the {capacity} byte capacity for {target}")
            }
        }
    }
}

/// Checks a resource for one profile, answering the first thing that blocks it.
pub fn validate(resource: &Resource, target: &str) -> Result<(), Finding> {
    if !is_target(target) {
        return Err(Finding::UnknownTarget {
            resource: resource.name.clone(),
            target: target.to_owned(),
        });
    }
    let Some(variant) = resource
        .variants
        .iter()
        .find(|variant| variant.target == target)
    else {
        return Err(Finding::MissingTarget {
            resource: resource.name.clone(),
            target: target.to_owned(),
        });
    };
    if !resource.kind.supports(&variant.encoding) {
        return Err(Finding::UnsupportedEncoding {
            resource: resource.name.clone(),
            target: target.to_owned(),
            encoding: variant.encoding.clone(),
            supported: resource
                .kind
                .encodings()
                .iter()
                .map(|encoding| encoding.to_string())
                .collect(),
        });
    }
    let found = capacity(resource.kind, target).unwrap_or_default();
    if variant.bytes > found {
        return Err(Finding::OverCapacity {
            resource: resource.name.clone(),
            target: target.to_owned(),
            bytes: variant.bytes,
            capacity: found,
        });
    }
    Ok(())
}

/// Every resource's blocking finding for a profile.
pub fn check(resources: &[Resource], target: &str) -> Vec<Finding> {
    resources
        .iter()
        .filter_map(|resource| validate(resource, target).err())
        .collect()
}

/// Whether any resource blocks generation for a profile.
pub fn blocks_generation(resources: &[Resource], target: &str) -> bool {
    !check(resources, target).is_empty()
}

#[cfg(test)]
mod tests {
    use super::{
        Finding, Resource, ResourceKind, TargetVariant, blocks_generation, capacity, check,
        validate,
    };

    fn resource(kind: ResourceKind, target: &str, encoding: &str, bytes: u64) -> Resource {
        Resource {
            name: "face".to_owned(),
            kind,
            variants: vec![TargetVariant {
                target: target.to_owned(),
                encoding: encoding.to_owned(),
                bytes,
            }],
        }
    }

    #[test]
    fn an_unsupported_encoding_blocks_generation() {
        let asset = resource(ResourceKind::Font, "desktop", "png", 10);
        let finding = validate(&asset, "desktop").expect_err("an encoding");
        assert!(matches!(finding, Finding::UnsupportedEncoding { .. }));
        assert!(finding.render().contains("not supported"));
        assert!(blocks_generation(&[asset], "desktop"));
    }

    #[test]
    fn an_over_capacity_asset_blocks_generation() {
        let asset = resource(ResourceKind::Icon, "mini", "svg", 100_000);
        let finding = validate(&asset, "mini").expect_err("a size");
        assert!(matches!(finding, Finding::OverCapacity { .. }));
        assert!(finding.render().contains("over the"));
        assert!(blocks_generation(&[asset], "mini"));
    }

    #[test]
    fn a_missing_variant_for_a_target_blocks_generation() {
        let asset = resource(ResourceKind::Image, "desktop", "png", 10);
        let finding = validate(&asset, "tablet").expect_err("a missing variant");
        assert_eq!(
            finding,
            Finding::MissingTarget {
                resource: "face".to_owned(),
                target: "tablet".to_owned(),
            }
        );
    }

    #[test]
    fn an_unknown_target_is_a_distinct_finding() {
        let asset = resource(ResourceKind::Image, "desktop", "png", 10);
        let finding = validate(&asset, "watch").expect_err("an unknown target");
        assert_eq!(
            finding,
            Finding::UnknownTarget {
                resource: "face".to_owned(),
                target: "watch".to_owned(),
            }
        );
    }

    #[test]
    fn a_supported_variant_within_capacity_passes() {
        let asset = resource(ResourceKind::Image, "desktop", "png", 100_000);
        assert!(validate(&asset, "desktop").is_ok());
        assert!(check(&[asset], "desktop").is_empty());
    }

    #[test]
    fn the_capacity_table_is_per_kind_and_target() {
        assert_eq!(capacity(ResourceKind::Font, "desktop"), Some(120_000));
        assert_eq!(capacity(ResourceKind::Icon, "desktop"), Some(64_000));
        assert!(capacity(ResourceKind::Icon, "desktop") < capacity(ResourceKind::Image, "desktop"));
        assert!(capacity(ResourceKind::Font, "mini") < capacity(ResourceKind::Font, "desktop"));
        assert_eq!(capacity(ResourceKind::Font, "watch"), None);
    }
}
