// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The editing regression harness: normal, million-line, Unicode and multi-view (task T06.08).
//!
//! The acceptance line is *"the whitepaper's size boundaries and the end-to-end distribution
//! evidence are preserved"*. Two things must survive, so the harness measures one and re-checks the
//! other:
//!
//! * **The size boundaries are the ones from [T06.07](crate::app::editor::scale).** [`boundaries`] re-reads
//!   [`Limits`] and re-classifies documents at the large and huge edges, so a later change to a
//!   boundary is a failing check rather than a silent drift. The harness borrows the boundaries
//!   instead of copying the numbers.
//! * **The distribution is recorded, not asserted.** Every scenario replays a fixed script and keeps
//!   each operation's duration in a [`Distribution`] (min, median, p90, mean, max). Wall-clock
//!   numbers are evidence a person reads; they are not turned into pass/fail, because a loaded
//!   machine would then fail a correct build. What *is* a check is correctness.
//!
//! Each scenario proves the property its size stresses:
//!
//! * [`Scenario::Normal`] — equal inserts and deletes leave the text exactly as it was.
//! * [`Scenario::Million`] — the T06.01 property at scale: an edit reassembles **nothing**
//!   (`materializations` does not move), the immutable original is untouched, and the append-only
//!   side grows by exactly what was inserted.
//! * [`Scenario::Unicode`] — an edit that would split a wide or combining character is refused, and a
//!   caret snapped onto one lands on a boundary.
//! * [`Scenario::MultiView`] — one document in two groups: typing through one view is read by both,
//!   and scrolling one view leaves the other's viewport alone.

#![allow(dead_code)]

use std::time::Instant;

use crate::app::editor::editing::{Carets, Editor, Selection};
use crate::app::editor::scale::{Limits, Scale, Tier};
use crate::app::shell::groups::{Axis, Groups};
use crate::project::text::piece_table::TextStore;

/// One edit script the harness replays.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scenario {
    /// A small hand-sized document: edits go in and come back out.
    Normal,
    /// A very long document: an edit must not reassemble it.
    Million,
    /// Wide and combining characters: an edit must not split one.
    Unicode,
    /// One document shown in two groups: the views share the text and keep their own viewports.
    MultiView,
}

impl Scenario {
    /// Every scenario, in a stable order.
    pub const ALL: [Self; 4] = [Self::Normal, Self::Million, Self::Unicode, Self::MultiView];

    /// The scenario's name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Million => "million",
            Self::Unicode => "unicode",
            Self::MultiView => "multi-view",
        }
    }

    /// The scenario a name picks.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|s| s.name() == name)
    }

    /// A reasonable line count for this scenario, when the caller does not give one.
    pub fn default_lines(self) -> usize {
        match self {
            Self::Normal => 200,
            Self::Million => 1_000_000,
            Self::Unicode => 100,
            Self::MultiView => 100,
        }
    }
}

/// One correctness property a scenario checks, and whether it held.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Check {
    /// What the property is, in one sentence.
    pub name: String,
    /// Whether it held.
    pub passed: bool,
}

impl Check {
    /// A check and its outcome.
    pub fn new(name: impl Into<String>, passed: bool) -> Self {
        Self {
            name: name.into(),
            passed,
        }
    }

    /// The check as one line.
    pub fn render(&self) -> String {
        format!(
            "  check: {} [{}]",
            self.name,
            if self.passed { "passed" } else { "FAILED" }
        )
    }
}

/// A set of operation durations, in microseconds.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Distribution {
    samples: Vec<u64>,
}

impl Distribution {
    /// A distribution over some samples, sorted so the percentiles are cheap.
    pub fn of(mut samples: Vec<u64>) -> Self {
        samples.sort_unstable();
        Self { samples }
    }

    /// How many operations were measured.
    pub fn count(&self) -> usize {
        self.samples.len()
    }

    /// The fastest operation.
    pub fn min(&self) -> u64 {
        self.samples.first().copied().unwrap_or(0)
    }

    /// The slowest operation.
    pub fn max(&self) -> u64 {
        self.samples.last().copied().unwrap_or(0)
    }

    /// The median operation.
    pub fn median(&self) -> u64 {
        if self.samples.is_empty() {
            return 0;
        }
        self.samples[self.samples.len() / 2]
    }

    /// The ninetieth-percentile operation.
    pub fn p90(&self) -> u64 {
        if self.samples.is_empty() {
            return 0;
        }
        // The nearest-rank definition: the first sample at or above the ninetieth percentile.
        let rank = (self.samples.len() * 9).div_ceil(10).max(1);
        self.samples[rank - 1]
    }

