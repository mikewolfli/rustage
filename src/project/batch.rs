// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! A batch of edits to one page's model: Add, Remove, Move, Set and Clear, all committed or all
//! refused (task T03.02).
//!
//! The acceptance line is "a batch of operations is all committed or all refused". That is the
//! store's own contract ([`DocumentStore::apply`] stages every operation and commits once), and what
//! this module adds is everything a person needs to *use* it safely:
//!
//! * **A batch is a document.** [`Batch`] is read from JSON — an intent and the operations in the
//!   order they are meant to happen — so a batch can be reviewed, stored, replayed and compared
//!   instead of living in somebody's head. Its operations use the *model's* vocabulary: a node entry
//!   is read and written by the page reader's own codec
//!   ([`page::node_from_json`](crate::project::page::node_from_json)), so a batch cannot come to mean
//!   something a page document does not.
//! * **A batch is prechecked.** [`Plan::of`] reads the document and reports what the batch would do
//!   *before* anything happens: which operations cannot land and why (an id the document does not
//!   have, an id it retired, the page root, a subtree removed without its children, a move into the
//!   node's own subtree, a placement past the end, a property written twice), and which asset
//!   references leave the project. A blocked plan refuses the whole batch and nothing is attempted.
//! * **A batch that lands says what would undo it.** [`Outcome::inverse`] is the inverse change set
//!   rendered back as a [`Batch`], because an edit that cannot be described as an inverse batch is an
//!   edit nobody can take back.
//! * **The document is built through change sets.** [`store_of`] turns a page document read from a
//!   file into a [`DocumentStore`] by *adopting the file's identities* and then adding every node
//!   through one change set. This is the file → model seam's other half (T03.01 left the page and its
//!   root): a node is never written by a reader, and the ids in the file are the ids in the model.
//!
//! Two rules of the store are worth stating here because a caller will meet them:
//!
//! * A removal **retires** the identity. Undoing a removal by adding the node back under the same id
//!   is refused (`ChangeRejection::RetiredNode`) — deliberately, since "an undone addition does not
//!   come back under the same identity" — so [`Finding::RetiredIdentity`] names that in the plan
//!   rather than letting it look like an accident.
//! * The store only accepts an id it issued, so an `add` step's id is **adopted** by the runner.
//!   That is what lets a batch written by hand (or by another build) name the identity it means.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use super::format::ProjectError;
use super::json::{self, Json, JsonObject};
use super::page::{self, PageDocument, PageNode};
use crate::contracts::change::{ApplyOutcome, ChangeOp, ChangeOrigin, ChangeRejection, ChangeSet};
use crate::contracts::identity::{NodeId, PageId};
use crate::contracts::model::{Node, PropertyName, PropertyValue};
use crate::contracts::permission::PermissionGate;
use crate::contracts::revision::FormatVersion;
use crate::contracts::revision::Revision;
use crate::contracts::store::DocumentStore;
use crate::host::paths::ProjectRoot;

/// The format this build reads and writes a batch as.
pub const BATCH_FORMAT: FormatVersion = FormatVersion::from_raw(1);

/// The members a batch document defines, in the order this build writes them.
pub const BATCH_MEMBERS: [&str; 3] = ["format", "intent", "operations"];

/// The operation kinds a batch document may hold.
pub const STEP_KINDS: [&str; 5] = ["add", "remove", "move", "set", "clear"];

/// Why a batch document is not a batch, or cannot be run against a document.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum BatchRefusal {
    /// The file could not be read.
    Unreadable {
        /// Why.
        reason: String,
    },
    /// The text is not JSON.
    NotJson {
        /// Where and why.
        reason: String,
    },
    /// The text is JSON but not a batch document.
    NotABatch {
        /// What the reader objected to.
        reason: String,
    },
    /// The batch has no operations, so it would move the revision without changing anything.
    NothingToDo,
    /// The page has no root node, so there is no tree to edit.
    NoRoot,
    /// The page document's nodes could not be turned into a model.
    NotAModel {
        /// What was missing or refused.
        reason: String,
    },
}

impl BatchRefusal {
    /// The refusal in one sentence.
    pub fn message(&self) -> String {
        match self {
            Self::Unreadable { reason } => format!("the batch could not be read: {reason}"),
            Self::NotJson { reason } => format!("the batch is not JSON: {reason}"),
            Self::NotABatch { reason } => format!("the text is JSON but not a batch: {reason}"),
            Self::NothingToDo => {
                "the batch has no operations: an empty batch would move the revision without \
                 changing anything"
                    .to_owned()
            }
            Self::NoRoot => {
                "the page document has no root node, so there is no tree to edit".to_owned()
            }
            Self::NotAModel { reason } => format!("the document is not a model: {reason}"),
        }
    }

    fn of_reading(error: &ProjectError) -> Self {
        match error {
            ProjectError::NotJson { error, .. } => Self::NotJson {
                reason: error.to_string(),
            },
            other => Self::NotABatch {
                reason: other.message(),
            },
        }
    }
}

/// One operation of a batch, in the batch's own words.
///
/// An `add` names the identity it means: a batch is written by an editor that allocated the id, or by
/// a person who wrote one down, and the runner adopts it. A `remove` names only the node, and the
/// runner reads its position from the document — writing the position into the batch as well would
/// be a second copy of a fact the document already holds, and the copy is the one that goes stale.
#[derive(Clone, PartialEq, Debug)]
pub enum Step {
    /// Adds a node under a parent at a position.
    Add {
        /// The parent.
        parent: NodeId,
        /// The position among the parent's children.
        index: usize,
        /// The node, as a page document writes one: id, kind, children (none) and properties.
        node: PageNode,
    },
    /// Removes a node that has no children.
    Remove {
        /// The node.
        id: NodeId,
    },
    /// Moves a node to another position.
    Move {
        /// The node.
        id: NodeId,
        /// The new parent.
        to_parent: NodeId,
        /// The new position.
        to_index: usize,
    },
    /// Sets a property.
    Set {
        /// The node.
        id: NodeId,
        /// The property.
        name: PropertyName,
        /// The value.
        value: PropertyValue,
    },
    /// Clears a property.
    Clear {
        /// The node.
        id: NodeId,
        /// The property.
        name: PropertyName,
    },
}

