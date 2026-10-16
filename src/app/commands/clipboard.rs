// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The designer's clipboard: what a copy puts on it, and what a paste is allowed to do with it
//! (task T05.06).
//!
//! The acceptance line is *"illegal or oversized payloads are refused explicitly, and a reference
//! does not slip from one project into another"*. Both halves are about refusing to guess, so the
//! module is built around two explicit decisions:
//!
//! * **A payload is typed, and an unknown one is refused by name.** The designer copies three kinds
//!   of thing — plain text, a node subtree, and a resource reference — and each carries a MIME type
//!   (see [`Payload::mime`]). A paste decodes only the shapes it names; anything else is
//!   [`PasteError::UnknownFormat`] or [`PasteError::Malformed`], never "treated as text and hoped
//!   for". A payload past a size [limit](DEFAULT_LIMIT) is refused *before* it is parsed, so a huge
//!   paste cannot be turned into a huge allocation first.
//!
//! * **A paste reissues every identity, and a reference that means something else on this side is
//!   surfaced rather than carried.** Node ids are document-local, so a pasted subtree gets fresh
//!   ids from the *target* document's [`IdAllocator`] — an id from one document can never collide
//!   with an id in another. An asset property is a project-relative path, so the *same* path names a
//!   different file in a different project: a cross-project paste keeps the path but reports it in
//!   [`Placement::assets_to_reconcile`], because silently keeping a path that now points elsewhere
//!   is exactly the drift this task forbids. A lone resource pasted into another project is refused
//!   outright ([`PasteError::ForeignResource`]): there is nothing to reconcile it *to*, and the
//!   honest fix is an import (F12), not a paste.
//!
//! **Transport.** The pinned dependency's clipboard carries text, HTML, RTF, images and file lists,
//! and its rich backend names those by MIME; it has no entry point for an *application-defined* MIME
//! type. The node and resource payloads are therefore carried as a small JSON envelope in the
//! clipboard's text, tagged with this crate's vendor name so a paste can recognise its own payload
//! and a stray JSON document is still plain text. [`decode`] sniffs that envelope; [`decode_as`]
//! uses an explicit MIME when a richer clipboard supplies one. This gap is recorded upstream.

// The payloads are put on and taken off the OS clipboard by the copy/paste commands (T05.01/T05.06's
// menu entries) as those land; until then the tests and the `--clipboard` script exercise them, and
// the silence is stated once here with its reason, as the neighbouring trees do.
#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use crate::contracts::identity::{IdAllocator, NodeId};
use crate::contracts::model::PropertyValue;
use crate::host::clipboard::{ClipboardHost, ClipboardRead, ClipboardWrite};
use crate::project::record::json::{Json, JsonObject, parse};
use crate::project::record::page::{PageNode, node_from_json, node_to_json};

/// The vendor tag inside an envelope, so a paste recognises its own payload and nothing else.
const VENDOR: &str = "rustage";

/// The envelope format this build writes and the only one it reads.
const FORMAT: u64 = 1;

/// The MIME type of a plain-text payload.
pub const TEXT_MIME: &str = "text/plain";

/// The MIME type of a node-subtree payload.
pub const NODES_MIME: &str = "application/vnd.rustage.nodes+json";

/// The MIME type of a resource-reference payload.
pub const RESOURCE_MIME: &str = "application/vnd.rustage.resource+json";

/// The default ceiling on a clipboard payload, in bytes.
///
/// A paste is a single user action, so a payload larger than a megabyte is far more likely to be a
/// mistake or an attack than a copy somebody meant to make; refusing it is the safe default.
pub const DEFAULT_LIMIT: usize = 1 << 20;

/// A subtree of nodes copied together: a root and everything under it.
#[derive(Clone, PartialEq, Debug)]
pub struct CopySubtree {
    project: String,
    root: NodeId,
    nodes: Vec<PageNode>,
}

impl CopySubtree {
    /// A copied subtree. The project it was copied from is named so a paste can tell whether the
    /// asset references it carries still mean the same thing on the other side.
    pub fn new(project: impl Into<String>, root: NodeId, nodes: Vec<PageNode>) -> Self {
        Self {
            project: project.into(),
            root,
            nodes,
        }
    }

    /// The project the subtree was copied from.
    pub fn project(&self) -> &str {
        &self.project
    }

