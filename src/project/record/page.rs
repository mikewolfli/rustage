// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! A page's own document, format version 1: contract **C-PROJECT**.
//!
//! The project document names one page document per page (decision `F1`); this is what such a
//! document holds. A version 1 page document is a JSON object:
//!
//! | member | kind | meaning |
//! |---|---|---|
//! | `format` | whole number | the format version; required, and this build writes `1` |
//! | `page` | object | `id`, `name` and `root`, all required |
//! | `nodes` | array | one entry per node: `id` and `type` (both required), `children`, `properties` |
//!
//! A node is:
//!
//! ```json
//! { "id": 2, "type": "button", "children": [3], "properties": [ { "name": "label", "value": { "text": "Save" } } ] }
//! ```
//!
//! `children` and `properties` may be left out, which means "none" — the one place a version 1
//! document says "empty" without a member, documented here and tested, because decision `F1`
//! forbids a *newer* build's missing member being read as a default.
//!
//! ## The file holds one tree, and the model builds the links
//!
//! A node does **not** write its parent: the tree is written once, as each node's children, and the
//! parent links are built by the store when the file becomes a document. Two places holding the same
//! tree is how a file comes to disagree with itself, and the model's own writers are crate-internal
//! on purpose — the registered contract says only the store's apply path calls them, so a file reader
//! must not try.
//!
//! **What this module deliberately does not do** is turn a whole file into an editable model. The
//! store accepts a node only if its identity was issued to the document, so the file → model path
//! needs a seam that adopts a file's identities; that seam is
//! [`Buffer::store`](crate::project::document::buffer::Buffer::store), which hands the page and its root to
//! a [`DocumentStore`](crate::contracts::store::DocumentStore) with the identities the text declared.
//! The remaining nodes arrive through change sets, which is where a node is *changed* — and that is
//! T03.02's subject, because changing a node means changing it through an inverse-carrying change
//! set rather than by re-reading a file.
//!
//! ## Unknown members
//!
//! Like the project document, a member this build does not know is **preserved and reported**
//! wherever it appears, and written back where it was, so an older build cannot damage a newer
//! file's content.
//!
//! ## What a file must refuse
//!
//! The reader refuses what makes a file not a file: a version it does not know, a member of the
//! wrong shape, a duplicate node identity, a child or root that no node declares, a node that is
//! not reachable from the root exactly once, and a property value that is not one of the model's
//! value families. Whether the tree is *legal for the model* — the kinds that exist, the parents
//! that may hold a child — is the store's decision, and a file that the store would refuse is
//! refused when it is applied, not guessed at here.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::contracts::identity::{NodeId, PageId};
use crate::contracts::model::{NodeKind, Page, PropertyName, PropertyValue};
use crate::contracts::revision::FormatVersion;
use crate::host::paths::ProjectRoot;
use crate::project::record::format::{
    Preserved, ProjectError, Reader, Reference, ReferenceKind, UnknownField, sub_object,
    sub_objects,
};
use crate::project::record::json::{self, Json, JsonKind, JsonObject, Number};

/// The page document format this build writes.
///
/// Its own sequence: a page document and a project document are different formats, and a member
/// added to one of them says nothing about the other.
pub const PAGE_FORMAT: FormatVersion = FormatVersion::from_raw(1);

/// The member names a page document version 1 defines, in the order the writer writes them.
pub const PAGE_DOCUMENT_MEMBERS: [&str; 3] = ["format", "page", "nodes"];

/// The member names a node may have, in the order the writer writes them.
pub const PAGE_NODE_MEMBERS: [&str; 4] = ["id", "type", "children", "properties"];

/// One node of a page document, as the file holds it.
///
/// Deliberately not [`crate::contracts::model::Node`]: that type's parent and children links are
/// written only by the store's apply path, which is what keeps "one writable model" true. A file
/// carries the tree once and the store builds the links
/// ([`PageDocument::to_change_set`]); the vocabulary a node uses — kinds, property names, property
/// values — *is* the model's, so the two cannot describe different things.
#[derive(Clone, PartialEq, Debug)]
pub struct PageNode {
    id: NodeId,
    kind: NodeKind,
    properties: BTreeMap<PropertyName, PropertyValue>,
    children: Vec<NodeId>,
    preserved: Preserved,
}

impl PageNode {
    /// A node with no children and no properties.
    pub fn new(id: NodeId, kind: NodeKind) -> Self {
        Self {
            id,
            kind,
            properties: BTreeMap::new(),
            children: Vec::new(),
            preserved: Preserved::default(),
        }
    }

    /// A node with one more property set.
    pub fn with_property(mut self, name: PropertyName, value: PropertyValue) -> Self {
        self.properties.insert(name, value);
        self
    }

    /// A node with the given children, in order.
    pub fn with_children(mut self, children: impl IntoIterator<Item = NodeId>) -> Self {
        self.children = children.into_iter().collect();
        self
    }

    /// The same node under a different identity.
    ///
    /// A cross-project paste (T05.06) must not let an id copied out of one document collide with an
    /// id already in another, so it reissues every id. Reissuing is the *only* change here, so the
    /// kind, the properties, the children and any member this build does not know move across
    /// untouched — nothing is rebuilt from parts that could drop something.
    pub fn with_id(self, id: NodeId) -> Self {
        Self { id, ..self }
    }

    /// The node's identity.
    pub const fn id(&self) -> NodeId {
        self.id
    }

    /// The control kind.
    pub fn kind(&self) -> &NodeKind {
        &self.kind
    }

    /// The node's properties, by name.
    pub fn properties(&self) -> &BTreeMap<PropertyName, PropertyValue> {
        &self.properties
    }

    /// The node's children, in layout order.
    pub fn children(&self) -> &[NodeId] {
        &self.children
    }

    /// What interpretation left over.
    pub fn preserved(&self) -> &Preserved {
        &self.preserved
    }
}

/// A page's document: the page, its tree of nodes, and what resolved against the project root.
#[derive(Clone, Debug)]
pub struct PageDocument {
    path: PathBuf,
    /// The root its asset references resolve against, when it was read from a project.
    root: Option<ProjectRoot>,
    page: Page,
    page_preserved: Preserved,
    nodes: Vec<PageNode>,
    references: Vec<Reference>,
    preserved: Preserved,
    unknown: Vec<UnknownField>,
}

