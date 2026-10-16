// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! CSS: the text/rule view, the simple selectors, and the computed style with the source each value
//! came from (task T11.03).
//!
//! The acceptance line is *"invalid CSS is preserved, and a computed value is explainable"*. Both
//! are promises about not losing information, so the module keeps every rule and every provenance:
//!
//! * **CSS it cannot read is kept, not dropped.** [`Rule::Invalid`] carries the original text and a
//!   reason, so a rule the parser cannot read survives into the report — the user can see and fix it
//!   instead of watching it vanish, which is what a failed parse must do.
//! * **A computed value names the declaration it came from.** [`Computed::source_of`] answers the
//!   [`Source`] — the rule's index, its selector and the declaration's [`Span`] — so the value on
//!   screen can be jumped back to the exact text that produced it.
//! * **The cascade is one rule: specificity, then order.** [`compute`] lets a higher specificity win
//!   and breaks a tie with the later rule, so the inline text and the rule view cannot disagree about
//!   which declaration applies.
//! * **A selector matches the element's own facts only.** [`Selector`] reads a simple compound (tag,
//!   `#id`, `.class`) and, optionally, one interaction [`State`]; syntax outside that subset is
//!   refused as invalid rather than silently matching nothing, so "unsupported" stays visible
//!   instead of turning into "no match".
//!
//! T11.05 extended [`Selector`] with the optional interaction state and added [`compute_in`]; the
//! task above and its tests are otherwise unchanged.

#![allow(dead_code)]

/// One interaction state a selector can name and a preview can show.
///
/// A state is a *style* state: it says how a widget is drawn while it is hovered, pressed, focused or
/// disabled. It is never a business event, and previewing one triggers nothing (task T11.05).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    /// The pointer is over the widget.
    Hover,
    /// The widget is being pressed.
    Active,
    /// The widget holds keyboard focus.
    Focus,
    /// The widget is disabled.
    Disabled,
}

impl State {
    /// Every state, in the order a preview shows them.
    pub const ALL: [Self; 4] = [Self::Hover, Self::Active, Self::Focus, Self::Disabled];

    /// The state's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Hover => "hover",
            Self::Active => "active",
            Self::Focus => "focus",
            Self::Disabled => "disabled",
        }
    }

    /// The pseudo-class a selector writes (including the colon).
    pub fn selector(&self) -> String {
        format!(":{}", self.name())
    }

    /// Reads a state's name.
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|state| state.name() == text)
    }
}

/// A byte range in the document the rule was read from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Span {
    /// The first byte.
    pub start: usize,
    /// One past the last byte.
    pub end: usize,
}

impl Span {
    /// How many bytes the range covers.
    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    /// Whether the range is empty.
    pub fn is_empty(&self) -> bool {
        self.end <= self.start
    }
}

/// One `property: value` declaration and where in the document it sits.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Declaration {
    /// The property name, as written.
    pub property: String,
    /// The value, trimmed.
    pub value: String,
    /// The declaration's range.
    pub span: Span,
}

/// A simple compound selector: a tag, an id, any number of classes and one state.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Selector {
    /// The tag, when the selector names one.
    pub tag: Option<String>,
    /// The id, when the selector names one.
    pub id: Option<String>,
    /// The classes, in the order written.
    pub classes: Vec<String>,
    /// The interaction state, when the selector names one.
    pub state: Option<State>,
}

