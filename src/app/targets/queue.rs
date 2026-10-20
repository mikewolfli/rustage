// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The build/deployment queue, artifact tracking and signing references (task T18.07).
//!
//! The acceptance line is *"one target's failure is not a matrix success, and no key is recorded"*.
//! The design answers both with types:
//!
//! * **A queued or failed job is not done.** [`Queue::is_success`] is true only when every job is
//!   [`State::Done`], so one failure, one cancellation and one job still waiting all keep the queue
//!   from looking green. [`Summary`] names each count so the reason is visible.
//! * **An artifact exists only when a job produced one.** [`Queue::pass`] records an
//!   [`Artifact`], and [`Queue::artifacts`] lists exactly those; a failed job has none, and an
//!   artifact is measured from the file on disk ([`Artifact::of_file`]) rather than invented.
//! * **A credential is a reference, never a value.**
//!   [`CredentialRef`] has a name and a source and no field that could hold a secret, so a signed
//!   job's report cannot contain one. [`redact`] is the same rule for text a caller already holds.
//!
//! Signing is attached to a job rather than run here: this module decides what is recorded, and a
//! later tool adapter decides how a signature is produced.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use super::configuration::Os;
use super::matrix::Proof;

/// What a queued job does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    /// Build a target.
    Build,
    /// Deliver a built artifact.
    Deploy,
    /// Sign a built artifact.
    Sign,
}

impl Action {
    /// The action's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::Deploy => "deploy",
            Self::Sign => "sign",
        }
    }

    /// Reads an action name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "build" => Some(Self::Build),
            "deploy" => Some(Self::Deploy),
            "sign" => Some(Self::Sign),
            _ => None,
        }
    }
}

/// What one job is doing.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum State {
    /// It is waiting for the queue.
    Queued,
    /// It is running.
    Running,
    /// It finished, reaching a proof level.
    Done(Proof),
    /// It failed, with the reason.
    Failed(String),
    /// It was cancelled.
    Cancelled,
}

impl State {
    /// The state in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Queued => "queued".to_owned(),
            Self::Running => "running".to_owned(),
            Self::Done(proof) => format!("done {}", proof.name()),
            Self::Failed(reason) => format!("failed: {reason}"),
            Self::Cancelled => "cancelled".to_owned(),
        }
    }
}

/// Where a signing credential is read from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CredentialSource {
    /// An environment variable.
    Environment,
    /// The platform keychain.
    Keychain,
    /// A file the person controls.
    File,
}

impl CredentialSource {
    /// The source's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Environment => "environment",
            Self::Keychain => "keychain",
            Self::File => "file",
        }
    }

    /// Reads a source name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "environment" => Some(Self::Environment),
            "keychain" => Some(Self::Keychain),
            "file" => Some(Self::File),
            _ => None,
        }
    }
}

/// A reference to a credential, with no way to hold its value.
///
/// There is deliberately no `value` field: a type that cannot store a secret cannot leak one into a
/// report, a log line or an artifact.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CredentialRef {
    /// The variable, keychain item or file the secret is read from.
    pub name: String,
    /// Where it is read from.
    pub source: CredentialSource,
}

impl CredentialRef {
    /// A reference to a credential.
    pub fn new(name: impl Into<String>, source: CredentialSource) -> Self {
        Self {
            name: name.into(),
            source,
        }
    }

    /// The reference in one line, deliberately without a value.
    pub fn render(&self) -> String {
        format!("{} from {} (reference only)", self.name, self.source.name())
    }

    /// Whether the referent exists, without reading its value.
    ///
    /// An environment variable is checked for presence, a file for existence, and a keychain item
    /// through `security find-generic-password -s <name>`, which reports metadata and does not ask
    /// for the secret. The value itself is never read, so this cannot put one into a report.
    pub fn check(&self, host: Os) -> Result<(), Refusal> {
        match self.source {
            CredentialSource::Environment => match std::env::var_os(&self.name) {
                Some(_) => Ok(()),
                None => Err(Refusal::CredentialMissing {
                    name: self.name.clone(),
                    source: self.source,
                }),
            },
            CredentialSource::File => {
                if Path::new(&self.name).is_file() {
                    Ok(())
                } else {
                    Err(Refusal::CredentialMissing {
                        name: self.name.clone(),
                        source: self.source,
                    })
                }
            }
            CredentialSource::Keychain => {
                if host != Os::Macos {
                    return Err(Refusal::CredentialUnverifiable {
                        name: self.name.clone(),
                        reason: "a keychain is a macOS service; this host has no such store"
                            .to_owned(),
                    });
                }
                let answer = std::process::Command::new("security")
                    .args(["find-generic-password", "-s", &self.name])
                    .output();
                match answer {
                    Ok(output) if output.status.success() => Ok(()),
                    Ok(_) => Err(Refusal::CredentialMissing {
                        name: self.name.clone(),
                        source: self.source,
                    }),
                    Err(error) => Err(Refusal::CredentialUnverifiable {
                        name: self.name.clone(),
                        reason: format!("security could not be started: {error}"),
                    }),
                }
            }
        }
    }
}