// The root is where the references were resolved, not part of what the document says: two reads
// of the same bytes describe the same document even when they were resolved against different
// roots, and the resolved paths in `references` already show any difference that matters.
impl PartialEq for PageDocument {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path
            && self.page == other.page
            && self.page_preserved == other.page_preserved
            && self.nodes == other.nodes
            && self.references == other.references
            && self.preserved == other.preserved
            && self.unknown == other.unknown
    }
}

impl PageDocument {
    /// The format this document is written as, which is the version this build writes.
    pub const fn format(&self) -> FormatVersion {
        PAGE_FORMAT
    }

    /// A page with one root node and nothing else: what an empty page is.
    ///
    /// `None` when there is nothing to name the page with — a blank name is not a name (see
    /// [`Page::new`]), and the caller has to fix that rather than this build inventing one.
    pub fn empty_page(
        page: PageId,
        root: NodeId,
        name: &str,
        root_kind: NodeKind,
        path: impl Into<PathBuf>,
    ) -> Option<Self> {
        let page = Page::new(page, name, root)?;
        Some(Self {
            path: path.into(),
            root: None,
            page,
            page_preserved: Preserved::default(),
            nodes: vec![PageNode::new(root, root_kind)],
            references: Vec::new(),
            preserved: Preserved::default(),
            unknown: Vec::new(),
        })
    }

    /// Reads a page document from text that was already obtained, for tests and for tools.
    ///
    /// The root is needed because a property may hold a project-relative path
    /// ([`PropertyValue::Asset`]), and those resolve against the project root like every other path
    /// in a project.
    pub fn parse(
        root: &ProjectRoot,
        path: impl Into<PathBuf>,
        text: &str,
    ) -> Result<Self, ProjectError> {
        let path = path.into();
        let value = json::parse(text).map_err(|error| ProjectError::NotJson {
            path: path.clone(),
            error,
        })?;
        interpret(root, path, &value)
    }

    /// Reads a page document from a path inside a project root.
    pub fn read(root: &ProjectRoot, relative: impl AsRef<Path>) -> Result<Self, ProjectError> {
        let relative = relative.as_ref();
        let path = root.resolve(relative).map_err(|error| ProjectError::Io {
            path: root.as_path().join(relative),
            action: "resolving the page document",
            reason: error.message(),
        })?;
        let text = std::fs::read_to_string(&path).map_err(|error| ProjectError::Io {
            path: path.clone(),
            action: "reading",
            reason: error.to_string(),
        })?;
        Self::parse(root, path, &text)
    }

    /// The file this document is (or would be) written to.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The page.
    pub fn page(&self) -> &Page {
        &self.page
    }

    /// The nodes, in identity order.
    pub fn nodes(&self) -> &[PageNode] {
        &self.nodes
    }

    /// One node.
    pub fn node(&self, id: NodeId) -> Option<&PageNode> {
        self.nodes.iter().find(|node| node.id == id)
    }

    /// The page's root node.
    pub fn root_node(&self) -> Option<&PageNode> {
        self.node(self.page.root())
    }

    /// Every project-relative path a property holds, with what resolving it found.
    pub fn references(&self) -> &[Reference] {
        &self.references
    }

    /// Rewrites every asset property that names `from` so that it names `to`.
    ///
    /// This is the page-document half of moving or renaming a resource: only values of the
    /// [`PropertyValue::Asset`] family are touched, every other property, child and member this
    /// build does not know is left exactly as it was, and the number of references changed is
    /// returned so a caller can report it rather than claim a rewrite it did not make.
    ///
    /// A reference that names another path is not rewritten, and `from == to` is a no-op that
    /// returns `0`. The document is written back through the same writer a read document
    /// round-trips with, so the result can be checked by reading it again.
    pub fn rewire_asset(&mut self, from: &str, to: &str) -> Result<usize, String> {
        if from.is_empty() || to.is_empty() {
            return Err(
                "an asset reference needs both the path it names and the path to write".to_owned(),
            );
        }
        if from == to {
            return Ok(0);
        }
        if let Some(root) = &self.root {
            // The target may live in a directory that does not exist yet, so this is the same rule
            // a file about to be created follows: stay inside the root, and do not require the
            // path's own directory to exist.
            root.resolve_for_creation(to).map_err(|error| {
                format!(
                    "`{to}` cannot be a page asset reference: {}",
                    error.message()
                )
            })?;
        }

        let mut rewritten = 0usize;
        for node in &mut self.nodes {
            for value in node.properties.values_mut() {
                if let PropertyValue::Asset(written) = value
                    && written == from
                {
                    *written = to.to_owned();
                    rewritten += 1;
                }
            }
        }
        if rewritten == 0 {
            return Ok(0);
        }

        // The reference list is a diagnostic of what the document names. Rebuilding it here keeps
        // a caller that asks "what does this page point at?" from being told about the old path.
        if let Some(root) = &self.root {
            let context = Context { root };
            let mut references = Vec::new();
            for node in &self.nodes {
                for value in node.properties().values() {
                    if let PropertyValue::Asset(written) = value {
                        references
                            .push(context.reference(ReferenceKind::Resource, written.clone()));
                    }
                }
            }
            self.references = references;
        }
        Ok(rewritten)
    }

    /// The references that would leave the project root.
    pub fn refused_references(&self) -> Vec<&Reference> {
        self.references
            .iter()
            .filter(|reference| reference.is_refused())
            .collect()
    }

    /// Every member this build did not know, wherever it was in the document.
    pub fn unknown_fields(&self) -> &[UnknownField] {
        &self.unknown
    }

    /// What interpretation left over at the top level.
    pub fn preserved(&self) -> &Preserved {
        &self.preserved
    }

    /// What interpretation left over inside the page header.
    pub fn page_preserved(&self) -> &Preserved {
        &self.page_preserved
    }

