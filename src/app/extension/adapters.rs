// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Language, debug, build and deploy tool adapters: registration and compatibility probing
//! (roadmap task T22.03).
//!
//! The acceptance line is *"a capability the host does not support has a disable explanation, and a
//! failure does not take the host down"*. Both halves are properties of what registration answers:
//!
//! * **An incompatible adapter is registered, not dropped.** It keeps its place in the list and
//!   reports [`Status::Disabled`] with the reason — the host version it was built for, or the
//!   capability it needs that the host does not publish. A tool that silently disappeared would be
//!   the explanation no one can read.
//! * **Compatibility is checked against the host contract, not assumed.** The host range comes from
//!   [`Version`], the needs come from [`Host::capabilities`], and both are re-checked by
//!   [`Adapters::probe`] so an upgrade can disable or re-enable an adapter without a restart.
//! * **A failed adapter degrades only itself.** [`Adapters::fail`] records the reason on that id;
//!   every other adapter keeps its status and the registry keeps answering — the one thing the host
//!   must not do is stop working because a tool did.
//!
//! * **A duplicate id is refused.** Two adapters answering to one name would make "disable the
//!   language adapter" ambiguous, so registration is refused rather than resolved arbitrarily.
//!
//! Nothing here starts a process or speaks a protocol: what the host knows is what the host was
//! given. A real adapter's transport is [`super::supervisor`]'s job, not this list's.

#![allow(dead_code)]

use std::collections::BTreeMap;

use crate::app::extension::package::{Host, Version};

/// What kind of tool an adapter drives.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Kind {
    /// A language service.
    Language,
    /// A debugger.
    Debug,
    /// A build tool.
    Build,
    /// A deployment tool.
    Deploy,
}

impl Kind {
    /// The kind's name, as written.
    pub fn name(self) -> &'static str {
        match self {
            Self::Language => "language",
            Self::Debug => "debug",
            Self::Build => "build",
            Self::Deploy => "deploy",
        }
    }

    /// Reads a kind's name, or `None` when it is not one.
    pub fn parse(text: &str) -> Option<Self> {
        let kind = match text {
            "language" => Self::Language,
            "debug" => Self::Debug,
            "build" => Self::Build,
            "deploy" => Self::Deploy,
            _ => return None,
        };
        Some(kind)
    }
}

/// One adapter's declaration.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Adapter {
    /// Its stable id.
    pub id: String,
    /// What it drives.
    pub kind: Kind,
    /// Its version.
    pub version: Version,
    /// The oldest host it works with.
    pub host_min: Version,
    /// The newest host it works with.
    pub host_max: Version,
    /// The host capabilities it needs.
    pub needs: Vec<String>,
}

impl Adapter {
    /// An adapter that works with the host range it is given, needing nothing.
    pub fn new(
        id: impl Into<String>,
        kind: Kind,
        version: Version,
        host_min: Version,
        host_max: Version,
    ) -> Self {
        Self {
            id: id.into(),
            kind,
            version,
            host_min,
            host_max,
            needs: Vec::new(),
        }
    }

    /// The same adapter needing one more host capability.
    pub fn needing(mut self, capability: impl Into<String>) -> Self {
        self.needs.push(capability.into());
        self
    }

    /// Why the host cannot use it, or `None` when it can.
    ///
    /// The reason names both sides of the comparison, so a person can see what to upgrade.
    pub fn incompatibility(&self, host: &Host) -> Option<String> {
        if self.host_min > self.host_max {
            return Some(format!(
                "the declared host range is reversed: {} > {}",
                self.host_min.render(),
                self.host_max.render()
            ));
        }
        if !host.version.within(self.host_min, self.host_max) {
            return Some(format!(
                "host {} is outside {}..={}",
                host.version.render(),
                self.host_min.render(),
                self.host_max.render()
            ));
        }
        for need in &self.needs {
            if !host.capabilities.contains(need) {
                return Some(format!("the host does not provide {need:?}"));
            }
        }
        None
    }
}

/// Whether an adapter can be used, and why not when it cannot.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Status {
    /// The host can use it.
    Enabled,
    /// The host cannot use it, for this reason.
    Disabled {
        /// The reason, in the host's words.
        reason: String,
    },
    /// It was enabled, and a call failed for this reason.
    Failed {
        /// What went wrong.
        reason: String,
    },
}

impl Status {
    /// Whether the host can use the adapter right now.
    pub fn enabled(&self) -> bool {
        matches!(self, Self::Enabled)
    }

    /// The status in one word, plus the reason.
    pub fn render(&self) -> String {
        match self {
            Self::Enabled => "enabled".to_owned(),
            Self::Disabled { reason } => format!("disabled ({reason})"),
            Self::Failed { reason } => format!("failed ({reason})"),
        }
    }
}

/// The registered adapters.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Adapters {
    entries: BTreeMap<String, (Adapter, Status)>,
}

