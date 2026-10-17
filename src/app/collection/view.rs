// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The specialised graphic, chart and media views the library actually supports, and what stands in
//! for the ones it does not (task T27.07).
//!
//! The acceptance line is *"an editor is offered only for a real capability; do not use an image to
//! pretend a running feature"*. The design makes both structural rather than a matter of discipline:
//!
//! * **Support is a question with two honest answers.** [`capability`] answers [`Support::Supported`]
//!   with the data formats the view reads, or [`Support::Unsupported`] with a reason. There is no
//!   third "unknown" answer that a caller could read as supported.
//! * **An unsupported view gets no editor.** [`View::editor`] returns `None` for an unsupported kind,
//!   so the designer cannot offer to edit a view it could not draw.
//! * **No picture stands in for a feature.** [`Placeholder`] has a [`Placeholder::Notice`] variant for
//!   a view that is not live and a [`Placeholder::Sample`] variant that renders from the design's own
//!   data; it deliberately has no image variant, so a static picture cannot be placed where a running
//!   feature should be.

#![allow(dead_code)]

/// A specialised view kind.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ViewKind {
    /// A single-value gauge.
    Gauge,
    /// A bar chart.
    BarChart,
    /// A line chart.
    LineChart,
    /// A small inline sparkline.
    Sparkline,
    /// An image.
    Image,
    /// A video.
    Video,
    /// A map.
    Map,
}

impl ViewKind {
    /// The kind's name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Gauge => "gauge",
            Self::BarChart => "bar-chart",
            Self::LineChart => "line-chart",
            Self::Sparkline => "sparkline",
            Self::Image => "image",
            Self::Video => "video",
            Self::Map => "map",
        }
    }

    /// Every kind the designer knows about.
    pub fn all() -> [Self; 7] {
        [
            Self::Gauge,
            Self::BarChart,
            Self::LineChart,
            Self::Sparkline,
            Self::Image,
            Self::Video,
            Self::Map,
        ]
    }

    /// Reads a kind's name.
    pub fn parse(text: &str) -> Option<Self> {
        Self::all().into_iter().find(|kind| kind.name() == text)
    }
}

/// What the library can do for a view kind.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Support {
    /// The view is supported, and reads these data formats.
    Supported {
        /// The data formats the view reads.
        formats: Vec<&'static str>,
    },
    /// The view is not supported, and this is why.
    Unsupported {
        /// Why the view is not supported.
        reason: String,
    },
}

impl Support {
    /// Whether the view is supported.
    pub fn is_supported(&self) -> bool {
        matches!(self, Self::Supported { .. })
    }

    /// Why the view is not supported, when it is not.
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Supported { .. } => None,
            Self::Unsupported { reason } => Some(reason),
        }
    }

    /// The support in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Supported { formats } => {
                format!("supported, reads {}", formats.join(", "))
            }
            Self::Unsupported { reason } => format!("unsupported: {reason}"),
        }
    }
}

/// What the library can actually do for a view kind. A gap names its reason rather than being
/// represented by something that looks like the feature.
pub fn capability(kind: ViewKind) -> Support {
    match kind {
        ViewKind::Gauge => Support::Supported {
            formats: vec!["value"],
        },
        ViewKind::BarChart | ViewKind::LineChart => Support::Supported {
            formats: vec!["series", "labels"],
        },
        ViewKind::Sparkline => Support::Supported {
            formats: vec!["series"],
        },
        ViewKind::Image => Support::Supported {
            formats: vec!["png", "svg"],
        },
        ViewKind::Video => Support::Unsupported {
            reason: "the preview host has no media decoder".to_owned(),
        },
        ViewKind::Map => Support::Unsupported {
            reason: "a map needs a network service the project has not declared".to_owned(),
        },
    }
}

/// The editor offered for a supported view: the fields its data is edited through.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Editor {
    /// The fields the editor exposes.
    pub fields: Vec<&'static str>,
}

