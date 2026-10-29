// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Permissions: who may change what, contract **C-PERMISSION**.
//!
//! The permission table is not invented here. It is the decision table in whitepaper §12.5, which
//! fixes what an agent may do on its own and what has to go through a human, and it is checked
//! against that table by `node tools/contracts/check-contracts.mjs`. Restating it in code without
//! that check would be a second, drifting copy of a security-relevant table.
//!
//! Three properties matter more than the individual rows:
//!
//! 1. **Default is deny.** A capability with no grant row, an untrusted project, a revoked
//!    authorization — all of them end in [`Decision::Denied`], never in an accidental allow.
//! 2. **A denial is explicit and explained.** It carries the capability and a reason that names
//!    what would have to change ([`DenialReason::explanation`]). A caller cannot mistake a denial
//!    for an empty success (principle #22).
//! 3. **Grants are revocable.** Approvals and execution authorizations are stored so a withdrawal
//!    is a real state change, not a matter of remembering not to ask again.

use std::collections::BTreeSet;

/// A principal is who is asking. The designer runs on behalf of a human and, when AI assistance is
/// on, on behalf of an agent; the same operation can be allowed for one and refused for the other.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Principal {
    /// The person using the designer, acting through a menu, a panel or a command.
    Human,
    /// An agent's proposal, which must be carried by a change set to have any effect at all.
    Agent,
}

/// Something a principal can attempt. One variant per row of whitepaper §12.5, so a row cannot be
/// silently dropped without a capability losing its meaning.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Capability {
    /// Changing a control's property on the canvas: reversible, no side effects, recorded.
    EditWidgetProperty,
    /// Adding or removing control nodes: changes layout and the shape of the generated code.
    AddOrRemoveNode,
    /// Changing the generated Rust output, which the next generation would overwrite.
    EditGeneratedCode,
    /// Changing hand-written source, which no generator protects.
    EditHandwrittenCode,
    /// Writing files to disk.
    WriteFile,
    /// Running `cargo check`: it can run build scripts, proc macros and reach the network.
    RunCargoCheck,
    /// Making a request to a service the project's network policy declares.
    ///
    /// The row this comes from is "a request to a service in the project's network policy", and the
    /// table did not have it until T01.09: the roadmap asks for network authorization in several
    /// places (§5 F16 "no silent network", §5 F20 "network authorization and diagnosable auth
    /// failures", §5 F23 "everything sent out is authorized item by item"), and a decision list
    /// without a row for it could only refuse the effect with "no decision exists".
    RequestNetworkService,
    /// Adding a dependency or editing `Cargo.toml`, which affects every profile.
    ChangeDependencies,
}

impl Capability {
    /// Every capability, in the order of the whitepaper's table. The contract check depends on this
    /// list being complete.
    pub const ALL: [Self; 8] = [
        Self::EditWidgetProperty,
        Self::AddOrRemoveNode,
        Self::EditGeneratedCode,
        Self::EditHandwrittenCode,
        Self::WriteFile,
        Self::RunCargoCheck,
        Self::ChangeDependencies,
        Self::RequestNetworkService,
    ];

    /// The machine name used by the contract data file and the checks. Stable: it is how the table
    /// in the data file and this enum are kept in step.
    pub const fn name(self) -> &'static str {
        match self {
            Self::EditWidgetProperty => "edit-widget-property",
            Self::AddOrRemoveNode => "add-or-remove-node",
            Self::EditGeneratedCode => "edit-generated-code",
            Self::EditHandwrittenCode => "edit-handwritten-code",
            Self::WriteFile => "write-file",
            Self::RunCargoCheck => "run-cargo-check",
            Self::RequestNetworkService => "request-network-service",
            Self::ChangeDependencies => "change-dependencies",
        }
    }

    /// The default grant for this capability, from whitepaper §12.5.
    pub const fn default_grant(self) -> Grant {
        match self {
            Self::EditWidgetProperty => Grant::AgentAutonomous,
            Self::AddOrRemoveNode => Grant::NeedsReview,
            Self::EditGeneratedCode => Grant::AgentReadOnly,
            Self::EditHandwrittenCode | Self::ChangeDependencies => Grant::NeedsConfirmation,
            Self::WriteFile => Grant::HumanOnly,
            Self::RunCargoCheck | Self::RequestNetworkService => Grant::AuthorizedWhenTrusted,
        }
    }
}

