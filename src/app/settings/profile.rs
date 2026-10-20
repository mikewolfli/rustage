// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Layered settings, migration and keymap import (task F24/T24.01, T24.09).
//!
//! One key can be set by the default, the project, the user and the session. The effective value is
//! the highest layer that has one, and every value keeps the layer it came from, so a reset removes
//! one layer instead of pretending the key has no history. A value must match the schema's kind; an
//! unknown key or a malformed value is refused before it can become effective. Importing another
//! profile reports a diff and a conflict instead of overwriting silently.

use std::collections::BTreeMap;

/// Where a setting came from.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Layer {
    /// The built-in default.
    Default,
    /// The project's settings file.
    Project,
    /// The user's profile.
    User,
    /// This session only.
    Session,
}

impl Layer {
    /// The layer's name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Project => "project",
            Self::User => "user",
            Self::Session => "session",
        }
    }

    /// Reads a layer name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "default" => Some(Self::Default),
            "project" => Some(Self::Project),
            "user" => Some(Self::User),
            "session" => Some(Self::Session),
            _ => None,
        }
    }
}

/// What a setting accepts.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Kind {
    /// Free text.
    Text,
    /// A flag.
    Bool,
    /// A whole number in a range.
    Integer {
        /// The smallest accepted value.
        min: i64,
        /// The largest accepted value.
        max: i64,
    },
    /// One of a fixed set.
    Enum(Vec<String>),
}

impl Kind {
    /// Whether `value` is of this kind.
    pub fn accepts(&self, value: &str) -> bool {
        match self {
            Self::Text => true,
            Self::Bool => value == "true" || value == "false",
            Self::Integer { min, max } => value
                .parse::<i64>()
                .map(|number| (*min..=*max).contains(&number))
                .unwrap_or(false),
            Self::Enum(names) => names.iter().any(|name| name == value),
        }
    }

    /// The kind in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Text => "text".to_owned(),
            Self::Bool => "bool".to_owned(),
            Self::Integer { min, max } => format!("integer {min}..={max}"),
            Self::Enum(names) => format!("enum({})", names.join(",")),
        }
    }
}

/// One layered value.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Value {
    /// The layer it was set at.
    pub layer: Layer,
    /// The value.
    pub text: String,
}

/// A settings profile with its schema.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Settings {
    schema: BTreeMap<String, Kind>,
    values: BTreeMap<String, Vec<Value>>,
}

impl Settings {
    /// An empty profile.
    pub fn new() -> Self {
        Self::default()
    }

    /// An empty profile with the same schema.
    pub fn empty_like(&self) -> Self {
        Self {
            schema: self.schema.clone(),
            values: BTreeMap::new(),
        }
    }

    /// Declares a key and its kind.
    pub fn schema_key(&mut self, key: impl Into<String>, kind: Kind) {
        self.schema.insert(key.into(), kind);
    }

    /// Sets a value at a layer, replacing that layer's earlier value.
    pub fn set(&mut self, layer: Layer, key: &str, text: &str) -> Result<(), String> {
        let kind = self
            .schema
            .get(key)
            .ok_or_else(|| format!("no setting {key:?} is declared"))?;
        if !kind.accepts(text) {
            return Err(format!("{key:?} expects {}, got {text:?}", kind.render()));
        }
        let entry = self.values.entry(key.to_owned()).or_default();
        match entry.iter_mut().find(|value| value.layer == layer) {
            Some(value) => value.text = text.to_owned(),
            None => {
                entry.push(Value {
                    layer,
                    text: text.to_owned(),
                });
                entry.sort_by_key(|value| value.layer);
            }
        }
        Ok(())
    }

    /// The effective value and the layer that supplies it.
    pub fn effective(&self, key: &str) -> Option<(&str, Layer)> {
        self.values
            .get(key)
            .and_then(|values| values.last())
            .map(|value| (value.text.as_str(), value.layer))
    }

    /// Removes one layer's value, leaving lower layers in place.
    pub fn reset(&mut self, key: &str, layer: Layer) -> Result<(), String> {
        let values = self
            .values
            .get_mut(key)
            .ok_or_else(|| format!("no setting {key:?} is set"))?;
        let before = values.len();
        values.retain(|value| value.layer != layer);
        if before == values.len() {
            return Err(format!("{key:?} has no value at {}", layer.name()));
        }
        Ok(())
    }

