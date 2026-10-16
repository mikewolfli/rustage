// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Interaction states, and reviewing what a theme or a state different makes (task T11.05).
//!
//! The acceptance line is *"a state does not pretend to be a triggered business event"*. The module
//! keeps those two ideas apart, and compares what is left:
//!
//! * **A state is a style, not an event.** [`Preview`] shows how an element computes in one
//!   [`State`]; [`Report::triggered`] answers the business events a preview claims fired, which is
//!   always none, and the declared events are listed as *declarations*. So "shown as pressed" can
//!   never be read as "a pressed event happened".
//! * **A state preview layers on the resting style.** [`preview`] computes each state through
//!   `css_view::compute_in`, so the base rules still apply and only the state's own rules override
//!   them — the same cascade, not a second one (principle 6).
//! * **A difference is a property with both sides.** [`compare`] answers one [`Difference`] per
//!   property that differs, naming the side that does not set it as absent rather than as an empty
//!   value, so "removed under this theme" cannot be mistaken for "set to nothing".
//! * **Two themes compare as literals.** [`resolved`] expands each value through the token set before
//!   comparing, because two themes that both hold `var(--accent)` differ only once the token is
//!   resolved; an unresolvable reference is refused, not compared as text (principle 13).

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use crate::app::style::css_view::{Computed, Css, Element, State, compute_in};
use crate::app::style::design_tokens::{TokenError, TokenSet};

/// One interaction state's computed style.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Preview {
    /// The state it shows.
    pub state: State,
    /// What the element computes to in that state.
    pub style: Computed,
}

impl Preview {
    /// The preview, indented under the state it shows.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!("state {}:", self.state.name())];
        let rendered = self.style.render();
        if rendered.is_empty() {
            lines.push("  (no declarations apply)".to_owned());
        } else {
            for line in rendered {
                lines.push(format!("  {line}"));
            }
        }
        lines
    }
}

/// What an interaction preview says, and what it refuses to say, about events.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Report {
    /// The state previews.
    pub previews: Vec<Preview>,
    /// The business events the node declares.
    pub declared: Vec<String>,
}

impl Report {
    /// The business events a preview claims fired.
    ///
    /// A style state is not an event, so this is always empty. The accessor exists so a caller has to
    /// write the claim down — and cannot accidentally read "shown" as "fired".
    pub fn triggered(&self) -> Vec<&str> {
        Vec::new()
    }

    /// The business events the node declares. They are declarations, not things that happened.
    pub fn declared(&self) -> Vec<&str> {
        self.declared.iter().map(String::as_str).collect()
    }

    /// The report: every preview, then the declared events and what was triggered.
    pub fn render(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for preview in &self.previews {
            lines.extend(preview.render());
        }
        if self.declared.is_empty() {
            lines.push("declared events: (none)".to_owned());
        } else {
            lines.push(format!(
                "declared events (declared, not triggered): {}",
                self.declared.join(", ")
            ));
        }
        lines.push(format!(
            "previewed {} state(s); {} event(s) triggered — a style state is not an event",
            self.previews.len(),
            self.triggered().len()
        ));
        lines
    }
}

/// Computes each requested state's preview for an element.
pub fn preview(css: &Css, element: &Element, states: &[State]) -> Vec<Preview> {
    states
        .iter()
        .map(|state| Preview {
            state: *state,
            style: compute_in(css, element, Some(*state)),
        })
        .collect()
}

/// One property that differs between two styles.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Difference {
    /// The property.
    pub property: String,
    /// Its value on the left, if the left style sets it.
    pub left: Option<String>,
    /// Its value on the right, if the right style sets it.
    pub right: Option<String>,
}

