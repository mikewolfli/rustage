// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Project trust, the permission gate the host actually enforces, and the authorization record.
//! Contract **HOST**.
//!
//! Roadmap T01.05 asks for project trust, permissions for commands, files, network and deployment,
//! and an authorization record. Its acceptance line is "an untrusted project runs no code, and an
//! authorized scope can be withdrawn". [`crate::contracts::permission`] already holds the decision
//! table — a copy of whitepaper §12.5 that a check keeps in step — and this module is what makes
//! that table bite:
//!
//! * **An untrusted project runs no code, structurally.** The only way to start a program is
//!   [`super::subprocess::OwnedProcess::spawn`], which requires an [`ExecutionPermit`]. A permit
//!   has private fields and is minted by exactly one function, [`PermissionService::permit`], which
//!   asks the gate first. There is no code path that starts a program without a decision, so "we
//!   remembered to check" is not part of the argument.
//! * **A permit says which effect it is for.** `spawn` refuses a permit for any other effect, so a
//!   grant to write a file cannot be presented as a grant to run a command.
//! * **Default is deny, and the default is reachable.** A project nobody trusted behaves exactly
//!   like an untrusted one — there is no separate "unknown" answer that could be read as a pass.
//! * **The scope is granted, not the operation.** Trust opens nothing by itself; running a command
//!   in a trusted project still needs `RunCargoCheck` authorized, which is what makes "withdraw the
//!   authorization" a real state change and not a matter of remembering not to ask again.
//! * **Every decision is recorded.** [`AuthorizationRecord`] is append-only within a process and
//!   holds trust, distrust, grants, revocations and each allow/deny. Roadmap §4.1 puts "permission
//!   and authorized scope" in the Task object and roadmap §3.x requires the settings page to show
//!   scope, the source of an override and the reset; a record that cannot be read back shows none
//!   of that. Withdrawing trust also records *which* grants it withdrew, so a reset is visible
//!   rather than silent.
//! * **An effect the decision table does not cover is refused, with the reason.** The table has no
//!   row for the network or for deployment, so this module says so instead of inventing a grant
//!   ([`PermissionRefusal::Undecided`]). Naming the missing decision is the honest answer; a
//!   default-allow would be the one answer the table forbids.
//!
//! **What this module deliberately does not own.** It is not the trust *prompt's* user interface
//! (T02.03 shows it when a project is opened, T05.07 explains a denial); [`TrustPrompt`] is the
//! facts such a prompt needs, read from the directory itself. It does not persist trust between
//! runs (T02.01 owns the project's public configuration and the machine-local store). It does not
//! scope anything to a project automatically (T01.07): a caller names the [`Scope`] it is asking
//! about, and until T01.07 there is one service per host process rather than one per project.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use super::log::{Clock, Level, SystemClock, emit};
use super::paths::ProjectRoot;
use crate::contracts::permission::{
    Capability, Decision, DenialReason, EffectClass, Grant, PermissionGate, Principal, TrustState,
};

/// What a decision is about: the host's own session, or one project.
///
/// The difference is not cosmetic. §12.5's "an untrusted project runs nothing" rule is about *the
/// project*: the session's own tools — the probes, the icon renderer, the build the user asked this
/// host to run for them — are not project code fetched from somewhere else, so they are decided on
/// their own gate. An agent still has to be authorized for them.
#[derive(Clone, Copy, Debug)]
pub enum Scope<'a> {
    /// The host's own working session.
    Session,
    /// One resolved project root.
    Project(&'a ProjectRoot),
}

impl Scope<'_> {
    /// The scope's name, as the record, a log line and a permit write it.
    pub fn label(&self) -> String {
        match self {
            Self::Session => "session".to_owned(),
            Self::Project(root) => format!("project {}", root.as_path().display()),
        }
    }
}

/// The facts a trust prompt has to show before a human trusts a project directory.
///
/// Read from the directory, not guessed: a prompt that lists "build script" for every project would
/// teach a human to click through it. Each finding carries *why* it matters, because "this project
/// contains a build script" is not a reason to hesitate on its own.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TrustPrompt {
    root: PathBuf,
    manifest: Option<PathBuf>,
    build_script: Option<PathBuf>,
    cargo_config: Option<PathBuf>,
    toolchain_pin: Option<PathBuf>,
}

impl TrustPrompt {
    /// Reads what is in the directory now.
    pub fn read(root: &ProjectRoot) -> Self {
        let at = |name: &str| {
            let candidate = root.as_path().join(name);
            candidate.is_file().then_some(candidate)
        };
        Self {
            root: root.as_path().to_path_buf(),
            manifest: at("Cargo.toml"),
            build_script: at("build.rs"),
            cargo_config: at(".cargo/config.toml").or_else(|| at(".cargo/config")),
            toolchain_pin: at("rust-toolchain.toml").or_else(|| at("rust-toolchain")),
        }
    }

    /// The resolved root this prompt is about.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Whether the project has a manifest, i.e. whether cargo would find anything to build.
    pub fn has_manifest(&self) -> bool {
        self.manifest.is_some()
    }

    /// The build script, when there is one.
    pub fn build_script(&self) -> Option<&Path> {
        self.build_script.as_deref()
    }

    /// The cargo configuration, when there is one.
    pub fn cargo_config(&self) -> Option<&Path> {
        self.cargo_config.as_deref()
    }

    /// The toolchain pin, when there is one.
    pub fn toolchain_pin(&self) -> Option<&Path> {
        self.toolchain_pin.as_deref()
    }

