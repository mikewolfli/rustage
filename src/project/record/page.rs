// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! A page's own document, format version 7: contract **C-PROJECT**.
//!
//! A version 7 page document adds the per-node `constraints` member to the version 6 nodes:
//!
//! | member | kind | meaning |
//! |---|---|---|
//! | `format` | whole number | the format version; required, and this build writes `7` |
//! | `page` | object | `id`, `name` and `root`, all required |
//! | `nodes` | array | one entry per node: `id` and `type` (both required), `children`, `properties`, `layout`, `constraints` |
//! | `states` | object or null | one explicit state machine, or null |
//! | `collections` | array | one entry per collection control: columns, rows and source |
//! | `components` | object | definitions, instances and variants |
//! | `motion` | object | transitions and animations with their timing |
//!
//! Version 2 added `states` (T13.06), 3 `collections` (T27.01), 4 `components` (T12.01–T12.03),
//! 5 `motion` (T11.06), 6 the root node's `layout`, and 7 the per-node `constraints` (upstream
//! RW-109, released in rust-widgets 4.0.1). Older documents are read with the members they have and
//! written back as version 7; a caller that writes a migrated document keeps the bytes it read
//! (decision `Q6`).
//!
//! A node is:
//!
//! ```json
//! { "id": 2, "type": "button", "children": [3], "properties": [ { "name": "label", "value": { "text": "Save" } } ],
//!   "layout": null, "constraints": { "min_width": 120, "margin": [8, 4] } }
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
pub const PAGE_FORMAT: FormatVersion = FormatVersion::from_raw(7);

/// The oldest page format this build reads. Version 1 has no `states`; version 2 has no
/// `collections`; version 3 has no `components`; version 4 has no `motion`; version 5 has no root
/// `layout`; version 6 has no per-node `constraints`.
pub const PAGE_OLDEST_READABLE: FormatVersion = FormatVersion::from_raw(1);

/// The member names a page document version 7 defines, in the order the writer writes them.
pub const PAGE_DOCUMENT_MEMBERS: [&str; 7] = [
    "format",
    "page",
    "nodes",
    "states",
    "collections",
    "components",
    "motion",
];

/// The member names a node may have, in the order the writer writes them.
pub const PAGE_NODE_MEMBERS: [&str; 6] = [
    "id",
    "type",
    "children",
    "properties",
    "layout",
    "constraints",
];

/// The constraint names a node's `constraints` object may carry (upstream RW-109).
///
/// Six size bounds, a margin and an overflow policy — the same spellings the library's
/// `ChildLayoutAttrs` reads from a node's attribute object. Spelled here once so the page model,
/// the translation and the panel cannot drift from each other or from the library.
pub const PAGE_CONSTRAINT_MEMBERS: [&str; 8] = [
    "min_width",
    "min_height",
    "pref_width",
    "pref_height",
    "max_width",
    "max_height",
    "margin",
    "overflow",
];

/// One guarded transition in a page's state machine.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PageTransition {
    from: String,
    on: String,
    to: String,
    guard: Option<String>,
    preserved: Preserved,
}

impl PageTransition {
    /// A transition from `from` to `to` on `on`, with an optional guard.
    pub fn new(
        from: impl Into<String>,
        on: impl Into<String>,
        to: impl Into<String>,
        guard: Option<String>,
    ) -> Self {
        Self {
            from: from.into(),
            on: on.into(),
            to: to.into(),
            guard,
            preserved: Preserved::default(),
        }
    }

    /// The state it leaves.
    pub fn from(&self) -> &str {
        &self.from
    }

    /// The event that triggers it.
    pub fn on(&self) -> &str {
        &self.on
    }

    /// The state it enters.
    pub fn to(&self) -> &str {
        &self.to
    }

    /// The guard that must pass, when one is named.
    pub fn guard(&self) -> Option<&str> {
        self.guard.as_deref()
    }

    /// What a newer document wrote here that this build does not know.
    pub fn preserved(&self) -> &Preserved {
        &self.preserved
    }
}

/// A page's explicit state machine (task T13.06), as the page document holds it.
///
/// The format keeps the declaration only: the runtime session, the target capability and the
/// actions an event runs are not document facts and are not written here.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct PageStates {
    declared: Vec<String>,
    initial: Option<String>,
    finals: Vec<String>,
    transitions: Vec<PageTransition>,
    preserved: Preserved,
}

impl PageStates {
    /// A machine from its declaration, refused when the declaration is not one.
    pub fn new(
        declared: Vec<String>,
        initial: Option<String>,
        finals: Vec<String>,
        transitions: Vec<PageTransition>,
    ) -> Result<Self, String> {
        let states = Self {
            declared,
            initial,
            finals,
            transitions,
            preserved: Preserved::default(),
        };
        states.check()?;
        Ok(states)
    }

    /// The declared states, in declaration order.
    pub fn declared(&self) -> &[String] {
        &self.declared
    }

    /// The initial state, when one is named.
    pub fn initial(&self) -> Option<&str> {
        self.initial.as_deref()
    }

    /// The final states.
    pub fn finals(&self) -> &[String] {
        &self.finals
    }

    /// The transitions, in declaration order.
    pub fn transitions(&self) -> &[PageTransition] {
        &self.transitions
    }

    /// What a newer document wrote here that this build does not know.
    pub fn preserved(&self) -> &Preserved {
        &self.preserved
    }

    /// Checks the seven rules the format states, in the order a reader can report them.
    pub fn check(&self) -> Result<(), String> {
        if self.declared.is_empty() {
            return Err("`declared` must name at least one state".to_owned());
        }
        let mut seen = std::collections::BTreeSet::new();
        for name in &self.declared {
            if name.trim().is_empty() {
                return Err("a state name is empty".to_owned());
            }
            if name.split_whitespace().count() != 1 {
                return Err(format!("the state name {name:?} contains whitespace"));
            }
            if !seen.insert(name.as_str()) {
                return Err(format!("the state {name:?} is declared twice"));
            }
        }
        if let Some(initial) = &self.initial
            && !seen.contains(initial.as_str())
        {
            return Err(format!(
                "the initial state {initial:?} is not declared in `declared`"
            ));
        }
        let mut finals = std::collections::BTreeSet::new();
        for name in &self.finals {
            if !seen.contains(name.as_str()) {
                return Err(format!(
                    "the final state {name:?} is not declared in `declared`"
                ));
            }
            if !finals.insert(name.as_str()) {
                return Err(format!("the final state {name:?} is declared twice"));
            }
        }
        for transition in &self.transitions {
            if !seen.contains(transition.from.as_str()) {
                return Err(format!(
                    "a transition leaves {:?}, which is not declared in `declared`",
                    transition.from
                ));
            }
            if !seen.contains(transition.to.as_str()) {
                return Err(format!(
                    "a transition enters {:?}, which is not declared in `declared`",
                    transition.to
                ));
            }
            if transition.on.trim().is_empty() {
                return Err("a transition names no event in `on`".to_owned());
            }
            if let Some(guard) = &transition.guard
                && guard.trim().is_empty()
            {
                return Err("a transition names an empty guard".to_owned());
            }
        }
        Ok(())
    }
}

/// Whether a collection's content is a design-time sample or runtime source content.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PageOrigin {
    /// An example typed into the design.
    Sample,
    /// Content a data source provides.
    Source,
}

impl PageOrigin {
    /// The origin's name, as the file writes it.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Sample => "sample",
            Self::Source => "source",
        }
    }

    /// Reads an origin's name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "sample" => Some(Self::Sample),
            "source" => Some(Self::Source),
            _ => None,
        }
    }
}

/// Whether the collection's source can currently be reached.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PageAvailability {
    /// Reachable.
    Online,
    /// Not reachable.
    Offline,
}

impl PageAvailability {
    /// The name the file writes.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Offline => "offline",
        }
    }

    /// Reads an availability's name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "online" => Some(Self::Online),
            "offline" => Some(Self::Offline),
            _ => None,
        }
    }
}

/// What a column's cells are for.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PageCellRole {
    /// Plain display text.
    Text,
    /// A checkbox-like flag.
    Check,
    /// A badge drawn from a named set.
    Badge(String),
    /// A link to another row, held by identity.
    Reference(u64),
}

/// One column of a collection, as the file holds it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PageColumn {
    id: u64,
    header: String,
    role: PageCellRole,
    preserved: Preserved,
}

impl PageColumn {
    /// A column of the given identity, header and role.
    pub fn new(id: u64, header: impl Into<String>, role: PageCellRole) -> Self {
        Self {
            id,
            header: header.into(),
            role,
            preserved: Preserved::default(),
        }
    }

    /// The column's identity.
    pub const fn id(&self) -> u64 {
        self.id
    }

    /// The header shown above the column.
    pub fn header(&self) -> &str {
        &self.header
    }

    /// What the column's cells are for.
    pub fn role(&self) -> &PageCellRole {
        &self.role
    }
}

/// One cell's value, held against a column identity.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PageCell {
    column: u64,
    value: String,
    preserved: Preserved,
}

impl PageCell {
    /// A cell for a column.
    pub fn new(column: u64, value: impl Into<String>) -> Self {
        Self {
            column,
            value: value.into(),
            preserved: Preserved::default(),
        }
    }

    /// The column this cell belongs to.
    pub const fn column(&self) -> u64 {
        self.column
    }

    /// The cell's text.
    pub fn value(&self) -> &str {
        &self.value
    }
}

/// One row of a collection.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PageRow {
    id: u64,
    cells: Vec<PageCell>,
    preserved: Preserved,
}

impl PageRow {
    /// A row of the given identity with its cells.
    pub fn new(id: u64, cells: Vec<PageCell>) -> Self {
        Self {
            id,
            cells,
            preserved: Preserved::default(),
        }
    }

    /// The row's identity.
    pub const fn id(&self) -> u64 {
        self.id
    }

    /// The row's cells.
    pub fn cells(&self) -> &[PageCell] {
        &self.cells
    }
}

/// The contract that says where a collection's real rows come from.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PageSource {
    adapter: String,
    provides: Vec<String>,
    availability: PageAvailability,
    preserved: Preserved,
}

impl PageSource {
    /// A contract for an adapter and the columns it promises.
    pub fn new(
        adapter: impl Into<String>,
        provides: Vec<String>,
        availability: PageAvailability,
    ) -> Self {
        Self {
            adapter: adapter.into(),
            provides,
            availability,
            preserved: Preserved::default(),
        }
    }

    /// The adapter that answers at runtime.
    pub fn adapter(&self) -> &str {
        &self.adapter
    }

    /// The headers the source promises.
    pub fn provides(&self) -> &[String] {
        &self.provides
    }

    /// Whether the source can be reached.
    pub const fn availability(&self) -> PageAvailability {
        self.availability
    }
}

/// One collection control's content: its columns, rows and source contract.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PageCollection {
    node: NodeId,
    origin: PageOrigin,
    columns: Vec<PageColumn>,
    rows: Vec<PageRow>,
    source: Option<PageSource>,
    preserved: Preserved,
}

impl PageCollection {
    /// A collection for a control.
    pub fn new(
        node: NodeId,
        origin: PageOrigin,
        columns: Vec<PageColumn>,
        rows: Vec<PageRow>,
        source: Option<PageSource>,
    ) -> Result<Self, String> {
        let collection = Self {
            node,
            origin,
            columns,
            rows,
            source,
            preserved: Preserved::default(),
        };
        collection.check()?;
        Ok(collection)
    }

    /// The control node this content belongs to.
    pub const fn node(&self) -> NodeId {
        self.node
    }

    /// Whether the content is a sample or source content.
    pub const fn origin(&self) -> PageOrigin {
        self.origin
    }

    /// The columns, in display order.
    pub fn columns(&self) -> &[PageColumn] {
        &self.columns
    }

    /// The rows, in display order.
    pub fn rows(&self) -> &[PageRow] {
        &self.rows
    }

    /// The source contract, when there is one.
    pub fn source(&self) -> Option<&PageSource> {
        self.source.as_ref()
    }

    /// Checks the identity, role and source rules the format states.
    pub fn check(&self) -> Result<(), String> {
        let mut columns = std::collections::BTreeSet::new();
        for column in &self.columns {
            if !columns.insert(column.id) {
                return Err(format!("column id {} appears twice", column.id));
            }
            if column.header.trim().is_empty() {
                return Err(format!("column {} has an empty header", column.id));
            }
            if let PageCellRole::Badge(label) = &column.role
                && label.trim().is_empty()
            {
                return Err(format!("column {} has an empty badge", column.id));
            }
        }
        let mut rows = std::collections::BTreeSet::new();
        for row in &self.rows {
            if !rows.insert(row.id) {
                return Err(format!("row id {} appears twice", row.id));
            }
            let mut cells = std::collections::BTreeSet::new();
            for cell in &row.cells {
                if !columns.contains(&cell.column) {
                    return Err(format!(
                        "row {} has a cell for unknown column {}",
                        row.id, cell.column
                    ));
                }
                if !cells.insert(cell.column) {
                    return Err(format!(
                        "row {} has two cells for column {}",
                        row.id, cell.column
                    ));
                }
            }
        }
        for column in &self.columns {
            if let PageCellRole::Reference(target) = &column.role
                && !rows.contains(target)
            {
                return Err(format!(
                    "column {} references unknown row {target}",
                    column.id
                ));
            }
        }
        match (&self.origin, &self.source) {
            (PageOrigin::Source, None) => {
                return Err("a source collection needs a source contract".to_owned());
            }
            (PageOrigin::Sample, Some(_)) => {
                return Err("a sample collection cannot carry a source contract".to_owned());
            }
            (PageOrigin::Source, Some(source)) => {
                if source.adapter.trim().is_empty() {
                    return Err("the source adapter is empty".to_owned());
                }
                let mut provides = std::collections::BTreeSet::new();
                for header in &source.provides {
                    if header.trim().is_empty() {
                        return Err("the source promises an empty header".to_owned());
                    }
                    if !provides.insert(header.as_str()) {
                        return Err(format!("the source promises {header:?} twice"));
                    }
                }
            }
            (PageOrigin::Sample, None) => {}
        }
        Ok(())
    }
}

