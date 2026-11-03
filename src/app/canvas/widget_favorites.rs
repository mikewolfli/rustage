// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The widget library's favorites, persisted under the host's data directory (task T09.01/T24.01).
//!
//! The catalog already knows how to lift favorites in [`crate::app::canvas::widget_catalog::Catalog::search`];
//! what it does not own is a file. This module is that file and nothing else:
//!
//! * **A missing file is "no favorites yet".** A host that has never toggled one must not report a
//!   read failure, and must not create an empty file just to prove it looked.
//! * **A file that exists and cannot be read is a failure.** A corrupt or future-version document is
//!   reported instead of being treated as empty, because "empty" would silently discard a person's
//!   list on the next save.
//! * **Saving is atomic.** [`crate::project::record::json::write_atomically`] writes a temporary file
//!   and renames it, so an interrupted save leaves the previous list intact.
//!
//! The shape is `{"version": 1, "favorites": ["button", ...]}`; the ids are contract ids, which are
//! stable across sessions and do not depend on the display name.

use std::collections::BTreeSet;
use std::path::Path;

use crate::project::record::json::{self, Json, JsonObject};

/// The favorites document this build reads and writes.
pub const FAVORITES_VERSION: u64 = 1;

/// The contract ids a person marked in the widget library.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Favorites {
    ids: BTreeSet<String>,
}

impl Favorites {
    /// No favorites.
    pub fn new() -> Self {
        Self::default()
    }

    /// The ids, in order.
    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.ids.iter().map(String::as_str)
    }

    /// Marks or unmarks one id and returns the new state.
    pub fn set(&mut self, id: &str, on: bool) -> bool {
        if on {
            self.ids.insert(id.to_owned());
        } else {
            self.ids.remove(id);
        }
        on
    }

    /// Reads the file, or answers "empty" when it does not exist.
    pub fn load(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            return Ok(Self::new());
        }
        let text = std::fs::read_to_string(path)
            .map_err(|error| format!("{} could not be read: {error}", path.display()))?;
        Self::parse(&text)
    }

    /// Writes the file atomically.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        json::write_atomically(path, &self.render())
            .map(|_| ())
            .map_err(|error| format!("{} could not be written: {error}", path.display()))
    }

    /// The document as the file writes it.
    pub fn render(&self) -> String {
        let mut root = JsonObject::new();
        root.insert("version", Json::number(FAVORITES_VERSION));
        root.insert(
            "favorites",
            Json::Array(self.ids.iter().map(|id| Json::string(id.clone())).collect()),
        );
        Json::Object(root).render()
    }

    /// Reads a document, refusing a version or shape this build cannot vouch for.
    pub fn parse(text: &str) -> Result<Self, String> {
        let value = json::parse(text).map_err(|error| {
            format!(
                "the widget favorites are not JSON: {} (line {}, column {})",
                error.kind().message(),
                error.line(),
                error.column()
            )
        })?;
        let root = value
            .as_object()
            .ok_or_else(|| "the widget favorites must be a JSON object".to_owned())?;
        let version = root
            .get("version")
            .and_then(|value| value.as_u64())
            .ok_or_else(|| "the widget favorites have no version number".to_owned())?;
        if version != FAVORITES_VERSION {
            return Err(format!(
                "the widget favorites are version {version}; this build reads version \
                 {FAVORITES_VERSION}"
            ));
        }
        let entries = root
            .get("favorites")
            .and_then(|value| value.as_array())
            .ok_or_else(|| "the widget favorites have no `favorites` array".to_owned())?;
        let mut ids = BTreeSet::new();
        for (index, entry) in entries.iter().enumerate() {
            let id = entry
                .as_str()
                .ok_or_else(|| format!("favorite {index} is not a string"))?;
            ids.insert(id.to_owned());
        }
        Ok(Self { ids })
    }
}

#[cfg(test)]
mod tests {
    use super::{FAVORITES_VERSION, Favorites};

    fn sandbox(name: &str) -> std::path::PathBuf {
        let base = std::env::temp_dir().join(format!(
            "rustage-favorites-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("a clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&base).expect("a sandbox");
        base
    }

    #[test]
    fn a_favorites_file_round_trips_and_a_missing_one_is_empty() {
        let dir = sandbox("roundtrip");
        let path = dir.join("widget-favorites.json");
        assert_eq!(
            Favorites::load(&path).expect("missing is empty"),
            Favorites::new()
        );

        let mut favorites = Favorites::new();
        favorites.set("button", true);
        favorites.set("label", true);
        favorites.set("button", false);
        favorites.save(&path).expect("saved");

        let back = Favorites::load(&path).expect("loaded");
        assert!(back.ids().any(|id| id == "label"));
        assert!(!back.ids().any(|id| id == "button"));
        assert!(
            back.render()
                .contains(&format!("\"version\": {FAVORITES_VERSION}"))
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_future_version_or_a_bad_shape_is_refused_rather_than_treated_as_empty() {
        let future = "{\"version\": 99, \"favorites\": [\"button\"]}".to_owned();
        let refusal = Favorites::parse(&future).expect_err("a future version is not read");
        assert!(refusal.contains("version 99"), "{refusal}");

        let no_array = "{\"version\": 1, \"favorites\": \"button\"}";
        let refusal = Favorites::parse(no_array).expect_err("the array is required");
        assert!(refusal.contains("favorites"), "{refusal}");

        let not_json = "{";
        assert!(Favorites::parse(not_json).is_err());
    }

    #[test]
    fn a_toggle_reports_the_state_it_left_the_id_in() {
        let mut favorites = Favorites::new();
        assert!(favorites.set("button", true));
        assert!(favorites.ids().any(|id| id == "button"));
        assert!(!favorites.set("button", false));
        assert!(!favorites.ids().any(|id| id == "button"));
    }
}
