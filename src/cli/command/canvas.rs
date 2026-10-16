// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The canvas's commands: the widget catalog, the scene, structural edits, the viewport,
//! dragging, design-time object state and the design surface.

// Every branch of the flag table answers with `return`, including the ones that could be a
// trailing expression: the branches are independent flags, and writing them alike is worth more
// than saving a keyword. The returns that are *not* needless — the ones inside a loop that refuses
// a bad value — are why the table reads the way it does.
#![allow(clippy::needless_return)]

use super::*;

/// How to show the widget catalog (task T09.01).
#[derive(Debug, PartialEq, Eq)]
pub struct CatalogOptions {
    /// The operations: `widget:<id>:<name>:<category>`, `unsupported:<id>:<reason>`,
    /// `property:<id>:<name>:<type>[:<default>]`, `limit:<id>:<note>`, `search:<query>`,
    /// `category:<name>`, `favorite:<id>`, `unfavorite:<id>`, `schema:<id>`, `show`.
    pub ops: Vec<String>,
}

impl CatalogOptions {
    /// Options for an empty catalog.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to drive the design scene (task T09.02).
#[derive(Debug, PartialEq, Eq)]
pub struct SceneOptions {
    /// The operations: `add:<node>:<x>,<y>,<w>,<h>`, `rebuild:<node>:<x>,<y>,<w>,<h>`,
    /// `remove:<node>`, `hit:<x>,<y>`, `resolve:<node>:<generation>`, `raise:<node>`, `show`.
    pub ops: Vec<String>,
}

impl SceneOptions {
    /// Options for an empty scene.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to run structural edits on the object tree (task T09.04).
#[derive(Debug, PartialEq, Eq)]
pub struct StructureOptions {
    /// The operations: `root:<kind>:<name>`, `insert:<parent>:<kind>:<name>`, `delete:<node>`,
    /// `copy:<node>`, `rename:<node>:<name>`, `reorder:<node>:<index>`, `reparent:<node>:<parent|->`,
    /// `reference:<from>:<to>`, `show`.
    pub ops: Vec<String>,
}

impl StructureOptions {
    /// Options for an empty tree.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to pan, zoom and show the design aids (task T09.05).
#[derive(Debug, PartialEq, Eq)]
pub struct ViewportOptions {
    /// The operations: `transform:<panx>,<pany>,<zoom>`, `pan:<dx>,<dy>`, `zoom:<factor>:`
    /// `<vw>,<vh>`, `zoom-at:<factor>:<ax>,<ay>`, `fit:<x>,<y>,<w>,<h>:<vw>,<vh>[:<margin>]`,
    /// `actual-size:<vw>,<vh>`, `spacing:<s>`, `ruler:<step>`, `grid:<on|off>`, `snap:<on|off>`,
    /// `snap:<x>,<y>`, `align:<x>,<y>,<w>,<h>:<x>,<y>,<w>,<h>[:<tol>]`, `to-screen:<x>,<y>`,
    /// `to-model:<x>,<y>`, `overlay:<vw>,<vh>`, `state`, `show`.
    pub ops: Vec<String>,
}

impl ViewportOptions {
    /// Options for an unmodified viewport.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to drag a node and commit the change as a transaction (task T09.06).
#[derive(Debug, PartialEq, Eq)]
pub struct DragOptions {
    /// The operations: `root:<kind>:<name>`, `insert:<parent>:<kind>:<name>`, `begin:<node>`,
    /// `candidates`, `aim:<parent|->:<index>`, `preview`, `release`, `cancel`,
    /// `stage:<node>:<parent|->:<index>`, `commit`, `discard`, `undo`, `redo`, `state`, `show`.
    pub ops: Vec<String>,
}

impl DragOptions {
    /// Options for an empty design.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to inspect design-time object state, filter and locate, and pages (task T09.07).
#[derive(Debug, PartialEq, Eq)]
pub struct ObjectsOptions {
    /// The operations: `node:<id>:<kind>[:visible=<bool>][:name=<text>]`, `hide:<id>`,
    /// `unhide:<id>`, `lock:<id>`, `unlock:<id>`, `raise:<id>`, `lower:<id>`, `published:<id>`,
    /// `visible:<id>`, `filter-text:<text>`, `filter-kind:<kind>`,
    /// `filter-state:<hidden|shown|locked|unlocked>`, `filter-clear`, `filter`, `locate:<text>`,
    /// `page:<id>:<name>:<root>`, `activate:<id>`, `close-page:<id>`, `rename-page:<id>:<name>`,
    /// `state`, `show`.
    pub ops: Vec<String>,
}

impl ObjectsOptions {
    /// Options for an empty document.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to drive the design surface's keyboard and stress harness (task T09.08).
#[derive(Debug, PartialEq, Eq)]
pub struct DesignOptions {
    /// The operations: `node:<parent|->:<kind>:<name>`, `select:<node>`, `all`, `deselect`,
    /// `delete`, `undo`, `redo`, `next`, `previous`, `parent`, `child`, `press:<chord>`,
    /// `stress:<nodes>:<rounds>`, `state`, `show`.
    pub ops: Vec<String>,
}

impl DesignOptions {
    /// Options for an empty surface.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// Parses this family's flags, answering `None` when the flag is another family's.
pub(super) fn parse_canvas(
    text: &str,
    _argument: OsString,
    arguments: &mut std::vec::IntoIter<OsString>,
) -> Option<Result<Command, ParseError>> {
    match text {
        "--catalog" => {
            let mut options = CatalogOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Catalog(options)));
        }
        "--scene" => {
            let mut options = SceneOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Scene(options)));
        }
        "--structure" => {
            let mut options = StructureOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Structure(options)));
        }
        "--viewport" => {
            let mut options = ViewportOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Viewport(options)));
        }
        "--drag" => {
            let mut options = DragOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Drag(options)));
        }
        "--objects" => {
            let mut options = ObjectsOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Objects(options)));
        }
        "--design" => {
            let mut options = DesignOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Design(options)));
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;

    #[test]
    fn parses_the_catalog_operations() {
        assert_eq!(
            parse(["--catalog"]),
            Ok(Command::Catalog(CatalogOptions::new())),
        );
        let mut expected = CatalogOptions::new();
        expected.ops = vec![
            "widget:button:Button:controls".to_owned(),
            "property:button:label:text".to_owned(),
            "search:b".to_owned(),
        ];
        assert_eq!(
            parse([
                "--catalog",
                "widget:button:Button:controls",
                "property:button:label:text",
                "search:b"
            ]),
            Ok(Command::Catalog(expected)),
        );
        assert!(parse(["--catalog", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_scene_operations() {
        assert_eq!(parse(["--scene"]), Ok(Command::Scene(SceneOptions::new())));
        let mut expected = SceneOptions::new();
        expected.ops = vec!["add:1:0,0,10,10".to_owned(), "hit:5,5".to_owned()];
        assert_eq!(
            parse(["--scene", "add:1:0,0,10,10", "hit:5,5"]),
            Ok(Command::Scene(expected)),
        );
        assert!(parse(["--scene", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_structure_operations() {
        assert_eq!(
            parse(["--structure"]),
            Ok(Command::Structure(StructureOptions::new())),
        );
        let mut expected = StructureOptions::new();
        expected.ops = vec![
            "root:container:panel".to_owned(),
            "insert:0:widget:button".to_owned(),
            "show".to_owned(),
        ];
        assert_eq!(
            parse([
                "--structure",
                "root:container:panel",
                "insert:0:widget:button",
                "show"
            ]),
            Ok(Command::Structure(expected)),
        );
        assert!(parse(["--structure", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_viewport_operations() {
        assert_eq!(
            parse(["--viewport"]),
            Ok(Command::Viewport(ViewportOptions::new())),
        );
        let mut expected = ViewportOptions::new();
        expected.ops = vec![
            "fit:0,0,100,100:800,600".to_owned(),
            "zoom-at:2:10,10".to_owned(),
            "overlay:800,600".to_owned(),
        ];
        assert_eq!(
            parse([
                "--viewport",
                "fit:0,0,100,100:800,600",
                "zoom-at:2:10,10",
                "overlay:800,600"
            ]),
            Ok(Command::Viewport(expected)),
        );
        assert!(parse(["--viewport", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_drag_operations() {
        assert_eq!(parse(["--drag"]), Ok(Command::Drag(DragOptions::new())));
        let mut expected = DragOptions::new();
        expected.ops = vec![
            "root:container:panel".to_owned(),
            "begin:0".to_owned(),
            "aim:-:0".to_owned(),
            "release".to_owned(),
        ];
        assert_eq!(
            parse([
                "--drag",
                "root:container:panel",
                "begin:0",
                "aim:-:0",
                "release"
            ]),
            Ok(Command::Drag(expected)),
        );
        assert!(parse(["--drag", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_objects_operations() {
        assert_eq!(
            parse(["--objects"]),
            Ok(Command::Objects(ObjectsOptions::new())),
        );
        let mut expected = ObjectsOptions::new();
        expected.ops = vec![
            "node:1:button:visible=false".to_owned(),
            "hide:1".to_owned(),
            "filter".to_owned(),
        ];
        assert_eq!(
            parse([
                "--objects",
                "node:1:button:visible=false",
                "hide:1",
                "filter"
            ]),
            Ok(Command::Objects(expected)),
        );
        assert!(parse(["--objects", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_design_operations() {
        assert_eq!(
            parse(["--design"]),
            Ok(Command::Design(DesignOptions::new())),
        );
        let mut expected = DesignOptions::new();
        expected.ops = vec![
            "node:-:container:panel".to_owned(),
            "all".to_owned(),
            "press:Ctrl+Z".to_owned(),
        ];
        assert_eq!(
            parse(["--design", "node:-:container:panel", "all", "press:Ctrl+Z"]),
            Ok(Command::Design(expected)),
        );
        assert!(parse(["--design", "--unknown"]).is_err());
    }
}