/// One piece of a component definition's body, as the file holds it.
#[derive(Clone, PartialEq, Debug)]
pub enum PagePart {
    /// A concrete control with its properties.
    Control {
        /// The control kind.
        kind: NodeKind,
        /// The control's properties.
        properties: BTreeMap<PropertyName, PropertyValue>,
    },
    /// A reference to another definition.
    Nested(String),
    /// A slot the placed instance fills.
    Slot(String),
}

/// One declared parameter of a component definition.
#[derive(Clone, PartialEq, Debug)]
pub struct PageParameter {
    name: String,
    default: Option<PropertyValue>,
    preserved: Preserved,
}

impl PageParameter {
    /// A parameter with its optional default.
    pub fn new(name: impl Into<String>, default: Option<PropertyValue>) -> Self {
        Self {
            name: name.into(),
            default,
            preserved: Preserved::default(),
        }
    }

    /// The parameter's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The value used when an instance leaves the argument out.
    pub fn default(&self) -> Option<&PropertyValue> {
        self.default.as_ref()
    }
}

/// One reusable component definition.
#[derive(Clone, PartialEq, Debug)]
pub struct PageDefinition {
    name: String,
    version: u32,
    parameters: Vec<PageParameter>,
    slots: Vec<String>,
    body: Vec<PagePart>,
    preserved: Preserved,
}

impl PageDefinition {
    /// A definition from its parts.
    pub fn new(
        name: impl Into<String>,
        version: u32,
        parameters: Vec<PageParameter>,
        slots: Vec<String>,
        body: Vec<PagePart>,
    ) -> Self {
        Self {
            name: name.into(),
            version,
            parameters,
            slots,
            body,
            preserved: Preserved::default(),
        }
    }

    /// The definition's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The definition's version.
    pub const fn version(&self) -> u32 {
        self.version
    }

    /// The declared parameters.
    pub fn parameters(&self) -> &[PageParameter] {
        &self.parameters
    }

    /// The declared slot names.
    pub fn slots(&self) -> &[String] {
        &self.slots
    }

    /// The body, in order.
    pub fn body(&self) -> &[PagePart] {
        &self.body
    }
}

/// One fill for a slot.
#[derive(Clone, PartialEq, Debug)]
pub struct PageFill {
    slot: String,
    parts: Vec<PagePart>,
    preserved: Preserved,
}

impl PageFill {
    /// A fill for a slot.
    pub fn new(slot: impl Into<String>, parts: Vec<PagePart>) -> Self {
        Self {
            slot: slot.into(),
            parts,
            preserved: Preserved::default(),
        }
    }

    /// The slot this fill is for.
    pub fn slot(&self) -> &str {
        &self.slot
    }

    /// The parts substituted into the slot.
    pub fn parts(&self) -> &[PagePart] {
        &self.parts
    }
}

/// Placed use of a definition.
#[derive(Clone, PartialEq, Debug)]
pub struct PageInstance {
    definition: String,
    node: NodeId,
    arguments: BTreeMap<String, PropertyValue>,
    fills: Vec<PageFill>,
    preserved: Preserved,
}

impl PageInstance {
    /// An instance of a definition under a document identity.
    pub fn new(definition: impl Into<String>, node: NodeId) -> Self {
        Self {
            definition: definition.into(),
            node,
            arguments: BTreeMap::new(),
            fills: Vec::new(),
            preserved: Preserved::default(),
        }
    }

    /// The definition this instance is of.
    pub fn definition(&self) -> &str {
        &self.definition
    }

    /// The instance's identity.
    pub const fn node(&self) -> NodeId {
        self.node
    }

    /// The arguments, by parameter name.
    pub fn arguments(&self) -> &BTreeMap<String, PropertyValue> {
        &self.arguments
    }

    /// The slot fills.
    pub fn fills(&self) -> &[PageFill] {
        &self.fills
    }
}

/// A variant's targets and local overrides.
#[derive(Clone, PartialEq, Debug)]
pub struct PageVariant {
    instance: NodeId,
    definition: String,
    kind: NodeKind,
    targets: Vec<String>,
    overrides: BTreeMap<PropertyName, PropertyValue>,
    preserved: Preserved,
}

impl PageVariant {
    /// A variant of an instance.
    pub fn new(
        instance: NodeId,
        definition: impl Into<String>,
        kind: NodeKind,
        targets: Vec<String>,
        overrides: BTreeMap<PropertyName, PropertyValue>,
    ) -> Self {
        Self {
            instance,
            definition: definition.into(),
            kind,
            targets,
            overrides,
            preserved: Preserved::default(),
        }
    }

    /// The instance this variant is of.
    pub const fn instance(&self) -> NodeId {
        self.instance
    }

    /// The definition the instance inherits from.
    pub fn definition(&self) -> &str {
        &self.definition
    }

    /// The root control kind.
    pub fn kind(&self) -> &NodeKind {
        &self.kind
    }

    /// The targets the variant is aimed at.
    pub fn targets(&self) -> &[String] {
        &self.targets
    }

    /// The local overrides.
    pub fn overrides(&self) -> &BTreeMap<PropertyName, PropertyValue> {
        &self.overrides
    }
}

/// A page's component definitions, instances and variants (format 4).
#[derive(Clone, PartialEq, Debug, Default)]
pub struct PageComponents {
    definitions: Vec<PageDefinition>,
    instances: Vec<PageInstance>,
    variants: Vec<PageVariant>,
    preserved: Preserved,
}

impl PageComponents {
    /// A component section from its three lists.
    pub fn new(
        definitions: Vec<PageDefinition>,
        instances: Vec<PageInstance>,
        variants: Vec<PageVariant>,
    ) -> Self {
        Self {
            definitions,
            instances,
            variants,
            preserved: Preserved::default(),
        }
    }

    /// The definitions, in document order.
    pub fn definitions(&self) -> &[PageDefinition] {
        &self.definitions
    }

    /// The placed instances, in document order.
    pub fn instances(&self) -> &[PageInstance] {
        &self.instances
    }

    /// The variants, in document order.
    pub fn variants(&self) -> &[PageVariant] {
        &self.variants
    }

    /// Checks the graph, the instance arguments/fills and the variant targets.
    pub fn check(&self, nodes: &[PageNode]) -> Result<(), String> {
        let mut names = std::collections::BTreeSet::new();
        for definition in &self.definitions {
            if definition.name.trim().is_empty() {
                return Err("a definition name is empty".to_owned());
            }
            if !names.insert(definition.name.as_str()) {
                return Err(format!("definition {:?} appears twice", definition.name));
            }
            let mut parameters = std::collections::BTreeSet::new();
            for parameter in &definition.parameters {
                if parameter.name.trim().is_empty() {
                    return Err(format!(
                        "definition {:?} has an empty parameter",
                        definition.name
                    ));
                }
                if !parameters.insert(parameter.name.as_str()) {
                    return Err(format!(
                        "definition {:?} declares parameter {:?} twice",
                        definition.name, parameter.name
                    ));
                }
            }
            let mut slots = std::collections::BTreeSet::new();
            for slot in &definition.slots {
                if slot.trim().is_empty() {
                    return Err(format!(
                        "definition {:?} has an empty slot",
                        definition.name
                    ));
                }
                if !slots.insert(slot.as_str()) {
                    return Err(format!(
                        "definition {:?} declares slot {slot:?} twice",
                        definition.name
                    ));
                }
            }
            for part in &definition.body {
                if let PagePart::Slot(slot) = part
                    && !slots.contains(slot.as_str())
                {
                    return Err(format!(
                        "definition {:?} uses undeclared slot {slot:?}",
                        definition.name
                    ));
                }
            }
        }
        for definition in &self.definitions {
            for part in &definition.body {
                if let PagePart::Nested(name) = part
                    && !names.contains(name.as_str())
                {
                    return Err(format!(
                        "definition {:?} nests {name:?}, which is not declared",
                        definition.name
                    ));
                }
            }
        }
        if let Some(cycle) = nested_cycle(&self.definitions) {
            return Err(format!("the definition graph contains a cycle: {cycle}"));
        }
        let mut instance_nodes = std::collections::BTreeSet::new();
        for instance in &self.instances {
            let Some(definition) = self
                .definitions
                .iter()
                .find(|definition| definition.name == instance.definition)
            else {
                return Err(format!(
                    "instance of {:?} has no definition",
                    instance.definition
                ));
            };
            if !nodes.iter().any(|node| node.id == instance.node) {
                return Err(format!(
                    "instance names node {}, which the page does not declare",
                    instance.node.get()
                ));
            }
            if !instance_nodes.insert(instance.node) {
                return Err(format!("node {} is two instances", instance.node.get()));
            }
            for name in instance.arguments.keys() {
                if !definition.parameters.iter().any(|p| p.name == *name) {
                    return Err(format!(
                        "instance of {:?} has no parameter {name:?}",
                        instance.definition
                    ));
                }
            }
            for fill in &instance.fills {
                if !definition.slots.contains(&fill.slot) {
                    return Err(format!(
                        "instance of {:?} fills undeclared slot {:?}",
                        instance.definition, fill.slot
                    ));
                }
            }
        }
        for variant in &self.variants {
            let instance = self
                .instances
                .iter()
                .find(|instance| instance.node == variant.instance)
                .ok_or_else(|| {
                    format!(
                        "variant names node {}, which is not an instance",
                        variant.instance.get()
                    )
                })?;
            if instance.definition != variant.definition {
                return Err(format!(
                    "variant of node {} names definition {:?}, but the instance is of {:?}",
                    variant.instance.get(),
                    variant.definition,
                    instance.definition
                ));
            }
            if variant.kind.as_str().trim().is_empty() {
                return Err("a variant has an empty kind".to_owned());
            }
            let mut targets = std::collections::BTreeSet::new();
            for target in &variant.targets {
                if target.trim().is_empty() {
                    return Err("a variant target is empty".to_owned());
                }
                if !targets.insert(target.as_str()) {
                    return Err(format!("a variant lists target {target:?} twice"));
                }
            }
            for name in variant.overrides.keys() {
                if name.as_str().trim().is_empty() {
                    return Err("a variant override has an empty name".to_owned());
                }
            }
        }
        Ok(())
    }
}

/// The first cycle in the definition graph, as `a -> b -> a`, or `None`.
fn nested_cycle(definitions: &[PageDefinition]) -> Option<String> {
    fn visit(
        name: &str,
        definitions: &[PageDefinition],
        visiting: &mut Vec<String>,
        done: &mut std::collections::BTreeSet<String>,
    ) -> Option<String> {
        if visiting.iter().any(|seen| seen == name) {
            let mut path = visiting.clone();
            path.push(name.to_owned());
            return Some(path.join(" -> "));
        }
        if done.contains(name) {
            return None;
        }
        let definition = definitions.iter().find(|d| d.name == name)?;
        visiting.push(name.to_owned());
        for part in &definition.body {
            if let PagePart::Nested(nested) = part
                && let Some(cycle) = visit(nested, definitions, visiting, done)
            {
                return Some(cycle);
            }
        }
        visiting.pop();
        done.insert(name.to_owned());
        None
    }
    let mut done = std::collections::BTreeSet::new();
    for definition in definitions {
        let mut visiting = Vec::new();
        if let Some(cycle) = visit(&definition.name, definitions, &mut visiting, &mut done) {
            return Some(cycle);
        }
    }
    None
}

/// Whether a motion is a transition or an animation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PageEffect {
    /// Runs once, on a state change.
    Transition,
    /// Runs forward, possibly more than once.
    Animation,
}

impl PageEffect {
    /// The name the file writes.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Transition => "transition",
            Self::Animation => "animation",
        }
    }

    /// Reads an effect's name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "transition" => Some(Self::Transition),
            "animation" => Some(Self::Animation),
            _ => None,
        }
    }
}

/// How a motion's value moves over time.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PageEasing {
    /// Constant speed.
    Linear,
    /// Slow start.
    EaseIn,
    /// Slow end.
    EaseOut,
    /// Slow start and end.
    EaseInOut,
}

