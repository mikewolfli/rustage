// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! A real icon toolbar: the first host consumer of the icon seam.
//!
//! # Why this exists
//!
//! Until now the host's icons had only ever been rendered by the resource tooling, which drew the
//! *authoring geometry* into review sheets. That proves a glyph is a glyph; it does not prove that
//! a **control** consumes it, that the pinned dependency's flattener accepts it, or that a theme
//! switch reaches it. This module builds the controls for real — `ToolButton` for tools, `Icon`
//! for role-bearing status glyphs — assigns the seam's sources to them, and lays them out as a
//! strip that a window and the baseline exporter both draw.
//!
//! # The two content channels, and why both are here
//!
//! The dependency carries a compiled-in icon two ways, and the strip exercises both so neither can
//! regress unnoticed:
//!
//! * [`HostIconSource::Builtin`](crate::resources::icons::HostIconSource::Builtin) — a token of
//!   the dependency's own set, drawn from its outlines.
//! * [`HostIconSource::StaticGeometry`](crate::resources::icons::HostIconSource::StaticGeometry) —
//!   rustage's own glyphs, emitted as polygon geometry on the dependency's grid and drawn through
//!   the same flattener a built-in uses.
//!
//! # Where the colour comes from, per control
//!
//! The two controls do **not** have the same colour channel, and pretending they do is the "one
//! API for every consumer" claim the resource plan rejects:
//!
//! * A `ToolButton` paints its icon with the control's **text colour** — the theme's resolved one,
//!   or one the caller set for the whole control — and with its disabled ink when the control is
//!   disabled. There is no icon-colour setter, so a role cannot be assigned to it; the host
//!   chooses the geometry and the control's own style decides the ink. That is the toolbar case.
//! * An `Icon` widget resolves its own style colour, so the host assigns the role's colour to it
//!   explicitly through [`IconRole::color`]. That is how all eight semantic roles — including the
//!   ones a toolbar button has no reason to paint, such as `OnAccent` or `Danger` — get a consumer.
//!
//! # Theme changes
//!
//! A role colour is resolved *from* the theme and then stored on the control, so a theme switch
//! leaves a stale value behind unless something re-resolves it. The strip subscribes to the
//! manager's `theme_changed` signal, raises a flag, and re-applies on the next refresh; see
//! [`ToolbarSample::refresh_if_stale`]. Without that, the light and dark renders of a glyph would
//! differ only in the backdrop — the defect the dependency's own SVG exporter once shipped, and
//! wrote a test for.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rust_widgets::core::{Color, Rect, Size};
use rust_widgets::render::RenderContext;
use rust_widgets::signal::ConnectionHandle;
use rust_widgets::theme::{self, AppearanceMode, HighContrastMode};
use rust_widgets::widget::Draw;
use rust_widgets::widget::{Icon, ToolButton, Widget};

use crate::resources::icons::{self, IconResolutionError, IconRole};

/// Side of a tool button and of a standalone status glyph's box.
const SLOT: u32 = 32;
/// Icon size inside a slot.
const ICON: f32 = 20.0;
/// Gap between two controls in the same group.
const GAP: u32 = 4;
/// Gap between two groups.
const GROUP_GAP: u32 = 14;
/// Padding at both ends and above and below the strip.
const PADDING: u32 = 6;

/// One tool: the dependency's toolbar button carrying the icon the host assigned to it.
struct Tool {
    button: ToolButton,
    /// Whether the assigned source is the host's own geometry rather than a built-in token.
    geometry: bool,
}

/// One standalone status glyph with the role it is painted in.
struct Glyph {
    widget: Icon,
    role: IconRole,
    /// Whether the glyph sits on the accent surface, which is what `OnAccent` is defined against.
    on_accent: bool,
}

/// One entry of the tool strip, in painting order.
struct ToolPlan {
    id: &'static str,
    /// A group boundary is left before this tool.
    new_group: bool,
    disabled: bool,
}

