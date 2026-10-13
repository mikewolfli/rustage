// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The host icon seam: the layer between rustage's semantic icon vocabulary and the pinned
//! `rust-widgets` icon API.
//!
//! # What this module is responsible for
//!
//! * **Semantic identity.** A stable host id (`host/design/canvas`) resolves to exactly one icon,
//!   and an unknown id is an error rather than a silent fallback to an unrelated glyph.
//! * **Channel selection.** The dependency carries a compiled-in icon two ways — a built-in token
//!   and borrowed static geometry — and each host icon declares which one it uses. See
//!   [`HostIconSource`].
//! * **Semantic colour.** The eight roles of `docs/plan/resources.md` section 9.1 resolve to
//!   tokens of the dependency's own theme, so a role follows the active theme instead of naming a
//!   colour of its own.
//!
//! # What it deliberately is not
//!
//! There is no loader, no file IO, no pixel cache and no registry here. The dependency's
//! dynamic-resource path (`IconSpec::Resource` with an `IconResourceResolver`, and
//! `SharedIconPixels` for the two controls that have a pixel channel) is not implemented by this
//! layer, because nothing in rustage imports a project asset yet; that arrives with the project
//! resource import work at T12.05/T12.06/T12.09, and an adapter built before its consumer exists
//! would be exactly the placeholder the principles forbid.
//!
//! Nor does this layer own layout, DPI or widget construction. A consumer asks for a [`HostIcon`]
//! and applies it to a control it owns.

use std::fmt;

use rust_widgets::core::Color;
use rust_widgets::theme::{HighContrastMode, Theme};
use rust_widgets::widget::{IconData, IconName, IconSource};

pub use super::generated_host_icons::ICON_ROLE_NAMES;

/// The contrast ratio an icon is expected to reach against the surface it is read on.
///
/// 3.0 is WCAG 2.x's threshold for graphical objects and the ratio the pinned dependency uses for
/// its own non-text indicators (`Badge` severity ink, the emoji picker's previews). Text roles in
/// the dependency use 4.5; an icon is not text, so it follows the graphics threshold.
pub const ICON_MIN_CONTRAST: f32 = 3.0;

/// A semantic icon role.
///
/// The vocabulary comes from `docs/plan/resources.md` section 9.1 and is declared in
/// `resources/host/icons/theme.json`. The names are the host's design suggestion; the *colours*
/// resolve from the active theme in [`IconRole::color`], which is what keeps this layer from
/// becoming a second palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IconRole {
    /// Primary reading ink: a default toolbar, menu or status glyph.
    Default,
    /// Low-emphasis ink for a secondary or decorative glyph.
    Muted,
    /// Emphasis ink for an active tool, a selection or the host's own brand accent.
    Accent,
    /// Ink drawn *on* the accent surface, such as the glyph of a pressed toggle button.
    OnAccent,
    /// A glyph that is present but not operable.
    Disabled,
    /// A completed, successful outcome.
    Success,
    /// Something that needs attention but is not a failure.
    Warning,
    /// A failure or an invalid state.
    Danger,
}

impl IconRole {
    /// Every role, in the order `resources/host/icons/theme.json` declares them.
    ///
    /// Enumerated so a test can walk all of them rather than sample the ones someone remembered,
    /// and so the generated `ICON_ROLE_NAMES` can be checked against the vocabulary instead of
    /// trusted.
    pub const ALL: [IconRole; 8] = [
        IconRole::Default,
        IconRole::Muted,
        IconRole::Accent,
        IconRole::OnAccent,
        IconRole::Disabled,
        IconRole::Success,
        IconRole::Warning,
        IconRole::Danger,
    ];