impl PageEasing {
    /// The name the file writes.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Linear => "linear",
            Self::EaseIn => "ease-in",
            Self::EaseOut => "ease-out",
            Self::EaseInOut => "ease-in-out",
        }
    }

    /// Reads an easing's name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "linear" => Some(Self::Linear),
            "ease-in" => Some(Self::EaseIn),
            "ease-out" => Some(Self::EaseOut),
            "ease-in-out" => Some(Self::EaseInOut),
            _ => None,
        }
    }
}

/// One transition or animation on one property.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PageMotion {
    effect: PageEffect,
    property: String,
    duration_ms: u32,
    delay_ms: u32,
    easing: PageEasing,
    preserved: Preserved,
}

impl PageMotion {
    /// A motion with its time parameters.
    pub fn new(
        effect: PageEffect,
        property: impl Into<String>,
        duration_ms: u32,
        delay_ms: u32,
        easing: PageEasing,
    ) -> Self {
        Self {
            effect,
            property: property.into(),
            duration_ms,
            delay_ms,
            easing,
            preserved: Preserved::default(),
        }
    }

    /// Whether it is a transition or an animation.
    pub const fn effect(&self) -> PageEffect {
        self.effect
    }

    /// The property it moves.
    pub fn property(&self) -> &str {
        &self.property
    }

    /// How long it runs.
    pub const fn duration_ms(&self) -> u32 {
        self.duration_ms
    }

    /// How long before it starts.
    pub const fn delay_ms(&self) -> u32 {
        self.delay_ms
    }

    /// How it moves.
    pub const fn easing(&self) -> PageEasing {
        self.easing
    }

    /// The delay plus duration, saturating.
    pub const fn total_ms(&self) -> u32 {
        self.delay_ms.saturating_add(self.duration_ms)
    }
}

/// A page's transitions and animations (format 5).
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct PageMotions {
    motions: Vec<PageMotion>,
    preserved: Preserved,
}

impl PageMotions {
    /// A motion section from its declaration list.
    pub fn new(motions: Vec<PageMotion>) -> Self {
        Self {
            motions,
            preserved: Preserved::default(),
        }
    }

    /// The motions, in document order.
    pub fn motions(&self) -> &[PageMotion] {
        &self.motions
    }

    /// The longest delay plus duration, or zero when there is none.
    pub fn duration_ms(&self) -> u32 {
        self.motions
            .iter()
            .map(PageMotion::total_ms)
            .max()
            .unwrap_or(0)
    }

    /// Checks that every motion names a property and a known easing.
    pub fn check(&self) -> Result<(), String> {
        for motion in &self.motions {
            if motion.property.trim().is_empty() {
                return Err("a motion names no property".to_owned());
            }
        }
        Ok(())
    }
}

/// A container's layout declaration, kept in the library's own shape.
///
/// `type` is the only member this build interprets (it must be a non-blank token); every other
/// member is kept verbatim and copied into the document the generator consumes. That is deliberate:
/// the parameter schema belongs to the library (`json::LAYOUT_KIND_SCHEMAS`), and a second copy
/// here would be a second answer to "is `spacing` a valid parameter". A parameter the library does
/// not know comes back in `GenerationReport::layout_issues`, which the layout panel shows.
///
/// `children` is refused: the page's tree is the node's `children` array, and a second one inside
/// the layout object would let one node have two child lists.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PageLayout {
    kind: String,
    params: Vec<(String, Json)>,
}

impl PageLayout {
    /// A declaration of one layout kind with its parameters, in the order given.
    pub fn new(kind: impl Into<String>, params: impl IntoIterator<Item = (String, Json)>) -> Self {
        Self {
            kind: kind.into(),
            params: params.into_iter().collect(),
        }
    }

    /// The layout kind token (`flex`, `grid`, `border`, …), as the library spells it.
    pub fn kind(&self) -> &str {
        &self.kind
    }

    /// The parameters the declaration carries, in the order they were written.
    pub fn params(&self) -> &[(String, Json)] {
        &self.params
    }

    /// The declaration in one line, for a panel row.
    pub fn render(&self) -> String {
        let mut out = format!("type={}", self.kind);
        for (name, value) in &self.params {
            out.push_str(&format!(", {name}={}", value.render_compact()));
        }
        out
    }

    /// The declaration as the JSON the library document carries.
    pub fn to_json(&self) -> Json {
        let mut out = JsonObject::new();
        out.insert("type", Json::string(&self.kind));
        for (name, value) in &self.params {
            out.insert(name.as_str(), value.clone());
        }
        Json::Object(out)
    }

    /// Reads a declaration, keeping every parameter verbatim.
    fn from_json(path: &str, value: &Json) -> Result<Self, ProjectError> {
        let Some(object) = value.as_object() else {
            return Err(ProjectError::WrongType {
                path: path.to_owned(),
                expected: "an object with a layout `type`",
                found: value.kind(),
            });
        };
        let mut kind: Option<String> = None;
        let mut params = Vec::new();
        for (name, member) in object.iter() {
            if name == "type" {
                let Some(text) = member.as_str() else {
                    return Err(ProjectError::WrongType {
                        path: format!("{path}.type"),
                        expected: "a layout kind token",
                        found: member.kind(),
                    });
                };
                if text.trim().is_empty() {
                    return Err(ProjectError::EmptyValue {
                        path: format!("{path}.type"),
                    });
                }
                kind = Some(text.to_owned());
                continue;
            }
            if name == "children" {
                return Err(ProjectError::Inconsistent {
                    path: format!("{path}.children"),
                    reason: "the node's `children` array is the tree; a layout object cannot hold \
                             a second one"
                        .to_owned(),
                });
            }
            params.push((name.to_owned(), member.clone()));
        }
        let Some(kind) = kind else {
            return Err(ProjectError::MissingMember {
                path: format!("{path}.type"),
            });
        };
        Ok(Self { kind, params })
    }
}

/// One node's size, margin and overflow constraints (page format 7, upstream RW-109).
///
/// The members are kept in the order the document writes them and copied verbatim into the
/// document generation consumes, where the library reads them from the node's attribute object as
/// `ChildLayoutAttrs`. This module validates the *shape* of the eight known members; whether a
/// layout kind honours one is the library's answer, reported in `GenerationReport::layout_issues`
/// as a dropped field (for example `overflow`, which has no runtime carrier yet).
///
/// A member this build does not know is kept too: a newer build's constraint reaches the library,
/// which names it, instead of being silently dropped by an older reader (the document's
/// unknown-member policy).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PageConstraints {
    members: Vec<(String, Json)>,
}

impl PageConstraints {
    /// Constraints from their declarations, in the order given.
    ///
    /// The caller is responsible for the values being the shape the library reads; use
    /// [`Self::from_json`] for a declaration read from a file.
    pub fn new(members: impl IntoIterator<Item = (String, Json)>) -> Self {
        Self {
            members: members.into_iter().collect(),
        }
    }

    /// The declarations, in the order they were written.
    pub fn members(&self) -> &[(String, Json)] {
        &self.members
    }

    /// One declared value by name.
    pub fn get(&self, name: &str) -> Option<&Json> {
        self.members
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value)
    }

    /// Whether nothing is declared.
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// The declaration in one line, for a panel row.
    pub fn render(&self) -> String {
        self.members
            .iter()
            .map(|(name, value)| format!("{name}={}", value.render_compact()))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// The constraints as the object the page document carries.
    pub fn to_json(&self) -> Json {
        let mut out = JsonObject::new();
        for (name, value) in &self.members {
            out.insert(name.as_str(), value.clone());
        }
        Json::Object(out)
    }

    /// Reads a declaration, keeping every member verbatim after checking the shape of the ones
    /// this build knows.
    fn from_json(path: &str, value: &Json) -> Result<Self, ProjectError> {
        let Some(object) = value.as_object() else {
            return Err(ProjectError::WrongType {
                path: path.to_owned(),
                expected: "an object of node constraints",
                found: value.kind(),
            });
        };
        let mut members = Vec::with_capacity(object.len());
        for (name, member) in object.iter() {
            if name == "children" {
                return Err(ProjectError::Inconsistent {
                    path: format!("{path}.children"),
                    reason: "the node's `children` array is the tree; a constraints object cannot \
                             hold a second one"
                        .to_owned(),
                });
            }
            check_constraint(&format!("{path}.{name}"), name, member)?;
            members.push((name.to_owned(), member.clone()));
        }
        Ok(Self { members })
    }
}

/// Refuses a known constraint whose value the library could not read.
///
/// An unknown name is not refused: the library reports it as an unknown child attribute during
/// generation, which is the one place that knows the complete attribute vocabulary (principle 12).
fn check_constraint(path: &str, name: &str, value: &Json) -> Result<(), ProjectError> {
    match name {
        "min_width" | "min_height" | "pref_width" | "pref_height" | "max_width" | "max_height" => {
            let Some(raw) = value.as_u64() else {
                return Err(ProjectError::WrongType {
                    path: path.to_owned(),
                    expected: "a non-negative whole number of logical pixels",
                    found: value.kind(),
                });
            };
            if u32::try_from(raw).is_err() {
                return Err(ProjectError::Unrepresentable {
                    path: path.to_owned(),
                    reason: format!("`{raw}` is larger than a logical-pixel extent can be"),
                });
            }
        }
        "margin" => {
            // The library accepts one uniform value, `[vertical, horizontal]` or
            // `[top, right, bottom, left]`; any other length is a typo, not a third spelling.
            let mut lengths = Vec::new();
            match value {
                Json::Number(_) => {
                    if value.as_u64().is_none() {
                        return Err(ProjectError::WrongType {
                            path: path.to_owned(),
                            expected: "a non-negative whole number of logical pixels",
                            found: value.kind(),
                        });
                    }
                }
                Json::Array(items) => {
                    lengths.push(items.len());
                    for (index, item) in items.iter().enumerate() {
                        if item.as_u64().is_none() {
                            return Err(ProjectError::WrongType {
                                path: format!("{path}[{index}]"),
                                expected: "a non-negative whole number of logical pixels",
                                found: item.kind(),
                            });
                        }
                    }
                }
                _ => {
                    return Err(ProjectError::WrongType {
                        path: path.to_owned(),
                        expected: "a whole number or a list of 1, 2 or 4 whole numbers",
                        found: value.kind(),
                    });
                }
            }
            if let Some(length) = lengths.pop()
                && !matches!(length, 1 | 2 | 4)
            {
                return Err(ProjectError::Inconsistent {
                    path: path.to_owned(),
                    reason: format!(
                        "a margin list has 1, 2 or 4 values, not {length}; the library would \
                         refuse this rather than reinterpret it"
                    ),
                });
            }
        }
        "overflow" => {
            let Some(token) = value.as_str() else {
                return Err(ProjectError::WrongType {
                    path: path.to_owned(),
                    expected: "`clip`, `visible` or `scroll`",
                    found: value.kind(),
                });
            };
            if !matches!(token, "clip" | "visible" | "scroll") {
                return Err(ProjectError::Inconsistent {
                    path: path.to_owned(),
                    reason: format!(
                        "`{token}` is not an overflow policy; the library reads `clip`, \
                         `visible` or `scroll`"
                    ),
                });
            }
        }
        _ => {}
    }
    Ok(())
}

/// One node of a page document, as the file holds it.
///
/// Deliberately not [`crate::contracts::model::Node`]: that type's parent and children links are
/// written only by the store's apply path, which is what keeps "one writable model" true. A file
/// carries the tree once and the open buffer builds the model, with its parent and child links,
/// from it ([`Buffer::model`](crate::project::document::buffer::Buffer::model)); the vocabulary a node uses — kinds, property names, property
/// values — *is* the model's, so the two cannot describe different things.
#[derive(Clone, PartialEq, Debug)]
pub struct PageNode {
    id: NodeId,
    kind: NodeKind,
    properties: BTreeMap<PropertyName, PropertyValue>,
    children: Vec<NodeId>,
    layout: Option<PageLayout>,
    constraints: Option<PageConstraints>,
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
            layout: None,
            constraints: None,
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

    /// The same node with a container layout declaration.
    pub fn with_layout(mut self, layout: PageLayout) -> Self {
        self.layout = Some(layout);
        self
    }