/// How a job is signed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Signing {
    /// The identity the signature names.
    pub identity: String,
    /// The credential the signature uses, by reference.
    pub credential: CredentialRef,
}

/// What one signing attempt achieved.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SignatureState {
    /// The tool signed the artifact and then verified the signature.
    Signed,
    /// The host has no signing tool this build drives.
    Unavailable {
        /// Why.
        reason: String,
    },
    /// The tool ran and refused.
    Failed {
        /// What it said.
        reason: String,
    },
}

/// The outcome of one signing attempt, with no secret in it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Signature {
    /// The identity that was asked for.
    pub identity: String,
    /// The program that was used.
    pub program: String,
    /// What it achieved.
    pub state: SignatureState,
}

impl Signature {
    /// The signature in one line.
    pub fn render(&self) -> String {
        match &self.state {
            SignatureState::Signed => {
                format!("signed with {} by {}", self.identity, self.program)
            }
            SignatureState::Unavailable { reason } => {
                format!("signing unavailable: {reason}")
            }
            SignatureState::Failed { reason } => {
                format!("signing failed with {}: {reason}", self.program)
            }
        }
    }
}

/// Signs an artifact with the host's own tool, then verifies the signature.
///
/// This is the real seam T18.07's record points at: macOS runs `codesign --force --sign
/// <identity>` and Linux runs `gpg --detach-sign`; the tool's own verification must agree before the
/// result is [`SignatureState::Signed`]. Windows answers `Unavailable` naming `signtool`, rather
/// than pretending it was driven.
///
/// `gpg` is invoked without a passphrase: the credential reference is checked for existence by the
/// caller and is never passed on a command line. A key that requires a passphrase therefore fails
/// with the tool's own message, which is the honest outcome for a non-interactive signing job.
pub fn sign(identity: &str, artifact: &Path, host: Os) -> Result<Signature, Refusal> {
    sign_with(identity, artifact, host, None)
}

/// `sign`, with an optional GnuPG home used by the unit test.
///
/// The test must not create a key in the user's keyring, so it passes a throwaway `--homedir`.
/// Production passes `None` and lets `gpg` read the user's own configuration.
fn sign_with(
    identity: &str,
    artifact: &Path,
    host: Os,
    gpg_home: Option<&Path>,
) -> Result<Signature, Refusal> {
    if !artifact.is_file() {
        return Err(Refusal::ArtifactMissing {
            path: artifact.display().to_string(),
        });
    }
    match host {
        Os::Macos => sign_with_codesign(identity, artifact),
        Os::Linux => sign_with_gpg(identity, artifact, gpg_home),
        Os::Windows => Ok(Signature {
            identity: identity.to_owned(),
            program: "signtool".to_owned(),
            state: SignatureState::Unavailable {
                reason:
                    "this build has no windows signing adapter; only codesign and gpg are driven"
                        .to_owned(),
            },
        }),
    }
}

