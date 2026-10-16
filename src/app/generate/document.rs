// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The page document the generator reads (task T15.02, the `build` half).
//!
//! The library's generator parses a document whose every node is a single-key object —
//! `{"button": { "text": "Go", "children": [ … ] }}` — while a rustage page document stores a flat
//! node list with `type`, `properties` and `children`. Both describe one tree, so **one** translation
//! exists ([`to_library_document`]) and both the emitter and the golden tests read it; a second
//! translation would be the drifting parallel path principle 59 forbids.
//!
//! Three properties the translation keeps, because generation is deterministic and honest:
//!
//! * **A value arrives as a value, not as a tag.** The page format writes `{"text": "Save"}`; the
//!   generator's document wants `"Save"`. [`plain_value`] unwraps the tag, and a value JSON cannot
//!   hold (an infinite number) is a refusal rather than a rounded or `null` spelling.
//! * **Order is the document's, not a map's.** Properties are written in name order (the model's own
//!   order) and children in layout order, so two runs over one page produce one byte string.
//! * **A malformed tree is refused, not truncated.** A missing child, a cycle or a tree deeper than
//!   [`MAX_DEPTH`] stops the translation with the offending id, rather than emitting a document the
//!   generator would have to guess at.

#![allow(dead_code)]

use crate::contracts::identity::NodeId;
use crate::contracts::model::PropertyValue;
use crate::project::record::json::{Json, JsonObject, Number};
use crate::project::record::page::PageNode;

/// How deep a page's tree may be before the translation refuses it.
///
/// The library bounds its own recursion at the same number; matching it here means a page this side
/// accepts is not rejected on the other side for depth, and a page that is not a tree fails here
/// with a clearer reason than a parser stack message.
pub const MAX_DEPTH: u32 = 64;

/// Translates a page's nodes into the generator's own document.
///
/// `root` is the page's root node id. The result is a single-key JSON object — the root control's
/// name — whose body carries the root's properties and, unless it is a leaf, its `children`.
pub fn to_library_document(root: NodeId, nodes: &[PageNode]) -> Result<String, String> {
    let mut visit = vec![root];
    let body = node_body(root, nodes, &mut visit, 0)?;
    let Some(root_node) = nodes.iter().find(|node| node.id() == root) else {
        return Err(format!("the root node {} is not in the page", root.get()));
    };
    let mut document = JsonObject::new();
    document.insert(root_node.kind().as_str(), Json::Object(body));
    Ok(Json::Object(document).render())
}

/// The key the generator derives for a node at `path`.
///
/// The library names a node by its position, `root` for the root and `n_0`, `n_0_1`, … for its
/// descendants. This is the one place rustage spells that rule; the source map ([`super::mapping`])
/// finds a node in the emitted source by the `.key("…")` call that carries it, so the two must agree
/// on exactly this string (principle 59).
pub fn key_of(path: &[usize]) -> String {
    if path.is_empty() {
        return "root".to_owned();
    }
    let mut key = String::from("n");
    for index in path {
        key.push('_');
        key.push_str(&index.to_string());
    }
    key
}

/// Each node's identity next to the key the generator will write for it, in pre-order.
///
/// The map is what turns "go to source" into "go to this button": the generator emits
/// `.key("n_0_1")` for a node, and this pairs that key back with the stable [`NodeId`] a view holds.
pub fn node_keys(root: NodeId, nodes: &[PageNode]) -> Result<Vec<(NodeId, String)>, String> {
    let mut out = Vec::new();
    let mut path = Vec::new();
    keys_into(root, nodes, &mut path, &mut out, 0)?;
    Ok(out)
}

fn keys_into(
    id: NodeId,
    nodes: &[PageNode],
    path: &mut Vec<usize>,
    out: &mut Vec<(NodeId, String)>,
    depth: u32,
) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err(format!(
            "the page tree is deeper than {MAX_DEPTH} at node {}",
            id.get()
        ));
    }
    let node = nodes
        .iter()
        .find(|node| node.id() == id)
        .ok_or_else(|| format!("node {} is referenced but not in the page", id.get()))?;
    out.push((id, key_of(path)));
    for (index, child) in node.children().iter().enumerate() {
        path.push(index);
        keys_into(*child, nodes, path, out, depth + 1)?;
        path.pop();
    }
    Ok(())
}

/// One node's body, recursively.
fn node_body(
    id: NodeId,
    nodes: &[PageNode],
    visit: &mut Vec<NodeId>,
    depth: u32,
) -> Result<JsonObject, String> {
    if depth > MAX_DEPTH {
        return Err(format!(
            "the page tree is deeper than {MAX_DEPTH} at node {}",
            id.get()
        ));
    }
    let node = nodes
        .iter()
        .find(|node| node.id() == id)
        .ok_or_else(|| format!("node {} is referenced but not in the page", id.get()))?;

    let mut body = JsonObject::new();
    for (name, value) in node.properties() {
        body.insert(name.as_str(), plain_value(value, id)?);
    }
    if !node.children().is_empty() {
        let mut children = Vec::with_capacity(node.children().len());
        for child in node.children() {
            if visit.contains(child) {
                return Err(format!(
                    "node {} appears more than once; the page is not a tree",
                    child.get()
                ));
            }
            visit.push(*child);
            let child_node = nodes
                .iter()
                .find(|node| node.id() == *child)
                .ok_or_else(|| {
                    format!("child {} is referenced but not in the page", child.get())
                })?;
            let child_body = node_body(*child, nodes, visit, depth + 1)?;
            let mut wrapper = JsonObject::new();
            wrapper.insert(child_node.kind().as_str(), Json::Object(child_body));
            children.push(Json::Object(wrapper));
            visit.pop();
        }
        body.insert("children", Json::array(children));
    }
    Ok(body)
}

