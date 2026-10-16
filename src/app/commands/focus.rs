// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Focus routing, context menus and the toolbar (task T05.05): one dispatch, and every entry point
//! reading the same state.
//!
//! The acceptance line is *"the same command behaves the same from every entry point, and nothing runs
//! twice"*. Both are about there being **one dispatch**, and they are answered by construction:
//!
//! * **[`press`] resolves a key once and returns exactly one [`Route`].** The keymap decides which
//!   command (T05.03), the registry decides whether it may run (T05.01), and the result is a single
//!   value: `Ran`, `Refused` (the key was claimed but the command is unavailable — it does **not** fall
//!   through to whatever is behind it), `Suppressed` (the key belongs to a text field), `Pending` (a
//!   chord is half-typed), or `Ignored`. A route that fell through after a refusal would let one
//!   keypress run two things; a route that ran and also passed the key on would be the "duplicate
//!   execution" this forbids.
//! * **The context menu and the toolbar are the same question about the same registry.** [`menu_entries`]
//!   and [`toolbar_entries`] both ask [`Registry::why_not`] for each command, so an item that is greyed
//!   in the menu bar is greyed on the toolbar and in the right-click menu, with the same reason — the
//!   three cannot drift, because there is one answer and three places that print it.
//! * **Focus is a value, and it is an input to the router, not a second registry.** [`Target`] says
//!   where the keyboard is; the scene (T05.03) and the context (T05.01) are built from it, so "the
//!   canvas has the keyboard" changes availability through the one path that already decides it.
//!
//! Nothing here is a window or a widget: it is the decision a window makes before it runs anything.

// The entry accessors are read by the toolbar and context-menu surfaces (T04.07/T26) as those land;
// until then the tests and `--focus` exercise them, and the silence is stated once with its reason.
#![allow(dead_code)]

use crate::app::commands::command::{Call, CommandId, Context, Registry};
use crate::app::commands::menu::{Item, Menu};
use crate::app::commands::shortcuts::{Key, Keymap, Resolution, Scene};

/// Where the keyboard is.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Target {
    /// The welcome page.
    Welcome,
    /// The design canvas.
    Canvas,
    /// The hierarchy or outline tree.
    Outline,
    /// The property inspector.
    Inspector,
    /// A text editor.
    Editor,
    /// The document tab strip.
    Tabs,
    /// The status bar.
    Status,
}

impl Target {
    /// The area id the keymap's scopes and the commands' focus use.
    pub const fn area(&self) -> &'static str {
        match self {
            Self::Welcome => "welcome",
            Self::Canvas => "canvas",
            Self::Outline => "outline",
            Self::Inspector => "inspector",
            Self::Editor => "editor",
            Self::Tabs => "tabs",
            Self::Status => "status",
        }
    }

    /// Whether the keyboard is in a text field, which changes what a bare key means.
    pub const fn is_text_field(&self) -> bool {
        matches!(self, Self::Editor)
    }

    /// The scene a keypress happens in, for the keymap.
    pub fn scene(&self, mode: &str) -> Scene {
        if self.is_text_field() {
            Scene::text_field(mode)
        } else {
            Scene::in_mode(mode, Some(self.area().to_owned()))
        }
    }
}

/// What one keypress did.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Route {
    /// A command ran; the key is claimed and nothing else sees it.
    Ran {
        /// The command.
        command: String,
    },
    /// A command was named but is not available; the key is claimed anyway.
    Refused {
        /// The command.
        command: String,
        /// Why it did not run.
        reason: String,
    },
    /// The key belongs to a text field.
    Suppressed {
        /// Why.
        reason: String,
    },
    /// A chord is half-typed.
    Pending {
        /// What could still run.
        candidates: Vec<String>,
    },
    /// Nothing is bound to the key.
    Ignored,
}

impl Route {
    /// Whether the key was claimed (so nothing behind the focused widget sees it).
    pub const fn is_claimed(&self) -> bool {
        matches!(
            self,
            Self::Ran { .. }
                | Self::Refused { .. }
                | Self::Suppressed { .. }
                | Self::Pending { .. }
        )
    }

    /// The route in one sentence.
    pub fn message(&self) -> String {
        match self {
            Self::Ran { command } => format!("ran {command}"),
            Self::Refused { command, reason } => format!("{command} was refused: {reason}"),
            Self::Suppressed { reason } => format!("the key belongs to a text field ({reason})"),
            Self::Pending { candidates } => {
                format!("waiting for more keys (could be {})", candidates.join(", "))
            }
            Self::Ignored => "nothing is bound to the key".to_owned(),
        }
    }
}