/// A class of external effect a task can have, in the four categories roadmap T01.05 names:
/// running a command, writing a file, reaching the network, and deploying.
///
/// [`Capability`] is the decision table of whitepaper §12.5; this is the *effect* a caller declares
/// before the host will perform one, because a caller that cannot say what it is about to do cannot
/// be authorized for it. [`EffectClass::decided_by`] is the whole point: an effect that maps to no
/// capability has no decision behind it, and the host refuses it instead of inventing a grant.
///
/// The mapping of `Command` and `FileWrite` to §12.5 rows is a reading of that table and is written
/// down here so it can be argued with:
///
/// * `Command` → [`Capability::RunCargoCheck`]. The row is named after `cargo check`, but its
///   stated reason is "can run build scripts / proc macros or reach the network, and is not
///   read-only; it must go through the host's task and permission gate". That reason is about
///   *running a command*, with `cargo check` as its instance, so the class maps to it. Nothing in
///   the table is about running commands in general, and inventing a second row for one would be a
///   second copy of a security decision.
/// * `FileWrite` → [`Capability::WriteFile`], whose grant is human-only.
/// * `Network` → [`Capability::RequestNetworkService`]. T01.09 added that row to §12.5, because the
///   roadmap asks for network authorization in several places and a table with no row for it could
///   only refuse the effect with "no decision exists". The row's reason has the same shape as the
///   command row's: a request can carry a credential and send data out, so it is not read-only.
/// * `Deploy` → none. Deployment is a deferred capability (`C-REMOTE`, roadmap §1.3 "future
///   extension"), so there is no decision to refer to either.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum EffectClass {
    /// Starting a program, which may run project code.
    Command,
    /// Writing a file.
    FileWrite,
    /// Reaching the network.
    Network,
    /// Deploying to a device, a container or a remote host.
    Deploy,
}

impl EffectClass {
    /// Every external effect a caller can declare.
    pub const ALL: [Self; 4] = [Self::Command, Self::FileWrite, Self::Network, Self::Deploy];

    /// The machine name, used by the host's authorization record and its reports.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Command => "command",
            Self::FileWrite => "file-write",
            Self::Network => "network",
            Self::Deploy => "deploy",
        }
    }

    /// What the effect is, in words a trust prompt can show.
    pub const fn description(self) -> &'static str {
        match self {
            Self::Command => "running a program",
            Self::FileWrite => "writing a file",
            Self::Network => "reaching the network",
            Self::Deploy => "deploying to a device, a container or a remote host",
        }
    }

    /// The §12.5 capability that decides this effect, when the table has one.
    pub const fn decided_by(self) -> Option<Capability> {
        match self {
            Self::Command => Some(Capability::RunCargoCheck),
            Self::FileWrite => Some(Capability::WriteFile),
            Self::Network => Some(Capability::RequestNetworkService),
            Self::Deploy => None,
        }
    }

    /// Why no capability decides this effect, in a line fit for a report. Set exactly when
    /// [`Self::decided_by`] is `None`, so a caller cannot get an unexplained "no".
    pub const fn undecided_reason(self) -> Option<&'static str> {
        match self {
            Self::Command | Self::FileWrite | Self::Network => None,
            Self::Deploy => Some(
                "deployment is a deferred capability (C-REMOTE), so no row covers it and nothing \
                 authorizes it",
            ),
        }
    }
}

/// How far the default grant lets an agent go on its own.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Grant {
    /// The agent may do it; the change set is the record.
    AgentAutonomous,
    /// A human has to approve the proposal.
    NeedsReview,
    /// A human has to confirm it, and the prompt says what will change.
    NeedsConfirmation,
    /// Only a human may trigger it, with no agent path at all.
    HumanOnly,
    /// The artifact is readable by an agent and writable by nobody: generation rewrites it.
    AgentReadOnly,
    /// An agent may do it once the project is trusted and the execution scope is authorized.
    AuthorizedWhenTrusted,
}