fn sign_with_codesign(identity: &str, artifact: &Path) -> Result<Signature, Refusal> {
    let artifact = artifact.display().to_string();
    let sign = std::process::Command::new("codesign")
        .args(["--force", "--sign", identity, &artifact])
        .output();
    let answer = match sign {
        Ok(answer) => answer,
        Err(error) => {
            return Ok(Signature {
                identity: identity.to_owned(),
                program: "codesign".to_owned(),
                state: SignatureState::Failed {
                    reason: format!("codesign could not be started: {error}"),
                },
            });
        }
    };
    if !answer.status.success() {
        return Ok(Signature {
            identity: identity.to_owned(),
            program: "codesign".to_owned(),
            state: SignatureState::Failed {
                reason: String::from_utf8_lossy(&answer.stderr).trim().to_owned(),
            },
        });
    }
    let verify = std::process::Command::new("codesign")
        .args(["--verify", "--verbose", &artifact])
        .output();
    let state = match verify {
        Ok(answer) if answer.status.success() => SignatureState::Signed,
        Ok(answer) => SignatureState::Failed {
            reason: format!(
                "codesign signed the file but verification failed: {}",
                String::from_utf8_lossy(&answer.stderr).trim()
            ),
        },
        Err(error) => SignatureState::Failed {
            reason: format!("codesign verification could not be started: {error}"),
        },
    };
    Ok(Signature {
        identity: identity.to_owned(),
        program: "codesign".to_owned(),
        state,
    })
}

fn sign_with_gpg(
    identity: &str,
    artifact: &Path,
    gpg_home: Option<&Path>,
) -> Result<Signature, Refusal> {
    // `artifact.sig` rather than `with_extension("sig")`: an artifact named `app.apk` must keep its
    // name and gain a suffix, not become `app.sig`.
    let mut signature_path = artifact.as_os_str().to_owned();
    signature_path.push(".asc");
    let signature_path = PathBuf::from(signature_path);

    let mut sign = std::process::Command::new("gpg");
    if let Some(home) = gpg_home {
        sign.arg("--homedir").arg(home);
    }
    let answer = sign
        .args([
            "--batch",
            "--yes",
            "--detach-sign",
            "--local-user",
            identity,
        ])
        .arg("--output")
        .arg(&signature_path)
        .arg(artifact)
        .output();
    let answer = match answer {
        Ok(answer) => answer,
        Err(error) => {
            return Ok(Signature {
                identity: identity.to_owned(),
                program: "gpg".to_owned(),
                state: SignatureState::Failed {
                    reason: format!("gpg could not be started: {error}"),
                },
            });
        }
    };
    if !answer.status.success() {
        return Ok(Signature {
            identity: identity.to_owned(),
            program: "gpg".to_owned(),
            state: SignatureState::Failed {
                reason: String::from_utf8_lossy(&answer.stderr).trim().to_owned(),
            },
        });
    }

    let mut verify = std::process::Command::new("gpg");
    if let Some(home) = gpg_home {
        verify.arg("--homedir").arg(home);
    }
    let verify = verify
        .args(["--batch", "--verify"])
        .arg(&signature_path)
        .arg(artifact)
        .output();
    let state = match verify {
        Ok(answer) if answer.status.success() => SignatureState::Signed,
        Ok(answer) => SignatureState::Failed {
            reason: format!(
                "gpg signed the file but verification failed: {}",
                String::from_utf8_lossy(&answer.stderr).trim()
            ),
        },
        Err(error) => SignatureState::Failed {
            reason: format!("gpg verification could not be started: {error}"),
        },
    };
    Ok(Signature {
        identity: identity.to_owned(),
        program: "gpg".to_owned(),
        state,
    })
}

impl Signing {
    /// A signing identity and its credential reference.
    pub fn new(identity: impl Into<String>, credential: CredentialRef) -> Self {
        Self {
            identity: identity.into(),
            credential,
        }
    }

    /// The signing in one line.
    pub fn render(&self) -> String {
        format!("{} with {}", self.identity, self.credential.render())
    }
}

/// One artifact a job produced.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Artifact {
    /// The job that produced it.
    pub job: String,
    /// The path, as reported.
    pub path: String,
    /// Its size in bytes.
    pub bytes: u64,
    /// A hash of its bytes, so two artifacts from different jobs can be told apart.
    pub digest: u64,
}

impl Artifact {
    /// Measures a file.
    ///
    /// The digest is FNV-1a, chosen because it needs no dependency and its only job here is to tell
    /// one artifact from another in a report — it is not a signature and is not used as one.
    pub fn of_file(job: impl Into<String>, path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let bytes = std::fs::read(path)
            .map_err(|error| format!("{} could not be read: {error}", path.display()))?;
        let mut digest = 0xcbf2_9ce4_8422_2325_u64;
        for byte in &bytes {
            digest ^= u64::from(*byte);
            digest = digest.wrapping_mul(0x0000_0100_0000_01b3);
        }
        Ok(Self {
            job: job.into(),
            path: path.display().to_string(),
            bytes: bytes.len() as u64,
            digest,
        })
    }

