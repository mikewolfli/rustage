// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Credential references, proxy and certificate configuration, and the gate a network request goes
//! through. Contract **HOST**.
//!
//! Roadmap T01.09 asks for system credential-store *references*, proxy and certificate
//! configuration, network service authorization, and redaction. Its acceptance line is **"an expired
//! credential is diagnosable, TLS verification is never disabled, and a token is not stored in the
//! project or in the logs"**. Each half is a property of a type rather than a rule somebody has to
//! remember:
//!
//! * **A secret cannot be printed.** [`Secret`] has no `Display` and its `Debug` writes
//!   `Secret([redacted])`, so `{}` does not compile on it and `{:?}` cannot leak it. Getting the
//!   value out requires [`Secret::expose`], which is the call a reviewer can search for.
//! * **Nothing that reaches a log or a notification is assumed clean.** [`redact`] is applied to
//!   text that may contain a credential before it is logged or shown, and
//!   [`NetworkPermit::render`] does the same for a resolved URL. Roadmap §5 F23 asks for
//!   "everything sent out authorized item by item"; this is the other end of it, "everything shown
//!   was filtered".
//! * **A reference is not a secret.** [`CredentialRef`] names *where* material lives (a system
//!   keyring entry, an environment variable) and when it is expected to expire. Both are project
//!   configuration: a keyring entry id is not a token, and an expiry date is how "the credential is
//!   stale" can be answered before anything is sent anywhere.
//! * **An unhealthy credential is diagnosed, not discovered at the far end.** [`CredentialRef::health`]
//!   answers `Usable`, `ExpiringSoon`, `Expired`, `Missing` or `Unavailable`, and
//!   [`CredentialHealth::next_step`] says what a person can do about each. A request is refused
//!   before it is made, with that reason.
//! * **TLS verification has no switch.** [`TlsPolicy`] has no field and no constructor that turns
//!   verification off: a custom root *adds* a trust anchor, and a client certificate is a reference
//!   like any other, so the worst a caller can configure is *more* verification, never less.
//!   [`TlsPolicy::resolve`] fails when a configured anchor or certificate is unusable rather than
//!   silently continuing without it.
//! * **A network request needs a permit, and the permit is for one declared service.** The only
//!   thing that can open a connection is [`connect`], which takes a [`NetworkPermit`]; the only
//!   thing that mints one is [`NetworkGateway::permit`], which asks the project's permission gate
//!   ([`PermissionService`]) *and* requires the service to be declared in the project's
//!   [`NetworkPolicy`]. So "the host does not reach the network on its own" is the same kind of
//!   structural claim as "an untrusted project runs no code".
//!
//! **What this module deliberately does not own.** It has no TLS implementation: this build has no
//! TLS client at all, so [`TlsPolicy`] is configuration that a future transport (T16.x, T20.x, T23.x)
//! will consume, and what is verified here is that the configuration cannot be weakened and cannot
//! be resolved from material that is missing. It does not decide where a project stores its policy
//! (T02.01 owns the project's public configuration) and it does not implement an HTTP client, a
//! retry policy or a proxy dialer.

use std::collections::BTreeMap;
use std::fmt;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use super::log::{Level, emit};
use super::paths::ProjectRoot;
use super::permission::{PermissionRefusal, PermissionService, Scope as PermissionScope};
use crate::contracts::permission::{Capability, EffectClass, Principal};

/// How long before its expiry a credential is called "expiring soon".
pub const EXPIRING_SOON: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// A value that must not be printed.
///
/// The point is the missing trait: there is no `Display`, `Debug` is written out by hand to hide the
/// value, and the only way to read it is [`Secret::expose`], whose name says what it does at every
/// call site a reviewer reads.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    /// Wraps a value that must not be printed.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The value, for the one place that really needs it.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Whether it is empty, which is a configuration error rather than "no credential".
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// How long it is, which is safe to print and often enough to tell two credentials apart.
    pub fn len(&self) -> usize {
        self.0.len()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The length stays: it is not the value, and it is what a person needs to tell "the
        // credential is empty" from "the credential is there".
        write!(formatter, "Secret([redacted; {} bytes])", self.0.len())
    }
}

/// Replaces every occurrence of every secret in `text` with a placeholder.
///
/// Used before anything is logged or shown. A secret shorter than three characters is *not*
/// substituted: replacing a one- or two-character value would mangle unrelated text and give a false
/// sense of having removed something, so the caller is told instead
/// ([`redaction_is_reliable_for`]).
pub fn redact(text: &str, secrets: &[&Secret]) -> String {
    let mut redacted = text.to_owned();
    for secret in secrets {
        if !redaction_is_reliable_for(secret) {
            continue;
        }
        redacted = redacted.replace(secret.expose(), "[redacted]");
    }
    redacted
}

/// Whether a secret is long enough to redact by substitution.
///
/// A three-character floor is a judgement, and it is written down rather than buried: shorter values
/// collide with ordinary text, so the host says "this cannot be redacted reliably" instead of
/// pretending it did.
pub fn redaction_is_reliable_for(secret: &Secret) -> bool {
    secret.len() >= 3
}

/// Where a credential's material lives.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum CredentialSource {
    /// The operating system's credential store, addressed by the reference's name.
    SystemKeyring,
    /// An environment variable, which is what continuous integration uses.
    Environment(String),
}

impl CredentialSource {
    /// The name used in reports and tests.
    pub const fn name(&self) -> &'static str {
        match self {
            Self::SystemKeyring => "system-keyring",
            Self::Environment(_) => "environment",
        }
    }

    /// Where it is, without saying what is in it.
    pub fn render(&self) -> String {
        match self {
            Self::SystemKeyring => "the system credential store".to_owned(),
            Self::Environment(variable) => format!("the {variable} environment variable"),
        }
    }
}

/// A reference to a credential: where it is, and when it is expected to expire.
///
/// Deliberately not the credential. The expiry is configuration rather than secret (it is a date),
/// and it is what makes "this token is stale" answerable without asking the far end.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CredentialRef {
    name: String,
    source: CredentialSource,
    expires_at: Option<SystemTime>,
}

impl CredentialRef {
    /// A reference to a credential that does not expire.
    pub fn new(name: impl Into<String>, source: CredentialSource) -> Self {
        Self {
            name: name.into(),
            source,
            expires_at: None,
        }
    }

    /// A reference whose credential is expected to expire at a known time.
    pub fn expiring_at(mut self, at: SystemTime) -> Self {
        self.expires_at = Some(at);
        self
    }

    /// The name a person sees and a keyring addresses.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Where the material lives.
    pub fn source(&self) -> &CredentialSource {
        &self.source
    }

    /// When it is expected to expire, when that is known.
    pub const fn expires_at(&self) -> Option<SystemTime> {
        self.expires_at
    }

    /// The reference in one line. Never the credential, because it does not have it.
    pub fn render(&self) -> String {
        match self.expires_at {
            Some(at) => format!(
                "{} in {} (expires {} ms after the epoch)",
                self.name,
                self.source.render(),
                at.duration_since(SystemTime::UNIX_EPOCH)
                    .map_or(0, |since| since.as_millis()),
            ),
            None => format!("{} in {}", self.name, self.source.render()),
        }
    }

