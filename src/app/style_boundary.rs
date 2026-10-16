// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The style boundaries: multi-selection, themes, Unicode fonts, contrast, states, and undoing a
//! style edit back to the reference *and* the computed value (task T11.08).
//!
//! The acceptance line is *"an undo restores the style reference and the computed result"*. That is
//! what the module is built around:
//!
//! * **A session snapshots before it changes.** [`Session`] records the tokens, the inline values and
//!   the theme before each accepted step, so [`Session::undo`] puts back the reference *and*, because
//!   the computed value is derived from them, the same value too — nothing is restored in half.
//! * **A refused step records nothing.** A rename onto an existing name or a switch to a theme that
//!   does not exist answers `Err` and leaves the history alone, so undo never walks back over a step
//!   that never happened.
//! * **A contrast is a number, or it is absent.** [`contrast`] answers `None` for a colour it cannot
//!   read rather than a plausible ratio, so an unreadable colour cannot pass a check (principle 22).
//! * **Unicode is not a special case.** Token names and values are just text; [`Session`] and the
//!   token set handle `var(--字体)` because names are made of alphanumerics, which CJK characters are.

#![allow(dead_code)]

use std::collections::BTreeMap;

use crate::app::css_view::{Css, Element};
use crate::app::design_tokens::{TokenSet, rename_references};
use crate::app::style_emit::{Diagnostic, Input, Style, emit};

/// The state a step recorded, so it can be put back.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Snapshot {
    tokens: TokenSet,
    inline: BTreeMap<String, String>,
    theme: Option<String>,
}

/// A style editing session whose steps can be undone.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Session {
    tokens: TokenSet,
    inline: BTreeMap<String, String>,
    theme: Option<String>,
    history: Vec<Snapshot>,
}

impl Session {
    /// A session over a token set, with no inline values, no theme and nothing to undo.
    pub fn new(tokens: TokenSet) -> Self {
        Self {
            tokens,
            inline: BTreeMap::new(),
            theme: None,
            history: Vec::new(),
        }
    }

    /// The tokens the session is holding.
    pub fn tokens(&self) -> &TokenSet {
        &self.tokens
    }

    /// The inline values, exactly as they are written.
    pub fn inline(&self) -> &BTreeMap<String, String> {
        &self.inline
    }

    /// The theme in force, if any.
    pub fn theme(&self) -> Option<&str> {
        self.theme.as_deref()
    }

    /// Defines a token, recording the step.
    pub fn define_token(
        &mut self,
        kind: crate::app::design_tokens::TokenKind,
        name: impl Into<String>,
        value: impl Into<String>,
    ) {
        self.record();
        self.tokens.define(kind, name, value);
    }

    /// Defines a theme, recording the step.
    pub fn define_theme(&mut self, name: impl Into<String>, values: &[(String, String)]) {
        self.record();
        self.tokens.define_theme(name, values);
    }

    /// Sets an inline value, recording the step.
    pub fn set_inline(&mut self, property: impl Into<String>, value: impl Into<String>) {
        self.record();
        self.inline.insert(property.into(), value.into());
    }

    /// Takes an inline value away, recording the step.
    pub fn clear_inline(&mut self, property: &str) {
        self.record();
        self.inline.remove(property);
    }

    /// Renames a token and rewrites the inline references to it, recording the step.
    ///
    /// The rename happens on a copy first, so a refused rename records nothing.
    pub fn rename_token(&mut self, from: &str, to: &str) -> Result<(), String> {
        let mut tokens = self.tokens.clone();
        tokens.rename(from, to)?;
        self.record();
        self.tokens = tokens;
        for value in self.inline.values_mut() {
            *value = rename_references(value, from, to).0;
        }
        Ok(())
    }

    /// Switches the theme, recording the step. An unknown theme is refused and records nothing.
    pub fn switch_theme(&mut self, theme: Option<&str>) -> Result<(), String> {
        if let Some(name) = theme
            && !self.tokens.has_theme(name)
        {
            return Err(format!("no theme {name:?} is defined"));
        }
        self.record();
        self.theme = theme.map(str::to_owned);
        Ok(())
    }