impl Selector {
    /// Reads a simple compound selector, or says what is not supported.
    ///
    /// The subset is deliberately small: a tag, at most one `#id`, any number of `.class`es and, at
    /// the end, one interaction state such as `:hover`. Combinators, `*`, attribute selectors and
    /// unknown pseudo-classes are refused rather than approximated, because a selector the target
    /// cannot evaluate is a difference worth showing (principle 8).
    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        let mut tag: Option<String> = None;
        let mut id: Option<String> = None;
        let mut classes: Vec<String> = Vec::new();
        let mut state: Option<State> = None;
        let mut index = 0;
        while index < text.len() {
            let rest = &text[index..];
            let marker = rest.chars().next().expect("a character");
            match marker {
                '#' | '.' => {
                    if state.is_some() {
                        return Err(format!("{text:?} is not a simple selector"));
                    }
                    let name = &rest[1..];
                    let len = identifier_len(name);
                    if len == 0 {
                        return Err(format!("{text:?} is not a simple selector"));
                    }
                    let value = name[..len].to_owned();
                    if marker == '#' {
                        if id.is_some() {
                            return Err(format!("{text:?} names more than one id"));
                        }
                        id = Some(value);
                    } else {
                        classes.push(value);
                    }
                    index += 1 + len;
                }
                ':' => {
                    let name = &rest[1..];
                    let len = identifier_len(name);
                    let found = (len > 0)
                        .then(|| State::parse(&name[..len]))
                        .flatten()
                        .ok_or_else(|| format!("{:?} is not a supported state", &name[..len]))?;
                    if state.is_some() {
                        return Err(format!("{text:?} names more than one state"));
                    }
                    state = Some(found);
                    index += 1 + len;
                }
                _ => {
                    let len = identifier_len(rest);
                    if len == 0 {
                        return Err(format!("{text:?} is not a simple selector"));
                    }
                    if tag.is_some() || id.is_some() || !classes.is_empty() {
                        return Err(format!("{text:?} is not a simple selector"));
                    }
                    tag = Some(rest[..len].to_owned());
                    index += len;
                }
            }
        }
        if tag.is_none() && id.is_none() && classes.is_empty() {
            return Err(format!("{text:?} is not a simple selector"));
        }
        Ok(Self {
            tag,
            id,
            classes,
            state,
        })
    }

    /// How specific the selector is: id, then class, then state, then tag.
    pub fn specificity(&self) -> u32 {
        u32::from(self.id.is_some()) * 100
            + self.classes.len() as u32 * 10
            + u32::from(self.state.is_some()) * 10
            + u32::from(self.tag.is_some())
    }

    /// Whether the selector describes this element at rest.
    pub fn matches(&self, element: &Element) -> bool {
        self.matches_in(element, None)
    }

    /// Whether the selector describes this element while it is in `state`.
    ///
    /// A selector with no state applies in every state; a selector with one applies only while the
    /// element is in that state.
    pub fn matches_in(&self, element: &Element, state: Option<State>) -> bool {
        if let Some(tag) = &self.tag
            && tag != &element.tag
        {
            return false;
        }
        if let Some(id) = &self.id
            && Some(id) != element.id.as_ref()
        {
            return false;
        }
        if let Some(required) = self.state
            && Some(required) != state
        {
            return false;
        }
        self.classes
            .iter()
            .all(|class| element.classes.contains(class))
    }

    /// The selector as it was written.
    pub fn render(&self) -> String {
        let mut out = String::new();
        if let Some(tag) = &self.tag {
            out.push_str(tag);
        }
        if let Some(id) = &self.id {
            out.push('#');
            out.push_str(id);
        }
        for class in &self.classes {
            out.push('.');
            out.push_str(class);
        }
        if let Some(state) = self.state {
            out.push_str(&state.selector());
        }
        out
    }
}

/// The element a computed style is asked about.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Element {
    /// The tag it renders as.
    pub tag: String,
    /// Its id, if it has one.
    pub id: Option<String>,
    /// Its classes.
    pub classes: Vec<String>,
}

impl Element {
    /// An element with a tag and nothing else.
    pub fn new(tag: impl Into<String>) -> Self {
        Self {
            tag: tag.into(),
            id: None,
            classes: Vec::new(),
        }
    }

    /// Reads an element from the same compound syntax a selector uses.
    ///
    /// An element must name a tag; a bare `#id` or `.class` describes a selector, not something on
    /// the canvas, and is refused rather than guessed at.
    pub fn parse(text: &str) -> Result<Self, String> {
        let selector = Selector::parse(text)?;
        if selector.state.is_some() {
            return Err(format!("{text:?} names a state, so it is not an element"));
        }
        match selector.tag {
            Some(tag) => Ok(Self {
                tag,
                id: selector.id,
                classes: selector.classes,
            }),
            None => Err(format!("{text:?} names no tag, so it is not an element")),
        }
    }

    /// The element as it was written.
    pub fn render(&self) -> String {
        Selector {
            tag: Some(self.tag.clone()),
            id: self.id.clone(),
            classes: self.classes.clone(),
            state: None,
        }
        .render()
    }
}

