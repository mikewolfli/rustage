// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! File and network adapters, the sample doubles that let design happen offline, cancellation, and the
//! rule that an unknown result is never retried (task T13.07).
//!
//! The acceptance line is *"an unknown result is not retried automatically, and designing offline
//! does not depend on the service"*. Both are about not inventing an answer:
//!
//! * **An unknown result is its own outcome.** [`Outcome::Unknown`] is not a failure: the request left
//!   and the result is not known, so [`attempt`] stops on it — retrying could repeat a side effect.
//!   A *known* failure is the only outcome a policy may retry.
//! * **A sample double is local.** [`Adapter::Sample`] answers from a queue, so [`Availability::Offline`]
//!   can still design against it; only a request that needs the network is refused offline, and an
//!   exhausted double says so rather than answering "fine".
//! * **A side effect needs its capability.** The request kind names the [`Capability`] it needs and the
//!   project's [`PermissionGate`] decides, so a write or a network call is not performed without the
//!   grant the host would require (principle 12/21).
//! * **Cancellation settles a request.** A cancelled request answers [`Outcome::Cancelled`] before the
//!   adapter is consulted, so a cancelled request has one honest answer.

#![allow(dead_code)]

use std::collections::{BTreeMap, VecDeque};

use crate::contracts::permission::{Capability, Decision, PermissionGate, Principal};

/// What kind of external request this is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// Reading a file. Local, and needing no capability.
    FileRead,
    /// Writing a file.
    FileWrite,
    /// A request to a service.
    Network,
}

impl Kind {
    /// The kind's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::FileRead => "file-read",
            Self::FileWrite => "file-write",
            Self::Network => "network",
        }
    }

    /// Reads a kind's name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "file-read" => Some(Self::FileRead),
            "file-write" => Some(Self::FileWrite),
            "network" => Some(Self::Network),
            _ => None,
        }
    }

    /// The capability the request needs, if any.
    pub fn capability(&self) -> Option<Capability> {
        match self {
            Self::FileRead => None,
            Self::FileWrite => Some(Capability::WriteFile),
            Self::Network => Some(Capability::RequestNetworkService),
        }
    }

    /// Whether the request leaves the machine.
    pub fn needs_the_network(&self) -> bool {
        matches!(self, Self::Network)
    }
}

/// An external request.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Request {
    /// What kind of request it is.
    pub kind: Kind,
    /// What it addresses.
    pub target: String,
}

impl Request {
    /// A request of a kind to a target.
    pub fn new(kind: Kind, target: impl Into<String>) -> Self {
        Self {
            kind,
            target: target.into(),
        }
    }

    /// The request in one line.
    pub fn render(&self) -> String {
        format!("{} {}", self.kind.name(), self.target)
    }
}

/// What came back.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Outcome {
    /// A known, good result.
    Ok(String),
    /// A known failure.
    Failed(String),
    /// The request was cancelled before it was performed.
    Cancelled,
    /// The request was performed and the result is not known.
    Unknown(String),
}

impl Outcome {
    /// The outcome's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Ok(_) => "ok",
            Self::Failed(_) => "failed",
            Self::Cancelled => "cancelled",
            Self::Unknown(_) => "unknown",
        }
    }

    /// Whether the outcome is settled, so no further attempt makes sense.
    pub fn settled(&self) -> bool {
        !matches!(self, Self::Failed(_))
    }

    /// Whether this is the outcome that must never be retried.
    pub fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown(_))
    }

    /// The outcome in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Ok(value) => format!("ok: {value}"),
            Self::Failed(reason) => format!("failed: {reason}"),
            Self::Cancelled => "cancelled".to_owned(),
            Self::Unknown(reason) => format!("unknown: {reason}"),
        }
    }
}

/// Whether the design session has a service at all.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Availability {
    /// The service may be reached.
    Online,
    /// Only local work; the network is out of reach.
    Offline,
}

/// A cancellation flag.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Cancel {
    /// Whether the request has been cancelled.
    pub cancelled: bool,
}

impl Cancel {
    /// A flag that is not set.
    pub fn new() -> Self {
        Self::default()
    }

    /// The flag, set.
    pub fn set() -> Self {
        Self { cancelled: true }
    }
}

/// Something that can perform a request.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Adapter {
    /// A local double answering from a queue, so design needs no service.
    Sample {
        /// The answers, in order.
        responses: VecDeque<Outcome>,
    },
    /// A service-backed adapter answering by target.
    Live {
        /// The answers, by target.
        answers: BTreeMap<String, Outcome>,
    },
}

impl Adapter {
    /// A sample double with these answers, in order.
    pub fn sample(responses: &[Outcome]) -> Self {
        Self::Sample {
            responses: responses.iter().cloned().collect(),
        }
    }

    /// A service-backed adapter with these answers, by target.
    pub fn live(answers: &[(&str, Outcome)]) -> Self {
        Self::Live {
            answers: answers
                .iter()
                .map(|(target, outcome)| ((*target).to_owned(), outcome.clone()))
                .collect(),
        }
    }

