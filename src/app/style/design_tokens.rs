// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Design tokens: colour/font/spacing resources, references, renaming and themes (task T11.04).
//!
//! The acceptance line is *"when the library does not support variables, they are expanded only at
//! generation time, and traceably"*. That splits the module in two, and the split is the point:
//!
//! * **The model keeps the reference; only expansion inlines it.** A [`Usage`] stores the value a
//!   designer typed — `var(--accent)` — and stays that way; [`TokenSet::expand`] is the generation
//!   step that turns it into a literal. So a target that cannot read variables is served by inlining
//!   at generation, not by the design model quietly dropping the reference.
//! * **An expansion is traceable.** [`Expansion`] carries the literal and the chain of tokens it went
//!   through, so the generated `#3b82f6` can still be told from one that was written by hand.
//! * **Expanding is legal or it is refused.** An unknown token, a cycle or a `var(...)` the parser
//!   cannot read is a [`TokenError`], never a literal that looks resolved but is not (principle 13).
//! * **A rename rewires every reference.** [`TokenSet::rename`] renames the token and rewrites the
//!   references inside the theme keys, the other token values and the theme values, and
//!   [`rename_references`] rewrites the usages, so a rename cannot leave a dangling name behind;
//!   renaming onto a name that already exists is refused rather than silently merging two resources.

#![allow(dead_code)]

use std::collections::BTreeMap;

/// What kind of resource a token is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TokenKind {
    /// A colour resource.
    Colour,
    /// A font resource.
    Font,
    /// A spacing resource.
    Spacing,
}

impl TokenKind {
    /// The kind's name, as a caller writes it.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Colour => "colour",
            Self::Font => "font",
            Self::Spacing => "spacing",
        }
    }

    /// Reads a kind's name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "colour" => Some(Self::Colour),
            "font" => Some(Self::Font),
            "spacing" => Some(Self::Spacing),
            _ => None,
        }
    }
}

/// One named resource.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Token {
    /// The name it is referenced by.
    pub name: String,
    /// What kind of resource it is.
    pub kind: TokenKind,
    /// Its value.
    pub value: String,
}

/// One theme: values a set of tokens takes under it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Theme {
    /// The theme's name.
    pub name: String,
    /// The token values it overrides.
    pub values: BTreeMap<String, String>,
}

/// One place a value is used, kept exactly as the designer wrote it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Usage {
    /// The property it is set on.
    pub property: String,
    /// The value, references and all.
    pub value: String,
}

/// Why a value could not be expanded.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum TokenError {
    /// The value references a token that is not defined.
    Unknown {
        /// The missing name.
        name: String,
    },
    /// The references form a cycle.
    Cycle {
        /// A name on the cycle.
        name: String,
    },
    /// The value names a theme that is not defined.
    NoTheme {
        /// The missing theme.
        name: String,
    },
    /// The value holds something that is not a readable `var(...)`.
    Syntax {
        /// What could not be read.
        reason: String,
    },
}

impl TokenError {
    /// The error in one line.
    pub fn message(&self) -> String {
        match self {
            Self::Unknown { name } => format!("no token {name} is defined"),
            Self::Cycle { name } => format!("{name} refers, directly or not, back to itself"),
            Self::NoTheme { name } => format!("no theme {name} is defined"),
            Self::Syntax { reason } => reason.clone(),
        }
    }
}

/// A resolved value and the tokens it went through.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Expansion {
    /// The literal value.
    pub value: String,
    /// The tokens used, outermost first, each named once.
    pub chain: Vec<String>,
}

impl Expansion {
    /// Whether the value was already a literal and named no token.
    pub fn is_literal(&self) -> bool {
        self.chain.is_empty()
    }

    /// The expansion in one line, naming its chain when there is one.
    pub fn render(&self) -> String {
        if self.chain.is_empty() {
            self.value.clone()
        } else {
            format!("{}  (via {})", self.value, self.chain.join(" -> "))
        }
    }
}

/// The tokens and themes of a document.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct TokenSet {
    /// The tokens, by name.
    pub tokens: BTreeMap<String, Token>,
    /// The themes, by name.
    pub themes: BTreeMap<String, Theme>,
}

