// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Automated accessibility checks, kept apart from manual screen-reader acceptance (T21.03).
//!
//! The acceptance line has two halves and they are not interchangeable: *automated checks* can find
//! a control with no accessible name, a dangling `labelled_by`, a focus flag on something that
//! cannot take focus, or a node the tree cannot reach; *manual acceptance* is a person driving a
//! screen reader on a real target. A tool that passed its static checks has not thereby been heard
//! by a screen reader, and this module never lets one stand in for the other:
//!
//! * [`A11yReport::automated_passed`] answers only the static half.
//! * [`A11yReport::manual`] starts as [`ManualResult::NotPerformed`] and only a caller can set it.
//! * [`A11yReport::accepted`] requires **both**, and also requires the target to have an
//!   accessibility bridge at all: `mini` and `embedded` do not, so their report carries
//!   [`A11yReport::unsupported`] and can never be accepted.
//!
//! Roles and the tree come from the library (`A11yRole`, `A11yState`, `A11yTree`), not from a host
//! copy. Nodes are validated *before* they are registered: the library's focus traversal is a
//! recursive depth-first walk with no cycle guard, so a cycle in the declared child links would
//! overflow the stack inside the library. [`A11yCheck::run`] detects that in the host and refuses to
//! build the tree, which is why a malicious or accidental document cannot take the process down.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use rust_widgets::core::ObjectId;
use rust_widgets::platform::accessibility::{A11yNode, A11yRole, A11yState, A11yTree};

/// The restricted profiles with no accessibility bridge.
pub const UNSUPPORTED_PROFILES: [&str; 2] = ["mini", "embedded"];

/// One element declared for the check.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Element {
    /// Its stable id.
    pub id: u64,
    /// Its accessibility role.
    pub role: A11yRole,
    /// Its accessible name; empty means it has none.
    pub label: String,
    /// Whether it can take keyboard focus.
    pub focusable: bool,
    /// Whether it currently holds focus.
    pub focused: bool,
    /// The id of the element that labels this one, when the name is separate.
    pub labelled_by: Option<u64>,
    /// For a label element, the id of the control it names.
    pub control_for: Option<u64>,
    /// Its children, in traversal order.
    pub children: Vec<u64>,
}

impl Element {
    /// An element with a role, a name and no relations.
    pub fn new(id: u64, role: A11yRole, label: impl Into<String>) -> Self {
        Self {
            id,
            role,
            label: label.into(),
            focusable: false,
            focused: false,
            labelled_by: None,
            control_for: None,
            children: Vec::new(),
        }
    }

    /// Whether this role is interactive and therefore needs an accessible name.
    pub fn needs_a_name(&self) -> bool {
        self.focusable
            || matches!(
                self.role,
                A11yRole::Button
                    | A11yRole::TextField
                    | A11yRole::CheckBox
                    | A11yRole::RadioButton
                    | A11yRole::Slider
                    | A11yRole::SpinButton
                    | A11yRole::ComboBox
                    | A11yRole::Link
                    | A11yRole::MenuItem
                    | A11yRole::Tab
                    | A11yRole::Switch
            )
    }
}

/// What an automated check found.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum IssueKind {
    /// Two elements declared the same id.
    DuplicateId,
    /// A child link names an element that was not declared.
    UnknownChild,
    /// The child links contain a cycle.
    Cycle,
    /// No element is a root.
    NoRoot,
    /// An element is a child of more than one element.
    MultipleParents,
    /// More than one element has no parent.
    MultipleRoots,
    /// An element is not reachable from the root.
    Unreachable,
    /// An interactive element has no accessible name.
    MissingLabel,
    /// `labelled_by` names an element that was not declared.
    DanglingLabelledBy,
    /// `control_for` names an element that was not declared.
    DanglingControlFor,
    /// An element holds focus but cannot take it.
    FocusedNotFocusable,
    /// More than one element holds focus.
    MultipleFocused,
    /// An interactive element has no role.
    UnknownRole,
}

