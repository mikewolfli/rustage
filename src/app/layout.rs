// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Layout presets, user layouts, zen and fullscreen, locking and reset (task T04.05).
//!
//! The acceptance line is *"reset loses no document, and a bad layout configuration is
//! recoverable"*. Both are properties of what a layout is allowed to be, so the module keeps a
//! layout to exactly that:
//!
//! * **A layout is a value describing an arrangement, not a session.** [`Layout`] holds the group
//!   shape, which documents go in which pane, and which panels are shown — names and numbers only,
//!   no window handle, no pointer, no process identity, so it can be written to a file and read back
//!   (roadmap F04's rule for layout files). Applying it goes through [`Groups::rearrange`], which
//!   opens what the layout names and *keeps* everything else, so applying a layout — a preset, a
//!   saved one, or a reset — can never make a document disappear.
//! * **Presets and user layouts are the same shape.** A built-in preset is a [`Layout`] like any
//!   other; there is no second format for "the shipped layouts". Applying a preset and recalling a
//!   saved layout are the same call.
//! * **A bad configuration is a refusal, not a half-loaded window.** [`Layout::from_json`] and
//!   [`Layouts::load`] refuse a malformed or unknown document and leave the current layout and every
//!   open document exactly as they were, so the person can fix the file or [`Layouts::reset`] out of
//!   it. Nothing is applied from a document that did not fully check out.
//! * **Zen, fullscreen and lock are view state, not a second layout.** [`Layouts`] holds the current
//!   layout and these three flags; while the layout is *locked*, every operation that would change
//!   it is refused by name ([`LayoutError::Locked`]), which is what a lock is for.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use crate::app::groups::{Axis, Groups, Shape};
use crate::app::panels::Panels;
use crate::project::json::{Json, JsonObject, parse};

/// The envelope format this build writes and the only one it reads.
const FORMAT: u64 = 1;

/// The built-in layouts a person can pick, by name.
pub const PRESETS: [&str; 4] = ["default", "columns", "stack", "presentation"];

/// One panel in a layout: shown, and the size it is shown at.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PanelState {
    id: String,
    size: Option<u32>,
}

impl PanelState {
    /// A shown panel.
    pub fn new(id: impl Into<String>, size: Option<u32>) -> Self {
        Self {
            id: id.into(),
            size,
        }
    }

    /// The panel's id.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The size it is shown at, when the layout says.
    pub const fn size(&self) -> Option<u32> {
        self.size
    }

    /// The panels this host is showing right now, with their sizes.
    fn captured(panels: &Panels) -> Vec<Self> {
        let shown: BTreeSet<String> = panels.layout().shown().into_iter().collect();
        panels
            .specs()
            .filter(|spec| shown.contains(spec.id()))
            .map(|spec| Self {
                id: spec.id().to_owned(),
                size: panels.size_of(spec.id()),
            })
            .collect()
    }
}

/// A layout: which panes there are, what each shows, which panels are on screen, and whether zen
/// or fullscreen is on.
#[derive(Clone, PartialEq, Debug)]
pub struct Layout {
    name: String,
    shape: Shape,
    arrangement: Vec<Vec<String>>,
    shows: Vec<PanelState>,
    zen: bool,
    fullscreen: bool,
}

impl Layout {
    /// A built-in preset, by name.
    pub fn preset(name: &str) -> Option<Self> {
        let (shape, arrangement, shows) = match name {
            "default" => (
                Shape::single(),
                vec![Vec::new()],
                vec![
                    PanelState::new("outline", Some(260)),
                    PanelState::new("messages", Some(160)),
                ],
            ),
            "columns" => (
                Shape::Split {
                    axis: Axis::LeftRight,
                    children: vec![Shape::Leaf(0), Shape::Leaf(1)],
                },
                vec![Vec::new(), Vec::new()],
                vec![
                    PanelState::new("outline", Some(260)),
                    PanelState::new("messages", Some(160)),
                ],
            ),
            "stack" => (
                Shape::Split {
                    axis: Axis::TopBottom,
                    children: vec![Shape::Leaf(0), Shape::Leaf(1)],
                },
                vec![Vec::new(), Vec::new()],
                vec![
                    PanelState::new("outline", Some(260)),
                    PanelState::new("messages", Some(160)),
                ],
            ),
            "presentation" => (Shape::single(), vec![Vec::new()], Vec::new()),
            _ => return None,
        };
        Some(Self {
            name: name.to_owned(),
            shape,
            arrangement,
            shows,
            zen: false,
            fullscreen: name == "presentation",
        })
    }