impl Adapters {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers an adapter, refusing an id that is already registered.
    ///
    /// The status is decided here from the host contract: an incompatible adapter is kept and
    /// disabled with its reason rather than refused, because the list is the explanation.
    pub fn register(&mut self, adapter: Adapter, host: &Host) -> Result<Status, String> {
        if self.entries.contains_key(&adapter.id) {
            return Err(format!("an adapter {:?} is already registered", adapter.id));
        }
        let status = match adapter.incompatibility(host) {
            Some(reason) => Status::Disabled { reason },
            None => Status::Enabled,
        };
        self.entries
            .insert(adapter.id.clone(), (adapter, status.clone()));
        Ok(status)
    }

    /// The adapter, whatever its status.
    pub fn adapter(&self, id: &str) -> Option<&Adapter> {
        self.entries.get(id).map(|(adapter, _)| adapter)
    }

    /// The status of an adapter.
    pub fn status(&self, id: &str) -> Option<&Status> {
        self.entries.get(id).map(|(_, status)| status)
    }

    /// The registered ids, in order.
    pub fn ids(&self) -> Vec<String> {
        self.entries.keys().cloned().collect()
    }

    /// Re-checks every adapter against the host, answering what changed.
    ///
    /// A failed adapter stays failed: the host has no evidence that the tool recovered, and clearing
    /// the reason would hide the failure. [`Self::ok`] is the only way back.
    pub fn probe(&mut self, host: &Host) -> Vec<String> {
        let mut changes = Vec::new();
        for (id, (adapter, status)) in &mut self.entries {
            if matches!(status, Status::Failed { .. }) {
                continue;
            }
            let next = match adapter.incompatibility(host) {
                Some(reason) => Status::Disabled { reason },
                None => Status::Enabled,
            };
            if &next != status {
                changes.push(format!("{id}: {} -> {}", status.render(), next.render()));
                *status = next;
            }
        }
        changes
    }

    /// Records that an adapter failed. The other adapters are not touched.
    pub fn fail(&mut self, id: &str, reason: impl Into<String>) -> Result<(), String> {
        let entry = self
            .entries
            .get_mut(id)
            .ok_or_else(|| format!("no adapter {id:?} is registered"))?;
        entry.1 = Status::Failed {
            reason: reason.into(),
        };
        Ok(())
    }

    /// Records that an adapter answered again.
    pub fn ok(&mut self, id: &str, host: &Host) -> Result<Status, String> {
        let entry = self
            .entries
            .get_mut(id)
            .ok_or_else(|| format!("no adapter {id:?} is registered"))?;
        let status = match entry.0.incompatibility(host) {
            Some(reason) => Status::Disabled { reason },
            None => Status::Enabled,
        };
        entry.1 = status.clone();
        Ok(status)
    }

    /// Whether a usable adapter drives this kind of tool.
    pub fn has_enabled(&self, kind: Kind) -> bool {
        self.entries
            .values()
            .any(|(adapter, status)| adapter.kind == kind && status.enabled())
    }

    /// The kinds with no usable adapter, in kind order.
    pub fn missing_kinds(&self) -> Vec<Kind> {
        [Kind::Language, Kind::Debug, Kind::Build, Kind::Deploy]
            .into_iter()
            .filter(|kind| !self.has_enabled(*kind))
            .collect()
    }