    /// The same node with size, margin and overflow constraints.
    pub fn with_constraints(mut self, constraints: PageConstraints) -> Self {
        self.constraints = Some(constraints);
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

    /// The container layout this node declares, when it declares one.
    pub fn layout(&self) -> Option<&PageLayout> {
        self.layout.as_ref()
    }

    /// The size, margin and overflow constraints this node declares (format 7).
    pub fn constraints(&self) -> Option<&PageConstraints> {
        self.constraints.as_ref()
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
    /// The page's explicit state machine, when it declares one (format 2 and later).
    states: Option<PageStates>,
    /// The page's collection contents (format 3 and later).
    collections: Vec<PageCollection>,
    /// The page's component definitions, instances and variants (format 4 and later).
    components: PageComponents,
    /// The page's transitions and animations (format 5 and later).
    motions: PageMotions,
    references: Vec<Reference>,
    /// What interpretation left over.
    preserved: Preserved,
    unknown: Vec<UnknownField>,
    /// The bytes this was read from, for the conflict check a save performs.
    original: Option<Vec<u8>>,
    /// The version this was read as, when it is older than the one this build writes.
    migrated_from: Option<FormatVersion>,
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
            && self.states == other.states
            && self.collections == other.collections
            && self.components == other.components
            && self.motions == other.motions
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
            states: None,
            collections: Vec::new(),
            components: PageComponents::default(),
            motions: PageMotions::default(),
            references: Vec::new(),
            preserved: Preserved::default(),
            unknown: Vec::new(),
            original: None,
            migrated_from: None,
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
        let mut document = interpret(root, path, &value)?;
        document.original = Some(text.as_bytes().to_vec());
        Ok(document)
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

    /// The page's explicit state machine, when it declares one.
    pub fn states(&self) -> Option<&PageStates> {
        self.states.as_ref()
    }

    /// The page's collection contents, in document order.
    pub fn collections(&self) -> &[PageCollection] {
        &self.collections
    }

    /// The page's component definitions, instances and variants.
    pub fn components(&self) -> &PageComponents {
        &self.components
    }

    /// The page's transitions and animations.
    pub fn motions(&self) -> &PageMotions {
        &self.motions
    }

    /// The members this build writes but the generator does not consume yet.
    ///
    /// Generation maps nodes and their scalar properties; a page that also carries a state
    /// machine, collection content, components or motion cannot be generated as if those were
    /// absent. The caller refuses generation and names these members instead of dropping them
    /// (principles #13/#22).
    pub fn unconsumed_by_generator(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.states.is_some() {
            out.push("states");
        }
        if !self.collections.is_empty() {
            out.push("collections");
        }
        if !self.components.definitions().is_empty()
            || !self.components.instances().is_empty()
            || !self.components.variants().is_empty()
        {
            out.push("components");
        }
        if !self.motions.motions().is_empty() {
            out.push("motion");
        }
        out
    }

    /// The version this was read as, when it is older than the one this build writes.
    pub const fn migrated_from(&self) -> Option<FormatVersion> {
        self.migrated_from
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
        match &self.states {
            Some(states) => {
                out.insert("states", Json::Object(encode_states(states)));
            }
            None => {
                out.insert("states", Json::Null);
            }
        }
        out.insert(
            "collections",
            Json::array(
                self.collections
                    .iter()
                    .map(encode_collection)
                    .collect::<Vec<_>>(),
            ),
        );
        out.insert("components", encode_components(&self.components)?);
        out.insert("motion", encode_motions(&self.motions));
        self.preserved.adopt(&mut out);
        Ok(Json::Object(out))
    }

    /// The document as text.
    pub fn render(&self) -> Result<String, ProjectError> {
        Ok(self.to_json()?.render())
    }

    /// Writes this document back through the host's guarded save path.
    ///
    /// The save refuses when the file changed after it was read, keeps the bytes that were read when
    /// the document is a version 1 file being migrated to version 2, and writes atomically. The
    /// backup is a copy of the file that was read, named `<file>.v<n>.bak`, so a migration never
    /// destroys what it came from (decision `Q6`).
    pub fn save(&self) -> Result<PageWrite, String> {
        use crate::project::document::save::{Saving, Verb};

        let Some(root) = &self.root else {
            return Err("the document was not read from a project root".to_owned());
        };
        let relative = self.path.strip_prefix(root.as_path()).map_err(|_| {
            format!(
                "{} is not inside the project root {}",
                self.path.display(),
                root.as_path().display()
            )
        })?;
        let relative = relative
            .to_str()
            .ok_or_else(|| format!("{} is not a path this build can name", relative.display()))?;
        let text = self.render().map_err(|error| error.message())?;
        let backup = match (self.migrated_from, &self.original) {
            (Some(from), Some(original)) => {
                let backup = self.path.with_extension(format!("v{}.bak", from.get()));
                std::fs::write(&backup, original).map_err(|error| {
                    format!(
                        "{} could not keep the file that was read: {error}",
                        backup.display()
                    )
                })?;
                Some(backup)
            }
            _ => None,
        };
        let saving =
            Saving::plan_guarded(root, relative, text, Verb::Save, self.original.as_deref())
                .map_err(|problem| problem.message())?;
        let committed = saving.execute().map_err(|failure| failure.message())?;
        Ok(PageWrite {
            target: committed.target().to_path_buf(),
            backup,
            migrated_from: self.migrated_from,
        })
    }
}

/// What a page-document save wrote, and what it kept.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PageWrite {
    target: PathBuf,
    backup: Option<PathBuf>,
    migrated_from: Option<FormatVersion>,
}

impl PageWrite {
    /// The file that was written.
    pub fn target(&self) -> &Path {
        &self.target
    }

    /// The copy of the file that was read, when this write migrated it.
    pub fn backup(&self) -> Option<&Path> {
        self.backup.as_deref()
    }

