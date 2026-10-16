// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The window chrome: the toolbar for the current context, the breadcrumb, the status items, and
//! the rule that a notification never takes the keyboard (task T04.07).
//!
//! The acceptance line is *"errors are discoverable and typing is not interrupted"*. Both are about a
//! notice being able to arrive **without** changing what the person is doing, so the module is built
//! so that one of them cannot happen at all and the other is checkable:
//!
//! * **A notice cannot reach the keyboard, by the types.** [`Chrome::notices`] hands out a
//!   [`Notices`] view that borrows the notice list and *nothing else*: it has no method that moves the
//!   focus or the text being typed, so "a notification does not steal the keyboard" is a property of
//!   the shapes, not a promise in a comment. The host's centre (T01.06) stays the record — dedup,
//!   severities, next steps and dismissal live there — and this view is a projection the caller feeds
//!   from it.
//! * **An error is discoverable without being modal.** [`Notices::errors`] names every open error with
//!   its source, and [`Chrome::status`] carries a `problems` item with the counts and the latest
//!   notice, so a person can see that something failed and where from without a banner stealing the
//!   cursor. Nothing here auto-dismisses; clearing is a deliberate [`Notices::clear`].
//! * **The toolbar is the context, and it is the same registry the menus read.** [`toolbar_for`] picks
//!   the command set for an area and [`Chrome::toolbar`] resolves it through [`focus::toolbar_entries`],
//!   which asks the registry — so a command greyed on the toolbar is greyed in the menu with the same
//!   reason, and the two cannot drift.
//! * **The breadcrumb is the identity, spelled out.** [`Breadcrumb`] is built from the project, the
//!   document and (optionally) a node; an absent piece is left out rather than shown as a placeholder.
//!
//! Nothing here is a window or a widget: it is the state the window's edges render.

#![allow(dead_code)]

use crate::app::commands::command::{Context, Registry};
use crate::app::commands::focus::{self, Entry, Target};
use crate::app::shell::workbench::StatusItem;
use crate::host::notify::{Counts, Severity};

/// The commands the toolbar shows in an area.
///
/// An area with no special set shows [`focus::built_in_toolbar`], so there is one list of the common
/// commands rather than a second one per area; only the areas that genuinely differ are named.
pub fn toolbar_for(area: &str) -> Vec<String> {
    match area {
        "welcome" => Vec::new(),
        "debug" => vec!["build.run".to_owned(), "file.save".to_owned()],
        "code" | "editor" => vec![
            "file.save".to_owned(),
            "file.save-all".to_owned(),
            "edit.undo".to_owned(),
            "edit.redo".to_owned(),
            "view.palette".to_owned(),
        ],
        _ => focus::built_in_toolbar(),
    }
}

/// The width one toolbar button is assumed to take, in logical pixels.
pub const BUTTON_WIDTH: u32 = 96;

/// Splits the toolbar into what a width shows and what overflows into a "more" menu.
///
/// A narrow window does not lose buttons: the first `width / BUTTON_WIDTH` are shown and the rest
/// overflow, which a caller draws in a "more" menu. A width too small for even one button overflows
/// all of them, because a clipped button is a button that cannot be pressed.
pub fn fit_toolbar(entries: &[Entry], width: u32) -> (Vec<&Entry>, Vec<&Entry>) {
    let capacity = (width / BUTTON_WIDTH) as usize;
    let split = capacity.min(entries.len());
    (
        entries[..split].iter().collect(),
        entries[split..].iter().collect(),
    )
}

/// One kind of step in the breadcrumb trail.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CrumbKind {
    /// The project.
    Project,
    /// A document inside it.
    Document,
    /// A node inside the document.
    Node,
}

impl CrumbKind {
    /// The kind's name, for a report.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Document => "document",
            Self::Node => "node",
        }
    }
}

/// One step of the trail: what it is and what it is called.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Crumb {
    kind: CrumbKind,
    label: String,
}

impl Crumb {
    /// What the step is.
    pub const fn kind(&self) -> CrumbKind {
        self.kind
    }