    /// Whether anything in the directory can run a program or fetch one during a build.
    ///
    /// This is the question the prompt exists to answer, and it is answered from files rather than
    /// from a promise: a build script runs with the user's rights, a cargo configuration can name a
    /// compiler wrapper or a target runner, and a toolchain pin can make cargo download a compiler.
    pub fn runs_code_at_build_time(&self) -> bool {
        self.build_script.is_some() || self.cargo_config.is_some() || self.toolchain_pin.is_some()
    }

    /// One sentence per finding, saying what was found and what it would let the project do.
    pub fn findings(&self) -> Vec<String> {
        let mut findings = Vec::new();
        if let Some(path) = &self.build_script {
            findings.push(format!(
                "{} runs arbitrary code with your rights every time the project is built",
                short(path),
            ));
        }
        if let Some(path) = &self.cargo_config {
            findings.push(format!(
                "{} can name a compiler wrapper or a target runner, so a build runs what it names",
                short(path),
            ));
        }
        if let Some(path) = &self.toolchain_pin {
            findings.push(format!(
                "{} can make cargo download and run a different compiler",
                short(path),
            ));
        }
        if self.manifest.is_none() {
            findings.push(
                "there is no Cargo.toml here yet; a project can gain one, and this decision covers \
                 the directory"
                    .to_owned(),
            );
        }
        if findings.is_empty() {
            findings.push(
                "nothing here can run a program or fetch one during a build today; trust is still \
                 recorded because the directory can gain a build script later"
                    .to_owned(),
            );
        }
        findings
    }

    /// The prompt in one line, for a record, a log or a report.
    pub fn summary(&self) -> String {
        format!(
            "{}: {}",
            self.root.display(),
            if self.runs_code_at_build_time() {
                "the directory can run a program during a build"
            } else {
                "nothing in the directory runs a program during a build today"
            },
        )
    }
}

/// The last path component, or the whole path when it has none.
fn short(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// One project's trust decision and everything granted under it.
#[derive(Clone, Debug)]
pub struct ProjectPermission {
    prompt: TrustPrompt,
    gate: PermissionGate,
    trusted_at_millis: u128,
}

impl ProjectPermission {
    /// What the human was shown when trust was recorded.
    pub fn prompt(&self) -> &TrustPrompt {
        &self.prompt
    }

    /// The gate the project's decisions go through.
    pub fn gate(&self) -> &PermissionGate {
        &self.gate
    }

    /// Milliseconds between the service starting and trust being recorded.
    pub fn trusted_at_millis(&self) -> u128 {
        self.trusted_at_millis
    }
}

/// Why the host refuses to perform an effect.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PermissionRefusal {
    /// The decision table has a row for this effect, and the answer was no.
    Denied {
        /// The capability that decided it.
        capability: Capability,
        /// Why it was refused.
        reason: DenialReason,
        /// What would have to change.
        explanation: &'static str,
        /// Which scope was asked about.
        scope: String,
    },
    /// The decision table has no row for this effect, so there is nothing to be authorized for.
    Undecided {
        /// The effect that was asked for.
        effect: EffectClass,
        /// Why no decision exists.
        reason: &'static str,
        /// Which scope was asked about.
        scope: String,
    },
}

impl PermissionRefusal {
    /// A sentence for a log line, a problem panel or a terminal.
    pub fn message(&self) -> String {
        match self {
            Self::Denied {
                capability,
                reason,
                explanation,
                scope,
            } => format!(
                "refused: {} is not allowed in {scope} ({}) and {explanation}",
                capability.name(),
                reason.name(),
            ),
            Self::Undecided {
                effect,
                reason,
                scope,
            } => format!(
                "refused: {} in {scope} has no decision to grant it: {reason}",
                effect.name()
            ),
        }
    }

    /// The scope the refusal is about.
    pub fn scope(&self) -> &str {
        match self {
            Self::Denied { scope, .. } | Self::Undecided { scope, .. } => scope,
        }
    }

    /// The capability that decided it, when a decision existed.
    pub fn capability(&self) -> Option<Capability> {
        match self {
            Self::Denied { capability, .. } => Some(*capability),
            Self::Undecided { .. } => None,
        }
    }
}

impl fmt::Display for PermissionRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message())
    }
}

impl std::error::Error for PermissionRefusal {}

/// The host's written permission to perform one effect, once.
///
/// The fields are private and nothing outside this module can build one, so a program cannot be
/// started, a file cannot be written and a socket cannot be opened by a caller that skipped the
/// gate: there is no value of this type to pass. It carries the decision that produced it so a
/// report can show *what* allowed the work rather than that something did.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ExecutionPermit {
    effect: EffectClass,
    scope: String,
    principal: Principal,
    via: Grant,
    event: u64,
}

impl ExecutionPermit {
    /// Which effect this permit is for.
    pub fn effect(&self) -> EffectClass {
        self.effect
    }

    /// Which scope it was granted in.
    pub fn scope(&self) -> &str {
        &self.scope
    }

    /// On whose behalf the work runs.
    pub fn principal(&self) -> Principal {
        self.principal
    }

    /// The grant that allowed it.
    pub fn via(&self) -> Grant {
        self.via
    }

    /// The record entry this permit came from, so an artifact can be traced to a decision.
    pub fn event(&self) -> u64 {
        self.event
    }

    /// The permit as one line of evidence.
    pub fn evidence(&self) -> String {
        format!(
            "{} permitted in {} for {:?} via {} (record entry #{})",
            self.effect.name(),
            self.scope,
            self.principal,
            self.via.name(),
            self.event,
        )
    }
}

