// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Keyboard shortcuts: scopes, chording, conflict detection and user overrides (task T05.03).
//!
//! The acceptance line is *"typing, including an input method, does not misfire a global command"*.
//! That is a rule about the **keyboard's owner**, and it is the first thing this module decides:
//!
//! * **A bare printable key belongs to the text.** When a text field has the keyboard, a chord that is
//!   one character with no `Ctrl`/`Alt`/`Cmd` is [`Resolution::Suppressed`] — it goes to the field,
//!   whatever a global binding says. `Ctrl+S` still saves, because a modifier is how a person says "I
//!   mean the command, not the letter". An input-method composition suppresses everything, because the
//!   keys in it are the IME's, not the host's.
//! * **A chord is a sequence, and a sequence is pending until it finishes.** `Ctrl+K Ctrl+S` resolves
//!   to [`Resolution::Pending`] after the first key and to a command after the second; a key that
//!   cannot continue any chord drops the pending sequence and is resolved on its own, so a mis-typed
//!   chord does not swallow the next command.
//! * **A conflict is reported, not silently ordered.** Two bindings for the same chord whose scopes can
//!   both be active are a [`Conflict`]; [`Keymap::add`] refuses it and names both commands. The one
//!   place scopes may deliberately overlap is where one is `Global` and the other is narrower — there
//!   the narrower one wins, and that is stated rather than discovered.
//! * **Overrides are data, and they round trip.** [`Keymap::export`] writes `chord = command` lines and
//!   [`Keymap::import`] reads them back, so a person can see, edit and move their keymap without the
//!   format being a secret. An override that does not parse is refused with the line, not dropped.

// The keymap's editor and its import/export are a person's keymap file (T05.03's later half) and
// are exercised by the tests until a settings surface (T26) reads them; the silence is stated once
// here with its reason, as the contracts and project trees do.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::fmt;

/// The modifiers a chord may carry.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
pub struct Modifiers {
    ctrl: bool,
    alt: bool,
    shift: bool,
    meta: bool,
}

impl Modifiers {
    /// No modifiers.
    pub const NONE: Self = Self {
        ctrl: false,
        alt: false,
        shift: false,
        meta: false,
    };

    /// `Ctrl`.
    pub const fn ctrl() -> Self {
        Self {
            ctrl: true,
            ..Self::NONE
        }
    }

    /// With `Shift` added.
    pub const fn and_shift(self) -> Self {
        Self {
            shift: true,
            ..self
        }
    }

    /// Whether any command modifier (`Ctrl`/`Alt`/`Cmd`) is held.
    ///
    /// `Shift` alone does not count: `Shift+A` types a capital A, it does not mean a command.
    pub const fn has_command_modifier(self) -> bool {
        self.ctrl || self.alt || self.meta
    }

    /// The modifiers as they are written in a keymap.
    pub fn render(self) -> String {
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("Ctrl");
        }
        if self.alt {
            parts.push("Alt");
        }
        if self.shift {
            parts.push("Shift");
        }
        if self.meta {
            parts.push("Cmd");
        }
        parts.join("+")
    }

    /// Parses a `+`-separated modifier list, refusing an unknown one.
    fn parse(text: &str) -> Result<Self, String> {
        let mut modifiers = Self::NONE;
        for part in text.split('+') {
            match part {
                "Ctrl" | "ctrl" => modifiers.ctrl = true,
                "Alt" | "alt" => modifiers.alt = true,
                "Shift" | "shift" => modifiers.shift = true,
                "Cmd" | "Meta" | "cmd" => modifiers.meta = true,
                other => return Err(format!("{other:?} is not a modifier")),
            }
        }
        Ok(modifiers)
    }
}

/// A key without modifiers.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum KeyCode {
    /// A character key.
    Char(char),
    /// A named key.
    Named(NamedKey),
}