    /// The subtree's root, as it was in the document it came from.
    pub const fn root(&self) -> NodeId {
        self.root
    }

    /// Every node of the subtree, in the order it was copied.
    pub fn nodes(&self) -> &[PageNode] {
        &self.nodes
    }
}

/// What the designer put on the clipboard.
#[derive(Clone, PartialEq, Debug)]
pub enum Payload {
    /// Plain text.
    Text(String),
    /// A subtree of nodes, with the project it came from.
    Nodes(CopySubtree),
    /// A reference to a project resource, with the project it came from.
    Resource { project: String, path: String },
}

impl Payload {
    /// The MIME type this payload is offered as.
    pub const fn mime(&self) -> &'static str {
        match self {
            Self::Text(_) => TEXT_MIME,
            Self::Nodes(_) => NODES_MIME,
            Self::Resource { .. } => RESOURCE_MIME,
        }
    }

    /// Encodes the payload as the text to put on the clipboard, refusing one over the limit.
    pub fn encode(&self, limit: usize) -> Result<String, PasteError> {
        let text = match self {
            Self::Text(text) => text.clone(),
            Self::Nodes(subtree) => encode_subtree(subtree)?,
            Self::Resource { project, path } => encode_resource(project, path),
        };
        if text.len() > limit {
            return Err(PasteError::TooLarge {
                bytes: text.len(),
                limit,
            });
        }
        Ok(text)
    }

    /// Pastes the payload into a project, reissuing identities and surfacing references that would
    /// no longer mean the same thing.
    pub fn paste(
        &self,
        target_project: &str,
        allocator: &mut IdAllocator,
    ) -> Result<Pasted, PasteError> {
        match self {
            Self::Text(text) => Ok(Pasted::Text(text.clone())),
            Self::Nodes(subtree) => paste_subtree(subtree, target_project, allocator),
            Self::Resource { project, path } => {
                if project == target_project {
                    return Ok(Pasted::Resource { path: path.clone() });
                }
                Err(PasteError::ForeignResource {
                    path: path.clone(),
                    source: project.clone(),
                    target: target_project.to_owned(),
                })
            }
        }
    }

    /// Puts the payload on the clipboard, and answers which MIME type it is offered as.
    pub fn put(
        host: &dyn ClipboardHost,
        payload: &Payload,
        limit: usize,
    ) -> Result<&'static str, PasteError> {
        let text = payload.encode(limit)?;
        match host.write_text(&text) {
            ClipboardWrite::Written => Ok(payload.mime()),
            ClipboardWrite::Refused { reason } => Err(PasteError::Refused { reason }),
        }
    }

    /// Takes a payload off the clipboard.
    ///
    /// Text is sniffed, because the node and resource payloads travel in a text envelope: a
    /// clipboard that reports `text/plain` may be holding either. Anything that is not text — an
    /// image somebody copied elsewhere, say — is refused by its own MIME type rather than flattened.
    pub fn take(host: &dyn ClipboardHost, limit: usize) -> Result<Self, PasteError> {
        match host.read() {
            ClipboardRead::Text(text) => decode(&text, limit),
            ClipboardRead::NotText { content_type } => {
                Err(PasteError::UnknownFormat { mime: content_type })
            }
            ClipboardRead::Empty => Err(PasteError::Empty),
            ClipboardRead::Unavailable { reason } => Err(PasteError::Unavailable { reason }),
        }
    }
}

/// What a paste produced.
#[derive(Clone, PartialEq, Debug)]
pub enum Pasted {
    /// The text, to be inserted at the cursor.
    Text(String),
    /// A subtree with fresh identities, to be attached at a place the caller chooses.
    Nodes(Placement),
    /// A resource reference that still means the same thing in this project.
    Resource { path: String },
}

/// A pasted subtree, rebuilt with identities from the target document.
#[derive(Clone, PartialEq, Debug)]
pub struct Placement {
    root: NodeId,
    nodes: Vec<PageNode>,
    ids: BTreeMap<NodeId, NodeId>,
    assets_to_reconcile: Vec<String>,
}

impl Placement {
    /// The new identity of the subtree's root.
    pub const fn root(&self) -> NodeId {
        self.root
    }