impl IssueKind {
    /// The kind's name.
    pub fn name(self) -> &'static str {
        match self {
            Self::DuplicateId => "duplicate-id",
            Self::UnknownChild => "unknown-child",
            Self::Cycle => "cycle",
            Self::NoRoot => "no-root",
            Self::MultipleParents => "multiple-parents",
            Self::MultipleRoots => "multiple-roots",
            Self::Unreachable => "unreachable",
            Self::MissingLabel => "missing-label",
            Self::DanglingLabelledBy => "dangling-labelled-by",
            Self::DanglingControlFor => "dangling-control-for",
            Self::FocusedNotFocusable => "focused-not-focusable",
            Self::MultipleFocused => "multiple-focused",
            Self::UnknownRole => "unknown-role",
        }
    }
}

/// One automated finding.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Issue {
    /// The element it is about.
    pub node: u64,
    /// What kind of problem it is.
    pub kind: IssueKind,
    /// What is wrong, in a sentence a person can act on.
    pub message: String,
}

/// What a person driving a screen reader found.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ManualResult {
    /// Nobody ran one.
    #[default]
    NotPerformed,
    /// It was run and passed.
    Passed,
    /// It was run and failed.
    Failed,
}

impl ManualResult {
    /// The result's name.
    pub fn name(self) -> &'static str {
        match self {
            Self::NotPerformed => "not-performed",
            Self::Passed => "passed",
            Self::Failed => "failed",
        }
    }

    /// Reads a result name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "not-performed" => Some(Self::NotPerformed),
            "passed" => Some(Self::Passed),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

/// One manual acceptance record.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ManualCheck {
    /// The screen reader or assistive tool used.
    pub tool: String,
    /// What it found.
    pub result: ManualResult,
    /// Free text: what was read, and by whom.
    pub note: String,
}

/// What one check run found.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct A11yReport {
    /// The profile the check was for.
    pub profile: String,
    /// How many elements were declared.
    pub elements: usize,
    /// The automated findings.
    pub issues: Vec<Issue>,
    /// The manual acceptance record.
    pub manual: ManualCheck,
    /// Why the target cannot give screen-reader evidence, when it cannot.
    pub unsupported: Option<String>,
}

impl A11yReport {
    /// Whether the automated half passed.
    pub fn automated_passed(&self) -> bool {
        self.issues.is_empty() && self.unsupported.is_none()
    }

    /// Whether both halves passed, which is the only thing that counts as acceptance.
    pub fn accepted(&self) -> bool {
        self.automated_passed()
            && self.manual.result == ManualResult::Passed
            && !self.manual.tool.trim().is_empty()
    }

    /// The report in one line per fact.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "a11y: {} element(s) on {}; automated issue(s): {}",
            self.elements,
            self.profile,
            self.issues.len()
        )];
        if let Some(reason) = &self.unsupported {
            lines.push(format!("a11y: target unsupported: {reason}"));
        }
        for issue in &self.issues {
            lines.push(format!(
                "a11y issue {}: node {}: {}",
                issue.kind.name(),
                issue.node,
                issue.message
            ));
        }
        lines.push(format!(
            "a11y automated: {}",
            if self.automated_passed() {
                "passed"
            } else {
                "failed"
            }
        ));
        lines.push(format!(
            "a11y manual screen reader: {} {} ({})",
            if self.manual.tool.is_empty() {
                "no tool recorded"
            } else {
                self.manual.tool.as_str()
            },
            self.manual.result.name(),
            if self.manual.note.is_empty() {
                "no note"
            } else {
                self.manual.note.as_str()
            }
        ));
        lines.push(format!(
            "a11y accepted: {}",
            if self.accepted() { "yes" } else { "no" }
        ));
        lines
    }
}

/// Builds a check from declared elements and runs it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct A11yCheck {
    profile: String,
    elements: Vec<Element>,
    manual: ManualCheck,
}

impl A11yCheck {
    /// A check for a profile, with no elements and no manual record.
    pub fn new(profile: impl Into<String>) -> Self {
        Self {
            profile: profile.into(),
            elements: Vec::new(),
            manual: ManualCheck::default(),
        }
    }

    /// The profile this check is for.
    pub fn profile(&self) -> &str {
        &self.profile
    }

