// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Resource import, the reference index, and the transactional replace and move that keep the
//! original file and its references on failure (task T12.05).
//!
//! The acceptance line is *"a failed replace/move keeps the original file and references"*. The
//! design makes that structural rather than a promise:
//!
//! * **Every refusal happens before any write.** [`Library::replace`] and [`Library::move_asset`]
//!   validate the new path and the capacity first and return `Err` while the library still holds the
//!   original [`Asset`]; the only method that mutates the path or the reference key is the private
//!   [`Library::rename`], which is reached only after all checks pass.
//! * **References are keyed by logical path.** Renaming moves the whole reference set in one step,
//!   so a successful move cannot lose a referrer, and a failed move cannot split the set: the key
//!   is either the old path with its nodes or the new one with the same nodes.
//! * **A missing asset is a named fact.** A reference to a path with no [`Asset`] shows up in
//!   [`Library::missing`], and [`Library::repoint`] moves those references onto a real asset,
//!   refusing to re-point a path that already resolves or onto a path that does not.
//!
//! Paths are validated at the edge: an absolute path, a `..` segment or a backslash is not a
//! project-relative logical path and is refused before an asset is built around it.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use crate::contracts::identity::NodeId;

/// A stable resource identity, independent of the file's logical path.
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

/// The kind of a resource, as the target variants care about it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AssetKind {
    /// A font.
    Font,
    /// An image.
    Image,
    /// An icon.
    Icon,
    /// A language table.
    Language,
}

impl AssetKind {
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
}

/// One imported resource: a stable identity, a logical path and a size against a capacity.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Asset {
    /// The resource's stable identity.
    pub id: AssetId,
    /// The resource's project-relative logical path.
    pub path: String,
    /// The resource's kind.
    pub kind: AssetKind,
    /// The largest size the project allows for this resource.
    pub capacity: u64,
    /// The resource's current size.
    pub bytes: u64,
}

/// A reference to a path that resolves to no asset.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Missing {
    /// The path nothing resolves to.
    pub path: String,
    /// The nodes that still reference it.
    pub referrers: Vec<NodeId>,
}

/// The imported assets and the index of which nodes reference which path.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Library {
    assets: BTreeMap<AssetId, Asset>,
    references: BTreeMap<String, BTreeSet<NodeId>>,
}

impl Library {
    /// An empty library.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an asset, refusing an unsafe path or one already held by another identity.
    pub fn add(
        &mut self,
        id: AssetId,
        path: &str,
        kind: AssetKind,
        capacity: u64,
        bytes: u64,
    ) -> Result<(), String> {
        let path = logical_path(path)
            .ok_or_else(|| format!("{path:?} is not a project-relative logical path"))?;
        if self.assets.contains_key(&id) {
            return Err(format!("asset {} already exists", id.get()));
        }
        if self.assets.values().any(|asset| asset.path == path) {
            return Err(format!("path {path:?} is already an asset"));
        }
        self.assets.insert(
            id,
            Asset {
                id,
                path: path.clone(),
                kind,
                capacity,
                bytes,
            },
        );
        self.references.entry(path).or_default();
        Ok(())
    }

    /// Records that a node references a logical path.
    pub fn reference(&mut self, path: &str, node: NodeId) {
        self.references
            .entry(path.to_owned())
            .or_default()
            .insert(node);
    }

    /// Removes one node's reference to a logical path.
    pub fn unreference(&mut self, path: &str, node: NodeId) {
        if let Some(nodes) = self.references.get_mut(path) {
            nodes.remove(&node);
        }
    }

    /// One asset, if it exists.
    pub fn asset(&self, id: AssetId) -> Option<&Asset> {
        self.assets.get(&id)
    }

    /// The assets, in identity order.
    pub fn assets(&self) -> Vec<&Asset> {
        self.assets.values().collect()
    }

    /// One asset's logical path, if it exists.
    pub fn path_of(&self, id: AssetId) -> Option<&str> {
        self.assets.get(&id).map(|asset| asset.path.as_str())
    }

    /// The nodes that reference an asset, if the asset exists.
    pub fn references_of(&self, id: AssetId) -> Option<Vec<NodeId>> {
        let asset = self.assets.get(&id)?;
        Some(
            self.references
                .get(&asset.path)
                .map(|nodes| nodes.iter().copied().collect())
                .unwrap_or_default(),
        )
    }

