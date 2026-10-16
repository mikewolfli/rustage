// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Running many bind/rebuild cycles and checking that what must not grow does not (task T14.08).
//!
//! The acceptance line is *"subscriptions and memory do not keep growing, and invalid input is
//! kept"*. Both are checks rather than promises:
//!
//! * **A run is steady when it ends where it started.** [`Load::steady`] compares each viewport's
//!   last cycle with its first, so a driver that returns to its starting subscription and node
//!   counts passes and one that keeps climbing does not — and [`Load::growth`] names the cycle, the
//!   count it was and the count it became.
//! * **Invalid input is kept, not dropped.** Every rejected operation is recorded with its index and
//!   its text, and the report counts them, so a load run cannot quietly discard what it could not
//!   read.
//! * **Latency is measured or it is not.** A cycle's latency is what the driver observed; when the
//!   driver observed none the report says *not measured* rather than printing a zero that looks like
//!   a very fast cycle.

#![allow(dead_code)]

use std::collections::BTreeMap;

/// One bind/rebuild cycle as the driver observed it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Cycle {
    /// Which cycle this was, counting from 1.
    pub index: u32,
    /// The live subscription count after it.
    pub subscriptions: u32,
    /// The live node count after it.
    pub nodes: u32,
    /// What the driver observed the cycle cost, if it observed anything.
    pub latency_ms: Option<u32>,
}

/// A count that climbed when it should have stayed put.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Growth {
    /// The viewport it happened in.
    pub viewport: String,
    /// The cycle it happened in.
    pub index: u32,
    /// The subscription count it rose from.
    pub from: u32,
    /// The subscription count it rose to.
    pub to: u32,
}

/// One viewport's cycles.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Series {
    /// The viewport.
    pub viewport: String,
    /// Its cycles, in order.
    pub cycles: Vec<Cycle>,
}

impl Series {
    /// The first cycle's subscription count.
    fn first_subscriptions(&self) -> Option<u32> {
        self.cycles.first().map(|cycle| cycle.subscriptions)
    }

    /// The last cycle's subscription count.
    fn last_subscriptions(&self) -> Option<u32> {
        self.cycles.last().map(|cycle| cycle.subscriptions)
    }

    /// The highest subscription count seen.
    fn peak_subscriptions(&self) -> Option<u32> {
        self.cycles.iter().map(|cycle| cycle.subscriptions).max()
    }

    /// Whether the series ended where it started.
    fn steady(&self) -> bool {
        match (self.cycles.first(), self.cycles.last()) {
            (Some(first), Some(last)) => {
                first.subscriptions == last.subscriptions && first.nodes == last.nodes
            }
            _ => true,
        }
    }

    /// Every cycle whose subscription count rose above the series' starting count.
    fn growth(&self) -> Vec<Growth> {
        let Some(start) = self.first_subscriptions() else {
            return Vec::new();
        };
        self.cycles
            .iter()
            .filter(|cycle| cycle.subscriptions > start)
            .map(|cycle| Growth {
                viewport: self.viewport.clone(),
                index: cycle.index,
                from: start,
                to: cycle.subscriptions,
            })
            .collect()
    }
}

/// Many cycles across many viewports, with what the driver could not read.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Load {
    series: BTreeMap<String, Series>,
    invalid: Vec<(u32, String)>,
    budget_ms: u32,
    operations: u32,
}

impl Load {
    /// A run with no cycles and no budget.
    pub fn new() -> Self {
        Self::default()
    }

    /// The same run with a latency budget in milliseconds.
    pub fn with_budget(mut self, budget_ms: u32) -> Self {
        self.budget_ms = budget_ms;
        self
    }

    /// The latency budget in milliseconds.
    pub fn budget_ms(&self) -> u32 {
        self.budget_ms
    }

    /// How many operations were asked for, valid or not.
    pub fn operations(&self) -> u32 {
        self.operations
    }

    /// The viewports that were driven.
    pub fn viewports(&self) -> Vec<&str> {
        self.series.keys().map(String::as_str).collect()
    }

    /// How many cycles were recorded in total.
    pub fn cycles(&self) -> usize {
        self.series.values().map(|series| series.cycles.len()).sum()
    }

    /// One viewport's cycles.
    pub fn series(&self, viewport: &str) -> Option<&Series> {
        self.series.get(viewport)
    }

    /// Records one cycle in a viewport.
    pub fn cycle(
        &mut self,
        viewport: &str,
        subscriptions: u32,
        nodes: u32,
        latency_ms: Option<u32>,
    ) {
        self.operations += 1;
        let series = self
            .series
            .entry(viewport.to_owned())
            .or_insert_with(|| Series {
                viewport: viewport.to_owned(),
                cycles: Vec::new(),
            });
        let index = series.cycles.len() as u32 + 1;
        series.cycles.push(Cycle {
            index,
            subscriptions,
            nodes,
            latency_ms,
        });
    }