/// What stands in for a view that cannot be drawn live.
///
/// There is deliberately no image variant: an unsupported feature may not be drawn as a picture of
/// itself, because that would show a running feature where none runs.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Placeholder {
    /// Renders from the design's own sample data; the view is live.
    Sample(String),
    /// A labelled notice that the view is not live, with the reason.
    Notice(String),
}

impl Placeholder {
    /// Whether the placeholder is a live view.
    pub fn is_live(&self) -> bool {
        matches!(self, Self::Sample(_))
    }

    /// The placeholder in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Sample(data) => format!("sample: {data}"),
            Self::Notice(reason) => format!("not live: {reason}"),
        }
    }
}

/// A view of a kind, with its data.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct View {
    /// The view's kind.
    pub kind: ViewKind,
    /// The data the view reads.
    pub data: Vec<String>,
}

impl View {
    /// A view of a kind with some data.
    pub fn new(kind: ViewKind, data: &[&str]) -> Self {
        Self {
            kind,
            data: data.iter().map(|item| (*item).to_owned()).collect(),
        }
    }

    /// The editor offered for a view kind, or none when the library does not support it.
    pub fn editor(kind: ViewKind) -> Option<Editor> {
        match capability(kind) {
            Support::Supported { formats } => Some(Editor { fields: formats }),
            Support::Unsupported { .. } => None,
        }
    }

    /// What stands in for the view.
    ///
    /// A supported view renders from the design's own data; an unsupported one is a labelled notice,
    /// never a picture of the feature.
    pub fn placeholder(&self) -> Placeholder {
        match capability(self.kind) {
            Support::Supported { .. } => Placeholder::Sample(self.data.join(", ")),
            Support::Unsupported { reason } => Placeholder::Notice(reason),
        }
    }

    /// The view as lines, stating its support and what stands in for it.
    pub fn render(&self) -> Vec<String> {
        vec![
            format!(
                "{} view: {}",
                self.kind.name(),
                capability(self.kind).render()
            ),
            format!("data: {}", self.data.join(", ")),
            self.placeholder().render(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::{Placeholder, Support, View, ViewKind, capability};

    #[test]
    fn a_supported_view_is_offered_an_editor() {
        let editor = View::editor(ViewKind::BarChart).expect("an editor");
        assert!(editor.fields.contains(&"series"));
        assert!(capability(ViewKind::Gauge).is_supported());
    }

    #[test]
    fn an_unsupported_view_is_offered_no_editor() {
        assert_eq!(View::editor(ViewKind::Video), None);
        assert_eq!(View::editor(ViewKind::Map), None);
    }

    #[test]
    fn an_unsupported_view_states_its_reason() {
        let support = capability(ViewKind::Map);
        assert!(matches!(support, Support::Unsupported { .. }));
        assert!(
            support
                .reason()
                .is_some_and(|reason| reason.contains("network service"))
        );
        assert!(support.render().starts_with("unsupported:"));
    }

    #[test]
    fn an_unsupported_view_is_a_notice_and_never_a_picture() {
        let view = View::new(ViewKind::Video, &["clip.mp4"]);
        let placeholder = view.placeholder();
        assert!(matches!(placeholder, Placeholder::Notice(_)));
        assert!(!placeholder.is_live());
        assert!(placeholder.render().starts_with("not live:"));
    }

    #[test]
    fn a_supported_view_renders_from_sample_data() {
        let view = View::new(ViewKind::Sparkline, &["1", "3", "2"]);
        let placeholder = view.placeholder();
        assert!(placeholder.is_live());
        assert!(placeholder.render().contains("1, 3, 2"));
        assert!(view.render().iter().any(|line| line.contains("supported")));
    }

    #[test]
    fn every_kind_has_a_name_that_round_trips() {
        for kind in ViewKind::all() {
            assert_eq!(ViewKind::parse(kind.name()), Some(kind));
        }
        assert_eq!(ViewKind::parse("pie"), None);
    }
}
