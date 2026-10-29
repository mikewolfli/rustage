// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Disabled-extension startup and fault bisection (roadmap task T22.10).
//!
//! When a start fails with several extensions installed, the useful answer is *which* extension made
//! it fail — and the safe way to find out is to start again with some of them disabled, never to
//! delete anybody's extension. This module is that search, over a probe the caller supplies:
//!
//! * **A probe is one startup attempt** with a given set of enabled extension ids. `Ok(())` means
//!   it started, `Err(reason)` means it failed. The probe is the only thing that touches the
//!   system, so this module is testable without one and the command supplies a real process.
//! * **The empty set is asked too.** If a start fails with every extension disabled, then no
//!   extension is responsible and the finding says so instead of blaming one.
//! * **A single disable that fixes it names the culprit.** Otherwise the set is split and the
//!   failing half is bisected; when both halves pass alone but fail together, the finding is an
//!   `Interaction` naming both halves rather than a guessed single id.
//! * **The search is bounded.** Every probe is recorded and [`MAX_ATTEMPTS`] caps the run, so a
//!   pathological probe cannot make the command try forever; the finding then says the budget ran
//!   out.
//!
//! Nothing here changes the installed extensions: the caller disables ids for one probe and reports
//! the finding. Deciding to keep one disabled is a separate, explicit act.

#![allow(dead_code)]

/// The most probes one bisection may make.
pub const MAX_ATTEMPTS: usize = 256;

/// Why a failure with no extension enabled cannot be blamed on one.
const NO_EXTENSION: &str = "the start fails with every extension disabled, so none of them is \
                            responsible";

/// One startup attempt with a set of enabled extensions.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Attempt {
    /// The extension ids that were enabled for this attempt.
    pub enabled: Vec<String>,
    /// Whether the start succeeded.
    pub passed: bool,
    /// Why it failed, when it did.
    pub reason: Option<String>,
}

/// What the bisection concluded.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Finding {
    /// Every extension together starts successfully.
    Healthy,
    /// Disabling this one made the start succeed.
    Culprit {
        /// The extension that is responsible.
        id: String,
    },
    /// Each half starts alone, but together they fail.
    Interaction {
        /// The first half.
        left: Vec<String>,
        /// The second half.
        right: Vec<String>,
    },
    /// The search could not name a set, with the reason.
    Unresolved {
        /// Why no finding was reached.
        reason: String,
    },
}

impl Finding {
    /// The finding's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Culprit { .. } => "culprit",
            Self::Interaction { .. } => "interaction",
            Self::Unresolved { .. } => "unresolved",
        }
    }

    /// The finding in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Healthy => "every extension starts together".to_owned(),
            Self::Culprit { id } => format!("disabling {id} makes the start succeed"),
            Self::Interaction { left, right } => format!(
                "each half starts alone but {} together with {} fails",
                left.join(", "),
                right.join(", ")
            ),
            Self::Unresolved { reason } => format!("no extension could be named: {reason}"),
        }
    }
}

/// The attempts and what they concluded.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Outcome {
    /// Every startup attempt, in order.
    pub attempts: Vec<Attempt>,
    /// What the attempts concluded.
    pub finding: Finding,
}

impl Outcome {
    /// The outcome in one line per fact.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "bisection: {} attempt(s); {}",
            self.attempts.len(),
            self.finding.render()
        )];
        for attempt in &self.attempts {
            lines.push(format!(
                "  [{}] {} ({})",
                if attempt.passed { "passed" } else { "failed" },
                if attempt.enabled.is_empty() {
                    "no extension".to_owned()
                } else {
                    attempt.enabled.join(", ")
                },
                attempt.reason.as_deref().unwrap_or("started")
            ));
        }
        lines
    }
}

/// Finds the extension responsible for a failing start.
pub fn bisect<F>(ids: &[String], probe: F) -> Outcome
where
    F: FnMut(&[String]) -> Result<(), String>,
{
    let mut bisector = Bisector {
        probe,
        attempts: Vec::new(),
    };
    let full = ids.to_vec();
    let finding = match bisector.fails(&full) {
        Ok(false) => Finding::Healthy,
        Ok(true) => bisector.isolate(&full),
        Err(reason) => Finding::Unresolved { reason },
    };
    Outcome {
        attempts: bisector.attempts,
        finding,
    }
}

struct Bisector<F> {
    probe: F,
    attempts: Vec<Attempt>,
}