/// The keys a keymap may name that are not characters.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum NamedKey {
    /// Return.
    Enter,
    /// Escape.
    Escape,
    /// Tab.
    Tab,
    /// Backspace.
    Backspace,
    /// Delete.
    Delete,
    /// The space bar.
    Space,
    /// The left arrow.
    Left,
    /// The right arrow.
    Right,
    /// The up arrow.
    Up,
    /// The down arrow.
    Down,
}

impl NamedKey {
    /// The name a keymap uses.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Enter => "Enter",
            Self::Escape => "Escape",
            Self::Tab => "Tab",
            Self::Backspace => "Backspace",
            Self::Delete => "Delete",
            Self::Space => "Space",
            Self::Left => "Left",
            Self::Right => "Right",
            Self::Up => "Up",
            Self::Down => "Down",
        }
    }

    /// The named key a name asks for, when it is one.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|key| key.name() == name)
    }

    /// Every named key.
    pub const ALL: [Self; 10] = [
        Self::Enter,
        Self::Escape,
        Self::Tab,
        Self::Backspace,
        Self::Delete,
        Self::Space,
        Self::Left,
        Self::Right,
        Self::Up,
        Self::Down,
    ];
}

/// One keypress: what was held, and what was pressed.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Key {
    modifiers: Modifiers,
    code: KeyCode,
}

impl Key {
    /// A key.
    pub const fn new(modifiers: Modifiers, code: KeyCode) -> Self {
        Self { modifiers, code }
    }

    /// A character with no modifiers.
    pub const fn character(character: char) -> Self {
        Self::new(Modifiers::NONE, KeyCode::Char(character))
    }

    /// The modifiers held.
    pub const fn modifiers(&self) -> Modifiers {
        self.modifiers
    }

    /// The key itself.
    pub const fn code(&self) -> KeyCode {
        self.code
    }

    /// Whether this key is one a text field would want: a single character with no command modifier.
    ///
    /// `Shift+A` counts — it is a capital A, not a command — which is why the test is about the
    /// command modifiers rather than about the whole modifier set.
    pub const fn is_text(&self) -> bool {
        matches!(self.code, KeyCode::Char(_)) && !self.modifiers.has_command_modifier()
    }

    /// Parses a key such as `Ctrl+S`, `Escape` or `a`.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut modifiers = Modifiers::NONE;
        let mut name = text;
        // Split off a modifier prefix: every `+` before the last segment is a modifier.
        if let Some((head, tail)) = text.rsplit_once('+') {
            modifiers = Modifiers::parse(head)?;
            name = tail;
        }
        let code = if let Some(named) = NamedKey::from_name(name) {
            KeyCode::Named(named)
        } else {
            let mut characters = name.chars();
            match (characters.next(), characters.next()) {
                (Some(character), None) => KeyCode::Char(character),
                _ => return Err(format!("{name:?} is not a key")),
            }
        };
        Ok(Self { modifiers, code })
    }
}

impl fmt::Display for Key {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let modifiers = self.modifiers.render();
        let name = match self.code {
            KeyCode::Char(character) => character.to_string(),
            KeyCode::Named(named) => named.name().to_owned(),
        };
        if modifiers.is_empty() {
            write!(formatter, "{name}")
        } else {
            write!(formatter, "{modifiers}+{name}")
        }
    }
}

/// A chord: one key, or two in sequence.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Chord(Vec<Key>);

impl Chord {
    /// A single key chord.
    pub fn key(key: Key) -> Self {
        Self(vec![key])
    }

    /// A two-key chord.
    pub fn sequence(first: Key, second: Key) -> Self {
        Self(vec![first, second])
    }

