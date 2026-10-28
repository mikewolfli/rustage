// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! A versioned capability snapshot, its diagnostics, and the markers a target switch leaves behind
//! (task T18.02).
//!
//! The acceptance line is *"an incompatible node is kept, but illegal generation is blocked"*. The
//! three parts of that sentence are three properties of this module:
//!
//! * **The snapshot is versioned.** [`Snapshot::generator`] names the build that answered, and
//!   [`stale`] compares it with the version a project pinned. A project whose pinned version differs
//!   is told so; its nodes are not changed.
//! * **Answers keep their confidence.** [`Level`] is not a boolean: [`Level::Local`] was measured
//!   under the target profile, [`Level::CrossProfile`] was measured under a different one and is
//!   therefore *unsettled* for the target, and [`Level::Unavailable`]/[`Level::Unknown`] are real
//!   refusals. [`Snapshot::unverified`] counts the middle group instead of reporting it as a pass.
//! * **A switch marks, it does not delete.** [`switch`] returns the [`Marker`]s for a new target and
//!   leaves the nodes it was given untouched, so a person can see what became incompatible and
//!   decide. [`Switch::blocks_generation`] is true for every marker except a cross-profile caveat,
//!   which is a warning rather than a refusal — the designer necessarily runs on a desktop host
//!   (principle 12).
//!
//! The answers come from the library's own factory ([`rust_widgets::designer::availability_with`]),
//! never from a list kept here: a names list would drift the first time either crate changed a
//! feature gate (principle 12/59).

#![allow(dead_code)]

use std::collections::BTreeMap;

use rust_widgets::designer::generator::availability_with;
use rust_widgets::designer::{Availability, TargetProfile};
use rust_widgets::widget::WidgetFactory;
use rust_widgets::widget::capability::designer_manifest;

use super::configuration::{PROFILES, is_profile};

/// The snapshot file's header.
pub const FORMAT: &str = "rustage-capability 1";

/// How conclusive an answer is for one name on one profile.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
    /// The name resolves under the profile it was asked about.
    Local,
    /// The name resolves in this build, but the build's profile differs from the target.
    CrossProfile,
    /// The capability table knows the name and no constructor can build it.
    Unavailable,
    /// Nothing knows the name.
    Unknown,
}

impl Level {
    /// The level's name, as the file form writes it.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::CrossProfile => "cross-profile",
            Self::Unavailable => "unavailable",
            Self::Unknown => "unknown",
        }
    }

    /// Reads a level name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "local" => Some(Self::Local),
            "cross-profile" => Some(Self::CrossProfile),
            "unavailable" => Some(Self::Unavailable),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }

    /// Reads the library's answer as a level.
    pub fn of(availability: Availability) -> Self {
        match availability {
            Availability::Local => Self::Local,
            Availability::CrossProfileCaveat => Self::CrossProfile,
            Availability::Unavailable => Self::Unavailable,
            Availability::Unknown => Self::Unknown,
        }
    }

    /// Whether generated code may construct the name at this level.
    pub fn permits_generation(&self) -> bool {
        matches!(self, Self::Local | Self::CrossProfile)
    }
}

/// The generation target a profile is built as.
///
/// The library collapses `desktop`/`tablet`/`mobile` into one target because they emit identical
/// code; this function is the single place that mapping lives, so a caller cannot invent a fourth.
pub fn target_profile(profile: &str) -> Option<TargetProfile> {
    match profile {
        "desktop" | "tablet" | "mobile" => Some(TargetProfile::Default),
        "mini" | "embedded" => Some(TargetProfile::Stripped),
        _ => None,
    }
}

/// The child capacity of a profile, from the library's own target.
pub fn capacity(profile: &str) -> Option<usize> {
    target_profile(profile).map(TargetProfile::child_capacity)
}

/// A versioned set of capability answers.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Snapshot {
    generator: String,
    built_for: String,
    protocol: u32,
    library: String,
    build_profile: String,
    triple: String,
    rows: BTreeMap<String, BTreeMap<String, Level>>,
}

impl Snapshot {
    /// An empty snapshot made by `generator` under the `built_for` profile.
    pub fn new(generator: impl Into<String>, built_for: impl Into<String>) -> Self {
        Self {
            generator: generator.into(),
            built_for: built_for.into(),
            protocol: 0,
            library: String::new(),
            build_profile: String::new(),
            triple: String::new(),
            rows: BTreeMap::new(),
        }
    }