/// Something the record holds.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum AuthorizationEventKind {
    /// A human trusted the project, having been shown the prompt.
    Trusted {
        /// What the human was shown.
        prompt: TrustPrompt,
    },
    /// Trust was withdrawn. Everything granted under it went with it.
    Distrusted {
        /// The approvals and authorizations that were in force and are now gone.
        withdrawn: Vec<Capability>,
    },
    /// An approval or an execution authorization was recorded.
    Granted {
        /// What was granted.
        capability: Capability,
        /// The grant it unlocks.
        via: Grant,
    },
    /// A grant was withdrawn.
    Revoked {
        /// What was withdrawn.
        capability: Capability,
        /// The grant it had unlocked.
        via: Grant,
    },
    /// A decision was asked for and given.
    Decided {
        /// What the decision was about.
        capability: Capability,
        /// The answer.
        decision: Decision,
    },
}

impl AuthorizationEventKind {
    /// The name used in reports and tests.
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Trusted { .. } => "trusted",
            Self::Distrusted { .. } => "distrusted",
            Self::Granted { .. } => "granted",
            Self::Revoked { .. } => "revoked",
            Self::Decided { .. } => "decided",
        }
    }
}

/// One entry in the authorization record.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AuthorizationEvent {
    /// Sequence number, starting at one, unique within the service.
    pub id: u64,
    /// Milliseconds between the service starting and this entry.
    pub at_millis: u128,
    /// Which scope it is about.
    pub scope: String,
    /// Who asked. Trust changes are a human's act, so they carry [`Principal::Human`].
    pub principal: Principal,
    /// What happened.
    pub kind: AuthorizationEventKind,
}

impl AuthorizationEvent {
    /// The entry as one line.
    pub fn render(&self) -> String {
        let detail = match &self.kind {
            AuthorizationEventKind::Trusted { prompt } => prompt.summary(),
            AuthorizationEventKind::Distrusted { withdrawn } => {
                if withdrawn.is_empty() {
                    "nothing was granted under it".to_owned()
                } else {
                    format!(
                        "withdrew {}",
                        withdrawn
                            .iter()
                            .map(|capability| capability.name())
                            .collect::<Vec<_>>()
                            .join(", "),
                    )
                }
            }
            AuthorizationEventKind::Granted { capability, via } => {
                format!("{} via {}", capability.name(), via.name())
            }
            AuthorizationEventKind::Revoked { capability, via } => {
                format!("{} that used {}", capability.name(), via.name())
            }
            AuthorizationEventKind::Decided {
                capability,
                decision,
            } => match decision {
                Decision::Allowed { via } => {
                    format!("{} allowed via {}", capability.name(), via.name())
                }
                Decision::Denied { reason, .. } => {
                    format!("{} denied ({})", capability.name(), reason.name())
                }
            },
        };
        format!(
            "#{} +{}ms {:<9} {} {:?} {detail}",
            self.id,
            self.at_millis,
            self.kind.name(),
            self.scope,
            self.principal,
        )
    }
}

/// The authorization record: what was trusted, what was granted, and what was decided.
///
/// It is the artifact the roadmap asks for in two places — the Task/Permission object holds
/// "permission and authorized scope" (§4.1), and the trust-and-permissions settings page has to
/// show the scope, where an override came from and what a reset removed (§3.2). A record that only
/// held the current state could show none of the three.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct AuthorizationRecord {
    events: Vec<AuthorizationEvent>,
}

impl AuthorizationRecord {
    /// Every entry, oldest first.
    pub fn events(&self) -> &[AuthorizationEvent] {
        &self.events
    }

    /// How many entries there are.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Whether nothing has happened yet.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// How many decisions were refusals.
    pub fn denials(&self) -> usize {
        self.events
            .iter()
            .filter(|event| {
                matches!(
                    &event.kind,
                    AuthorizationEventKind::Decided {
                        decision,
                        ..
                    } if !decision.is_allowed()
                )
            })
            .count()
    }

    /// The entries that granted or revoked something, in order.
    pub fn grants(&self) -> Vec<&AuthorizationEvent> {
        self.events
            .iter()
            .filter(|event| {
                matches!(
                    event.kind,
                    AuthorizationEventKind::Trusted { .. }
                        | AuthorizationEventKind::Granted { .. }
                        | AuthorizationEventKind::Revoked { .. }
                )
            })
            .collect()
    }

    /// The whole record, one entry per line.
    pub fn render_text(&self) -> String {
        let mut out = String::new();
        for event in &self.events {
            out.push_str(&event.render());
            out.push('\n');
        }
        out
    }
}

/// One host process's trust and permission state, and the record of what it did.
pub struct PermissionService {
    clock: Box<dyn Clock>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    projects: BTreeMap<PathBuf, ProjectPermission>,
    session: PermissionGate,
    events: Vec<AuthorizationEvent>,
    next_entry: u64,
}

impl PermissionService {
    /// A service with no trusted project and no grants.
    ///
    /// The session scope is trusted from the start, and that is a decision rather than a loophole:
    /// the session's own tools are not a project that arrived from somewhere else, and §12.5's rule
    /// is about projects. An agent asking for a session-scoped command still needs the capability
    /// authorized, so the session gate is trusted but grants nothing.
    pub fn new() -> Self {
        Self::with_clock(Box::new(SystemClock::start()))
    }