impl<F> Bisector<F>
where
    F: FnMut(&[String]) -> Result<(), String>,
{
    /// Runs the probe once. `Ok(true)` means it failed.
    fn fails(&mut self, enabled: &[String]) -> Result<bool, String> {
        if self.attempts.len() >= MAX_ATTEMPTS {
            return Err(format!(
                "the probe budget of {MAX_ATTEMPTS} attempt(s) is exhausted"
            ));
        }
        let outcome = (self.probe)(enabled);
        let (passed, reason) = match &outcome {
            Ok(()) => (true, None),
            Err(reason) => (false, Some(reason.clone())),
        };
        self.attempts.push(Attempt {
            enabled: enabled.to_vec(),
            passed,
            reason,
        });
        Ok(!passed)
    }

    /// Narrows a set known to fail.
    fn isolate(&mut self, ids: &[String]) -> Finding {
        if ids.is_empty() {
            return Finding::Unresolved {
                reason: NO_EXTENSION.to_owned(),
            };
        }
        if ids.len() == 1 {
            // A single id is only the culprit if removing it makes the start succeed. Otherwise the
            // host itself fails and no extension is responsible.
            return match self.fails(&[]) {
                Ok(true) => Finding::Unresolved {
                    reason: NO_EXTENSION.to_owned(),
                },
                Ok(false) => Finding::Culprit { id: ids[0].clone() },
                Err(reason) => Finding::Unresolved { reason },
            };
        }
        for index in 0..ids.len() {
            let mut reduced = ids.to_vec();
            reduced.remove(index);
            match self.fails(&reduced) {
                Ok(true) => {}
                Ok(false) => {
                    return Finding::Culprit {
                        id: ids[index].clone(),
                    };
                }
                Err(reason) => return Finding::Unresolved { reason },
            }
        }
        let middle = ids.len() / 2;
        let left = ids[..middle].to_vec();
        let right = ids[middle..].to_vec();
        match self.fails(&left) {
            Ok(true) => return self.isolate(&left),
            Ok(false) => {}
            Err(reason) => return Finding::Unresolved { reason },
        }
        match self.fails(&right) {
            Ok(true) => return self.isolate(&right),
            Ok(false) => {}
            Err(reason) => return Finding::Unresolved { reason },
        }
        Finding::Interaction { left, right }
    }
}

#[cfg(test)]
mod tests {
    use super::{Finding, MAX_ATTEMPTS, bisect};
    use std::collections::BTreeSet;

    fn ids(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    /// A probe that fails when any of `bad` is enabled.
    fn probe_over<'a>(
        bad: &'a BTreeSet<&'a str>,
    ) -> impl FnMut(&[String]) -> Result<(), String> + 'a {
        move |enabled: &[String]| {
            if enabled.iter().any(|id| bad.contains(id.as_str())) {
                Err("the extension crashed the start".to_owned())
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn a_healthy_start_needs_one_attempt() {
        let outcome = bisect(&ids(&["a", "b", "c"]), |_| Ok(()));
        assert_eq!(outcome.finding, Finding::Healthy);
        assert_eq!(outcome.attempts.len(), 1);
        assert!(outcome.attempts[0].passed);
    }

    #[test]
    fn one_bad_extension_is_named_and_every_attempt_is_recorded() {
        let bad: BTreeSet<&str> = ["b"].into_iter().collect();
        let outcome = bisect(&ids(&["a", "b", "c"]), probe_over(&bad));
        assert_eq!(outcome.finding, Finding::Culprit { id: "b".to_owned() });
        // The first attempt is the whole set, and the culprit's disabling attempt is in the list.
        assert_eq!(outcome.attempts[0].enabled, ids(&["a", "b", "c"]));
        assert!(!outcome.attempts[0].passed);
        assert!(
            outcome
                .attempts
                .iter()
                .any(|attempt| attempt.enabled == ids(&["a", "c"]) && attempt.passed)
        );
    }

    #[test]
    fn an_interaction_is_reported_when_no_single_extension_is_responsible() {
        // Fails when at least one of the first pair and one of the second pair are enabled, so
        // removing any single id still fails while each pair alone passes.
        let probe = |enabled: &[String]| {
            let has = |id: &str| enabled.iter().any(|other| other == id);
            if (has("a") || has("b")) && (has("c") || has("d")) {
                Err("the two extensions disagree about a shared resource".to_owned())
            } else {
                Ok(())
            }
        };
        let outcome = bisect(&ids(&["a", "b", "c", "d"]), probe);
        match &outcome.finding {
            Finding::Interaction { left, right } => {
                assert_eq!(left, &ids(&["a", "b"]));
                assert_eq!(right, &ids(&["c", "d"]));
            }
            other => panic!("expected an interaction, got {other:?}"),
        }
    }

    #[test]
    fn a_failure_with_every_extension_disabled_blames_none() {
        let outcome = bisect(&ids(&["a", "b"]), |_| {
            Err("the host itself is broken".to_owned())
        });
        assert_eq!(
            outcome.finding,
            Finding::Unresolved {
                reason: "the start fails with every extension disabled, so none of them is \
                         responsible"
                    .to_owned()
            }
        );
    }

    #[test]
    fn a_probe_that_never_passes_is_stopped_by_the_budget() {
        // 300 ids and a probe that always fails: the per-id loop would try every one, so the cap
        // must end the run rather than the command looping.
        let many: Vec<String> = (0..MAX_ATTEMPTS + 44)
            .map(|index| format!("id{index}"))
            .collect();
        let outcome = bisect(&many, |_| Err("still failing".to_owned()));
        assert!(matches!(outcome.finding, Finding::Unresolved { .. }));
        assert!(outcome.attempts.len() <= MAX_ATTEMPTS);
    }

    #[test]
    fn the_outcome_renders_each_attempt_and_the_finding() {
        let bad: BTreeSet<&str> = ["b"].into_iter().collect();
        let lines = bisect(&ids(&["a", "b"]), probe_over(&bad)).render();
        assert!(lines[0].contains("3 attempt(s)"), "{lines:?}");
        assert!(lines[0].contains("disabling b"), "{lines:?}");
        assert!(
            lines.iter().any(|line| line.contains("[failed]")),
            "{lines:?}"
        );
        assert!(
            lines.iter().any(|line| line.contains("[passed]")),
            "{lines:?}"
        );
    }
}
