// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The test wizard: declarations become data vectors, and the vectors run over both call seams
//! (roadmap task T21.05).
//!
//! The acceptance line is *"the IPC call and the static call share the core test vectors"*. Sharing
//! is a property of where the vectors come from, so this module never writes two sets:
//!
//! * **The wizard declares, and checks each step as it is declared.** A transition from a state that
//!   was never declared, on an event whose payload was never given, or to a state that does not
//!   exist, is refused at the moment it is entered — not later, in a run whose failure would be about
//!   something else.
//! * **Vectors are generated from the table, not typed by hand.** [`Wizard::generate`] walks the
//!   declared transitions breadth-first from the initial state, so every reachable state is exercised
//!   by a shortest path of real events, and a state the table cannot reach is reported rather than
//!   silently missing.
//! * **Guarded transitions are named, not faked.** A transition whose guard reads a runtime field is
//!   listed as not generated: its vector would have to assume the field's value, and the wizard does
//!   not invent state.
//! * **Both seams run the same vectors.** [`Wizard::run`] hands one set to
//!   [`crate::app::logic::regression::run`], which drives the core directly and again with every event
//!   carried through the typed channel; the report's verdict is the identity of the two walks.
//! * **Navigation is replayed, not assumed.** A declared page edge is replayed through a real
//!   [`Window`], so "the graph says you can go there" and "the window went there" are the same fact.
//! * **The artifact is the plan.** [`Wizard::save`] writes the declarations; [`Wizard::load`] reads them
//!   back strictly, with line numbers, so the vectors are reproducible instead of pasted.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;

use crate::app::data::routes::{Navigator, Route, Window};
use crate::app::logic::core::{Core, Event, Guard, Seam, State, Transition, Value};
use crate::app::logic::regression::{self, Report, Vector};

/// The artifact's header line.
const HEADER: &str = "rustage-vectors 1";

/// One event vector generated from the transition table.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct EventVector {
    /// A name that says where the walk goes.
    pub name: String,
    /// The events, starting at the plan's initial state.
    pub events: Vec<Event>,
    /// The state the declaration says the walk ends in.
    pub ends_at: String,
}

/// One navigation vector generated from the declared page graph.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct NavVector {
    /// A name that names both ends.
    pub name: String,
    /// The page the window opens on.
    pub from: String,
    /// The page it navigates to.
    pub to: String,
}

/// The vectors one plan generates.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct DataVectors {
    /// The event vectors, in discovery order.
    pub events: Vec<EventVector>,
    /// The navigation vectors, in declaration order.
    pub navigation: Vec<NavVector>,
    /// Declared states no event walk reaches.
    pub unreachable: Vec<String>,
    /// Transitions left out because their guard reads runtime state, as `state/event`.
    pub guarded: Vec<String>,
}

/// What one run produced.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct WizardRun {
    /// The plan's name.
    pub plan: String,
    /// The per-vector reports, one per event vector.
    pub vectors: Vec<Report>,
    /// One line per navigation vector.
    pub navigation: Vec<String>,
    /// Declared states no event walk reaches.
    pub unreachable: Vec<String>,
    /// Transitions whose guard kept them out of the vectors.
    pub guarded: Vec<String>,
    /// Whether every navigation replay succeeded.
    pub navigation_ok: bool,
}

impl WizardRun {
    /// Whether the run is evidence: no divergence and every navigation replayed.
    pub fn passed(&self) -> bool {
        self.vectors.iter().all(|report| report.identical) && self.navigation_ok
    }

    /// The run in lines.
    pub fn render(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for report in &self.vectors {
            lines.push(format!("vector {}", report.render()));
        }
        for line in &self.navigation {
            lines.push(format!("navigation {line}"));
        }
        for state in &self.unreachable {
            lines.push(format!("unreachable: {state} has no declared event walk"));
        }
        for guarded in &self.guarded {
            lines.push(format!(
                "not generated: {guarded} has a guard that reads runtime state"
            ));
        }
        let diverged = self
            .vectors
            .iter()
            .filter(|report| !report.identical)
            .count();
        let verdict = if self.passed() {
            "identical over both seams".to_owned()
        } else if diverged > 0 {
            format!("{diverged} vector(s) diverged between the static and IPC calls")
        } else {
            "a navigation did not replay".to_owned()
        };
        lines.push(format!(
            "wizard {}: {} event vector(s), {} navigation vector(s); {verdict}",
            self.plan,
            self.vectors.len(),
            self.navigation.len()
        ));
        lines
    }
}

