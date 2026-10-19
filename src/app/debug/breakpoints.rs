// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Breakpoints, their conditions and the binding state that says whether one is really set
//! (task T19.03).
//!
//! The acceptance line is *"breakpoints are re-mapped after a regeneration, and unsupported
//! capabilities are disabled"*. The model answers with two decisions:
//!
//! * **A breakpoint belongs to a node, not to a line.** [`Anchor::Node`] stores a stable node id and
//!   a byte offset inside that node's generated span. [`Book::remap`] recomputes the line from the
//!   new [`SourceMap`]; when the node is gone from the map the breakpoint stays **unbound** with a
//!   reason, instead of silently pointing at whatever now occupies its old line. [`Anchor::Source`]
//!   is the explicit "this generated line, and nothing else" form, used when no node mapping exists.
//! * **A request is not a binding.** [`Binding::Bound`] is set only after the adapter answers that
//!   the breakpoint was verified; a breakpoint with no answer or a rejected one is visible as
//!   [`Binding::Unbound`] or [`Binding::Rejected`]. The DAP body the book builds only carries what
//!   the adapter said it supports: applying capabilities turns a log point or hit condition the
//!   adapter cannot honour into a rejected entry rather than a request that is silently ignored.

#![allow(dead_code)]

use crate::app::debug::adapter::Capabilities;
use crate::contracts::identity::NodeId;
use crate::contracts::revision::Revision;
use crate::contracts::source_map::SourceMap;
use crate::project::record::json::{Json, JsonObject};

/// Where a breakpoint is anchored.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Anchor {
    /// A generated file and line, with no document mapping behind it.
    Source {
        /// The generated file.
        file: String,
        /// The line, counting from one.
        line: u32,
    },
    /// A stable document node and an offset inside its generated span.
    Node {
        /// The node.
        node: NodeId,
        /// The generated file the node was last mapped to.
        file: String,
        /// The byte offset inside the node's span.
        offset: u32,
    },
}

/// Whether a breakpoint is really set.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Binding {
    /// The adapter has not confirmed it, or no attempt was made.
    Unbound {
        /// Why, when there is a reason.
        reason: Option<String>,
    },
    /// The adapter verified it at a line.
    Bound {
        /// The line the adapter bound it to.
        line: u32,
    },
    /// The adapter refused it, or this build disabled it.
    Rejected {
        /// What was said.
        reason: String,
    },
}

impl Binding {
    /// Whether it is bound.
    pub fn is_bound(&self) -> bool {
        matches!(self, Self::Bound { .. })
    }

    /// The binding in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Unbound {
                reason: Some(reason),
            } => format!("unbound ({reason})"),
            Self::Unbound { reason: None } => "unbound".to_owned(),
            Self::Bound { line } => format!("bound at {line}"),
            Self::Rejected { reason } => format!("rejected ({reason})"),
        }
    }
}

/// One breakpoint.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Breakpoint {
    /// The id this session uses.
    pub id: u32,
    /// Where it is anchored.
    pub anchor: Anchor,
    /// A condition expression.
    pub condition: Option<String>,
    /// A message to log instead of stopping.
    pub log_message: Option<String>,
    /// A hit-count condition.
    pub hit_condition: Option<String>,
    /// Whether a person left it on.
    pub enabled: bool,
    /// Whether the adapter bound it.
    pub binding: Binding,
    /// The adapter's own id for it, once the adapter answered.
    pub dap_id: Option<i64>,
}

impl Breakpoint {
    /// The breakpoint in one line.
    pub fn render(&self) -> String {
        let condition = self
            .condition
            .as_ref()
            .map(|condition| format!(" if {condition}"))
            .unwrap_or_default();
        let log = self
            .log_message
            .as_ref()
            .map(|message| format!(" log {message:?}"))
            .unwrap_or_default();
        let hit = self
            .hit_condition
            .as_ref()
            .map(|condition| format!(" hit {condition}"))
            .unwrap_or_default();
        let place = match &self.anchor {
            Anchor::Source { file, line } => format!("{file}:{line}"),
            Anchor::Node { node, file, offset } => {
                format!("{file} node {} +{offset}", node.get())
            }
        };
        format!(
            "breakpoint {} at {place}{condition}{log}{hit}: {}{}",
            self.id,
            if self.enabled { "" } else { "disabled; " },
            self.binding.render()
        )
    }
}

/// One breakpoint as the protocol wants it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PointArg {
    /// The line, counting from one.
    pub line: u32,
    /// Its condition.
    pub condition: Option<String>,
    /// Its log message.
    pub log_message: Option<String>,
    /// Its hit condition.
    pub hit_condition: Option<String>,
}