impl Grant {
    /// The machine name used by the contract data file and the checks.
    pub const fn name(self) -> &'static str {
        match self {
            Self::AgentAutonomous => "agent-autonomous",
            Self::NeedsReview => "needs-review",
            Self::NeedsConfirmation => "needs-confirmation",
            Self::HumanOnly => "human-only",
            Self::AgentReadOnly => "agent-read-only",
            Self::AuthorizedWhenTrusted => "authorized-when-trusted",
        }
    }
}

/// Whether the project has been trusted. Untrusted projects are opened read-only: nothing runs.
///
/// The prompt surface is [`PermissionService::trust`](crate::host::permission::PermissionService::trust),
/// reached by the project-open path and the CLI's `--trust`; tests construct `Trusted` directly.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum TrustState {
    /// Never trusted, or trust withdrawn. No code from the project runs.
    #[default]
    Untrusted,
    /// Trusted by a human, who saw the path and the reason for the prompt.
    Trusted,
}

/// Why an attempt was refused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DenialReason {
    /// The project is not trusted, so nothing that could run project code is allowed.
    UntrustedProject,
    /// The grant requires a human review of the proposal.
    NeedsReview,
    /// The grant requires a human confirmation.
    NeedsConfirmation,
    /// The operation is human-only; no approval makes an agent path exist.
    HumanOnly,
    /// The artifact is generated output, which an agent may read and nobody may hand-edit.
    GeneratedCodeReadOnly,
    /// The project is trusted but this execution scope was never authorized.
    ScopeNotAuthorized,
    /// The service is not in the project's network policy, so nothing may reach it.
    ServiceNotConfigured,
}

impl DenialReason {
    /// The machine name used by the contract data file and the checks.
    pub const fn name(self) -> &'static str {
        match self {
            Self::UntrustedProject => "untrusted-project",
            Self::NeedsReview => "needs-review",
            Self::NeedsConfirmation => "needs-confirmation",
            Self::HumanOnly => "human-only",
            Self::GeneratedCodeReadOnly => "generated-code-read-only",
            Self::ScopeNotAuthorized => "scope-not-authorized",
            Self::ServiceNotConfigured => "service-not-configured",
        }
    }

    /// What a caller is expected to do next. Returned with the denial so the UI can say something
    /// more useful than "denied".
    pub const fn explanation(self) -> &'static str {
        match self {
            Self::UntrustedProject => "trust the project first; an untrusted project runs no code",
            Self::NeedsReview => "ask a human to review and approve the proposed change set",
            Self::NeedsConfirmation => "ask a human to confirm this change",
            Self::HumanOnly => "only a human may perform this; there is no agent path",
            Self::GeneratedCodeReadOnly => {
                "change the design instead; the generator owns this file and would overwrite edits"
            }
            Self::ScopeNotAuthorized => "authorize this execution scope for the project first",
            Self::ServiceNotConfigured => {
                "declare this service in the project's network policy, and authorize network requests \
                 for the project"
            }
        }
    }
}

/// The answer to an attempt.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Decision {
    /// Allowed, and the record of why is the grant that allowed it.
    Allowed { via: Grant },
    /// Refused, with a reason that says what would have to change.
    Denied {
        capability: Capability,
        reason: DenialReason,
    },
}

impl Decision {
    /// Whether the attempt may proceed. Callers ask this rather than matching on the variant, so a
    /// new denial reason cannot be forgotten at one call site.
    pub fn is_allowed(self) -> bool {
        matches!(self, Self::Allowed { .. })
    }

    /// The denial, if there is one.
    pub fn denial(self) -> Option<(Capability, DenialReason)> {
        match self {
            Self::Allowed { .. } => None,
            Self::Denied { capability, reason } => Some((capability, reason)),
        }
    }

    const fn denied(capability: Capability, reason: DenialReason) -> Self {
        Self::Denied { capability, reason }
    }
}