/// One rule of the sheet: either read, or kept verbatim with the reason it was not.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Rule {
    /// A rule the parser understood.
    Valid {
        /// What it selects.
        selector: Selector,
        /// What it declares.
        declarations: Vec<Declaration>,
        /// The rule's whole range, braces included.
        span: Span,
    },
    /// A block the parser could not read, kept exactly as written.
    Invalid {
        /// The original text.
        text: String,
        /// Why it could not be read.
        reason: String,
        /// The block's range in the document.
        span: Span,
    },
}

impl Rule {
    /// Whether the parser understood the rule.
    pub fn is_valid(&self) -> bool {
        matches!(self, Self::Valid { .. })
    }

    /// The rule's range in the document.
    pub fn span(&self) -> Span {
        match self {
            Self::Valid { span, .. } | Self::Invalid { span, .. } => *span,
        }
    }

    /// The rule in one line, naming an invalid rule's reason.
    pub fn render(&self, index: usize) -> String {
        match self {
            Self::Valid {
                selector,
                declarations,
                span,
            } => format!(
                "rule {index}: {} -> {} declaration(s) [{}..{}]",
                selector.render(),
                declarations.len(),
                span.start,
                span.end
            ),
            Self::Invalid { text, reason, span } => format!(
                "rule {index}: INVALID {text:?} — {reason} [{}..{}]",
                span.start, span.end
            ),
        }
    }
}

/// A parsed stylesheet: every rule, valid or not, in document order.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Css {
    /// The rules, in document order.
    pub rules: Vec<Rule>,
}

impl Css {
    /// Reads a stylesheet, keeping whatever it cannot read.
    ///
    /// A block is valid when it has a simple selector and `property: value` declarations. Anything
    /// else — stray text, an unclosed brace, a malformed declaration — becomes one [`Rule::Invalid`]
    /// that carries the original text, so nothing is quietly discarded.
    pub fn parse(text: &str) -> Self {
        let mut rules = Vec::new();
        let mut pos = 0;
        while pos < text.len() {
            let trimmed = text[pos..].trim_start();
            pos += text[pos..].len() - trimmed.len();
            if pos >= text.len() {
                break;
            }
            match text[pos..].find('{') {
                Some(open_rel) => {
                    let open = pos + open_rel;
                    match text[open + 1..].find('}') {
                        Some(close_rel) => {
                            let close = open + 1 + close_rel;
                            let span = Span {
                                start: pos,
                                end: close + 1,
                            };
                            let block = text[pos..=close].trim();
                            let selector_text = text[pos..open].trim();
                            let body = &text[open + 1..close];
                            rules.push(build_rule(span, block, selector_text, body, open + 1));
                            pos = close + 1;
                        }
                        None => {
                            rules.push(Rule::Invalid {
                                text: text[pos..].trim().to_owned(),
                                reason: "the rule is not closed — there is no '}'".to_owned(),
                                span: Span {
                                    start: pos,
                                    end: text.len(),
                                },
                            });
                            pos = text.len();
                        }
                    }
                }
                None => {
                    rules.push(Rule::Invalid {
                        text: text[pos..].trim().to_owned(),
                        reason: "not a rule — there is no '{'".to_owned(),
                        span: Span {
                            start: pos,
                            end: text.len(),
                        },
                    });
                    pos = text.len();
                }
            }
        }
        Self { rules }
    }

    /// How many rules the parser could not read.
    pub fn invalid_count(&self) -> usize {
        self.rules.iter().filter(|rule| !rule.is_valid()).count()
    }

    /// The rule view, one line per rule plus a summary.
    pub fn render(&self) -> Vec<String> {
        let mut lines: Vec<String> = self
            .rules
            .iter()
            .enumerate()
            .map(|(index, rule)| rule.render(index))
            .collect();
        lines.push(format!(
            "css: {} rule(s), {} invalid",
            self.rules.len(),
            self.invalid_count()
        ));
        lines
    }
}

/// Where a computed value came from.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Source {
    /// The index of the rule in the sheet.
    pub rule: usize,
    /// The declaration's range, the place to jump to.
    pub span: Span,
    /// The selector that won.
    pub selector: String,
}

impl Source {
    /// The source in one line: the rule, its selector and the jump target.
    pub fn render(&self) -> String {
        format!(
            "rule {} ({}) @ {}..{}",
            self.rule, self.selector, self.span.start, self.span.end
        )
    }
}