    /// Captures the real answers for `names` on every profile.
    ///
    /// One factory answers every question: `availability` would build a fresh registry per name,
    /// and a snapshot of a real widget set is hundreds of names.
    pub fn capture(
        generator: impl Into<String>,
        built_for: impl Into<String>,
        names: &[&str],
    ) -> Self {
        let factory = WidgetFactory::new_with_defaults();
        let mut snapshot = Self::new(generator, built_for);
        // The manifest is the library's own declaration of which protocol, library build and
        // target triple these answers describe. The snapshot records it verbatim rather than
        // letting a caller supply a second, drifting set of strings.
        if let Some(manifest) = names
            .iter()
            .find_map(|name| designer_manifest(&factory, name).ok())
        {
            snapshot.protocol = manifest.protocol;
            snapshot.library = manifest.library;
            snapshot.build_profile = manifest.profile;
            snapshot.triple = manifest.triple;
        }
        for profile in PROFILES {
            let Some(target) = target_profile(profile) else {
                continue;
            };
            for name in names {
                snapshot.insert(
                    profile,
                    name,
                    Level::of(availability_with(&factory, name, target)),
                );
            }
        }
        snapshot
    }

    /// Every widget name this build's factory knows.
    ///
    /// Sorted and de-duplicated because two aliases can share one canonical name, and a file form
    /// that repeats a row would depend on registration order.
    pub fn widget_names() -> Vec<String> {
        let factory = WidgetFactory::new_with_defaults();
        let mut names: Vec<String> = factory
            .widget_names()
            .into_iter()
            .map(str::to_owned)
            .collect();
        names.sort();
        names.dedup();
        names
    }

    /// The build that answered.
    pub fn generator(&self) -> &str {
        &self.generator
    }

    /// The profile the answers were measured under.
    pub fn built_for(&self) -> &str {
        &self.built_for
    }

    /// The capability-manifest protocol the header came from.
    pub fn protocol(&self) -> u32 {
        self.protocol
    }

    /// The library build named by the manifest.
    pub fn library(&self) -> &str {
        &self.library
    }

    /// The build profile named by the manifest.
    pub fn build_profile(&self) -> &str {
        &self.build_profile
    }

    /// The target triple named by the manifest.
    pub fn triple(&self) -> &str {
        &self.triple
    }

    /// Records one answer.
    pub fn insert(&mut self, profile: &str, name: &str, level: Level) {
        self.rows
            .entry(profile.to_owned())
            .or_default()
            .insert(name.to_owned(), level);
    }

    /// One answer, or `None` when the snapshot never asked.
    pub fn level(&self, profile: &str, name: &str) -> Option<Level> {
        self.rows
            .get(profile)
            .and_then(|row| row.get(name))
            .copied()
    }

    /// How many answers the snapshot holds for a profile.
    pub fn answers(&self, profile: &str) -> usize {
        self.rows.get(profile).map(BTreeMap::len).unwrap_or(0)
    }

    /// How many answers for `profile` are cross-profile rather than measured.
    pub fn unverified(&self, profile: &str) -> usize {
        self.rows
            .get(profile)
            .map(|row| {
                row.values()
                    .filter(|level| **level == Level::CrossProfile)
                    .count()
            })
            .unwrap_or(0)
    }

    /// The snapshot written as its file form.
    pub fn encode(&self) -> String {
        let mut text = String::from(FORMAT);
        text.push('\n');
        text.push_str(&format!("generator {}\n", escape(&self.generator)));
        text.push_str(&format!("built-for {}\n", self.built_for));
        text.push_str(&format!("protocol {}\n", self.protocol));
        text.push_str(&format!("library {}\n", escape(&self.library)));
        text.push_str(&format!("build-profile {}\n", escape(&self.build_profile)));
        text.push_str(&format!("triple {}\n", escape(&self.triple)));
        for (profile, row) in &self.rows {
            for (name, level) in row {
                text.push_str(profile);
                text.push('\t');
                text.push_str(name);
                text.push('\t');
                text.push_str(level.name());
                text.push('\n');
            }
        }
        text
    }

