// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Diagnostics: contract **C-DIAGNOSTIC**.
//!
//! A diagnostic answers three questions: how bad is it, what does it say, and where is it. The
//! fourth question — "is this still true?" — is the one that goes wrong in practice, so every
//! diagnostic carries the revision it was computed at, and a set of them can be labelled
//! ([`DiagnosticSet::staleness`]) instead of being displayed as current.
//!
//! Two rules from whitepaper §17.6 are baked in:
//!
//! * A diagnostic that is linked to a control **also** keeps its source location. When the
//!   source map has no entry for a node — the case the generation contract expects, not an
//!   exception — the source position is still there to jump to.
//! * A stale diagnostic is shown as stale, and [`DiagnosticSet::current_entries`] exists so a
//!   caller that must not show outdated errors (a gate, a badge count) does not have to filter by
//!   hand and get it wrong. This is the same discipline as the probe report: an outdated result
//!   must not be presented as a current one.

use std::fmt;

use super::identity::NodeId;
use super::revision::{Revision, Staleness};
use super::source_map::SourceSpan;

/// How bad a diagnostic is. Ordered from most to least severe so a caller can sort or threshold
/// without a second mapping table.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Severity {
    /// Something is wrong and will not build or cannot be trusted.
    Error,
    /// Something is probably wrong, or is wrong but tolerable for now.
    Warning,
    /// Neither right nor wrong: information the user asked to see.
    Information,
    /// A suggestion, including the "quick fix" style prompts.
    Hint,
}

impl Severity {
    /// The machine name used in the contract data and in reports.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Information => "information",
            Self::Hint => "hint",
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// Where a diagnostic points.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum DiagnosticLocation {
    /// A place in a source file.
    Source(SourceSpan),
    /// A control, with the source position when there is one.
    ///
    /// The source field is not a fallback the caller may drop: without it, a diagnostic about a
    /// generated or hand-written region would have nothing to jump to when the node has no mapping.
    Node {
        node: NodeId,
        source: Option<SourceSpan>,
    },
    /// The project as a whole: a missing manifest, a target that is not installed.
    Project,
}

impl DiagnosticLocation {
    /// The source position, when the location has one.
    pub const fn source(&self) -> Option<&SourceSpan> {
        match self {
            Self::Source(span) => Some(span),
            Self::Node { source, .. } => source.as_ref(),
            Self::Project => None,
        }
    }

    /// The control, when the location names one.
    pub const fn node(&self) -> Option<NodeId> {
        match self {
            Self::Node { node, .. } => Some(*node),
            Self::Source(_) | Self::Project => None,
        }
    }

    /// The machine name of the location kind.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Source(_) => "source",
            Self::Node { .. } => "node",
            Self::Project => "project",
        }
    }
}

/// One diagnostic.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Diagnostic {
    severity: Severity,
    message: String,
    location: DiagnosticLocation,
    model: Revision,
}

impl Diagnostic {
    /// Builds a diagnostic, or `None` when the message is blank.
    pub fn new(
        severity: Severity,
        message: &str,
        location: DiagnosticLocation,
        model: Revision,
    ) -> Option<Self> {
        let message = message.trim();
        if message.is_empty() {
            return None;
        }
        Some(Self {
            severity,
            message: message.to_owned(),
            location,
            model,
        })
    }

    /// How bad it is.
    pub const fn severity(&self) -> Severity {
        self.severity
    }

    /// What it says.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Where it points.
    pub const fn location(&self) -> &DiagnosticLocation {
        &self.location
    }

    /// The model revision it was computed at.
    pub const fn model(&self) -> Revision {
        self.model
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} at {}: {}",
            self.severity,
            self.location.kind(),
            self.message
        )
    }
}

/// A set of diagnostics produced by one pass.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DiagnosticSet {
    model: Revision,
    entries: Vec<Diagnostic>,
}

impl DiagnosticSet {
    /// A set computed against `model`.
    pub fn new(model: Revision, entries: Vec<Diagnostic>) -> Self {
        Self { model, entries }
    }

    /// The revision the pass ran against.
    pub const fn model(&self) -> Revision {
        self.model
    }

    /// Every entry, including the ones that no longer apply.
    pub fn entries(&self) -> &[Diagnostic] {
        &self.entries
    }

    /// Whether the document has moved on since this pass.
    pub fn staleness(&self, current: Revision) -> Staleness {
        Staleness::between(self.model, current)
    }