/// The tool strip, in order: file, edit, design, build, debug, tools.
const TOOL_PLAN: [ToolPlan; 17] = [
    ToolPlan {
        id: "host/file/new",
        new_group: false,
        disabled: false,
    },
    ToolPlan {
        id: "host/file/open-project",
        new_group: false,
        disabled: false,
    },
    ToolPlan {
        id: "host/file/save",
        new_group: false,
        disabled: false,
    },
    ToolPlan {
        id: "host/edit/clipboard",
        new_group: true,
        disabled: false,
    },
    ToolPlan {
        id: "host/edit/find",
        new_group: false,
        disabled: false,
    },
    ToolPlan {
        id: "host/design/canvas",
        new_group: true,
        disabled: false,
    },
    ToolPlan {
        id: "host/design/layout",
        new_group: false,
        disabled: false,
    },
    ToolPlan {
        id: "host/design/component",
        new_group: false,
        disabled: false,
    },
    ToolPlan {
        id: "host/design/event",
        new_group: false,
        disabled: false,
    },
    ToolPlan {
        id: "host/design/flow",
        new_group: false,
        disabled: false,
    },
    ToolPlan {
        id: "host/design/theme",
        new_group: false,
        disabled: false,
    },
    ToolPlan {
        id: "host/build/pipeline",
        new_group: true,
        disabled: false,
    },
    ToolPlan {
        id: "host/test/flask",
        new_group: false,
        disabled: false,
    },
    ToolPlan {
        id: "host/debug/bug",
        new_group: true,
        disabled: false,
    },
    ToolPlan {
        id: "host/debug/breakpoint",
        new_group: false,
        disabled: false,
    },
    ToolPlan {
        id: "host/tools/terminal",
        new_group: false,
        disabled: true,
    },
    ToolPlan {
        id: "host/tools/ai",
        new_group: true,
        disabled: false,
    },
];

/// The role glyphs, one per role, with the icon each is painted as.
///
/// Three entries deliberately paint a role the icon does not declare, because a role is a
/// *presentation* decision: a successful test report and a disabled setting are ordinary icons
/// shown in an exceptional state.
const GLYPH_PLAN: [(IconRole, &str); 8] = [
    (IconRole::Default, "host/design/canvas"),
    (IconRole::Muted, "host/tasks/queue"),
    (IconRole::Accent, "host/tools/ai"),
    (IconRole::OnAccent, "host/design/component"),
    (IconRole::Disabled, "host/tools/settings"),
    (IconRole::Success, "host/test/report"),
    (IconRole::Warning, "host/nav/issue"),
    (IconRole::Danger, "host/debug/breakpoint"),
];

/// What went into the strip, so a caller and a test can state what was built instead of guessing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolbarComposition {
    /// Tools in the strip.
    pub tools: usize,
    /// How many of them draw rustage's own geometry rather than a built-in token.
    pub static_geometry_tools: usize,
    /// Standalone role glyphs, one per semantic role.
    pub role_glyphs: usize,
    /// The strip's width in logical pixels.
    pub width: u32,
    /// The strip's height in logical pixels.
    pub height: u32,
}

/// The sample strip: a row of tools followed by one glyph per semantic role.
pub struct ToolbarSample {
    tools: Vec<Tool>,
    glyphs: Vec<Glyph>,
    stale: Arc<AtomicBool>,
    subscription: Option<ConnectionHandle>,
    size: Size,
}

impl ToolbarSample {
    /// Builds the strip, resolving every icon through the seam.
    ///
    /// # Errors
    ///
    /// A plan entry naming an id the manifest does not declare, or a built-in token the pinned
    /// dependency has dropped, is reported rather than skipped: a silently shorter toolbar would
    /// look like a working sample until someone counted the buttons.
    pub fn new() -> Result<Self, IconResolutionError> {
        let mut x = PADDING;
        let mut tools = Vec::with_capacity(TOOL_PLAN.len());
        for plan in &TOOL_PLAN {
            let icon = icons::resolve(plan.id)?;
            if plan.new_group {
                x += GROUP_GAP;
            }
            let mut button = ToolButton::new("", Rect::new(x as i32, PADDING as i32, SLOT, SLOT));
            button.set_icon_source(Some(icon.source()));
            button.set_enabled(!plan.disabled);
            tools.push(Tool {
                button,
                geometry: icon.geometry().is_some(),
            });
            x += SLOT + GAP;
        }
        x += GROUP_GAP - GAP;

        let mut glyphs = Vec::with_capacity(GLYPH_PLAN.len());
        for (role, id) in GLYPH_PLAN {
            let icon = icons::resolve(id)?;
            let mut widget = Icon::new(Rect::new(x as i32, PADDING as i32, SLOT, SLOT));
            widget.set_icon_source(icon.source());
            widget.set_size(ICON);
            glyphs.push(Glyph {
                widget,
                role,
                on_accent: role == IconRole::OnAccent,
            });
            x += SLOT + GAP;
        }
        x += PADDING - GAP;

        let stale = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stale);
        // The closure only raises a flag: taking the manager lock inside it would deadlock, since
        // the signal is emitted while the manager is already borrowed.
        let subscription = Some(
            theme::global_theme_manager()
                .on_theme_changed()
                .connect(move |_| flag.store(true, Ordering::Relaxed)),
        );