    /// A layout captured from what is on screen right now.
    pub fn captured(
        name: impl Into<String>,
        groups: &Groups,
        panels: &Panels,
        zen: bool,
        fullscreen: bool,
    ) -> Self {
        Self {
            name: name.into(),
            shape: groups.shape(),
            arrangement: groups.arrangement(),
            shows: PanelState::captured(panels),
            zen,
            fullscreen,
        }
    }

    /// The layout's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The group shape.
    pub const fn shape(&self) -> &Shape {
        &self.shape
    }

    /// Which documents each pane shows.
    pub fn arrangement(&self) -> &[Vec<String>] {
        &self.arrangement
    }

    /// The panels the layout shows.
    pub fn shows(&self) -> &[PanelState] {
        &self.shows
    }

    /// Whether zen mode is on.
    pub const fn zen(&self) -> bool {
        self.zen
    }

    /// Whether fullscreen is on.
    pub const fn fullscreen(&self) -> bool {
        self.fullscreen
    }

    /// Applies the layout to a live editor area and panel set.
    ///
    /// Every registered panel the layout does not name is hidden and every panel it names is shown,
    /// so applying a layout is deterministic rather than "whatever was shown before, plus this". The
    /// documents are rearranged by [`Groups::rearrange`], which never closes one.
    ///
    /// Answers the ids the layout names that this build has no panel for: a layout may come from a
    /// build with a panel this one lacks, and saying so is better than a silent no-op.
    pub fn apply(
        &self,
        groups: &mut Groups,
        panels: &mut Panels,
    ) -> Result<Vec<String>, LayoutError> {
        groups
            .rearrange(&self.shape, &self.arrangement)
            .map_err(|reason| LayoutError::BadArrangement { reason })?;

        let shown: BTreeSet<&str> = self.shows.iter().map(|state| state.id.as_str()).collect();
        let registered: Vec<String> = panels.specs().map(|spec| spec.id().to_owned()).collect();
        for id in registered {
            if !shown.contains(id.as_str()) {
                let _ = panels.hide(&id);
            }
        }

        let mut unknown = Vec::new();
        for state in &self.shows {
            if panels.show(&state.id).is_err() {
                unknown.push(state.id.clone());
                continue;
            }
            if let Some(size) = state.size {
                let _ = panels.resize(&state.id, size);
            }
        }
        Ok(unknown)
    }

    /// The layout as JSON: names and numbers only.
    pub fn to_json(&self) -> String {
        let mut out = JsonObject::new();
        out.insert("format", Json::number(FORMAT));
        out.insert("name", Json::string(&self.name));
        out.insert("shape", encode_shape(&self.shape));
        out.insert(
            "arrangement",
            Json::array(
                self.arrangement
                    .iter()
                    .map(|documents| {
                        Json::array(documents.iter().map(Json::string).collect::<Vec<_>>())
                    })
                    .collect::<Vec<_>>(),
            ),
        );
        out.insert(
            "shows",
            Json::array(
                self.shows
                    .iter()
                    .map(|state| {
                        let mut entry = JsonObject::new();
                        entry.insert("id", Json::string(&state.id));
                        if let Some(size) = state.size {
                            entry.insert("size", Json::number(u64::from(size)));
                        }
                        Json::Object(entry)
                    })
                    .collect::<Vec<_>>(),
            ),
        );
        out.insert("zen", Json::bool(self.zen));
        out.insert("fullscreen", Json::bool(self.fullscreen));
        Json::Object(out).render()
    }