/// The style an element computes to, with each value's source.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Computed {
    values: std::collections::BTreeMap<String, (String, Source)>,
}

impl Computed {
    /// What a property computes to, if any rule sets it.
    pub fn value_of(&self, property: &str) -> Option<&str> {
        self.values.get(property).map(|(value, _)| value.as_str())
    }

    /// Which declaration produced a property's value, if any.
    pub fn source_of(&self, property: &str) -> Option<&Source> {
        self.values.get(property).map(|(_, source)| source)
    }

    /// The properties the element computes, in name order.
    pub fn properties(&self) -> Vec<&str> {
        self.values.keys().map(String::as_str).collect()
    }

    /// The computed style, one line per property, each naming its source.
    pub fn render(&self) -> Vec<String> {
        self.values
            .iter()
            .map(|(property, (value, source))| {
                format!("{property}: {value}  <- {}", source.render())
            })
            .collect()
    }
}

/// Computes an element's style at rest: specificity decides, and a tie goes to the later rule.
pub fn compute(css: &Css, element: &Element) -> Computed {
    compute_in(css, element, None)
}

/// Computes an element's style while it is in `state`, or at rest when `state` is `None`.
///
/// State rules (`button:hover { … }`) carry one extra class of specificity, so they override a base
/// rule of the same selector while the element is in that state, and only then.
pub fn compute_in(css: &Css, element: &Element, state: Option<State>) -> Computed {
    let mut winners: std::collections::BTreeMap<String, u32> = std::collections::BTreeMap::new();
    let mut values: std::collections::BTreeMap<String, (String, Source)> =
        std::collections::BTreeMap::new();
    for (index, rule) in css.rules.iter().enumerate() {
        let Rule::Valid {
            selector,
            declarations,
            ..
        } = rule
        else {
            continue;
        };
        if !selector.matches_in(element, state) {
            continue;
        }
        let specificity = selector.specificity();
        for declaration in declarations {
            // Rules are visited in document order, so `>=` lets a later rule of equal specificity
            // take over while a plain higher specificity always wins.
            let wins = winners
                .get(&declaration.property)
                .is_none_or(|current| specificity >= *current);
            if wins {
                winners.insert(declaration.property.clone(), specificity);
                values.insert(
                    declaration.property.clone(),
                    (
                        declaration.value.clone(),
                        Source {
                            rule: index,
                            span: declaration.span,
                            selector: selector.render(),
                        },
                    ),
                );
            }
        }
    }
    Computed { values }
}

/// Reads one rule's selector and body, or keeps the block with a reason.
fn build_rule(span: Span, block: &str, selector_text: &str, body: &str, body_start: usize) -> Rule {
    if selector_text.is_empty() {
        return Rule::Invalid {
            text: block.to_owned(),
            reason: "the rule has no selector".to_owned(),
            span,
        };
    }
    let selector = match Selector::parse(selector_text) {
        Ok(selector) => selector,
        Err(reason) => {
            return Rule::Invalid {
                text: block.to_owned(),
                reason,
                span,
            };
        }
    };
    match parse_declarations(body, body_start) {
        Ok(declarations) => Rule::Valid {
            selector,
            declarations,
            span,
        },
        Err(reason) => Rule::Invalid {
            text: block.to_owned(),
            reason,
            span,
        },
    }
}

/// Splits a rule body into declarations, refusing a piece without a `property: value` pair.
fn parse_declarations(body: &str, body_start: usize) -> Result<Vec<Declaration>, String> {
    let mut declarations = Vec::new();
    let mut offset = 0;
    for piece in body.split(';') {
        let piece_start = body_start + offset;
        offset += piece.len() + 1;
        let trimmed = piece.trim();
        if trimmed.is_empty() {
            continue;
        }
        let leading = piece.len() - piece.trim_start().len();
        let span = Span {
            start: piece_start + leading,
            end: piece_start + leading + trimmed.len(),
        };
        match trimmed.split_once(':') {
            Some((property, value)) => {
                let property = property.trim();
                if property.is_empty() {
                    return Err(format!("a declaration has no property: {trimmed:?}"));
                }
                declarations.push(Declaration {
                    property: property.to_owned(),
                    value: value.trim().to_owned(),
                    span,
                });
            }
            None => {
                return Err(format!("a declaration is missing ':': {trimmed:?}"));
            }
        }
    }
    Ok(declarations)
}