    /// The artifact in one line.
    pub fn render(&self) -> String {
        format!(
            "{}: {} ({} bytes, {:016x})",
            self.job, self.path, self.bytes, self.digest
        )
    }
}

/// One queued job.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Job {
    /// The job's id.
    pub id: String,
    /// The target it acts on.
    pub target: String,
    /// What it does.
    pub action: Action,
    /// What it is doing.
    pub state: State,
    /// How it is signed, for a signing job.
    pub signing: Option<Signing>,
    /// What signing it achieved, when the host tool was driven.
    pub signature: Option<Signature>,
    /// The artifact it produced, for a build or deploy job that finished.
    pub artifact: Option<Artifact>,
}

impl Job {
    fn new(id: impl Into<String>, target: impl Into<String>, action: Action) -> Self {
        Self {
            id: id.into(),
            target: target.into(),
            action,
            state: State::Queued,
            signing: None,
            signature: None,
            artifact: None,
        }
    }

    /// The job in one line.
    pub fn render(&self) -> String {
        let signing = self
            .signing
            .as_ref()
            .map(|signing| format!(" signing {}", signing.render()))
            .unwrap_or_default();
        format!(
            "{} {} {}: {}{signing}",
            self.id,
            self.action.name(),
            self.target,
            self.state.render()
        )
    }

    /// The signature in one line, when one was attempted.
    pub fn signature_line(&self) -> Option<String> {
        self.signature.as_ref().map(Signature::render)
    }
}

/// Counts for a queue.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Summary {
    /// Every job.
    pub total: usize,
    /// Jobs waiting.
    pub queued: usize,
    /// Jobs running.
    pub running: usize,
    /// Jobs that finished.
    pub done: usize,
    /// Jobs that failed.
    pub failed: usize,
    /// Jobs that were cancelled.
    pub cancelled: usize,
}

impl Summary {
    /// The summary in one line.
    pub fn render(&self) -> String {
        format!(
            "{} job(s): {} queued, {} running, {} done, {} failed, {} cancelled",
            self.total, self.queued, self.running, self.done, self.failed, self.cancelled
        )
    }
}

/// Why a queue operation cannot happen.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Refusal {
    /// The id is already used.
    DuplicateJob {
        /// The id.
        id: String,
    },
    /// There is no job with this id.
    NoSuchJob {
        /// The id.
        id: String,
    },
    /// A signing job has no credential reference.
    SignatureMissing {
        /// The id.
        id: String,
    },
    /// A signing reference was attached to a job that does not sign.
    NotSigning {
        /// The id.
        id: String,
    },
    /// The artifact is not there.
    ArtifactMissing {
        /// The path.
        path: String,
    },
    /// A finished job already recorded an artifact.
    ArtifactAlreadyRecorded {
        /// The id.
        id: String,
    },
    /// The credential reference does not resolve on this machine.
    CredentialMissing {
        /// The name it refers to.
        name: String,
        /// Where it was looked for.
        source: CredentialSource,
    },
    /// The credential source cannot be checked on this host.
    CredentialUnverifiable {
        /// The name it refers to.
        name: String,
        /// Why.
        reason: String,
    },
    /// An artifact exists but could not be read for its record.
    ArtifactUnreadable {
        /// The path.
        path: String,
        /// Why.
        reason: String,
    },
}

impl Refusal {
    /// The refusal in one line.
    pub fn render(&self) -> String {
        match self {
            Self::DuplicateJob { id } => format!("job {id} is already queued"),
            Self::NoSuchJob { id } => format!("there is no job {id}"),
            Self::SignatureMissing { id } => {
                format!("job {id} signs, so it needs a credential reference")
            }
            Self::NotSigning { id } => format!("job {id} does not sign"),
            Self::ArtifactMissing { path } => format!("there is no artifact at {path}"),
            Self::ArtifactAlreadyRecorded { id } => {
                format!("job {id} already recorded an artifact")
            }
            Self::CredentialMissing { name, source } => {
                format!("credential {name} is not in the {} store", source.name())
            }
            Self::CredentialUnverifiable { name, reason } => {
                format!("credential {name} cannot be checked: {reason}")
            }
            Self::ArtifactUnreadable { path, reason } => {
                format!("{path} could not be read: {reason}")
            }
        }
    }
}