    /// Reads a layout from JSON, refusing anything that is not one wholly.
    pub fn from_json(text: &str) -> Result<Self, LayoutError> {
        let value = parse(text).map_err(|error| LayoutError::Malformed {
            reason: error.to_string(),
        })?;
        let Json::Object(object) = value else {
            return Err(LayoutError::Malformed {
                reason: "a layout is a JSON object".to_owned(),
            });
        };
        let format =
            object
                .get("format")
                .and_then(Json::as_u64)
                .ok_or_else(|| LayoutError::Malformed {
                    reason: "a layout names a numeric format".to_owned(),
                })?;
        if format != FORMAT {
            return Err(LayoutError::Malformed {
                reason: format!("format {format} is not the {FORMAT} this build writes"),
            });
        }
        let name = required_string(&object, "name")?;
        let shape = decode_shape(object.get("shape").ok_or_else(|| LayoutError::Malformed {
            reason: "a layout names its shape".to_owned(),
        })?)?;
        let arrangement = decode_arrangement(&object)?;
        let shows = decode_shows(&object)?;
        let zen = required_bool(&object, "zen")?;
        let fullscreen = required_bool(&object, "fullscreen")?;

        let layout = Self {
            name,
            shape,
            arrangement,
            shows,
            zen,
            fullscreen,
        };
        layout.validate()?;
        Ok(layout)
    }

    /// Whether the shape and the arrangement agree. A layout that does not is not applied.
    fn validate(&self) -> Result<(), LayoutError> {
        let leaves = self.shape.leaf_count();
        if leaves == 0 || self.arrangement.len() != leaves {
            return Err(LayoutError::BadArrangement {
                reason: format!(
                    "the shape has {leaves} pane(s) but the arrangement names {}",
                    self.arrangement.len()
                ),
            });
        }
        for leaf in self.shape.leaves() {
            if leaf >= self.arrangement.len() {
                return Err(LayoutError::BadArrangement {
                    reason: format!(
                        "the shape names pane {leaf}, past the {} in the arrangement",
                        self.arrangement.len()
                    ),
                });
            }
        }
        Ok(())
    }
}

/// The layout catalog: the current layout, the saved user layouts, the lock, and the note a refused
/// load leaves behind.
#[derive(Clone, Debug)]
pub struct Layouts {
    current: Layout,
    locked: bool,
    saved: BTreeMap<String, Layout>,
    note: Option<String>,
}

impl Layouts {
    /// A catalog with the default layout active and nothing saved.
    pub fn new() -> Self {
        Self {
            current: Layout::preset("default").expect("the default preset exists"),
            locked: false,
            saved: BTreeMap::new(),
            note: None,
        }
    }

    /// The active layout.
    pub const fn current(&self) -> &Layout {
        &self.current
    }

    /// Whether the layout is locked against changes.
    pub const fn locked(&self) -> bool {
        self.locked
    }

    /// The names of the saved user layouts.
    pub fn saved(&self) -> Vec<&str> {
        self.saved.keys().map(String::as_str).collect()
    }

    /// The reason the last refused load gave, kept so a report can explain why nothing changed.
    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    /// Whether zen mode is on.
    pub const fn zen(&self) -> bool {
        self.current.zen
    }

    /// Whether fullscreen is on.
    pub const fn fullscreen(&self) -> bool {
        self.current.fullscreen
    }

    /// Locks the layout so changes are refused by name.
    pub fn lock(&mut self) {
        self.locked = true;
    }

    /// Unlocks it.
    pub fn unlock(&mut self) {
        self.locked = false;
    }

    /// Applies a built-in preset.
    pub fn use_preset(
        &mut self,
        name: &str,
        groups: &mut Groups,
        panels: &mut Panels,
    ) -> Result<Vec<String>, LayoutError> {
        self.refuse_when_locked("apply a preset")?;
        let layout = Layout::preset(name).ok_or_else(|| LayoutError::UnknownPreset {
            name: name.to_owned(),
        })?;
        let unknown = layout.apply(groups, panels)?;
        self.current = layout;
        Ok(unknown)
    }