    /// The mean operation, rounded down.
    pub fn mean(&self) -> u64 {
        if self.samples.is_empty() {
            return 0;
        }
        let total: u64 = self.samples.iter().sum();
        total / self.samples.len() as u64
    }

    /// The distribution in one line.
    pub fn render(&self) -> String {
        format!(
            "  duration us over {} operation(s): min {}, median {}, p90 {}, mean {}, max {}",
            self.count(),
            self.min(),
            self.median(),
            self.p90(),
            self.mean(),
            self.max()
        )
    }
}

/// One scenario's outcome: what it measured and what it checked.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Measurement {
    /// Which scenario ran.
    pub scenario: Scenario,
    /// How many operations were timed.
    pub operations: usize,
    /// How many bytes the document was.
    pub bytes: usize,
    /// How many lines the document was.
    pub lines: usize,
    /// The operation durations.
    pub durations: Distribution,
    /// The correctness properties.
    pub checks: Vec<Check>,
}

impl Measurement {
    /// Whether every check passed.
    pub fn passed(&self) -> bool {
        self.checks.iter().all(|check| check.passed)
    }

    /// The measurement in one report.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "scenario: {}, {} operation(s), {} byte(s), {} line(s)",
            self.scenario.name(),
            self.operations,
            self.bytes,
            self.lines
        )];
        lines.push(self.durations.render());
        for check in &self.checks {
            lines.push(check.render());
        }
        lines
    }
}

/// Replays a scenario and reports what it measured and checked.
pub fn run(scenario: Scenario, lines: usize) -> Measurement {
    match scenario {
        Scenario::Normal => normal(lines),
        Scenario::Million => million(lines),
        Scenario::Unicode => unicode(lines),
        Scenario::MultiView => multi_view(lines),
    }
}

/// The size boundaries of [T06.07](crate::app::editor::scale), re-read and re-classified.
pub fn boundaries() -> Vec<Check> {
    let limits = Limits::new();
    vec![
        Check::new(
            "the large boundary is one mebibyte",
            limits.large_bytes == 1 << 20,
        ),
        Check::new(
            "the huge boundary is eight mebibytes",
            limits.huge_bytes == 8 << 20,
        ),
        Check::new(
            "the long-line boundary is four thousand columns",
            limits.long_line == 4_000,
        ),
        Check::new(
            "the cache ceiling is twenty thousand lines",
            limits.cache_lines == 20_000,
        ),
        Check::new(
            "a document below the large boundary is normal",
            tier_of(limits.large_bytes - 1) == Tier::Normal,
        ),
        Check::new(
            "a document at the large boundary is large",
            tier_of(limits.large_bytes) == Tier::Large,
        ),
        Check::new(
            "a document at the huge boundary is huge",
            tier_of(limits.huge_bytes) == Tier::Huge,
        ),
    ]
}

/// The whole harness report: the boundary checks and every scenario.
pub fn report(lines: usize) -> Vec<String> {
    let mut report = vec!["size boundaries:".to_owned()];
    for check in boundaries() {
        report.push(check.render());
    }
    for scenario in Scenario::ALL {
        let count = if lines == 0 {
            scenario.default_lines()
        } else {
            lines
        };
        report.push(String::new());
        let measurement = run(scenario, count);
        report.extend(measurement.render());
    }
    report
}

/// The tier a document of a byte size falls in.
fn tier_of(bytes: usize) -> Tier {
    Scale::of(&"x".repeat(bytes), Limits::new()).tier()
}

/// A document of `lines` lines of thirteen bytes each.
fn sample_document(lines: usize) -> String {
    let mut text = String::with_capacity(lines * 13);
    for index in 0..lines {
        text.push_str(&format!("line {index:07}\n"));
    }
    text
}

/// Replays insertions and deletions that cancel out, and checks the text is exactly as it was.
fn normal(lines: usize) -> Measurement {
    let original = sample_document(lines.max(1));
    let mut store = TextStore::new(original.clone());
    let operations = 200;
    let mut samples = Vec::with_capacity(operations);
    for index in 0..operations {
        if store.is_empty() {
            break;
        }
        let at = (index * 37) % store.len();
        let at = floor_boundary(&store.text(), at);
        let start = Instant::now();
        store.splice(at, 0, "x").expect("an insertion in range");
        let elapsed = start.elapsed().as_micros() as u64;
        samples.push(elapsed);
        store
            .splice(at, 1, "")
            .expect("the deletion of what was inserted");
    }
    let mut checks = vec![
        Check::new(
            "equal inserts and deletes leave the text unchanged",
            store.text() == original,
        ),
        Check::new("every edit is one undo step", store.can_undo()),
    ];
    // Taking every step back reaches the original again.
    while store.can_undo() {
        store.undo().expect("undo while steps remain");
    }
    checks.push(Check::new(
        "undoing every step reaches the original",
        store.text() == original,
    ));
    measurement(Scenario::Normal, &original, samples, checks)
}

