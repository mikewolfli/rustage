// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Running many bind/rebuild cycles and checking that what must not grow does not (task T14.08).
//!
//! The acceptance line is *"subscriptions and memory do not keep growing, and invalid input is
//! kept"*. A number the caller typed in cannot answer that, so the run says where its numbers came
//! from:
//!
//! * **A cycle is measured or it is declared.** [`Load::drive`] mounts a real [`Preview`], applies a
//!   real `ViewEngine::update`, stops the session, and then asks the runtime about the control ids
//!   the session created *before* stopping — so "nothing is still mounted" is a check rather than the
//!   session's own word. Those cycles are [`Source::Measured`]. [`Load::declared_cycle`] records
//!   counts the caller supplied; it exists to exercise the checker and to model a leaking driver, and
//!   a run made only of those cycles cannot pass — [`Load::passed`] requires at least one measured
//!   cycle.
//! * **Steady means it ended where it started.** [`Load::steady`] compares each viewport's last cycle
//!   with its first, and [`Load::growth`] names the cycle, the counter, the count it was and the
//!   count it became.
//! * **A stop that leaves anything behind is residue, not a pass.** [`Load::drive`] records the live
//!   controls left after the last stop, and [`Load::residual`] reports them.
//! * **What is not measured is named.** There is no portable resident-memory or thread-count reading
//!   here, so [`Load::unmeasured`] states that rather than printing a plausible number. The bounded
//!   proxy is the set of counters a cycle does carry: live mounted controls, engine generations,
//!   render instances and child process handles.
//! * **Invalid input is kept, not dropped.** Every rejected operation is recorded with its index and
//!   its text, and the report counts them, so a load run cannot quietly discard what it could not
//!   read.
//! * **Latency is measured or it is not.** A measured cycle's latency is the wall time the driver
//!   observed around mount and update; a declared cycle may carry a caller's number, and a cycle with
//!   neither says *not measured* rather than printing a zero that looks like a very fast cycle.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::time::Instant;

use rust_widgets::core::{ObjectId, Rect};
use rust_widgets::widget::{Widget, WidgetFactory, runtime};

use crate::app::preview::preview_service::{Document, Preview};

/// Where a cycle's numbers came from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    /// Read from a live preview session during [`Load::drive`].
    Measured,
    /// Supplied by the caller: a model of a driver, not evidence.
    Declared,
}

impl Source {
    /// The source's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Measured => "measured",
            Self::Declared => "declared",
        }
    }
}

/// Which counter climbed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Counter {
    /// Controls the session still had mounted.
    Controls,
    /// The engine generation the session ended at.
    Generation,
}

impl Counter {
    /// The counter's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Controls => "live controls",
            Self::Generation => "engine generation",
        }
    }
}

/// One bind/rebuild cycle, as the live session measured it or as the caller declared it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Cycle {
    /// Which cycle this was, counting from 1.
    pub index: u32,
    /// Live mounted controls at the cycle's peak.
    pub controls: u32,
    /// The engine generation the cycle ended at.
    pub generation: u64,
    /// Render instances the session held.
    pub instances: u32,
    /// Child processes the session held.
    pub processes: u32,
    /// The node count the caller declared, or `None` for a measured cycle, whose tree size is the
    /// engine's rather than a counter the host keeps.
    pub nodes: Option<u32>,
    /// What the driver observed the cycle cost, if it observed anything.
    pub latency_ms: Option<u32>,
    /// Where the numbers came from.
    pub source: Source,
}