/// What a remap changed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Remap {
    /// The breakpoint.
    pub id: u32,
    /// Whether it is still bound-able.
    pub mapped: bool,
    /// What happened, in one line.
    pub detail: String,
}

/// The breakpoints of one session.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Book {
    next: u32,
    points: Vec<Breakpoint>,
}

impl Book {
    /// An empty book.
    pub fn new() -> Self {
        Self {
            next: 1,
            points: Vec::new(),
        }
    }

    /// Every breakpoint.
    pub fn points(&self) -> &[Breakpoint] {
        &self.points
    }

    /// One breakpoint by id.
    pub fn get(&self, id: u32) -> Option<&Breakpoint> {
        self.points.iter().find(|point| point.id == id)
    }

    fn get_mut(&mut self, id: u32) -> Result<&mut Breakpoint, String> {
        self.points
            .iter_mut()
            .find(|point| point.id == id)
            .ok_or_else(|| format!("there is no breakpoint {id}"))
    }

    /// Adds a breakpoint at a generated source line.
    pub fn add_source(&mut self, file: impl Into<String>, line: u32) -> Result<u32, String> {
        if line == 0 {
            return Err("a breakpoint line counts from one".to_owned());
        }
        Ok(self.push(Anchor::Source {
            file: file.into(),
            line,
        }))
    }

    /// Adds a breakpoint anchored to a document node and a byte offset inside it.
    pub fn add_node(
        &mut self,
        node: NodeId,
        file: impl Into<String>,
        offset: u32,
    ) -> Result<u32, String> {
        Ok(self.push(Anchor::Node {
            node,
            file: file.into(),
            offset,
        }))
    }

    fn push(&mut self, anchor: Anchor) -> u32 {
        let id = self.next;
        self.next += 1;
        self.points.push(Breakpoint {
            id,
            anchor,
            condition: None,
            log_message: None,
            hit_condition: None,
            enabled: true,
            binding: Binding::Unbound { reason: None },
            dap_id: None,
        });
        id
    }

    /// Sets a condition, or clears it with an empty expression.
    pub fn condition(&mut self, id: u32, condition: &str) -> Result<(), String> {
        let point = self.get_mut(id)?;
        point.condition = (!condition.is_empty()).then(|| condition.to_owned());
        Ok(())
    }

    /// Sets a log message, or clears it with an empty one.
    pub fn log_message(&mut self, id: u32, message: &str) -> Result<(), String> {
        let point = self.get_mut(id)?;
        point.log_message = (!message.is_empty()).then(|| message.to_owned());
        Ok(())
    }

    /// Sets a hit condition, or clears it with an empty one.
    pub fn hit_condition(&mut self, id: u32, condition: &str) -> Result<(), String> {
        let point = self.get_mut(id)?;
        point.hit_condition = (!condition.is_empty()).then(|| condition.to_owned());
        Ok(())
    }

    /// Turns one breakpoint on or off.
    pub fn set_enabled(&mut self, id: u32, enabled: bool) -> Result<(), String> {
        let point = self.get_mut(id)?;
        point.enabled = enabled;
        if !enabled {
            point.binding = Binding::Unbound {
                reason: Some("disabled".to_owned()),
            };
        }
        Ok(())
    }

    /// Turns every breakpoint off.
    pub fn disable_all(&mut self) {
        for point in &mut self.points {
            point.enabled = false;
            point.binding = Binding::Unbound {
                reason: Some("disabled".to_owned()),
            };
        }
    }

    /// Turns every breakpoint on.
    pub fn enable_all(&mut self) {
        for point in &mut self.points {
            point.enabled = true;
        }
    }

    /// Removes one breakpoint.
    pub fn remove(&mut self, id: u32) -> Result<(), String> {
        let before = self.points.len();
        self.points.retain(|point| point.id != id);
        if self.points.len() == before {
            return Err(format!("there is no breakpoint {id}"));
        }
        Ok(())
    }

    /// Removes every breakpoint.
    pub fn clear(&mut self) {
        self.points.clear();
    }

    /// Applies an adapter's capabilities, disabling what it cannot honour.
    ///
    /// A log point on an adapter without log points, or a hit condition without hit conditions, is
    /// turned into a rejected entry with its reason. It is not sent and not silently ignored.
    pub fn apply_capabilities(&mut self, capabilities: &Capabilities) {
        for point in &mut self.points {
            let (supported, reason) = if point.log_message.is_some() && !capabilities.logpoints {
                (false, "the adapter does not support log points")
            } else if point.hit_condition.is_some() && !capabilities.hit_conditions {
                (false, "the adapter does not support hit conditions")
            } else if point.condition.is_some() && !capabilities.conditional_breakpoints {
                (
                    false,
                    "the adapter does not support conditional breakpoints",
                )
            } else {
                (true, "")
            };
            if !supported {
                point.enabled = false;
                point.binding = Binding::Rejected {
                    reason: reason.to_owned(),
                };
            }
        }
    }