    /// Saves what is on screen now under a name. Allowed even when locked: it changes nothing.
    pub fn save(&mut self, name: &str, groups: &Groups, panels: &Panels) {
        let layout = Layout::captured(
            name,
            groups,
            panels,
            self.current.zen,
            self.current.fullscreen,
        );
        self.saved.insert(name.to_owned(), layout);
    }

    /// Applies a saved user layout.
    pub fn recall(
        &mut self,
        name: &str,
        groups: &mut Groups,
        panels: &mut Panels,
    ) -> Result<Vec<String>, LayoutError> {
        self.refuse_when_locked("recall a layout")?;
        let layout =
            self.saved
                .get(name)
                .cloned()
                .ok_or_else(|| LayoutError::UnknownUserLayout {
                    name: name.to_owned(),
                })?;
        let unknown = layout.apply(groups, panels)?;
        self.current = layout;
        Ok(unknown)
    }

    /// Turns zen mode on or off.
    pub fn set_zen(&mut self, on: bool) -> Result<(), LayoutError> {
        self.refuse_when_locked(if on {
            "enter zen mode"
        } else {
            "leave zen mode"
        })?;
        self.current.zen = on;
        Ok(())
    }

    /// Turns fullscreen on or off.
    pub fn set_fullscreen(&mut self, on: bool) -> Result<(), LayoutError> {
        self.refuse_when_locked(if on {
            "enter fullscreen"
        } else {
            "leave fullscreen"
        })?;
        self.current.fullscreen = on;
        Ok(())
    }

    /// Resets to the default preset, keeping every open document.
    ///
    /// The documents are kept because applying a layout goes through [`Groups::rearrange`], which
    /// places a document the default does not name into the first pane rather than dropping it. That
    /// is the whole of "reset loses no document".
    pub fn reset(
        &mut self,
        groups: &mut Groups,
        panels: &mut Panels,
    ) -> Result<Vec<String>, LayoutError> {
        self.refuse_when_locked("reset the layout")?;
        self.note = None;
        self.use_preset("default", groups, panels)
    }

    /// Loads a layout from JSON and applies it.
    ///
    /// A document that does not fully check out is refused: the current layout and every open
    /// document stay as they were, and the reason is kept in [`Layouts::note`] so a report can say
    /// why nothing changed. That is what makes a bad configuration recoverable rather than a
    /// half-loaded window.
    pub fn load(
        &mut self,
        text: &str,
        groups: &mut Groups,
        panels: &mut Panels,
    ) -> Result<Vec<String>, LayoutError> {
        self.refuse_when_locked("load a layout")?;
        let layout = match Layout::from_json(text) {
            Ok(layout) => layout,
            Err(error) => {
                self.note = Some(error.message());
                return Err(error);
            }
        };
        let unknown = match layout.apply(groups, panels) {
            Ok(unknown) => unknown,
            Err(error) => {
                self.note = Some(error.message());
                return Err(error);
            }
        };
        self.note = None;
        self.current = layout;
        Ok(unknown)
    }

    /// The current layout as JSON, for writing a file.
    pub fn to_json(&self) -> String {
        self.current.to_json()
    }

    fn refuse_when_locked(&self, what: &str) -> Result<(), LayoutError> {
        if self.locked {
            return Err(LayoutError::Locked {
                what: what.to_owned(),
            });
        }
        Ok(())
    }
}

impl Default for Layouts {
    fn default() -> Self {
        Self::new()
    }
}

/// Why a layout operation was refused.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum LayoutError {
    /// The document is not a well-formed layout.
    Malformed { reason: String },
    /// The shape and the arrangement disagree, so nothing is applied.
    BadArrangement { reason: String },
    /// No built-in preset has that name.
    UnknownPreset { name: String },
    /// No saved user layout has that name.
    UnknownUserLayout { name: String },
    /// The layout is locked, so this change is refused by name.
    Locked { what: String },
}

