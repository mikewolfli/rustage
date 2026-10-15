// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The workbench: the main window's regions, its modes, the title and its dirty marker, the welcome
//! page and the status bar (task T04.01).
//!
//! The acceptance line is *"the active project, the target and the document's identity are clear"*.
//! "Clear" is made checkable by deriving all three from one place rather than by writing them in a
//! title somewhere:
//!
//! * **The workbench holds the identities, and the title is derived from them.** [`Workbench::title`]
//!   is computed from the active document, project and target, so a title cannot say one thing while
//!   the state says another — there is no second copy to drift.
//! * **A missing identity is missing, not filled in.** With no document open the title names the
//!   welcome page; it does not show an empty file name or the last document that happened to be open.
//!   The same for the project and the target: an absent one is absent, and the status bar says so.
//! * **The dirty marker is the document's own state.** [`Workbench::mark_dirty`] is called by whoever
//!   changed the text and cleared when the document is saved or discarded; the title shows `•` for it
//!   and nothing else does.
//! * **Regions follow the mode.** [`Workbench::regions`] answers which of the main window's regions a
//!   mode shows, so a mode switch is a change to one value and the layout follows, rather than each
//!   consumer deciding for itself what "the code view" contains.
//!
//! What this is not: a window. Nothing here creates a native window or draws anything; it is the state
//! a window renders, which is why it can be exercised without one.

// The workbench state is rendered by the window (T04.06) and read by the status bar and the menus as
// those land; until then the tests exercise it, and the silence is stated once here with its reason.
#![allow(dead_code)]

use std::fmt;

/// The workbench's mode (roadmap §2.1's seven, of which the designer shows these five today).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mode {
    /// The welcome page: no project, or a project just opened.
    #[default]
    Welcome,
    /// The design surface: the canvas and the property panels.
    Design,
    /// The code view and the text editors.
    Code,
    /// Running and debugging.
    Debug,
    /// Test and measurement.
    Test,
    /// Packaging and release.
    Release,
}

impl Mode {
    /// The mode's name, as a command line or a layout file writes it.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Welcome => "welcome",
            Self::Design => "design",
            Self::Code => "code",
            Self::Debug => "debug",
            Self::Test => "test",
            Self::Release => "release",
        }
    }

    /// The mode a name asks for, when it is one.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|mode| mode.name() == name)
    }

    /// Every mode.
    pub const ALL: [Self; 6] = [
        Self::Welcome,
        Self::Design,
        Self::Code,
        Self::Debug,
        Self::Test,
        Self::Release,
    ];
}

impl fmt::Display for Mode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// One of the main window's regions.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Region {
    /// The welcome page.
    Welcome,
    /// The menu and tool bar.
    Toolbar,
    /// The document's editors (the canvas in design mode, text in code mode).
    Editors,
    /// The side panels (hierarchy, outline, inspector).
    Panels,
    /// The message and task area.
    Messages,
    /// The status bar.
    StatusBar,
}

impl Region {
    /// The region's name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Welcome => "welcome",
            Self::Toolbar => "toolbar",
            Self::Editors => "editors",
            Self::Panels => "panels",
            Self::Messages => "messages",
            Self::StatusBar => "status",
        }
    }
}

/// One status bar item: a label and its value.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StatusItem {
    label: String,
    value: String,
}

impl StatusItem {
    /// An item.
    pub fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
        }
    }

    /// The label.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// The value.
    pub fn value(&self) -> &str {
        &self.value
    }

    /// The item as a status line shows it.
    pub fn render(&self) -> String {
        format!("{}: {}", self.label, self.value)
    }
}

/// The workbench: what is open, what it is called, and where the designer is.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Workbench {
    mode: Mode,
    document: Option<String>,
    project: Option<String>,
    target: Option<String>,
    dirty: bool,
    message: Option<String>,
}

impl Workbench {
    /// A workbench on its welcome page.
    pub fn new() -> Self {
        Self::default()
    }

    /// The mode the designer is in.
    pub const fn mode(&self) -> Mode {
        self.mode
    }