    /// Records what the adapter answered for a breakpoint.
    ///
    /// `verified: false` is `Unbound`, not `Rejected`: DAP allows an adapter to report a breakpoint
    /// as pending and verify it later with a `breakpoint` event. Rejection is reserved for this build
    /// disabling a capability or the adapter refusing the request outright.
    pub fn record(&mut self, id: u32, verified: bool, line: Option<u32>, message: Option<&str>) {
        if let Ok(point) = self.get_mut(id) {
            point.binding = if verified {
                Binding::Bound {
                    line: line.unwrap_or(match &point.anchor {
                        Anchor::Source { line, .. } => *line,
                        Anchor::Node { .. } => 0,
                    }),
                }
            } else {
                Binding::Unbound {
                    reason: Some(
                        message
                            .unwrap_or("the adapter has not verified it yet")
                            .to_owned(),
                    ),
                }
            };
        }
    }

    /// Remembers the adapter's id for a breakpoint.
    ///
    /// DAP reports later binding changes (`breakpoint` events) by that id, not by the line the
    /// client asked for, so the two have to be joined here.
    pub fn remember_dap_id(&mut self, id: u32, dap_id: i64) {
        if let Ok(point) = self.get_mut(id) {
            point.dap_id = Some(dap_id);
        }
    }

    /// Applies a `breakpoint` event that carries the adapter's id.
    ///
    /// An unverified event does not demote a breakpoint the adapter already bound: adapters emit a
    /// pending event when the breakpoint is created and a verified one when the module loads, and
    /// the pending event can arrive after the answer that already bound it.
    pub fn record_event(&mut self, dap_id: i64, verified: bool, line: Option<u32>) {
        if let Some(point) = self
            .points
            .iter_mut()
            .find(|point| point.dap_id == Some(dap_id))
        {
            point.binding = if verified {
                Binding::Bound {
                    line: line.unwrap_or(0),
                }
            } else if point.binding.is_bound() {
                point.binding.clone()
            } else {
                Binding::Unbound {
                    reason: Some("the adapter has not verified it yet".to_owned()),
                }
            };
        }
    }

    /// The arguments for one file, in id order, for enabled breakpoints only.
    pub fn arguments_for(&self, file: &str) -> Vec<PointArg> {
        self.points
            .iter()
            .filter(|point| point.enabled)
            .filter_map(|point| match &point.anchor {
                Anchor::Source {
                    file: point_file,
                    line,
                } if point_file == file => Some(PointArg {
                    line: *line,
                    condition: point.condition.clone(),
                    log_message: point.log_message.clone(),
                    hit_condition: point.hit_condition.clone(),
                }),
                Anchor::Node {
                    file: point_file, ..
                } if point_file == file => match &point.binding {
                    Binding::Bound { line } if *line > 0 => Some(PointArg {
                        line: *line,
                        condition: point.condition.clone(),
                        log_message: point.log_message.clone(),
                        hit_condition: point.hit_condition.clone(),
                    }),
                    _ => None,
                },
                _ => None,
            })
            .collect()
    }

    /// Every file that has at least one enabled breakpoint.
    pub fn files(&self) -> Vec<String> {
        let mut files: Vec<String> = self
            .points
            .iter()
            .filter(|point| point.enabled)
            .map(|point| match &point.anchor {
                Anchor::Source { file, .. } | Anchor::Node { file, .. } => file.clone(),
            })
            .collect();
        files.sort();
        files.dedup();
        files
    }

    /// The DAP `setBreakpoints` body for one file.
    pub fn set_breakpoints_body(&self, file: &str) -> Json {
        let points: Vec<Json> = self
            .arguments_for(file)
            .into_iter()
            .map(|point| {
                let mut object = JsonObject::new();
                object.insert("line", Json::number(u64::from(point.line)));
                if let Some(condition) = point.condition {
                    object.insert("condition", Json::string(condition));
                }
                if let Some(message) = point.log_message {
                    object.insert("logMessage", Json::string(message));
                }
                if let Some(condition) = point.hit_condition {
                    object.insert("hitCondition", Json::string(condition));
                }
                Json::Object(object)
            })
            .collect();
        let mut source = JsonObject::new();
        source.insert("path", Json::string(file));
        let mut body = JsonObject::new();
        body.insert("source", Json::Object(source));
        body.insert("breakpoints", Json::Array(points));
        Json::Object(body)
    }

