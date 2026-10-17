// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Docking, split layout and persisted layout as a target capability (task T26.07).
//!
//! The acceptance line is *"only a target that supports it can generate; do not impose desktop
//! structure on a restricted target"*. The design makes that a property of generation, not a hope:
//!
//! * **A target's support is a table, not an assumption.** [`Support::of`] reports which targets
//!   support docking and persisted layout; an unknown target supports neither, which is the safe
//!   answer rather than a guess.
//! * **Generation refuses what the target cannot do.** [`generate`] answers
//!   [`Unsupported { target, reason }`](Unsupported) for a docked layout on a target that has no
//!   docking — a *distinct* outcome from success, so a restricted target never silently receives
//!   desktop structure it cannot show. Persistence is refused the same way.
//! * **Single and split layouts are not desktop structure.** Every target can show them, so they
//!   generate everywhere and the restriction applies only where it is real.

#![allow(dead_code)]

/// Whether a target supports the desktop-structure features.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Support {
    /// The target's name.
    pub target: String,
    /// Whether it supports docked panels.
    pub docking: bool,
    /// Whether it supports a persisted layout.
    pub persisted_layout: bool,
}

impl Support {
    /// The support of a target, by name.
    ///
    /// A desktop target supports both; a tablet supports persistence but not docking; the smaller
    /// targets support neither; an unknown target supports neither, because an unknown capability is
    /// not a supported one.
    pub fn of(target: &str) -> Self {
        let (docking, persisted_layout) = match target {
            "desktop" => (true, true),
            "tablet" => (false, true),
            _ => (false, false),
        };
        Self {
            target: target.to_owned(),
            docking,
            persisted_layout,
        }
    }
}

/// A split's direction.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Orientation {
    /// Side by side.
    Horizontal,
    /// Stacked.
    Vertical,
}

impl Orientation {
    /// The orientation's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Horizontal => "horizontal",
            Self::Vertical => "vertical",
        }
    }

    /// Reads an orientation name, or `None` when it is not one.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "horizontal" => Some(Self::Horizontal),
            "vertical" => Some(Self::Vertical),
            _ => None,
        }
    }
}

/// A docked panel.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Panel {
    /// The panel's name.
    pub name: String,
}

impl Panel {
    /// A panel, refusing a blank name.
    pub fn new(name: &str) -> Result<Self, String> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err("a panel needs a name".to_owned());
        }
        Ok(Self {
            name: trimmed.to_owned(),
        })
    }
}

/// The layout the designer asked the target to generate.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Layout {
    /// One view, no desktop structure.
    Single,
    /// A split of panes, which every target can show.
    Split {
        /// The direction.
        orientation: Orientation,
        /// The panes.
        panes: Vec<Layout>,
    },
    /// Docked panels, which only a supporting target can show.
    Docked {
        /// The panels.
        panels: Vec<Panel>,
    },
}

/// A target that cannot generate the requested structure, and why.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Unsupported {
    /// The target.
    pub target: String,
    /// The reason it cannot.
    pub reason: String,
}

impl Unsupported {
    /// The refusal in one line.
    pub fn render(&self) -> String {
        format!("unsupported on {}: {}", self.target, self.reason)
    }
}

/// What generation produced, as structural lines.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Plan {
    /// The plan's lines.
    pub lines: Vec<String>,
}

impl Plan {
    /// The plan's lines.
    pub fn render(&self) -> &[String] {
        &self.lines
    }
}

/// Generates a layout for a target, refusing structure the target does not support.
pub fn generate(support: &Support, layout: &Layout, persist: bool) -> Result<Plan, Unsupported> {
    if persist && !support.persisted_layout {
        return Err(Unsupported {
            target: support.target.clone(),
            reason: "the target does not support a persisted layout".to_owned(),
        });
    }
    let mut lines = Vec::new();
    walk(support, layout, 0, &mut lines)?;
    if persist {
        lines.push("persist: layout is remembered across launches".to_owned());
    }
    Ok(Plan { lines })
}