    /// The snapshot read back from its file form.
    pub fn decode(text: &str) -> Result<Self, Fault> {
        let mut lines = text.lines();
        let header = lines.next().unwrap_or_default();
        if header.trim() != FORMAT {
            return Err(Fault::BadHeader {
                line: header.to_owned(),
            });
        }
        let generator_line = lines.next().unwrap_or_default();
        let Some(generator) = generator_line.trim().strip_prefix("generator ") else {
            return Err(Fault::BadHeader {
                line: generator_line.to_owned(),
            });
        };
        let built_for_line = lines.next().unwrap_or_default();
        let Some(built_for) = built_for_line.trim().strip_prefix("built-for ") else {
            return Err(Fault::BadHeader {
                line: built_for_line.to_owned(),
            });
        };
        let mut snapshot = Self::new(unescape(generator), built_for);
        for line in lines {
            if line.is_empty() {
                continue;
            }
            if let Some(protocol) = line.trim().strip_prefix("protocol ") {
                snapshot.protocol = protocol.parse().map_err(|_| Fault::BadHeader {
                    line: line.to_owned(),
                })?;
                continue;
            }
            if let Some(library) = line.trim().strip_prefix("library ") {
                snapshot.library = unescape(library);
                continue;
            }
            if let Some(profile) = line.trim().strip_prefix("build-profile ") {
                snapshot.build_profile = unescape(profile);
                continue;
            }
            if let Some(triple) = line.trim().strip_prefix("triple ") {
                snapshot.triple = unescape(triple);
                continue;
            }
            let parts: Vec<&str> = line.split('\t').collect();
            let [profile, name, level] = parts.as_slice() else {
                return Err(Fault::BadEntry {
                    line: line.to_owned(),
                });
            };
            if !is_profile(profile) {
                return Err(Fault::UnknownProfile {
                    profile: (*profile).to_owned(),
                });
            }
            let Some(level) = Level::parse(level) else {
                return Err(Fault::UnknownLevel {
                    level: (*level).to_owned(),
                });
            };
            snapshot.insert(profile, name, level);
        }
        Ok(snapshot)
    }

    /// What changed between `self` and `newer`.
    pub fn changes(&self, newer: &Snapshot) -> Vec<Change> {
        let mut changes = Vec::new();
        let profiles: Vec<&String> = {
            let mut profiles: Vec<&String> = self.rows.keys().chain(newer.rows.keys()).collect();
            profiles.sort();
            profiles.dedup();
            profiles
        };
        for profile in profiles {
            let empty = BTreeMap::new();
            let old_row = self.rows.get(profile).unwrap_or(&empty);
            let new_row = newer.rows.get(profile).unwrap_or(&empty);
            let names: Vec<&String> = {
                let mut names: Vec<&String> = old_row.keys().chain(new_row.keys()).collect();
                names.sort();
                names.dedup();
                names
            };
            for name in names {
                match (old_row.get(name), new_row.get(name)) {
                    (None, Some(level)) => changes.push(Change::Added {
                        profile: profile.clone(),
                        name: name.clone(),
                        level: *level,
                    }),
                    (Some(level), None) => changes.push(Change::Removed {
                        profile: profile.clone(),
                        name: name.clone(),
                        level: *level,
                    }),
                    (Some(old), Some(new)) if old != new => changes.push(Change::Changed {
                        profile: profile.clone(),
                        name: name.clone(),
                        from: *old,
                        to: *new,
                    }),
                    _ => {}
                }
            }
        }
        changes
    }
}

/// One difference between two snapshots.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Change {
    /// The newer snapshot answers for a name the older one did not ask about.
    Added {
        /// The profile.
        profile: String,
        /// The name.
        name: String,
        /// The newer answer.
        level: Level,
    },
    /// The newer snapshot no longer asks about a name.
    Removed {
        /// The profile.
        profile: String,
        /// The name.
        name: String,
        /// The older answer.
        level: Level,
    },
    /// The answer moved.
    Changed {
        /// The profile.
        profile: String,
        /// The name.
        name: String,
        /// The older answer.
        from: Level,
        /// The newer answer.
        to: Level,
    },
}

impl Change {
    /// The change in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Added {
                profile,
                name,
                level,
            } => format!("{profile}/{name}: added as {}", level.name()),
            Self::Removed {
                profile,
                name,
                level,
            } => format!("{profile}/{name}: removed, was {}", level.name()),
            Self::Changed {
                profile,
                name,
                from,
                to,
            } => format!("{profile}/{name}: {} -> {}", from.name(), to.name()),
        }
    }
}

/// Whether a project's pinned capability version differs from the build's.
///
/// A difference is a diagnostic, not a refusal: the project still describes the same design, and
/// the honest thing is to say the answers moved rather than to reject the document or to pretend
/// nothing changed.
pub fn stale(pinned: &str, current: &str) -> Option<String> {
    (pinned != current).then(|| {
        format!("the project pinned capabilities from {pinned}; this build answers from {current}")
    })
}

