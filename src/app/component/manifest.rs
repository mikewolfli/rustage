// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The asset manifest: identity to logical path, embedded or external packaging, and the
//! derived-resource pipeline with provenance (task T12.09).
//!
//! The acceptance line is *"move/duplicate-name/missing/convert-failure recoverable; static and
//! runtime artifacts resolve the same resource identity"*.
//!
//! * **Every refusal is a [`Finding`] with a fix.** A duplicate logical name, a failed move, a
//!   missing asset and an unknown transform each carry the text that says what to do next, so a
//!   recovery is a stated step rather than a guess.
//! * **A logical name is unique, but an identity is the anchor.** [`AssetManifest::move_asset`]
//!   refuses a name another identity holds and, on success, rewrites both the identity table and
//!   the name table together, so the two never disagree.
//! * **A derived asset remembers its source.** [`AssetManifest::derive`] records the source
//!   identity and the transform that produced the derived asset, so provenance survives the
//!   pipeline and a missing source is a named [`Finding`] rather than a dangling reference.
//! * **The static artifact and the runtime lookup are one mapping read twice.** The static manifest
//!   is a serialized identity-to-path table; the runtime index is the same table in memory. Parsing
//!   the one and comparing it with the other is the assertion that an identity resolves to the same
//!   logical path in both.

#![allow(dead_code)]

use std::collections::BTreeMap;

/// A stable asset identity, independent of the logical path.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct AssetId(u64);

impl AssetId {
    /// Rebuilds an identity from the number it was written as.
    pub const fn from_raw(value: u64) -> Self {
        Self(value)
    }

    /// The number this identity is written as.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// How an asset is carried into the artifact.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Packaging {
    /// Copied into the artifact.
    Embedded,
    /// Referenced beside the artifact.
    External,
}

impl Packaging {
    /// The packaging's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Embedded => "embedded",
            Self::External => "external",
        }
    }

    /// Reads a packaging name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "embedded" => Some(Self::Embedded),
            "external" => Some(Self::External),
            _ => None,
        }
    }
}

/// One asset in the manifest.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Asset {
    /// The asset's stable identity.
    pub id: AssetId,
    /// The asset's logical path.
    pub path: String,
    /// How it is carried into the artifact.
    pub packaging: Packaging,
    /// The source identity, when this asset is derived.
    pub provenance: Option<AssetId>,
    /// The transform that produced it, when it is derived.
    pub transform: Option<String>,
}

/// A reason an asset operation did not happen, with the fix.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Finding {
    /// A logical name is already held by another identity.
    DuplicateName {
        /// The contested path.
        path: String,
        /// The identity that holds it.
        holder: AssetId,
        /// What to do.
        fix: String,
    },
    /// An identity is already used.
    DuplicateIdentity {
        /// The identity.
        asset: AssetId,
        /// What to do.
        fix: String,
    },
    /// A path is not a project-relative logical path.
    InvalidPath {
        /// The identity.
        asset: AssetId,
        /// The path.
        path: String,
        /// What to do.
        fix: String,
    },
    /// A move was refused.
    MoveFailed {
        /// The identity.
        asset: AssetId,
        /// The path it was moved to.
        path: String,
        /// Why.
        reason: String,
        /// What to do.
        fix: String,
    },
    /// An asset or a source is absent.
    Missing {
        /// The identity that is absent.
        asset: AssetId,
        /// What is missing.
        what: String,
        /// What to do.
        fix: String,
    },
    /// A transform this pipeline does not run.
    TransformFailed {
        /// The identity.
        asset: AssetId,
        /// The transform.
        transform: String,
        /// What to do.
        fix: String,
    },
}

