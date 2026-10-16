// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Inlay hints, code lenses, hierarchies and semantic selection (task T07.09).
//!
//! The acceptance line is *"only negotiated capabilities are presented; a hint is versioned and
//! dismissible, and it neither crowds out input nor runs a command by itself"*. Four separate
//! promises, so four separate mechanisms:
//!
//! * **A hierarchy is presented only if its capability was negotiated.** [`Hierarchy::of`] takes the
//!   [`Capabilities`] and the [`HierarchyKind`] and refuses — [`Hierarchy::refusal`] — when the
//!   service did not offer call or type hierarchy. There is no path that shows an unnegotiated one.
//! * **A hint layer is versioned like everything else.** [`Hints`] reuses the one
//!   [`Pending`](crate::app::language::features::Pending) rule, so a layer for an old version is
//!   [`Delivery::Stale`](crate::app::language::features::Delivery::Stale) and never becomes the shown layer.
//! * **A hint is dismissible by kind.** [`Switches`] answers which kinds are on, and
//!   [`Hints::dismiss`] turns one off, so a person can stop the noise without stopping the service.
//! * **A hint is a decoration, never a write.** [`Hints`] holds no document and has no edit path, so a
//!   hint cannot be inserted into the buffer — "it does not crowd out input" is structural. And a
//!   [`Hint::command`] runs only through [`Hints::invoke`], so displaying a lens does not execute
//!   anything; a lens with no command cannot be invoked at all.
//!
//! Semantic selection is the [`structural`](crate::app::editor::structural) ladder driven by a syntax
//! capability, so this module does not build a second one; it is listed here because it is what the
//! negotiated [`Capability::SemanticSelection`] turns on.

#![allow(dead_code)]

use crate::app::language::features::{Delivery, Pending, Request, Response};
use crate::app::language::navigation::Span;
use crate::app::language::session::{Capabilities, Capability};

/// What a hint is for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HintKind {
    /// An inlay hint: a label shown inside the text.
    Inlay,
    /// A code lens: an actionable line above a definition.
    CodeLens,
}

impl HintKind {
    /// Every kind, in a stable order.
    pub const ALL: [Self; 2] = [Self::Inlay, Self::CodeLens];

    /// The kind's name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Inlay => "inlay",
            Self::CodeLens => "code-lens",
        }
    }

    /// The kind a name picks.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|kind| kind.name() == name)
    }

    /// The capability that must be negotiated for this kind to be shown.
    pub fn capability(self) -> Capability {
        match self {
            Self::Inlay => Capability::InlayHint,
            Self::CodeLens => Capability::CodeLens,
        }
    }
}

/// A command a code lens offers. It runs only when invoked.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LensCommand {
    /// The command's id.
    pub id: String,
    /// What a person sees.
    pub title: String,
}

impl LensCommand {
    /// A command.
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
        }
    }

    /// The command in one line.
    pub fn render(&self) -> String {
        format!("{} ({})", self.title, self.id)
    }
}

/// One hint.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Hint {
    /// Where it is.
    pub span: Span,
    /// What kind it is.
    pub kind: HintKind,
    /// What it says.
    pub label: String,
    /// The command it offers, for a lens that has one.
    pub command: Option<LensCommand>,
}

impl Hint {
    /// An inlay hint.
    pub fn inlay(span: Span, label: impl Into<String>) -> Self {
        Self {
            span,
            kind: HintKind::Inlay,
            label: label.into(),
            command: None,
        }
    }

    /// A code lens with a command.
    pub fn lens(span: Span, label: impl Into<String>, command: LensCommand) -> Self {
        Self {
            span,
            kind: HintKind::CodeLens,
            label: label.into(),
            command: Some(command),
        }
    }

    /// The hint in one line.
    pub fn render(&self) -> String {
        match &self.command {
            Some(command) => format!(
                "{}: {} [{}] {}",
                self.span.render(),
                self.label,
                self.kind.name(),
                command.render()
            ),
            None => format!(
                "{}: {} [{}]",
                self.span.render(),
                self.label,
                self.kind.name()
            ),
        }
    }
}

/// Which hint kinds are shown.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Switches {
    /// Whether inlay hints are shown.
    pub inlay: bool,
    /// Whether code lenses are shown.
    pub code_lens: bool,
}

impl Switches {
    /// Both kinds on.
    pub const fn all() -> Self {
        Self {
            inlay: true,
            code_lens: true,
        }
    }

    /// Whether a kind is shown.
    pub fn is_on(&self, kind: HintKind) -> bool {
        match kind {
            HintKind::Inlay => self.inlay,
            HintKind::CodeLens => self.code_lens,
        }
    }