    /// Replaces an asset's file, refusing an unsafe path, a path already taken by another asset and
    /// a size over the asset's capacity.
    ///
    /// On any refusal the asset keeps its original path and its reference set unchanged.
    pub fn replace(&mut self, id: AssetId, new_path: &str, new_bytes: u64) -> Result<(), String> {
        let path = logical_path(new_path)
            .ok_or_else(|| format!("{new_path:?} is not a project-relative logical path"))?;
        let asset = self
            .assets
            .get(&id)
            .ok_or_else(|| format!("no asset {}", id.get()))?
            .clone();
        if new_bytes > asset.capacity {
            return Err(format!(
                "asset {} would be {new_bytes} bytes, over its {} byte capacity",
                id.get(),
                asset.capacity
            ));
        }
        if path != asset.path
            && self
                .assets
                .values()
                .any(|other| other.id != id && other.path == path)
        {
            return Err(format!("path {path:?} is already another asset's path"));
        }
        self.rename(id, &asset.path, &path);
        if let Some(asset) = self.assets.get_mut(&id) {
            asset.bytes = new_bytes;
        }
        Ok(())
    }

    /// Moves an asset to a new logical path, refusing an unsafe path and a path already taken.
    ///
    /// On any refusal the asset keeps its original path and its reference set unchanged.
    pub fn move_asset(&mut self, id: AssetId, new_path: &str) -> Result<(), String> {
        let path = logical_path(new_path)
            .ok_or_else(|| format!("{new_path:?} is not a project-relative logical path"))?;
        let asset = self
            .assets
            .get(&id)
            .ok_or_else(|| format!("no asset {}", id.get()))?
            .clone();
        if path != asset.path
            && self
                .assets
                .values()
                .any(|other| other.id != id && other.path == path)
        {
            return Err(format!("path {path:?} is already another asset's path"));
        }
        self.rename(id, &asset.path, &path);
        Ok(())
    }

    /// The paths that are referenced but resolve to no asset.
    pub fn missing(&self) -> Vec<Missing> {
        let known: BTreeSet<&str> = self
            .assets
            .values()
            .map(|asset| asset.path.as_str())
            .collect();
        self.references
            .iter()
            .filter(|(path, nodes)| !nodes.is_empty() && !known.contains(path.as_str()))
            .map(|(path, nodes)| Missing {
                path: path.clone(),
                referrers: nodes.iter().copied().collect(),
            })
            .collect()
    }

    /// Re-points every reference from a missing path onto a real asset's path, answering how many
    /// references moved.
    pub fn repoint(&mut self, from: &str, to: &str) -> Result<usize, String> {
        if self.assets.values().any(|asset| asset.path == from) {
            return Err(format!("{from:?} already resolves to an asset"));
        }
        if !self.assets.values().any(|asset| asset.path == to) {
            return Err(format!("{to:?} is not a known asset path"));
        }
        let moving = self.references.remove(from).unwrap_or_default();
        let count = moving.len();
        self.references
            .entry(to.to_owned())
            .or_default()
            .extend(moving);
        Ok(count)
    }

    /// Moves an asset's reference set and path in one step. Reached only after every check passes.
    fn rename(&mut self, id: AssetId, from: &str, to: &str) {
        if from == to {
            return;
        }
        let moving = self.references.remove(from).unwrap_or_default();
        self.references
            .entry(to.to_owned())
            .or_default()
            .extend(moving);
        if let Some(asset) = self.assets.get_mut(&id) {
            asset.path = to.to_owned();
        }
    }
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

#[cfg(test)]
mod tests {
    use super::{AssetId, AssetKind, Library};
    use crate::contracts::identity::NodeId;

    fn library() -> Library {
        let mut library = Library::new();
        library
            .add(
                AssetId::from_raw(1),
                "assets/logo.png",
                AssetKind::Image,
                100,
                40,
            )
            .expect("an asset");
        library.reference("assets/logo.png", NodeId::from_raw(7));
        library
    }

    #[test]
    fn a_blank_or_traversing_path_is_refused() {
        let mut library = Library::new();
        assert!(
            library
                .add(AssetId::from_raw(1), "  ", AssetKind::Image, 10, 1)
                .is_err()
        );
        assert!(
            library
                .add(AssetId::from_raw(1), "/etc/passwd", AssetKind::Image, 10, 1)
                .is_err()
        );
        assert!(
            library
                .add(
                    AssetId::from_raw(1),
                    "assets/../secret",
                    AssetKind::Image,
                    10,
                    1
                )
                .is_err()
        );
    }