    /// Points the check at another target profile.
    pub fn set_profile(&mut self, profile: impl Into<String>) {
        self.profile = profile.into();
    }

    /// Declares one element.
    pub fn add(&mut self, element: Element) {
        self.elements.push(element);
    }

    /// Records what a person found with a screen reader.
    pub fn set_manual(&mut self, manual: ManualCheck) {
        self.manual = manual;
    }

    /// Links a parent to a child, refusing an id that was not declared.
    pub fn add_child(&mut self, parent: u64, child: u64) -> Result<(), String> {
        let index = self
            .elements
            .iter()
            .position(|element| element.id == parent)
            .ok_or_else(|| format!("no element {parent} was declared"))?;
        self.elements[index].children.push(child);
        Ok(())
    }

    /// Records that `label` names `node`.
    pub fn label_for(&mut self, node: u64, label: u64) -> Result<(), String> {
        let index = self
            .elements
            .iter()
            .position(|element| element.id == node)
            .ok_or_else(|| format!("no element {node} was declared"))?;
        self.elements[index].labelled_by = Some(label);
        Ok(())
    }

    /// Records that `label` names the control `node`.
    pub fn control_for(&mut self, label: u64, node: u64) -> Result<(), String> {
        let index = self
            .elements
            .iter()
            .position(|element| element.id == label)
            .ok_or_else(|| format!("no element {label} was declared"))?;
        self.elements[index].control_for = Some(node);
        Ok(())
    }

    /// Runs every automated check and returns the report.
    ///
    /// The tree is built only when the declared links are safe: duplicate ids, unknown children,
    /// cycles, a missing root and extra roots are all reported first, and any of the structural ones
    /// stops the tree from being built. Everything else is checked directly from the declarations, so
    /// a missing name is still reported even when the links are broken.
    pub fn run(&self) -> A11yReport {
        let mut issues = Vec::new();
        let known: BTreeSet<u64> = self.elements.iter().map(|element| element.id).collect();
        let mut seen = BTreeSet::new();
        for element in &self.elements {
            if !seen.insert(element.id) {
                issues.push(Issue {
                    node: element.id,
                    kind: IssueKind::DuplicateId,
                    message: format!("element {} is declared more than once", element.id),
                });
            }
            if element.needs_a_name()
                && element.label.trim().is_empty()
                && element.labelled_by.is_none()
            {
                issues.push(Issue {
                    node: element.id,
                    kind: IssueKind::MissingLabel,
                    message: format!(
                        "an element with role {} has no accessible name",
                        role_name(element.role)
                    ),
                });
            }
            if element.needs_a_name() && element.role == A11yRole::Unknown {
                issues.push(Issue {
                    node: element.id,
                    kind: IssueKind::UnknownRole,
                    message: "an interactive element has no accessibility role".to_owned(),
                });
            }
            if let Some(label) = element.labelled_by
                && !known.contains(&label)
            {
                issues.push(Issue {
                    node: element.id,
                    kind: IssueKind::DanglingLabelledBy,
                    message: format!("its labelled_by names {label}, which is not declared"),
                });
            }
            if let Some(control) = element.control_for
                && !known.contains(&control)
            {
                issues.push(Issue {
                    node: element.id,
                    kind: IssueKind::DanglingControlFor,
                    message: format!("its control_for names {control}, which is not declared"),
                });
            }
            if element.focused && !element.focusable {
                issues.push(Issue {
                    node: element.id,
                    kind: IssueKind::FocusedNotFocusable,
                    message: "it holds focus but is not focusable".to_owned(),
                });
            }
            for child in &element.children {
                if !known.contains(child) {
                    issues.push(Issue {
                        node: element.id,
                        kind: IssueKind::UnknownChild,
                        message: format!("its child {child} is not declared"),
                    });
                }
            }
        }
        let focused: Vec<u64> = self
            .elements
            .iter()
            .filter(|element| element.focused)
            .map(|element| element.id)
            .collect();
        if focused.len() > 1 {
            issues.push(Issue {
                node: focused[1],
                kind: IssueKind::MultipleFocused,
                message: format!("{} elements hold focus: {focused:?}", focused.len()),
            });
        }

        let structure = self.check_structure(&mut issues);
        if structure {
            self.build_tree(&mut issues);
        }

        issues.sort_by_key(|left| (left.node, left.kind));
        issues.dedup();
        let unsupported = UNSUPPORTED_PROFILES
            .contains(&self.profile.as_str())
            .then(|| {
                format!(
                    "the {} profile has no platform accessibility bridge, so screen readers cannot \
                     be given evidence for it",
                    self.profile
                )
            });
        A11yReport {
            profile: self.profile.clone(),
            elements: self.elements.len(),
            issues,
            manual: self.manual.clone(),
            unsupported,
        }
    }