    /// The document as JSON, with the members this build did not know put back where they were.
    ///
    /// Fallible because a value the model allows has no JSON spelling: a `number` property that is
    /// not finite is a value nobody can write down, and writing `null` or a rounded number instead
    /// would be a project file that says something the model does not.
    pub fn to_json(&self) -> Result<Json, ProjectError> {
        let mut out = JsonObject::new();
        out.insert("format", Json::number(u64::from(PAGE_FORMAT.get())));
        let mut page = JsonObject::new();
        page.insert("id", Json::number(self.page.id().get()));
        page.insert("name", Json::string(self.page.name()));
        page.insert("root", Json::number(self.page.root().get()));
        self.page_preserved.adopt(&mut page);
        out.insert("page", Json::Object(page));
        let mut nodes = Vec::with_capacity(self.nodes.len());
        for node in &self.nodes {
            nodes.push(encode_node(node)?);
        }
        out.insert("nodes", Json::Array(nodes));
        self.preserved.adopt(&mut out);
        Ok(Json::Object(out))
    }

    /// The document as text.
    pub fn render(&self) -> Result<String, ProjectError> {
        Ok(self.to_json()?.render())
    }
}

fn encode_node(node: &PageNode) -> Result<Json, ProjectError> {
    let mut out = JsonObject::new();
    out.insert("id", Json::number(node.id.get()));
    out.insert("type", Json::string(node.kind.as_str()));
    out.insert(
        "children",
        Json::array(
            node.children()
                .iter()
                .map(|id| Json::number(id.get()))
                .collect::<Vec<_>>(),
        ),
    );
    let mut properties = Vec::with_capacity(node.properties().len());
    for (name, value) in node.properties() {
        let mut entry = JsonObject::new();
        entry.insert("name", Json::string(name.as_str()));
        entry.insert(
            "value",
            encode_property_value(
                &format!("nodes[{}].properties.{}", node.id.get(), name.as_str()),
                value,
            )?,
        );
        properties.push(Json::Object(entry));
    }
    out.insert("properties", Json::Array(properties));
    node.preserved.adopt(&mut out);
    Ok(Json::Object(out))
}

/// The seven value families the model has, each written as a one-member object.
///
/// The match is exhaustive on purpose: a family added to the model stops this compiling instead of
/// being dropped from every page file (principle #35).
fn encode_property_value(path: &str, value: &PropertyValue) -> Result<Json, ProjectError> {
    let mut out = JsonObject::new();
    match value {
        PropertyValue::Bool(value) => {
            out.insert("bool", Json::Bool(*value));
        }
        PropertyValue::Integer(value) => {
            out.insert("integer", Json::Number(Number::from_i64(*value)));
        }
        PropertyValue::Number(value) => {
            if !value.is_finite() {
                return Err(ProjectError::Unrepresentable {
                    path: path.to_owned(),
                    reason: format!("`{value}` is not a number JSON can hold"),
                });
            }
            out.insert("number", Json::Number(Number::new(value.to_string())));
        }
        PropertyValue::Text(value) => {
            out.insert("text", Json::string(value));
        }
        PropertyValue::Variant(value) => {
            out.insert("variant", Json::string(value));
        }
        PropertyValue::Color(value) => {
            out.insert("color", Json::string(value));
        }
        PropertyValue::Asset(value) => {
            out.insert("asset", Json::string(value));
        }
    }
    Ok(Json::Object(out))
}

fn decode_property_value(path: &str, value: &Json) -> Result<PropertyValue, ProjectError> {
    let Some(object) = value.as_object() else {
        return Err(ProjectError::WrongType {
            path: path.to_owned(),
            expected: "an object naming one value family",
            found: value.kind(),
        });
    };
    if object.len() != 1 {
        return Err(ProjectError::Inconsistent {
            path: path.to_owned(),
            reason: format!(
                "a value names exactly one family, and this one names {}",
                object.len()
            ),
        });
    }
    let (family, inner) = object.iter().next().expect("the object has one member");
    let wrong = |expected: &'static str| ProjectError::WrongType {
        path: format!("{path}.{family}"),
        expected,
        found: inner.kind(),
    };
    match family {
        "bool" => match inner {
            Json::Bool(value) => Ok(PropertyValue::Bool(*value)),
            _ => Err(wrong("a boolean")),
        },
        "integer" => inner
            .as_i64()
            .map(PropertyValue::Integer)
            .ok_or_else(|| wrong("a whole number")),
        "number" => {
            let Json::Number(number) = inner else {
                return Err(wrong("a fractional number"));
            };
            let parsed = number
                .literal()
                .parse::<f64>()
                .map_err(|_| ProjectError::WrongType {
                    path: format!("{path}.number"),
                    expected: "a fractional number",
                    found: JsonKind::Number,
                })?;
            if !parsed.is_finite() {
                return Err(ProjectError::Inconsistent {
                    path: format!("{path}.number"),
                    reason: format!("`{}` is not a number this build can hold", number.literal()),
                });
            }
            Ok(PropertyValue::Number(parsed))
        }
        "text" => inner
            .as_str()
            .map(|text| PropertyValue::Text(text.to_owned()))
            .ok_or_else(|| wrong("a string")),
        "variant" => inner
            .as_str()
            .map(|text| PropertyValue::Variant(text.to_owned()))
            .ok_or_else(|| wrong("a string")),
        "color" => inner
            .as_str()
            .map(|text| PropertyValue::Color(text.to_owned()))
            .ok_or_else(|| wrong("a string")),
        "asset" => inner
            .as_str()
            .map(|text| PropertyValue::Asset(text.to_owned()))
            .ok_or_else(|| wrong("a string")),
        other => Err(ProjectError::Inconsistent {
            path: format!("{path}.{other}"),
            reason:
                "a property value is one of bool, integer, number, text, variant, color or asset"
                    .to_owned(),
        }),
    }
}

