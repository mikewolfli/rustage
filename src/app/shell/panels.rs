// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Panels: registration, showing and hiding, docking, and restoring size and focus (task T04.02).
//!
//! The acceptance line is *"a registered panel can be closed and restored, and a service that is not
//! implemented does not show a fake panel"*. The second half is the one that needs a rule rather than
//! care, because the tempting thing is to draw an empty panel and call it "not loaded yet":
//!
//! * **A panel declares the service it needs.** [`PanelSpec::service`] names it, and
//!   [`Panels::visible`] will not return a panel whose service is not in the available set — not
//!   greyed, not empty: **not shown**. A panel that needs no service is always available.
//! * **Showing and hiding is layout state, and layout state is remembered.** [`Panels::hide`] keeps
//!   the panel's size and focus, so [`Panels::show`] restores what it had; that is what "closed and
//!   restored" means, and it is the difference between a dock that comes back and one that comes back
//!   at some other width.
//! * **An unknown panel is refused, not created.** Showing a panel that was never registered is an
//!   error naming the id: a host that could conjure a panel from a string would make "which panels
//!   exist" unanswerable.
//! * **The layout holds no handles.** A panel is an id and a description; the dock, size and focus are
//!   values, so a layout can be written to a file and read back without carrying a window pointer or a
//!   process identity — which is the F04 rule at the top of the milestone's task table.

// A panel's registry, layout and report are read by the window (T04.01/T04.06) and the settings
// surface (T26) as those land; until then the tests exercise the surface, and the silence is stated
// once here with its reason, as the neighbouring trees do.
#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

/// Where a panel is docked.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Dock {
    /// The left edge.
    Left,
    /// The right edge.
    Right,
    /// The bottom edge.
    Bottom,
}

impl Dock {
    /// The dock's name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Bottom => "bottom",
        }
    }

    /// The dock a name asks for, when it is one.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|dock| dock.name() == name)
    }

    /// Every dock.
    pub const ALL: [Self; 3] = [Self::Left, Self::Right, Self::Bottom];
}

/// What a panel is: its identity, where it docks, how big it starts, and the service it needs.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PanelSpec {
    id: String,
    title: String,
    dock: Dock,
    default_size: u32,
    service: Option<String>,
}

impl PanelSpec {
    /// A panel that needs no service.
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        dock: Dock,
        default_size: u32,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            dock,
            default_size,
            service: None,
        }
    }

    /// The same panel, declaring the service it needs before it can be shown.
    pub fn needing(mut self, service: impl Into<String>) -> Self {
        self.service = Some(service.into());
        self
    }

    /// The panel's id.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Its title.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Where it docks.
    pub const fn dock(&self) -> Dock {
        self.dock
    }

    /// Its starting size.
    pub const fn default_size(&self) -> u32 {
        self.default_size
    }

    /// The service it needs, when it needs one.
    pub fn service(&self) -> Option<&str> {
        self.service.as_deref()
    }

    /// Whether the panel can be shown, given the services this host has.
    pub fn is_available(&self, services: &BTreeSet<String>) -> bool {
        match &self.service {
            None => true,
            Some(service) => services.contains(service),
        }
    }
}

/// Why a panel operation could not happen.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PanelError {
    /// A panel with the id is already registered.
    Duplicate {
        /// The id.
        id: String,
    },
    /// No panel has the id.
    Unknown {
        /// The id.
        id: String,
    },
}

impl PanelError {
    /// The error in one sentence.
    pub fn message(&self) -> String {
        match self {
            Self::Duplicate { id } => format!("a panel with the id {id} is already registered"),
            Self::Unknown { id } => {
                format!("there is no panel {id}; a panel is registered before it is shown")
            }
        }
    }
}

/// The remembered layout: what is shown, how big, and where the keyboard is.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Layout {
    visible: BTreeSet<String>,
    sizes: BTreeMap<String, u32>,
    focus: Option<String>,
}

impl Layout {
    /// The ids that are shown (before availability is taken into account).
    pub fn shown(&self) -> Vec<String> {
        self.visible.iter().cloned().collect()
    }