    /// Whether any step can be undone.
    pub fn can_undo(&self) -> bool {
        !self.history.is_empty()
    }

    /// Puts back the state before the last step; answers whether there was one.
    pub fn undo(&mut self) -> bool {
        match self.history.pop() {
            Some(snapshot) => {
                self.tokens = snapshot.tokens;
                self.inline = snapshot.inline;
                self.theme = snapshot.theme;
                true
            }
            None => false,
        }
    }

    /// Emits the session's style for an element.
    pub fn emit(&self, css: &Css, element: &Element) -> Result<Style, Vec<Diagnostic>> {
        emit(&Input {
            css,
            element,
            inline: &self.inline,
            tokens: &self.tokens,
            theme: self.theme.as_deref(),
        })
    }

    /// The session in one line per fact.
    pub fn render(&self) -> Vec<String> {
        let mut lines = self.tokens.render();
        lines.push(format!("inline: {}", self.inline.len()));
        for (property, value) in &self.inline {
            lines.push(format!("  {property}: {value}"));
        }
        lines.push(format!(
            "theme: {}",
            self.theme.as_deref().unwrap_or("(no theme)")
        ));
        lines.push(format!("undo steps: {}", self.history.len()));
        lines
    }

    fn record(&mut self) {
        self.history.push(Snapshot {
            tokens: self.tokens.clone(),
            inline: self.inline.clone(),
            theme: self.theme.clone(),
        });
    }
}

/// A contrast requirement.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
    /// The AA level for normal text.
    Aa,
    /// The AAA level for normal text.
    Aaa,
}

impl Level {
    /// The level's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Aa => "AA",
            Self::Aaa => "AAA",
        }
    }

    /// The contrast ratio the level requires for normal text.
    pub fn minimum(&self) -> f64 {
        match self {
            Self::Aa => 4.5,
            Self::Aaa => 7.0,
        }
    }
}

/// Reads a `#rgb` or `#rrggbb` colour as three channels in `0..=1`.
pub fn parse_colour(text: &str) -> Option<(f64, f64, f64)> {
    let hex = text.strip_prefix('#')?;
    let digits: String = match hex.len() {
        3 => hex
            .chars()
            .flat_map(|character| [character, character])
            .collect(),
        6 => hex.to_owned(),
        _ => return None,
    };
    let channel = |start: usize| u8::from_str_radix(&digits[start..start + 2], 16).ok();
    let red = channel(0)?;
    let green = channel(2)?;
    let blue = channel(4)?;
    Some((
        f64::from(red) / 255.0,
        f64::from(green) / 255.0,
        f64::from(blue) / 255.0,
    ))
}

