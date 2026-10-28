// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! What a change makes the preview do: a live update, or a rebuild that needs cargo (task T14.02).
//!
//! The acceptance line is *"a UI or existing-binding change does not start cargo"*. The decision is
//! therefore made from the edit itself, not from the file it arrived in:
//!
//! * **An edit names what it is.** [`Edit`] is a property, a style, an *existing* binding, an added
//!   binding, a node change or source text; [`Edit::kind`] answers whether it can be absorbed live or
//!   needs a rebuild (principle 7).
//! * **A burst is one update.** [`Updater::flush`] coalesces the pending edits into one [`Update`], so
//!   a fast series of edits becomes one preview update rather than one per keystroke.
//! * **One rebuilding edit makes the batch a rebuild.** The batch's kind is the strongest kind in it,
//!   so a structural edit hidden in a burst is not absorbed live by accident.
//! * **Our own write is not somebody else's edit.** [`Updater::push`] answers whether the source is an
//!   echo of what this process wrote, so a file watcher seeing our own output cannot loop.
//!
//! The classification stops at [`Kind`]; what the live session does with it is
//! [`Preview::apply_update`](crate::app::preview::preview_service::Preview::apply_update), which
//! answers `Applied`, `RestartRequired` or `Noop` — including for a live-classified edit the library
//! still refuses.

#![allow(dead_code)]

/// What an edit makes the preview do.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// The preview can absorb it without a build.
    Live,
    /// The generated code cannot absorb it; a rebuild is needed.
    Rebuild,
}

impl Kind {
    /// The kind's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Rebuild => "rebuild",
        }
    }

    /// Whether this kind starts cargo.
    pub fn needs_cargo(&self) -> bool {
        matches!(self, Self::Rebuild)
    }
}

/// One edit the designer made.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Edit {
    /// A property value changed.
    Property(String),
    /// An interaction style changed.
    Style(String),
    /// An existing binding's value or conversion changed.
    ExistingBinding(String),
    /// A binding was added, which the generated code must carry.
    AddBinding(String),
    /// A binding was removed.
    RemoveBinding(String),
    /// A node was added.
    AddNode(String),
    /// A node was removed.
    RemoveNode(String),
    /// Hand-written or generated text changed.
    Text(String),
}

impl Edit {
    /// What the edit makes the preview do.
    pub fn kind(&self) -> Kind {
        match self {
            // A property, an interaction style and an *existing* binding are all absorbed by the
            // running preview: the document model changed, not the generated code.
            Self::Property(_) | Self::Style(_) | Self::ExistingBinding(_) => Kind::Live,
            Self::AddBinding(_)
            | Self::RemoveBinding(_)
            | Self::AddNode(_)
            | Self::RemoveNode(_)
            | Self::Text(_) => Kind::Rebuild,
        }
    }

    /// The edit's kind name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Property(_) => "property",
            Self::Style(_) => "style",
            Self::ExistingBinding(_) => "existing-binding",
            Self::AddBinding(_) => "add-binding",
            Self::RemoveBinding(_) => "remove-binding",
            Self::AddNode(_) => "add-node",
            Self::RemoveNode(_) => "remove-node",
            Self::Text(_) => "text",
        }
    }

    /// What the edit changed.
    pub fn subject(&self) -> &str {
        match self {
            Self::Property(subject)
            | Self::Style(subject)
            | Self::ExistingBinding(subject)
            | Self::AddBinding(subject)
            | Self::RemoveBinding(subject)
            | Self::AddNode(subject)
            | Self::RemoveNode(subject)
            | Self::Text(subject) => subject,
        }
    }

    /// The edit in one line.
    pub fn render(&self) -> String {
        format!("{} {}", self.name(), self.subject())
    }
}

/// One preview update: the edits it carried, where they came from and what it needs.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Update {
    /// The sources the edits came from, in order of first appearance.
    pub sources: Vec<String>,
    /// The edits, in order.
    pub edits: Vec<Edit>,
    /// What the batch needs.
    pub kind: Kind,
}

impl Update {
    /// The update in one line per fact.
    pub fn render(&self) -> String {
        format!(
            "{} update from {} ({} edit(s))",
            self.kind.name(),
            self.sources.join(", "),
            self.edits.len()
        )
    }
}

/// Coalesces edits into preview updates and tells our own writes apart from external edits.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Updater {
    /// The source this process writes under; a change from it is an echo, not an edit.
    pub self_source: String,
    pending: Vec<(String, Edit)>,
    history: Vec<Update>,
}

impl Updater {
    /// An updater that treats this source as its own.
    pub fn new(self_source: impl Into<String>) -> Self {
        Self {
            self_source: self_source.into(),
            pending: Vec::new(),
            history: Vec::new(),
        }
    }

    /// Records an edit, answering whether it was taken (a change from our own source is not).
    pub fn push(&mut self, source: impl Into<String>, edit: Edit) -> bool {
        let source = source.into();
        if source == self.self_source {
            return false;
        }
        self.pending.push((source, edit));
        true
    }

