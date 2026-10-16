// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Analysis configuration after a Cargo member, feature, target or Kit switch (task T07.10).
//!
//! The acceptance line is *"macro and build-script analysis is gated on trust, a diagnostic for an old
//! configuration is not marked current, and a failure still allows editing"*. Three different
//! guarantees, so three different mechanisms:
//!
//! * **Trust gates the code execution analysis needs.** Macro expansion and build scripts *run code*,
//!   so [`Analysis::configure`] turns [`Config::proc_macros`] and [`Config::build_scripts`] off when the
//!   project is [`Trust::Untrusted`]. The gate is in the configuration the server is given, not in a
//!   warning someone might ignore.
//! * **A diagnostic belongs to a configuration generation.** Every [`Analysis::configure`] bumps the
//!   [`Analysis::generation`], and replies are keyed by it through the one
//!   [`Pending`](crate::app::language::features::Pending) rule, so a diagnostic computed for an earlier
//!   configuration is [`Delivery::Stale`] and is not marked current.
//! * **A failed configuration does not stop editing.** An unknown target is recorded as
//!   [`Analysis::failure`] and the configuration still moves on (which is why the old diagnostics go
//!   stale), while the editor's own work is untouched — the same separation [T07.08](crate::app::language::service_health)
//!   checks between a failing service and editing.
//!
//! The `Cargo.toml` reading that produces a member list is [T02.x](crate::project::build::workspace)'; this
//! module is what the analysis does with the selection.

#![allow(dead_code)]

use std::collections::BTreeSet;

use crate::app::language::diagnostics::DiagnosticSet;
use crate::app::language::features::{Delivery, Pending, Request, Response};

/// Whether a project is trusted (allowed to run code for analysis).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Trust {
    /// The project is trusted: macros and build scripts may run.
    Trusted,
    /// The project is not trusted: analysis that runs code is off.
    Untrusted,
}

impl Trust {
    /// The trust's name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Trusted => "trusted",
            Self::Untrusted => "untrusted",
        }
    }

    /// The trust a name picks.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "trusted" => Some(Self::Trusted),
            "untrusted" => Some(Self::Untrusted),
            _ => None,
        }
    }
}

/// What the analysis is being configured for.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Selection {
    /// The Cargo member.
    pub member: String,
    /// The features that are on.
    pub features: BTreeSet<String>,
    /// The target triple or alias.
    pub target: String,
    /// The Kit's name, when one is active.
    pub kit: Option<String>,
}

impl Selection {
    /// A selection of a member and a target.
    pub fn new(member: impl Into<String>, target: impl Into<String>) -> Self {
        Self {
            member: member.into(),
            features: BTreeSet::new(),
            target: target.into(),
            kit: None,
        }
    }
}

/// One analysis configuration, with the generation it belongs to.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Config {
    /// The generation this configuration is.
    pub generation: u64,
    /// The member.
    pub member: String,
    /// The features, in order.
    pub features: Vec<String>,
    /// The target.
    pub target: String,
    /// The Kit.
    pub kit: Option<String>,
    /// Whether proc-macro expansion runs.
    pub proc_macros: bool,
    /// Whether build scripts run.
    pub build_scripts: bool,
}

impl Config {
    /// The configuration in one line.
    pub fn render(&self) -> String {
        format!(
            "gen {}: member {}, target {}, features [{}], kit {}, macros {}, build-scripts {}",
            self.generation,
            self.member,
            self.target,
            self.features.join(", "),
            self.kit.as_deref().unwrap_or("none"),
            self.proc_macros,
            self.build_scripts
        )
    }
}

/// The analysis: the current configuration, its generation, and the diagnostics for it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Analysis {
    generation: u64,
    current: Option<Config>,
    failure: Option<String>,
    pending: Pending,
    shown: Option<DiagnosticSet>,
}

impl Analysis {
    /// No configuration yet.
    pub fn new() -> Self {
        Self {
            generation: 0,
            current: None,
            failure: None,
            pending: Pending::new(),
            shown: None,
        }
    }

    /// Rebuilds the configuration for a selection, a trust, and the targets this build knows.
    ///
    /// The generation always moves, so diagnostics for the previous configuration go stale even when
    /// this one fails.
    pub fn configure(
        &mut self,
        selection: Selection,
        trust: Trust,
        known_targets: &[String],
    ) -> Config {
        self.generation += 1;
        let trusted = trust == Trust::Trusted;
        self.failure = if known_targets.iter().any(|t| t == &selection.target) {
            None
        } else {
            Some(format!(
                "the target {} is not known to this build",
                selection.target
            ))
        };
        let config = Config {
            generation: self.generation,
            member: selection.member,
            features: selection.features.into_iter().collect(),
            target: selection.target,
            kit: selection.kit,
            // Macro expansion and build scripts run code, so they need trust.
            proc_macros: trusted,
            build_scripts: trusted,
        };
        self.current = Some(config.clone());
        config
    }

    /// The current generation.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The current configuration.
    pub fn current(&self) -> Option<&Config> {
        self.current.as_ref()
    }