/// How many bytes of `text` are a name (letters, digits, `-` and `_`).
fn identifier_len(text: &str) -> usize {
    let mut len = 0;
    for character in text.chars() {
        if character.is_alphanumeric() || character == '-' || character == '_' {
            len += character.len_utf8();
        } else {
            break;
        }
    }
    len
}

#[cfg(test)]
mod tests {
    use super::{Css, Declaration, Element, Rule, Selector, Span, State, compute, compute_in};

    #[test]
    fn a_valid_rule_keeps_its_declarations_and_span() {
        let css = Css::parse(".a { color: red; gap: 8 }");
        assert_eq!(css.rules.len(), 1);
        let Rule::Valid {
            selector,
            declarations,
            span,
        } = &css.rules[0]
        else {
            panic!("a valid rule");
        };
        assert_eq!(selector.render(), ".a");
        assert_eq!(declarations.len(), 2);
        assert_eq!(
            declarations[0],
            Declaration {
                property: "color".to_owned(),
                value: "red".to_owned(),
                span: Span { start: 5, end: 15 },
            }
        );
        assert_eq!(declarations[1].property, "gap");
        assert_eq!(declarations[1].value, "8");
        assert_eq!(*span, Span { start: 0, end: 25 });
    }

    #[test]
    fn a_rule_with_a_malformed_declaration_is_invalid() {
        let css = Css::parse(".a { color }");
        assert_eq!(css.rules.len(), 1);
        assert_eq!(css.invalid_count(), 1);
        match &css.rules[0] {
            Rule::Invalid { text, reason, .. } => {
                assert_eq!(text, ".a { color }");
                assert!(reason.contains("':'"), "reason: {reason}");
            }
            other => panic!("expected an invalid rule, got {other:?}"),
        }
    }

    #[test]
    fn a_rule_with_no_selector_is_invalid() {
        let css = Css::parse("{ color: red }");
        assert_eq!(css.invalid_count(), 1);
        assert_eq!(css.rules.len(), 1);
    }

    #[test]
    fn an_unclosed_rule_is_preserved() {
        let css = Css::parse(".a { color: red");
        assert_eq!(css.rules.len(), 1);
        assert_eq!(css.invalid_count(), 1);
        match &css.rules[0] {
            Rule::Invalid { text, reason, .. } => {
                assert_eq!(text, ".a { color: red");
                assert!(reason.contains("'}'"), "reason: {reason}");
            }
            other => panic!("expected an invalid rule, got {other:?}"),
        }
    }

    #[test]
    fn trailing_text_that_is_not_a_rule_is_preserved() {
        let css = Css::parse(".a { color: red }\ncolor: blue");
        assert_eq!(css.rules.len(), 2);
        assert!(css.rules[0].is_valid());
        match &css.rules[1] {
            Rule::Invalid { text, reason, .. } => {
                assert_eq!(text, "color: blue");
                assert!(reason.contains("'{'"), "reason: {reason}");
            }
            other => panic!("expected an invalid rule, got {other:?}"),
        }
    }

    #[test]
    fn empty_text_has_no_rules() {
        assert!(Css::parse("").rules.is_empty());
        assert!(Css::parse("   \n\t ").rules.is_empty());
    }

    #[test]
    fn invalid_count_counts_only_the_rules_that_could_not_be_read() {
        let css = Css::parse(".a { color: red }\n.broken { gap }\nstray");
        assert_eq!(css.rules.len(), 3);
        assert_eq!(css.invalid_count(), 2);
    }

    #[test]
    fn the_report_names_every_rule() {
        let css = Css::parse(".a { color: red }\n.broken { gap }\nstray");
        let lines = css.render();
        for index in 0..3 {
            assert!(
                lines
                    .iter()
                    .any(|line| line.contains(&format!("rule {index}:"))),
                "no line for rule {index}"
            );
        }
        assert!(lines.iter().any(|line| line.contains("INVALID")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("3 rule(s), 2 invalid"))
        );
    }

    #[test]
    fn a_selector_matches_only_the_elements_it_describes() {
        let element = Element::parse("button#save.primary").expect("an element");
        assert!(
            Selector::parse("button.primary")
                .expect("a selector")
                .matches(&element)
        );
        assert!(
            Selector::parse("#save")
                .expect("a selector")
                .matches(&element)
        );
        assert!(
            !Selector::parse("button.secondary")
                .expect("a selector")
                .matches(&element)
        );
        assert!(
            !Selector::parse("input")
                .expect("a selector")
                .matches(&element)
        );
        assert!(
            !Selector::parse("#cancel")
                .expect("a selector")
                .matches(&element)
        );
    }