fn walk(
    support: &Support,
    layout: &Layout,
    depth: usize,
    lines: &mut Vec<String>,
) -> Result<(), Unsupported> {
    let indent = "  ".repeat(depth);
    match layout {
        Layout::Single => lines.push(format!("{indent}single")),
        Layout::Split { orientation, panes } => {
            lines.push(format!("{indent}split {}", orientation.name()));
            for pane in panes {
                walk(support, pane, depth + 1, lines)?;
            }
        }
        Layout::Docked { panels } => {
            if !support.docking {
                return Err(Unsupported {
                    target: support.target.clone(),
                    reason: "the target does not support docked panels".to_owned(),
                });
            }
            lines.push(format!("{indent}docked"));
            for panel in panels {
                lines.push(format!("{indent}  panel {}", panel.name));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Layout, Orientation, Panel, Support, generate};

    #[test]
    fn the_support_table_says_what_each_target_can_do() {
        assert!(Support::of("desktop").docking);
        assert!(Support::of("desktop").persisted_layout);
        assert!(!Support::of("tablet").docking);
        assert!(Support::of("tablet").persisted_layout);
        assert!(!Support::of("mobile").docking);
        assert!(!Support::of("mobile").persisted_layout);
        assert!(!Support::of("mystery").docking);
        assert!(!Support::of("mystery").persisted_layout);
    }

    #[test]
    fn an_orientation_is_one_of_two() {
        assert_eq!(
            Orientation::parse("horizontal"),
            Some(Orientation::Horizontal)
        );
        assert_eq!(Orientation::parse("vertical"), Some(Orientation::Vertical));
        assert_eq!(Orientation::parse("diagonal"), None);
    }

    #[test]
    fn a_panel_needs_a_name() {
        assert!(Panel::new("  ").is_err());
        assert_eq!(
            Panel::new(" inspector ").expect("a panel").name,
            "inspector"
        );
    }

    #[test]
    fn a_desktop_target_generates_docked_panels() {
        let support = Support::of("desktop");
        let layout = Layout::Docked {
            panels: vec![Panel::new("inspector").expect("a panel")],
        };
        let plan = generate(&support, &layout, false).expect("a plan");
        let report = plan.render().join("\n");
        assert!(report.contains("docked"), "{report}");
        assert!(report.contains("panel inspector"), "{report}");
    }

    #[test]
    fn a_restricted_target_reports_unsupported_rather_than_receiving_desktop_structure() {
        let support = Support::of("mobile");
        let layout = Layout::Docked {
            panels: vec![Panel::new("inspector").expect("a panel")],
        };
        let refusal = generate(&support, &layout, false).expect_err("a refusal");
        assert_eq!(refusal.target, "mobile");
        assert!(refusal.render().contains("unsupported on mobile"));
        assert!(refusal.reason.contains("docked panels"));
    }

    #[test]
    fn an_unknown_target_does_not_receive_docking() {
        let support = Support::of("mystery");
        let layout = Layout::Docked { panels: Vec::new() };
        assert!(generate(&support, &layout, false).is_err());
    }

    #[test]
    fn a_restricted_target_still_generates_a_single_or_split_layout() {
        let support = Support::of("mobile");
        assert!(generate(&support, &Layout::Single, false).is_ok());
        let split = Layout::Split {
            orientation: Orientation::Horizontal,
            panes: vec![Layout::Single, Layout::Single],
        };
        let plan = generate(&support, &split, false).expect("a plan");
        assert!(plan.render().join("\n").contains("split horizontal"));
    }

    #[test]
    fn persisting_a_layout_needs_the_target_to_support_it() {
        let support = Support::of("mobile");
        let refusal = generate(&support, &Layout::Single, true).expect_err("a refusal");
        assert!(refusal.reason.contains("persisted layout"));

        let desktop = Support::of("desktop");
        let plan = generate(&desktop, &Layout::Single, true).expect("a plan");
        assert!(plan.render().join("\n").contains("persist:"));
    }

    #[test]
    fn a_nested_dock_inside_a_split_is_still_refused_on_a_restricted_target() {
        let support = Support::of("mini");
        let layout = Layout::Split {
            orientation: Orientation::Vertical,
            panes: vec![
                Layout::Single,
                Layout::Docked {
                    panels: vec![Panel::new("outline").expect("a panel")],
                },
            ],
        };
        assert!(generate(&support, &layout, false).is_err());
    }
}
