// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The target application's window shell and its capability seam (task T26.01).
//!
//! The acceptance line is *"when the library lacks the target capability it must be marked as a gap,
//! not assembled into a fake native menu"*. The whole module is that one distinction:
//!
//! * [`Capability::Provided`] says the target's library really gives this build the thing: a native
//!   menu bar, a native dock. It is a fact about the target, not a wish.
//! * [`Capability::Gap`] says it does not, and names *what* is missing and *why*. [`assemble`] then
//!   refuses a shell that asked for that region, so a requested native menu bar can never be turned
//!   into a hand-drawn imitation and reported as the same thing.
//!
//! Two further decisions are deliberate:
//!
//! * Only the menu bar and the dock have a native seam. A tool bar and a status bar are always drawn
//!   by the application itself, which is why requesting them against a gapped library still succeeds.
//! * The shell is versioned structural data ([`FORMAT`]) — the window and *which* regions exist, not
//!   how they look. Appearance stays CSS, so nothing here carries a colour, a font or a point size.

#![allow(dead_code)]

use crate::contracts::revision::FormatVersion;

/// The shell schema version this build writes. A reader compares it rather than guessing.
pub const FORMAT: FormatVersion = FormatVersion::from_raw(1);

/// A target application this shell is designed for.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Target(String);

impl Target {
    /// Builds a target from its name, or `None` when the name is blank.
    pub fn new(name: &str) -> Option<Self> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return None;
        }
        Some(Self(trimmed.to_owned()))
    }

    /// The target's name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Whether the target's library provides a capability the shell would like to have.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Capability {
    /// The library really provides it.
    Provided,
    /// It does not, and this says what is missing and why.
    Gap {
        /// What is missing, such as `native menu bar`.
        what: String,
        /// Why the target cannot provide it.
        reason: String,
    },
}

impl Capability {
    /// A provided capability.
    pub fn provided() -> Self {
        Self::Provided
    }

    /// A gap naming what is missing and why. A blank reason is refused at the edge, so a gap always
    /// explains itself rather than merely being absent.
    pub fn gap(what: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::Gap {
            what: what.into(),
            reason: reason.into(),
        }
    }

    /// Whether the library provides it.
    pub fn is_provided(&self) -> bool {
        matches!(self, Self::Provided)
    }

    /// The gap's reason, when this is a gap.
    pub fn gap_reason(&self) -> Option<&str> {
        match self {
            Self::Provided => None,
            Self::Gap { reason, .. } => Some(reason),
        }
    }

    /// The capability in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Provided => "provided".to_owned(),
            Self::Gap { what, reason } => format!("gap: {what} ({reason})"),
        }
    }
}

/// One chrome region the shell may have.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Region {
    /// The menu bar.
    MenuBar,
    /// The tool bar.
    ToolBar,
    /// The status bar.
    StatusBar,
    /// The dock area.
    Dock,
}

impl Region {
    /// Every region, in the order reports list them.
    pub const ALL: [Region; 4] = [
        Region::MenuBar,
        Region::ToolBar,
        Region::StatusBar,
        Region::Dock,
    ];

    /// The region's name as an operation writes it.
    pub fn name(&self) -> &'static str {
        match self {
            Self::MenuBar => "menu",
            Self::ToolBar => "toolbar",
            Self::StatusBar => "status",
            Self::Dock => "dock",
        }
    }

    /// Reads a region's name, or `None` when the name is not one of the four.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "menu" => Some(Self::MenuBar),
            "toolbar" => Some(Self::ToolBar),
            "status" => Some(Self::StatusBar),
            "dock" => Some(Self::Dock),
            _ => None,
        }
    }
}

/// Which regions a shell was asked to have.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Regions {
    /// Whether a menu bar was requested.
    pub menu_bar: bool,
    /// Whether a tool bar was requested.
    pub tool_bar: bool,
    /// Whether a status bar was requested.
    pub status_bar: bool,
    /// Whether a dock area was requested.
    pub dock: bool,
}

impl Regions {
    /// No regions.
    pub fn none() -> Self {
        Self::default()
    }

    /// Every region.
    pub fn all() -> Self {
        Self {
            menu_bar: true,
            tool_bar: true,
            status_bar: true,
            dock: true,
        }
    }