    /// The rebuilt nodes, in the order they were copied.
    pub fn nodes(&self) -> &[PageNode] {
        &self.nodes
    }

    /// The new identity of a node that was copied, if it was part of this subtree.
    pub fn mapped(&self, old: NodeId) -> Option<NodeId> {
        self.ids.get(&old).copied()
    }

    /// Every old-to-new identity, for a caller that has references to rewrite.
    pub fn ids(&self) -> &BTreeMap<NodeId, NodeId> {
        &self.ids
    }

    /// Asset paths this subtree refers to that named a different project where they were copied.
    ///
    /// They were kept as written, because dropping them would lose the reference; the caller shows
    /// them so a person can re-link them. Empty when the subtree came from this project.
    pub fn assets_to_reconcile(&self) -> &[String] {
        &self.assets_to_reconcile
    }
}

/// Why a payload was refused.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PasteError {
    /// The clipboard held nothing.
    Empty,
    /// The payload is larger than the limit. Refused before it is parsed.
    TooLarge { bytes: usize, limit: usize },
    /// The clipboard holds a kind this build does not read.
    UnknownFormat { mime: String },
    /// The payload claims to be one of ours but is not shaped the way it says.
    Malformed { reason: String },
    /// The platform refused the write.
    Refused { reason: String },
    /// The clipboard could not be asked.
    Unavailable { reason: String },
    /// A resource was copied from another project, where the same path means another file.
    ForeignResource {
        path: String,
        source: String,
        target: String,
    },
    /// A node refers to an id that is not part of the payload.
    DanglingReference { id: NodeId },
    /// The payload is not a subtree: its root is also a child of one of its own nodes.
    NotASubtree { id: NodeId },
}

impl PasteError {
    /// The one-line explanation a person is shown.
    pub fn message(&self) -> String {
        match self {
            Self::Empty => "the clipboard is empty".to_owned(),
            Self::TooLarge { bytes, limit } => {
                format!("the payload is {bytes} byte(s), past the {limit}-byte limit")
            }
            Self::UnknownFormat { mime } => {
                format!("the clipboard holds {mime}, which is not a payload this designer reads")
            }
            Self::Malformed { reason } => format!("the clipboard payload is malformed: {reason}"),
            Self::Refused { reason } => format!("the clipboard refused the write: {reason}"),
            Self::Unavailable { reason } => format!("the clipboard could not be read: {reason}"),
            Self::ForeignResource {
                path,
                source,
                target,
            } => format!(
                "{path} was copied from project {source} and means a different file in {target}; \
                 import it instead of pasting it"
            ),
            Self::DanglingReference { id } => {
                format!(
                    "the payload refers to node {}, which it does not contain",
                    id.get()
                )
            }
            Self::NotASubtree { id } => {
                format!(
                    "the payload is not a subtree: node {} is its own root's child",
                    id.get()
                )
            }
        }
    }
}

/// Decodes clipboard text, recognising the vendor envelope and treating everything else as text.
pub fn decode(text: &str, limit: usize) -> Result<Payload, PasteError> {
    if text.len() > limit {
        return Err(PasteError::TooLarge {
            bytes: text.len(),
            limit,
        });
    }
    match parse(text) {
        Ok(Json::Object(object)) if claims_to_be_ours(&object) => decode_envelope(&object),
        // Not our envelope: plain text, kept exactly as it arrived.
        _ => Ok(Payload::Text(text.to_owned())),
    }
}

/// Decodes clipboard text that arrived with an explicit MIME type.
pub fn decode_as(mime: &str, text: &str, limit: usize) -> Result<Payload, PasteError> {
    match mime {
        TEXT_MIME => {
            if text.len() > limit {
                return Err(PasteError::TooLarge {
                    bytes: text.len(),
                    limit,
                });
            }
            Ok(Payload::Text(text.to_owned()))
        }
        NODES_MIME | RESOURCE_MIME => {
            if text.len() > limit {
                return Err(PasteError::TooLarge {
                    bytes: text.len(),
                    limit,
                });
            }
            let value = parse(text).map_err(|error| PasteError::Malformed {
                reason: error.to_string(),
            })?;
            let Json::Object(object) = value else {
                return Err(PasteError::Malformed {
                    reason: format!("a {mime} payload is a JSON object"),
                });
            };
            if mime == NODES_MIME && !claims_to_be_ours(&object) {
                return Err(PasteError::Malformed {
                    reason: "a nodes payload names the rustage vendor".to_owned(),
                });
            }
            decode_envelope(&object)
        }
        other => Err(PasteError::UnknownFormat {
            mime: other.to_owned(),
        }),
    }
}