impl Step {
    /// The machine name a batch document uses for this kind.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Add { .. } => "add",
            Self::Remove { .. } => "remove",
            Self::Move { .. } => "move",
            Self::Set { .. } => "set",
            Self::Clear { .. } => "clear",
        }
    }

    /// One line for a report.
    pub fn describe(&self) -> String {
        match self {
            Self::Add {
                parent,
                index,
                node,
            } => format!(
                "add {} #{} under #{} at {}",
                node.kind().as_str(),
                node.id().get(),
                parent.get(),
                index
            ),
            Self::Remove { id } => format!("remove #{}", id.get()),
            Self::Move {
                id,
                to_parent,
                to_index,
            } => format!(
                "move #{} under #{} at {}",
                id.get(),
                to_parent.get(),
                to_index
            ),
            Self::Set { id, name, .. } => format!("set {} on #{}", name.as_str(), id.get()),
            Self::Clear { id, name } => format!("clear {} on #{}", name.as_str(), id.get()),
        }
    }

    /// The node the step is about.
    pub fn target(&self) -> NodeId {
        match self {
            Self::Add { parent, .. } => *parent,
            Self::Remove { id } => *id,
            Self::Move { id, .. } => *id,
            Self::Set { id, .. } => *id,
            Self::Clear { id, .. } => *id,
        }
    }

    /// The id this step would make live, when it adds one.
    pub fn mints(&self) -> Option<NodeId> {
        match self {
            Self::Add { node, .. } => Some(node.id()),
            _ => None,
        }
    }

    fn to_json(&self) -> Result<Json, ProjectError> {
        let mut out = JsonObject::new();
        out.insert("kind", Json::string(self.kind()));
        match self {
            Self::Add {
                parent,
                index,
                node,
            } => {
                out.insert("parent", Json::number(parent.get()));
                out.insert("index", Json::number(*index as u64));
                out.insert("node", page::node_to_json(node)?);
            }
            Self::Remove { id } => {
                out.insert("id", Json::number(id.get()));
            }
            Self::Move {
                id,
                to_parent,
                to_index,
            } => {
                out.insert("id", Json::number(id.get()));
                out.insert("to_parent", Json::number(to_parent.get()));
                out.insert("to_index", Json::number(*to_index as u64));
            }
            Self::Set { id, name, value } => {
                out.insert("id", Json::number(id.get()));
                out.insert("name", Json::string(name.as_str()));
                out.insert(
                    "value",
                    page::property_to_json(&format!("operations.{}.value", name.as_str()), value)?,
                );
            }
            Self::Clear { id, name } => {
                out.insert("id", Json::number(id.get()));
                out.insert("name", Json::string(name.as_str()));
            }
        }
        Ok(Json::Object(out))
    }

    fn from_json(index: usize, value: &Json) -> Result<Self, BatchRefusal> {
        let at = format!("operations[{index}]");
        let Some(object) = value.as_object() else {
            return Err(BatchRefusal::NotABatch {
                reason: format!(
                    "`{at}` must be an object, and it is {}",
                    value.kind().name()
                ),
            });
        };
        let kind =
            object
                .get("kind")
                .and_then(Json::as_str)
                .ok_or_else(|| BatchRefusal::NotABatch {
                    reason: format!("`{at}.kind` is missing, and every operation says what it is"),
                })?;
        let id = |member: &str| -> Result<NodeId, BatchRefusal> {
            object
                .get(member)
                .and_then(Json::as_u64)
                .map(NodeId::from_raw)
                .ok_or_else(|| BatchRefusal::NotABatch {
                    reason: format!("`{at}.{member}` must be a whole number"),
                })
        };
        let position = |member: &str| -> Result<usize, BatchRefusal> {
            object
                .get(member)
                .and_then(Json::as_u64)
                .map(|value| value as usize)
                .ok_or_else(|| BatchRefusal::NotABatch {
                    reason: format!("`{at}.{member}` must be a whole number"),
                })
        };
        let name = || -> Result<PropertyName, BatchRefusal> {
            let written = object.get("name").and_then(Json::as_str).ok_or_else(|| {
                BatchRefusal::NotABatch {
                    reason: format!("`{at}.name` must be a string"),
                }
            })?;
            PropertyName::new(written).ok_or_else(|| BatchRefusal::NotABatch {
                reason: format!("`{at}.name` is blank, and a property name is not blank"),
            })
        };
        match kind {
            "add" => {
                let Some(node) = object.get("node") else {
                    return Err(BatchRefusal::NotABatch {
                        reason: format!("`{at}.node` is missing"),
                    });
                };
                let node = page::node_from_json(&format!("{at}.node"), node)
                    .map_err(|error| BatchRefusal::of_reading(&error))?;
                if !node.children().is_empty() {
                    return Err(BatchRefusal::NotABatch {
                        reason: format!(
                            "`{at}.node` has children: a batch adds one node at a position, and a \
                             tree that arrives whole is a page document, not a batch"
                        ),
                    });
                }
                Ok(Self::Add {
                    parent: id("parent")?,
                    index: position("index")?,
                    node,
                })
            }
            "remove" => Ok(Self::Remove { id: id("id")? }),
            "move" => Ok(Self::Move {
                id: id("id")?,
                to_parent: id("to_parent")?,
                to_index: position("to_index")?,
            }),
            "set" => {
                let name = name()?;
                let Some(value) = object.get("value") else {
                    return Err(BatchRefusal::NotABatch {
                        reason: format!(
                            "`{at}.value` is missing, and a set says what to set it to"
                        ),
                    });
                };
                let value = page::property_from_json(&format!("{at}.value"), value)
                    .map_err(|error| BatchRefusal::of_reading(&error))?;
                Ok(Self::Set {
                    id: id("id")?,
                    name,
                    value,
                })
            }
            "clear" => Ok(Self::Clear {
                id: id("id")?,
                name: name()?,
            }),
            other => Err(BatchRefusal::NotABatch {
                reason: format!(
                    "`{at}.kind` is `{other}`, and a batch holds {}",
                    STEP_KINDS.join(", ")
                ),
            }),
        }
    }
}

/// A batch: what it is for, and what it does, in order.
#[derive(Clone, PartialEq, Debug)]
pub struct Batch {
    path: PathBuf,
    intent: String,
    steps: Vec<Step>,
}

impl Batch {
    /// Reads a batch from text that was already obtained.
    pub fn parse(path: impl Into<PathBuf>, text: &str) -> Result<Self, BatchRefusal> {
        let path = path.into();
        let value = json::parse(text).map_err(|error| BatchRefusal::NotJson {
            reason: error.to_string(),
        })?;
        let Some(object) = value.as_object() else {
            return Err(BatchRefusal::NotABatch {
                reason: format!("a batch is an object, and this is {}", value.kind().name()),
            });
        };
        if let Some(version) = object.get("format").and_then(Json::as_u64)
            && version != u64::from(BATCH_FORMAT.get())
        {
            return Err(BatchRefusal::NotABatch {
                reason: format!(
                    "the batch is format v{version} and this build reads v{}",
                    BATCH_FORMAT.get()
                ),
            });
        }
        let intent = object
            .get("intent")
            .and_then(Json::as_str)
            .map(str::trim)
            .filter(|intent| !intent.is_empty())
            .ok_or_else(|| BatchRefusal::NotABatch {
                reason: "`intent` is missing: a batch says what it is for, or nobody can review it"
                    .to_owned(),
            })?
            .to_owned();
        let Some(operations) = object.get("operations").and_then(Json::as_array) else {
            return Err(BatchRefusal::NotABatch {
                reason: "`operations` is missing, and a batch is the operations".to_owned(),
            });
        };
        if operations.is_empty() {
            return Err(BatchRefusal::NothingToDo);
        }
        let mut steps = Vec::with_capacity(operations.len());
        for (index, value) in operations.iter().enumerate() {
            steps.push(Step::from_json(index, value)?);
        }
        Ok(Self {
            path,
            intent,
            steps,
        })
    }

    /// Reads a batch from a file.
    pub fn read(path: impl AsRef<Path>) -> Result<Self, BatchRefusal> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|error| BatchRefusal::Unreadable {
            reason: error.to_string(),
        })?;
        Self::parse(path.to_path_buf(), &text)
    }

    /// The file this batch came from, for reports.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// What the batch is for.
    pub fn intent(&self) -> &str {
        &self.intent
    }

    /// The operations, in the order they happen.
    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    /// The batch as JSON.
    pub fn to_json(&self) -> Result<Json, ProjectError> {
        let mut out = JsonObject::new();
        out.insert("format", Json::number(u64::from(BATCH_FORMAT.get())));
        out.insert("intent", Json::string(&self.intent));
        let mut operations = Vec::with_capacity(self.steps.len());
        for step in &self.steps {
            operations.push(step.to_json()?);
        }
        out.insert("operations", Json::Array(operations));
        Ok(Json::Object(out))
    }

    /// The batch as text, which is what a caller writes down or pipes back in.
    pub fn render(&self) -> Result<String, ProjectError> {
        Ok(self.to_json()?.render())
    }

    /// The batch that undoes a change set, in the reverse order the change set carried.
    ///
    /// The steps are derived from what the operations already hold — a removal carries the node it
    /// took out, a set carries what was there before — so this is a *description* of the same facts
    /// rather than a second computation that could disagree with them.
    pub fn of_change_set(path: impl Into<PathBuf>, intent: &str, set: &ChangeSet) -> Self {
        let steps = set
            .ops()
            .iter()
            .map(step_of_operation)
            .collect::<Vec<Step>>();
        Self {
            path: path.into(),
            intent: intent.to_owned(),
            steps,
        }
    }

    /// One line for the intent, then one per operation.
    pub fn describe(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "batch `{}`: {} operation(s), read from {}",
            self.intent,
            self.steps.len(),
            self.path.display()
        )];
        for (index, step) in self.steps.iter().enumerate() {
            lines.push(format!("  [{}] {}", index, step.describe()));
        }
        lines
    }
}

/// The step that says the same thing as an operation.
fn step_of_operation(op: &ChangeOp) -> Step {
    match op {
        ChangeOp::AddNode {
            parent,
            index,
            node,
        } => {
            let mut written = PageNode::new(node.id(), node.kind().clone());
            for (name, value) in node.properties() {
                written = written.with_property(name.clone(), value.clone());
            }
            Step::Add {
                parent: *parent,
                index: *index,
                node: written,
            }
        }
        ChangeOp::RemoveNode { node, .. } => Step::Remove { id: node.id() },
        ChangeOp::MoveNode {
            id,
            to_parent,
            to_index,
            ..
        } => Step::Move {
            id: *id,
            to_parent: *to_parent,
            to_index: *to_index,
        },
        ChangeOp::SetProperty {
            id, name, value, ..
        } => Step::Set {
            id: *id,
            name: name.clone(),
            value: value.clone(),
        },
        ChangeOp::ClearProperty { id, name, .. } => Step::Clear {
            id: *id,
            name: name.clone(),
        },
    }
}

