// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Rich content: formatted runs, fonts, links and images, and what a target actually supports
//! (task T27.06).
//!
//! The acceptance line is *"link execution is permission-gated; rich-text format / target limits are
//! visible"*. Two rules keep the design honest about what it can show:
//!
//! * **A link needs its capability.** [`execute_link`] asks the project's [`PermissionGate`] for
//!   [`Capability::RequestNetworkService`] before it does anything. A denied capability is a refusal
//!   with the gate's stated reason, not a quiet success.
//! * **The limits are a value, not a hope.** [`Limits`] names the formats and the length a target
//!   accepts, and [`Limits::report`] prints them. [`RichText::validate`] checks content against them
//!   and names every violation, so a format the target lacks is reported rather than silently
//!   dropped.
//! * **An image is described.** [`Image::new`] refuses a blank alt description, because an image with
//!   no description is content a reader cannot reach.

#![allow(dead_code)]

use crate::contracts::permission::{Capability, Decision, PermissionGate, Principal};

/// A rich-text format the editor offers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    /// No formatting.
    Plain,
    /// Bold.
    Bold,
    /// Italic.
    Italic,
    /// Underlined.
    Underline,
    /// Inline code.
    Code,
    /// A hyperlink.
    Link,
}

impl Format {
    /// The format's name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::Bold => "bold",
            Self::Italic => "italic",
            Self::Underline => "underline",
            Self::Code => "code",
            Self::Link => "link",
        }
    }

    /// Every format the library can render.
    pub fn all() -> [Self; 6] {
        [
            Self::Plain,
            Self::Bold,
            Self::Italic,
            Self::Underline,
            Self::Code,
            Self::Link,
        ]
    }

    /// Reads a format's name.
    pub fn parse(text: &str) -> Option<Self> {
        Self::all().into_iter().find(|format| format.name() == text)
    }
}

/// A run of text with its formats.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Run {
    /// The run's text.
    pub text: String,
    /// The formats applied to it.
    pub formats: Vec<Format>,
}

impl Run {
    /// A run of text with formats, refusing empty text or a `plain` format mixed with another.
    pub fn new(text: &str, formats: &[Format]) -> Result<Self, String> {
        if text.is_empty() {
            return Err("a run needs some text".to_owned());
        }
        if formats.contains(&Format::Plain) && formats.len() > 1 {
            return Err("plain text cannot be combined with another format".to_owned());
        }
        Ok(Self {
            text: text.to_owned(),
            formats: formats.to_vec(),
        })
    }

    /// Whether the run carries a format.
    pub fn has(&self, format: Format) -> bool {
        self.formats.contains(&format)
    }

    /// The run in one line.
    pub fn render(&self) -> String {
        if self.formats.is_empty() {
            return self.text.clone();
        }
        let names: Vec<&str> = self.formats.iter().map(|format| format.name()).collect();
        format!("{} [{}]", self.text, names.join("+"))
    }
}

/// A font a run is set in.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Font {
    /// The font family.
    pub family: String,
    /// The size in points; always positive.
    pub size: u32,
    /// The weight, a hundred at a time from 100 to 900.
    pub weight: u32,
}

impl Font {
    /// A font, refusing a blank family, a zero size or a weight outside 100..=900.
    pub fn new(family: &str, size: u32, weight: u32) -> Result<Self, String> {
        if family.trim().is_empty() {
            return Err("a font needs a family".to_owned());
        }
        if size == 0 {
            return Err("a font needs a positive size".to_owned());
        }
        if !(100..=900).contains(&weight) || !weight.is_multiple_of(100) {
            return Err(format!("{weight} is not a font weight from 100 to 900"));
        }
        Ok(Self {
            family: family.to_owned(),
            size,
            weight,
        })
    }

    /// The font in one line.
    pub fn render(&self) -> String {
        format!("{} {}pt at weight {}", self.family, self.size, self.weight)
    }
}