    /// What it is called.
    pub fn label(&self) -> &str {
        &self.label
    }
}

/// Where the current view sits: the project, the document, and the node under the cursor.
///
/// A piece that does not exist is left out, not spelled `(none)`: a breadcrumb that invented a step
/// would be pointing at something that is not there.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Breadcrumb {
    crumbs: Vec<Crumb>,
}

impl Breadcrumb {
    /// The trail for a project and a document, either of which may be absent.
    pub fn of(project: Option<&str>, document: Option<&str>) -> Self {
        let mut crumbs = Vec::new();
        if let Some(project) = project.filter(|name| !name.is_empty()) {
            crumbs.push(Crumb {
                kind: CrumbKind::Project,
                label: project.to_owned(),
            });
        }
        if let Some(document) = document.filter(|name| !name.is_empty()) {
            crumbs.push(Crumb {
                kind: CrumbKind::Document,
                label: document.to_owned(),
            });
        }
        Self { crumbs }
    }

    /// The same trail with a node as its last step.
    pub fn into_node(mut self, label: impl Into<String>) -> Self {
        let label = label.into();
        if !label.is_empty() {
            self.crumbs.push(Crumb {
                kind: CrumbKind::Node,
                label,
            });
        }
        self
    }

    /// Every step, in order.
    pub fn crumbs(&self) -> &[Crumb] {
        &self.crumbs
    }

    /// Whether there is nothing to show (no project and no document).
    pub fn is_empty(&self) -> bool {
        self.crumbs.is_empty()
    }

    /// The trail as one line, or an empty string when there is none.
    pub fn trail(&self) -> String {
        self.crumbs
            .iter()
            .map(|crumb| crumb.label.as_str())
            .collect::<Vec<_>>()
            .join(" › ")
    }
}

/// One notice the chrome is showing: a projection of a host notification.
///
/// Only what the edges render lives here — how loud it is, what it says, and where it came from. The
/// host's [`NotificationCentre`](crate::host::notify::NotificationCentre) remains the record: it owns
/// de-duplication, dismissal and the next steps, and this view is fed from it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Notice {
    severity: Severity,
    summary: String,
    source: String,
}

impl Notice {
    /// A notice.
    pub fn new(severity: Severity, summary: impl Into<String>, source: impl Into<String>) -> Self {
        Self {
            severity,
            summary: summary.into(),
            source: source.into(),
        }
    }

    /// How loud it is.
    pub const fn severity(&self) -> Severity {
        self.severity
    }

    /// What happened, in one line.
    pub fn summary(&self) -> &str {
        &self.summary
    }

    /// Where it came from.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Whether it is a problem (a warning or an error), rather than news.
    pub const fn is_problem(&self) -> bool {
        self.severity.is_problem()
    }

    /// The notice as one line.
    pub fn render(&self) -> String {
        format!(
            "{}: {} ({})",
            self.severity.name(),
            self.summary,
            self.source
        )
    }
}

/// The notification view: it can add and read notices, and it cannot reach the keyboard.
///
/// This is a *borrowed sub-view* of [`Chrome`], deliberately unable to see or change `focus` and
/// `typed`. A method that posted a notice and also moved the cursor would have to be written
/// somewhere else.
pub struct Notices<'a> {
    notices: &'a mut Vec<Notice>,
}

impl Notices<'_> {
    /// Shows a notice.
    pub fn show(
        &mut self,
        severity: Severity,
        summary: impl Into<String>,
        source: impl Into<String>,
    ) {
        self.notices.push(Notice::new(severity, summary, source));
    }

    /// Every notice shown, oldest first.
    pub fn all(&self) -> &[Notice] {
        self.notices
    }

    /// The problems: warnings and errors, oldest first.
    pub fn problems(&self) -> Vec<&Notice> {
        self.notices
            .iter()
            .filter(|notice| notice.is_problem())
            .collect()
    }

    /// The errors, oldest first.
    pub fn errors(&self) -> Vec<&Notice> {
        self.notices
            .iter()
            .filter(|notice| notice.severity == Severity::Error)
            .collect()
    }

    /// How many of each severity are shown, counted from the list itself.
    pub fn counts(&self) -> Counts {
        let mut counts = Counts::default();
        for notice in self.notices.iter() {
            match notice.severity {
                Severity::Error => counts.errors += 1,
                Severity::Warning => counts.warnings += 1,
                Severity::Info => counts.information += 1,
            }
        }
        counts
    }

    /// Clears the shown notices. The host's record is untouched; this is only what the edges draw.
    pub fn clear(&mut self) {
        self.notices.clear();
    }
}