    /// A service whose record is timed by the caller's clock, so a test can pin it.
    pub fn with_clock(clock: Box<dyn Clock>) -> Self {
        let mut session = PermissionGate::new();
        session.trust_project();
        Self {
            clock,
            state: Mutex::new(State {
                projects: BTreeMap::new(),
                session,
                events: Vec::new(),
                next_entry: 1,
            }),
        }
    }

    /// What a trust prompt would show for this directory right now, changing nothing.
    pub fn prompt(&self, root: &ProjectRoot) -> TrustPrompt {
        TrustPrompt::read(root)
    }

    /// Records that a human trusted this project, returning what was shown.
    ///
    /// The directory is read again rather than taken from an earlier [`Self::prompt`] call, so the
    /// record holds what was true when trust was recorded; a caller that wants to notice a project
    /// changing between the two reads has both values.
    pub fn trust(&self, root: &ProjectRoot) -> TrustPrompt {
        let prompt = TrustPrompt::read(root);
        let at = self.clock.elapsed_millis();
        let mut state = lock(&self.state);
        let entry = state
            .projects
            .entry(root.as_path().to_path_buf())
            .or_insert_with(|| ProjectPermission {
                prompt: prompt.clone(),
                gate: PermissionGate::new(),
                trusted_at_millis: at,
            });
        entry.prompt = prompt.clone();
        entry.gate.trust_project();
        record(
            &mut state,
            at,
            Scope::Project(root).label(),
            Principal::Human,
            AuthorizationEventKind::Trusted {
                prompt: prompt.clone(),
            },
        );
        emit(
            Level::Info,
            "host::permission",
            &format!("trusted {}", prompt.summary()),
        );
        prompt
    }

    /// Withdraws trust, along with every approval and authorization given under it.
    ///
    /// Returns what was withdrawn, and records the same list, so a reset is visible: a human who
    /// withdraws trust can see which execution scopes went with it rather than discovering later
    /// that a build no longer runs.
    pub fn distrust(&self, root: &ProjectRoot) -> Vec<Capability> {
        let at = self.clock.elapsed_millis();
        let mut state = lock(&self.state);
        let withdrawn = state
            .projects
            .remove(root.as_path())
            .map(|entry| granted_under(&entry.gate))
            .unwrap_or_default();
        record(
            &mut state,
            at,
            Scope::Project(root).label(),
            Principal::Human,
            AuthorizationEventKind::Distrusted {
                withdrawn: withdrawn.clone(),
            },
        );
        emit(
            Level::Warn,
            "host::permission",
            &format!(
                "withdrew trust in {}; {}",
                root.as_path().display(),
                if withdrawn.is_empty() {
                    "nothing was granted under it".to_owned()
                } else {
                    format!(
                        "{} grant(s) went with it: {}",
                        withdrawn.len(),
                        withdrawn
                            .iter()
                            .map(|capability| capability.name())
                            .collect::<Vec<_>>()
                            .join(", "),
                    )
                },
            ),
        );
        withdrawn
    }

    /// Whether this project is trusted right now.
    pub fn is_trusted(&self, root: &ProjectRoot) -> bool {
        lock(&self.state)
            .projects
            .get(root.as_path())
            .is_some_and(|entry| entry.gate.trust() == TrustState::Trusted)
    }

    /// The project's trust decision and grants, when it has been trusted.
    pub fn permission(&self, root: &ProjectRoot) -> Option<ProjectPermission> {
        lock(&self.state).projects.get(root.as_path()).cloned()
    }

    /// Every trusted project root, in path order.
    pub fn trusted_projects(&self) -> Vec<PathBuf> {
        lock(&self.state).projects.keys().cloned().collect()
    }

    /// Records a human approving a capability that needs review or confirmation.
    pub fn approve(
        &self,
        scope: Scope<'_>,
        capability: Capability,
    ) -> Result<(), PermissionRefusal> {
        self.grant(scope, capability, true)
    }

    /// Records a human authorizing an execution scope.
    pub fn authorize(
        &self,
        scope: Scope<'_>,
        capability: Capability,
    ) -> Result<(), PermissionRefusal> {
        self.grant(scope, capability, false)
    }

    /// Withdraws an approval.
    pub fn revoke_approval(&self, scope: Scope<'_>, capability: Capability) {
        self.revoke(scope, capability, true);
    }

    /// Withdraws an execution authorization.
    pub fn revoke_authorization(&self, scope: Scope<'_>, capability: Capability) {
        self.revoke(scope, capability, false);
    }

    /// Whether an approval is in force for this scope.
    pub fn is_approved(&self, scope: Scope<'_>, capability: Capability) -> bool {
        lock(&self.state)
            .gate_for(scope)
            .is_some_and(|gate| gate.is_approved(capability))
    }

    /// Whether an execution authorization is in force for this scope.
    pub fn is_authorized(&self, scope: Scope<'_>, capability: Capability) -> bool {
        lock(&self.state)
            .gate_for(scope)
            .is_some_and(|gate| gate.is_authorized(capability))
    }

    /// Decides an attempt, and records the answer.
    ///
    /// Asking twice records twice on purpose: the record answers "what was decided", and a caller
    /// that asked for a command three times made three decisions, not one.
    pub fn decide(
        &self,
        scope: Scope<'_>,
        principal: Principal,
        capability: Capability,
    ) -> Decision {
        let at = self.clock.elapsed_millis();
        let mut state = lock(&self.state);
        let decision = state.decide(scope, principal, capability);
        record(
            &mut state,
            at,
            scope.label(),
            principal,
            AuthorizationEventKind::Decided {
                capability,
                decision,
            },
        );
        decision
    }