    /// Turns a kind on or off.
    pub fn set(&mut self, kind: HintKind, on: bool) {
        match kind {
            HintKind::Inlay => self.inlay = on,
            HintKind::CodeLens => self.code_lens = on,
        }
    }
}

impl Default for Switches {
    fn default() -> Self {
        Self::all()
    }
}

/// The hint layer shown, versioned and dismissible.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Hints {
    pending: Pending,
    version: u64,
    switches: Switches,
    shown: Option<Vec<Hint>>,
    executed: Vec<String>,
}

impl Hints {
    /// A layer view over a document at a version.
    pub fn new(version: u64) -> Self {
        Self {
            pending: Pending::new(),
            version,
            switches: Switches::all(),
            shown: None,
            executed: Vec::new(),
        }
    }

    /// Records that the document moved to a new version.
    pub fn advance(&mut self, version: u64) {
        self.version = version;
    }

    /// Asks for hints against the current version.
    pub fn request(&mut self) -> Request {
        self.pending.request(self.version)
    }

    /// Asks for hints against a named version (a delayed request).
    pub fn request_at(&mut self, version: u64) -> Request {
        self.pending.request(version)
    }

    /// Takes a hint reply; only a fresh one becomes the shown layer.
    pub fn accept(&mut self, response: Response<Vec<Hint>>) -> Delivery<usize> {
        let current = self.version;
        match self.pending.deliver(response, current) {
            Delivery::Fresh(hints) => {
                let count = hints.len();
                self.shown = Some(hints);
                Delivery::Fresh(count)
            }
            Delivery::Stale { computed, current } => Delivery::Stale { computed, current },
            Delivery::Cancelled => Delivery::Cancelled,
        }
    }

    /// The switches in force.
    pub fn switches(&self) -> Switches {
        self.switches
    }

    /// Turns a hint kind on or off.
    pub fn dismiss(&mut self, kind: HintKind, on: bool) {
        self.switches.set(kind, on);
    }

    /// The shown hints whose kind is on.
    pub fn visible(&self) -> Vec<&Hint> {
        match &self.shown {
            Some(hints) => hints
                .iter()
                .filter(|hint| self.switches.is_on(hint.kind))
                .collect(),
            None => Vec::new(),
        }
    }

    /// Invokes a code lens's command by id, recording that it ran.
    ///
    /// This is the only path that runs a command, so showing a lens never does.
    pub fn invoke(&mut self, id: &str) -> Result<LensCommand, String> {
        let command = self
            .visible()
            .into_iter()
            .filter_map(|hint| hint.command.as_ref())
            .find(|command| command.id == id)
            .cloned();
        match command {
            Some(command) => {
                self.executed.push(id.to_owned());
                Ok(command)
            }
            None => Err(format!("no visible code lens offers {id:?}")),
        }
    }

    /// The command ids that have been invoked.
    pub fn executed(&self) -> &[String] {
        &self.executed
    }

    /// The layer in one report.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "hints at version {}, inlay {} code-lens {}",
            self.version,
            if self.switches.inlay { "on" } else { "off" },
            if self.switches.code_lens { "on" } else { "off" }
        )];
        for hint in self.visible() {
            lines.push(format!("  {}", hint.render()));
        }
        lines
    }
}

/// Which hierarchy a request asks for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HierarchyKind {
    /// Call hierarchy.
    Call,
    /// Type hierarchy.
    Type,
}

impl HierarchyKind {
    /// Both kinds, in a stable order.
    pub const ALL: [Self; 2] = [Self::Call, Self::Type];

    /// The kind's name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Call => "call",
            Self::Type => "type",
        }
    }

    /// The kind a name picks.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|kind| kind.name() == name)
    }

    /// The capability that must be negotiated for this hierarchy.
    pub fn capability(self) -> Capability {
        match self {
            Self::Call => Capability::CallHierarchy,
            Self::Type => Capability::TypeHierarchy,
        }
    }
}

/// A node of a hierarchy.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct HierarchyNode {
    /// The node's name.
    pub name: String,
    /// A detail line.
    pub detail: Option<String>,
    /// The nodes below it.
    pub children: Vec<HierarchyNode>,
}

impl HierarchyNode {
    /// A node with no children.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            detail: None,
            children: Vec::new(),
        }
    }

    /// The same node with children.
    pub fn with_children(mut self, children: Vec<HierarchyNode>) -> Self {
        self.children = children;
        self
    }

    /// The node in lines, indented by depth.
    pub fn render(&self, depth: usize) -> Vec<String> {
        let detail = self
            .detail
            .as_ref()
            .map(|detail| format!(" ({detail})"))
            .unwrap_or_default();
        let mut lines = vec![format!("{}{}{detail}", "  ".repeat(depth), self.name)];
        for child in &self.children {
            lines.extend(child.render(depth + 1));
        }
        lines
    }
}