        let mut sample = Self {
            tools,
            glyphs,
            stale,
            subscription,
            size: Size::new(x, PADDING * 2 + SLOT),
        };
        // The ink is assigned before the first draw, so the opening frame is styled; a sample whose
        // colours only appeared after a theme switch would render unstyled once.
        sample.apply_theme();
        Ok(sample)
    }

    /// What the strip contains.
    pub fn composition(&self) -> ToolbarComposition {
        ToolbarComposition {
            tools: self.tools.len(),
            static_geometry_tools: self.tools.iter().filter(|tool| tool.geometry).count(),
            role_glyphs: self.glyphs.len(),
            width: self.size.width,
            height: self.size.height,
        }
    }

    /// The strip's size in logical pixels.
    pub const fn size(&self) -> Size {
        self.size
    }

    /// The tools in painting order, so a caller can drive states it owns.
    pub fn tools_mut(&mut self) -> impl Iterator<Item = &mut ToolButton> {
        self.tools.iter_mut().map(|tool| &mut tool.button)
    }

    /// The role glyphs and their roles, in painting order.
    pub fn glyphs_mut(&mut self) -> impl Iterator<Item = (&mut Icon, IconRole)> {
        self.glyphs
            .iter_mut()
            .map(|glyph| (&mut glyph.widget, glyph.role))
    }

    /// Re-resolves every role colour from the active theme and restyles every control with it.
    ///
    /// This is the refresh half of the invalidation contract. It is safe to call at any time —
    /// the exit state is a pure function of the active theme — so the flag is an optimisation
    /// rather than a correctness requirement.
    pub fn apply_theme(&mut self) {
        for tool in &mut self.tools {
            theme::apply_theme_to_widget(&mut tool.button);
        }
        for glyph in &mut self.glyphs {
            if let Some(color) = icons::role_color(glyph.role) {
                glyph.widget.set_color(color);
            }
            theme::apply_theme_to_widget(&mut glyph.widget);
        }
    }

    /// Applies a pending theme change, reporting whether one was pending.
    ///
    /// The flag is cleared **before** the refresh, so a theme switch that lands during the refresh
    /// is not lost by having been observed on the way in.
    pub fn refresh_if_stale(&mut self) -> bool {
        if !self.stale.swap(false, Ordering::Relaxed) {
            return false;
        }
        self.apply_theme();
        true
    }
}

impl Draw for ToolbarSample {
    fn draw(&mut self, context: &mut RenderContext) {
        let plate = accent_surface();
        for glyph in &mut self.glyphs {
            if glyph.on_accent {
                // A plate, so the glyph is read the way the role is defined: on the accent colour
                // rather than on the window background.
                context.fill_rect(glyph.widget.geometry(), plate);
            }
        }
        for tool in &mut self.tools {
            tool.button.draw(context);
        }
        for glyph in &mut self.glyphs {
            glyph.widget.draw(context);
        }
    }
}

impl Drop for ToolbarSample {
    fn drop(&mut self) {
        if let Some(handle) = self.subscription.take() {
            theme::global_theme_manager()
                .on_theme_changed()
                .disconnect(handle);
        }
    }
}