    /// Turns an effect into a permit, or gives the reason there is none.
    ///
    /// This is the single mint for [`ExecutionPermit`]: whoever performs an external effect has to
    /// come through here, which is why "an untrusted project runs no code" is a property of the
    /// host rather than of a caller's discipline.
    pub fn permit(
        &self,
        effect: EffectClass,
        scope: Scope<'_>,
        principal: Principal,
    ) -> Result<ExecutionPermit, PermissionRefusal> {
        let label = scope.label();
        let Some(capability) = effect.decided_by() else {
            let reason = effect
                .undecided_reason()
                .unwrap_or("the decision table has no row for this effect");
            let refusal = PermissionRefusal::Undecided {
                effect,
                reason,
                scope: label,
            };
            emit(Level::Warn, "host::permission", &refusal.message());
            return Err(refusal);
        };

        let at = self.clock.elapsed_millis();
        let mut state = lock(&self.state);
        let decision = state.decide(scope, principal, capability);
        let entry = record(
            &mut state,
            at,
            scope.label(),
            principal,
            AuthorizationEventKind::Decided {
                capability,
                decision,
            },
        );
        match decision {
            Decision::Allowed { via } => {
                let permit = ExecutionPermit {
                    effect,
                    scope: label,
                    principal,
                    via,
                    event: entry,
                };
                emit(Level::Debug, "host::permission", &permit.evidence());
                Ok(permit)
            }
            Decision::Denied { capability, reason } => {
                let refusal = PermissionRefusal::Denied {
                    capability,
                    reason,
                    explanation: reason.explanation(),
                    scope: label,
                };
                emit(Level::Warn, "host::permission", &refusal.message());
                Err(refusal)
            }
        }
    }

    /// The record of everything this service was asked and told.
    pub fn record(&self) -> AuthorizationRecord {
        AuthorizationRecord {
            events: lock(&self.state).events.clone(),
        }
    }

    fn grant(
        &self,
        scope: Scope<'_>,
        capability: Capability,
        approval: bool,
    ) -> Result<(), PermissionRefusal> {
        let at = self.clock.elapsed_millis();
        let mut state = lock(&self.state);
        let trusted = state
            .gate_for(scope)
            .is_some_and(|gate| gate.trust() == TrustState::Trusted);
        if !trusted {
            // An untrusted project gets no grant recorded, and saying so is the point: recording one
            // would leave a state that looks authorized and is not, and the human would learn about
            // it from a build that refused to run.
            let reason = DenialReason::UntrustedProject;
            let refusal = PermissionRefusal::Denied {
                capability,
                reason,
                explanation: reason.explanation(),
                scope: scope.label(),
            };
            record(
                &mut state,
                at,
                scope.label(),
                Principal::Human,
                AuthorizationEventKind::Decided {
                    capability,
                    decision: Decision::Denied { capability, reason },
                },
            );
            emit(Level::Warn, "host::permission", &refusal.message());
            return Err(refusal);
        }

        if let Some(gate) = state.gate_for_mut(scope) {
            if approval {
                gate.approve(capability);
            } else {
                gate.authorize(capability);
            }
        }
        let via = capability.default_grant();
        record(
            &mut state,
            at,
            scope.label(),
            Principal::Human,
            AuthorizationEventKind::Granted { capability, via },
        );
        emit(
            Level::Info,
            "host::permission",
            &format!(
                "{} {} in {}",
                if approval { "approved" } else { "authorized" },
                capability.name(),
                scope.label(),
            ),
        );
        Ok(())
    }

    fn revoke(&self, scope: Scope<'_>, capability: Capability, approval: bool) {
        let at = self.clock.elapsed_millis();
        let mut state = lock(&self.state);
        if let Some(gate) = state.gate_for_mut(scope) {
            if approval {
                gate.revoke_approval(capability);
            } else {
                gate.revoke_authorization(capability);
            }
        }
        let via = capability.default_grant();
        record(
            &mut state,
            at,
            scope.label(),
            Principal::Human,
            AuthorizationEventKind::Revoked { capability, via },
        );
        emit(
            Level::Warn,
            "host::permission",
            &format!(
                "withdrew the {} for {} in {}",
                if approval {
                    "approval"
                } else {
                    "authorization"
                },
                capability.name(),
                scope.label(),
            ),
        );
    }
}

impl Default for PermissionService {
    fn default() -> Self {
        Self::new()
    }
}

impl State {
    /// The gate a decision goes through. An unknown project is an untrusted one, because that is
    /// what it is: no human has decided anything about it.
    fn gate_for(&self, scope: Scope<'_>) -> Option<&PermissionGate> {
        match scope {
            Scope::Session => Some(&self.session),
            Scope::Project(root) => self.projects.get(root.as_path()).map(|entry| &entry.gate),
        }
    }

    fn gate_for_mut(&mut self, scope: Scope<'_>) -> Option<&mut PermissionGate> {
        match scope {
            Scope::Session => Some(&mut self.session),
            // The session is always present, the project lookup is not: a grant for a project that
            // was never trusted has no gate to land in. The caller refuses *before* reaching here
            // in that case, so this stays a lookup and not a second policy.
            Scope::Project(root) => self
                .projects
                .get_mut(root.as_path())
                .map(|entry| &mut entry.gate),
        }
    }