    /// Switches mode.
    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
    }

    /// Records the active document's identity (its project-relative path).
    pub fn open_document(&mut self, document: impl Into<String>) {
        self.document = Some(document.into());
        self.dirty = false;
    }

    /// Forgets the active document, so the title no longer names one.
    pub fn close_document(&mut self) {
        self.document = None;
        self.dirty = false;
    }

    /// Records the active project's name.
    pub fn open_project(&mut self, project: impl Into<String>) {
        self.project = Some(project.into());
    }

    /// Records the target the project would build for.
    pub fn set_target(&mut self, target: impl Into<String>) {
        self.target = Some(target.into());
    }

    /// Marks the active document as having unsaved changes.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Marks the active document as saved or discarded.
    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }

    /// Whether the active document has unsaved changes.
    pub const fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Sets the one-line message the status bar shows (a task's progress, the last failure).
    pub fn set_message(&mut self, message: impl Into<String>) {
        self.message = Some(message.into());
    }

    /// The active document, when one is open.
    pub fn document(&self) -> Option<&str> {
        self.document.as_deref()
    }

    /// Whether the welcome page is what should be shown: no document is open.
    pub const fn is_welcome(&self) -> bool {
        self.document.is_none()
    }

    /// The window title: the document, its dirty marker, the project and target, and the host.
    ///
    /// The pieces are separated so a person can tell them apart at a glance, and an absent piece is
    /// simply absent — a title that invented a project name would be exactly the "clear identity"
    /// failure this task is about.
    pub fn title(&self) -> String {
        let marker = if self.dirty { "• " } else { "" };
        let document = match &self.document {
            Some(document) => document.clone(),
            None => "Welcome".to_owned(),
        };
        let mut context = Vec::new();
        match &self.project {
            Some(project) => context.push(project.clone()),
            None => context.push("no project".to_owned()),
        }
        match &self.target {
            Some(target) => context.push(target.clone()),
            None => context.push("no target".to_owned()),
        }
        format!("{marker}{document} — {} — rustage", context.join(" · "))
    }

    /// The status bar's items, in the order it shows them.
    ///
    /// Every item names where its value came from, so "the active project" is not a bare string with
    /// no provenance: the project, the target, the document and its state each appear as themselves.
    pub fn status(&self) -> Vec<StatusItem> {
        let mut items = vec![
            StatusItem::new("mode", self.mode.name()),
            StatusItem::new(
                "project",
                self.project.clone().unwrap_or_else(|| "(none)".to_owned()),
            ),
            StatusItem::new(
                "target",
                self.target.clone().unwrap_or_else(|| "(none)".to_owned()),
            ),
            StatusItem::new(
                "document",
                self.document.clone().unwrap_or_else(|| "(none)".to_owned()),
            ),
            StatusItem::new("state", if self.dirty { "unsaved" } else { "saved" }),
        ];
        if let Some(message) = &self.message {
            items.push(StatusItem::new("message", message.clone()));
        }
        items
    }

    /// The regions this mode shows, in the order they appear.
    ///
    /// The welcome page is the whole window; the other modes share the toolbar, the editors, the
    /// panels, the messages and the status bar, which is what makes a mode switch a change to one
    /// value rather than to each consumer's own idea of the layout.
    pub fn regions(&self) -> Vec<Region> {
        match self.mode {
            Mode::Welcome => vec![Region::Welcome, Region::StatusBar],
            _ => vec![
                Region::Toolbar,
                Region::Editors,
                Region::Panels,
                Region::Messages,
                Region::StatusBar,
            ],
        }
    }

    /// The workbench in one report, for a terminal.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!("title: {}", self.title())];
        lines.push(format!(
            "regions: {}",
            self.regions()
                .iter()
                .map(|region| region.name())
                .collect::<Vec<_>>()
                .join(", ")
        ));
        lines.push("status:".to_owned());
        for item in self.status() {
            lines.push(format!("  {}", item.render()));
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_title_names_the_document_the_project_and_the_target() {
        let mut workbench = Workbench::new();
        workbench.open_project("Demo");
        workbench.set_target("x86_64-unknown-linux-gnu");
        workbench.open_document("pages/home.json");
        workbench.set_mode(Mode::Design);

        let title = workbench.title();
        assert!(title.contains("pages/home.json"), "{title}");
        assert!(title.contains("Demo"), "{title}");
        assert!(title.contains("x86_64-unknown-linux-gnu"), "{title}");
        assert!(title.contains("rustage"), "{title}");
        assert!(
            !title.contains('•'),
            "a saved document has no dirty marker: {title}"
        );
    }

    #[test]
    fn the_dirty_marker_tracks_the_documents_own_state() {
        let mut workbench = Workbench::new();
        workbench.open_document("pages/home.json");
        assert!(!workbench.is_dirty());
        workbench.mark_dirty();
        assert!(workbench.title().starts_with("• "), "{}", workbench.title());
        assert!(
            workbench
                .status()
                .iter()
                .any(|item| item.label() == "state" && item.value() == "unsaved")
        );
        workbench.mark_clean();
        assert!(!workbench.title().starts_with('•'));
        workbench.mark_dirty();
        workbench.close_document();
        assert!(
            !workbench.title().starts_with('•'),
            "closing the document forgets its dirty state: {}",
            workbench.title()
        );
    }

    #[test]
    fn an_absent_identity_is_absent_rather_than_invented() {
        let workbench = Workbench::new();
        assert!(workbench.is_welcome());
        let title = workbench.title();
        assert!(title.starts_with("Welcome"), "{title}");
        assert!(title.contains("no project"), "{title}");
        assert!(title.contains("no target"), "{title}");

        let status = workbench.status();
        let value = |label: &str| {
            status
                .iter()
                .find(|item| item.label() == label)
                .map(StatusItem::value)
                .unwrap_or("")
                .to_owned()
        };
        assert_eq!(value("project"), "(none)");
        assert_eq!(value("target"), "(none)");
        assert_eq!(value("document"), "(none)");
        assert_eq!(value("mode"), "welcome");
    }

    #[test]
    fn a_projects_target_becomes_visible_once_it_is_set() {
        let mut workbench = Workbench::new();
        workbench.open_project("Demo");
        workbench.set_target("aarch64-apple-darwin");
        let status = workbench.status();
        assert!(
            status
                .iter()
                .any(|item| item.label() == "target" && item.value() == "aarch64-apple-darwin")
        );
        assert!(
            status
                .iter()
                .any(|item| item.label() == "project" && item.value() == "Demo")
        );
    }

    #[test]
    fn a_message_appears_only_when_there_is_one() {
        let mut workbench = Workbench::new();
        assert!(
            !workbench
                .status()
                .iter()
                .any(|item| item.label() == "message")
        );
        workbench.set_message("building (3/7)");
        assert!(
            workbench
                .status()
                .iter()
                .any(|item| item.label() == "message" && item.value() == "building (3/7)")
        );
    }

    #[test]
    fn the_regions_follow_the_mode() {
        let mut workbench = Workbench::new();
        assert_eq!(workbench.regions(), [Region::Welcome, Region::StatusBar]);
        workbench.set_mode(Mode::Design);
        let regions = workbench.regions();
        assert!(regions.contains(&Region::Editors));
        assert!(regions.contains(&Region::Panels));
        assert!(!regions.contains(&Region::Welcome));
        assert_eq!(regions.last(), Some(&Region::StatusBar));
        workbench.set_mode(Mode::Code);
        assert!(workbench.regions().contains(&Region::Editors));
    }

    #[test]
    fn every_mode_has_a_name_and_the_report_says_which_is_showing() {
        let mut workbench = Workbench::new();
        for mode in Mode::ALL {
            assert_eq!(Mode::from_name(mode.name()), Some(mode));
            workbench.set_mode(mode);
            assert!(workbench.render()[0].contains(&workbench.title()));
            assert!(
                workbench
                    .status()
                    .iter()
                    .any(|item| item.label() == "mode" && item.value() == mode.name())
            );
        }
        assert!(Mode::from_name("nowhere").is_none());
    }

    #[test]
    fn switching_to_a_mode_does_not_change_what_is_open() {
        let mut workbench = Workbench::new();
        workbench.open_project("Demo");
        workbench.open_document("pages/home.json");
        workbench.mark_dirty();
        let before = workbench.clone();
        workbench.set_mode(Mode::Debug);
        assert_eq!(workbench.document(), before.document());
        assert_eq!(workbench.is_dirty(), before.is_dirty());
        assert_eq!(workbench.project, before.project);
    }
}