    /// The version the document was read as, when it was older than the written one.
    pub const fn migrated_from(&self) -> Option<FormatVersion> {
        self.migrated_from
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
    // Written even when absent, like every other member of this format: a page has one spelling.
    out.insert(
        "layout",
        node.layout
            .as_ref()
            .map(PageLayout::to_json)
            .unwrap_or(Json::Null),
    );
    out.insert(
        "constraints",
        node.constraints
            .as_ref()
            .map(PageConstraints::to_json)
            .unwrap_or(Json::Null),
    );
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

/// Encodes a state machine in the member order the format defines.
fn encode_states(states: &PageStates) -> JsonObject {
    let mut out = JsonObject::new();
    out.insert(
        "declared",
        Json::array(
            states
                .declared
                .iter()
                .map(|name| Json::string(name.clone()))
                .collect::<Vec<_>>(),
        ),
    );
    out.insert(
        "initial",
        match &states.initial {
            Some(initial) => Json::string(initial.clone()),
            None => Json::Null,
        },
    );
    out.insert(
        "final",
        Json::array(
            states
                .finals
                .iter()
                .map(|name| Json::string(name.clone()))
                .collect::<Vec<_>>(),
        ),
    );
    out.insert(
        "transitions",
        Json::array(
            states
                .transitions
                .iter()
                .map(|transition| {
                    let mut entry = JsonObject::new();
                    entry.insert("from", Json::string(transition.from.clone()));
                    entry.insert("on", Json::string(transition.on.clone()));
                    entry.insert("to", Json::string(transition.to.clone()));
                    entry.insert(
                        "guard",
                        match &transition.guard {
                            Some(guard) => Json::string(guard.clone()),
                            None => Json::Null,
                        },
                    );
                    transition.preserved.adopt(&mut entry);
                    Json::Object(entry)
                })
                .collect::<Vec<_>>(),
        ),
    );
    states.preserved.adopt(&mut out);
    out
}

/// Encodes a collection in the member order the format defines.
fn encode_collection(collection: &PageCollection) -> Json {
    let mut out = JsonObject::new();
    out.insert("node", Json::number(collection.node.get()));
    out.insert("origin", Json::string(collection.origin.name()));
    out.insert(
        "columns",
        Json::array(
            collection
                .columns
                .iter()
                .map(|column| {
                    let mut entry = JsonObject::new();
                    entry.insert("id", Json::number(column.id));
                    entry.insert("header", Json::string(column.header.clone()));
                    entry.insert("role", Json::string(encode_role(&column.role)));
                    column.preserved.adopt(&mut entry);
                    Json::Object(entry)
                })
                .collect::<Vec<_>>(),
        ),
    );
    out.insert(
        "rows",
        Json::array(
            collection
                .rows
                .iter()
                .map(|row| {
                    let mut entry = JsonObject::new();
                    entry.insert("id", Json::number(row.id));
                    entry.insert(
                        "cells",
                        Json::array(
                            row.cells
                                .iter()
                                .map(|cell| {
                                    let mut entry = JsonObject::new();
                                    entry.insert("column", Json::number(cell.column));
                                    entry.insert("value", Json::string(cell.value.clone()));
                                    cell.preserved.adopt(&mut entry);
                                    Json::Object(entry)
                                })
                                .collect::<Vec<_>>(),
                        ),
                    );
                    row.preserved.adopt(&mut entry);
                    Json::Object(entry)
                })
                .collect::<Vec<_>>(),
        ),
    );
    out.insert(
        "source",
        match &collection.source {
            Some(source) => {
                let mut entry = JsonObject::new();
                entry.insert("adapter", Json::string(source.adapter.clone()));
                entry.insert(
                    "provides",
                    Json::array(
                        source
                            .provides
                            .iter()
                            .map(|header| Json::string(header.clone()))
                            .collect::<Vec<_>>(),
                    ),
                );
                entry.insert("availability", Json::string(source.availability.name()));
                source.preserved.adopt(&mut entry);
                Json::Object(entry)
            }
            None => Json::Null,
        },
    );
    collection.preserved.adopt(&mut out);
    Json::Object(out)
}

/// A role as one string: `text`, `check`, `badge:<label>` or `reference:<row>`.
fn encode_role(role: &PageCellRole) -> String {
    match role {
        PageCellRole::Text => "text".to_owned(),
        PageCellRole::Check => "check".to_owned(),
        PageCellRole::Badge(label) => format!("badge:{label}"),
        PageCellRole::Reference(row) => format!("reference:{row}"),
    }
}

fn decode_role(text: &str, path: &str) -> Result<PageCellRole, ProjectError> {
    match text {
        "text" => Ok(PageCellRole::Text),
        "check" => Ok(PageCellRole::Check),
        other => match other.split_once(':') {
            Some(("badge", label)) if !label.is_empty() => {
                Ok(PageCellRole::Badge(label.to_owned()))
            }
            Some(("reference", id)) => {
                id.parse::<u64>().map(PageCellRole::Reference).map_err(|_| {
                    ProjectError::Inconsistent {
                        path: path.to_owned(),
                        reason: format!("{id:?} is not a row identity"),
                    }
                })
            }
            _ => Err(ProjectError::Inconsistent {
                path: path.to_owned(),
                reason: format!(
                    "{other:?} is not a cell role; use text, check, badge:<label> or reference:<row>"
                ),
            }),
        },
    }
}

/// The optional `collections` member: one entry per collection control.
fn decode_collections(
    reader: &mut Reader<'_>,
    nodes: &[PageNode],
) -> Result<Vec<PageCollection>, ProjectError> {
    let mut out = Vec::new();
    for mut entry in sub_objects(reader, "collections")? {
        let path = entry.path().to_owned();
        let node = NodeId::from_raw(entry.required_u64("node")?);
        if !nodes.iter().any(|declared| declared.id == node) {
            return Err(ProjectError::Inconsistent {
                path,
                reason: format!(
                    "the collection names node {}, which the page does not declare",
                    node.get()
                ),
            });
        }
        let origin_text = entry.required_string("origin")?;
        let origin = PageOrigin::parse(&origin_text).ok_or(ProjectError::Inconsistent {
            path: entry.member_path("origin"),
            reason: format!("{origin_text:?} is not sample or source"),
        })?;
        let mut columns = Vec::new();
        for column in sub_objects(&mut entry, "columns")? {
            let ColumnParts {
                id,
                header,
                role,
                preserved,
            } = decode_column(column)?;
            columns.push(PageColumn {
                id,
                header,
                role,
                preserved,
            });
        }
        let mut rows = Vec::new();
        for row in sub_objects(&mut entry, "rows")? {
            let RowParts {
                id,
                cells,
                preserved,
            } = decode_row(row)?;
            rows.push(PageRow {
                id,
                cells,
                preserved,
            });
        }
        let source = match entry.take("source") {
            None => None,
            Some(value) if matches!(value.kind(), JsonKind::Null) => None,
            Some(value) => {
                let mut source = Reader::object(entry.member_path("source"), value)?;
                let adapter = source.required_string("adapter")?;
                let provides = source.take_string_array("provides")?;
                let availability_text = source.required_string("availability")?;
                let availability = PageAvailability::parse(&availability_text).ok_or(
                    ProjectError::Inconsistent {
                        path: source.member_path("availability"),
                        reason: format!("{availability_text:?} is not online or offline"),
                    },
                )?;
                Some(PageSource {
                    adapter,
                    provides,
                    availability,
                    preserved: source.finish(),
                })
            }
        };
        let preserved = entry.finish();
        let collection = PageCollection {
            node,
            origin,
            columns,
            rows,
            source,
            preserved,
        };
        collection
            .check()
            .map_err(|reason| ProjectError::Inconsistent { path, reason })?;
        out.push(collection);
    }
    Ok(out)
}

struct ColumnParts {
    id: u64,
    header: String,
    role: PageCellRole,
    preserved: Preserved,
}

fn decode_column(mut reader: Reader<'_>) -> Result<ColumnParts, ProjectError> {
    let id = reader.required_u64("id")?;
    let header = reader.required_string("header")?;
    let role_text = reader.required_string("role")?;
    let role = decode_role(&role_text, &reader.member_path("role"))?;
    Ok(ColumnParts {
        id,
        header,
        role,
        preserved: reader.finish(),
    })
}

struct RowParts {
    id: u64,
    cells: Vec<PageCell>,
    preserved: Preserved,
}

fn decode_row(mut reader: Reader<'_>) -> Result<RowParts, ProjectError> {
    let id = reader.required_u64("id")?;
    let mut cells = Vec::new();
    for mut cell in sub_objects(&mut reader, "cells")? {
        let column = cell.required_u64("column")?;
        let value = cell.required_string("value")?;
        cells.push(PageCell {
            column,
            value,
            preserved: cell.finish(),
        });
    }
    Ok(RowParts {
        id,
        cells,
        preserved: reader.finish(),
    })
}

/// Encodes a property map as the page format writes node properties.
fn encode_properties(
    path: &str,
    properties: &BTreeMap<PropertyName, PropertyValue>,
) -> Result<Vec<Json>, ProjectError> {
    let mut out = Vec::with_capacity(properties.len());
    for (name, value) in properties {
        let mut entry = JsonObject::new();
        entry.insert("name", Json::string(name.as_str()));
        entry.insert(
            "value",
            encode_property_value(&format!("{path}.{}", name.as_str()), value)?,
        );
        out.push(Json::Object(entry));
    }
    Ok(out)
}

fn decode_properties(
    reader: &mut Reader<'_>,
    name: &str,
) -> Result<BTreeMap<PropertyName, PropertyValue>, ProjectError> {
    let path = reader.member_path(name);
    let mut out = BTreeMap::new();
    let Some(values) = reader.take_array(name)? else {
        return Ok(out);
    };
    for (index, value) in values.iter().enumerate() {
        let entry_path = format!("{path}[{index}]");
        let mut entry = Reader::object(entry_path.clone(), value)?;
        let name = entry.required_string("name")?;
        let name = PropertyName::new(&name).ok_or_else(|| ProjectError::Inconsistent {
            path: format!("{entry_path}.name"),
            reason: "a property name is not blank".to_owned(),
        })?;
        let Some(raw) = entry.take("value") else {
            return Err(ProjectError::MissingMember {
                path: format!("{entry_path}.value"),
            });
        };
        let decoded = decode_property_value(&format!("{entry_path}.value"), raw)?;
        if !entry.finish().is_empty() {
            return Err(ProjectError::Inconsistent {
                path: entry_path.clone(),
                reason: "a property entry is a name and a value, and nothing else".to_owned(),
            });
        }
        if out.insert(name.clone(), decoded).is_some() {
            return Err(ProjectError::Duplicate {
                path: entry_path,
                claim: format!("the property `{}`", name.as_str()),
            });
        }
    }
    Ok(out)
}

fn encode_part(path: &str, part: &PagePart) -> Result<Json, ProjectError> {
    let mut out = JsonObject::new();
    match part {
        PagePart::Control { kind, properties } => {
            out.insert("control", Json::string(kind.as_str()));
            out.insert(
                "properties",
                Json::array(encode_properties(path, properties)?),
            );
        }
        PagePart::Nested(name) => {
            out.insert("nested", Json::string(name.clone()));
        }
        PagePart::Slot(name) => {
            out.insert("slot", Json::string(name.clone()));
        }
    }
    Ok(Json::Object(out))
}

fn encode_parts(path: &str, parts: &[PagePart]) -> Result<Vec<Json>, ProjectError> {
    parts
        .iter()
        .enumerate()
        .map(|(index, part)| encode_part(&format!("{path}[{index}]"), part))
        .collect()
}

fn decode_part(mut entry: Reader<'_>) -> Result<PagePart, ProjectError> {
    let path = entry.path().to_owned();
    let control = entry.take_string("control")?;
    let nested = entry.take_string("nested")?;
    let slot = entry.take_string("slot")?;
    let named = [control.is_some(), nested.is_some(), slot.is_some()]
        .into_iter()
        .filter(|present| *present)
        .count();
    if named != 1 {
        return Err(ProjectError::Inconsistent {
            path,
            reason: "a body part is exactly one of control, nested or slot".to_owned(),
        });
    }
    if let Some(kind) = control {
        let kind = NodeKind::new(&kind).ok_or_else(|| ProjectError::Inconsistent {
            path: entry.member_path("control"),
            reason: "a control kind is one non-blank word".to_owned(),
        })?;
        let properties = decode_properties(&mut entry, "properties")?;
        entry.finish();
        return Ok(PagePart::Control { kind, properties });
    }
    entry.finish();
    if let Some(nested) = nested {
        return Ok(PagePart::Nested(nested));
    }
    Ok(PagePart::Slot(slot.unwrap_or_default()))
}

fn encode_components(components: &PageComponents) -> Result<Json, ProjectError> {
    let mut out = JsonObject::new();

    let mut definitions = Vec::new();
    for definition in &components.definitions {
        let mut entry = JsonObject::new();
        entry.insert("name", Json::string(definition.name.clone()));
        entry.insert("version", Json::number(u64::from(definition.version)));
        let mut parameters = Vec::new();
        for parameter in &definition.parameters {
            let mut item = JsonObject::new();
            item.insert("name", Json::string(parameter.name.clone()));
            item.insert(
                "default",
                match &parameter.default {
                    Some(value) => encode_property_value(
                        &format!("components.parameters.{}.default", parameter.name),
                        value,
                    )?,
                    None => Json::Null,
                },
            );
            parameter.preserved.adopt(&mut item);
            parameters.push(Json::Object(item));
        }
        entry.insert("parameters", Json::array(parameters));
        entry.insert(
            "slots",
            Json::array(
                definition
                    .slots
                    .iter()
                    .map(|slot| Json::string(slot.clone()))
                    .collect::<Vec<_>>(),
            ),
        );
        entry.insert(
            "body",
            Json::array(encode_parts(
                &format!("components.definitions.{}.body", definition.name),
                &definition.body,
            )?),
        );
        definition.preserved.adopt(&mut entry);
        definitions.push(Json::Object(entry));
    }
    out.insert("definitions", Json::array(definitions));

    let mut instances = Vec::new();
    for instance in &components.instances {
        let mut entry = JsonObject::new();
        entry.insert("definition", Json::string(instance.definition.clone()));
        entry.insert("node", Json::number(instance.node.get()));
        let mut arguments = Vec::new();
        for (name, value) in &instance.arguments {
            let mut item = JsonObject::new();
            item.insert("name", Json::string(name.clone()));
            item.insert(
                "value",
                encode_property_value(&format!("components.arguments.{name}"), value)?,
            );
            arguments.push(Json::Object(item));
        }
        entry.insert("arguments", Json::array(arguments));
        let mut fills = Vec::new();
        for fill in &instance.fills {
            let mut item = JsonObject::new();
            item.insert("slot", Json::string(fill.slot.clone()));
            item.insert(
                "parts",
                Json::array(encode_parts(
                    &format!("components.fills.{}", fill.slot),
                    &fill.parts,
                )?),
            );
            fill.preserved.adopt(&mut item);
            fills.push(Json::Object(item));
        }
        entry.insert("fills", Json::array(fills));
        instance.preserved.adopt(&mut entry);
        instances.push(Json::Object(entry));
    }
    out.insert("instances", Json::array(instances));

    let mut variants = Vec::new();
    for variant in &components.variants {
        let mut entry = JsonObject::new();
        entry.insert("instance", Json::number(variant.instance.get()));
        entry.insert("definition", Json::string(variant.definition.clone()));
        entry.insert("kind", Json::string(variant.kind.as_str()));
        entry.insert(
            "targets",
            Json::array(
                variant
                    .targets
                    .iter()
                    .map(|target| Json::string(target.clone()))
                    .collect::<Vec<_>>(),
            ),
        );
        entry.insert(
            "overrides",
            Json::array(encode_properties(
                "components.overrides",
                &variant.overrides,
            )?),
        );
        variant.preserved.adopt(&mut entry);
        variants.push(Json::Object(entry));
    }
    out.insert("variants", Json::array(variants));

    components.preserved.adopt(&mut out);
    Ok(Json::Object(out))
}
fn decode_parts(reader: &mut Reader<'_>, name: &str) -> Result<Vec<PagePart>, ProjectError> {
    let mut out = Vec::new();
    for entry in sub_objects(reader, name)? {
        out.push(decode_part(entry)?);
    }
    Ok(out)
}

fn decode_components(
    reader: &mut Reader<'_>,
    nodes: &[PageNode],
) -> Result<PageComponents, ProjectError> {
    let path = reader.member_path("components");
    let Some(value) = reader.take("components") else {
        return Ok(PageComponents::default());
    };
    if matches!(value.kind(), JsonKind::Null) {
        return Ok(PageComponents::default());
    }
    let mut entry = Reader::object(path.clone(), value)?;
    let mut definitions = Vec::new();
    for mut definition in sub_objects(&mut entry, "definitions")? {
        let name = definition.required_string("name")?;
        let version = u32::try_from(definition.required_u64("version")?).map_err(|_| {
            ProjectError::Inconsistent {
                path: definition.member_path("version"),
                reason: "a version is between 0 and 4294967295".to_owned(),
            }
        })?;
        let mut parameters = Vec::new();
        for mut parameter in sub_objects(&mut definition, "parameters")? {
            let name = parameter.required_string("name")?;
            let default = match parameter.take("default") {
                None => None,
                Some(value) if matches!(value.kind(), JsonKind::Null) => None,
                Some(value) => Some(decode_property_value(
                    &parameter.member_path("default"),
                    value,
                )?),
            };
            parameters.push(PageParameter {
                name,
                default,
                preserved: parameter.finish(),
            });
        }
        let slots = definition.take_string_array("slots")?;
        let body = decode_parts(&mut definition, "body")?;
        definitions.push(PageDefinition {
            name,
            version,
            parameters,
            slots,
            body,
            preserved: definition.finish(),
        });
    }
    let mut instances = Vec::new();
    for mut instance in sub_objects(&mut entry, "instances")? {
        let definition = instance.required_string("definition")?;
        let node = NodeId::from_raw(instance.required_u64("node")?);
        let arguments = decode_named_values(&mut instance, "arguments")?;
        let mut fills = Vec::new();
        for mut fill in sub_objects(&mut instance, "fills")? {
            let slot = fill.required_string("slot")?;
            let parts = decode_parts(&mut fill, "parts")?;
            fills.push(PageFill {
                slot,
                parts,
                preserved: fill.finish(),
            });
        }
        instances.push(PageInstance {
            definition,
            node,
            arguments,
            fills,
            preserved: instance.finish(),
        });
    }
    let mut variants = Vec::new();
    for mut variant in sub_objects(&mut entry, "variants")? {
        let instance = NodeId::from_raw(variant.required_u64("instance")?);
        let definition = variant.required_string("definition")?;
        let kind_text = variant.required_string("kind")?;
        let kind = NodeKind::new(&kind_text).ok_or_else(|| ProjectError::Inconsistent {
            path: variant.member_path("kind"),
            reason: "a kind is one non-blank word".to_owned(),
        })?;
        let targets = variant.take_string_array("targets")?;
        let overrides = decode_properties(&mut variant, "overrides")?;
        variants.push(PageVariant {
            instance,
            definition,
            kind,
            targets,
            overrides,
            preserved: variant.finish(),
        });
    }
    let components = PageComponents {
        definitions,
        instances,
        variants,
        preserved: entry.finish(),
    };
    components
        .check(nodes)
        .map_err(|reason| ProjectError::Inconsistent { path, reason })?;
    Ok(components)
}

fn decode_named_values(
    reader: &mut Reader<'_>,
    name: &str,
) -> Result<BTreeMap<String, PropertyValue>, ProjectError> {
    let path = reader.member_path(name);
    let mut out = BTreeMap::new();
    for mut entry in sub_objects(reader, name)? {
        let name = entry.required_string("name")?;
        let Some(raw) = entry.take("value") else {
            return Err(ProjectError::MissingMember {
                path: format!("{path}.{name}.value"),
            });
        };
        let decoded = decode_property_value(&format!("{path}.{name}"), raw)?;
        entry.finish();
        if out.insert(name.clone(), decoded).is_some() {
            return Err(ProjectError::Duplicate {
                path: path.clone(),
                claim: format!("the argument `{name}`"),
            });
        }
    }
    Ok(out)
}

fn encode_motions(motions: &PageMotions) -> Json {
    let mut out = JsonObject::new();
    out.insert(
        "motions",
        Json::array(
            motions
                .motions
                .iter()
                .map(|motion| {
                    let mut entry = JsonObject::new();
                    entry.insert("effect", Json::string(motion.effect.name()));
                    entry.insert("property", Json::string(motion.property.clone()));
                    entry.insert("duration_ms", Json::number(u64::from(motion.duration_ms)));
                    entry.insert("delay_ms", Json::number(u64::from(motion.delay_ms)));
                    entry.insert("easing", Json::string(motion.easing.name()));
                    motion.preserved.adopt(&mut entry);
                    Json::Object(entry)
                })
                .collect::<Vec<_>>(),
        ),
    );
    motions.preserved.adopt(&mut out);
    Json::Object(out)
}

fn decode_motions(reader: &mut Reader<'_>) -> Result<PageMotions, ProjectError> {
    let path = reader.member_path("motion");
    let Some(value) = reader.take("motion") else {
        return Ok(PageMotions::default());
    };
    if matches!(value.kind(), JsonKind::Null) {
        return Ok(PageMotions::default());
    }
    let mut entry = Reader::object(path.clone(), value)?;
    let mut motions = Vec::new();
    for mut motion in sub_objects(&mut entry, "motions")? {
        let effect_text = motion.required_string("effect")?;
        let effect = PageEffect::parse(&effect_text).ok_or_else(|| ProjectError::Inconsistent {
            path: motion.member_path("effect"),
            reason: format!("{effect_text:?} is not transition or animation"),
        })?;
        let property = motion.required_string("property")?;
        let duration = u32::try_from(motion.required_u64("duration_ms")?).map_err(|_| {
            ProjectError::Inconsistent {
                path: motion.member_path("duration_ms"),
                reason: "a duration is between 0 and 4294967295".to_owned(),
            }
        })?;
        let delay = u32::try_from(motion.required_u64("delay_ms")?).map_err(|_| {
            ProjectError::Inconsistent {
                path: motion.member_path("delay_ms"),
                reason: "a delay is between 0 and 4294967295".to_owned(),
            }
        })?;
        let easing_text = motion.required_string("easing")?;
        let easing = PageEasing::parse(&easing_text).ok_or_else(|| ProjectError::Inconsistent {
            path: motion.member_path("easing"),
            reason: format!("{easing_text:?} is not an easing"),
        })?;
        motions.push(PageMotion {
            effect,
            property,
            duration_ms: duration,
            delay_ms: delay,
            easing,
            preserved: motion.finish(),
        });
    }
    let motions = PageMotions {
        motions,
        preserved: entry.finish(),
    };
    motions
        .check()
        .map_err(|reason| ProjectError::Inconsistent { path, reason })?;
    Ok(motions)
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
            if version < PAGE_OLDEST_READABLE {
                return Err(ProjectError::OlderFormat {
                    found: version,
                    current: PAGE_FORMAT,
                });
            }
        }
    }
    let migrated_from = (version < PAGE_FORMAT).then_some(version);

