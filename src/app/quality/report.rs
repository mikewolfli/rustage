// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The problem centre, design checks, test/visual evidence and the evidence bundle (task F21).
//!
//! The module keeps the sources and versions of a quality claim explicit. A problem is not "an
//! error": it names where it came from (design, build, test, visual, performance), which target and
//! revision it is about, and the node or source position that answers *where*. A test result is not
//! "passed": it is passed, failed, skipped or **not run**, and the last one stays visibly not run.
//! Contrast is computed from the two colours rather than trusted from a theme name.

use std::collections::{BTreeMap, BTreeSet};

use crate::project::record::json::{Json, JsonObject};

/// Where a problem came from.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Source {
    /// The document model or a design check.
    Design,
    /// A compiler or build.
    Build,
    /// A test run.
    Test,
    /// A visual comparison.
    Visual,
    /// A performance measurement.
    Performance,
}

impl Source {
    /// The source's name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Design => "design",
            Self::Build => "build",
            Self::Test => "test",
            Self::Visual => "visual",
            Self::Performance => "performance",
        }
    }

    /// Reads a source name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "design" => Some(Self::Design),
            "build" => Some(Self::Build),
            "test" => Some(Self::Test),
            "visual" => Some(Self::Visual),
            "performance" => Some(Self::Performance),
            _ => None,
        }
    }
}

/// How a problem reads.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Severity {
    /// The product cannot be used.
    Error,
    /// The product works with a documented limit.
    Warning,
    /// Context only.
    Note,
}

impl Severity {
    /// The severity's name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Note => "note",
        }
    }

    /// Reads a severity name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "error" => Some(Self::Error),
            "warning" => Some(Self::Warning),
            "note" => Some(Self::Note),
            _ => None,
        }
    }
}

/// One problem with its source and the target/revision it belongs to.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Problem {
    /// Where it came from.
    pub source: Source,
    /// The target profile or toolchain it applies to.
    pub target: String,
    /// The document revision it was read at.
    pub revision: u64,
    /// The node it points at, when it has one.
    pub node: Option<u64>,
    /// The source position, when it has one.
    pub location: Option<String>,
    /// How serious it is.
    pub severity: Severity,
    /// What is wrong.
    pub message: String,
}

impl Problem {
    /// The problem in one line.
    pub fn render(&self) -> String {
        let place = self
            .node
            .map(|node| format!(" node {node}"))
            .or_else(|| self.location.clone().map(|location| format!(" {location}")))
            .unwrap_or_default();
        format!(
            "[{}] {} {}@r{}{place}: {}",
            self.severity.name(),
            self.source.name(),
            self.target,
            self.revision,
            self.message
        )
    }
}

/// A filter over the problem centre.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Filter {
    /// Only this source.
    pub source: Option<Source>,
    /// Only this target.
    pub target: Option<String>,
    /// Only problems at or after this revision.
    pub revision: Option<u64>,
    /// Only this severity.
    pub severity: Option<Severity>,
}

/// The problems this session has, from every source.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Center {
    problems: Vec<Problem>,
}

impl Center {
    /// An empty centre.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a problem.
    pub fn add(&mut self, problem: Problem) {
        self.problems.push(problem);
    }

    /// The problems matching `filter`, in insertion order.
    pub fn matching(&self, filter: &Filter) -> Vec<&Problem> {
        self.problems
            .iter()
            .filter(|problem| {
                filter.source.is_none_or(|source| problem.source == source)
                    && filter
                        .target
                        .as_ref()
                        .is_none_or(|target| &problem.target == target)
                    && filter
                        .revision
                        .is_none_or(|revision| problem.revision >= revision)
                    && filter
                        .severity
                        .is_none_or(|severity| problem.severity == severity)
            })
            .collect()
    }

    /// What "go to" answers for a problem: its node or source position.
    pub fn navigate(problem: &Problem) -> Option<String> {
        problem
            .node
            .map(|node| format!("node {node}"))
            .or_else(|| problem.location.clone())
    }

    /// The centre in one line per problem.
    pub fn render(&self, filter: &Filter) -> Vec<String> {
        self.matching(filter)
            .into_iter()
            .map(Problem::render)
            .collect()
    }
}

/// Facts a design check reads. It is deliberately small: every field is something the document
/// already knows, not a second document.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct DesignFacts {
    /// Node ids in the tree.
    pub nodes: Vec<u64>,
    /// Each node's parent (`None` for a root).
    pub parents: BTreeMap<u64, Option<u64>>,
    /// Capabilities the target was asked for and whether it has them.
    pub capabilities: BTreeMap<String, bool>,
    /// The control count and the target's limit.
    pub capacity: (usize, usize),
    /// Resources the document references and whether they exist.
    pub resources: BTreeMap<String, bool>,
    /// The focus order, as written.
    pub focus_order: Vec<u64>,
}