/// A declared plan: the transition table, the page graph, the seam answers and the initial state.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Wizard {
    name: String,
    initial: Option<String>,
    states: BTreeSet<String>,
    events: BTreeMap<String, Value>,
    transitions: Vec<Transition>,
    navigator: Navigator,
    seam: BTreeMap<String, Value>,
}

impl Wizard {
    /// An empty plan with a name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    /// Its name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Its declared initial state, if one was set.
    pub fn initial(&self) -> Option<&str> {
        self.initial.as_deref()
    }

    /// Declares a state; the first one declared becomes the initial state.
    pub fn state(&mut self, name: &str) -> Result<(), String> {
        if name.trim().is_empty() {
            return Err("a state needs a name".to_owned());
        }
        if !self.states.insert(name.to_owned()) {
            return Err(format!("{name:?} is already declared"));
        }
        if self.initial.is_none() {
            self.initial = Some(name.to_owned());
        }
        Ok(())
    }

    /// Declares which state the walks start in.
    pub fn start_at(&mut self, state: &str) -> Result<(), String> {
        if !self.states.contains(state) {
            return Err(format!("no state {state:?} is declared"));
        }
        self.initial = Some(state.to_owned());
        Ok(())
    }

    /// Declares an event and the payload its vectors carry.
    pub fn event(&mut self, name: &str, payload: Value) -> Result<(), String> {
        if name.trim().is_empty() {
            return Err("an event needs a name".to_owned());
        }
        if self.events.insert(name.to_owned(), payload).is_some() {
            return Err(format!("the payload of {name:?} is already declared"));
        }
        Ok(())
    }

    /// Declares a transition, checking every name it mentions.
    pub fn transition(&mut self, from: &str, event: &str, to: &str) -> Result<(), String> {
        self.declare(Transition::new(from, event, to), from, event, to)
    }

    /// Declares a transition that only fires while a field holds a value.
    pub fn transition_if(
        &mut self,
        from: &str,
        event: &str,
        to: &str,
        field: &str,
        equals: Value,
    ) -> Result<(), String> {
        let mut transition = Transition::new(from, event, to);
        transition.guard = Guard::Field {
            field: field.to_owned(),
            equals,
        };
        self.declare(transition, from, event, to)
    }

    /// The check both transition declarations go through.
    fn declare(
        &mut self,
        transition: Transition,
        from: &str,
        event: &str,
        to: &str,
    ) -> Result<(), String> {
        if !self.states.contains(from) {
            return Err(format!("no state {from:?} is declared"));
        }
        if !self.states.contains(to) {
            return Err(format!("no state {to:?} is declared"));
        }
        if !self.events.contains_key(event) {
            return Err(format!("no payload for {event:?} is declared"));
        }
        if self
            .transitions
            .iter()
            .any(|known| known.state == from && known.event == event)
        {
            return Err(format!("{from}/{event} already has a transition"));
        }
        self.transitions.push(transition);
        Ok(())
    }

    /// Declares a page.
    pub fn page(&mut self, page: &str) -> Result<(), String> {
        if page.trim().is_empty() {
            return Err("a page needs a name".to_owned());
        }
        if self.navigator.has_page(page) {
            return Err(format!("the page {page:?} is already declared"));
        }
        self.navigator = std::mem::take(&mut self.navigator).with_page(page);
        Ok(())
    }

    /// Declares the page the application starts on.
    pub fn entry(&mut self, page: &str) -> Result<(), String> {
        if !self.navigator.has_page(page) {
            return Err(format!("no page {page:?} is declared"));
        }
        self.navigator = std::mem::take(&mut self.navigator).with_entry(page);
        Ok(())
    }