/// Something the plan found out about a batch before it ran.
///
/// A blocking finding is a reason the whole batch would be refused, said in the batch's own terms
/// rather than as a rejection code; a note is something true and worth knowing that does not stop it.
#[derive(Clone, PartialEq, Debug)]
pub enum Finding {
    /// A step names a node the document does not have.
    UnknownNode {
        /// Which step.
        index: usize,
        /// The node.
        id: NodeId,
    },
    /// A step adds a node under an identity the document already uses.
    IdentityInUse {
        /// Which step.
        index: usize,
        /// The identity.
        id: NodeId,
    },
    /// A step names an identity the document retired: a removal is not undone by putting the same id
    /// back, because ids are never reused.
    RetiredIdentity {
        /// Which step.
        index: usize,
        /// The identity.
        id: NodeId,
    },
    /// A step would remove or move the page's root node.
    ThePageRoot {
        /// Which step.
        index: usize,
        /// The root.
        id: NodeId,
    },
    /// A step removes a node that has a subtree under it: the whole subtree goes, in the same
    /// revision, and this names what went with it.
    SubtreeRemoval {
        /// Which step.
        index: usize,
        /// The node.
        id: NodeId,
        /// The children that would be left behind.
        children: Vec<NodeId>,
    },
    /// A step moves a node inside its own subtree.
    CyclicMove {
        /// Which step.
        index: usize,
        /// The node.
        id: NodeId,
        /// The destination that is inside it.
        parent: NodeId,
    },
    /// A step places a node past the end of its parent's children.
    Placement {
        /// Which step.
        index: usize,
        /// The node.
        id: NodeId,
        /// The parent.
        parent: NodeId,
        /// The position asked for.
        at: usize,
        /// How many children the parent has.
        children: usize,
    },
    /// A step clears a property the document does not have set (or that this batch already cleared).
    NothingToClear {
        /// Which step.
        index: usize,
        /// The node.
        id: NodeId,
        /// The property.
        name: PropertyName,
    },
    /// Two steps in one batch write the same property of the same node.
    WrittenTwice {
        /// The node.
        id: NodeId,
        /// The property.
        name: PropertyName,
        /// The first step.
        first: usize,
        /// The second step.
        second: usize,
    },
    /// A step adds a node under a parent on another page.
    OtherPage {
        /// Which step.
        index: usize,
        /// The node.
        id: NodeId,
        /// The parent's page.
        page: PageId,
    },
    /// A step points an asset property at a path that leaves the project.
    AssetLeavesTheProject {
        /// Which step.
        index: usize,
        /// The path as written.
        written: String,
        /// What the path layer said.
        reason: String,
    },
    /// A step points an asset property at a file that is not there. A note: a designer may be
    /// pointing at a file they are about to add, and refusing that would be this build inventing a
    /// rule about the order somebody works in.
    AssetNotThereYet {
        /// Which step.
        index: usize,
        /// The path as written.
        written: String,
    },
}