    /// What is wrong with this credential, if anything.
    ///
    /// The order matters: a store that is not there is not the same finding as a credential that is
    /// not in it, and an expired credential is reported even when the store is fine, because the
    /// point of the check is to refuse *before* the request rather than after the far end says 401.
    pub fn health(&self, store: &dyn CredentialStore, now: SystemTime) -> CredentialHealth {
        if let Some(at) = self.expires_at
            && let Ok(expired_for) = now.duration_since(at)
        {
            return CredentialHealth::Expired { since: expired_for };
        }
        let material = match store.lookup(self) {
            Some(material) => material,
            None => return store.absence(self),
        };
        if material.is_empty() {
            return CredentialHealth::Missing {
                reason: format!("{} holds an empty value", self.source.render()),
            };
        }
        if let Some(at) = self.expires_at
            && let Ok(remaining) = at.duration_since(now)
            && remaining <= EXPIRING_SOON
        {
            return CredentialHealth::ExpiringSoon { within: remaining };
        }
        CredentialHealth::Usable
    }

    /// The material, or the health finding that explains why there is none.
    /// The time is a parameter rather than a call to the clock inside: "is this stale" has to be
    /// answerable at a chosen instant, or the one thing the acceptance line asks for — that an expiry
    /// is diagnosable — could only be tested by waiting.
    pub fn material(
        &self,
        store: &dyn CredentialStore,
        now: SystemTime,
    ) -> Result<Secret, CredentialHealth> {
        match self.health(store, now) {
            CredentialHealth::Usable | CredentialHealth::ExpiringSoon { .. } => {
                match store.lookup(self) {
                    Some(material) => Ok(material),
                    None => Err(store.absence(self)),
                }
            }
            unhealthy => Err(unhealthy),
        }
    }
}

/// Where a credential store keeps its material.
pub trait CredentialStore: Send + Sync {
    /// The material for a reference, or `None` when it is not there.
    fn lookup(&self, reference: &CredentialRef) -> Option<Secret>;

    /// What to tell a person when [`Self::lookup`] returned `None` for this reference. A store that
    /// cannot be reached says so, rather than letting "missing" stand for "unavailable".
    fn absence(&self, reference: &CredentialRef) -> CredentialHealth;

    /// The store in one line, for a report.
    fn description(&self) -> String;
}

/// An environment-variable store: what continuous integration has instead of a keyring.
pub struct EnvironmentStore;

impl CredentialStore for EnvironmentStore {
    fn lookup(&self, reference: &CredentialRef) -> Option<Secret> {
        match &reference.source {
            CredentialSource::Environment(variable) => {
                std::env::var(variable).ok().map(Secret::new)
            }
            // A keyring reference cannot be answered by the environment: pretending otherwise would
            // be the "found it somewhere" failure this trait exists to avoid.
            CredentialSource::SystemKeyring => None,
        }
    }

    fn absence(&self, reference: &CredentialRef) -> CredentialHealth {
        match &reference.source {
            CredentialSource::Environment(variable) => CredentialHealth::Missing {
                reason: format!("the {variable} environment variable is not set"),
            },
            CredentialSource::SystemKeyring => CredentialHealth::Unavailable {
                reason:
                    "this store answers environment variables only; the system credential store \
                         is a platform adapter this build does not have"
                        .to_owned(),
            },
        }
    }

    fn description(&self) -> String {
        "environment variables".to_owned()
    }
}

/// A store that is not there, which is the honest answer on a machine with no keyring session.
pub struct UnavailableStore {
    reason: String,
}

impl UnavailableStore {
    /// A store that explains why it cannot answer.
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl CredentialStore for UnavailableStore {
    fn lookup(&self, _reference: &CredentialRef) -> Option<Secret> {
        None
    }

    fn absence(&self, _reference: &CredentialRef) -> CredentialHealth {
        CredentialHealth::Unavailable {
            reason: self.reason.clone(),
        }
    }

    fn description(&self) -> String {
        format!("no credential store: {}", self.reason)
    }
}

/// What is known about a credential before it is used.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum CredentialHealth {
    /// There is material and it is not near its expiry.
    Usable,
    /// There is material, and it expires soon enough that a person should renew it.
    ExpiringSoon {
        /// How long is left.
        within: Duration,
    },
    /// The recorded expiry has passed. Diagnosable without asking the far end.
    Expired {
        /// How long ago it expired.
        since: Duration,
    },
    /// The reference names material the store does not have.
    Missing {
        /// What the store looked for.
        reason: String,
    },
    /// The store itself cannot be reached, which is a different problem from a missing credential.
    Unavailable {
        /// Why.
        reason: String,
    },
}

impl CredentialHealth {
    /// The name used in reports and tests.
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Usable => "usable",
            Self::ExpiringSoon { .. } => "expiring-soon",
            Self::Expired { .. } => "expired",
            Self::Missing { .. } => "missing",
            Self::Unavailable { .. } => "unavailable",
        }
    }

    /// Whether a request could go ahead.
    pub const fn is_usable(&self) -> bool {
        matches!(self, Self::Usable | Self::ExpiringSoon { .. })
    }

    /// What a person can do about it. `None` when there is nothing to do.
    pub fn next_step(&self) -> Option<String> {
        match self {
            Self::Usable => None,
            Self::ExpiringSoon { within } => Some(format!(
                "renew {}: it expires within about {} day(s)",
                "this credential",
                within.as_secs() / (24 * 60 * 60),
            )),
            Self::Expired { since } => Some(format!(
                "renew this credential: it expired about {} hour(s) ago",
                since.as_secs() / (60 * 60),
            )),
            Self::Missing { reason } => Some(format!("add the credential: {reason}")),
            Self::Unavailable { reason } => {
                Some(format!("make a credential store reachable: {reason}"))
            }
        }
    }

    /// One line for a report or a notification's detail.
    pub fn message(&self) -> String {
        match self {
            Self::Usable => "usable".to_owned(),
            Self::ExpiringSoon { within } => format!("expires in {within:?}"),
            Self::Expired { since } => format!("expired {since:?} ago"),
            Self::Missing { reason } => format!("missing: {reason}"),
            Self::Unavailable { reason } => format!("unavailable: {reason}"),
        }
    }
}

/// Where the trust anchors for a connection come from.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub enum CertificateSource {
    /// The operating system's own store, which is the default and the only one that exists without
    /// somebody naming a file.
    #[default]
    System,
    /// The system's store *plus* a file, which is what an internal certificate authority needs.
    ///
    /// "Plus", not "instead": the file adds an anchor and never removes one.
    AdditionalFile(PathBuf),
}

/// The TLS configuration for one service.
///
/// There is no field, constructor or builder that turns verification off, and the doc comment is not
/// the guarantee — the absence of one is: the only two things a caller can set are a trust anchor to
/// add and a client certificate to present. [`Self::resolve`] then refuses to continue when what was
/// configured cannot be used, rather than quietly carrying on with less.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct TlsPolicy {
    roots: CertificateSource,
    client_certificate: Option<CredentialRef>,
}

impl TlsPolicy {
    /// The default: the system's anchors, no client certificate.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a trust anchor from a file. The system's anchors stay in force.
    pub fn with_additional_roots(mut self, file: PathBuf) -> Self {
        self.roots = CertificateSource::AdditionalFile(file);
        self
    }

