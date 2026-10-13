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

use super::permission::{Capability, Decision, PermissionGate, Principal};
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
    use super::{permission_lines, version_line};
    use crate::contracts::permission::Capability;

    #[test]
    fn the_version_line_names_both_versions() {
        let line = version_line();

        assert!(line.contains("format v1"), "{line}");
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
}