/// Whether an object claims to be one of our envelopes.
fn claims_to_be_ours(object: &JsonObject) -> bool {
    object.get("vendor").and_then(Json::as_str) == Some(VENDOR)
}

/// Reads an envelope that claims to be ours, strictly.
fn decode_envelope(object: &JsonObject) -> Result<Payload, PasteError> {
    let format =
        object
            .get("format")
            .and_then(Json::as_u64)
            .ok_or_else(|| PasteError::Malformed {
                reason: "a rustage payload names a numeric format".to_owned(),
            })?;
    if format != FORMAT {
        return Err(PasteError::Malformed {
            reason: format!("format {format} is not the {FORMAT} this build writes"),
        });
    }
    let kind = object
        .get("kind")
        .and_then(Json::as_str)
        .ok_or_else(|| PasteError::Malformed {
            reason: "a rustage payload names its kind".to_owned(),
        })?;
    match kind {
        "nodes" => Ok(Payload::Nodes(decode_subtree(object)?)),
        "resource" => {
            let project = required_string(object, "project")?;
            let path = required_string(object, "path")?;
            Ok(Payload::Resource { project, path })
        }
        other => Err(PasteError::Malformed {
            reason: format!("{other:?} is not a rustage payload kind"),
        }),
    }
}

fn decode_subtree(object: &JsonObject) -> Result<CopySubtree, PasteError> {
    let project = required_string(object, "project")?;
    let root = object
        .get("root")
        .and_then(Json::as_u64)
        .map(NodeId::from_raw)
        .ok_or_else(|| PasteError::Malformed {
            reason: "a nodes payload names its root".to_owned(),
        })?;
    let Some(entries) = object.get("nodes").and_then(Json::as_array) else {
        return Err(PasteError::Malformed {
            reason: "a nodes payload carries an array of nodes".to_owned(),
        });
    };
    let mut nodes = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let node = node_from_json(&format!("nodes[{index}]"), entry).map_err(|error| {
            PasteError::Malformed {
                reason: error.to_string(),
            }
        })?;
        nodes.push(node);
    }
    Ok(CopySubtree::new(project, root, nodes))
}

fn required_string(object: &JsonObject, name: &str) -> Result<String, PasteError> {
    object
        .get(name)
        .and_then(Json::as_str)
        .map(str::to_owned)
        .ok_or_else(|| PasteError::Malformed {
            reason: format!("a rustage payload names its {name} as a string"),
        })
}

fn encode_subtree(subtree: &CopySubtree) -> Result<String, PasteError> {
    let mut out = JsonObject::new();
    out.insert("vendor", Json::string(VENDOR));
    out.insert("format", Json::number(FORMAT));
    out.insert("kind", Json::string("nodes"));
    out.insert("project", Json::string(&subtree.project));
    out.insert("root", Json::number(subtree.root.get()));
    let mut nodes = Vec::with_capacity(subtree.nodes.len());
    for node in &subtree.nodes {
        let encoded = node_to_json(node).map_err(|error| PasteError::Malformed {
            reason: error.to_string(),
        })?;
        nodes.push(encoded);
    }
    out.insert("nodes", Json::Array(nodes));
    Ok(Json::Object(out).render())
}

fn encode_resource(project: &str, path: &str) -> String {
    let mut out = JsonObject::new();
    out.insert("vendor", Json::string(VENDOR));
    out.insert("format", Json::number(FORMAT));
    out.insert("kind", Json::string("resource"));
    out.insert("project", Json::string(project));
    out.insert("path", Json::string(path));
    Json::Object(out).render()
}