    /// Parses `Ctrl+K Ctrl+S` (or a single key).
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut keys = Vec::new();
        for part in text.split_whitespace() {
            keys.push(Key::parse(part)?);
        }
        match keys.len() {
            1 | 2 => Ok(Self(keys)),
            other => Err(format!(
                "a chord is one or two keys, and this one has {other}"
            )),
        }
    }

    /// The keys in order.
    pub fn keys(&self) -> &[Key] {
        &self.0
    }

    /// Whether the chord begins with the keys pressed so far.
    pub fn starts_with(&self, pressed: &[Key]) -> bool {
        self.0.len() >= pressed.len() && self.0[..pressed.len()] == *pressed
    }

    /// Whether the chord is exactly these keys.
    pub fn matches(&self, pressed: &[Key]) -> bool {
        self.0 == pressed
    }

    /// Whether the chord is a single bare text key.
    pub fn is_text(&self) -> bool {
        self.0.len() == 1 && self.0[0].is_text()
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let parts: Vec<String> = self.0.iter().map(Key::to_string).collect();
        write!(formatter, "{}", parts.join(" "))
    }
}

/// Where a binding applies.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Scope {
    /// Everywhere.
    Global,
    /// In one mode.
    Mode(String),
    /// In one panel.
    Panel(String),
    /// Only while a text field has the keyboard (a field's own keys).
    TextField,
}

impl Scope {
    /// How specific the scope is; a narrower scope wins over a wider one.
    pub fn specificity(&self) -> u8 {
        match self {
            Self::Global => 0,
            Self::Mode(_) => 1,
            Self::Panel(_) => 2,
            Self::TextField => 3,
        }
    }

    /// The scope as it is written in a keymap (`global`, `mode:design`, `panel:outline`, `text`).
    pub fn render(&self) -> String {
        match self {
            Self::Global => "global".to_owned(),
            Self::Mode(name) => format!("mode:{name}"),
            Self::Panel(name) => format!("panel:{name}"),
            Self::TextField => "text".to_owned(),
        }
    }

    /// Parses a scope.
    pub fn parse(text: &str) -> Result<Self, String> {
        match text.split_once(':') {
            Some(("mode", name)) if !name.is_empty() => Ok(Self::Mode(name.to_owned())),
            Some(("panel", name)) if !name.is_empty() => Ok(Self::Panel(name.to_owned())),
            _ => match text {
                "global" => Ok(Self::Global),
                "text" => Ok(Self::TextField),
                other => Err(format!("{other:?} is not a scope")),
            },
        }
    }
}

/// What the host looks like when the keyboard is used.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Scene {
    /// The mode the designer is in.
    mode: String,
    /// The focused panel, when one is.
    panel: Option<String>,
    /// Whether a text field has the keyboard.
    text_field: bool,
    /// Whether an input method is composing.
    composing: bool,
}

impl Scene {
    /// A scene in a mode with a panel focused.
    pub fn in_mode(mode: impl Into<String>, panel: Option<String>) -> Self {
        Self {
            mode: mode.into(),
            panel,
            text_field: false,
            composing: false,
        }
    }

    /// A scene where a text field has the keyboard.
    pub fn text_field(mode: impl Into<String>) -> Self {
        Self {
            mode: mode.into(),
            panel: None,
            text_field: true,
            composing: false,
        }
    }

    /// The same scene while an input method composes.
    pub fn composing(mut self) -> Self {
        self.composing = true;
        self
    }

    /// Whether a scope is active in this scene.
    ///
    /// `Global` is active everywhere, a text field included: `Ctrl+S` means "save" while somebody is
    /// typing. What a text field takes away is the *bare* keys, and that is decided in [`Keymap::resolve`]
    /// by the key itself rather than by hiding the global scope.
    pub fn admits(&self, scope: &Scope) -> bool {
        match scope {
            Scope::Global => true,
            Scope::Mode(name) => name == &self.mode,
            Scope::Panel(name) => self.panel.as_deref() == Some(name.as_str()),
            Scope::TextField => self.text_field,
        }
    }
}

/// One binding: a chord that runs a command in a scope.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Binding {
    chord: Chord,
    command: String,
    scope: Scope,
}

impl Binding {
    /// A binding.
    pub fn new(chord: Chord, command: impl Into<String>, scope: Scope) -> Self {
        Self {
            chord,
            command: command.into(),
            scope,
        }
    }