/// A hyperlink with its target and an alternative description.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Link {
    /// Where the link goes.
    pub target: String,
    /// The description shown when the target cannot be read.
    pub alt: String,
}

impl Link {
    /// A link, refusing a blank target or a blank description.
    pub fn new(target: &str, alt: &str) -> Result<Self, String> {
        if target.trim().is_empty() {
            return Err("a link needs a target".to_owned());
        }
        if alt.trim().is_empty() {
            return Err("a link needs an alternative description".to_owned());
        }
        Ok(Self {
            target: target.to_owned(),
            alt: alt.to_owned(),
        })
    }
}

/// An image with its asset and an alternative description.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Image {
    /// The project-relative path of the image.
    pub asset: String,
    /// The description shown when the image cannot be read.
    pub alt: String,
}

impl Image {
    /// An image, refusing a blank asset or a blank description.
    pub fn new(asset: &str, alt: &str) -> Result<Self, String> {
        if asset.trim().is_empty() {
            return Err("an image needs an asset".to_owned());
        }
        if alt.trim().is_empty() {
            return Err("an image needs an alternative description".to_owned());
        }
        Ok(Self {
            asset: asset.to_owned(),
            alt: alt.to_owned(),
        })
    }
}

/// What a target accepts.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Limits {
    /// The longest text the target accepts, in characters.
    pub max_length: usize,
    /// The formats the target can render.
    pub formats: Vec<Format>,
}

impl Limits {
    /// The limits of a target.
    pub fn new(max_length: usize, formats: &[Format]) -> Self {
        Self {
            max_length,
            formats: formats.to_vec(),
        }
    }

    /// The limits of a target that accepts every format the library has.
    pub fn permissive(max_length: usize) -> Self {
        Self::new(max_length, &Format::all())
    }

    /// Whether the target can render a format.
    pub fn supports(&self, format: Format) -> bool {
        self.formats.contains(&format)
    }

    /// The limits as lines, so the formats and the length are visible rather than assumed.
    pub fn report(&self) -> Vec<String> {
        vec![
            format!(
                "formats: {}",
                self.formats
                    .iter()
                    .map(|format| format.name())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            format!("maximum length: {} character(s)", self.max_length),
        ]
    }

    /// Checks content against the limits, naming every violation.
    pub fn check(&self, text: &str, formats: &[Format]) -> Result<(), Vec<String>> {
        let mut violations = Vec::new();
        if text.chars().count() > self.max_length {
            violations.push(format!(
                "the content is {} character(s), over the target's {}",
                text.chars().count(),
                self.max_length
            ));
        }
        for format in formats {
            if !self.supports(*format) {
                violations.push(format!(
                    "the target does not support the {} format",
                    format.name()
                ));
            }
        }
        if violations.is_empty() {
            Ok(())
        } else {
            Err(violations)
        }
    }
}

/// A rich-text value: its runs and the font they are set in.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RichText {
    /// The runs, in order.
    pub runs: Vec<Run>,
    /// The font the runs are set in.
    pub font: Font,
}

impl RichText {
    /// The whole text, runs concatenated.
    pub fn text(&self) -> String {
        self.runs.iter().map(|run| run.text.as_str()).collect()
    }

    /// Every format used, in the library's order and without repetition.
    pub fn formats(&self) -> Vec<Format> {
        Format::all()
            .into_iter()
            .filter(|format| self.runs.iter().any(|run| run.has(*format)))
            .collect()
    }

    /// The content as lines.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![self.font.render()];
        lines.extend(self.runs.iter().map(|run| run.render()));
        lines
    }

    /// Checks the content and its formats against a target's limits.
    pub fn validate(&self, limits: &Limits) -> Result<(), Vec<String>> {
        limits.check(&self.text(), &self.formats())
    }
}