/// The project's permission state: trust, the approvals a human has given, and the execution
/// scopes a human has authorized.
#[derive(Clone, Debug, Default)]
pub struct PermissionGate {
    trust: TrustState,
    approvals: BTreeSet<Capability>,
    authorizations: BTreeSet<Capability>,
    /// Whether the project has a network policy at all.
    ///
    /// Not "which services": the allow-list is the project's own configuration (T02.01 owns where it
    /// lives), and duplicating it here would be a second copy of a security-relevant list. This flag
    /// is only what the decision table needs to tell "no policy" from "policy that does not name
    /// this service" — both are refusals, but the second one is a human who can fix it by name.
    has_network_policy: bool,
}

impl PermissionGate {
    /// A gate for an untrusted project with nothing approved or authorized.
    pub fn new() -> Self {
        Self::default()
    }

    /// The current trust state.
    pub fn trust(&self) -> TrustState {
        self.trust
    }

    /// Records a human's decision to trust the project.
    pub fn trust_project(&mut self) {
        self.trust = TrustState::Trusted;
    }

    /// Withdraws trust. Approvals and authorizations are dropped with it, because they were given
    /// for a project the human had seen; keeping them alive would make "withdraw trust" a lie.
    pub fn distrust_project(&mut self) {
        self.trust = TrustState::Untrusted;
        self.approvals.clear();
        self.authorizations.clear();
        // The policy belongs to a project a human has seen; keeping it after trust is withdrawn
        // would leave a reachable destination behind a project nobody vouches for.
        self.has_network_policy = false;
    }

    /// Records that the project declares a network policy, so service requests have a list to be
    /// judged against. Setting it is a project-configuration act, not a grant.
    pub fn set_network_policy(&mut self, declared: bool) {
        self.has_network_policy = declared;
    }

    /// Whether the project declares a network policy.
    pub const fn has_network_policy(&self) -> bool {
        self.has_network_policy
    }

    /// Records a human approving an agent proposal category (review or confirmation).
    pub fn approve(&mut self, capability: Capability) {
        self.approvals.insert(capability);
    }

    /// Withdraws an approval.
    pub fn revoke_approval(&mut self, capability: Capability) {
        self.approvals.remove(&capability);
    }

    /// Records a human authorizing an execution scope for this project.
    pub fn authorize(&mut self, capability: Capability) {
        self.authorizations.insert(capability);
    }

    /// Withdraws an authorization.
    pub fn revoke_authorization(&mut self, capability: Capability) {
        self.authorizations.remove(&capability);
    }

    /// Whether an approval is currently in force.
    pub fn is_approved(&self, capability: Capability) -> bool {
        self.approvals.contains(&capability)
    }

    /// Whether an execution scope is currently authorized.
    pub fn is_authorized(&self, capability: Capability) -> bool {
        self.authorizations.contains(&capability)
    }