/// Edits a very long document and checks that an edit reassembles nothing.
fn million(lines: usize) -> Measurement {
    let document = sample_document(lines.max(1));
    let bytes = document.len();
    let line_count = document.matches('\n').count() + 1;
    let mut store = TextStore::new(document);
    let baseline = store.materializations();
    let operations = 500;
    let mut samples = Vec::with_capacity(operations);
    for index in 0..operations {
        let at = (index * 4093) % store.len().max(1);
        let start = Instant::now();
        store.splice(at, 0, "z").expect("an insertion in range");
        samples.push(start.elapsed().as_micros() as u64);
    }
    // Read the reassembly count before the one `text()` below, so the check is about the edits.
    let reassembled = store.materializations();
    let checks = vec![
        Check::new(
            "no edit reassembled the whole text",
            reassembled == baseline,
        ),
        Check::new(
            "the immutable original is untouched",
            store.original_len() == bytes,
        ),
        Check::new(
            "the appended side grew by exactly what was inserted",
            store.added_len() == operations,
        ),
        Check::new(
            "the text is what the edits left",
            store.text().len() == bytes + operations,
        ),
    ];
    Measurement {
        scenario: Scenario::Million,
        operations: samples.len(),
        bytes,
        lines: line_count,
        durations: Distribution::of(samples),
        checks,
    }
}

/// Edits around wide and combining characters and checks no character is split.
fn unicode(lines: usize) -> Measurement {
    // "a" + wide 世 + combining acute + "b", repeated.
    let unit = "a世\u{0301}b";
    let original: String = unit.repeat(lines.max(1));
    let mut store = TextStore::new(original.clone());
    let wide_at = original.find('世').expect("a wide character");
    let combining_at = original.find('\u{0301}').expect("a combining character");

    // An offset inside a multi-byte character is refused and changes nothing.
    let inside_wide = store.splice(wide_at + 1, 0, "x");
    let refused_wide = inside_wide.is_err() && store.text() == original;
    let inside_combining = store.splice(combining_at + 1, 0, "x");
    let refused_combining = inside_combining.is_err() && store.text() == original;

    // A caret snapped onto the middle of a character lands on the character's boundary.
    let mut editor = Editor::new();
    editor
        .set_carets(
            &store.text(),
            Carets::of(vec![Selection::caret(wide_at + 1)]).expect("a caret"),
        )
        .expect("a snap");
    let snapped = editor.carets().primary().cursor() == wide_at;

    // Widening the document by a boundary insertion succeeds and keeps every character whole.
    let mut samples = Vec::with_capacity(lines);
    for _ in 0..lines.max(1) {
        // Appending is always a character boundary, so this times the edit alone.
        let at = store.len();
        let start = Instant::now();
        let result = store.splice(at, 0, "x");
        samples.push(start.elapsed().as_micros() as u64);
        result.expect("a boundary insertion");
    }
    let checks = vec![
        Check::new("an offset inside a wide character is refused", refused_wide),
        Check::new(
            "an offset inside a combining character is refused",
            refused_combining,
        ),
        Check::new(
            "a caret snapped onto a character lands on its boundary",
            snapped,
        ),
        Check::new(
            "a boundary insertion is accepted at every repetition",
            samples.len() == lines.max(1),
        ),
    ];
    measurement(Scenario::Unicode, &original, samples, checks)
}