/// Resolves one keypress: the keymap names the command, the registry decides whether it runs.
///
/// One keypress produces one route. A refused command does not fall through — the key was bound, so it
/// is claimed — which is what keeps one press from running two things.
pub fn press(
    keymap: &Keymap,
    registry: &Registry,
    context: &Context,
    target: &Target,
    mode: &str,
    pressed: &[Key],
) -> Route {
    match keymap.resolve(pressed, &target.scene(mode)) {
        Resolution::Suppressed { reason } => Route::Suppressed { reason },
        Resolution::Nothing => Route::Ignored,
        Resolution::Pending { candidates, .. } => Route::Pending { candidates },
        Resolution::Triggered { command } => {
            match registry.resolve(&Call::to(command.clone()), context) {
                Ok(_) => Route::Ran { command },
                Err(refusal) => Route::Refused {
                    command,
                    reason: refusal.message(),
                },
            }
        }
    }
}

/// One entry a menu or a toolbar shows.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Entry {
    id: String,
    title: String,
    enabled: bool,
    reason: Option<String>,
}

impl Entry {
    /// The command's id.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The title shown.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Whether it would run.
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Why it would not, when it would not.
    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }

    /// The entry as a row reads.
    pub fn render(&self) -> String {
        match &self.reason {
            None => format!("{:<18} {}", self.id, self.title),
            Some(reason) => format!("{:<18} {} — {reason}", self.id, self.title),
        }
    }
}

/// The state of one command, from the registry, for every entry point to print.
fn state(registry: &Registry, id: &str, title: &str, context: &Context) -> Entry {
    let reason = registry.why_not(&Call::to(id.to_owned()), context);
    Entry {
        id: id.to_owned(),
        title: title.to_owned(),
        enabled: reason.is_none(),
        reason,
    }
}

/// The entries a context menu offers for a menu group, in menu order.
///
/// A leaf the registry says is unavailable is kept and carries its reason, so a right-click menu says
/// the same thing the menu bar does; a separator and a submenu are ignored (a context menu is flat).
pub fn menu_entries(registry: &Registry, menu: &Menu, context: &Context) -> Vec<Entry> {
    let mut entries = Vec::new();
    for (_, item) in menu.leaves() {
        if let Item::Command { title, command } = item {
            entries.push(state(registry, command, title, context));
        }
    }
    entries
}

/// The entries a toolbar shows, which is a chosen set of commands rather than a whole menu.
pub fn toolbar_entries(registry: &Registry, commands: &[String], context: &Context) -> Vec<Entry> {
    commands
        .iter()
        .map(|id| {
            let title = CommandId::parse(id)
                .and_then(|parsed| registry.command(&parsed))
                .map(|command| command.title().to_owned())
                .unwrap_or_else(|| id.clone());
            state(registry, id, &title, context)
        })
        .collect()
}