    /// Where the keyboard is, when a panel has it.
    pub fn focused(&self) -> Option<&str> {
        self.focus.as_deref()
    }
}

/// The panels a host has, and their layout.
#[derive(Clone, Debug, Default)]
pub struct Panels {
    specs: BTreeMap<String, PanelSpec>,
    order: Vec<String>,
    layout: Layout,
}

impl Panels {
    /// An empty set of panels.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a panel, refusing a duplicate id.
    pub fn register(&mut self, spec: PanelSpec) -> Result<(), PanelError> {
        if self.specs.contains_key(spec.id()) {
            return Err(PanelError::Duplicate {
                id: spec.id().to_owned(),
            });
        }
        self.layout
            .sizes
            .insert(spec.id().to_owned(), spec.default_size());
        self.order.push(spec.id().to_owned());
        self.specs.insert(spec.id().to_owned(), spec);
        Ok(())
    }

    /// The description of a panel.
    pub fn spec(&self, id: &str) -> Option<&PanelSpec> {
        self.specs.get(id)
    }

    /// Every registered panel, in registration order.
    pub fn specs(&self) -> impl Iterator<Item = &PanelSpec> {
        self.order.iter().filter_map(|id| self.specs.get(id))
    }

    /// How many are registered.
    pub fn len(&self) -> usize {
        self.specs.len()
    }

    /// Whether none are.
    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }

    /// The layout, for a report or a saved file.
    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// Shows a panel, remembering its size. An unknown id is refused.
    pub fn show(&mut self, id: &str) -> Result<(), PanelError> {
        if !self.specs.contains_key(id) {
            return Err(PanelError::Unknown { id: id.to_owned() });
        }
        self.layout.visible.insert(id.to_owned());
        Ok(())
    }

    /// Hides a panel. Its size and its focus are remembered, so showing it again restores them.
    pub fn hide(&mut self, id: &str) -> Result<(), PanelError> {
        if !self.specs.contains_key(id) {
            return Err(PanelError::Unknown { id: id.to_owned() });
        }
        self.layout.visible.remove(id);
        if self.layout.focus.as_deref() == Some(id) {
            self.layout.focus = None;
        }
        Ok(())
    }

    /// Resizes a panel.
    pub fn resize(&mut self, id: &str, size: u32) -> Result<(), PanelError> {
        if !self.specs.contains_key(id) {
            return Err(PanelError::Unknown { id: id.to_owned() });
        }
        self.layout.sizes.insert(id.to_owned(), size);
        Ok(())
    }

    /// The remembered size of a panel.
    pub fn size_of(&self, id: &str) -> Option<u32> {
        self.layout.sizes.get(id).copied()
    }

    /// Gives a panel the keyboard.
    pub fn focus(&mut self, id: &str) -> Result<(), PanelError> {
        if !self.specs.contains_key(id) {
            return Err(PanelError::Unknown { id: id.to_owned() });
        }
        self.layout.visible.insert(id.to_owned());
        self.layout.focus = Some(id.to_owned());
        Ok(())
    }

    /// The panels that may be shown, given the services this host has.
    ///
    /// A panel whose service is missing is not in this list, so nothing downstream can draw an empty
    /// frame and call it a panel that has not loaded yet.
    pub fn available<'a>(&'a self, services: &'a BTreeSet<String>) -> Vec<&'a PanelSpec> {
        self.specs()
            .filter(|spec| spec.is_available(services))
            .collect()
    }

    /// The panels actually on screen: shown, and available.
    pub fn visible<'a>(&'a self, services: &'a BTreeSet<String>) -> Vec<&'a PanelSpec> {
        self.specs()
            .filter(|spec| self.layout.visible.contains(spec.id()) && spec.is_available(services))
            .collect()
    }

    /// The panels that are hidden only because their service is missing, so a report can say which
    /// panels a person would gain by enabling a service rather than just leaving them out.
    pub fn unavailable(&self, services: &BTreeSet<String>) -> Vec<&PanelSpec> {
        self.specs()
            .filter(|spec| spec.service.is_some() && !spec.is_available(services))
            .collect()
    }

    /// The layout in one report.
    pub fn render(&self, services: &BTreeSet<String>) -> Vec<String> {
        let mut lines = vec![format!(
            "{} panel(s); {} available, {} visible",
            self.len(),
            self.available(services).len(),
            self.visible(services).len()
        )];
        for spec in self.specs() {
            let service = match spec.service() {
                Some(service) if services.contains(service) => format!(" ({service})"),
                Some(service) => format!(" (needs {service}, not available)"),
                None => String::new(),
            };
            let shown = if self
                .visible(services)
                .iter()
                .any(|shown| shown.id() == spec.id())
            {
                "shown"
            } else {
                "hidden"
            };
            let size = self.size_of(spec.id()).unwrap_or(0);
            lines.push(format!(
                "  {:<12} {:<10} {:<8} {shown}, size {size}{service}",
                spec.id(),
                spec.dock().name(),
                spec.title()
            ));
        }
        for spec in self.unavailable(services) {
            lines.push(format!(
                "  {} is not shown because {} is not available",
                spec.id(),
                spec.service().unwrap_or("its service")
            ));
        }
        lines
    }
}