    /// A service-backed adapter with these owned answers, by target.
    pub fn live_owned(answers: &[(String, Outcome)]) -> Self {
        Self::Live {
            answers: answers.iter().cloned().collect(),
        }
    }

    /// Whether this adapter is a local double.
    pub fn is_sample(&self) -> bool {
        matches!(self, Self::Sample { .. })
    }

    /// Performs the request, or answers `Cancelled` when the flag is set.
    pub fn perform(&mut self, request: &Request, cancel: &Cancel) -> Outcome {
        if cancel.cancelled {
            return Outcome::Cancelled;
        }
        match self {
            Self::Sample { responses } => match responses.pop_front() {
                Some(outcome) => outcome,
                None => Outcome::Failed(format!(
                    "the sample double has no answer left for {}",
                    request.render()
                )),
            },
            Self::Live { answers } => match answers.get(&request.target) {
                Some(outcome) => outcome.clone(),
                None => Outcome::Failed(format!(
                    "the service did not answer for {}",
                    request.render()
                )),
            },
        }
    }
}

/// How many times a request may be attempted.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Policy {
    /// The number of attempts allowed; at least one.
    pub attempts: u32,
}

impl Policy {
    /// A policy allowing this many attempts.
    pub fn new(attempts: u32) -> Self {
        Self {
            attempts: attempts.max(1),
        }
    }
}

/// Runs a request once, refusing a side effect the project does not allow.
pub fn run(
    adapter: &mut Adapter,
    request: &Request,
    cancel: &Cancel,
    gate: &PermissionGate,
    principal: Principal,
    availability: Availability,
) -> Result<Outcome, String> {
    if request.kind.needs_the_network() && availability == Availability::Offline {
        return Err(format!(
            "{} needs the network, and this session is offline",
            request.render()
        ));
    }
    if let Some(capability) = request.kind.capability()
        && let Decision::Denied { reason, .. } = gate.decide(principal, capability)
    {
        return Err(format!(
            "the {capability:?} capability is denied: {reason:?}",
        ));
    }
    Ok(adapter.perform(request, cancel))
}

/// Attempts a request under a policy, answering every outcome it saw.
///
/// The rule is the acceptance line: an [`Outcome::Unknown`] stops the attempts, because the request
/// may already have had its effect and repeating it would repeat that effect. Only a known failure is
/// retried, and only while the policy allows.
pub fn attempt(
    adapter: &mut Adapter,
    request: &Request,
    cancel: &Cancel,
    policy: Policy,
) -> Vec<Outcome> {
    let mut outcomes = Vec::new();
    for _ in 0..policy.attempts {
        let outcome = adapter.perform(request, cancel);
        let stop = outcome.settled();
        outcomes.push(outcome);
        if stop {
            break;
        }
    }
    outcomes
}

#[cfg(test)]
mod tests {
    use super::{Adapter, Availability, Cancel, Kind, Outcome, Policy, Request, attempt, run};
    use crate::contracts::permission::{Capability, PermissionGate, Principal};

    fn request(kind: Kind) -> Request {
        Request::new(kind, "user.png")
    }

    #[test]
    fn a_sample_answers_from_its_queue_in_order() {
        let mut adapter = Adapter::sample(&[
            Outcome::Ok("first".to_owned()),
            Outcome::Ok("second".to_owned()),
        ]);
        let cancel = Cancel::new();
        assert_eq!(
            adapter.perform(&request(Kind::FileRead), &cancel),
            Outcome::Ok("first".to_owned())
        );
        assert_eq!(
            adapter.perform(&request(Kind::FileRead), &cancel),
            Outcome::Ok("second".to_owned())
        );
    }

    #[test]
    fn an_exhausted_sample_says_so_instead_of_succeeding() {
        let mut adapter = Adapter::sample(&[]);
        let outcome = adapter.perform(&request(Kind::FileRead), &Cancel::new());
        assert_eq!(outcome.name(), "failed");
        assert!(outcome.render().contains("no answer left"));
    }

    #[test]
    fn cancelling_settles_the_request_before_the_adapter_is_consulted() {
        let mut adapter = Adapter::sample(&[Outcome::Ok("never used".to_owned())]);
        let outcome = adapter.perform(&request(Kind::FileRead), &Cancel::set());
        assert_eq!(outcome, Outcome::Cancelled);
        assert!(outcome.settled());
    }

    #[test]
    fn an_offline_network_request_is_refused() {
        let mut adapter = Adapter::live(&[]);
        let outcome = run(
            &mut adapter,
            &request(Kind::Network),
            &Cancel::new(),
            &PermissionGate::new(),
            Principal::Human,
            Availability::Offline,
        );
        assert!(outcome.unwrap_err().contains("offline"));
    }