    /// The role's documented id, spelled as `theme.json` and the resource plan spell it.
    pub const fn as_str(self) -> &'static str {
        match self {
            IconRole::Default => "icon-default",
            IconRole::Muted => "icon-muted",
            IconRole::Accent => "icon-accent",
            IconRole::OnAccent => "icon-on-accent",
            IconRole::Disabled => "icon-disabled",
            IconRole::Success => "icon-success",
            IconRole::Warning => "icon-warning",
            IconRole::Danger => "icon-danger",
        }
    }

    /// Parses a documented role id.
    ///
    /// A role name is a host id, so an unknown one is `None` rather than a guess at the closest
    /// role: a mistyped `icon-warn` must not quietly render as `icon-default`.
    pub fn from_str(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|role| role.as_str() == name)
    }

    /// The theme surface this role is designed to be read against.
    ///
    /// `OnAccent` is the only role that is not read on the window background, which is why the
    /// surface belongs to the role rather than being a constant at the call site.
    pub fn surface(self, theme: &Theme) -> Color {
        match self {
            IconRole::OnAccent => theme.colors.primary,
            _ => theme.colors.background,
        }
    }

    /// Resolves the role to the theme's ink for it, with the dependency's contrast step applied.
    ///
    /// # Why this is more than a token lookup
    ///
    /// A theme token is chosen for its *meaning*: `warning` means "needs attention" and `primary`
    /// means "this is the accent". Neither is chosen for its luminance against the window
    /// background, and the pinned presets show the consequence — in the light appearance
    /// `warning` measures 1.43:1 and `primary` 2.74:1 against `background`, so a glyph drawn in
    /// the raw token is close to invisible. The dependency's own documented remedy for exactly
    /// this is [`Color::legible_on`], which steps the token toward black or white on the surface
    /// while keeping the hue recognisable, and which is a no-op when the token is already legible.
    /// The roles that are fine therefore keep their exact theme colour.
    ///
    /// [`IconRole::Disabled`] is deliberately excluded: a disabled glyph is *supposed* to recede,
    /// and forcing it to 3:1 would make an inoperable control look operable.
    ///
    /// [`IconRole::OnAccent`] may resolve to the same ink as a background role — in the light
    /// appearance both `icon-default` and `icon-on-accent` are black — because the two are read on
    /// different surfaces. That is not a collision to fix; the surfaces are what differ.
    pub fn color(self, theme: &Theme) -> Color {
        self.color_in(theme, HighContrastMode::None)
    }

    /// Resolves the role against a theme and an explicit high-contrast override.
    ///
    /// # Why an override flattens every role
    ///
    /// A high-contrast mode is a *forced pair*: the dependency replaces the resolved background
    /// and text colour of every control rather than tinting them, which is what makes the result
    /// predictable enough to rely on. The role colours here are resolved by this host and
    /// **assigned to the control explicitly**, so without this branch they would win over the
    /// forced pair and an accessibility mode would leave every semantic glyph at its normal ink.
    /// Keeping the roles distinct is worth less than honouring the mode the user selected.
    pub fn color_in(self, theme: &Theme, high_contrast: HighContrastMode) -> Color {
        if let Some((_, foreground)) = high_contrast.forced_pair() {
            return foreground;
        }
        let ink = match self {
            IconRole::Default => theme.colors.foreground,
            IconRole::Muted => theme.colors.secondary,
            IconRole::Accent => theme.colors.primary,
            // The dependency's own ink rule for a filled surface: black or white, decided on WCAG
            // relative luminance rather than on a byte average.
            IconRole::OnAccent => theme.colors.primary.contrast_color(),
            IconRole::Disabled => return theme.colors.disabled,
            IconRole::Success => theme.colors.success,
            IconRole::Warning => theme.colors.warning,
            IconRole::Danger => theme.colors.error,
        };
        ink.legible_on(self.surface(theme), ICON_MIN_CONTRAST)
    }
}