/// The queue: jobs in submission order.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Queue {
    jobs: Vec<Job>,
}

impl Queue {
    /// An empty queue.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a job.
    pub fn enqueue(
        &mut self,
        id: impl Into<String>,
        target: impl Into<String>,
        action: Action,
    ) -> Result<(), Refusal> {
        let id = id.into();
        if self.jobs.iter().any(|job| job.id == id) {
            return Err(Refusal::DuplicateJob { id });
        }
        self.jobs.push(Job::new(id, target, action));
        Ok(())
    }

    /// Every job, in submission order.
    pub fn jobs(&self) -> &[Job] {
        &self.jobs
    }

    /// One job by id.
    pub fn job(&self, id: &str) -> Option<&Job> {
        self.jobs.iter().find(|job| job.id == id)
    }

    fn job_mut(&mut self, id: &str) -> Result<&mut Job, Refusal> {
        self.jobs
            .iter_mut()
            .find(|job| job.id == id)
            .ok_or_else(|| Refusal::NoSuchJob { id: id.to_owned() })
    }

    /// Attaches a signing identity and credential reference to a signing job.
    pub fn attach_signing(&mut self, id: &str, signing: Signing) -> Result<(), Refusal> {
        let job = self.job_mut(id)?;
        if job.action != Action::Sign {
            return Err(Refusal::NotSigning { id: id.to_owned() });
        }
        job.signing = Some(signing);
        Ok(())
    }

    /// Starts the first waiting job.
    pub fn start(&mut self, id: &str) -> Result<(), Refusal> {
        let job = self.job_mut(id)?;
        job.state = State::Running;
        Ok(())
    }

    /// Signs a signing job's artifact with the host tool, recording what it achieved.
    ///
    /// The credential reference is checked *before* the tool runs, so a signature that needs a
    /// secret the machine does not have fails without leaving a half-signed file. A failed or
    /// unavailable signature becomes the job's state; it is never recorded as done.
    pub fn sign(&mut self, id: &str, artifact_path: &Path, host: Os) -> Result<Signature, Refusal> {
        let job = self.job_mut(id)?;
        if job.action != Action::Sign {
            return Err(Refusal::NotSigning { id: id.to_owned() });
        }
        let Some(signing) = job.signing.clone() else {
            return Err(Refusal::SignatureMissing { id: id.to_owned() });
        };
        signing.credential.check(host)?;
        let signature = sign(&signing.identity, artifact_path, host)?;
        match &signature.state {
            SignatureState::Signed => {
                job.artifact = Some(Artifact::of_file(id, artifact_path).map_err(|reason| {
                    Refusal::ArtifactUnreadable {
                        path: artifact_path.display().to_string(),
                        reason,
                    }
                })?);
                job.state = State::Done(Proof::Built);
            }
            SignatureState::Unavailable { reason } => {
                job.state = State::Failed(format!("signing unavailable: {reason}"));
            }
            SignatureState::Failed { reason } => {
                job.state = State::Failed(format!("signing failed: {reason}"));
            }
        }
        job.signature = Some(signature.clone());
        Ok(signature)
    }

    /// Finishes a job, recording the artifact it produced when there is one.
    ///
    /// A signing job without a credential reference is refused here rather than accepted and
    /// rendered as if it had one.
    pub fn pass(
        &mut self,
        id: &str,
        proof: Proof,
        artifact: Option<Artifact>,
    ) -> Result<(), Refusal> {
        let job = self.job_mut(id)?;
        if job.action == Action::Sign && job.signing.is_none() {
            return Err(Refusal::SignatureMissing { id: id.to_owned() });
        }
        if job.artifact.is_some() && artifact.is_some() {
            return Err(Refusal::ArtifactAlreadyRecorded { id: id.to_owned() });
        }
        if artifact.is_some() {
            job.artifact = artifact;
        }
        job.state = State::Done(proof);
        Ok(())
    }

    /// Fails a job; a failed job records no artifact.
    pub fn fail(&mut self, id: &str, reason: impl Into<String>) -> Result<(), Refusal> {
        let job = self.job_mut(id)?;
        job.state = State::Failed(reason.into());
        job.artifact = None;
        Ok(())
    }

