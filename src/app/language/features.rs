// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Completion, snippets, hover, signature help, documentation and cancellation (task T07.03).
//!
//! The acceptance line is *"a stale response does not overwrite the current buffer"*. A service
//! answers asynchronously, so a reply can arrive after the person has typed past the place it was
//! asked about; the module makes that impossible to mishandle by **stamping every request with the
//! version it was made against and refusing to show a reply for any other version**:
//!
//! * **A request carries its version; a response carries its request back.** [`Request`] is an id and
//!   a document version, [`Response`] hands the same [`Request`] back, so a reply is never anonymous.
//! * **Freshness is decided in one place.** [`Pending::deliver`] compares the reply's version with
//!   the document's current one and answers [`Delivery::Fresh`], [`Delivery::Stale`] or
//!   [`Delivery::Cancelled`]. Only a [`Delivery::Fresh`] is ever stored as the shown list, so a stale
//!   list cannot become the list a person accepts.
//! * **Cancellation is a request fact, not a race.** [`Pending::cancel`] records the id, and a reply
//!   for it is [`Delivery::Cancelled`] even if it happens to be current — the person already moved on.
//! * **A snippet completion is a [T06.09](crate::app::editor::snippets) snippet.** An item can carry a body in
//!   the one snippet grammar the editor already has, so a service snippet expands through exactly the
//!   placeholder machinery (`Snippet::parse`) rather than a second one.
//!
//! The rendering of hover, signature help and documentation is here as data; where they are painted
//! is the designer's subject, not this module's.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use crate::app::editor::snippets::{Snippet, SnippetError};
use crate::app::language::protocol::{Position, Range};

/// What kind of completion item this is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ItemKind {
    /// Plain text.
    Text,
    /// A function.
    Function,
    /// A method.
    Method,
    /// A variable.
    Variable,
    /// A keyword.
    Keyword,
    /// A snippet, whose insert text is in the snippet grammar.
    Snippet,
}

impl ItemKind {
    /// The kind's name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Function => "function",
            Self::Method => "method",
            Self::Variable => "variable",
            Self::Keyword => "keyword",
            Self::Snippet => "snippet",
        }
    }

    /// The kind a name picks.
    pub fn from_name(name: &str) -> Option<Self> {
        [
            Self::Text,
            Self::Function,
            Self::Method,
            Self::Variable,
            Self::Keyword,
            Self::Snippet,
        ]
        .into_iter()
        .find(|kind| kind.name() == name)
    }
}

/// One completion the service offered.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CompletionItem {
    /// What a person sees in the list.
    pub label: String,
    /// What is inserted.
    pub insert_text: String,
    /// What kind of item it is.
    pub kind: ItemKind,
    /// A short detail beside the label.
    pub detail: Option<String>,
    /// Documentation shown for the item.
    pub documentation: Option<String>,
}

impl CompletionItem {
    /// A plain item.
    pub fn text(label: impl Into<String>) -> Self {
        let label = label.into();
        Self {
            insert_text: label.clone(),
            label,
            kind: ItemKind::Text,
            detail: None,
            documentation: None,
        }
    }

    /// An item of a kind, with a label and its insert text.
    pub fn new(label: impl Into<String>, insert_text: impl Into<String>, kind: ItemKind) -> Self {
        Self {
            label: label.into(),
            insert_text: insert_text.into(),
            kind,
            detail: None,
            documentation: None,
        }
    }

    /// A snippet item: its body is in the snippet grammar.
    pub fn snippet(label: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            insert_text: body.into(),
            kind: ItemKind::Snippet,
            detail: None,
            documentation: None,
        }
    }

    /// The snippet this item stands for, when it is one.
    pub fn as_snippet(&self) -> Option<Result<Snippet, SnippetError>> {
        match self.kind {
            ItemKind::Snippet => Some(Snippet::parse(&self.label, &self.insert_text)),
            _ => None,
        }
    }

    /// The item in one line.
    pub fn render(&self) -> String {
        match &self.detail {
            Some(detail) => format!("{} [{}] ({detail})", self.label, self.kind.name()),
            None => format!("{} [{}]", self.label, self.kind.name()),
        }
    }
}

/// A hover reply.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Hover {
    /// The text to show.
    pub contents: String,
    /// The range it applies to, when the service gave one.
    pub range: Option<Range>,
}