/// The chrome: where the keyboard is, what is being typed, where the view sits, and what the edges
/// are showing. It is not the thing that decides any of them.
#[derive(Clone, PartialEq, Debug)]
pub struct Chrome {
    focus: Target,
    area: String,
    typed: String,
    breadcrumb: Breadcrumb,
    notices: Vec<Notice>,
}

impl Chrome {
    /// A fresh chrome: the welcome page, nothing typed, no trail, no notices.
    pub fn new() -> Self {
        Self {
            focus: Target::Welcome,
            area: "welcome".to_owned(),
            typed: String::new(),
            breadcrumb: Breadcrumb::default(),
            notices: Vec::new(),
        }
    }

    /// Moves the keyboard to a target, which is also the toolbar's context.
    pub fn set_focus(&mut self, target: Target) {
        self.area = target.area().to_owned();
        self.focus = target;
    }

    /// Where the keyboard is.
    pub const fn focus(&self) -> &Target {
        &self.focus
    }

    /// The toolbar's context (the area the keyboard is in).
    pub fn area(&self) -> &str {
        &self.area
    }

    /// What has been typed into the focused text field.
    pub fn typed(&self) -> &str {
        &self.typed
    }

    /// Types one character, when the keyboard is in a text field. Answers whether it went in.
    pub fn type_char(&mut self, character: char) -> bool {
        if !self.focus.is_text_field() {
            return false;
        }
        self.typed.push(character);
        true
    }

    /// Backspaces one character, when the keyboard is in a text field. Answers whether it removed one.
    pub fn backspace(&mut self) -> bool {
        if !self.focus.is_text_field() {
            return false;
        }
        self.typed.pop().is_some()
    }

    /// The breadcrumb trail.
    pub const fn breadcrumb(&self) -> &Breadcrumb {
        &self.breadcrumb
    }

    /// Sets the trail.
    pub fn set_breadcrumb(&mut self, breadcrumb: Breadcrumb) {
        self.breadcrumb = breadcrumb;
    }

    /// The notification view. It can add and read notices, and cannot reach the keyboard.
    pub fn notices(&mut self) -> Notices<'_> {
        Notices {
            notices: &mut self.notices,
        }
    }

    /// Every notice shown, oldest first.
    pub fn all_notices(&self) -> &[Notice] {
        &self.notices
    }

    /// The toolbar for the focused area, resolved through the registry, so it agrees with the menus.
    pub fn toolbar(&self, registry: &Registry, context: &Context) -> Vec<Entry> {
        focus::toolbar_entries(registry, &toolbar_for(&self.area), context)
    }

    /// The status bar's items: where the view sits, what problems there are, and the latest notice.
    ///
    /// The `problems` item is what makes an error discoverable without a modal: it is always there,
    /// and it counts the notices the edges are showing. The `notice` item carries the newest one's
    /// line, so the first glance says what happened.
    pub fn status(&self) -> Vec<StatusItem> {
        let counts = self.counts();
        let mut items = vec![
            StatusItem::new(
                "where",
                if self.breadcrumb.is_empty() {
                    "(nowhere)".to_owned()
                } else {
                    self.breadcrumb.trail()
                },
            ),
            StatusItem::new("area", self.area.clone()),
            StatusItem::new(
                "problems",
                format!("{} error(s), {} warning(s)", counts.errors, counts.warnings),
            ),
        ];
        if let Some(latest) = self.notices.last() {
            items.push(StatusItem::new(
                "notice",
                format!("{}: {}", latest.source, latest.summary),
            ));
        }
        items
    }

    /// How many notices of each severity are shown.
    pub fn counts(&self) -> Counts {
        let mut counts = Counts::default();
        for notice in &self.notices {
            match notice.severity {
                Severity::Error => counts.errors += 1,
                Severity::Warning => counts.warnings += 1,
                Severity::Info => counts.information += 1,
            }
        }
        counts
    }

    /// The chrome in one report.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "breadcrumb: {}",
            if self.breadcrumb.is_empty() {
                "(nowhere)".to_owned()
            } else {
                self.breadcrumb.trail()
            }
        )];
        lines.push(format!("area: {}", self.area));
        lines.push(format!("typed: {}", self.typed));
        lines.push(format!(
            "notices: {} shown, {} problem(s), {} error(s)",
            self.notices.len(),
            self.counts().warnings + self.counts().errors,
            self.counts().errors
        ));
        for notice in &self.notices {
            lines.push(format!("  {}", notice.render()));
        }
        lines
    }
}