    /// Presents a client certificate, whose material is a credential like any other.
    pub fn with_client_certificate(mut self, certificate: CredentialRef) -> Self {
        self.client_certificate = Some(certificate);
        self
    }

    /// Where the anchors come from.
    pub fn roots(&self) -> &CertificateSource {
        &self.roots
    }

    /// The client certificate, when one is configured.
    pub fn client_certificate(&self) -> Option<&CredentialRef> {
        self.client_certificate.as_ref()
    }

    /// Whether verification is required. Always true, and it is a method rather than a field so that
    /// the answer cannot be configured: nothing in this type can make it false.
    pub const fn verification_required(&self) -> bool {
        true
    }

    /// The configuration, checked against the material it names.
    pub fn resolve(
        &self,
        store: &dyn CredentialStore,
        now: SystemTime,
    ) -> Result<ResolvedTls, TlsRefusal> {
        let roots = match &self.roots {
            CertificateSource::System => CertificateSource::System,
            CertificateSource::AdditionalFile(path) => {
                let text = std::fs::read_to_string(path).map_err(|error| {
                    TlsRefusal::RootFileUnreadable {
                        path: path.clone(),
                        reason: error.to_string(),
                    }
                })?;
                if !text.contains("-----BEGIN CERTIFICATE-----") {
                    return Err(TlsRefusal::RootFileNotACertificate { path: path.clone() });
                }
                CertificateSource::AdditionalFile(path.clone())
            }
        };
        let client_certificate = match &self.client_certificate {
            None => None,
            Some(reference) => Some(reference.material(store, now).map_err(|health| {
                TlsRefusal::ClientCertificateUnhealthy {
                    name: reference.name().to_owned(),
                    health,
                }
            })?),
        };
        Ok(ResolvedTls {
            roots,
            client_certificate,
        })
    }
}

/// A TLS configuration whose material was found and checked.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ResolvedTls {
    roots: CertificateSource,
    client_certificate: Option<Secret>,
}

impl ResolvedTls {
    /// Where the anchors come from.
    pub fn roots(&self) -> &CertificateSource {
        &self.roots
    }

    /// Whether a client certificate is presented, without saying what it is.
    pub fn presents_client_certificate(&self) -> bool {
        self.client_certificate.is_some()
    }

    /// One line for a report. Never the certificate.
    pub fn render(&self) -> String {
        format!(
            "{} verification, anchors from {}, client certificate {}",
            if TlsPolicy::new().verification_required() {
                "certificate"
            } else {
                // Unreachable by construction: the policy type has no state that could get here.
                // Written out anyway so the sentence is decided in one place.
                "no"
            },
            match &self.roots {
                CertificateSource::System => "the system store".to_owned(),
                CertificateSource::AdditionalFile(path) => {
                    format!("the system store plus {}", path.display())
                }
            },
            if self.presents_client_certificate() {
                "presented"
            } else {
                "not configured"
            },
        )
    }
}

/// Why a TLS configuration cannot be used.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum TlsRefusal {
    /// The configured anchor file could not be read.
    RootFileUnreadable {
        /// The file.
        path: PathBuf,
        /// The platform's reason.
        reason: String,
    },
    /// The file exists but does not look like a certificate, so adding it would add nothing.
    RootFileNotACertificate {
        /// The file.
        path: PathBuf,
    },
    /// The client certificate's material is not usable.
    ClientCertificateUnhealthy {
        /// Which credential.
        name: String,
        /// What is wrong with it.
        health: CredentialHealth,
    },
}

impl TlsRefusal {
    /// A sentence for a log line, a panel or a terminal.
    pub fn message(&self) -> String {
        match self {
            Self::RootFileUnreadable { path, reason } => format!(
                "the trust anchors in {} could not be read: {reason}",
                path.display(),
            ),
            Self::RootFileNotACertificate { path } => format!(
                "{} does not contain a certificate, so it would not add a trust anchor",
                path.display(),
            ),
            Self::ClientCertificateUnhealthy { name, health } => format!(
                "the client certificate {name} cannot be used: {}",
                health.message(),
            ),
        }
    }

    /// What a person can do about it.
    pub fn next_step(&self) -> String {
        match self {
            Self::RootFileUnreadable { .. } | Self::RootFileNotACertificate { .. } => {
                "correct the certificate path in the service's configuration".to_owned()
            }
            Self::ClientCertificateUnhealthy { health, .. } => health
                .next_step()
                .unwrap_or_else(|| "replace the client certificate".to_owned()),
        }
    }
}

impl fmt::Display for TlsRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message())
    }
}

impl std::error::Error for TlsRefusal {}

/// The proxy a request should use, taken from the environment when the project does not say.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ProxyConfig {
    url: Option<Secret>,
    no_proxy: Vec<String>,
    from_environment: bool,
}

impl ProxyConfig {
    /// A configuration with no proxy.
    pub fn none() -> Self {
        Self::default()
    }

    /// Reads `HTTPS_PROXY` / `HTTP_PROXY` / `ALL_PROXY` and `NO_PROXY`.
    ///
    /// A proxy URL can carry a username and a password, so it is a [`Secret`] here: the host reads
    /// it, hands it to a transport, and never prints it.
    pub fn from_environment() -> Self {
        let url = [
            "HTTPS_PROXY",
            "https_proxy",
            "HTTP_PROXY",
            "http_proxy",
            "ALL_PROXY",
        ]
        .into_iter()
        .find_map(|name| std::env::var(name).ok())
        .filter(|url| !url.trim().is_empty())
        .map(Secret::new);
        let no_proxy = std::env::var("NO_PROXY")
            .or_else(|_| std::env::var("no_proxy"))
            .map(|value| {
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|entry| !entry.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        Self {
            url,
            no_proxy,
            from_environment: true,
        }
    }

    /// An explicit proxy URL, which is what a project setting provides.
    pub fn explicit(url: Secret) -> Self {
        Self {
            url: Some(url),
            no_proxy: Vec::new(),
            from_environment: false,
        }
    }

    /// Whether a proxy is configured at all.
    pub fn is_set(&self) -> bool {
        self.url.is_some()
    }

    /// Whether it came from the environment rather than from the project.
    pub const fn came_from_environment(&self) -> bool {
        self.from_environment
    }

    /// Whether this host bypasses the proxy.
    pub fn bypasses(&self, host: &str) -> bool {
        self.no_proxy.iter().any(|entry| {
            entry == "*"
                || entry == host
                || host.ends_with(entry.strip_prefix('.').unwrap_or(entry))
                    && entry.starts_with('.')
        })
    }

    /// Whether a request to `host` goes through the proxy.
    pub fn applies_to(&self, host: &str) -> bool {
        self.is_set() && !self.bypasses(host)
    }

    /// The configuration in one line, with the proxy URL redacted.
    pub fn render(&self) -> String {
        match &self.url {
            None => "no proxy configured".to_owned(),
            Some(url) => format!(
                "proxy {} from {}, bypassing {:?}",
                redact(url.expose(), &[url]),
                if self.from_environment {
                    "the environment"
                } else {
                    "the project's configuration"
                },
                self.no_proxy,
            ),
        }
    }

    /// The proxy URL, for the transport that dials it.
    pub fn url(&self) -> Option<&Secret> {
        self.url.as_ref()
    }
}

/// One service a project's network policy declares.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ServicePolicy {
    service: String,
    host: String,
    port: u16,
    tls: TlsPolicy,
    credential: Option<CredentialRef>,
}

impl ServicePolicy {
    /// Declares a service by name, destination and TLS configuration.
    pub fn new(
        service: impl Into<String>,
        host: impl Into<String>,
        port: u16,
        tls: TlsPolicy,
    ) -> Self {
        Self {
            service: service.into(),
            host: host.into(),
            port,
            tls,
            credential: None,
        }
    }