    /// The chord.
    pub fn chord(&self) -> &Chord {
        &self.chord
    }

    /// The command it runs.
    pub fn command(&self) -> &str {
        &self.command
    }

    /// The scope it applies in.
    pub fn scope(&self) -> &Scope {
        &self.scope
    }
}

/// Two bindings that could both fire.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Conflict {
    chord: Chord,
    first: String,
    first_scope: Scope,
    second: String,
    second_scope: Scope,
}

impl Conflict {
    /// The chord both bindings use.
    pub fn chord(&self) -> &Chord {
        &self.chord
    }

    /// The conflict in one sentence.
    pub fn message(&self) -> String {
        format!(
            "{} is bound to {} in {} and to {} in {}",
            self.chord,
            self.first,
            self.first_scope.render(),
            self.second,
            self.second_scope.render(),
        )
    }
}

/// What pressing keys did.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Resolution {
    /// A command runs.
    Triggered {
        /// The command.
        command: String,
    },
    /// The keys are the start of a chord; more are expected.
    Pending {
        /// The keys pressed so far.
        pressed: Vec<Key>,
        /// The commands that could still fire, in order.
        candidates: Vec<String>,
    },
    /// The keys belong to a text field, so no command runs.
    Suppressed {
        /// Why.
        reason: String,
    },
    /// Nothing is bound to these keys.
    Nothing,
}

/// A user's change to the keymap, and the file format they round trip through.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Override {
    /// Bind a chord, replacing any binding for the same chord in the same scope.
    Bind {
        /// The chord.
        chord: Chord,
        /// The command.
        command: String,
        /// The scope.
        scope: Scope,
    },
    /// Remove every binding for a chord.
    Unbind {
        /// The chord.
        chord: Chord,
    },
}

/// A keymap: the bindings, their conflicts, and the one resolver.
#[derive(Clone, Debug, Default)]
pub struct Keymap {
    bindings: Vec<Binding>,
    overrides: Vec<Override>,
}

impl Keymap {
    /// An empty keymap.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a binding, refusing one that would conflict with an existing binding.
    ///
    /// A `Global` binding and a narrower one for the same chord are **allowed**: the narrower wins, and
    /// that is the one deliberate overlap. Two bindings of the same specificity for one chord are not.
    /// The conflict is boxed because it is only built when something went wrong and is immediately
    /// turned into a message, so carrying 136 bytes in every `Ok` is the wrong trade.
    pub fn add(&mut self, binding: Binding) -> Result<(), Box<Conflict>> {
        for existing in &self.bindings {
            if existing.chord != binding.chord {
                continue;
            }
            let overlapping = match (&existing.scope, &binding.scope) {
                (Scope::Global, Scope::Global) => true,
                (Scope::Global, _) | (_, Scope::Global) => false,
                (a, b) => a == b,
            };
            if overlapping {
                return Err(Box::new(Conflict {
                    chord: binding.chord.clone(),
                    first: existing.command.clone(),
                    first_scope: existing.scope.clone(),
                    second: binding.command.clone(),
                    second_scope: binding.scope.clone(),
                }));
            }
        }
        self.bindings.push(binding);
        Ok(())
    }

    /// How many bindings are in the map.
    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    /// Whether it is empty.
    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    /// Applies a user override.
    pub fn apply(&mut self, rule: Override) {
        match &rule {
            Override::Bind { chord, .. } | Override::Unbind { chord } => {
                self.bindings.retain(|binding| &binding.chord != chord);
            }
        }
        if let Override::Bind {
            chord,
            command,
            scope,
        } = &rule
        {
            self.bindings
                .push(Binding::new(chord.clone(), command.clone(), scope.clone()));
        }
        // The applied rules are kept so an export writes them, not the merged result: what a person
        // wrote is what they should see back.
        self.overrides.push(rule);
    }