    /// Declares a navigation between two pages.
    pub fn edge(&mut self, from: &str, to: &str) -> Result<(), String> {
        for page in [from, to] {
            if !self.navigator.has_page(page) {
                return Err(format!("no page {page:?} is declared"));
            }
        }
        if self.navigator.can_navigate(from, to) {
            return Err(format!(
                "the navigation {from:?} -> {to:?} is already declared"
            ));
        }
        self.navigator = std::mem::take(&mut self.navigator).with_edge(from, to);
        Ok(())
    }

    /// Declares what the seam answers for a function.
    pub fn seam_answer(&mut self, function: &str, value: Value) {
        self.seam.insert(function.to_owned(), value);
    }

    /// The seam as the core is driven with it.
    pub fn seam(&self) -> Seam {
        let mut seam = Seam::new();
        for (function, value) in &self.seam {
            seam = seam.with(function.clone(), value.clone());
        }
        seam
    }

    /// The declared transition table.
    pub fn core(&self) -> Result<Core, String> {
        let initial = self
            .initial
            .as_deref()
            .ok_or_else(|| "the plan declares no initial state".to_owned())?;
        let mut core = Core::new(initial);
        for transition in &self.transitions {
            core.add(transition.clone())
                .map_err(|refusal| refusal.render())?;
        }
        Ok(core)
    }

    /// The declared page graph.
    pub fn navigator(&self) -> Navigator {
        self.navigator.clone()
    }

    /// The vectors this plan generates.
    ///
    /// One walk per reachable state, breadth-first from the initial state, so the walks are a
    /// shortest path each. A guarded transition is not walked: its vector would have to assume a
    /// runtime field, and this module does not invent state.
    pub fn generate(&self) -> Result<DataVectors, String> {
        let core = self.core()?;
        let initial = self
            .initial
            .clone()
            .ok_or_else(|| "the plan declares no initial state".to_owned())?;
        let mut vectors = DataVectors::default();
        let mut visited: BTreeSet<String> = BTreeSet::new();
        visited.insert(initial.clone());
        let mut queue: VecDeque<(String, Vec<Event>)> = VecDeque::new();
        queue.push_back((initial.clone(), Vec::new()));

        while let Some((state, path)) = queue.pop_front() {
            for transition in core.transitions() {
                if transition.state != state {
                    continue;
                }
                let payload =
                    self.events.get(&transition.event).cloned().ok_or_else(|| {
                        format!("no payload for {:?} is declared", transition.event)
                    })?;
                if !matches!(transition.guard, Guard::Always) {
                    let name = format!("{}/{}", transition.state, transition.event);
                    if !vectors.guarded.contains(&name) {
                        vectors.guarded.push(name);
                    }
                    continue;
                }
                if !visited.insert(transition.next.clone()) {
                    continue;
                }
                let mut events = path.clone();
                events.push(Event::new(transition.event.clone(), payload));
                vectors.events.push(EventVector {
                    name: format!("{initial}->{}", transition.next),
                    events: events.clone(),
                    ends_at: transition.next.clone(),
                });
                queue.push_back((transition.next.clone(), events));
            }
        }

        for state in &self.states {
            if !visited.contains(state) {
                vectors.unreachable.push(state.clone());
            }
        }
        for (from, to) in &self.navigator.edges {
            vectors.navigation.push(NavVector {
                name: format!("{from}->{to}"),
                from: from.clone(),
                to: to.clone(),
            });
        }
        Ok(vectors)
    }