    /// Names the credential this service uses.
    pub fn with_credential(mut self, credential: CredentialRef) -> Self {
        self.credential = Some(credential);
        self
    }

    /// The name the policy addresses it by.
    pub fn service(&self) -> &str {
        &self.service
    }

    /// The host a request goes to.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// The port a request goes to.
    pub const fn port(&self) -> u16 {
        self.port
    }

    /// The TLS configuration.
    pub fn tls(&self) -> &TlsPolicy {
        &self.tls
    }

    /// The credential, when the service needs one.
    pub fn credential(&self) -> Option<&CredentialRef> {
        self.credential.as_ref()
    }

    /// One line for a report. Never a credential.
    pub fn render(&self) -> String {
        format!(
            "{} -> {}:{} ({})",
            self.service,
            self.host,
            self.port,
            match &self.credential {
                Some(reference) => format!("with {}", reference.render()),
                None => "no credential".to_owned(),
            },
        )
    }
}

/// What a project declares about reaching the network.
///
/// The policy is the list of destinations: a service that is not named here cannot be reached at all,
/// which is what "the host does not reach the network on its own" means in configuration. It carries
/// no authorization of its own — that stays in the permission table, so there is one place where
/// "who may do this" is decided.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct NetworkPolicy {
    services: BTreeMap<String, ServicePolicy>,
}

impl NetworkPolicy {
    /// An empty policy, which reaches nothing.
    pub fn new() -> Self {
        Self::default()
    }

    /// Declares a service, replacing any earlier declaration of the same name.
    pub fn declare(&mut self, policy: ServicePolicy) {
        self.services.insert(policy.service.clone(), policy);
    }

    /// The declared services, in name order.
    pub fn declared(&self) -> Vec<&str> {
        self.services.keys().map(String::as_str).collect()
    }

    /// Whether the policy names any service at all.
    pub fn is_empty(&self) -> bool {
        self.services.is_empty()
    }

    /// One declaration.
    pub fn service(&self, name: &str) -> Option<&ServicePolicy> {
        self.services.get(name)
    }

    /// Checks every declaration's material, so a broken policy is a finding before a request is
    /// attempted. Returns one line per service that could not be resolved.
    pub fn resolve_all(&self, store: &dyn CredentialStore, now: SystemTime) -> Vec<String> {
        let mut findings = Vec::new();
        for policy in self.services.values() {
            if let Err(refusal) = policy.tls.resolve(store, now) {
                findings.push(format!("{}: {}", policy.service, refusal.message()));
            }
            if let Some(reference) = &policy.credential {
                let health = reference.health(store, now);
                if !health.is_usable() {
                    findings.push(format!(
                        "{}: {} is {}",
                        policy.service,
                        reference.name(),
                        health.message()
                    ));
                }
            }
        }
        findings
    }
}

/// Why a network request was refused.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum NetworkRefusal {
    /// The project's policy does not name this service.
    NotConfigured {
        /// Which service was asked for.
        service: String,
        /// The services that *are* declared, so the answer is actionable.
        declared: Vec<String>,
    },
    /// The permission gate refused the effect.
    Denied(PermissionRefusal),
    /// The service's credential cannot be used.
    Credential {
        /// Which service.
        service: String,
        /// Which credential.
        name: String,
        /// What is wrong with it.
        health: CredentialHealth,
    },
    /// The service's TLS configuration cannot be resolved.
    Tls {
        /// Which service.
        service: String,
        /// Why.
        refusal: TlsRefusal,
    },
}

impl NetworkRefusal {
    /// A sentence for a log line, a panel or a terminal.
    pub fn message(&self) -> String {
        match self {
            Self::NotConfigured { service, declared } => format!(
                "either declare \"{service}\" in the project's network policy and authorize it, or \
                 do not reach it (declared: {})",
                if declared.is_empty() {
                    "none".to_owned()
                } else {
                    declared.join(", ")
                },
            ),
            Self::Denied(refusal) => refusal.message(),
            Self::Credential {
                service,
                name,
                health,
            } => format!("{service} needs {name}, which is {}", health.message()),
            Self::Tls { service, refusal } => {
                format!("{service} cannot be configured: {}", refusal.message())
            }
        }
    }

    /// What a person can do about it.
    pub fn next_step(&self) -> String {
        match self {
            Self::NotConfigured { .. } => {
                "add the service to the project's network policy and authorize network requests"
                    .to_owned()
            }
            Self::Denied(refusal) => refusal.capability().map_or_else(
                || "check the project's permissions".to_owned(),
                |capability| format!("authorize {} for this project", capability.name()),
            ),
            Self::Credential { health, .. } => health
                .next_step()
                .unwrap_or_else(|| "replace the credential".to_owned()),
            Self::Tls { refusal, .. } => refusal.next_step(),
        }
    }
}

impl fmt::Display for NetworkRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message())
    }
}

impl std::error::Error for NetworkRefusal {}

/// The host's written permission to reach one declared service.
///
/// Like [`super::permission::ExecutionPermit`], its fields are private and exactly one function mints
/// it, so a connection cannot be opened by a caller that skipped the gate and the policy. It carries
/// the resolved material the transport needs and the redaction the report needs.
#[derive(Clone, Debug)]
pub struct NetworkPermit {
    service: String,
    host: String,
    port: u16,
    tls: ResolvedTls,
    credential: Option<Secret>,
    via: Capability,
    policy_source: &'static str,
}

impl NetworkPermit {
    /// The service it is for.
    pub fn service(&self) -> &str {
        &self.service
    }

    /// The host it may reach.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// The port it may reach.
    pub const fn port(&self) -> u16 {
        self.port
    }

    /// The resolved TLS configuration.
    pub fn tls(&self) -> &ResolvedTls {
        &self.tls
    }

    /// Whether a credential is available, without saying what it is.
    pub fn presents_credential(&self) -> bool {
        self.credential.is_some()
    }

    /// The capability that decided this, for the authorization record.
    pub const fn via(&self) -> Capability {
        self.via
    }

    /// Whether the policy that allowed it came from the project or the environment.
    pub const fn policy_source(&self) -> &'static str {
        self.policy_source
    }

    /// The credential in the form a transport needs. The name says what it does.
    pub fn credential_value(&self) -> Option<&Secret> {
        self.credential.as_ref()
    }

    /// The permit in one line, and it is what a log line carries.
    ///
    /// The credential appears through [`Secret`]'s own `Debug`, which prints its length and nothing
    /// else. An earlier version printed the first character "for diagnosis" — which is one character
    /// of a secret in a log, and exactly the leak this module exists to prevent.
    pub fn render(&self) -> String {
        format!(
            "{} -> {}:{} ({}); credential {}",
            self.service,
            self.host,
            self.port,
            self.tls.render(),
            match &self.credential {
                Some(value) => format!("{value:?}"),
                None => "not configured".to_owned(),
            },
        )
    }
}