    /// Records `cycles` cycles at a steady count.
    ///
    /// This is the "bind and rebuild a hundred times" driver: what it produces must come back to
    /// where it started, or the run has a leak.
    pub fn repeat(
        &mut self,
        viewport: &str,
        cycles: u32,
        subscriptions: u32,
        nodes: u32,
        latency_ms: Option<u32>,
    ) {
        for _ in 0..cycles {
            self.cycle(viewport, subscriptions, nodes, latency_ms);
        }
    }

    /// Records `cycles` cycles where the subscription count climbs by one each time.
    ///
    /// This is a driver that leaks, and it exists so the check that catches a leak has something to
    /// catch.
    pub fn leak(&mut self, viewport: &str, cycles: u32, subscriptions: u32, nodes: u32) {
        for step in 0..cycles {
            self.cycle(viewport, subscriptions + step, nodes, None);
        }
    }

    /// Records input the driver could not read.
    pub fn invalid(&mut self, text: impl Into<String>) {
        self.operations += 1;
        let index = self.invalid.len() as u32 + 1;
        self.invalid.push((index, text.into()));
    }

    /// The input the driver could not read, with its index.
    pub fn invalid_input(&self) -> &[(u32, String)] {
        &self.invalid
    }

    /// Whether every viewport ended where it started.
    pub fn steady(&self) -> bool {
        self.series.values().all(Series::steady)
    }

    /// Every cycle whose subscription count climbed.
    pub fn growth(&self) -> Vec<Growth> {
        self.series.values().flat_map(Series::growth).collect()
    }

    /// Whether the driver measured any latency at all.
    pub fn measured(&self) -> bool {
        self.series
            .values()
            .any(|series| series.cycles.iter().any(|cycle| cycle.latency_ms.is_some()))
    }

    /// The highest latency observed, if any was.
    pub fn worst_latency(&self) -> Option<u32> {
        self.series
            .values()
            .flat_map(|series| series.cycles.iter())
            .filter_map(|cycle| cycle.latency_ms)
            .max()
    }

    /// Every cycle that took longer than the declared budget.
    pub fn over_budget(&self) -> Vec<(String, u32, u32)> {
        self.series
            .values()
            .flat_map(|series| {
                series
                    .cycles
                    .iter()
                    .filter_map(|cycle| match cycle.latency_ms {
                        Some(latency) if latency > self.budget_ms => {
                            Some((series.viewport.clone(), cycle.index, latency))
                        }
                        _ => None,
                    })
            })
            .collect()
    }

    /// Whether the run passed: it ended where it started and nothing was over budget.
    pub fn passed(&self) -> bool {
        self.steady() && self.over_budget().is_empty()
    }