    #[test]
    fn a_duplicate_identity_or_path_is_refused() {
        let mut library = library();
        assert!(
            library
                .add(
                    AssetId::from_raw(1),
                    "assets/other.png",
                    AssetKind::Image,
                    10,
                    1
                )
                .is_err()
        );
        assert!(
            library
                .add(
                    AssetId::from_raw(2),
                    "assets/logo.png",
                    AssetKind::Image,
                    10,
                    1
                )
                .is_err()
        );
    }

    #[test]
    fn a_replace_over_capacity_keeps_the_original_path_and_references() {
        let mut library = library();
        assert!(
            library
                .replace(AssetId::from_raw(1), "assets/logo.png", 200)
                .is_err()
        );
        assert_eq!(
            library.path_of(AssetId::from_raw(1)),
            Some("assets/logo.png")
        );
        assert_eq!(
            library.references_of(AssetId::from_raw(1)),
            Some(vec![NodeId::from_raw(7)])
        );
        assert_eq!(
            library.asset(AssetId::from_raw(1)).expect("an asset").bytes,
            40
        );
    }

    #[test]
    fn a_replace_onto_a_taken_path_keeps_the_original_path_and_references() {
        let mut library = library();
        library
            .add(
                AssetId::from_raw(2),
                "assets/icon.svg",
                AssetKind::Icon,
                100,
                10,
            )
            .expect("a second asset");
        assert!(
            library
                .replace(AssetId::from_raw(1), "assets/icon.svg", 50)
                .is_err()
        );
        assert_eq!(
            library.path_of(AssetId::from_raw(1)),
            Some("assets/logo.png")
        );
        assert_eq!(
            library.references_of(AssetId::from_raw(1)),
            Some(vec![NodeId::from_raw(7)])
        );
    }

    #[test]
    fn a_failed_move_keeps_the_original_path_and_references() {
        let mut library = library();
        library
            .add(
                AssetId::from_raw(2),
                "assets/icon.svg",
                AssetKind::Icon,
                100,
                10,
            )
            .expect("a second asset");
        assert!(
            library
                .move_asset(AssetId::from_raw(1), "assets/icon.svg")
                .is_err()
        );
        assert!(
            library
                .move_asset(AssetId::from_raw(1), "../logo.png")
                .is_err()
        );
        assert_eq!(
            library.path_of(AssetId::from_raw(1)),
            Some("assets/logo.png")
        );
        assert_eq!(
            library.references_of(AssetId::from_raw(1)),
            Some(vec![NodeId::from_raw(7)])
        );
    }

    #[test]
    fn a_successful_move_keeps_the_references() {
        let mut library = library();
        library
            .move_asset(AssetId::from_raw(1), "assets/brand/logo.png")
            .expect("a move");
        assert_eq!(
            library.path_of(AssetId::from_raw(1)),
            Some("assets/brand/logo.png")
        );
        assert_eq!(
            library.references_of(AssetId::from_raw(1)),
            Some(vec![NodeId::from_raw(7)])
        );
        assert!(library.missing().is_empty());
    }

    #[test]
    fn a_missing_asset_can_be_repointed() {
        let mut library = library();
        library.reference("assets/gone.png", NodeId::from_raw(9));
        let missing = library.missing();
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].path, "assets/gone.png");
        assert_eq!(missing[0].referrers, vec![NodeId::from_raw(9)]);

        let moved = library
            .repoint("assets/gone.png", "assets/logo.png")
            .expect("a re-point");
        assert_eq!(moved, 1);
        assert!(library.missing().is_empty());
        assert_eq!(
            library.references_of(AssetId::from_raw(1)),
            Some(vec![NodeId::from_raw(7), NodeId::from_raw(9)])
        );
    }

    #[test]
    fn repointing_a_resolved_path_or_onto_an_unknown_one_is_refused() {
        let mut library = library();
        assert!(
            library
                .repoint("assets/logo.png", "assets/logo.png")
                .is_err()
        );
        assert!(
            library
                .repoint("assets/gone.png", "assets/nope.png")
                .is_err()
        );
    }
}