impl fmt::Display for IconRole {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Where a host icon's geometry comes from in the pinned dependency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostIconSource {
    /// A token of the dependency's own built-in set.
    ///
    /// Held as the token string rather than as an `IconName` variant so the generated table
    /// cannot name a variant the dependency does not have; [`HostIconSource::source`] resolves
    /// through `IconName::from_name` and reports a token the pinned revision has dropped.
    Builtin(&'static str),
    /// Borrowed static path geometry, emitted in the dependency's grid convention.
    ///
    /// This does not occupy a slot in the dependency's string-name registry, so the host's own
    /// glyphs stay out of the shared namespace entirely.
    StaticGeometry(&'static IconData),
}

impl HostIconSource {
    /// Resolves to the dependency's icon source, or reports why it cannot.
    pub fn source(self) -> Result<IconSource, IconResolutionError> {
        match self {
            HostIconSource::Builtin(token) => match IconName::from_name(token) {
                Some(name) => Ok(IconSource::builtin(name)),
                None => Err(IconResolutionError::UnknownBuiltinToken(token)),
            },
            HostIconSource::StaticGeometry(data) => Ok(IconSource::static_geometry(data)),
        }
    }
}

/// One host icon: its stable id, its content channel and its default semantic role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostIconRecord {
    /// The stable host id, `host/<segment>/<segment>`.
    pub id: &'static str,
    /// Where the geometry comes from.
    pub source: HostIconSource,
    /// The role this icon is designed for. A consumer painting another role — a disabled tool, an
    /// invalid field — asks for that role explicitly rather than rewriting this default.
    pub role: IconRole,
    /// Whether the glyph must be mirrored for right-to-left layouts.
    pub rtl_mirror: bool,
}

/// Why a host icon could not be resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IconResolutionError {
    /// No icon with that id is declared in the host manifest.
    UnknownIcon(String),
    /// The pinned dependency no longer publishes a built-in token the manifest maps.
    ///
    /// A separate variant rather than a shared "not found" because the two have different owners:
    /// an unknown id is a host bug, a missing token means the dependency moved.
    UnknownBuiltinToken(&'static str),
}

impl fmt::Display for IconResolutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IconResolutionError::UnknownIcon(id) => {
                write!(formatter, "no host icon is declared for id `{id}`")
            }
            IconResolutionError::UnknownBuiltinToken(token) => write!(
                formatter,
                "the pinned rust-widgets revision has no built-in icon token `{token}`"
            ),
        }
    }
}

impl std::error::Error for IconResolutionError {}

/// A resolved host icon: its record plus the dependency source a control can be given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostIcon {
    record: &'static HostIconRecord,
    source: IconSource,
}

impl HostIcon {
    /// The dependency source to assign to a control.
    pub const fn source(self) -> IconSource {
        self.source
    }

    /// The `IconData` this icon draws, when its channel is static geometry.
    ///
    /// The accessor exists because the two channels differ in ways a consumer has to know about:
    /// only static geometry can be measured and validated as source geometry before a control
    /// takes it, and only static geometry is what the host actually owns.
    pub fn geometry(self) -> Option<&'static IconData> {
        match self.source {
            IconSource::StaticGeometry(data) => Some(data),
            IconSource::Builtin(_) => None,
        }
    }

    /// The record this icon was resolved from.
    pub const fn record(self) -> &'static HostIconRecord {
        self.record
    }
}

/// Every icon the host declares, sorted by id.
pub fn icons() -> &'static [HostIconRecord] {
    &super::generated_host_icons::HOST_ICON_RECORDS
}

/// Looks up one icon by its stable host id.
pub fn icon(id: &str) -> Option<&'static HostIconRecord> {
    icons().iter().find(|record| record.id == id)
}

/// Resolves a host id to something a control can be given.
///
/// This is the one call a consumer needs: it reports an unknown id and an unavailable built-in
/// token as separate errors, and it never substitutes a different glyph for a missing one.
pub fn resolve(id: &str) -> Result<HostIcon, IconResolutionError> {
    let record = icon(id).ok_or_else(|| IconResolutionError::UnknownIcon(id.to_owned()))?;
    Ok(HostIcon {
        record,
        source: record.source.source()?,
    })
}