    let mut reader = Reader::object(String::new(), value)?;
    let _ = reader.take("format");
    let (page, page_preserved) = decode_page(&mut reader)?;
    let mut nodes = decode_nodes(&mut reader, version)?;
    let states = if version.get() >= 2 {
        decode_states(&mut reader)?
    } else {
        None
    };
    let collections = if version.get() >= 3 {
        decode_collections(&mut reader, &nodes)?
    } else {
        Vec::new()
    };
    let components = if version.get() >= 4 {
        decode_components(&mut reader, &nodes)?
    } else {
        PageComponents::default()
    };
    let motions = if version.get() >= 5 {
        decode_motions(&mut reader)?
    } else {
        PageMotions::default()
    };
    let preserved = reader.finish();
    nodes.sort_by_key(|node| node.id.get());

    check_tree(&page, &nodes)?;
    // The library solves one container per document, from the root node's declaration. A layout on
    // any other node would be read by this build and ignored by generation, which is the second
    // source of truth the layout panel must not create.
    for node in &nodes {
        if node.layout.is_some() && node.id != page.root() {
            return Err(ProjectError::Inconsistent {
                path: format!("nodes[{}].layout", node.id.get()),
                reason: "a layout declaration belongs to the page's root node; the library solves \
                         one container per document"
                    .to_owned(),
            });
        }
    }
    // Constraints describe how a parent places a node. The root is placed by the generation
    // request's client size, so a constraint on it would be read by this build and ignored by the
    // solve — the silent drop the format refuses rather than a second placement rule.
    for node in &nodes {
        if node.constraints.is_some() && node.id == page.root() {
            return Err(ProjectError::Inconsistent {
                path: format!("nodes[{}].constraints", node.id.get()),
                reason: "size constraints describe how a parent places a node; the root is placed \
                         by the generation request's size"
                    .to_owned(),
            });
        }
    }

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
    if let Some(states) = &states {
        states.preserved.names_at("states", &mut unknown);
        for (index, transition) in states.transitions.iter().enumerate() {
            transition
                .preserved
                .names_at(&format!("states.transitions[{index}]"), &mut unknown);
        }
    }
    for (index, collection) in collections.iter().enumerate() {
        collection
            .preserved
            .names_at(&format!("collections[{index}]"), &mut unknown);
        for (column_index, column) in collection.columns.iter().enumerate() {
            column.preserved.names_at(
                &format!("collections[{index}].columns[{column_index}]"),
                &mut unknown,
            );
        }
        for (row_index, row) in collection.rows.iter().enumerate() {
            row.preserved.names_at(
                &format!("collections[{index}].rows[{row_index}]"),
                &mut unknown,
            );
            for (cell_index, cell) in row.cells.iter().enumerate() {
                cell.preserved.names_at(
                    &format!("collections[{index}].rows[{row_index}].cells[{cell_index}]"),
                    &mut unknown,
                );
            }
        }
        if let Some(source) = &collection.source {
            source
                .preserved
                .names_at(&format!("collections[{index}].source"), &mut unknown);
        }
    }

    components.preserved.names_at("components", &mut unknown);
    components.preserved.names_at("components", &mut unknown);
    motions.preserved.names_at("motion", &mut unknown);
    for (index, motion) in motions.motions.iter().enumerate() {
        motion
            .preserved
            .names_at(&format!("motion.motions[{index}]"), &mut unknown);
    }