    /// The entries that still describe the document.
    ///
    /// Returns an empty slice for a stale set on purpose: a caller that asks for current entries
    /// and silently receives outdated ones would be back to showing last pass's errors.
    pub fn current_entries(&self, current: Revision) -> &[Diagnostic] {
        if self.staleness(current).is_current() {
            &self.entries
        } else {
            &[]
        }
    }

    /// How many entries exist at or above the given severity, ignoring staleness.
    pub fn count_at_least(&self, severity: Severity) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.severity <= severity)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::{Diagnostic, DiagnosticLocation, DiagnosticSet, Severity};
    use crate::contracts::identity::NodeId;
    use crate::contracts::revision::{Revision, Staleness};
    use crate::contracts::source_map::SourceSpan;

    fn span() -> SourceSpan {
        SourceSpan::new("src/ui/main.rs", 10, 20).expect("the fixture span is valid")
    }

    #[test]
    fn a_blank_message_is_not_a_diagnostic() {
        assert!(
            Diagnostic::new(
                Severity::Error,
                "   ",
                DiagnosticLocation::Project,
                Revision::INITIAL
            )
            .is_none(),
        );
    }

    #[test]
    fn a_node_diagnostic_keeps_its_source_position() {
        let located = Diagnostic::new(
            Severity::Error,
            "the label overflows",
            DiagnosticLocation::Node {
                node: NodeId::from_raw(3),
                source: Some(span()),
            },
            Revision::from_raw(2),
        )
        .unwrap();

        assert_eq!(located.location().node(), Some(NodeId::from_raw(3)));
        assert_eq!(located.location().source(), Some(&span()));
        assert_eq!(located.location().kind(), "node");
        assert_eq!(located.model(), Revision::from_raw(2));

        let unmapped = Diagnostic::new(
            Severity::Warning,
            "generated node",
            DiagnosticLocation::Node {
                node: NodeId::from_raw(4),
                source: None,
            },
            Revision::from_raw(2),
        )
        .unwrap();

        assert_eq!(unmapped.location().source(), None);
        assert_eq!(unmapped.location().node(), Some(NodeId::from_raw(4)));
    }

    #[test]
    fn a_project_level_diagnostic_points_at_nothing_in_particular() {
        let project = Diagnostic::new(
            Severity::Error,
            "Cargo.toml is missing",
            DiagnosticLocation::Project,
            Revision::INITIAL,
        )
        .unwrap();

        assert_eq!(project.location().kind(), "project");
        assert_eq!(project.location().node(), None);
        assert_eq!(project.location().source(), None);
    }

    #[test]
    fn a_stale_set_offers_no_current_entries_instead_of_old_ones() {
        let entry = Diagnostic::new(
            Severity::Error,
            "unresolved name",
            DiagnosticLocation::Source(span()),
            Revision::from_raw(3),
        )
        .unwrap();
        let set = DiagnosticSet::new(Revision::from_raw(3), vec![entry]);

        assert_eq!(set.staleness(Revision::from_raw(3)), Staleness::Current);
        assert_eq!(set.current_entries(Revision::from_raw(3)).len(), 1);

        assert_eq!(
            set.staleness(Revision::from_raw(9)),
            Staleness::Stale {
                computed_at: Revision::from_raw(3),
                current: Revision::from_raw(9)
            },
        );
        assert!(set.current_entries(Revision::from_raw(9)).is_empty());
        assert_eq!(
            set.entries().len(),
            1,
            "the entries are kept, just not offered as current"
        );
    }

    #[test]
    fn severity_orders_from_error_downwards() {
        let entries = [
            Diagnostic::new(
                Severity::Hint,
                "consider this",
                DiagnosticLocation::Project,
                Revision::INITIAL,
            ),
            Diagnostic::new(
                Severity::Error,
                "this is broken",
                DiagnosticLocation::Project,
                Revision::INITIAL,
            ),
            Diagnostic::new(
                Severity::Warning,
                "check this",
                DiagnosticLocation::Project,
                Revision::INITIAL,
            ),
        ]
        .into_iter()
        .flatten()
        .collect();
        let set = DiagnosticSet::new(Revision::INITIAL, entries);

        assert_eq!(set.count_at_least(Severity::Error), 1);
        assert_eq!(set.count_at_least(Severity::Warning), 2);
        assert_eq!(set.count_at_least(Severity::Hint), 3);
        assert_eq!(Severity::Error.name(), "error");
    }
}