/// A count that climbed when it should have stayed put.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Growth {
    /// The viewport it happened in.
    pub viewport: String,
    /// The cycle it happened in.
    pub index: u32,
    /// Which counter climbed.
    pub counter: Counter,
    /// The count it rose from.
    pub from: u64,
    /// The count it rose to.
    pub to: u64,
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
    /// The first cycle's control count.
    fn first_controls(&self) -> Option<u32> {
        self.cycles.first().map(|cycle| cycle.controls)
    }

    /// The last cycle's control count.
    fn last_controls(&self) -> Option<u32> {
        self.cycles.last().map(|cycle| cycle.controls)
    }

    /// The highest control count seen.
    fn peak_controls(&self) -> Option<u32> {
        self.cycles.iter().map(|cycle| cycle.controls).max()
    }

    /// Whether the series ended where it started.
    ///
    /// Every measured counter is compared, not just controls: an engine generation that climbs
    /// cycle after cycle is the same unbounded growth in a different counter. A declared cycle
    /// carries no generation, so only a measured series can be judged on it.
    fn steady(&self) -> bool {
        let (Some(first), Some(last)) = (self.cycles.first(), self.cycles.last()) else {
            return true;
        };
        let nodes_kept = match (first.nodes, last.nodes) {
            (Some(from), Some(to)) => from == to,
            _ => true,
        };
        first.controls == last.controls
            && first.generation == last.generation
            && first.instances == last.instances
            && first.processes == last.processes
            && nodes_kept
    }

    /// Every cycle whose controls or generation rose above the series' starting count.
    fn growth(&self) -> Vec<Growth> {
        let Some(first) = self.cycles.first() else {
            return Vec::new();
        };
        let mut growth = Vec::new();
        for cycle in &self.cycles {
            if cycle.controls > first.controls {
                growth.push(Growth {
                    viewport: self.viewport.clone(),
                    index: cycle.index,
                    counter: Counter::Controls,
                    from: u64::from(first.controls),
                    to: u64::from(cycle.controls),
                });
            }
            let both_measured =
                first.source == Source::Measured && cycle.source == Source::Measured;
            if both_measured && cycle.generation > first.generation {
                growth.push(Growth {
                    viewport: self.viewport.clone(),
                    index: cycle.index,
                    counter: Counter::Generation,
                    from: first.generation,
                    to: cycle.generation,
                });
            }
        }
        growth
    }

    /// Whether any cycle in this series was measured.
    fn measured(&self) -> bool {
        self.cycles
            .iter()
            .any(|cycle| cycle.source == Source::Measured)
    }

    /// How many of this series' cycles were measured.
    fn measured_cycles(&self) -> usize {
        self.cycles
            .iter()
            .filter(|cycle| cycle.source == Source::Measured)
            .count()
    }
}

/// What a run's sessions left behind when they were stopped.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Residue {
    /// The viewport it was driven in.
    pub viewport: String,
    /// Controls still mounted after the stops, summed over the run.
    ///
    /// Counted by asking the runtime about the ids each session created *before* it was stopped; the
    /// session has forgotten them afterwards, so asking it then would answer zero either way. Each
    /// session's ids are disjoint, so the sum is how many controls the run left live — an early
    /// cycle's leak is not overwritten by a later clean one.
    pub controls: u32,
    /// Whether every stop in the run also reported itself clean (no engine, no instances, no
    /// process).
    pub clean: bool,
}