/// One signature of a callable.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Signature {
    /// The signature's label.
    pub label: String,
    /// One label per parameter.
    pub parameters: Vec<String>,
    /// Documentation for the signature.
    pub documentation: Option<String>,
}

/// A signature-help reply.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SignatureHelp {
    /// The overloads.
    pub signatures: Vec<Signature>,
    /// Which overload is active.
    pub active_signature: usize,
    /// Which parameter is active.
    pub active_parameter: usize,
}

impl SignatureHelp {
    /// The active signature, when the index is in range.
    pub fn active(&self) -> Option<&Signature> {
        self.signatures.get(self.active_signature)
    }

    /// The reply in one line.
    pub fn render(&self) -> String {
        match self.active() {
            Some(signature) => format!(
                "signature {}/{}: {} (parameter {})",
                self.active_signature + 1,
                self.signatures.len(),
                signature.label,
                self.active_parameter
            ),
            None => "no signature".to_owned(),
        }
    }
}

/// A request for a service: an id, and the document version it was made against.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Request {
    /// The request's id.
    pub id: u64,
    /// The document version at the time of the request.
    pub version: u64,
}

/// A reply: the request it answers, and its value.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Response<T> {
    /// The request being answered.
    pub request: Request,
    /// The value.
    pub value: T,
}

impl<T> Response<T> {
    /// A reply to a request.
    pub fn new(request: Request, value: T) -> Self {
        Self { request, value }
    }
}

/// What became of a reply.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Delivery<T> {
    /// The reply is for the current document and may be shown.
    Fresh(T),
    /// The reply is for an older version and must not be shown.
    Stale {
        /// The version the reply was computed against.
        computed: u64,
        /// The document's current version.
        current: u64,
    },
    /// The request was cancelled before the reply arrived.
    Cancelled,
}

/// The outstanding requests and the freshness rule.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Pending {
    next: u64,
    outstanding: BTreeMap<u64, u64>,
    cancelled: BTreeSet<u64>,
}

impl Pending {
    /// No requests outstanding.
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts a request against a document version.
    pub fn request(&mut self, version: u64) -> Request {
        let id = self.next;
        self.next += 1;
        self.outstanding.insert(id, version);
        Request { id, version }
    }

    /// Cancels a request, so a reply for it is dropped.
    pub fn cancel(&mut self, id: u64) -> bool {
        if self.outstanding.remove(&id).is_some() {
            self.cancelled.insert(id);
            true
        } else {
            false
        }
    }

    /// Decides what becomes of a reply against the document's current version.
    pub fn deliver<T>(&mut self, response: Response<T>, current: u64) -> Delivery<T> {
        let id = response.request.id;
        self.outstanding.remove(&id);
        if self.cancelled.remove(&id) {
            return Delivery::Cancelled;
        }
        if response.request.version != current {
            return Delivery::Stale {
                computed: response.request.version,
                current,
            };
        }
        Delivery::Fresh(response.value)
    }

    /// How many requests are outstanding.
    pub fn outstanding(&self) -> usize {
        self.outstanding.len()
    }
}

/// A completion session: the shown list, kept fresh by the request rule.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Completions {
    pending: Pending,
    version: u64,
    shown: Option<Vec<CompletionItem>>,
}

impl Completions {
    /// A session over a document at a version.
    pub fn new(version: u64) -> Self {
        Self {
            pending: Pending::new(),
            version,
            shown: None,
        }
    }

    /// Records that the document moved to a new version.
    pub fn advance(&mut self, version: u64) {
        self.version = version;
    }

    /// Asks for completions against the current version.
    pub fn request(&mut self) -> Request {
        self.pending.request(self.version)
    }

    /// Cancels a request.
    pub fn cancel(&mut self, id: u64) -> bool {
        self.pending.cancel(id)
    }

    /// Takes a completion reply. Only a fresh one becomes the shown list.
    pub fn accept(&mut self, response: Response<Vec<CompletionItem>>) -> Delivery<usize> {
        let current = self.version;
        match self.pending.deliver(response, current) {
            Delivery::Fresh(items) => {
                let count = items.len();
                self.shown = Some(items);
                Delivery::Fresh(count)
            }
            Delivery::Stale { computed, current } => Delivery::Stale { computed, current },
            Delivery::Cancelled => Delivery::Cancelled,
        }
    }