/// The active theme's ink for `role`, honouring an active high-contrast override.
///
/// `None` means no theme is active, which the process-wide manager does not normally produce.
/// Reporting it keeps "there is no theme" from looking like "the role resolved to a colour"; the
/// caller decides what to paint instead.
pub fn role_color(role: IconRole) -> Option<Color> {
    let manager = rust_widgets::theme::global_theme_manager();
    let theme = manager.current_theme()?;
    Some(role.color_in(theme, manager.high_contrast()))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use rust_widgets::core::{Color, Rect, Size};
    use rust_widgets::render::path::MAX_OUTLINE_CONTOURS;
    use rust_widgets::render::{PaintBackend, RenderContext, SoftwarePaintBackend};
    use rust_widgets::theme::{AppearanceMode, Theme};
    use rust_widgets::widget::{IconPaintOutcome, IconRef, IconSource, paint_icon, validate_icon};

    use super::{
        HostIconSource, ICON_MIN_CONTRAST, ICON_ROLE_NAMES, IconRole, icon, icons, resolve,
        role_color,
    };

    /// A colour as a comparable tuple, for the "did this change?" assertions.
    fn rgba(color: Color) -> (u8, u8, u8, u8) {
        (color.r, color.g, color.b, color.a)
    }

    /// The appearances the pinned theme manager can select.
    fn appearances() -> [(AppearanceMode, Theme); 2] {
        [
            (AppearanceMode::Light, Theme::default()),
            (AppearanceMode::Dark, Theme::dark()),
        ]
    }

    /// Renders one source into a square slot and reports whether anything was drawn.
    ///
    /// A software backend is enough: the question is whether the pinned flattener and painter
    /// accept the geometry at this size, not what a particular backend looks like.
    fn draws_at(source: IconSource, side: u32) -> IconPaintOutcome {
        let mut backend = SoftwarePaintBackend::new(Size::new(side, side), 1.0);
        backend.begin_frame(Color::TRANSPARENT);
        let mut context = RenderContext::new(&mut backend);
        paint_icon(
            &mut context,
            IconRef::from(source),
            Rect::new(0, 0, side, side),
            Color::BLACK,
        )
        .unwrap_or_else(|error| panic!("{source:?} could not be painted at {side}px: {error:?}"))
    }

    /// The generated table names the roles the Rust vocabulary names, in the same order.
    ///
    /// This is the check that keeps two definitions of the role vocabulary from drifting: the
    /// list is generated from `resources/host/icons/theme.json`, and the enum is hand written.
    #[test]
    fn the_generated_role_vocabulary_matches_the_enum() {
        let declared: Vec<&str> = IconRole::ALL.iter().map(|role| role.as_str()).collect();
        assert_eq!(ICON_ROLE_NAMES, declared.as_slice());
        for name in ICON_ROLE_NAMES {
            assert_eq!(IconRole::from_str(name).map(IconRole::as_str), Some(name));
        }
        // A misspelt id must not resolve to the nearest role.
        assert_eq!(IconRole::from_str("icon-warn"), None);
        assert_eq!(IconRole::from_str(""), None);
    }

    /// Every declared icon resolves in the pinned dependency, and the ids stay usable as keys.
    #[test]
    fn every_declared_icon_resolves() {
        let table = icons();
        assert!(!table.is_empty(), "the generated table must not be empty");

        let mut seen = HashSet::new();
        let mut previous = "";
        let mut builtin = 0usize;
        let mut geometry = 0usize;
        for record in table {
            assert!(
                seen.insert(record.id),
                "duplicate host icon id {}",
                record.id
            );
            assert!(
                record.id > previous,
                "the table must stay sorted by id: {}",
                record.id
            );
            previous = record.id;

            let source = record
                .source
                .source()
                .unwrap_or_else(|error| panic!("{} does not resolve: {error}", record.id));
            match source {
                IconSource::Builtin(_) => builtin += 1,
                IconSource::StaticGeometry(data) => {
                    geometry += 1;
                    assert_eq!(
                        data.name, record.id,
                        "geometry should carry its own host id"
                    );
                }
            }
            assert_eq!(icon(record.id).map(|found| found.id), Some(record.id));
            let resolved = resolve(record.id).expect("resolves");
            assert_eq!(resolved.record().id, record.id);
            assert_eq!(resolved.record().role, record.role);
            assert_eq!(resolved.record().rtl_mirror, record.rtl_mirror);
        }
        assert!(builtin > 0 && geometry > 0, "both channels must be present");
        assert_eq!(builtin + geometry, table.len());
    }

    /// A built-in token the pinned revision does not publish is reported by name.
    #[test]
    fn an_unavailable_builtin_token_is_reported() {
        let error = HostIconSource::Builtin("not_a_real_token")
            .source()
            .unwrap_err();
        assert_eq!(
            error,
            super::IconResolutionError::UnknownBuiltinToken("not_a_real_token"),
            "the token itself must be named so the dependency can be blamed precisely"
        );
        assert!(error.to_string().contains("not_a_real_token"));
    }

    /// An unknown host id is refused rather than answered with another glyph.
    #[test]
    fn an_unknown_id_is_refused() {
        assert_eq!(
            resolve("host/design/not-declared"),
            Err(super::IconResolutionError::UnknownIcon(
                "host/design/not-declared".to_owned()
            ))
        );
        assert!(resolve("").is_err());
        assert!(
            icon("host/design/canvas").is_some(),
            "a declared id resolves"
        );
    }

    /// Every role reaches the contrast the module promises, in both appearances.
    #[test]
    fn every_role_is_legible_on_its_surface() {
        for (appearance, theme) in appearances() {
            for role in IconRole::ALL {
                let ink = role.color(&theme);
                let surface = role.surface(&theme);
                if role == IconRole::Disabled {
                    assert_eq!(
                        ink, theme.colors.disabled,
                        "{appearance:?}: a disabled glyph keeps the theme's disabled ink; forcing \
                         it to {ICON_MIN_CONTRAST}:1 would make an inoperable control look operable"
                    );
                    continue;
                }
                let ratio = ink.contrast_ratio(surface);
                assert!(
                    ratio >= ICON_MIN_CONTRAST,
                    "{appearance:?}: {role} measures {ratio:.2}:1 on its surface, below \
                     {ICON_MIN_CONTRAST}:1"
                );
            }
        }
    }

    /// The background roles are not seven names for one colour: each resolves to a distinct ink.
    ///
    /// Without this, a mapping mistake that pointed several roles at `foreground` would still pass
    /// the contrast test above.
    ///
    /// `OnAccent` is excluded on purpose rather than by oversight: its ink is chosen for the
    /// accent plate, not the window background, so it may legitimately coincide with a
    /// background role's ink — in the light appearance both are black, and both are correct
    /// because they are read on different surfaces. Its distinctness is asserted through its
    /// surface instead.
    #[test]
    fn the_background_roles_stay_distinguishable() {
        for (appearance, theme) in appearances() {
            let mut seen = HashSet::new();
            for role in IconRole::ALL {
                if role == IconRole::OnAccent {
                    assert_ne!(
                        role.surface(&theme),
                        theme.colors.background,
                        "{appearance:?}: the accent role must be read on the accent surface"
                    );
                    continue;
                }
                assert!(
                    seen.insert(rgba(role.color(&theme))),
                    "{appearance:?}: {role} resolves to an ink another role already uses"
                );
            }
            assert_eq!(seen.len(), IconRole::ALL.len() - 1);
        }
    }

    /// Switching appearance changes what the roles resolve to, which is what a theme switch is
    /// supposed to do.
    ///
    /// `OnAccent` is checked through its surface: its ink is chosen for the accent plate, so the
    /// *plate* is what has to follow the theme.
    #[test]
    fn a_theme_switch_changes_the_ink() {
        let light = Theme::default();
        let dark = Theme::dark();
        let mut changed = 0usize;
        for role in IconRole::ALL {
            if role == IconRole::OnAccent {
                assert_ne!(
                    rgba(role.surface(&light)),
                    rgba(role.surface(&dark)),
                    "{role} keeps the same accent surface in both appearances"
                );
                continue;
            }
            assert_ne!(
                rgba(role.color(&light)),
                rgba(role.color(&dark)),
                "{role} resolves identically in both appearances, so it does not follow the theme"
            );
            changed += 1;
        }
        assert_eq!(changed, IconRole::ALL.len() - 1);
    }

    /// A forced-colour mode replaces every role rather than tinting it.
    #[test]
    fn a_high_contrast_override_replaces_every_role() {
        use rust_widgets::theme::HighContrastMode;

        let theme = Theme::default();
        let forced = Color::rgba(1, 2, 3, 255);
        for role in IconRole::ALL {
            assert_eq!(
                role.color_in(&theme, HighContrastMode::BlackOnWhite),
                Color::BLACK,
                "{role} must follow a forced pair: the host assigns role inks explicitly, so \
                 without this branch they would win over the mode the user selected"
            );
            assert_eq!(
                role.color_in(
                    &theme,
                    HighContrastMode::Custom {
                        fg: forced,
                        bg: Color::WHITE
                    }
                ),
                forced
            );
            // With no override the role keeps its own resolution.
            assert_eq!(
                role.color_in(&theme, HighContrastMode::None),
                role.color(&theme)
            );
        }
    }

    /// The active manager is what `role_color` reads, and it follows a switch.
    #[test]
    fn the_active_manager_drives_role_color() {
        let _guard = rust_widgets::theme::theme_test_guard();
        {
            let mut manager = rust_widgets::theme::global_theme_manager();
            manager.set_high_contrast(rust_widgets::theme::HighContrastMode::None);
            assert!(manager.set_appearance(AppearanceMode::Light));
        }
        let light = role_color(IconRole::Default).expect("a theme is active");
        {
            let mut manager = rust_widgets::theme::global_theme_manager();
            assert!(manager.set_appearance(AppearanceMode::Dark));
        }
        let dark = role_color(IconRole::Default).expect("a theme is active");
        assert_ne!(
            rgba(light),
            rgba(dark),
            "the switch must reach the resolved ink"
        );

        {
            let mut manager = rust_widgets::theme::global_theme_manager();
            manager.set_high_contrast(rust_widgets::theme::HighContrastMode::WhiteOnBlack);
        }
        assert_eq!(role_color(IconRole::Danger), Some(Color::WHITE));

        let mut manager = rust_widgets::theme::global_theme_manager();
        manager.set_high_contrast(rust_widgets::theme::HighContrastMode::None);
        manager.set_appearance(AppearanceMode::Light);
    }

    /// Every glyph clears the dependency's source-level budget.
    ///
    /// The contraction count *is* a source property, so it is checked here for every icon; the
    /// point budget is a property of the size asked for, which the next test covers.
    #[test]
    fn every_source_clears_the_dependency_contour_budget() {
        for record in icons() {
            let IconSource::StaticGeometry(data) = record.source.source().expect("resolves") else {
                continue;
            };
            let metadata = validate_icon(IconRef::StaticGeometry(data)).unwrap_or_else(|error| {
                panic!("{} is not valid source geometry: {error:?}", record.id)
            });
            assert!(metadata.contour_count >= 1, "{}: no contours", record.id);
            assert!(
                metadata.contour_count <= MAX_OUTLINE_CONTOURS,
                "{}: {} contours exceeds the dependency's {MAX_OUTLINE_CONTOURS}",
                record.id,
                metadata.contour_count
            );
            assert_eq!(
                u32::from(metadata.grid),
                24,
                "{}: unexpected grid",
                record.id
            );
        }
    }

    /// A slot with no area is skipped rather than reported as a successful draw.
    ///
    /// Proves the seam propagates the dependency's own refusal instead of turning "drew nothing"
    /// into a pass.
    #[test]
    fn an_empty_slot_is_skipped_not_drawn() {
        let source = icon("host/design/canvas")
            .expect("declared")
            .source
            .source()
            .expect("resolves");
        let mut backend = SoftwarePaintBackend::new(Size::new(0, 0), 1.0);
        backend.begin_frame(Color::TRANSPARENT);
        let mut context = RenderContext::new(&mut backend);
        assert_eq!(
            paint_icon(
                &mut context,
                IconRef::from(source),
                Rect::new(0, 0, 0, 0),
                Color::BLACK
            ),
            Ok(IconPaintOutcome::SkippedEmptySlot)
        );
    }

    /// Every declared icon draws, at every size the strip can display, through the real painter.
    ///
    /// This is the check the resource tooling could not make: the point budget depends on the box
    /// the icon is drawn in, so a source that fits at 20 px can exceed it at a large size. The
    /// printed count is the dependency's own, measured after its flattening.
    #[test]
    fn every_icon_draws_at_every_displayed_size() {
        let mut drawn = 0usize;
        for record in icons() {
            let source = record.source.source().expect("resolves");
            for side in [16u32, 20, 24, 32, 48] {
                assert_eq!(
                    draws_at(source, side),
                    IconPaintOutcome::Drawn,
                    "{} did not draw at {side}px",
                    record.id
                );
                drawn += 1;
            }
        }
        assert_eq!(drawn, icons().len() * 5);
    }
}