/// Why a hierarchy could not be shown.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct HierarchyRefusal {
    /// The kind asked for.
    pub kind: HierarchyKind,
    /// The capability it needs.
    pub capability: Capability,
}

impl HierarchyRefusal {
    /// The refusal in one sentence.
    pub fn message(&self) -> String {
        format!(
            "the service did not negotiate {}, so the {} hierarchy is disabled",
            self.capability.name(),
            self.kind.name()
        )
    }
}

/// Builds a hierarchy only if its capability was negotiated.
pub fn hierarchy(
    kind: HierarchyKind,
    capabilities: &Capabilities,
    root: HierarchyNode,
) -> Result<HierarchyNode, HierarchyRefusal> {
    if !capabilities.supports(kind.capability()) {
        return Err(HierarchyRefusal {
            kind,
            capability: kind.capability(),
        });
    }
    Ok(root)
}

#[cfg(test)]
mod tests {
    use super::{
        HierarchyKind, HierarchyNode, Hint, HintKind, Hints, LensCommand, Switches, hierarchy,
    };
    use crate::app::language::features::{Delivery, Response};
    use crate::app::language::navigation::Span;
    use crate::app::language::session::{Capabilities, Capability};

    fn hints() -> Vec<Hint> {
        vec![
            Hint::inlay(Span::new(2, 2), ": i32"),
            Hint::lens(
                Span::new(0, 0),
                "3 references",
                LensCommand::new("showReferences", "Show references"),
            ),
        ]
    }

    #[test]
    fn only_a_negotiated_hierarchy_is_presented() {
        let root = HierarchyNode::new("main").with_children(vec![HierarchyNode::new("helper")]);
        let all = Capabilities::all();
        assert!(hierarchy(HierarchyKind::Call, &all, root.clone()).is_ok());
        // Without the call-hierarchy capability it is refused, not shown empty.
        let without = Capabilities::of([Capability::Completion]);
        match hierarchy(HierarchyKind::Call, &without, root) {
            Err(refusal) => {
                assert_eq!(refusal.kind, HierarchyKind::Call);
                assert!(refusal.message().contains("call-hierarchy"));
            }
            Ok(_) => panic!("an unnegotiated hierarchy was presented"),
        }
    }

    #[test]
    fn a_stale_hint_layer_is_not_shown() {
        let mut layer = Hints::new(1);
        let request = layer.request();
        layer.advance(2);
        assert_eq!(
            layer.accept(Response::new(request, hints())),
            Delivery::Stale {
                computed: 1,
                current: 2
            }
        );
        assert!(layer.visible().is_empty());
    }

    #[test]
    fn a_dismissed_kind_is_not_shown() {
        let mut layer = Hints::new(1);
        let request = layer.request();
        layer.accept(Response::new(request, hints()));
        assert_eq!(layer.visible().len(), 2);
        layer.dismiss(HintKind::Inlay, false);
        // Only the lens remains, and the reason is the switch.
        let visible = layer.visible();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].kind, HintKind::CodeLens);
    }

    #[test]
    fn showing_hints_runs_no_command_but_invoking_does() {
        let mut layer = Hints::new(1);
        let request = layer.request();
        layer.accept(Response::new(request, hints()));
        // Displaying ran nothing.
        assert!(layer.executed().is_empty());
        // Invoking the lens runs exactly that command.
        let command = layer.invoke("showReferences").expect("the lens command");
        assert_eq!(command.title, "Show references");
        assert_eq!(layer.executed(), &["showReferences".to_owned()]);
    }

    #[test]
    fn an_inlay_hint_has_no_command_to_invoke() {
        let mut layer = Hints::new(1);
        let request = layer.request();
        layer.accept(Response::new(
            request,
            vec![Hint::inlay(Span::new(0, 0), ": i32")],
        ));
        assert!(layer.invoke("anything").is_err());
        assert!(layer.executed().is_empty());
    }

    #[test]
    fn a_dismissed_lens_cannot_be_invoked() {
        let mut layer = Hints::new(1);
        let request = layer.request();
        layer.accept(Response::new(request, hints()));
        layer.dismiss(HintKind::CodeLens, false);
        assert!(layer.invoke("showReferences").is_err());
        assert!(layer.executed().is_empty());
    }

    #[test]
    fn switches_and_names_round_trip() {
        let mut switches = Switches::all();
        assert!(switches.is_on(HintKind::Inlay));
        switches.set(HintKind::Inlay, false);
        assert!(!switches.is_on(HintKind::Inlay));
        for kind in HintKind::ALL {
            assert_eq!(HintKind::from_name(kind.name()), Some(kind));
        }
        for kind in HierarchyKind::ALL {
            assert_eq!(HierarchyKind::from_name(kind.name()), Some(kind));
        }
    }
}