    /// The user overrides that have been applied, in order.
    pub fn overrides(&self) -> &[Override] {
        &self.overrides
    }

    /// Resolves a key sequence in a scene.
    ///
    /// The order of the three questions is the acceptance line: first, does this key belong to a text
    /// field; then, is it the start of a chord; then, does a binding match.
    pub fn resolve(&self, pressed: &[Key], scene: &Scene) -> Resolution {
        if pressed.is_empty() {
            return Resolution::Nothing;
        }
        // An input method's keys are the IME's, not the host's.
        if scene.composing {
            return Resolution::Suppressed {
                reason: "an input method is composing, so the keys are its own".to_owned(),
            };
        }
        // A single bare printable key belongs to the text field, whatever is bound to it.
        if pressed.len() == 1 && pressed[0].is_text() {
            return Resolution::Suppressed {
                reason: format!(
                    "`{}` is a typed character, so it goes to the text field rather than to a command",
                    pressed[0]
                ),
            };
        }

        let pending: Vec<String> = self
            .bindings
            .iter()
            .filter(|binding| binding.chord.starts_with(pressed) && scene.admits(&binding.scope))
            .map(|binding| binding.command.clone())
            .collect();
        if let Some(command) = self.binding_for(pressed, scene) {
            return Resolution::Triggered { command };
        }
        // Nothing completes the sequence, but something could still: it is pending.
        let longer: Vec<String> = self
            .bindings
            .iter()
            .filter(|binding| {
                binding.chord.keys().len() > pressed.len() && binding.chord.starts_with(pressed)
            })
            .map(|binding| binding.command.clone())
            .collect();
        if !longer.is_empty() {
            return Resolution::Pending {
                pressed: pressed.to_vec(),
                candidates: longer,
            };
        }
        let _ = pending;
        Resolution::Nothing
    }

    /// The command a completed chord runs, with the narrowest scope winning.
    fn binding_for(&self, pressed: &[Key], scene: &Scene) -> Option<String> {
        self.bindings
            .iter()
            .filter(|binding| binding.chord.matches(pressed) && scene.admits(&binding.scope))
            .max_by_key(|binding| binding.scope.specificity())
            .map(|binding| binding.command.clone())
    }

    /// The keymap as `chord = command (scope)` lines, the user's overrides first.
    ///
    /// What is exported is what the person changed, not the merged table: a keymap file that listed
    /// every default would make "my change" invisible in the middle of a hundred lines.
    pub fn export(&self) -> String {
        let mut lines = Vec::new();
        for rule in &self.overrides {
            match rule {
                Override::Bind {
                    chord,
                    command,
                    scope,
                } => lines.push(format!("{chord} = {command} ({})", scope.render())),
                Override::Unbind { chord } => lines.push(format!("{chord} =")),
            }
        }
        lines.join("\n")
    }

    /// Reads overrides from the exported form, refusing a line that does not parse.
    ///
    /// All-or-nothing: a keymap that half-imported would leave a person with a keyboard they did not
    /// ask for, so one bad line refuses the whole file and names it.
    pub fn import(&mut self, text: &str) -> Result<usize, String> {
        let mut rules = Vec::new();
        for (number, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((left, right)) = line.split_once('=') else {
                return Err(format!(
                    "line {}: {line:?} is not `chord = command`",
                    number + 1
                ));
            };
            let chord = Chord::parse(left.trim())
                .map_err(|reason| format!("line {}: {reason}", number + 1))?;
            let right = right.trim();
            if right.is_empty() {
                rules.push(Override::Unbind { chord });
                continue;
            }
            let (command, scope) = match right.rsplit_once(" (") {
                Some((command, scope)) if scope.ends_with(')') => {
                    let scope = Scope::parse(&scope[..scope.len() - 1])
                        .map_err(|reason| format!("line {}: {reason}", number + 1))?;
                    (command.trim().to_owned(), scope)
                }
                _ => (right.to_owned(), Scope::Global),
            };
            if command.is_empty() {
                return Err(format!("line {}: no command", number + 1));
            }
            rules.push(Override::Bind {
                chord,
                command,
                scope,
            });
        }
        for rule in &rules {
            self.apply(rule.clone());
        }
        Ok(rules.len())
    }