impl Finding {
    /// The finding in one line, naming the fix.
    pub fn render(&self) -> String {
        match self {
            Self::DuplicateName { path, holder, fix } => format!(
                "logical name {path:?} is already held by asset {}; fix: {fix}",
                holder.get()
            ),
            Self::DuplicateIdentity { asset, fix } => {
                format!("asset {} already exists; fix: {fix}", asset.get())
            }
            Self::InvalidPath { asset, path, fix } => {
                format!(
                    "asset {} path {path:?} is not project-relative; fix: {fix}",
                    asset.get()
                )
            }
            Self::MoveFailed {
                asset,
                path,
                reason,
                fix,
            } => format!(
                "moving asset {} to {path:?} failed: {reason}; fix: {fix}",
                asset.get()
            ),
            Self::Missing { asset, what, fix } => {
                format!("asset {} is missing its {what}; fix: {fix}", asset.get())
            }
            Self::TransformFailed {
                asset,
                transform,
                fix,
            } => format!(
                "asset {} cannot use transform {transform:?}; fix: {fix}",
                asset.get()
            ),
        }
    }
}

/// The transforms the derived-resource pipeline runs.
pub const TRANSFORMS: [&str; 4] = ["thumbnail", "reencode", "subset", "flatten"];

/// The asset manifest: identity to asset, and logical path to identity.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct AssetManifest {
    assets: BTreeMap<AssetId, Asset>,
    by_path: BTreeMap<String, AssetId>,
}

impl AssetManifest {
    /// An empty manifest.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an asset, refusing an invalid path, a duplicate identity and a duplicate logical name.
    pub fn insert(&mut self, mut asset: Asset) -> Result<(), Finding> {
        let path = logical_path(&asset.path).ok_or_else(|| Finding::InvalidPath {
            asset: asset.id,
            path: asset.path.clone(),
            fix: "use a relative path with no .. segment".to_owned(),
        })?;
        if self.assets.contains_key(&asset.id) {
            return Err(Finding::DuplicateIdentity {
                asset: asset.id,
                fix: "choose a fresh asset identity".to_owned(),
            });
        }
        if let Some(holder) = self.by_path.get(&path) {
            return Err(Finding::DuplicateName {
                path,
                holder: *holder,
                fix: "rename one of the two assets".to_owned(),
            });
        }
        asset.path = path.clone();
        self.by_path.insert(path, asset.id);
        self.assets.insert(asset.id, asset);
        Ok(())
    }

    /// Moves an asset to a new logical path, refusing an absent identity, an invalid path and a name
    /// another identity holds.
    pub fn move_asset(&mut self, id: AssetId, new_path: &str) -> Result<(), Finding> {
        let old = self
            .assets
            .get(&id)
            .ok_or_else(|| Finding::Missing {
                asset: id,
                what: "identity".to_owned(),
                fix: "import the asset first".to_owned(),
            })?
            .path
            .clone();
        let path = logical_path(new_path).ok_or_else(|| Finding::MoveFailed {
            asset: id,
            path: new_path.to_owned(),
            reason: "not a project-relative logical path".to_owned(),
            fix: "use a relative path with no .. segment".to_owned(),
        })?;
        if path == old {
            return Ok(());
        }
        if let Some(holder) = self.by_path.get(&path) {
            return Err(Finding::DuplicateName {
                path,
                holder: *holder,
                fix: "move one of the two assets to a different name".to_owned(),
            });
        }
        self.by_path.remove(&old);
        self.by_path.insert(path.clone(), id);
        if let Some(asset) = self.assets.get_mut(&id) {
            asset.path = path;
        }
        Ok(())
    }

    /// Builds a derived asset from a source through a transform, recording both as provenance.
    pub fn derive(
        &mut self,
        source: AssetId,
        id: AssetId,
        path: &str,
        transform: &str,
        packaging: Packaging,
    ) -> Result<(), Finding> {
        if !TRANSFORMS.contains(&transform) {
            return Err(Finding::TransformFailed {
                asset: id,
                transform: transform.to_owned(),
                fix: format!("use one of {}", TRANSFORMS.join(", ")),
            });
        }
        if !self.assets.contains_key(&source) {
            return Err(Finding::Missing {
                asset: source,
                what: "source asset".to_owned(),
                fix: "restore or re-import the source asset".to_owned(),
            });
        }
        self.insert(Asset {
            id,
            path: path.to_owned(),
            packaging,
            provenance: Some(source),
            transform: Some(transform.to_owned()),
        })
    }

