// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The contract vocabulary this build implements, as text `--status` can print.
//!
//! This is the one place where the contracts meet a real call chain today. It reports facts about
//! the build, not a demonstration: the format and generator versions are the constants this binary
//! was compiled with, and the permission lines are what a fresh, untrusted project with no
//! approvals actually answers when an agent asks — computed by the same
//! [`PermissionGate`](super::permission::PermissionGate) the store uses, not typed out by hand.
//!
//! Keeping the report derived means it cannot flatter the code: if the permission table changes,
//! this output changes with it, and the contract check against whitepaper §12.5 still has to pass.

use super::permission::{Capability, Decision, EffectClass, PermissionGate, Principal};
use super::revision::{FormatVersion, GeneratorVersion};

/// One line naming the format and generator versions this build was compiled with.
pub fn version_line() -> String {
    format!(
        "Contracts (T00.05): {format}, {generator}",
        format = FormatVersion::CURRENT,
        generator = GeneratorVersion::CURRENT,
    )
}

/// One line per capability: what it is, and what an agent is told today.
///
/// The project is untrusted and has no approvals, which is the state a project starts in. The
/// answer is therefore "denied" for everything except the reversible case the policy leaves open,
/// and each denial names the reason rather than an empty refusal.
/// One line per external effect a task can declare: what decides it, or why nothing does.
///
/// This is the half a capability list cannot show. `run-cargo-check` says *who* may run a command;
/// this says *which* effects there is a decision for at all — and the two effects the whitepaper's
/// decision table does not cover print the reason instead of a verdict, because "no row" is not the
/// same answer as "denied".
pub fn effect_lines() -> Vec<String> {
    EffectClass::ALL
        .iter()
        .map(|effect| match effect.decided_by() {
            Some(capability) => format!(
                "  {:<12} decided by {} ({})",
                effect.name(),
                capability.name(),
                capability.default_grant().name(),
            ),
            None => format!(
                "  {:<12} undecided: {}",
                effect.name(),
                effect
                    .undecided_reason()
                    .unwrap_or("the decision table has no row for it"),
            ),
        })
        .collect()
}

pub fn permission_lines() -> Vec<String> {
    let gate = PermissionGate::new();
    Capability::ALL
        .iter()
        .map(|capability| {
            let grant = capability.default_grant().name();
            match gate.decide(Principal::Agent, *capability) {
                Decision::Allowed { .. } => {
                    format!("  {:<22} {:<24} allowed", capability.name(), grant)
                }
                Decision::Denied { reason, .. } => {
                    format!(
                        "  {:<22} {:<24} denied: {}",
                        capability.name(),
                        grant,
                        reason.name()
                    )
                }
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{effect_lines, permission_lines, version_line};
    use crate::contracts::permission::{Capability, EffectClass};

    #[test]
    fn the_version_line_names_both_versions() {
        let line = version_line();

        assert!(line.contains("format v2"), "{line}");
        assert!(line.contains("generator 0.1.0"), "{line}");
    }

    #[test]
    fn every_capability_appears_once_with_its_grant_and_verdict() {
        let lines = permission_lines();

        assert_eq!(lines.len(), Capability::ALL.len());
        for (line, capability) in lines.iter().zip(Capability::ALL) {
            assert!(line.contains(capability.name()), "{line}");
            assert!(line.contains(capability.default_grant().name()), "{line}");
        }
        assert!(lines.iter().any(|line| line.ends_with("allowed")));
        assert!(lines.iter().any(|line| line.contains("denied: human-only")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("denied: generated-code-read-only"))
        );
    }

    #[test]
    fn every_external_effect_appears_once_with_its_decision_or_its_reason() {
        let lines = effect_lines();

        assert_eq!(lines.len(), EffectClass::ALL.len());
        for (line, effect) in lines.iter().zip(EffectClass::ALL) {
            assert!(line.contains(effect.name()), "{line}");
            match effect.decided_by() {
                Some(capability) => assert!(line.contains(capability.name()), "{line}"),
                None => {
                    assert!(line.contains("undecided"), "{line}");
                    assert!(
                        effect
                            .undecided_reason()
                            .is_some_and(|reason| line.contains(reason)),
                        "the reason has to be the one the effect itself gives: {line}",
                    );
                }
            }
        }
        // Deployment is the one effect the decision table still has no row for, and the line has to
        // say so rather than look like a refusal of a capability that exists.
        let undecided: Vec<&String> = lines
            .iter()
            .filter(|line| line.contains("undecided"))
            .collect();
        assert_eq!(undecided.len(), 1, "{lines:?}");
        assert!(
            undecided[0].contains(EffectClass::Deploy.name()),
            "{undecided:?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains(Capability::RequestNetworkService.name())),
            "the network row now decides an effect, so it has to be named: {lines:?}",
        );
    }
}