    /// Checks the parent/child graph, returning whether the tree may be built.
    fn check_structure(&self, issues: &mut Vec<Issue>) -> bool {
        let has_links = self
            .elements
            .iter()
            .any(|element| !element.children.is_empty());
        if !has_links {
            // A flat set of elements is not a broken hierarchy: the caller declared names and focus
            // flags, not a tree. The structural checks only make sense once links exist.
            return true;
        }
        let known: BTreeSet<u64> = self.elements.iter().map(|element| element.id).collect();
        let mut parent: BTreeMap<u64, u64> = BTreeMap::new();
        for element in &self.elements {
            for child in &element.children {
                if !known.contains(child) {
                    continue;
                }
                if parent.insert(*child, element.id).is_some() {
                    issues.push(Issue {
                        node: *child,
                        kind: IssueKind::MultipleParents,
                        message: "it is a child of more than one element".to_owned(),
                    });
                }
            }
        }
        // A cycle is a node whose ancestor chain returns to itself. The walk is bounded by the
        // number of elements, so a graph with a loop cannot make this checks loop.
        let mut cycle = false;
        for id in &known {
            let mut at = *id;
            for _ in 0..=known.len() {
                match parent.get(&at) {
                    Some(up) => at = *up,
                    None => break,
                }
                if at == *id {
                    cycle = true;
                    issues.push(Issue {
                        node: *id,
                        kind: IssueKind::Cycle,
                        message: "its parent links form a cycle".to_owned(),
                    });
                    break;
                }
            }
            if cycle {
                break;
            }
        }
        let roots: Vec<u64> = known
            .iter()
            .copied()
            .filter(|id| !parent.contains_key(id))
            .collect();
        match roots.len() {
            0 => {
                issues.push(Issue {
                    node: 0,
                    kind: IssueKind::NoRoot,
                    message: "every element is a child, so there is no root".to_owned(),
                });
                false
            }
            1 => {
                let mut reached = BTreeSet::new();
                let mut pending = vec![roots[0]];
                while let Some(id) = pending.pop() {
                    if !reached.insert(id) {
                        continue;
                    }
                    if let Some(element) = self.elements.iter().find(|element| element.id == id) {
                        pending.extend(
                            element
                                .children
                                .iter()
                                .copied()
                                .filter(|child| known.contains(child)),
                        );
                    }
                }
                for id in &known {
                    if !reached.contains(id) {
                        issues.push(Issue {
                            node: *id,
                            kind: IssueKind::Unreachable,
                            message: format!("it is not reachable from the root {}", roots[0]),
                        });
                    }
                }
                !cycle
            }
            _ => {
                for id in roots.iter().skip(1) {
                    issues.push(Issue {
                        node: *id,
                        kind: IssueKind::MultipleRoots,
                        message: format!(
                            "there are {} elements with no parent: {roots:?}",
                            roots.len()
                        ),
                    });
                }
                // Anything the first root cannot reach is announced to no screen reader from there,
                // including the other roots. Reporting both is deliberate: one says the tree is
                // ambiguous, the other says which nodes are outside it.
                let mut reached = BTreeSet::new();
                let mut pending = vec![roots[0]];
                while let Some(id) = pending.pop() {
                    if !reached.insert(id) {
                        continue;
                    }
                    if let Some(element) = self.elements.iter().find(|element| element.id == id) {
                        pending.extend(
                            element
                                .children
                                .iter()
                                .copied()
                                .filter(|child| known.contains(child)),
                        );
                    }
                }
                for id in &known {
                    if !reached.contains(id) {
                        issues.push(Issue {
                            node: *id,
                            kind: IssueKind::Unreachable,
                            message: format!("it is not reachable from the root {}", roots[0]),
                        });
                    }
                }
                !cycle
            }
        }
    }