/// Many cycles across many viewports, with what the driver could not read.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Load {
    series: BTreeMap<String, Series>,
    residue: BTreeMap<String, Residue>,
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

    /// Appends one cycle and numbers it.
    fn record(&mut self, viewport: &str, cycle: Cycle) {
        self.operations += 1;
        let series = self
            .series
            .entry(viewport.to_owned())
            .or_insert_with(|| Series {
                viewport: viewport.to_owned(),
                cycles: Vec::new(),
            });
        let index = series.cycles.len() as u32 + 1;
        series.cycles.push(Cycle { index, ..cycle });
    }

    /// Records one cycle whose counts the caller declared.
    ///
    /// This is a model of a driver, not a measurement: see [`Self::drive`] for the run that reads a
    /// real session.
    pub fn declared_cycle(
        &mut self,
        viewport: &str,
        controls: u32,
        nodes: u32,
        latency_ms: Option<u32>,
    ) {
        self.record(
            viewport,
            Cycle {
                index: 0,
                controls,
                generation: 0,
                instances: 0,
                processes: 0,
                nodes: Some(nodes),
                latency_ms,
                source: Source::Declared,
            },
        );
    }

    /// Records one cycle from a live preview session.
    ///
    /// The counts are read from the session and the runtime, never from the caller: live mounted
    /// controls through [`Preview::live_controls`], the engine generation, the render instances and
    /// whether a child process is held. They describe *this* session; a control an earlier session
    /// leaked is not in them, which is why [`Self::note_residue`] exists as the second half of the
    /// check.
    pub fn measure(&mut self, viewport: &str, preview: &Preview, latency_ms: Option<u32>) {
        let controls = u32::try_from(preview.live_controls()).unwrap_or(u32::MAX);
        let instances = u32::try_from(preview.instances.len()).unwrap_or(u32::MAX);
        self.record(
            viewport,
            Cycle {
                index: 0,
                controls,
                generation: preview.view_generation,
                instances,
                processes: u32::from(preview.process.is_some()),
                nodes: None,
                latency_ms,
                source: Source::Measured,
            },
        );
    }

    /// Drives `cycles` real mount/update/stop cycles of `document` and records what they measured.
    ///
    /// Each cycle starts a session, mounts `document` through the library's declarative layer,
    /// applies an update that changes the first node's text, measures the live session, and stops.
    /// The residue after the stop is recorded too: a run that ends with anything still mounted has
    /// not returned to where it started, whatever the per-cycle counts say.
    ///
    /// The document is reused as the declaration; the changed text is a new copy per cycle so the
    /// update is a real diff rather than an unchanged build the engine short-circuits.
    pub fn drive(
        &mut self,
        viewport: &str,
        cycles: u32,
        document: &Document,
    ) -> Result<(), String> {
        let factory = WidgetFactory::new_with_defaults();
        let create = |node: &rust_widgets::view::Node| -> Option<ObjectId> {
            let widget: Box<dyn Widget> = factory.create(
                &node.widget,
                Rect::new(0, 0, 200, 28),
                node.key_str().unwrap_or("anon"),
            )?;
            runtime::register(widget)
        };

        for step in 0..cycles {
            let started = Instant::now();
            let mut preview = Preview::new();
            preview.start(format!("load-{viewport}-{}", step + 1))?;
            preview.ready()?;
            preview.mount_view(document, &create)?;

            let mut next = document.clone();
            if let Some(node) = next.nodes.first_mut() {
                node.text = format!("{viewport} cycle {}", step + 1);
            }
            preview.update_view(&next, &create)?;

            let latency_ms = Some(u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX));
            self.measure(viewport, &preview, latency_ms);

            // Capture the ids while the session still remembers them, then ask the runtime whether
            // each is gone after the stop. Asking the stopped session would answer zero either way.
            let watched = preview.control_ids();
            preview.stop()?;
            preview.finish_stop()?;
            let left = watched
                .iter()
                .filter(|id| runtime::is_mounted(**id))
                .count();
            self.note_residue(
                viewport,
                u32::try_from(left).unwrap_or(u32::MAX),
                preview.is_clean(),
            );
        }
        Ok(())
    }

    /// Records what a stop left mounted, and whether the session also reported itself clean.
    ///
    /// `controls` is how many of the ids the session created are still mounted after its stop — the
    /// caller checks them against the runtime, because the stopped session no longer holds them.
    /// Each call adds to the viewport's residue: the sessions a run stops own disjoint control ids,
    /// so summing them is how many controls the run left live.
    pub fn note_residue(&mut self, viewport: &str, controls: u32, clean: bool) {
        let residue = self
            .residue
            .entry(viewport.to_owned())
            .or_insert_with(|| Residue {
                viewport: viewport.to_owned(),
                controls: 0,
                clean: true,
            });
        residue.controls = residue.controls.saturating_add(controls);
        residue.clean &= clean;
    }

    /// Records `cycles` declared cycles at a steady count.
    ///
    /// This is the "bind and rebuild a hundred times" model a caller can write without a session.
    /// What it produces must come back to where it started, but it is not evidence that a real
    /// session does — [`Self::drive`] is.
    pub fn repeat(
        &mut self,
        viewport: &str,
        cycles: u32,
        controls: u32,
        nodes: u32,
        latency_ms: Option<u32>,
    ) {
        for _ in 0..cycles {
            self.declared_cycle(viewport, controls, nodes, latency_ms);
        }
    }

    /// Records `cycles` declared cycles where the control count climbs by one each time.
    ///
    /// This is a driver that leaks, and it exists so the check that catches a leak has something to
    /// catch.
    pub fn leak(&mut self, viewport: &str, cycles: u32, controls: u32, nodes: u32) {
        for step in 0..cycles {
            self.declared_cycle(viewport, controls + step, nodes, None);
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

    /// Every cycle whose counters climbed.
    pub fn growth(&self) -> Vec<Growth> {
        self.series.values().flat_map(Series::growth).collect()
    }

    /// Whether any cycle was measured from a live session.
    pub fn measured(&self) -> bool {
        self.series.values().any(Series::measured)
    }

    /// How many cycles were measured.
    pub fn measured_cycles(&self) -> usize {
        self.series.values().map(Series::measured_cycles).sum()
    }

    /// What the run left mounted after stopping.
    pub fn residual(&self) -> Vec<&Residue> {
        self.residue
            .values()
            .filter(|residue| residue.controls > 0 || !residue.clean)
            .collect()
    }

    /// The facts this run does not measure, stated rather than assumed.
    pub fn unmeasured(&self) -> Vec<&'static str> {
        vec![
            "resident memory: no allocator hook is installed, so the bounded proxy is the set of \
             counters this run carries (live mounted controls, engine generations, render instances, \
             child process handles)",
            "thread count: the standard library exposes no portable counter, and the driver starts \
             no thread of its own",
            "signal subscriptions: this session holds none; the dispatcher's handles identify \
             records, not connected slots",
        ]
    }

    /// Whether the driver measured any latency at all.
    pub fn measured_latency(&self) -> bool {
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

    /// Whether the run passed: a live session was measured, it ended where it started, nothing was
    /// left mounted, and nothing was over budget.
    pub fn passed(&self) -> bool {
        self.measured()
            && self.steady()
            && self.growth().is_empty()
            && self.residual().is_empty()
            && self.over_budget().is_empty()
    }

    /// Why the run did not pass, naming the first reason that applies.
    ///
    /// A run that measured nothing is not evidence, so that is the first answer rather than "all
    /// zero counts are equal".
    pub fn verdict(&self) -> String {
        if !self.measured() {
            return "no live preview session was measured, so the run is not evidence".to_owned();
        }
        if !self.growth().is_empty() {
            return format!(
                "{} counter(s) grew, so the run does not end where it started",
                self.growth().len()
            );
        }
        if !self.steady() {
            return "the last cycle does not match the first, so the run does not end where it \
                    started"
                .to_owned();
        }
        if !self.residual().is_empty() {
            let controls: u32 = self.residual().iter().map(|residue| residue.controls).sum();
            return format!("{controls} control(s) were still mounted after the run stopped");
        }
        let over = self.over_budget().len();
        if over > 0 {
            return format!(
                "{over} cycle(s) took longer than the {}-ms budget",
                self.budget_ms
            );
        }
        "the run did not pass".to_owned()
    }

    /// The run in one line per fact.
    pub fn render(&self) -> Vec<String> {
        let declared = self.cycles().saturating_sub(self.measured_cycles());
        let mut lines = vec![format!(
            "budget: {} ms; viewports: {}; cycles: {} (measured {}, declared {}); invalid input: {} \
             (kept); residual controls: {}",
            self.budget_ms,
            self.series.len(),
            self.cycles(),
            self.measured_cycles(),
            declared,
            self.invalid.len(),
            self.residual()
                .iter()
                .map(|residue| residue.controls)
                .sum::<u32>()
        )];
        lines.push(format!(
            "steady: {}",
            if self.steady() { "yes" } else { "no" }
        ));
        for series in self.series.values() {
            let cycles = &series.cycles;
            lines.push(format!(
                "  {}: {}; live controls {}..{} peak {}; generation {}..{}; instances {}..{}; \
                 processes {}..{}",
                series.viewport,
                if series.measured() {
                    "measured"
                } else {
                    "declared"
                },
                series.first_controls().unwrap_or(0),
                series.last_controls().unwrap_or(0),
                series.peak_controls().unwrap_or(0),
                cycles.first().map(|cycle| cycle.generation).unwrap_or(0),
                cycles.last().map(|cycle| cycle.generation).unwrap_or(0),
                cycles.first().map(|cycle| cycle.instances).unwrap_or(0),
                cycles.last().map(|cycle| cycle.instances).unwrap_or(0),
                cycles.first().map(|cycle| cycle.processes).unwrap_or(0),
                cycles.last().map(|cycle| cycle.processes).unwrap_or(0),
            ));
        }
        for growth in self.growth() {
            lines.push(format!(
                "growth: {} cycle {}: {} {} -> {}",
                growth.viewport,
                growth.index,
                growth.counter.name(),
                growth.from,
                growth.to
            ));
        }
        for residue in self.residual() {
            lines.push(format!(
                "residue: {}: {} control(s) still mounted after the run's stops; every session \
                 clean: {}",
                residue.viewport,
                residue.controls,
                if residue.clean { "yes" } else { "no" }
            ));
        }
        lines.push(match (self.measured_latency(), self.worst_latency()) {
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
        for (index, text) in &self.invalid {
            lines.push(format!("invalid {index}: {text}"));
        }
        for fact in self.unmeasured() {
            lines.push(format!("unmeasured: {fact}"));
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use rust_widgets::widget::WidgetFactory;

    use super::{Counter, Load, Source};
    use crate::app::preview::preview_service::{Document, DocumentNode, Preview};
    use crate::contracts::identity::NodeId;

    fn document(text: &str) -> Document {
        Document::new("group_box").with(DocumentNode::new(NodeId::from_raw(1), "label", text))
    }

    /// The real library factory, so a mount creates registered controls rather than stub ids.
    fn creator(
        factory: &WidgetFactory,
    ) -> impl Fn(&rust_widgets::view::Node) -> Option<rust_widgets::core::ObjectId> + '_ {
        move |node| {
            let widget: Box<dyn rust_widgets::widget::Widget> = factory.create(
                &node.widget,
                rust_widgets::core::Rect::new(0, 0, 200, 28),
                node.key_str().unwrap_or("anon"),
            )?;
            rust_widgets::widget::runtime::register(widget)
        }
    }

    #[test]
    fn a_hundred_real_cycles_leave_nothing_mounted_and_are_steady() {
        // Generous budget so the assertion that matters is the counters, not the machine's speed.
        let mut load = Load::new().with_budget(5_000);
        load.drive("phone", 100, &document("alpha"))
            .expect("a driven run");

        assert_eq!(load.cycles(), 100);
        assert_eq!(load.measured_cycles(), 100);
        assert!(load.measured());
        assert!(load.steady(), "the run ended where it started");
        assert!(load.growth().is_empty(), "no counted counter climbed");
        assert!(
            load.residual().is_empty(),
            "no control survived the stop: {:?}",
            load.residual()
        );
        assert!(load.passed());

        let series = load.series("phone").expect("a series");
        // The counts are the session's, not the caller's: a mount created the root and the node,
        // and each cycle stopped at the same generation.
        assert!(
            series.cycles.iter().all(|cycle| cycle.controls == 2),
            "every cycle measured the same two live controls"
        );
        assert!(
            series
                .cycles
                .iter()
                .all(|cycle| cycle.source == Source::Measured)
        );
        assert!(
            series.cycles.iter().all(|cycle| cycle.latency_ms.is_some()),
            "the driver measured wall time around mount and update"
        );
        assert_eq!(series.peak_controls(), Some(2));
    }

    #[test]
    fn a_run_of_declared_cycles_is_not_evidence() {
        let mut load = Load::new();
        load.repeat("phone", 100, 12, 40, None);
        assert!(load.steady());
        assert!(!load.measured());
        assert!(
            !load.passed(),
            "a run nobody measured cannot pass the load check"
        );
        assert!(load.render()[0].contains("(measured 0, declared 100)"));
    }

    #[test]
    fn a_leak_in_the_model_is_reported_with_its_cycle_and_its_counts() {
        let mut load = Load::new();
        load.leak("phone", 3, 12, 40);
        assert!(!load.steady());
        let growth = load.growth();
        assert_eq!(growth.len(), 2);
        assert_eq!(growth[0].viewport, "phone");
        assert_eq!(growth[0].index, 2);
        assert_eq!(growth[0].counter, Counter::Controls);
        assert_eq!((growth[0].from, growth[0].to), (12, 13));
        assert_eq!((growth[1].index, growth[1].to), (3, 14));
    }

    #[test]
    fn one_viewport_climbing_is_enough_to_fail_the_run() {
        let mut load = Load::new();
        load.repeat("phone", 50, 12, 40, None);
        load.leak("desktop", 50, 20, 60);
        assert!(!load.steady());
        assert_eq!(load.growth().len(), 49);
        assert_eq!(load.growth()[0].viewport, "desktop");
    }

    #[test]
    fn a_measured_generation_that_climbs_is_growth_too() {
        let mut load = Load::new();
        let mut preview = Preview::new();
        preview.start("load").expect("a start");
        preview.ready().expect("a ready");
        load.measure("phone", &preview, None);
        // Pretend the same session kept updating: the generation counter is the second counter
        // that must not keep climbing when a driver is supposed to return to its starting state.
        let mut series = load.series("phone").expect("a series").clone();
        let mut later = series.cycles[0].clone();
        later.index = 2;
        later.generation += 4;
        series.cycles.push(later);
        load.series.insert("phone".to_owned(), series);
        assert!(!load.steady());
        assert_eq!(load.growth()[0].counter, Counter::Generation);
        assert!(
            load.render()
                .iter()
                .any(|line| { line.starts_with("growth: phone cycle 2: engine generation") })
        );
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
        assert!(lines[0].contains("invalid input: 2 (kept)"));
        assert!(lines.contains(&"invalid 1: bind:phone:missing<-nothing".to_owned()));
        assert!(lines.contains(&"invalid 2: rebuild:".to_owned()));
    }

    #[test]
    fn a_latency_over_budget_is_reported() {
        let mut load = Load::new().with_budget(16);
        load.declared_cycle("phone", 12, 40, Some(9));
        load.declared_cycle("phone", 12, 40, Some(21));
        load.declared_cycle("phone", 12, 40, Some(14));
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
        assert!(!load.measured_latency());
        assert_eq!(load.worst_latency(), None);
        assert!(load.over_budget().is_empty());
        assert!(load.render().contains(&"latency: not measured".to_owned()));
    }

    #[test]
    fn a_run_with_no_cycles_is_steady_and_measures_nothing() {
        let load = Load::new();
        assert!(load.steady());
        assert!(!load.passed(), "nothing was measured");
        assert!(!load.measured());
        assert_eq!(load.worst_latency(), None);
        assert_eq!(load.render()[1], "steady: yes");
    }

    #[test]
    fn the_peak_is_the_highest_count_seen() {
        let mut load = Load::new();
        load.declared_cycle("phone", 12, 40, None);
        load.declared_cycle("phone", 15, 40, None);
        load.declared_cycle("phone", 12, 40, None);
        let series = load.series("phone").expect("a series");
        assert_eq!(series.peak_controls(), Some(15));
        assert!(load.steady());
        assert_eq!(load.growth().len(), 1);
    }

    #[test]
    fn the_report_names_every_viewport_and_what_it_did_not_measure() {
        let mut load = Load::new().with_budget(16);
        load.repeat("phone", 3, 12, 40, None);
        load.repeat("desktop", 3, 20, 60, None);
        let lines = load.render();
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("  desktop: declared;"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("  phone: declared;"))
        );
        assert_eq!(
            lines[0],
            "budget: 16 ms; viewports: 2; cycles: 6 (measured 0, declared 6); invalid input: 0 \
             (kept); residual controls: 0"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("unmeasured: resident memory")),
            "what is not measured is named"
        );
    }

    #[test]
    fn a_single_cycle_is_steady_in_every_count() {
        let mut load = Load::new();
        load.declared_cycle("phone", 12, 40, None);
        assert!(load.steady());
        let mut nodes_changed = Load::new();
        nodes_changed.declared_cycle("phone", 12, 40, None);
        nodes_changed.declared_cycle("phone", 12, 41, None);
        assert!(!nodes_changed.steady());
        assert!(nodes_changed.growth().is_empty());
    }

    #[test]
    fn a_session_left_mounted_is_residue_rather_than_a_pass() {
        let factory = WidgetFactory::new_with_defaults();
        let make = creator(&factory);
        let mut preview = Preview::new();
        preview.start("load").expect("a start");
        preview.ready().expect("a ready");
        preview
            .mount_view(&document("alpha"), &make)
            .expect("a mount");

        let mut load = Load::new();
        load.measure("phone", &preview, Some(1));
        // A driver that never stops the session: the ids it created are still mounted, and that is
        // exactly what the residue check asks the runtime.
        let watched = preview.control_ids();
        let left = watched
            .iter()
            .filter(|id| rust_widgets::widget::runtime::is_mounted(**id))
            .count();
        load.note_residue(
            "phone",
            u32::try_from(left).unwrap_or(u32::MAX),
            preview.is_clean(),
        );

        assert_eq!(load.residual().len(), 1);
        assert_eq!(
            load.residual()[0].controls,
            2,
            "both controls were left live"
        );
        assert!(!load.passed(), "a session still mounted cannot pass");
        assert!(
            load.verdict().contains("still mounted"),
            "{}",
            load.verdict()
        );
        assert!(
            load.render()
                .iter()
                .any(|line| line.contains("2 control(s) still mounted")),
            "the residue is in the report"
        );

        preview.stop().expect("a stop");
        preview.finish_stop().expect("a finish");
    }

    #[test]
    fn a_driven_document_needs_a_factory_that_creates_real_controls() {
        // The driver uses the library's own factory, so a control the factory does not know is a
        // real error rather than a silent zero. This is the same check the mount makes, stated
        // here so a future factory change fails loudly.
        let factory = WidgetFactory::new_with_defaults();
        assert!(
            factory
                .create(
                    "label",
                    rust_widgets::core::Rect::new(0, 0, 10, 10),
                    "probe"
                )
                .is_some()
        );
        assert!(
            factory
                .create(
                    "not_a_widget",
                    rust_widgets::core::Rect::new(0, 0, 10, 10),
                    "nope"
                )
                .is_none()
        );
    }
}