    /// This set plus one region.
    pub fn with(mut self, region: Region) -> Self {
        match region {
            Region::MenuBar => self.menu_bar = true,
            Region::ToolBar => self.tool_bar = true,
            Region::StatusBar => self.status_bar = true,
            Region::Dock => self.dock = true,
        }
        self
    }

    /// Whether the region is requested.
    pub fn has(&self, region: Region) -> bool {
        match region {
            Region::MenuBar => self.menu_bar,
            Region::ToolBar => self.tool_bar,
            Region::StatusBar => self.status_bar,
            Region::Dock => self.dock,
        }
    }

    /// The requested regions, in report order.
    pub fn requested(&self) -> Vec<Region> {
        Region::ALL
            .into_iter()
            .filter(|region| self.has(*region))
            .collect()
    }
}

/// The window the application opens.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Window {
    /// The window's title.
    pub title: String,
    /// The window's width in logical pixels.
    pub width: u32,
    /// The window's height in logical pixels.
    pub height: u32,
}

impl Window {
    /// A window, refusing a blank title or a zero dimension.
    pub fn new(title: &str, width: u32, height: u32) -> Result<Self, String> {
        let trimmed = title.trim();
        if trimmed.is_empty() {
            return Err("a window needs a title".to_owned());
        }
        if width == 0 || height == 0 {
            return Err(format!(
                "a window needs a non-zero size, got {width}x{height}"
            ));
        }
        Ok(Self {
            title: trimmed.to_owned(),
            width,
            height,
        })
    }

    /// The window in one line.
    pub fn render(&self) -> String {
        format!("window {:?} {}x{}", self.title, self.width, self.height)
    }
}

/// What the target's library provides for the shell.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Library {
    /// The target.
    pub target: Target,
    /// Whether a native menu is available.
    pub menu: Capability,
    /// Whether a native dock is available.
    pub dock: Capability,
}

impl Library {
    /// A library that provides both seams.
    pub fn provided(target: Target) -> Self {
        Self {
            target,
            menu: Capability::Provided,
            dock: Capability::Provided,
        }
    }

    /// The same library with its menu capability replaced.
    pub fn with_menu(mut self, menu: Capability) -> Self {
        self.menu = menu;
        self
    }

    /// The same library with its dock capability replaced.
    pub fn with_dock(mut self, dock: Capability) -> Self {
        self.dock = dock;
        self
    }
}

/// The assembled application shell: structural data, with the seams it was built against.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Shell {
    /// The window.
    pub window: Window,
    /// The regions the shell has.
    pub regions: Regions,
    /// The menu capability the shell was built against.
    pub menu: Capability,
    /// The dock capability the shell was built against.
    pub dock: Capability,
}

/// Which capability a region needs from the library, if any.
///
/// Only the menu bar and the dock are native seams; the others are always drawn by the application.
fn needed_capability(region: Region) -> Option<Region> {
    match region {
        Region::MenuBar | Region::Dock => Some(region),
        Region::ToolBar | Region::StatusBar => None,
    }
}

/// Assembles a shell from a window, the requested regions and the target's library.
///
/// A requested region whose capability the library only gaps is a refusal naming the gap — this is the
/// acceptance line: the shell is never assembled with a fabricated stand-in for a missing native menu.
pub fn assemble(window: Window, regions: Regions, library: &Library) -> Result<Shell, String> {
    for region in regions.requested() {
        match needed_capability(region) {
            None => {}
            Some(Region::MenuBar) if !library.menu.is_provided() => {
                return Err(format!(
                    "the menu bar needs a native menu on {}, which is a {}",
                    library.target.as_str(),
                    library.menu.render(),
                ));
            }
            Some(Region::Dock) if !library.dock.is_provided() => {
                return Err(format!(
                    "the dock needs target support on {}, which is a {}",
                    library.target.as_str(),
                    library.dock.render(),
                ));
            }
            Some(_) => {}
        }
    }
    Ok(Shell {
        window,
        regions,
        menu: library.menu.clone(),
        dock: library.dock.clone(),
    })
}

impl Shell {
    /// The schema version the shell was written as.
    pub fn format(&self) -> FormatVersion {
        FORMAT
    }