    /// Registers the declared nodes in a real library tree and checks it accepted them.
    fn build_tree(&self, issues: &mut Vec<Issue>) {
        let mut tree = A11yTree::new();
        for element in &self.elements {
            tree.register_node(A11yNode::new(
                element.id as ObjectId,
                A11yState {
                    role: element.role,
                    label: element.label.clone(),
                    enabled: true,
                    focused: element.focused,
                    children: element
                        .children
                        .iter()
                        .map(|child| *child as ObjectId)
                        .collect(),
                    ..Default::default()
                },
            ));
        }
        if tree.len() != self.elements.len() {
            issues.push(Issue {
                node: tree.root_id().unwrap_or(0),
                kind: IssueKind::DuplicateId,
                message: format!(
                    "the library tree holds {} node(s) for {} declared element(s)",
                    tree.len(),
                    self.elements.len()
                ),
            });
        }
    }
}

/// The name of a role, for a report.
pub fn role_name(role: A11yRole) -> &'static str {
    match role {
        A11yRole::Unknown => "unknown",
        A11yRole::Button => "button",
        A11yRole::Label => "label",
        A11yRole::TextField => "text field",
        A11yRole::CheckBox => "check box",
        A11yRole::RadioButton => "radio button",
        A11yRole::Slider => "slider",
        A11yRole::ProgressBar => "progress bar",
        A11yRole::List => "list",
        A11yRole::Table => "table",
        A11yRole::Image => "image",
        A11yRole::Link => "link",
        A11yRole::Heading => "heading",
        A11yRole::Paragraph => "paragraph",
        A11yRole::Group => "group",
        A11yRole::Window => "window",
        A11yRole::Dialog => "dialog",
        A11yRole::Menu => "menu",
        A11yRole::MenuItem => "menu item",
        A11yRole::Tab => "tab",
        A11yRole::Switch => "switch",
        A11yRole::Alert => "alert",
        A11yRole::ComboBox => "combo box",
        A11yRole::SpinButton => "spin button",
        A11yRole::StatusBar => "status bar",
        A11yRole::ToolTip => "tool tip",
        A11yRole::Tree => "tree",
    }
}