    /// Generates the vectors and runs them over both seams and through a real window.
    pub fn run(&self) -> Result<WizardRun, String> {
        let core = self.core()?;
        let start = State::new(
            self.initial
                .clone()
                .ok_or_else(|| "the plan declares no initial state".to_owned())?,
        );
        let generated = self.generate()?;

        let vectors: Vec<Vector> = generated
            .events
            .iter()
            .map(|generated| {
                let mut vector = Vector::new(&generated.name);
                for event in &generated.events {
                    vector = vector.say(event.clone());
                }
                vector
            })
            .collect();
        let reports = if vectors.is_empty() {
            Vec::new()
        } else {
            regression::run(&vectors, &core, &start, &self.seam())
                .map_err(|error| error.render())?
        };

        let navigator = self.navigator();
        let mut navigation = Vec::new();
        let mut navigation_ok = true;
        for generated in &generated.navigation {
            let mut window = Window::open(generated.from.clone());
            window.ready();
            let outcome = window.navigate(&navigator, Route::new(generated.to.clone()));
            match outcome {
                Ok(())
                    if window.current().map(|route| route.page.as_str())
                        == Some(generated.to.as_str()) =>
                {
                    navigation.push(format!("{}: replayed", generated.name));
                }
                Ok(()) => {
                    navigation_ok = false;
                    navigation.push(format!(
                        "{}: the window is on {} after navigating",
                        generated.name,
                        window
                            .current()
                            .map(|route| route.page.clone())
                            .unwrap_or_else(|| "(nothing)".to_owned())
                    ));
                }
                Err(reason) => {
                    navigation_ok = false;
                    navigation.push(format!("{}: refused, {reason}", generated.name));
                }
            }
        }

        Ok(WizardRun {
            plan: self.name.clone(),
            vectors: reports,
            navigation,
            unreachable: generated.unreachable,
            guarded: generated.guarded,
            navigation_ok,
        })
    }

    /// Writes the plan as the vector artifact, answering its byte count.
    pub fn save(&self, path: &Path) -> Result<usize, String> {
        let mut text = String::from(HEADER);
        text.push('\n');
        let mut field = |name: &str, parts: &[&str]| {
            text.push_str(name);
            for part in parts {
                text.push('\t');
                text.push_str(part);
            }
            text.push('\n');
        };
        field("name", &[&self.name]);
        if let Some(initial) = &self.initial {
            field("initial", &[initial]);
        }
        for state in &self.states {
            field("state", &[state]);
        }
        for (name, payload) in &self.events {
            field("event", &[name, &payload_token(payload)]);
        }
        for transition in &self.transitions {
            field(
                "transition",
                &[&transition.state, &transition.event, &transition.next],
            );
        }
        for page in &self.navigator.pages {
            field("page", &[page]);
        }
        for entry in &self.navigator.entries {
            field("entry", &[entry]);
        }
        for (from, to) in &self.navigator.edges {
            field("edge", &[from, to]);
        }
        for (function, value) in &self.seam {
            field("seam", &[function, &payload_token(value)]);
        }
        std::fs::write(path, &text)
            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
        Ok(text.len())
    }

    /// Reads a plan back, refusing anything this module cannot read with its line.
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        let refuse = |number: usize, reason: &str| format!("{}:{number}: {reason}", path.display());
        let mut lines = text.lines();
        match lines.next() {
            Some(header) if header.trim() == HEADER => {}
            Some(header) => {
                return Err(format!(
                    "{}: expected the {HEADER:?} header, found {header:?}",
                    path.display()
                ));
            }
            None => return Err(format!("{}: the vector artifact is empty", path.display())),
        }
        let mut name = String::new();
        let mut initial: Option<String> = None;
        let mut states: Vec<String> = Vec::new();
        let mut events: Vec<(String, Value)> = Vec::new();
        let mut transitions: Vec<(String, String, String)> = Vec::new();
        let mut pages: Vec<String> = Vec::new();
        let mut entries: Vec<String> = Vec::new();
        let mut edges: Vec<(String, String)> = Vec::new();
        let mut seam: Vec<(String, Value)> = Vec::new();