    /// How many edits are waiting to be coalesced.
    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    /// Coalesces the pending edits into one update, or answers `None` when nothing is pending.
    pub fn flush(&mut self) -> Option<Update> {
        if self.pending.is_empty() {
            return None;
        }
        let taken = std::mem::take(&mut self.pending);
        let mut sources: Vec<String> = Vec::new();
        let mut edits: Vec<Edit> = Vec::new();
        let mut kind = Kind::Live;
        for (source, edit) in taken {
            if !sources.contains(&source) {
                sources.push(source);
            }
            if edit.kind().needs_cargo() {
                kind = Kind::Rebuild;
            }
            edits.push(edit);
        }
        let update = Update {
            sources,
            edits,
            kind,
        };
        self.history.push(update.clone());
        Some(update)
    }

    /// The updates already made.
    pub fn history(&self) -> &[Update] {
        &self.history
    }
}

#[cfg(test)]
mod tests {
    use super::{Edit, Kind, Update, Updater};

    #[test]
    fn a_property_edit_is_live_and_does_not_start_cargo() {
        let edit = Edit::Property("width".to_owned());
        assert_eq!(edit.kind(), Kind::Live);
        assert!(!edit.kind().needs_cargo());
    }

    #[test]
    fn a_style_edit_is_live() {
        assert_eq!(Edit::Style(":hover".to_owned()).kind(), Kind::Live);
    }

    #[test]
    fn an_existing_binding_edit_is_live() {
        let edit = Edit::ExistingBinding("color <- status".to_owned());
        assert_eq!(edit.kind(), Kind::Live);
        assert!(!edit.kind().needs_cargo());
    }

    #[test]
    fn adding_or_removing_a_binding_needs_a_rebuild() {
        assert_eq!(Edit::AddBinding("x".to_owned()).kind(), Kind::Rebuild);
        assert_eq!(Edit::RemoveBinding("x".to_owned()).kind(), Kind::Rebuild);
    }

    #[test]
    fn a_node_change_needs_a_rebuild() {
        assert_eq!(Edit::AddNode("button".to_owned()).kind(), Kind::Rebuild);
        assert_eq!(Edit::RemoveNode("button".to_owned()).kind(), Kind::Rebuild);
    }

    #[test]
    fn a_text_edit_needs_a_rebuild() {
        assert_eq!(Edit::Text("main.rs".to_owned()).kind(), Kind::Rebuild);
    }

    #[test]
    fn a_burst_coalesces_into_one_update() {
        let mut updater = Updater::new("preview");
        updater.push("editor", Edit::Property("width".to_owned()));
        updater.push("editor", Edit::Style(":hover".to_owned()));
        assert_eq!(updater.pending(), 2);
        let update = updater.flush().expect("an update");
        assert_eq!(update.edits.len(), 2);
        assert_eq!(update.kind, Kind::Live);
        assert_eq!(updater.pending(), 0);
    }

    #[test]
    fn one_rebuilding_edit_makes_the_whole_batch_a_rebuild() {
        let mut updater = Updater::new("preview");
        updater.push("editor", Edit::Property("width".to_owned()));
        updater.push("editor", Edit::AddNode("label".to_owned()));
        let update = updater.flush().expect("an update");
        assert_eq!(update.kind, Kind::Rebuild);
        assert!(update.kind.needs_cargo());
    }

    #[test]
    fn flushing_with_nothing_pending_answers_none() {
        let mut updater = Updater::new("preview");
        assert_eq!(updater.flush(), None);
        assert!(updater.history().is_empty());
    }

    #[test]
    fn an_echo_of_our_own_write_is_not_an_edit() {
        let mut updater = Updater::new("preview");
        assert!(!updater.push("preview", Edit::Text("generated.rs".to_owned())));
        assert_eq!(updater.pending(), 0);
        assert_eq!(updater.flush(), None);
        assert!(updater.push("editor", Edit::Property("width".to_owned())));
    }

    #[test]
    fn the_update_names_its_sources_in_order() {
        let mut updater = Updater::new("preview");
        updater.push("editor", Edit::Property("a".to_owned()));
        updater.push("inspector", Edit::Property("b".to_owned()));
        updater.push("editor", Edit::Property("c".to_owned()));
        let update = updater.flush().expect("an update");
        assert_eq!(update.sources, vec!["editor", "inspector"]);
    }

    #[test]
    fn the_history_keeps_every_update() {
        let mut updater = Updater::new("preview");
        updater.push("editor", Edit::Property("a".to_owned()));
        updater.flush();
        updater.push("editor", Edit::AddNode("n".to_owned()));
        updater.flush();
        assert_eq!(updater.history().len(), 2);
        assert_eq!(updater.history()[1].kind, Kind::Rebuild);
    }

    #[test]
    fn an_update_and_an_edit_render() {
        let update = Update {
            sources: vec!["editor".to_owned()],
            edits: vec![Edit::Property("width".to_owned())],
            kind: Kind::Live,
        };
        assert_eq!(update.render(), "live update from editor (1 edit(s))");
        assert_eq!(
            Edit::ExistingBinding("color".to_owned()).render(),
            "existing-binding color"
        );
        assert_eq!(Kind::Rebuild.name(), "rebuild");
    }
}