/// Reads a role name.
pub fn parse_role(text: &str) -> Option<A11yRole> {
    Some(match text {
        "unknown" => A11yRole::Unknown,
        "button" => A11yRole::Button,
        "label" => A11yRole::Label,
        "text-field" => A11yRole::TextField,
        "check-box" => A11yRole::CheckBox,
        "radio-button" => A11yRole::RadioButton,
        "slider" => A11yRole::Slider,
        "progress-bar" => A11yRole::ProgressBar,
        "list" => A11yRole::List,
        "table" => A11yRole::Table,
        "image" => A11yRole::Image,
        "link" => A11yRole::Link,
        "heading" => A11yRole::Heading,
        "paragraph" => A11yRole::Paragraph,
        "group" => A11yRole::Group,
        "window" => A11yRole::Window,
        "dialog" => A11yRole::Dialog,
        "menu" => A11yRole::Menu,
        "menu-item" => A11yRole::MenuItem,
        "tab" => A11yRole::Tab,
        "switch" => A11yRole::Switch,
        "alert" => A11yRole::Alert,
        "combo-box" => A11yRole::ComboBox,
        "spin-button" => A11yRole::SpinButton,
        "status-bar" => A11yRole::StatusBar,
        "tool-tip" => A11yRole::ToolTip,
        "tree" => A11yRole::Tree,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::{A11yCheck, Element, IssueKind, ManualCheck, ManualResult, parse_role};
    use rust_widgets::platform::accessibility::A11yRole;

    fn new_check() -> A11yCheck {
        A11yCheck::new("desktop")
    }

    fn button(id: u64, label: &str) -> Element {
        Element::new(id, A11yRole::Button, label)
    }

    fn text_field(id: u64, label: &str) -> Element {
        let mut field = Element::new(id, A11yRole::TextField, label);
        field.focusable = true;
        field
    }

    fn kinds(check: &A11yCheck) -> Vec<IssueKind> {
        check.run().issues.iter().map(|issue| issue.kind).collect()
    }

    #[test]
    fn a_control_without_a_name_is_reported_and_a_named_one_is_not() {
        let mut check = new_check();
        check.add(button(1, "Save"));
        assert!(!kinds(&check).contains(&IssueKind::MissingLabel));

        let mut check = new_check();
        check.add(button(1, ""));
        check.add(text_field(2, "Email"));
        let report = check.run();
        assert_eq!(report.issues.len(), 1);
        assert_eq!(report.issues[0].kind, IssueKind::MissingLabel);
        assert_eq!(report.issues[0].node, 1);
        assert!(!report.automated_passed());
    }

    #[test]
    fn a_separate_label_can_name_a_control_and_a_dangling_one_is_refused() {
        let mut check = new_check();
        let mut label = Element::new(1, A11yRole::Label, "Email");
        label.control_for = Some(2);
        check.add(label);
        let mut field = text_field(2, "");
        field.labelled_by = Some(1);
        check.add(field);
        // `labelled_by` is the association the checker reads; the empty own label is fine.
        assert!(kinds(&check).is_empty(), "{:?}", check.run().issues);

        let mut broken = self::new_check();
        let mut field = text_field(2, "");
        field.labelled_by = Some(99);
        broken.add(field);
        let report = broken.run();
        assert_eq!(report.issues[0].kind, IssueKind::DanglingLabelledBy);
        assert_eq!(report.issues[0].node, 2);
    }

    #[test]
    fn a_control_for_that_names_nothing_is_reported() {
        let mut check = self::new_check();
        let mut label = Element::new(1, A11yRole::Label, "Email");
        label.control_for = Some(42);
        check.add(label);
        let report = check.run();
        assert_eq!(report.issues[0].kind, IssueKind::DanglingControlFor);
    }

    #[test]
    fn focus_flags_are_checked() {
        let mut check = self::new_check();
        let mut field = text_field(1, "Name");
        field.focused = true;
        check.add(field);
        assert!(kinds(&check).is_empty());

        let mut check = self::new_check();
        let mut label = Element::new(1, A11yRole::Label, "Name");
        label.focused = true;
        check.add(label);
        assert!(kinds(&check).contains(&IssueKind::FocusedNotFocusable));

        let mut check = self::new_check();
        let mut first = text_field(1, "One");
        first.focused = true;
        let mut second = text_field(2, "Two");
        second.focused = true;
        check.add(first);
        check.add(second);
        assert!(kinds(&check).contains(&IssueKind::MultipleFocused));
    }

    #[test]
    fn an_interactive_element_with_no_role_is_reported() {
        let mut check = self::new_check();
        let mut element = Element::new(1, A11yRole::Unknown, "");
        element.focusable = true;
        check.add(element);
        let report = check.run();
        assert!(
            report
                .issues
                .iter()
                .any(|issue| issue.kind == IssueKind::UnknownRole)
        );
        assert!(
            report
                .issues
                .iter()
                .any(|issue| issue.kind == IssueKind::MissingLabel)
        );
    }

    #[test]
    fn duplicate_ids_unknown_children_and_cycles_are_refused_before_the_tree_is_built() {
        let mut check = self::new_check();
        check.add(button(1, "One"));
        check.add(button(1, "Two"));
        assert!(kinds(&check).contains(&IssueKind::DuplicateId));

        let mut check = self::new_check();
        check.add(button(1, "One"));
        check.add_child(1, 9).expect("a declared parent");
        assert!(kinds(&check).contains(&IssueKind::UnknownChild));

        let mut check = self::new_check();
        check.add(Element::new(1, A11yRole::Group, ""));
        check.add(Element::new(2, A11yRole::Group, ""));
        check.add_child(1, 2).expect("a parent");
        check.add_child(2, 1).expect("a parent");
        let report = check.run();
        assert!(
            report
                .issues
                .iter()
                .any(|issue| issue.kind == IssueKind::Cycle)
        );
        assert!(!report.automated_passed());
    }

    #[test]
    fn a_reachable_tree_with_one_root_passes_the_structure_check() {
        let mut check = self::new_check();
        check.add(Element::new(1, A11yRole::Window, "Demo"));
        check.add(Element::new(2, A11yRole::Group, ""));
        check.add(button(3, "Save"));
        check.add_child(1, 2).expect("a parent");
        check.add_child(2, 3).expect("a parent");
        let report = check.run();
        assert!(report.issues.is_empty(), "{:?}", report.issues);
        assert!(report.automated_passed());
    }

    #[test]
    fn an_unreachable_element_is_reported() {
        let mut check = new_check();
        check.add(Element::new(1, A11yRole::Window, "Demo"));
        check.add(Element::new(2, A11yRole::Group, ""));
        check.add(button(3, "Save"));
        check.add_child(1, 2).expect("a parent");
        let report = check.run();
        assert!(
            report
                .issues
                .iter()
                .any(|issue| issue.kind == IssueKind::MultipleRoots)
        );
        assert!(
            report
                .issues
                .iter()
                .any(|issue| issue.kind == IssueKind::Unreachable)
        );
    }

    #[test]
    fn a_restricted_profile_is_automatically_unsupported() {
        let mut check = A11yCheck::new("mini");
        check.add(button(1, "Save"));
        check.set_manual(ManualCheck {
            tool: "VoiceOver".to_owned(),
            result: ManualResult::Passed,
            note: "checked".to_owned(),
        });
        let report = check.run();
        assert!(report.unsupported.is_some());
        assert!(!report.automated_passed());
        assert!(!report.accepted());
    }

    #[test]
    fn automated_success_is_not_acceptance_without_a_manual_result() {
        let mut check = new_check();
        check.add(button(1, "Save"));
        let report = check.run();
        assert!(report.automated_passed());
        assert_eq!(report.manual.result, ManualResult::NotPerformed);
        assert!(!report.accepted());

        let mut check = self::new_check();
        check.add(button(1, "Save"));
        check.set_manual(ManualCheck {
            tool: "VoiceOver".to_owned(),
            result: ManualResult::Passed,
            note: "read the toolbar and the dialog".to_owned(),
        });
        assert!(check.run().accepted());

        let mut check = self::new_check();
        check.add(button(1, "Save"));
        check.set_manual(ManualCheck {
            tool: "VoiceOver".to_owned(),
            result: ManualResult::Failed,
            note: "the button is announced as 'button'".to_owned(),
        });
        let report = check.run();
        assert!(report.automated_passed());
        assert!(!report.accepted());
    }

    #[test]
    fn the_report_renders_both_halves() {
        let mut check = new_check();
        check.add(button(1, ""));
        let lines = check.run().render();
        assert!(
            lines
                .iter()
                .any(|line| line.contains("a11y issue missing-label"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("a11y automated: failed"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("a11y manual screen reader"))
        );
        assert!(lines.iter().any(|line| line.contains("a11y accepted: no")));
    }

    #[test]
    fn every_role_has_a_name_and_reads_back() {
        for name in [
            "unknown",
            "button",
            "label",
            "text-field",
            "check-box",
            "radio-button",
            "slider",
            "progress-bar",
            "list",
            "table",
            "image",
            "link",
            "heading",
            "paragraph",
            "group",
            "window",
            "dialog",
            "menu",
            "menu-item",
            "tab",
            "switch",
            "alert",
            "combo-box",
            "spin-button",
            "status-bar",
            "tool-tip",
            "tree",
        ] {
            let role = parse_role(name).unwrap_or_else(|| panic!("{name} is a role"));
            assert_eq!(super::role_name(role), name.replace('-', " ").as_str());
        }
        assert!(parse_role("toaster").is_none());
    }
}