    /// Recomputes node-anchored breakpoints from a new source map and generated text.
    ///
    /// A stale map is refused as a whole: mapping a breakpoint through a map built for another model
    /// revision is exactly the wrong-but-plausible jump the acceptance line rules out.
    pub fn remap(&mut self, map: &SourceMap, current: Revision, source_text: &str) -> Vec<Remap> {
        let stale = map.staleness(current);
        let mut changes = Vec::new();
        for point in &mut self.points {
            let Anchor::Node { node, offset, .. } = &point.anchor else {
                continue;
            };
            if !stale.is_current() {
                point.binding = Binding::Unbound {
                    reason: Some(format!("the source map is {stale}")),
                };
                changes.push(Remap {
                    id: point.id,
                    mapped: false,
                    detail: format!("unbound: the source map is {stale}"),
                });
                continue;
            }
            let Some(span) = map.span_of(*node) else {
                point.binding = Binding::Unbound {
                    reason: Some("the node is not in the generated source".to_owned()),
                };
                changes.push(Remap {
                    id: point.id,
                    mapped: false,
                    detail: "unbound: the node is not in the new map".to_owned(),
                });
                continue;
            };
            let start = span.start() + (*offset).min(span.len().saturating_sub(1));
            let (line, _) = line_of(source_text, start);
            point.anchor = Anchor::Node {
                node: *node,
                file: span.file().to_owned(),
                offset: *offset,
            };
            point.binding = Binding::Bound { line };
            changes.push(Remap {
                id: point.id,
                mapped: true,
                detail: format!("mapped to {} line {line}", span.file()),
            });
        }
        changes
    }
}