/// Executes a link through the project's permission gate.
///
/// The link needs [`Capability::RequestNetworkService`] because following it reaches a destination the
/// project has to have declared. A denied capability is a refusal carrying the gate's stated reason.
pub fn execute_link(
    gate: &PermissionGate,
    principal: Principal,
    link: &Link,
) -> Result<String, String> {
    match gate.decide(principal, Capability::RequestNetworkService) {
        Decision::Allowed { .. } => Ok(format!("opened {} ({})", link.target, link.alt)),
        Decision::Denied { reason, .. } => Err(format!(
            "the link to {} is denied: {}",
            link.target,
            reason.explanation()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{Font, Format, Image, Limits, Link, RichText, Run, execute_link};
    use crate::contracts::permission::{Capability, PermissionGate, Principal};

    fn font() -> Font {
        Font::new("Inter", 14, 400).expect("a font")
    }

    #[test]
    fn a_denied_link_is_refused() {
        let gate = PermissionGate::new();
        let link = Link::new("https://example.test", "the docs").expect("a link");
        let refused = execute_link(&gate, Principal::Human, &link).expect_err("a refusal");
        assert!(refused.contains("denied"));
    }

    #[test]
    fn an_allowed_link_opens_its_target() {
        let mut gate = PermissionGate::new();
        gate.trust_project();
        gate.set_network_policy(true);
        gate.authorize(Capability::RequestNetworkService);
        let link = Link::new("https://example.test", "the docs").expect("a link");
        let opened = execute_link(&gate, Principal::Human, &link).expect("an opening");
        assert!(opened.contains("https://example.test"));
    }

    #[test]
    fn a_format_the_target_lacks_is_reported_not_dropped() {
        let limits = Limits::new(100, &[Format::Plain]);
        let runs = vec![
            Run::new("plain ", &[Format::Plain]).expect("a run"),
            Run::new("bold", &[Format::Bold]).expect("a run"),
        ];
        let body = RichText { runs, font: font() };
        let violations = body.validate(&limits).expect_err("a violation");
        assert_eq!(violations.len(), 1);
        assert!(violations[0].contains("bold"));
        // The format is still part of the content: it was reported, not removed.
        assert!(body.formats().contains(&Format::Bold));
    }

    #[test]
    fn too_long_content_is_refused() {
        let limits = Limits::permissive(4);
        let body = RichText {
            runs: vec![Run::new("longer than four", &[]).expect("a run")],
            font: font(),
        };
        let violations = body.validate(&limits).expect_err("a violation");
        assert!(violations[0].contains("over the target's 4"));
    }

    #[test]
    fn the_limits_report_names_the_formats_and_the_length() {
        let limits = Limits::new(80, &[Format::Plain, Format::Bold]);
        let report = limits.report().join("\n");
        assert!(report.contains("plain, bold"));
        assert!(report.contains("80"));
    }

    #[test]
    fn an_image_needs_an_alt_description() {
        assert!(Image::new("logo.svg", "").is_err());
        let image = Image::new("logo.svg", "the project logo").expect("an image");
        assert_eq!(image.alt, "the project logo");
    }

    #[test]
    fn a_link_needs_a_target_and_a_description() {
        assert!(Link::new("", "text").is_err());
        assert!(Link::new("https://example.test", "  ").is_err());
    }

    #[test]
    fn a_run_refuses_empty_text_and_a_mixed_plain_format() {
        assert!(Run::new("", &[]).is_err());
        assert!(Run::new("x", &[Format::Plain, Format::Bold]).is_err());
    }

    #[test]
    fn a_font_is_validated() {
        assert!(Font::new("", 12, 400).is_err());
        assert!(Font::new("Inter", 0, 400).is_err());
        assert!(Font::new("Inter", 12, 450).is_err());
    }

    #[test]
    fn rich_text_reports_the_formats_it_uses_in_order() {
        let body = RichText {
            runs: vec![
                Run::new("a", &[Format::Bold]).expect("a run"),
                Run::new("b", &[Format::Code]).expect("a run"),
            ],
            font: font(),
        };
        assert_eq!(body.formats(), vec![Format::Bold, Format::Code]);
        assert_eq!(body.text(), "ab");
    }
}