impl Finding {
    /// The machine name, for the contract data and for tests.
    pub const fn name(&self) -> &'static str {
        match self {
            Self::UnknownNode { .. } => "unknown-node",
            Self::IdentityInUse { .. } => "identity-in-use",
            Self::RetiredIdentity { .. } => "retired-identity",
            Self::ThePageRoot { .. } => "the-page-root",
            Self::SubtreeRemoval { .. } => "subtree-removal",
            Self::CyclicMove { .. } => "cyclic-move",
            Self::Placement { .. } => "placement-out-of-range",
            Self::NothingToClear { .. } => "nothing-to-clear",
            Self::WrittenTwice { .. } => "written-twice",
            Self::OtherPage { .. } => "other-page",
            Self::AssetLeavesTheProject { .. } => "asset-leaves-the-project",
            Self::AssetNotThereYet { .. } => "asset-not-there-yet",
        }
    }

    /// Whether this finding stops the batch. Only the notes do not.
    pub const fn is_blocking(&self) -> bool {
        match self {
            // A batch is a sequence and the last write wins, so writing a property twice is what the
            // batch asked for; naming the file an asset will live at is how a designer works; and
            // removing a control is understood to remove what is under it.
            Self::AssetNotThereYet { .. }
            | Self::WrittenTwice { .. }
            | Self::SubtreeRemoval { .. } => false,
            // Everything else is a reason the store would refuse the whole batch, said here first.
            _ => true,
        }
    }

    /// What it means, in one sentence, with what to do about it.
    pub fn message(&self) -> String {
        match self {
            Self::UnknownNode { index, id } => {
                format!("operation {index}: the document has no node #{}", id.get())
            }
            Self::IdentityInUse { index, id } => format!(
                "operation {index}: the document already has a node #{}; an addition names an \
                 identity nothing is using",
                id.get()
            ),
            Self::RetiredIdentity { index, id } => format!(
                "operation {index}: #{} was removed from this document, and a removed identity is \
                 never reused — add the node again under a fresh id and point what referenced it at \
                 the new one",
                id.get()
            ),
            Self::ThePageRoot { index, id } => format!(
                "operation {index}: #{} is the page's root, and a page owns its root (replacing one \
                 is a page operation, not a batch step)",
                id.get()
            ),
            Self::SubtreeRemoval {
                index,
                id,
                children,
            } => format!(
                "operation {index}: removing #{} also removes what is under it ({}), children first \
                 and in the same revision",
                id.get(),
                children
                    .iter()
                    .map(|child| format!("#{}", child.get()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::CyclicMove { index, id, parent } => format!(
                "operation {index}: moving #{} under #{} would put it inside itself",
                id.get(),
                parent.get()
            ),
            Self::Placement {
                index,
                id,
                parent,
                at,
                children,
            } => format!(
                "operation {index}: #{} cannot go at position {at} under #{}, which has {children} \
                 child(ren)",
                id.get(),
                parent.get()
            ),
            Self::NothingToClear { index, id, name } => format!(
                "operation {index}: {} on #{} is not set, so there is nothing to clear",
                name.as_str(),
                id.get()
            ),
            Self::WrittenTwice {
                id,
                name,
                first,
                second,
            } => format!(
                "operation {second} writes {} on #{} again (operation {first} already did): the last \
                 one is what the document ends up with",
                name.as_str(),
                id.get()
            ),
            Self::OtherPage { index, id, page } => format!(
                "operation {index}: #{} belongs to page {}, and this batch edits one page",
                id.get(),
                page.get()
            ),
            Self::AssetLeavesTheProject {
                index,
                written,
                reason,
            } => format!(
                "operation {index}: the asset `{written}` is not inside the project: {reason}"
            ),
            Self::AssetNotThereYet { index, written } => format!(
                "operation {index}: there is nothing at `{written}` yet; the reference is written as \
                 it is, and the file can arrive later"
            ),
        }
    }
}

/// What a batch would do to a document, worked out before it does anything.
///
/// A plan holds the change set it built and everything it found, so the two cannot disagree: the
/// operations were derived by reading the document once, and the findings are about the same reading.
#[derive(Clone, PartialEq, Debug)]
pub struct Plan {
    intent: String,
    base: Revision,
    set: Option<ChangeSet>,
    findings: Vec<Finding>,
}

impl Plan {
    /// Reads a batch against a document, without changing it.
    pub fn of(
        batch: &Batch,
        document: &DocumentStore,
        root: &ProjectRoot,
    ) -> Result<Self, BatchRefusal> {
        let Some(page) = document.pages().next().map(|page| page.id()) else {
            return Err(BatchRefusal::NoRoot);
        };
        let page_root = document
            .pages()
            .next()
            .map(|page| page.root())
            .ok_or(BatchRefusal::NoRoot)?;

        let mut findings = Vec::new();
        let mut ops: Vec<ChangeOp> = Vec::new();
        // What the batch has already done. A batch is a sequence of operations, so a check has to be
        // made against the document *as the batch leaves it*: removing a node's last child and then
        // the node is one batch a person can write, and reading only the document would refuse it.
        // This mirrors what the store's own staging does, and the store is still the one that decides:
        // a plan that gets this wrong is caught by `apply`, which refuses the whole thing.
        let mut structure = Structure::of(document);
        let mut gone: BTreeSet<NodeId> = BTreeSet::new();
        let mut written: BTreeMap<(NodeId, PropertyName), usize> = BTreeMap::new();
        let mut staged: BTreeMap<(NodeId, PropertyName), Option<PropertyValue>> = BTreeMap::new();

        for (index, step) in batch.steps().iter().enumerate() {
            match step {
                Step::Add {
                    parent,
                    index: at,
                    node,
                } => {
                    let id = node.id();
                    let Some(parent_page) = document.node(*parent).map(|node| node.page()) else {
                        findings.push(Finding::UnknownNode { index, id: *parent });
                        continue;
                    };
                    if parent_page != page {
                        findings.push(Finding::OtherPage {
                            index,
                            id,
                            page: parent_page,
                        });
                        continue;
                    }
                    if document.node(id).is_some() && !gone.contains(&id) {
                        findings.push(Finding::IdentityInUse { index, id });
                        continue;
                    }
                    if document.is_retired(id) {
                        findings.push(Finding::RetiredIdentity { index, id });
                        continue;
                    }
                    let children = structure.children(*parent).len();
                    if *at > children {
                        findings.push(Finding::Placement {
                            index,
                            id,
                            parent: *parent,
                            at: *at,
                            children,
                        });
                        continue;
                    }
                    structure.insert(*parent, *at, id);
                    gone.remove(&id);
                    let bare = Node::new(page, id, node.kind().clone());
                    ops.push(ChangeOp::AddNode {
                        parent: *parent,
                        index: *at,
                        node: Box::new(bare),
                    });
                    for (name, value) in node.properties() {
                        written.insert((id, name.clone()), index);
                        staged.insert((id, name.clone()), Some(value.clone()));
                        ops.push(ChangeOp::SetProperty {
                            id,
                            name: name.clone(),
                            previous: None,
                            value: value.clone(),
                        });
                    }
                }
                Step::Remove { id } => {
                    // Removing a node takes its subtree with it, children first: the store refuses a
                    // node that still has children, and "remove this control" is what a person means.
                    // What went with it is reported rather than silently implied.
                    if document.node(*id).is_none() && !structure.knows(*id) {
                        findings.push(Finding::UnknownNode { index, id: *id });
                        continue;
                    }
                    if *id == page_root {
                        findings.push(Finding::ThePageRoot { index, id: *id });
                        continue;
                    }
                    let taken = structure.subtree(*id);
                    if taken.len() > 1 {
                        findings.push(Finding::SubtreeRemoval {
                            index,
                            id: *id,
                            children: taken.iter().copied().skip(1).collect(),
                        });
                    }
                    for removal in structure.remove_subtree(*id) {
                        let Some(node) = document.node(removal.id) else {
                            continue;
                        };
                        gone.insert(removal.id);
                        ops.push(ChangeOp::RemoveNode {
                            node: Box::new(record(node)),
                            parent: removal.parent,
                            index: removal.index,
                        });
                    }
                }
                Step::Move {
                    id,
                    to_parent,
                    to_index,
                } => {
                    if document.node(*id).is_none() || gone.contains(id) {
                        findings.push(Finding::UnknownNode { index, id: *id });
                        continue;
                    }
                    if *id == page_root {
                        findings.push(Finding::ThePageRoot { index, id: *id });
                        continue;
                    }
                    let Some(target_page) = document.node(*to_parent).map(|node| node.page())
                    else {
                        findings.push(Finding::UnknownNode {
                            index,
                            id: *to_parent,
                        });
                        continue;
                    };
                    if target_page != page {
                        findings.push(Finding::OtherPage {
                            index,
                            id: *id,
                            page: target_page,
                        });
                        continue;
                    }
                    if structure.is_inside(*to_parent, *id) {
                        findings.push(Finding::CyclicMove {
                            index,
                            id: *id,
                            parent: *to_parent,
                        });
                        continue;
                    }
                    let children = structure.children(*to_parent).len();
                    let from_parent = document
                        .node(*id)
                        .and_then(Node::parent)
                        .unwrap_or(page_root);
                    let Some(from_index) = structure.index_of(from_parent, *id) else {
                        findings.push(Finding::UnknownNode { index, id: *id });
                        continue;
                    };
                    let allowed = if from_parent == *to_parent {
                        children.saturating_sub(1)
                    } else {
                        children
                    };
                    if *to_index > allowed {
                        findings.push(Finding::Placement {
                            index,
                            id: *id,
                            parent: *to_parent,
                            at: *to_index,
                            children,
                        });
                        continue;
                    }
                    structure.move_node(from_parent, from_index, *to_parent, *to_index);
                    ops.push(ChangeOp::MoveNode {
                        id: *id,
                        from_parent,
                        from_index,
                        to_parent: *to_parent,
                        to_index: *to_index,
                    });
                }
                Step::Set { id, name, value } => {
                    let Some(node) = document.node(*id).filter(|_| !gone.contains(id)) else {
                        findings.push(Finding::UnknownNode { index, id: *id });
                        continue;
                    };
                    if let Some(first) = written.insert((*id, name.clone()), index) {
                        findings.push(Finding::WrittenTwice {
                            id: *id,
                            name: name.clone(),
                            first,
                            second: index,
                        });
                    }
                    let previous = staged
                        .get(&(*id, name.clone()))
                        .cloned()
                        .unwrap_or_else(|| node.property(name).cloned());
                    if let PropertyValue::Asset(asset) = value {
                        match root.resolve_for_creation(asset) {
                            Err(error) => {
                                findings.push(Finding::AssetLeavesTheProject {
                                    index,
                                    written: asset.clone(),
                                    reason: error.message(),
                                });
                                continue;
                            }
                            Ok(path) if !path.is_file() => {
                                findings.push(Finding::AssetNotThereYet {
                                    index,
                                    written: asset.clone(),
                                });
                            }
                            Ok(_) => {}
                        }
                    }
                    staged.insert((*id, name.clone()), Some(value.clone()));
                    ops.push(ChangeOp::SetProperty {
                        id: *id,
                        name: name.clone(),
                        previous,
                        value: value.clone(),
                    });
                }
                Step::Clear { id, name } => {
                    let Some(node) = document.node(*id).filter(|_| !gone.contains(id)) else {
                        findings.push(Finding::UnknownNode { index, id: *id });
                        continue;
                    };
                    if let Some(first) = written.insert((*id, name.clone()), index) {
                        findings.push(Finding::WrittenTwice {
                            id: *id,
                            name: name.clone(),
                            first,
                            second: index,
                        });
                    }
                    let removed = staged
                        .get(&(*id, name.clone()))
                        .cloned()
                        .unwrap_or_else(|| node.property(name).cloned());
                    let Some(removed) = removed else {
                        findings.push(Finding::NothingToClear {
                            index,
                            id: *id,
                            name: name.clone(),
                        });
                        continue;
                    };
                    staged.insert((*id, name.clone()), None);
                    ops.push(ChangeOp::ClearProperty {
                        id: *id,
                        name: name.clone(),
                        removed,
                    });
                }
            }
        }

        // A batch whose every step was blocked has nothing to run: that is a blocked plan, not an
        // error, because the findings are what say why.
        let set = ChangeSet::new(
            document.revision(),
            ChangeOrigin::Human,
            batch.intent(),
            ops,
        );
        Ok(Self {
            intent: batch.intent().to_owned(),
            base: document.revision(),
            set,
            findings,
        })
    }

    /// The batch as a change set, which is what the document applies, when there is anything to run.
    pub fn change_set(&self) -> Option<&ChangeSet> {
        self.set.as_ref()
    }

    /// The revision the plan was prepared against.
    pub const fn base(&self) -> Revision {
        self.base
    }

    /// What the plan found.
    pub fn findings(&self) -> &[Finding] {
        &self.findings
    }

    /// Whether the whole batch is refused: something the plan found blocks it, or nothing in it can
    /// land at all.
    pub fn is_blocked(&self) -> bool {
        self.set.is_none() || self.findings.iter().any(Finding::is_blocking)
    }

    /// The plan in one report: the intent, the operations, and what was found.
    pub fn render(&self) -> Vec<String> {
        let blocked = self.is_blocked();
        let mut lines = vec![match &self.set {
            Some(set) if blocked => format!(
                "plan for `{}`: {} operation(s) could land against {}, and the batch is refused as a \
                 whole, so none of them do",
                self.intent,
                set.ops().len(),
                self.base
            ),
            Some(set) => format!(
                "plan for `{}`: {} operation(s) against {}",
                self.intent,
                set.ops().len(),
                self.base
            ),
            None => format!(
                "plan for `{}`: no operation can land, so the batch would do nothing",
                self.intent
            ),
        }];
        if let Some(set) = &self.set {
            lines.push(format!(
                "it needs {}: {}",
                if set.capabilities().len() == 1 {
                    "this capability"
                } else {
                    "these capabilities"
                },
                set.capabilities()
                    .iter()
                    .map(|capability| capability.name())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if self.findings.is_empty() {
            lines.push("nothing was found against it: every operation can land".to_owned());
        } else {
            for finding in &self.findings {
                lines.push(format!(
                    "  {} {}: {}",
                    if finding.is_blocking() {
                        "blocks"
                    } else {
                        "note  "
                    },
                    finding.name(),
                    finding.message()
                ));
            }
        }
        lines
    }
}

/// What happened when a batch was applied.
#[derive(Clone, PartialEq, Debug)]
pub struct Outcome {
    applied: bool,
    from: Revision,
    to: Revision,
    rejection: Option<ChangeRejection>,
    inverse: Option<ChangesetInverse>,
}

/// The change set that undoes what was applied, and the batch that says so.
#[derive(Clone, PartialEq, Debug)]
pub struct ChangesetInverse {
    /// The inverse change set, which is what the document would apply.
    pub set: ChangeSet,
    /// The same thing as a batch, which is what a person can read, store and replay.
    pub batch: Batch,
}

impl Outcome {
    /// Whether every operation landed.
    pub const fn is_applied(&self) -> bool {
        self.applied
    }

    /// The revision before the attempt.
    pub const fn from(&self) -> Revision {
        self.from
    }

    /// The revision afterwards: the new one when it was applied, and the unchanged one when it was
    /// refused.
    pub const fn to(&self) -> Revision {
        self.to
    }

    /// Why it was refused, when it was.
    pub const fn rejection(&self) -> Option<&ChangeRejection> {
        self.rejection.as_ref()
    }

    /// What would undo it, when it was applied.
    pub fn inverse(&self) -> Option<&ChangesetInverse> {
        self.inverse.as_ref()
    }

    /// The outcome in one report.
    pub fn render(&self) -> Vec<String> {
        match (&self.applied, &self.rejection) {
            (true, _) => vec![format!(
                "applied: everything landed in one step, {} -> {}",
                self.from, self.to
            )],
            (false, Some(rejection)) => vec![
                format!(
                    "refused: nothing landed and the document is still at {}",
                    self.to
                ),
                format!("  {} : {}", rejection.name(), rejection.message()),
            ],
            (false, None) => vec!["refused: nothing landed".to_owned()],
        }
    }
}

/// Applies a plan to a document: all of it, or none of it.
///
/// The store is what makes that true (it stages every operation and commits once), so this function
/// does not re-implement it — it only reads the answer and works out the inverse when the answer was
/// yes. A plan that was blocked by its own findings is not applied at all: a batch with a known
/// blocking finding would be refused by the store anyway, and stopping at the plan is what keeps the
/// refusal's *reason* in the batch's own words.
pub fn apply(plan: &Plan, document: &mut DocumentStore) -> Outcome {
    let from = document.revision();
    if plan.is_blocked() {
        return Outcome {
            applied: false,
            from,
            to: from,
            rejection: None,
            inverse: None,
        };
    }
    let Some(set) = plan.set.clone() else {
        return Outcome {
            applied: false,
            from,
            to: from,
            rejection: None,
            inverse: None,
        };
    };
    // A batch names the identities it adds, so the runner adopts them first: the store only accepts
    // an id it issued, and a batch's ids are exactly as real as a file's (adopting one changes no
    // node and no property, and it cannot bring a retired identity back to life).
    for op in set.ops() {
        if let ChangeOp::AddNode { node, .. } = op {
            document.adopt_id(node.id());
        }
    }
    let outcome = document.apply(set.clone());
    match outcome {
        ApplyOutcome::Applied { from, to } => {
            let inverse_set = set.inverse(to);
            let inverse = ChangesetInverse {
                batch: Batch::of_change_set(
                    set.intent().to_owned(),
                    &format!("undo: {}", plan.intent),
                    &inverse_set,
                ),
                set: inverse_set,
            };
            Outcome {
                applied: true,
                from,
                to,
                rejection: None,
                inverse: Some(inverse),
            }
        }
        ApplyOutcome::Rejected {
            rejection,
            revision,
        } => Outcome {
            applied: false,
            from,
            to: revision,
            rejection: Some(rejection),
            inverse: None,
        },
    }
}

/// The model a page document describes, built through change sets.
///
/// This is the file → model seam's other half: the page and its root are handed to the store by
/// [`DocumentStore::new`], every other node arrives as an `add`, and its properties arrive as
/// property operations — the same operations a batch would use, so a file cannot end up in the model
/// in a way an edit cannot reach. The file's identities are **adopted** first, because the store only
/// accepts ids it issued: a file's ids are exactly as real as the store's own.
pub fn store_of(
    document: &PageDocument,
    gate: PermissionGate,
) -> Result<DocumentStore, BatchRefusal> {
    let page = document.page().clone();
    let root = document.root_node().ok_or(BatchRefusal::NoRoot)?;
    let mut store = DocumentStore::new(
        gate,
        page.clone(),
        Node::new(page.id(), root.id(), root.kind().clone()),
    )
    .ok_or(BatchRefusal::NotAModel {
        reason: format!(
            "the page {} does not own the root node #{}",
            page.id().get(),
            root.id().get()
        ),
    })?;
    for node in document.nodes() {
        store.adopt_id(node.id());
    }

    // The root's own properties are the first thing to arrive: the root is in the model already
    // (that is what `DocumentStore::new` does with it), and a property of it is a property operation
    // like any other.
    let mut ops: Vec<ChangeOp> = Vec::new();
    for (name, value) in root.properties() {
        ops.push(ChangeOp::SetProperty {
            id: root.id(),
            name: name.clone(),
            previous: None,
            value: value.clone(),
        });
    }
    // Depth first from the root, so a parent is always in the model before its children are added.
    let mut queue: Vec<NodeId> = vec![root.id()];
    while let Some(id) = queue.pop() {
        let Some(node) = document.node(id) else {
            continue;
        };
        for (index, child) in node.children().iter().enumerate() {
            let Some(child_node) = document.node(*child) else {
                return Err(BatchRefusal::NotAModel {
                    reason: format!(
                        "#{} names child #{} and no node declares it",
                        id.get(),
                        child.get()
                    ),
                });
            };
            let bare = Node::new(page.id(), *child, child_node.kind().clone());
            ops.push(ChangeOp::AddNode {
                parent: id,
                index,
                node: Box::new(bare),
            });
            for (name, value) in child_node.properties() {
                ops.push(ChangeOp::SetProperty {
                    id: *child,
                    name: name.clone(),
                    previous: None,
                    value: value.clone(),
                });
            }
            queue.push(*child);
        }
    }
    if ops.is_empty() {
        return Ok(store);
    }
    let set = ChangeSet::new(
        store.revision(),
        ChangeOrigin::Human,
        "read the page document into the model",
        ops,
    )
    .ok_or(BatchRefusal::NothingToDo)?;
    let outcome = store.apply(set);
    if let Some(rejection) = outcome.rejection() {
        return Err(BatchRefusal::NotAModel {
            reason: format!("the file's nodes were refused: {}", rejection.message()),
        });
    }
    Ok(store)
}

/// The child lists a batch is working on: the document's, with what the batch has already done.
///
/// The store stages its own working set and measures every position against it, so a plan that
/// measured positions against the *document* would disagree with the store the moment a batch removes
/// one node and moves another. This is the same staging, small enough to read: the child lists are
/// read from the document once and updated as the batch's operations are derived. Nothing here writes
/// to the document — it is what the plan *would* do, and the store still decides.
struct Structure {
    children: BTreeMap<NodeId, Vec<NodeId>>,
    parents: BTreeMap<NodeId, NodeId>,
}

/// One node on its way out: where it sits *now*, which is the position the store will be told.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Removal {
    id: NodeId,
    parent: NodeId,
    index: usize,
}

impl Structure {
    fn of(document: &DocumentStore) -> Self {
        let mut children = BTreeMap::new();
        let mut parents = BTreeMap::new();
        for node in document.nodes() {
            children.insert(node.id(), node.children().to_vec());
            if let Some(parent) = node.parent() {
                parents.insert(node.id(), parent);
            }
        }
        Self { children, parents }
    }

    fn knows(&self, id: NodeId) -> bool {
        self.children.contains_key(&id)
    }

    fn children(&self, parent: NodeId) -> &[NodeId] {
        self.children.get(&parent).map(Vec::as_slice).unwrap_or(&[])
    }

    fn index_of(&self, parent: NodeId, id: NodeId) -> Option<usize> {
        self.children(parent).iter().position(|child| *child == id)
    }

    fn insert(&mut self, parent: NodeId, index: usize, id: NodeId) {
        let list = self.children.entry(parent).or_default();
        let index = index.min(list.len());
        list.insert(index, id);
        self.children.entry(id).or_default();
    }

    fn move_node(
        &mut self,
        from_parent: NodeId,
        from_index: usize,
        to_parent: NodeId,
        to_index: usize,
    ) {
        let id = match self.children.get_mut(&from_parent) {
            Some(list) if from_index < list.len() => list.remove(from_index),
            _ => return,
        };
        let list = self.children.entry(to_parent).or_default();
        let index = to_index.min(list.len());
        list.insert(index, id);
    }

    /// Whether `candidate` is `of` or inside its subtree.
    fn is_inside(&self, candidate: NodeId, of: NodeId) -> bool {
        if candidate == of {
            return true;
        }
        let mut stack = vec![of];
        while let Some(id) = stack.pop() {
            for child in self.children(id) {
                if *child == candidate {
                    return true;
                }
                stack.push(*child);
            }
        }
        false
    }

    /// The ids of a subtree, the node first, in the order a person would read them.
    fn subtree(&self, id: NodeId) -> Vec<NodeId> {
        let mut out = vec![id];
        let mut stack = vec![id];
        while let Some(next) = stack.pop() {
            for child in self.children(next) {
                out.push(*child);
                stack.push(*child);
            }
        }
        out
    }

    /// Takes a subtree out, deepest first, and says where each node sat as it went.
    ///
    /// The order is the store's requirement rather than a preference: a node is removed only when it
    /// has no children, so the children go first — and each position is read *before* the node leaves
    /// its parent's list, which is what makes the positions the ones the store will check. A list
    /// copied after the removals would name positions that are already gone.
    fn remove_subtree(&mut self, id: NodeId) -> Vec<Removal> {
        let mut order = Vec::new();
        let mut stack = vec![(id, false)];
        while let Some((node, visited)) = stack.pop() {
            if !visited {
                stack.push((node, true));
                for child in self.children(node).to_vec() {
                    stack.push((child, false));
                }
                continue;
            }
            if let Some(list) = self.children.get_mut(&node) {
                list.clear();
            }
            if let Some(parent) = self.parents.get(&node).copied()
                && let Some(index) = self.index_of(parent, node)
            {
                if let Some(list) = self.children.get_mut(&parent) {
                    list.remove(index);
                }
                order.push(Removal {
                    id: node,
                    parent,
                    index,
                });
            }
            self.children.remove(&node);
        }
        self.parents.remove(&id);
        order
    }
}

/// A fingerprint of the model: what a caller compares to ask "is the document back where it was".
///
/// Text rather than a hash, and complete rather than sampled: the ids, kinds, positions and property
/// values in one canonical string, so two fingerprints being equal means the two documents describe
/// the same tree. It is a *reading*, not the document, so it cannot be used to write anything.
///
/// The revision is deliberately not part of it: two documents that describe the same tree are the
/// same document, and how many change sets it took to get there is a different question.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Fingerprint(String);

impl Fingerprint {
    /// Reads a model into a fingerprint.
    pub fn of(document: &DocumentStore) -> Self {
        let mut lines: Vec<String> = Vec::new();
        for page in document.pages() {
            lines.push(format!(
                "page #{} `{}` root #{}",
                page.id().get(),
                page.name(),
                page.root().get()
            ));
        }
        let mut nodes: Vec<&Node> = document.nodes().collect();
        nodes.sort_by_key(|node| node.id().get());
        for node in nodes {
            let children = node
                .children()
                .iter()
                .map(|id| id.get().to_string())
                .collect::<Vec<_>>()
                .join(",");
            let properties = node
                .properties()
                .iter()
                .map(|(name, value)| format!("{}={}", name.as_str(), spelling_of(value)))
                .collect::<Vec<_>>()
                .join(";");
            lines.push(format!(
                "#{} {} page #{} parent {} children [{}] properties [{}]",
                node.id().get(),
                node.kind().as_str(),
                node.page().get(),
                node.parent()
                    .map(|id| id.get().to_string())
                    .unwrap_or_else(|| "-".to_owned()),
                children,
                properties
            ));
        }
        Self(lines.join("\n"))
    }

    /// The fingerprint as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn spelling_of(value: &PropertyValue) -> String {
    match value {
        PropertyValue::Bool(value) => format!("bool:{value}"),
        PropertyValue::Integer(value) => format!("integer:{value}"),
        PropertyValue::Number(value) => format!("number:{value}"),
        PropertyValue::Text(value) => format!("text:{value}"),
        PropertyValue::Variant(value) => format!("variant:{value}"),
        PropertyValue::Color(value) => format!("color:{value}"),
        PropertyValue::Asset(value) => format!("asset:{value}"),
    }
}

/// A node as the document holds it, for a removal to carry: the record is what lets an inverse name
/// the same node, with the same kind and the same properties.
fn record(node: &Node) -> Node {
    Node::record(
        node.page(),
        node.id(),
        node.kind().clone(),
        node.properties().clone(),
    )
}

fn child_index(document: &DocumentStore, parent: NodeId, id: NodeId) -> Option<usize> {
    document
        .node(parent)?
        .children()
        .iter()
        .position(|child| *child == id)
}

fn is_inside(document: &DocumentStore, candidate: NodeId, of: NodeId) -> bool {
    let mut walk = document.node(candidate).and_then(Node::parent);
    while let Some(parent) = walk {
        if parent == of {
            return true;
        }
        walk = document.node(parent).and_then(Node::parent);
    }
    false
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::{Batch, BatchRefusal, Finding, Fingerprint, Plan, Step, apply, store_of};
    use crate::contracts::identity::NodeId;
    use crate::contracts::model::{PropertyName, PropertyValue};
    use crate::contracts::permission::PermissionGate;
    use crate::host::paths::ProjectRoot;
    use crate::project::page::PageDocument;

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
        let root = std::env::temp_dir().join(format!("rustage-batch-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&root).expect("a temporary directory is creatable");
        Sandbox {
            root: root.clone(),
            _guard: TempDirectory(root),
        }
    }

    impl Sandbox {
        fn write(&self, relative: &str, text: &str) -> PathBuf {
            let path = self.root.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("a directory");
            }
            std::fs::write(&path, text).expect("a file");
            path
        }

        fn project(&self) -> ProjectRoot {
            self.write("project.json", "{\"format\": 2}");
            ProjectRoot::new(&self.root).expect("a project root")
        }

        /// The page the batches in these tests edit: a frame with a label and a column holding a
        /// button, so there is a subtree to remove and a place to move things to.
        fn page(&self) -> PageDocument {
            let text = "{\"format\": 1, \"page\": {\"id\": 7, \"name\": \"Main\", \"root\": 0}, \
                 \"nodes\": [ \
                   {\"id\": 0, \"type\": \"Frame\", \"children\": [1, 2], \
                    \"properties\": [{\"name\": \"width\", \"value\": {\"number\": 640}}]}, \
                   {\"id\": 1, \"type\": \"Label\", \"children\": [], \
                    \"properties\": [{\"name\": \"text\", \"value\": {\"text\": \"hello\"}}]}, \
                   {\"id\": 2, \"type\": \"Column\", \"children\": [3], \"properties\": []}, \
                   {\"id\": 3, \"type\": \"Button\", \"children\": [], \"properties\": []} \
                 ]}";
            let path = self.write("pages/main.json", text);
            PageDocument::parse(&self.project(), path, text).expect("a page document")
        }
    }

    fn batch(intent: &str, operations: &str) -> Batch {
        Batch::parse(
            "batch.json",
            &format!("{{\"format\": 1, \"intent\": \"{intent}\", \"operations\": [{operations}]}}"),
        )
        .expect("a batch")
    }

    fn label(text: &str) -> Step {
        batch(
            "a label",
            &format!(
                "{{\"kind\": \"add\", \"parent\": 2, \"index\": 0, \"node\": \
                  {{\"id\": 5, \"type\": \"Label\", \"children\": [], \
                    \"properties\": [{{\"name\": \"text\", \"value\": {{\"text\": \"{text}\"}}}}]}}}}"
            ),
        )
        .steps()[0]
            .clone()
    }

    #[test]
    fn a_file_becomes_a_model_through_change_sets_and_keeps_the_files_identities() {
        let sandbox = sandbox();
        let root = sandbox.project();
        let page = sandbox.page();
        let store = store_of(&page, PermissionGate::new()).expect("a model");

        assert_eq!(store.pages().count(), 1);
        assert_eq!(store.nodes().count(), 4);
        // The tree is the file's tree, with the file's ids, and the parent links the file does not
        // write are the ones the store built.
        let frame = store.node(NodeId::from_raw(0)).expect("the root");
        assert_eq!(frame.kind().as_str(), "Frame");
        assert_eq!(
            frame.children(),
            [NodeId::from_raw(1), NodeId::from_raw(2)],
            "the children are in the file's order"
        );
        assert_eq!(
            frame.property(&PropertyName::new("width").expect("a name")),
            Some(&PropertyValue::Number(640.0))
        );
        let button = store.node(NodeId::from_raw(3)).expect("the button");
        assert_eq!(button.parent(), Some(NodeId::from_raw(2)));
        // Reading a file is one change set, so the revision says the model was built rather than
        // 4 revisions of pretending each node arrived on its own.
        assert_eq!(store.revision().get(), 1);

        // Reading the same file twice gives the same model, which is what "the file is the identity"
        // means: the ids are not re-allocated per read.
        let again = store_of(&page, PermissionGate::new()).expect("a model");
        assert_eq!(Fingerprint::of(&store), Fingerprint::of(&again));
        let _ = root;
    }

    #[test]
    fn a_batch_of_moves_and_sets_lands_and_its_inverse_puts_the_document_back() {
        let sandbox = sandbox();
        let root = sandbox.project();
        let page = sandbox.page();
        let mut store = store_of(&page, PermissionGate::new()).expect("a model");
        let before = Fingerprint::of(&store);

        // Two operations of different kinds in one batch: a move and a set. Either both happen or
        // neither does.
        let edit = batch(
            "label the button and put it first",
            "{\"kind\": \"set\", \"id\": 3, \"name\": \"text\", \"value\": {\"text\": \"Save\"}}, \
             {\"kind\": \"move\", \"id\": 3, \"to_parent\": 0, \"to_index\": 0}",
        );
        let plan = Plan::of(&edit, &store, &root).expect("a plan");
        assert!(!plan.is_blocked(), "{:?}", plan.render());
        assert_eq!(plan.change_set().expect("a change set").ops().len(), 2);

        let outcome = apply(&plan, &mut store);
        assert!(outcome.is_applied(), "{:?}", outcome.render());
        assert_eq!(outcome.from().get(), 1);
        assert_eq!(outcome.to().get(), 2);
        assert_eq!(
            store
                .node(NodeId::from_raw(0))
                .expect("the root")
                .children(),
            [
                NodeId::from_raw(3),
                NodeId::from_raw(1),
                NodeId::from_raw(2)
            ],
        );
        assert_eq!(
            store
                .node(NodeId::from_raw(3))
                .expect("the button")
                .property(&PropertyName::new("text").expect("a name")),
            Some(&PropertyValue::Text("Save".into()))
        );
        assert_ne!(before, Fingerprint::of(&store));

        // The inverse is a batch, and applying it puts the document back exactly: same tree, same
        // property values, and the revision is the one the inverse moved on to.
        let inverse = outcome.inverse().expect("an inverse").clone();
        assert!(inverse.batch.intent().starts_with("undo: "));
        assert_eq!(inverse.batch.steps().len(), 2);
        let plan = Plan::of(&inverse.batch, &store, &root).expect("a plan");
        assert!(!plan.is_blocked(), "{:?}", plan.render());
        let undone = apply(&plan, &mut store);
        assert!(undone.is_applied(), "{:?}", undone.render());
        let after = Fingerprint::of(&store);
        assert_eq!(
            after, before,
            "the model is back where it started, tree and property values alike"
        );
    }

    #[test]
    fn one_operation_that_cannot_land_refuses_the_whole_batch_and_changes_nothing() {
        let sandbox = sandbox();
        let root = sandbox.project();
        let page = sandbox.page();
        let mut store = store_of(&page, PermissionGate::new()).expect("a model");
        let before = Fingerprint::of(&store);

        // A set that is fine, and a move of a node the document does not have.
        let edit = batch(
            "one good operation and one that names nothing",
            "{\"kind\": \"set\", \"id\": 1, \"name\": \"text\", \"value\": {\"text\": \"changed\"}}, \
             {\"kind\": \"move\", \"id\": 99, \"to_parent\": 0, \"to_index\": 0}",
        );
        let plan = Plan::of(&edit, &store, &root).expect("a plan");
        assert!(plan.is_blocked());
        assert!(
            plan.findings()
                .iter()
                .any(|finding| matches!(finding, Finding::UnknownNode { .. })
                    && finding.is_blocking())
        );

        // Blocked at the plan: the bad operation is reported in the batch's own words, and the good
        // one is not attempted either.
        let outcome = apply(&plan, &mut store);
        assert!(!outcome.is_applied());
        assert!(
            outcome.rejection().is_none(),
            "the plan refused it, not the store"
        );
        assert_eq!(outcome.to().get(), 1);
        assert_eq!(Fingerprint::of(&store), before);
        assert!(
            outcome.render()[0].contains("nothing landed"),
            "{:?}",
            outcome.render()
        );

        // A plan that looks fine but is stale is refused by the store, and the document is still
        // untouched: the store is the guarantee, the plan is the courtesy.
        let edit = batch(
            "set the label",
            "{\"kind\": \"set\", \"id\": 1, \"name\": \"text\", \"value\": {\"text\": \"later\"}}",
        );
        let plan = Plan::of(&edit, &store, &root).expect("a plan");
        let other = batch(
            "set the label first, from somewhere else",
            "{\"kind\": \"set\", \"id\": 1, \"name\": \"text\", \"value\": {\"text\": \"first\"}}",
        );
        let other_plan = Plan::of(&other, &store, &root).expect("a plan");
        assert!(apply(&other_plan, &mut store).is_applied());
        let stale = apply(&plan, &mut store);
        assert!(!stale.is_applied());
        assert_eq!(
            stale.rejection().map(|rejection| rejection.name()),
            Some("stale-base")
        );
        assert_eq!(stale.to().get(), 2, "the document stayed where it was");
        assert!(
            store
                .node(NodeId::from_raw(1))
                .expect("the label")
                .property(&PropertyName::new("text").expect("a name"))
                == Some(&PropertyValue::Text("first".into()))
        );
    }

    #[test]
    fn a_subtree_and_an_identity_are_two_different_kinds_of_no() {
        let sandbox = sandbox();
        let root = sandbox.project();
        let page = sandbox.page();
        let mut store = store_of(&page, PermissionGate::new()).expect("a model");

        // Removing a node that has a subtree under it removes the subtree: what went with it is
        // named (a note, because that is what the batch asked for) and the whole thing is one
        // revision.
        let edit = batch("remove the column", "{\"kind\": \"remove\", \"id\": 2}");
        let plan = Plan::of(&edit, &store, &root).expect("a plan");
        assert!(!plan.is_blocked(), "{:?}", plan.render());
        assert!(
            plan.findings()
                .iter()
                .any(|finding| matches!(finding, Finding::SubtreeRemoval { .. }))
        );
        assert!(plan.findings().iter().all(|finding| !finding.is_blocking()));
        let message = plan.render().join("\n");
        assert!(
            message.contains("also removes what is under it (#3)"),
            "{message}"
        );
        assert!(message.contains("same revision"), "{message}");
        let outcome = apply(&plan, &mut store);
        assert!(outcome.is_applied(), "{:?}", outcome.render());
        assert_eq!(
            store.nodes().count(),
            2,
            "the column and the button went together"
        );
        assert_eq!(outcome.to().get(), 2, "a whole subtree is one revision");

        // Undoing it names the identities that were removed — and the store retires those, so the
        // inverse is *refused*, in the store's own words, rather than silently bringing an identity
        // back to life.
        let inverse = outcome.inverse().expect("an inverse").clone();
        let plan = Plan::of(&inverse.batch, &store, &root).expect("a plan");
        assert!(plan.is_blocked(), "a retired identity is not live again");
        assert!(
            plan.findings()
                .iter()
                .any(|finding| matches!(finding, Finding::RetiredIdentity { .. }))
        );
        let refused = apply(&plan, &mut store);
        assert!(!refused.is_applied());
        assert_eq!(store.nodes().count(), 2, "nothing came back");
        assert!(plan.render().join("\n").contains("never reused"));

        // An addition is the other way round: its inverse is a removal, and that lands, because a
        // removal does not need the identity to be live again.
        let added = batch(
            "add a label under the frame",
            "{\"kind\": \"add\", \"parent\": 0, \"index\": 1, \"node\": {\"id\": 9, \
              \"type\": \"Label\", \"children\": [], \
              \"properties\": [{\"name\": \"text\", \"value\": {\"text\": \"new\"}}]}}",
        );
        let plan = Plan::of(&added, &store, &root).expect("a plan");
        assert!(!plan.is_blocked(), "{:?}", plan.render());
        let outcome = apply(&plan, &mut store);
        assert!(outcome.is_applied(), "{:?}", outcome.render());
        assert_eq!(store.nodes().count(), 3);
        let inverse = outcome.inverse().expect("an inverse").clone();
        assert_eq!(inverse.batch.steps().len(), 2, "the add and its property");
        let plan = Plan::of(&inverse.batch, &store, &root).expect("a plan");
        assert!(!plan.is_blocked(), "{:?}", plan.render());
        let undone = apply(&plan, &mut store);
        assert!(undone.is_applied(), "{:?}", undone.render());
        assert_eq!(store.nodes().count(), 2);
    }

    #[test]
    fn a_batch_that_writes_a_property_twice_says_which_one_wins() {
        let sandbox = sandbox();
        let root = sandbox.project();
        let page = sandbox.page();
        let mut store = store_of(&page, PermissionGate::new()).expect("a model");

        let edit = batch(
            "set the same property twice",
            "{\"kind\": \"set\", \"id\": 1, \"name\": \"text\", \"value\": {\"text\": \"first\"}}, \
             {\"kind\": \"set\", \"id\": 1, \"name\": \"text\", \"value\": {\"text\": \"second\"}}",
        );
        let plan = Plan::of(&edit, &store, &root).expect("a plan");
        // A note, not a block: the batch is legal, and the last write is what the document gets.
        assert!(!plan.is_blocked(), "{:?}", plan.render());
        assert!(
            plan.findings()
                .iter()
                .any(|finding| matches!(finding, Finding::WrittenTwice { .. }))
        );
        assert!(
            plan.render()
                .join("\n")
                .contains("the last one is what the document ends up with")
        );
        let outcome = apply(&plan, &mut store);
        assert!(outcome.is_applied());
        assert_eq!(
            store
                .node(NodeId::from_raw(1))
                .expect("the label")
                .property(&PropertyName::new("text").expect("a name")),
            Some(&PropertyValue::Text("second".into()))
        );

        // Clearing a property that is not set is blocked: the store would refuse it, and the plan
        // says why first.
        let edit = batch(
            "clear nothing",
            "{\"kind\": \"clear\", \"id\": 1, \"name\": \"color\"}",
        );
        let plan = Plan::of(&edit, &store, &root).expect("a plan");
        assert!(plan.is_blocked());
        assert!(
            plan.findings()
                .iter()
                .any(|finding| matches!(finding, Finding::NothingToClear { .. })),
            "{:?}",
            plan.render()
        );
        assert!(
            plan.render()
                .join("\n")
                .contains("there is nothing to clear"),
            "{:?}",
            plan.render()
        );
    }

    #[test]
    fn an_asset_that_leaves_the_project_is_blocked_and_one_that_is_not_there_yet_is_a_note() {
        let sandbox = sandbox();
        let root = sandbox.project();
        let page = sandbox.page();
        let store = store_of(&page, PermissionGate::new()).expect("a model");

        let outside = batch(
            "point the label at a file outside the project",
            "{\"kind\": \"set\", \"id\": 1, \"name\": \"icon\", \
              \"value\": {\"asset\": \"../../elsewhere.svg\"}}",
        );
        let plan = Plan::of(&outside, &store, &root).expect("a plan");
        assert!(plan.is_blocked());
        assert!(
            plan.findings()
                .iter()
                .any(|finding| matches!(finding, Finding::AssetLeavesTheProject { .. }))
        );

        let later = batch(
            "point the label at an icon that is coming",
            "{\"kind\": \"set\", \"id\": 1, \"name\": \"icon\", \
              \"value\": {\"asset\": \"assets/icon.svg\"}}",
        );
        let plan = Plan::of(&later, &store, &root).expect("a plan");
        assert!(!plan.is_blocked(), "{:?}", plan.render());
        assert!(
            plan.findings()
                .iter()
                .any(|finding| matches!(finding, Finding::AssetNotThereYet { .. }))
        );
        assert!(plan.render().join("\n").contains("there is nothing at"));

        // Once the file is there, the note goes away and the batch is quiet.
        sandbox.write("assets/icon.svg", "<svg/>");
        let plan = Plan::of(&later, &store, &root).expect("a plan");
        assert!(plan.findings().is_empty(), "{:?}", plan.render());
    }

    #[test]
    fn a_batch_says_what_it_is_for_and_refuses_to_be_read_without_one() {
        let sandbox = sandbox();

        // No intent: a batch nobody can review is refused.
        let refusal = Batch::parse("batch.json", "{\"format\": 1, \"operations\": []}")
            .expect_err("no intent");
        assert!(matches!(refusal, BatchRefusal::NotABatch { .. }));
        assert!(refusal.message().contains("nobody can review it"));

        // No operations: an empty batch would move the revision without changing anything.
        let refusal = Batch::parse("batch.json", "{\"format\": 1, \"intent\": \"nothing\"}")
            .expect_err("no operations");
        assert!(matches!(refusal, BatchRefusal::NotABatch { .. }));
        let refusal = Batch::parse(
            "batch.json",
            "{\"format\": 1, \"intent\": \"nothing\", \"operations\": []}",
        )
        .expect_err("no operations");
        assert_eq!(refusal, BatchRefusal::NothingToDo);
        assert!(refusal.message().contains("without changing anything"));

        // A kind this build does not know, a node whose id is not written down, a blank property
        // name, a newer format, and text that is not JSON at all.
        for (text, expected) in [
            (
                "{\"format\": 1, \"intent\": \"x\", \"operations\": [{\"kind\": \"rotate\"}]}",
                "a batch holds add, remove, move, set, clear",
            ),
            (
                "{\"format\": 1, \"intent\": \"x\", \"operations\": [{\"kind\": \"remove\"}]}",
                "`operations[0].id` must be a whole number",
            ),
            (
                "{\"format\": 1, \"intent\": \"x\", \"operations\": \
                  [{\"kind\": \"set\", \"id\": 1, \"name\": \"  \", \"value\": {\"text\": \"a\"}}]}",
                "a property name is not blank",
            ),
            (
                "{\"format\": 2, \"intent\": \"x\", \"operations\": []}",
                "this build reads v1",
            ),
            ("not json", "the batch is not JSON"),
        ] {
            let refusal = Batch::parse("batch.json", text).expect_err(text);
            assert!(
                refusal.message().contains(expected),
                "{text}: {}",
                refusal.message()
            );
        }

        // A file that is not there, and a batch read from a file that is.
        let refusal = Batch::read(sandbox.root.join("gone.json")).expect_err("no file");
        assert!(matches!(refusal, BatchRefusal::Unreadable { .. }));
        let path = sandbox.write(
            "batch.json",
            "{\"format\": 1, \"intent\": \"read me\", \"operations\": [{\"kind\": \"remove\", \"id\": 3}]}",
        );
        let read = Batch::read(&path).expect("a batch");
        assert_eq!(read.intent(), "read me");
        assert_eq!(
            read.steps(),
            &[Step::Remove {
                id: NodeId::from_raw(3)
            }]
        );
        assert!(read.describe()[0].contains("read me"));
        assert!(read.describe()[1].contains("remove #3"));
        let text = read.render().expect("a rendering");
        assert_eq!(Batch::parse(read.path(), &text).expect("reads back"), read);
    }

    #[test]
    fn the_plan_says_what_the_page_and_the_root_leave_alone() {
        let sandbox = sandbox();
        let root = sandbox.project();
        let page = sandbox.page();
        let mut store = store_of(&page, PermissionGate::new()).expect("a model");
        let before = Fingerprint::of(&store);

        // The page root is not a batch step.
        let edit = batch("remove the root", "{\"kind\": \"remove\", \"id\": 0}");
        let plan = Plan::of(&edit, &store, &root).expect("a plan");
        assert!(plan.is_blocked());
        assert!(plan.render().join("\n").contains("a page owns its root"));

        // A move into the node's own subtree is blocked.
        let edit = batch(
            "move the column into its own child",
            "{\"kind\": \"move\", \"id\": 2, \"to_parent\": 3, \"to_index\": 0}",
        );
        let plan = Plan::of(&edit, &store, &root).expect("a plan");
        assert!(plan.is_blocked());
        assert!(
            plan.findings()
                .iter()
                .any(|finding| matches!(finding, Finding::CyclicMove { .. }))
        );
        assert!(
            plan.render()
                .join("\n")
                .contains("would put it inside itself")
        );

        // A placement past the end of the parent's children is blocked.
        let edit = batch(
            "put the label past the end",
            "{\"kind\": \"move\", \"id\": 1, \"to_parent\": 2, \"to_index\": 9}",
        );
        let plan = Plan::of(&edit, &store, &root).expect("a plan");
        assert!(plan.is_blocked());
        assert!(
            plan.findings()
                .iter()
                .any(|finding| matches!(finding, Finding::Placement { .. }))
        );

        // None of those attempts changed anything, and the plan's report is the reason.
        assert_eq!(Fingerprint::of(&store), before);
        let blocked = Plan::of(&edit, &store, &root).expect("a plan");
        assert!(
            blocked
                .render()
                .join("\n")
                .contains("no operation can land")
        );
        assert!(!apply(&blocked, &mut store).is_applied());
        assert_eq!(Fingerprint::of(&store), before);

        // A plan says which capability the store will ask about, and removing a node is the node one.
        let removal = batch("remove the button", "{\"kind\": \"remove\", \"id\": 3}");
        let plan = Plan::of(&removal, &store, &root).expect("a plan");
        let report = plan.render().join("\n");
        assert!(report.contains("plan for `"), "{report}");
        assert!(report.contains("it needs this capability"), "{report}");
        assert!(report.contains("add-or-remove-node"), "{report}");
        assert_eq!(Fingerprint::of(&store), before, "planning changes nothing");

        // A plan for a batch that only sets a property needs the other capability.
        let edit = batch(
            "set the label",
            "{\"kind\": \"set\", \"id\": 1, \"name\": \"text\", \"value\": {\"text\": \"later\"}}",
        );
        let plan = Plan::of(&edit, &store, &root).expect("a plan");
        assert!(plan.render().join("\n").contains("edit-widget-property"));
    }
}