/// The host's network side: the gate, the credential store and the proxy.
pub struct NetworkGateway {
    permission: Arc<PermissionService>,
    store: Arc<dyn CredentialStore>,
}

impl NetworkGateway {
    /// A gateway around the permission service that decides, and a store that answers.
    pub fn new(permission: Arc<PermissionService>, store: Arc<dyn CredentialStore>) -> Self {
        Self { permission, store }
    }

    /// The credential store this gateway asks.
    pub fn store(&self) -> &Arc<dyn CredentialStore> {
        &self.store
    }

    /// Records that this project declares a policy, which is what lets the gate judge a request.
    ///
    /// A policy with no declarations is recorded as "no policy": an empty list is not a destination.
    pub fn declare_policy(&self, root: &ProjectRoot, policy: &NetworkPolicy) {
        self.permission
            .declare_network_policy(root, !policy.is_empty());
    }

    /// Mints the permit for one request, or gives the reason there is none.
    ///
    /// Four questions, in this order, because each one's answer is more specific than the last: does
    /// the policy name the service, does the gate allow the effect, can the service's certificate
    /// configuration be resolved, and is its credential usable. `Principal::Human` still has to pass
    /// the first two: the policy says *what* may be reached, the table says *who* may reach it.
    pub fn permit(
        &self,
        root: &ProjectRoot,
        policy: &NetworkPolicy,
        service: &str,
        principal: Principal,
    ) -> Result<NetworkPermit, NetworkRefusal> {
        let Some(declared) = policy.service(service) else {
            let refusal = NetworkRefusal::NotConfigured {
                service: service.to_owned(),
                declared: policy.declared().into_iter().map(str::to_owned).collect(),
            };
            log_refusal(&refusal);
            return Err(refusal);
        };

        let scope = PermissionScope::Project(root);
        match self
            .permission
            .permit(EffectClass::Network, scope, principal)
        {
            Ok(_) => {}
            Err(refusal) => {
                let refusal = NetworkRefusal::Denied(refusal);
                log_refusal(&refusal);
                return Err(refusal);
            }
        }

        let now = SystemTime::now();
        let tls = match declared.tls.resolve(self.store.as_ref(), now) {
            Ok(tls) => tls,
            Err(refusal) => {
                let refusal = NetworkRefusal::Tls {
                    service: service.to_owned(),
                    refusal,
                };
                log_refusal(&refusal);
                return Err(refusal);
            }
        };

        let credential = match declared.credential() {
            None => None,
            Some(reference) => match reference.material(self.store.as_ref(), now) {
                Ok(material) => Some(material),
                Err(health) => {
                    let refusal = NetworkRefusal::Credential {
                        service: service.to_owned(),
                        name: reference.name().to_owned(),
                        health,
                    };
                    log_refusal(&refusal);
                    return Err(refusal);
                }
            },
        };

        let permit = NetworkPermit {
            service: declared.service.clone(),
            host: declared.host.clone(),
            port: declared.port,
            tls,
            credential,
            via: Capability::RequestNetworkService,
            policy_source: "the project's network policy",
        };
        emit(
            Level::Info,
            "host::network",
            &format!("permitted a request to {}", permit.render()),
        );
        Ok(permit)
    }
}

/// Opens a connection to the service a permit is for.
///
/// The only way to reach a declared service, and the reason the permit type exists: a caller cannot
/// open a socket without one. Plain TCP for now — this build has no TLS client (see the module docs)
/// — and it says so with the address it used.
pub fn connect(permit: &NetworkPermit, timeout: Duration) -> Result<TcpStream, ConnectFailure> {
    let addresses = (permit.host(), permit.port())
        .to_socket_addrs()
        .map_err(|error| ConnectFailure {
            host: permit.host().to_owned(),
            port: permit.port(),
            reason: error.to_string(),
        })?;
    let mut last = None;
    for address in addresses {
        match TcpStream::connect_timeout(&address, timeout) {
            Ok(stream) => return Ok(stream),
            Err(error) => last = Some(error.to_string()),
        }
    }
    Err(ConnectFailure {
        host: permit.host().to_owned(),
        port: permit.port(),
        reason: last.unwrap_or_else(|| "the host resolved to no address".to_owned()),
    })
}

/// Why a connection could not be made.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ConnectFailure {
    /// The host.
    pub host: String,
    /// The port.
    pub port: u16,
    /// The platform's own reason.
    pub reason: String,
}

impl ConnectFailure {
    /// A sentence for a log line, a panel or a terminal.
    pub fn message(&self) -> String {
        format!(
            "{}:{} could not be reached: {}",
            self.host, self.port, self.reason,
        )
    }
}

impl fmt::Display for ConnectFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message())
    }
}

impl std::error::Error for ConnectFailure {}

/// Writes a line and reads the answer, which is as much protocol as a loopback check needs.
pub fn exchange(stream: &mut TcpStream, request: &str) -> std::io::Result<String> {
    stream.write_all(request.as_bytes())?;
    stream.flush()?;
    let mut answer = String::new();
    stream.read_to_string(&mut answer)?;
    Ok(answer)
}

/// The environment store this build can always offer, or the honest reason it cannot answer.
pub fn default_store() -> Arc<dyn CredentialStore> {
    Arc::new(EnvironmentStore)
}

/// Reads a file and returns its text, redacting any of `secrets` that appear in it.
///
/// Exists so a caller that wants to *show* a configuration file has one function to call: the
/// alternative is every caller remembering to redact, which is how a token ends up in a log.
pub fn read_redacted(path: &Path, secrets: &[&Secret]) -> std::io::Result<String> {
    let text = std::fs::read_to_string(path)?;
    Ok(redact(&text, secrets))
}