    /// Why the last configuration failed, when it did.
    pub fn failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }

    /// Asks for diagnostics against the current generation.
    pub fn request(&mut self) -> Request {
        self.pending.request(self.generation)
    }

    /// Asks for diagnostics against a named generation (a delayed request).
    pub fn request_at(&mut self, generation: u64) -> Request {
        self.pending.request(generation)
    }

    /// Takes a diagnostic reply for a configuration generation.
    pub fn accept(&mut self, response: Response<DiagnosticSet>) -> Delivery<usize> {
        let current = self.generation;
        match self.pending.deliver(response, current) {
            Delivery::Fresh(set) => {
                let count = set.len();
                self.shown = Some(set);
                Delivery::Fresh(count)
            }
            Delivery::Stale { computed, current } => Delivery::Stale { computed, current },
            Delivery::Cancelled => Delivery::Cancelled,
        }
    }

    /// The diagnostics shown for the current configuration, when there is one.
    pub fn shown(&self) -> Option<&DiagnosticSet> {
        self.shown.as_ref()
    }

    /// The analysis in one report.
    pub fn render(&self) -> Vec<String> {
        let mut lines = Vec::new();
        match &self.current {
            Some(config) => lines.push(config.render()),
            None => lines.push("no configuration".to_owned()),
        }
        if let Some(failure) = &self.failure {
            lines.push(format!("  failure: {failure}"));
        }
        match &self.shown {
            Some(set) => lines.push(format!("  diagnostics: {} for this generation", set.len())),
            None => lines.push("  diagnostics: none".to_owned()),
        }
        lines
    }
}

impl Default for Analysis {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{Analysis, Selection, Trust};
    use crate::app::language::diagnostics::DiagnosticSet;
    use crate::app::language::features::{Delivery, Response};
    use crate::project::text::piece_table::TextStore;

    fn targets() -> Vec<String> {
        vec![
            "x86_64-unknown-linux-gnu".to_owned(),
            "wasm32-unknown-unknown".to_owned(),
        ]
    }

    #[test]
    fn macro_and_build_script_analysis_is_gated_on_trust() {
        let mut analysis = Analysis::new();
        let untrusted = analysis.configure(
            Selection::new("app", "x86_64-unknown-linux-gnu"),
            Trust::Untrusted,
            &targets(),
        );
        assert!(!untrusted.proc_macros);
        assert!(!untrusted.build_scripts);
        let trusted = analysis.configure(
            Selection::new("app", "x86_64-unknown-linux-gnu"),
            Trust::Trusted,
            &targets(),
        );
        assert!(trusted.proc_macros);
        assert!(trusted.build_scripts);
    }

    #[test]
    fn a_diagnostic_for_an_old_configuration_is_not_current() {
        let mut analysis = Analysis::new();
        analysis.configure(
            Selection::new("app", "x86_64-unknown-linux-gnu"),
            Trust::Trusted,
            &targets(),
        );
        let request = analysis.request();
        // The person switches the target; the configuration generation moves.
        analysis.configure(
            Selection::new("app", "wasm32-unknown-unknown"),
            Trust::Trusted,
            &targets(),
        );
        assert_eq!(
            analysis.accept(Response::new(request, DiagnosticSet::default())),
            Delivery::Stale {
                computed: 1,
                current: 2
            }
        );
        assert!(analysis.shown().is_none());
    }

    #[test]
    fn a_feature_and_kit_switch_changes_the_configuration() {
        let mut analysis = Analysis::new();
        let mut selection = Selection::new("app", "x86_64-unknown-linux-gnu");
        selection.features.insert("gui".to_owned());
        selection.kit = Some("desktop".to_owned());
        let config = analysis.configure(selection, Trust::Trusted, &targets());
        assert_eq!(config.features, vec!["gui".to_owned()]);
        assert_eq!(config.kit.as_deref(), Some("desktop"));
        assert_eq!(config.generation, 1);
    }

    #[test]
    fn a_failure_still_lets_editing_work_and_stales_old_diagnostics() {
        let mut analysis = Analysis::new();
        analysis.configure(
            Selection::new("app", "x86_64-unknown-linux-gnu"),
            Trust::Trusted,
            &targets(),
        );
        let request = analysis.request();
        // A configuration for a target this build does not know fails...
        analysis.configure(Selection::new("app", "mystery"), Trust::Trusted, &targets());
        assert!(analysis.failure().is_some());
        // ...but it still moves the generation, so the old diagnostics are stale...
        assert!(matches!(
            analysis.accept(Response::new(request, DiagnosticSet::default())),
            Delivery::Stale { .. }
        ));
        // ...and the editor still edits.
        let mut store = TextStore::new("abc");
        assert!(store.splice(0, 0, "Z").is_ok());
        assert_eq!(store.text(), "Zabc");
    }

    #[test]
    fn a_fresh_generation_takes_its_diagnostics() {
        let mut analysis = Analysis::new();
        analysis.configure(
            Selection::new("app", "x86_64-unknown-linux-gnu"),
            Trust::Trusted,
            &targets(),
        );
        let request = analysis.request();
        assert_eq!(
            analysis.accept(Response::new(request, DiagnosticSet::default())),
            Delivery::Fresh(0)
        );
    }

    #[test]
    fn a_trust_name_round_trips() {
        assert_eq!(Trust::from_name("trusted"), Some(Trust::Trusted));
        assert_eq!(Trust::from_name("untrusted"), Some(Trust::Untrusted));
        assert_eq!(Trust::from_name("maybe"), None);
    }
}