    #[test]
    fn specificity_ranks_id_above_class_above_tag() {
        assert_eq!(Selector::parse("button").unwrap().specificity(), 1);
        assert_eq!(Selector::parse(".a").unwrap().specificity(), 10);
        assert_eq!(Selector::parse("#a").unwrap().specificity(), 100);
        assert_eq!(Selector::parse("button.a.b#c").unwrap().specificity(), 121);
    }

    #[test]
    fn higher_specificity_wins_even_when_it_is_earlier() {
        let css = Css::parse("#save { color: red }\nbutton { color: blue }");
        let element = Element::parse("button#save").expect("an element");
        let computed = compute(&css, &element);
        assert_eq!(computed.value_of("color"), Some("red"));
    }

    #[test]
    fn at_equal_specificity_the_later_rule_wins() {
        let css = Css::parse(".a { color: red }\n.a { color: blue }");
        let element = Element::parse("button.a").expect("an element");
        let computed = compute(&css, &element);
        assert_eq!(computed.value_of("color"), Some("blue"));
    }

    #[test]
    fn a_computed_value_names_the_rule_and_span_it_came_from() {
        let css = Css::parse(".a { color: red }");
        let element = Element::parse("button.a").expect("an element");
        let computed = compute(&css, &element);
        let source = computed.source_of("color").expect("a source");
        assert_eq!(source.rule, 0);
        assert_eq!(source.selector, ".a");
        assert_eq!(source.span, Span { start: 5, end: 15 });
        assert!(computed.render()[0].contains("rule 0"));
    }

    #[test]
    fn a_property_no_rule_sets_has_no_value_or_source() {
        let css = Css::parse(".a { color: red }");
        let element = Element::parse("button.a").expect("an element");
        let computed = compute(&css, &element);
        assert_eq!(computed.value_of("gap"), None);
        assert_eq!(computed.source_of("gap"), None);
        assert_eq!(computed.properties(), vec!["color"]);
    }

    #[test]
    fn unsupported_selector_syntax_is_refused() {
        assert!(Selector::parse("button > span").is_err());
        assert!(Selector::parse(".a.b#c > .d").is_err());
        assert!(Selector::parse("*").is_err());
        assert!(Selector::parse(":wat").is_err());
        assert!(Selector::parse("button:hover:focus").is_err());
        assert!(Element::parse(".primary").is_err());
        assert!(Element::parse("button:hover").is_err());
        assert!(Element::parse("").is_err());
    }

    #[test]
    fn a_state_selector_carries_its_state_and_adds_specificity() {
        let selector = Selector::parse("button.primary:hover").expect("a selector");
        assert_eq!(selector.state, Some(State::Hover));
        assert_eq!(selector.render(), "button.primary:hover");
        assert_eq!(selector.specificity(), 21);
        assert_eq!(Selector::parse("button").expect("plain").specificity(), 1);
    }

    #[test]
    fn a_state_selector_matches_only_while_the_element_is_in_that_state() {
        let element = Element::parse("button.primary").expect("an element");
        let hover = Selector::parse("button:hover").expect("a selector");
        assert!(hover.matches_in(&element, Some(State::Hover)));
        assert!(!hover.matches_in(&element, Some(State::Focus)));
        assert!(!hover.matches_in(&element, None));
        let base = Selector::parse("button").expect("a selector");
        assert!(base.matches_in(&element, Some(State::Hover)));
        assert!(base.matches(&element));
    }

    #[test]
    fn a_state_rule_overrides_the_base_rule_only_in_that_state() {
        let css = Css::parse("button { color: red }\nbutton:hover { color: blue }");
        let element = Element::parse("button").expect("an element");
        assert_eq!(compute(&css, &element).value_of("color"), Some("red"));
        assert_eq!(
            compute_in(&css, &element, Some(State::Hover)).value_of("color"),
            Some("blue")
        );
        assert_eq!(
            compute_in(&css, &element, Some(State::Focus)).value_of("color"),
            Some("red")
        );
    }
}
