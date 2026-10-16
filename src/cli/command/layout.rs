// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Layout's commands: the contract, container layouts, constraints, arrange, frames,
//! focus order, variants and the probe.

// Every branch of the flag table answers with `return`, including the ones that could be a
// trailing expression: the branches are independent flags, and writing them alike is worth more
// than saving a keyword. The returns that are *not* needless — the ones inside a loop that refuses
// a bad value — are why the table reads the way it does.
#![allow(clippy::needless_return)]

use super::*;

/// How to exercise the layout contract and its diagnostics (task T10.01).
#[derive(Debug, PartialEq, Eq)]
pub struct LayoutPlanOptions {
    /// The operations: `available:<node>:<w>,<h>`, `constraint:<node>:<property>:<origin>`,
    /// `box:<node>:<x>,<y>,<w>,<h>`, `run:<preview|generation>`, `source:<node>`, `state`, `show`.
    pub ops: Vec<String>,
}

impl LayoutPlanOptions {
    /// Options for an empty plan.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to switch a container's layout and migrate its children (task T10.02).
#[derive(Debug, PartialEq, Eq)]
pub struct LayoutSwitchOptions {
    /// The operations: `layout:<kind>`, `child:<node>:<property>`, `switch`, `detach`,
    /// `state`, `show`.
    pub ops: Vec<String>,
}

impl LayoutSwitchOptions {
    /// Options for a container with no children.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to set and validate a node's box constraints (task T10.03).
#[derive(Debug, PartialEq, Eq)]
pub struct ConstraintsOptions {
    /// The operations: `min:<node>:<w>,<h>`, `max:<node>:<w>,<h>`, `preferred:<node>:<w>,<h>`,
    /// `grow:<node>:<f>`, `margin:<node>:<f>`, `gap:<node>:<f>`, `overflow:<node>:<kind>`,
    /// `validate`, `state`, `show`.
    pub ops: Vec<String>,
}

impl ConstraintsOptions {
    /// Options for a node with no constraints.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to align, distribute and nudge a selection as constraint writes (task T10.04).
#[derive(Debug, PartialEq, Eq)]
pub struct ArrangeOptions {
    /// The operations: `layout:<kind>`, `select:<node>`, `align:<axis>:<mode>`,
    /// `distribute:<axis>`, `match:<axis>`, `nudge:<axis>:<direction>:<step>[:<spacing>]`,
    /// `state`, `show`.
    pub ops: Vec<String>,
}

impl ArrangeOptions {
    /// Options for an empty selection.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to check simulated frames against a target's support (task T10.05).
#[derive(Debug, PartialEq, Eq)]
pub struct FramesOptions {
    /// The operations: `support-size:<w>,<h>`, `support-orientation:<portrait|landscape>`,
    /// `support-safe-area:<true|false>`, `support-max-scale:<f>`, `frame:<label>:<w>,<h>:<orientation>`,
    /// `frame-scale:<label>:<f>`, `frame-insets:<label>:<t>,<r>,<b>,<l>`, `check`, `state`, `show`.
    pub ops: Vec<String>,
}

impl FramesOptions {
    /// Options for no frames and no support.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to build the focus order and inspect it (task T10.06).
#[derive(Debug, PartialEq, Eq)]
pub struct FocusOrderOptions {
    /// The operations: `root:<kind>:<name>`, `insert:<parent>:<kind>:<name>`, `focusable:<node>`,
    /// `exclude:<node>`, `index:<node>:<n>`, `build`, `step:<node>:<delta>`, `export`, `state`, `show`.
    pub ops: Vec<String>,
}

impl FocusOrderOptions {
    /// Options for an empty tree.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to declare a target's responsive capability and build layout variants (task T10.07).
#[derive(Debug, PartialEq, Eq)]
pub struct VariantsOptions {
    /// The operations: `capability:<kind>`, `variant:<name>`, `when:<condition>`,
    /// `value:<key>=<value>`, `add`, `state`, `show`.
    pub ops: Vec<String>,
}

impl VariantsOptions {
    /// Options for a target that supports nothing.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to compare the preview with a library-generated probe (task T10.08).
#[derive(Debug, PartialEq, Eq)]
pub struct LayoutProbeOptions {
    /// The operations: `scenario:<name>`, `available:<node>:<w>,<h>`, `box:<node>:<x>,<y>,<w>,<h>`,
    /// `probe-box:<node>:<x>,<y>,<w>,<h>`, `compare`, `state`, `show`.
    pub ops: Vec<String>,
}

impl LayoutProbeOptions {
    /// Options for an empty comparison.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// Parses this family's flags, answering `None` when the flag is another family's.
pub(super) fn parse_layout(
    text: &str,
    _argument: OsString,
    arguments: &mut std::vec::IntoIter<OsString>,
) -> Option<Result<Command, ParseError>> {
    match text {
        "--layout-plan" => {
            let mut options = LayoutPlanOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::LayoutPlan(options)));
        }
        "--layout-switch" => {
            let mut options = LayoutSwitchOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::LayoutSwitch(options)));
        }
        "--constraints" => {
            let mut options = ConstraintsOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Constraints(options)));
        }
        "--arrange" => {
            let mut options = ArrangeOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Arrange(options)));
        }
        "--frames" => {
            let mut options = FramesOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Frames(options)));
        }
        "--focus-order" => {
            let mut options = FocusOrderOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::FocusOrder(options)));
        }
        "--variants" => {
            let mut options = VariantsOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Variants(options)));
        }
        "--layout-probe" => {
            let mut options = LayoutProbeOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::LayoutProbe(options)));
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;

    #[test]
    fn parses_the_layout_plan_operations() {
        assert_eq!(
            parse(["--layout-plan"]),
            Ok(Command::LayoutPlan(LayoutPlanOptions::new())),
        );
        let mut expected = LayoutPlanOptions::new();
        expected.ops = vec![
            "available:1:400,300".to_owned(),
            "constraint:1:width:explicit".to_owned(),
            "run:preview".to_owned(),
        ];
        assert_eq!(
            parse([
                "--layout-plan",
                "available:1:400,300",
                "constraint:1:width:explicit",
                "run:preview"
            ]),
            Ok(Command::LayoutPlan(expected)),
        );
        assert!(parse(["--layout-plan", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_layout_switch_operations() {
        assert_eq!(
            parse(["--layout-switch"]),
            Ok(Command::LayoutSwitch(LayoutSwitchOptions::new())),
        );
        let mut expected = LayoutSwitchOptions::new();
        expected.ops = vec![
            "layout:row".to_owned(),
            "child:1:x".to_owned(),
            "switch".to_owned(),
        ];
        assert_eq!(
            parse(["--layout-switch", "layout:row", "child:1:x", "switch"]),
            Ok(Command::LayoutSwitch(expected)),
        );
        assert!(parse(["--layout-switch", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_constraint_operations() {
        assert_eq!(
            parse(["--constraints"]),
            Ok(Command::Constraints(ConstraintsOptions::new())),
        );
        let mut expected = ConstraintsOptions::new();
        expected.ops = vec![
            "min:1:10,10".to_owned(),
            "max:1:100,100".to_owned(),
            "validate".to_owned(),
        ];
        assert_eq!(
            parse(["--constraints", "min:1:10,10", "max:1:100,100", "validate"]),
            Ok(Command::Constraints(expected)),
        );
        assert!(parse(["--constraints", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_arrange_operations() {
        assert_eq!(
            parse(["--arrange"]),
            Ok(Command::Arrange(ArrangeOptions::new())),
        );
        let mut expected = ArrangeOptions::new();
        expected.ops = vec![
            "layout:row".to_owned(),
            "select:1".to_owned(),
            "align:vertical:centre".to_owned(),
        ];
        assert_eq!(
            parse([
                "--arrange",
                "layout:row",
                "select:1",
                "align:vertical:centre"
            ]),
            Ok(Command::Arrange(expected)),
        );
        assert!(parse(["--arrange", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_frames_operations() {
        assert_eq!(
            parse(["--frames"]),
            Ok(Command::Frames(FramesOptions::new()))
        );
        let mut expected = FramesOptions::new();
        expected.ops = vec![
            "support-size:390,844".to_owned(),
            "frame:phone:390,844:portrait".to_owned(),
            "check".to_owned(),
        ];
        assert_eq!(
            parse([
                "--frames",
                "support-size:390,844",
                "frame:phone:390,844:portrait",
                "check"
            ]),
            Ok(Command::Frames(expected)),
        );
        assert!(parse(["--frames", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_focus_order_operations() {
        assert_eq!(
            parse(["--focus-order"]),
            Ok(Command::FocusOrder(FocusOrderOptions::new())),
        );
        let mut expected = FocusOrderOptions::new();
        expected.ops = vec![
            "root:container:panel".to_owned(),
            "focusable:1".to_owned(),
            "step:1:1".to_owned(),
        ];
        assert_eq!(
            parse([
                "--focus-order",
                "root:container:panel",
                "focusable:1",
                "step:1:1"
            ]),
            Ok(Command::FocusOrder(expected)),
        );
        assert!(parse(["--focus-order", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_variants_operations() {
        assert_eq!(
            parse(["--variants"]),
            Ok(Command::Variants(VariantsOptions::new())),
        );
        let mut expected = VariantsOptions::new();
        expected.ops = vec![
            "capability:min-width".to_owned(),
            "variant:wide".to_owned(),
            "when:min-width=600".to_owned(),
        ];
        assert_eq!(
            parse([
                "--variants",
                "capability:min-width",
                "variant:wide",
                "when:min-width=600"
            ]),
            Ok(Command::Variants(expected)),
        );
        assert!(parse(["--variants", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_layout_probe_operations() {
        assert_eq!(
            parse(["--layout-probe"]),
            Ok(Command::LayoutProbe(LayoutProbeOptions::new())),
        );
        let mut expected = LayoutProbeOptions::new();
        expected.ops = vec![
            "scenario:nested".to_owned(),
            "available:1:400,300".to_owned(),
            "compare".to_owned(),
        ];
        assert_eq!(
            parse([
                "--layout-probe",
                "scenario:nested",
                "available:1:400,300",
                "compare"
            ]),
            Ok(Command::LayoutProbe(expected)),
        );
        assert!(parse(["--layout-probe", "--unknown"]).is_err());
    }
}