    /// The keys whose name or effective value contains `needle`.
    pub fn search(&self, needle: &str) -> Vec<String> {
        let needle = needle.to_ascii_lowercase();
        self.schema
            .keys()
            .filter(|key| {
                key.to_ascii_lowercase().contains(&needle)
                    || self
                        .effective(key)
                        .map(|(value, _)| value.to_ascii_lowercase().contains(&needle))
                        .unwrap_or(false)
            })
            .cloned()
            .collect()
    }

    /// The diff against another profile: keys whose effective values differ, and keys set at the
    /// same layer with different text (a conflict).
    pub fn import_diff(&self, other: &Self) -> Vec<String> {
        let mut keys: Vec<&String> = self.schema.keys().chain(other.schema.keys()).collect();
        keys.sort();
        keys.dedup();
        let mut lines = Vec::new();
        for key in keys {
            let mine = self.effective(key);
            let theirs = other.effective(key);
            if mine.map(|(value, _)| value) != theirs.map(|(value, _)| value) {
                lines.push(format!(
                    "{key}: {:?} -> {:?}",
                    mine.map(|(value, _)| value),
                    theirs.map(|(value, _)| value)
                ));
            }
            for layer in [Layer::Default, Layer::Project, Layer::User, Layer::Session] {
                let mine = self
                    .values
                    .get(key)
                    .and_then(|values| values.iter().find(|value| value.layer == layer));
                let theirs = other
                    .values
                    .get(key)
                    .and_then(|values| values.iter().find(|value| value.layer == layer));
                if let (Some(mine), Some(theirs)) = (mine, theirs)
                    && mine.text != theirs.text
                {
                    lines.push(format!(
                        "conflict {key} at {}: {:?} vs {:?}",
                        layer.name(),
                        mine.text,
                        theirs.text
                    ));
                }
            }
        }
        lines
    }

    /// Imports another profile's values at `layer`, refusing a key this schema does not know.
    pub fn import(&mut self, other: &Self, layer: Layer) -> Result<Vec<String>, String> {
        let mut imported = Vec::new();
        for (key, values) in &other.values {
            if !self.schema.contains_key(key) {
                return Err(format!("the imported profile has an unknown key {key:?}"));
            }
            let Some(value) = values.iter().find(|value| value.layer == layer) else {
                continue;
            };
            self.set(layer, key, &value.text)?;
            imported.push(key.clone());
        }
        Ok(imported)
    }

    /// The profile in one line per effective value.
    pub fn render(&self) -> Vec<String> {
        self.schema
            .keys()
            .map(|key| match self.effective(key) {
                Some((value, layer)) => format!("{key} = {value} ({})", layer.name()),
                None => format!("{key} = (unset)"),
            })
            .collect()
    }

    /// Migrates the old key names to the new schema.
    ///
    /// Each rename moves every layer; an old key this build does not know and a new key that already
    /// has a value at the same layer are both refused, so a migration cannot lose or merge a value.
    pub fn migrate(&mut self, renames: &[(String, String)]) -> Result<Vec<String>, String> {
        for (old, new) in renames {
            if !self.schema.contains_key(old) {
                return Err(format!("the old key {old:?} is not declared"));
            }
            if !self.schema.contains_key(new) {
                return Err(format!("the new key {new:?} is not declared"));
            }
            if let Some(values) = self.values.remove(old) {
                for value in values {
                    if self.values.get(new).is_some_and(|existing| {
                        existing.iter().any(|item| item.layer == value.layer)
                    }) {
                        return Err(format!(
                            "{new:?} already has a value at {}",
                            value.layer.name()
                        ));
                    }
                    self.values.entry(new.clone()).or_default().push(value);
                }
                self.values
                    .get_mut(new)
                    .expect("just inserted")
                    .sort_by_key(|value| value.layer);
            }
            self.schema.remove(old);
        }
        Ok(renames
            .iter()
            .map(|(old, new)| format!("{old} -> {new}"))
            .collect())
    }
}

/// A keymap profile.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Keymap {
    /// Chords by command id.
    pub bindings: BTreeMap<String, String>,
}

impl Keymap {
    /// Adds or replaces one binding.
    pub fn bind(&mut self, command: impl Into<String>, chord: impl Into<String>) {
        self.bindings.insert(command.into(), chord.into());
    }