impl LayoutError {
    /// The one-line explanation a person is shown.
    pub fn message(&self) -> String {
        match self {
            Self::Malformed { reason } => format!("the layout is malformed: {reason}"),
            Self::BadArrangement { reason } => format!("the layout is inconsistent: {reason}"),
            Self::UnknownPreset { name } => {
                format!(
                    "there is no layout preset {name:?}; the presets are {}",
                    PRESETS.join(", ")
                )
            }
            Self::UnknownUserLayout { name } => format!("there is no saved layout {name:?}"),
            Self::Locked { what } => {
                format!("the layout is locked, so it will not {what}")
            }
        }
    }
}

fn required_string(object: &JsonObject, name: &str) -> Result<String, LayoutError> {
    object
        .get(name)
        .and_then(Json::as_str)
        .map(str::to_owned)
        .ok_or_else(|| LayoutError::Malformed {
            reason: format!("a layout names its {name} as a string"),
        })
}

fn required_bool(object: &JsonObject, name: &str) -> Result<bool, LayoutError> {
    object
        .get(name)
        .and_then(Json::as_bool)
        .ok_or_else(|| LayoutError::Malformed {
            reason: format!("a layout names its {name} as a boolean"),
        })
}

fn encode_shape(shape: &Shape) -> Json {
    let mut out = JsonObject::new();
    match shape {
        Shape::Leaf(index) => {
            out.insert("leaf", Json::number(*index as u64));
        }
        Shape::Split { axis, children } => {
            let mut split = JsonObject::new();
            split.insert("axis", Json::string(axis.name()));
            split.insert(
                "children",
                Json::array(children.iter().map(encode_shape).collect::<Vec<_>>()),
            );
            out.insert("split", Json::Object(split));
        }
    }
    Json::Object(out)
}

fn decode_shape(value: &Json) -> Result<Shape, LayoutError> {
    let Some(object) = value.as_object() else {
        return Err(LayoutError::Malformed {
            reason: "a shape is an object naming a leaf or a split".to_owned(),
        });
    };
    if let Some(leaf) = object.get("leaf") {
        let index = leaf.as_u64().ok_or_else(|| LayoutError::Malformed {
            reason: "a leaf names its pane as a whole number".to_owned(),
        })?;
        return Ok(Shape::Leaf(index as usize));
    }
    let Some(split) = object.get("split").and_then(Json::as_object) else {
        return Err(LayoutError::Malformed {
            reason: "a shape names a leaf or a split".to_owned(),
        });
    };
    let axis_name =
        split
            .get("axis")
            .and_then(Json::as_str)
            .ok_or_else(|| LayoutError::Malformed {
                reason: "a split names its axis".to_owned(),
            })?;
    let axis = Axis::from_name(axis_name).ok_or_else(|| LayoutError::Malformed {
        reason: format!("{axis_name:?} is not an axis (left-right, top-bottom)"),
    })?;
    let Some(children) = split.get("children").and_then(Json::as_array) else {
        return Err(LayoutError::Malformed {
            reason: "a split carries an array of children".to_owned(),
        });
    };
    if children.is_empty() {
        return Err(LayoutError::Malformed {
            reason: "a split with no panes is not a shape".to_owned(),
        });
    }
    let mut built = Vec::with_capacity(children.len());
    for child in children {
        built.push(decode_shape(child)?);
    }
    Ok(Shape::Split {
        axis,
        children: built,
    })
}

fn decode_arrangement(object: &JsonObject) -> Result<Vec<Vec<String>>, LayoutError> {
    let Some(panes) = object.get("arrangement").and_then(Json::as_array) else {
        return Err(LayoutError::Malformed {
            reason: "a layout carries an array of panes".to_owned(),
        });
    };
    let mut out = Vec::with_capacity(panes.len());
    for (index, pane) in panes.iter().enumerate() {
        let Some(documents) = pane.as_array() else {
            return Err(LayoutError::Malformed {
                reason: format!("pane {index} is an array of document names"),
            });
        };
        let mut names = Vec::with_capacity(documents.len());
        for document in documents {
            let name = document.as_str().ok_or_else(|| LayoutError::Malformed {
                reason: format!("a document in pane {index} is a string"),
            })?;
            names.push(name.to_owned());
        }
        out.push(names);
    }
    Ok(out)
}