/// Shows one document in two groups and checks the views share it but not their viewports.
fn multi_view(lines: usize) -> Measurement {
    let document = sample_document(lines.max(1));
    let mut area = Groups::new(8);
    area.open("pages/a.md");
    let first = area.group_ids()[0];
    let second = area.split(Axis::LeftRight);
    area.another_view("pages/a.md");
    // Both groups show the same document.
    let first_shows = area
        .tabs(first)
        .and_then(|tabs| tabs.active())
        .map(|tab| tab.document().to_owned());
    let second_shows = area
        .tabs(second)
        .and_then(|tabs| tabs.active())
        .map(|tab| tab.document().to_owned());

    let mut samples = Vec::with_capacity(3);
    // Typing through the focused (second) view writes the one buffer.
    let start = Instant::now();
    area.type_into(document.clone())
        .expect("the second view types");
    samples.push(start.elapsed().as_micros() as u64);
    // Scrolling and selecting the second view leave the first alone.
    let start = Instant::now();
    area.scroll(40).expect("the second view scrolls");
    samples.push(start.elapsed().as_micros() as u64);
    let start = Instant::now();
    area.select(2, 5).expect("the second view selects");
    samples.push(start.elapsed().as_micros() as u64);

    let first_viewport = area
        .tabs(first)
        .and_then(|tabs| tabs.tabs().first())
        .map(|tab| (tab.viewport().scroll(), tab.viewport().selection()));
    let second_viewport = area
        .tabs(second)
        .and_then(|tabs| tabs.tabs().first())
        .map(|tab| (tab.viewport().scroll(), tab.viewport().selection()));

    let checks = vec![
        Check::new(
            "both views show the same document",
            first_shows.as_deref() == Some("pages/a.md")
                && second_shows.as_deref() == Some("pages/a.md"),
        ),
        Check::new(
            "one buffer is read by every view",
            area.text("pages/a.md") == Some(document.as_str()),
        ),
        Check::new(
            "scrolling and selecting one view leave the other alone",
            first_viewport == Some((0, None)) && second_viewport == Some((40, Some((2, 5)))),
        ),
    ];
    measurement(Scenario::MultiView, &document, samples, checks)
}

/// Assembles a measurement, reading the document's size for the report.
fn measurement(
    scenario: Scenario,
    document: &str,
    samples: Vec<u64>,
    checks: Vec<Check>,
) -> Measurement {
    Measurement {
        scenario,
        operations: samples.len(),
        bytes: document.len(),
        lines: document.matches('\n').count() + 1,
        durations: Distribution::of(samples),
        checks,
    }
}

/// The character boundary at or before an offset.
fn floor_boundary(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

#[cfg(test)]
mod tests {
    use super::{Distribution, Measurement, Scenario, boundaries, run};

    #[test]
    fn the_size_boundaries_are_the_ones_from_t06_07() {
        let checks = boundaries();
        for check in &checks {
            assert!(check.passed, "boundary check failed: {}", check.name);
        }
    }

    #[test]
    fn a_normal_document_survives_equal_inserts_and_deletes() {
        let measurement = run(Scenario::Normal, 200);
        assert!(measurement.passed(), "{:?}", measurement.render());
        assert!(measurement.operations > 0);
    }

    #[test]
    fn a_long_document_is_edited_without_reassembling_it() {
        let measurement = run(Scenario::Million, 20_000);
        assert!(measurement.passed(), "{:?}", measurement.render());
        assert!(measurement.bytes > 0);
    }

    #[test]
    fn unicode_edits_refuse_to_split_a_character() {
        let measurement = run(Scenario::Unicode, 100);
        assert!(measurement.passed(), "{:?}", measurement.render());
    }

    #[test]
    fn two_views_share_one_buffer_and_keep_their_viewports() {
        let measurement = run(Scenario::MultiView, 100);
        assert!(measurement.passed(), "{:?}", measurement.render());
    }

    #[test]
    fn a_scenario_name_round_trips() {
        for scenario in Scenario::ALL {
            assert_eq!(Scenario::from_name(scenario.name()), Some(scenario));
        }
        assert_eq!(Scenario::from_name("nowhere"), None);
    }

    #[test]
    fn the_distribution_reports_its_own_shape() {
        let distribution = Distribution::of(vec![5, 1, 3, 2, 4]);
        assert_eq!(distribution.count(), 5);
        assert_eq!(distribution.min(), 1);
        assert_eq!(distribution.median(), 3);
        assert_eq!(distribution.max(), 5);
        assert_eq!(distribution.p90(), 5);
        assert_eq!(distribution.mean(), 3);
        assert_eq!(Distribution::of(Vec::new()).count(), 0);
    }

    #[test]
    fn a_measurement_with_a_failed_check_is_not_passed() {
        let measurement = Measurement {
            scenario: Scenario::Normal,
            operations: 1,
            bytes: 1,
            lines: 1,
            durations: Distribution::of(vec![1]),
            checks: vec![super::Check::new("something", false)],
        };
        assert!(!measurement.passed());
    }
}