impl DesignFacts {
    /// Runs the structural, capability, capacity, resource and focus checks.
    pub fn check(&self, target: &str, revision: u64) -> Vec<Problem> {
        let mut problems = Vec::new();
        let mut seen = BTreeSet::new();
        for node in &self.nodes {
            if !seen.insert(*node) {
                problems.push(problem(
                    Source::Design,
                    target,
                    revision,
                    Some(*node),
                    Severity::Error,
                    format!("node {node} is declared twice"),
                ));
            }
        }
        for (node, parent) in &self.parents {
            if let Some(parent) = parent
                && !seen.contains(parent)
            {
                problems.push(problem(
                    Source::Design,
                    target,
                    revision,
                    Some(*node),
                    Severity::Error,
                    format!("node {node} names a missing parent {parent}"),
                ));
            }
        }
        for (capability, available) in &self.capabilities {
            if !available {
                problems.push(problem(
                    Source::Design,
                    target,
                    revision,
                    None,
                    Severity::Error,
                    format!("target {target} does not support {capability}"),
                ));
            }
        }
        if self.capacity.0 > self.capacity.1 {
            problems.push(problem(
                Source::Design,
                target,
                revision,
                None,
                Severity::Error,
                format!(
                    "the document uses {} controls, over the target limit of {}",
                    self.capacity.0, self.capacity.1
                ),
            ));
        }
        for (resource, present) in &self.resources {
            if !present {
                problems.push(problem(
                    Source::Design,
                    target,
                    revision,
                    None,
                    Severity::Error,
                    format!("resource {resource:?} is referenced but not present"),
                ));
            }
        }
        let mut focus = BTreeSet::new();
        for node in &self.focus_order {
            if !focus.insert(*node) {
                problems.push(problem(
                    Source::Design,
                    target,
                    revision,
                    Some(*node),
                    Severity::Warning,
                    format!("node {node} appears twice in the focus order"),
                ));
            }
        }
        problems
    }
}

/// Builds one problem with the common fields.
fn problem(
    source: Source,
    target: &str,
    revision: u64,
    node: Option<u64>,
    severity: Severity,
    message: String,
) -> Problem {
    Problem {
        source,
        target: target.to_owned(),
        revision,
        node,
        location: None,
        severity,
        message,
    }
}

/// A relative luminance from an sRGB channel.
fn linear(channel: u8) -> f64 {
    let value = f64::from(channel) / 255.0;
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

/// The WCAG relative luminance of an RGB colour.
pub fn luminance(rgb: (u8, u8, u8)) -> f64 {
    0.2126 * linear(rgb.0) + 0.7152 * linear(rgb.1) + 0.0722 * linear(rgb.2)
}

/// The WCAG contrast ratio of two colours, between 1 and 21.
pub fn contrast(foreground: (u8, u8, u8), background: (u8, u8, u8)) -> f64 {
    let first = luminance(foreground);
    let second = luminance(background);
    let (lighter, darker) = if first >= second {
        (first, second)
    } else {
        (second, first)
    };
    (lighter + 0.05) / (darker + 0.05)
}

/// One test's state.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TestStatus {
    /// It ran and passed.
    Passed,
    /// It ran and failed.
    Failed,
    /// It was explicitly ignored.
    Skipped,
    /// It exists but has not run in this session.
    NotRun,
}

impl TestStatus {
    /// The status name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
            Self::NotRun => "not-run",
        }
    }

    /// Reads a status name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "passed" | "pass" => Some(Self::Passed),
            "failed" | "fail" => Some(Self::Failed),
            "skipped" | "skip" => Some(Self::Skipped),
            "not-run" | "notrun" => Some(Self::NotRun),
            _ => None,
        }
    }
}

/// One test and its state.
#[derive(Clone, PartialEq, Debug)]
pub struct TestResult {
    /// Its name.
    pub name: String,
    /// What happened.
    pub status: TestStatus,
    /// How long it took, when it ran.
    pub seconds: Option<f64>,
}

/// The tests this session knows about.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Tests {
    results: BTreeMap<String, TestResult>,
}

impl Tests {
    /// No tests yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a test result, replacing an earlier result for the same name.
    pub fn record(&mut self, name: impl Into<String>, status: TestStatus, seconds: Option<f64>) {
        let name = name.into();
        self.results.insert(
            name.clone(),
            TestResult {
                name,
                status,
                seconds,
            },
        );
    }