/// Reads a page document, and says exactly what is wrong with one it refuses.
fn interpret(
    root: &ProjectRoot,
    path: PathBuf,
    value: &Json,
) -> Result<PageDocument, ProjectError> {
    let context = Context { root };
    let version = crate::project::record::format::format_version(value)?;
    match version.get().cmp(&PAGE_FORMAT.get()) {
        std::cmp::Ordering::Equal => {}
        std::cmp::Ordering::Greater => {
            return Err(ProjectError::NewerFormat {
                found: version,
                current: PAGE_FORMAT,
            });
        }
        std::cmp::Ordering::Less => {
            return Err(ProjectError::OlderFormat {
                found: version,
                current: PAGE_FORMAT,
            });
        }
    }

    let mut reader = Reader::object(String::new(), value)?;
    let _ = reader.take("format");
    let (page, page_preserved) = decode_page(&mut reader)?;
    let mut nodes = decode_nodes(&mut reader)?;
    let preserved = reader.finish();
    nodes.sort_by_key(|node| node.id.get());

    check_tree(&page, &nodes)?;

    let mut references = Vec::new();
    for node in &nodes {
        for value in node.properties().values() {
            if let PropertyValue::Asset(written) = value {
                references.push(context.reference(ReferenceKind::Resource, written.clone()));
            }
        }
    }

    let mut unknown = Vec::new();
    preserved.names_at("", &mut unknown);
    page_preserved.names_at("page", &mut unknown);
    for node in &nodes {
        node.preserved
            .names_at(&format!("nodes[{}]", node.id.get()), &mut unknown);
    }

    Ok(PageDocument {
        path,
        root: Some(root.clone()),
        page,
        page_preserved,
        nodes,
        references,
        preserved,
        unknown,
    })
}

fn decode_page(reader: &mut Reader<'_>) -> Result<(Page, Preserved), ProjectError> {
    let Some(mut member) = sub_object(reader, "page")? else {
        return Err(ProjectError::MissingMember {
            path: "page".to_owned(),
        });
    };
    let id = member.required_u64("id")?;
    let name = member.required_string("name")?;
    let root = member.required_u64("root")?;
    let page = Page::new(PageId::from_raw(id), &name, NodeId::from_raw(root)).ok_or(
        ProjectError::EmptyValue {
            path: "page.name".to_owned(),
        },
    )?;
    Ok((page, member.finish()))
}

fn decode_nodes(reader: &mut Reader<'_>) -> Result<Vec<PageNode>, ProjectError> {
    let mut nodes = Vec::new();
    for entry in sub_objects(reader, "nodes")? {
        nodes.push(decode_node(entry)?);
    }
    Ok(nodes)
}

/// One node entry: the page reader's single place that says what a node is.
///
/// Public because a batch of edits (T03.02) writes the same node shape a page document writes, and
/// a second decoder for it would be a second answer to "what is a node".
pub fn node_from_json(at: &str, value: &Json) -> Result<PageNode, ProjectError> {
    decode_node(Reader::object(at.to_owned(), value)?)
}

/// The same node, written the way a page document writes it.
pub fn node_to_json(node: &PageNode) -> Result<Json, ProjectError> {
    encode_node(node)
}

/// One property value, read the way a page document reads it.
///
/// The value families are the model's (`contracts::model::PropertyValue`), and this is the only
/// place that says how one is spelled in a file: a batch of edits (T03.02) sets properties, and a
/// second decoder would be a second answer to what `{"text": "Save"}` means.
pub fn property_from_json(at: &str, value: &Json) -> Result<PropertyValue, ProjectError> {
    decode_property_value(at, value)
}

/// One property value, written the way a page document writes it.
pub fn property_to_json(at: &str, value: &PropertyValue) -> Result<Json, ProjectError> {
    encode_property_value(at, value)
}

fn decode_node(mut entry: Reader<'_>) -> Result<PageNode, ProjectError> {
    {
        let id = entry.required_u64("id")?;
        let kind = entry.required_string("type")?;
        let kind = NodeKind::new(&kind)
            .ok_or_else(|| ProjectError::EmptyValue {
                path: entry.member_path("type"),
            })
            .map_err(|error| match error {
                ProjectError::EmptyValue { path } => ProjectError::Inconsistent {
                    path,
                    reason: "a kind is one non-blank word".to_owned(),
                },
                other => other,
            })?;
        let children = match entry.take_array("children")? {
            None => Vec::new(),
            Some(values) => {
                let mut children = Vec::with_capacity(values.len());
                for (index, value) in values.iter().enumerate() {
                    let Some(child) = value.as_u64() else {
                        return Err(ProjectError::WrongType {
                            path: format!("{}.children[{index}]", entry.path()),
                            expected: "a whole number",
                            found: value.kind(),
                        });
                    };
                    children.push(NodeId::from_raw(child));
                }
                children
            }
        };
        let mut properties = BTreeMap::new();
        if let Some(values) = entry.take_array("properties")? {
            for (index, value) in values.iter().enumerate() {
                let path = format!("{}.properties[{index}]", entry.path());
                let mut property = Reader::object(path.clone(), value)?;
                let name = property.required_string("name")?;
                let name = PropertyName::new(&name).ok_or_else(|| ProjectError::Inconsistent {
                    path: format!("{path}.name"),
                    reason: "a property name is not blank".to_owned(),
                })?;
                let Some(raw) = property.take("value") else {
                    return Err(ProjectError::MissingMember {
                        path: format!("{path}.value"),
                    });
                };
                let decoded = decode_property_value(&format!("{path}.value"), raw)?;
                if !property.finish().is_empty() {
                    return Err(ProjectError::Inconsistent {
                        path: path.clone(),
                        reason: "a property entry is a name and a value, and nothing else"
                            .to_owned(),
                    });
                }
                if properties.insert(name.clone(), decoded).is_some() {
                    return Err(ProjectError::Duplicate {
                        path,
                        claim: format!("the property `{}`", name.as_str()),
                    });
                }
            }
        }
        Ok(PageNode {
            id: NodeId::from_raw(id),
            kind,
            properties,
            children,
            preserved: entry.finish(),
        })
    }
}