    /// The registry in one line per adapter.
    pub fn render(&self) -> Vec<String> {
        self.entries
            .values()
            .map(|(adapter, status)| {
                let mut line = format!(
                    "{} {} {} [{}] host {}..={}",
                    adapter.id,
                    adapter.kind.name(),
                    adapter.version.render(),
                    status.render(),
                    adapter.host_min.render(),
                    adapter.host_max.render()
                );
                if !adapter.needs.is_empty() {
                    line.push_str(&format!(" needs {}", adapter.needs.join(", ")));
                }
                line
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{Adapter, Adapters, Kind, Status};
    use crate::app::extension::package::{Host, Version};

    fn version(text: &str) -> Version {
        Version::parse(text).expect("a version")
    }

    fn host(capabilities: &[&str]) -> Host {
        Host {
            version: version("0.1.0"),
            capabilities: capabilities
                .iter()
                .map(|name| (*name).to_owned())
                .collect::<BTreeSet<_>>(),
            permissions: BTreeSet::new(),
        }
    }

    fn language() -> Adapter {
        Adapter::new(
            "acme.rust",
            Kind::Language,
            version("1.0.0"),
            version("0.1.0"),
            version("0.2.0"),
        )
    }

    #[test]
    fn kinds_are_a_closed_set() {
        for kind in [Kind::Language, Kind::Debug, Kind::Build, Kind::Deploy] {
            assert_eq!(Kind::parse(kind.name()), Some(kind));
        }
        assert_eq!(Kind::parse("linter"), None);
    }

    #[test]
    fn a_compatible_adapter_is_enabled() {
        let mut adapters = Adapters::new();
        let status = adapters
            .register(language(), &host(&["lsp"]))
            .expect("a registration");
        assert_eq!(status, Status::Enabled);
        assert!(adapters.has_enabled(Kind::Language));
        assert_eq!(
            adapters.missing_kinds(),
            vec![Kind::Debug, Kind::Build, Kind::Deploy]
        );
    }

    #[test]
    fn an_incompatible_adapter_is_kept_and_explained() {
        let mut adapters = Adapters::new();
        let mut host = host(&["lsp"]);
        host.version = version("0.3.0");
        let status = adapters
            .register(language(), &host)
            .expect("a registration");
        match &status {
            Status::Disabled { reason } => {
                assert!(reason.contains("0.3.0"), "{reason}");
                assert!(reason.contains("0.1.0..=0.2.0"), "{reason}");
            }
            other => panic!("expected a disable explanation, got {other:?}"),
        }
        // The adapter is still listed, which is what makes the explanation discoverable.
        assert_eq!(adapters.ids(), vec!["acme.rust"]);
        assert!(!adapters.has_enabled(Kind::Language));
    }

    #[test]
    fn a_capability_the_host_does_not_provide_is_named() {
        let mut adapters = Adapters::new();
        let adapter = language().needing("lsp").needing("dap");
        let status = adapters
            .register(adapter, &host(&["lsp"]))
            .expect("a registration");
        match status {
            Status::Disabled { reason } => assert!(reason.contains("\"dap\""), "{reason}"),
            other => panic!("expected a disable explanation, got {other:?}"),
        }
    }

    #[test]
    fn a_duplicate_id_is_refused_rather_than_resolved() {
        let mut adapters = Adapters::new();
        adapters
            .register(language(), &host(&["lsp"]))
            .expect("a registration");
        let error = adapters
            .register(language(), &host(&["lsp"]))
            .expect_err("a duplicate");
        assert!(error.contains("acme.rust"), "{error}");
    }

    #[test]
    fn a_probe_flips_both_ways_when_the_host_changes() {
        let mut adapters = Adapters::new();
        adapters
            .register(language(), &host(&["lsp"]))
            .expect("a registration");

        let mut newer = host(&["lsp"]);
        newer.version = version("0.3.0");
        let changes = adapters.probe(&newer);
        assert_eq!(changes.len(), 1, "{changes:?}");
        assert!(changes[0].contains("-> disabled"), "{changes:?}");
        assert!(!adapters.has_enabled(Kind::Language));

        let changes = adapters.probe(&host(&["lsp", "dap"]));
        assert!(changes[0].contains("-> enabled"), "{changes:?}");
        assert!(adapters.has_enabled(Kind::Language));
    }

    #[test]
    fn a_failed_adapter_degrades_only_itself() {
        let mut adapters = Adapters::new();
        adapters
            .register(language(), &host(&["lsp"]))
            .expect("a language adapter");
        adapters
            .register(
                Adapter::new(
                    "acme.debug",
                    Kind::Debug,
                    version("1.0.0"),
                    version("0.1.0"),
                    version("0.2.0"),
                ),
                &host(&["lsp"]),
            )
            .expect("a debug adapter");

        adapters
            .fail("acme.debug", "the tool exited")
            .expect("a failure");
        let status = adapters.status("acme.debug").expect("a status");
        assert!(matches!(status, Status::Failed { .. }), "{status:?}");
        assert!(status.render().contains("the tool exited"));
        // The other adapter is untouched, and the host still has a language service.
        assert_eq!(adapters.status("acme.rust"), Some(&Status::Enabled));
        assert!(adapters.has_enabled(Kind::Language));
        assert!(!adapters.has_enabled(Kind::Debug));

        // A probe does not quietly clear a failure: only an answer does.
        assert!(adapters.probe(&host(&["lsp"])).is_empty());
        assert!(matches!(
            adapters.status("acme.debug"),
            Some(Status::Failed { .. })
        ));
        let status = adapters
            .ok("acme.debug", &host(&["lsp"]))
            .expect("an answer");
        assert_eq!(status, Status::Enabled);
    }

    #[test]
    fn an_unknown_id_cannot_fail_or_recover() {
        let mut adapters = Adapters::new();
        let host = host(&["lsp"]);
        assert!(adapters.fail("ghost", "x").is_err());
        assert!(adapters.ok("ghost", &host).is_err());
    }

    #[test]
    fn the_list_shows_status_host_range_and_needs() {
        let mut adapters = Adapters::new();
        adapters
            .register(language().needing("lsp"), &host(&["lsp"]))
            .expect("a registration");
        let rows = adapters.render();
        assert_eq!(rows.len(), 1);
        assert!(
            rows[0].contains("acme.rust language 1.0.0 [enabled]"),
            "{rows:?}"
        );
        assert!(rows[0].contains("host 0.1.0..=0.2.0"), "{rows:?}");
        assert!(rows[0].contains("needs lsp"), "{rows:?}");
    }
}