impl Default for Chrome {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{Breadcrumb, Chrome, CrumbKind, fit_toolbar, toolbar_for};
    use crate::app::commands::command::{Context, built_in as registry};
    use crate::app::commands::focus::Target;
    use crate::contracts::permission::EffectClass;
    use crate::host::notify::Severity;

    fn context() -> Context {
        Context::in_area("design")
            .trusted()
            .authorizing(EffectClass::Command)
    }

    #[test]
    fn the_breadcrumb_is_the_identity_and_leaves_out_what_is_not_there() {
        let trail = Breadcrumb::of(Some("Demo"), Some("pages/home.json")).into_node("Button");
        assert_eq!(trail.trail(), "Demo › pages/home.json › Button");
        assert_eq!(trail.crumbs().len(), 3);
        assert_eq!(trail.crumbs()[0].kind(), CrumbKind::Project);
        assert_eq!(trail.crumbs()[2].kind(), CrumbKind::Node);

        // No document yet: the document step is absent, not a placeholder.
        let partial = Breadcrumb::of(Some("Demo"), None);
        assert_eq!(partial.trail(), "Demo");
        assert!(Breadcrumb::of(None, None).is_empty());
        assert_eq!(Breadcrumb::of(Some(""), Some("")).trail(), "");
    }

    #[test]
    fn the_toolbar_is_the_context_and_reads_the_same_registry_the_menus_read() {
        let registry = registry();
        let context = context();

        let welcome = Chrome::new();
        assert!(
            welcome.toolbar(&registry, &context).is_empty(),
            "the welcome page offers nothing"
        );

        let mut editor = Chrome::new();
        editor.set_focus(Target::Editor);
        let entries = editor.toolbar(&registry, &context);
        assert!(entries.iter().any(|entry| entry.id() == "file.save-all"));
        assert_eq!(toolbar_for("editor"), toolbar_for("code"));

        // A command whose effect is not authorized keeps its reason, exactly as the menu would.
        let untrusted = Context::in_area("design");
        let mut canvas = Chrome::new();
        canvas.set_focus(Target::Canvas);
        let entries = canvas.toolbar(&registry, &untrusted);
        let run = entries
            .iter()
            .find(|entry| entry.id() == "build.run")
            .expect("the Run button");
        assert!(!run.is_enabled());
        assert!(run.reason().unwrap().contains("not trusted"));
    }