/// The file describes one tree rooted at `page.root`, and nothing else.
///
/// The walk is iterative because the tree's depth is not bounded by the JSON nesting: a flat array of
/// nodes can describe a chain as deep as it likes, and a recursive walk would be a stack overflow
/// waiting for a file somebody wrote by hand.
fn check_tree(page: &Page, nodes: &[PageNode]) -> Result<(), ProjectError> {
    let mut seen: Vec<NodeId> = Vec::new();
    for (index, node) in nodes.iter().enumerate() {
        if seen.contains(&node.id) {
            return Err(ProjectError::Duplicate {
                path: format!("nodes[{index}].id"),
                claim: format!("the identity {}", node.id.get()),
            });
        }
        seen.push(node.id);
    }
    if !seen.contains(&page.root()) {
        return Err(ProjectError::Dangling {
            path: "page.root".to_owned(),
            name: page.root().get().to_string(),
        });
    }
    for (index, node) in nodes.iter().enumerate() {
        for (position, child) in node.children.iter().enumerate() {
            if !seen.contains(child) {
                return Err(ProjectError::Dangling {
                    path: format!("nodes[{index}].children[{position}]"),
                    name: child.get().to_string(),
                });
            }
        }
    }

    let root_index = nodes
        .iter()
        .position(|node| node.id == page.root())
        .expect("the root is declared");
    let mut visited = vec![false; nodes.len()];
    let mut queue = std::collections::VecDeque::new();
    queue.push_back(root_index);
    while let Some(index) = queue.pop_front() {
        if visited[index] {
            return Err(ProjectError::Inconsistent {
                path: format!("nodes[{index}].id"),
                reason: format!(
                    "the node {} is reachable from the root more than once, so this is not one tree",
                    nodes[index].id.get()
                ),
            });
        }
        visited[index] = true;
        for child in &nodes[index].children {
            let child_index = nodes
                .iter()
                .position(|node| node.id == *child)
                .expect("every child is declared");
            queue.push_back(child_index);
        }
    }
    for (index, node) in nodes.iter().enumerate() {
        if !visited[index] {
            return Err(ProjectError::Inconsistent {
                path: format!("nodes[{index}].id"),
                reason: format!(
                    "the node {} is not reachable from the root {}",
                    node.id.get(),
                    page.root().get()
                ),
            });
        }
    }
    Ok(())
}

/// What the reader needs to turn a written path into a place.
struct Context<'a> {
    root: &'a ProjectRoot,
}

impl Context<'_> {
    fn reference(&self, kind: ReferenceKind, written: String) -> Reference {
        crate::project::record::format::reference(self.root, kind, written)
    }
}

#[cfg(test)]
mod tests {
    use super::{PAGE_DOCUMENT_MEMBERS, PAGE_NODE_MEMBERS, PageDocument, PageNode};
    use crate::contracts::identity::{NodeId, PageId};
    use crate::contracts::model::{NodeKind, PropertyName, PropertyValue};
    use crate::host::paths::ProjectRoot;
    use crate::project::record::format::{ProjectError, ReferenceVerdict};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// The smallest document this build writes, spelled exactly the way the writer spells it.
    const EMPTY: &str = "{\n  \"format\": 1,\n  \"page\": {\n    \"id\": 1,\n    \"name\": \"Main window\",\n    \"root\": 1\n  },\n  \"nodes\": [\n    {\n      \"id\": 1,\n      \"type\": \"page\",\n      \"children\": [],\n      \"properties\": []\n    }\n  ]\n}\n";

    struct TempDirectory(PathBuf);

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct Sandbox {
        root: PathBuf,
        _guard: TempDirectory,
    }