    /// Cancels a job.
    pub fn cancel(&mut self, id: &str) -> Result<(), Refusal> {
        let job = self.job_mut(id)?;
        job.state = State::Cancelled;
        Ok(())
    }

    /// The first job still waiting, in submission order.
    pub fn next(&self) -> Option<&Job> {
        self.jobs.iter().find(|job| job.state == State::Queued)
    }

    /// Counts for every state.
    pub fn summary(&self) -> Summary {
        let mut summary = Summary {
            total: self.jobs.len(),
            ..Summary::default()
        };
        for job in &self.jobs {
            match job.state {
                State::Queued => summary.queued += 1,
                State::Running => summary.running += 1,
                State::Done(_) => summary.done += 1,
                State::Failed(_) => summary.failed += 1,
                State::Cancelled => summary.cancelled += 1,
            }
        }
        summary
    }

    /// Whether every job finished successfully.
    ///
    /// An empty queue is not a success: nothing was done, and a matrix report that treats "no jobs"
    /// as green is how an empty deployment looks finished.
    pub fn is_success(&self) -> bool {
        !self.jobs.is_empty()
            && self
                .jobs
                .iter()
                .all(|job| matches!(job.state, State::Done(_)))
    }

    /// Every artifact a finished job recorded.
    pub fn artifacts(&self) -> Vec<&Artifact> {
        self.jobs
            .iter()
            .filter_map(|job| job.artifact.as_ref())
            .collect()
    }

    /// The queue in one line per job.
    pub fn render(&self) -> Vec<String> {
        self.jobs.iter().map(Job::render).collect()
    }
}