    fn decide(&self, scope: Scope<'_>, principal: Principal, capability: Capability) -> Decision {
        match self.gate_for(scope) {
            Some(gate) => gate.decide(principal, capability),
            None => PermissionGate::new().decide(principal, capability),
        }
    }
}

/// What this gate has in force, for the record's "distrust withdrew these" line.
fn granted_under(gate: &PermissionGate) -> Vec<Capability> {
    Capability::ALL
        .into_iter()
        .filter(|capability| gate.is_approved(*capability) || gate.is_authorized(*capability))
        .collect()
}

/// Appends one entry and returns its sequence number.
fn record(
    state: &mut State,
    at_millis: u128,
    scope: String,
    principal: Principal,
    kind: AuthorizationEventKind,
) -> u64 {
    let id = state.next_entry;
    state.next_entry += 1;
    state.events.push(AuthorizationEvent {
        id,
        at_millis,
        scope,
        principal,
        kind,
    });
    id
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .expect("the permission service is never poisoned: no panic escapes its critical sections")
}

#[cfg(test)]
mod tests {
    use super::{
        AuthorizationEventKind, EffectClass, PermissionRefusal, PermissionService, Scope,
        TrustPrompt,
    };
    use crate::contracts::permission::{Capability, DenialReason, Principal};
    use crate::host::log::FixedClock;
    use crate::host::paths::ProjectRoot;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    /// A project directory of its own per test, removed by the guard when the test ends.
    ///
    /// The counter matters for the same reason it does in the paths tests: tests run in parallel and
    /// a shared directory would let one test's cleanup delete another's project mid-run.
    struct Sandbox {
        path: PathBuf,
        _guard: TempDirectory,
    }

    struct TempDirectory(PathBuf);

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn sandbox(name: &str) -> Sandbox {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "rustage-permission-{name}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed),
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a temporary project directory is creatable");
        Sandbox {
            path: path.clone(),
            _guard: TempDirectory(path),
        }
    }

    impl Sandbox {
        fn write(&self, relative: &str, contents: &str) {
            let path = self.path.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("the parent directory is creatable");
            }
            std::fs::write(&path, contents).expect("the file is writable");
        }