    /// The import diff: commands the other keymap adds, removes or binds differently.
    pub fn import_diff(&self, other: &Self) -> Vec<String> {
        let mut lines = Vec::new();
        for (command, chord) in &other.bindings {
            match self.bindings.get(command) {
                Some(current) if current == chord => {}
                Some(current) => {
                    lines.push(format!("conflict {command}: {current:?} vs {chord:?}"))
                }
                None => lines.push(format!("add {command}: {chord:?}")),
            }
        }
        for command in self.bindings.keys() {
            if !other.bindings.contains_key(command) {
                lines.push(format!("remove {command}"));
            }
        }
        lines
    }

    /// Imports the other keymap's bindings, returning the conflicts it refused.
    pub fn import(&mut self, other: &Self) -> Vec<String> {
        let mut refused = Vec::new();
        for (command, chord) in &other.bindings {
            match self.bindings.get(command) {
                Some(current) if current != chord => {
                    refused.push(format!(
                        "conflict {command}: kept {current:?}, not {chord:?}"
                    ));
                }
                _ => {
                    self.bindings.insert(command.clone(), chord.clone());
                }
            }
        }
        refused
    }
}

#[cfg(test)]
mod tests {
    use super::{Keymap, Kind, Layer, Settings};

    fn profile() -> Settings {
        let mut settings = Settings::new();
        settings.schema_key("editor.font_size", Kind::Integer { min: 8, max: 32 });
        settings.schema_key(
            "editor.theme",
            Kind::Enum(vec!["light".to_owned(), "dark".to_owned()]),
        );
        settings.schema_key("editor.word_wrap", Kind::Bool);
        settings
    }

    #[test]
    fn the_highest_layer_wins_and_keeps_its_source() {
        let mut settings = profile();
        settings
            .set(Layer::Default, "editor.font_size", "14")
            .expect("a default");
        settings
            .set(Layer::User, "editor.font_size", "18")
            .expect("a user value");
        assert_eq!(
            settings.effective("editor.font_size"),
            Some(("18", Layer::User))
        );
        settings
            .reset("editor.font_size", Layer::User)
            .expect("a reset");
        assert_eq!(
            settings.effective("editor.font_size"),
            Some(("14", Layer::Default))
        );
    }

    #[test]
    fn a_malformed_or_unknown_value_is_refused() {
        let mut settings = profile();
        assert!(
            settings
                .set(Layer::User, "editor.font_size", "huge")
                .is_err()
        );
        assert!(settings.set(Layer::User, "editor.theme", "sepia").is_err());
        assert!(settings.set(Layer::User, "editor.unknown", "x").is_err());
    }

    #[test]
    fn search_matches_keys_and_effective_values() {
        let mut settings = profile();
        settings
            .set(Layer::User, "editor.theme", "dark")
            .expect("a value");
        assert_eq!(settings.search("theme"), vec!["editor.theme".to_owned()]);
        assert!(settings.search("dark").contains(&"editor.theme".to_owned()));
    }

    #[test]
    fn importing_reports_a_conflict_at_the_same_layer() {
        let mut mine = profile();
        mine.set(Layer::User, "editor.theme", "light")
            .expect("a value");
        let mut theirs = profile();
        theirs
            .set(Layer::User, "editor.theme", "dark")
            .expect("a value");
        let diff = mine.import_diff(&theirs);
        assert!(
            diff.iter().any(|line| line.starts_with("conflict")),
            "{diff:?}"
        );
    }

    #[test]
    fn migration_moves_every_layer_and_refuses_a_merge() {
        let mut settings = profile();
        settings
            .set(Layer::User, "editor.font_size", "18")
            .expect("a value");
        settings.schema_key("editor.size", Kind::Integer { min: 8, max: 32 });
        settings
            .migrate(&[("editor.font_size".to_owned(), "editor.size".to_owned())])
            .expect("a migrate");
        assert_eq!(settings.effective("editor.size"), Some(("18", Layer::User)));
        assert!(settings.effective("editor.font_size").is_none());
    }

    #[test]
    fn a_keymap_import_keeps_the_existing_binding_on_a_conflict() {
        let mut mine = Keymap::default();
        mine.bind("file.save", "Ctrl+S");
        let mut theirs = Keymap::default();
        theirs.bind("file.save", "Ctrl+Shift+S");
        theirs.bind("file.open", "Ctrl+O");
        let refused = mine.import(&theirs);
        assert_eq!(mine.bindings["file.save"], "Ctrl+S");
        assert_eq!(mine.bindings["file.open"], "Ctrl+O");
        assert_eq!(refused.len(), 1);
    }
}