        for (index, line) in lines.enumerate() {
            let number = index + 2;
            if line.trim().is_empty() {
                continue;
            }
            let parts: Vec<&str> = line.split('\t').collect();
            match parts.as_slice() {
                ["name", value] => name = (*value).to_owned(),
                ["initial", value] => initial = Some((*value).to_owned()),
                ["state", value] => states.push((*value).to_owned()),
                ["event", event, value] => {
                    let payload = parse_value(value).ok_or_else(|| {
                        refuse(number, "the event payload is not flag:/number:/text:")
                    })?;
                    events.push(((*event).to_owned(), payload));
                }
                ["transition", from, event, to] => {
                    transitions.push(((*from).to_owned(), (*event).to_owned(), (*to).to_owned()));
                }
                ["page", value] => pages.push((*value).to_owned()),
                ["entry", value] => entries.push((*value).to_owned()),
                ["edge", from, to] => edges.push(((*from).to_owned(), (*to).to_owned())),
                ["seam", function, value] => {
                    let answer = parse_value(value).ok_or_else(|| {
                        refuse(number, "the seam answer is not flag:/number:/text:")
                    })?;
                    seam.push(((*function).to_owned(), answer));
                }
                _ => {
                    return Err(refuse(
                        number,
                        "a line is <name>\\t<field>[,<field>...] and this is not one",
                    ));
                }
            }
        }
        if name.is_empty() {
            return Err(format!("{}: the artifact has no name", path.display()));
        }

        let mut plan = Wizard::new(name);
        for state in &states {
            plan.state(state)?;
        }
        if let Some(initial) = &initial {
            plan.start_at(initial)?;
        }
        for (event, payload) in events {
            plan.event(&event, payload)?;
        }
        for (from, event, to) in transitions {
            plan.transition(&from, &event, &to)?;
        }
        for page in &pages {
            plan.page(page)?;
        }
        for entry in &entries {
            plan.entry(entry)?;
        }
        for (from, to) in &edges {
            plan.edge(from, to)?;
        }
        for (function, value) in seam {
            plan.seam_answer(&function, value);
        }
        Ok(plan)
    }
}

/// A value as the artifact writes it.
fn payload_token(value: &Value) -> String {
    format!("{}:{}", value.type_name(), value.render())
}

/// A value as the artifact writes it, read back.
fn parse_value(text: &str) -> Option<Value> {
    Value::parse(text)
}

#[cfg(test)]
mod tests {
    use super::{DataVectors, Wizard};
    use crate::app::logic::core::Value;

    fn plan() -> Wizard {
        let mut plan = Wizard::new("checkout");
        for state in ["idle", "ready", "paid"] {
            plan.state(state).expect("a state");
        }
        plan.event("open", Value::Flag(true)).expect("an event");
        plan.event("pay", Value::Number(1)).expect("an event");
        plan.transition("idle", "open", "ready")
            .expect("a transition");
        plan.transition("ready", "pay", "paid")
            .expect("a transition");
        plan
    }

    #[test]
    fn a_transition_on_an_undeclared_name_is_refused_when_it_is_declared() {
        let mut plan = Wizard::new("checkout");
        plan.state("idle").expect("a state");
        plan.event("open", Value::Flag(true)).expect("an event");
        assert!(plan.transition("idle", "open", "ready").is_err());
        assert!(plan.transition("idle", "missing", "idle").is_err());
        // The payload is what makes a vector reproducible, so it is required.
        assert!(plan.event("open", Value::Flag(false)).is_err());
    }

    #[test]
    fn the_vectors_walk_every_reachable_state_by_a_shortest_path() {
        let plan = plan();
        let DataVectors {
            events,
            unreachable,
            guarded,
            ..
        } = plan.generate().expect("vectors");
        assert_eq!(events.len(), 2, "{events:?}");
        assert_eq!(events[0].name, "idle->ready");
        assert_eq!(events[0].events.len(), 1);
        assert_eq!(events[1].name, "idle->paid");
        assert_eq!(events[1].events.len(), 2);
        assert_eq!(events[1].events[1].name, "pay");
        assert!(unreachable.is_empty());
        assert!(guarded.is_empty());
    }

    #[test]
    fn a_state_no_event_reaches_is_reported_instead_of_skipped() {
        let mut plan = plan();
        plan.state("abandoned").expect("a state");
        let generated = plan.generate().expect("vectors");
        assert_eq!(generated.unreachable, vec!["abandoned"]);
    }