impl TokenSet {
    /// A set with no tokens and no themes.
    pub fn new() -> Self {
        Self::default()
    }

    /// Defines or replaces a token.
    pub fn define(&mut self, kind: TokenKind, name: impl Into<String>, value: impl Into<String>) {
        let name = name.into();
        self.tokens.insert(
            name.clone(),
            Token {
                name,
                kind,
                value: value.into(),
            },
        );
    }

    /// Defines or replaces a theme.
    pub fn define_theme(&mut self, name: impl Into<String>, values: &[(String, String)]) {
        let name = name.into();
        self.themes.insert(
            name.clone(),
            Theme {
                name,
                values: values.iter().cloned().collect(),
            },
        );
    }

    /// Whether a theme is defined.
    pub fn has_theme(&self, name: &str) -> bool {
        self.themes.contains_key(name)
    }

    /// Expands a value to a literal, naming the tokens it went through.
    ///
    /// This is the generation step: the model around it keeps the references.
    pub fn expand(&self, value: &str, theme: Option<&str>) -> Result<Expansion, TokenError> {
        let theme = match theme {
            Some(name) => Some(self.themes.get(name).ok_or_else(|| TokenError::NoTheme {
                name: name.to_owned(),
            })?),
            None => None,
        };
        let mut stack = Vec::new();
        let mut chain = Vec::new();
        let text = self.expand_value(value, theme, &mut stack, &mut chain)?;
        Ok(Expansion { value: text, chain })
    }

    /// Renames a token, rewiring the references to it everywhere in the set.
    ///
    /// The theme keys that override it, the references inside other token values and the references
    /// inside theme values are all rewritten, so a rename cannot leave a dangling `var(--name)`
    /// behind. Renaming onto a name that already exists is refused, and the original is put back, so
    /// a failed rename changes nothing.
    pub fn rename(&mut self, from: &str, to: &str) -> Result<(), String> {
        if from == to {
            return Ok(());
        }
        let token = self
            .tokens
            .remove(from)
            .ok_or_else(|| format!("no token {from} is defined"))?;
        if self.tokens.contains_key(to) {
            self.tokens.insert(from.to_owned(), token);
            return Err(format!("a token {to} already exists"));
        }
        self.tokens.insert(
            to.to_owned(),
            Token {
                name: to.to_owned(),
                kind: token.kind,
                value: token.value,
            },
        );
        for token in self.tokens.values_mut() {
            token.value = rename_references(&token.value, from, to).0;
        }
        for theme in self.themes.values_mut() {
            if let Some(value) = theme.values.remove(from) {
                theme.values.insert(to.to_owned(), value);
            }
            for value in theme.values.values_mut() {
                *value = rename_references(value, from, to).0;
            }
        }
        Ok(())
    }

    /// The token view, one line per token and theme, plus a summary.
    pub fn render(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for token in self.tokens.values() {
            lines.push(format!(
                "token {} ({}) = {}",
                token.name,
                token.kind.name(),
                token.value
            ));
        }
        for theme in self.themes.values() {
            let pairs: Vec<String> = theme
                .values
                .iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect();
            lines.push(format!("theme {}: {}", theme.name, pairs.join(", ")));
        }
        lines.push(format!(
            "tokens: {}, themes: {} (the model keeps references; only expansion inlines them)",
            self.tokens.len(),
            self.themes.len()
        ));
        lines
    }

    fn expand_value(
        &self,
        value: &str,
        theme: Option<&Theme>,
        stack: &mut Vec<String>,
        chain: &mut Vec<String>,
    ) -> Result<String, TokenError> {
        let names = references(value).map_err(|reason| TokenError::Syntax { reason })?;
        let mut out = value.to_owned();
        for name in names {
            if stack.iter().any(|seen| seen == &name) {
                return Err(TokenError::Cycle { name });
            }
            let token = self
                .tokens
                .get(&name)
                .ok_or_else(|| TokenError::Unknown { name: name.clone() })?;
            let raw = match theme.and_then(|theme| theme.values.get(&name)) {
                Some(value) => value.clone(),
                None => token.value.clone(),
            };
            stack.push(name.clone());
            if !chain.contains(&name) {
                chain.push(name.clone());
            }
            let expanded = self.expand_value(&raw, theme, stack, chain)?;
            stack.pop();
            out = replace_var(&out, &name, &expanded);
        }
        Ok(out)
    }
}