/// Renders the strip to an SVG document sized to it, on the active theme's backdrop.
///
/// The dependency's SVG backend serialises the same render commands the window backends execute,
/// so this is a real-backend rendering rather than a picture of the authoring geometry. It is what
/// the committed baselines under `tests/baselines/icons/` are produced from.
pub fn render_svg(sample: &mut ToolbarSample) -> String {
    let size = sample.size();
    rust_widgets::widget::svg::render_widget_to_svg_on(
        sample,
        Rect::new(0, 0, size.width, size.height),
        backdrop(),
    )
}

/// The surface the strip is composited on: the forced background under a high-contrast override,
/// otherwise the active theme's own background.
pub fn backdrop() -> Color {
    let manager = theme::global_theme_manager();
    if let Some((background, _)) = manager.high_contrast().forced_pair() {
        return background;
    }
    manager
        .current_theme()
        .map_or(Color::WHITE, |theme| theme.colors.background)
}

/// The accent plate colour, forced to the background under a high-contrast override.
///
/// One guard, no nesting: the manager's mutex is not re-entrant, so this cannot call
/// [`backdrop`] while it holds the guard — doing so deadlocks the thread.
fn accent_surface() -> Color {
    let manager = theme::global_theme_manager();
    if let Some((background, _)) = manager.high_contrast().forced_pair() {
        return background;
    }
    manager
        .current_theme()
        .map_or(Color::WHITE, |theme| theme.colors.primary)
}

/// One committed baseline render of the strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BaselineRender {
    /// The file stem the render is written under.
    pub name: &'static str,
    /// The appearance the render is taken in.
    pub appearance: AppearanceMode,
    /// The high-contrast override in force, if any.
    pub high_contrast: HighContrastMode,
    /// Whether every tool starts disabled.
    pub disabled: bool,
}

/// The baseline renders: the two appearances, the disabled state and the forced-colour state.
///
/// Four, not one, because each answers a question the others cannot. The light/dark pair is what
/// proves a theme switch reaches the glyphs rather than only the backdrop; the disabled render is
/// the only one that shows the dependency's disabled ink; and the forced-colour render is what
/// proves an accessibility override is not overridden back by the host's own role colours.
pub const BASELINE_RENDERS: [BaselineRender; 4] = [
    BaselineRender {
        name: "strip-light",
        appearance: AppearanceMode::Light,
        high_contrast: HighContrastMode::None,
        disabled: false,
    },
    BaselineRender {
        name: "strip-dark",
        appearance: AppearanceMode::Dark,
        high_contrast: HighContrastMode::None,
        disabled: false,
    },
    BaselineRender {
        name: "strip-dark-disabled",
        appearance: AppearanceMode::Dark,
        high_contrast: HighContrastMode::None,
        disabled: true,
    },
    BaselineRender {
        name: "strip-dark-high-contrast",
        appearance: AppearanceMode::Dark,
        high_contrast: HighContrastMode::BlackOnWhite,
        disabled: false,
    },
];

/// Produces one baseline render.
///
/// # Errors
///
/// Propagates an icon the manifest declares but the pinned dependency cannot resolve, so a
/// baseline is never silently written from a strip with a missing button.
pub fn render_baseline(render: BaselineRender) -> Result<String, IconResolutionError> {
    // Scoped, because building the strip subscribes to the manager's signal and therefore takes
    // the same lock; the manager's mutex is not re-entrant.
    {
        let mut manager = theme::global_theme_manager();
        manager.set_high_contrast(render.high_contrast);
        manager.set_appearance(render.appearance);
    }
    let mut sample = ToolbarSample::new()?;
    if render.disabled {
        for tool in sample.tools_mut() {
            tool.set_enabled(false);
        }
        sample.apply_theme();
    }
    Ok(decorate(render, &render_svg(&mut sample)))
}