/// The relative luminance of a colour (the WCAG definition).
pub fn luminance(colour: (f64, f64, f64)) -> f64 {
    let channel = |value: f64| {
        if value <= 0.039_28 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(colour.0) + 0.7152 * channel(colour.1) + 0.0722 * channel(colour.2)
}

/// The contrast ratio between two colours, or `None` if either cannot be read.
pub fn contrast(a: &str, b: &str) -> Option<f64> {
    let first = luminance(parse_colour(a)?);
    let second = luminance(parse_colour(b)?);
    let (lighter, darker) = if first >= second {
        (first, second)
    } else {
        (second, first)
    };
    Some((lighter + 0.05) / (darker + 0.05))
}

/// Whether a ratio meets a level for normal text.
pub fn meets(ratio: f64, level: Level) -> bool {
    ratio >= level.minimum()
}

#[cfg(test)]
mod tests {
    use super::{Level, Session, contrast, meets, parse_colour};
    use crate::app::css_view::{Css, Element, State};
    use crate::app::design_tokens::{TokenKind, TokenSet};
    use crate::app::interaction_states::preview;
    use crate::app::layout_plan::Origin;
    use crate::app::multi_property::{Common, NodeProperties, common, value_of};
    use crate::contracts::identity::NodeId;
    use crate::contracts::model::{PropertyName, PropertyValue};

    fn button() -> Element {
        Element::parse("button").expect("an element")
    }

    fn tokens() -> TokenSet {
        let mut set = TokenSet::new();
        set.define(TokenKind::Colour, "--accent", "#3b82f6");
        set.define(TokenKind::Colour, "--other", "#111111");
        set
    }

    #[test]
    fn undoing_an_inline_edit_restores_the_reference_and_the_value() {
        let css = Css::parse("button { color: var(--accent) }");
        let mut session = Session::new(tokens());
        session.set_inline("color", "var(--other)");
        let edited = session.emit(&css, &button()).expect("a style");
        assert_eq!(edited.values().get("color"), Some(&"#111111".to_owned()));
        assert_eq!(
            session.inline().get("color"),
            Some(&"var(--other)".to_owned())
        );
        assert!(session.undo());
        let restored = session.emit(&css, &button()).expect("a style");
        assert_eq!(restored.values().get("color"), Some(&"#3b82f6".to_owned()));
        assert!(session.inline().is_empty());
    }

    #[test]
    fn undoing_a_token_rename_restores_the_reference() {
        let css = Css::parse("button { color: var(--accent) }");
        let mut session = Session::new(tokens());
        session.set_inline("color", "var(--accent)");
        session
            .rename_token("--accent", "--primary")
            .expect("a rename");
        assert_eq!(
            session.inline().get("color"),
            Some(&"var(--primary)".to_owned())
        );
        assert_eq!(
            session
                .emit(&css, &button())
                .expect("a style")
                .values()
                .get("color"),
            Some(&"#3b82f6".to_owned())
        );
        assert!(session.undo());
        assert_eq!(
            session.inline().get("color"),
            Some(&"var(--accent)".to_owned())
        );
        assert!(session.tokens().tokens.contains_key("--accent"));
    }

    #[test]
    fn undoing_a_theme_switch_restores_the_computed_value() {
        let mut tokens = tokens();
        tokens.define_theme("dark", &[("--accent".to_owned(), "#60a5fa".to_owned())]);
        let css = Css::parse("button { color: var(--accent) }");
        let mut session = Session::new(tokens);
        assert_eq!(
            session
                .emit(&css, &button())
                .expect("a style")
                .values()
                .get("color"),
            Some(&"#3b82f6".to_owned())
        );
        session.switch_theme(Some("dark")).expect("a theme");
        assert_eq!(
            session
                .emit(&css, &button())
                .expect("a style")
                .values()
                .get("color"),
            Some(&"#60a5fa".to_owned())
        );
        assert!(session.undo());
        assert_eq!(session.theme(), None);
        assert_eq!(
            session
                .emit(&css, &button())
                .expect("a style")
                .values()
                .get("color"),
            Some(&"#3b82f6".to_owned())
        );
    }

    #[test]
    fn a_fresh_session_has_nothing_to_undo() {
        let session = Session::new(tokens());
        assert!(!session.can_undo());
        assert_eq!(session.theme(), None);
    }

    #[test]
    fn a_refused_step_records_nothing() {
        let mut session = Session::new(tokens());
        assert!(session.rename_token("--missing", "--brand").is_err());
        assert!(session.switch_theme(Some("missing")).is_err());
        assert!(!session.can_undo());
    }

    #[test]
    fn undo_walks_back_one_step_at_a_time() {
        let css = Css::parse("button { color: #000000 }");
        let mut session = Session::new(tokens());
        session.set_inline("color", "#111111");
        session.set_inline("color", "#222222");
        assert!(session.undo());
        assert_eq!(
            session
                .emit(&css, &button())
                .expect("a style")
                .values()
                .get("color"),
            Some(&"#111111".to_owned())
        );
        assert!(session.undo());
        assert_eq!(
            session
                .emit(&css, &button())
                .expect("a style")
                .values()
                .get("color"),
            Some(&"#000000".to_owned())
        );
        assert!(!session.can_undo());
        assert!(!session.undo());
    }

    #[test]
    fn contrast_of_black_on_white_is_the_maximum() {
        let ratio = contrast("#000000", "#ffffff").expect("a ratio");
        assert!((ratio - 21.0).abs() < 0.01, "ratio: {ratio}");
    }

    #[test]
    fn the_short_form_of_a_colour_is_the_same_colour() {
        assert_eq!(parse_colour("#fff"), parse_colour("#ffffff"));
    }

    #[test]
    fn contrast_of_a_colour_with_itself_is_one() {
        let ratio = contrast("#3b82f6", "#3b82f6").expect("a ratio");
        assert!((ratio - 1.0).abs() < 0.001, "ratio: {ratio}");
    }

    #[test]
    fn contrast_is_symmetric() {
        assert_eq!(
            contrast("#000000", "#ffffff"),
            contrast("#ffffff", "#000000")
        );
    }

    #[test]
    fn an_invalid_colour_has_no_contrast() {
        assert_eq!(contrast("#12", "#ffffff"), None);
        assert_eq!(contrast("red", "#ffffff"), None);
        assert_eq!(parse_colour("#gggggg"), None);
    }

    #[test]
    fn the_levels_are_the_wcag_thresholds() {
        assert_eq!(Level::Aa.minimum(), 4.5);
        assert_eq!(Level::Aaa.minimum(), 7.0);
        assert!(meets(4.6, Level::Aa));
        assert!(!meets(4.4, Level::Aa));
        assert!(meets(7.1, Level::Aaa));
        assert!(!meets(6.9, Level::Aaa));
    }

    #[test]
    fn a_unicode_token_name_expands() {
        let mut set = TokenSet::new();
        set.define(TokenKind::Font, "--字体", "Noto Sans CJK 汉字");
        let expanded = set.expand("var(--字体)", None).expect("an expansion");
        assert_eq!(expanded.value, "Noto Sans CJK 汉字");
        assert_eq!(expanded.chain, vec!["--字体"]);
    }

    #[test]
    fn every_state_is_previewed_even_with_no_rules() {
        let css = Css::parse("button { color: red } button:hover { color: blue }");
        let previews = preview(&css, &button(), &State::ALL);
        assert_eq!(previews.len(), 4);
        assert_eq!(previews[0].state, State::Hover);
        assert_eq!(previews[0].style.value_of("color"), Some("blue"));
        assert_eq!(previews[3].state, State::Disabled);
        assert_eq!(previews[3].style.value_of("color"), Some("red"));
    }

    #[test]
    fn the_intersection_holds_over_a_large_selection() {
        let name = |text: &str| PropertyName::new(text).expect("a name");
        let mut nodes: Vec<NodeProperties> = (0..100)
            .map(|index| {
                NodeProperties::new(NodeId::from_raw(index))
                    .with(
                        name("width"),
                        PropertyValue::Text("1".to_owned()),
                        Origin::Explicit,
                    )
                    .with(
                        name("gap"),
                        PropertyValue::Text("2".to_owned()),
                        Origin::Explicit,
                    )
            })
            .collect();
        nodes.push(NodeProperties::new(NodeId::from_raw(100)).with(
            name("width"),
            PropertyValue::Text("1".to_owned()),
            Origin::Explicit,
        ));
        let shared: Vec<String> = common(&nodes)
            .iter()
            .map(|name| name.as_str().to_owned())
            .collect();
        assert_eq!(shared, vec!["width"]);
    }

    #[test]
    fn mixed_values_are_reported_over_a_large_selection() {
        let name = |text: &str| PropertyName::new(text).expect("a name");
        let mut nodes: Vec<NodeProperties> = (0..100)
            .map(|index| {
                NodeProperties::new(NodeId::from_raw(index)).with(
                    name("width"),
                    PropertyValue::Text("1".to_owned()),
                    Origin::Explicit,
                )
            })
            .collect();
        nodes.push(NodeProperties::new(NodeId::from_raw(100)).with(
            name("width"),
            PropertyValue::Text("2".to_owned()),
            Origin::Explicit,
        ));
        assert_eq!(value_of(&nodes, &name("width")), Common::Mixed);
    }

    #[test]
    fn a_stale_theme_key_is_inert() {
        let mut tokens = tokens();
        tokens.define_theme("dark", &[("--gone".to_owned(), "#000000".to_owned())]);
        let expanded = tokens
            .expand("var(--accent)", Some("dark"))
            .expect("an expansion");
        assert_eq!(expanded.value, "#3b82f6");
    }
}