        fn root(&self) -> ProjectRoot {
            ProjectRoot::new(&self.path).expect("the sandbox is a directory")
        }
    }

    /// A project that is trusted and has `RunCargoCheck` authorized, which is what running a
    /// command in it needs.
    fn trusted_project(service: &PermissionService, root: &ProjectRoot) {
        service.trust(root);
        service
            .authorize(Scope::Project(root), Capability::RunCargoCheck)
            .expect("a trusted project can have its execution scope authorized");
    }

    #[test]
    fn an_untrusted_project_cannot_get_a_command_permit() {
        let sandbox = sandbox("untrusted");
        let root = sandbox.root();
        let service = PermissionService::new();

        let refusal = service
            .permit(
                EffectClass::Command,
                Scope::Project(&root),
                Principal::Human,
            )
            .expect_err("an untrusted project runs no code");

        match refusal {
            PermissionRefusal::Denied {
                capability,
                reason,
                explanation,
                scope,
            } => {
                assert_eq!(capability, Capability::RunCargoCheck);
                assert_eq!(reason, DenialReason::UntrustedProject);
                assert!(explanation.contains("trust the project first"));
                assert!(scope.starts_with("project "));
            }
            other => panic!("expected a denial with a reason, got {other:?}"),
        }
        assert!(!service.is_trusted(&root));
    }

    #[test]
    fn a_project_nobody_trusted_behaves_exactly_like_an_untrusted_one() {
        let sandbox = sandbox("unknown");
        let root = sandbox.root();
        let service = PermissionService::new();

        assert!(service.permission(&root).is_none());
        assert_eq!(
            service
                .permit(
                    EffectClass::Command,
                    Scope::Project(&root),
                    Principal::Human
                )
                .expect_err("nothing was decided about this project")
                .capability(),
            Some(Capability::RunCargoCheck),
        );
        assert_eq!(
            service.decide(
                Scope::Project(&root),
                Principal::Human,
                Capability::RunCargoCheck,
            ),
            crate::contracts::permission::Decision::Denied {
                capability: Capability::RunCargoCheck,
                reason: DenialReason::UntrustedProject,
            },
        );
    }

    #[test]
    fn trust_is_the_half_a_project_only_gets_from_a_human() {
        let sandbox = sandbox("scope");
        let root = sandbox.root();
        let service = PermissionService::new();
        service.trust(&root);

        assert!(service.is_trusted(&root));
        // §12.5 decides *agent* autonomy: a human acting on a project they trusted may run a
        // command, which is why the untrusted rule is the part that bites hardest.
        assert!(
            service
                .permit(
                    EffectClass::Command,
                    Scope::Project(&root),
                    Principal::Human
                )
                .is_ok(),
            "a human may run a command in a project they trusted",
        );
        assert!(
            service
                .permit(
                    EffectClass::Command,
                    Scope::Project(&root),
                    Principal::Agent
                )
                .is_err(),
            "an agent has no authorized execution scope yet",
        );
    }

    #[test]
    fn an_agent_gains_the_execution_scope_by_authorization_and_loses_it_by_revocation() {
        let sandbox = sandbox("agent-scope");
        let root = sandbox.root();
        let service = PermissionService::new();
        service.trust(&root);

        assert_eq!(
            service
                .permit(
                    EffectClass::Command,
                    Scope::Project(&root),
                    Principal::Agent
                )
                .expect_err("trust is not authorization for an agent")
                .capability(),
            Some(Capability::RunCargoCheck),
        );

        service
            .authorize(Scope::Project(&root), Capability::RunCargoCheck)
            .expect("the scope can be authorized once the project is trusted");
        let permit = service
            .permit(
                EffectClass::Command,
                Scope::Project(&root),
                Principal::Agent,
            )
            .expect("a trusted and authorized project may run the agent's command");
        assert_eq!(permit.effect(), EffectClass::Command);
        assert_eq!(permit.via().name(), "authorized-when-trusted");

        service.revoke_authorization(Scope::Project(&root), Capability::RunCargoCheck);

        assert!(!service.is_authorized(Scope::Project(&root), Capability::RunCargoCheck));
        assert!(service.is_trusted(&root), "the project is still trusted");
        assert_eq!(
            service
                .permit(
                    EffectClass::Command,
                    Scope::Project(&root),
                    Principal::Agent
                )
                .expect_err("the authorization is gone")
                .capability(),
            Some(Capability::RunCargoCheck),
        );
    }

    #[test]
    fn withdrawing_trust_takes_the_authorization_with_it_and_says_which() {
        let sandbox = sandbox("distrust");
        let root = sandbox.root();
        let service = PermissionService::new();
        trusted_project(&service, &root);

        let withdrawn = service.distrust(&root);

        assert_eq!(withdrawn, vec![Capability::RunCargoCheck]);
        assert!(service.permission(&root).is_none());
        assert_eq!(
            service
                .permit(
                    EffectClass::Command,
                    Scope::Project(&root),
                    Principal::Human
                )
                .expect_err("trust is gone")
                .capability(),
            Some(Capability::RunCargoCheck),
        );
    }

    #[test]
    fn authorizing_an_untrusted_project_is_refused_rather_than_remembered() {
        let sandbox = sandbox("grant-first");
        let root = sandbox.root();
        let service = PermissionService::new();

        let refusal = service
            .authorize(Scope::Project(&root), Capability::RunCargoCheck)
            .expect_err("an untrusted project gets no grant");
        assert_eq!(refusal.capability(), Some(Capability::RunCargoCheck));

        // The important half: trusting afterwards must not reveal a grant that was never recorded,
        // because that would make "authorized" mean two different things.
        service.trust(&root);
        assert!(
            !service.is_authorized(Scope::Project(&root), Capability::RunCargoCheck),
            "a refused authorization must not appear later",
        );
    }

    #[test]
    fn an_agent_may_never_get_a_file_write_permit() {
        let service = PermissionService::new();

        let refusal = service
            .permit(EffectClass::FileWrite, Scope::Session, Principal::Agent)
            .expect_err("writing a file is human-only");

        assert_eq!(
            match refusal {
                PermissionRefusal::Denied { reason, .. } => reason,
                other => panic!("expected a denial, got {other:?}"),
            },
            DenialReason::HumanOnly,
        );
        assert!(
            service
                .permit(EffectClass::FileWrite, Scope::Session, Principal::Human)
                .is_ok(),
            "a human may write a file",
        );
    }

    #[test]
    fn an_agent_needs_the_session_scope_authorized_too() {
        let service = PermissionService::new();

        assert_eq!(
            service
                .permit(EffectClass::Command, Scope::Session, Principal::Agent)
                .expect_err("an agent needs an authorized scope for any command")
                .capability(),
            Some(Capability::RunCargoCheck),
        );

        service
            .authorize(Scope::Session, Capability::RunCargoCheck)
            .expect("the session is trusted");
        assert!(
            service
                .permit(EffectClass::Command, Scope::Session, Principal::Agent)
                .is_ok()
        );
    }

    #[test]
    fn network_and_deployment_have_no_decision_to_give() {
        let service = PermissionService::new();

        for effect in [EffectClass::Network, EffectClass::Deploy] {
            let refusal = service
                .permit(effect, Scope::Session, Principal::Human)
                .expect_err("no §12.5 row decides this effect");
            match refusal {
                PermissionRefusal::Undecided {
                    effect: refused,
                    reason,
                    scope,
                } => {
                    assert_eq!(refused, effect);
                    assert_eq!(scope, "session");
                    assert!(
                        reason.contains("row"),
                        "the reason must name the missing decision: {reason}",
                    );
                }
                other => panic!("expected an undecided refusal, got {other:?}"),
            }
        }
    }

    #[test]
    fn the_session_scope_is_the_hosts_own_work_not_project_code() {
        let service = PermissionService::new();

        let permit = service
            .permit(EffectClass::Command, Scope::Session, Principal::Human)
            .expect("the human who runs the host may run a command of their own");
        assert_eq!(permit.scope(), "session");
        assert_eq!(permit.principal(), Principal::Human);
        assert_eq!(permit.via().name(), "authorized-when-trusted");
    }

    #[test]
    fn a_permit_names_the_record_entry_that_allowed_it() {
        let service = PermissionService::new();

        let permit = service
            .permit(EffectClass::Command, Scope::Session, Principal::Human)
            .expect("the session may run a command");

        let record = service.record();
        let last = record.events().last().expect("the decision was recorded");
        assert_eq!(permit.event(), last.id);
        assert_eq!(last.kind.name(), "decided");
        assert!(permit.evidence().contains("command permitted in session"));
        assert!(permit.evidence().contains(&format!("#{}", permit.event())));
    }

    #[test]
    fn the_record_holds_every_grant_and_every_decision_in_order() {
        let sandbox = sandbox("record");
        let root = sandbox.root();
        let service = PermissionService::new();

        service.trust(&root);
        service
            .authorize(Scope::Project(&root), Capability::RunCargoCheck)
            .expect("trusted");
        assert!(
            service
                .permit(
                    EffectClass::Command,
                    Scope::Project(&root),
                    Principal::Agent
                )
                .is_ok()
        );
        service.revoke_authorization(Scope::Project(&root), Capability::RunCargoCheck);
        assert!(
            service
                .permit(
                    EffectClass::Command,
                    Scope::Project(&root),
                    Principal::Agent
                )
                .is_err()
        );

        let record = service.record();
        let kinds: Vec<&str> = record
            .events()
            .iter()
            .map(|event| event.kind.name())
            .collect();
        assert_eq!(
            kinds,
            vec!["trusted", "granted", "decided", "revoked", "decided"],
        );
        assert_eq!(record.denials(), 1);
        assert_eq!(record.grants().len(), 3);
        let decided: Vec<bool> = record
            .events()
            .iter()
            .filter_map(|event| match &event.kind {
                AuthorizationEventKind::Decided { decision, .. } => Some(decision.is_allowed()),
                _ => None,
            })
            .collect();
        assert_eq!(decided, vec![true, false]);
        assert!(record.render_text().contains("trusted"));
    }

    #[test]
    fn the_record_is_ordered_and_timed_by_the_service_clock() {
        let clock = std::sync::Arc::new(FixedClock::new());
        struct SharedClock(std::sync::Arc<FixedClock>);
        impl crate::host::log::Clock for SharedClock {
            fn elapsed_millis(&self) -> u128 {
                self.0.elapsed_millis()
            }
        }
        let service = PermissionService::with_clock(Box::new(SharedClock(clock.clone())));

        service
            .authorize(Scope::Session, Capability::RunCargoCheck)
            .expect("the session is trusted");
        clock.advance(Duration::from_millis(200));
        assert!(
            service
                .permit(EffectClass::Command, Scope::Session, Principal::Human)
                .is_ok()
        );

        let record = service.record();
        let times: Vec<u128> = record
            .events()
            .iter()
            .map(|event| event.at_millis)
            .collect();
        assert_eq!(times, vec![0, 200]);
        let ids: Vec<u64> = record.events().iter().map(|event| event.id).collect();
        assert_eq!(ids, vec![1, 2]);
    }

    #[test]
    fn the_trust_prompt_reads_the_files_that_can_run_code() {
        let sandbox = sandbox("prompt");
        sandbox.write("Cargo.toml", "[package]\nname = \"demo\"\n");
        sandbox.write("build.rs", "fn main() {}\n");
        sandbox.write(".cargo/config.toml", "[build]\n");
        sandbox.write("rust-toolchain.toml", "[toolchain]\n");
        let root = sandbox.root();

        let prompt = TrustPrompt::read(&root);

        assert!(prompt.has_manifest());
        assert!(prompt.runs_code_at_build_time());
        let findings = prompt.findings().join(" | ");
        assert!(findings.contains("build.rs"), "{findings}");
        assert!(findings.contains("config.toml"), "{findings}");
        assert!(findings.contains("rust-toolchain.toml"), "{findings}");
        assert_eq!(prompt.root(), root.as_path());
        assert!(
            prompt
                .summary()
                .contains("can run a program during a build")
        );
    }

    #[test]
    fn a_directory_with_nothing_in_it_still_demands_trust() {
        let sandbox = sandbox("bare");
        let root = sandbox.root();
        let service = PermissionService::new();

        let prompt = TrustPrompt::read(&root);
        assert!(!prompt.has_manifest());
        assert!(!prompt.runs_code_at_build_time());
        let findings = prompt.findings();
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert!(
            findings[0].contains("no Cargo.toml"),
            "a bare directory must not read as \"nothing to decide\": {findings:?}",
        );

        assert!(
            service
                .permit(
                    EffectClass::Command,
                    Scope::Project(&root),
                    Principal::Human
                )
                .is_err()
        );
    }

    #[test]
    fn trusting_records_the_prompt_the_human_was_shown() {
        let sandbox = sandbox("shown");
        sandbox.write("build.rs", "fn main() {}\n");
        let root = sandbox.root();
        let service = PermissionService::new();

        let shown = service.prompt(&root);
        let recorded = service.trust(&root);

        assert_eq!(shown, recorded);
        let entry = service.permission(&root).expect("the project is trusted");
        assert_eq!(entry.prompt().build_script(), recorded.build_script());
        assert!(entry.gate().trust() == crate::contracts::permission::TrustState::Trusted);
        assert_eq!(
            service.trusted_projects(),
            vec![root.as_path().to_path_buf()]
        );
    }

    #[test]
    fn every_refusal_explains_itself_in_a_sentence() {
        let service = PermissionService::new();
        let refusals = [
            service
                .permit(EffectClass::Command, Scope::Session, Principal::Agent)
                .expect_err("an agent has no authorized scope"),
            service
                .permit(EffectClass::FileWrite, Scope::Session, Principal::Agent)
                .expect_err("writing is human-only"),
            service
                .permit(EffectClass::Network, Scope::Session, Principal::Human)
                .expect_err("the network has no decision"),
        ];

        for refusal in refusals {
            let message = refusal.message();
            assert!(
                message.split_whitespace().count() >= 10,
                "a refusal is not a label: {message}",
            );
            assert!(!refusal.scope().is_empty());
        }
    }
}