/// Prefixes a rendered strip with the provenance a committed baseline needs.
///
/// The document beneath the comment is left byte for byte as the backend produced it, so a diff
/// between two baselines is a diff of the drawing. The marker is there so that a reviewer — and a
/// future tool — can tell a generated baseline from a hand-written file, and so that anyone
/// tempted to "fix" one by editing it is told which command recreates it.
fn decorate(render: BaselineRender, svg: &str) -> String {
    format!(
        "<!-- GENERATED by `cargo run -- --render-icons tests/baselines/icons` -->\n\
         <!-- {}: appearance {:?}, high contrast {:?}, all tools disabled: {} -->\n{}",
        render.name, render.appearance, render.high_contrast, render.disabled, svg
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::fs;
    use std::path::PathBuf;

    use rust_widgets::core::{Color, Rect};
    use rust_widgets::theme::{self, AppearanceMode, HighContrastMode};
    use rust_widgets::widget::{Icon, IconRef, IconSource, ToolButton, measure_icon};

    use super::{BASELINE_RENDERS, ToolbarSample, render_baseline, render_svg};
    use crate::resources::icons::{self, IconRole};

    /// Where the committed baselines live, relative to the crate root.
    const BASELINE_DIR: &str = "tests/baselines/icons";

    /// Serialises the tests that switch the process-wide theme, and returns the guard.
    ///
    /// The guard is held for the whole test: unit tests in one binary run in parallel threads, and
    /// the theme registry is process-wide, so without it one test's appearance would leak into
    /// another's assertion.
    fn themed() -> rust_widgets::compat::MutexGuard<'static, ()> {
        theme::theme_test_guard()
    }

    /// Resets the process-wide theme to a known state.
    fn reset_theme() {
        let mut manager = theme::global_theme_manager();
        manager.set_high_contrast(HighContrastMode::None);
        manager.set_appearance(AppearanceMode::Light);
    }

    /// The strip is built from real controls on both channels, and every role has a consumer.
    ///
    /// The role coverage is the point: a mapping that resolved eight roles and painted six of them
    /// would look finished and would leave two roles unverified.
    #[test]
    fn the_strip_covers_both_channels_and_every_role() {
        let _guard = themed();
        reset_theme();
        let mut sample = ToolbarSample::new().expect("every planned icon resolves");

        let composition = sample.composition();
        assert_eq!(composition.tools, 17);
        assert!(
            composition.static_geometry_tools >= 8,
            "the strip must exercise rustage's own geometry, not only built-in tokens: {}",
            composition.static_geometry_tools
        );
        assert!(
            composition.static_geometry_tools < composition.tools,
            "and built-in tokens too"
        );

        let painted: Vec<IconRole> = sample.glyphs_mut().map(|(_, role)| role).collect();
        let distinct: HashSet<IconRole> = painted.iter().copied().collect();
        assert_eq!(distinct.len(), IconRole::ALL.len());
        for role in IconRole::ALL {
            assert!(
                distinct.contains(&role),
                "{role} has no consumer in the strip"
            );
        }

        // Both controls really hold a source: a plan that resolved an icon and then never assigned
        // it would still count as a composition.
        assert!(sample.tools_mut().all(|tool| tool.icon_source().is_some()));
        assert!(
            sample
                .glyphs_mut()
                .all(|(widget, _)| widget.icon_source().is_some())
        );
    }

    /// A theme switch changes what the strip draws, and the subscription is what applies it.
    #[test]
    fn a_theme_switch_reaches_the_controls() {
        let _guard = themed();
        reset_theme();
        let mut sample = ToolbarSample::new().expect("resolves");

        let light = render_svg(&mut sample);
        assert!(!sample.refresh_if_stale(), "nothing should be pending yet");

        {
            let mut manager = theme::global_theme_manager();
            assert!(manager.set_appearance(AppearanceMode::Dark));
        }
        assert!(
            sample.refresh_if_stale(),
            "the theme_changed signal must have been observed by the strip's subscription"
        );
        let dark = render_svg(&mut sample);
        assert_ne!(light, dark, "the two appearances rendered identically");
        assert!(
            !sample.refresh_if_stale(),
            "the flag is cleared by the refresh"
        );

        // The role inks specifically, not only the backdrop: two renders can differ by the window
        // colour alone while every glyph kept the ink it was assigned before the switch. The ink
        // lives on the control, so that is where it is checked.
        for (widget, role) in sample.glyphs_mut() {
            assert_eq!(
                widget.color(),
                icons::role_color(role).expect("a theme is active"),
                "{role} holds a stale ink after the refresh"
            );
        }

        reset_theme();
    }

    /// A theme switch that is not consumed by a refresh leaves stale ink on the controls.
    ///
    /// This is the half of the contract that a passing "it renders" test cannot show: the role ink
    /// is *stored* on the control, so the flag is what makes the stored value correct.
    #[test]
    fn skipping_the_refresh_leaves_the_previous_ink() {
        let _guard = themed();
        reset_theme();
        let mut sample = ToolbarSample::new().expect("resolves");
        let before = render_svg(&mut sample);

        {
            let mut manager = theme::global_theme_manager();
            assert!(manager.set_appearance(AppearanceMode::Dark));
        }
        // Deliberately no refresh: the glyphs keep the light appearance's explicit ink while the
        // control chrome (which the dependency resolves per draw) has already followed.
        let stale = render_svg(&mut sample);
        assert_ne!(before, stale, "the chrome follows the theme on its own");
        assert!(
            sample.refresh_if_stale(),
            "and the pending change is still recorded"
        );

        reset_theme();
    }

    /// The disabled render is a different drawing, and it is the only one that shows the
    /// dependency's disabled ink.
    #[test]
    fn the_disabled_state_is_visibly_its_own() {
        let _guard = themed();
        reset_theme();
        let enabled = render_baseline(BASELINE_RENDERS[1]).expect("resolves");
        let disabled = render_baseline(BASELINE_RENDERS[2]).expect("resolves");
        assert_ne!(enabled, disabled, "disabling every tool changed nothing");

        let disabled_ink = theme::Theme::dark().colors.disabled;
        let needle = format!(
            "rgba({},{},{},1.00)",
            disabled_ink.r, disabled_ink.g, disabled_ink.b
        );
        assert!(
            disabled.contains(&needle),
            "the disabled render must use the theme's disabled ink {needle}"
        );
        reset_theme();
    }

    /// A forced-colour mode is not overridden back by the host's own role inks.
    #[test]
    fn the_forced_colour_render_uses_only_the_forced_pair() {
        let _guard = themed();
        reset_theme();
        let forced = render_baseline(BASELINE_RENDERS[3]).expect("resolves");

        let accent = theme::Theme::dark().colors.primary;
        let accent_needle = format!("rgba({},{},{},1.00)", accent.r, accent.g, accent.b);
        assert!(
            !forced.contains(&accent_needle),
            "the accent plate survived the high-contrast override: the host's own role colours \
             must not win over a mode the user selected"
        );
        let danger = theme::Theme::dark().colors.error;
        let danger_needle = format!("rgba({},{},{},1.00)", danger.r, danger.g, danger.b);
        assert!(
            !forced.contains(&danger_needle),
            "a semantic role survived the override"
        );
        reset_theme();
    }

    /// The committed baselines are what the current controls draw, byte for byte.
    ///
    /// The message names the producer rather than the test: a diff here is a change to how the
    /// icons draw, and it has to be reviewed as a picture before the baseline is replaced.
    #[test]
    fn the_committed_baselines_match_the_rendered_strip() {
        let _guard = themed();
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(BASELINE_DIR);
        for render in BASELINE_RENDERS {
            let path = root.join(format!("{}.svg", render.name));
            let committed = fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("{} could not be read: {error}", path.display()));
            let drawn = render_baseline(render).expect("resolves");
            assert_eq!(
                drawn,
                committed,
                "{} is stale: after reviewing the difference, regenerate it with \
                 `cargo run -- --render-icons tests/baselines/icons`",
                path.display()
            );
        }
        reset_theme();
    }

    /// Every declared icon is drawn by a real control, not only by the resource tooling.
    ///
    /// This is the check the earlier rounds could not make at all. Each icon gets its own `Icon`
    /// widget, which paints through the same `paint_icon` a control uses, so a glyph the seam
    /// resolves but a control cannot draw is reported here.
    #[test]
    fn every_declared_icon_is_drawn_by_a_control() {
        let _guard = themed();
        reset_theme();
        let mut drawn = 0usize;
        for record in icons::icons() {
            let icon = icons::resolve(record.id).expect("resolves");
            let mut widget = Icon::new(Rect::new(0, 0, 24, 24));
            widget.set_icon_source(icon.source());
            widget.set_size(24.0);
            let svg = rust_widgets::widget::svg::render_widget_to_svg_on(
                &mut widget,
                Rect::new(0, 0, 24, 24),
                Color::WHITE,
            );
            assert!(
                svg.contains("<path"),
                "{} rendered no geometry through a control",
                record.id
            );
            drawn += 1;
        }
        assert_eq!(drawn, icons::icons().len());
    }

    /// A glyph occupies the box the control gives it, so scaling is the control's decision.
    ///
    /// Measured through the dependency's own `measure_icon`, which is the same derivation the
    /// painter uses, so a reserved box and a painted glyph cannot disagree.
    #[test]
    fn a_glyph_follows_the_slot_it_is_given() {
        let icon = icons::resolve("host/design/canvas").expect("resolves");
        let mut previous = None;
        for side in [16u32, 24, 32, 48] {
            let slot = Rect::new(10, 10, side, side);
            let metrics = measure_icon(IconRef::from(icon.source()), slot).expect("measurable");
            assert!(!metrics.is_empty(), "a {side}px slot measured empty");
            assert_eq!(
                metrics.source_extent, 24,
                "the glyph's design grid must travel with it"
            );
            if let Some((previous_side, previous_width)) = previous {
                assert!(
                    metrics.bounds.width > previous_width,
                    "the glyph did not grow from {previous_side}px to {side}px"
                );
            }
            previous = Some((side, metrics.bounds.width));
        }
    }

    /// The icon a control holds is the one the seam resolved, for both channels.
    #[test]
    fn a_tool_holds_the_source_the_seam_resolved() {
        let _guard = themed();
        reset_theme();
        let mut sample = ToolbarSample::new().expect("resolves");
        let sources: Vec<Option<IconSource>> =
            sample.tools_mut().map(|tool| tool.icon_source()).collect();
        assert!(
            sources
                .iter()
                .any(|source| matches!(source, Some(IconSource::Builtin(_))))
        );
        assert!(
            sources
                .iter()
                .any(|source| matches!(source, Some(IconSource::StaticGeometry(_))))
        );
        // A tool whose geometry resolved to nothing would leave an empty slot in a toolbar that
        // otherwise looks complete.
        assert!(sources.iter().all(Option::is_some));
    }

    /// The role ink survives the theme merge, which is what makes the refresh worth doing.
    ///
    /// `Icon` is the control the strip assigns role ink to, and the dependency merges the theme's
    /// style *under* an explicit value. If the merge clobbered it, the refresh would be a no-op and
    /// the light/dark renders would differ only by the backdrop.
    #[test]
    fn an_icon_keeps_the_role_ink_the_refresh_applies() {
        let _guard = themed();
        reset_theme();
        let icon = icons::resolve("host/design/event").expect("resolves");
        let source = icon.source();

        let mut button = ToolButton::new("", Rect::new(0, 0, 32, 32));
        button.set_icon_source(Some(source));
        let mut widget = Icon::new(Rect::new(0, 0, 32, 32));
        widget.set_icon_source(source);
        widget.set_size(24.0);

        // `ToolButton` has no icon-colour channel: its glyph is painted with the control's text
        // colour. `Icon` does, and that is the channel a role is assigned through.
        assert!(
            button.icon_rect().is_some(),
            "a tool reserves the box its icon is fitted into"
        );
        assert!(
            widget.explicit_color().is_none(),
            "an icon starts out following the theme"
        );

        let ink = icons::role_color(IconRole::Warning).expect("a theme is active");
        widget.set_color(ink);
        theme::apply_theme_to_widget(&mut widget);
        assert_eq!(
            widget.explicit_color(),
            Some(ink),
            "the merge must not replace the role ink"
        );
        assert_eq!(widget.color(), ink);

        // A role applies the dependency's contrast step, so in the light appearance its ink is
        // deliberately not the theme's raw token: the warning yellow measures 1.43:1 against the
        // light background, and the role is what makes it legible without losing the hue.
        let theme = theme::Theme::default();
        assert_ne!(
            ink, theme.colors.warning,
            "the light warning token needs the legibility step"
        );
        assert!(ink.contrast_ratio(theme.colors.background) >= 3.0);
    }
}