/// Replaces every occurrence of each secret with a marker.
///
/// Used on a line before it is logged: a command line can carry a token even when the queue's own
/// types cannot.
pub fn redact(text: &str, secrets: &[&str]) -> String {
    let mut redacted = text.to_owned();
    for secret in secrets {
        if !secret.is_empty() {
            redacted = redacted.replace(secret, "[redacted]");
        }
    }
    redacted
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    use super::sign_with;
    use super::{
        Action, Artifact, CredentialRef, CredentialSource, Proof, Queue, Refusal, SignatureState,
        Signing, Summary, redact, sign,
    };
    use crate::app::targets::configuration::Os;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    static SERIAL: AtomicU32 = AtomicU32::new(0);

    fn artifact(name: &str, bytes: &[u8]) -> Artifact {
        let id = SERIAL.fetch_add(1, Ordering::Relaxed);
        let path: PathBuf = std::env::temp_dir().join(format!(
            "rustage-target-queue-{}-{id}-{name}",
            std::process::id()
        ));
        std::fs::write(&path, bytes).expect("a temporary artifact");
        Artifact::of_file("job", &path).expect("an artifact")
    }

    #[test]
    fn actions_and_sources_round_trip_through_their_names() {
        for action in [Action::Build, Action::Deploy, Action::Sign] {
            assert_eq!(Action::parse(action.name()), Some(action));
        }
        for source in [
            CredentialSource::Environment,
            CredentialSource::Keychain,
            CredentialSource::File,
        ] {
            assert_eq!(CredentialSource::parse(source.name()), Some(source));
        }
        assert!(Action::parse("upload").is_none());
    }

    #[test]
    fn the_next_job_is_the_first_waiting_one() {
        let mut queue = Queue::new();
        queue.enqueue("a", "desktop", Action::Build).expect("a job");
        queue.enqueue("b", "mobile", Action::Deploy).expect("a job");
        assert_eq!(queue.next().expect("a job").id, "a");
        queue.start("a").expect("a start");
        assert_eq!(queue.next().expect("a job").id, "b");
    }

    #[test]
    fn an_empty_queue_is_not_a_success() {
        assert!(!Queue::new().is_success());
    }

    #[test]
    fn a_duplicate_id_is_refused() {
        let mut queue = Queue::new();
        queue
            .enqueue("build", "desktop", Action::Build)
            .expect("a job");
        assert!(matches!(
            queue.enqueue("build", "mobile", Action::Build),
            Err(Refusal::DuplicateJob { .. })
        ));
    }

    #[test]
    fn one_failed_job_keeps_the_queue_from_succeeding() {
        let mut queue = Queue::new();
        queue.enqueue("a", "desktop", Action::Build).expect("a job");
        queue.enqueue("b", "windows", Action::Build).expect("a job");
        queue
            .pass("a", Proof::Ran, Some(artifact("a", b"binary")))
            .expect("a pass");
        queue.fail("b", "the SDK is missing").expect("a failure");
        assert!(!queue.is_success());
        let summary = queue.summary();
        assert_eq!(
            summary,
            Summary {
                total: 2,
                done: 1,
                failed: 1,
                ..Summary::default()
            }
        );
        assert!(summary.render().contains("1 failed"));
    }

    #[test]
    fn a_waiting_or_cancelled_job_also_keeps_it_from_succeeding() {
        let mut queue = Queue::new();
        queue.enqueue("a", "desktop", Action::Build).expect("a job");
        queue.enqueue("b", "mobile", Action::Build).expect("a job");
        queue
            .pass("a", Proof::Ran, Some(artifact("a", b"binary")))
            .expect("a pass");
        assert!(!queue.is_success());
        queue.cancel("b").expect("a cancellation");
        assert!(!queue.is_success());
        assert_eq!(queue.summary().cancelled, 1);
    }

    #[test]
    fn artifacts_come_only_from_finished_jobs() {
        let mut queue = Queue::new();
        queue.enqueue("a", "desktop", Action::Build).expect("a job");
        queue.enqueue("b", "windows", Action::Build).expect("a job");
        queue
            .pass("a", Proof::Built, Some(artifact("a", b"binary")))
            .expect("a pass");
        queue.fail("b", "failed").expect("a failure");
        let artifacts = queue.artifacts();
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].job, "job");
        assert!(artifacts[0].render().contains("bytes"));
    }

    #[test]
    fn a_missing_artifact_is_refused_by_measurement() {
        let missing = Artifact::of_file("job", "/definitely/not/here");
        assert!(missing.is_err());
    }

    #[test]
    fn signing_records_a_reference_and_no_secret() {
        let secret = "super-secret-token";
        let mut queue = Queue::new();
        queue.enqueue("sign", "macos", Action::Sign).expect("a job");
        queue
            .attach_signing(
                "sign",
                Signing::new(
                    "Developer ID Application",
                    CredentialRef::new("RUSTAGE_SIGNING_TOKEN", CredentialSource::Environment),
                ),
            )
            .expect("a signing reference");
        queue.pass("sign", Proof::Built, None).expect("a pass");
        let rendered = queue.render().join("\n");
        assert!(rendered.contains("RUSTAGE_SIGNING_TOKEN"));
        assert!(rendered.contains("reference only"));
        assert!(!rendered.contains(secret));
    }

    #[test]
    fn a_signing_job_without_a_reference_does_not_pass() {
        let mut queue = Queue::new();
        queue.enqueue("sign", "macos", Action::Sign).expect("a job");
        assert!(matches!(
            queue.pass("sign", Proof::Built, None),
            Err(Refusal::SignatureMissing { .. })
        ));
    }

    #[test]
    fn a_signing_reference_cannot_be_attached_to_a_build() {
        let mut queue = Queue::new();
        queue
            .enqueue("build", "desktop", Action::Build)
            .expect("a job");
        let signing = Signing::new(
            "identity",
            CredentialRef::new("token", CredentialSource::Keychain),
        );
        assert!(matches!(
            queue.attach_signing("build", signing),
            Err(Refusal::NotSigning { .. })
        ));
    }

    #[test]
    fn a_credential_reference_checks_presence_without_reading_a_value() {
        let missing = CredentialRef::new(
            "RUSTAGE_TEST_CREDENTIAL_THAT_IS_NOT_SET",
            CredentialSource::Environment,
        );
        assert!(matches!(
            missing.check(Os::Macos),
            Err(Refusal::CredentialMissing { .. })
        ));
        let directory = std::env::temp_dir();
        let file = CredentialRef::new(directory.display().to_string(), CredentialSource::File);
        assert!(matches!(
            file.check(Os::Macos),
            Err(Refusal::CredentialMissing { .. })
        ));
        let keychain = CredentialRef::new("item", CredentialSource::Keychain);
        assert!(matches!(
            keychain.check(Os::Linux),
            Err(Refusal::CredentialUnverifiable { .. })
        ));
    }

    #[test]
    fn a_signing_job_with_an_unresolved_credential_does_not_run_the_tool() {
        let path = PathBuf::from("/tmp/rustage-never-signed");
        let mut queue = Queue::new();
        queue.enqueue("sign", "macos", Action::Sign).expect("a job");
        let refusal = queue
            .sign("sign", &path, Os::Macos)
            .expect_err("no credential");
        assert!(matches!(refusal, Refusal::SignatureMissing { .. }));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn codesign_signs_and_verifies_a_real_mach_o_copy() {
        let id = SERIAL.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "rustage-codesign-{}-{id}-binary",
            std::process::id()
        ));
        std::fs::copy(std::env::current_exe().expect("the test binary"), &path)
            .expect("a copy of the test binary");
        let signature = sign("-", &path, Os::Macos).expect("a signature attempt");
        assert_eq!(signature.program, "codesign");
        assert_eq!(
            signature.state,
            SignatureState::Signed,
            "{}",
            signature.render()
        );
        assert!(signature.render().contains("signed with -"));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_windows_host_reports_the_missing_adapter_instead_of_signing() {
        let directory = std::env::temp_dir();
        let path = directory.join(format!("rustage-sign-unavailable-{}", std::process::id()));
        std::fs::write(&path, b"artifact").expect("a temporary artifact");
        let signature = sign("identity", &path, Os::Windows).expect("an answer");
        assert!(matches!(
            signature.state,
            SignatureState::Unavailable { .. }
        ));
        assert!(signature.render().contains("no windows signing adapter"));
        std::fs::remove_file(&path).ok();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn gpg_signs_and_verifies_a_real_file_with_a_throwaway_key() {
        // The test creates its own keyring so it never touches the user's keys. A machine without
        // `gpg` cannot exercise the real path; it reports that rather than pretending a signature
        // happened.
        if std::process::Command::new("gpg")
            .arg("--version")
            .output()
            .map(|output| !output.status.success())
            .unwrap_or(true)
        {
            eprintln!("gpg is not installed; the real signing path was not exercised");
            return;
        }
        let id = SERIAL.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("rustage-gpg-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&root).expect("a temporary keyring");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700));
        }
        let generated = std::process::Command::new("gpg")
            .arg("--homedir")
            .arg(&root)
            .args([
                "--batch",
                "--passphrase",
                "",
                "--quick-generate-key",
                "Rustage Test <rustage@example.invalid>",
                "default",
                "default",
                "0",
            ])
            .output()
            .expect("gpg starts");
        assert!(
            generated.status.success(),
            "key generation failed: {}",
            String::from_utf8_lossy(&generated.stderr)
        );
        let listing = std::process::Command::new("gpg")
            .arg("--homedir")
            .arg(&root)
            .args(["--batch", "--with-colons", "--list-secret-keys"])
            .output()
            .expect("gpg lists keys");
        let fingerprint = String::from_utf8_lossy(&listing.stdout)
            .lines()
            .filter(|line| line.starts_with("fpr:"))
            .find_map(|line| line.split(':').nth(9))
            .expect("a fingerprint")
            .to_owned();

        let artifact = root.join("artifact.bin");
        std::fs::write(&artifact, b"signed bytes").expect("a temporary artifact");
        let signature = sign_with(&fingerprint, &artifact, Os::Linux, Some(&root))
            .expect("a signature attempt");
        assert_eq!(signature.program, "gpg");
        assert_eq!(
            signature.state,
            SignatureState::Signed,
            "{}",
            signature.render()
        );
        assert!(artifact.with_extension("bin.asc").is_file());

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn redaction_removes_a_secret_from_a_command_line() {
        let secret = "abc123";
        let line = format!("SIGN_TOKEN={secret} adb install app.apk");
        let redacted = redact(&line, &[secret, ""]);
        assert!(!redacted.contains(secret));
        assert!(redacted.contains("[redacted]"));
        assert!(redacted.contains("adb install app.apk"));
    }

    #[test]
    fn a_second_artifact_for_one_job_is_refused() {
        let mut queue = Queue::new();
        queue
            .enqueue("build", "desktop", Action::Build)
            .expect("a job");
        queue
            .pass("build", Proof::Built, Some(artifact("one", b"one")))
            .expect("a pass");
        assert!(matches!(
            queue.pass("build", Proof::Built, Some(artifact("two", b"two"))),
            Err(Refusal::ArtifactAlreadyRecorded { .. })
        ));
    }
}