/// One node of a document, as a switch needs it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Node {
    /// The control kind.
    pub name: String,
    /// How many children it places, when the kind is a container.
    pub children: usize,
}

impl Node {
    /// A node with a kind and a child count.
    pub fn new(name: impl Into<String>, children: usize) -> Self {
        Self {
            name: name.into(),
            children,
        }
    }
}

/// Why a node is marked after a target switch.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Marker {
    /// The target cannot construct the kind at all.
    Unavailable {
        /// The node.
        node: String,
        /// The kind.
        kind: String,
    },
    /// Nothing in the snapshot knows the kind.
    Unknown {
        /// The node.
        node: String,
        /// The kind.
        kind: String,
    },
    /// The node has more children than the target's storage holds.
    OverCapacity {
        /// The node.
        node: String,
        /// How many it has.
        children: usize,
        /// The target's bound.
        capacity: usize,
    },
    /// The answer is not conclusive for the target, but generation is permitted.
    Caveat {
        /// The node.
        node: String,
        /// The kind.
        kind: String,
    },
}

impl Marker {
    /// The marker in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Unavailable { node, kind } => {
                format!("{node}: {kind} cannot be constructed for the new target")
            }
            Self::Unknown { node, kind } => {
                format!("{node}: {kind} is not in the capability snapshot")
            }
            Self::OverCapacity {
                node,
                children,
                capacity,
            } => format!("{node}: {children} children, the target holds {capacity}"),
            Self::Caveat { node, kind } => {
                format!("{node}: {kind} is not verified under the new target's profile")
            }
        }
    }

    /// Whether this marker blocks generation.
    pub fn blocks_generation(&self) -> bool {
        !matches!(self, Self::Caveat { .. })
    }
}

/// The markers a target switch left, with the target it moved to.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Switch {
    /// The profile switched from.
    pub from: String,
    /// The profile switched to.
    pub to: String,
    /// Every node that needs attention, in document order.
    pub markers: Vec<Marker>,
}

impl Switch {
    /// Whether any marker makes generation illegal.
    pub fn blocks_generation(&self) -> bool {
        self.markers.iter().any(Marker::blocks_generation)
    }

    /// The markers in one line each, with the switch itself first.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!("switch {} -> {}", self.from, self.to)];
        for marker in &self.markers {
            let verdict = if marker.blocks_generation() {
                "blocked"
            } else {
                "caveat"
            };
            lines.push(format!("{verdict}  {}", marker.render()));
        }
        lines
    }
}

/// Marks a document's nodes for a new target, keeping the nodes themselves.
///
/// `Ok` is a real answer even when it blocks: the markers are the switch's result, not its failure.
/// `Err` means the switch itself is impossible — an unknown profile, so there is no row to ask.
pub fn switch(snapshot: &Snapshot, nodes: &[Node], from: &str, to: &str) -> Result<Switch, String> {
    if !is_profile(from) {
        return Err(format!("{from:?} is not a target profile"));
    }
    if !is_profile(to) {
        return Err(format!("{to:?} is not a target profile"));
    }
    let Some(row) = snapshot.rows.get(to) else {
        // A snapshot that never asked about the target has no answers to give; inventing
        // `Unknown` for every node would blame the design for a gap in the snapshot.
        return Err(format!("the snapshot has no answers for {to}"));
    };
    let bound = capacity(to).unwrap_or(usize::MAX);
    let mut markers = Vec::new();
    for node in nodes {
        let level = row.get(&node.name);
        match level {
            None => markers.push(Marker::Unknown {
                node: node.name.clone(),
                kind: node.name.clone(),
            }),
            Some(&Level::Unavailable) => markers.push(Marker::Unavailable {
                node: node.name.clone(),
                kind: node.name.clone(),
            }),
            Some(&Level::Unknown) => markers.push(Marker::Unknown {
                node: node.name.clone(),
                kind: node.name.clone(),
            }),
            Some(found) => {
                if node.children > bound {
                    markers.push(Marker::OverCapacity {
                        node: node.name.clone(),
                        children: node.children,
                        capacity: bound,
                    });
                } else if *found == Level::CrossProfile {
                    markers.push(Marker::Caveat {
                        node: node.name.clone(),
                        kind: node.name.clone(),
                    });
                }
            }
        }
    }
    Ok(Switch {
        from: from.to_owned(),
        to: to.to_owned(),
        markers,
    })
}