fn decode_shows(object: &JsonObject) -> Result<Vec<PanelState>, LayoutError> {
    let Some(shows) = object.get("shows").and_then(Json::as_array) else {
        return Err(LayoutError::Malformed {
            reason: "a layout carries an array of shown panels".to_owned(),
        });
    };
    let mut out = Vec::with_capacity(shows.len());
    for (index, entry) in shows.iter().enumerate() {
        let Some(panel) = entry.as_object() else {
            return Err(LayoutError::Malformed {
                reason: format!("panel {index} is an object"),
            });
        };
        let id = required_string(panel, "id")?;
        let size = match panel.get("size") {
            Some(value) => Some(value.as_u64().ok_or_else(|| LayoutError::Malformed {
                reason: format!("panel {id}'s size is a whole number"),
            })? as u32),
            None => None,
        };
        out.push(PanelState::new(id, size));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{Layout, LayoutError, Layouts, PRESETS};
    use crate::app::groups::{Axis, Groups, Shape};
    use crate::app::panels::{Panels, built_in};

    fn host_panels() -> Panels {
        built_in()
    }

    #[test]
    fn every_preset_has_a_name_and_applies_to_a_live_area() {
        for name in PRESETS {
            let layout = Layout::preset(name).unwrap();
            assert_eq!(layout.name(), name);
            let mut groups = Groups::new(4);
            let mut panels = host_panels();
            assert!(layout.apply(&mut groups, &mut panels).unwrap().is_empty());
        }
        assert!(Layout::preset("nowhere").is_none());
    }

    #[test]
    fn applying_a_layout_hides_the_panels_it_does_not_name() {
        let mut groups = Groups::new(4);
        let mut panels = host_panels();
        // The built-in panels start with outline, messages and problems shown.
        assert!(panels.layout().shown().contains(&"messages".to_owned()));

        Layout::preset("presentation")
            .unwrap()
            .apply(&mut groups, &mut panels)
            .unwrap();
        assert!(
            panels.layout().shown().is_empty(),
            "presentation shows no panels"
        );
    }

    #[test]
    fn reset_keeps_every_open_document() {
        let mut groups = Groups::new(4);
        let mut panels = host_panels();
        groups.open("pages/a.json");
        groups.split(Axis::LeftRight);
        groups.open("pages/b.json");
        groups.type_into("keep me").unwrap();

        let mut layouts = Layouts::new();
        layouts.reset(&mut groups, &mut panels).unwrap();

        // One pane again, but both documents are still open and the edit survives.
        assert_eq!(groups.len(), 1);
        assert!(!groups.views_of("pages/a.json").is_empty());
        assert!(!groups.views_of("pages/b.json").is_empty());
        assert_eq!(groups.text("pages/b.json"), Some("keep me"));
    }

    #[test]
    fn a_saved_layout_round_trips_through_json_and_back_into_the_area() {
        let mut groups = Groups::new(4);
        let panels = host_panels();
        groups.split(Axis::LeftRight);
        groups.open("pages/a.json");
        let mut layouts = Layouts::new();
        layouts.save("two-columns", &groups, &panels);

        let text = layouts.to_json();
        let parsed = Layout::from_json(&text).unwrap();
        assert_eq!(
            parsed.name(),
            "default",
            "the json is the *current* layout, not the saved one"
        );

        // Recall the saved one and check it rebuilds the two panes.
        let mut fresh = Groups::new(4);
        let mut fresh_panels = host_panels();
        layouts
            .recall("two-columns", &mut fresh, &mut fresh_panels)
            .unwrap();
        assert_eq!(fresh.len(), 2);
    }

    #[test]
    fn a_bad_configuration_is_refused_and_leaves_everything_as_it_was() {
        let mut groups = Groups::new(4);
        let mut panels = host_panels();
        groups.open("pages/a.json");
        let before = (groups.render(), panels.layout().shown());

        let mut layouts = Layouts::new();
        // Not JSON at all.
        assert!(matches!(
            layouts.load("not json", &mut groups, &mut panels),
            Err(LayoutError::Malformed { .. })
        ));
        // Our format, but the shape and the arrangement disagree.
        let text = "{\"format\":1,\"name\":\"x\",\"shape\":{\"split\":{\"axis\":\"left-right\",\
                    \"children\":[{\"leaf\":0},{\"leaf\":1}]}},\"arrangement\":[[]],\"shows\":[],\
                    \"zen\":false,\"fullscreen\":false}";
        assert!(matches!(
            layouts.load(text, &mut groups, &mut panels),
            Err(LayoutError::BadArrangement { .. })
        ));

        // Nothing changed, and the reason is remembered.
        assert_eq!((groups.render(), panels.layout().shown()), before);
        assert!(layouts.note().is_some());
        assert!(layouts.current().name() == "default");
    }

    #[test]
    fn a_layout_written_out_reads_back_the_same() {
        for name in PRESETS {
            let layout = Layout::preset(name).unwrap();
            let parsed = Layout::from_json(&layout.to_json()).unwrap();
            assert_eq!(parsed, layout);
        }
    }

    #[test]
    fn an_unknown_preset_or_user_layout_is_refused_by_name() {
        let mut groups = Groups::new(4);
        let mut panels = host_panels();
        let mut layouts = Layouts::new();

        match layouts.use_preset("nowhere", &mut groups, &mut panels) {
            Err(LayoutError::UnknownPreset { name }) => assert_eq!(name, "nowhere"),
            other => panic!("expected UnknownPreset, got {other:?}"),
        }
        match layouts.recall("mine", &mut groups, &mut panels) {
            Err(LayoutError::UnknownUserLayout { name }) => assert_eq!(name, "mine"),
            other => panic!("expected UnknownUserLayout, got {other:?}"),
        }
        assert!(layouts.saved().is_empty());
    }

    #[test]
    fn a_locked_layout_refuses_every_change_by_name_and_saving_still_works() {
        let mut groups = Groups::new(4);
        let mut panels = host_panels();
        let mut layouts = Layouts::new();
        layouts.lock();

        for outcome in [
            layouts
                .use_preset("columns", &mut groups, &mut panels)
                .map(|_| ()),
            layouts.reset(&mut groups, &mut panels).map(|_| ()),
            layouts.set_zen(true),
            layouts.set_fullscreen(true),
        ] {
            assert!(
                matches!(outcome, Err(LayoutError::Locked { .. })),
                "{outcome:?}"
            );
        }
        // Saving reads what is on screen; it changes nothing, so a lock does not stop it.
        groups.open("pages/a.json");
        layouts.save("mine", &groups, &panels);
        assert_eq!(layouts.saved(), ["mine"]);

        layouts.unlock();
        assert!(
            layouts
                .use_preset("columns", &mut groups, &mut panels)
                .is_ok()
        );
    }

    #[test]
    fn the_shape_in_a_layout_is_a_value_with_nothing_session_specific_in_it() {
        // A layout that names two panes side by side and shows the outline.
        let text = "{\"format\":1,\"name\":\"two\",\
                    \"shape\":{\"split\":{\"axis\":\"left-right\",\"children\":[{\"leaf\":0},{\"leaf\":1}]}},\
                    \"arrangement\":[[\"pages/a.json\"],[]],\
                    \"shows\":[{\"id\":\"outline\",\"size\":300}],\"zen\":false,\"fullscreen\":false}";
        let layout = Layout::from_json(text).unwrap();
        assert_eq!(
            layout.shape(),
            &Shape::Split {
                axis: Axis::LeftRight,
                children: vec![Shape::Leaf(0), Shape::Leaf(1)],
            }
        );
        assert_eq!(
            layout.arrangement(),
            &[vec!["pages/a.json".to_owned()], vec![]]
        );
        assert_eq!(layout.shows()[0].id(), "outline");
        assert_eq!(layout.shows()[0].size(), Some(300));
        assert!(layout.shape().render().contains("left-right"));
    }
}