    #[test]
    fn a_guarded_transition_is_named_and_not_walked() {
        let mut plan = plan();
        plan.state("blocked").expect("a state");
        plan.event("retry", Value::Flag(true)).expect("an event");
        plan.transition_if("ready", "retry", "blocked", "attempts", Value::Number(3))
            .expect("a guarded transition");

        let generated = plan.generate().expect("vectors");
        // The guarded transition is named, because no vector may assume the field's value.
        assert_eq!(generated.guarded, vec!["ready/retry"]);
        // Its only edge being guarded is also why `blocked` is reported unreachable: the wizard
        // states both facts rather than pretending the state was covered.
        assert_eq!(generated.unreachable, vec!["blocked"]);
        // And it is not silently walked: no vector ends at `blocked`.
        assert!(
            generated
                .events
                .iter()
                .all(|vector| vector.ends_at != "blocked"),
            "{generated:?}"
        );
        let report = plan.run().expect("a run");
        let lines = report.render().join("\n");
        assert!(lines.contains("not generated: ready/retry"), "{lines}");
        assert!(lines.contains("2 event vector(s)"), "{lines}");
    }

    #[test]
    fn both_seams_produce_the_same_states_and_the_run_passes() {
        let plan = plan();
        let report = plan.run().expect("a run");
        assert!(report.passed(), "{:?}", report.render());
        assert_eq!(report.vectors.len(), 2);
        assert!(report.vectors.iter().all(|report| report.identical));
        let lines = report.render().join("\n");
        assert!(lines.contains("identical over both seams"), "{lines}");
    }

    #[test]
    fn navigation_is_replayed_through_a_real_window() {
        let mut plan = plan();
        plan.page("home").expect("a page");
        plan.page("details").expect("a page");
        plan.entry("home").expect("an entry");
        plan.edge("home", "details").expect("an edge");
        let report = plan.run().expect("a run");
        assert!(report.passed(), "{:?}", report.render());
        assert_eq!(report.navigation, vec!["home->details: replayed"]);
    }

    #[test]
    fn an_entry_or_edge_on_an_undeclared_page_is_refused() {
        let mut plan = plan();
        assert!(plan.page("home").is_ok());
        assert!(plan.edge("home", "ghost").is_err());
        assert!(plan.entry("ghost").is_err());
        assert!(plan.page("home").is_err());
    }

    #[test]
    fn the_artifact_round_trips_and_still_runs_the_same_vectors() {
        let mut plan = plan();
        plan.page("home").expect("a page");
        plan.page("details").expect("a page");
        plan.entry("home").expect("an entry");
        plan.edge("home", "details").expect("an edge");
        plan.seam_answer("tax", Value::Number(7));
        let directory = std::env::temp_dir().join(format!(
            "rustage-wizard-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("a clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).expect("a directory");
        let path = directory.join("vectors.txt");
        let bytes = plan.save(&path).expect("a save");
        assert!(bytes > 0);
        let loaded = Wizard::load(&path).expect("a load");
        assert_eq!(loaded, plan);
        assert_eq!(
            loaded.generate().expect("vectors"),
            plan.generate().expect("vectors")
        );
        assert!(loaded.run().expect("a run").passed());
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_malformed_artifact_is_refused_with_its_line() {
        let directory = std::env::temp_dir().join(format!(
            "rustage-wizard-bad-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("a clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).expect("a directory");
        let path = directory.join("vectors.txt");

        std::fs::write(&path, "nonsense\n").expect("a file");
        assert!(Wizard::load(&path).is_err());

        std::fs::write(&path, "rustage-vectors 1\nname\tp\nstate\tidle\nnonsense\n")
            .expect("a file");
        let error = Wizard::load(&path).expect_err("a malformed line");
        assert!(error.contains(":4:"), "{error}");

        std::fs::write(
            &path,
            "rustage-vectors 1\nname\tp\nstate\tidle\nevent\topen\tnot-a-value\n",
        )
        .expect("a file");
        let error = Wizard::load(&path).expect_err("a malformed value");
        assert!(error.contains("payload"), "{error}");
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_plan_without_a_initial_state_cannot_generate() {
        let mut plan = Wizard::new("empty");
        assert!(plan.generate().is_err());
        plan.state("idle").expect("a state");
        assert!(plan.generate().is_ok());
    }
}