/// Why a snapshot file could not be read.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Fault {
    /// The first line is not the snapshot header.
    BadHeader {
        /// The line that was there.
        line: String,
    },
    /// A row is not `profile<TAB>name<TAB>level`.
    BadEntry {
        /// The line that was there.
        line: String,
    },
    /// A row names a profile this build does not know.
    UnknownProfile {
        /// The profile it named.
        profile: String,
    },
    /// A row names a level this build does not know.
    UnknownLevel {
        /// The level it named.
        level: String,
    },
}

impl Fault {
    /// The fault in one line.
    pub fn message(&self) -> String {
        match self {
            Self::BadHeader { line } => {
                format!("the snapshot header is {line:?}, not `{FORMAT}`")
            }
            Self::BadEntry { line } => {
                format!("{line:?} is not <profile><TAB><name><TAB><level>")
            }
            Self::UnknownProfile { profile } => {
                format!("{profile:?} is not a target profile")
            }
            Self::UnknownLevel { level } => {
                format!("{level:?} is not a capability level")
            }
        }
    }
}

/// Escapes the two characters the file form uses as separators.
fn escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('\n', "\\n")
}

/// Reads what [`escape`] wrote.
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut characters = text.chars();
    while let Some(character) = characters.next() {
        match character {
            '\\' => match characters.next() {
                Some('n') => out.push('\n'),
                Some(other) => out.push(other),
                None => out.push('\\'),
            },
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{
        Change, Fault, Level, Marker, Node, Snapshot, capacity, stale, switch, target_profile,
    };
    use rust_widgets::designer::TargetProfile;

    #[test]
    fn a_profile_maps_to_the_library_target() {
        assert_eq!(target_profile("desktop"), Some(TargetProfile::Default));
        assert_eq!(target_profile("mobile"), Some(TargetProfile::Default));
        assert_eq!(target_profile("embedded"), Some(TargetProfile::Stripped));
        assert_eq!(target_profile("watch"), None);
        assert_eq!(
            capacity("mini"),
            Some(TargetProfile::Stripped.child_capacity())
        );
        assert!(capacity("mini") < capacity("desktop"));
    }

    #[test]
    fn a_level_round_trips_through_its_name() {
        for level in [
            Level::Local,
            Level::CrossProfile,
            Level::Unavailable,
            Level::Unknown,
        ] {
            assert_eq!(Level::parse(level.name()), Some(level));
        }
        assert_eq!(Level::parse("probably"), None);
        assert!(Level::Local.permits_generation());
        assert!(Level::CrossProfile.permits_generation());
        assert!(!Level::Unavailable.permits_generation());
    }

    #[test]
    fn capture_asks_every_profile_about_every_name() {
        let snapshot = Snapshot::capture("0.1.0", "desktop", &["button", "label"]);
        assert_eq!(snapshot.generator(), "0.1.0");
        assert_eq!(snapshot.built_for(), "desktop");
        assert!(
            snapshot.protocol() > 0,
            "the manifest header was not captured"
        );
        assert!(!snapshot.library().is_empty());
        assert!(!snapshot.build_profile().is_empty());
        assert!(!snapshot.triple().is_empty());
        for profile in super::PROFILES {
            for name in ["button", "label"] {
                assert!(
                    snapshot.level(profile, name).is_some(),
                    "{profile}/{name} was not captured"
                );
            }
        }
        assert_eq!(snapshot.level("desktop", "button"), Some(Level::Local));
        assert_eq!(
            snapshot.level("mini", "button"),
            Some(Level::CrossProfile),
            "this build is desktop, so a mini answer cannot be conclusive"
        );
        assert!(snapshot.unverified("mini") > 0);
    }

    #[test]
    fn a_snapshot_round_trips_through_its_file_form() {
        let snapshot = Snapshot::capture("0.1.0", "desktop", &["button", "not-a-widget"]);
        let decoded = Snapshot::decode(&snapshot.encode()).expect("a snapshot");
        assert_eq!(decoded, snapshot);
    }

    #[test]
    fn a_bad_header_and_a_bad_row_are_refused() {
        let fault = Snapshot::decode("rustage-capability 2\ngenerator 0.1.0\nbuilt-for desktop\n")
            .expect_err("a header");
        assert!(matches!(fault, Fault::BadHeader { .. }));
        let fault = Snapshot::decode(
            "rustage-capability 1\ngenerator 0.1.0\nbuilt-for desktop\ndesktop\tbutton\n",
        )
        .expect_err("a row");
        assert!(matches!(fault, Fault::BadEntry { .. }));
        let fault = Snapshot::decode(
            "rustage-capability 1\ngenerator 0.1.0\nbuilt-for desktop\nwatch\tbutton\tlocal\n",
        )
        .expect_err("a profile");
        assert!(matches!(fault, Fault::UnknownProfile { .. }));
        let fault = Snapshot::decode(
            "rustage-capability 1\ngenerator 0.1.0\nbuilt-for desktop\ndesktop\tbutton\tmaybe\n",
        )
        .expect_err("a level");
        assert!(matches!(fault, Fault::UnknownLevel { .. }));
    }

    #[test]
    fn a_newer_snapshot_reports_added_removed_and_changed_answers() {
        let mut older = Snapshot::new("0.1.0", "desktop");
        older.insert("desktop", "button", Level::Local);
        older.insert("desktop", "chart", Level::Local);
        let mut newer = Snapshot::new("0.2.0", "desktop");
        newer.insert("desktop", "button", Level::CrossProfile);
        newer.insert("desktop", "gauge", Level::Local);
        let changes = older.changes(&newer);
        assert!(changes.iter().any(|change| matches!(
            change,
            Change::Changed { name, .. } if name == "button"
        )));
        assert!(changes.iter().any(|change| matches!(
            change,
            Change::Removed { name, .. } if name == "chart"
        )));
        assert!(changes.iter().any(|change| matches!(
            change,
            Change::Added { name, .. } if name == "gauge"
        )));
        assert!(older.changes(&older).is_empty());
        assert!(changes.iter().all(|change| !change.render().is_empty()));
    }

    #[test]
    fn a_different_generator_version_is_stale_but_not_a_refusal() {
        assert!(stale("0.1.0", "0.1.0").is_none());
        let message = stale("0.1.0", "0.2.0").expect("a difference");
        assert!(message.contains("0.1.0") && message.contains("0.2.0"));
    }

    #[test]
    fn a_switch_keeps_the_nodes_it_marks() {
        let snapshot = Snapshot::capture("0.1.0", "desktop", &["button", "label"]);
        let nodes = vec![Node::new("button", 0), Node::new("label", 0)];
        let before = nodes.clone();
        let switched = switch(&snapshot, &nodes, "desktop", "mini").expect("a switch");
        assert_eq!(nodes, before, "the design is kept");
        assert!(
            switched
                .markers
                .iter()
                .all(|marker| matches!(marker, Marker::Caveat { .. }))
        );
        assert!(!switched.blocks_generation());
    }

    #[test]
    fn an_unknown_kind_blocks_generation_but_stays_in_the_document() {
        let snapshot = Snapshot::capture("0.1.0", "desktop", &["button"]);
        let nodes = vec![Node::new("not-a-widget", 0)];
        let switched = switch(&snapshot, &nodes, "desktop", "desktop").expect("a switch");
        assert!(switched.blocks_generation());
        assert!(matches!(switched.markers[0], Marker::Unknown { .. }));
        assert_eq!(nodes[0].name, "not-a-widget");
    }

    #[test]
    fn an_over_capacity_container_is_marked() {
        let snapshot = Snapshot::capture("0.1.0", "desktop", &["panel"]);
        let bound = capacity("mini").expect("a capacity");
        let nodes = vec![Node::new("panel", bound + 1)];
        let switched = switch(&snapshot, &nodes, "desktop", "mini").expect("a switch");
        assert!(switched.blocks_generation());
        assert!(switched.markers.iter().any(|marker| matches!(
            marker,
            Marker::OverCapacity { children, capacity, .. } if *children == bound + 1 && *capacity == bound
        )));
    }

    #[test]
    fn a_snapshot_without_the_target_is_not_used_as_a_guess() {
        let mut snapshot = Snapshot::new("0.1.0", "desktop");
        snapshot.insert("desktop", "button", Level::Local);
        assert!(switch(&snapshot, &[], "desktop", "mini").is_err());
        assert!(switch(&snapshot, &[], "watch", "desktop").is_err());
        assert!(switch(&snapshot, &[], "desktop", "watch").is_err());
    }

    #[test]
    fn a_marker_says_which_ones_block() {
        let caveat = Marker::Caveat {
            node: "a".to_owned(),
            kind: "button".to_owned(),
        };
        let unavailable = Marker::Unavailable {
            node: "a".to_owned(),
            kind: "button".to_owned(),
        };
        assert!(!caveat.blocks_generation());
        assert!(unavailable.blocks_generation());
        assert!(caveat.render().contains("not verified"));
    }
}