    #[test]
    fn an_offline_sample_is_still_allowed_so_design_does_not_need_the_service() {
        let mut adapter = Adapter::sample(&[Outcome::Ok("local".to_owned())]);
        let outcome = run(
            &mut adapter,
            &request(Kind::FileRead),
            &Cancel::new(),
            &PermissionGate::new(),
            Principal::Human,
            Availability::Offline,
        )
        .expect("an outcome");
        assert_eq!(outcome, Outcome::Ok("local".to_owned()));
        assert!(adapter.is_sample());
    }

    #[test]
    fn a_denied_capability_is_refused() {
        let mut adapter = Adapter::live(&[("user.png", Outcome::Ok("bytes".to_owned()))]);
        let outcome = run(
            &mut adapter,
            &request(Kind::Network),
            &Cancel::new(),
            &PermissionGate::new(),
            Principal::Human,
            Availability::Online,
        );
        assert!(outcome.unwrap_err().contains("denied"));
    }

    #[test]
    fn an_allowed_file_write_is_performed() {
        let mut adapter = Adapter::live(&[("user.png", Outcome::Ok("written".to_owned()))]);
        let outcome = run(
            &mut adapter,
            &request(Kind::FileWrite),
            &Cancel::new(),
            &PermissionGate::new(),
            Principal::Human,
            Availability::Online,
        )
        .expect("an outcome");
        assert_eq!(outcome, Outcome::Ok("written".to_owned()));
    }

    #[test]
    fn a_file_read_needs_no_capability() {
        assert_eq!(Kind::FileRead.capability(), None);
        assert_eq!(Kind::FileWrite.capability(), Some(Capability::WriteFile));
        assert!(!Kind::FileRead.needs_the_network());
        assert!(Kind::Network.needs_the_network());
    }

    #[test]
    fn an_unknown_outcome_is_never_retried() {
        let mut adapter = Adapter::sample(&[
            Outcome::Unknown("the write timed out".to_owned()),
            Outcome::Ok("second".to_owned()),
        ]);
        let outcomes = attempt(
            &mut adapter,
            &request(Kind::FileWrite),
            &Cancel::new(),
            Policy::new(5),
        );
        assert_eq!(outcomes.len(), 1);
        assert!(outcomes[0].is_unknown());
    }

    #[test]
    fn a_known_failure_is_retried_while_the_policy_allows() {
        let mut adapter = Adapter::sample(&[
            Outcome::Failed("busy".to_owned()),
            Outcome::Failed("busy".to_owned()),
            Outcome::Ok("third time".to_owned()),
        ]);
        let outcomes = attempt(
            &mut adapter,
            &request(Kind::FileWrite),
            &Cancel::new(),
            Policy::new(5),
        );
        assert_eq!(outcomes.len(), 3);
        assert_eq!(outcomes[2], Outcome::Ok("third time".to_owned()));
    }

    #[test]
    fn a_known_failure_is_not_retried_beyond_the_policy() {
        let mut adapter = Adapter::sample(&[
            Outcome::Failed("busy".to_owned()),
            Outcome::Failed("busy".to_owned()),
            Outcome::Ok("never reached".to_owned()),
        ]);
        let outcomes = attempt(
            &mut adapter,
            &request(Kind::FileWrite),
            &Cancel::new(),
            Policy::new(2),
        );
        assert_eq!(outcomes.len(), 2);
        assert!(outcomes.iter().all(|outcome| outcome.name() == "failed"));
    }

    #[test]
    fn a_good_outcome_is_not_retried() {
        let mut adapter = Adapter::sample(&[Outcome::Ok("done".to_owned())]);
        let outcomes = attempt(
            &mut adapter,
            &request(Kind::FileRead),
            &Cancel::new(),
            Policy::new(3),
        );
        assert_eq!(outcomes.len(), 1);
    }

    #[test]
    fn a_cancelled_outcome_is_not_retried() {
        let mut adapter = Adapter::sample(&[Outcome::Ok("never".to_owned())]);
        let outcomes = attempt(
            &mut adapter,
            &request(Kind::FileRead),
            &Cancel::set(),
            Policy::new(3),
        );
        assert_eq!(outcomes, vec![Outcome::Cancelled]);
    }

    #[test]
    fn a_policy_allows_at_least_one_attempt() {
        assert_eq!(Policy::new(0).attempts, 1);
    }

    #[test]
    fn the_outcomes_name_what_they_are() {
        assert_eq!(Outcome::Ok("x".to_owned()).name(), "ok");
        assert_eq!(Outcome::Failed("x".to_owned()).render(), "failed: x");
        assert_eq!(Outcome::Cancelled.render(), "cancelled");
        assert_eq!(Outcome::Unknown("x".to_owned()).render(), "unknown: x");
        assert!(!Outcome::Failed("x".to_owned()).settled());
    }

    #[test]
    fn the_request_and_kinds_render() {
        assert_eq!(request(Kind::FileWrite).render(), "file-write user.png");
        assert_eq!(Kind::parse("network"), Some(Kind::Network));
        assert_eq!(Kind::parse("blob"), None);
    }
}