    /// The shell in a report: the version, the window, each region, then the seams.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!("shell {FORMAT}")];
        lines.push(self.window.render());
        for region in self.regions.requested() {
            lines.push(format!("region {}", region.name()));
        }
        lines.push(format!("menu: {}", self.menu.render()));
        lines.push(format!("dock: {}", self.dock.render()));
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{Capability, FORMAT, Library, Region, Regions, Shell, Target, Window, assemble};

    fn target() -> Target {
        Target::new("desktop").expect("a target")
    }

    fn window() -> Window {
        Window::new("Notes", 800, 600).expect("a window")
    }

    #[test]
    fn a_window_refuses_a_blank_title_or_a_zero_size() {
        assert!(Window::new("  ", 800, 600).is_err());
        assert!(Window::new("Notes", 0, 600).is_err());
        assert!(Window::new("Notes", 800, 0).is_err());
        assert!(Window::new("  Notes  ", 800, 600).is_ok());
        assert_eq!(window().title, "Notes");
    }

    #[test]
    fn a_blank_target_name_is_not_a_target() {
        assert!(Target::new("").is_none());
        assert!(Target::new("   ").is_none());
        assert_eq!(target().as_str(), "desktop");
    }

    #[test]
    fn a_gap_always_explains_itself() {
        let provided = Capability::provided();
        assert!(provided.is_provided());
        assert_eq!(provided.gap_reason(), None);
        assert_eq!(provided.render(), "provided");

        let gap = Capability::gap("native menu", "the target has no menu API");
        assert!(!gap.is_provided());
        assert_eq!(gap.gap_reason(), Some("the target has no menu API"));
        assert!(gap.render().starts_with("gap:"));
    }

    #[test]
    fn a_region_is_one_of_four_and_an_unknown_name_is_not_a_region() {
        for region in Region::ALL {
            assert_eq!(Region::parse(region.name()), Some(region));
        }
        assert_eq!(Region::parse("ribbon"), None);
    }

    #[test]
    fn a_shell_over_a_provided_library_has_the_regions_it_asked_for() {
        let library = Library::provided(target());
        let regions = Regions::none().with(Region::MenuBar).with(Region::ToolBar);
        let shell = assemble(window(), regions, &library).expect("an assembled shell");
        assert!(shell.regions.has(Region::MenuBar));
        assert!(shell.regions.has(Region::ToolBar));
        assert!(!shell.regions.has(Region::Dock));
        assert_eq!(shell.format(), FORMAT);
        let report = shell.render().join("\n");
        assert!(report.contains("region menu"), "{report}");
        assert!(report.contains("region toolbar"), "{report}");
    }

    #[test]
    fn a_gapped_menu_refuses_a_requested_menu_bar_instead_of_faking_one() {
        let library = Library::provided(target())
            .with_menu(Capability::gap("native menu", "the target has no menu API"));
        let regions = Regions::none().with(Region::MenuBar);
        let refusal = assemble(window(), regions, &library).expect_err("a refusal");
        assert!(refusal.contains("gap: native menu"), "{refusal}");
        assert!(refusal.contains("the target has no menu API"), "{refusal}");
    }

    #[test]
    fn a_gapped_menu_still_allows_a_tool_bar_which_the_app_draws_itself() {
        let library = Library::provided(target())
            .with_menu(Capability::gap("native menu", "the target has no menu API"));
        let regions = Regions::none()
            .with(Region::ToolBar)
            .with(Region::StatusBar);
        let shell = assemble(window(), regions, &library).expect("an assembled shell");
        assert!(shell.regions.has(Region::ToolBar));
        assert!(shell.regions.has(Region::StatusBar));
    }

    #[test]
    fn a_gapped_dock_refuses_a_requested_dock() {
        let library = Library::provided(target()).with_dock(Capability::gap(
            "docking",
            "the target is a single-window device",
        ));
        let regions = Regions::none().with(Region::Dock);
        let refusal = assemble(window(), regions, &library).expect_err("a refusal");
        assert!(refusal.contains("gap: docking"), "{refusal}");
    }

    #[test]
    fn an_assembled_shell_is_the_structural_data_it_was_built_from() {
        let library = Library::provided(target());
        let shell = assemble(window(), Regions::all(), &library).expect("an assembled shell");
        assert_eq!(
            shell,
            Shell {
                window: window(),
                regions: Regions::all(),
                menu: Capability::Provided,
                dock: Capability::Provided,
            }
        );
    }
}