    /// The logical path an identity resolves to, as a runtime lookup would answer it.
    pub fn resolve(&self, id: AssetId) -> Option<&str> {
        self.assets.get(&id).map(|asset| asset.path.as_str())
    }

    /// One asset, if it exists.
    pub fn asset(&self, id: AssetId) -> Option<&Asset> {
        self.assets.get(&id)
    }

    /// The runtime lookup: identity to logical path.
    pub fn runtime_index(&self) -> BTreeMap<AssetId, String> {
        self.assets
            .iter()
            .map(|(id, asset)| (*id, asset.path.clone()))
            .collect()
    }

    /// The static artifact: a serialized identity-to-path table.
    pub fn static_manifest(&self) -> String {
        self.assets
            .iter()
            .map(|(id, asset)| format!("{}\t{}", id.get(), escape(&asset.path)))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Reads a static manifest back into an identity-to-path table.
pub fn parse_static(text: &str) -> Option<BTreeMap<AssetId, String>> {
    let mut entries = BTreeMap::new();
    if text.is_empty() {
        return Some(entries);
    }
    for line in text.lines() {
        let (id, path) = line.split_once('\t')?;
        entries.insert(AssetId::from_raw(id.parse().ok()?), unescape(path)?);
    }
    Some(entries)
}

/// A trimmed project-relative logical path, or `None` when it is not one.
fn logical_path(path: &str) -> Option<String> {
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed.starts_with('/') || trimmed.contains('\\') {
        return None;
    }
    if trimmed.split('/').any(|segment| segment == "..") {
        return None;
    }
    Some(trimmed.to_owned())
}

/// Escapes the tab and newline separators the static manifest uses.
fn escape(text: &str) -> String {
    let mut out = String::new();
    for character in text.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out
}

/// Reverses [`escape`], or `None` on a malformed sequence.
fn unescape(text: &str) -> Option<String> {
    let mut out = String::new();
    let mut characters = text.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            out.push(character);
            continue;
        }
        match characters.next()? {
            '\\' => out.push('\\'),
            'n' => out.push('\n'),
            't' => out.push('\t'),
            _ => return None,
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::{Asset, AssetId, AssetManifest, Finding, Packaging, parse_static};

    fn asset(id: u64, path: &str) -> Asset {
        Asset {
            id: AssetId::from_raw(id),
            path: path.to_owned(),
            packaging: Packaging::Embedded,
            provenance: None,
            transform: None,
        }
    }

    #[test]
    fn a_duplicate_logical_name_is_recoverable_and_names_the_fix() {
        let mut manifest = AssetManifest::new();
        manifest
            .insert(asset(1, "assets/logo.png"))
            .expect("a first");
        let finding = manifest
            .insert(asset(2, "assets/logo.png"))
            .expect_err("a duplicate");
        assert!(matches!(finding, Finding::DuplicateName { .. }));
        assert!(finding.render().contains("fix:"));
        assert!(finding.render().contains("rename"));
    }

    #[test]
    fn a_failed_move_keeps_the_original_path() {
        let mut manifest = AssetManifest::new();
        manifest
            .insert(asset(1, "assets/logo.png"))
            .expect("a first");
        manifest
            .insert(asset(2, "assets/icon.svg"))
            .expect("a second");
        let finding = manifest
            .move_asset(AssetId::from_raw(1), "assets/icon.svg")
            .expect_err("a move");
        assert!(matches!(finding, Finding::DuplicateName { .. }));
        assert_eq!(
            manifest.resolve(AssetId::from_raw(1)),
            Some("assets/logo.png")
        );

        let finding = manifest
            .move_asset(AssetId::from_raw(1), "../logo.png")
            .expect_err("a move");
        assert!(matches!(finding, Finding::MoveFailed { .. }));
        assert_eq!(
            manifest.resolve(AssetId::from_raw(1)),
            Some("assets/logo.png")
        );
    }

    #[test]
    fn a_missing_asset_reports_a_recoverable_finding() {
        let mut manifest = AssetManifest::new();
        let finding = manifest
            .move_asset(AssetId::from_raw(9), "assets/x.png")
            .expect_err("a move");
        assert!(matches!(finding, Finding::Missing { .. }));
        assert!(finding.render().contains("fix:"));
    }

    #[test]
    fn an_unknown_transform_is_recoverable_and_names_the_fix() {
        let mut manifest = AssetManifest::new();
        manifest
            .insert(asset(1, "assets/logo.png"))
            .expect("a source");
        let finding = manifest
            .derive(
                AssetId::from_raw(1),
                AssetId::from_raw(2),
                "assets/logo.thumb.png",
                "warp",
                Packaging::Embedded,
            )
            .expect_err("a transform");
        assert!(matches!(finding, Finding::TransformFailed { .. }));
        assert!(finding.render().contains("thumbnail"));
    }

    #[test]
    fn a_derived_asset_carries_its_source_identity() {
        let mut manifest = AssetManifest::new();
        manifest
            .insert(asset(1, "assets/logo.png"))
            .expect("a source");
        manifest
            .derive(
                AssetId::from_raw(1),
                AssetId::from_raw(2),
                "assets/logo.thumb.png",
                "thumbnail",
                Packaging::External,
            )
            .expect("a derive");
        let derived = manifest
            .asset(AssetId::from_raw(2))
            .expect("a derived asset");
        assert_eq!(derived.provenance, Some(AssetId::from_raw(1)));
        assert_eq!(derived.transform.as_deref(), Some("thumbnail"));
        assert_eq!(derived.packaging, Packaging::External);

        let finding = manifest
            .derive(
                AssetId::from_raw(9),
                AssetId::from_raw(3),
                "assets/other.png",
                "thumbnail",
                Packaging::Embedded,
            )
            .expect_err("a missing source");
        assert!(matches!(finding, Finding::Missing { .. }));
    }

    #[test]
    fn a_successful_move_updates_both_lookups() {
        let mut manifest = AssetManifest::new();
        manifest
            .insert(asset(1, "assets/logo.png"))
            .expect("an asset");
        manifest
            .move_asset(AssetId::from_raw(1), "assets/brand/logo.png")
            .expect("a move");
        assert_eq!(
            manifest.resolve(AssetId::from_raw(1)),
            Some("assets/brand/logo.png")
        );
        assert_eq!(
            manifest
                .runtime_index()
                .get(&AssetId::from_raw(1))
                .map(String::as_str),
            Some("assets/brand/logo.png")
        );
    }

    #[test]
    fn the_static_manifest_and_the_runtime_lookup_resolve_the_same_path() {
        let mut manifest = AssetManifest::new();
        manifest
            .insert(asset(1, "assets/logo.png"))
            .expect("an asset");
        manifest
            .insert(asset(2, "assets/icon.svg"))
            .expect("an asset");
        manifest
            .move_asset(AssetId::from_raw(2), "assets/brand/icon.svg")
            .expect("a move");

        let runtime = manifest.runtime_index();
        let static_entries = parse_static(&manifest.static_manifest()).expect("a static manifest");
        assert_eq!(static_entries, runtime);
        for (id, path) in &runtime {
            assert_eq!(manifest.resolve(*id), Some(path.as_str()));
            assert_eq!(
                static_entries.get(id).map(String::as_str),
                Some(path.as_str())
            );
        }
    }
}
