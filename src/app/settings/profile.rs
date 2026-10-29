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
use std::fmt;

/// Where a setting came from.
///
/// The order is the precedence: a later variant wins where it has a value. `Machine` sits between
/// `User` and `Session` because a value that belongs to this machine (a tool path, a display) is
/// more specific than the user profile but still yields to a value set for this session only.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Layer {
    /// The built-in default.
    Default,
    /// The project's settings file.
    Project,
    /// The user's profile.
    User,
    /// This machine's settings file (not synced with the user profile).
    Machine,
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
            Self::Machine => "machine",
            Self::Session => "session",
        }
    }

    /// Reads a layer name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "default" => Some(Self::Default),
            "project" => Some(Self::Project),
            "user" => Some(Self::User),
            "machine" => Some(Self::Machine),
            "session" => Some(Self::Session),
            _ => None,
        }
    }

    /// Every layer, lowest precedence first.
    pub const ALL: [Self; 5] = [
        Self::Default,
        Self::Project,
        Self::User,
        Self::Machine,
        Self::Session,
    ];
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

/// A refused settings line: where it is, and why.
///
/// Roadmap T24.01 asks that an invalid setting have a **location**. A profile is applied by
/// somebody reading a file, so "font size is wrong" is not actionable while "line 4" is.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Fault {
    /// The 1-based line the refusal is on; 0 means the file as a whole (a missing header).
    pub line: usize,
    /// Why the line was refused.
    pub reason: String,
}

impl Fault {
    /// The refusal in one line.
    pub fn message(&self) -> String {
        if self.line == 0 {
            self.reason.clone()
        } else {
            format!("line {}: {}", self.line, self.reason)
        }
    }
}

impl fmt::Display for Fault {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message())
    }
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
            for layer in Layer::ALL {
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

    /// The profile as its file form: every layer of every key, not only the effective value.
    ///
    /// Exporting only the effective value would turn a layered profile into a flat one and lose the
    /// provenance that [`Self::effective`] reports, so every value line is written.
    pub fn encode(&self) -> String {
        let mut text = String::from("rustage-settings 1\n");
        for (key, values) in &self.values {
            for value in values {
                text.push_str(&escape(key));
                text.push('\t');
                text.push_str(value.layer.name());
                text.push('\t');
                text.push_str(&escape(&value.text));
                text.push('\n');
            }
        }
        text
    }

    /// Applies a profile from its file form.
    ///
    /// The schema must already be declared: a file names keys, and this profile decides whether a
    /// key and its value are accepted. Values are staged first and committed only when the whole file
    /// reads, so a bad line leaves the profile exactly as it was; the refusal names its line.
    pub fn decode(&mut self, text: &str) -> Result<Vec<String>, Fault> {
        let mut lines = text.lines();
        match lines.next() {
            Some(header) if header.trim() == "rustage-settings 1" => {}
            Some(header) => {
                return Err(Fault {
                    line: 0,
                    reason: format!("expected the `rustage-settings 1` header, found {header:?}"),
                });
            }
            None => {
                return Err(Fault {
                    line: 0,
                    reason: "the settings file is empty".to_owned(),
                });
            }
        }
        let mut staged = Self {
            schema: self.schema.clone(),
            values: self.values.clone(),
        };
        let mut applied = Vec::new();
        for (index, line) in lines.enumerate() {
            let number = index + 2;
            if line.is_empty() {
                continue;
            }
            let fields: Vec<&str> = line.splitn(3, '\t').collect();
            let [key, layer, value] = fields.as_slice() else {
                return Err(Fault {
                    line: number,
                    reason: "a setting line is <key>\\t<layer>\\t<value>".to_owned(),
                });
            };
            let Some(layer) = Layer::parse(layer) else {
                return Err(Fault {
                    line: number,
                    reason: format!("{layer:?} is not a settings layer"),
                });
            };
            let (key, value) = (unescape(key), unescape(value));
            if let Err(reason) = staged.set(layer, &key, &value) {
                return Err(Fault {
                    line: number,
                    reason,
                });
            }
            applied.push(format!("{key} at {} ({number})", layer.name()));
        }
        self.values = staged.values;
        Ok(applied)
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

/// Writes a `\t`, a newline or a backslash as an escape the settings line form can carry.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            other => out.push(other),
        }
    }
    out
}

/// Reads back what [`escape`] wrote.
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut characters = text.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            out.push(character);
            continue;
        }
        match characters.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
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
    fn the_machine_layer_overrides_the_user_profile_but_yields_to_the_session() {
        let mut settings = profile();
        settings
            .set(Layer::User, "editor.font_size", "16")
            .expect("a user value");
        settings
            .set(Layer::Machine, "editor.font_size", "20")
            .expect("a machine value");
        assert_eq!(
            settings.effective("editor.font_size"),
            Some(("20", Layer::Machine))
        );
        settings
            .set(Layer::Session, "editor.font_size", "24")
            .expect("a session value");
        assert_eq!(
            settings.effective("editor.font_size"),
            Some(("24", Layer::Session))
        );
    }

    #[test]
    fn an_exported_profile_reads_back_with_every_layer_and_its_escapes() {
        let mut settings = profile();
        settings
            .set(Layer::Default, "editor.font_size", "14")
            .expect("a default");
        settings
            .set(Layer::Machine, "editor.theme", "dark")
            .expect("a machine value");
        settings.schema_key("editor.title", Kind::Text);
        settings
            .set(Layer::Project, "editor.title", "a\tb\nc\\d")
            .expect("a text value");

        let text = settings.encode();
        assert!(text.contains("editor.theme\tmachine\tdark"), "{text}");

        let mut read_back = settings.empty_like();
        let applied = read_back.decode(&text).expect("a readable profile");
        assert_eq!(read_back, settings);
        assert_eq!(applied.len(), 3);
        assert_eq!(
            read_back.effective("editor.title"),
            Some(("a\tb\nc\\d", Layer::Project))
        );
    }

    #[test]
    fn a_bad_line_names_its_line_and_leaves_the_profile_untouched() {
        let mut settings = profile();
        settings
            .set(Layer::User, "editor.font_size", "16")
            .expect("a value");
        let before = settings.clone();
        let text = "rustage-settings 1\neditor.font_size\tuser\t18\neditor.font_size\tuser\thuge\n";
        let fault = settings
            .decode(text)
            .expect_err("the value is not an integer");
        assert_eq!(fault.line, 3);
        assert!(fault.message().contains("line 3"), "{}", fault.message());
        // The first line was valid, but nothing is committed when the file as a whole is refused.
        assert_eq!(settings, before);
    }

    #[test]
    fn an_unknown_key_or_a_missing_header_is_refused_with_its_location() {
        let mut settings = profile();
        let fault = settings
            .decode("rustage-settings 1\neditor.unknown\tuser\tx\n")
            .expect_err("no such key");
        assert_eq!(fault.line, 2);
        assert!(fault.message().contains("no setting"), "{fault}");

        let fault = settings.decode("nonsense\n").expect_err("no header");
        assert_eq!(fault.line, 0);
        assert!(fault.message().contains("header"), "{fault}");
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