    /// The list currently shown, when there is one.
    pub fn items(&self) -> Option<&[CompletionItem]> {
        self.shown.as_deref()
    }

    /// The session in one report.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "completions at version {}, {} outstanding",
            self.version,
            self.pending.outstanding()
        )];
        match &self.shown {
            Some(items) => {
                lines.push(format!("  shown: {} item(s)", items.len()));
                for item in items {
                    lines.push(format!("    {}", item.render()));
                }
            }
            None => lines.push("  shown: none".to_owned()),
        }
        lines
    }
}

/// A position the service returned, resolved against a document (used by hover and signatures).
pub fn position_of(text: &str, offset: usize) -> Position {
    crate::app::language::protocol::to_units(text, offset)
}

/// Reads a `textDocument/completion` result into the model's completion items.
///
/// The result is either a `CompletionList` (`{items: […]}`) or a bare array; both are accepted
/// because servers use both. `insertTextFormat: 2` means the insert text is in the snippet grammar
/// the editor already has, so the item keeps that fact instead of being shown as plain text.
pub fn completions_from_lsp(result: &crate::project::record::json::Json) -> Vec<CompletionItem> {
    use crate::project::record::json::Json;

    let items = match result {
        Json::Array(items) => items.as_slice(),
        Json::Object(object) => object.get("items").and_then(Json::as_array).unwrap_or(&[]),
        _ => return Vec::new(),
    };
    items.iter().filter_map(item_from_lsp).collect()
}

fn item_from_lsp(value: &crate::project::record::json::Json) -> Option<CompletionItem> {
    use crate::project::record::json::Json;

    let object = value.as_object()?;
    let label = object.get("label").and_then(Json::as_str)?;
    let kind = match object.get("kind").and_then(Json::as_u64) {
        Some(2) | Some(3) | Some(4) => ItemKind::Function,
        Some(5) | Some(6) | Some(10) | Some(12) | Some(20) | Some(21) => ItemKind::Variable,
        Some(14) => ItemKind::Keyword,
        Some(15) => ItemKind::Snippet,
        _ => ItemKind::Text,
    };
    let insert_text = object
        .get("textEdit")
        .and_then(Json::as_object)
        .and_then(|edit| edit.get("newText"))
        .and_then(Json::as_str)
        .or_else(|| object.get("insertText").and_then(Json::as_str))
        .unwrap_or(label);
    let snippet = object.get("insertTextFormat").and_then(Json::as_u64) == Some(2);
    let mut item = CompletionItem::new(
        label,
        insert_text,
        if snippet { ItemKind::Snippet } else { kind },
    );
    item.detail = object
        .get("detail")
        .and_then(Json::as_str)
        .map(str::to_owned);
    item.documentation = match object.get("documentation") {
        Some(Json::String(text)) => Some(text.clone()),
        Some(Json::Object(object)) => object
            .get("value")
            .and_then(Json::as_str)
            .map(str::to_owned),
        _ => None,
    };
    Some(item)
}

#[cfg(test)]
mod tests {
    use super::{
        CompletionItem, Completions, Delivery, ItemKind, Pending, Request, Response, Signature,
        SignatureHelp,
    };

    fn items(labels: &[&str]) -> Vec<CompletionItem> {
        labels
            .iter()
            .map(|label| CompletionItem::text(*label))
            .collect()
    }

    #[test]
    fn a_stale_reply_does_not_become_the_shown_list() {
        let mut completions = Completions::new(1);
        let request = completions.request();
        // The document moves on before the reply arrives.
        completions.advance(2);
        let delivery = completions.accept(Response::new(request, items(&["old"])));
        assert_eq!(
            delivery,
            Delivery::Stale {
                computed: 1,
                current: 2
            }
        );
        // Nothing stale was shown.
        assert!(completions.items().is_none());
        // A reply for the current version is shown.
        let request = completions.request();
        let delivery = completions.accept(Response::new(request, items(&["new", "next"])));
        assert_eq!(delivery, Delivery::Fresh(2));
        assert_eq!(completions.items().expect("shown").len(), 2);
    }