    fn sandbox() -> Sandbox {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("rustage-page-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&root).expect("a temporary directory is creatable");
        Sandbox {
            root: root.clone(),
            _guard: TempDirectory(root),
        }
    }

    impl Sandbox {
        fn root(&self) -> ProjectRoot {
            ProjectRoot::new(&self.root).expect("the sandbox is a directory")
        }

        fn parse(&self, text: &str) -> Result<PageDocument, ProjectError> {
            PageDocument::parse(&self.root(), self.root.join("pages/main.json"), text)
        }

        fn document(&self, text: &str) -> PageDocument {
            self.parse(text).expect("the document is readable")
        }

        fn failure(&self, text: &str) -> ProjectError {
            self.parse(text).expect_err("the document is refused")
        }

        fn node(&self, id: u64, kind: &str) -> PageNode {
            PageNode::new(NodeId::from_raw(id), NodeKind::new(kind).expect("a kind"))
        }
    }

    #[test]
    fn an_empty_page_is_written_the_way_the_writer_writes_it() {
        let document = PageDocument::empty_page(
            PageId::from_raw(1),
            NodeId::from_raw(1),
            "Main window",
            NodeKind::new("page").expect("a kind"),
            "pages/main.json",
        )
        .expect("a page with a name");

        assert_eq!(document.render().expect("writable"), EMPTY);
        assert_eq!(document.page().name(), "Main window");
        assert_eq!(document.page().root().get(), 1);
        assert_eq!(
            document.root_node().expect("a root").kind().as_str(),
            "page"
        );
        assert_eq!(document.nodes().len(), 1);
        assert_eq!(document.unknown_fields().len(), 0);
        assert!(document.refused_references().is_empty());
        assert_eq!(document.path(), Path::new("pages/main.json"));

        // A page without a name is not a page this build invents a name for.
        assert!(
            PageDocument::empty_page(
                PageId::from_raw(1),
                NodeId::from_raw(1),
                "   ",
                NodeKind::new("page").expect("a kind"),
                "pages/main.json",
            )
            .is_none()
        );
    }

    #[test]
    fn a_document_read_back_is_the_document_that_was_written() {
        let sandbox = sandbox();
        let text = "{\"format\": 1, \"page\": {\"id\": 4, \"name\": \"Settings\", \"root\": 9}, \
             \"nodes\": [{\"id\": 9, \"type\": \"page\", \"children\": [10, 11]}, \
             {\"id\": 10, \"type\": \"label\"}, {\"id\": 11, \"type\": \"button\"}]}";
        let document = sandbox.document(text);
        assert_eq!(document.nodes().len(), 3);
        assert_eq!(document.nodes()[0].id().get(), 9);
        assert_eq!(document.nodes()[0].children().len(), 2);
        assert!(
            document.nodes()[1].children().is_empty(),
            "children may be left out"
        );
        assert!(
            document.nodes()[2].properties().is_empty(),
            "properties may be left out"
        );

        let saved = document.render().expect("writable");
        let again = sandbox.document(&saved);
        assert_eq!(again, document);
        assert_eq!(again.render().expect("writable"), saved);
        // Every member the format defines is written, so a document has exactly one spelling.
        let json = document.to_json().expect("writable");
        assert_eq!(
            json.as_object()
                .expect("an object")
                .names()
                .collect::<Vec<_>>(),
            PAGE_DOCUMENT_MEMBERS
        );
        assert_eq!(
            json.as_object()
                .and_then(|object| object.get("nodes"))
                .and_then(|nodes| nodes.as_array())
                .and_then(|nodes| nodes.first())
                .and_then(|node| node.as_object())
                .map(|node| node.names().collect::<Vec<_>>()),
            Some(PAGE_NODE_MEMBERS.to_vec())
        );
    }

    #[test]
    fn every_value_family_the_model_has_survives_the_file() {
        let sandbox = sandbox();
        let node = |id: u64| sandbox.node(id, "label");
        let id = NodeId::from_raw;
        let name = |value: &str| PropertyName::new(value).expect("a name");
        let root = node(1)
            .with_children([id(2)])
            .with_property(name("visible"), PropertyValue::Bool(true));
        let child = node(2)
            .with_property(name("size"), PropertyValue::Integer(-3))
            .with_property(name("scale"), PropertyValue::Number(1.5))
            .with_property(
                name("label"),
                PropertyValue::Text("Save \u{4e2d}".to_owned()),
            )
            .with_property(name("align"), PropertyValue::Variant("center".to_owned()))
            .with_property(
                name("background"),
                PropertyValue::Color("#1e1e1e".to_owned()),
            )
            .with_property(
                name("icon"),
                PropertyValue::Asset("resources/icon.svg".to_owned()),
            );
        assert_eq!(root.children(), [id(2)]);
        assert_eq!(child.properties().len(), 6);
        assert_eq!(root.properties().len(), 1);

        let text = format!(
            "{{\"format\": 1, \"page\": {{\"id\": 1, \"name\": \"P\", \"root\": 1}}, \
             \"nodes\": [{}, {}]}}",
            // Written by hand so the *file's* spelling is what is under test, not the writer's.
            "{\"id\": 1, \"type\": \"label\", \"children\": [2], \
             \"properties\": [{\"name\": \"visible\", \"value\": {\"bool\": true}}]}",
            "{\"id\": 2, \"type\": \"label\", \"properties\": [\
             {\"name\": \"size\", \"value\": {\"integer\": -3}}, \
             {\"name\": \"scale\", \"value\": {\"number\": 1.5}}, \
             {\"name\": \"label\", \"value\": {\"text\": \"Save \u{4e2d}\"}}, \
             {\"name\": \"align\", \"value\": {\"variant\": \"center\"}}, \
             {\"name\": \"background\", \"value\": {\"color\": \"#1e1e1e\"}}, \
             {\"name\": \"icon\", \"value\": {\"asset\": \"resources/icon.svg\"}}]}",
        );
        let document = sandbox.document(&text);
        assert_eq!(
            document
                .node(NodeId::from_raw(1))
                .expect("the root")
                .properties()
                .len(),
            1
        );
        let properties = document
            .node(NodeId::from_raw(2))
            .expect("a child")
            .properties();
        assert_eq!(
            properties.get(&PropertyName::new("scale").expect("a name")),
            Some(&PropertyValue::Number(1.5))
        );
        assert_eq!(
            properties.get(&PropertyName::new("label").expect("a name")),
            Some(&PropertyValue::Text("Save \u{4e2d}".to_owned()))
        );
        assert_eq!(
            properties.get(&PropertyName::new("icon").expect("a name")),
            Some(&PropertyValue::Asset("resources/icon.svg".to_owned()))
        );

        // And writing it out gives something that reads back the same, family by family.
        let saved = document.render().expect("writable");
        assert!(saved.contains("\"number\": 1.5"));
        assert!(saved.contains("\"integer\": -3"));
        assert!(saved.contains("Save \u{4e2d}"));
        assert!(saved.contains("\"asset\": \"resources/icon.svg\""));
        assert_eq!(sandbox.document(&saved), document);
    }

    #[test]
    fn rewiring_asset_references_reports_the_count_and_keeps_unknown_members() {
        let sandbox = sandbox();
        let text = "{\"format\": 1, \"future\": {\"note\": \"keep\"}, \
             \"page\": {\"id\": 1, \"name\": \"P\", \"root\": 1, \"gridSize\": 8}, \
             \"nodes\": [ \
               {\"id\": 1, \"type\": \"page\", \"children\": [2], \"properties\": [ \
                 {\"name\": \"icon\", \"value\": {\"asset\": \"resources/icon.svg\"}}]}, \
               {\"id\": 2, \"type\": \"image\", \"locked\": true, \"properties\": [ \
                 {\"name\": \"source\", \"value\": {\"asset\": \"resources/icon.svg\"}}, \
                 {\"name\": \"label\", \"value\": {\"text\": \"Save\"}}]}]}";
        let mut document = sandbox.document(text);
        assert_eq!(document.references().len(), 2);

        let rewired = document
            .rewire_asset("resources/icon.svg", "resources/icons/app.svg")
            .expect("the asset references are rewritable");
        assert_eq!(rewired, 2);
        assert!(
            document
                .references()
                .iter()
                .all(|reference| reference.written() == "resources/icons/app.svg")
        );

        let saved = document.render().expect("writable");
        assert!(saved.contains("resources/icons/app.svg"));
        assert!(!saved.contains("resources/icon.svg"));
        assert!(saved.contains("\"gridSize\": 8"));
        assert!(saved.contains("\"locked\": true"));
        assert!(saved.contains("\"future\""));
        assert!(saved.contains("\"keep\""));

        // The members this build does not know are still reported after the rewrite.
        let again = sandbox.document(&saved);
        assert_eq!(again.unknown_fields().len(), 3);

        // Nothing names the old path any more, and a blank path is refused rather than written.
        assert_eq!(
            document
                .rewire_asset("resources/icon.svg", "elsewhere.svg")
                .expect("the asset references are rewritable"),
            0
        );
        assert!(document.rewire_asset("", "elsewhere.svg").is_err());
    }

    #[test]
    fn an_asset_reference_to_another_path_is_not_rewritten() {
        let sandbox = sandbox();
        let text = "{\"format\": 1, \"page\": {\"id\": 1, \"name\": \"P\", \"root\": 1}, \
             \"nodes\": [{\"id\": 1, \"type\": \"page\", \"properties\": [ \
               {\"name\": \"icon\", \"value\": {\"asset\": \"resources/icon.svg\"}}]}]}";
        let mut document = sandbox.document(text);
        let before = document.render().expect("writable");

        assert_eq!(
            document
                .rewire_asset("resources/other.svg", "resources/new.svg")
                .expect("the asset references are rewritable"),
            0
        );
        assert_eq!(document.render().expect("writable"), before);
        assert_eq!(document.references()[0].written(), "resources/icon.svg");

        // Rewriting a path to itself changes nothing either.
        assert_eq!(
            document
                .rewire_asset("resources/icon.svg", "resources/icon.svg")
                .expect("the asset references are rewritable"),
            0
        );
        assert_eq!(document.render().expect("writable"), before);
    }

    #[test]
    fn a_value_json_cannot_hold_is_refused_rather_than_rounded() {
        let document = PageDocument::empty_page(
            PageId::from_raw(1),
            NodeId::from_raw(1),
            "P",
            NodeKind::new("page").expect("a kind"),
            "pages/main.json",
        )
        .expect("a page");
        let mut nodes = document.nodes().to_vec();
        nodes[0] = nodes[0].clone().with_property(
            PropertyName::new("scale").expect("a name"),
            PropertyValue::Number(f64::NAN),
        );
        let document = PageDocument { nodes, ..document };
        let error = document.render().expect_err("a NaN has no spelling");
        assert!(matches!(error, ProjectError::Unrepresentable { .. }));
        assert!(error.message().contains("not a number JSON can hold"));
    }

    #[test]
    fn a_member_this_build_does_not_know_is_kept_and_reported() {
        let sandbox = sandbox();
        let text = "{\"format\": 1, \"future\": {\"a\": 1}, \
             \"page\": {\"id\": 1, \"name\": \"P\", \"root\": 1, \"gridSize\": 8}, \
             \"nodes\": [{\"id\": 1, \"type\": \"page\", \"parent\": null, \"locked\": true}]}";
        let document = sandbox.document(text);
        assert_eq!(
            document
                .unknown_fields()
                .iter()
                .map(|field| field.path())
                .collect::<Vec<_>>(),
            [
                "future",
                "page.gridSize",
                "nodes[1].parent",
                "nodes[1].locked"
            ]
        );
        let saved = document.render().expect("writable");
        assert!(saved.contains("\"gridSize\": 8"));
        assert!(saved.contains("\"locked\": true"));
        let again = sandbox.document(&saved);
        assert_eq!(again.unknown_fields(), document.unknown_fields());
        // A `parent` member is not part of the format: the tree is written once, as children, and a
        // member nobody reads is kept and reported rather than believed to describe the tree.
        assert!(saved.contains("\"parent\": null"));
        assert!(
            document
                .unknown_fields()
                .iter()
                .any(|field| field.path() == "nodes[1].parent"),
            "a member the format does not define is reported, not read"
        );
        assert_eq!(
            document
                .node(crate::contracts::identity::NodeId::from_raw(1))
                .expect("the root")
                .children()
                .len(),
            0,
            "the tree comes from `children`, so a `parent` member changes nothing"
        );
    }

    #[test]
    fn a_version_this_build_does_not_write_is_refused_with_a_reason() {
        let sandbox = sandbox();
        let newer = sandbox.failure(
            "{\"format\": 2, \"page\": {\"id\": 1, \"name\": \"P\", \"root\": 1}, \
             \"nodes\": [{\"id\": 1, \"type\": \"page\"}]}",
        );
        assert!(matches!(newer, ProjectError::NewerFormat { .. }));
        assert!(newer.message().contains("newer"));
        assert!(newer.message().contains("nothing was written"));

        let older = sandbox.failure(
            "{\"format\": 0, \"page\": {\"id\": 1, \"name\": \"P\", \"root\": 1}, \
             \"nodes\": [{\"id\": 1, \"type\": \"page\"}]}",
        );
        assert!(matches!(older, ProjectError::OlderFormat { .. }));

        let missing = sandbox.failure("{\"page\": {\"id\": 1, \"name\": \"P\", \"root\": 1}}");
        assert!(missing.message().contains("format is missing"));
        let wrong = sandbox.failure("{\"format\": \"1\"}");
        assert!(wrong.message().contains("format must be a whole number"));
        let not_an_object = sandbox.failure("[]");
        assert!(
            not_an_object
                .message()
                .contains("document must be an object")
        );
    }

    #[test]
    fn a_document_that_is_not_a_tree_is_refused() {
        let cases = [
            (
                "{\"format\": 1, \"page\": {\"id\": 1, \"name\": \"P\", \"root\": 1}, \
                 \"nodes\": [{\"id\": 1, \"type\": \"page\"}, {\"id\": 1, \"type\": \"page\"}]}",
                "the identity 1",
            ),
            (
                "{\"format\": 1, \"page\": {\"id\": 1, \"name\": \"P\", \"root\": 5}, \
                 \"nodes\": [{\"id\": 1, \"type\": \"page\"}]}",
                "page.root names `5`",
            ),
            (
                "{\"format\": 1, \"page\": {\"id\": 1, \"name\": \"P\", \"root\": 1}, \
                 \"nodes\": [{\"id\": 1, \"type\": \"page\", \"children\": [7]}]}",
                "nodes[0].children[0] names `7`",
            ),
            (
                "{\"format\": 1, \"page\": {\"id\": 1, \"name\": \"P\", \"root\": 1}, \
                 \"nodes\": [{\"id\": 1, \"type\": \"page\", \"children\": [2]}, \
                 {\"id\": 2, \"type\": \"label\"}, {\"id\": 3, \"type\": \"label\"}]}",
                "is not reachable from the root",
            ),
            (
                "{\"format\": 1, \"page\": {\"id\": 1, \"name\": \"P\", \"root\": 1}, \
                 \"nodes\": [{\"id\": 1, \"type\": \"page\", \"children\": [2, 2]}, \
                 {\"id\": 2, \"type\": \"label\"}]}",
                "reachable from the root more than once",
            ),
            (
                "{\"format\": 1, \"page\": {\"id\": 1, \"name\": \"P\", \"root\": 1}, \
                 \"nodes\": [{\"id\": 1, \"type\": \"page\", \"children\": [2]}, \
                 {\"id\": 2, \"type\": \"label\", \"children\": [1]}]}",
                "reachable from the root more than once",
            ),
        ];
        for (text, expected) in cases {
            let sandbox = sandbox();
            let error = sandbox.failure(text);
            assert!(
                error.message().contains(expected),
                "refused as `{}`",
                error.message()
            );
        }
    }

    #[test]
    fn a_member_of_the_wrong_shape_is_refused_with_the_path_that_holds_it() {
        for (text, expected) in [
            ("{\"format\": 1}", "page is missing"),
            (
                "{\"format\": 1, \"page\": {\"id\": 1, \"name\": \"\", \"root\": 1}}",
                "page.name is empty",
            ),
            (
                "{\"format\": 1, \"page\": {\"id\": 1, \"name\": \"P\", \"root\": 1}, \"nodes\": {}}",
                "nodes must be an array",
            ),
            (
                "{\"format\": 1, \"page\": {\"id\": 1, \"name\": \"P\", \"root\": 1}, \
                 \"nodes\": [{\"type\": \"page\"}]}",
                "nodes[0].id is missing",
            ),
            (
                "{\"format\": 1, \"page\": {\"id\": 1, \"name\": \"P\", \"root\": 1}, \
                 \"nodes\": [{\"id\": 1, \"type\": \"two words\"}]}",
                "a kind is one non-blank word",
            ),
            (
                "{\"format\": 1, \"page\": {\"id\": 1, \"name\": \"P\", \"root\": 1}, \
                 \"nodes\": [{\"id\": 1, \"type\": \"page\", \"children\": [\"x\"]}]}",
                "nodes[0].children[0] must be a whole number",
            ),
            (
                "{\"format\": 1, \"page\": {\"id\": 1, \"name\": \"P\", \"root\": 1}, \
                 \"nodes\": [{\"id\": 1, \"type\": \"page\", \
                 \"properties\": [{\"name\": \"a\"}]}]}",
                "nodes[0].properties[0].value is missing",
            ),
            (
                "{\"format\": 1, \"page\": {\"id\": 1, \"name\": \"P\", \"root\": 1}, \
                 \"nodes\": [{\"id\": 1, \"type\": \"page\", \
                 \"properties\": [{\"name\": \"a\", \"value\": {\"text\": \"x\", \"bool\": true}}]}]}",
                "names exactly one family",
            ),
            (
                "{\"format\": 1, \"page\": {\"id\": 1, \"name\": \"P\", \"root\": 1}, \
                 \"nodes\": [{\"id\": 1, \"type\": \"page\", \
                 \"properties\": [{\"name\": \"a\", \"value\": {\"font\": \"x\"}}]}]}",
                "is one of bool, integer, number, text, variant, color or asset",
            ),
            (
                "{\"format\": 1, \"page\": {\"id\": 1, \"name\": \"P\", \"root\": 1}, \
                 \"nodes\": [{\"id\": 1, \"type\": \"page\", \
                 \"properties\": [{\"name\": \"a\", \"value\": {\"integer\": 3.5}}]}]}",
                "nodes[0].properties[0].value.integer must be a whole number",
            ),
            (
                "{\"format\": 1, \"page\": {\"id\": 1, \"name\": \"P\", \"root\": 1}, \
                 \"nodes\": [{\"id\": 1, \"type\": \"page\", \
                 \"properties\": [{\"name\": \"a\", \"value\": {\"text\": \"x\"}}, \
                 {\"name\": \"a\", \"value\": {\"text\": \"y\"}}]}]}",
                "the property `a`",
            ),
        ] {
            let sandbox = sandbox();
            let error = sandbox.failure(text);
            assert!(
                error.message().contains(expected),
                "{text} was refused as `{}`",
                error.message()
            );
        }
    }

    #[test]
    fn an_asset_that_leaves_the_root_is_refused_and_reported() {
        let sandbox = sandbox();
        std::fs::create_dir_all(sandbox.root.join("resources")).expect("a directory");
        std::fs::write(sandbox.root.join("resources/icon.svg"), "<svg/>").expect("a file");
        let text = "{\"format\": 1, \"page\": {\"id\": 1, \"name\": \"P\", \"root\": 1}, \
             \"nodes\": [{\"id\": 1, \"type\": \"page\", \"properties\": [\
             {\"name\": \"icon\", \"value\": {\"asset\": \"resources/icon.svg\"}}, \
             {\"name\": \"other\", \"value\": {\"asset\": \"../outside.svg\"}}, \
             {\"name\": \"later\", \"value\": {\"asset\": \"resources/later.svg\"}}]}]}";
        let document = sandbox.document(text);
        assert_eq!(document.references().len(), 3);
        let by_written = |written: &str| {
            document
                .references()
                .iter()
                .find(|reference| reference.written() == written)
                .expect("the reference is reported")
        };
        assert_eq!(
            by_written("resources/icon.svg").verdict(),
            &ReferenceVerdict::Present
        );
        assert_eq!(by_written("../outside.svg").kind().name(), "resource");
        assert!(by_written("../outside.svg").is_refused());
        assert_eq!(
            by_written("resources/later.svg").verdict(),
            &ReferenceVerdict::Absent
        );
        assert_eq!(document.refused_references().len(), 1);
        assert!(by_written("../outside.svg").render().contains("refused"));
    }

    #[test]
    fn a_page_document_is_read_from_a_path_inside_its_project() {
        let sandbox = sandbox();
        std::fs::create_dir_all(sandbox.root.join("pages")).expect("a directory");
        std::fs::write(sandbox.root.join("pages/main.json"), EMPTY).expect("a file");
        let document = PageDocument::read(&sandbox.root(), "pages/main.json").expect("readable");
        assert_eq!(document.render().expect("writable"), EMPTY);
        assert!(document.path().ends_with("pages/main.json"));

        let missing = PageDocument::read(&sandbox.root(), "pages/other.json");
        assert!(matches!(missing, Err(ProjectError::Io { .. })));
        let outside = PageDocument::read(&sandbox.root(), "../main.json");
        assert!(matches!(outside, Err(ProjectError::Io { .. })));
    }

    #[test]
    fn text_that_is_not_json_is_refused_with_the_position() {
        let sandbox = sandbox();
        let error = sandbox.failure("{\n  \"format\": 1,\n");
        match error {
            ProjectError::NotJson { path, error } => {
                assert!(path.ends_with("main.json"));
                assert_eq!(error.line(), 3);
            }
            other => panic!("expected a JSON refusal, found {other:?}"),
        }
    }
}