/// The one-based line and column of a byte offset.
pub fn line_of(source: &str, offset: u32) -> (u32, u32) {
    let offset = offset as usize;
    let mut line = 1_u32;
    let mut column = 1_u32;
    for (index, character) in source.char_indices() {
        if index >= offset {
            break;
        }
        if character == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    (line, column)
}

#[cfg(test)]
mod tests {
    use super::{Binding, Book, line_of};
    use crate::app::debug::adapter::Capabilities;
    use crate::contracts::identity::NodeId;
    use crate::contracts::revision::{GeneratorVersion, Revision};
    use crate::contracts::source_map::{SourceMap, SourceSpan};

    fn caps() -> Capabilities {
        Capabilities {
            launch: true,
            attach: true,
            conditional_breakpoints: true,
            logpoints: true,
            hit_conditions: true,
            evaluate: true,
            restart: false,
        }
    }

    #[test]
    fn a_breakpoint_line_counts_from_one() {
        let mut book = Book::new();
        assert!(book.add_source("ui.rs", 0).is_err());
        let id = book.add_source("ui.rs", 4).expect("a breakpoint");
        assert_eq!(
            book.get(id).expect("the point").binding,
            Binding::Unbound { reason: None }
        );
    }

    #[test]
    fn a_verified_breakpoint_is_bound_and_a_pending_one_stays_unbound() {
        let mut book = Book::new();
        let bound = book.add_source("ui.rs", 4).expect("a breakpoint");
        let pending = book.add_source("ui.rs", 9).expect("a breakpoint");
        book.record(bound, true, Some(4), None);
        book.record(pending, false, None, Some("the module is not loaded"));
        assert_eq!(
            book.get(bound).expect("the point").binding,
            Binding::Bound { line: 4 }
        );
        assert!(matches!(
            book.get(pending).expect("the point").binding,
            Binding::Unbound { .. }
        ));
        assert!(
            book.get(pending)
                .expect("the point")
                .render()
                .contains("not loaded")
        );
    }

    #[test]
    fn a_verified_event_binds_a_breakpoint_and_an_unverified_one_does_not_demote_it() {
        let mut book = Book::new();
        let id = book.add_source("ui.rs", 7).expect("a breakpoint");
        book.remember_dap_id(id, 12);
        book.record_event(12, false, Some(8));
        assert!(matches!(
            book.get(id).expect("the point").binding,
            Binding::Unbound { .. }
        ));
        book.record_event(12, true, Some(8));
        assert_eq!(
            book.get(id).expect("the point").binding,
            Binding::Bound { line: 8 }
        );
        book.record_event(12, false, Some(8));
        assert_eq!(
            book.get(id).expect("the point").binding,
            Binding::Bound { line: 8 }
        );
    }

    #[test]
    fn the_dap_body_carries_conditions_and_log_messages() {
        let mut book = Book::new();
        let id = book.add_source("ui.rs", 4).expect("a breakpoint");
        book.condition(id, "x > 3").expect("a condition");
        book.log_message(id, "x is {x}").expect("a log message");
        let body = book.set_breakpoints_body("ui.rs");
        let text = body.render_compact();
        assert!(text.contains("\"condition\":\"x > 3\""), "{text}");
        assert!(text.contains("\"logMessage\":\"x is {x}\""), "{text}");
        assert_eq!(book.files(), vec!["ui.rs".to_owned()]);
    }

    #[test]
    fn capabilities_disable_what_an_adapter_cannot_honour() {
        let mut book = Book::new();
        let log = book.add_source("ui.rs", 1).expect("a breakpoint");
        book.log_message(log, "hello").expect("a log message");
        let plain = book.add_source("ui.rs", 2).expect("a breakpoint");
        book.apply_capabilities(&Capabilities {
            logpoints: false,
            ..caps()
        });
        assert!(matches!(
            book.get(log).expect("the point").binding,
            Binding::Rejected { .. }
        ));
        assert!(!book.get(log).expect("the point").enabled);
        assert!(book.get(plain).expect("the point").enabled);
    }

    #[test]
    fn regeneration_moves_a_node_breakpoint_to_its_new_line() {
        let node = NodeId::from_raw(7);
        let mut before = SourceMap::new(GeneratorVersion::new(1, 0, 0), Revision::from_raw(3));
        before.insert(node, SourceSpan::new("ui.rs", 0, 20).expect("a span"));
        let mut book = Book::new();
        let id = book.add_node(node, "ui.rs", 2).expect("a breakpoint");
        // The new source puts the node after three lines of imports.
        let source = "// one\n// two\n// three\nlet x = 1;\n";
        let mut after = SourceMap::new(GeneratorVersion::new(1, 0, 0), Revision::from_raw(4));
        after.insert(node, SourceSpan::new("ui.rs", 24, 44).expect("a span"));
        let changes = book.remap(&after, Revision::from_raw(4), source);
        assert_eq!(changes.len(), 1);
        assert!(changes[0].mapped);
        assert_eq!(
            book.get(id).expect("the point").binding,
            Binding::Bound { line: 4 }
        );
    }

    #[test]
    fn a_missing_node_goes_unbound_instead_of_guessing_a_nearby_line() {
        let node = NodeId::from_raw(7);
        let mut book = Book::new();
        let id = book.add_node(node, "ui.rs", 0).expect("a breakpoint");
        let map = SourceMap::new(GeneratorVersion::new(1, 0, 0), Revision::from_raw(4));
        let changes = book.remap(&map, Revision::from_raw(4), "let x = 1;\n");
        assert!(!changes[0].mapped);
        assert!(changes[0].detail.contains("not in the new map"));
        assert!(matches!(
            book.get(id).expect("the point").binding,
            Binding::Unbound { .. }
        ));
    }

    #[test]
    fn a_stale_map_is_refused_rather_than_used() {
        let node = NodeId::from_raw(7);
        let mut map = SourceMap::new(GeneratorVersion::new(1, 0, 0), Revision::from_raw(3));
        map.insert(node, SourceSpan::new("ui.rs", 0, 10).expect("a span"));
        let mut book = Book::new();
        let id = book.add_node(node, "ui.rs", 0).expect("a breakpoint");
        let changes = book.remap(&map, Revision::from_raw(4), "let x = 1;\n");
        assert!(!changes[0].mapped);
        assert!(changes[0].detail.contains("stale") || changes[0].detail.contains("older"));
        assert!(matches!(
            book.get(id).expect("the point").binding,
            Binding::Unbound { .. }
        ));
    }

    #[test]
    fn offsets_convert_to_one_based_lines_and_columns() {
        let source = "one\ntwo\nthree";
        assert_eq!(line_of(source, 0), (1, 1));
        assert_eq!(line_of(source, 4), (2, 1));
        assert_eq!(line_of(source, 8), (3, 1));
        assert_eq!(line_of(source, 10), (3, 3));
    }

    #[test]
    fn disabling_everything_leaves_nothing_to_send() {
        let mut book = Book::new();
        book.add_source("ui.rs", 1).expect("a breakpoint");
        book.disable_all();
        assert!(book.arguments_for("ui.rs").is_empty());
        book.enable_all();
        assert_eq!(book.arguments_for("ui.rs").len(), 1);
    }
}