    /// Every binding, for a report or an editor.
    pub fn bindings(&self) -> &[Binding] {
        &self.bindings
    }

    /// The bindings by command, for "what key runs this".
    pub fn keys_for(&self, command: &str) -> Vec<String> {
        self.bindings
            .iter()
            .filter(|binding| binding.command == command)
            .map(|binding| binding.chord.to_string())
            .collect()
    }

    /// A map from chord text to command, for a report.
    pub fn by_chord(&self) -> BTreeMap<String, String> {
        self.bindings
            .iter()
            .map(|binding| (binding.chord.to_string(), binding.command.clone()))
            .collect()
    }
}

/// The default keymap this build ships. Every chord here is one the registry also knows, so a
/// shortcut can never name a command that does not exist.
pub fn built_in() -> Keymap {
    let mut keymap = Keymap::new();
    let bindings = [
        ("Ctrl+S", "file.save", Scope::Global),
        ("Ctrl+Shift+S", "file.save-all", Scope::Global),
        ("Ctrl+W", "file.close", Scope::Global),
        ("Ctrl+Z", "edit.undo", Scope::Global),
        ("Ctrl+Shift+Z", "edit.redo", Scope::Global),
        ("Ctrl+B", "build.run", Scope::Global),
        ("Ctrl+K Ctrl+S", "keys.show", Scope::Global),
        ("Ctrl+P", "view.palette", Scope::Global),
        ("Escape", "panel.close", Scope::Panel("outline".to_owned())),
        ("Enter", "tree.activate", Scope::TextField),
    ];
    for (chord, command, scope) in bindings {
        keymap
            .add(Binding::new(
                Chord::parse(chord).expect("a valid default chord"),
                command,
                scope,
            ))
            .expect("the defaults do not conflict");
    }
    keymap
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(text: &str) -> Chord {
        Chord::parse(text).expect("a chord")
    }

    fn key(text: &str) -> Key {
        Key::parse(text).expect("a key")
    }

    fn keymap() -> Keymap {
        let mut keymap = Keymap::new();
        keymap
            .add(Binding::new(chord("Ctrl+S"), "file.save", Scope::Global))
            .expect("no conflict");
        keymap
            .add(Binding::new(
                chord("Ctrl+Shift+P"),
                "view.palette",
                Scope::Global,
            ))
            .expect("no conflict");
        keymap
            .add(Binding::new(
                chord("Ctrl+K Ctrl+S"),
                "keys.show",
                Scope::Global,
            ))
            .expect("no conflict");
        keymap
            .add(Binding::new(
                chord("Escape"),
                "panel.close",
                Scope::Panel("outline".into()),
            ))
            .expect("no conflict");
        keymap
    }

    #[test]
    fn the_default_keymap_binds_only_commands_and_has_no_conflicts() {
        let keymap = built_in();
        assert!(!keymap.is_empty());
        assert_eq!(keymap.len(), keymap.bindings().len());
        // Every default chord resolves in a scene its own scope admits.
        for binding in keymap.bindings() {
            let scene = match binding.scope() {
                Scope::Global => Scene::in_mode("design", None),
                Scope::Mode(mode) => Scene::in_mode(mode.clone(), None),
                Scope::Panel(panel) => Scene::in_mode("design", Some(panel.clone())),
                Scope::TextField => Scene::text_field("design"),
            };
            assert_eq!(
                keymap.resolve(binding.chord().keys(), &scene),
                Resolution::Triggered {
                    command: binding.command().to_owned()
                },
                "{} did not resolve",
                binding.chord()
            );
        }
    }

    #[test]
    fn a_typed_character_goes_to_the_text_field_and_never_to_a_command() {
        let mut keymap = keymap();
        // A command deliberately bound to a bare letter, to prove the rule is about the key, not
        // about what happens to be bound.
        keymap
            .add(Binding::new(
                chord("a"),
                "edit.select-all-left",
                Scope::Global,
            ))
            .expect("no conflict");

        let text = Scene::text_field("design");
        assert_eq!(
            keymap.resolve(&[key("a")], &text),
            Resolution::Suppressed {
                reason: "`a` is a typed character, so it goes to the text field rather than to a command".to_owned()
            }
        );
        // The same letter with a modifier is a command, even in a text field.
        assert_eq!(
            keymap.resolve(&[key("Ctrl+S")], &text),
            Resolution::Triggered {
                command: "file.save".to_owned()
            }
        );
    }

    #[test]
    fn a_capital_letter_is_text_and_an_input_method_suppresses_everything() {
        let keymap = keymap();
        // Shift+A is a capital A, not a command.
        let capital = Key::new(Modifiers::NONE.and_shift(), KeyCode::Char('a'));
        assert!(capital.is_text());
        assert!(matches!(
            keymap.resolve(&[capital], &Scene::in_mode("design", None)),
            Resolution::Suppressed { .. }
        ));

        // While an input method composes, even a modifier chord is the IME's.
        let composing = Scene::in_mode("design", None).composing();
        assert!(matches!(
            keymap.resolve(&[key("Ctrl+S")], &composing),
            Resolution::Suppressed { .. }
        ));
    }

    #[test]
    fn a_two_key_chord_is_pending_then_triggers_and_a_bad_second_key_is_not_swallowed() {
        let keymap = keymap();
        let scene = Scene::in_mode("design", None);
        match keymap.resolve(&[key("Ctrl+K")], &scene) {
            Resolution::Pending { candidates, .. } => assert_eq!(candidates, ["keys.show"]),
            other => panic!("expected pending, got {other:?}"),
        }
        assert_eq!(
            keymap.resolve(&[key("Ctrl+K"), key("Ctrl+S")], &scene),
            Resolution::Triggered {
                command: "keys.show".to_owned()
            }
        );
        // Ctrl+K then an unrelated key: nothing, and the sequence does not swallow the next command.
        assert_eq!(
            keymap.resolve(&[key("Ctrl+K"), key("Ctrl+Q")], &scene),
            Resolution::Nothing
        );
        assert_eq!(
            keymap.resolve(&[key("Ctrl+S")], &scene),
            Resolution::Triggered {
                command: "file.save".to_owned()
            }
        );
    }

    #[test]
    fn a_panel_binding_needs_its_panel_and_a_global_one_does_not() {
        let keymap = keymap();
        let outline = Scene::in_mode("design", Some("outline".to_owned()));
        let canvas = Scene::in_mode("design", Some("canvas".to_owned()));
        assert_eq!(
            keymap.resolve(&[key("Escape")], &outline),
            Resolution::Triggered {
                command: "panel.close".to_owned()
            }
        );
        assert_eq!(
            keymap.resolve(&[key("Escape")], &canvas),
            Resolution::Nothing
        );
        assert_eq!(
            keymap.resolve(&[key("Ctrl+S")], &canvas),
            Resolution::Triggered {
                command: "file.save".to_owned()
            }
        );
    }

    #[test]
    fn a_narrower_scope_may_deliberately_override_a_global_one() {
        let mut keymap = Keymap::new();
        keymap
            .add(Binding::new(chord("Ctrl+S"), "file.save", Scope::Global))
            .expect("no conflict");
        // A panel-scoped binding for the same chord is allowed: the narrower one wins.
        keymap
            .add(Binding::new(
                chord("Ctrl+S"),
                "component.save",
                Scope::Panel("inspector".into()),
            ))
            .expect("a narrower scope is allowed");
        assert_eq!(
            keymap.resolve(&[key("Ctrl+S")], &Scene::in_mode("design", None)),
            Resolution::Triggered {
                command: "file.save".to_owned()
            }
        );
        assert_eq!(
            keymap.resolve(
                &[key("Ctrl+S")],
                &Scene::in_mode("design", Some("inspector".to_owned()))
            ),
            Resolution::Triggered {
                command: "component.save".to_owned()
            }
        );
    }

    #[test]
    fn two_bindings_of_the_same_specificity_are_a_reported_conflict() {
        let mut keymap = Keymap::new();
        keymap
            .add(Binding::new(chord("Ctrl+S"), "file.save", Scope::Global))
            .expect("the first");
        let conflict = keymap
            .add(Binding::new(chord("Ctrl+S"), "file.save-as", Scope::Global))
            .expect_err("a conflict");
        assert_eq!(conflict.chord().to_string(), "Ctrl+S");
        assert!(conflict.message().contains("file.save"));
        assert!(conflict.message().contains("file.save-as"));
        assert_eq!(keymap.len(), 1, "the conflicting binding was not added");
    }

    #[test]
    fn overrides_round_trip_through_the_exported_form() {
        let mut keymap = keymap();
        keymap.apply(Override::Bind {
            chord: chord("Ctrl+Shift+S"),
            command: "file.save-as".to_owned(),
            scope: Scope::Global,
        });
        keymap.apply(Override::Unbind {
            chord: chord("Ctrl+Shift+P"),
        });
        let exported = keymap.export();
        assert!(exported.contains("Ctrl+Shift+S = file.save-as (global)"));
        assert!(exported.contains("Ctrl+Shift+P ="));

        // A fresh keymap imports the same file and behaves the same way.
        let mut imported = Keymap::new();
        let count = imported.import(&exported).expect("the file parses");
        assert_eq!(count, 2);
        assert_eq!(imported.export(), exported);
        assert_eq!(
            imported.resolve(&[key("Ctrl+Shift+S")], &Scene::in_mode("design", None)),
            Resolution::Triggered {
                command: "file.save-as".to_owned()
            }
        );
        assert_eq!(
            imported.resolve(&[key("Ctrl+Shift+P")], &Scene::in_mode("design", None)),
            Resolution::Nothing
        );
    }

    #[test]
    fn a_bad_override_line_refuses_the_whole_file_and_names_it() {
        let mut keymap = Keymap::new();
        let error = keymap
            .import("Ctrl+S = file.save (global)\nNotAKey+ = file.save\n")
            .expect_err("the second line is bad");
        assert!(error.contains("line 2"), "{error}");
        assert!(keymap.is_empty(), "nothing was applied when a line was bad");
    }

    #[test]
    fn a_chord_is_one_or_two_keys_and_the_key_syntax_is_strict() {
        assert_eq!(chord("Ctrl+K Ctrl+S").keys().len(), 2);
        assert_eq!(chord("Escape").keys().len(), 1);
        assert!(Chord::parse("Ctrl+K Ctrl+S Ctrl+X").is_err());
        assert!(Key::parse("Ctrl+").is_err());
        assert!(Key::parse("Ctrl+Shift").is_err());
        assert!(Key::parse("Ctrl+ab").is_err());
        assert_eq!(key("Ctrl+S").to_string(), "Ctrl+S");
        assert_eq!(key("Escape").to_string(), "Escape");
        assert_eq!(
            Scope::parse("panel:outline").unwrap().render(),
            "panel:outline"
        );
        assert!(Scope::parse("nowhere:x").is_err());
    }

    #[test]
    fn a_command_can_be_asked_which_keys_run_it() {
        let keymap = keymap();
        assert_eq!(keymap.keys_for("file.save"), ["Ctrl+S"]);
        assert!(keymap.keys_for("nothing.here").is_empty());
        assert_eq!(
            keymap.by_chord().get("Ctrl+K Ctrl+S").map(String::as_str),
            Some("keys.show")
        );
    }
}