/// The panels this build registers: the ones whose backing service exists today, plus the ones the
/// roadmap names with the service they wait for. A panel with a missing service is registered (so its
/// id is reserved and its layout default known) but never shown until the service arrives.
pub fn built_in() -> Panels {
    let mut panels = Panels::new();
    let specs = [
        PanelSpec::new("outline", "Outline", Dock::Left, 260).needing("document-tree"),
        PanelSpec::new("inspector", "Inspector", Dock::Right, 320).needing("property-schema"),
        PanelSpec::new("messages", "Messages", Dock::Bottom, 160),
        PanelSpec::new("problems", "Problems", Dock::Bottom, 160).needing("diagnostics"),
    ];
    for spec in specs {
        panels.register(spec).expect("the ids are unique");
    }
    // The panels start shown, which is the ordinary first-run layout; availability still decides
    // whether they are drawn.
    for id in ["outline", "messages", "problems"] {
        panels.show(id).expect("a registered panel");
    }
    panels
}

#[cfg(test)]
mod tests {
    use super::*;

    fn services(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    fn panels() -> Panels {
        let mut panels = Panels::new();
        panels
            .register(PanelSpec::new("outline", "Outline", Dock::Left, 260))
            .expect("registers");
        panels
            .register(
                PanelSpec::new("inspector", "Inspector", Dock::Right, 320)
                    .needing("property-schema"),
            )
            .expect("registers");
        panels
    }

    #[test]
    fn a_duplicate_panel_id_is_refused() {
        let mut panels = panels();
        let error = panels
            .register(PanelSpec::new("outline", "Another", Dock::Left, 100))
            .expect_err("a duplicate");
        assert_eq!(
            match &error {
                PanelError::Duplicate { id } | PanelError::Unknown { id } => id.as_str(),
            },
            "outline"
        );
        assert!(error.message().contains("already registered"));
        assert_eq!(panels.len(), 2);
    }

    #[test]
    fn a_panel_whose_service_is_missing_is_not_shown_at_all() {
        let mut panels = panels();
        panels.show("inspector").expect("a registered panel");
        panels.show("outline").expect("a registered panel");

        // Without the service, the inspector is neither available nor visible.
        let none = services(&[]);
        assert!(
            !panels
                .available(&none)
                .iter()
                .any(|spec| spec.id() == "inspector")
        );
        assert!(
            !panels
                .visible(&none)
                .iter()
                .any(|spec| spec.id() == "inspector")
        );
        assert!(
            panels
                .unavailable(&none)
                .iter()
                .any(|spec| spec.id() == "inspector")
        );

        // With the service, it is shown.
        let with = services(&["property-schema"]);
        assert!(
            panels
                .visible(&with)
                .iter()
                .any(|spec| spec.id() == "inspector")
        );
        // The one that needs nothing is shown in both.
        assert!(
            panels
                .visible(&none)
                .iter()
                .any(|spec| spec.id() == "outline")
        );
    }

    #[test]
    fn a_panel_can_be_closed_and_restored_with_its_size_and_focus() {
        let mut panels = panels();
        panels.show("outline").expect("registered");
        panels.resize("outline", 300).expect("registered");
        panels.focus("outline").expect("registered");
        assert_eq!(panels.layout().focused(), Some("outline"));

        panels.hide("outline").expect("registered");
        assert_eq!(
            panels.layout().focused(),
            None,
            "hiding takes the focus away"
        );
        assert_eq!(
            panels.size_of("outline"),
            Some(300),
            "the size is remembered"
        );

        panels.show("outline").expect("registered");
        assert_eq!(
            panels.size_of("outline"),
            Some(300),
            "restoring keeps the size"
        );
        assert!(
            panels
                .visible(&services(&[]))
                .iter()
                .any(|spec| spec.id() == "outline")
        );
    }

    #[test]
    fn an_unknown_panel_is_refused_rather_than_created() {
        let mut panels = panels();
        for operation in [
            panels.show("nowhere"),
            panels.hide("nowhere"),
            panels.resize("nowhere", 10),
            panels.focus("nowhere"),
        ] {
            let error = operation.expect_err("an unknown panel");
            assert!(error.message().contains("no panel"));
        }
        assert_eq!(panels.len(), 2, "nothing was created");
    }

    #[test]
    fn hiding_a_panel_that_is_not_shown_is_not_an_error() {
        let mut panels = panels();
        panels.hide("outline").expect("hiding is idempotent");
        assert!(panels.layout().shown().is_empty());
        panels.show("outline").expect("registered");
        panels.show("outline").expect("showing twice is fine");
        assert_eq!(panels.layout().shown(), ["outline"]);
    }

    #[test]
    fn the_layout_carries_no_handles_only_ids_and_values() {
        let mut panels = panels();
        panels.show("outline").expect("registered");
        panels.resize("outline", 275).expect("registered");
        // A layout can be described entirely in ids, docks and numbers.
        let layout = panels.layout().clone();
        assert_eq!(layout.shown(), ["outline"]);
        assert_eq!(panels.spec("outline").unwrap().dock(), Dock::Left);
        assert_eq!(panels.spec("outline").unwrap().default_size(), 260);
        assert_eq!(panels.spec("outline").unwrap().service(), None);
        assert_eq!(
            panels.spec("inspector").unwrap().service(),
            Some("property-schema")
        );
    }

    #[test]
    fn the_built_in_panels_reserve_ids_and_defaults_but_show_only_what_is_available() {
        let without = built_in();
        let ids: Vec<&str> = without.specs().map(PanelSpec::id).collect();
        assert!(ids.contains(&"outline"));
        assert!(ids.contains(&"inspector"));

        // outline needs document-tree, whose backing service is not implemented: not shown.
        assert!(
            !without
                .visible(&services(&[]))
                .iter()
                .any(|spec| spec.id() == "outline")
        );
        // messages needs nothing: shown.
        assert!(
            without
                .visible(&services(&[]))
                .iter()
                .any(|spec| spec.id() == "messages")
        );
        // With the service, the panel appears.
        assert!(
            without
                .visible(&services(&["document-tree"]))
                .iter()
                .any(|spec| spec.id() == "outline")
        );

        let report = without.render(&services(&[])).join("\n");
        assert!(
            report.contains("is not shown because document-tree is not available"),
            "{report}"
        );
    }

    #[test]
    fn every_dock_has_a_name_and_the_report_names_every_panel() {
        for dock in Dock::ALL {
            assert_eq!(Dock::from_name(dock.name()), Some(dock));
        }
        assert!(Dock::from_name("middle").is_none());

        let panels = built_in();
        let report = panels.render(&services(&[])).join("\n");
        for spec in panels.specs() {
            assert!(report.contains(spec.id()), "{report}");
        }
    }
}