/// Rebuilds a copied subtree under the target document's identities.
fn paste_subtree(
    subtree: &CopySubtree,
    target_project: &str,
    allocator: &mut IdAllocator,
) -> Result<Pasted, PasteError> {
    let known: BTreeSet<NodeId> = subtree.nodes.iter().map(PageNode::id).collect();
    if !known.contains(&subtree.root) {
        return Err(PasteError::DanglingReference { id: subtree.root });
    }
    for node in &subtree.nodes {
        for child in node.children() {
            if !known.contains(child) {
                return Err(PasteError::DanglingReference { id: *child });
            }
        }
        if node.children().contains(&subtree.root) {
            return Err(PasteError::NotASubtree { id: subtree.root });
        }
    }

    // Every identity is reissued from the target document's allocator, in the order copied, so an
    // id from one document cannot collide with an id in another.
    let mut ids = BTreeMap::new();
    for node in &subtree.nodes {
        ids.insert(node.id(), allocator.allocate());
    }
    let mut nodes = Vec::with_capacity(subtree.nodes.len());
    for node in &subtree.nodes {
        let new_id = ids[&node.id()];
        let children: Vec<NodeId> = node.children().iter().map(|child| ids[child]).collect();
        nodes.push(node.clone().with_id(new_id).with_children(children));
    }

    // Asset properties are project-relative paths; across projects the same path names another file.
    let mut assets = Vec::new();
    if subtree.project != target_project {
        for node in &subtree.nodes {
            for value in node.properties().values() {
                if let PropertyValue::Asset(path) = value {
                    assets.push(path.clone());
                }
            }
        }
        assets.sort();
        assets.dedup();
    }

    Ok(Pasted::Nodes(Placement {
        root: ids[&subtree.root],
        nodes,
        ids,
        assets_to_reconcile: assets,
    }))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::{
        CopySubtree, DEFAULT_LIMIT, NODES_MIME, PasteError, Pasted, Payload, RESOURCE_MIME,
        TEXT_MIME, decode, decode_as,
    };
    use crate::contracts::identity::{IdAllocator, NodeId};
    use crate::contracts::model::{NodeKind, PropertyName, PropertyValue};
    use crate::host::clipboard::{ClipboardHost, ClipboardRead, ClipboardWrite};
    use crate::project::record::page::PageNode;

    fn subtree(project: &str) -> CopySubtree {
        let root = NodeId::from_raw(10);
        let child = NodeId::from_raw(11);
        let mut allocator = IdAllocator::new();
        allocator.adopt(root);
        allocator.adopt(child);

        let root_node = PageNode::new(root, NodeKind::new("column").unwrap())
            .with_property(PropertyName::new("gap").unwrap(), PropertyValue::Integer(8))
            .with_children([child]);
        let child_node = PageNode::new(child, NodeKind::new("label").unwrap()).with_property(
            PropertyName::new("text").unwrap(),
            PropertyValue::Text("Save".to_owned()),
        );
        CopySubtree::new(project, root, vec![root_node, child_node])
    }

    /// An in-memory clipboard, so paste logic is tested without the user's real one.
    struct FakeClipboard {
        content: RefCell<Option<String>>,
        kind: RefCell<Option<String>>,
        break_reads: bool,
    }

    impl FakeClipboard {
        fn empty() -> Self {
            Self {
                content: RefCell::new(None),
                kind: RefCell::new(None),
                break_reads: false,
            }
        }
    }

    impl ClipboardHost for FakeClipboard {
        fn write_text(&self, text: &str) -> ClipboardWrite {
            if text.is_empty() {
                return ClipboardWrite::Refused {
                    reason: "the session clipboard is owned by another process".to_owned(),
                };
            }
            *self.content.borrow_mut() = Some(text.to_owned());
            *self.kind.borrow_mut() = Some(TEXT_MIME.to_owned());
            ClipboardWrite::Written
        }

        fn read(&self) -> ClipboardRead {
            if self.break_reads {
                return ClipboardRead::Unavailable {
                    reason: "the display server refused the selection request".to_owned(),
                };
            }
            match self.content.borrow().clone() {
                Some(text) => ClipboardRead::Text(text),
                None => ClipboardRead::Empty,
            }
        }
    }

    #[test]
    fn text_round_trips_and_plain_json_that_is_not_ours_stays_text() {
        let payload = Payload::Text("hello · 你好".to_owned());
        let text = payload.encode(DEFAULT_LIMIT).unwrap();
        assert_eq!(decode(&text, DEFAULT_LIMIT).unwrap(), payload);

        // A JSON document that does not name our vendor is plain text, not a payload we guess at.
        let stray = "{\"format\": 1, \"kind\": \"nodes\"}";
        assert_eq!(
            decode(stray, DEFAULT_LIMIT).unwrap(),
            Payload::Text(stray.to_owned())
        );
    }

    #[test]
    fn a_node_subtree_round_trips_through_its_envelope() {
        let payload = Payload::Nodes(subtree("Demo"));
        assert_eq!(payload.mime(), NODES_MIME);
        let text = payload.encode(DEFAULT_LIMIT).unwrap();
        let decoded = decode(&text, DEFAULT_LIMIT).unwrap();
        assert_eq!(decoded, payload);
    }

    #[test]
    fn a_paste_reissues_every_identity_and_remaps_the_children() {
        let payload = Payload::Nodes(subtree("Demo"));
        let mut allocator = IdAllocator::new();
        let Pasted::Nodes(placement) = payload.paste("Demo", &mut allocator).unwrap() else {
            panic!("a subtree pastes to a placement");
        };

        // The ids are new, distinct from the originals, and consistent across the subtree.
        let old_root = NodeId::from_raw(10);
        let old_child = NodeId::from_raw(11);
        let new_root = placement.root();
        let new_child = placement.mapped(old_child).unwrap();
        assert_ne!(new_root, old_root);
        assert_ne!(new_child, old_child);
        assert_eq!(placement.mapped(old_root), Some(new_root));

        // The root's child now names the new child id, not the old one.
        let rebuilt_root = &placement.nodes()[0];
        assert_eq!(rebuilt_root.id(), new_root);
        assert_eq!(rebuilt_root.children(), &[new_child]);
        // Properties came across untouched.
        assert_eq!(
            rebuilt_root
                .properties()
                .get(&PropertyName::new("gap").unwrap()),
            Some(&PropertyValue::Integer(8))
        );
    }

    #[test]
    fn a_cross_project_paste_reports_the_asset_references_to_reconcile() {
        let root = NodeId::from_raw(1);
        let node = PageNode::new(root, NodeKind::new("image").unwrap()).with_property(
            PropertyName::new("source").unwrap(),
            PropertyValue::Asset("assets/logo.svg".to_owned()),
        );
        let payload = Payload::Nodes(CopySubtree::new("Alpha", root, vec![node]));

        let mut allocator = IdAllocator::new();
        let Pasted::Nodes(same) = payload.paste("Alpha", &mut allocator).unwrap() else {
            panic!("pastes to a placement");
        };
        assert!(
            same.assets_to_reconcile().is_empty(),
            "the same project needs no reconciliation"
        );

        let mut allocator = IdAllocator::new();
        let Pasted::Nodes(other) = payload.paste("Beta", &mut allocator).unwrap() else {
            panic!("pastes to a placement");
        };
        assert_eq!(other.assets_to_reconcile(), ["assets/logo.svg"]);
    }

    #[test]
    fn a_foreign_resource_is_refused_and_a_local_one_is_not() {
        let payload = Payload::Resource {
            project: "Alpha".to_owned(),
            path: "assets/logo.svg".to_owned(),
        };
        assert_eq!(
            payload.paste("Alpha", &mut IdAllocator::new()).unwrap(),
            Pasted::Resource {
                path: "assets/logo.svg".to_owned()
            }
        );

        let error = payload.paste("Beta", &mut IdAllocator::new()).unwrap_err();
        assert_eq!(
            error,
            PasteError::ForeignResource {
                path: "assets/logo.svg".to_owned(),
                source: "Alpha".to_owned(),
                target: "Beta".to_owned(),
            }
        );
        assert!(error.message().contains("import it instead"));
    }

    #[test]
    fn an_oversized_payload_is_refused_before_it_is_parsed() {
        let big = "x".repeat(DEFAULT_LIMIT + 1);
        match decode(&big, DEFAULT_LIMIT) {
            Err(PasteError::TooLarge { bytes, limit }) => {
                assert_eq!(bytes, DEFAULT_LIMIT + 1);
                assert_eq!(limit, DEFAULT_LIMIT);
            }
            other => panic!("expected TooLarge, got {other:?}"),
        }

        // And on the way out: a payload that would exceed the limit is not written at all.
        let payload = Payload::Text(big);
        assert!(matches!(
            payload.encode(DEFAULT_LIMIT),
            Err(PasteError::TooLarge { .. })
        ));
    }

    #[test]
    fn a_malformed_or_unknown_payload_is_refused_by_name() {
        // Claims to be ours, but the format is not one this build writes.
        let text = "{\"vendor\":\"rustage\",\"format\":2,\"kind\":\"nodes\"}";
        assert!(matches!(
            decode(text, DEFAULT_LIMIT),
            Err(PasteError::Malformed { .. })
        ));

        // Claims to be ours, but names a kind we do not have.
        let text = "{\"vendor\":\"rustage\",\"format\":1,\"kind\":\"gadgets\"}";
        assert!(matches!(
            decode(text, DEFAULT_LIMIT),
            Err(PasteError::Malformed { .. })
        ));

        // An explicit MIME we do not read is refused by that MIME.
        match decode_as("image/png", "…", DEFAULT_LIMIT) {
            Err(PasteError::UnknownFormat { mime }) => assert_eq!(mime, "image/png"),
            other => panic!("expected UnknownFormat, got {other:?}"),
        }
        // A resource payload that is not our envelope is malformed, not silently accepted.
        assert!(matches!(
            decode_as(RESOURCE_MIME, "{}", DEFAULT_LIMIT),
            Err(PasteError::Malformed { .. })
        ));
    }

    #[test]
    fn a_payload_that_refers_to_a_node_it_does_not_contain_is_refused() {
        let root = NodeId::from_raw(1);
        let missing = NodeId::from_raw(99);
        let node = PageNode::new(root, NodeKind::new("column").unwrap()).with_children([missing]);
        let payload = Payload::Nodes(CopySubtree::new("Alpha", root, vec![node]));

        match payload.paste("Alpha", &mut IdAllocator::new()) {
            Err(PasteError::DanglingReference { id }) => assert_eq!(id, missing),
            other => panic!("expected DanglingReference, got {other:?}"),
        }
    }

    #[test]
    fn a_root_that_is_its_own_child_is_not_a_subtree() {
        let root = NodeId::from_raw(1);
        let node = PageNode::new(root, NodeKind::new("column").unwrap()).with_children([root]);
        let payload = Payload::Nodes(CopySubtree::new("Alpha", root, vec![node]));

        match payload.paste("Alpha", &mut IdAllocator::new()) {
            Err(PasteError::NotASubtree { id }) => assert_eq!(id, root),
            other => panic!("expected NotASubtree, got {other:?}"),
        }
    }

    #[test]
    fn putting_and_taking_a_payload_goes_through_the_clipboard() {
        let host = FakeClipboard::empty();
        let payload = Payload::Nodes(subtree("Demo"));

        let mime = Payload::put(&host, &payload, DEFAULT_LIMIT).unwrap();
        assert_eq!(mime, NODES_MIME);
        assert_eq!(Payload::take(&host, DEFAULT_LIMIT).unwrap(), payload);

        // An empty clipboard is a fact, and an unreadable one is not.
        let empty = FakeClipboard::empty();
        assert_eq!(
            Payload::take(&empty, DEFAULT_LIMIT).unwrap_err(),
            PasteError::Empty
        );
        let broken = FakeClipboard {
            break_reads: true,
            ..FakeClipboard::empty()
        };
        assert!(matches!(
            Payload::take(&broken, DEFAULT_LIMIT),
            Err(PasteError::Unavailable { .. })
        ));
    }

    #[test]
    fn a_refused_write_is_not_a_success() {
        let host = FakeClipboard::empty();
        // The fake refuses an empty write, standing in for a platform that will not take it.
        let outcome = Payload::put(&host, &Payload::Text(String::new()), DEFAULT_LIMIT);
        match outcome {
            Err(PasteError::Refused { reason }) => assert!(reason.contains("another process")),
            other => panic!("expected Refused, got {other:?}"),
        }
    }

    #[test]
    fn every_mime_type_is_the_one_the_payload_answers_to() {
        assert_eq!(Payload::Text(String::new()).mime(), TEXT_MIME);
        assert_eq!(Payload::Nodes(subtree("Demo")).mime(), NODES_MIME);
        assert_eq!(
            Payload::Resource {
                project: "Demo".to_owned(),
                path: "assets/logo.svg".to_owned()
            }
            .mime(),
            RESOURCE_MIME
        );
    }
}