/// The `var(--name)` references a value holds, in the order they appear.
pub fn references(value: &str) -> Result<Vec<String>, String> {
    let mut found = Vec::new();
    let mut rest = value;
    while let Some(start) = rest.find("var(") {
        let after = &rest[start + 4..];
        let Some(close) = after.find(')') else {
            return Err("a var(...) is not closed".to_owned());
        };
        let inner = &after[..close];
        let name = inner
            .strip_prefix("--")
            .ok_or_else(|| format!("var({inner}) is not a custom property"))?;
        if name.is_empty()
            || !name.chars().all(|character| {
                character.is_alphanumeric() || character == '-' || character == '_'
            })
        {
            return Err(format!("var({inner}) is not a custom property"));
        }
        found.push(inner.to_owned());
        rest = &after[close + 1..];
    }
    Ok(found)
}

/// Rewrites every `var(from)` in a value to `var(to)`, answering the new text and how many were.
pub fn rename_references(value: &str, from: &str, to: &str) -> (String, usize) {
    let needle = format!("var({from})");
    let count = value.matches(&needle).count();
    (value.replace(&needle, &format!("var({to})")), count)
}

/// Replaces exactly `var(name)` with a literal.
fn replace_var(text: &str, name: &str, replacement: &str) -> String {
    text.replace(&format!("var({name})"), replacement)
}

#[cfg(test)]
mod tests {
    use super::{TokenError, TokenKind, TokenSet, Usage, references, rename_references};

    fn tokens() -> TokenSet {
        let mut set = TokenSet::new();
        set.define(TokenKind::Colour, "--accent", "#3b82f6");
        set.define(TokenKind::Spacing, "--gap", "8px");
        set
    }

    #[test]
    fn a_token_keeps_its_kind_and_value() {
        let set = tokens();
        let token = set.tokens.get("--accent").expect("the token");
        assert_eq!(token.kind, TokenKind::Colour);
        assert_eq!(token.value, "#3b82f6");
    }

    #[test]
    fn a_usage_keeps_its_reference_until_expansion() {
        let usage = Usage {
            property: "color".to_owned(),
            value: "var(--accent)".to_owned(),
        };
        assert_eq!(
            references(&usage.value).expect("references"),
            vec!["--accent"]
        );
        assert!(
            usage.value.contains("var(--accent)"),
            "the model must keep the reference"
        );
    }

    #[test]
    fn expansion_replaces_a_reference_and_names_the_token() {
        let set = tokens();
        let expanded = set.expand("var(--accent)", None).expect("an expansion");
        assert_eq!(expanded.value, "#3b82f6");
        assert_eq!(expanded.chain, vec!["--accent"]);
        assert!(!expanded.is_literal());
        assert!(expanded.render().contains("via --accent"));
    }

    #[test]
    fn a_nested_reference_expands_through_the_chain() {
        let mut set = TokenSet::new();
        set.define(TokenKind::Colour, "--brand", "var(--accent)");
        set.define(TokenKind::Colour, "--accent", "#3b82f6");
        let expanded = set.expand("var(--brand)", None).expect("an expansion");
        assert_eq!(expanded.value, "#3b82f6");
        assert_eq!(expanded.chain, vec!["--brand", "--accent"]);
    }

    #[test]
    fn an_unknown_reference_is_refused() {
        let set = tokens();
        assert_eq!(
            set.expand("var(--missing)", None),
            Err(TokenError::Unknown {
                name: "--missing".to_owned()
            })
        );
    }

    #[test]
    fn a_cycle_is_refused() {
        let mut set = TokenSet::new();
        set.define(TokenKind::Colour, "--a", "var(--b)");
        set.define(TokenKind::Colour, "--b", "var(--a)");
        assert_eq!(
            set.expand("var(--a)", None),
            Err(TokenError::Cycle {
                name: "--a".to_owned()
            })
        );
    }