fn log_refusal(refusal: &NetworkRefusal) {
    emit(
        Level::Warn,
        "host::network",
        &format!("refused: {}; {}", refusal.message(), refusal.next_step()),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::paths::ProjectRoot;
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A store a test controls, so the read path is exercised without touching the process
    /// environment (mutating it from a parallel test binary is a data race, not a convenience).
    struct TestStore {
        material: std::collections::BTreeMap<String, Secret>,
        description: &'static str,
    }

    impl TestStore {
        fn holding(pairs: &[(&str, &str)]) -> Self {
            Self {
                material: pairs
                    .iter()
                    .map(|(name, value)| ((*name).to_owned(), Secret::new(*value)))
                    .collect(),
                description: "a test store",
            }
        }

        fn empty() -> Self {
            Self {
                material: std::collections::BTreeMap::new(),
                description: "a test store",
            }
        }
    }

    impl CredentialStore for TestStore {
        fn lookup(&self, reference: &CredentialRef) -> Option<Secret> {
            self.material.get(reference.name()).cloned()
        }

        fn absence(&self, reference: &CredentialRef) -> CredentialHealth {
            CredentialHealth::Missing {
                reason: format!("{} is not in {}", reference.name(), self.description),
            }
        }

        fn description(&self) -> String {
            self.description.to_owned()
        }
    }

    /// A temporary project directory, removed when the guard drops.
    ///
    /// The guard is the fix for a defect recorded in the fifth development record (§29.5 item 10):
    /// this helper used to return a bare path and every caller left its directory behind, so a
    /// machine accumulated one `rustage-network-*` per test run (112 of them were found on the
    /// machine that recorded it). `Drop` also covers the panic path, which a cleanup call at the end
    /// of a test does not — and the sibling test modules in `host` and `project` use the same shape.
    struct Sandbox {
        path: PathBuf,
        root: ProjectRoot,
        _guard: TempDirectory,
    }

    impl Sandbox {
        /// The directory itself.
        fn path(&self) -> &Path {
            &self.path
        }

        /// The same directory as a project root, which is what the gateway is asked about.
        fn root(&self) -> &ProjectRoot {
            &self.root
        }
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
            "rustage-network-{name}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed),
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a temporary project is creatable");
        let root = ProjectRoot::new(&path).expect("the sandbox is a directory");
        Sandbox {
            path: path.clone(),
            root,
            _guard: TempDirectory(path),
        }
    }

    fn service(name: &str, port: u16) -> ServicePolicy {
        ServicePolicy::new(name, "127.0.0.1", port, TlsPolicy::new())
    }

    #[test]
    fn a_secret_cannot_be_printed() {
        let secret = Secret::new("s3cret-token-value");

        let printed = format!("{secret:?}");

        assert!(!printed.contains("s3cret"), "{printed}");
        assert!(printed.contains("redacted"), "{printed}");
        assert!(
            printed.contains("18 bytes"),
            "the length is not the value: {printed}"
        );
        assert_eq!(secret.expose(), "s3cret-token-value");
        assert!(!secret.is_empty());
    }

    #[test]
    fn redaction_replaces_a_secret_and_refuses_to_pretend_for_a_short_one() {
        let long = Secret::new("abcdef123456");
        let short = Secret::new("ab");

        let text = format!("token={} other={}", long.expose(), short.expose());
        let redacted = redact(&text, &[&long, &short]);

        assert!(!redacted.contains("abcdef123456"), "{redacted}");
        assert!(redacted.contains("[redacted]"), "{redacted}");
        assert!(redaction_is_reliable_for(&long));
        assert!(
            !redaction_is_reliable_for(&short),
            "a two-character value collides with ordinary text",
        );
        assert_eq!(redact("nothing to hide", &[&long]), "nothing to hide");
    }

    #[test]
    fn a_reference_says_where_the_material_lives_and_nothing_else() {
        let keyring = CredentialRef::new("github-token", CredentialSource::SystemKeyring);
        let variable = CredentialRef::new(
            "ci-token",
            CredentialSource::Environment("RUSTAGE_CI_TOKEN".to_owned()),
        );

        assert!(keyring.render().contains("github-token"));
        assert!(keyring.render().contains("system credential store"));
        assert!(variable.render().contains("RUSTAGE_CI_TOKEN"));
        assert_eq!(keyring.source().name(), "system-keyring");
        assert_eq!(variable.source().name(), "environment");
        assert!(keyring.expires_at().is_none());
    }

    #[test]
    fn an_expired_credential_is_diagnosed_without_asking_anyone() {
        let now = SystemTime::now();
        let expired = CredentialRef::new("stale", CredentialSource::SystemKeyring)
            .expiring_at(now - Duration::from_secs(2 * 60 * 60));
        let store = TestStore::holding(&[("stale", "an-old-token")]);

        let health = expired.health(&store, now);

        match &health {
            CredentialHealth::Expired { since } => {
                assert!(*since >= Duration::from_secs(2 * 60 * 60))
            }
            other => panic!("expected an expiry finding, got {other:?}"),
        }
        assert!(!health.is_usable());
        assert_eq!(health.name(), "expired");
        let step = health.next_step().expect("an expiry is actionable");
        assert!(step.contains("renew"), "{step}");
        assert!(
            expired.material(&store, now).is_err(),
            "an expired credential is not handed out",
        );
    }

    #[test]
    fn a_fresh_credential_is_usable_and_one_near_its_expiry_is_flagged() {
        let now = SystemTime::now();
        let store = TestStore::holding(&[("fresh", "token"), ("soon", "token")]);
        let fresh = CredentialRef::new("fresh", CredentialSource::SystemKeyring)
            .expiring_at(now + Duration::from_secs(90 * 24 * 60 * 60));
        let soon = CredentialRef::new("soon", CredentialSource::SystemKeyring)
            .expiring_at(now + Duration::from_secs(60 * 60));

        assert_eq!(fresh.health(&store, now), CredentialHealth::Usable);
        assert_eq!(fresh.health(&store, now).next_step(), None);
        match soon.health(&store, now) {
            CredentialHealth::ExpiringSoon { within } => assert!(within <= EXPIRING_SOON),
            other => panic!("expected a warning, got {other:?}"),
        }
        assert!(
            soon.health(&store, now).is_usable(),
            "still usable, but flagged"
        );
        assert!(soon.material(&store, now).is_ok());
    }

    #[test]
    fn missing_material_and_an_unreachable_store_are_different_findings() {
        let now = SystemTime::now();
        let reference = CredentialRef::new("absent", CredentialSource::SystemKeyring);

        let missing = reference.health(&TestStore::empty(), now);
        let unavailable = reference.health(
            &UnavailableStore::new("no keyring session on this host"),
            now,
        );

        assert_eq!(missing.name(), "missing");
        assert_eq!(unavailable.name(), "unavailable");
        assert_ne!(
            missing, unavailable,
            "a store that is not there is not a missing secret"
        );
        assert!(unavailable.next_step().unwrap().contains("store"));
    }

    #[test]
    fn the_environment_store_answers_only_environment_references() {
        let store = EnvironmentStore;
        let keyring = CredentialRef::new("keyring-entry", CredentialSource::SystemKeyring);
        let variable = CredentialRef::new(
            "definitely-not-set",
            CredentialSource::Environment("RUSTAGE_DEFINITELY_NOT_SET".to_owned()),
        );

        assert!(store.lookup(&keyring).is_none());
        assert_eq!(store.absence(&keyring).name(), "unavailable");
        assert!(store.lookup(&variable).is_none());
        assert_eq!(store.absence(&variable).name(), "missing");
        assert_eq!(store.description(), "environment variables");
    }

    #[test]
    fn an_empty_value_is_missing_rather_than_usable() {
        let now = SystemTime::now();
        let store = TestStore::holding(&[("blank", "")]);
        let reference = CredentialRef::new("blank", CredentialSource::SystemKeyring);

        let health = reference.health(&store, now);

        assert_eq!(health.name(), "missing");
        assert!(health.message().contains("empty"));
    }

    #[test]
    fn every_tls_configuration_still_requires_verification() {
        let now = SystemTime::now();
        let certificate = CredentialRef::new("client", CredentialSource::SystemKeyring)
            .expiring_at(now + Duration::from_secs(3600));
        let store = TestStore::holding(&[("client", "a-certificate")]);

        let plain = TlsPolicy::new();
        let with_roots =
            TlsPolicy::new().with_additional_roots(PathBuf::from("/nonexistent/ca.pem"));
        let with_certificate = TlsPolicy::new().with_client_certificate(certificate);

        assert!(plain.verification_required());
        assert!(with_roots.verification_required());
        assert!(with_certificate.verification_required());
        // The only two knobs are "add an anchor" and "present a certificate": there is no third one
        // that could remove verification, which is what the acceptance line asks for.
        assert_eq!(plain.roots(), &CertificateSource::System);
        assert!(with_roots.roots() != &CertificateSource::System);
        assert!(with_certificate.client_certificate().is_some());
        assert!(with_certificate.resolve(&store, now).is_ok());
        assert!(
            plain
                .resolve(&store, now)
                .unwrap()
                .render()
                .contains("certificate verification")
        );
    }

    #[test]
    fn an_additional_anchor_has_to_be_a_certificate() {
        let now = SystemTime::now();
        let sandbox = sandbox("anchors");
        let path = sandbox.path();
        let missing = path.join("not-there.pem");
        let junk = path.join("junk.pem");
        let certificate = path.join("ca.pem");
        std::fs::write(&junk, "this is not a certificate").expect("writable");
        std::fs::write(
            &certificate,
            "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n",
        )
        .expect("writable");
        let store = TestStore::empty();

        let unreadable = TlsPolicy::new()
            .with_additional_roots(missing)
            .resolve(&store, now)
            .expect_err("the file is not there");
        let not_a_certificate = TlsPolicy::new()
            .with_additional_roots(junk)
            .resolve(&store, now)
            .expect_err("the file is not a certificate");
        let resolved = TlsPolicy::new()
            .with_additional_roots(certificate.clone())
            .resolve(&store, now)
            .expect("a real certificate resolves");

        assert!(matches!(unreadable, TlsRefusal::RootFileUnreadable { .. }));
        assert!(matches!(
            not_a_certificate,
            TlsRefusal::RootFileNotACertificate { .. }
        ));
        assert!(resolved.render().contains("plus"));
        assert!(!not_a_certificate.message().contains("MIIB"));
        assert!(not_a_certificate.next_step().contains("path"));
    }

    #[test]
    fn a_client_certificate_that_cannot_be_used_stops_the_configuration() {
        let now = SystemTime::now();
        let reference = CredentialRef::new("client", CredentialSource::SystemKeyring)
            .expiring_at(now - Duration::from_secs(60));
        let policy = TlsPolicy::new().with_client_certificate(reference);

        let refusal = policy
            .resolve(&TestStore::holding(&[("client", "a-certificate")]), now)
            .expect_err("an expired certificate is not presented");

        match &refusal {
            TlsRefusal::ClientCertificateUnhealthy { name, health } => {
                assert_eq!(name, "client");
                assert_eq!(health.name(), "expired");
            }
            other => panic!("expected an unhealthy-certificate refusal, got {other:?}"),
        }
        assert!(refusal.next_step().contains("renew"));
    }

    #[test]
    fn a_proxy_url_is_redacted_wherever_it_is_shown() {
        let proxy = ProxyConfig::explicit(Secret::new("http://user:hunter2@proxy.invalid:3128"));

        let rendered = proxy.render();

        assert!(proxy.is_set());
        assert!(!rendered.contains("hunter2"), "{rendered}");
        assert!(rendered.contains("[redacted]"), "{rendered}");
        assert!(
            proxy
                .url()
                .is_some_and(|url| url.expose().contains("hunter2")),
            "the transport still gets the real URL",
        );
        assert!(!proxy.came_from_environment());
        assert!(!ProxyConfig::none().is_set());
    }

    #[test]
    fn bypass_entries_decide_whether_a_host_goes_through_the_proxy() {
        let proxy = ProxyConfig {
            url: Some(Secret::new("http://proxy.invalid:3128")),
            no_proxy: vec![
                "example.invalid".to_owned(),
                ".internal".to_owned(),
                "127.0.0.1".to_owned(),
            ],
            from_environment: false,
        };

        assert!(proxy.applies_to("crates.io"));
        assert!(!proxy.applies_to("example.invalid"));
        assert!(!proxy.applies_to("build.internal"));
        assert!(!proxy.applies_to("127.0.0.1"));
        assert!(!ProxyConfig::none().applies_to("crates.io"));
    }

    #[test]
    fn a_policy_reports_the_services_whose_material_is_broken() {
        let now = SystemTime::now();
        let mut policy = NetworkPolicy::new();
        policy.declare(service("fine", 443));
        policy.declare(
            service("stale", 443).with_credential(
                CredentialRef::new("stale", CredentialSource::SystemKeyring)
                    .expiring_at(now - Duration::from_secs(60)),
            ),
        );
        assert_eq!(policy.declared(), vec!["fine", "stale"]);
        assert!(!policy.is_empty());

        let findings = policy.resolve_all(&TestStore::empty(), now);

        assert_eq!(findings.len(), 1, "{findings:?}");
        assert!(findings[0].contains("stale"), "{findings:?}");
        assert!(!findings[0].contains("fine"));
    }

    #[test]
    fn a_service_that_is_not_declared_is_refused_with_the_list_that_is() {
        let sandbox = sandbox("undeclared");
        let root = sandbox.root();
        let permission = Arc::new(PermissionService::new());
        let gateway = NetworkGateway::new(permission, Arc::new(TestStore::empty()));
        let mut policy = NetworkPolicy::new();
        policy.declare(service("declared", 443));

        let refusal = gateway
            .permit(root, &policy, "something-else", Principal::Human)
            .expect_err("a service nobody declared is not reachable");

        match &refusal {
            NetworkRefusal::NotConfigured { service, declared } => {
                assert_eq!(service, "something-else");
                assert_eq!(declared, &vec!["declared".to_owned()]);
            }
            other => panic!("expected a configuration refusal, got {other:?}"),
        }
        assert!(refusal.next_step().contains("network policy"));
        assert!(NetworkPolicy::new().declared().is_empty());
    }

    #[test]
    fn an_untrusted_project_reaches_nothing_even_when_the_service_is_declared() {
        let sandbox = sandbox("untrusted");
        let root = sandbox.root();
        let permission = Arc::new(PermissionService::new());
        let gateway = NetworkGateway::new(permission, Arc::new(TestStore::empty()));
        let mut policy = NetworkPolicy::new();
        policy.declare(service("declared", 443));

        let refusal = gateway
            .permit(root, &policy, "declared", Principal::Human)
            .expect_err("an untrusted project runs nothing, and nothing leaves it");

        match refusal {
            NetworkRefusal::Denied(denial) => {
                assert_eq!(denial.capability(), Some(Capability::RequestNetworkService));
            }
            other => panic!("expected the gate's refusal, got {other:?}"),
        }
    }

    #[test]
    fn a_trusted_project_with_the_scope_authorized_gets_a_permit() {
        let sandbox = sandbox("permit");
        let root = sandbox.root();
        let permission = Arc::new(PermissionService::new());
        permission.trust(root);
        permission
            .authorize(
                PermissionScope::Project(root),
                Capability::RequestNetworkService,
            )
            .expect("authorized");
        let gateway = NetworkGateway::new(Arc::clone(&permission), Arc::new(TestStore::empty()));
        let mut policy = NetworkPolicy::new();
        policy.declare(service("declared", 8443));
        gateway.declare_policy(root, &policy);

        let permit = gateway
            .permit(root, &policy, "declared", Principal::Agent)
            .expect("the project is trusted and the scope is authorized");

        assert_eq!(permit.service(), "declared");
        assert_eq!(permit.host(), "127.0.0.1");
        assert_eq!(permit.port(), 8443);
        assert_eq!(permit.via(), Capability::RequestNetworkService);
        assert!(!permit.presents_credential());
        assert!(permit.render().contains("certificate verification"));
    }

    #[test]
    fn a_permit_carries_the_credential_and_never_prints_it() {
        let now = SystemTime::now();
        let sandbox = sandbox("credential");
        let root = sandbox.root();
        let permission = Arc::new(PermissionService::new());
        permission.trust(root);
        let store = Arc::new(TestStore::holding(&[("service-token", "hunter2-token")]));
        let gateway = NetworkGateway::new(permission, store);
        let mut policy = NetworkPolicy::new();
        policy.declare(
            service("declared", 443).with_credential(
                CredentialRef::new("service-token", CredentialSource::SystemKeyring)
                    .expiring_at(now + Duration::from_secs(90 * 24 * 60 * 60)),
            ),
        );
        gateway.declare_policy(root, &policy);

        let permit = gateway
            .permit(root, &policy, "declared", Principal::Human)
            .expect("permitted");

        assert!(permit.presents_credential());
        assert_eq!(
            permit.credential_value().map(Secret::expose),
            Some("hunter2-token")
        );
        assert!(!permit.render().contains("hunter2"), "{}", permit.render());
        assert_eq!(
            redact(&permit.render(), &[&Secret::new("hunter2-token")]),
            permit.render()
        );
    }

    #[test]
    fn an_expired_credential_stops_the_request_before_it_is_made() {
        let sandbox = sandbox("stale");
        let root = sandbox.root();
        let permission = Arc::new(PermissionService::new());
        permission.trust(root);
        let gateway = NetworkGateway::new(
            permission,
            Arc::new(TestStore::holding(&[("stale", "an-old-token")])),
        );
        let mut policy = NetworkPolicy::new();
        policy.declare(
            service("declared", 443).with_credential(
                CredentialRef::new("stale", CredentialSource::SystemKeyring)
                    .expiring_at(SystemTime::now() - Duration::from_secs(60 * 60)),
            ),
        );
        gateway.declare_policy(root, &policy);

        let refusal = gateway
            .permit(root, &policy, "declared", Principal::Human)
            .expect_err("a stale credential is diagnosed, not sent");

        match &refusal {
            NetworkRefusal::Credential {
                service,
                name,
                health,
            } => {
                assert_eq!(service, "declared");
                assert_eq!(name, "stale");
                assert_eq!(health.name(), "expired");
            }
            other => panic!("expected a credential refusal, got {other:?}"),
        }
        assert!(refusal.next_step().contains("renew"));
        assert!(refusal.message().contains("expired"));
    }

    #[test]
    fn a_real_request_goes_through_a_permit_and_answers_on_the_wire() {
        let sandbox = sandbox("loopback");
        let root = sandbox.root();
        let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback listener");
        let port = listener.local_addr().expect("an address").port();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buffer = [0u8; 64];
                if let Ok(read) = stream.read(&mut buffer) {
                    let _ = stream.write_all(&buffer[..read]);
                }
            }
        });
        let permission = Arc::new(PermissionService::new());
        permission.trust(root);
        permission
            .authorize(
                PermissionScope::Project(root),
                Capability::RequestNetworkService,
            )
            .expect("authorized");
        let gateway = NetworkGateway::new(permission, Arc::new(TestStore::empty()));
        let mut policy = NetworkPolicy::new();
        policy.declare(service("readiness-loopback", port));
        gateway.declare_policy(root, &policy);

        let permit = gateway
            .permit(root, &policy, "readiness-loopback", Principal::Human)
            .expect("permitted");
        let mut stream = connect(&permit, Duration::from_secs(5)).expect("connected");
        let answer = exchange(&mut stream, "ping").expect("answered");

        assert_eq!(answer, "ping", "the permit opened a real connection");
        // And a connection the permit does not cover is not reachable: a port nothing listens on.
        let refused = connect(
            &NetworkPermit {
                service: "unused".to_owned(),
                host: "127.0.0.1".to_owned(),
                port: 1,
                tls: ResolvedTls {
                    roots: CertificateSource::System,
                    client_certificate: None,
                },
                credential: None,
                via: Capability::RequestNetworkService,
                policy_source: "a test",
            },
            Duration::from_millis(200),
        );
        assert!(refused.is_err(), "port 1 is not listening");
        assert!(refused.unwrap_err().message().contains("127.0.0.1"));
    }

    #[test]
    fn a_refusal_is_a_sentence_with_a_next_step() {
        let sandbox = sandbox("sentences");
        let root = sandbox.root();
        let permission = Arc::new(PermissionService::new());
        let gateway = NetworkGateway::new(permission, Arc::new(TestStore::empty()));
        let policy = NetworkPolicy::new();

        let refusal = gateway
            .permit(root, &policy, "anything", Principal::Human)
            .expect_err("nothing is declared");

        assert!(
            refusal.message().split_whitespace().count() >= 10,
            "{}",
            refusal.message()
        );
        assert!(!refusal.next_step().is_empty());
        assert!(!refusal.to_string().is_empty());
    }

    #[test]
    fn a_file_can_be_read_with_its_secrets_redacted() {
        let sandbox = sandbox("redacted-file");
        let path = sandbox.path();
        let file = path.join("config.toml");
        std::fs::write(
            &file,
            "token = \"hunter2-token\"\nproxy = \"http://proxy.invalid\"\n",
        )
        .expect("writable");
        let secret = Secret::new("hunter2-token");

        let text = read_redacted(&file, &[&secret]).expect("readable");

        assert!(!text.contains("hunter2"), "{text}");
        assert!(text.contains("[redacted]"));
        assert!(text.contains("proxy.invalid"), "only the secret is removed");
    }

    #[test]
    fn a_port_nothing_listens_on_is_reported_with_the_address_it_tried() {
        // The shape of a failure a person will see: the message has to name the address rather than
        // swallow the platform's reason.
        let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback listener");
        let port = listener.local_addr().expect("an address").port();
        drop(listener);

        let permit = NetworkPermit {
            service: "closed".to_owned(),
            host: "127.0.0.1".to_owned(),
            port,
            tls: ResolvedTls {
                roots: CertificateSource::System,
                client_certificate: None,
            },
            credential: None,
            via: Capability::RequestNetworkService,
            policy_source: "a test",
        };
        let failure = connect(&permit, Duration::from_millis(300)).expect_err("nothing listens");

        assert!(
            failure.message().contains(&port.to_string()),
            "{}",
            failure.message()
        );
        assert!(failure.message().contains("127.0.0.1"));
        assert!(failure.to_string().contains("could not be reached"));
    }
}