    /// The results in name order.
    pub fn results(&self) -> Vec<&TestResult> {
        self.results.values().collect()
    }

    /// The number of tests in each state.
    pub fn tally(&self) -> BTreeMap<&'static str, usize> {
        let mut tally = BTreeMap::new();
        for result in self.results.values() {
            *tally.entry(result.status.name()).or_insert(0) += 1;
        }
        tally
    }

    /// Writes JUnit XML. A test that did not run has no `<testcase>` and is listed in a
    /// `<properties>` entry, because JUnit has no "not run" state and pretending it passed is the
    /// failure this module exists to avoid.
    pub fn junit(&self) -> String {
        let failures = self
            .results
            .values()
            .filter(|result| result.status == TestStatus::Failed)
            .count();
        let skipped = self
            .results
            .values()
            .filter(|result| result.status == TestStatus::Skipped)
            .count();
        let total = self.results.len();
        let mut xml = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<testsuite name=\"rustage\" tests=\"{total}\" failures=\"{failures}\" skipped=\"{skipped}\">\n"
        );
        for result in self.results.values() {
            if result.status == TestStatus::NotRun {
                continue;
            }
            let time = result.seconds.unwrap_or(0.0);
            xml.push_str(&format!(
                "  <testcase name=\"{}\" time=\"{time:.3}\">",
                xml_escape(&result.name)
            ));
            match result.status {
                TestStatus::Failed => xml.push_str("<failure/>"),
                TestStatus::Skipped => xml.push_str("<skipped/>"),
                TestStatus::Passed | TestStatus::NotRun => {}
            }
            xml.push_str("</testcase>\n");
        }
        let not_run: Vec<&str> = self
            .results
            .values()
            .filter(|result| result.status == TestStatus::NotRun)
            .map(|result| result.name.as_str())
            .collect();
        if !not_run.is_empty() {
            xml.push_str(&format!(
                "  <properties><property name=\"not-run\" value=\"{}\"/></properties>\n",
                xml_escape(&not_run.join(", "))
            ));
        }
        xml.push_str("</testsuite>\n");
        xml
    }
}

/// Escapes the five XML metacharacters.
fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// The environment a visual baseline was captured in.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct VisualCondition {
    /// The platform or host name.
    pub platform: String,
    /// The display scale, as a percentage.
    pub scale: u32,
    /// A stable font-set identifier (a digest, not a font name that can drift).
    pub font_digest: String,
}

impl VisualCondition {
    /// Whether two captures are comparable.
    pub fn compare(&self, other: &Self) -> Result<(), String> {
        if self.platform != other.platform {
            return Err(format!(
                "platform differs: {} vs {}",
                self.platform, other.platform
            ));
        }
        if self.scale != other.scale {
            return Err(format!("scale differs: {} vs {}", self.scale, other.scale));
        }
        if self.font_digest != other.font_digest {
            return Err("font set differs".to_owned());
        }
        Ok(())
    }
}

/// The evidence bundle written for one revision.
#[derive(Clone, PartialEq, Debug)]
pub struct Evidence {
    /// The document revision.
    pub revision: u64,
    /// The target.
    pub target: String,
    /// The problem centre.
    pub center: Center,
    /// The test results.
    pub tests: Tests,
    /// The visual condition, when a capture exists.
    pub visual: Option<VisualCondition>,
}

impl Evidence {
    /// The bundle as JSON, with every problem and test carrying its state.
    pub fn json(&self) -> String {
        let problems: Vec<Json> = self
            .center
            .problems
            .iter()
            .map(|problem| {
                let mut object = JsonObject::new();
                object.insert("source", Json::string(problem.source.name()));
                object.insert("severity", Json::string(problem.severity.name()));
                object.insert("target", Json::string(&problem.target));
                object.insert("revision", Json::number(problem.revision));
                if let Some(node) = problem.node {
                    object.insert("node", Json::number(node));
                }
                if let Some(location) = &problem.location {
                    object.insert("location", Json::string(location));
                }
                object.insert("message", Json::string(&problem.message));
                Json::Object(object)
            })
            .collect();
        let tests: Vec<Json> = self
            .tests
            .results()
            .into_iter()
            .map(|result| {
                let mut object = JsonObject::new();
                object.insert("name", Json::string(&result.name));
                object.insert("status", Json::string(result.status.name()));
                if let Some(seconds) = result.seconds {
                    object.insert("seconds", Json::number(seconds as u64));
                }
                Json::Object(object)
            })
            .collect();
        let mut root = JsonObject::new();
        root.insert("revision", Json::number(self.revision));
        root.insert("target", Json::string(&self.target));
        root.insert("problems", Json::Array(problems));
        root.insert("tests", Json::Array(tests));
        if let Some(visual) = &self.visual {
            let mut object = JsonObject::new();
            object.insert("platform", Json::string(&visual.platform));
            object.insert("scale", Json::number(visual.scale as u64));
            object.insert("font", Json::string(&visual.font_digest));
            root.insert("visual", Json::Object(object));
        }
        Json::Object(root).render()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Center, DesignFacts, Evidence, Filter, Problem, Severity, Source, TestStatus, Tests,
        VisualCondition, contrast,
    };