    /// Decides an attempt. The order of the checks is the contract: trust cannot be bypassed by an
    /// approval, and "human only" cannot be unlocked by one either.
    pub fn decide(&self, principal: Principal, capability: Capability) -> Decision {
        // Trust comes first for the capabilities §12.5 makes conditional on it: a project nobody has
        // vouched for is the more basic problem, and saying "declare this service" to somebody whose
        // project is untrusted would point at the wrong next step.
        let needs_trust = matches!(
            capability,
            Capability::RunCargoCheck | Capability::RequestNetworkService
        );
        if needs_trust && self.trust == TrustState::Untrusted {
            return Decision::denied(capability, DenialReason::UntrustedProject);
        }
        if capability == Capability::RequestNetworkService && !self.has_network_policy {
            // Nothing may reach a service the project has not declared, whoever is asking: the
            // policy is what turns "the network" into a list of named destinations.
            return Decision::denied(capability, DenialReason::ServiceNotConfigured);
        }

        let grant = capability.default_grant();
        if principal == Principal::Human {
            return Decision::Allowed { via: grant };
        }

        match grant {
            Grant::AgentAutonomous => Decision::Allowed { via: grant },
            Grant::NeedsReview if self.is_approved(capability) => Decision::Allowed { via: grant },
            Grant::NeedsReview => Decision::denied(capability, DenialReason::NeedsReview),
            Grant::NeedsConfirmation if self.is_approved(capability) => {
                Decision::Allowed { via: grant }
            }
            Grant::NeedsConfirmation => {
                Decision::denied(capability, DenialReason::NeedsConfirmation)
            }
            Grant::HumanOnly => Decision::denied(capability, DenialReason::HumanOnly),
            Grant::AgentReadOnly => {
                Decision::denied(capability, DenialReason::GeneratedCodeReadOnly)
            }
            Grant::AuthorizedWhenTrusted if self.is_authorized(capability) => {
                Decision::Allowed { via: grant }
            }
            Grant::AuthorizedWhenTrusted => {
                Decision::denied(capability, DenialReason::ScopeNotAuthorized)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Capability, Decision, DenialReason, EffectClass, Grant, PermissionGate, Principal,
        TrustState,
    };

    #[test]
    fn the_default_table_matches_the_whitepaper_rows() {
        let expected = [
            (Capability::EditWidgetProperty, Grant::AgentAutonomous),
            (Capability::AddOrRemoveNode, Grant::NeedsReview),
            (Capability::EditGeneratedCode, Grant::AgentReadOnly),
            (Capability::EditHandwrittenCode, Grant::NeedsConfirmation),
            (Capability::WriteFile, Grant::HumanOnly),
            (Capability::RunCargoCheck, Grant::AuthorizedWhenTrusted),
            (Capability::ChangeDependencies, Grant::NeedsConfirmation),
            (
                Capability::RequestNetworkService,
                Grant::AuthorizedWhenTrusted,
            ),
        ];

        assert_eq!(Capability::ALL.len(), expected.len());
        for (capability, grant) in expected {
            assert_eq!(capability.default_grant(), grant, "{}", capability.name());
        }
    }

    #[test]
    fn an_agent_may_change_a_widget_property_without_asking() {
        let gate = PermissionGate::new();

        assert_eq!(
            gate.decide(Principal::Agent, Capability::EditWidgetProperty),
            Decision::Allowed {
                via: Grant::AgentAutonomous
            },
        );
    }

    #[test]
    fn an_agent_proposal_needing_review_is_refused_until_a_human_approves_it() {
        let mut gate = PermissionGate::new();

        let denial = gate.decide(Principal::Agent, Capability::AddOrRemoveNode);
        assert_eq!(
            denial.denial(),
            Some((Capability::AddOrRemoveNode, DenialReason::NeedsReview)),
        );
        assert!(!denial.is_allowed());

        gate.approve(Capability::AddOrRemoveNode);
        assert!(
            gate.decide(Principal::Agent, Capability::AddOrRemoveNode)
                .is_allowed()
        );

        gate.revoke_approval(Capability::AddOrRemoveNode);
        assert!(
            !gate
                .decide(Principal::Agent, Capability::AddOrRemoveNode)
                .is_allowed()
        );
    }

    #[test]
    fn an_approval_never_opens_a_human_only_path() {
        let mut gate = PermissionGate::new();
        gate.approve(Capability::WriteFile);

        assert_eq!(
            gate.decide(Principal::Agent, Capability::WriteFile),
            Decision::Denied {
                capability: Capability::WriteFile,
                reason: DenialReason::HumanOnly
            },
        );
        assert!(
            gate.decide(Principal::Human, Capability::WriteFile)
                .is_allowed()
        );
    }

    #[test]
    fn an_agent_may_never_write_generated_output_even_with_approval() {
        let mut gate = PermissionGate::new();
        gate.approve(Capability::EditGeneratedCode);

        assert_eq!(
            gate.decide(Principal::Agent, Capability::EditGeneratedCode)
                .denial(),
            Some((
                Capability::EditGeneratedCode,
                DenialReason::GeneratedCodeReadOnly
            )),
        );
    }

    #[test]
    fn running_cargo_needs_trust_first_and_the_scope_afterwards() {
        let mut gate = PermissionGate::new();

        assert_eq!(
            gate.decide(Principal::Agent, Capability::RunCargoCheck)
                .denial(),
            Some((Capability::RunCargoCheck, DenialReason::UntrustedProject)),
        );
        assert_eq!(
            gate.decide(Principal::Human, Capability::RunCargoCheck)
                .denial(),
            Some((Capability::RunCargoCheck, DenialReason::UntrustedProject)),
        );

        gate.trust_project();
        assert_eq!(
            gate.decide(Principal::Agent, Capability::RunCargoCheck)
                .denial(),
            Some((Capability::RunCargoCheck, DenialReason::ScopeNotAuthorized)),
        );

        gate.authorize(Capability::RunCargoCheck);
        assert!(
            gate.decide(Principal::Agent, Capability::RunCargoCheck)
                .is_allowed()
        );

        gate.revoke_authorization(Capability::RunCargoCheck);
        assert!(
            !gate
                .decide(Principal::Agent, Capability::RunCargoCheck)
                .is_allowed()
        );
    }

    #[test]
    fn withdrawing_trust_drops_the_grants_it_was_given_with() {
        let mut gate = PermissionGate::new();
        gate.trust_project();
        gate.approve(Capability::AddOrRemoveNode);
        gate.authorize(Capability::RunCargoCheck);

        gate.distrust_project();

        assert_eq!(gate.trust(), TrustState::Untrusted);
        assert!(!gate.is_approved(Capability::AddOrRemoveNode));
        assert!(!gate.is_authorized(Capability::RunCargoCheck));
        assert_eq!(
            gate.decide(Principal::Agent, Capability::RunCargoCheck)
                .denial(),
            Some((Capability::RunCargoCheck, DenialReason::UntrustedProject)),
        );
    }

    #[test]
    fn every_denial_explains_what_would_have_to_change() {
        let reasons = [
            DenialReason::UntrustedProject,
            DenialReason::NeedsReview,
            DenialReason::NeedsConfirmation,
            DenialReason::HumanOnly,
            DenialReason::GeneratedCodeReadOnly,
            DenialReason::ScopeNotAuthorized,
            DenialReason::ServiceNotConfigured,
        ];

        for reason in reasons {
            assert!(
                reason.explanation().split_whitespace().count() >= 3,
                "{} needs a sentence, not a label",
                reason.name(),
            );
        }
    }

    #[test]
    fn every_capability_and_grant_has_a_stable_machine_name() {
        let names: Vec<&str> = Capability::ALL
            .iter()
            .map(|capability| capability.name())
            .collect();
        let mut unique = names.clone();
        unique.sort_unstable();
        unique.dedup();

        assert_eq!(unique.len(), names.len());
        for capability in Capability::ALL {
            assert!(
                capability.default_grant().name().contains('-'),
                "{}",
                capability.name()
            );
        }
    }

    #[test]
    fn the_effect_classes_named_by_the_roadmap_map_to_the_decisions_that_exist() {
        assert_eq!(
            EffectClass::Command.decided_by(),
            Some(Capability::RunCargoCheck),
        );
        assert_eq!(
            EffectClass::FileWrite.decided_by(),
            Some(Capability::WriteFile),
        );
        assert_eq!(
            EffectClass::Network.decided_by(),
            Some(Capability::RequestNetworkService),
        );
        assert_eq!(EffectClass::Deploy.decided_by(), None);

        for effect in EffectClass::ALL {
            let name = effect.name();
            assert!(
                name.starts_with(|character: char| character.is_ascii_lowercase())
                    && name
                        .chars()
                        .all(|character| character.is_ascii_lowercase() || character == '-'),
                "{name} is not a machine name",
            );
        }
    }

    #[test]
    fn a_network_request_needs_the_service_declared_and_authorized() {
        let mut gate = PermissionGate::new();
        gate.trust_project();
        gate.authorize(Capability::RequestNetworkService);
        gate.approve(Capability::RequestNetworkService);

        // Trusted and authorized, but the project declares no policy: there is no destination a
        // request could be judged against, so nothing may go out.
        assert_eq!(
            gate.decide(Principal::Agent, Capability::RequestNetworkService)
                .denial(),
            Some((
                Capability::RequestNetworkService,
                DenialReason::ServiceNotConfigured
            )),
        );
        assert_eq!(
            gate.decide(Principal::Human, Capability::RequestNetworkService)
                .denial(),
            Some((
                Capability::RequestNetworkService,
                DenialReason::ServiceNotConfigured
            )),
            "a policy nobody declared is not a shortcut for a human either",
        );

        gate.set_network_policy(true);
        assert!(
            gate.decide(Principal::Agent, Capability::RequestNetworkService)
                .is_allowed()
        );
        assert!(gate.has_network_policy());

        // Withdrawing trust withdraws the policy with it, and the refusal goes back to the more
        // basic fact: the project is not trusted at all.
        gate.distrust_project();
        assert!(!gate.has_network_policy());
        assert_eq!(
            gate.decide(Principal::Human, Capability::RequestNetworkService)
                .denial(),
            Some((
                Capability::RequestNetworkService,
                DenialReason::UntrustedProject
            )),
        );
    }

    #[test]
    fn every_external_effect_class_is_decided_or_explained() {
        for effect in EffectClass::ALL {
            match (effect.decided_by(), effect.undecided_reason()) {
                (Some(capability), None) => assert!(
                    Capability::ALL.contains(&capability),
                    "{} maps to a capability that is not in the decision table",
                    effect.name(),
                ),
                (None, Some(reason)) => assert!(
                    reason.split_whitespace().count() >= 8,
                    "{} refuses without saying enough about why",
                    effect.name(),
                ),
                // Both are "an effect with no decision" or "an effect with a decision and a
                // reason against it not having one"; either way the host could not say what
                // happened, which is the state this test exists to make unreachable.
                (Some(_), Some(_)) | (None, None) => panic!(
                    "{} must be either decided by a capability or explained, not both or neither",
                    effect.name(),
                ),
            }
        }
    }

    /// Two-way drift gate between this table and `docs/plan/contracts.json`.
    ///
    /// The data file is where the whitepaper comparison happens (`node tools/contracts/check-contracts.mjs`
    /// parses §12.5 and compares it), so the code has to agree with the data file, not with the
    /// whitepaper directly. Checking the pair inside one window of the file keeps the operation and
    /// its grant bound together: a table where two rows had their grants swapped would still
    /// contain every name, and that is exactly the mistake a name-only check misses.
    #[test]
    fn the_permission_table_matches_the_contract_data_file() {
        const WINDOW: usize = 400;
        const DATA: &str = include_str!("../../docs/plan/contracts.json");

        assert_eq!(
            DATA.matches("\"operation\":").count(),
            Capability::ALL.len(),
            "the data file must have exactly one row per capability",
        );

        for capability in Capability::ALL {
            let marker = format!("\"operation\": \"{}\",", capability.name());
            let start = DATA.find(&marker).unwrap_or_else(|| {
                panic!(
                    "docs/plan/contracts.json is missing the {} row",
                    capability.name()
                )
            });
            // Taken by characters: the data file is Chinese, so a byte window can split a code point.
            let row: String = DATA[start..].chars().take(WINDOW).collect();

            let grant = format!("\"grant\": \"{}\",", capability.default_grant().name());
            assert!(
                row.contains(&grant),
                "{} does not carry its grant {}",
                capability.name(),
                grant
            );
            assert!(
                row.contains(&format!(
                    "\"deniedAs\": {}",
                    expected_denial_field(capability)
                )),
                "{} and the data file disagree about what an agent is told",
                capability.name(),
            );
        }
    }

    /// The denial an agent sees for a fresh, untrusted project with nothing approved. Only
    /// `run-cargo-check` is refused for trust before its own grant is even consulted; the data file
    /// records that first refusal, because it is the one that happens first.
    fn expected_denial_field(capability: Capability) -> String {
        let gate = PermissionGate::new();
        match gate.decide(Principal::Agent, capability) {
            Decision::Allowed { .. } => "null".to_owned(),
            Decision::Denied { reason, .. } => format!("\"{}\"", reason.name()),
        }
    }
}