impl Difference {
    /// The difference in one line, naming which side is which.
    pub fn render(&self, left_label: &str, right_label: &str) -> String {
        let left = match &self.left {
            Some(value) => format!("{value} ({left_label})"),
            None => format!("(unset in {left_label})"),
        };
        let right = match &self.right {
            Some(value) => format!("{value} ({right_label})"),
            None => format!("(unset in {right_label})"),
        };
        format!("{}: {left} -> {right}", self.property)
    }
}

/// The properties of a computed style as its raw values.
pub fn literal(style: &Computed) -> BTreeMap<String, String> {
    style
        .properties()
        .into_iter()
        .map(|property| {
            (
                property.to_owned(),
                style.value_of(property).unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

/// The properties of a computed style with every reference expanded through the token set.
///
/// A theme is named, not assumed: without one the tokens' own values are used.
pub fn resolved(
    style: &Computed,
    tokens: &TokenSet,
    theme: Option<&str>,
) -> Result<BTreeMap<String, String>, TokenError> {
    let mut out = BTreeMap::new();
    for property in style.properties() {
        let value = style.value_of(property).unwrap_or_default();
        out.insert(property.to_owned(), tokens.expand(value, theme)?.value);
    }
    Ok(out)
}

/// The properties that differ between two styles, in property order.
pub fn compare(
    left: &BTreeMap<String, String>,
    right: &BTreeMap<String, String>,
) -> Vec<Difference> {
    let mut properties: BTreeSet<&String> = left.keys().collect();
    properties.extend(right.keys());
    let mut differences = Vec::new();
    for property in properties {
        let left_value = left.get(property).cloned();
        let right_value = right.get(property).cloned();
        if left_value != right_value {
            differences.push(Difference {
                property: property.clone(),
                left: left_value,
                right: right_value,
            });
        }
    }
    differences
}

#[cfg(test)]
mod tests {
    use super::{Difference, Report, compare, literal, preview, resolved};
    use crate::app::style::css_view::{Css, Element, State, compute_in};
    use crate::app::style::design_tokens::{TokenError, TokenKind, TokenSet};
    use std::collections::BTreeMap;

    fn button() -> Element {
        Element::parse("button").expect("an element")
    }

    fn map(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
        entries
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn a_state_preview_layers_on_the_resting_style() {
        let css = Css::parse("button { color: red; gap: 8 }\nbutton:hover { color: blue }");
        let previews = preview(&css, &button(), &[State::Hover]);
        let style = &previews[0].style;
        assert_eq!(style.value_of("color"), Some("blue"));
        assert_eq!(style.value_of("gap"), Some("8"));
    }

    #[test]
    fn a_state_preview_only_applies_its_own_state() {
        let css = Css::parse(
            "button { color: red }\nbutton:hover { color: blue }\nbutton:focus { color: green }",
        );
        let previews = preview(&css, &button(), &[State::Hover, State::Focus]);
        assert_eq!(previews[0].style.value_of("color"), Some("blue"));
        assert_eq!(previews[1].style.value_of("color"), Some("green"));
    }

    #[test]
    fn a_state_preview_names_the_rule_it_came_from() {
        let css = Css::parse("button { color: red }\nbutton:hover { color: blue }");
        let previews = preview(&css, &button(), &[State::Hover]);
        let source = previews[0].style.source_of("color").expect("a source");
        assert_eq!(source.selector, "button:hover");
        assert_eq!(source.rule, 1);
    }

    #[test]
    fn previewing_a_state_triggers_no_business_event() {
        let css = Css::parse("button:active { color: blue }");
        let report = Report {
            previews: preview(&css, &button(), &[State::Active]),
            declared: vec!["pressed".to_owned()],
        };
        assert!(report.triggered().is_empty());
        assert_eq!(report.declared(), vec!["pressed"]);
    }

    #[test]
    fn the_report_separates_declared_events_from_a_state() {
        let css = Css::parse("button:active { color: blue }");
        let report = Report {
            previews: preview(&css, &button(), &[State::Active]),
            declared: vec!["pressed".to_owned()],
        };
        let text = report.render().join("\n");
        assert!(text.contains("state active:"));
        assert!(text.contains("declared, not triggered"));
        assert!(text.contains("0 event(s) triggered"));
    }

    #[test]
    fn the_report_says_when_nothing_is_declared() {
        let report = Report {
            previews: Vec::new(),
            declared: Vec::new(),
        };
        assert!(
            report
                .render()
                .join("\n")
                .contains("declared events: (none)")
        );
    }

    #[test]
    fn comparing_two_styles_lists_only_the_properties_that_differ() {
        let left = map(&[("color", "red"), ("gap", "8")]);
        let right = map(&[("color", "blue"), ("gap", "8")]);
        let differences = compare(&left, &right);
        assert_eq!(differences.len(), 1);
        assert_eq!(differences[0].property, "color");
    }

    #[test]
    fn a_property_only_one_side_sets_shows_the_other_as_unset() {
        let left = map(&[("color", "red")]);
        let right = map(&[("gap", "8")]);
        let differences = compare(&left, &right);
        assert_eq!(differences.len(), 2);
        let color = differences
            .iter()
            .find(|difference| difference.property == "color")
            .expect("the color difference");
        assert_eq!(color.left, Some("red".to_owned()));
        assert_eq!(color.right, None);
        assert!(color.render("light", "dark").contains("(unset in dark)"));
    }

    #[test]
    fn identical_styles_do_not_differ() {
        let style = map(&[("color", "red"), ("gap", "8")]);
        assert!(compare(&style, &style).is_empty());
    }

    #[test]
    fn resolving_through_a_theme_uses_the_theme_value() {
        let mut tokens = TokenSet::new();
        tokens.define(TokenKind::Colour, "--accent", "#3b82f6");
        tokens.define_theme("dark", &[("--accent".to_owned(), "#60a5fa".to_owned())]);
        let css = Css::parse("button { color: var(--accent) }");
        let style = compute_in(&css, &button(), None);
        assert_eq!(
            literal(&style).get("color"),
            Some(&"var(--accent)".to_owned())
        );
        assert_eq!(
            resolved(&style, &tokens, None).expect("light").get("color"),
            Some(&"#3b82f6".to_owned())
        );
        assert_eq!(
            resolved(&style, &tokens, Some("dark"))
                .expect("dark")
                .get("color"),
            Some(&"#60a5fa".to_owned())
        );
    }

    #[test]
    fn the_theme_difference_is_what_compare_reports() {
        let mut tokens = TokenSet::new();
        tokens.define(TokenKind::Colour, "--accent", "#3b82f6");
        tokens.define_theme("dark", &[("--accent".to_owned(), "#60a5fa".to_owned())]);
        let css = Css::parse("button { color: var(--accent); gap: 8 }");
        let style = compute_in(&css, &button(), None);
        let light = resolved(&style, &tokens, None).expect("light");
        let dark = resolved(&style, &tokens, Some("dark")).expect("dark");
        let differences = compare(&light, &dark);
        assert_eq!(differences.len(), 1);
        assert_eq!(
            differences[0].render("light", "dark"),
            "color: #3b82f6 (light) -> #60a5fa (dark)"
        );
    }

    #[test]
    fn a_resolved_style_with_an_unknown_reference_is_refused() {
        let tokens = TokenSet::new();
        let css = Css::parse("button { color: var(--missing) }");
        let style = compute_in(&css, &button(), None);
        assert_eq!(
            resolved(&style, &tokens, None),
            Err(TokenError::Unknown {
                name: "--missing".to_owned()
            })
        );
    }

    #[test]
    fn the_difference_kind_is_a_plain_value() {
        let difference = Difference {
            property: "color".to_owned(),
            left: Some("red".to_owned()),
            right: Some("blue".to_owned()),
        };
        assert_eq!(
            difference.render("light", "dark"),
            "color: red (light) -> blue (dark)"
        );
    }
}
