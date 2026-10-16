// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The one place a style is emitted: cascade plus inline, resolved through tokens, for both the
//! preview and the generator (task T11.07).
//!
//! The acceptance line is *"the same input gives the same preview and generation result"*. That is a
//! structural promise, so the module makes one emitter and nothing else:
//!
//! * **Preview and generation are the same [`emit`].** Both consumers call this one function; only
//!   [`Mode`] changes how the result is *printed*, never the values. So the two cannot drift: there is
//!   no second cascade to keep in step (principle 6/14).
//! * **Inline wins over the sheet, in both modes.** An inline declaration replaces a rule's value and
//!   is remembered as [`Origin::Inline`], so the precedence is one rule rather than a preview-only
//!   convention.
//! * **An unresolvable reference is a diagnostic, not a value here.** [`emit`] answers `Err` listing
//!   every property whose reference cannot be expanded, so generation blocks on the whole set instead
//!   of emitting a reference the target cannot read (principle 13/22).
//! * **Migration names the token that names a literal.** [`migrate`] answers the references a style
//!   would take and the migrations it made, deterministically (first token by name), so moving a
//!   hard-coded value to a token is reviewable and repeatable.

#![allow(dead_code)]

use std::collections::BTreeMap;

use crate::app::css_view::{Css, Element, Source, compute};
use crate::app::design_tokens::TokenSet;

/// Which consumer the style is for. Both read the same emitter.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// The design-time preview.
    Preview,
    /// The generated style.
    Generate,
}

impl Mode {
    /// The mode's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Preview => "preview",
            Self::Generate => "generate",
        }
    }
}

/// Where a declaration's value came from.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Origin {
    /// Set inline on the element; it wins over the sheet.
    Inline,
    /// Set by a rule.
    Rule(Source),
}

impl Origin {
    /// The origin in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Inline => "inline (wins over the sheet)".to_owned(),
            Self::Rule(source) => source.render(),
        }
    }
}

/// A property that could not be emitted, and why.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Diagnostic {
    /// The property.
    pub property: String,
    /// Why it could not be emitted.
    pub reason: String,
}

impl Diagnostic {
    /// The diagnostic in one line.
    pub fn render(&self) -> String {
        format!("{}: {}", self.property, self.reason)
    }
}

/// A style ready to be shown or generated: one literal per property, with its origin.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Style {
    /// The properties, as literals.
    pub declarations: BTreeMap<String, String>,
    /// Where each value came from.
    pub sources: BTreeMap<String, Origin>,
}

impl Style {
    /// The literal each property took. This is what preview and generation must agree on.
    pub fn values(&self) -> &BTreeMap<String, String> {
        &self.declarations
    }

    /// The style in the form a consumer wants.
    pub fn render(&self, mode: Mode) -> Vec<String> {
        self.declarations
            .iter()
            .map(|(property, value)| match mode {
                Mode::Preview => match self.sources.get(property) {
                    Some(origin) => format!("{property}: {value}  <- {}", origin.render()),
                    None => format!("{property}: {value}"),
                },
                Mode::Generate => format!("{property}: {value};"),
            })
            .collect()
    }
}

/// Everything the emitter reads.
#[derive(Clone, Copy)]
pub struct Input<'a> {
    /// The stylesheet.
    pub css: &'a Css,
    /// The element being styled.
    pub element: &'a Element,
    /// Declarations set inline on the element; they win over the sheet.
    pub inline: &'a BTreeMap<String, String>,
    /// The document's tokens.
    pub tokens: &'a TokenSet,
    /// The theme to resolve under, if any.
    pub theme: Option<&'a str>,
}

/// Emits a style: the cascade, then inline, then tokens resolved to literals.
pub fn emit(input: &Input<'_>) -> Result<Style, Vec<Diagnostic>> {
    let computed = compute(input.css, input.element);
    let mut raw: BTreeMap<String, (String, Origin)> = BTreeMap::new();
    for property in computed.properties() {
        let value = computed.value_of(property).unwrap_or_default().to_owned();
        let source = computed
            .source_of(property)
            .cloned()
            .expect("a computed property has a source");
        raw.insert(property.to_owned(), (value, Origin::Rule(source)));
    }
    for (property, value) in input.inline {
        raw.insert(property.clone(), (value.clone(), Origin::Inline));
    }
    let mut declarations = BTreeMap::new();
    let mut sources = BTreeMap::new();
    let mut diagnostics = Vec::new();
    for (property, (value, origin)) in raw {
        match input.tokens.expand(&value, input.theme) {
            Ok(expansion) => {
                declarations.insert(property.clone(), expansion.value);
                sources.insert(property, origin);
            }
            Err(error) => diagnostics.push(Diagnostic {
                property,
                reason: error.message(),
            }),
        }
    }
    if diagnostics.is_empty() {
        Ok(Style {
            declarations,
            sources,
        })
    } else {
        Err(diagnostics)
    }
}