/// The toolbar this build declares: the commands a person reaches most often.
pub fn built_in_toolbar() -> Vec<String> {
    vec![
        "file.save".to_owned(),
        "file.save-all".to_owned(),
        "edit.undo".to_owned(),
        "edit.redo".to_owned(),
        "build.run".to_owned(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::commands::command::built_in as registry;
    use crate::app::commands::menu::built_in as menus;
    use crate::app::commands::shortcuts::{Chord, built_in as keymap};
    use crate::contracts::permission::EffectClass;

    fn context() -> Context {
        Context::in_area("design")
            .trusted()
            .authorizing(EffectClass::Command)
    }

    #[test]
    fn a_bound_key_runs_exactly_one_command_and_claims_the_key() {
        let route = press(
            &keymap(),
            &registry(),
            &context(),
            &Target::Canvas,
            "design",
            Chord::parse("Ctrl+S").unwrap().keys(),
        );
        assert_eq!(
            route,
            Route::Ran {
                command: "file.save".to_owned()
            }
        );
        assert!(route.is_claimed(), "a route that ran must not fall through");
    }

    #[test]
    fn a_bound_key_whose_command_is_unavailable_is_refused_and_still_claimed() {
        // build.run has the command effect, which is not authorized here (the project is trusted).
        let untrusted = Context::in_area("design").trusted();
        let route = press(
            &keymap(),
            &registry(),
            &untrusted,
            &Target::Canvas,
            "design",
            Chord::parse("Ctrl+B").unwrap().keys(),
        );
        match &route {
            Route::Refused { command, reason } => {
                assert_eq!(command, "build.run");
                assert!(reason.contains("not authorized"), "{reason}");
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
        assert!(
            route.is_claimed(),
            "the key was bound, so nothing behind it should see it"
        );
        assert!(
            route.is_claimed(),
            "the key was bound, so nothing behind it should see it"
        );
    }

    #[test]
    fn a_typed_character_in_the_editor_is_suppressed_and_an_unbound_key_is_ignored() {
        // In the editor, a bare character goes to the text field rather than to a command.
        let route = press(
            &keymap(),
            &registry(),
            &context(),
            &Target::Editor,
            "code",
            &[Key::character('a')],
        );
        assert!(matches!(route, Route::Suppressed { .. }));
        assert!(route.is_claimed());

        // A chord nothing is bound to is the one route that is not claimed.
        let route = press(
            &keymap(),
            &registry(),
            &context(),
            &Target::Canvas,
            "design",
            Chord::parse("Ctrl+Alt+Q").unwrap().keys(),
        );
        assert_eq!(route, Route::Ignored);
        assert!(!route.is_claimed());
    }

    #[test]
    fn a_half_typed_chord_is_pending_and_claims_the_key() {
        let route = press(
            &keymap(),
            &registry(),
            &context(),
            &Target::Canvas,
            "design",
            Chord::parse("Ctrl+K").unwrap().keys(),
        );
        match &route {
            Route::Pending { candidates } => assert!(candidates.contains(&"keys.show".to_owned())),
            other => panic!("expected pending, got {other:?}"),
        }
        assert!(route.is_claimed());
    }

    #[test]
    fn the_menu_the_toolbar_and_the_context_menu_agree_about_one_command() {
        let registry = registry();
        let untrusted = Context::in_area("design");
        let file_menu = menus()
            .into_iter()
            .find(|menu| menu.title() == "File")
            .expect("a File menu");

        let from_menu = menu_entries(&registry, &file_menu, &untrusted)
            .into_iter()
            .find(|entry| entry.id() == "file.save")
            .expect("the Save item");
        let from_toolbar = toolbar_entries(&registry, &built_in_toolbar(), &untrusted)
            .into_iter()
            .find(|entry| entry.id() == "file.save")
            .expect("the Save button");
        assert_eq!(from_menu, from_toolbar, "the two entry points must agree");
        assert!(from_menu.is_enabled());

        // A command that is unavailable is unavailable in both, with the same reason.
        let build_menu = menus()
            .into_iter()
            .find(|menu| menu.title() == "Build")
            .expect("a Build menu");
        let from_menu = menu_entries(&registry, &build_menu, &untrusted)
            .into_iter()
            .find(|entry| entry.id() == "build.run")
            .expect("the Run item");
        let from_toolbar = toolbar_entries(&registry, &built_in_toolbar(), &untrusted)
            .into_iter()
            .find(|entry| entry.id() == "build.run")
            .expect("the Run button");
        assert_eq!(from_menu.reason(), from_toolbar.reason());
        assert!(!from_menu.is_enabled());
        assert!(from_menu.reason().unwrap().contains("not trusted"));
    }

    #[test]
    fn the_context_menu_keeps_the_reason_of_a_disabled_item_rather_than_dropping_it() {
        let registry = registry();
        let untrusted = Context::in_area("design");
        let build_menu = menus()
            .into_iter()
            .find(|menu| menu.title() == "Build")
            .expect("a Build menu");
        let entries = menu_entries(&registry, &build_menu, &untrusted);
        assert_eq!(entries.len(), 1);
        assert!(
            entries[0].render().contains("not trusted"),
            "{}",
            entries[0].render()
        );
    }

    #[test]
    fn the_target_decides_the_scene_and_so_what_a_bare_key_means() {
        let editor = Target::Editor.scene("code");
        let canvas = Target::Canvas.scene("design");
        // The editor is a text field; the canvas is not.
        let keymap = keymap();
        assert!(matches!(
            keymap.resolve(&[Key::character('a')], &editor),
            Resolution::Suppressed { .. }
        ));
        assert!(!matches!(
            keymap.resolve(&[Key::character('a')], &canvas),
            Resolution::Triggered { .. }
        ));
        assert_eq!(Target::Editor.area(), "editor");
        assert!(Target::Editor.is_text_field());
        assert!(!Target::Canvas.is_text_field());
    }

    #[test]
    fn a_toolbar_entry_for_a_command_that_does_not_exist_says_so() {
        let registry = registry();
        let entries = toolbar_entries(
            &registry,
            &["file.save".to_owned(), "nowhere.here".to_owned()],
            &context(),
        );
        assert!(entries[0].is_enabled());
        assert!(!entries[1].is_enabled());
        assert!(entries[1].reason().unwrap().contains("no command"));
    }
}