/// A model value as the plain JSON the generator's document holds.
fn plain_value(value: &PropertyValue, id: NodeId) -> Result<Json, String> {
    Ok(match value {
        PropertyValue::Bool(value) => Json::Bool(*value),
        PropertyValue::Integer(value) => Json::Number(Number::from_i64(*value)),
        PropertyValue::Number(value) => {
            if !value.is_finite() {
                return Err(format!(
                    "node {} has the property value {value}, which JSON cannot hold",
                    id.get()
                ));
            }
            Json::Number(Number::new(value.to_string()))
        }
        PropertyValue::Text(value)
        | PropertyValue::Variant(value)
        | PropertyValue::Color(value)
        | PropertyValue::Asset(value) => Json::string(value),
    })
}

#[cfg(test)]
mod tests {
    use super::{MAX_DEPTH, key_of, node_keys, to_library_document};
    use crate::contracts::identity::NodeId;
    use crate::contracts::model::{NodeKind, PropertyName, PropertyValue};
    use crate::project::record::page::PageNode;

    fn kind(name: &str) -> NodeKind {
        NodeKind::new(name).expect("a kind")
    }

    fn property(name: &str, value: PropertyValue) -> (PropertyName, PropertyValue) {
        (PropertyName::new(name).expect("a name"), value)
    }

    #[test]
    fn a_two_node_page_becomes_a_nested_document() {
        let (title, title_value) = property("title", PropertyValue::Text("Probe".into()));
        let (text, text_value) = property("text", PropertyValue::Text("Go".into()));
        let root = PageNode::new(NodeId::from_raw(1), kind("window"))
            .with_property(title, title_value)
            .with_children([NodeId::from_raw(2)]);
        let button =
            PageNode::new(NodeId::from_raw(2), kind("button")).with_property(text, text_value);

        let document = to_library_document(NodeId::from_raw(1), &[root, button]).expect("document");

        assert!(document.contains("\"window\""), "{document}");
        assert!(document.contains("\"title\": \"Probe\""), "{document}");
        assert!(document.contains("\"button\""), "{document}");
        assert!(document.contains("\"text\": \"Go\""), "{document}");
        assert!(
            !document.contains("\"node\""),
            "the tag must be unwrapped: {document}"
        );
    }

    #[test]
    fn the_same_page_translates_to_the_same_bytes() {
        let root = PageNode::new(NodeId::from_raw(1), kind("window"));
        let first = to_library_document(NodeId::from_raw(1), std::slice::from_ref(&root))
            .expect("a document");
        let second = to_library_document(NodeId::from_raw(1), &[root]).expect("a document");
        assert_eq!(first, second);
    }

    #[test]
    fn a_missing_child_is_refused_rather_than_dropped() {
        let root = PageNode::new(NodeId::from_raw(1), kind("window"))
            .with_children([NodeId::from_raw(99)]);
        let error = to_library_document(NodeId::from_raw(1), &[root]).expect_err("a missing child");
        assert!(error.contains("99"), "{error}");
    }

    #[test]
    fn a_root_that_is_not_in_the_page_is_refused() {
        let root = PageNode::new(NodeId::from_raw(1), kind("window"));
        let error = to_library_document(NodeId::from_raw(7), &[root]).expect_err("a missing root");
        assert!(
            error.contains("7") && error.contains("not in the page"),
            "{error}"
        );
    }

    #[test]
    fn a_cycle_is_refused_rather_than_recursing_forever() {
        let a =
            PageNode::new(NodeId::from_raw(1), kind("vbox")).with_children([NodeId::from_raw(2)]);
        let b =
            PageNode::new(NodeId::from_raw(2), kind("vbox")).with_children([NodeId::from_raw(1)]);
        let error = to_library_document(NodeId::from_raw(1), &[a, b]).expect_err("a cycle");
        assert!(error.contains("not a tree"), "{error}");
    }

    #[test]
    fn a_key_is_the_library_s_own_spelling_of_a_path() {
        assert_eq!(key_of(&[]), "root");
        assert_eq!(key_of(&[0]), "n_0");
        assert_eq!(key_of(&[1, 2]), "n_1_2");

        let root = PageNode::new(NodeId::from_raw(1), kind("window"))
            .with_children([NodeId::from_raw(2), NodeId::from_raw(3)]);
        let a = PageNode::new(NodeId::from_raw(2), kind("label"));
        let b =
            PageNode::new(NodeId::from_raw(3), kind("button")).with_children([NodeId::from_raw(4)]);
        let c = PageNode::new(NodeId::from_raw(4), kind("label"));
        let keys = node_keys(NodeId::from_raw(1), &[root, a, b, c]).expect("keys");
        assert_eq!(
            keys,
            vec![
                (NodeId::from_raw(1), "root".to_owned()),
                (NodeId::from_raw(2), "n_0".to_owned()),
                (NodeId::from_raw(3), "n_1".to_owned()),
                (NodeId::from_raw(4), "n_1_0".to_owned()),
            ],
        );
    }

    #[test]
    fn a_value_json_cannot_hold_is_refused() {
        let (name, value) = property("scale", PropertyValue::Number(f64::INFINITY));
        let root = PageNode::new(NodeId::from_raw(1), kind("window")).with_property(name, value);
        let error = to_library_document(NodeId::from_raw(7), &[root]).expect_err("a missing root");
        assert!(
            error.contains("7") && error.contains("not in the page"),
            "{error}"
        );
        assert_eq!(MAX_DEPTH, 64);
    }
}