/// A literal that a token already names.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Migration {
    /// The property whose value could become a reference.
    pub property: String,
    /// The literal it holds now.
    pub literal: String,
    /// The token that names it.
    pub token: String,
}

impl Migration {
    /// The migration in one line.
    pub fn render(&self) -> String {
        format!("{}: {} -> var({})", self.property, self.literal, self.token)
    }
}

/// The declarations a style would take if every literal a token names became a reference, plus the
/// migrations made. The earliest token by name wins, so the answer is repeatable.
pub fn migrate(style: &Style, tokens: &TokenSet) -> (BTreeMap<String, String>, Vec<Migration>) {
    let mut rewritten = BTreeMap::new();
    let mut migrations = Vec::new();
    for (property, literal) in &style.declarations {
        match tokens.tokens.values().find(|token| &token.value == literal) {
            Some(token) => {
                rewritten.insert(property.clone(), format!("var({})", token.name));
                migrations.push(Migration {
                    property: property.clone(),
                    literal: literal.clone(),
                    token: token.name.clone(),
                });
            }
            None => {
                rewritten.insert(property.clone(), literal.clone());
            }
        }
    }
    (rewritten, migrations)
}

#[cfg(test)]
mod tests {
    use super::{Input, Migration, Mode, Origin, Style, emit, migrate};
    use crate::app::css_view::{Css, Element};
    use crate::app::design_tokens::{TokenKind, TokenSet};
    use std::collections::BTreeMap;

    fn button() -> Element {
        Element::parse("button").expect("an element")
    }

    fn tokens() -> TokenSet {
        let mut set = TokenSet::new();
        set.define(TokenKind::Colour, "--accent", "#3b82f6");
        set
    }

    fn empty() -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    #[test]
    fn preview_and_generation_carry_the_same_values() {
        let css = Css::parse("button { color: #3b82f6; gap: 8 }");
        let tokens = tokens();
        let inline = empty();
        let input = Input {
            css: &css,
            element: &button(),
            inline: &inline,
            tokens: &tokens,
            theme: None,
        };
        let style = emit(&input).expect("a style");
        let preview = style.render(Mode::Preview).join("\n");
        let generated = style.render(Mode::Generate).join("\n");
        for (property, value) in style.values() {
            assert!(preview.contains(&format!("{property}: {value}")));
            assert!(generated.contains(&format!("{property}: {value};")));
        }
        assert_eq!(preview.lines().count(), generated.lines().count());
    }

    #[test]
    fn the_same_input_emits_the_same_style_twice() {
        let css = Css::parse("button { color: var(--accent); gap: 8 }");
        let tokens = tokens();
        let inline = empty();
        let input = Input {
            css: &css,
            element: &button(),
            inline: &inline,
            tokens: &tokens,
            theme: None,
        };
        assert_eq!(emit(&input), emit(&input));
    }

    #[test]
    fn a_reference_is_expanded_to_a_literal() {
        let css = Css::parse("button { color: var(--accent) }");
        let tokens = tokens();
        let inline = empty();
        let style = emit(&Input {
            css: &css,
            element: &button(),
            inline: &inline,
            tokens: &tokens,
            theme: None,
        })
        .expect("a style");
        assert_eq!(style.values().get("color"), Some(&"#3b82f6".to_owned()));
    }

    #[test]
    fn an_inline_declaration_wins_over_a_rule() {
        let css = Css::parse("button { color: #3b82f6 }");
        let tokens = tokens();
        let mut inline = BTreeMap::new();
        inline.insert("color".to_owned(), "#111111".to_owned());
        let style = emit(&Input {
            css: &css,
            element: &button(),
            inline: &inline,
            tokens: &tokens,
            theme: None,
        })
        .expect("a style");
        assert_eq!(style.values().get("color"), Some(&"#111111".to_owned()));
        assert_eq!(style.sources.get("color"), Some(&Origin::Inline));
    }