    #[test]
    fn an_error_is_discoverable_without_a_modal_and_names_its_source() {
        let mut chrome = Chrome::new();
        {
            let mut notices = chrome.notices();
            notices.show(Severity::Info, "build finished", "task #1");
            notices.show(Severity::Error, "the linker failed", "build.run");
        }

        // It is findable: the errors are named, with where they came from.
        let errors: Vec<String> = chrome
            .notices()
            .errors()
            .iter()
            .map(|notice| format!("{} @{}", notice.summary(), notice.source()))
            .collect();
        assert_eq!(errors, ["the linker failed @build.run"]);

        // And it is on the status bar, so the first glance already says something failed.
        let status = chrome.status();
        let value = |label: &str| {
            status
                .iter()
                .find(|item| item.label() == label)
                .map(|item| item.value().to_owned())
                .unwrap_or_default()
        };
        assert_eq!(value("problems"), "1 error(s), 0 warning(s)");
        assert!(value("notice").contains("build.run"), "{}", value("notice"));

        // Clearing is deliberate; nothing auto-dismisses.
        chrome.notices().clear();
        assert!(chrome.notices().errors().is_empty());
    }

    #[test]
    fn typing_is_not_interrupted_by_a_notice_arriving() {
        let mut chrome = Chrome::new();
        chrome.set_focus(Target::Editor);
        assert!(chrome.type_char('a'));
        assert!(chrome.type_char('b'));

        // A notice arrives in the middle of typing.
        chrome
            .notices()
            .show(Severity::Error, "the linker failed", "build.run");
        chrome
            .notices()
            .show(Severity::Warning, "the build is slow", "task #2");

        // The keyboard is where it was, and the next character still goes into the field.
        assert_eq!(chrome.focus(), &Target::Editor);
        assert_eq!(chrome.area(), "editor");
        assert!(chrome.type_char('c'));
        assert_eq!(chrome.typed(), "abc");
    }

    #[test]
    fn a_bare_key_only_reaches_a_text_field() {
        let mut chrome = Chrome::new();
        assert!(
            !chrome.type_char('a'),
            "the welcome page is not a text field"
        );
        assert_eq!(chrome.typed(), "");

        chrome.set_focus(Target::Canvas);
        assert!(!chrome.type_char('a'));
        assert!(!chrome.backspace());
        assert_eq!(chrome.typed(), "");

        chrome.set_focus(Target::Editor);
        assert!(chrome.type_char('x'));
        assert_eq!(chrome.typed(), "x");
        assert!(chrome.backspace());
        assert_eq!(chrome.typed(), "");
        assert!(!chrome.backspace(), "nothing left to remove");
    }

    #[test]
    fn a_narrow_window_overflows_the_toolbar_rather_than_dropping_buttons() {
        let registry = registry();
        let context = context();
        let mut chrome = Chrome::new();
        chrome.set_focus(Target::Canvas);
        let entries = chrome.toolbar(&registry, &context);
        assert_eq!(entries.len(), 5);

        // Wide enough for every button.
        let (shown, overflowed) = fit_toolbar(&entries, 1000);
        assert_eq!(shown.len(), entries.len());
        assert!(overflowed.is_empty());

        // A narrow window shows what fits and overflows the rest; nothing is lost.
        let (shown, overflowed) = fit_toolbar(&entries, 200);
        assert_eq!(shown.len(), 2);
        assert_eq!(overflowed.len(), 3);
        assert_eq!(shown.len() + overflowed.len(), entries.len());

        // Too narrow for even one button: all overflow rather than a clipped button.
        let (shown, overflowed) = fit_toolbar(&entries, 10);
        assert!(shown.is_empty());
        assert_eq!(overflowed.len(), entries.len());
    }

    #[test]
    fn the_report_names_the_trail_the_area_and_the_notices() {
        let mut chrome = Chrome::new();
        chrome.set_focus(Target::Editor);
        chrome.set_breadcrumb(Breadcrumb::of(Some("Demo"), Some("pages/home.json")));
        chrome.notices().show(Severity::Error, "boom", "build.run");

        let report = chrome.render().join("\n");
        assert!(
            report.contains("breadcrumb: Demo › pages/home.json"),
            "{report}"
        );
        assert!(report.contains("area: editor"), "{report}");
        assert!(
            report.contains("1 shown, 1 problem(s), 1 error(s)"),
            "{report}"
        );
        assert!(report.contains("boom"), "{report}");

        // With nothing set the report says so rather than printing an empty trail.
        let empty = Chrome::new();
        assert!(empty.render()[0].contains("(nowhere)"));
    }
}