    #[test]
    fn a_theme_changes_the_value_expansion_uses() {
        let mut set = tokens();
        set.define_theme("dark", &[("--accent".to_owned(), "#60a5fa".to_owned())]);
        assert_eq!(
            set.expand("var(--accent)", None).expect("light").value,
            "#3b82f6"
        );
        assert_eq!(
            set.expand("var(--accent)", Some("dark"))
                .expect("dark")
                .value,
            "#60a5fa"
        );
    }

    #[test]
    fn a_theme_that_does_not_exist_is_refused() {
        let set = tokens();
        assert_eq!(
            set.expand("var(--accent)", Some("missing")),
            Err(TokenError::NoTheme {
                name: "missing".to_owned()
            })
        );
    }

    #[test]
    fn expanding_a_literal_names_no_token() {
        let set = tokens();
        let expanded = set.expand("#fff", None).expect("an expansion");
        assert_eq!(expanded.value, "#fff");
        assert!(expanded.is_literal());
        assert_eq!(expanded.render(), "#fff");
    }

    #[test]
    fn a_var_that_is_not_a_custom_property_is_refused() {
        let set = tokens();
        assert!(matches!(
            set.expand("var(accent)", None),
            Err(TokenError::Syntax { .. })
        ));
        assert!(matches!(
            set.expand("var(--accent", None),
            Err(TokenError::Syntax { .. })
        ));
    }

    #[test]
    fn renaming_updates_the_token_and_its_theme_references() {
        let mut set = tokens();
        set.define_theme("dark", &[("--accent".to_owned(), "#60a5fa".to_owned())]);
        set.rename("--accent", "--brand").expect("a rename");
        assert!(set.tokens.contains_key("--brand"));
        assert!(!set.tokens.contains_key("--accent"));
        let theme = set.themes.get("dark").expect("the theme");
        assert_eq!(theme.values.get("--brand"), Some(&"#60a5fa".to_owned()));
        assert!(!theme.values.contains_key("--accent"));
        assert_eq!(
            set.expand("var(--brand)", None).expect("expands").value,
            "#3b82f6"
        );
    }

    #[test]
    fn renaming_rewires_references_inside_other_token_values() {
        let mut set = TokenSet::new();
        set.define(TokenKind::Colour, "--brand", "var(--accent)");
        set.define(TokenKind::Colour, "--accent", "#3b82f6");
        set.rename("--accent", "--primary").expect("a rename");
        assert_eq!(
            set.tokens.get("--brand").expect("the token").value,
            "var(--primary)"
        );
        assert_eq!(
            set.expand("var(--brand)", None).expect("expands").value,
            "#3b82f6"
        );
    }

    #[test]
    fn renaming_onto_an_existing_name_is_refused_and_changes_nothing() {
        let mut set = tokens();
        assert!(set.rename("--accent", "--gap").is_err());
        assert!(set.tokens.contains_key("--accent"));
        assert_eq!(
            set.tokens.get("--gap").expect("the token").kind,
            TokenKind::Spacing
        );
    }

    #[test]
    fn renaming_a_token_that_does_not_exist_is_refused() {
        let mut set = tokens();
        assert!(set.rename("--missing", "--brand").is_err());
    }

    #[test]
    fn rename_references_rewrites_every_usage_and_counts_them() {
        let (text, count) =
            rename_references("var(--accent) 1px var(--accent)", "--accent", "--brand");
        assert_eq!(text, "var(--brand) 1px var(--brand)");
        assert_eq!(count, 2);
        let (untouched, none) = rename_references("var(--gap)", "--accent", "--brand");
        assert_eq!(untouched, "var(--gap)");
        assert_eq!(none, 0);
    }

    #[test]
    fn the_report_lists_tokens_themes_and_their_count() {
        let mut set = tokens();
        set.define_theme("dark", &[("--accent".to_owned(), "#60a5fa".to_owned())]);
        let lines = set.render();
        assert!(
            lines
                .iter()
                .any(|line| line.contains("token --accent (colour)"))
        );
        assert!(lines.iter().any(|line| line.contains("theme dark:")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("tokens: 2, themes: 1"))
        );
    }
}
