// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The states-and-transitions panel's rows (roadmap F13, `panel.states`).
//!
//! The rows come from the page document's own `states` member (page format v2). The declaration is
//! turned into the host's [`Machine`] only to reuse its diagnostics — reachability and dead ends —
//! so the panel does not keep a second copy of the graph.
//!
//! **Boundary.** Page format v2 stores the declaration (`declared` / `initial` / `final` /
//! `transitions`), not the actions an event runs, the target's capability or a running session.
//! Those three facts are reported as *not declared / not running* rather than invented; a session
//! is started by the runtime, not by this panel.

use crate::app::data::state_machine::{Machine, Transition};
use crate::project::record::page::PageStates;

/// The rows for one page's declaration.
///
/// `source` names the page document the declaration was read from, so a row never claims a machine
/// that belongs to some other file.
pub(super) fn rows(source: &str, states: Option<&PageStates>) -> Vec<String> {
    let mut rows = vec![format!("page: {source}")];
    let Some(states) = states else {
        rows.push("no state machine is declared in this page".to_owned());
        rows.push("runtime: not running".to_owned());
        return rows;
    };

    let machine = Machine {
        states: states.declared().to_vec(),
        initial: states.initial().map(str::to_owned),
        finals: states.finals().to_vec(),
        transitions: states
            .transitions()
            .iter()
            .map(|transition| Transition {
                from: transition.from().to_owned(),
                on: transition.on().to_owned(),
                to: transition.to().to_owned(),
                guard: transition.guard().map(str::to_owned),
            })
            .collect(),
    };
    rows.extend(machine.render());
    // The three facts the page format deliberately does not carry. Saying them is the difference
    // between "nothing is declared" and "the panel cannot see it".
    rows.push("actions: not declared in page format v2".to_owned());
    rows.push("target capability: not checked".to_owned());
    rows.push("runtime: not running".to_owned());
    rows
}

#[cfg(test)]
mod tests {
    use super::rows;
    use crate::project::record::page::{PageStates, PageTransition};

    #[test]
    fn a_page_without_a_machine_says_so() {
        let rows = rows("pages/main.json", None);
        assert_eq!(rows[0], "page: pages/main.json");
        assert!(rows.iter().any(|row| row.contains("no state machine")));
        assert!(rows.iter().any(|row| row == "runtime: not running"));
    }

    #[test]
    fn a_machine_renders_its_declaration_and_the_gaps_it_cannot_fill() {
        let states = PageStates::new(
            vec!["idle".to_owned(), "running".to_owned(), "done".to_owned()],
            Some("idle".to_owned()),
            vec!["done".to_owned()],
            vec![
                PageTransition::new("idle", "start", "running", None),
                PageTransition::new("running", "finish", "done", Some("is_complete".to_owned())),
            ],
        )
        .expect("a valid machine");
        let rows = rows("pages/main.json", Some(&states));
        assert_eq!(rows[0], "page: pages/main.json");
        assert!(rows.iter().any(|row| row.starts_with("states: 3;")));
        assert!(
            rows.iter()
                .any(|row| row.contains("idle --start--> running")),
            "{rows:?}"
        );
        assert!(
            rows.iter()
                .any(|row| row.contains("running --finish--> done [is_complete]")),
            "{rows:?}"
        );
        assert!(rows.iter().any(|row| row.contains("actions: not declared")));
        assert!(
            rows.iter()
                .any(|row| row.contains("target capability: not checked"))
        );
        assert!(rows.iter().any(|row| row == "runtime: not running"));
    }
}