    /// The run in one line per fact.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "budget: {} ms; viewports: {}; cycles: {}; invalid input: {} (kept)",
            self.budget_ms,
            self.series.len(),
            self.cycles(),
            self.invalid.len()
        )];
        lines.push(format!(
            "steady: {}",
            if self.steady() { "yes" } else { "no" }
        ));
        for series in self.series.values() {
            lines.push(format!(
                "  {}: subscriptions {}..{} peak {}; nodes {}..{}",
                series.viewport,
                series.first_subscriptions().unwrap_or(0),
                series.last_subscriptions().unwrap_or(0),
                series.peak_subscriptions().unwrap_or(0),
                series.cycles.first().map(|cycle| cycle.nodes).unwrap_or(0),
                series.cycles.last().map(|cycle| cycle.nodes).unwrap_or(0),
            ));
        }
        for growth in self.growth() {
            lines.push(format!(
                "growth: {} cycle {}: subscriptions {} -> {}",
                growth.viewport, growth.index, growth.from, growth.to
            ));
        }
        for (index, text) in &self.invalid {
            lines.push(format!("invalid {index}: {text}"));
        }
        lines.push(match (self.measured(), self.worst_latency()) {
            (false, _) => "latency: not measured".to_owned(),
            (true, Some(worst)) => format!(
                "latency: worst {worst} ms ({} cycle(s) over budget)",
                self.over_budget().len()
            ),
            (true, None) => "latency: not measured".to_owned(),
        });
        for (viewport, index, latency) in self.over_budget() {
            lines.push(format!(
                "over budget: {viewport} cycle {index}: {latency} ms over {} ms",
                self.budget_ms
            ));
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::Load;

    #[test]
    fn a_steady_run_returns_to_its_starting_counts() {
        let mut load = Load::new();
        load.repeat("phone", 100, 12, 40, None);
        assert_eq!(load.cycles(), 100);
        assert!(load.steady());
        assert!(load.growth().is_empty());
        assert!(load.passed());
    }

    #[test]
    fn a_leak_makes_the_run_not_steady() {
        let mut load = Load::new();
        load.leak("phone", 100, 12, 40);
        assert!(!load.steady());
        assert!(!load.passed());
    }

    #[test]
    fn a_growth_is_reported_with_its_cycle_and_its_counts() {
        let mut load = Load::new();
        load.leak("phone", 3, 12, 40);
        let growth = load.growth();
        assert_eq!(growth.len(), 2);
        assert_eq!(growth[0].viewport, "phone");
        assert_eq!(growth[0].index, 2);
        assert_eq!((growth[0].from, growth[0].to), (12, 13));
        assert_eq!((growth[1].index, growth[1].to), (3, 14));
    }

    #[test]
    fn one_viewport_leaking_is_enough_to_fail_the_run() {
        let mut load = Load::new();
        load.repeat("phone", 50, 12, 40, None);
        load.leak("desktop", 50, 20, 60);
        assert!(!load.steady());
        assert_eq!(load.growth().len(), 49);
        assert_eq!(load.growth()[0].viewport, "desktop");
    }

    #[test]
    fn the_peak_is_the_highest_count_seen() {
        let mut load = Load::new();
        load.cycle("phone", 12, 40, None);
        load.cycle("phone", 15, 40, None);
        load.cycle("phone", 12, 40, None);
        let series = load.series("phone").expect("a series");
        assert_eq!(series.peak_subscriptions(), Some(15));
        assert!(load.steady());
        assert_eq!(load.growth().len(), 1);
    }

    #[test]
    fn a_run_with_no_cycles_is_steady_and_measures_nothing() {
        let load = Load::new();
        assert!(load.steady());
        assert!(load.passed());
        assert!(!load.measured());
        assert_eq!(load.worst_latency(), None);
        assert_eq!(load.render()[1], "steady: yes");
    }

    #[test]
    fn invalid_input_is_kept_with_its_index_and_its_text() {
        let mut load = Load::new();
        load.repeat("phone", 2, 12, 40, None);
        load.invalid("bind:phone:missing<-nothing");
        load.invalid("rebuild:");
        assert_eq!(load.operations(), 4);
        assert_eq!(
            load.invalid_input(),
            &[
                (1, "bind:phone:missing<-nothing".to_owned()),
                (2, "rebuild:".to_owned()),
            ]
        );
        let lines = load.render();
        assert!(
            lines[0].starts_with("budget: 0 ms; viewports: 1; cycles: 2; invalid input: 2 (kept)")
        );
        assert!(lines.contains(&"invalid 1: bind:phone:missing<-nothing".to_owned()));
        assert!(lines.contains(&"invalid 2: rebuild:".to_owned()));
    }

    #[test]
    fn a_latency_over_budget_is_reported() {
        let mut load = Load::new().with_budget(16);
        load.cycle("phone", 12, 40, Some(9));
        load.cycle("phone", 12, 40, Some(21));
        load.cycle("phone", 12, 40, Some(14));
        assert_eq!(load.worst_latency(), Some(21));
        assert_eq!(load.over_budget(), vec![("phone".to_owned(), 2, 21)]);
        assert!(!load.passed());
        let lines = load.render();
        assert!(lines.contains(&"latency: worst 21 ms (1 cycle(s) over budget)".to_owned()));
        assert!(lines.contains(&"over budget: phone cycle 2: 21 ms over 16 ms".to_owned()));
    }

    #[test]
    fn no_latency_says_not_measured_rather_than_zero() {
        let mut load = Load::new().with_budget(16);
        load.repeat("phone", 100, 12, 40, None);
        assert!(!load.measured());
        assert_eq!(load.worst_latency(), None);
        assert!(load.over_budget().is_empty());
        assert!(load.render().contains(&"latency: not measured".to_owned()));
    }

    #[test]
    fn a_hundred_steady_cycles_with_a_latency_are_all_measured() {
        let mut load = Load::new().with_budget(16);
        load.repeat("phone", 100, 12, 40, Some(8));
        load.repeat("desktop", 100, 20, 60, Some(15));
        assert_eq!(load.cycles(), 200);
        assert_eq!(load.viewports(), vec!["desktop", "phone"]);
        assert!(load.measured());
        assert_eq!(load.worst_latency(), Some(15));
        assert!(load.passed());
    }

    #[test]
    fn the_report_names_every_viewport() {
        let mut load = Load::new().with_budget(16);
        load.repeat("phone", 3, 12, 40, None);
        load.repeat("desktop", 3, 20, 60, None);
        let lines = load.render();
        assert!(
            lines.contains(&"  desktop: subscriptions 20..20 peak 20; nodes 60..60".to_owned())
        );
        assert!(lines.contains(&"  phone: subscriptions 12..12 peak 12; nodes 40..40".to_owned()));
        assert_eq!(
            lines[0],
            "budget: 16 ms; viewports: 2; cycles: 6; invalid input: 0 (kept)"
        );
    }

    #[test]
    fn a_single_cycle_is_steady_in_both_counts() {
        let mut load = Load::new();
        load.cycle("phone", 12, 40, None);
        assert!(load.steady());
        let mut nodes_changed = Load::new();
        nodes_changed.cycle("phone", 12, 40, None);
        nodes_changed.cycle("phone", 12, 41, None);
        assert!(!nodes_changed.steady());
        assert!(nodes_changed.growth().is_empty());
    }
}