    Ok(PageDocument {
        path,
        root: Some(root.clone()),
        page,
        page_preserved,
        nodes,
        states,
        collections,
        components,
        motions,
        references,
        preserved,
        unknown,
        original: None,
        migrated_from,
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

fn decode_nodes(
    reader: &mut Reader<'_>,
    version: FormatVersion,
) -> Result<Vec<PageNode>, ProjectError> {
    let mut nodes = Vec::new();
    for entry in sub_objects(reader, "nodes")? {
        nodes.push(decode_node(entry, version)?);
    }
    Ok(nodes)
}

/// The optional `states` member: an object or null, and what a newer document left in it.
fn decode_states(reader: &mut Reader<'_>) -> Result<Option<PageStates>, ProjectError> {
    let path = reader.member_path("states");
    let Some(value) = reader.take("states") else {
        return Ok(None);
    };
    if matches!(value.kind(), JsonKind::Null) {
        return Ok(None);
    }
    let mut states = Reader::object(path.clone(), value)?;
    let declared = states.take_string_array("declared")?;
    let initial = optional_string(&mut states, "initial")?;
    let finals = states.take_string_array("final")?;
    let mut transitions = Vec::new();
    for mut entry in sub_objects(&mut states, "transitions")? {
        let from = entry.required_string("from")?;
        let on = entry.required_string("on")?;
        let to = entry.required_string("to")?;
        let guard = optional_string(&mut entry, "guard")?;
        let preserved = entry.finish();
        transitions.push(PageTransition {
            from,
            on,
            to,
            guard,
            preserved,
        });
    }
    let preserved = states.finish();
    let machine = PageStates {
        declared,
        initial,
        finals,
        transitions,
        preserved,
    };
    machine
        .check()
        .map_err(|reason| ProjectError::Inconsistent { path, reason })?;
    Ok(Some(machine))
}

/// An optional string that may be written as `null`; a null is the member being absent.
fn optional_string(reader: &mut Reader<'_>, name: &str) -> Result<Option<String>, ProjectError> {
    let path = reader.member_path(name);
    let Some(value) = reader.take(name) else {
        return Ok(None);
    };
    match value.kind() {
        JsonKind::Null => Ok(None),
        JsonKind::String => Ok(Some(value.as_str().unwrap_or_default().to_owned())),
        other => Err(ProjectError::WrongType {
            path,
            expected: "a string or null",
            found: other,
        }),
    }
}

/// One node entry: the page reader's single place that says what a node is.
///
/// Public because a batch of edits (T03.02) writes the same node shape a page document writes, and
/// a second decoder for it would be a second answer to "what is a node".
pub fn node_from_json(at: &str, value: &Json) -> Result<PageNode, ProjectError> {
    // A batch edit writes one node in the shape this build writes, so the current format's members
    // (including `constraints`) are read exactly as the page reader reads them.
    decode_node(Reader::object(at.to_owned(), value)?, PAGE_FORMAT)
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

fn decode_node(mut entry: Reader<'_>, version: FormatVersion) -> Result<PageNode, ProjectError> {
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
        let layout = match entry.take("layout") {
            None | Some(Json::Null) => None,
            Some(value) => Some(PageLayout::from_json(&entry.member_path("layout"), value)?),
        };
        let constraints = if version.get() >= 7 {
            match entry.take("constraints") {
                None | Some(Json::Null) => None,
                Some(value) => Some(PageConstraints::from_json(
                    &entry.member_path("constraints"),
                    value,
                )?),
            }
        } else {
            // A version 6 document does not define this member; leaving it in `preserved` keeps a
            // newer build's content intact instead of reading it under an older version's rules.
            None
        };
        Ok(PageNode {
            id: NodeId::from_raw(id),
            kind,
            properties,
            children,
            layout,
            constraints,
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
    use super::{
        PAGE_DOCUMENT_MEMBERS, PAGE_NODE_MEMBERS, PageDocument, PageNode, PageStates,
        PageTransition,
    };
    use crate::contracts::identity::{NodeId, PageId};
    use crate::contracts::model::{NodeKind, PropertyName, PropertyValue};
    use crate::contracts::revision::FormatVersion;
    use crate::host::paths::ProjectRoot;
    use crate::project::record::format::{ProjectError, ReferenceVerdict};
    use crate::project::record::json::Json;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// The smallest document this build writes, spelled exactly the way the writer spells it.
    const EMPTY: &str = "{\n  \"format\": 7,\n  \"page\": {\n    \"id\": 1,\n    \"name\": \"Main window\",\n    \"root\": 1\n  },\n  \"nodes\": [\n    {\n      \"id\": 1,\n      \"type\": \"page\",\n      \"children\": [],\n      \"properties\": [],\n      \"layout\": null,\n      \"constraints\": null\n    }\n  ],\n  \"states\": null,\n  \"collections\": [],\n  \"components\": {\n    \"definitions\": [],\n    \"instances\": [],\n    \"variants\": []\n  },\n  \"motion\": {\n    \"motions\": []\n  }\n}\n";

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
            "{\"format\": 8, \"page\": {\"id\": 1, \"name\": \"P\", \"root\": 1}, \
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
    fn saving_a_version_one_page_keeps_the_bytes_it_read_and_writes_the_current_version() {
        let sandbox = sandbox();
        std::fs::create_dir_all(sandbox.root.join("pages")).expect("a directory");
        let version_one = "{\"format\": 1, \"page\": {\"id\": 1, \"name\": \"P\", \"root\": 1}, \
             \"nodes\": [{\"id\": 1, \"type\": \"page\"}]}";
        let path = sandbox.root.join("pages/main.json");
        std::fs::write(&path, version_one).expect("the file");
        let document = PageDocument::read(&sandbox.root(), "pages/main.json").expect("readable");
        assert_eq!(document.migrated_from(), Some(FormatVersion::from_raw(1)));

        let written = document.save().expect("the save succeeds");
        assert_eq!(written.migrated_from(), Some(FormatVersion::from_raw(1)));
        let backup = written
            .backup()
            .expect("a migration keeps the file it read");
        assert_eq!(
            std::fs::read_to_string(backup).expect("the backup"),
            version_one
        );
        let saved = std::fs::read_to_string(written.target()).expect("the written page");
        assert!(saved.starts_with("{\n  \"format\": 7,"), "{saved}");
        assert!(saved.contains("\"states\": null"), "{saved}");
    }

    #[test]
    fn a_save_refuses_to_write_over_a_change_someone_else_made() {
        let sandbox = sandbox();
        std::fs::create_dir_all(sandbox.root.join("pages")).expect("a directory");
        let path = sandbox.root.join("pages/main.json");
        std::fs::write(&path, EMPTY).expect("the file");
        let document = PageDocument::read(&sandbox.root(), "pages/main.json").expect("readable");
        std::fs::write(&path, "changed outside the designer").expect("the change");
        let error = document.save().expect_err("a changed file is refused");
        assert!(error.contains("changed"), "{error}");
    }

    #[test]
    fn a_collection_round_trips_with_its_columns_rows_and_roles() {
        let sandbox = sandbox();
        let text = "{\"format\": 3, \"page\": {\"id\": 1, \"name\": \"P\", \"root\": 1}, \
             \"nodes\": [{\"id\": 1, \"type\": \"page\", \"children\": [2]}, \
             {\"id\": 2, \"type\": \"table\"}], \"states\": null, \
             \"collections\": [{\"node\": 2, \"origin\": \"sample\", \
             \"columns\": [{\"id\": 1, \"header\": \"Name\", \"role\": \"text\"}, \
             {\"id\": 2, \"header\": \"Done\", \"role\": \"check\"}, \
             {\"id\": 3, \"header\": \"Next\", \"role\": \"reference:1\"}], \
             \"rows\": [{\"id\": 1, \"cells\": [{\"column\": 1, \"value\": \"Write\"}, \
             {\"column\": 2, \"value\": \"false\"}]}], \"source\": null}]}";
        let document = sandbox.document(text);
        assert_eq!(document.collections().len(), 1);
        let collection = &document.collections()[0];
        assert_eq!(collection.node().get(), 2);
        assert_eq!(collection.origin(), super::PageOrigin::Sample);
        assert_eq!(collection.columns().len(), 3);
        assert_eq!(collection.columns()[0].header(), "Name");
        assert_eq!(
            collection.columns()[2].role(),
            &super::PageCellRole::Reference(1)
        );
        assert_eq!(collection.rows().len(), 1);
        assert_eq!(collection.rows()[0].cells()[0].value(), "Write");
        assert!(collection.source().is_none());

        let saved = document.render().expect("writable");
        let again = sandbox.document(&saved);
        assert_eq!(again, document);
        assert_eq!(again.render().expect("writable"), saved);
    }

    #[test]
    fn a_source_collection_carries_its_contract() {
        let sandbox = sandbox();
        let text = "{\"format\": 3, \"page\": {\"id\": 1, \"name\": \"P\", \"root\": 1}, \
             \"nodes\": [{\"id\": 1, \"type\": \"page\", \"children\": [2]}, \
             {\"id\": 2, \"type\": \"table\"}], \"states\": null, \
             \"collections\": [{\"node\": 2, \"origin\": \"source\", \"columns\": [], \
             \"rows\": [], \"source\": {\"adapter\": \"api\", \"provides\": [\"Name\"], \
             \"availability\": \"offline\"}}]}";
        let document = sandbox.document(text);
        let source = document.collections()[0]
            .source()
            .expect("a source contract");
        assert_eq!(source.adapter(), "api");
        assert_eq!(source.provides(), &["Name".to_owned()]);
        assert_eq!(source.availability(), super::PageAvailability::Offline);
    }

    #[test]
    fn an_invalid_collection_is_refused_with_the_reason() {
        let sandbox = sandbox();
        let page = |collection: &str| {
            format!(
                "{{\"format\": 3, \"page\": {{\"id\": 1, \"name\": \"P\", \"root\": 1}}, \
                 \"nodes\": [{{\"id\": 1, \"type\": \"page\", \"children\": [2]}}, \
                 {{\"id\": 2, \"type\": \"table\"}}], \"states\": null, \
                 \"collections\": [{collection}]}}"
            )
        };
        for (collection, reason) in [
            (
                "{\"node\": 9, \"origin\": \"sample\", \"columns\": [], \"rows\": []}",
                "does not declare",
            ),
            (
                "{\"node\": 2, \"origin\": \"sample\", \"columns\": [{\"id\": 1, \"header\": \"A\", \"role\": \"text\"}, {\"id\": 1, \"header\": \"B\", \"role\": \"text\"}], \"rows\": []}",
                "appears twice",
            ),
            (
                "{\"node\": 2, \"origin\": \"sample\", \"columns\": [{\"id\": 1, \"header\": \"A\", \"role\": \"text\"}], \"rows\": [{\"id\": 1, \"cells\": [{\"column\": 9, \"value\": \"x\"}]}]}",
                "unknown column",
            ),
            (
                "{\"node\": 2, \"origin\": \"sample\", \"columns\": [{\"id\": 1, \"header\": \"A\", \"role\": \"reference:9\"}], \"rows\": []}",
                "references unknown row",
            ),
            (
                "{\"node\": 2, \"origin\": \"sample\", \"columns\": [], \"rows\": [], \"source\": {\"adapter\": \"api\", \"provides\": [], \"availability\": \"online\"}}",
                "sample collection",
            ),
            (
                "{\"node\": 2, \"origin\": \"source\", \"columns\": [], \"rows\": []}",
                "needs a source contract",
            ),
            (
                "{\"node\": 2, \"origin\": \"source\", \"columns\": [], \"rows\": [], \"source\": {\"adapter\": \"\", \"provides\": [], \"availability\": \"online\"}}",
                "adapter is empty",
            ),
        ] {
            let error = sandbox.failure(&page(collection));
            assert!(
                error.message().contains(reason),
                "{collection}: expected {reason:?}, found {}",
                error.message()
            );
        }
    }

    #[test]
    fn a_component_graph_round_trips_with_instances_and_variants() {
        let sandbox = sandbox();
        let text = "{\"format\": 4, \"page\": {\"id\": 1, \"name\": \"P\", \"root\": 1}, \
             \"nodes\": [{\"id\": 1, \"type\": \"page\", \"children\": [2]}, \
             {\"id\": 2, \"type\": \"panel\"}], \"states\": null, \"collections\": [], \
             \"components\": {\"definitions\": [
               {\"name\": \"Card\", \"version\": 2, \
                \"parameters\": [{\"name\": \"title\", \"default\": {\"text\": \"Hi\"}}], \
                \"slots\": [\"body\"], \
                \"body\": [{\"control\": \"panel\", \"properties\": [{\"name\": \"title\", \"value\": {\"text\": \"Card\"}}]}, \
                {\"nested\": \"Badge\"}, {\"slot\": \"body\"}]},
               {\"name\": \"Badge\", \"version\": 1, \"parameters\": [], \"slots\": [], \
                \"body\": [{\"control\": \"label\", \"properties\": []}]}
             ], \"instances\": [{\"definition\": \"Card\", \"node\": 2, \
               \"arguments\": [{\"name\": \"title\", \"value\": {\"text\": \"Save\"}}], \
               \"fills\": [{\"slot\": \"body\", \"parts\": [{\"control\": \"label\", \"properties\": []}]}]}], \
             \"variants\": [{\"instance\": 2, \"definition\": \"Card\", \"kind\": \"panel\", \
               \"targets\": [\"desktop\"], \"overrides\": [{\"name\": \"title\", \"value\": {\"text\": \"Save\"}}]}]}}";
        let document = sandbox.document(text);
        let components = document.components();
        assert_eq!(components.definitions.len(), 2);
        assert_eq!(components.definitions[0].name(), "Card");
        assert_eq!(components.definitions[0].version(), 2);
        assert_eq!(components.definitions[0].parameters().len(), 1);
        assert_eq!(components.definitions[0].slots(), &["body"]);
        assert_eq!(components.definitions[0].body().len(), 3);
        assert_eq!(components.instances.len(), 1);
        assert_eq!(components.instances[0].definition(), "Card");
        assert_eq!(components.instances[0].node().get(), 2);
        assert_eq!(components.instances[0].fills().len(), 1);
        assert_eq!(components.variants.len(), 1);
        assert_eq!(components.variants[0].targets(), &["desktop"]);
        assert_eq!(components.variants[0].overrides().len(), 1);

        let saved = document.render().expect("writable");
        let again = sandbox.document(&saved);
        assert_eq!(again, document);
        assert_eq!(again.render().expect("writable"), saved);
    }

    #[test]
    fn an_invalid_component_graph_is_refused_with_the_reason() {
        let sandbox = sandbox();
        let page = |components: &str| {
            format!(
                "{{\"format\": 4, \"page\": {{\"id\": 1, \"name\": \"P\", \"root\": 1}}, \
                 \"nodes\": [{{\"id\": 1, \"type\": \"page\", \"children\": [2]}}, \
                 {{\"id\": 2, \"type\": \"panel\"}}], \"states\": null, \"collections\": [], \
                 \"components\": {components}}}"
            )
        };
        for (components, reason) in [
            (
                "{\"definitions\": [{\"name\": \"A\", \"version\": 1, \"parameters\": [], \"slots\": [], \"body\": [{\"nested\": \"B\"}]}], \"instances\": [], \"variants\": []}",
                "not declared",
            ),
            (
                "{\"definitions\": [{\"name\": \"A\", \"version\": 1, \"parameters\": [], \"slots\": [], \"body\": [{\"nested\": \"B\"}]}, {\"name\": \"B\", \"version\": 1, \"parameters\": [], \"slots\": [], \"body\": [{\"nested\": \"A\"}]}], \"instances\": [], \"variants\": []}",
                "cycle",
            ),
            (
                "{\"definitions\": [], \"instances\": [{\"definition\": \"Card\", \"node\": 2, \"arguments\": [], \"fills\": []}], \"variants\": []}",
                "no definition",
            ),
            (
                "{\"definitions\": [{\"name\": \"Card\", \"version\": 1, \"parameters\": [], \"slots\": [], \"body\": []}], \"instances\": [{\"definition\": \"Card\", \"node\": 2, \"arguments\": [{\"name\": \"x\", \"value\": {\"text\": \"y\"}}], \"fills\": []}], \"variants\": []}",
                "no parameter",
            ),
            (
                "{\"definitions\": [{\"name\": \"Card\", \"version\": 1, \"parameters\": [], \"slots\": [], \"body\": []}], \"instances\": [{\"definition\": \"Card\", \"node\": 2, \"arguments\": [], \"fills\": [{\"slot\": \"body\", \"parts\": []}]}], \"variants\": []}",
                "undeclared slot",
            ),
            (
                "{\"definitions\": [], \"instances\": [], \"variants\": [{\"instance\": 2, \"definition\": \"Card\", \"kind\": \"panel\", \"targets\": [], \"overrides\": []}]}",
                "not an instance",
            ),
        ] {
            let error = sandbox.failure(&page(components));
            assert!(
                error.message().contains(reason),
                "{components}: expected {reason:?}, found {}",
                error.message()
            );
        }
    }

    #[test]
    fn a_motion_timeline_round_trips_with_its_timing() {
        let sandbox = sandbox();
        let text = "{\"format\": 5, \"page\": {\"id\": 1, \"name\": \"P\", \"root\": 1}, \
             \"nodes\": [{\"id\": 1, \"type\": \"page\"}], \"states\": null, \"collections\": [], \
             \"components\": {\"definitions\": [], \"instances\": [], \"variants\": []}, \
             \"motion\": {\"motions\": [
               {\"effect\": \"transition\", \"property\": \"color\", \"duration_ms\": 200, \"delay_ms\": 0, \"easing\": \"ease-out\"},
               {\"effect\": \"animation\", \"property\": \"opacity\", \"duration_ms\": 400, \"delay_ms\": 100, \"easing\": \"linear\"}
             ]}}";
        let document = sandbox.document(text);
        let motions = document.motions();
        assert_eq!(motions.motions().len(), 2);
        assert_eq!(motions.duration_ms(), 500);
        assert_eq!(motions.motions()[0].effect(), super::PageEffect::Transition);
        assert_eq!(motions.motions()[0].property(), "color");
        assert_eq!(motions.motions()[0].duration_ms(), 200);
        assert_eq!(motions.motions()[0].easing(), super::PageEasing::EaseOut);
        assert_eq!(motions.motions()[1].total_ms(), 500);

        let saved = document.render().expect("writable");
        let again = sandbox.document(&saved);
        assert_eq!(again, document);
        assert_eq!(again.render().expect("writable"), saved);
    }

    #[test]
    fn an_invalid_motion_is_refused_with_the_reason() {
        let sandbox = sandbox();
        let page = |motion: &str| {
            format!(
                "{{\"format\": 5, \"page\": {{\"id\": 1, \"name\": \"P\", \"root\": 1}}, \
                 \"nodes\": [{{\"id\": 1, \"type\": \"page\"}}], \"states\": null, \"collections\": [], \
                 \"components\": {{\"definitions\": [], \"instances\": [], \"variants\": []}}, \"motion\": {motion}}}"
            )
        };
        for (motion, reason) in [
            (
                "{\"motions\": [{\"effect\": \"transition\", \"property\": \"\", \"duration_ms\": 1, \"delay_ms\": 0, \"easing\": \"linear\"}]}",
                "property is empty",
            ),
            (
                "{\"motions\": [{\"effect\": \"bounce\", \"property\": \"color\", \"duration_ms\": 1, \"delay_ms\": 0, \"easing\": \"linear\"}]}",
                "not transition or animation",
            ),
            (
                "{\"motions\": [{\"effect\": \"transition\", \"property\": \"color\", \"duration_ms\": 1, \"delay_ms\": 0, \"easing\": \"spring\"}]}",
                "not an easing",
            ),
        ] {
            let error = sandbox.failure(&page(motion));
            assert!(
                error.message().contains(reason),
                "{motion}: expected {reason:?}, found {}",
                error.message()
            );
        }
    }

    #[test]
    fn a_root_layout_declaration_round_trips_verbatim() {
        let sandbox = sandbox();
        let text = "{\"format\": 6, \"page\": {\"id\": 1, \"name\": \"P\", \"root\": 1}, \
             \"nodes\": [{\"id\": 1, \"type\": \"page\", \"layout\": \
               {\"type\": \"vbox\", \"spacing\": 8, \"margin\": 4, \"columns\": 2}}], \
             \"states\": null, \"collections\": [], \
             \"components\": {\"definitions\": [], \"instances\": [], \"variants\": []}, \
             \"motion\": {\"motions\": []}}";
        let document = sandbox.document(text);
        let layout = document
            .root_node()
            .expect("a root")
            .layout()
            .expect("a declaration");
        assert_eq!(layout.kind(), "vbox");
        assert_eq!(layout.params().len(), 3);
        assert_eq!(layout.render(), "type=vbox, spacing=8, margin=4, columns=2");

        let saved = document.render().expect("writable");
        let again = sandbox.document(&saved);
        assert_eq!(again, document);
        assert_eq!(again.render().expect("writable"), saved);
    }

    #[test]
    fn an_invalid_layout_declaration_is_refused_with_the_reason() {
        let sandbox = sandbox();
        let page = |node: &str| {
            format!(
                "{{\"format\": 6, \"page\": {{\"id\": 1, \"name\": \"P\", \"root\": 1}}, \
                 \"nodes\": [{node}], \"states\": null, \"collections\": [], \
                 \"components\": {{\"definitions\": [], \"instances\": [], \"variants\": []}}, \
                 \"motion\": {{\"motions\": []}}}}"
            )
        };
        for (node, reason) in [
            (
                "{\"id\": 1, \"type\": \"page\", \"layout\": {\"spacing\": 8}}",
                "type",
            ),
            (
                "{\"id\": 1, \"type\": \"page\", \"layout\": {\"type\": \"  \"}}",
                "is empty",
            ),
            (
                "{\"id\": 1, \"type\": \"page\", \"layout\": {\"type\": 7}}",
                "a layout kind token",
            ),
            (
                "{\"id\": 1, \"type\": \"page\", \"layout\": {\"type\": \"vbox\", \"children\": []}}",
                "second one",
            ),
            (
                "{\"id\": 1, \"type\": \"page\", \"layout\": [\"vbox\"]}",
                "an object with a layout",
            ),
        ] {
            let error = sandbox.failure(&page(node));
            assert!(
                error.message().contains(reason),
                "{node}: expected {reason:?}, found {}",
                error.message()
            );
        }
    }

    #[test]
    fn a_layout_on_a_non_root_node_is_refused_by_name() {
        let sandbox = sandbox();
        let text = "{\"format\": 6, \"page\": {\"id\": 1, \"name\": \"P\", \"root\": 1}, \
             \"nodes\": [{\"id\": 1, \"type\": \"page\", \"children\": [2]}, \
               {\"id\": 2, \"type\": \"panel\", \"layout\": {\"type\": \"vbox\"}}], \
             \"states\": null, \"collections\": [], \
             \"components\": {\"definitions\": [], \"instances\": [], \"variants\": []}, \
             \"motion\": {\"motions\": []}}";
        let error = sandbox.failure(text);
        assert!(
            error.message().contains("root node") && error.message().contains("nodes[2].layout"),
            "{}",
            error.message()
        );
    }

    #[test]
    fn node_constraints_round_trip_verbatim() {
        let sandbox = sandbox();
        let text = "{\"format\": 7, \"page\": {\"id\": 1, \"name\": \"P\", \"root\": 1}, \
             \"nodes\": [{\"id\": 1, \"type\": \"page\", \"children\": [2]}, \
               {\"id\": 2, \"type\": \"button\", \"constraints\": \
                 {\"min_width\": 120, \"pref_height\": 32, \"margin\": [8, 4], \
                  \"overflow\": \"clip\"}}], \
             \"states\": null, \"collections\": [], \
             \"components\": {\"definitions\": [], \"instances\": [], \"variants\": []}, \
             \"motion\": {\"motions\": []}}";
        let document = sandbox.document(text);
        let child = document
            .nodes()
            .iter()
            .find(|node| node.id().get() == 2)
            .expect("the child");
        let constraints = child.constraints().expect("constraints");
        assert_eq!(
            constraints.get("min_width").and_then(Json::as_u64),
            Some(120)
        );
        assert_eq!(
            constraints.get("overflow").and_then(Json::as_str),
            Some("clip")
        );
        assert_eq!(
            constraints.render(),
            "min_width=120, pref_height=32, margin=[8,4], overflow=\"clip\""
        );

        let saved = document.render().expect("writable");
        let again = sandbox.document(&saved);
        assert_eq!(again, document);
        assert_eq!(again.render().expect("writable"), saved);
    }

    #[test]
    fn invalid_constraints_are_refused_with_the_reason() {
        let sandbox = sandbox();
        let page = |node: &str| {
            format!(
                "{{\"format\": 7, \"page\": {{\"id\": 1, \"name\": \"P\", \"root\": 1}}, \
                 \"nodes\": [{{\"id\": 1, \"type\": \"page\", \"children\": [2]}}, {node}], \
                 \"states\": null, \"collections\": [], \
                 \"components\": {{\"definitions\": [], \"instances\": [], \"variants\": []}}, \
                 \"motion\": {{\"motions\": []}}}}"
            )
        };
        for (node, reason) in [
            (
                "{\"id\": 2, \"type\": \"button\", \"constraints\": [8]}",
                "an object of node constraints",
            ),
            (
                "{\"id\": 2, \"type\": \"button\", \"constraints\": {\"min_width\": \"wide\"}}",
                "non-negative whole number",
            ),
            (
                "{\"id\": 2, \"type\": \"button\", \"constraints\": {\"min_width\": 4294967296}}",
                "larger than a logical-pixel extent",
            ),
            (
                "{\"id\": 2, \"type\": \"button\", \"constraints\": {\"margin\": [1, 2, 3]}}",
                "1, 2 or 4 values",
            ),
            (
                "{\"id\": 2, \"type\": \"button\", \"constraints\": {\"margin\": [1, \"x\"]}}",
                "non-negative whole number",
            ),
            (
                "{\"id\": 2, \"type\": \"button\", \"constraints\": {\"overflow\": \"hide\"}}",
                "not an overflow policy",
            ),
            (
                "{\"id\": 2, \"type\": \"button\", \"constraints\": {\"children\": []}}",
                "second one",
            ),
        ] {
            let error = sandbox.failure(&page(node));
            assert!(
                error.message().contains(reason),
                "{node}: expected {reason:?}, found {}",
                error.message()
            );
        }
    }

    #[test]
    fn constraints_on_the_root_are_refused_by_name() {
        let sandbox = sandbox();
        let text = "{\"format\": 7, \"page\": {\"id\": 1, \"name\": \"P\", \"root\": 1}, \
             \"nodes\": [{\"id\": 1, \"type\": \"page\", \"constraints\": {\"min_width\": 100}}], \
             \"states\": null, \"collections\": [], \
             \"components\": {\"definitions\": [], \"instances\": [], \"variants\": []}, \
             \"motion\": {\"motions\": []}}";
        let error = sandbox.failure(text);
        assert!(
            error.message().contains("nodes[1].constraints")
                && error.message().contains("parent places a node"),
            "{}",
            error.message()
        );
    }

    #[test]
    fn the_generator_sees_which_members_it_cannot_consume() {
        let sandbox = sandbox();
        assert!(sandbox.document(EMPTY).unconsumed_by_generator().is_empty());
        let text = "{\"format\": 5, \"page\": {\"id\": 1, \"name\": \"P\", \"root\": 1}, \
             \"nodes\": [{\"id\": 1, \"type\": \"page\", \"children\": [2, 3]}, \
             {\"id\": 2, \"type\": \"table\"}, {\"id\": 3, \"type\": \"panel\"}], \
             \"states\": {\"declared\": [\"idle\"]}, \"collections\": [{\"node\": 2, \"origin\": \"sample\", \"columns\": [], \"rows\": []}], \
             \"components\": {\"definitions\": [{\"name\": \"Card\", \"version\": 1, \"parameters\": [], \"slots\": [], \"body\": []}], \"instances\": [], \"variants\": []}, \
             \"motion\": {\"motions\": [{\"effect\": \"transition\", \"property\": \"color\", \"duration_ms\": 1, \"delay_ms\": 0, \"easing\": \"linear\"}]}}";
        let document = sandbox.document(text);
        assert_eq!(
            document.unconsumed_by_generator(),
            vec!["states", "collections", "components", "motion"]
        );
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

    #[test]
    fn a_state_machine_round_trips_with_its_declaration_order() {
        let sandbox = sandbox();
        let text = "{\"format\": 2, \"page\": {\"id\": 1, \"name\": \"P\", \"root\": 1}, \
             \"nodes\": [{\"id\": 1, \"type\": \"page\"}], \
             \"states\": {\"declared\": [\"idle\", \"running\", \"done\"], \
             \"initial\": \"idle\", \"final\": [\"done\"], \
             \"transitions\": [{\"from\": \"idle\", \"on\": \"start\", \"to\": \"running\"}, \
             {\"from\": \"running\", \"on\": \"finish\", \"to\": \"done\", \"guard\": \"is_complete\"}]}}";
        let document = sandbox.document(text);
        let states = document.states().expect("a machine");
        assert_eq!(states.declared(), &["idle", "running", "done"]);
        assert_eq!(states.initial(), Some("idle"));
        assert_eq!(states.finals(), &["done"]);
        assert_eq!(states.transitions().len(), 2);
        assert_eq!(states.transitions()[0].from(), "idle");
        assert_eq!(states.transitions()[0].on(), "start");
        assert_eq!(states.transitions()[0].to(), "running");
        assert_eq!(states.transitions()[0].guard(), None);
        assert_eq!(states.transitions()[1].guard(), Some("is_complete"));

        let saved = document.render().expect("writable");
        let again = sandbox.document(&saved);
        assert_eq!(again, document);
        assert_eq!(again.render().expect("writable"), saved);
    }

    #[test]
    fn a_version_one_page_is_read_without_a_machine_and_written_as_the_current_version() {
        let sandbox = sandbox();
        let version_one = "{\"format\": 1, \"page\": {\"id\": 1, \"name\": \"P\", \"root\": 1}, \
             \"nodes\": [{\"id\": 1, \"type\": \"page\"}]}";
        let document = sandbox.document(version_one);
        assert!(document.states().is_none());
        assert_eq!(document.migrated_from(), Some(FormatVersion::from_raw(1)));
        let saved = document.render().expect("writable");
        assert!(saved.starts_with("{\n  \"format\": 7,"), "{saved}");
        assert!(saved.contains("\"states\": null"), "{saved}");
        assert!(saved.contains("\"collections\": []"), "{saved}");
        assert!(saved.contains("\"definitions\": []"), "{saved}");
    }

    #[test]
    fn an_invalid_state_machine_is_refused_with_the_reason() {
        let sandbox = sandbox();
        let page = |states: &str| {
            format!(
                "{{\"format\": 2, \"page\": {{\"id\": 1, \"name\": \"P\", \"root\": 1}}, \
                 \"nodes\": [{{\"id\": 1, \"type\": \"page\"}}], \"states\": {states}}}"
            )
        };
        for (states, reason) in [
            ("{\"declared\": []}", "at least one state"),
            (
                "{\"declared\": [\"idle\"], \"initial\": \"missing\"}",
                "initial state",
            ),
            ("{\"declared\": [\"idle\", \"idle\"]}", "declared twice"),
            (
                "{\"declared\": [\"idle\"], \"transitions\": [{\"from\": \"idle\", \"on\": \"go\", \"to\": \"missing\"}]}",
                "enters",
            ),
            (
                "{\"declared\": [\"idle\"], \"transitions\": [{\"from\": \"idle\", \"on\": \"\", \"to\": \"idle\"}]}",
                "on is empty",
            ),
            (
                "{\"declared\": [\"idle\"], \"transitions\": [{\"from\": \"idle\", \"on\": \"go\", \"to\": \"idle\", \"guard\": \"\"}]}",
                "empty guard",
            ),
        ] {
            let error = sandbox.failure(&page(states));
            assert!(
                error.message().contains(reason),
                "{states}: expected {reason:?}, found {}",
                error.message()
            );
        }
    }

    #[test]
    fn unknown_members_inside_the_machine_are_preserved() {
        let sandbox = sandbox();
        let text = "{\"format\": 2, \"page\": {\"id\": 1, \"name\": \"P\", \"root\": 1}, \
             \"nodes\": [{\"id\": 1, \"type\": \"page\"}], \
             \"states\": {\"declared\": [\"idle\"], \"future\": 1, \
             \"transitions\": [{\"from\": \"idle\", \"on\": \"go\", \"to\": \"idle\", \"later\": true}]}}";
        let document = sandbox.document(text);
        let paths: Vec<&str> = document
            .unknown_fields()
            .iter()
            .map(|field| field.path())
            .collect();
        assert!(paths.contains(&"states.future"), "{paths:?}");
        assert!(paths.contains(&"states.transitions[0].later"), "{paths:?}");
        let saved = document.render().expect("writable");
        assert!(saved.contains("\"future\": 1"), "{saved}");
        assert!(saved.contains("\"later\": true"), "{saved}");
        assert_eq!(sandbox.document(&saved), document);
    }

    #[test]
    fn a_machine_built_in_code_is_refused_when_it_is_not_one() {
        let transition = PageTransition::new("idle", "go", "running", None);
        let error = PageStates::new(
            vec!["idle".to_owned()],
            Some("idle".to_owned()),
            Vec::new(),
            vec![transition],
        )
        .expect_err("`running` is not declared");
        assert!(error.contains("enters"), "{error}");
    }
}