    fn problem(source: Source, target: &str, revision: u64, message: &str) -> Problem {
        Problem {
            source,
            target: target.to_owned(),
            revision,
            node: Some(7),
            location: Some("src/main.rs:3:1".to_owned()),
            severity: Severity::Error,
            message: message.to_owned(),
        }
    }

    #[test]
    fn the_centre_keeps_source_target_and_revision_apart() {
        let mut center = Center::new();
        center.add(problem(Source::Design, "desktop", 4, "missing capability"));
        center.add(problem(Source::Build, "desktop", 5, "compile failed"));
        let design = Filter {
            source: Some(Source::Design),
            ..Filter::default()
        };
        assert_eq!(center.matching(&design).len(), 1);
        let recent = Filter {
            revision: Some(5),
            ..Filter::default()
        };
        assert_eq!(center.matching(&recent).len(), 1);
        assert_eq!(
            Center::navigate(&problem(Source::Design, "desktop", 4, "x")),
            Some("node 7".to_owned())
        );
    }

    #[test]
    fn a_design_check_names_each_breakage() {
        let mut parents = std::collections::BTreeMap::new();
        parents.insert(1, None);
        parents.insert(2, Some(99));
        let mut capabilities = std::collections::BTreeMap::new();
        capabilities.insert("graphics".to_owned(), false);
        let mut resources = std::collections::BTreeMap::new();
        resources.insert("hero.png".to_owned(), false);
        let facts = DesignFacts {
            nodes: vec![1, 2, 2],
            parents,
            capabilities,
            capacity: (12, 8),
            resources,
            focus_order: vec![1, 1],
        };
        let problems = facts.check("mini", 3);
        assert!(problems.iter().any(|p| p.message.contains("twice")));
        assert!(
            problems
                .iter()
                .any(|p| p.message.contains("missing parent"))
        );
        assert!(problems.iter().any(|p| p.message.contains("graphics")));
        assert!(
            problems
                .iter()
                .any(|p| p.message.contains("over the target limit"))
        );
        assert!(problems.iter().any(|p| p.message.contains("hero.png")));
        assert!(problems.iter().any(|p| p.message.contains("focus order")));
    }

    #[test]
    fn contrast_is_computed_from_colours() {
        assert!((contrast((0, 0, 0), (255, 255, 255)) - 21.0).abs() < 0.01);
        assert!((contrast((255, 255, 255), (255, 255, 255)) - 1.0).abs() < 0.01);
        assert!(contrast((0x0A, 0x84, 0xFF), (255, 255, 255)) < 4.5);
    }

    #[test]
    fn a_not_run_test_is_not_reported_as_passed() {
        let mut tests = Tests::new();
        tests.record("a", TestStatus::Passed, Some(0.1));
        tests.record("b", TestStatus::NotRun, None);
        let tally = tests.tally();
        assert_eq!(tally.get("passed"), Some(&1));
        assert_eq!(tally.get("not-run"), Some(&1));
        let xml = tests.junit();
        assert!(xml.contains("not-run"));
        assert!(!xml.contains("<testcase name=\"b\""));
    }

    #[test]
    fn a_visual_capture_refuses_a_different_environment() {
        let first = VisualCondition {
            platform: "macos".to_owned(),
            scale: 200,
            font_digest: "abc".to_owned(),
        };
        let second = VisualCondition {
            scale: 100,
            ..first.clone()
        };
        assert!(first.compare(&second).is_err());
    }

    #[test]
    fn the_evidence_bundle_carries_every_state() {
        let mut center = Center::new();
        center.add(problem(Source::Test, "desktop", 2, "failed"));
        let mut tests = Tests::new();
        tests.record("suite", TestStatus::Failed, None);
        let evidence = Evidence {
            revision: 2,
            target: "desktop".to_owned(),
            center,
            tests,
            visual: None,
        };
        let json = evidence.json();
        assert!(json.contains("\"status\": \"failed\""), "{json}");
        assert!(json.contains("\"source\": \"test\""), "{json}");
    }
}