    #[test]
    fn an_inline_only_property_is_included() {
        let css = Css::parse("button { color: #3b82f6 }");
        let tokens = tokens();
        let mut inline = BTreeMap::new();
        inline.insert("gap".to_owned(), "8".to_owned());
        let style = emit(&Input {
            css: &css,
            element: &button(),
            inline: &inline,
            tokens: &tokens,
            theme: None,
        })
        .expect("a style");
        assert_eq!(style.values().get("gap"), Some(&"8".to_owned()));
        assert_eq!(style.sources.get("gap"), Some(&Origin::Inline));
    }

    #[test]
    fn an_unknown_reference_is_a_diagnostic_and_nothing_is_emitted() {
        let css = Css::parse("button { color: var(--missing) }");
        let tokens = tokens();
        let inline = empty();
        let outcome = emit(&Input {
            css: &css,
            element: &button(),
            inline: &inline,
            tokens: &tokens,
            theme: None,
        });
        let diagnostics = outcome.expect_err("a diagnostic");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].property, "color");
        assert!(diagnostics[0].render().contains("--missing"));
    }

    #[test]
    fn a_bad_reference_does_not_hide_a_good_property() {
        let css = Css::parse("button { color: var(--missing); gap: 8 }");
        let tokens = tokens();
        let inline = empty();
        let diagnostics = emit(&Input {
            css: &css,
            element: &button(),
            inline: &inline,
            tokens: &tokens,
            theme: None,
        })
        .expect_err("a diagnostic");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].property, "color");
    }

    #[test]
    fn the_preview_form_names_the_origin() {
        let css = Css::parse("button { color: #3b82f6 }");
        let tokens = tokens();
        let inline = empty();
        let style = emit(&Input {
            css: &css,
            element: &button(),
            inline: &inline,
            tokens: &tokens,
            theme: None,
        })
        .expect("a style");
        let preview = style.render(Mode::Preview).join("\n");
        assert!(preview.contains("<- rule 0 (button)"));
    }

    #[test]
    fn the_generated_form_is_a_declaration_list() {
        let css = Css::parse("button { color: #3b82f6; gap: 8 }");
        let tokens = tokens();
        let inline = empty();
        let style = emit(&Input {
            css: &css,
            element: &button(),
            inline: &inline,
            tokens: &tokens,
            theme: None,
        })
        .expect("a style");
        let generated = style.render(Mode::Generate);
        assert_eq!(
            generated,
            vec!["color: #3b82f6;".to_owned(), "gap: 8;".to_owned()]
        );
    }

    #[test]
    fn migration_finds_the_token_that_names_a_literal() {
        let style = Style {
            declarations: [
                ("color".to_owned(), "#3b82f6".to_owned()),
                ("gap".to_owned(), "8".to_owned()),
            ]
            .into_iter()
            .collect(),
            sources: BTreeMap::new(),
        };
        let (rewritten, migrations) = migrate(&style, &tokens());
        assert_eq!(migrations.len(), 1);
        assert_eq!(
            migrations[0],
            Migration {
                property: "color".to_owned(),
                literal: "#3b82f6".to_owned(),
                token: "--accent".to_owned(),
            }
        );
        assert_eq!(rewritten.get("color"), Some(&"var(--accent)".to_owned()));
        assert_eq!(rewritten.get("gap"), Some(&"8".to_owned()));
    }

    #[test]
    fn migration_is_empty_when_no_token_matches() {
        let style = Style {
            declarations: [("gap".to_owned(), "8".to_owned())].into_iter().collect(),
            sources: BTreeMap::new(),
        };
        let (rewritten, migrations) = migrate(&style, &tokens());
        assert!(migrations.is_empty());
        assert_eq!(rewritten.get("gap"), Some(&"8".to_owned()));
    }

    #[test]
    fn migrating_then_emitting_gives_the_same_literals() {
        let css = Css::parse("button { color: #3b82f6 }");
        let tokens = tokens();
        let inline = empty();
        let before = emit(&Input {
            css: &css,
            element: &button(),
            inline: &inline,
            tokens: &tokens,
            theme: None,
        })
        .expect("a style");
        let (rewritten, migrations) = migrate(&before, &tokens);
        assert_eq!(migrations.len(), 1);
        let after = emit(&Input {
            css: &css,
            element: &button(),
            inline: &rewritten,
            tokens: &tokens,
            theme: None,
        })
        .expect("a style");
        assert_eq!(before.values(), after.values());
    }
}