    #[test]
    fn a_cancelled_request_is_dropped_even_when_current() {
        let mut completions = Completions::new(5);
        let request = completions.request();
        assert!(completions.cancel(request.id));
        let delivery = completions.accept(Response::new(request, items(&["x"])));
        assert_eq!(delivery, Delivery::Cancelled);
        assert!(completions.items().is_none());
        // Cancelling an unknown request is not a cancellation.
        assert!(!completions.cancel(999));
    }

    #[test]
    fn only_a_fresh_reply_replaces_the_list() {
        let mut completions = Completions::new(1);
        let request = completions.request();
        completions.accept(Response::new(request, items(&["first"])));
        assert_eq!(completions.items().expect("shown").len(), 1);
        // A stale reply leaves the first list alone.
        completions.advance(2);
        completions.accept(Response::new(
            Request { id: 0, version: 1 },
            items(&["stale", "stale"]),
        ));
        assert_eq!(completions.items().expect("shown").len(), 1);
    }

    #[test]
    fn a_snippet_item_expands_through_the_one_snippet_grammar() {
        let item = CompletionItem::snippet("fn", "fn ${1:name}(${2}) {$0}");
        assert_eq!(item.kind, ItemKind::Snippet);
        let snippet = item
            .as_snippet()
            .expect("a snippet item")
            .expect("a valid body");
        let expansion = snippet.expand();
        assert_eq!(expansion.text(), "fn name() {}");
        // A plain item is not a snippet at all.
        assert!(CompletionItem::text("plain").as_snippet().is_none());
    }

    #[test]
    fn a_reply_for_a_version_below_the_current_one_is_stale() {
        let mut pending = Pending::new();
        let request = pending.request(3);
        assert_eq!(pending.outstanding(), 1);
        let delivery = pending.deliver(Response::new(request, "value"), 4);
        assert_eq!(
            delivery,
            Delivery::Stale {
                computed: 3,
                current: 4
            }
        );
        assert_eq!(pending.outstanding(), 0);
    }

    #[test]
    fn signature_help_names_its_active_signature() {
        let help = SignatureHelp {
            signatures: vec![
                Signature {
                    label: "f(a)".to_owned(),
                    parameters: vec!["a".to_owned()],
                    documentation: None,
                },
                Signature {
                    label: "f(a, b)".to_owned(),
                    parameters: vec!["a".to_owned(), "b".to_owned()],
                    documentation: Some("two".to_owned()),
                },
            ],
            active_signature: 1,
            active_parameter: 0,
        };
        assert_eq!(help.active().expect("active").label, "f(a, b)");
        assert!(help.render().contains("signature 2/2"));
    }

    #[test]
    fn an_item_kind_name_round_trips() {
        for kind in [
            ItemKind::Text,
            ItemKind::Function,
            ItemKind::Method,
            ItemKind::Variable,
            ItemKind::Keyword,
            ItemKind::Snippet,
        ] {
            assert_eq!(ItemKind::from_name(kind.name()), Some(kind));
        }
        assert_eq!(ItemKind::from_name("nowhere"), None);
    }

    #[test]
    fn an_lsp_completion_list_becomes_typed_items() {
        let json = crate::project::record::json::parse(
            r#"{
                "isIncomplete": false,
                "items": [
                    {"label": "alpha", "kind": 6, "detail": "u32"},
                    {"label": "println!", "kind": 3, "insertText": "println!($0)",
                     "insertTextFormat": 2, "documentation": {"kind": "markdown", "value": "print"}},
                    {"label": "bar", "kind": 5, "textEdit": {"newText": "bar()"}}
                ]
            }"#,
        )
        .expect("json");
        let items = super::completions_from_lsp(&json);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].label, "alpha");
        assert_eq!(items[0].kind, ItemKind::Variable);
        assert_eq!(items[0].detail.as_deref(), Some("u32"));
        assert_eq!(items[1].kind, ItemKind::Snippet);
        assert_eq!(items[1].insert_text, "println!($0)");
        assert_eq!(items[1].documentation.as_deref(), Some("print"));
        assert_eq!(items[2].insert_text, "bar()");
    }

    #[test]
    fn an_empty_or_wrong_completion_result_is_empty() {
        let empty = crate::project::record::json::parse("[]").expect("json");
        assert!(super::completions_from_lsp(&empty).is_empty());
        let wrong = crate::project::record::json::parse("null").expect("json");
        assert!(super::completions_from_lsp(&wrong).is_empty());
    }
}
